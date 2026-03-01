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
pub(crate) mod process;
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
mod arch_prctl;
mod futex;
mod thread_clone;
mod userspace;
pub use futex::sys_futex_wake;
pub use userspace::copy_to_user;

// Import Phase 6 syscall modules
mod graphics_syscalls;
use self::graphics_syscalls::*;
mod wayland_syscalls;
use self::wayland_syscalls::*;
mod network_ext_syscalls;
use self::network_ext_syscalls::*;

// Import Phase 6.5 PTY syscall module
mod pty;
#[allow(unused_imports)]
use self::pty::{sys_grantpt, sys_openpty, sys_ptsname, sys_unlockpt};

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
    ThreadClone = 46,

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
    FsFsync = 73,

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
    FutexWait = 201,
    FutexWake = 202,
    ArchPrctl = 203,

    // System information
    ProcessUname = 204,
    /// Look up an environment variable by name from the process's env_vars.
    ///
    /// Required because some CRT implementations (e.g. GCC's internal CRT)
    /// skip __libc_start_main, leaving the libc `environ` pointer NULL.
    ProcessGetenv = 205,

    // POSIX shared memory
    ShmOpen = 210,
    ShmUnlink = 211,
    ShmTruncate = 212,

    // Socket operations
    SocketCreate = 220,
    SocketBind = 221,
    SocketListen = 222,
    SocketConnect = 223,
    SocketAccept = 224,
    SocketSend = 225,
    SocketRecv = 226,
    SocketClose = 227,
    SocketPair = 228,

    // Graphics / framebuffer (Phase 6)
    FbGetInfo = 230,
    FbMap = 231,
    InputPoll = 232,
    InputRead = 233,
    FbSwap = 234,

    // Wayland compositor (Phase 6)
    WlConnect = 240,
    WlDisconnect = 241,
    WlSendMessage = 242,
    WlRecvMessage = 243,
    WlCreateShmPool = 244,
    WlCreateSurface = 245,
    WlCommitSurface = 246,
    WlGetEvents = 247,

    // Network (Phase 6) -- AF_INET extensions
    NetSendTo = 250,
    NetRecvFrom = 251,
    NetGetSockName = 252,
    NetGetPeerName = 253,
    NetSetSockOpt = 254,
    NetGetSockOpt = 255,

    // Resource limits (Phase 6.5)
    GetRlimit = 260,
    SetRlimit = 261,

    // epoll I/O multiplexing (Phase 6.5)
    EpollCreate = 262,
    EpollCtl = 263,
    EpollWait = 264,

    // Process groups / sessions (Phase 6.5)
    SetPgid = 270,
    GetPgid = 271,
    SetSid = 272,
    GetSid = 273,
    TcSetPgrp = 274,
    TcGetPgrp = 275,

    // PTY (Phase 6.5)
    OpenPty = 280,
    GrantPty = 281,
    UnlockPty = 282,
    PtsName = 283,

    // Filesystem extensions (Phase 6.5)
    Link = 290,
    Symlink = 291,
    Readlink = 292,
    Lstat = 293,
    Fchmod = 294,
    Fchown = 295,
    Umask = 296,
    Access = 297,

    // Poll/fcntl (Phase 6.5)
    Poll = 300,
    Fcntl = 301,

    // Threading (Phase 6.5)
    Clone = 310,
    Futex = 311,

    // Audio (Phase 7)
    AudioOpen = 320,
    AudioClose = 321,
    AudioWrite = 322,
    AudioSetVolume = 323,
    AudioGetInfo = 324,
    AudioStart = 325,
    AudioStop = 326,
    AudioPause = 327,
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

    // Filesystem errors (values match veridian/errno.h)
    FileExists = -20,
    BadFileDescriptor = -21,
    IoError = -22,

    // Exec errors
    ArgumentListTooLong = -24,

    NotADirectory = -28,
    IsADirectory = -29,
    NotATerminal = -32,
    BrokenPipe = -39,
    DirectoryNotEmpty = -45,
    /// Resource limit exceeded (process table full, fd table full, etc.)
    /// Maps to ERESOURCELIMIT (errno 79) in user space.
    /// For POSIX fork() EAGAIN semantics, prefer WouldBlock (errno 6).
    ResourceLimitExceeded = -79,
    /// Syscall registered but not yet implemented (Phase 6.5 stubs).
    /// Maps to ENOSYS (errno 38) in user space.
    NotImplemented = -38,
    /// Too many levels of symbolic links (ELOOP).
    /// Maps to ELOOP (errno 40) in user space.
    SymlinkLoop = -40,
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

