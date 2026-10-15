/*
 * VeridianOS libc -- tcb.h (internal)
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * The head every thread control block starts with, shared by pthread.c
 * and errno.c. The thread pointer (FS base on x86_64, TPIDR_EL0, tp)
 * points at it, and `self` must be its first word: on x86_64
 * __builtin_thread_pointer() reads %fs:0, the TLS ABI's self pointer.
 */

#ifndef VERIDIAN_LIBC_TCB_H
#define VERIDIAN_LIBC_TCB_H

struct veridian_tcb_head {
    struct veridian_tcb_head *self;
    int errno_val; /* this thread's errno */
};

/* Set once the calling process has a thread pointer (the first
 * pthread_create); until then errno is one global. */
extern int __veridian_tp_ready;

#endif
