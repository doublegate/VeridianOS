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
 * Calling convention to main():
 *   %rdi = argc (int)
 *   %rsi = argv (char **)
 *   %rdx = envp (char **)
 */

.section .text
.global _start
.type _start, @function

_start:
    /* Clear the frame pointer for clean backtraces. */
    xorl    %ebp, %ebp

    /* Load argc from the top of the stack. */
    movq    (%rsp), %rdi

    /* argv = rsp + 8 (pointer to the first argument string pointer). */
    leaq    8(%rsp), %rsi

    /*
     * envp = argv + (argc + 1) * 8
     *      = rsp + 8 + (argc + 1) * 8
     *      = rsp + 16 + argc * 8
     * We skip past argc pointers plus the NULL terminator.
     */
    leaq    16(%rsp,%rdi,8), %rdx

    /*
     * Align the stack to a 16-byte boundary before the call.
     * The System V AMD64 ABI requires 16-byte alignment at the point
     * of a CALL instruction (i.e., rsp % 16 == 0 just before CALL).
     */
    andq    $-16, %rsp

    /* Call main(argc, argv, envp). */
    call    main

    /*
     * main() returned in %eax.  Pass it to SYS_PROCESS_EXIT (syscall 11).
     * Argument goes in %rdi, syscall number in %rax.
     */
    movl    %eax, %edi
    movq    $11, %rax           /* SYS_PROCESS_EXIT */
    syscall

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