/// Map a KernelError (especially filesystem errors) to the appropriate
/// SyscallError. Values match veridian/errno.h so libc's `__syscall_ret()` sets
/// correct errno.
pub fn map_kernel_error(err: crate::error::KernelError) -> SyscallError {
    use crate::error::{FsError, KernelError};
    match err {
        KernelError::FsError(fs_err) => match fs_err {
            FsError::NotFound => SyscallError::ResourceNotFound,
            FsError::AlreadyExists => SyscallError::FileExists,
            FsError::PermissionDenied => SyscallError::PermissionDenied,
            FsError::NotADirectory => SyscallError::NotADirectory,
            FsError::IsADirectory => SyscallError::IsADirectory,
            FsError::DirectoryNotEmpty => SyscallError::DirectoryNotEmpty,
            FsError::BadFileDescriptor => SyscallError::BadFileDescriptor,
            FsError::IoError => SyscallError::IoError,
            FsError::NotAFile => SyscallError::InvalidArgument,
            FsError::ReadOnly => SyscallError::PermissionDenied,
            FsError::InvalidPath => SyscallError::InvalidArgument,
            FsError::NoRootFs => SyscallError::ResourceNotFound,
            FsError::TooManyOpenFiles => SyscallError::OutOfMemory,
            _ => SyscallError::InvalidState,
        },
        KernelError::OutOfMemory { .. } => SyscallError::OutOfMemory,
        KernelError::PermissionDenied { .. } => SyscallError::PermissionDenied,
        KernelError::AlreadyExists { .. } => SyscallError::FileExists,
        KernelError::NotFound { .. } => SyscallError::ResourceNotFound,
        KernelError::BrokenPipe => SyscallError::BrokenPipe,
        _ => SyscallError::InvalidState,
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
    // Speculation barrier at syscall entry to mitigate Spectre-style attacks.
    // Prevents speculative execution of kernel code with user-controlled values.
    crate::arch::speculation_barrier();

    // KPTI: switch to full kernel page tables on syscall entry.
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::kpti::on_syscall_entry();

    // Track syscall count
    SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed);

    // Trace: syscall entry
    crate::trace!(
        crate::perf::trace::TraceEventType::SyscallEntry,
        syscall_num as u64,
        arg1 as u64
    );

    // Rate limiting check
    if !SYSCALL_RATE_LIMITER.check() {
        SYSCALL_ERRORS.fetch_add(1, Ordering::Relaxed);
        return SyscallError::WouldBlock as i32 as isize;
    }

    // Get caller PID for audit logging
    let caller_pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let result = match Syscall::try_from(syscall_num) {
        Ok(syscall) => handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5),
        Err(_) => Err(SyscallError::InvalidSyscall),
    };

    // Audit log: syscall with result.
    // Safe to call even from syscall context - uses try_lock() with graceful
    // fallback if locks are held, preventing deadlocks during syscall return.
    let success = result.is_ok();
    if !success {
        SYSCALL_ERRORS.fetch_add(1, Ordering::Relaxed);
    }

    // Audit logging: CR3 switching was removed in v0.4.9 so VFS/heap
    // access from syscall context is safe.  log_event() uses try_lock()
    // to avoid deadlocks.
    crate::security::audit::log_syscall(caller_pid, 0, syscall_num, success);

    let ret = match result {
        Ok(value) => value as isize,
        Err(error) => error as i32 as isize,
    };

    // Trace: syscall exit
    crate::trace!(
        crate::perf::trace::TraceEventType::SyscallExit,
        syscall_num as u64,
        ret as u64
    );

    // KPTI: switch to shadow page tables before returning to user mode.
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::kpti::on_syscall_exit();

    ret
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
    crate::perf::count_syscall();
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
        Syscall::ThreadClone => thread_clone::sys_thread_clone(arg1, arg2, arg3, arg4, arg5),

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
        Syscall::FsFsync => sys_fsync(arg1),

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
        // Futex entrypoint: dispatch all futex ops (wait/wake/requeue/bitset/wake_op)
        Syscall::FutexWait => {
            futex::sys_futex_dispatch(arg1, arg2, arg3, arg4, arg5).map(|v| v as usize)
        }
        Syscall::FutexWake => futex::sys_futex_wake(arg1, arg2, arg3).map(|v| v as usize),
        Syscall::ArchPrctl => arch_prctl::sys_arch_prctl(arg1, arg2).map(|v| v as usize),
        Syscall::ProcessUname => sys_uname(arg1),
        Syscall::ProcessGetenv => sys_getenv(arg1, arg2, arg3, arg4),

        // POSIX shared memory
        Syscall::ShmOpen => sys_shm_open(arg1, arg2, arg3),
        Syscall::ShmUnlink => sys_shm_unlink(arg1, arg2),
        Syscall::ShmTruncate => sys_shm_truncate(arg1, arg2, arg3),

        // Socket operations
        Syscall::SocketCreate => sys_socket_create(arg1, arg2),
        Syscall::SocketBind => sys_socket_bind(arg1, arg2, arg3),
        Syscall::SocketListen => sys_socket_listen(arg1, arg2),
        Syscall::SocketConnect => sys_socket_connect(arg1, arg2, arg3),
        Syscall::SocketAccept => sys_socket_accept(arg1),
        Syscall::SocketSend => sys_socket_send(arg1, arg2, arg3),
        Syscall::SocketRecv => sys_socket_recv(arg1, arg2, arg3),
        Syscall::SocketClose => sys_socket_close(arg1),
        Syscall::SocketPair => sys_socket_pair(arg1, arg2),

        // Graphics / framebuffer (Phase 6)
        Syscall::FbGetInfo => sys_fb_get_info(arg1),
        Syscall::FbMap => sys_fb_map(arg1, arg2),
        Syscall::InputPoll => sys_input_poll(arg1),
        Syscall::InputRead => sys_input_read(arg1, arg2),
        Syscall::FbSwap => sys_fb_swap(),

        // Wayland compositor (Phase 6)
        Syscall::WlConnect => sys_wl_connect(),
        Syscall::WlDisconnect => sys_wl_disconnect(arg1),
        Syscall::WlSendMessage => sys_wl_send_message(arg1, arg2, arg3),
        Syscall::WlRecvMessage => sys_wl_recv_message(arg1, arg2, arg3),
        Syscall::WlCreateShmPool => sys_wl_create_shm_pool(arg1, arg2),
        Syscall::WlCreateSurface => sys_wl_create_surface(arg1, arg2, arg3, arg4),
        Syscall::WlCommitSurface => sys_wl_commit_surface(arg1, arg2),
        Syscall::WlGetEvents => sys_wl_get_events(arg1, arg2, arg3),

        // Network extensions (Phase 6)
        Syscall::NetSendTo => sys_net_sendto(arg1, arg2, arg3, arg4, arg5),
        Syscall::NetRecvFrom => sys_net_recvfrom(arg1, arg2, arg3, arg4),
        Syscall::NetGetSockName => sys_net_getsockname(arg1, arg2, arg3),
        Syscall::NetGetPeerName => sys_net_getpeername(arg1, arg2, arg3),
        Syscall::NetSetSockOpt => sys_net_setsockopt(arg1, arg2, arg3, arg4, arg5),
        Syscall::NetGetSockOpt => sys_net_getsockopt(arg1, arg2, arg3, arg4),

        // Resource limits (Phase 6.5)
        Syscall::GetRlimit => memory::sys_getrlimit(arg1, arg2),
        Syscall::SetRlimit => memory::sys_setrlimit(arg1, arg2),

        // epoll I/O multiplexing (Phase 6.5)
        Syscall::EpollCreate => {
            let pid = crate::process::current_process()
                .map(|p| p.pid.0)
                .unwrap_or(0);
            crate::net::epoll::epoll_create(pid)
                .map(|id| id as usize)
                .map_err(|_| SyscallError::OutOfMemory)
        }
        Syscall::EpollCtl => {
            let epoll_id = arg1 as u32;
            let op = arg2 as u32;
            let fd = arg3 as i32;
            let event_ptr = arg4;
            let event = if event_ptr != 0 {
                validate_user_ptr_typed::<crate::net::epoll::EpollEvent>(event_ptr)?;
                Some(unsafe { &*(event_ptr as *const crate::net::epoll::EpollEvent) })
            } else {
                None
            };
            crate::net::epoll::epoll_ctl(epoll_id, op, fd, event)
                .map(|_| 0)
                .map_err(|_| SyscallError::InvalidArgument)
        }
        Syscall::EpollWait => {
            let epoll_id = arg1 as u32;
            let events_ptr = arg2;
            let max_events = arg3;
            let timeout_ms = arg4 as i32;
            if max_events == 0 {
                return Err(SyscallError::InvalidArgument);
            }
            validate_user_buffer(
                events_ptr,
                max_events * core::mem::size_of::<crate::net::epoll::EpollEvent>(),
            )?;
            let events = unsafe {
                core::slice::from_raw_parts_mut(
                    events_ptr as *mut crate::net::epoll::EpollEvent,
                    max_events,
                )
            };
            crate::net::epoll::epoll_wait(epoll_id, events, timeout_ms)
                .map_err(|_| SyscallError::InvalidArgument)
        }
        // Process groups / sessions (Phase 6.5) -- delegate to existing
        // implementations which also back the older syscall numbers 176-180.
        Syscall::SetPgid => sys_setpgid(arg1, arg2),
        Syscall::GetPgid => sys_getpgid(arg1),
        Syscall::SetSid => sys_setsid(),
        Syscall::GetSid => sys_getsid(arg1),
        Syscall::TcSetPgrp => sys_tcsetpgrp(arg1, arg2),
        Syscall::TcGetPgrp => sys_tcgetpgrp(arg1),
        // PTY syscalls (Phase 6.5)
        Syscall::OpenPty => pty::sys_openpty(arg1, arg2),
        Syscall::GrantPty => pty::sys_grantpt(arg1),
        Syscall::UnlockPty => pty::sys_unlockpt(arg1),
        Syscall::PtsName => pty::sys_ptsname(arg1, arg2, arg3),
        Syscall::Link => sys_link(arg1, arg2),
        Syscall::Symlink => sys_symlink(arg1, arg2),
        Syscall::Readlink => sys_readlink(arg1, arg2, arg3),
        Syscall::Lstat => sys_lstat(arg1, arg2),
        Syscall::Fchmod => sys_fchmod(arg1, arg2),
        Syscall::Fchown => sys_fchown(arg1, arg2, arg3),
        Syscall::Umask => sys_umask(arg1),
        Syscall::Access => sys_access(arg1, arg2),
        // Duplicate POSIX aliases -- delegate to the primary implementations.
        Syscall::Poll => filesystem::sys_poll(arg1, arg2, arg3),
        Syscall::Fcntl => filesystem::sys_fcntl(arg1, arg2, arg3),
        Syscall::Clone => thread_clone::sys_thread_clone(arg1, arg2, arg3, arg4, arg5),
        Syscall::Futex => {
            // arg1=op (0=WAIT, 1=WAKE), arg2=addr, arg3=val, arg4=timeout/aux, arg5=op2
            match arg1 {
                0 => futex::sys_futex_wait(arg2, arg3 as u32, arg4, arg5, arg1).map(|v| v as usize),
                1 => futex::sys_futex_wake(arg2, arg3, arg4).map(|v| v as usize),
                _ => Err(SyscallError::InvalidArgument),
            }
        }

        // Audio syscalls (Phase 7) -- wired to audio subsystem
        Syscall::AudioOpen => {
            // arg1=sample_rate, arg2=channels -> returns stream_id
            let sample_rate = arg1 as u32;
            let channels = if arg2 == 0 { 2u8 } else { arg2 as u8 };
            let config = crate::audio::AudioConfig {
                sample_rate: if sample_rate == 0 { 48000 } else { sample_rate },
                channels,
                format: crate::audio::SampleFormat::S16Le,
                buffer_frames: 1024,
            };
            crate::audio::client::with_client(|client| {
                client
                    .create_stream("user_stream", config)
                    .map(|id| id.as_u32() as usize)
            })
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|_| SyscallError::OutOfMemory)
        }
        Syscall::AudioClose => {
            // arg1=stream_id
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            crate::audio::client::with_client(|client| client.close_stream(stream_id))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }
        Syscall::AudioWrite => {
            // arg1=stream_id, arg2=buffer_ptr, arg3=sample_count
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            let buf_ptr = arg2;
            let sample_count = arg3;
            let byte_len = sample_count * 2; // i16 = 2 bytes
            validate_user_buffer(buf_ptr, byte_len)?;
            let samples =
                unsafe { core::slice::from_raw_parts(buf_ptr as *const i16, sample_count) };
            crate::audio::client::with_client(|client| client.write_samples(stream_id, samples))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)
        }
        Syscall::AudioSetVolume => {
            // arg1=stream_id, arg2=volume (0-100)
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            let volume = arg2 as u16;
            crate::audio::client::with_client(|client| client.set_volume(stream_id, volume))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }
        Syscall::AudioGetInfo => {
            // arg1=info_ptr -> writes (sample_rate: u32, channels: u32, streams: u32)
            let info_ptr = arg1;
            validate_user_buffer(info_ptr, 12)?; // 3 x u32
            let info = crate::audio::client::with_client(|client| {
                (
                    client.default_sample_rate(),
                    client.default_channels() as u32,
                    client.stream_count() as u32,
                )
            })
            .map_err(|_| SyscallError::InvalidState)?;
            unsafe {
                let ptr = info_ptr as *mut u32;
                ptr.write(info.0);
                ptr.add(1).write(info.1);
                ptr.add(2).write(info.2);
            }
            Ok(0)
        }
        Syscall::AudioStart => {
            // arg1=stream_id
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            crate::audio::client::with_client(|client| client.play(stream_id))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }
        Syscall::AudioStop => {
            // arg1=stream_id
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            crate::audio::client::with_client(|client| client.stop(stream_id))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }
        Syscall::AudioPause => {
            // arg1=stream_id
            let stream_id = crate::audio::client::AudioStreamId(arg1 as u32);
            crate::audio::client::with_client(|client| client.pause(stream_id))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidArgument)?;
            Ok(0)
        }

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
    let mut rights = crate::cap::memory_integration::MemoryRights::MAP
        | crate::cap::memory_integration::MemoryRights::SHARE;
    if permissions & 0b001 != 0 {
        rights |= crate::cap::memory_integration::MemoryRights::READ;
    }
    if permissions & 0b010 != 0 {
        rights |= crate::cap::memory_integration::MemoryRights::WRITE;
    }
    if permissions & 0b100 != 0 {
        rights |= crate::cap::memory_integration::MemoryRights::EXECUTE;
    }

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
    let region = match SharedRegion::new(current_process.pid, size, perms) {
        Ok(region) => region,
        Err(_) => return Err(SyscallError::OutOfMemory),
    };

    // Use the region's actual physical base address for the capability
    let phys_addr = region.physical_base();
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

    // Look up the capability's backing object to get the physical region info
    let (object_ref, _cap_rights) = cap_space
        .lookup_entry(cap_token)
        .ok_or(SyscallError::InvalidArgument)?;

    let (base_phys, region_size) = match object_ref {
        crate::cap::object::ObjectRef::Memory { base, size, .. } => (base, size),
        _ => return Err(SyscallError::InvalidArgument),
    };

    // Suppress unused-variable warning for page_flags (used by MappingType::Shared
    // defaults)
    let _ = page_flags;

    // Determine the virtual address to map at
    let vaddr = if addr_hint == 0 {
        // Allocate in the user mmap region (above heap, below stack)
        // Use a simple deterministic address based on physical address
        0x4000_0000usize + (base_phys & 0x0FFF_FFFF)
    } else {
        addr_hint
    };

    // Map the physical pages into the process's address space
    let memory_space = current_process.memory_space.lock();
    if let Err(_e) = memory_space.map_region(
        crate::mm::VirtualAddress::new(vaddr as u64),
        region_size,
        crate::mm::vas::MappingType::Shared,
    ) {
        return Err(SyscallError::OutOfMemory);
    }

    Ok(vaddr)
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
            46 => Ok(Syscall::ThreadClone),

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
            73 => Ok(Syscall::FsFsync),

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
            201 => Ok(Syscall::FutexWait),
            202 => Ok(Syscall::FutexWake),
            203 => Ok(Syscall::ArchPrctl),
            204 => Ok(Syscall::ProcessUname),
            205 => Ok(Syscall::ProcessGetenv),

            // POSIX shared memory
            210 => Ok(Syscall::ShmOpen),
            211 => Ok(Syscall::ShmUnlink),
            212 => Ok(Syscall::ShmTruncate),

            // Socket operations
            220 => Ok(Syscall::SocketCreate),
            221 => Ok(Syscall::SocketBind),
            222 => Ok(Syscall::SocketListen),
            223 => Ok(Syscall::SocketConnect),
            224 => Ok(Syscall::SocketAccept),
            225 => Ok(Syscall::SocketSend),
            226 => Ok(Syscall::SocketRecv),
            227 => Ok(Syscall::SocketClose),
            228 => Ok(Syscall::SocketPair),

            // Graphics / framebuffer (Phase 6)
            230 => Ok(Syscall::FbGetInfo),
            231 => Ok(Syscall::FbMap),
            232 => Ok(Syscall::InputPoll),
            233 => Ok(Syscall::InputRead),
            234 => Ok(Syscall::FbSwap),

            // Wayland compositor (Phase 6)
            240 => Ok(Syscall::WlConnect),
            241 => Ok(Syscall::WlDisconnect),
            242 => Ok(Syscall::WlSendMessage),
            243 => Ok(Syscall::WlRecvMessage),
            244 => Ok(Syscall::WlCreateShmPool),
            245 => Ok(Syscall::WlCreateSurface),
            246 => Ok(Syscall::WlCommitSurface),
            247 => Ok(Syscall::WlGetEvents),

            // Network extensions (Phase 6)
            250 => Ok(Syscall::NetSendTo),
            251 => Ok(Syscall::NetRecvFrom),
            252 => Ok(Syscall::NetGetSockName),
            253 => Ok(Syscall::NetGetPeerName),
            254 => Ok(Syscall::NetSetSockOpt),
            255 => Ok(Syscall::NetGetSockOpt),

            // Resource limits (Phase 6.5)
            260 => Ok(Syscall::GetRlimit),
            261 => Ok(Syscall::SetRlimit),

            // epoll I/O multiplexing (Phase 6.5)
            262 => Ok(Syscall::EpollCreate),
            263 => Ok(Syscall::EpollCtl),
            264 => Ok(Syscall::EpollWait),

            // Process groups / sessions (Phase 6.5)
            270 => Ok(Syscall::SetPgid),
            271 => Ok(Syscall::GetPgid),
            272 => Ok(Syscall::SetSid),
            273 => Ok(Syscall::GetSid),
            274 => Ok(Syscall::TcSetPgrp),
            275 => Ok(Syscall::TcGetPgrp),

            // PTY (Phase 6.5)
            280 => Ok(Syscall::OpenPty),
            281 => Ok(Syscall::GrantPty),
            282 => Ok(Syscall::UnlockPty),
            283 => Ok(Syscall::PtsName),

            // Filesystem extensions (Phase 6.5)
            290 => Ok(Syscall::Link),
            291 => Ok(Syscall::Symlink),
            292 => Ok(Syscall::Readlink),
            293 => Ok(Syscall::Lstat),
            294 => Ok(Syscall::Fchmod),
            295 => Ok(Syscall::Fchown),
            296 => Ok(Syscall::Umask),
            297 => Ok(Syscall::Access),

            // Poll/fcntl (Phase 6.5)
            300 => Ok(Syscall::Poll),
            301 => Ok(Syscall::Fcntl),

            // Threading (Phase 6.5)
            310 => Ok(Syscall::Clone),
            311 => Ok(Syscall::Futex),

            // Audio (Phase 7)
            320 => Ok(Syscall::AudioOpen),
            321 => Ok(Syscall::AudioClose),
            322 => Ok(Syscall::AudioWrite),
            323 => Ok(Syscall::AudioSetVolume),
            324 => Ok(Syscall::AudioGetInfo),
            325 => Ok(Syscall::AudioStart),
            326 => Ok(Syscall::AudioStop),
            327 => Ok(Syscall::AudioPause),

            _ => Err(()),
        }
    }
}

