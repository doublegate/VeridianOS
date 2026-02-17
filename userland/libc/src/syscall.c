/*
 * VeridianOS libc -- syscall.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Thin wrappers around the raw syscall interface defined in
 * <veridian/syscall.h>.  Each wrapper invokes the appropriate
 * veridian_syscallN() macro and translates negative return values
 * into errno + return -1 (POSIX convention).
 *
 * Functions declared in <unistd.h>, <fcntl.h>, <sys/stat.h>,
 * <sys/wait.h>, and <sys/mman.h> that directly map to a single syscall
 * are implemented here.
 */

#include <veridian/syscall.h>
#include <veridian/types.h>
#include <veridian/stat.h>
#include <veridian/fcntl.h>
#include <veridian/mman.h>
#include <errno.h>
#include <stddef.h>

/* ========================================================================= */
/* Helper: translate raw syscall result to POSIX return value                */
/* ========================================================================= */

/*
 * Most VeridianOS syscalls return >= 0 on success and a negative errno
 * value on failure.  __syscall_ret() translates that into the POSIX
 * convention: success value unchanged, failure returns -1 with errno set.
 */
static inline long __syscall_ret(long r)
{
    if (r < 0) {
        errno = (int)(-r);
        return -1;
    }
    return r;
}

/* ========================================================================= */
/* File I/O                                                                  */
/* ========================================================================= */

ssize_t read(int fd, void *buf, size_t count)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_FILE_READ, fd, buf, count));
}

ssize_t write(int fd, const void *buf, size_t count)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_FILE_WRITE, fd, buf, count));
}

int open(const char *pathname, int flags, ...)
{
    /* Mode argument is only meaningful with O_CREAT. */
    mode_t mode = 0;
    if (flags & O_CREAT) {
        __builtin_va_list ap;
        __builtin_va_start(ap, flags);
        mode = __builtin_va_arg(ap, mode_t);
        __builtin_va_end(ap);
    }
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_OPEN, pathname, flags, mode));
}

int close(int fd)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_FILE_CLOSE, fd));
}

off_t lseek(int fd, off_t offset, int whence)
{
    return (off_t)__syscall_ret(
        veridian_syscall3(SYS_FILE_SEEK, fd, offset, whence));
}

int dup(int oldfd)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_FILE_DUP, oldfd));
}

int dup2(int oldfd, int newfd)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_DUP2, oldfd, newfd));
}

int pipe(int pipefd[2])
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_FILE_PIPE, pipefd));
}

int unlink(const char *pathname)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_FILE_UNLINK, pathname));
}

int fcntl(int fd, int cmd, ...)
{
    long arg = 0;
    __builtin_va_list ap;
    __builtin_va_start(ap, cmd);
    arg = __builtin_va_arg(ap, long);
    __builtin_va_end(ap);
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_FCNTL, fd, cmd, arg));
}

int rename(const char *oldpath, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_RENAME, oldpath, newpath));
}

int access(const char *pathname, int mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_ACCESS, pathname, mode));
}

ssize_t readlink(const char *pathname, char *buf, size_t bufsiz)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_FILE_READLINK, pathname, buf, bufsiz));
}

/* ========================================================================= */
/* File status                                                               */
/* ========================================================================= */

int stat(const char *pathname, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_STAT_PATH, pathname, statbuf));
}

int fstat(int fd, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_STAT, fd, statbuf));
}

int lstat(const char *pathname, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_LSTAT, pathname, statbuf));
}

/* ========================================================================= */
/* Directories                                                               */
/* ========================================================================= */

int mkdir(const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_DIR_MKDIR, pathname, mode));
}

int rmdir(const char *pathname)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_DIR_RMDIR, pathname));
}

/* ========================================================================= */
/* Process control                                                           */
/* ========================================================================= */

pid_t fork(void)
{
    return (pid_t)__syscall_ret(
        veridian_syscall0(SYS_PROCESS_FORK));
}

int execve(const char *pathname, char *const argv[], char *const envp[])
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_PROCESS_EXEC, pathname, argv, envp));
}

void _exit(int status)
{
    veridian_syscall1(SYS_PROCESS_EXIT, status);
    __builtin_unreachable();
}

pid_t getpid(void)
{
    return (pid_t)veridian_syscall0(SYS_PROCESS_GETPID);
}

pid_t getppid(void)
{
    return (pid_t)veridian_syscall0(SYS_PROCESS_GETPPID);
}

pid_t waitpid(pid_t pid, int *wstatus, int options)
{
    return (pid_t)__syscall_ret(
        veridian_syscall3(SYS_PROCESS_WAIT, pid, wstatus, options));
}

pid_t wait(int *wstatus)
{
    return waitpid(-1, wstatus, 0);
}

int sched_yield(void)
{
    return (int)__syscall_ret(
        veridian_syscall0(SYS_PROCESS_YIELD));
}

int kill(pid_t pid, int sig)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_PROCESS_KILL, pid, sig));
}

/* ========================================================================= */
/* Working directory                                                         */
/* ========================================================================= */

char *getcwd(char *buf, size_t size)
{
    long ret = veridian_syscall2(SYS_PROCESS_GETCWD, buf, size);
    if (ret < 0) {
        errno = (int)(-ret);
        return NULL;
    }
    return buf;
}

int chdir(const char *path)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_PROCESS_CHDIR, path));
}

/* ========================================================================= */
/* Memory management                                                         */
/* ========================================================================= */

/*
 * brk/sbrk are implemented here because they directly map to SYS_MEMORY_BRK.
 * The malloc implementation in stdlib.c uses sbrk() from here.
 */

