/*
 * VeridianOS libc -- epoll.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Userland wrappers for the epoll system calls.
 * Kernel syscalls: EpollCreate=262, EpollCtl=263, EpollWait=264.
 */

#include <sys/epoll.h>
#include <errno.h>
#include <veridian/syscall.h>

/* Epoll syscall numbers (must match kernel/src/syscall/mod.rs) */
#define SYS_EPOLL_CREATE  262
#define SYS_EPOLL_CTL     263
#define SYS_EPOLL_WAIT    264

/*
 * Translate raw syscall return to POSIX convention.
 * Negative values become errno + return -1.
 */
static inline int __epoll_ret(long r)
{
    if (r < 0) {
        errno = (int)(-r);
        return -1;
    }
    return (int)r;
}

int epoll_create(int size)
{
    if (size <= 0) {
        errno = EINVAL;
        return -1;
    }
    long ret = veridian_syscall1(SYS_EPOLL_CREATE, 0);
    return __epoll_ret(ret);
}

int epoll_create1(int flags)
{
    long ret = veridian_syscall1(SYS_EPOLL_CREATE, flags);
    return __epoll_ret(ret);
}

int epoll_ctl(int epfd, int op, int fd, struct epoll_event *event)
{
    long ret = veridian_syscall4(SYS_EPOLL_CTL, epfd, op, fd, event);
    return __epoll_ret(ret);
}

int epoll_wait(int epfd, struct epoll_event *events,
               int maxevents, int timeout)
{
    if (maxevents <= 0) {
        errno = EINVAL;
        return -1;
    }
    long ret = veridian_syscall4(SYS_EPOLL_WAIT, epfd, events,
                                  maxevents, timeout);
    return __epoll_ret(ret);
}
