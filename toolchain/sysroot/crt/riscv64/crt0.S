/*
 * VeridianOS C Runtime Startup -- RISC-V 64
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Entry point for statically linked user-space executables.
 * The kernel sets up the initial stack as:
 *
 *   (sp)        -> argc       (8 bytes)
 *   sp + 8      -> argv[0]    (pointer)
 *   ...         -> argv[argc-1]
 *   ...         -> NULL       (argv terminator)
 *   ...         -> envp[0]    (pointer)
 *   ...         -> NULL       (envp terminator)
 *
 * Calling convention to main():
 *   a0 = argc (int)
 *   a1 = argv (char **)
 *   a2 = envp (char **)
 */

.section .text
.global _start
.type _start, @function

_start:
    /* Clear the frame pointer (s0/fp) and return address for clean backtraces. */
    li      s0, 0
    li      ra, 0

    /* Load argc from the top of the stack. */
    ld      a0, 0(sp)

    /* argv = sp + 8 */
    addi    a1, sp, 8

    /*
     * envp = argv + (argc + 1) * 8
     *      = sp + 8 + (argc + 1) * 8
     *      = sp + 16 + argc * 8
     */
    slli    a2, a0, 3           /* argc * 8 */
    add     a2, a2, sp          /* sp + argc * 8 */
    addi    a2, a2, 16          /* sp + argc * 8 + 16 = envp */

    /*
     * Align the stack to a 16-byte boundary.
     * RISC-V calling convention requires 16-byte stack alignment.
     */
    andi    sp, sp, -16

    /* Set up the global pointer (gp) for relaxed addressing. */
.option push
.option norelax
    la      gp, __global_pointer$
.option pop

    /* Call main(argc, argv, envp). */
    call    main

    /*
     * main() returned in a0.
     * Pass it to SYS_PROCESS_EXIT (syscall 11).
     * RISC-V VeridianOS: syscall number in a7, args in a0-a5.
     * a0 already holds the exit code from main()'s return value.
     */
    li      a7, 11              /* SYS_PROCESS_EXIT */
    ecall

    /* Should never reach here.  Trigger an illegal instruction. */
    unimp

.size _start, . - _start

/*
 * Provide a weak _exit symbol so bare-metal programs can call _exit()
 * without pulling in a full libc.
 */
.global _exit
.weak _exit
.type _exit, @function

_exit:
    li      a7, 11              /* SYS_PROCESS_EXIT */
    /* a0 already holds the exit code per calling convention. */
    ecall
    unimp

.size _exit, . - _exit

/*
 * Weak global pointer symbol.  The linker will override this
 * with the actual __global_pointer$ value from the linker script.
 */
.weak __global_pointer$
