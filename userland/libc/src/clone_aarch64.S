/*
 * VeridianOS libc -- clone_aarch64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * long __veridian_clone(unsigned long flags, void *stack, int *ptid,
 *                       int *ctid, void *tls, int (*fn)(void *), void *arg)
 *
 * See clone_x86_64.S: the new thread calls fn(arg) on its own stack and
 * exits the thread with the result, never returning through C (N-102).
 * x0-x4 are the system call's arguments in the kernel's order (flags,
 * stack, ptid, ctid, tls); x5 fn, x6 arg. (AArch64 has no user mode yet,
 * sprint E; this is untested.)
 */

#include <veridian/sysno.h>

    .text
    .global __veridian_clone
    .type   __veridian_clone, %function
__veridian_clone:
    and     x1, x1, #-16
    stp     x5, x6, [x1, #-16]!     /* fn, arg for the child */
    mov     x8, #SYS_clone
    svc     #0
    cbnz    x0, 1f

    /* Child: sp is the new stack. */
    ldp     x5, x0, [sp], #16
    mov     x29, #0
    mov     x30, #0
    blr     x5
    mov     x8, #SYS_exit           /* this thread only */
    svc     #0
    b       .
1:
    ret
    .size   __veridian_clone, . - __veridian_clone

    .section .note.GNU-stack, "", %progbits
