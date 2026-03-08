/*
 * VeridianOS C Library -- <sys/inotify.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Filesystem event notification interface.  Monitors files and
 * directories for changes (create, delete, modify, move, etc.).
 * Used by Qt 6 QFileSystemWatcher for real-time file monitoring.
 */

#ifndef _SYS_INOTIFY_H
#define _SYS_INOTIFY_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* inotify_init1 flags                                                       */
/* ========================================================================= */

#define IN_CLOEXEC      0x80000 /* O_CLOEXEC */
#define IN_NONBLOCK     0x00800 /* O_NONBLOCK */

/* ========================================================================= */
/* Watch event mask bits                                                     */
/* ========================================================================= */

#define IN_ACCESS        0x00000001  /* File was accessed (read) */
#define IN_MODIFY        0x00000002  /* File was modified (write) */
#define IN_ATTRIB        0x00000004  /* Metadata changed */
#define IN_CLOSE_WRITE   0x00000008  /* Writable file was closed */
#define IN_CLOSE_NOWRITE 0x00000010  /* Non-writable file was closed */
#define IN_OPEN          0x00000020  /* File was opened */
#define IN_MOVED_FROM    0x00000040  /* File moved out of watched dir */
#define IN_MOVED_TO      0x00000080  /* File moved into watched dir */
#define IN_CREATE        0x00000100  /* File/dir created in watched dir */
#define IN_DELETE        0x00000200  /* File/dir deleted from watched dir */
#define IN_DELETE_SELF   0x00000400  /* Watched file/dir was deleted */
#define IN_MOVE_SELF     0x00000800  /* Watched file/dir was moved */

/* Convenience macros */
#define IN_CLOSE         (IN_CLOSE_WRITE | IN_CLOSE_NOWRITE)
#define IN_MOVE          (IN_MOVED_FROM | IN_MOVED_TO)
#define IN_ALL_EVENTS    (IN_ACCESS | IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE \
                         | IN_CLOSE_NOWRITE | IN_OPEN | IN_MOVED_FROM        \
                         | IN_MOVED_TO | IN_CREATE | IN_DELETE               \
                         | IN_DELETE_SELF | IN_MOVE_SELF)

/* Additional flags for inotify_add_watch */
#define IN_DONT_FOLLOW   0x02000000  /* Don't follow symlinks */
#define IN_EXCL_UNLINK   0x04000000  /* Exclude events on unlinked objects */
#define IN_MASK_CREATE   0x10000000  /* Only create watches */
#define IN_MASK_ADD      0x20000000  /* Add to existing watch mask */
#define IN_ONLYDIR       0x01000000  /* Only watch if path is a directory */
#define IN_ONESHOT       0x80000000  /* Only send event once */

/* Flags returned in the event mask */
#define IN_IGNORED       0x00008000  /* Watch was removed */
#define IN_ISDIR         0x40000000  /* Event subject is a directory */
#define IN_Q_OVERFLOW    0x00004000  /* Event queue overflowed */
#define IN_UNMOUNT       0x00002000  /* Filesystem was unmounted */

/* ========================================================================= */
/* Data structures                                                           */
/* ========================================================================= */

/**
 * Inotify event structure.  Read from the inotify fd.
 *
 * Variable-length: the name field contains 0 to len bytes of the
 * filename (NUL-padded to alignment boundary).
 */
struct inotify_event {
    int      wd;        /* Watch descriptor */
    uint32_t mask;      /* Mask of events */
    uint32_t cookie;    /* Cookie for related events (rename) */
    uint32_t len;       /* Length of name field */
    char     name[];    /* Optional NUL-terminated filename */
};

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Create an inotify instance.
 * @return inotify file descriptor, or -1 on error.
 */
int inotify_init(void);

/**
 * Create an inotify instance with flags.
 * @param flags  IN_CLOEXEC, IN_NONBLOCK, or 0.
 * @return inotify file descriptor, or -1 on error.
 */
int inotify_init1(int flags);

/**
 * Add a watch to an inotify instance.
 * @param fd       inotify file descriptor.
 * @param pathname Path to watch.
 * @param mask     Events to watch for (IN_CREATE, IN_MODIFY, etc.).
 * @return Watch descriptor (>= 0) on success, -1 on error.
 */
int inotify_add_watch(int fd, const char *pathname, uint32_t mask);

/**
 * Remove a watch from an inotify instance.
 * @param fd   inotify file descriptor.
 * @param wd   Watch descriptor returned by inotify_add_watch().
 * @return 0 on success, -1 on error.
 */
int inotify_rm_watch(int fd, int wd);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_INOTIFY_H */
