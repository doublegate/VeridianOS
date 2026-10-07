/*
 * VeridianOS End-to-End Test -- fork_test.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Tests fork() + waitpid() using raw syscalls only -- no libc.
 *
 * Expected output on success:
 *   CHILD_OK
 *   FORK_TEST_PASS
 *
 * Syscall numbers from kernel/src/syscall/mod.rs:
 *   SYS_exit_group = 231  (status)
 *   SYS_fork = 57  ()
 *   SYS_wait4 = 61  (pid, status_ptr, options)
 *   SYS_write   = 53  (fd, buf, count)
 *
 * Build: ${CC} -nostdlib -nostdinc -static -ffreestanding -o fork_test fork_test.c
 */

#include <veridian/sysno.h> /* system call numbers (ADR 0009) */
#define STDOUT_FD        1

#if defined(__x86_64__)

static long syscall0(long nr)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr)
        : "rcx", "r11", "memory"
    );
    return ret;
}

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

#else
#error "fork_test only supports x86_64 for now"
#endif

static void write_str(const char *s)
{
    long len = 0;
    while (s[len]) len++;
    syscall3(SYS_write, STDOUT_FD, (long)s, len);
}

void _start(void)
{
    long pid = syscall0(SYS_fork);

    if (pid == 0) {
        /* Child process */
        write_str("CHILD_OK\n");
        syscall1(SYS_exit_group, 42);
        __builtin_unreachable();
    } else if (pid > 0) {
        /* Parent process -- wait for child */
        int status = 0;
        syscall3(SYS_wait4, pid, (long)&status, 0);
        write_str("FORK_TEST_PASS\n");
        syscall1(SYS_exit_group, 0);
        __builtin_unreachable();
    } else {
        /* Fork failed */
        write_str("FORK_FAILED\n");
        syscall1(SYS_exit_group, 1);
        __builtin_unreachable();
    }
}
