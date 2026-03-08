/*
 * VeridianOS C Library -- <sys/xattr.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Extended attributes interface for file metadata.  Used by KIO for
 * rich file metadata storage (security labels, user tags, etc.).
 */

#ifndef _SYS_XATTR_H
#define _SYS_XATTR_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Flags for setxattr / lsetxattr / fsetxattr                                */
/* ========================================================================= */

#define XATTR_CREATE    0x1  /* Set value: fail if attr already exists */
#define XATTR_REPLACE   0x2  /* Set value: fail if attr does not exist */

/* ========================================================================= */
/* Extended attribute operations (path-based)                                */
/* ========================================================================= */

/**
 * Get an extended attribute value.
 *
 * @param path   File path.
 * @param name   Attribute name (e.g., "user.mime_type").
 * @param value  Buffer to receive value (NULL to query size).
 * @param size   Buffer size.
 * @return Value length on success, -1 on error (errno set).
 *         If value is NULL, returns the size needed.
 */
ssize_t getxattr(const char *path, const char *name,
                 void *value, size_t size);

/**
 * Set an extended attribute value.
 *
 * @param path   File path.
 * @param name   Attribute name.
 * @param value  Attribute value.
 * @param size   Value size in bytes.
 * @param flags  XATTR_CREATE, XATTR_REPLACE, or 0.
 * @return 0 on success, -1 on error (errno set).
 */
int setxattr(const char *path, const char *name,
             const void *value, size_t size, int flags);

/**
 * Remove an extended attribute.
 *
 * @param path   File path.
 * @param name   Attribute name.
 * @return 0 on success, -1 on error (errno set).
 */
int removexattr(const char *path, const char *name);

/**
 * List extended attribute names.
 *
 * @param path   File path.
 * @param list   Buffer to receive NUL-separated name list (NULL to query size).
 * @param size   Buffer size.
 * @return Total length of name list on success, -1 on error.
 */
ssize_t listxattr(const char *path, char *list, size_t size);

/* ========================================================================= */
/* Extended attribute operations (symlink-aware: do not follow symlinks)      */
/* ========================================================================= */

ssize_t lgetxattr(const char *path, const char *name,
                  void *value, size_t size);

int lsetxattr(const char *path, const char *name,
              const void *value, size_t size, int flags);

int lremovexattr(const char *path, const char *name);

ssize_t llistxattr(const char *path, char *list, size_t size);

/* ========================================================================= */
/* Extended attribute operations (file descriptor-based)                      */
/* ========================================================================= */

ssize_t fgetxattr(int fd, const char *name,
                  void *value, size_t size);

int fsetxattr(int fd, const char *name,
              const void *value, size_t size, int flags);

int fremovexattr(int fd, const char *name);

ssize_t flistxattr(int fd, char *list, size_t size);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_XATTR_H */
