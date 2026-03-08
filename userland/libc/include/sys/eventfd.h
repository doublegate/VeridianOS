/*
 * VeridianOS C Library -- <sys/eventfd.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Event file descriptor interface for cross-thread/process signaling.
 * Used by Qt 6 QEventDispatcher for wakeUp() and by various event loop
 * implementations as a lightweight signaling primitive.
 */

#ifndef _SYS_EVENTFD_H
#define _SYS_EVENTFD_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* eventfd flags                                                             */
/* ========================================================================= */

#define EFD_CLOEXEC     0x80000 /* O_CLOEXEC */
#define EFD_NONBLOCK    0x00800 /* O_NONBLOCK */
#define EFD_SEMAPHORE   0x00001 /* Semaphore-like semantics */

/* ========================================================================= */
/* Type definitions                                                          */
/* ========================================================================= */

typedef uint64_t eventfd_t;

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Create an event file descriptor.
 *
 * @param initval  Initial counter value.
 * @param flags    EFD_CLOEXEC, EFD_NONBLOCK, EFD_SEMAPHORE, or 0.
 * @return File descriptor on success, -1 on error (errno set).
 */
int eventfd(unsigned int initval, int flags);

/**
 * Read the event counter.
 *
 * Reads the 8-byte counter value.  If EFD_SEMAPHORE is set, the
 * returned value is 1 and the counter is decremented by 1; otherwise
 * the returned value is the counter and the counter is reset to 0.
 * Blocks if the counter is 0 (unless EFD_NONBLOCK is set).
 *
 * @param fd   eventfd file descriptor.
 * @param value  Pointer to receive the counter value.
 * @return 0 on success, -1 on error.
 */
int eventfd_read(int fd, eventfd_t *value);

/**
 * Write (add) to the event counter.
 *
 * Adds the given value to the counter.  Blocks if the resulting
 * counter would overflow UINT64_MAX - 1 (unless EFD_NONBLOCK).
 *
 * @param fd     eventfd file descriptor.
 * @param value  Value to add to the counter (must be < UINT64_MAX).
 * @return 0 on success, -1 on error.
 */
int eventfd_write(int fd, eventfd_t value);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_EVENTFD_H */
