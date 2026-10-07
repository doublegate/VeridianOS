# VeridianOS SDK Reference

System call reference for VeridianOS. This document covers the syscall ABI
for all three supported architectures, how calls are numbered, error values,
and usage examples.

## Syscall Convention by Architecture

### x86_64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `rax`                |
| Argument 1       | `rdi`                |
| Argument 2       | `rsi`                |
| Argument 3       | `rdx`                |
| Argument 4       | `r10`                |
| Argument 5       | `r8`                 |
| Argument 6       | `r9`                 |
| Return value     | `rax`                |
| Instruction      | `syscall`            |
| Clobbered        | `rcx`, `r11`         |

The `syscall` instruction saves `rip` into `rcx` and `rflags` into `r11`,
then jumps to the kernel entry point configured in the `LSTAR` MSR.

### AArch64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `x8`                 |
| Argument 1       | `x0`                 |
| Argument 2       | `x1`                 |
| Argument 3       | `x2`                 |
| Argument 4       | `x3`                 |
| Argument 5       | `x4`                 |
| Return value     | `x0`                 |
| Instruction      | `svc #0`             |

The `svc` (supervisor call) instruction generates a synchronous exception
routed to the kernel's EL1 exception vector.

### RISC-V 64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `a7`                 |
| Argument 1       | `a0`                 |
| Argument 2       | `a1`                 |
| Argument 3       | `a2`                 |
| Argument 4       | `a3`                 |
| Argument 5       | `a4`                 |
| Return value     | `a0`                 |
| Instruction      | `ecall`              |

The `ecall` instruction generates an environment call exception, trapping to
the supervisor (S-mode) handler.

### Return Value Convention

On success, the return value in the result register is a non-negative value
(zero or a positive result such as bytes read, a PID, or a file descriptor).

On error, the return value is a negative Linux errno value (`-EINVAL` is -22),
as on Linux, so a C library's `__syscall_ret` can be used unchanged.

## System Call Numbers

VeridianOS uses the Linux system call ABI (ADR 0009). A call Linux also has
uses its Linux x86_64 number and name (`write` is 1, `openat` 257); a call only
VeridianOS has (IPC, capabilities, packages, the framebuffer and Wayland
calls, audio) has a private number from 1024 up. One file defines all of
them, `abi/syscalls.map`, and `tools/compat/gen_syscalls.py` generates:

- `docs/compat/SYSCALL-NUMBERS.md`: the table of every call and its number
- `<veridian/sysno.h>`: `SYS_<linux name>` (`SYS_write`) and
  `SYS_<NAME>` (`SYS_IPC_SEND`) for C
- `userland/abi/syscall_numbers.rs`: the same constants for Rust
- `kernel/src/syscall/numbers.rs`: the kernel's `Syscall` enum

Arguments and results of a Linux-numbered call follow the Linux man page.
How fully each one is implemented is tracked in
`docs/compat/LINUX-SYSCALL-COVERAGE.md`; known differences are in
`docs/KNOWN-LIMITATIONS.md`. AArch64 and RISC-V use the same numbers for now;
they move to Linux's generic table when they get user mode.

## Error Codes

Errors are returned as negative Linux errno values (`-ENOENT`, `-EBADF`, ...).
Inside the kernel they are `SyscallError` values (`kernel/src/syscall/mod.rs`),
converted on the way out by `to_linux_errno`
(`kernel/src/syscall/linux_compat.rs`); an unknown call number is `-ENOSYS`.

### User-Space Pointer Validation

Every syscall that accepts a user-space pointer validates it before use:

1. Non-null (pointer is not zero)
2. User-space range (entire buffer falls below `0x0000_7FFF_FFFF_FFFF`)
3. No arithmetic overflow (`ptr + size` does not wrap)
4. Size cap (buffer does not exceed 256 MB)
5. Alignment (for typed access, pointer is suitably aligned for the type)

Violation of any check returns `-EFAULT` (or `-EACCES` for a kernel address).

### Rate Limiting

Syscalls are rate-limited using a token bucket algorithm. If the rate limit is
exceeded, the syscall returns `-EAGAIN`. The default configuration
allows approximately 10,000 syscalls per refill period.

## C Wrapper Examples

These examples assume a freestanding environment (no libc). For a libc-based
environment, use the standard POSIX wrappers once a libc port is available.

### x86_64 Inline Assembly

