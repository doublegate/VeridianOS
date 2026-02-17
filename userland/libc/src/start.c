/*
 * VeridianOS libc -- start.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * C runtime startup glue.
 *
 * The arch-specific crt0.S (in toolchain/sysroot/crt/<arch>/crt0.S) is
 * the actual ELF entry point (_start).  It extracts argc/argv/envp from
 * the stack and calls main() directly, then invokes SYS_PROCESS_EXIT
 * with main's return value.
 *
 * This file provides __libc_start_main(), an alternative entry point
 * that initializes libc state (environ, stdio buffers) before calling
 * main().  If a future linker script routes _start -> __libc_start_main
 * instead of directly to main(), this will be used automatically.
 *
 * It also provides __libc_init() for CRT0 implementations that want
 * to call libc initialization without replacing the main() call path.
 */

#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>

/* Defined in stdlib.c. */
extern char **environ;

/* Defined in stdio.c. */
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

/*
 * Initialize libc subsystems.  Called before main().
 *
 * @param envp  Environment variable array from the kernel.
 */
void __libc_init(char **envp)
{
    /* Store the environment pointer. */
    environ = envp;

    /*
     * stdio streams (stdin/stdout/stderr) are statically initialized
     * in stdio.c, so no dynamic setup is needed.  Buffers are lazily
     * allocated on first use.
     */
}

/*
 * Alternative entry point for libc-aware startup.
 *
 * If crt0.S calls __libc_start_main instead of main(), this function
 * sets up the environment, calls main(), runs atexit handlers, and exits.
 *
 * @param sp    Pointer to the initial stack (points to argc).
 *
 * The stack layout (set up by the kernel) is:
 *   sp[0]          = argc
 *   sp[1..argc]    = argv pointers
 *   sp[argc+1]     = NULL (argv terminator)
 *   sp[argc+2..]   = envp pointers
 *   ...            = NULL (envp terminator)
 */
__attribute__((weak))
void __libc_start_main(long *sp)
{
    int argc = (int)sp[0];
    char **argv = (char **)(sp + 1);
    char **envp = argv + argc + 1;

    __libc_init(envp);

    extern int main(int, char **, char **);
    int ret = main(argc, argv, envp);

    exit(ret);
}
