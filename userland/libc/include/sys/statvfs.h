/*
 * VeridianOS libc -- <sys/statvfs.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Filesystem statistics (stub).
 */

#ifndef _SYS_STATVFS_H
#define _SYS_STATVFS_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef unsigned long fsblkcnt_t;
typedef unsigned long fsfilcnt_t;

struct statvfs {
    unsigned long f_bsize;    /* Filesystem block size */
    unsigned long f_frsize;   /* Fragment size */
    fsblkcnt_t    f_blocks;   /* Total blocks */
    fsblkcnt_t    f_bfree;    /* Free blocks */
    fsblkcnt_t    f_bavail;   /* Free blocks for non-root */
    fsfilcnt_t    f_files;    /* Total inodes */
    fsfilcnt_t    f_ffree;    /* Free inodes */
    fsfilcnt_t    f_favail;   /* Free inodes for non-root */
    unsigned long f_fsid;     /* Filesystem ID */
    unsigned long f_flag;     /* Mount flags */
    unsigned long f_namemax;  /* Max filename length */
};

/* Mount flags */
#define ST_RDONLY    1
#define ST_NOSUID    2

int statvfs(const char *path, struct statvfs *buf);
int fstatvfs(int fd, struct statvfs *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_STATVFS_H */
