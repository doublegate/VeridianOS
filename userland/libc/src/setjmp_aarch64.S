/*
 * VeridianOS libc -- setjmp_aarch64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * setjmp / longjmp for AArch64 (AAPCS64).
 *
 * jmp_buf layout (14 x 8-byte slots):
 *   [0]  x19
 *   [1]  x20
 *   [2]  x21
 *   [3]  x22
 *   [4]  x23
 *   [5]  x24
 *   [6]  x25
 *   [7]  x26
 *   [8]  x27
 *   [9]  x28
 *   [10] x29 (fp)
 *   [11] x30 (lr / return address)
 *   [12] sp
 *   [13] reserved (alignment / future NEON state)
 */

    .text
    .global setjmp
    .type   setjmp, @function

/*
 * int setjmp(jmp_buf env)
 *
 * On entry:
 *   x0 = pointer to jmp_buf
 *
 * Save callee-saved registers x19-x28, fp (x29), lr (x30), and sp.
 * Return 0.
 */
setjmp:
    stp     x19, x20, [x0, #0]
    stp     x21, x22, [x0, #16]
    stp     x23, x24, [x0, #32]
    stp     x25, x26, [x0, #48]
    stp     x27, x28, [x0, #64]
    stp     x29, x30, [x0, #80]
    mov     x1, sp
    str     x1, [x0, #96]

    mov     w0, #0
    ret

    .size setjmp, . - setjmp

/* ----------------------------------------------------------------------- */

    .global longjmp
    .type   longjmp, @function

/*
 * void longjmp(jmp_buf env, int val)
 *
 * On entry:
 *   x0 = pointer to jmp_buf
 *   w1 = value to return from setjmp (forced to 1 if 0)
 *
 * Restore all saved registers and return to the setjmp call site.
 */
longjmp:
    ldp     x19, x20, [x0, #0]
    ldp     x21, x22, [x0, #16]
    ldp     x23, x24, [x0, #32]
    ldp     x25, x26, [x0, #48]
    ldp     x27, x28, [x0, #64]
    ldp     x29, x30, [x0, #80]
    ldr     x2, [x0, #96]
    mov     sp, x2

    /* Ensure return value is non-zero. */
    cmp     w1, #0
    csinc   w0, w1, wzr, ne
    ret

    .size longjmp, . - longjmp
