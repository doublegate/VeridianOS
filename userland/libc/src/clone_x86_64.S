/*
 * VeridianOS libc -- clone_x86_64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * long __veridian_clone(unsigned long flags, void *stack, int *ptid,
 *                       int *ctid, void *tls, int (*fn)(void *), void *arg)
 *
 * The new thread starts on `stack` with nothing of the caller's frame, so
 * it must not return through C (N-102: the C wrapper's epilogue popped a
 * return address that was never there). It calls fn(arg) and exits the
 * thread with its result. The parent gets the new tid, or -errno.
 *
 * On entry: rdi flags, rsi stack, rdx ptid, rcx ctid, r8 tls, r9 fn,
 * 8(%rsp) arg. The system call takes rdi, rsi, rdx, r10, r8.
 */

#include <veridian/sysno.h>

    .text
    .global __veridian_clone
    .type   __veridian_clone, @function
__veridian_clone:
    mov     8(%rsp), %rax       /* arg */
    and     $-16, %rsi          /* 16-byte aligned top */
    sub     $16, %rsi
    mov     %r9, 0(%rsi)        /* fn, popped by the child */
    mov     %rax, 8(%rsi)       /* arg */
    mov     %rcx, %r10
    mov     $SYS_clone, %eax
    syscall
    test    %rax, %rax
    jnz     1f

    /* Child: rsp is the new stack. */
    xor     %ebp, %ebp          /* end of the frame chain */
    pop     %rax                /* fn */
    pop     %rdi                /* arg; rsp is 16-byte aligned again */
    call    *%rax
    mov     %eax, %edi
    mov     $SYS_exit, %eax     /* this thread only */
    syscall
    hlt                         /* not reached */
1:
    ret
    .size   __veridian_clone, . - __veridian_clone

    .section .note.GNU-stack, "", @progbits