static void *__brk_cur = NULL;

int brk(void *addr)
{
    long ret = veridian_syscall1(SYS_MEMORY_BRK, addr);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    __brk_cur = (void *)ret;
    return 0;
}

void *sbrk(intptr_t increment)
{
    if (__brk_cur == NULL) {
        /* Query current break. */
        long cur = veridian_syscall1(SYS_MEMORY_BRK, 0);
        if (cur < 0) {
            errno = ENOMEM;
            return (void *)-1;
        }
        __brk_cur = (void *)cur;
    }

    if (increment == 0)
        return __brk_cur;

    void *old = __brk_cur;
    long new_brk = veridian_syscall1(SYS_MEMORY_BRK,
                                      (long)__brk_cur + increment);
    if (new_brk < 0) {
        errno = ENOMEM;
        return (void *)-1;
    }
    __brk_cur = (void *)new_brk;
    return old;
}

void *mmap(void *addr, size_t length, int prot, int flags,
           int fd, off_t offset)
{
    long ret = veridian_syscall6(SYS_MEMORY_MAP, addr, length,
                                  prot, flags, fd, offset);
    if (ret < 0) {
        errno = (int)(-ret);
        return MAP_FAILED;
    }
    return (void *)ret;
}

int munmap(void *addr, size_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_MEMORY_UNMAP, addr, length));
}

int mprotect(void *addr, size_t length, int prot)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_MEMORY_PROTECT, addr, length, prot));
}

/* ========================================================================= */
/* User / group identity                                                     */
/* ========================================================================= */

uid_t getuid(void)
{
    return (uid_t)veridian_syscall0(SYS_GETUID);
}

uid_t geteuid(void)
{
    return (uid_t)veridian_syscall0(SYS_GETEUID);
}

gid_t getgid(void)
{
    return (gid_t)veridian_syscall0(SYS_GETGID);
}

gid_t getegid(void)
{
    return (gid_t)veridian_syscall0(SYS_GETEGID);
}

int setuid(uid_t uid)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_SETUID, uid));
}

int setgid(gid_t gid)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_SETGID, gid));
}

/* ========================================================================= */
/* Process groups and sessions                                               */
/* ========================================================================= */

int setpgid(pid_t pid, pid_t pgid)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_SETPGID, pid, pgid));
}

pid_t getpgid(pid_t pid)
{
    return (pid_t)__syscall_ret(
        veridian_syscall1(SYS_GETPGID, pid));
}

pid_t getpgrp(void)
{
    return (pid_t)veridian_syscall0(SYS_GETPGRP);
}

pid_t setsid(void)
{
    return (pid_t)__syscall_ret(
        veridian_syscall0(SYS_SETSID));
}

pid_t getsid(pid_t pid)
{
    return (pid_t)__syscall_ret(
        veridian_syscall1(SYS_GETSID, pid));
}

/* ========================================================================= */
/* File I/O control                                                          */
/* ========================================================================= */

int ioctl(int fd, unsigned long request, void *argp)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_IOCTL, fd, request, argp));
}

/* ========================================================================= */
/* Filesystem: link, symlink, chmod, umask, truncate, poll, pread/pwrite     */
/* ========================================================================= */

int link(const char *oldpath, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_LINK, oldpath, newpath));
}

int symlink(const char *target, const char *linkpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_SYMLINK, target, linkpath));
}

int chmod(const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_CHMOD, pathname, mode));
}

int fchmod(int fd, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_FCHMOD, fd, mode));
}

mode_t umask(mode_t mask)
{
    return (mode_t)veridian_syscall1(SYS_PROCESS_UMASK, mask);
}

int truncate(const char *path, off_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_TRUNCATE_PATH, path, length));
}

int ftruncate(int fd, off_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_FILE_TRUNCATE, fd, length));
}

ssize_t pread(int fd, void *buf, size_t count, off_t offset)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall4(SYS_FILE_PREAD, fd, buf, count, offset));
}

ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall4(SYS_FILE_PWRITE, fd, buf, count, offset));
}

/* ========================================================================= */
/* *at() family: openat, fstatat, unlinkat, mkdirat, renameat               */
/* ========================================================================= */

int openat(int dirfd, const char *pathname, int flags, ...)
{
    mode_t mode = 0;
    if (flags & O_CREAT) {
        __builtin_va_list ap;
        __builtin_va_start(ap, flags);
        mode = __builtin_va_arg(ap, mode_t);
        __builtin_va_end(ap);
    }
    return (int)__syscall_ret(
        veridian_syscall4(SYS_FILE_OPENAT, dirfd, pathname, flags, mode));
}

int fstatat(int dirfd, const char *pathname, struct stat *statbuf, int flags)
{
    return (int)__syscall_ret(
        veridian_syscall4(SYS_FILE_FSTATAT, dirfd, pathname, statbuf, flags));
}

int unlinkat(int dirfd, const char *pathname, int flags)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_UNLINKAT, dirfd, pathname, flags));
}

int mkdirat(int dirfd, const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_MKDIRAT, dirfd, pathname, mode));
}

int renameat(int olddirfd, const char *oldpath,
             int newdirfd, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall4(SYS_FILE_RENAMEAT, olddirfd, oldpath,
                          newdirfd, newpath));
}

/* ========================================================================= */
/* poll                                                                      */
/* ========================================================================= */

int poll(struct pollfd *fds, unsigned long nfds, int timeout)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FILE_POLL, fds, nfds, timeout));
}
