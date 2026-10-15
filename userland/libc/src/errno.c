/*
 * VeridianOS libc -- errno.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * errno, per thread (N-102). Once a process has created a thread every
 * thread has a thread pointer to its control block (tcb.h), which holds
 * its errno; before that there is one thread and one global.
 */

#include <errno.h>

#include "tcb.h"

static int __errno_val = 0;
int __veridian_tp_ready = 0;

/*
 * Return the address of the errno variable for the calling thread.
 * <veridian/errno.h> defines:  #define errno (*__veridian_errno_location())
 */
int *__veridian_errno_location(void)
{
    if (__atomic_load_n(&__veridian_tp_ready, __ATOMIC_ACQUIRE)) {
        struct veridian_tcb_head *t;
#if defined(__x86_64__)
        /* A volatile load, not __builtin_thread_pointer(): GCC treats that
         * as a plain read it may hoist above the check, and without a
         * thread pointer %fs:0 is address 0. */
        __asm__ volatile("mov %%fs:0, %0" : "=r"(t));
#else
        t = __builtin_thread_pointer(); /* a register read, cannot fault */
#endif
        return &t->errno_val;
    }
    return &__errno_val;
}
