/*
 * VeridianOS libc -- <setjmp.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Non-local jumps.
 */

#ifndef _SETJMP_H
#define _SETJMP_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * jmp_buf layout for x86_64 (8 64-bit slots):
 *   [0] rbx
 *   [1] rbp
 *   [2] r12
 *   [3] r13
 *   [4] r14
 *   [5] r15
 *   [6] rsp  (stack pointer after setjmp returns)
 *   [7] rip  (return address)
 */
typedef long jmp_buf[8];

/**
 * Save the calling environment for later use by longjmp.
 *
 * @param env  Buffer to save the environment into.
 * @return 0 when called directly; non-zero when returning via longjmp.
 */
int setjmp(jmp_buf env);

/**
 * Restore the environment saved by setjmp.
 *
 * @param env  Buffer previously filled by setjmp.
 * @param val  Value that setjmp will appear to return (1 if val is 0).
 */
void longjmp(jmp_buf env, int val) __attribute__((noreturn));

#ifdef __cplusplus
}
#endif

#endif /* _SETJMP_H */
