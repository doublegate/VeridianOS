/*
 * VeridianOS libc -- xattr.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Extended attributes stub implementation.  Provides the POSIX xattr
 * API surface needed by KIO for file metadata operations.
 *
 * This initial implementation uses a simple in-memory store.  Extended
 * attributes are not yet persisted to the filesystem -- they last only
 * for the lifetime of the process.  A production implementation would
 * store xattrs in the filesystem's inode metadata.
 *
 * Limitations:
 *   - Maximum 256 xattr entries (across all files)
 *   - Maximum 255 bytes per attribute name
 *   - Maximum 4096 bytes per attribute value
 *   - Path-based and fd-based calls share the same pool
 *   - lgetxattr/lsetxattr are aliases (symlinks not yet distinct)
 */

#include <sys/xattr.h>
#include <errno.h>
#include <string.h>
#include <stddef.h>

/* ========================================================================= */
/* Internal storage                                                          */
/* ========================================================================= */

#define MAX_XATTR_ENTRIES   256
#define MAX_XATTR_NAME_LEN  255
#define MAX_XATTR_VALUE_LEN 4096
#define MAX_XATTR_PATH_LEN  1024

struct xattr_entry {
    int    in_use;
    char   path[MAX_XATTR_PATH_LEN];
    char   name[MAX_XATTR_NAME_LEN + 1];
    char   value[MAX_XATTR_VALUE_LEN];
    size_t value_len;
};

static struct xattr_entry s_xattr_pool[MAX_XATTR_ENTRIES];

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

/**
 * Find an xattr entry by path + name.
 * Returns index or -1 if not found.
 */
static int find_xattr(const char *path, const char *name)
{
    for (int i = 0; i < MAX_XATTR_ENTRIES; i++) {
        if (s_xattr_pool[i].in_use &&
            strcmp(s_xattr_pool[i].path, path) == 0 &&
            strcmp(s_xattr_pool[i].name, name) == 0) {
            return i;
        }
    }
    return -1;
}

/**
 * Find a free slot in the pool.
 * Returns index or -1 if full.
 */
static int alloc_xattr(void)
{
    for (int i = 0; i < MAX_XATTR_ENTRIES; i++) {
        if (!s_xattr_pool[i].in_use)
            return i;
    }
    return -1;
}

/* ========================================================================= */
/* Path-based operations                                                     */
/* ========================================================================= */

ssize_t getxattr(const char *path, const char *name,
                 void *value, size_t size)
{
    if (!path || !name) {
        errno = EINVAL;
        return -1;
    }

    int idx = find_xattr(path, name);
    if (idx < 0) {
        errno = ENODATA;
        return -1;
    }

    size_t vlen = s_xattr_pool[idx].value_len;

    /* If value is NULL, return size needed */
    if (!value)
        return (ssize_t)vlen;

    if (size < vlen) {
        errno = ERANGE;
        return -1;
    }

    memcpy(value, s_xattr_pool[idx].value, vlen);
    return (ssize_t)vlen;
}

int setxattr(const char *path, const char *name,
             const void *value, size_t size, int flags)
{
    if (!path || !name) {
        errno = EINVAL;
        return -1;
    }

    if (strlen(name) > MAX_XATTR_NAME_LEN) {
        errno = ERANGE;
        return -1;
    }

    if (size > MAX_XATTR_VALUE_LEN) {
        errno = E2BIG;
        return -1;
    }

    if (strlen(path) >= MAX_XATTR_PATH_LEN) {
        errno = ENAMETOOLONG;
        return -1;
    }

    int idx = find_xattr(path, name);

    if (flags & XATTR_CREATE) {
        if (idx >= 0) {
            errno = EEXIST;
            return -1;
        }
    }

    if (flags & XATTR_REPLACE) {
        if (idx < 0) {
            errno = ENODATA;
            return -1;
        }
    }

    if (idx < 0) {
        /* Allocate new entry */
        idx = alloc_xattr();
        if (idx < 0) {
            errno = ENOSPC;
            return -1;
        }
        s_xattr_pool[idx].in_use = 1;
        strncpy(s_xattr_pool[idx].path, path, MAX_XATTR_PATH_LEN - 1);
        s_xattr_pool[idx].path[MAX_XATTR_PATH_LEN - 1] = '\0';
        strncpy(s_xattr_pool[idx].name, name, MAX_XATTR_NAME_LEN);
        s_xattr_pool[idx].name[MAX_XATTR_NAME_LEN] = '\0';
    }

    /* Store value */
    if (value && size > 0) {
        memcpy(s_xattr_pool[idx].value, value, size);
    }
    s_xattr_pool[idx].value_len = size;

    return 0;
}

