/*
 * VeridianOS libc -- <dirent.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Directory entry reading: opendir, readdir, closedir, rewinddir.
 */

#ifndef _DIRENT_H
#define _DIRENT_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

/** Maximum filename length (not including NUL). */
#define NAME_MAX 255

/** Directory entry types (d_type field). */
#define DT_UNKNOWN  0
#define DT_FIFO     1
#define DT_CHR      2
#define DT_DIR      4
#define DT_BLK      6
#define DT_REG      8
#define DT_LNK      10
#define DT_SOCK     12

/** Directory entry returned by readdir(). */
struct dirent {
    ino_t           d_ino;              /* Inode number */
    unsigned char   d_type;             /* File type (DT_*) */
    char            d_name[NAME_MAX+1]; /* Null-terminated filename */
};

/** Opaque directory stream. */
typedef struct _DIR DIR;

/* ========================================================================= */
/* Functions                                                                 */
/* ========================================================================= */

/** Open a directory stream for the given path. */
DIR *opendir(const char *name);

/** Read the next directory entry. Returns NULL at end-of-directory. */
struct dirent *readdir(DIR *dirp);

/** Close a directory stream. */
int closedir(DIR *dirp);

/** Reset a directory stream to the beginning. */
void rewinddir(DIR *dirp);

/** Return the file descriptor associated with a directory stream. */
int dirfd(DIR *dirp);

#ifdef __cplusplus
}
#endif

#endif /* _DIRENT_H */
