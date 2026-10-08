/*
 * VeridianOS libc -- resource.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stub implementations of resource limit and usage functions.
 * getrlimit() returns RLIM_INFINITY for all resources.
 * setrlimit() is a no-op returning 0.
 * getrusage() reports the kernel's accounting.
 */

#include <sys/resource.h>
#include <string.h>
#include <errno.h>
#include <veridian/syscall.h>
#include <veridian/sysno.h>

int getrlimit(int resource, struct rlimit *rlp)
{
    if (!rlp) {
        errno = EINVAL;
        return -1;
    }

    (void)resource;

    /* Return unlimited for all resources. */
    rlp->rlim_cur = RLIM_INFINITY;
    rlp->rlim_max = RLIM_INFINITY;
    return 0;
}

int setrlimit(int resource, const struct rlimit *rlp)
{
    (void)resource;
    (void)rlp;

    /* Accept but ignore — no enforcement. */
    return 0;
}

int getrusage(int who, struct rusage *usage)
{
    return (int)__syscall_ret(veridian_syscall2(SYS_getrusage, who, usage));
}
