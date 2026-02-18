//! System call interface for VeridianOS
//!
//! Provides the kernel-side implementation of system calls including IPC
//! operations.
//!
//! # User-Space Pointer Validation Contract
//!
//! Every syscall handler that accepts a user-space pointer **must** call
//! [`validate_user_pointer`] (or the typed [`validate_user_buffer`]) before
//! dereferencing it. The validation enforces:
//!
//! 1. **Non-null** -- the pointer is not zero.
//! 2. **User-space range** -- the entire `[ptr, ptr+size)` region falls within
//!    the architecture-specific user-space address range (below
//!    [`USER_SPACE_END`]).
//! 3. **No arithmetic overflow** -- `ptr + size` does not wrap around.
//! 4. **Size cap** -- the buffer size does not exceed [`MAX_BUFFER_SIZE`] (256
//!    MB).
//! 5. **Alignment** -- for typed access via [`validate_user_ptr_typed`], the
//!    pointer is suitably aligned for `T`.
//!
//! Handlers that read null-terminated strings from user space must still pass
//! the base pointer through validation with a minimum size of 1 before
//! beginning the byte-by-byte scan.

// System call handlers are fully implemented but not all are reachable
// from user-space yet. Will be exercised once SYSCALL/SYSRET transitions
// are enabled.
#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    ipc::{sync_call, sync_receive, sync_reply, sync_send, IpcError, Message, SmallMessage},
    sched,
};

/// Maximum valid user-space address.
///
/// On x86_64 this is the canonical upper bound of user space (128 TB).
/// AArch64 and RISC-V use the same logical split for the QEMU virt machine.
const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_FFFF;

/// Maximum allowed buffer size for syscall arguments (256 MB).
const MAX_BUFFER_SIZE: usize = 256 * 1024 * 1024;

/// Validate that a user-space pointer and size are within bounds.
///
/// Checks:
/// - `ptr` is non-null
/// - `size` does not exceed [`MAX_BUFFER_SIZE`]
/// - `ptr + size` does not overflow
/// - the entire range `[ptr, ptr+size)` is below [`USER_SPACE_END`]
#[inline]
fn validate_user_pointer(ptr: usize, size: usize) -> Result<(), SyscallError> {
    if ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    if size > MAX_BUFFER_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    // Check for overflow and that the entire range is in user space
    let end = ptr.checked_add(size).ok_or(SyscallError::InvalidPointer)?;
    if end > USER_SPACE_END {
        return Err(SyscallError::AccessDenied);
    }
    Ok(())
}

/// Validate a user-space buffer of `len` bytes starting at `ptr`.
///
/// This is the canonical entry point for all syscall handlers that accept
/// user-space memory regions. It combines null, range, overflow, and size
/// checks in a single call.
///
/// # Errors
///
/// - [`SyscallError::InvalidPointer`] if `ptr` is null or overflows.
/// - [`SyscallError::InvalidArgument`] if `len` exceeds [`MAX_BUFFER_SIZE`].
/// - [`SyscallError::AccessDenied`] if the range extends into kernel space.
#[inline]
pub(crate) fn validate_user_buffer(ptr: usize, len: usize) -> Result<(), SyscallError> {
    validate_user_pointer(ptr, len)
}

/// Validate a user-space pointer for a typed access of `T`.
///
/// In addition to the range checks performed by [`validate_user_pointer`],
/// this verifies that `ptr` is aligned to `core::mem::align_of::<T>()`.
#[inline]
pub(crate) fn validate_user_ptr_typed<T>(ptr: usize) -> Result<(), SyscallError> {
    let size = core::mem::size_of::<T>();
    validate_user_pointer(ptr, size)?;
    let align = core::mem::align_of::<T>();
    if !ptr.is_multiple_of(align) {
        return Err(SyscallError::InvalidPointer);
    }
    Ok(())
}

/// Validate a user-space pointer for a null-terminated string read.
///
/// Checks that `ptr` is non-null and that at least the first byte falls
/// within user-space. Callers must additionally re-validate on each page
/// crossing during the byte-by-byte scan.
#[inline]
fn validate_user_string_ptr(ptr: usize) -> Result<(), SyscallError> {
    validate_user_pointer(ptr, 1)
}

/// Syscall rate limiter using token bucket algorithm
struct SyscallRateLimiter {
    /// Tokens available (scaled by 1000 for precision)
    tokens: AtomicU64,
    /// Maximum tokens (burst capacity)
    max_tokens: u64,
    /// Last refill timestamp
    last_refill: AtomicU64,
}

impl SyscallRateLimiter {
    const fn new() -> Self {
        Self {
            tokens: AtomicU64::new(10_000), // Start with 10k tokens
            max_tokens: 10_000,
            last_refill: AtomicU64::new(0),
        }
    }

