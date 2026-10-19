/*
 * VeridianOS libc -- clone_riscv64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * long __veridian_clone(unsigned long flags, void *stack, int *ptid,
 *                       int *ctid, void *tls, int (*fn)(void *), void *arg)
 *
 * See clone_x86_64.S: the new thread calls fn(arg) on its own stack and
 * exits the thread with the result, never returning through C (N-102).
 * a0-a4 are the system call's arguments in the kernel's order (flags,
 * stack, ptid, ctid, tls); a5 fn, a6 arg. (RISC-V has no user mode yet,
 * sprint E; this is untested.)
 */

#include <veridian/sysno.h>

    .text
    .global __veridian_clone
    .type   __veridian_clone, @function
__veridian_clone:
    andi    a1, a1, -16
    addi    a1, a1, -16
    sd      a5, 0(a1)               /* fn, for the child */
    sd      a6, 8(a1)               /* arg */
    li      a7, SYS_clone
    ecall
    bnez    a0, 1f

    /* Child: sp is the new stack. */
    ld      a5, 0(sp)
    ld      a0, 8(sp)
    addi    sp, sp, 16
    mv      ra, zero
    mv      s0, zero
    jalr    a5
    li      a7, SYS_exit            /* this thread only */
    ecall
    j       .
1:
    ret
    .size   __veridian_clone, . - __veridian_clone

    .section .note.GNU-stack, "", @progbits
