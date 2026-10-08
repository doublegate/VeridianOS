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
#include <sys/utsname.h>
#include <poll.h>
#include <time.h>
#include <errno.h>
#include <stddef.h>

/* ========================================================================= */
/* Helper: translate raw syscall result to POSIX return value                */
/* ========================================================================= */

/* __syscall_ret: <veridian/syscall.h>. */

/* ========================================================================= */
/* File I/O                                                                  */
/* ========================================================================= */

ssize_t read(int fd, void *buf, size_t count)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_read, fd, buf, count));
}

ssize_t write(int fd, const void *buf, size_t count)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_write, fd, buf, count));
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
        veridian_syscall3(SYS_open, pathname, flags, mode));
}

int close(int fd)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_close, fd));
}

off_t lseek(int fd, off_t offset, int whence)
{
    return (off_t)__syscall_ret(
        veridian_syscall3(SYS_lseek, fd, offset, whence));
}

int dup(int oldfd)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_dup, oldfd));
}

int dup2(int oldfd, int newfd)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_dup2, oldfd, newfd));
}

int pipe(int pipefd[2])
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_pipe, pipefd));
}

int unlink(const char *pathname)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_unlink, pathname));
}

int fsync(int fd)
{
    return (int)__syscall_ret(veridian_syscall1(SYS_fsync, fd));
}

int flock(int fd, int operation)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_flock, fd, operation));
}

void sync(void)
{
    veridian_syscall0(SYS_sync);
}

int fcntl(int fd, int cmd, ...)
{
    long arg = 0;
    __builtin_va_list ap;
    __builtin_va_start(ap, cmd);
    arg = __builtin_va_arg(ap, long);
    __builtin_va_end(ap);
    return (int)__syscall_ret(
        veridian_syscall3(SYS_fcntl, fd, cmd, arg));
}

int rename(const char *oldpath, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_rename, oldpath, newpath));
}

int access(const char *pathname, int mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_access, pathname, mode));
}

ssize_t readlink(const char *pathname, char *buf, size_t bufsiz)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall3(SYS_readlink, pathname, buf, bufsiz));
}

/* ========================================================================= */
/* File status                                                               */
/* ========================================================================= */

int stat(const char *pathname, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_stat, pathname, statbuf));
}

int fstat(int fd, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_fstat, fd, statbuf));
}

int lstat(const char *pathname, struct stat *statbuf)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_lstat, pathname, statbuf));
}

/* ========================================================================= */
/* Directories                                                               */
/* ========================================================================= */

int mkdir(const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_mkdir, pathname, mode));
}

int rmdir(const char *pathname)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_rmdir, pathname));
}

/* ========================================================================= */
/* Process control                                                           */
/* ========================================================================= */

pid_t fork(void)
{
    return (pid_t)__syscall_ret(
        veridian_syscall0(SYS_fork));
}

int execve(const char *pathname, char *const argv[], char *const envp[])
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_execve, pathname, argv, envp));
}

void _exit(int status)
{
    veridian_syscall1(SYS_exit_group, status);
    __builtin_unreachable();
}

pid_t getpid(void)
{
    return (pid_t)veridian_syscall0(SYS_getpid);
}

pid_t getppid(void)
{
    return (pid_t)veridian_syscall0(SYS_getppid);
}

pid_t gettid(void)
{
    return (pid_t)veridian_syscall0(SYS_gettid);
}

/*
 * Low-level thread creation (kernel thread_clone).
 * Shared-address-space threads must set CLONE_VM|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD.
 */
long veridian_thread_clone(unsigned long flags,
                           void *newsp,
                           int *parent_tidptr,
                           int *child_tidptr,
                           void *tls)
{
    return __syscall_ret(veridian_syscall5(SYS_clone,
                                           flags,
                                           (long)newsp,
                                           (long)parent_tidptr,
                                           (long)child_tidptr,
                                           (long)tls));
}

