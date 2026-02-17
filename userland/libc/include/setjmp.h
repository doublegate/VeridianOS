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
 * jmp_buf layout is architecture-specific:
 *
 * x86_64 (8 slots): rbx, rbp, r12-r15, rsp, rip
 * AArch64 (14 slots): x19-x28, x29(fp), x30(lr), sp, reserved
 * RISC-V (14 slots): ra, sp, s0-s11
 */
#if defined(__x86_64__) || defined(_M_X64)
typedef long jmp_buf[8];
#elif defined(__aarch64__)
typedef long jmp_buf[14];
#elif defined(__riscv) && __riscv_xlen == 64
typedef long jmp_buf[14];
#else
#error "Unsupported architecture for setjmp"
#endif

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
