/*
 * VeridianOS System Call Interface
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Syscall numbers and inline assembly wrappers matching kernel/src/syscall/mod.rs.
 * Architecture-specific calling conventions:
 *   x86_64:  syscall instruction, nr in rax, args in rdi/rsi/rdx/r10/r8/r9
 *   aarch64: svc #0, nr in x8, args in x0-x5
 *   riscv64: ecall, nr in a7, args in a0-a5
 */

#ifndef VERIDIAN_SYSCALL_H
#define VERIDIAN_SYSCALL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Syscall Numbers                                                           */
/* ========================================================================= */

/* IPC system calls (0-7) */
#define SYS_IPC_SEND            0
#define SYS_IPC_RECEIVE         1
#define SYS_IPC_CALL            2
#define SYS_IPC_REPLY           3
#define SYS_IPC_CREATE_ENDPOINT 4
#define SYS_IPC_BIND_ENDPOINT   5
#define SYS_IPC_SHARE_MEMORY   6
#define SYS_IPC_MAP_MEMORY      7

/* Process management (10-18) */
#define SYS_PROCESS_YIELD       10
#define SYS_PROCESS_EXIT        11
#define SYS_PROCESS_FORK        12
#define SYS_PROCESS_EXEC        13
#define SYS_PROCESS_WAIT        14
#define SYS_PROCESS_GETPID      15
#define SYS_PROCESS_GETPPID     16
#define SYS_PROCESS_SETPRIORITY 17
#define SYS_PROCESS_GETPRIORITY 18

/* Memory management (20-23) */
#define SYS_MEMORY_MAP          20
#define SYS_MEMORY_UNMAP        21
#define SYS_MEMORY_PROTECT      22
#define SYS_MEMORY_BRK          23

/* Capability management (30-31) */
#define SYS_CAPABILITY_GRANT    30
#define SYS_CAPABILITY_REVOKE   31

/* Thread management (40-46) */
#define SYS_THREAD_CREATE       40
#define SYS_THREAD_EXIT         41
#define SYS_THREAD_JOIN         42
#define SYS_THREAD_GETTID       43
#define SYS_THREAD_SET_AFFINITY 44
#define SYS_THREAD_GET_AFFINITY 45
#define SYS_THREAD_CLONE        46

/* Filesystem operations (50-59) */
#define SYS_FILE_OPEN           50
#define SYS_FILE_CLOSE          51
#define SYS_FILE_READ           52
#define SYS_FILE_WRITE          53
#define SYS_FILE_SEEK           54
#define SYS_FILE_STAT           55
#define SYS_FILE_TRUNCATE       56
#define SYS_FILE_DUP            57
#define SYS_FILE_DUP2           58
#define SYS_FILE_PIPE           59

/* Directory operations (60-66) */
#define SYS_DIR_MKDIR           60
#define SYS_DIR_RMDIR           61
#define SYS_DIR_OPENDIR         62
#define SYS_DIR_READDIR         63
#define SYS_DIR_CLOSEDIR        64
#define SYS_FILE_PIPE2          65
#define SYS_FILE_DUP3           66

/* Filesystem management (70-73) */
#define SYS_FS_MOUNT            70
#define SYS_FS_UNMOUNT          71
#define SYS_FS_SYNC             72
#define SYS_FS_FSYNC            73

/* Kernel information (80) */
#define SYS_KERNEL_GET_INFO     80

/* Package management (90-94) */
#define SYS_PKG_INSTALL         90
#define SYS_PKG_REMOVE          91
#define SYS_PKG_QUERY           92
#define SYS_PKG_LIST            93
#define SYS_PKG_UPDATE          94

/* Time management (100-102) */
#define SYS_TIME_GET_UPTIME     100
#define SYS_TIME_CREATE_TIMER   101
#define SYS_TIME_CANCEL_TIMER   102

/* Extended process operations (110-113) */
#define SYS_PROCESS_GETCWD      110
#define SYS_PROCESS_CHDIR       111
#define SYS_FILE_IOCTL          112
#define SYS_PROCESS_KILL        113

/* Signal handling (120-123) */
#define SYS_SIGACTION           120
#define SYS_SIGPROCMASK         121
#define SYS_SIGSUSPEND          122
#define SYS_SIGRETURN           123

/* Debugging (140) */
#define SYS_PTRACE              140

/* Extended file operations (150-158) */
#define SYS_FILE_STAT_PATH      150
#define SYS_FILE_LSTAT          151
#define SYS_FILE_READLINK       152
#define SYS_FILE_ACCESS         153
#define SYS_FILE_RENAME         154
#define SYS_FILE_UNLINK         157
#define SYS_FILE_FCNTL          158

/* POSIX time syscalls (160-163) */
#define SYS_CLOCK_GETTIME       160
#define SYS_CLOCK_GETRES        161
#define SYS_NANOSLEEP           162
#define SYS_GETTIMEOFDAY        163

