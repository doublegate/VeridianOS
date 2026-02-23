/*
 * VeridianOS C Runtime Startup -- x86_64
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Entry point for statically linked user-space executables.
 * The kernel sets up the initial stack as:
 *
 *   (%rsp)      -> argc       (8 bytes)
 *   8(%rsp)     -> argv[0]    (pointer)
 *   ...         -> argv[argc-1]
 *   ...         -> NULL       (argv terminator)
 *   ...         -> envp[0]    (pointer)
 *   ...         -> NULL       (envp terminator)
 *
 * _start passes the raw stack pointer to __libc_start_main(), which
 * processes .init_array (C++ static constructors) before calling main().
 */

.section .text
.global _start
.type _start, @function

_start:
    /* Clear the frame pointer for clean backtraces. */
    xorl    %ebp, %ebp

    /* Pass the raw stack pointer to __libc_start_main. */
    movq    %rsp, %rdi

    /*
     * Align the stack to a 16-byte boundary before the call.
     * The System V AMD64 ABI requires 16-byte alignment at the point
     * of a CALL instruction (i.e., rsp % 16 == 0 just before CALL).
     */
    andq    $-16, %rsp

    /* __libc_start_main processes .init_array, calls main(), and exits. */
    call    __libc_start_main

    /* Should never reach here.  Trigger an invalid-opcode exception. */
    ud2

.size _start, . - _start

/*
 * Provide a weak _exit symbol so bare-metal programs can call _exit()
 * without pulling in a full libc.
 */
.global _exit
.weak _exit
.type _exit, @function

_exit:
    movq    $11, %rax           /* SYS_PROCESS_EXIT */
    /* %rdi already holds the exit code per calling convention. */
    syscall
    ud2

.size _exit, . - _exit
