/*
 * VeridianOS libc -- qt_core_platform.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * OS-level shims for QtCore subsystems that need platform hooks beyond
 * standard POSIX.  Provides eventfd, timerfd, inotify, madvise, and
 * getauxval implementations.
 *
 * Already implemented elsewhere in the libc:
 *   - epoll_create/ctl/wait  (epoll.c)
 *   - clock_gettime          (time.c)
 *   - posix_memalign         (stdlib.c)
 *   - prctl                  (posix_stubs3.c)
 */

#include <stddef.h>
#include <stdint.h>
#include <errno.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/timerfd.h>
#include <sys/inotify.h>
#include <sys/mman.h>

/* Forward declarations for syscall wrappers */
extern long __veridian_syscall(long number, long arg1, long arg2,
                               long arg3, long arg4, long arg5, long arg6);
extern int open(const char *pathname, int flags, ...);
extern int close(int fd);
extern long read(int fd, void *buf, unsigned long count);
extern long write(int fd, const void *buf, unsigned long count);

/* ========================================================================= */
/* Syscall numbers for new VeridianOS syscalls                               */
/* ========================================================================= */

/* Syscall numbers -- MUST match kernel/src/syscall/mod.rs enum values */
#define SYS_EVENTFD         331
#define SYS_EVENTFD_READ    332
#define SYS_EVENTFD_WRITE   333
#define SYS_TIMERFD_CREATE  334
#define SYS_TIMERFD_SETTIME 335
#define SYS_TIMERFD_GETTIME 336
#define SYS_SIGNALFD        337
#define SYS_GETRANDOM       330
#define SYS_INOTIFY_INIT1   290
#define SYS_INOTIFY_ADD_WATCH 291
#define SYS_INOTIFY_RM_WATCH 292
#define SYS_MADVISE         233
#define SYS_GETAUXVAL       270
#define SYS_MEMFD_CREATE    319

/* ========================================================================= */
/* eventfd                                                                   */
/* ========================================================================= */

int eventfd(unsigned int initval, int flags)
{
    long ret = __veridian_syscall(SYS_EVENTFD, (long)initval, (long)flags,
                                  0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (int)ret;
}

int eventfd_read(int fd, eventfd_t *value)
{
    long ret = read(fd, value, sizeof(*value));
    if (ret != (long)sizeof(*value)) {
        if (ret >= 0)
            errno = EINVAL;
        return -1;
    }
    return 0;
}

int eventfd_write(int fd, eventfd_t value)
{
    long ret = write(fd, &value, sizeof(value));
    if (ret != (long)sizeof(value)) {
        if (ret >= 0)
            errno = EINVAL;
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* timerfd                                                                   */
/* ========================================================================= */

int timerfd_create(int clockid, int flags)
{
    long ret = __veridian_syscall(SYS_TIMERFD_CREATE, (long)clockid,
                                  (long)flags, 0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (int)ret;
}

int timerfd_settime(int fd, int flags,
                    const struct itimerspec *new_value,
                    struct itimerspec *old_value)
{
    long ret = __veridian_syscall(SYS_TIMERFD_SETTIME, (long)fd, (long)flags,
                                  (long)new_value, (long)old_value, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

int timerfd_gettime(int fd, struct itimerspec *curr_value)
{
    long ret = __veridian_syscall(SYS_TIMERFD_GETTIME, (long)fd,
                                  (long)curr_value, 0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* inotify                                                                   */
/* ========================================================================= */

int inotify_init(void)
{
    return inotify_init1(0);
}

int inotify_init1(int flags)
{
    long ret = __veridian_syscall(SYS_INOTIFY_INIT1, (long)flags,
                                  0, 0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (int)ret;
}

int inotify_add_watch(int fd, const char *pathname, uint32_t mask)
{
    long ret = __veridian_syscall(SYS_INOTIFY_ADD_WATCH, (long)fd,
                                  (long)pathname, (long)mask, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (int)ret;
}

int inotify_rm_watch(int fd, int wd)
{
    long ret = __veridian_syscall(SYS_INOTIFY_RM_WATCH, (long)fd, (long)wd,
                                  0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* madvise                                                                   */
/* ========================================================================= */

int madvise(void *addr, size_t length, int advice)
{
    long ret = __veridian_syscall(SYS_MADVISE, (long)addr, (long)length,
                                  (long)advice, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* getauxval                                                                 */
/* ========================================================================= */

/*
 * Auxiliary vector types used by QtCore and other programs.
 */
#define AT_NULL         0   /* End of vector */
#define AT_RANDOM       25  /* Address of 16 random bytes */
#define AT_PAGESZ       6   /* System page size */
#define AT_CLKTCK       17  /* Clock ticks per second */
#define AT_HWCAP        16  /* Hardware capabilities */
#define AT_HWCAP2       26  /* Extended hardware capabilities */
#define AT_SECURE       23  /* Secure mode (setuid) */
#define AT_EXECFN       31  /* Filename of program */

/*
 * Static random bytes for AT_RANDOM.  In a full implementation the
 * dynamic linker (ld-veridian) would pass the auxv from the kernel.
 * For now, provide deterministic but non-zero bytes.  The kernel's
 * CSPRNG seeds these at process start.
 */
static uint8_t __at_random_bytes[16] = {
    0x4f, 0x89, 0xc3, 0x7a, 0x15, 0xde, 0x62, 0xb8,
    0x91, 0x3d, 0xa7, 0x05, 0xf4, 0x28, 0x6c, 0xe0
};

unsigned long getauxval(unsigned long type)
{
    switch (type) {
    case AT_RANDOM:
        return (unsigned long)__at_random_bytes;
    case AT_PAGESZ:
        return 4096;
    case AT_CLKTCK:
        return 100;  /* HZ = 100 */
    case AT_HWCAP:
        return 0;
    case AT_HWCAP2:
        return 0;
    case AT_SECURE:
        return 0;
    default:
        errno = ENOENT;
        return 0;
    }
}

/* ========================================================================= */
/* memfd_create                                                              */
/* ========================================================================= */

#define MFD_CLOEXEC     0x0001U
#define MFD_ALLOW_SEALING 0x0002U

int memfd_create(const char *name, unsigned int flags)
{
    long ret = __veridian_syscall(SYS_MEMFD_CREATE, (long)name, (long)flags,
                                  0, 0, 0, 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return (int)ret;
}
