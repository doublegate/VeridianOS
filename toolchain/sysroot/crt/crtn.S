/*
 * VeridianOS C Runtime Initialization Epilogue -- All Architectures
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * This file provides the .init and .fini section epilogues.
 * The linker concatenates:
 *   crti.o (.init prologue)  +  crtbegin.o (.init body)  +  crtn.o (.init epilogue)
 *   crti.o (.fini prologue)  +  crtend.o  (.fini body)   +  crtn.o (.fini epilogue)
 *
 * This file closes the _init() and _fini() functions opened by crti.S.
 */

#if defined(__x86_64__)

/*
 * x86_64: Function epilogue -- pop frame and return.
 */

.section .init
    popq    %rbp
    ret

.section .fini
    popq    %rbp
    ret

#elif defined(__aarch64__)

/*
 * AArch64: Restore frame pointer, link register, and return.
 */

.section .init
    ldp     x29, x30, [sp], #16
    ret

.section .fini
    ldp     x29, x30, [sp], #16
    ret

#elif defined(__riscv) && __riscv_xlen == 64

/*
 * RISC-V 64: Restore return address, frame pointer, and return.
 */

.section .init
    ld      ra, 8(sp)
    ld      s0, 0(sp)
    addi    sp, sp, 16
    ret

.section .fini
    ld      ra, 8(sp)
    ld      s0, 0(sp)
    addi    sp, sp, 16
    ret

#else
#error "Unsupported architecture for VeridianOS crtn.S"
#endif
