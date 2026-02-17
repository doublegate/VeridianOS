/*
 * VeridianOS libc -- setjmp_riscv64.S
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * setjmp / longjmp for RISC-V 64-bit (LP64D ABI).
 *
 * jmp_buf layout (14 x 8-byte slots):
 *   [0]  ra   (return address)
 *   [1]  sp
 *   [2]  s0   (fp)
 *   [3]  s1
 *   [4]  s2
 *   [5]  s3
 *   [6]  s4
 *   [7]  s5
 *   [8]  s6
 *   [9]  s7
 *   [10] s8
 *   [11] s9
 *   [12] s10
 *   [13] s11
 */

    .text
    .global setjmp
    .type   setjmp, @function

/*
 * int setjmp(jmp_buf env)
 *
 * On entry:
 *   a0 = pointer to jmp_buf
 *
 * Save callee-saved registers ra, sp, s0-s11.
 * Return 0.
 */
setjmp:
    sd      ra,    0(a0)
    sd      sp,    8(a0)
    sd      s0,   16(a0)
    sd      s1,   24(a0)
    sd      s2,   32(a0)
    sd      s3,   40(a0)
    sd      s4,   48(a0)
    sd      s5,   56(a0)
    sd      s6,   64(a0)
    sd      s7,   72(a0)
    sd      s8,   80(a0)
    sd      s9,   88(a0)
    sd      s10,  96(a0)
    sd      s11, 104(a0)

    li      a0, 0
    ret

    .size setjmp, . - setjmp

/* ----------------------------------------------------------------------- */

    .global longjmp
    .type   longjmp, @function

/*
 * void longjmp(jmp_buf env, int val)
 *
 * On entry:
 *   a0 = pointer to jmp_buf
 *   a1 = value to return from setjmp (forced to 1 if 0)
 *
 * Restore all saved registers and return to the setjmp call site.
 */
longjmp:
    ld      ra,    0(a0)
    ld      sp,    8(a0)
    ld      s0,   16(a0)
    ld      s1,   24(a0)
    ld      s2,   32(a0)
    ld      s3,   40(a0)
    ld      s4,   48(a0)
    ld      s5,   56(a0)
    ld      s6,   64(a0)
    ld      s7,   72(a0)
    ld      s8,   80(a0)
    ld      s9,   88(a0)
    ld      s10,  96(a0)
    ld      s11, 104(a0)

    /* Ensure return value is non-zero. */
    seqz    a0, a1
    add     a0, a0, a1
    ret

    .size longjmp, . - longjmp
