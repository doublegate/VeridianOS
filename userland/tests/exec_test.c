/*
 * VeridianOS End-to-End Test -- exec_test.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Tests fork() + execve() + waitpid() using raw syscalls only -- no libc.
 * The child process exec's /bin/minimal which writes MINIMAL_TEST_PASS.
 *
 * Expected output on success:
 *   MINIMAL_TEST_PASS
 *   EXEC_TEST_PASS
 *
 * Syscall numbers from kernel/src/syscall/mod.rs:
 *   SYS_PROCESS_EXIT = 11  (status)
 *   SYS_PROCESS_FORK = 12  ()
 *   SYS_PROCESS_EXEC = 13  (path_ptr, argv_ptr, envp_ptr)
 *   SYS_PROCESS_WAIT = 14  (pid, status_ptr, options)
 *   SYS_FILE_WRITE   = 53  (fd, buf, count)
 *
 * Build: ${CC} -nostdlib -nostdinc -static -ffreestanding -o exec_test exec_test.c
 */

#define SYS_PROCESS_EXIT 11
#define SYS_PROCESS_FORK 12
#define SYS_PROCESS_EXEC 13
#define SYS_PROCESS_WAIT 14
#define SYS_FILE_WRITE   53
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
#error "exec_test only supports x86_64 for now"
#endif

static void write_str(const char *s)
{
    long len = 0;
    while (s[len]) len++;
    syscall3(SYS_FILE_WRITE, STDOUT_FD, (long)s, len);
}

void _start(void)
{
    long pid = syscall0(SYS_PROCESS_FORK);

    if (pid == 0) {
        /* Child process -- exec /bin/minimal */
        static const char path[] = "/bin/minimal";
        static const char *argv[] = { "minimal", (const char *)0 };
        static const char *envp[] = { (const char *)0 };

        syscall3(SYS_PROCESS_EXEC, (long)path, (long)argv, (long)envp);

        /* exec failed if we reach here */
        write_str("EXEC_FAILED\n");
        syscall1(SYS_PROCESS_EXIT, 1);
        __builtin_unreachable();
    } else if (pid > 0) {
        /* Parent process -- wait for child */
        int status = 0;
        syscall3(SYS_PROCESS_WAIT, pid, (long)&status, 0);
        write_str("EXEC_TEST_PASS\n");
        syscall1(SYS_PROCESS_EXIT, 0);
        __builtin_unreachable();
    } else {
        /* Fork failed */
        write_str("FORK_FAILED\n");
        syscall1(SYS_PROCESS_EXIT, 1);
        __builtin_unreachable();
    }
}