```c
#include <stddef.h>  /* size_t -- or define manually */
#include <veridian/sysno.h>  /* SYS_write, ... */

typedef long ssize_t;

static inline long veridian_syscall0(long num) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall1(long num, long a1) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num), "D"(a1) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall2(long num, long a1, long a2) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num), "D"(a1), "S"(a2) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(num), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory");
    return ret;
}

/* High-level wrappers */

static inline long vos_write(int fd, const void *buf, size_t count) {
    return veridian_syscall3(SYS_write, fd, (long)buf, (long)count);
}

static inline long vos_read(int fd, void *buf, size_t count) {
    return veridian_syscall3(SYS_read, fd, (long)buf, (long)count);
}

static inline long vos_getpid(void) {
    return veridian_syscall0(SYS_getpid);
}

static inline _Noreturn void vos_exit(int code) {
    veridian_syscall1(SYS_exit_group, code);
    __builtin_unreachable();
}

static inline long vos_fork(void) {
    return veridian_syscall0(SYS_fork);
}

static inline long vos_open(const char *path, int flags, int mode) {
    return veridian_syscall3(SYS_open, (long)path, flags, mode);
}

static inline long vos_close(int fd) {
    return veridian_syscall1(SYS_close, fd);
}

static inline long vos_mkdir(const char *path, int mode) {
    return veridian_syscall2(SYS_mkdir, (long)path, mode);
}

static inline long vos_uptime_ms(void) {
    return veridian_syscall0(SYS_TIME_GET_UPTIME);
}
```

### AArch64 Inline Assembly

```c
static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    register long x8 __asm__("x8") = num;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    __asm__ volatile("svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2)
        : "memory");
    return x0;
}
```

### RISC-V 64 Inline Assembly

```c
static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    register long a7 __asm__("a7") = num;
    register long a0 __asm__("a0") = a1;
    register long a1r __asm__("a1") = a2;
    register long a2r __asm__("a2") = a3;
    __asm__ volatile("ecall"
        : "+r"(a0)
        : "r"(a7), "r"(a1r), "r"(a2r)
        : "memory");
    return a0;
}
```

## Rust Wrapper Examples

For `no_std` Rust programs targeting VeridianOS:

```rust
#![no_std]
#![no_main]

use core::arch::asm;

/// Raw syscall with 3 arguments (x86_64).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall3(num: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    asm!(
        "syscall",
        inlateout("rax") num as isize => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

/// Raw syscall with 0 arguments (x86_64).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall0(num: usize) -> isize {
    let ret: isize;
    asm!(
        "syscall",
        inlateout("rax") num as isize => ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

// Syscall numbers, generated from abi/syscalls.map
include!("../abi/syscall_numbers.rs");
use sysno::*;

/// Write bytes to a file descriptor.
pub fn write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_write, fd, buf.as_ptr() as usize, buf.len()) }
}

/// Read bytes from a file descriptor.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_read, fd, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Get the current process ID.
pub fn getpid() -> isize {
    unsafe { syscall0(SYS_getpid) }
}

/// Get monotonic uptime in milliseconds.
pub fn uptime_ms() -> isize {
    unsafe { syscall0(SYS_TIME_GET_UPTIME) }
}

/// Exit the current process.
pub fn exit(code: usize) -> ! {
    unsafe {
        let _: isize;
        asm!(
            "syscall",
            in("rax") SYS_exit_group,
            in("rdi") code,
            lateout("rcx") _,
            lateout("r11") _,
            options(noreturn, nostack)
        );
    }
}

/// Entry point for a freestanding Rust program on VeridianOS.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello from Rust on VeridianOS!\n";
    write(1, msg);
    exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let msg = b"PANIC\n";
    write(2, msg);
    exit(1);
}
```

## Memory Layout

User-space programs occupy the lower half of the virtual address space:

```
User space:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF  (128 TB)
Kernel space: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF  (128 TB)
```

All user-space pointers passed to syscalls must fall within the user-space
range. The kernel validates this for every pointer argument.

## Audit and Rate Limiting

Every syscall is:

1. **Counted** -- a global atomic counter tracks total syscall invocations
2. **Rate-limited** -- a token bucket algorithm throttles excessive syscall
   rates, returning `WouldBlock` (-6) when the limit is exceeded
3. **Audited** -- the caller PID, syscall number, and success/failure are
   logged to the security audit subsystem
4. **Speculation-barriered** -- a speculation barrier is issued at syscall
   entry to mitigate Spectre-style attacks
