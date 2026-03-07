/*
 * VeridianOS C Library -- <sys/epoll.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Event polling interface (Linux-compatible epoll API).
 * Kernel syscalls: EpollCreate=262, EpollCtl=263, EpollWait=264.
 */

#ifndef _SYS_EPOLL_H
#define _SYS_EPOLL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* epoll event flags                                                         */
/* ========================================================================= */

#define EPOLLIN         0x001
#define EPOLLPRI        0x002
#define EPOLLOUT        0x004
#define EPOLLERR        0x008
#define EPOLLHUP        0x010
#define EPOLLRDNORM     0x040
#define EPOLLRDBAND     0x080
#define EPOLLWRNORM     0x100
#define EPOLLWRBAND     0x200
#define EPOLLMSG        0x400
#define EPOLLRDHUP      0x2000
#define EPOLLEXCLUSIVE  (1U << 28)
#define EPOLLWAKEUP     (1U << 29)
#define EPOLLONESHOT    (1U << 30)
#define EPOLLET         (1U << 31)

/* ========================================================================= */
/* epoll_ctl operations                                                      */
/* ========================================================================= */

#define EPOLL_CTL_ADD   1
#define EPOLL_CTL_DEL   2
#define EPOLL_CTL_MOD   3

/* ========================================================================= */
/* epoll_create1 flags                                                       */
/* ========================================================================= */

#define EPOLL_CLOEXEC   0x80000 /* O_CLOEXEC */

/* ========================================================================= */
/* Data structures                                                           */
/* ========================================================================= */

typedef union epoll_data {
    void     *ptr;
    int       fd;
    uint32_t  u32;
    uint64_t  u64;
} epoll_data_t;

struct epoll_event {
    uint32_t     events;
    epoll_data_t data;
} __attribute__((packed));

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Create an epoll instance.
 * @param size  Ignored (kept for API compatibility); must be > 0.
 * @return epoll file descriptor, or -1 on error.
 */
int epoll_create(int size);

/**
 * Create an epoll instance with flags.
 * @param flags  0 or EPOLL_CLOEXEC.
 * @return epoll file descriptor, or -1 on error.
 */
int epoll_create1(int flags);

/**
 * Control an epoll instance.
 * @param epfd   epoll file descriptor.
 * @param op     EPOLL_CTL_ADD, EPOLL_CTL_DEL, or EPOLL_CTL_MOD.
 * @param fd     Target file descriptor.
 * @param event  Event configuration (ignored for EPOLL_CTL_DEL).
 * @return 0 on success, -1 on error.
 */
int epoll_ctl(int epfd, int op, int fd, struct epoll_event *event);

/**
 * Wait for events on an epoll instance.
 * @param epfd       epoll file descriptor.
 * @param events     Output array for ready events.
 * @param maxevents  Maximum number of events to return.
 * @param timeout    Timeout in milliseconds (-1 = block, 0 = poll).
 * @return Number of ready file descriptors, or -1 on error.
 */
int epoll_wait(int epfd, struct epoll_event *events,
               int maxevents, int timeout);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_EPOLL_H */
