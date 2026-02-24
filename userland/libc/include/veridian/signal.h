/*
 * VeridianOS Signal Definitions
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Signal numbers and structures matching kernel/src/process/exit.rs signals module.
 * POSIX-compatible signal numbering.
 */

#ifndef VERIDIAN_SIGNAL_H
#define VERIDIAN_SIGNAL_H

#include <veridian/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

/** Atomic type safe for use in signal handlers. */
typedef volatile int sig_atomic_t;

/* ========================================================================= */
/* Signal Numbers (matching kernel/src/process/exit.rs signals::*)           */
/* ========================================================================= */

#define SIGHUP      1       /* Hangup */
#define SIGINT      2       /* Interrupt (Ctrl-C) */
#define SIGQUIT     3       /* Quit (Ctrl-\) */
#define SIGILL      4       /* Illegal instruction */
#define SIGTRAP     5       /* Trace/breakpoint trap */
#define SIGABRT     6       /* Abort */
#define SIGBUS      7       /* Bus error */
#define SIGFPE      8       /* Floating-point exception */
#define SIGKILL     9       /* Kill (cannot be caught or ignored) */
#define SIGUSR1     10      /* User-defined signal 1 */
#define SIGSEGV     11      /* Segmentation violation */
#define SIGUSR2     12      /* User-defined signal 2 */
#define SIGPIPE     13      /* Broken pipe */
#define SIGALRM     14      /* Alarm clock */
#define SIGTERM     15      /* Termination */
#define SIGSTKFLT   16      /* Stack fault */
#define SIGCHLD     17      /* Child status changed */
#define SIGCONT     18      /* Continue (if stopped) */
#define SIGSTOP     19      /* Stop (cannot be caught or ignored) */
#define SIGTSTP     20      /* Terminal stop (Ctrl-Z) */
#define SIGTTIN     21      /* Background read from tty */
#define SIGTTOU     22      /* Background write to tty */
#define SIGURG      23      /* Urgent data on socket */
#define SIGXCPU     24      /* CPU time limit exceeded */
#define SIGXFSZ     25      /* File size limit exceeded */
#define SIGVTALRM   26      /* Virtual timer expired */
#define SIGPROF     27      /* Profiling timer expired */
#define SIGWINCH    28      /* Window size changed */
#define SIGIO       29      /* I/O possible */
#define SIGPWR      30      /* Power failure */
#define SIGSYS      31      /* Bad system call */

/** Number of signals (1-based, signal 0 is reserved for error checking) */
#define _NSIG       32
#define NSIG        _NSIG

/* ========================================================================= */
/* Signal Handler Types                                                      */
/* ========================================================================= */

/** Signal handler function pointer */
typedef void (*sighandler_t)(int);

/** Default signal action */
#define SIG_DFL     ((sighandler_t)0)

/** Ignore signal */
#define SIG_IGN     ((sighandler_t)1)

/** Error return from signal() */
#define SIG_ERR     ((sighandler_t)-1)

/* ========================================================================= */
/* Signal Action Flags                                                       */
/* ========================================================================= */

/** Restart interrupted syscalls automatically */
#define SA_RESTART      0x10000000

/** Do not generate SIGCHLD when child stops */
#define SA_NOCLDSTOP    0x00000001

/** Do not create zombie on child death */
#define SA_NOCLDWAIT    0x00000002

/** Use sa_sigaction handler (3-arg form) */
#define SA_SIGINFO      0x00000004

/** Use alternate signal stack */
#define SA_ONSTACK      0x08000000

/** Reset handler to SIG_DFL on entry */
#define SA_RESETHAND    0x80000000

/** Do not add signal to mask during handler */
#define SA_NODEFER      0x40000000

/* ========================================================================= */
/* Signal Set Type                                                           */
/* ========================================================================= */

/** Signal set (bitmask, one bit per signal) */
typedef uint64_t sigset_t;

/** Initialize empty signal set */
static inline int sigemptyset(sigset_t *set)
{
    if (!set) return -1;
    *set = 0;
    return 0;
}

/** Initialize full signal set (all signals blocked) */
static inline int sigfillset(sigset_t *set)
{
    if (!set) return -1;
    *set = ~(uint64_t)0;
    return 0;
}

