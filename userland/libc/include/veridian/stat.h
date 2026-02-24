/*
 * VeridianOS File Status Definitions
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * struct stat and related macros for file metadata.
 * Layout matches the kernel's FileStat structure in syscall/filesystem.rs.
 */

#ifndef VERIDIAN_STAT_H
#define VERIDIAN_STAT_H

#include <veridian/types.h>
#include <time.h>  /* struct timespec */

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* struct stat                                                               */
/* ========================================================================= */

struct stat {
    dev_t       st_dev;     /* Device ID of file */
    ino_t       st_ino;     /* Inode number */
    mode_t      st_mode;    /* File type and permission bits */
    nlink_t     st_nlink;   /* Number of hard links */
    uid_t       st_uid;     /* Owner user ID */
    gid_t       st_gid;     /* Owner group ID */
    dev_t       st_rdev;    /* Device ID (for special files) */
    off_t       st_size;    /* File size in bytes */
    blksize_t   st_blksize; /* Preferred I/O block size */
    blkcnt_t    st_blocks;  /* Number of 512-byte blocks allocated */
    struct timespec st_atim; /* Last access time */
    struct timespec st_mtim; /* Last modification time */
    struct timespec st_ctim; /* Last status change time */
};

/* POSIX compatibility: st_atime is shorthand for st_atim.tv_sec */
#define st_atime st_atim.tv_sec
#define st_mtime st_mtim.tv_sec
#define st_ctime st_ctim.tv_sec

/* ========================================================================= */
/* File Type Masks                                                           */
/* ========================================================================= */

/** Bitmask for file type field */
#define S_IFMT      0170000

/** Socket */
#define S_IFSOCK    0140000
/** Symbolic link */
#define S_IFLNK     0120000
/** Regular file */
#define S_IFREG     0100000
/** Block device */
#define S_IFBLK     0060000
/** Directory */
#define S_IFDIR     0040000
/** Character device */
#define S_IFCHR     0020000
/** Named pipe (FIFO) */
#define S_IFIFO     0010000

/* ========================================================================= */
/* File Type Test Macros                                                     */
/* ========================================================================= */

/** Test for regular file */
#define S_ISREG(m)  (((m) & S_IFMT) == S_IFREG)
/** Test for directory */
#define S_ISDIR(m)  (((m) & S_IFMT) == S_IFDIR)
/** Test for character device */
#define S_ISCHR(m)  (((m) & S_IFMT) == S_IFCHR)
/** Test for block device */
#define S_ISBLK(m)  (((m) & S_IFMT) == S_IFBLK)
/** Test for FIFO (named pipe) */
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
/** Test for symbolic link */
#define S_ISLNK(m)  (((m) & S_IFMT) == S_IFLNK)
/** Test for socket */
#define S_ISSOCK(m) (((m) & S_IFMT) == S_IFSOCK)

/* ========================================================================= */
/* Permission Bits                                                           */
/* ========================================================================= */

/* Owner permissions */
#define S_IRWXU     0700    /* Owner: read, write, execute */
#define S_IRUSR     0400    /* Owner: read */
#define S_IWUSR     0200    /* Owner: write */
#define S_IXUSR     0100    /* Owner: execute */

/* Group permissions */
#define S_IRWXG     0070    /* Group: read, write, execute */
#define S_IRGRP     0040    /* Group: read */
#define S_IWGRP     0020    /* Group: write */
#define S_IXGRP     0010    /* Group: execute */

/* Other permissions */
#define S_IRWXO     0007    /* Other: read, write, execute */
#define S_IROTH     0004    /* Other: read */
#define S_IWOTH     0002    /* Other: write */
#define S_IXOTH     0001    /* Other: execute */

/* Special permission bits */
#define S_ISUID     04000   /* Set-user-ID on execution */
#define S_ISGID     02000   /* Set-group-ID on execution */
#define S_ISVTX     01000   /* Sticky bit */

/* ========================================================================= */
/* Function Declarations                                                     */
/* ========================================================================= */

/**
 * Get file status by path.
 *
 * @param pathname  Path to the file.
 * @param statbuf   Buffer to receive file status.
 * @return 0 on success, -1 on error.
 */
int stat(const char *pathname, struct stat *statbuf);

/**
 * Get file status by file descriptor.
 *
 * @param fd        Open file descriptor.
 * @param statbuf   Buffer to receive file status.
 * @return 0 on success, -1 on error.
 */
int fstat(int fd, struct stat *statbuf);

/**
 * Get file status (do not follow symlinks).
 *
 * @param pathname  Path to the file.
 * @param statbuf   Buffer to receive file status.
 * @return 0 on success, -1 on error.
 */
int lstat(const char *pathname, struct stat *statbuf);

/**
 * Create a directory.
 *
 * @param pathname  Path for the new directory.
 * @param mode      Permission bits.
 * @return 0 on success, -1 on error.
 */
int mkdir(const char *pathname, mode_t mode);

/**
 * Change file permission bits.
 *
 * @param pathname  Path to the file.
 * @param mode      New permission bits.
 * @return 0 on success, -1 on error.
 */
int chmod(const char *pathname, mode_t mode);

/**
 * Set the file mode creation mask.
 *
 * @param mask  New umask value.
 * @return Previous umask value.
 */
mode_t umask(mode_t mask);

/**
 * Create a special (or ordinary) file.
 *
 * @param pathname  Path for the new node.
 * @param mode      File type and permission bits.
 * @param dev       Device number (for S_IFBLK/S_IFCHR).
 * @return 0 on success, -1 on error.
 */
int mknod(const char *pathname, mode_t mode, dev_t dev);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_STAT_H */
