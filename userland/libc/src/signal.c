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

/* The kernel's sigaction layout (Linux x86_64, 32 bytes). The public
 * struct sigaction is 24 bytes, so passing it directly let the kernel
 * write 8 bytes past the caller's `oldact` (N-95). */
struct k_sigaction {
    void *handler;
    unsigned long flags;
    void *restorer;
    unsigned long mask;
};

/* The kernel returns from a handler to `restorer`, which issues
 * rt_sigreturn (native 123); as on Linux x86_64 it is required
 * (SA_RESTORER). */
#define SA_RESTORER 0x04000000UL
void __veridian_restore_rt(void);
__asm__(".text\n"
        ".global __veridian_restore_rt\n"
        ".type __veridian_restore_rt,@function\n"
        "__veridian_restore_rt:\n"
        "    movl $123, %eax\n"
        "    syscall\n"
        "    hlt\n");

int sigaction(int signum, const struct sigaction *act,
              struct sigaction *oldact)
{
    struct k_sigaction kact, kold;
    if (act) {
        kact.handler = (void *)act->sa_handler;
        kact.flags = (unsigned long)(unsigned int)act->sa_flags | SA_RESTORER;
        kact.restorer = (void *)__veridian_restore_rt;
        kact.mask = (unsigned long)act->sa_mask;
    }
    long ret = veridian_syscall3(SYS_SIGACTION, signum, act ? &kact : 0,
                                 oldact ? &kold : 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    if (oldact) {
        memset(oldact, 0, sizeof(*oldact));
        oldact->sa_handler = (sighandler_t)kold.handler;
        oldact->sa_flags = (int)(kold.flags & ~SA_RESTORER);
        oldact->sa_mask = (sigset_t)kold.mask;
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