/* Identity syscalls (170-175) */
#define SYS_GETUID              170
#define SYS_GETEUID             171
#define SYS_GETGID              172
#define SYS_GETEGID             173
#define SYS_SETUID              174
#define SYS_SETGID              175

/* Process group / session syscalls (176-180) */
#define SYS_SETPGID             176
#define SYS_GETPGID             177
#define SYS_GETPGRP             178
#define SYS_SETSID              179
#define SYS_GETSID              180

/* Scatter/gather I/O (183-184) */
#define SYS_READV               183
#define SYS_WRITEV              184

/* Filesystem link/symlink/chmod (155-156, 185-196) */
#define SYS_FILE_LINK           155
#define SYS_FILE_SYMLINK        156
#define SYS_FILE_CHMOD          185
#define SYS_FILE_FCHMOD         186
#define SYS_PROCESS_UMASK       187
#define SYS_FILE_TRUNCATE_PATH  188
#define SYS_FILE_POLL           189
#define SYS_FILE_OPENAT         190
#define SYS_FILE_FSTATAT        191
#define SYS_FILE_UNLINKAT       192
#define SYS_FILE_MKDIRAT        193
#define SYS_FILE_RENAMEAT       194
#define SYS_FILE_PREAD          195
#define SYS_FILE_PWRITE         196

/* Ownership and device node syscalls (197-200) */
#define SYS_FILE_CHOWN          197
#define SYS_FILE_FCHOWN         198
#define SYS_FILE_MKNOD          199
#define SYS_FILE_SELECT         200

/* Futex + arch-specific control (201-203) */
#define SYS_FUTEX_WAIT          201
#define SYS_FUTEX_WAKE          202
#define SYS_ARCH_PRCTL          203

/* System information (204-205) */
#define SYS_PROCESS_UNAME       204
#define SYS_PROCESS_GETENV      205

/* AT_* constants for *at() syscalls */
#define AT_FDCWD                (-100)
#define AT_REMOVEDIR            0x200
#define AT_SYMLINK_NOFOLLOW     0x100

/* poll() event flags */
#define POLLIN                  0x0001
#define POLLOUT                 0x0004
#define POLLERR                 0x0008
#define POLLHUP                 0x0010
#define POLLNVAL                0x0020

/* clone(2) flags (subset aligned with kernel) */
#define CLONE_VM                0x00000100
#define CLONE_FS                0x00000200
#define CLONE_FILES             0x00000400
#define CLONE_SIGHAND           0x00000800
#define CLONE_THREAD            0x00010000
#define CLONE_SETTLS            0x00080000
#define CLONE_PARENT_SETTID     0x00100000
#define CLONE_CHILD_CLEARTID    0x00200000
#define CLONE_CHILD_SETTID      0x01000000

/* arch_prctl codes (x86_64 compatible) */
#define ARCH_SET_FS             0x1002
#define ARCH_GET_FS             0x1003

/* Futex operations (subset) */
#define FUTEX_WAIT              0
#define FUTEX_WAKE              1
#define FUTEX_REQUEUE           3
#define FUTEX_WAIT_BITSET       9
#define FUTEX_WAKE_OP           5
#define FUTEX_PRIVATE_FLAG      0x80
#define FUTEX_CLOCK_REALTIME    0x100
#define FUTEX_BITSET_MATCH_ANY  0xFFFFFFFF

/* ========================================================================= */
/* Architecture-Specific Syscall Wrappers                                    */
/* ========================================================================= */

#if defined(__x86_64__)

static inline long __veridian_syscall0(long nr)
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

static inline long __veridian_syscall1(long nr, long a1)
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

static inline long __veridian_syscall2(long nr, long a1, long a2)
{
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long __veridian_syscall3(long nr, long a1, long a2, long a3)
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

static inline long __veridian_syscall4(long nr, long a1, long a2, long a3,
                                       long a4)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3), "r"(r10)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long __veridian_syscall5(long nr, long a1, long a2, long a3,
                                       long a4, long a5)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    register long r8  __asm__("r8")  = a5;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3), "r"(r10), "r"(r8)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static inline long __veridian_syscall6(long nr, long a1, long a2, long a3,
                                       long a4, long a5, long a6)
{
    long ret;
    register long r10 __asm__("r10") = a4;
    register long r8  __asm__("r8")  = a5;
    register long r9  __asm__("r9")  = a6;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(nr), "D"(a1), "S"(a2), "d"(a3), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

#elif defined(__aarch64__)

static inline long __veridian_syscall0(long nr)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0");
    __asm__ volatile (
        "svc #0"
        : "=r"(x0)
        : "r"(x8)
        : "memory"
    );
    return x0;
}

static inline long __veridian_syscall1(long nr, long a1)
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

