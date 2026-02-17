/*
 * VeridianOS libc -- <poll.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Poll API for monitoring file descriptor readiness.
 */

#ifndef _POLL_H
#define _POLL_H

#include <veridian/syscall.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Event flags are defined in <veridian/syscall.h>:
 * POLLIN, POLLOUT, POLLERR, POLLHUP, POLLNVAL
 */

typedef unsigned long nfds_t;

struct pollfd {
    int   fd;       /* File descriptor to poll */
    short events;   /* Requested events */
    short revents;  /* Returned events */
};

/** Wait for events on file descriptors. */
int poll(struct pollfd *fds, nfds_t nfds, int timeout);

#ifdef __cplusplus
}
#endif

#endif /* _POLL_H */
