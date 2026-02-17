/*
 * VeridianOS libc -- mman.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Memory mapping functions.
 *
 * The core mmap/munmap/mprotect wrappers are in syscall.c since they are
 * direct 1:1 syscall mappings.  This file provides additional memory
 * management functions that may be needed (msync, etc.).
 */

#include <sys/mman.h>
#include <veridian/syscall.h>
#include <errno.h>

/* ========================================================================= */
/* msync                                                                     */
/* ========================================================================= */

/*
 * msync is not yet implemented in the VeridianOS kernel.
 * Provide a stub that succeeds silently (no-op) so programs that call
 * msync() don't fail at link time.
 */
int msync(void *addr, size_t length, int flags)
{
    (void)addr;
    (void)length;
    (void)flags;
    /* No-op: all VeridianOS mappings are currently coherent. */
    return 0;
}
