/*
 * VeridianOS C Library -- <sys/timerfd.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Timer file descriptor interface.  Delivers timer expirations via
 * readable file descriptors, integrating with epoll/poll/select.
 * Used by Qt 6 QTimer for precise event-loop-integrated timing.
 */

#ifndef _SYS_TIMERFD_H
#define _SYS_TIMERFD_H

#include <time.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* timerfd flags                                                             */
/* ========================================================================= */

#define TFD_CLOEXEC     0x80000 /* O_CLOEXEC */
#define TFD_NONBLOCK    0x00800 /* O_NONBLOCK */

/* ========================================================================= */
/* timerfd_settime flags                                                     */
/* ========================================================================= */

#define TFD_TIMER_ABSTIME       (1 << 0)
#define TFD_TIMER_CANCEL_ON_SET (1 << 1)

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Create a timer file descriptor.
 *
 * @param clockid  Clock to use: CLOCK_REALTIME or CLOCK_MONOTONIC.
 * @param flags    TFD_CLOEXEC, TFD_NONBLOCK, or 0.
 * @return File descriptor on success, -1 on error (errno set).
 */
int timerfd_create(int clockid, int flags);

/**
 * Arm (start) or disarm (stop) the timer.
 *
 * @param fd         timerfd file descriptor.
 * @param flags      0 or TFD_TIMER_ABSTIME.
 * @param new_value  New timer setting (it_value = initial expiration,
 *                   it_interval = period; both zero = disarm).
 * @param old_value  If non-NULL, receives the previous timer setting.
 * @return 0 on success, -1 on error (errno set).
 */
int timerfd_settime(int fd, int flags,
                    const struct itimerspec *new_value,
                    struct itimerspec *old_value);

/**
 * Get the current timer setting.
 *
 * @param fd         timerfd file descriptor.
 * @param curr_value Receives the current timer setting.
 * @return 0 on success, -1 on error (errno set).
 */
int timerfd_gettime(int fd, struct itimerspec *curr_value);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_TIMERFD_H */
