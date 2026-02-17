/*
 * VeridianOS C Runtime Initialization Prologue -- All Architectures
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * This file provides the .init and .fini section prologues.
 * The linker concatenates:
 *   crti.o (.init prologue)  +  crtbegin.o (.init body)  +  crtn.o (.init epilogue)
 *   crti.o (.fini prologue)  +  crtend.o  (.fini body)   +  crtn.o (.fini epilogue)
 *
 * Together they form the _init() and _fini() functions that run
 * global constructors and destructors respectively.
 */

#if defined(__x86_64__)

/*
 * x86_64: Function prologue is just a stack frame push.
 * The CALL to _init will push the return address; we push rbp
 * so that debuggers can walk the frame chain.
 */

.section .init
.global _init
.type _init, @function
_init:
    pushq   %rbp
    movq    %rsp, %rbp
    /* --- crtbegin.o inserts constructor calls here --- */

.section .fini
.global _fini
.type _fini, @function
_fini:
    pushq   %rbp
    movq    %rsp, %rbp
    /* --- crtend.o inserts destructor calls here --- */

#elif defined(__aarch64__)

/*
 * AArch64: Save frame pointer (x29) and link register (x30).
 */

.section .init
.global _init
.type _init, %function
_init:
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp
    /* --- crtbegin.o inserts constructor calls here --- */

.section .fini
.global _fini
.type _fini, %function
_fini:
    stp     x29, x30, [sp, #-16]!
    mov     x29, sp
    /* --- crtend.o inserts destructor calls here --- */

#elif defined(__riscv) && __riscv_xlen == 64

/*
 * RISC-V 64: Save return address (ra) and frame pointer (s0).
 */

.section .init
.global _init
.type _init, @function
_init:
    addi    sp, sp, -16
    sd      ra, 8(sp)
    sd      s0, 0(sp)
    addi    s0, sp, 16
    /* --- crtbegin.o inserts constructor calls here --- */

.section .fini
.global _fini
.type _fini, @function
_fini:
    addi    sp, sp, -16
    sd      ra, 8(sp)
    sd      s0, 0(sp)
    addi    s0, sp, 16
    /* --- crtend.o inserts destructor calls here --- */

#else
#error "Unsupported architecture for VeridianOS crti.S"
#endif
