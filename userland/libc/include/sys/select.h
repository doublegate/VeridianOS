/*
 * VeridianOS libc -- <sys/select.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Synchronous I/O multiplexing via select().
 */

#ifndef _SYS_SELECT_H
#define _SYS_SELECT_H

#include <sys/types.h>
#include <signal.h>     /* sigset_t */
#include <time.h>       /* struct timeval */

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

/** Maximum number of file descriptors in an fd_set. */
#define FD_SETSIZE  1024

/** Bits per long. */
#define __NFDBITS   ((int)(sizeof(unsigned long) * 8))

/** Number of longs needed for FD_SETSIZE bits. */
#define __FD_NWORDS (FD_SETSIZE / __NFDBITS)

/** File descriptor set. */
typedef struct {
    unsigned long fds_bits[__FD_NWORDS];
} fd_set;

/* ========================================================================= */
/* Macros                                                                    */
/* ========================================================================= */

/** Clear all bits in an fd_set. */
#define FD_ZERO(set) \
    do { \
        unsigned long *__p = (unsigned long *)(set); \
        int __i; \
        for (__i = 0; __i < __FD_NWORDS; __i++) \
            __p[__i] = 0; \
    } while (0)

/** Set a bit in an fd_set. */
#define FD_SET(fd, set) \
    ((set)->fds_bits[(fd) / __NFDBITS] |= (1UL << ((fd) % __NFDBITS)))

/** Clear a bit in an fd_set. */
#define FD_CLR(fd, set) \
    ((set)->fds_bits[(fd) / __NFDBITS] &= ~(1UL << ((fd) % __NFDBITS)))

/** Test a bit in an fd_set. */
#define FD_ISSET(fd, set) \
    (((set)->fds_bits[(fd) / __NFDBITS] & (1UL << ((fd) % __NFDBITS))) != 0)

/* ========================================================================= */
/* Functions                                                                 */
/* ========================================================================= */

/**
 * Synchronous I/O multiplexing.
 *
 * Monitors file descriptors for readability, writability, or exceptions.
 * Implemented as a thin wrapper around poll() internally.
 */
int select(int nfds, fd_set *readfds, fd_set *writefds,
           fd_set *exceptfds, struct timeval *timeout);

/** Like select() but with a signal mask argument. */
int pselect(int nfds, fd_set *readfds, fd_set *writefds,
            fd_set *exceptfds, const struct timespec *timeout,
            const sigset_t *sigmask);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SELECT_H */
