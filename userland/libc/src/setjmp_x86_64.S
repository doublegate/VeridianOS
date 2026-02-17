/*
 * VeridianOS libc -- setjmp_x86_64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * setjmp / longjmp for x86_64 (System V AMD64 ABI).
 *
 * jmp_buf layout (8 x 8-byte slots):
 *   [0]  rbx
 *   [1]  rbp
 *   [2]  r12
 *   [3]  r13
 *   [4]  r14
 *   [5]  r15
 *   [6]  rsp   (caller's stack pointer)
 *   [7]  rip   (return address)
 */

    .text
    .global setjmp
    .type   setjmp, @function

/*
 * int setjmp(jmp_buf env)
 *
 * On entry:
 *   %rdi = pointer to jmp_buf
 *   (%rsp) = return address pushed by call instruction
 *
 * Save callee-saved registers, the stack pointer (adjusted past the
 * return address), and the return address itself.  Return 0.
 */
setjmp:
    mov     %rbx,    (%rdi)        /* [0] rbx */
    mov     %rbp,   8(%rdi)        /* [1] rbp */
    mov     %r12,  16(%rdi)        /* [2] r12 */
    mov     %r13,  24(%rdi)        /* [3] r13 */
    mov     %r14,  32(%rdi)        /* [4] r14 */
    mov     %r15,  40(%rdi)        /* [5] r15 */

    /* Save the caller's stack pointer (rsp after our ret pops the
       return address).  At this point rsp points to the return addr,
       so the caller's rsp is rsp+8. */
    lea     8(%rsp), %rax
    mov     %rax,  48(%rdi)        /* [6] rsp */

    /* Save the return address. */
    mov     (%rsp), %rax
    mov     %rax,  56(%rdi)        /* [7] rip */

    xor     %eax, %eax             /* return 0 */
    ret

    .size setjmp, . - setjmp

/* ----------------------------------------------------------------------- */

    .global longjmp
    .type   longjmp, @function

/*
 * void longjmp(jmp_buf env, int val)
 *
 * On entry:
 *   %rdi = pointer to jmp_buf
 *   %esi = value to return from setjmp (forced to 1 if 0)
 *
 * Restore all saved registers and jump back to the setjmp call site,
 * making it appear that setjmp returned val.
 */
longjmp:
    /* Ensure return value is non-zero. */
    mov     %esi, %eax
    test    %eax, %eax
    jnz     1f
    mov     $1, %eax
1:
    mov      (%rdi), %rbx          /* [0] rbx */
    mov     8(%rdi), %rbp          /* [1] rbp */
    mov    16(%rdi), %r12          /* [2] r12 */
    mov    24(%rdi), %r13          /* [3] r13 */
    mov    32(%rdi), %r14          /* [4] r14 */
    mov    40(%rdi), %r15          /* [5] r15 */
    mov    48(%rdi), %rsp          /* [6] rsp */
    jmp    *56(%rdi)               /* [7] rip -- jump to saved return address */

    .size longjmp, . - longjmp
