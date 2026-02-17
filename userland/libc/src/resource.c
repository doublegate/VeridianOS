/*
 * VeridianOS libc -- resource.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Stub implementations of resource limit and usage functions.
 * getrlimit() returns RLIM_INFINITY for all resources.
 * setrlimit() is a no-op returning 0.
 * getrusage() zeroes the output structure.
 */

#include <sys/resource.h>
#include <string.h>
#include <errno.h>

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
    if (!usage) {
        errno = EINVAL;
        return -1;
    }

    (void)who;

    /* Zero everything — no real accounting yet. */
    memset(usage, 0, sizeof(*usage));
    return 0;
}
