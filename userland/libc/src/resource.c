/*
 * VeridianOS libc -- resource.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Resource limits, usage and priorities: thin wrappers over the kernel's
 * prlimit64, getrusage, getpriority and setpriority (Linux semantics).
 */

#include <sys/resource.h>
#include <string.h>
#include <errno.h>
#include <veridian/syscall.h>
#include <veridian/sysno.h>

int prlimit(pid_t pid, int resource, const struct rlimit *new_limit,
            struct rlimit *old_limit)
{
    return (int)__syscall_ret(veridian_syscall4(SYS_prlimit64, pid, resource,
                                                new_limit, old_limit));
}

int getrlimit(int resource, struct rlimit *rlp)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_getrlimit, resource, rlp));
}

int setrlimit(int resource, const struct rlimit *rlp)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_setrlimit, resource, rlp));
}

int getrusage(int who, struct rusage *usage)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_getrusage, who, usage));
}

int getpriority(int which, id_t who)
{
    /* The system call returns 20 - nice, so that a valid result is never
     * negative; -1 can be a nice value, so callers clear errno first. */
    long r = __syscall_ret(veridian_syscall2(SYS_getpriority, which, who));
    return r < 0 ? -1 : 20 - (int)r;
}

int setpriority(int which, id_t who, int prio)
{
    return (int)__syscall_ret(veridian_syscall3(SYS_setpriority, which, who,
                                                prio));
}