int removexattr(const char *path, const char *name)
{
    if (!path || !name) {
        errno = EINVAL;
        return -1;
    }

    int idx = find_xattr(path, name);
    if (idx < 0) {
        errno = ENODATA;
        return -1;
    }

    s_xattr_pool[idx].in_use = 0;
    return 0;
}

ssize_t listxattr(const char *path, char *list, size_t size)
{
    if (!path) {
        errno = EINVAL;
        return -1;
    }

    /* Calculate total size needed */
    size_t total = 0;
    for (int i = 0; i < MAX_XATTR_ENTRIES; i++) {
        if (s_xattr_pool[i].in_use &&
            strcmp(s_xattr_pool[i].path, path) == 0) {
            total += strlen(s_xattr_pool[i].name) + 1; /* +1 for NUL */
        }
    }

    if (!list)
        return (ssize_t)total;

    if (size < total) {
        errno = ERANGE;
        return -1;
    }

    /* Fill list with NUL-separated names */
    size_t offset = 0;
    for (int i = 0; i < MAX_XATTR_ENTRIES; i++) {
        if (s_xattr_pool[i].in_use &&
            strcmp(s_xattr_pool[i].path, path) == 0) {
            size_t nlen = strlen(s_xattr_pool[i].name) + 1;
            memcpy(list + offset, s_xattr_pool[i].name, nlen);
            offset += nlen;
        }
    }

    return (ssize_t)total;
}

/* ========================================================================= */
/* Symlink-aware operations (aliases -- VeridianOS doesn't distinguish yet)  */
/* ========================================================================= */

ssize_t lgetxattr(const char *path, const char *name,
                  void *value, size_t size)
{
    return getxattr(path, name, value, size);
}

int lsetxattr(const char *path, const char *name,
              const void *value, size_t size, int flags)
{
    return setxattr(path, name, value, size, flags);
}

int lremovexattr(const char *path, const char *name)
{
    return removexattr(path, name);
}

ssize_t llistxattr(const char *path, char *list, size_t size)
{
    return listxattr(path, list, size);
}

/* ========================================================================= */
/* File descriptor-based operations                                          */
/* ========================================================================= */

/*
 * fd-based xattr operations use a synthetic path "/proc/self/fd/<fd>"
 * to key into the same pool.  In a real implementation, these would
 * operate on the inode directly via the file descriptor.
 */

static void fd_to_path(int fd, char *buf, size_t bufsize)
{
    /* Construct a synthetic path for fd-based lookups */
    int n = 0;
    char tmp[32];
    int val = fd < 0 ? 0 : fd;

    /* Manual int-to-string to avoid printf dependency */
    if (val == 0) {
        tmp[0] = '0';
        n = 1;
    } else {
        while (val > 0 && n < 30) {
            tmp[n++] = '0' + (char)(val % 10);
            val /= 10;
        }
    }

    const char *prefix = "/proc/self/fd/";
    size_t plen = strlen(prefix);
    if (plen + (size_t)n >= bufsize) {
        buf[0] = '\0';
        return;
    }
    memcpy(buf, prefix, plen);
    /* Reverse the digits */
    for (int i = 0; i < n; i++) {
        buf[plen + (size_t)i] = tmp[n - 1 - i];
    }
    buf[plen + (size_t)n] = '\0';
}

ssize_t fgetxattr(int fd, const char *name,
                  void *value, size_t size)
{
    char path[64];
    fd_to_path(fd, path, sizeof(path));
    return getxattr(path, name, value, size);
}

int fsetxattr(int fd, const char *name,
              const void *value, size_t size, int flags)
{
    char path[64];
    fd_to_path(fd, path, sizeof(path));
    return setxattr(path, name, value, size, flags);
}

int fremovexattr(int fd, const char *name)
{
    char path[64];
    fd_to_path(fd, path, sizeof(path));
    return removexattr(path, name);
}

ssize_t flistxattr(int fd, char *list, size_t size)
{
    char path[64];
    fd_to_path(fd, path, sizeof(path));
    return listxattr(path, list, size);
}