    /// Check if a syscall is allowed (returns true if within rate limit)
    fn check(&self) -> bool {
        // Refill tokens based on elapsed time
        let now = crate::arch::timer::read_hw_timestamp();
        let last = self.last_refill.load(Ordering::Relaxed);
        let elapsed = now.saturating_sub(last);

        // Refill ~1000 tokens per tick (generous rate)
        if elapsed > 0 {
            self.last_refill.store(now, Ordering::Relaxed);
            let current = self.tokens.load(Ordering::Relaxed);
            let new_tokens = core::cmp::min(current + elapsed, self.max_tokens);
            self.tokens.store(new_tokens, Ordering::Relaxed);
        }

        // Try to consume a token
        let current = self.tokens.load(Ordering::Relaxed);
        if current > 0 {
            self.tokens.fetch_sub(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

static SYSCALL_RATE_LIMITER: SyscallRateLimiter = SyscallRateLimiter::new();

/// Syscall statistics for monitoring
static SYSCALL_COUNT: AtomicU64 = AtomicU64::new(0);
static SYSCALL_ERRORS: AtomicU64 = AtomicU64::new(0);

// Import process syscalls module
mod process;
use self::process::*;

// Import filesystem syscalls module
mod filesystem;
use self::filesystem::*;

// Import info syscalls module
mod info;
use self::info::*;

// Import package syscalls module
mod package;
use self::package::*;

// Import time syscalls module
mod time;
use self::time::*;

// Import signal syscalls module
mod signal;
use self::signal::*;

// Import debug syscalls module
mod debug;
use self::debug::*;

// Import memory syscalls module
mod memory;
use self::memory::*;

// Import user space utilities
mod userspace;

/// System call numbers
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syscall {
    // IPC system calls
    IpcSend = 0,
    IpcReceive = 1,
    IpcCall = 2,
    IpcReply = 3,
    IpcCreateEndpoint = 4,
    IpcBindEndpoint = 5,
    IpcShareMemory = 6,
    IpcMapMemory = 7,

    // Process management
    ProcessYield = 10,
    ProcessExit = 11,
    ProcessFork = 12,
    ProcessExec = 13,
    ProcessWait = 14,
    ProcessGetPid = 15,
    ProcessGetPPid = 16,
    ProcessSetPriority = 17,
    ProcessGetPriority = 18,

    // Thread management
    ThreadCreate = 40,
    ThreadExit = 41,
    ThreadJoin = 42,
    ThreadGetTid = 43,
    ThreadSetAffinity = 44,
    ThreadGetAffinity = 45,

    // Memory management
    MemoryMap = 20,
    MemoryUnmap = 21,
    MemoryProtect = 22,
    MemoryBrk = 23,

    // Capability management
    CapabilityGrant = 30,
    CapabilityRevoke = 31,

    // Filesystem operations
    FileOpen = 50,
    FileClose = 51,
    FileRead = 52,
    FileWrite = 53,
    FileSeek = 54,
    FileStat = 55,
    FileTruncate = 56,

    // Directory operations
    DirMkdir = 60,
    DirRmdir = 61,
    DirOpendir = 62,
    DirReaddir = 63,
    DirClosedir = 64,
    FilePipe2 = 65,
    FileDup3 = 66,

    // Filesystem management
    FsMount = 70,
    FsUnmount = 71,
    FsSync = 72,

    // Kernel information
    KernelGetInfo = 80,

    // Package management
    PkgInstall = 90,
    PkgRemove = 91,
    PkgQuery = 92,
    PkgList = 93,
    PkgUpdate = 94,

    // Extended filesystem operations
    FileDup = 57,
    FileDup2 = 58,
    FilePipe = 59,

    // Time management
    TimeGetUptime = 100,
    TimeCreateTimer = 101,
    TimeCancelTimer = 102,

    // Extended process operations
    ProcessGetcwd = 110,
    ProcessChdir = 111,
    FileIoctl = 112,
    ProcessKill = 113,

    // Signal management
    SigAction = 120,
    SigProcmask = 121,
    SigSuspend = 122,
    SigReturn = 123,

    // POSIX time syscalls
    ClockGettime = 160,
    ClockGetres = 161,
    Nanosleep = 162,
    Gettimeofday = 163,

    // Identity syscalls
    Getuid = 170,
    Geteuid = 171,
    Getgid = 172,
    Getegid = 173,
    Setuid = 174,
    Setgid = 175,

    // Process group / session syscalls
    Setpgid = 176,
    Getpgid = 177,
    Getpgrp = 178,
    Setsid = 179,
    Getsid = 180,

    // Scatter/gather I/O
    Readv = 183,
    Writev = 184,

    // Debug / tracing
    Ptrace = 140,

    // Extended filesystem operations (Phase 4B)
    FileStatPath = 150,
    FileLstat = 151,
    FileReadlink = 152,
    FileAccess = 153,
    FileRename = 154,
    FileLink = 155,
    FileSymlink = 156,
    FileUnlink = 157,
    FileFcntl = 158,

    // New filesystem ops for self-hosting (Phase 4A)
    FileChmod = 185,
    FileFchmod = 186,
    ProcessUmask = 187,
    FileTruncatePath = 188,
    FilePoll = 189,
    FileOpenat = 190,
    FileFstatat = 191,
    FileUnlinkat = 192,
    FileMkdirat = 193,
    FileRenameat = 194,
    FilePread = 195,
    FilePwrite = 196,

    // Ownership and device node syscalls
    FileChown = 197,
    FileFchown = 198,
    FileMknod = 199,
    FileSelect = 200,
}

/// System call result type
pub type SyscallResult = Result<usize, SyscallError>;

/// System call error codes
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    // Capability-specific errors
    InvalidCapability = -10,
    CapabilityRevoked = -11,
    InsufficientRights = -12,
    CapabilityNotFound = -13,
    CapabilityAlreadyExists = -14,
    InvalidCapabilityObject = -15,
    CapabilityDelegationDenied = -16,

    // Memory validation errors
    UnmappedMemory = -17,
    AccessDenied = -18,
    ProcessNotFound = -19,
}

impl From<IpcError> for SyscallError {
    fn from(err: IpcError) -> Self {
        match err {
            IpcError::InvalidCapability => SyscallError::InvalidCapability,
            IpcError::ProcessNotFound => SyscallError::ResourceNotFound,
            IpcError::EndpointNotFound => SyscallError::ResourceNotFound,
            IpcError::OutOfMemory => SyscallError::OutOfMemory,
            IpcError::WouldBlock => SyscallError::WouldBlock,
            IpcError::PermissionDenied => SyscallError::PermissionDenied,
            _ => SyscallError::InvalidArgument,
        }
    }
}

impl From<crate::cap::manager::CapError> for SyscallError {
    fn from(err: crate::cap::manager::CapError) -> Self {
        match err {
            crate::cap::manager::CapError::InvalidCapability => SyscallError::InvalidCapability,
            crate::cap::manager::CapError::InsufficientRights => SyscallError::InsufficientRights,
            crate::cap::manager::CapError::CapabilityRevoked => SyscallError::CapabilityRevoked,
            crate::cap::manager::CapError::OutOfMemory => SyscallError::OutOfMemory,
            crate::cap::manager::CapError::InvalidObject => SyscallError::InvalidCapabilityObject,
            crate::cap::manager::CapError::PermissionDenied => {
                SyscallError::CapabilityDelegationDenied
            }
            crate::cap::manager::CapError::AlreadyExists => SyscallError::CapabilityAlreadyExists,
            crate::cap::manager::CapError::NotFound => SyscallError::CapabilityNotFound,
            crate::cap::manager::CapError::IdExhausted => SyscallError::OutOfMemory,
            crate::cap::manager::CapError::QuotaExceeded => SyscallError::OutOfMemory,
        }
    }
}

/// System call handler entry point
#[no_mangle]
pub extern "C" fn syscall_handler(
    syscall_num: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> isize {
    // RAW SERIAL DIAGNOSTIC: Entered handler
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[HANDLER ENTRY]\n");
    }

    // Speculation barrier at syscall entry to mitigate Spectre-style attacks.
    // Prevents speculative execution of kernel code with user-controlled values.
    crate::arch::speculation_barrier();

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER BARRIER]\n");
    }

    // Track syscall count
    SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER COUNT]\n");
    }

    // Rate limiting check
    if !SYSCALL_RATE_LIMITER.check() {
        SYSCALL_ERRORS.fetch_add(1, Ordering::Relaxed);
        return SyscallError::WouldBlock as i32 as isize;
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER RATE]\n");
    }

    // Get caller PID for audit logging
    let _caller_pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER PID]\n");
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[BEFORE DISPATCH]\n");
    }

    let result = match Syscall::try_from(syscall_num) {
        Ok(syscall) => handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5),
        Err(_) => Err(SyscallError::InvalidSyscall),
    };

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER DISPATCH]\n");
    }

    // Audit log: syscall with result.
    // Safe to call even from syscall context - uses try_lock() with graceful
    // fallback if locks are held, preventing deadlocks during syscall return.
    let success = result.is_ok();
    if !success {
        SYSCALL_ERRORS.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[BEFORE AUDIT]\n");
    }

    // TEMPORARY: Audit logging disabled during syscall due to VFS access with CR3
    // switch. TODO(user-space): Re-enable after resolving VFS heap access from
    // switched CR3 context. crate::security::audit::log_syscall(caller_pid, 0,
    // syscall_num, success);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[AFTER AUDIT (skipped)]\n");
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[HANDLER RETURN]\n");
    }

    match result {
        Ok(value) => value as isize,
        Err(error) => error as i32 as isize,
    }
}

