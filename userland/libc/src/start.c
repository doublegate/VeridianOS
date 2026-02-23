/*
 * VeridianOS libc -- start.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * C runtime startup glue.
 *
 * The arch-specific crt0.S (in toolchain/sysroot/crt/<arch>/crt0.S) is
 * the actual ELF entry point (_start).  It passes the raw stack pointer
 * to __libc_start_main(), which:
 *   1. Initializes libc state (environ, stdio)
 *   2. Runs .init_array constructors (C++ static init)
 *   3. Calls main(argc, argv, envp)
 *   4. Runs .fini_array destructors in reverse order
 *   5. Calls exit()
 *
 * It also provides __libc_init() for CRT0 implementations that want
 * to call libc initialization without replacing the main() call path.
 */

#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
#include <stddef.h>

/* Defined in stdlib.c. */
extern char **environ;

/* Defined in stdio.c. */
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

/*
 * Linker-defined symbols bracketing the .init_array and .fini_array
 * sections.  The default linker script creates these via PROVIDE_HIDDEN.
 * For binaries without constructors, start == end (zero iterations).
 */
typedef void (*init_func)(void);

extern init_func __init_array_start[] __attribute__((weak));
extern init_func __init_array_end[]   __attribute__((weak));
extern init_func __fini_array_start[] __attribute__((weak));
extern init_func __fini_array_end[]   __attribute__((weak));

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
 * Entry point called by crt0.S.
 *
 * Processes .init_array (C++ static constructors), calls main(),
 * then processes .fini_array (static destructors) and exits.
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
void __libc_start_main(long *sp)
{
    int argc = (int)sp[0];
    char **argv = (char **)(sp + 1);
    char **envp = argv + argc + 1;

    __libc_init(envp);

    /* Run C/C++ static constructors (.init_array).
     * The linker merges .ctors entries into .init_array, so processing
     * .init_array alone covers both modern and legacy constructors. */
    if (__init_array_start != NULL) {
        size_t count = (size_t)(__init_array_end - __init_array_start);
        for (size_t i = 0; i < count; i++) {
            __init_array_start[i]();
        }
    }

    extern int main(int, char **, char **);
    int ret = main(argc, argv, envp);

    /* Run C/C++ static destructors (.fini_array) in reverse order. */
    if (__fini_array_start != NULL) {
        size_t count = (size_t)(__fini_array_end - __fini_array_start);
        for (size_t i = count; i > 0; i--) {
            __fini_array_start[i - 1]();
        }
    }

    exit(ret);
}