// ---------------------------------------------------------------------------
// POSIX Shared Memory syscall handlers
// ---------------------------------------------------------------------------

/// Read a null-terminated name string from user space (for shm/socket paths).
fn read_user_name(ptr: usize, max_len: usize) -> Result<alloc::string::String, SyscallError> {
    validate_user_string_ptr(ptr)?;
    // SAFETY: ptr was validated as non-null and in user-space.
    let bytes = unsafe {
        let mut buf = alloc::vec::Vec::new();
        let mut p = ptr as *const u8;
        for _ in 0..max_len {
            let byte = *p;
            if byte == 0 {
                break;
            }
            buf.push(byte);
            p = p.add(1);
        }
        buf
    };
    core::str::from_utf8(&bytes)
        .map(alloc::string::String::from)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// SYS_SHM_OPEN: Create or open a named shared memory object.
///
/// # Arguments
/// - name_ptr: user-space pointer to null-terminated name
/// - flags: open flags (bit 0 = create, bit 1 = exclusive, bit 2 = read-only)
/// - _mode: permission mode (reserved)
fn sys_shm_open(name_ptr: usize, flags: usize, _mode: usize) -> SyscallResult {
    let name = read_user_name(name_ptr, crate::ipc::posix_shm::SHM_NAME_MAX)?;

    let shm_flags = crate::ipc::posix_shm::ShmOpenFlags {
        create: flags & 1 != 0,
        exclusive: flags & 2 != 0,
        read_only: flags & 4 != 0,
    };

    let pid = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(crate::process::ProcessId(0));

    crate::ipc::posix_shm::shm_open(&name, shm_flags, pid)
        .map(|id| id as usize)
        .map_err(|_| SyscallError::InvalidState)
}

/// SYS_SHM_UNLINK: Remove a named shared memory object.
///
/// # Arguments
/// - name_ptr: user-space pointer to null-terminated name
/// - name_len: length hint (unused, reads until null terminator)
fn sys_shm_unlink(name_ptr: usize, _name_len: usize) -> SyscallResult {
    let name = read_user_name(name_ptr, crate::ipc::posix_shm::SHM_NAME_MAX)?;

    crate::ipc::posix_shm::shm_unlink(&name)
        .map(|()| 0)
        .map_err(|_| SyscallError::ResourceNotFound)
}

/// SYS_SHM_TRUNCATE: Set the size of a shared memory object.
///
/// # Arguments
/// - name_ptr: user-space pointer to null-terminated name
/// - _name_len: length hint (unused)
/// - size: new size in bytes
fn sys_shm_truncate(name_ptr: usize, _name_len: usize, size: usize) -> SyscallResult {
    let name = read_user_name(name_ptr, crate::ipc::posix_shm::SHM_NAME_MAX)?;

    crate::ipc::posix_shm::shm_truncate(&name, size)
        .map(|()| 0)
        .map_err(|_| SyscallError::OutOfMemory)
}

// ---------------------------------------------------------------------------
// Socket syscall handlers
// ---------------------------------------------------------------------------

/// Socket domain constants matching POSIX/libc.
const AF_UNIX: usize = 1;
const AF_INET: usize = 2;

/// Socket type constants matching POSIX/libc.
const SOCK_STREAM: usize = 1;
const SOCK_DGRAM: usize = 2;

/// High-bit flag to distinguish INET socket IDs from Unix socket IDs.
/// Applied to socket IDs returned by `sys_socket_create` for AF_INET sockets.
const INET_SOCKET_FLAG: usize = 0x4000_0000;

/// Check if a socket ID refers to an INET socket.
#[inline]
fn is_inet_socket(id: usize) -> bool {
    id & INET_SOCKET_FLAG != 0
}

/// Strip the INET flag to get the raw socket table ID.
#[inline]
fn inet_socket_id(id: usize) -> usize {
    id & !INET_SOCKET_FLAG
}

/// Convert user-space socket type to UnixSocketType.
fn to_unix_socket_type(
    sock_type: usize,
) -> Result<crate::net::unix_socket::UnixSocketType, SyscallError> {
    match sock_type {
        SOCK_STREAM => Ok(crate::net::unix_socket::UnixSocketType::Stream),
        SOCK_DGRAM => Ok(crate::net::unix_socket::UnixSocketType::Datagram),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// SYS_SOCKET_CREATE: Create a new socket.
///
/// # Arguments
/// - domain: AF_UNIX (1) or AF_INET (2)
/// - sock_type: SOCK_STREAM (1) or SOCK_DGRAM (2)
fn sys_socket_create(domain: usize, sock_type: usize) -> SyscallResult {
    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    match domain {
        AF_UNIX => {
            let utype = to_unix_socket_type(sock_type)?;
            crate::net::unix_socket::socket_create(utype, pid)
                .map(|id| id as usize)
                .map_err(|_| SyscallError::OutOfMemory)
        }
        AF_INET => {
            let sock_domain = crate::net::socket::SocketDomain::Inet;
            let (sock_tp, proto) = match sock_type {
                SOCK_STREAM => (
                    crate::net::socket::SocketType::Stream,
                    crate::net::socket::SocketProtocol::Tcp,
                ),
                SOCK_DGRAM => (
                    crate::net::socket::SocketType::Dgram,
                    crate::net::socket::SocketProtocol::Udp,
                ),
                _ => return Err(SyscallError::InvalidArgument),
            };
            crate::net::socket::create_socket(sock_domain, sock_tp, proto)
                .map(|id| id | INET_SOCKET_FLAG)
                .map_err(|_| SyscallError::OutOfMemory)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// SYS_SOCKET_BIND: Bind a socket to an address/path.
///
/// # Arguments
/// - socket_id: socket descriptor
/// - addr_ptr: user-space pointer to address (path for AF_UNIX)
/// - addr_len: address length
fn sys_socket_bind(socket_id: usize, addr_ptr: usize, _addr_len: usize) -> SyscallResult {
    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        // Parse addr as (ip_u32, port_u16) from user space
        validate_user_buffer(addr_ptr, 6)?;
        // SAFETY: addr_ptr validated above as non-null, in user-space, 6 bytes.
        let ip_bytes = unsafe { core::ptr::read_unaligned(addr_ptr as *const [u8; 4]) };
        let port = unsafe { core::ptr::read_unaligned((addr_ptr + 4) as *const u16) }.to_be();
        let addr = crate::net::SocketAddr::v4(crate::net::Ipv4Address(ip_bytes), port);
        crate::net::socket::with_socket_mut(id, |s| s.bind(addr))
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|_| SyscallError::InvalidState)?;
        return Ok(0);
    }
    let path = read_user_name(addr_ptr, crate::net::unix_socket::UNIX_PATH_MAX)?;
    crate::net::unix_socket::socket_bind(socket_id as u64, &path)
        .map(|()| 0)
        .map_err(|_| SyscallError::InvalidState)
}

/// SYS_SOCKET_LISTEN: Start listening on a bound socket.
fn sys_socket_listen(socket_id: usize, backlog: usize) -> SyscallResult {
    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        crate::net::socket::with_socket_mut(id, |s| s.listen(backlog))
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|_| SyscallError::InvalidState)?;
        return Ok(0);
    }
    crate::net::unix_socket::socket_listen(socket_id as u64, backlog)
        .map(|()| 0)
        .map_err(|_| SyscallError::InvalidState)
}

/// SYS_SOCKET_CONNECT: Connect to a listening socket.
fn sys_socket_connect(socket_id: usize, addr_ptr: usize, _addr_len: usize) -> SyscallResult {
    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        validate_user_buffer(addr_ptr, 6)?;
        // SAFETY: addr_ptr validated above as non-null, in user-space, 6 bytes.
        let ip_bytes = unsafe { core::ptr::read_unaligned(addr_ptr as *const [u8; 4]) };
        let port = unsafe { core::ptr::read_unaligned((addr_ptr + 4) as *const u16) }.to_be();
        let addr = crate::net::SocketAddr::v4(crate::net::Ipv4Address(ip_bytes), port);
        crate::net::socket::with_socket_mut(id, |s| s.connect(addr))
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|_| SyscallError::InvalidState)?;
        return Ok(0);
    }
    let path = read_user_name(addr_ptr, crate::net::unix_socket::UNIX_PATH_MAX)?;
    crate::net::unix_socket::socket_connect(socket_id as u64, &path)
        .map(|()| 0)
        .map_err(|_| SyscallError::InvalidState)
}

