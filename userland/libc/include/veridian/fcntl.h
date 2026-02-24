/*
 * VeridianOS File Control Definitions
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Open flags, fcntl commands, and file control operations.
 * Flag values are VeridianOS-specific (not Linux ABI).
 */

#ifndef VERIDIAN_FCNTL_H
#define VERIDIAN_FCNTL_H

#include <veridian/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* File Open Flags                                                           */
/* ========================================================================= */

/*
 * Access mode flags (mutually exclusive lower 2 bits).
 * These are bitmask flags, not Linux-style 0/1/2 values.
 */
#define O_RDONLY        0x0001  /* Open for reading only */
#define O_WRONLY        0x0002  /* Open for writing only */
#define O_RDWR          0x0003  /* Open for reading and writing */
#define O_ACCMODE       0x0003  /* Mask for access mode bits */

/* Creation and status flags */
#define O_CREAT         0x0100  /* Create file if it does not exist */
#define O_TRUNC         0x0200  /* Truncate to zero length on open */
#define O_APPEND        0x0400  /* Append on each write */
#define O_EXCL          0x0800  /* Fail if file already exists (with O_CREAT) */
#define O_NONBLOCK      0x1000  /* Non-blocking mode */
#define O_CLOEXEC       0x2000  /* Close-on-exec flag */
#define O_NOCTTY        0x4000  /* Do not become controlling terminal */
#define O_DIRECTORY     0x8000  /* Must be a directory */
#define O_NOFOLLOW     0x10000  /* Do not follow symlinks */

/* Aliases */
#define O_NDELAY        O_NONBLOCK

/* ========================================================================= */
/* File Seek Whence Values                                                   */
/* ========================================================================= */

#define SEEK_SET        0   /* Seek from beginning of file */
#define SEEK_CUR        1   /* Seek from current position */
#define SEEK_END        2   /* Seek from end of file */

/* ========================================================================= */
/* fcntl() Commands                                                          */
/* ========================================================================= */

/** Duplicate file descriptor (lowest available >= arg) */
#define F_DUPFD         0

/** Get file descriptor flags (FD_CLOEXEC) */
#define F_GETFD         1

/** Set file descriptor flags */
#define F_SETFD         2

/** Get file status flags (O_APPEND, O_NONBLOCK, etc.) */
#define F_GETFL         3

/** Set file status flags */
#define F_SETFL         4

/** Duplicate fd with close-on-exec set */
#define F_DUPFD_CLOEXEC 1030

/** Get record lock */
#define F_GETLK         5
/** Set record lock (blocking) */
#define F_SETLK         6
/** Set record lock (wait) */
#define F_SETLKW        7

/* ========================================================================= */
/* File Lock Types                                                           */
/* ========================================================================= */

#define F_RDLCK         0   /* Read (shared) lock */
#define F_WRLCK         1   /* Write (exclusive) lock */
#define F_UNLCK         2   /* Unlock */

/** POSIX advisory record lock structure. */
struct flock {
    short l_type;       /* F_RDLCK, F_WRLCK, or F_UNLCK */
    short l_whence;     /* SEEK_SET, SEEK_CUR, SEEK_END */
    off_t l_start;      /* Offset where the lock begins */
    off_t l_len;        /* Size of the locked area; 0 means to EOF */
    pid_t l_pid;        /* PID of the process holding the lock */
};

/* ========================================================================= */
/* File Descriptor Flags                                                     */
/* ========================================================================= */

/** Close file descriptor on exec() */
#define FD_CLOEXEC      1

/* ========================================================================= */
/* File Operations                                                           */
/* ========================================================================= */

/** Standard file descriptors */
#define STDIN_FILENO    0
#define STDOUT_FILENO   1
#define STDERR_FILENO   2

/* ========================================================================= */
/* Function Declarations                                                     */
/* ========================================================================= */

/**
 * Open a file.
 *
 * @param pathname  Path to the file.
 * @param flags     Open flags (O_RDONLY, O_WRONLY, O_RDWR | O_CREAT, etc.).
 * @param mode      Permission bits when creating (ignored if O_CREAT not set).
 * @return File descriptor on success, -1 on error.
 */
int open(const char *pathname, int flags, ...);

/**
 * Close a file descriptor.
 *
 * @param fd    File descriptor to close.
 * @return 0 on success, -1 on error.
 */
int close(int fd);

/**
 * Perform file control operation.
 *
 * @param fd    File descriptor.
 * @param cmd   Command (F_DUPFD, F_GETFD, F_SETFD, F_GETFL, F_SETFL).
 * @param ...   Optional argument (depends on cmd).
 * @return Command-dependent value on success, -1 on error.
 */
int fcntl(int fd, int cmd, ...);

/**
 * Duplicate a file descriptor.
 *
 * @param oldfd     File descriptor to duplicate.
 * @return New file descriptor on success, -1 on error.
 */
int dup(int oldfd);

/**
 * Duplicate a file descriptor to a specified number.
 *
 * @param oldfd     File descriptor to duplicate.
 * @param newfd     Target file descriptor number.
 * @return newfd on success, -1 on error.
 */
int dup2(int oldfd, int newfd);

/**
 * Create a pipe.
 *
 * @param pipefd    Array of two ints: pipefd[0] is read end, pipefd[1] is write end.
 * @return 0 on success, -1 on error.
 */
int pipe(int pipefd[2]);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_FCNTL_H */