static inline long __veridian_syscall2(long nr, long a1, long a2)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1)
        : "memory"
    );
    return x0;
}

static inline long __veridian_syscall3(long nr, long a1, long a2, long a3)
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

static inline long __veridian_syscall4(long nr, long a1, long a2, long a3,
                                       long a4)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    register long x3 __asm__("x3") = a4;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2), "r"(x3)
        : "memory"
    );
    return x0;
}

static inline long __veridian_syscall5(long nr, long a1, long a2, long a3,
                                       long a4, long a5)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    register long x3 __asm__("x3") = a4;
    register long x4 __asm__("x4") = a5;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4)
        : "memory"
    );
    return x0;
}

static inline long __veridian_syscall6(long nr, long a1, long a2, long a3,
                                       long a4, long a5, long a6)
{
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    register long x3 __asm__("x3") = a4;
    register long x4 __asm__("x4") = a5;
    register long x5 __asm__("x5") = a6;
    __asm__ volatile (
        "svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5)
        : "memory"
    );
    return x0;
}

#elif defined(__riscv) && __riscv_xlen == 64

static inline long __veridian_syscall0(long nr)
{
    register long a7 __asm__("a7") = nr;
    register long a0 __asm__("a0");
    __asm__ volatile (
        "ecall"
        : "=r"(a0)
        : "r"(a7)
        : "memory"
    );
    return a0;
}

static inline long __veridian_syscall1(long nr, long a1)
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

static inline long __veridian_syscall2(long nr, long a1, long a2)
{
    register long a7  __asm__("a7") = nr;
    register long a0  __asm__("a0") = a1;
    register long ra1 __asm__("a1") = a2;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7), "r"(ra1)
        : "memory"
    );
    return a0;
}

static inline long __veridian_syscall3(long nr, long a1, long a2, long a3)
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

static inline long __veridian_syscall4(long nr, long a1, long a2, long a3,
                                       long a4)
{
    register long a7  __asm__("a7") = nr;
    register long a0  __asm__("a0") = a1;
    register long ra1 __asm__("a1") = a2;
    register long ra2 __asm__("a2") = a3;
    register long ra3 __asm__("a3") = a4;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7), "r"(ra1), "r"(ra2), "r"(ra3)
        : "memory"
    );
    return a0;
}

static inline long __veridian_syscall5(long nr, long a1, long a2, long a3,
                                       long a4, long a5)
{
    register long a7  __asm__("a7") = nr;
    register long a0  __asm__("a0") = a1;
    register long ra1 __asm__("a1") = a2;
    register long ra2 __asm__("a2") = a3;
    register long ra3 __asm__("a3") = a4;
    register long ra4 __asm__("a4") = a5;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7), "r"(ra1), "r"(ra2), "r"(ra3), "r"(ra4)
        : "memory"
    );
    return a0;
}

static inline long __veridian_syscall6(long nr, long a1, long a2, long a3,
                                       long a4, long a5, long a6)
{
    register long a7  __asm__("a7") = nr;
    register long a0  __asm__("a0") = a1;
    register long ra1 __asm__("a1") = a2;
    register long ra2 __asm__("a2") = a3;
    register long ra3 __asm__("a3") = a4;
    register long ra4 __asm__("a4") = a5;
    register long ra5 __asm__("a5") = a6;
    __asm__ volatile (
        "ecall"
        : "+r"(a0)
        : "r"(a7), "r"(ra1), "r"(ra2), "r"(ra3), "r"(ra4), "r"(ra5)
        : "memory"
    );
    return a0;
}

#else
#error "Unsupported architecture for VeridianOS syscall wrappers"
#endif

/* ========================================================================= */
/* Convenience Macros                                                        */
/* ========================================================================= */

#define veridian_syscall0(nr)                       __veridian_syscall0((nr))
#define veridian_syscall1(nr, a1)                   __veridian_syscall1((nr), (long)(a1))
#define veridian_syscall2(nr, a1, a2)               __veridian_syscall2((nr), (long)(a1), (long)(a2))
#define veridian_syscall3(nr, a1, a2, a3)           __veridian_syscall3((nr), (long)(a1), (long)(a2), (long)(a3))
#define veridian_syscall4(nr, a1, a2, a3, a4)       __veridian_syscall4((nr), (long)(a1), (long)(a2), (long)(a3), (long)(a4))
#define veridian_syscall5(nr, a1, a2, a3, a4, a5)   __veridian_syscall5((nr), (long)(a1), (long)(a2), (long)(a3), (long)(a4), (long)(a5))
#define veridian_syscall6(nr, a1, a2, a3, a4, a5, a6) __veridian_syscall6((nr), (long)(a1), (long)(a2), (long)(a3), (long)(a4), (long)(a5), (long)(a6))

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_SYSCALL_H */
