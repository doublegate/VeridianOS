/*
 * VeridianOS libc -- signal.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Signal handling wrappers.
 */

#include <signal.h>
#include <unistd.h>
#include <veridian/syscall.h>
#include <errno.h>
#include <string.h>

/* ========================================================================= */
/* sigaction                                                                 */
/* ========================================================================= */

int sigaction(int signum, const struct sigaction *act,
              struct sigaction *oldact)
{
    long ret = veridian_syscall3(SYS_SIGACTION, signum, act, oldact);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* sigprocmask                                                               */
/* ========================================================================= */

int sigprocmask(int how, const sigset_t *set, sigset_t *oldset)
{
    long ret = veridian_syscall3(SYS_SIGPROCMASK, how, set, oldset);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    return 0;
}

/* ========================================================================= */
/* signal (simplified POSIX interface)                                       */
/* ========================================================================= */

sighandler_t signal(int signum, sighandler_t handler)
{
    struct sigaction sa;
    struct sigaction old;

    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = handler;
    sa.sa_flags = SA_RESTART;
    sigemptyset(&sa.sa_mask);

    if (sigaction(signum, &sa, &old) < 0)
        return SIG_ERR;

    return old.sa_handler;
}

/* ========================================================================= */
/* kill                                                                      */
/* ========================================================================= */

/*
 * kill() is implemented in syscall.c via SYS_PROCESS_KILL.
 * Declared here to avoid a duplicate definition.
 */

/* ========================================================================= */
/* raise                                                                     */
/* ========================================================================= */

int raise(int sig)
{
    return kill(getpid(), sig);
}
