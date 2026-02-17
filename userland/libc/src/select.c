/*
 * VeridianOS libc -- select.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * select() implementation built on top of the kernel's poll() syscall.
 */

#include <sys/select.h>
#include <poll.h>
#include <errno.h>
#include <string.h>

int select(int nfds, fd_set *readfds, fd_set *writefds,
           fd_set *exceptfds, struct timeval *timeout)
{
    if (nfds < 0 || nfds > FD_SETSIZE) {
        errno = EINVAL;
        return -1;
    }

    /* Convert timeout to milliseconds for poll(). */
    int timeout_ms = -1;  /* Infinite by default */
    if (timeout) {
        timeout_ms = (int)(timeout->tv_sec * 1000 +
                           timeout->tv_usec / 1000);
        if (timeout_ms < 0)
            timeout_ms = 0;
    }

    /* Count how many fds are set across all three sets. */
    int nset = 0;
    for (int fd = 0; fd < nfds; fd++) {
        int want = 0;
        if (readfds  && FD_ISSET(fd, readfds))  want = 1;
        if (writefds && FD_ISSET(fd, writefds)) want = 1;
        if (exceptfds && FD_ISSET(fd, exceptfds)) want = 1;
        if (want) nset++;
    }

    if (nset == 0) {
        /* No fds to watch — just sleep for timeout. */
        if (timeout_ms > 0) {
            /* Use poll with 0 fds for the sleep. */
            poll(NULL, 0, timeout_ms);
        }
        return 0;
    }

    /* Build pollfd array on the stack (nfds <= FD_SETSIZE = 1024). */
    struct pollfd pfds[FD_SETSIZE];
    int pfd_count = 0;

    for (int fd = 0; fd < nfds; fd++) {
        short events = 0;
        if (readfds  && FD_ISSET(fd, readfds))   events |= POLLIN;
        if (writefds && FD_ISSET(fd, writefds))  events |= POLLOUT;
        if (exceptfds && FD_ISSET(fd, exceptfds)) events |= POLLERR;

        if (events) {
            pfds[pfd_count].fd = fd;
            pfds[pfd_count].events = events;
            pfds[pfd_count].revents = 0;
            pfd_count++;
        }
    }

    int ret = poll(pfds, (unsigned long)pfd_count, timeout_ms);
    if (ret < 0)
        return -1;

    /* Clear all fd_sets and re-populate based on poll results. */
    if (readfds)  FD_ZERO(readfds);
    if (writefds) FD_ZERO(writefds);
    if (exceptfds) FD_ZERO(exceptfds);

    int ready = 0;
    for (int i = 0; i < pfd_count; i++) {
        int fd = pfds[i].fd;
        int got = 0;

        if (readfds && (pfds[i].revents & (POLLIN | POLLHUP | POLLERR))) {
            FD_SET(fd, readfds);
            got = 1;
        }
        if (writefds && (pfds[i].revents & POLLOUT)) {
            FD_SET(fd, writefds);
            got = 1;
        }
        if (exceptfds && (pfds[i].revents & (POLLERR | POLLNVAL))) {
            FD_SET(fd, exceptfds);
            got = 1;
        }
        if (got) ready++;
    }

    return ready;
}

int pselect(int nfds, fd_set *readfds, fd_set *writefds,
            fd_set *exceptfds, const struct timespec *timeout,
            const sigset_t *sigmask)
{
    (void)sigmask;  /* Signal mask handling not implemented yet */

    struct timeval tv;
    struct timeval *tvp = NULL;

    if (timeout) {
        tv.tv_sec  = timeout->tv_sec;
        tv.tv_usec = timeout->tv_nsec / 1000;
        tvp = &tv;
    }

    return select(nfds, readfds, writefds, exceptfds, tvp);
}
