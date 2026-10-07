/*
 * VeridianOS libc -- vfork_x86_64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * pid_t vfork(void)
 *
 * The child runs on the parent's stack, in the parent's memory, until it
 * execs or exits, while the parent waits (N-210). A C wrapper would return
 * in the child and so overwrite its own frame, return address included,
 * before the parent returns through it; the return address is therefore
 * held in a register across the system call, as in musl and glibc.
 */

#include <veridian/sysno.h>

    .text
    .global vfork
    .type   vfork, @function
vfork:
    pop     %rdx                    /* return address, out of the stack */
    mov     $SYS_vfork, %eax
    syscall
    push    %rdx
    cmp     $-4095, %rax            /* -4095..-1: -errno */
    jae     1f
    ret
1:
    neg     %eax
    push    %rax                    /* errno; also realigns the stack */
    call    __veridian_errno_location
    pop     %rcx
    mov     %ecx, (%rax)
    mov     $-1, %rax
    ret
    .size   vfork, .-vfork

    .section .note.GNU-stack, "", @progbits