/// SYS_SOCKET_ACCEPT: Accept a pending connection.
///
/// Returns the new connected socket ID.
fn sys_socket_accept(socket_id: usize) -> SyscallResult {
    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        let result = crate::net::socket::with_socket(id, |s| s.accept())
            .map_err(|_| SyscallError::InvalidState)?;
        match result {
            Ok((new_sock, _remote)) => {
                // Register the accepted socket in the socket table
                let new_id = crate::net::socket::create_socket(
                    new_sock.domain,
                    new_sock.socket_type,
                    new_sock.protocol,
                )
                .map_err(|_| SyscallError::OutOfMemory)?;
                Ok(new_id | INET_SOCKET_FLAG)
            }
            Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
            Err(_) => Err(SyscallError::InvalidState),
        }
    } else {
        crate::net::unix_socket::socket_accept(socket_id as u64)
            .map(|(new_id, _connecting_id)| new_id as usize)
            .map_err(|e| match e {
                crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
                _ => SyscallError::InvalidState,
            })
    }
}

/// SYS_SOCKET_SEND: Send data on a connected socket.
fn sys_socket_send(socket_id: usize, buf_ptr: usize, buf_len: usize) -> SyscallResult {
    validate_user_buffer(buf_ptr, buf_len)?;
    // SAFETY: buf_ptr validated above as non-null, in user-space, within size
    // limits.
    let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, buf_len) };

    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        crate::net::socket::with_socket_mut(id, |s| s.send(data, 0))
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|e| match e {
                crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
                _ => SyscallError::InvalidState,
            })
    } else {
        crate::net::unix_socket::socket_send(socket_id as u64, data, None)
            .map_err(|_| SyscallError::InvalidState)
    }
}

