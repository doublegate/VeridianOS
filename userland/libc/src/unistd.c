/*
 * VeridianOS libc -- unistd.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX-like functions that are not direct 1:1 syscall wrappers.
 * The raw syscall wrappers (read, write, fork, etc.) are in syscall.c.
 * This file provides composite functions built on top of them.
 */

#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <time.h>
#include <sys/ioctl.h>

/* ========================================================================= */
/* Sleep                                                                     */
/* ========================================================================= */

unsigned int sleep(unsigned int seconds)
{
    struct timespec req = { .tv_sec = (time_t)seconds, .tv_nsec = 0 };
    struct timespec rem = { 0, 0 };

    if (nanosleep(&req, &rem) == 0)
        return 0;

    /* If interrupted, return remaining seconds (rounded up). */
    return (unsigned int)rem.tv_sec + (rem.tv_nsec > 0 ? 1 : 0);
}

int usleep(unsigned int usec)
{
    struct timespec req;
    req.tv_sec  = (time_t)(usec / 1000000);
    req.tv_nsec = (long)(usec % 1000000) * 1000;
    return nanosleep(&req, NULL);
}

/* ========================================================================= */
/* exec family                                                               */
/* ========================================================================= */

/*
 * execvp: search PATH for the program.
 * This is a simplified implementation that only looks in a few
 * hard-coded directories if there is no '/' in the filename.
 */
int execvp(const char *file, char *const argv[])
{
    /* If file contains a slash, use it directly. */
    if (strchr(file, '/'))
        return execve(file, argv, environ);

    /* Search a basic PATH. */
    static const char *search_dirs[] = {
        "/bin", "/usr/bin", "/sbin", "/usr/sbin", NULL
    };

    /* Try PATH from environment first. */
    const char *path_env = getenv("PATH");
    if (path_env) {
        char buf[256];
        const char *p = path_env;
        while (*p) {
            const char *end = p;
            while (*end && *end != ':')
                end++;
            size_t dirlen = (size_t)(end - p);
            if (dirlen > 0 && dirlen + 1 + strlen(file) + 1 <= sizeof(buf)) {
                memcpy(buf, p, dirlen);
                buf[dirlen] = '/';
                strcpy(buf + dirlen + 1, file);
                execve(buf, argv, environ);
                /* If execve returns, the file wasn't found/executable. */
            }
            if (*end == ':')
                end++;
            p = end;
        }
    } else {
        /* Fallback: search hard-coded directories. */
        for (int i = 0; search_dirs[i]; i++) {
            char buf[256];
            size_t dlen = strlen(search_dirs[i]);
            size_t flen = strlen(file);
            if (dlen + 1 + flen + 1 > sizeof(buf))
                continue;
            memcpy(buf, search_dirs[i], dlen);
            buf[dlen] = '/';
            memcpy(buf + dlen + 1, file, flen + 1);
            execve(buf, argv, environ);
        }
    }

    errno = ENOENT;
    return -1;
}

/* ========================================================================= */
/* Miscellaneous                                                             */
/* ========================================================================= */

int gethostname(char *name, size_t len)
{
    static const char hostname[] = "veridian";
    size_t n = sizeof(hostname);
    if (n > len)
        n = len;
    memcpy(name, hostname, n);
    return 0;
}

int isatty(int fd)
{
    struct winsize ws;
    if (ioctl(fd, TIOCGWINSZ, &ws) == 0)
        return 1;
    /* ioctl failed — check if errno suggests "not a terminal" */
    if (errno != ENOTTY)
        errno = ENOTTY;
    return 0;
}

/* ========================================================================= */
/* Path resolution                                                           */
/* ========================================================================= */

/* PATH_MAX -- no <limits.h> yet, define locally. */
#ifndef PATH_MAX
#define PATH_MAX 4096
#endif

/*
 * realpath: resolve a pathname to an absolute, canonical form.
 *
 * Handles:
 *   - Relative paths (prepend cwd)
 *   - "." (current directory)
 *   - ".." (parent directory)
 *   - Consecutive slashes
 *   - Trailing slashes
 *
 * Does NOT follow symlinks (no readlink loop) -- adequate for early userland.
 * If resolved_path is NULL, a buffer is malloc'd for the caller to free.
 */
char *realpath(const char *path, char *resolved_path)
{
    if (!path || *path == '\0') {
        errno = EINVAL;
        return NULL;
    }

    char *buf = resolved_path;
    if (!buf) {
        buf = (char *)malloc(PATH_MAX);
        if (!buf) {
            errno = ENOMEM;
            return NULL;
        }
    }

    /*
     * Build the absolute path in a working buffer, then copy resolved
     * components into buf.
     */
    char work[PATH_MAX];

    if (path[0] != '/') {
        /* Relative path: prepend cwd. */
        if (!getcwd(work, sizeof(work))) {
            if (!resolved_path) free(buf);
            return NULL;  /* errno set by getcwd */
        }
        size_t cwdlen = strlen(work);
        if (cwdlen + 1 + strlen(path) + 1 > sizeof(work)) {
            if (!resolved_path) free(buf);
            errno = ENAMETOOLONG;
            return NULL;
        }
        work[cwdlen] = '/';
        strcpy(work + cwdlen + 1, path);
    } else {
        if (strlen(path) >= sizeof(work)) {
            if (!resolved_path) free(buf);
            errno = ENAMETOOLONG;
            return NULL;
        }
        strcpy(work, path);
    }

    /*
     * Now walk through 'work' component by component, building the
     * canonical result in 'buf'.
     */
    buf[0] = '/';
    buf[1] = '\0';
    size_t blen = 1;

    const char *p = work;
    while (*p) {
        /* Skip slashes. */
        while (*p == '/')
            p++;
        if (*p == '\0')
            break;

        /* Find end of this component. */
        const char *end = p;
        while (*end && *end != '/')
            end++;
        size_t clen = (size_t)(end - p);

        if (clen == 1 && p[0] == '.') {
            /* "." -- stay in current directory. */
        } else if (clen == 2 && p[0] == '.' && p[1] == '.') {
            /* ".." -- go up one level. */
            if (blen > 1) {
                /* Remove trailing slash if present. */
                if (buf[blen - 1] == '/')
                    blen--;
                /* Remove last component. */
                while (blen > 1 && buf[blen - 1] != '/')
                    blen--;
                /* Keep the root slash. */
                if (blen == 0)
                    blen = 1;
                buf[blen] = '\0';
            }
        } else {
            /* Normal component -- append. */
            /* Ensure trailing slash on current buf. */
            if (blen > 1 && buf[blen - 1] != '/') {
                if (blen + 1 >= PATH_MAX) {
                    if (!resolved_path) free(buf);
                    errno = ENAMETOOLONG;
                    return NULL;
                }
                buf[blen++] = '/';
            }
            if (blen + clen >= PATH_MAX) {
                if (!resolved_path) free(buf);
                errno = ENAMETOOLONG;
                return NULL;
            }
            memcpy(buf + blen, p, clen);
            blen += clen;
            buf[blen] = '\0';
        }

        p = end;
    }

    /* Ensure we never return an empty string. */
    if (blen == 0) {
        buf[0] = '/';
        buf[1] = '\0';
    }

    return buf;
}