int arch_prctl(int code, unsigned long addr)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_arch_prctl, code, addr));
}

pid_t waitpid(pid_t pid, int *wstatus, int options)
{
    /* wait4 reads its fourth argument: no rusage wanted. */
    return (pid_t)__syscall_ret(
        veridian_syscall4(SYS_wait4, pid, wstatus, options, 0));
}

pid_t wait(int *wstatus)
{
    return waitpid(-1, wstatus, 0);
}

int sched_yield(void)
{
    return (int)__syscall_ret(
        veridian_syscall0(SYS_sched_yield));
}

int kill(pid_t pid, int sig)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_kill, pid, sig));
}

/* ========================================================================= */
/* Working directory                                                         */
/* ========================================================================= */

char *getcwd(char *buf, size_t size)
{
    long ret = veridian_syscall2(SYS_getcwd, buf, size);
    if (ret < 0) {
        errno = (int)(-ret);
        return NULL;
    }
    return buf;
}

int chdir(const char *path)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_chdir, path));
}

/* ========================================================================= */
/* Memory management                                                         */
/* ========================================================================= */

/*
 * brk/sbrk are implemented here because they directly map to SYS_brk.
 * The malloc implementation in stdlib.c uses sbrk() from here.
 *
 * DESIGN NOTES (B-9 hardening for cc1-scale workloads):
 *
 * 1. __brk_cur tracks the *kernel-visible* program break (what the kernel
 *    last confirmed via sys_brk).
 * 2. __brk_committed tracks the highest address the kernel has page-mapped.
 *    When sbrk() is called with a small increment, we round up to a full
 *    page boundary and ask the kernel for the rounded-up address. This means
 *    subsequent small sbrk() calls that fit within the already-committed
 *    region can be satisfied without any syscall at all.
 * 3. brk() uses Linux-style failure detection: the kernel always returns the
 *    current break, so failure is detected by comparing the return value to
 *    the requested address rather than checking for a negative value.
 */

#define PAGE_SIZE_LIBC  4096
#define BRK_CHUNK_SIZE  (64 * 1024)   /* 64KB pre-allocation granularity */

static void *__brk_cur       = NULL;  /* Logical break: next byte to hand out */
static void *__brk_committed = NULL;  /* Kernel-committed break (page-aligned) */

/*
 * Internal: ensure the kernel break is at least `target`.
 * Returns 0 on success, -1 on failure (sets errno).
 */
static int __brk_ensure(void *target)
{
    if (__brk_committed && target <= __brk_committed)
        return 0;  /* Already committed. */

    /* Round up to page boundary. */
    unsigned long t = (unsigned long)target;
    unsigned long aligned = (t + PAGE_SIZE_LIBC - 1) & ~(PAGE_SIZE_LIBC - 1);

    /* Commit at least BRK_CHUNK_SIZE beyond the current committed point
     * to amortize syscalls.  This means many small sbrk() calls can be
     * satisfied from the already-committed region without any syscall. */
    unsigned long cur = (unsigned long)(__brk_committed ? __brk_committed : __brk_cur);
    unsigned long chunked = (cur + BRK_CHUNK_SIZE + PAGE_SIZE_LIBC - 1) & ~(PAGE_SIZE_LIBC - 1);
    if (chunked > aligned)
        aligned = chunked;

    long ret = veridian_syscall1(SYS_brk, (long)aligned);
    if (ret < 0 || (unsigned long)ret < t) {
        /* Kernel could not satisfy even the minimum request.
         * Try the exact target without chunk rounding. */
        unsigned long exact = (t + PAGE_SIZE_LIBC - 1) & ~(PAGE_SIZE_LIBC - 1);
        ret = veridian_syscall1(SYS_brk, (long)exact);
        if (ret < 0 || (unsigned long)ret < t) {
            errno = ENOMEM;
            return -1;
        }
    }
    __brk_committed = (void *)(unsigned long)ret;
    return 0;
}