/// SYS_SOCKET_RECV: Receive data from a socket.
fn sys_socket_recv(socket_id: usize, buf_ptr: usize, buf_len: usize) -> SyscallResult {
    validate_user_buffer(buf_ptr, buf_len)?;
    // SAFETY: buf_ptr validated above as non-null, in user-space, within size
    // limits.
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };

    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        crate::net::socket::with_socket_mut(id, |s| s.recv(buf, 0))
            .map_err(|_| SyscallError::InvalidState)?
            .map_err(|e| match e {
                crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
                _ => SyscallError::InvalidState,
            })
    } else {
        crate::net::unix_socket::socket_recv(socket_id as u64, buf)
            .map(|(received, _rights)| received)
            .map_err(|e| match e {
                crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
                _ => SyscallError::InvalidState,
            })
    }
}

/// SYS_SOCKET_CLOSE: Close a socket.
fn sys_socket_close(socket_id: usize) -> SyscallResult {
    if is_inet_socket(socket_id) {
        let id = inet_socket_id(socket_id);
        crate::net::socket::close_socket(id)
            .map(|()| 0)
            .map_err(|_| SyscallError::InvalidState)
    } else {
        crate::net::unix_socket::socket_close(socket_id as u64)
            .map(|()| 0)
            .map_err(|_| SyscallError::InvalidState)
    }
}

/// SYS_SOCKET_PAIR: Create a connected socket pair.
///
/// # Arguments
/// - domain: AF_UNIX only
/// - result_ptr: user-space pointer to write two u64 socket IDs
fn sys_socket_pair(domain: usize, result_ptr: usize) -> SyscallResult {
    if domain != AF_UNIX {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr_typed::<[u64; 2]>(result_ptr)?;

    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let (id_a, id_b) =
        crate::net::unix_socket::socketpair(crate::net::unix_socket::UnixSocketType::Stream, pid)
            .map_err(|_| SyscallError::OutOfMemory)?;

    // SAFETY: result_ptr validated above as aligned and in user-space.
    unsafe {
        let ptr = result_ptr as *mut u64;
        *ptr = id_a;
        *ptr.add(1) = id_b;
    }
    Ok(0)
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
