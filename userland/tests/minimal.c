/*
 * VeridianOS End-to-End Test -- minimal.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal test using raw syscalls only -- no libc, no headers.
 * This validates that the syscall ABI is correct and that a
 * freestanding static binary can execute on VeridianOS.
 *
 * Syscall numbers from kernel/src/syscall/mod.rs:
 *   SYS_FILE_WRITE   = 53   (fd, buf, count)
 *   SYS_PROCESS_EXIT = 11   (status)
 *
 * Architecture-specific calling conventions:
 *   x86_64:  syscall   -- nr in rax, args in rdi/rsi/rdx/r10/r8/r9
 *   aarch64: svc #0    -- nr in x8,  args in x0-x5
 *   riscv64: ecall     -- nr in a7,  args in a0-a5
 *
 * Build: ${CC} -nostdlib -nostdinc -static -ffreestanding -o minimal minimal.c
 * (The arch-specific crt0.S from toolchain/sysroot/crt/ provides _start
 *  which calls main().  For the truly minimal path, this file provides
 *  its own _start.)
 */

#define SYS_FILE_WRITE   53
#define SYS_PROCESS_EXIT 11

/* Stdout file descriptor */
#define STDOUT_FD 1

/* ========================================================================= */
/* Architecture-specific raw syscall wrappers                                */
/* ========================================================================= */

#if defined(__x86_64__)

static long syscall1(long nr, long a1)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long syscall3(long nr, long a1, long a2, long a3)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory"
    );
    return ret;
}

#elif defined(__aarch64__)

static long syscall1(long nr, long a1)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8)
        : "memory"
    );
    return x0;
}

static long syscall3(long nr, long a1, long a2, long a3)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2)
        : "memory"
    );
    return x0;
}

#elif defined(__riscv) && __riscv_xlen == 64

static long syscall1(long nr, long a1)
{
    register long a7 __asm__("a7") = nr;
    register long a0 __asm__("a0") = a1;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7)
        : "memory"
    );
    return a0;
}

static long syscall3(long nr, long a1, long a2, long a3)
{
    register long a7  __asm__("a7") = nr;
    register long a0  __asm__("a0") = a1;
    register long ra1 __asm__("a1") = a2;
    register long ra2 __asm__("a2") = a3;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7), "r"(ra1), "r"(ra2)
        : "memory"
    );
    return a0;
}

#else
#error "Unsupported architecture for minimal test"
#endif

/* ========================================================================= */
/* Entry point                                                               */
/* ========================================================================= */

/*
 * _start: true entry point with no C runtime.
 *
 * If linked with crt0.o, the linker will use crt0's _start instead
 * and call main().  When built with -nostdlib and no crt0, this
 * _start is the ELF entry point.
 */
void _start(void)
{
    /* Write the success marker to stdout */
    static const char msg[] = "MINIMAL_TEST_PASS\n";
    syscall3(SYS_FILE_WRITE, STDOUT_FD, (long)msg, sizeof(msg) - 1);

    /* Exit with success */
    syscall1(SYS_PROCESS_EXIT, 0);

    /* Should never reach here */
    __builtin_unreachable();
}
