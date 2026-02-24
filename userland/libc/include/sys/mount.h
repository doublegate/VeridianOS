/*
 * VeridianOS libc -- <sys/mount.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Mount/umount syscall stubs and mount flags.
 */

#ifndef _SYS_MOUNT_H
#define _SYS_MOUNT_H

#ifdef __cplusplus
extern "C" {
#endif

/* Mount flags */
#define MS_RDONLY       (1 << 0)
#define MS_NOSUID       (1 << 1)
#define MS_NODEV        (1 << 2)
#define MS_NOEXEC       (1 << 3)
#define MS_SYNCHRONOUS  (1 << 4)
#define MS_REMOUNT      (1 << 5)
#define MS_MANDLOCK     (1 << 6)
#define MS_DIRSYNC      (1 << 7)
#define MS_NOATIME      (1 << 10)
#define MS_NODIRATIME   (1 << 11)
#define MS_BIND         (1 << 12)
#define MS_MOVE         (1 << 13)
#define MS_SILENT       (1 << 15)

/* umount2 flags */
#define MNT_FORCE       1
#define MNT_DETACH      2
#define MNT_EXPIRE      4
#define UMOUNT_NOFOLLOW 8

int mount(const char *source, const char *target,
          const char *filesystemtype, unsigned long mountflags,
          const void *data);
int umount(const char *target);
int umount2(const char *target, int flags);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_MOUNT_H */
