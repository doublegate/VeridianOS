//! VeridianOS platform implementation.
//!
//! This module contains all VeridianOS-specific implementations that bridge
//! Rust code to kernel syscalls. Each submodule corresponds to a functional
//! area of the platform layer.
//!
//! # Syscall Convention
//!
//! All syscall numbers match `kernel/src/syscall/mod.rs` and
//! `toolchain/sysroot/include/veridian/syscall.h`.
//!
//! Architecture-specific calling conventions:
//! - **x86_64**: `syscall` instruction, nr in `rax`, args in `rdi/rsi/rdx/r10/r8/r9`
//! - **aarch64**: `svc #0`, nr in `x8`, args in `x0-x5`
//! - **riscv64**: `ecall`, nr in `a7`, args in `a0-a5`

pub mod alloc;
pub mod fs;
pub mod io;
pub mod net;
pub mod os;
pub mod process;
pub mod thread;
pub mod time;

// ============================================================================
// Raw Syscall Interface
// ============================================================================

/// Invoke a syscall with 0 arguments.
#[inline(always)]
pub unsafe fn syscall0(nr: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees nr is a valid syscall number.
        // The syscall instruction is the standard x86_64 mechanism for
        // entering the kernel. rcx and r11 are clobbered by the CPU.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees nr is a valid syscall number.
        // svc #0 is the standard AArch64 supervisor call instruction.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") 0isize => ret,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees nr is a valid syscall number.
        // ecall is the standard RISC-V environment call instruction.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") 0isize => ret,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 1 argument.
#[inline(always)]
pub unsafe fn syscall1(nr: usize, a1: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and argument.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and argument.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and argument.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 2 arguments.
#[inline(always)]
pub unsafe fn syscall2(nr: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 3 arguments.
#[inline(always)]
pub unsafe fn syscall3(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 4 arguments.
#[inline(always)]
pub unsafe fn syscall4(nr: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x3") a4,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a3") a4,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 5 arguments.
#[inline(always)]
pub unsafe fn syscall5(
    nr: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x3") a4,
                in("x4") a5,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a3") a4,
                in("a4") a5,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

/// Invoke a syscall with 6 arguments.
#[inline(always)]
pub unsafe fn syscall6(
    nr: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
) -> isize {
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                in("r9") a6,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x3") a4,
                in("x4") a5,
                in("x5") a6,
                in("x8") nr,
                options(nostack),
            );
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: Caller guarantees valid syscall number and arguments.
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a3") a4,
                in("a4") a5,
                in("a5") a6,
                in("a7") nr,
                options(nostack),
            );
        }
    }

    ret
}

// ============================================================================
// Syscall Number Constants
// ============================================================================

// These must match kernel/src/syscall/mod.rs exactly.

// IPC (0-7)
pub const SYS_IPC_SEND: usize = 0;
pub const SYS_IPC_RECEIVE: usize = 1;
pub const SYS_IPC_CALL: usize = 2;
pub const SYS_IPC_REPLY: usize = 3;

// Process management (10-18)
pub const SYS_PROCESS_YIELD: usize = 10;
pub const SYS_PROCESS_EXIT: usize = 11;
pub const SYS_PROCESS_FORK: usize = 12;
pub const SYS_PROCESS_EXEC: usize = 13;
pub const SYS_PROCESS_WAIT: usize = 14;
pub const SYS_PROCESS_GETPID: usize = 15;
pub const SYS_PROCESS_GETPPID: usize = 16;

// Memory management (20-23)
pub const SYS_MEMORY_MAP: usize = 20;
pub const SYS_MEMORY_UNMAP: usize = 21;
pub const SYS_MEMORY_PROTECT: usize = 22;
pub const SYS_MEMORY_BRK: usize = 23;

// Thread management (40-46)
pub const SYS_THREAD_CREATE: usize = 40;
pub const SYS_THREAD_EXIT: usize = 41;
pub const SYS_THREAD_JOIN: usize = 42;
pub const SYS_THREAD_GETTID: usize = 43;
pub const SYS_THREAD_CLONE: usize = 46;

// Filesystem operations (50-59)
pub const SYS_FILE_OPEN: usize = 50;
pub const SYS_FILE_CLOSE: usize = 51;
pub const SYS_FILE_READ: usize = 52;
pub const SYS_FILE_WRITE: usize = 53;
pub const SYS_FILE_SEEK: usize = 54;
pub const SYS_FILE_STAT: usize = 55;
pub const SYS_FILE_TRUNCATE: usize = 56;
pub const SYS_FILE_DUP: usize = 57;
pub const SYS_FILE_DUP2: usize = 58;
pub const SYS_FILE_PIPE: usize = 59;

// Directory operations (60-66)
pub const SYS_DIR_MKDIR: usize = 60;
pub const SYS_DIR_RMDIR: usize = 61;
pub const SYS_DIR_OPENDIR: usize = 62;
pub const SYS_DIR_READDIR: usize = 63;
pub const SYS_DIR_CLOSEDIR: usize = 64;

// Filesystem management (70-72)
pub const SYS_FS_MOUNT: usize = 70;
pub const SYS_FS_UNMOUNT: usize = 71;
pub const SYS_FS_SYNC: usize = 72;

// Kernel info (80)
pub const SYS_KERNEL_GET_INFO: usize = 80;

// Package management (90-94)
pub const SYS_PKG_INSTALL: usize = 90;
pub const SYS_PKG_REMOVE: usize = 91;
pub const SYS_PKG_QUERY: usize = 92;
pub const SYS_PKG_LIST: usize = 93;
pub const SYS_PKG_UPDATE: usize = 94;

// Time management (100-102)
pub const SYS_TIME_GET_UPTIME: usize = 100;

// Extended process operations (110-113)
pub const SYS_PROCESS_GETCWD: usize = 110;
pub const SYS_PROCESS_CHDIR: usize = 111;
pub const SYS_PROCESS_KILL: usize = 113;

// Signal handling (120-123)
pub const SYS_SIGACTION: usize = 120;
pub const SYS_SIGPROCMASK: usize = 121;

// Extended filesystem (150-158)
pub const SYS_FILE_STAT_PATH: usize = 150;
pub const SYS_FILE_UNLINK: usize = 157;
pub const SYS_FILE_RENAME: usize = 154;
pub const SYS_FILE_ACCESS: usize = 153;

// POSIX time (160-163)
pub const SYS_CLOCK_GETTIME: usize = 160;
pub const SYS_CLOCK_GETRES: usize = 161;
pub const SYS_NANOSLEEP: usize = 162;
pub const SYS_GETTIMEOFDAY: usize = 163;

// Identity (170-175)
pub const SYS_GETUID: usize = 170;
pub const SYS_GETEUID: usize = 171;
pub const SYS_GETGID: usize = 172;
pub const SYS_GETEGID: usize = 173;

// Futex + arch_prctl (201-203)
pub const SYS_FUTEX_WAIT: usize = 201;
pub const SYS_FUTEX_WAKE: usize = 202;
pub const SYS_ARCH_PRCTL: usize = 203;

// ============================================================================
// Error Handling
// ============================================================================

/// Convert a raw syscall return value to a Result.
///
/// VeridianOS syscalls return >= 0 on success and negative error codes
/// on failure (matching the `SyscallError` repr(i32) in the kernel).
#[inline]
pub fn syscall_result(ret: isize) -> Result<usize, SyscallError> {
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(SyscallError::from_raw(ret as i32))
    }
}