/** Add signal to set */
static inline int sigaddset(sigset_t *set, int signum)
{
    if (!set || signum < 1 || signum >= _NSIG) return -1;
    *set |= (uint64_t)1 << signum;
    return 0;
}

/** Remove signal from set */
static inline int sigdelset(sigset_t *set, int signum)
{
    if (!set || signum < 1 || signum >= _NSIG) return -1;
    *set &= ~((uint64_t)1 << signum);
    return 0;
}

/** Test if signal is in set */
static inline int sigismember(const sigset_t *set, int signum)
{
    if (!set || signum < 1 || signum >= _NSIG) return -1;
    return (*set & ((uint64_t)1 << signum)) ? 1 : 0;
}

/* ========================================================================= */
/* siginfo_t                                                                 */
/* ========================================================================= */

/** Signal information structure (passed when SA_SIGINFO is set) */
typedef struct {
    int         si_signo;   /* Signal number */
    int         si_errno;   /* Errno value associated with signal */
    int         si_code;    /* Signal code (SI_USER, SI_KERNEL, etc.) */
    pid_t       si_pid;     /* Sending process PID */
    uid_t       si_uid;     /* Sending process real UID */
    int         si_status;  /* Exit value or signal */
    void       *si_addr;    /* Faulting instruction/memory address */
} siginfo_t;

/** si_code values */
#define SI_USER     0       /* Sent by kill(), raise(), or abort() */
#define SI_KERNEL   128     /* Sent by the kernel */

/* ========================================================================= */
/* struct sigaction                                                          */
/* ========================================================================= */

/** Signal action structure */
struct sigaction {
    union {
        /** Simple signal handler (when SA_SIGINFO is not set) */
        sighandler_t    sa_handler;
        /** Extended signal handler (when SA_SIGINFO is set) */
        void (*sa_sigaction)(int, siginfo_t *, void *);
    };
    /** Signals to block during handler execution */
    sigset_t            sa_mask;
    /** Signal action flags (SA_RESTART, SA_SIGINFO, etc.) */
    int                 sa_flags;
};

/* ========================================================================= */
/* Signal Function Declarations                                              */
/* ========================================================================= */

/**
 * Set signal disposition (simplified POSIX interface).
 *
 * @param signum    Signal number (SIGHUP .. SIGSYS).
 * @param handler   New handler (SIG_DFL, SIG_IGN, or function pointer).
 * @return Previous handler, or SIG_ERR on failure.
 */
sighandler_t signal(int signum, sighandler_t handler);

/**
 * Examine or change signal action (full POSIX interface).
 *
 * @param signum    Signal number.
 * @param act       New action (NULL to query only).
 * @param oldact    Previous action (NULL to discard).
 * @return 0 on success, -1 on error (errno set).
 */
int sigaction(int signum, const struct sigaction *act,
              struct sigaction *oldact);

/**
 * Send signal to a process.
 *
 * @param pid       Target process ID.
 * @param sig       Signal number (0 to test existence without sending).
 * @return 0 on success, -1 on error.
 */
int kill(pid_t pid, int sig);

/**
 * Send signal to the calling process.
 *
 * @param sig       Signal number.
 * @return 0 on success, -1 on error.
 */
int raise(int sig);

/**
 * Change the signal mask of the calling thread.
 *
 * @param how       SIG_BLOCK, SIG_UNBLOCK, or SIG_SETMASK.
 * @param set       New signal set (NULL to query only).
 * @param oldset    Previous signal set (NULL to discard).
 * @return 0 on success, -1 on error.
 */
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);

/** sigprocmask 'how' values */
#define SIG_BLOCK       0   /* Add signals to current mask */
#define SIG_UNBLOCK     1   /* Remove signals from current mask */
#define SIG_SETMASK     2   /* Replace current mask entirely */

/**
 * Examine pending signals.
 *
 * @param set   Receives the set of pending signals.
 * @return 0 on success, -1 on error.
 */
int sigpending(sigset_t *set);

/**
 * Wait for a signal.
 *
 * @param set   Set of signals to wait for.
 * @return -1 with errno EINTR when a signal is delivered.
 */
int sigsuspend(const sigset_t *mask);

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_SIGNAL_H */
