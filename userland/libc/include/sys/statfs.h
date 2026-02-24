/*
 * VeridianOS libc -- <sys/statfs.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Filesystem statistics (Linux-compatible).
 */

#ifndef _SYS_STATFS_H
#define _SYS_STATFS_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    int val[2];
} fsid_t;

struct statfs {
    long    f_type;
    long    f_bsize;
    long    f_blocks;
    long    f_bfree;
    long    f_bavail;
    long    f_files;
    long    f_ffree;
    fsid_t  f_fsid;
    long    f_namelen;
    long    f_frsize;
    long    f_flags;
    long    f_spare[4];
};

int statfs(const char *path, struct statfs *buf);
int fstatfs(int fd, struct statfs *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_STATFS_H */
