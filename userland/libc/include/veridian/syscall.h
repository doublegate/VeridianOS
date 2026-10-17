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
#include <errno.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Syscall Numbers                                                           */
/* ========================================================================= */

/*
 * A raw system call result in the POSIX convention: a negative value is
 * -errno, so set errno and return -1; anything else is the result.
 */
static inline long __syscall_ret(long r)
{
    if (r < 0) {
        errno = (int)(-r);
        return -1;
    }
    return r;
}

/* System call numbers: generated from abi/syscalls.map (ADR 0009). */
#include <veridian/sysno.h>

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
#define CLONE_VFORK             0x00004000
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