/*
 * Internal: query the current program break from the kernel and initialize
 * tracking state. Called once lazily on first brk/sbrk use.
 */
static int __brk_init(void)
{
    if (__brk_cur)
        return 0;

    long cur = veridian_syscall1(SYS_brk, 0);
    if (cur <= 0) {
        errno = ENOMEM;
        return -1;
    }
    __brk_cur = (void *)cur;
    __brk_committed = (void *)cur;
    return 0;
}

int brk(void *addr)
{
    if (__brk_init() < 0)
        return -1;

    if (__brk_ensure(addr) < 0)
        return -1;

    __brk_cur = addr;
    return 0;
}

void *sbrk(intptr_t increment)
{
    if (__brk_init() < 0)
        return (void *)-1;

    if (increment == 0)
        return __brk_cur;

    void *old = __brk_cur;
    void *target = (char *)__brk_cur + increment;

    /* Negative increment (shrink): just move the logical pointer. */
    if (increment < 0) {
        __brk_cur = target;
        return old;
    }

    /* Positive increment (grow): ensure kernel has committed enough pages. */
    if (__brk_ensure(target) < 0)
        return (void *)-1;

    __brk_cur = target;
    return old;
}

void *mmap(void *addr, size_t length, int prot, int flags,
           int fd, off_t offset)
{
    /* Linux layout: fd in the fifth argument, offset in the sixth. */
    long ret = veridian_syscall6(SYS_mmap, addr, length, prot, flags, fd,
                                 offset);
    if (ret < 0) {
        errno = (int)(-ret);
        return MAP_FAILED;
    }
    return (void *)ret;
}

int munmap(void *addr, size_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_munmap, addr, length));
}

int mprotect(void *addr, size_t length, int prot)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_mprotect, addr, length, prot));
}

/* ========================================================================= */
/* User / group identity                                                     */
/* ========================================================================= */

uid_t getuid(void)
{
    return (uid_t)veridian_syscall0(SYS_getuid);
}

uid_t geteuid(void)
{
    return (uid_t)veridian_syscall0(SYS_geteuid);
}

gid_t getgid(void)
{
    return (gid_t)veridian_syscall0(SYS_getgid);
}

gid_t getegid(void)
{
    return (gid_t)veridian_syscall0(SYS_getegid);
}

int setuid(uid_t uid)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_setuid, uid));
}

int setgid(gid_t gid)
{
    return (int)__syscall_ret(
        veridian_syscall1(SYS_setgid, gid));
}

/* ========================================================================= */
/* Process groups and sessions                                               */
/* ========================================================================= */

int setpgid(pid_t pid, pid_t pgid)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_setpgid, pid, pgid));
}

pid_t getpgid(pid_t pid)
{
    return (pid_t)__syscall_ret(
        veridian_syscall1(SYS_getpgid, pid));
}

pid_t getpgrp(void)
{
    return (pid_t)veridian_syscall0(SYS_getpgrp);
}

pid_t setsid(void)
{
    return (pid_t)__syscall_ret(
        veridian_syscall0(SYS_setsid));
}

pid_t getsid(pid_t pid)
{
    return (pid_t)__syscall_ret(
        veridian_syscall1(SYS_getsid, pid));
}

/* ========================================================================= */
/* File I/O control                                                          */
/* ========================================================================= */

int ioctl(int fd, unsigned long request, void *argp)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_ioctl, fd, request, argp));
}

/* ========================================================================= */
/* Filesystem: link, symlink, chmod, umask, truncate, poll, pread/pwrite     */
/* ========================================================================= */

int link(const char *oldpath, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_link, oldpath, newpath));
}

int symlink(const char *target, const char *linkpath)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_symlink, target, linkpath));
}

int chmod(const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_chmod, pathname, mode));
}

int fchmod(int fd, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_fchmod, fd, mode));
}