/// Syscall error codes matching kernel/src/syscall/mod.rs SyscallError.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SyscallError {
    InvalidSyscall = -1,
    InvalidArgument = -2,
    PermissionDenied = -3,
    ResourceNotFound = -4,
    OutOfMemory = -5,
    WouldBlock = -6,
    Interrupted = -7,
    InvalidState = -8,
    InvalidPointer = -9,
    InvalidCapability = -10,
    AccessDenied = -18,
    ProcessNotFound = -19,
    Unknown = -128,
}

impl SyscallError {
    /// Convert a raw negative return value to a SyscallError.
    pub fn from_raw(code: i32) -> Self {
        match code {
            -1 => SyscallError::InvalidSyscall,
            -2 => SyscallError::InvalidArgument,
            -3 => SyscallError::PermissionDenied,
            -4 => SyscallError::ResourceNotFound,
            -5 => SyscallError::OutOfMemory,
            -6 => SyscallError::WouldBlock,
            -7 => SyscallError::Interrupted,
            -8 => SyscallError::InvalidState,
            -9 => SyscallError::InvalidPointer,
            -10 => SyscallError::InvalidCapability,
            -18 => SyscallError::AccessDenied,
            -19 => SyscallError::ProcessNotFound,
            _ => SyscallError::Unknown,
        }
    }
}
