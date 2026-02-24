/*
 * VeridianOS libc -- <mntent.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Mount table parsing (stub -- returns empty mount table).
 */

#ifndef _MNTENT_H
#define _MNTENT_H

#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MOUNTED     "/etc/mtab"
#define MNTTAB      "/etc/fstab"

struct mntent {
    char *mnt_fsname;   /* Device or server name */
    char *mnt_dir;      /* Mount point */
    char *mnt_type;     /* Filesystem type */
    char *mnt_opts;     /* Mount options */
    int   mnt_freq;     /* Dump frequency */
    int   mnt_passno;   /* fsck pass number */
};

FILE *setmntent(const char *filename, const char *type);
struct mntent *getmntent(FILE *stream);
int endmntent(FILE *stream);
char *hasmntopt(const struct mntent *mnt, const char *opt);

#ifdef __cplusplus
}
#endif

#endif /* _MNTENT_H */