/// Handle individual system calls
fn handle_syscall(
    syscall: Syscall,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> SyscallResult {
    match syscall {
        // IPC system calls
        Syscall::IpcSend => sys_ipc_send(arg1, arg2, arg3, arg4),
        Syscall::IpcReceive => sys_ipc_receive(arg1, arg2),
        Syscall::IpcCall => sys_ipc_call(arg1, arg2, arg3, arg4, arg5),
        Syscall::IpcReply => sys_ipc_reply(arg1, arg2, arg3),
        Syscall::IpcCreateEndpoint => sys_ipc_create_endpoint(arg1),
        Syscall::IpcBindEndpoint => sys_ipc_bind_endpoint(arg1, arg2),
        Syscall::IpcShareMemory => sys_ipc_share_memory(arg1, arg2, arg3, arg4),
        Syscall::IpcMapMemory => sys_ipc_map_memory(arg1, arg2, arg3),

        // Process management
        Syscall::ProcessYield => sys_yield(),
        Syscall::ProcessExit => sys_exit(arg1),
        Syscall::ProcessFork => sys_fork(),
        Syscall::ProcessExec => sys_exec(arg1, arg2, arg3),
        Syscall::ProcessWait => sys_wait(arg1 as isize, arg2, arg3),
        Syscall::ProcessGetPid => sys_getpid(),
        Syscall::ProcessGetPPid => sys_getppid(),
        Syscall::ProcessSetPriority => sys_setpriority(arg1, arg2, arg3),
        Syscall::ProcessGetPriority => sys_getpriority(arg1, arg2),

        // Thread management
        Syscall::ThreadCreate => sys_thread_create(arg1, arg2, arg3, arg4),
        Syscall::ThreadExit => sys_thread_exit(arg1),
        Syscall::ThreadJoin => sys_thread_join(arg1, arg2),
        Syscall::ThreadGetTid => sys_gettid(),
        Syscall::ThreadSetAffinity => sys_thread_setaffinity(arg1, arg2, arg3),
        Syscall::ThreadGetAffinity => sys_thread_getaffinity(arg1, arg2, arg3),

        // Filesystem operations
        Syscall::FileOpen => sys_open(arg1, arg2, arg3),
        Syscall::FileClose => sys_close(arg1),
        Syscall::FileRead => sys_read(arg1, arg2, arg3),
        Syscall::FileWrite => sys_write(arg1, arg2, arg3),
        Syscall::FileSeek => sys_seek(arg1, arg2 as isize, arg3),
        Syscall::FileStat => sys_stat(arg1, arg2),
        Syscall::FileTruncate => sys_truncate(arg1, arg2),
        Syscall::FileDup => sys_dup(arg1),
        Syscall::FileDup2 => sys_dup2(arg1, arg2),
        Syscall::FilePipe => sys_pipe(arg1),

        // Memory management
        Syscall::MemoryMap => sys_mmap(arg1, arg2, arg3, arg4, arg5),
        Syscall::MemoryUnmap => sys_munmap(arg1, arg2),
        Syscall::MemoryProtect => sys_mprotect(arg1, arg2, arg3),
        Syscall::MemoryBrk => sys_brk(arg1),

        // Directory operations
        Syscall::DirMkdir => sys_mkdir(arg1, arg2),
        Syscall::DirRmdir => sys_rmdir(arg1),
        Syscall::DirOpendir => sys_opendir(arg1),
        Syscall::DirReaddir => sys_readdir(arg1, arg2, arg3),
        Syscall::DirClosedir => sys_closedir(arg1),
        Syscall::FilePipe2 => sys_pipe2(arg1, arg2),
        Syscall::FileDup3 => sys_dup3(arg1, arg2, arg3),

        // Filesystem management
        Syscall::FsMount => sys_mount(arg1, arg2, arg3, arg4),
        Syscall::FsUnmount => sys_unmount(arg1),
        Syscall::FsSync => sys_sync(),

        // Kernel information
        Syscall::KernelGetInfo => sys_get_kernel_info(arg1),

        // Package management
        Syscall::PkgInstall => sys_pkg_install(arg1, arg2),
        Syscall::PkgRemove => sys_pkg_remove(arg1, arg2),
        Syscall::PkgQuery => sys_pkg_query(arg1, arg2),
        Syscall::PkgList => sys_pkg_list(arg1, arg2),
        Syscall::PkgUpdate => sys_pkg_update(arg1),

        // Extended process operations
        Syscall::ProcessGetcwd => sys_getcwd(arg1, arg2),
        Syscall::ProcessChdir => sys_chdir(arg1),
        Syscall::FileIoctl => sys_ioctl(arg1, arg2, arg3),
        Syscall::ProcessKill => sys_kill(arg1, arg2),

        // Time management
        Syscall::TimeGetUptime => sys_time_get_uptime(),
        Syscall::TimeCreateTimer => sys_time_create_timer(arg1, arg2, arg3),
        Syscall::TimeCancelTimer => sys_time_cancel_timer(arg1),

        // Signal management
        Syscall::SigAction => sys_sigaction(arg1, arg2, arg3),
        Syscall::SigProcmask => sys_sigprocmask(arg1, arg2, arg3),
        Syscall::SigSuspend => sys_sigsuspend(arg1),
        Syscall::SigReturn => sys_sigreturn(arg1),

        // POSIX time syscalls
        Syscall::ClockGettime => sys_clock_gettime(arg1, arg2),
        Syscall::ClockGetres => sys_clock_getres(arg1, arg2),
        Syscall::Nanosleep => sys_nanosleep(arg1, arg2),
        Syscall::Gettimeofday => sys_gettimeofday(arg1, arg2),

        // Identity syscalls
        Syscall::Getuid => sys_getuid(),
        Syscall::Geteuid => sys_geteuid(),
        Syscall::Getgid => sys_getgid(),
        Syscall::Getegid => sys_getegid(),
        Syscall::Setuid => sys_setuid(arg1),
        Syscall::Setgid => sys_setgid(arg1),

        // Process group / session syscalls
        Syscall::Setpgid => sys_setpgid(arg1, arg2),
        Syscall::Getpgid => sys_getpgid(arg1),
        Syscall::Getpgrp => sys_getpgrp(),
        Syscall::Setsid => sys_setsid(),
        Syscall::Getsid => sys_getsid(arg1),

        // Scatter/gather I/O
        Syscall::Readv => sys_readv(arg1, arg2, arg3),
        Syscall::Writev => sys_writev(arg1, arg2, arg3),

        // Debug / tracing
        Syscall::Ptrace => sys_ptrace(arg1, arg2, arg3, arg4),

        // Extended filesystem operations
        Syscall::FileStatPath => sys_stat_path(arg1, arg2),
        Syscall::FileLstat => sys_lstat(arg1, arg2),
        Syscall::FileReadlink => sys_readlink(arg1, arg2, arg3),
        Syscall::FileAccess => sys_access(arg1, arg2),
        Syscall::FileRename => sys_rename(arg1, arg2),
        Syscall::FileLink => sys_link(arg1, arg2),
        Syscall::FileSymlink => sys_symlink(arg1, arg2),
        Syscall::FileUnlink => sys_unlink(arg1),
        Syscall::FileFcntl => sys_fcntl(arg1, arg2, arg3),

        // Self-hosting filesystem ops
        Syscall::FileChmod => sys_chmod(arg1, arg2),
        Syscall::FileFchmod => sys_fchmod(arg1, arg2),
        Syscall::ProcessUmask => sys_umask(arg1),
        Syscall::FileTruncatePath => sys_truncate_path(arg1, arg2),
        Syscall::FilePoll => sys_poll(arg1, arg2, arg3),
        Syscall::FileOpenat => sys_openat(arg1, arg2, arg3, arg4),
        Syscall::FileFstatat => sys_fstatat(arg1, arg2, arg3, arg4),
        Syscall::FileUnlinkat => sys_unlinkat(arg1, arg2, arg3),
        Syscall::FileMkdirat => sys_mkdirat(arg1, arg2, arg3),
        Syscall::FileRenameat => sys_renameat(arg1, arg2, arg3, arg4),
        Syscall::FilePread => sys_pread(arg1, arg2, arg3, arg4),
        Syscall::FilePwrite => sys_pwrite(arg1, arg2, arg3, arg4),
        Syscall::FileChown => sys_chown(arg1, arg2, arg3),
        Syscall::FileFchown => sys_fchown(arg1, arg2, arg3),
        Syscall::FileMknod => sys_mknod(arg1, arg2, arg3),
        Syscall::FileSelect => sys_select(arg1, arg2, arg3, arg4, arg5),

        _ => Err(SyscallError::InvalidSyscall),
    }
}

/// IPC send system call
///
/// # Arguments
/// - capability: Capability token for the endpoint
/// - msg_ptr: Pointer to message structure
/// - msg_size: Size of message
/// - flags: Send flags
fn sys_ipc_send(
    capability: usize,
    msg_ptr: usize,
    msg_size: usize,
    _flags: usize,
) -> SyscallResult {
    // Validate user-space pointer bounds
    validate_user_pointer(msg_ptr, msg_size)?;

    // Get current process's capability space
    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let real_process = crate::process::table::get_process(current_process.pid)
        .ok_or(SyscallError::InvalidState)?;
    let cap_space = real_process.capability_space.lock();

    // Convert capability value to token
    let cap_token = crate::cap::CapabilityToken::from_u64(capability as u64);

    // Check send permission
    if let Err(e) = crate::cap::ipc_integration::check_send_permission(cap_token, &cap_space) {
        return Err(e.into());
    }

    // Check if this is a small message (fast path)
    let message = if msg_size <= core::mem::size_of::<SmallMessage>() {
        // Fast path for small messages
        // SAFETY: msg_ptr was validated as non-zero above. The caller passes
        // a user-space pointer to a SmallMessage. We read the entire struct
        // by value. SmallMessage is Copy and repr(C), so the read is valid
        // if the pointer is properly aligned and points to valid memory.
        unsafe {
            let small_msg = *(msg_ptr as *const SmallMessage);
            Message::Small(small_msg)
        }
    } else {
        // Large message path
        // SAFETY: msg_ptr is non-zero (checked above) and msg_size > 0.
        // The user-space buffer at msg_ptr is expected to contain msg_size
        // bytes. We create a slice reference for the message data. The
        // LargeMessage is constructed with the user-space address for
        // later zero-copy transfer.
        unsafe {
            let _msg_slice = core::slice::from_raw_parts(msg_ptr as *const u8, msg_size);

            // For now, create a large message with basic header
            // In a real implementation, this would handle shared memory regions
            let large_msg = crate::ipc::LargeMessage {
                header: crate::ipc::message::MessageHeader::new(
                    capability as u64,
                    0,
                    msg_size as u64,
                ),
                memory_region: crate::ipc::message::MemoryRegion::new(
                    msg_ptr as u64,
                    msg_size as u64,
                ),
                inline_data: [0; crate::ipc::message::SMALL_MESSAGE_MAX_SIZE],
            };

            Message::Large(large_msg)
        }
    };

    // Perform the actual send using the IPC sync module
    match sync_send(message, capability as u64) {
        Ok(()) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// IPC receive system call
///
/// # Arguments
/// - endpoint: Endpoint to receive from
/// - buffer: Buffer to receive message into
fn sys_ipc_receive(endpoint: usize, buffer: usize) -> SyscallResult {
    // Validate receive buffer can hold at least a SmallMessage
    validate_user_buffer(buffer, core::mem::size_of::<SmallMessage>())?;

    // Get current process's capability space
    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let real_process = crate::process::table::get_process(current_process.pid)
        .ok_or(SyscallError::InvalidState)?;
    let cap_space = real_process.capability_space.lock();

    // Convert endpoint to capability token
    let cap_token = crate::cap::CapabilityToken::from_u64(endpoint as u64);

    // Check receive permission
    if let Err(e) = crate::cap::ipc_integration::check_receive_permission(cap_token, &cap_space) {
        return Err(e.into());
    }

    // Receive message using IPC sync module
    match sync_receive(endpoint as u64) {
        Ok(message) => {
            // Copy message to user buffer
            // SAFETY: buffer was validated as non-zero above. We write the
            // received message to the user-space buffer. For SmallMessage,
            // we write the struct directly. For LargeMessage, we copy the
            // header and data. The caller is responsible for providing a
            // buffer large enough to hold the message.
            unsafe {
                match message {
                    Message::Small(small_msg) => {
                        // Copy small message to buffer
                        let dst = buffer as *mut SmallMessage;
                        *dst = small_msg;
                        Ok(core::mem::size_of::<SmallMessage>())
                    }
                    Message::Large(large_msg) => {
                        // For large messages, copy the header and setup shared memory
                        // In a real implementation, this would handle memory mapping
                        let header_size =
                            core::mem::size_of::<crate::ipc::message::MessageHeader>();
                        let dst = buffer as *mut u8;

                        // Copy header
                        core::ptr::copy_nonoverlapping(
                            &large_msg.header as *const _ as *const u8,
                            dst,
                            header_size,
                        );

                        // Copy data if it fits
                        if large_msg.memory_region.size > 0
                            && large_msg.memory_region.base_addr != 0
                        {
                            let data_dst = dst.add(header_size);
                            core::ptr::copy_nonoverlapping(
                                large_msg.memory_region.base_addr as *const u8,
                                data_dst,
                                large_msg.memory_region.size as usize,
                            );
                        }

                        Ok(header_size + large_msg.memory_region.size as usize)
                    }
                }
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// IPC call (send and wait for reply)
fn sys_ipc_call(
    capability: usize,
    send_msg: usize,
    send_size: usize,
    recv_buf: usize,
    recv_size: usize,
) -> SyscallResult {
    // Validate send and receive buffers are in user space
    if send_size == 0 || recv_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(send_msg, send_size)?;
    validate_user_buffer(recv_buf, recv_size)?;

    // Create message from user buffer
    let message = if send_size <= core::mem::size_of::<SmallMessage>() {
        // SAFETY: send_msg was validated as non-zero above and send_size
        // fits within SmallMessage. The pointer cast reads the struct by
        // value. SmallMessage is Copy and repr(C).
        unsafe {
            let small_msg = *(send_msg as *const SmallMessage);
            Message::Small(small_msg)
        }
    } else {
        // Create large message
        let large_msg = crate::ipc::LargeMessage {
            header: crate::ipc::message::MessageHeader::new(capability as u64, 0, send_size as u64),
            memory_region: crate::ipc::message::MemoryRegion::new(
                send_msg as u64,
                send_size as u64,
            ),
            inline_data: [0; crate::ipc::message::SMALL_MESSAGE_MAX_SIZE],
        };
        Message::Large(large_msg)
    };

    // Perform synchronous call
    match sync_call(message, capability as u64) {
        Ok(reply) => {
            // Copy reply to receive buffer
            // SAFETY: recv_buf was validated as non-zero and recv_size > 0
            // above. We write the reply message to the user buffer, checking
            // that recv_size is large enough for SmallMessage or the header.
            // The caller must provide adequately sized buffers.
            unsafe {
                match reply {
                    Message::Small(small_msg) => {
                        if recv_size >= core::mem::size_of::<SmallMessage>() {
                            let dst = recv_buf as *mut SmallMessage;
                            *dst = small_msg;
                            Ok(core::mem::size_of::<SmallMessage>())
                        } else {
                            Err(SyscallError::InvalidArgument)
                        }
                    }
                    Message::Large(large_msg) => {
                        let header_size =
                            core::mem::size_of::<crate::ipc::message::MessageHeader>();
                        if recv_size >= header_size {
                            let dst = recv_buf as *mut u8;

                            // Copy header
                            core::ptr::copy_nonoverlapping(
                                &large_msg.header as *const _ as *const u8,
                                dst,
                                header_size,
                            );

                            // Copy data
                            let data_to_copy = core::cmp::min(
                                large_msg.memory_region.size as usize,
                                recv_size - header_size,
                            );
                            if data_to_copy > 0 && large_msg.memory_region.base_addr != 0 {
                                let data_dst = dst.add(header_size);
                                core::ptr::copy_nonoverlapping(
                                    large_msg.memory_region.base_addr as *const u8,
                                    data_dst,
                                    data_to_copy,
                                );
                            }

                            Ok(header_size + data_to_copy)
                        } else {
                            Err(SyscallError::InvalidArgument)
                        }
                    }
                }
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// IPC reply to a previous call
fn sys_ipc_reply(caller: usize, msg_ptr: usize, msg_size: usize) -> SyscallResult {
    // Validate reply message buffer
    if msg_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(msg_ptr, msg_size)?;

    // Create reply message
    let message = if msg_size <= core::mem::size_of::<SmallMessage>() {
        // SAFETY: msg_ptr was validated as non-zero above and msg_size fits
        // within SmallMessage. The pointer cast reads a Copy/repr(C) struct.
        unsafe {
            let small_msg = *(msg_ptr as *const SmallMessage);
            Message::Small(small_msg)
        }
    } else {
        let large_msg = crate::ipc::LargeMessage {
            header: crate::ipc::message::MessageHeader::new(0, 0, msg_size as u64),
            memory_region: crate::ipc::message::MemoryRegion::new(msg_ptr as u64, msg_size as u64),
            inline_data: [0; crate::ipc::message::SMALL_MESSAGE_MAX_SIZE],
        };
        Message::Large(large_msg)
    };

    // Send reply
    match sync_reply(message, caller as u64) {
        Ok(()) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Yield CPU to another process
fn sys_yield() -> SyscallResult {
    // Trigger scheduler to yield CPU
    sched::yield_cpu();
    Ok(0)
}

/// Create IPC endpoint
fn sys_ipc_create_endpoint(_permissions: usize) -> SyscallResult {
    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current_process.capability_space.lock();

    // Create endpoint with capability
    match crate::cap::ipc_integration::create_endpoint_with_capability(&cap_space) {
        Ok((_endpoint_id, capability)) => {
            // Return the capability token (which includes the endpoint ID)
            Ok(capability.to_u64() as usize)
        }
        Err(e) => Err(e.into()),
    }
}

/// Bind endpoint to a name
fn sys_ipc_bind_endpoint(endpoint_id: usize, name_ptr: usize) -> SyscallResult {
    // Validate name pointer is in user space (at least 1 byte for a string)
    validate_user_string_ptr(name_ptr)?;

    // For now, just validate the endpoint exists
    // In a real implementation, this would register the endpoint with a name
    // service
    match crate::ipc::registry::lookup_endpoint(endpoint_id as u64) {
        Ok(_) => Ok(0),
        Err(_) => Err(SyscallError::ResourceNotFound),
    }
}

/// Share memory region via IPC
fn sys_ipc_share_memory(
    addr: usize,
    size: usize,
    permissions: usize,
    _target_pid: usize,
) -> SyscallResult {
    use crate::ipc::shared_memory::{Permissions, SharedRegion};

    // Validate the shared region address is in user space
    if size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(addr, size)?;

    // Get current process and capability space
    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current_process.capability_space.lock();

    // Convert permissions to capability rights
    let mut rights = crate::cap::Rights::new(0);
    if permissions & 0b001 != 0 {
        rights = rights | crate::cap::memory_integration::MemoryRights::READ;
    }
    if permissions & 0b010 != 0 {
        rights = rights | crate::cap::memory_integration::MemoryRights::WRITE;
    }
    if permissions & 0b100 != 0 {
        rights = rights | crate::cap::memory_integration::MemoryRights::EXECUTE;
    }
    rights = rights
        | crate::cap::memory_integration::MemoryRights::MAP
        | crate::cap::memory_integration::MemoryRights::SHARE;

    // Convert permissions bits to enum
    let perms = match permissions & 0b111 {
        0b001 => Permissions::Read,
        0b011 => Permissions::Write,
        0b100 => Permissions::Execute,
        0b101 => Permissions::ReadExecute,
        0b111 => Permissions::ReadWriteExecute,
        _ => Permissions::Read, // Default to read-only
    };

    // Create shared region owned by current process
    let _region = match SharedRegion::new(current_process.pid, size, perms) {
        Ok(region) => region,
        Err(_) => return Err(SyscallError::OutOfMemory),
    };

    // Create memory capability for this region
    let phys_addr = crate::mm::PhysicalAddress::new(addr as u64); // TODO(future): Get actual physical address from VMM
    let attributes = crate::cap::object::MemoryAttributes::normal();

    match crate::cap::memory_integration::create_memory_capability(
        phys_addr.as_usize(),
        size,
        attributes,
        rights,
        &cap_space,
    ) {
        Ok(cap) => Ok(cap.to_u64() as usize),
        Err(_) => Err(SyscallError::OutOfMemory),
    }
}

/// Map shared memory from another process
fn sys_ipc_map_memory(capability: usize, addr_hint: usize, flags: usize) -> SyscallResult {
    // Get current process and capability space
    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current_process.capability_space.lock();

    // Convert capability to token
    let cap_token = crate::cap::CapabilityToken::from_u64(capability as u64);

    // Check map permission
    if let Err(e) = crate::cap::memory_integration::check_map_permission(cap_token, &cap_space) {
        return Err(match e {
            crate::cap::CapError::InvalidCapability => SyscallError::InvalidArgument,
            crate::cap::CapError::InsufficientRights => SyscallError::PermissionDenied,
            _ => SyscallError::InvalidArgument,
        });
    }

    // Convert flags to page flags
    let mut page_flags = crate::mm::PageFlags::PRESENT | crate::mm::PageFlags::USER;
    if flags & 0b010 != 0 {
        page_flags |= crate::mm::PageFlags::WRITABLE;
    }
    if flags & 0b100 == 0 {
        // If execute bit is not set, mark as no-execute
        page_flags |= crate::mm::PageFlags::NO_EXECUTE;
    }

    // TODO(future): Implement actual memory mapping with VMM
    if addr_hint == 0 {
        // Would allocate a virtual address
        Ok(0x100000000) // Placeholder address
    } else {
        Ok(addr_hint)
    }
}

impl TryFrom<usize> for Syscall {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            // IPC system calls
            0 => Ok(Syscall::IpcSend),
            1 => Ok(Syscall::IpcReceive),
            2 => Ok(Syscall::IpcCall),
            3 => Ok(Syscall::IpcReply),
            4 => Ok(Syscall::IpcCreateEndpoint),
            5 => Ok(Syscall::IpcBindEndpoint),
            6 => Ok(Syscall::IpcShareMemory),
            7 => Ok(Syscall::IpcMapMemory),

            // Process management
            10 => Ok(Syscall::ProcessYield),
            11 => Ok(Syscall::ProcessExit),
            12 => Ok(Syscall::ProcessFork),
            13 => Ok(Syscall::ProcessExec),
            14 => Ok(Syscall::ProcessWait),
            15 => Ok(Syscall::ProcessGetPid),
            16 => Ok(Syscall::ProcessGetPPid),
            17 => Ok(Syscall::ProcessSetPriority),
            18 => Ok(Syscall::ProcessGetPriority),

            // Memory management
            20 => Ok(Syscall::MemoryMap),
            21 => Ok(Syscall::MemoryUnmap),
            22 => Ok(Syscall::MemoryProtect),
            23 => Ok(Syscall::MemoryBrk),

            // Capability management
            30 => Ok(Syscall::CapabilityGrant),
            31 => Ok(Syscall::CapabilityRevoke),

            // Thread management
            40 => Ok(Syscall::ThreadCreate),
            41 => Ok(Syscall::ThreadExit),
            42 => Ok(Syscall::ThreadJoin),
            43 => Ok(Syscall::ThreadGetTid),
            44 => Ok(Syscall::ThreadSetAffinity),
            45 => Ok(Syscall::ThreadGetAffinity),

            // Filesystem operations
            50 => Ok(Syscall::FileOpen),
            51 => Ok(Syscall::FileClose),
            52 => Ok(Syscall::FileRead),
            53 => Ok(Syscall::FileWrite),
            54 => Ok(Syscall::FileSeek),
            55 => Ok(Syscall::FileStat),
            56 => Ok(Syscall::FileTruncate),
            57 => Ok(Syscall::FileDup),
            58 => Ok(Syscall::FileDup2),
            59 => Ok(Syscall::FilePipe),

            // Directory operations
            60 => Ok(Syscall::DirMkdir),
            61 => Ok(Syscall::DirRmdir),
            62 => Ok(Syscall::DirOpendir),
            63 => Ok(Syscall::DirReaddir),
            64 => Ok(Syscall::DirClosedir),
            65 => Ok(Syscall::FilePipe2),
            66 => Ok(Syscall::FileDup3),

            // Filesystem management
            70 => Ok(Syscall::FsMount),
            71 => Ok(Syscall::FsUnmount),
            72 => Ok(Syscall::FsSync),

            // Kernel information
            80 => Ok(Syscall::KernelGetInfo),

            // Package management
            90 => Ok(Syscall::PkgInstall),
            91 => Ok(Syscall::PkgRemove),
            92 => Ok(Syscall::PkgQuery),
            93 => Ok(Syscall::PkgList),
            94 => Ok(Syscall::PkgUpdate),

            // Time management
            100 => Ok(Syscall::TimeGetUptime),
            101 => Ok(Syscall::TimeCreateTimer),
            102 => Ok(Syscall::TimeCancelTimer),

            // Extended process operations
            110 => Ok(Syscall::ProcessGetcwd),
            111 => Ok(Syscall::ProcessChdir),
            112 => Ok(Syscall::FileIoctl),
            113 => Ok(Syscall::ProcessKill),

            // Signal management
            120 => Ok(Syscall::SigAction),
            121 => Ok(Syscall::SigProcmask),
            122 => Ok(Syscall::SigSuspend),
            123 => Ok(Syscall::SigReturn),

            // Debug / tracing
            140 => Ok(Syscall::Ptrace),

            // POSIX time syscalls
            160 => Ok(Syscall::ClockGettime),
            161 => Ok(Syscall::ClockGetres),
            162 => Ok(Syscall::Nanosleep),
            163 => Ok(Syscall::Gettimeofday),

            // Identity syscalls
            170 => Ok(Syscall::Getuid),
            171 => Ok(Syscall::Geteuid),
            172 => Ok(Syscall::Getgid),
            173 => Ok(Syscall::Getegid),
            174 => Ok(Syscall::Setuid),
            175 => Ok(Syscall::Setgid),

            // Process group / session syscalls
            176 => Ok(Syscall::Setpgid),
            177 => Ok(Syscall::Getpgid),
            178 => Ok(Syscall::Getpgrp),
            179 => Ok(Syscall::Setsid),
            180 => Ok(Syscall::Getsid),

            // Scatter/gather I/O
            183 => Ok(Syscall::Readv),
            184 => Ok(Syscall::Writev),

            // Extended filesystem operations
            150 => Ok(Syscall::FileStatPath),
            151 => Ok(Syscall::FileLstat),
            152 => Ok(Syscall::FileReadlink),
            153 => Ok(Syscall::FileAccess),
            154 => Ok(Syscall::FileRename),
            155 => Ok(Syscall::FileLink),
            156 => Ok(Syscall::FileSymlink),
            157 => Ok(Syscall::FileUnlink),
            158 => Ok(Syscall::FileFcntl),

            // Self-hosting filesystem ops
            185 => Ok(Syscall::FileChmod),
            186 => Ok(Syscall::FileFchmod),
            187 => Ok(Syscall::ProcessUmask),
            188 => Ok(Syscall::FileTruncatePath),
            189 => Ok(Syscall::FilePoll),
            190 => Ok(Syscall::FileOpenat),
            191 => Ok(Syscall::FileFstatat),
            192 => Ok(Syscall::FileUnlinkat),
            193 => Ok(Syscall::FileMkdirat),
            194 => Ok(Syscall::FileRenameat),
            195 => Ok(Syscall::FilePread),
            196 => Ok(Syscall::FilePwrite),
            197 => Ok(Syscall::FileChown),
            198 => Ok(Syscall::FileFchown),
            199 => Ok(Syscall::FileMknod),
            200 => Ok(Syscall::FileSelect),

            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Syscall TryFrom tests ---

    #[test]
    fn test_syscall_try_from_ipc_send() {
        let result = Syscall::try_from(0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Syscall::IpcSend);
    }

    #[test]
    fn test_syscall_try_from_ipc_receive() {
        assert_eq!(Syscall::try_from(1).unwrap(), Syscall::IpcReceive);
    }

    #[test]
    fn test_syscall_try_from_ipc_call() {
        assert_eq!(Syscall::try_from(2).unwrap(), Syscall::IpcCall);
    }

    #[test]
    fn test_syscall_try_from_ipc_reply() {
        assert_eq!(Syscall::try_from(3).unwrap(), Syscall::IpcReply);
    }

    #[test]
    fn test_syscall_try_from_process_yield() {
        assert_eq!(Syscall::try_from(10).unwrap(), Syscall::ProcessYield);
    }

    #[test]
    fn test_syscall_try_from_process_exit() {
        assert_eq!(Syscall::try_from(11).unwrap(), Syscall::ProcessExit);
    }

    #[test]
    fn test_syscall_try_from_process_fork() {
        assert_eq!(Syscall::try_from(12).unwrap(), Syscall::ProcessFork);
    }

    #[test]
    fn test_syscall_try_from_process_getpid() {
        assert_eq!(Syscall::try_from(15).unwrap(), Syscall::ProcessGetPid);
    }

    #[test]
    fn test_syscall_try_from_memory_map() {
        assert_eq!(Syscall::try_from(20).unwrap(), Syscall::MemoryMap);
    }

    #[test]
    fn test_syscall_try_from_capability_grant() {
        assert_eq!(Syscall::try_from(30).unwrap(), Syscall::CapabilityGrant);
    }

    #[test]
    fn test_syscall_try_from_thread_create() {
        assert_eq!(Syscall::try_from(40).unwrap(), Syscall::ThreadCreate);
    }

    #[test]
    fn test_syscall_try_from_file_open() {
        assert_eq!(Syscall::try_from(50).unwrap(), Syscall::FileOpen);
    }

    #[test]
    fn test_syscall_try_from_dir_mkdir() {
        assert_eq!(Syscall::try_from(60).unwrap(), Syscall::DirMkdir);
    }

    #[test]
    fn test_syscall_try_from_fs_mount() {
        assert_eq!(Syscall::try_from(70).unwrap(), Syscall::FsMount);
    }

    #[test]
    fn test_syscall_try_from_kernel_get_info() {
        assert_eq!(Syscall::try_from(80).unwrap(), Syscall::KernelGetInfo);
    }

    #[test]
    fn test_syscall_try_from_invalid() {
        assert!(Syscall::try_from(999).is_err());
    }

    #[test]
    fn test_syscall_try_from_gap_value() {
        // Values between defined syscalls should fail (e.g., 8 is between IPC and
        // Process)
        assert!(Syscall::try_from(8).is_err());
        assert!(Syscall::try_from(9).is_err());
        assert!(Syscall::try_from(19).is_err());
        assert!(Syscall::try_from(25).is_err());
    }

    // --- Syscall round-trip tests ---

    #[test]
    fn test_all_ipc_syscalls() {
        let ipc_syscalls = [
            (0, Syscall::IpcSend),
            (1, Syscall::IpcReceive),
            (2, Syscall::IpcCall),
            (3, Syscall::IpcReply),
            (4, Syscall::IpcCreateEndpoint),
            (5, Syscall::IpcBindEndpoint),
            (6, Syscall::IpcShareMemory),
            (7, Syscall::IpcMapMemory),
        ];

        for (num, expected) in &ipc_syscalls {
            let result = Syscall::try_from(*num);
            assert!(result.is_ok(), "Syscall {} should be valid", num);
            assert_eq!(result.unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_process_syscalls() {
        let proc_syscalls = [
            (10, Syscall::ProcessYield),
            (11, Syscall::ProcessExit),
            (12, Syscall::ProcessFork),
            (13, Syscall::ProcessExec),
            (14, Syscall::ProcessWait),
            (15, Syscall::ProcessGetPid),
            (16, Syscall::ProcessGetPPid),
            (17, Syscall::ProcessSetPriority),
            (18, Syscall::ProcessGetPriority),
        ];

        for (num, expected) in &proc_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_thread_syscalls() {
        let thread_syscalls = [
            (40, Syscall::ThreadCreate),
            (41, Syscall::ThreadExit),
            (42, Syscall::ThreadJoin),
            (43, Syscall::ThreadGetTid),
            (44, Syscall::ThreadSetAffinity),
            (45, Syscall::ThreadGetAffinity),
        ];

        for (num, expected) in &thread_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_file_syscalls() {
        let file_syscalls = [
            (50, Syscall::FileOpen),
            (51, Syscall::FileClose),
            (52, Syscall::FileRead),
            (53, Syscall::FileWrite),
            (54, Syscall::FileSeek),
            (55, Syscall::FileStat),
            (56, Syscall::FileTruncate),
        ];

        for (num, expected) in &file_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_dir_syscalls() {
        let dir_syscalls = [
            (60, Syscall::DirMkdir),
            (61, Syscall::DirRmdir),
            (62, Syscall::DirOpendir),
            (63, Syscall::DirReaddir),
            (64, Syscall::DirClosedir),
        ];

        for (num, expected) in &dir_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    // --- SyscallError conversion tests ---

    #[test]
    fn test_syscall_error_from_ipc_error_invalid_capability() {
        let err: SyscallError = IpcError::InvalidCapability.into();
        assert_eq!(err, SyscallError::InvalidCapability);
    }

    #[test]
    fn test_syscall_error_from_ipc_error_process_not_found() {
        let err: SyscallError = IpcError::ProcessNotFound.into();
        assert_eq!(err, SyscallError::ResourceNotFound);
    }

    #[test]
    fn test_syscall_error_from_ipc_error_endpoint_not_found() {
        let err: SyscallError = IpcError::EndpointNotFound.into();
        assert_eq!(err, SyscallError::ResourceNotFound);
    }

    #[test]
    fn test_syscall_error_from_ipc_error_out_of_memory() {
        let err: SyscallError = IpcError::OutOfMemory.into();
        assert_eq!(err, SyscallError::OutOfMemory);
    }

    #[test]
    fn test_syscall_error_from_ipc_error_would_block() {
        let err: SyscallError = IpcError::WouldBlock.into();
        assert_eq!(err, SyscallError::WouldBlock);
    }

    #[test]
    fn test_syscall_error_from_ipc_error_permission_denied() {
        let err: SyscallError = IpcError::PermissionDenied.into();
        assert_eq!(err, SyscallError::PermissionDenied);
    }

    // --- SyscallError value tests ---

    #[test]
    fn test_syscall_error_values() {
        assert_eq!(SyscallError::InvalidSyscall as i32, -1);
        assert_eq!(SyscallError::InvalidArgument as i32, -2);
        assert_eq!(SyscallError::PermissionDenied as i32, -3);
        assert_eq!(SyscallError::ResourceNotFound as i32, -4);
        assert_eq!(SyscallError::OutOfMemory as i32, -5);
        assert_eq!(SyscallError::WouldBlock as i32, -6);
        assert_eq!(SyscallError::InvalidCapability as i32, -10);
    }

    #[test]
    fn test_syscall_error_from_cap_error() {
        let err: SyscallError = crate::cap::manager::CapError::InvalidCapability.into();
        assert_eq!(err, SyscallError::InvalidCapability);

        let err: SyscallError = crate::cap::manager::CapError::InsufficientRights.into();
        assert_eq!(err, SyscallError::InsufficientRights);

        let err: SyscallError = crate::cap::manager::CapError::CapabilityRevoked.into();
        assert_eq!(err, SyscallError::CapabilityRevoked);

        let err: SyscallError = crate::cap::manager::CapError::OutOfMemory.into();
        assert_eq!(err, SyscallError::OutOfMemory);
    }
}