mode_t umask(mode_t mask)
{
    return (mode_t)veridian_syscall1(SYS_umask, mask);
}

int truncate(const char *path, off_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_truncate, path, length));
}

int ftruncate(int fd, off_t length)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_ftruncate, fd, length));
}

int memfd_create(const char *name, unsigned int flags)
{
    return (int)__syscall_ret(
        veridian_syscall2(SYS_memfd_create, name, flags));
}

ssize_t pread(int fd, void *buf, size_t count, off_t offset)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall4(SYS_pread64, fd, buf, count, offset));
}

ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset)
{
    return (ssize_t)__syscall_ret(
        veridian_syscall4(SYS_pwrite64, fd, buf, count, offset));
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
        veridian_syscall4(SYS_openat, dirfd, pathname, flags, mode));
}

int fstatat(int dirfd, const char *pathname, struct stat *statbuf, int flags)
{
    return (int)__syscall_ret(
        veridian_syscall4(SYS_newfstatat, dirfd, pathname, statbuf, flags));
}

int unlinkat(int dirfd, const char *pathname, int flags)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_unlinkat, dirfd, pathname, flags));
}

int mkdirat(int dirfd, const char *pathname, mode_t mode)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_mkdirat, dirfd, pathname, mode));
}

int renameat(int olddirfd, const char *oldpath,
             int newdirfd, const char *newpath)
{
    return (int)__syscall_ret(
        veridian_syscall4(SYS_renameat, olddirfd, oldpath,
                          newdirfd, newpath));
}

/* ========================================================================= */
/* poll                                                                      */
/* ========================================================================= */

int poll(struct pollfd *fds, nfds_t nfds, int timeout)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_poll, fds, nfds, timeout));
}

/* ========================================================================= */
/* Futex primitives (kernel-provided, pthread uses)                          */
/* ========================================================================= */

int futex_wait(int *uaddr, int expected, const struct timespec *timeout)
{
    uint64_t ticks = 0;
    if (timeout) {
        ticks = (uint64_t)timeout->tv_sec * 1000ULL +
                (uint64_t)(timeout->tv_nsec / 1000000ULL);
    }
    return (int)__syscall_ret(
        veridian_syscall5(SYS_FUTEX_WAIT,
                          uaddr,
                          expected,
                          timeout ? &ticks : 0,
                          timeout ? sizeof(uint64_t) : 0,
                          0));
}

int futex_wake(int *uaddr, int count)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_FUTEX_WAKE, uaddr, count, 0));
}

int futex_requeue(int *uaddr, int wake_count, int *uaddr2, int requeue_count)
{
    return (int)__syscall_ret(
        veridian_syscall5(SYS_FUTEX_WAIT,
                          uaddr,
                          wake_count,
                          uaddr2,
                          requeue_count,
                          FUTEX_REQUEUE));
}

/* ========================================================================= */
/* Ownership                                                                 */
/* ========================================================================= */

int chown(const char *pathname, uid_t owner, gid_t group)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_chown, pathname, owner, group));
}

int fchown(int fd, uid_t owner, gid_t group)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_fchown, fd, owner, group));
}

int lchown(const char *pathname, uid_t owner, gid_t group)
{
    /* Same as chown — symlink following not implemented */
    return (int)__syscall_ret(
        veridian_syscall3(SYS_chown, pathname, owner, group));
}

/* ========================================================================= */
/* Device nodes                                                              */
/* ========================================================================= */

int mknod(const char *pathname, mode_t mode, dev_t dev)
{
    return (int)__syscall_ret(
        veridian_syscall3(SYS_mknod, pathname, mode, dev));
}

/* ========================================================================= */
/* System information                                                        */
/* ========================================================================= */

int uname(struct utsname *buf)
{
    if (!buf) {
        errno = EFAULT;
        return -1;
    }
    return (int)__syscall_ret(
        veridian_syscall1(SYS_uname, buf));
}
