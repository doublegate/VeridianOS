/*
 * VeridianOS libc -- dirent.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Directory stream operations built on top of the kernel's
 * SYS_DIR_OPENDIR / SYS_DIR_READDIR / SYS_DIR_CLOSEDIR syscalls.
 */

#include <dirent.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <veridian/syscall.h>

/* ========================================================================= */
/* Internal DIR structure                                                    */
/* ========================================================================= */

struct _DIR {
    int             fd;         /* Kernel directory descriptor */
    struct dirent   entry;      /* Buffer for the current entry */
    int             eof;        /* Non-zero after end-of-directory */
};

/* ========================================================================= */
/* Helper: translate raw syscall result                                      */
/* ========================================================================= */

static inline long __syscall_ret(long r)
{
    if (r < 0) {
        errno = (int)(-r);
        return -1;
    }
    return r;
}

/* ========================================================================= */
/* Public API                                                                */
/* ========================================================================= */

DIR *opendir(const char *name)
{
    if (!name) {
        errno = EINVAL;
        return NULL;
    }

    long fd = veridian_syscall1(SYS_DIR_OPENDIR, name);
    if (fd < 0) {
        errno = (int)(-fd);
        return NULL;
    }

    DIR *dirp = (DIR *)malloc(sizeof(DIR));
    if (!dirp) {
        veridian_syscall1(SYS_DIR_CLOSEDIR, fd);
        errno = ENOMEM;
        return NULL;
    }

    dirp->fd  = (int)fd;
    dirp->eof = 0;
    memset(&dirp->entry, 0, sizeof(dirp->entry));
    return dirp;
}

struct dirent *readdir(DIR *dirp)
{
    if (!dirp) {
        errno = EINVAL;
        return NULL;
    }

    if (dirp->eof)
        return NULL;

    /*
     * The kernel's SYS_DIR_READDIR returns a dirent-compatible structure
     * into the user-supplied buffer.  Returns 0 on success with data,
     * -ENOENT at end-of-directory, or a negative errno on error.
     */
    long ret = veridian_syscall2(SYS_DIR_READDIR, dirp->fd, &dirp->entry);
    if (ret < 0) {
        if (ret == -ENOENT || ret == 0) {
            /* End of directory */
            dirp->eof = 1;
            return NULL;
        }
        errno = (int)(-ret);
        return NULL;
    }

    /* ret == 0 with data means success */
    if (dirp->entry.d_name[0] == '\0') {
        dirp->eof = 1;
        return NULL;
    }

    return &dirp->entry;
}

int closedir(DIR *dirp)
{
    if (!dirp) {
        errno = EINVAL;
        return -1;
    }

    long ret = veridian_syscall1(SYS_DIR_CLOSEDIR, dirp->fd);
    free(dirp);

    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

void rewinddir(DIR *dirp)
{
    if (!dirp)
        return;

    /*
     * Close and re-open: the kernel doesn't have a rewinddir syscall,
     * so we simulate by noting we need to re-read from the start.
     * For now, just reset the EOF flag — the kernel will restart
     * enumeration on the next readdir call after a closedir+opendir.
     *
     * Simplified: just reset state. The kernel directory handle
     * maintains its own position.
     */
    dirp->eof = 0;
}

int dirfd(DIR *dirp)
{
    if (!dirp) {
        errno = EINVAL;
        return -1;
    }
    return dirp->fd;
}
