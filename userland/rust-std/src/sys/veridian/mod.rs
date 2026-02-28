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
//! - **x86_64**: `syscall` instruction, nr in `rax`, args in
//!   `rdi/rsi/rdx/r10/r8/r9`
//! - **aarch64**: `svc #0`, nr in `x8`, args in `x0-x5`
//! - **riscv64**: `ecall`, nr in `a7`, args in `a0-a5`

pub mod alloc;
pub mod fd;
pub mod fs;
pub mod io;
pub mod locks;
pub mod net;
pub mod os;
pub mod path;
pub mod process;
pub mod target_spec;
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
pub unsafe fn syscall5(nr: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> isize {
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

// Scatter/gather I/O (183-184)
pub const SYS_READV: usize = 183;
pub const SYS_WRITEV: usize = 184;

// Self-hosting filesystem ops (185-200)
pub const SYS_FILE_CHMOD: usize = 185;
pub const SYS_FILE_FCHMOD: usize = 186;
pub const SYS_PROCESS_UMASK: usize = 187;
pub const SYS_FILE_TRUNCATE_PATH: usize = 188;
pub const SYS_FILE_POLL: usize = 189;
pub const SYS_FILE_OPENAT: usize = 190;
pub const SYS_FILE_FSTATAT: usize = 191;
pub const SYS_FILE_UNLINKAT: usize = 192;
pub const SYS_FILE_MKDIRAT: usize = 193;
pub const SYS_FILE_RENAMEAT: usize = 194;
pub const SYS_FILE_PREAD: usize = 195;
pub const SYS_FILE_PWRITE: usize = 196;
pub const SYS_FILE_CHOWN: usize = 197;
pub const SYS_FILE_FCHOWN: usize = 198;
pub const SYS_FILE_MKNOD: usize = 199;
pub const SYS_FILE_SELECT: usize = 200;

// Futex + arch_prctl (201-203)
pub const SYS_FUTEX_WAIT: usize = 201;
pub const SYS_FUTEX_WAKE: usize = 202;
pub const SYS_ARCH_PRCTL: usize = 203;

// System information (204-205)
pub const SYS_PROCESS_UNAME: usize = 204;
pub const SYS_PROCESS_GETENV: usize = 205;

// POSIX shared memory (210-212)
pub const SYS_SHM_OPEN: usize = 210;
pub const SYS_SHM_UNLINK: usize = 211;
pub const SYS_SHM_TRUNCATE: usize = 212;

// Socket operations (220-228)
pub const SYS_SOCKET_CREATE: usize = 220;
pub const SYS_SOCKET_BIND: usize = 221;
pub const SYS_SOCKET_LISTEN: usize = 222;
pub const SYS_SOCKET_CONNECT: usize = 223;
pub const SYS_SOCKET_ACCEPT: usize = 224;
pub const SYS_SOCKET_SEND: usize = 225;
pub const SYS_SOCKET_RECV: usize = 226;
pub const SYS_SOCKET_CLOSE: usize = 227;
pub const SYS_SOCKET_PAIR: usize = 228;

// Network extensions (250-255)
pub const SYS_NET_SENDTO: usize = 250;
pub const SYS_NET_RECVFROM: usize = 251;
pub const SYS_NET_GETSOCKNAME: usize = 252;
pub const SYS_NET_GETPEERNAME: usize = 253;
pub const SYS_NET_SETSOCKOPT: usize = 254;
pub const SYS_NET_GETSOCKOPT: usize = 255;

// Extended filesystem (150-158) -- additional entries not yet declared
pub const SYS_FILE_LSTAT: usize = 151;
pub const SYS_FILE_READLINK: usize = 152;
pub const SYS_FILE_LINK: usize = 155;
pub const SYS_FILE_SYMLINK: usize = 156;
pub const SYS_FILE_FCNTL: usize = 158;

// File ioctl (112)
pub const SYS_FILE_IOCTL: usize = 112;

// Process group/session (176-180)
pub const SYS_SETPGID: usize = 176;
pub const SYS_GETPGID: usize = 177;
pub const SYS_GETPGRP: usize = 178;
pub const SYS_SETSID: usize = 179;
pub const SYS_GETSID: usize = 180;

// Filesystem management additions (73)
pub const SYS_FS_FSYNC: usize = 73;

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
///
/// Values are negative integers returned directly by syscall instructions.
/// They correspond to the `SyscallError` `repr(i32)` values in the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SyscallError {
    // --- Core errors (kernel/src/syscall/mod.rs) ---
    /// Invalid or unrecognised syscall number.
    InvalidSyscall = -1,
    /// One or more arguments are invalid (EINVAL).
    InvalidArgument = -2,
    /// Insufficient permissions (EPERM).
    PermissionDenied = -3,
    /// Requested resource does not exist (ENOENT).
    ResourceNotFound = -4,
    /// Out of memory (ENOMEM).
    OutOfMemory = -5,
    /// Operation would block (EAGAIN / EWOULDBLOCK).
    WouldBlock = -6,
    /// Operation interrupted by signal (EINTR).
    Interrupted = -7,
    /// Object is in an invalid state for this operation.
    InvalidState = -8,
    /// Pointer argument is invalid (EFAULT).
    InvalidPointer = -9,

    // --- Capability errors ---
    /// Capability token is invalid.
    InvalidCapability = -10,
    /// Capability has been revoked.
    CapabilityRevoked = -11,
    /// Insufficient rights on capability.
    InsufficientRights = -12,
    /// Capability not found.
    CapabilityNotFound = -13,
    /// Capability already exists.
    CapabilityAlreadyExists = -14,
    /// Invalid capability object.
    InvalidCapabilityObject = -15,
    /// Capability delegation denied.
    CapabilityDelegationDenied = -16,

    // --- Memory / access errors ---
    /// Address references unmapped memory.
    UnmappedMemory = -17,
    /// Access denied (EACCES).
    AccessDenied = -18,
    /// No such process (ESRCH).
    ProcessNotFound = -19,

    // --- Filesystem errors ---
    /// File already exists (EEXIST).
    FileExists = -20,
    /// Bad file descriptor (EBADF).
    BadFileDescriptor = -21,
    /// I/O error (EIO).
    IoError = -22,

    // --- Exec errors ---
    /// Argument list too long (E2BIG).
    ArgumentListTooLong = -24,

    // --- Path / directory errors ---
    /// Not a directory (ENOTDIR).
    NotADirectory = -28,
    /// Is a directory (EISDIR).
    IsADirectory = -29,
    /// Not a terminal / not a tty (ENOTTY).
    NotATerminal = -32,
    /// Not implemented / no such syscall (ENOSYS).
    NotImplemented = -38,
    /// Broken pipe (EPIPE).
    BrokenPipe = -39,
    /// Too many symbolic link levels (ELOOP).
    SymlinkLoop = -40,
    /// Directory not empty (ENOTEMPTY).
    DirectoryNotEmpty = -45,

    // --- Resource limits ---
    /// Resource limit exceeded (custom, maps to errno 79).
    ResourceLimitExceeded = -79,

    // --- Additional POSIX-compatible errors ---
    /// No space left on device (ENOSPC).
    NoSpace = -80,
    /// Device or resource busy (EBUSY).
    Busy = -81,
    /// Read-only file system (EROFS).
    ReadOnlyFs = -82,
    /// Too many open files (EMFILE).
    TooManyOpenFiles = -83,
    /// Too many open files in system (ENFILE).
    TooManyOpenFilesSystem = -84,
    /// File name too long (ENAMETOOLONG).
    NameTooLong = -85,
    /// No such device (ENODEV).
    NoDevice = -86,
    /// Cross-device link (EXDEV).
    CrossDevice = -87,
    /// File too large (EFBIG).
    FileTooLarge = -88,
    /// Invalid seek (ESPIPE).
    InvalidSeek = -89,
    /// Connection refused (ECONNREFUSED).
    ConnectionRefused = -90,
    /// Connection reset (ECONNRESET).
    ConnectionReset = -91,
    /// Connection aborted (ECONNABORTED).
    ConnectionAborted = -92,
    /// Network unreachable (ENETUNREACH).
    NetworkUnreachable = -93,
    /// Host unreachable (EHOSTUNREACH).
    HostUnreachable = -94,
    /// Address already in use (EADDRINUSE).
    AddressInUse = -95,
    /// Address not available (EADDRNOTAVAIL).
    AddressNotAvailable = -96,
    /// Already connected (EISCONN).
    AlreadyConnected = -97,
    /// Not connected (ENOTCONN).
    NotConnected = -98,
    /// Operation timed out (ETIMEDOUT).
    TimedOut = -99,
    /// Operation already in progress (EALREADY).
    AlreadyInProgress = -100,
    /// Operation now in progress (EINPROGRESS).
    InProgress = -101,

    /// Unknown / unmapped error code.
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
            -11 => SyscallError::CapabilityRevoked,
            -12 => SyscallError::InsufficientRights,
            -13 => SyscallError::CapabilityNotFound,
            -14 => SyscallError::CapabilityAlreadyExists,
            -15 => SyscallError::InvalidCapabilityObject,
            -16 => SyscallError::CapabilityDelegationDenied,
            -17 => SyscallError::UnmappedMemory,
            -18 => SyscallError::AccessDenied,
            -19 => SyscallError::ProcessNotFound,
            -20 => SyscallError::FileExists,
            -21 => SyscallError::BadFileDescriptor,
            -22 => SyscallError::IoError,
            -24 => SyscallError::ArgumentListTooLong,
            -28 => SyscallError::NotADirectory,
            -29 => SyscallError::IsADirectory,
            -32 => SyscallError::NotATerminal,
            -38 => SyscallError::NotImplemented,
            -39 => SyscallError::BrokenPipe,
            -40 => SyscallError::SymlinkLoop,
            -45 => SyscallError::DirectoryNotEmpty,
            -79 => SyscallError::ResourceLimitExceeded,
            -80 => SyscallError::NoSpace,
            -81 => SyscallError::Busy,
            -82 => SyscallError::ReadOnlyFs,
            -83 => SyscallError::TooManyOpenFiles,
            -84 => SyscallError::TooManyOpenFilesSystem,
            -85 => SyscallError::NameTooLong,
            -86 => SyscallError::NoDevice,
            -87 => SyscallError::CrossDevice,
            -88 => SyscallError::FileTooLarge,
            -89 => SyscallError::InvalidSeek,
            -90 => SyscallError::ConnectionRefused,
            -91 => SyscallError::ConnectionReset,
            -92 => SyscallError::ConnectionAborted,
            -93 => SyscallError::NetworkUnreachable,
            -94 => SyscallError::HostUnreachable,
            -95 => SyscallError::AddressInUse,
            -96 => SyscallError::AddressNotAvailable,
            -97 => SyscallError::AlreadyConnected,
            -98 => SyscallError::NotConnected,
            -99 => SyscallError::TimedOut,
            -100 => SyscallError::AlreadyInProgress,
            -101 => SyscallError::InProgress,
            _ => SyscallError::Unknown,
        }
    }

    /// Return the raw integer error code.
    #[inline]
    pub fn raw_code(self) -> i32 {
        self as i32
    }

    /// A short human-readable description of the error.
    pub fn as_str(self) -> &'static str {
        match self {
            SyscallError::InvalidSyscall => "invalid syscall",
            SyscallError::InvalidArgument => "invalid argument",
            SyscallError::PermissionDenied => "permission denied",
            SyscallError::ResourceNotFound => "no such file or directory",
            SyscallError::OutOfMemory => "out of memory",
            SyscallError::WouldBlock => "resource temporarily unavailable",
            SyscallError::Interrupted => "interrupted system call",
            SyscallError::InvalidState => "invalid state",
            SyscallError::InvalidPointer => "bad address",
            SyscallError::InvalidCapability => "invalid capability",
            SyscallError::CapabilityRevoked => "capability revoked",
            SyscallError::InsufficientRights => "insufficient rights",
            SyscallError::CapabilityNotFound => "capability not found",
            SyscallError::CapabilityAlreadyExists => "capability already exists",
            SyscallError::InvalidCapabilityObject => "invalid capability object",
            SyscallError::CapabilityDelegationDenied => "capability delegation denied",
            SyscallError::UnmappedMemory => "unmapped memory",
            SyscallError::AccessDenied => "access denied",
            SyscallError::ProcessNotFound => "no such process",
            SyscallError::FileExists => "file exists",
            SyscallError::BadFileDescriptor => "bad file descriptor",
            SyscallError::IoError => "input/output error",
            SyscallError::ArgumentListTooLong => "argument list too long",
            SyscallError::NotADirectory => "not a directory",
            SyscallError::IsADirectory => "is a directory",
            SyscallError::NotATerminal => "not a terminal",
            SyscallError::NotImplemented => "function not implemented",
            SyscallError::BrokenPipe => "broken pipe",
            SyscallError::SymlinkLoop => "too many levels of symbolic links",
            SyscallError::DirectoryNotEmpty => "directory not empty",
            SyscallError::ResourceLimitExceeded => "resource limit exceeded",
            SyscallError::NoSpace => "no space left on device",
            SyscallError::Busy => "device or resource busy",
            SyscallError::ReadOnlyFs => "read-only file system",
            SyscallError::TooManyOpenFiles => "too many open files",
            SyscallError::TooManyOpenFilesSystem => "too many open files in system",
            SyscallError::NameTooLong => "file name too long",
            SyscallError::NoDevice => "no such device",
            SyscallError::CrossDevice => "cross-device link",
            SyscallError::FileTooLarge => "file too large",
            SyscallError::InvalidSeek => "illegal seek",
            SyscallError::ConnectionRefused => "connection refused",
            SyscallError::ConnectionReset => "connection reset by peer",
            SyscallError::ConnectionAborted => "connection aborted",
            SyscallError::NetworkUnreachable => "network is unreachable",
            SyscallError::HostUnreachable => "host is unreachable",
            SyscallError::AddressInUse => "address already in use",
            SyscallError::AddressNotAvailable => "address not available",
            SyscallError::AlreadyConnected => "transport endpoint is already connected",
            SyscallError::NotConnected => "transport endpoint is not connected",
            SyscallError::TimedOut => "connection timed out",
            SyscallError::AlreadyInProgress => "operation already in progress",
            SyscallError::InProgress => "operation now in progress",
            SyscallError::Unknown => "unknown error",
        }
    }
}

impl core::fmt::Display for SyscallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
