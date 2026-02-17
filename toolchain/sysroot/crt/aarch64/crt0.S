/*
 * VeridianOS C Runtime Startup -- AArch64
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
 *   x0 = argc (int)
 *   x1 = argv (char **)
 *   x2 = envp (char **)
 */

.section .text
.global _start
.type _start, %function

_start:
    /* Clear the frame pointer for clean backtraces. */
    mov     x29, #0
    mov     x30, #0

    /* Load argc from the top of the stack. */
    ldr     x0, [sp]

    /* argv = sp + 8 */
    add     x1, sp, #8

    /*
     * envp = argv + (argc + 1) * 8
     *      = sp + 8 + (argc + 1) * 8
     *      = sp + 16 + argc * 8
     */
    add     x2, x0, #2         /* argc + 2 (to skip argc slot + NULL) */
    lsl     x2, x2, #3         /* * 8 bytes per pointer */
    add     x2, sp, x2         /* envp = sp + (argc + 2) * 8 */

    /*
     * Align the stack to a 16-byte boundary.
     * AArch64 ABI requires 16-byte stack alignment.
     */
    mov     x3, sp
    and     x3, x3, #-16
    mov     sp, x3

    /* Call main(argc, argv, envp). */
    bl      main

    /*
     * main() returned in w0 (lower 32 bits of x0).
     * Pass it to SYS_PROCESS_EXIT (syscall 11).
     * AArch64 VeridianOS: syscall number in x8, args in x0-x5.
     */
    mov     x8, #11             /* SYS_PROCESS_EXIT */
    svc     #0

    /* Should never reach here.  Trigger an undefined instruction exception. */
    brk     #1

.size _start, . - _start

/*
 * Provide a weak _exit symbol so bare-metal programs can call _exit()
 * without pulling in a full libc.
 */
.global _exit
.weak _exit
.type _exit, %function

_exit:
    mov     x8, #11             /* SYS_PROCESS_EXIT */
    /* x0 already holds the exit code per calling convention. */
    svc     #0
    brk     #1

.size _exit, . - _exit
