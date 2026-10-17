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
/* Linux's struct sigaction as the kernel reads it. The handler and the
 * restorer are function pointers; ISO C has no conversion from those to
 * void * (-Wpedantic). */
struct k_sigaction {
    sighandler_t handler;
    unsigned long flags;
    void (*restorer)(void);
    unsigned long mask;
};

/* The kernel returns from a handler to `restorer`, which issues
 * rt_sigreturn; as on Linux x86_64 it is required (SA_RESTORER). */
#define SA_RESTORER 0x04000000UL
#define __VERIDIAN_XSTR(x) #x
#define __VERIDIAN_STR(x) __VERIDIAN_XSTR(x)
void __veridian_restore_rt(void);
__asm__(".text\n"
        ".global __veridian_restore_rt\n"
        ".type __veridian_restore_rt,@function\n"
        "__veridian_restore_rt:\n"
        "    movl $" __VERIDIAN_STR(SYS_rt_sigreturn) ", %eax\n"
        "    syscall\n"
        "    hlt\n");

int sigaction(int signum, const struct sigaction *act,
              struct sigaction *oldact)
{
    struct k_sigaction kact, kold;
    if (act) {
        kact.handler = act->sa_handler;
        kact.flags = (unsigned long)(unsigned int)act->sa_flags | SA_RESTORER;
        kact.restorer = __veridian_restore_rt;
        kact.mask = (unsigned long)act->sa_mask;
    }
    long ret = veridian_syscall3(SYS_rt_sigaction, signum, act ? &kact : 0,
                                 oldact ? &kold : 0);
    if (ret < 0) {
        errno = (int)(-ret);
        return -1;
    }
    if (oldact) {
        memset(oldact, 0, sizeof(*oldact));
        oldact->sa_handler = kold.handler;
        oldact->sa_flags = (int)(kold.flags & ~SA_RESTORER);
        oldact->sa_mask = (sigset_t)kold.mask;
    }
    return 0;
}

/* ========================================================================= */
/* sigaltstack                                                               */
/* ========================================================================= */

_Static_assert(sizeof(stack_t) == 24, "stack_t must match the kernel's layout");

int sigaltstack(const stack_t *ss, stack_t *old_ss)
{
    long ret = veridian_syscall2(SYS_sigaltstack, ss, old_ss);
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
    long ret = veridian_syscall3(SYS_rt_sigprocmask, how, set, oldset);
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
 * kill() is implemented in syscall.c via SYS_kill.
 * Declared here to avoid a duplicate definition.
 */

/* ========================================================================= */
/* raise                                                                     */
/* ========================================================================= */

int raise(int sig)
{
    return kill(getpid(), sig);
}
