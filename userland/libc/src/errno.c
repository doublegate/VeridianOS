/*
 * VeridianOS libc -- errno.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Global errno variable and accessor function.
 *
 * Currently a single global (no TLS).  When VeridianOS gains thread-local
 * storage support, __veridian_errno_location() should return a per-thread
 * pointer instead.
 */

#include <errno.h>

/* The actual storage for errno. */
static int __errno_val = 0;

/*
 * Return the address of the errno variable for the calling thread.
 * <veridian/errno.h> defines:  #define errno (*__veridian_errno_location())
 */
int *__veridian_errno_location(void)
{
    return &__errno_val;
}
