/*
 * VeridianOS libc -- <unistd.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX symbolic constants and function declarations for process control,
 * file I/O, and miscellaneous system interfaces.
 */

#ifndef _UNISTD_H
#define _UNISTD_H

#include <sys/types.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* POSIX version                                                             */
/* ========================================================================= */

/* Claim basic POSIX.1-2001 compliance so that portable software recognizes  */
/* our standard function signatures (e.g. getcwd) and skips legacy fallbacks */
#define _POSIX_VERSION  200112L

/* ========================================================================= */
/* Standard file descriptors                                                 */
/* ========================================================================= */

#ifndef STDIN_FILENO
#define STDIN_FILENO    0
#define STDOUT_FILENO   1
#define STDERR_FILENO   2
#endif

/* ========================================================================= */
/* Seek whence values                                                        */
/* ========================================================================= */

#ifndef SEEK_SET
#define SEEK_SET        0
#define SEEK_CUR        1
#define SEEK_END        2
#endif

/* ========================================================================= */
/* File I/O                                                                  */
/* ========================================================================= */

/** Read up to count bytes from fd into buf. */
ssize_t read(int fd, void *buf, size_t count);

/** Write up to count bytes from buf to fd. */
ssize_t write(int fd, const void *buf, size_t count);

/** Reposition read/write offset of fd. */
off_t lseek(int fd, off_t offset, int whence);

/** Close a file descriptor. */
int close(int fd);

/** Duplicate a file descriptor. */
int dup(int oldfd);

/** Duplicate a file descriptor to a specific number. */
int dup2(int oldfd, int newfd);

/** Create a pipe. */
int pipe(int pipefd[2]);

/** Delete a name from the filesystem. */
int unlink(const char *pathname);

/** Delete a name / decrement link count. */
int rmdir(const char *pathname);

/** Check file accessibility. */
int access(const char *pathname, int mode);

/* access() mode flags */
#define F_OK    0   /* Test for existence */
#define R_OK    4   /* Test for read permission */
#define W_OK    2   /* Test for write permission */
#define X_OK    1   /* Test for execute permission */

/** Read the target of a symbolic link. */
ssize_t readlink(const char *pathname, char *buf, size_t bufsiz);

/** Create a hard link. */
int link(const char *oldpath, const char *newpath);

/** Create a symbolic link. */
int symlink(const char *target, const char *linkpath);

/** Truncate a file to a specified length (by path). */
int truncate(const char *path, off_t length);

/** Truncate a file to a specified length (by fd). */
int ftruncate(int fd, off_t length);

/** Read from fd at offset without changing file position. */
ssize_t pread(int fd, void *buf, size_t count, off_t offset);

/** Write to fd at offset without changing file position. */
ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset);

/** Resolve a pathname to an absolute path. */
char *realpath(const char *path, char *resolved_path);

/* ========================================================================= */
/* Process control                                                           */
/* ========================================================================= */

/** Create a child process (copy of the caller). */
pid_t fork(void);

/** Replace the current process image. */
int execve(const char *pathname, char *const argv[], char *const envp[]);

/** Execute a file with argument vector. */
int execv(const char *pathname, char *const argv[]);

/** Convenience: search PATH for the file. */
int execvp(const char *file, char *const argv[]);

/** Execute with argument list (variadic). */
int execl(const char *pathname, const char *arg, ...);

/** Execute with argument list, search PATH. */
int execlp(const char *file, const char *arg, ...);

/** Execute with argument list and environment. */
int execle(const char *pathname, const char *arg, ...);

/** Terminate the calling process immediately. */
void _exit(int status) __attribute__((noreturn));

/** Return the process ID of the calling process. */
pid_t getpid(void);

/** Return the parent process ID. */
pid_t getppid(void);

/** Return the thread ID of the calling thread. */
pid_t gettid(void);

/** Yield the processor. */
int sched_yield(void);

/* ========================================================================= */
/* Working directory                                                         */
/* ========================================================================= */

/** Get the current working directory. */
char *getcwd(char *buf, size_t size);

/** Change the working directory. */
int chdir(const char *path);

/* ========================================================================= */
/* User / group identity                                                     */
/* ========================================================================= */

uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);
int setuid(uid_t uid);
int setgid(gid_t gid);

/** Get login name of the user. */
char *getlogin(void);

/** Get the hostname. */
int gethostname(char *name, size_t len);

/* ========================================================================= */
/* Process groups and sessions                                               */
/* ========================================================================= */

int setpgid(pid_t pid, pid_t pgid);
pid_t getpgid(pid_t pid);
pid_t getpgrp(void);
pid_t setsid(void);
pid_t getsid(pid_t pid);

/* ========================================================================= */
/* Memory management                                                         */
/* ========================================================================= */

/** Set the program break (end of the data segment). */
int brk(void *addr);

/** Increment the program break by increment bytes. */
void *sbrk(intptr_t increment);

/* ========================================================================= */
/* Sleep                                                                     */
/* ========================================================================= */

/** Sleep for the specified number of seconds. */
unsigned int sleep(unsigned int seconds);

/** Sleep for the specified number of microseconds. */
int usleep(unsigned int usec);

/** Set an alarm timer (deliver SIGALRM after seconds). */
unsigned int alarm(unsigned int seconds);

/* ========================================================================= */
/* sysconf                                                                   */
/* ========================================================================= */

/** sysconf name values */
#define _SC_ARG_MAX             0
#define _SC_CLK_TCK             2
#define _SC_NPROCESSORS_CONF    83
#define _SC_NPROCESSORS_ONLN    84
#define _SC_PAGESIZE            30
#define _SC_PAGE_SIZE           _SC_PAGESIZE
#define _SC_OPEN_MAX            4

/** Get configurable system variables. */
long sysconf(int name);

/* ========================================================================= */
/* pathconf                                                                  */
/* ========================================================================= */

/** pathconf name values */
#define _PC_LINK_MAX            0
#define _PC_MAX_CANON           1
#define _PC_MAX_INPUT           2
#define _PC_NAME_MAX            3
#define _PC_PATH_MAX            4
#define _PC_PIPE_BUF            5
#define _PC_CHOWN_RESTRICTED    6
#define _PC_NO_TRUNC            7
#define _PC_VDISABLE            8
#define _PC_SYNC_IO             9
#define _PC_ASYNC_IO            10
#define _PC_PRIO_IO             11
#define _PC_FILESIZEBITS        12

/** Get configurable pathname variables. */
long pathconf(const char *path, int name);

/** Get configurable pathname variables (by fd). */
long fpathconf(int fd, int name);

/* ========================================================================= */
/* Miscellaneous                                                             */
/* ========================================================================= */

/** Check if fd refers to a terminal. */
int isatty(int fd);

/** Change ownership of a file. */
int chown(const char *pathname, uid_t owner, gid_t group);

/** Change ownership of a file (by fd). */
int fchown(int fd, uid_t owner, gid_t group);

/** Change ownership of a symlink (no follow). */
int lchown(const char *pathname, uid_t owner, gid_t group);

/* ========================================================================= */
/* getopt (POSIX requires these in unistd.h)                                 */
/* ========================================================================= */

#include <getopt.h>

#ifdef __cplusplus
}
#endif

#endif /* _UNISTD_H */
