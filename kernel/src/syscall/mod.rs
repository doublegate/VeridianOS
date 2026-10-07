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

/// Maximum valid user-space address.
///
/// On x86_64 this is the canonical upper bound of user space (128 TB).
/// AArch64 and RISC-V use the same logical split for the QEMU virt machine.
/// Exclusive end of user space (see mm::user_layout for why).
use crate::mm::user_layout::USER_SPACE_END;
use crate::{
    ipc::{sync_call, sync_receive, sync_reply, sync_send, IpcError, Message, SmallMessage},
    sched,
};

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
    // Reject pointers in the null guard page (first 4KB).  No legitimate
    // user-space data lives below 0x1000; this catches common NULL-derived
    // offsets (e.g. struct field access on a NULL pointer) without a page
    // table walk.
    if ptr < 0x1000 {
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

/// The sixth syscall argument, which the 5-argument handlers do not receive.
/// On x86_64 it is r9, saved in the syscall frame. Elsewhere it is not
/// captured yet, and callers that need it must fail rather than act on a
/// made-up 0: FUTEX_WAKE_OP would store 0 to `*uaddr2` and FUTEX_WAIT_BITSET
/// would wait on an empty bitset (review of the v0.26.0 stack, PR #9). Only
/// x86_64 has user mode today, so this is not yet reachable elsewhere.
fn syscall_arg6() -> Result<usize, SyscallError> {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        crate::arch::x86_64::syscall::get_syscall_frame()
            .map(|frame| frame.r9 as usize)
            .ok_or(SyscallError::NotImplemented)
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        Err(SyscallError::NotImplemented)
    }
}

/// Syscall rate limiter using a token bucket.
///
/// Consumption is a single atomic `fetch_update` that never goes below zero,
/// and the refill is claimed by one caller at a time through a CAS on
/// `last_refill`, so concurrent syscalls can neither double-spend a token
/// nor wrap the counter (SYS-PERF-01). Elapsed time is converted from
/// hardware ticks to seconds before it becomes tokens.
struct SyscallRateLimiter {
    /// Tokens available.
    tokens: AtomicU64,
    /// Hardware timestamp up to which elapsed time has been credited.
    last_refill: AtomicU64,
}

impl SyscallRateLimiter {
    /// Burst capacity.
    const MAX_TOKENS: u64 = 100_000;
    /// Sustained rate. The bucket is global and a denial fails *any*
    /// syscall with WouldBlock, so a low rate would let one spinning process
    /// starve every other process. Until limiting is per process, the rate
    /// sits above what one CPU can issue (~50 ns per syscall under KVM), so
    /// the limiter is correct but does not throttle normal workloads.
    const REFILL_PER_SEC: u64 = 50_000_000;

    const fn new() -> Self {
        Self::with_tokens(Self::MAX_TOKENS)
    }

    const fn with_tokens(tokens: u64) -> Self {
        Self {
            tokens: AtomicU64::new(tokens),
            last_refill: AtomicU64::new(0),
        }
    }

    /// Check if a syscall is allowed (returns true if within rate limit)
    fn check(&self) -> bool {
        self.check_at(
            crate::arch::timer::read_hw_timestamp(),
            crate::arch::timer::hw_ticks_per_second(),
        )
    }

    /// [`check`](Self::check) with the clock supplied by the caller.
    fn check_at(&self, now: u64, ticks_per_sec: u64) -> bool {
        self.refill(now, ticks_per_sec);
        self.tokens
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |t| t.checked_sub(1))
            .is_ok()
    }

    fn refill(&self, now: u64, ticks_per_sec: u64) {
        if ticks_per_sec == 0 {
            return;
        }
        let last = self.last_refill.load(Ordering::Acquire);
        let elapsed = now.saturating_sub(last) as u128;
        let earned = elapsed * Self::REFILL_PER_SEC as u128 / ticks_per_sec as u128;
        if earned == 0 {
            // Leave last_refill alone so sub-token intervals accumulate.
            return;
        }
        // Credit only the ticks that produced whole tokens, so the
        // remainder is not lost; only the CAS winner adds the tokens.
        let credited = (earned * ticks_per_sec as u128 / Self::REFILL_PER_SEC as u128) as u64;
        if self
            .last_refill
            .compare_exchange(last, last + credited, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let earned = earned.min(Self::MAX_TOKENS as u128) as u64;
        let _ = self
            .tokens
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |t| {
                Some(t.saturating_add(earned).min(Self::MAX_TOKENS))
            });
    }
}

static SYSCALL_RATE_LIMITER: SyscallRateLimiter = SyscallRateLimiter::new();

/// Syscall statistics for monitoring
static SYSCALL_COUNT: AtomicU64 = AtomicU64::new(0);
static SYSCALL_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Last syscall number (for diagnostics in page fault handler).
/// Written on every syscall entry, read from PF handler via raw atomic load.
pub(crate) static LAST_SYSCALL_NUM: AtomicU64 = AtomicU64::new(0);
/// Last syscall arg1 (for diagnostics).
pub(crate) static LAST_SYSCALL_ARG1: AtomicU64 = AtomicU64::new(0);
/// Last syscall arg2 (for diagnostics).
pub(crate) static LAST_SYSCALL_ARG2: AtomicU64 = AtomicU64::new(0);

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
pub(crate) mod linux_compat;
mod thread_clone;
pub(crate) mod userspace;
pub use futex::sys_futex_wake;

// Import Phase 6 syscall modules
mod graphics_syscalls;
use self::graphics_syscalls::*;
mod wayland_syscalls;
use self::wayland_syscalls::*;
mod network_ext_syscalls;
use self::network_ext_syscalls::*;

// Phase 7.5 Wave 8: Shell/Userland extensions (io_uring, ptrace, core dump,
// users, sudo, cron)
pub(crate) mod userland_ext;

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

    // musl libc compatibility syscalls
    Getdents64 = 340,
    Prlimit64 = 341,
    InotifyInit1 = 342,
    InotifyAddWatch = 343,
    InotifyRmWatch = 344,
    Madvise = 345,

    // *at() syscalls for musl (dirfd-relative path operations)
    Fchmodat = 346,
    Fchownat = 347,
    Linkat = 348,
    Symlinkat = 349,
    Readlinkat = 350,
    MemfdCreate = 351,
    SetTidAddress = 352,
    SetRobustList = 353,
    ClockNanosleep = 354,

    // Linux calls musl used to pass through unmapped, landing on unrelated
    // native syscalls (N-103: prctl 157 = FileUnlink, tkill 200 =
    // FileSelect, tgkill 234 = FbSwap, waitid 247 = WlGetEvents; flock 73
    // shares FsFsync). The musl patch maps them here.
    Prctl = 355,
    Flock = 356,
    Tkill = 357,
    Tgkill = 358,
    Waitid = 359,
    /// rt_sigpending: signals pending for the caller while blocked.
    SigPending = 360,

    // Event/timer notification fds (KDE/Wayland infrastructure)
    Getrandom = 330,
    EventfdCreate = 331,
    EventfdRead = 332,
    EventfdWrite = 333,
    TimerfdCreate = 334,
    TimerfdSettime = 335,
    TimerfdGettime = 336,
    SignalfdCreate = 337,
    SendMsg = 338,
    RecvMsg = 339,
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
    /// Rename or link across filesystems (EXDEV, errno 48).
    CrossDevice = -48,
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
    /// Socket operation on a descriptor that is not a socket (ENOTSOCK).
    NotASocket = -88,
    /// Unknown or unsupported socket option (ENOPROTOOPT, errno 92).
    ProtocolOptionNotAvailable = -92,
    /// No child process to wait for (ECHILD, errno 10; N-99).
    NoChildProcess = -110,
    /// Operation not permitted for this caller (EPERM, errno 1): signals,
    /// credentials, process groups. File access denials stay EACCES.
    OperationNotPermitted = -111,
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
            FsError::CrossDevice => SyscallError::CrossDevice,
            _ => SyscallError::InvalidState,
        },
        KernelError::OutOfMemory { .. } => SyscallError::OutOfMemory,
        KernelError::PermissionDenied { .. } => SyscallError::PermissionDenied,
        KernelError::AlreadyExists { .. } => SyscallError::FileExists,
        KernelError::NotFound { .. } => SyscallError::ResourceNotFound,
        KernelError::BrokenPipe => SyscallError::BrokenPipe,
        KernelError::WouldBlock => SyscallError::WouldBlock,
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

    // Track syscall count and last syscall info (for PF handler diagnostics)
    #[cfg_attr(
        not(all(target_arch = "x86_64", feature = "trace")),
        allow(unused_variables)
    )]
    let count = SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed);
    LAST_SYSCALL_NUM.store(syscall_num as u64, Ordering::Relaxed);
    LAST_SYSCALL_ARG1.store(arg1 as u64, Ordering::Relaxed);
    LAST_SYSCALL_ARG2.store(arg2 as u64, Ordering::Relaxed);

    // Diagnostic (`trace` feature): the first 500 syscalls via raw serial,
    // unbuffered so output appears immediately regardless of serial mode.
    // Off by default: it costs a port write per byte and shows every
    // process's arguments on the console.
    #[cfg(all(target_arch = "x86_64", feature = "trace"))]
    if count < 500 {
        // SAFETY: COM1 (port 0x3F8) is present on the x86_64 platforms this
        // kernel targets, which is the raw_serial_* contract.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"SC#");
            crate::arch::x86_64::idt::raw_serial_hex(syscall_num as u64);
            crate::arch::x86_64::idt::raw_serial_str(b" a1=");
            crate::arch::x86_64::idt::raw_serial_hex(arg1 as u64);
            crate::arch::x86_64::idt::raw_serial_str(b" a2=");
            crate::arch::x86_64::idt::raw_serial_hex(arg2 as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    // Trace: syscall entry
    crate::trace!(
        crate::perf::trace::TraceEventType::SyscallEntry,
        syscall_num as u64,
        arg1 as u64
    );

    // Rate limiting check
    if !SYSCALL_RATE_LIMITER.check() {
        SYSCALL_ERRORS.fetch_add(1, Ordering::Relaxed);
        // Through the shared conversion like every other error: the raw
        // VeridianOS value (-6) is not EAGAIN (-11) to musl (review of the
        // v0.26.0 stack, PR #15).
        return linux_compat::to_linux_errno(SyscallError::WouldBlock);
    }

    // Get caller PID for audit logging
    //
    // Per-process flag: does this process use the Linux x86_64 syscall ABI?
    // When set, ALL syscall numbers are dispatched through the Linux compat
    // layer (not VeridianOS numbering), and error codes are translated to
    // Linux errno values on return. Nothing sets it yet (see the loader).
    // The process reference is dropped here: exit and exec do not return,
    // so an Arc held across the dispatch would never be released.
    let (caller_pid, linux_abi) = crate::process::current_process()
        .map(|p| (p.pid.0, linux_compat::is_linux_abi(&p)))
        .unwrap_or((0, false));

    let result = if linux_abi {
        // Handle ppoll specially (different arg layout from poll)
        if syscall_num == 271 {
            // ppoll(fds, nfds, timespec*, sigmask, sigsetsize)
            linux_compat::handle_ppoll(arg1, arg2, arg3)
        } else if linux_compat::is_faccessat(syscall_num) {
            sys_faccessat(arg1, arg2, arg3, arg4)
        } else if let Some(syscall) = linux_compat::translate_linux_syscall(syscall_num) {
            handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5)
        } else if let Some(result) = linux_compat::handle_linux_stub(syscall_num, arg1, arg2) {
            result
        } else {
            // SAFETY: Writing to COM1 I/O port for diagnostic output.
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86_64::idt::raw_serial_str(b"SC_UNK#");
                crate::arch::x86_64::idt::raw_serial_hex(syscall_num as u64);
                crate::arch::x86_64::idt::raw_serial_str(b"\n");
            }
            Err(SyscallError::InvalidSyscall)
        }
    } else {
        // VeridianOS native ABI: dispatch syscalls from musl-patched binaries.
        //
        // Cross-compiled musl binaries (kwin, plasmashell, dbus-daemon) contain
        // a __veridian_remap_syscall() patch that translates Linux syscall
        // numbers to VeridianOS numbers. However, the musl patch has several
        // bugs (swapped epoll numbers, wrong timerfd numbers, sigaltstack->
        // setsid mismap, etc.) that must be corrected at the kernel level since
        // the musl binary is pre-compiled and cannot be re-patched.
        //
        // Additionally, Qt/KWin C++ code and GCC runtime libraries may issue
        // raw Linux x86_64 syscall numbers that bypass musl's remap entirely
        // (via inline asm or direct `syscall` instructions). The kernel must
        // handle BOTH musl-remapped VeridianOS numbers AND raw Linux numbers.
        //
        // The dispatch strategy is:
        // 1. Fix known musl remap bugs (specific number intercepts)
        // 2. For IPC range (0-7): disambiguate Linux file I/O vs VeridianOS IPC
        // 3. Try VeridianOS Syscall::try_from() for musl-remapped numbers
        // 4. Fall back to Linux translation for raw Linux numbers
        // 5. Try Linux stubs for optional/advisory syscalls
        //
        // === musl remap bugs fixed here ===
        //
        // Bug 1: epoll_ctl/epoll_create1 SWAPPED
        //   musl: Linux epoll_ctl(233) -> 262, Linux epoll_create1(281) -> 263
        //   Correct: epoll_ctl -> EpollCtl(263), epoll_create1 -> EpollCreate(262)
        //   Effect: 262 and 263 are swapped. Fix: swap them back.
        //
        // Bug 2: epoll_create1(291) -> FileDup3(66) instead of EpollCreate(262)
        //   musl confused Linux 291 (epoll_create1) with dup3.
        //   Already handled by arg-pattern heuristic at 66.
        //
        // Bug 3: dup3(292) -> FilePipe2(65) instead of FileDup3(66)
        //   musl confused Linux 292 (dup3) with pipe2.
        //   Already handled by arg-pattern heuristic at 65.
        //
        // Bug 4: sigaltstack(131) -> Setsid(179)
        //   Linux 131 = sigaltstack, NOT setsid. musl incorrectly maps it.
        //   Fix: intercept 179 and check if it's really setsid or sigaltstack.
        //
        // Bug 5: timerfd numbers wrong (322/325/326 instead of 283/286/287)
        //   Dead code in musl remap (never triggered). Kernel intercepts
        //   the correct raw Linux numbers (283/286/287) via default passthrough.
        //
        // Bug 6: truncate(76)/ftruncate(77) SWAPPED
        //   musl: truncate(76) -> FileTruncate(56, fd-based)
        //         ftruncate(77) -> FileTruncatePath(188, path-based)
        //   Fix: intercept 56 and 188 for correct routing.
        dispatch_native_abi(syscall_num, arg1, arg2, arg3, arg4, arg5)
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
        Err(error) => {
            // Always translate error codes to Linux errno values.
            //
            // Both the linux_abi path (raw Linux syscall numbers) and the
            // native ABI path (musl-remapped VeridianOS numbers) ultimately
            // return to musl's __syscall_ret(), which interprets negative
            // return values as -errno (Linux convention).
            //
            // Previously, native ABI returned raw VeridianOS error codes
            // (e.g., ResourceNotFound = -4). musl interpreted -4 as -EINTR
            // (errno 4) and retried the syscall in an infinite loop, because
            // Linux ENOENT is -2, not -4.
            linux_compat::to_linux_errno(error)
        }
    };

    // Diagnostic (`trace` feature): results of the first 500 syscalls.
    #[cfg(all(target_arch = "x86_64", feature = "trace"))]
    if count < 500 {
        // SAFETY: COM1 (port 0x3F8) is present on the x86_64 platforms this
        // kernel targets, which is the raw_serial_* contract.
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"  =>");
            crate::arch::x86_64::idt::raw_serial_hex(ret as u64);
            crate::arch::x86_64::idt::raw_serial_str(b"\n");
        }
    }

    // Trace: syscall exit
    crate::trace!(
        crate::perf::trace::TraceEventType::SyscallExit,
        syscall_num as u64,
        ret as u64
    );

    // KPTI: switch to shadow page tables before returning to user mode.
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::kpti::on_syscall_exit();

    // Boot-path cooperative yield: if this process was dispatched as a clone
    // child from futex_wait's spin loop, yield back to the parent after each
    // syscall so the parent can re-check the futex word.
    #[cfg(target_arch = "x86_64")]
    {
        if crate::arch::x86_64::usermode::BOOT_CLONE_YIELD_PENDING
            .load(core::sync::atomic::Ordering::Acquire)
            && crate::arch::x86_64::usermode::has_boot_return_context()
        {
            // Save the child's live register state back to its ThreadContext
            // so the next dispatch uses the correct RIP/RSP/GPRs.
            // Without this, the child would restart from the clone return
            // point on every dispatch instead of progressing.
            if let Some(thread) = crate::process::current_thread() {
                if let Some(frame) = crate::arch::x86_64::syscall::get_syscall_frame() {
                    let mut ctx = thread.context.lock();
                    ctx.rip = frame.rip;
                    ctx.rflags = frame.rflags;
                    ctx.rsp = frame.rsp;
                    ctx.rax = ret as u64; // syscall return value
                    ctx.rbx = frame.rbx;
                    ctx.rbp = frame.rbp;
                    ctx.rdi = frame.rdi;
                    ctx.rsi = frame.rsi;
                    ctx.rdx = frame.rdx;
                    ctx.r8 = frame.r8;
                    ctx.r9 = frame.r9;
                    ctx.r10 = frame.r10;
                    ctx.r12 = frame.r12;
                    ctx.r13 = frame.r13;
                    ctx.r14 = frame.r14;
                    ctx.r15 = frame.r15;
                }
            }

            // SAFETY: boot return context is valid (checked above).
            // boot_return_to_kernel restores the parent's saved
            // context from enter_forked_child_returnable and returns
            // to the parent's dispatch loop in boot_futex_spin.
            unsafe {
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
    }

    ret
}

/// Dispatch a syscall using the native VeridianOS ABI, with corrections for
/// musl remap bugs and fallback to Linux x86_64 translation.
///
/// This is the primary dispatch path for cross-compiled musl binaries
/// (kwin_wayland, plasmashell, dbus-daemon). It handles:
/// 1. Known musl __veridian_remap_syscall() bugs (swapped/wrong numbers)
/// 2. Raw Linux x86_64 syscall numbers from C++ code bypassing musl
/// 3. Correct VeridianOS-numbered syscalls from musl's remap
fn dispatch_native_abi(
    syscall_num: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> SyscallResult {
    // ---------------------------------------------------------------
    // Phase 1: Fix known musl remap bugs (specific number intercepts)
    // ---------------------------------------------------------------

    // --- Raw Linux epoll_ctl (233) bypass fix ---
    // Some code paths (e.g., statically linked Qt/KDE) may call epoll_ctl
    // via raw syscall(233, ...) bypassing musl's __veridian_remap_syscall().
    // VeridianOS 233 = InputRead, which silently fails. Intercept it here.
    if syscall_num == 233 {
        // epoll_ctl(epfd, op, fd, event_ptr): op is 1/2/3
        if arg2 <= 3 {
            return handle_syscall(Syscall::EpollCtl, arg1, arg2, arg3, arg4, arg5);
        }
        // Fall through to InputRead for genuine InputRead calls
    }

    // --- epoll swap fix (Bug 1) ---
    // musl maps: Linux epoll_ctl(233) -> 262, Linux epoll_create1(281) -> 263
    // Correct:   epoll_ctl -> EpollCtl(263), epoll_create1 -> EpollCreate(262)
    // So 262 arrives when musl meant EpollCtl, and 263 when musl meant EpollCreate.
    if syscall_num == 262 {
        // 262 can be EITHER:
        // (a) musl's buggy remap: Linux epoll_ctl(233) -> 262 (swap bug)
        // (b) Raw Linux newfstatat(262) via musl's SYS_newfstatat path
        //
        // Disambiguate by argument patterns:
        //   newfstatat(dirfd, pathname, statbuf, flags):
        //     arg1 = AT_FDCWD (0xffffffffffffff9c = -100 sign-extended) or valid fd
        //     arg2 = pathname pointer (large user-space address)
        //   epoll_ctl(epfd, op, fd, event_ptr):
        //     arg1 = epoll fd (small non-negative integer, usually < 256)
        //     arg2 = EPOLL_CTL_ADD/MOD/DEL (1, 2, or 3)
        let at_fdcwd = 0xffffffffffffff9c_usize; // AT_FDCWD = -100 as usize
        if arg1 == at_fdcwd || (arg2 > 4096 && arg2 < USER_SPACE_END) {
            // Looks like fstatat(AT_FDCWD, path, ...) or fstatat(fd, path, ...)
            return handle_syscall(Syscall::FileFstatat, arg1, arg2, arg3, arg4, arg5);
        }
        // Looks like epoll_ctl(epfd, op, fd, event_ptr)
        return handle_syscall(Syscall::EpollCtl, arg1, arg2, arg3, arg4, arg5);
    }
    if syscall_num == 263 {
        // 263 can be EITHER:
        // (a) musl's buggy remap of Linux epoll_create1(291) -> 263 (swap bug)
        // (b) musl's buggy remap of Linux epoll_pwait(281) -> 263 (mislabeled as
        // epoll_ctl)
        //
        // Disambiguate by argument patterns:
        //   epoll_create1(flags): arg1 = 0 or O_CLOEXEC(0x80000), arg2 = 0
        //   epoll_pwait(epfd, events_ptr, maxevents, timeout, sigmask, sigsetsize):
        //     arg1 = epoll fd (small non-negative int)
        //     arg2 = events pointer (large user-space address)
        //     arg3 = maxevents (small positive int)
        if arg2 > 4096 && arg2 < USER_SPACE_END {
            // Looks like epoll_pwait (arg2 is a user-space pointer to events array)
            return handle_syscall(Syscall::EpollWait, arg1, arg2, arg3, arg4, arg5);
        }
        // Looks like epoll_create1 (arg2 = 0, arg1 = flags)
        return handle_syscall(Syscall::EpollCreate, arg1, arg2, arg3, arg4, arg5);
    }

    // --- epoll_create1 via wrong dup3 mapping (Bug 2) ---
    // musl maps: Linux epoll_create1(291) -> 66 (FileDup3, wrong)
    // Also: VeridianOS FileDup3 IS 66 (correct for actual dup3 calls)
    // Heuristic: epoll_create1 flags are 0 or O_CLOEXEC(0x80000);
    //            dup3(oldfd, newfd, flags) has oldfd as a small non-zero integer.
    if syscall_num == 66 {
        if arg1 == 0 || arg1 == 0x80000 {
            return handle_syscall(Syscall::EpollCreate, arg1, arg2, arg3, arg4, arg5);
        }
        return handle_syscall(Syscall::FileDup3, arg1, arg2, arg3, arg4, arg5);
    }

    // --- dup3 via wrong pipe2 mapping (Bug 3) ---
    // musl maps: Linux dup3(292) -> 65 (FilePipe2, wrong; should be FileDup3=66)
    // Also: VeridianOS FilePipe2 IS 65 (correct for actual pipe2 calls)
    // Heuristic: pipe2(pipefd_ptr, flags) has arg1 as a pointer (large value);
    //            dup3(oldfd, newfd, flags) has arg1 as a small fd integer.
    if syscall_num == 65 {
        if arg1 > 4096 {
            return handle_syscall(Syscall::FilePipe2, arg1, arg2, arg3, arg4, arg5);
        }
        return handle_syscall(Syscall::FileDup3, arg1, arg2, arg3, arg4, arg5);
    }

    // --- ppoll passthrough (not in musl remap, arrives as raw Linux 271) ---
    // Linux ppoll(271) has no musl remap case; `default: return nr` passes
    // 271 through. Collides with VeridianOS GetPgid(271).
    // ppoll args: (fds_ptr, nfds, timespec_ptr, sigmask, sigsetsize)
    //   arg1 = fds pointer (large value)
    // getpgid args: (pid) where arg1 is a small integer or 0
    if syscall_num == 271 {
        if arg1 > 4096 {
            // Looks like ppoll (fds pointer)
            return linux_compat::handle_ppoll(arg1, arg2, arg3);
        }
        // Looks like getpgid (pid as small int)
        return handle_syscall(Syscall::Getpgid, arg1, arg2, arg3, arg4, arg5);
    }

    // --- epoll_pwait passthrough (musl remap maps 281->263, but raw Linux 281
    // also arrives from Qt/KDE C++ code or libstdc++ bypassing musl) ---
    // Linux epoll_pwait(281) collides with VeridianOS GrantPty(281).
    // epoll_pwait args: (epfd, events_ptr, maxevents, timeout, sigmask, sigsetsize)
    //   arg1 = epoll fd (small non-negative int)
    //   arg2 = events pointer (large user-space address)
    // grantpt args: (master_fd) where arg2 is unused/0
    if syscall_num == 281 {
        if arg2 > 4096 && arg2 < USER_SPACE_END {
            // Looks like epoll_pwait (arg2 is user-space events pointer)
            return handle_syscall(Syscall::EpollWait, arg1, arg2, arg3, arg4, arg5);
        }
        // Looks like grantpt (arg2 = 0 or small)
        return handle_syscall(Syscall::GrantPty, arg1, arg2, arg3, arg4, arg5);
    }

    // --- timerfd passthrough (not in musl remap, arrives as raw Linux 283/286/287)
    // ---
    if syscall_num == 283 {
        return handle_syscall(Syscall::TimerfdCreate, arg1, arg2, arg3, arg4, arg5);
    }
    if syscall_num == 286 {
        return handle_syscall(Syscall::TimerfdSettime, arg1, arg2, arg3, arg4, arg5);
    }
    if syscall_num == 287 {
        return handle_syscall(Syscall::TimerfdGettime, arg1, arg2, arg3, arg4, arg5);
    }

    // --- statx stub (Linux 332, not in musl remap) ---
    // Collides with VeridianOS EventfdRead(332).
    // Qt may call statx directly. Return ENOSYS so caller falls back to fstatat.
    // Heuristic: statx has arg1=dirfd (small int), EventfdRead has arg1=efd_id.
    // Since eventfd IDs are also small ints, prefer ENOSYS which is safe for both
    // (Qt retries with fstatat, eventfd read falls back to VfsNode path).
    if syscall_num == 332 {
        return Err(SyscallError::NotImplemented);
    }

    // ---------------------------------------------------------------
    // Phase 2: IPC range (0-7) -> always route to Linux translation
    // ---------------------------------------------------------------
    // musl's remap translates Linux file I/O (0-7) to VeridianOS equivalents
    // that are ALL outside the 0-7 range:
    //   Linux 0(read)->52, 1(write)->53, 2(open)->50, 3(close)->51,
    //   4(stat)->150, 5(fstat)->55, 6(lstat)->151, 7(poll)->300
    //
    // Therefore, if the kernel receives 0-7, it is ALWAYS a raw Linux
    // syscall (from C++ code or libstdc++ bypassing musl's remap).
    // It is NEVER a musl-remapped output. Safe to route through Linux
    // translation unconditionally.
    //
    // This fixes kwin crash: Qt/libstdc++ code issues raw Linux open(2)
    // and close(3) which collide with VeridianOS IpcCall(2)/IpcReply(3).
    if syscall_num <= 7 {
        if let Some(syscall) = linux_compat::translate_linux_syscall(syscall_num) {
            return handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5);
        }
    }

    // ---------------------------------------------------------------
    // Phase 3: Handle specific known collisions in the 8-63 range
    // ---------------------------------------------------------------
    // We CANNOT use blanket Linux-first dispatch for these ranges because
    // musl's remap OUTPUTS fall within them. For example:
    //   - musl remaps Linux mmap(9) -> VeridianOS 20 (MemoryMap)
    //   - Linux 20 = writev
    //   - Linux-first would wrongly dispatch VeridianOS 20 as writev
    //
    // However, some specific Linux syscalls DO bypass musl's remap
    // (via libstdc++ or Qt inline asm). Handle these individually:

    // Linux connect(42) collides with VeridianOS ThreadJoin(42).
    // Heuristic: connect(fd, sockaddr_ptr, addrlen) always has arg2 as
    // a user-space pointer (large address > 4096). ThreadJoin(tid, retval_ptr)
    // typically has a small tid and retval_ptr is 0 or a stack pointer.
    // When arg2 is a valid user-space pointer AND we just created a socket
    // (arg1 is a plausible fd), treat as connect.
    if syscall_num == 42 && arg2 > 4096 && arg3 > 0 && arg3 < 256 {
        // arg3 is addrlen (small value like 110 for sockaddr_un)
        return handle_syscall(Syscall::SocketConnect, arg1, arg2, arg3, arg4, arg5);
    }

    // Linux getpeername(52) collides with VeridianOS NetGetPeerName(253).
    // musl maps it to 253. Raw Linux 52 would collide with FileRead(52).
    // But musl also maps Linux read(0)->52. So 52 is always FileRead.
    // (No fix needed -- covered by musl remap)

    // Linux setsockopt(54) collides with VeridianOS FileSeek(54).
    // musl maps it to 254. So 54 is always FileSeek. (No fix needed)

    // Syscall 73 is both fsync and flock: native FsFsync is 73, the musl
    // patch remaps Linux fsync (74) to it, and musl passes Linux flock (73)
    // through unchanged. Both C libraries make the second argument decide:
    // musl zero-pads unused syscall arguments, the native libc's fsync
    // passes an explicit 0, and a flock operation is never 0
    // (LOCK_SH/EX/UN = 1/2/8). Every process uses the native numbering,
    // because musl remaps before the syscall instruction, so the
    // `linux_abi` flag cannot tell them apart (N-120).
    if syscall_num == 73 && arg2 != 0 {
        return sys_flock(arg1, arg2);
    }
    // musl binaries built before the remap fix send prctl as 157, which is
    // FileUnlink here. A prctl option is a small integer and a path pointer
    // never is (nothing is mapped below 0x1000), so the first argument
    // tells them apart (N-103).
    if syscall_num == 157 && arg1 < crate::mm::user_layout::USER_SPACE_START {
        return linux_compat::sys_prctl(arg1, arg2);
    }

    // ---------------------------------------------------------------
    // Phase 5: Standard VeridianOS dispatch with Linux fallback
    // ---------------------------------------------------------------
    // At this point, the number is either:
    // - A correctly musl-remapped VeridianOS number (most common case)
    // - A VeridianOS-only number (no Linux collision)
    // - An unmapped Linux number > 63 that fell through Phase 4
    match Syscall::try_from(syscall_num) {
        Ok(syscall) => handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5),
        Err(_) => {
            // Not a valid VeridianOS number -- try Linux translation
            if linux_compat::is_faccessat(syscall_num) {
                sys_faccessat(arg1, arg2, arg3, arg4)
            } else if let Some(syscall) = linux_compat::translate_linux_syscall(syscall_num) {
                handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5)
            } else if let Some(result) = linux_compat::handle_linux_stub(syscall_num, arg1, arg2) {
                result
            } else {
                // SAFETY: Writing to COM1 I/O port for diagnostic output.
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(b"SC_UNK#");
                    crate::arch::x86_64::idt::raw_serial_hex(syscall_num as u64);
                    crate::arch::x86_64::idt::raw_serial_str(b"\n");
                }
                Err(SyscallError::InvalidSyscall)
            }
        }
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
    crate::perf::count_syscall();
    match syscall {
        // IPC system calls
        Syscall::IpcSend => sys_ipc_send(arg1, arg2, arg3, arg4),
        Syscall::IpcReceive => sys_ipc_receive(arg1, arg2, arg3),
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
        // Linux ABI: accept4(fd, addr, addrlen_ptr, flags)
        // accept (Linux 43) also maps here via remap patch.
        Syscall::SocketAccept => sys_socket_accept(arg1, arg2, arg3),
        Syscall::SocketSend => sys_socket_send(arg1, arg2, arg3),
        Syscall::SocketRecv => sys_socket_recv(arg1, arg2, arg3),
        Syscall::SocketClose => sys_socket_close(arg1),
        // Linux ABI: socketpair(domain, type, protocol, sv[2])
        // arg1=domain, arg2=type, arg3=protocol, arg4=sv pointer
        Syscall::SocketPair => sys_socket_pair(arg1, arg2, arg3, arg4),

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
        Syscall::NetRecvFrom => sys_net_recvfrom(arg1, arg2, arg3, arg4, arg5),
        Syscall::NetGetSockName => sys_net_getsockname(arg1, arg2, arg3),
        Syscall::NetGetPeerName => sys_net_getpeername(arg1, arg2, arg3),
        Syscall::NetSetSockOpt => sys_net_setsockopt(arg1, arg2, arg3, arg4, arg5),
        Syscall::NetGetSockOpt => sys_net_getsockopt(arg1, arg2, arg3, arg4, arg5),

        // Resource limits (Phase 6.5)
        Syscall::GetRlimit => memory::sys_getrlimit(arg1, arg2),
        Syscall::SetRlimit => memory::sys_setrlimit(arg1, arg2),

        // epoll I/O multiplexing (Phase 6.5)
        Syscall::EpollCreate => {
            let _flags = arg1; // epoll_create1 flags (EPOLL_CLOEXEC)
            let cloexec = (arg1 & 0x80000) != 0; // EPOLL_CLOEXEC = O_CLOEXEC
            let pid = crate::process::current_process()
                .map(|p| p.pid.0)
                .unwrap_or(0);
            let epoll_id =
                crate::net::epoll::epoll_create(pid).map_err(|_| SyscallError::OutOfMemory)?;
            // Wrap as VfsNode for real fd semantics
            let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
                alloc::sync::Arc::new(crate::net::epoll::EpollNode::new(epoll_id));
            let file = crate::fs::file::File::new(node, crate::fs::OpenFlags::read_only());
            let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
            let file_table = proc.file_table.lock();
            let fd = file_table
                .open_with_flags(alloc::sync::Arc::new(file), cloexec)
                .map_err(|_| SyscallError::OutOfMemory)?;
            Ok(fd)
        }
        Syscall::EpollCtl => {
            let epoll_fd = arg1;
            let op = arg2 as u32;
            let fd = arg3 as i32;
            let event_ptr = arg4;

            let epoll_id = resolve_epoll_id(epoll_fd)?;
            // struct epoll_event is packed (12 bytes on x86_64): copied
            // field by field through the fault-tolerant reader rather than
            // borrowed from user memory (review of the v0.26.0 stack).
            let event = if event_ptr != 0 {
                let mut raw = [0u8; EPOLL_EVENT_BYTES];
                userspace::read_user_bytes(event_ptr, &mut raw)?;
                Some(epoll_event_from_bytes(&raw))
            } else {
                None
            };
            crate::net::epoll::epoll_ctl(epoll_id, op, fd, event.as_ref())
                .map(|_| 0)
                .map_err(|_| SyscallError::InvalidArgument)
        }
        Syscall::EpollWait => {
            let epoll_fd = arg1;
            let events_ptr = arg2;
            let max_events = arg3;
            let timeout_ms = arg4 as i32;
            let epoll_id = resolve_epoll_id(epoll_fd)?;
            validate_user_buffer(events_ptr, epoll_events_buffer_len(max_events)?)?;
            // Events are gathered in a kernel array (at most 1024 per call,
            // as a short count is always allowed) and copied out packed,
            // through the fault-tolerant writer (N-43).
            let mut events = alloc::vec![
                crate::net::epoll::EpollEvent { events: 0, data: 0 };
                max_events.min(1024)
            ];
            let n =
                crate::net::epoll::epoll_wait(epoll_id, &mut events, timeout_ms).map_err(|e| {
                    match e {
                        crate::error::KernelError::WouldBlock => SyscallError::Interrupted,
                        _ => SyscallError::InvalidArgument,
                    }
                })?;
            userspace::write_user_bytes(events_ptr, &epoll_events_to_bytes(&events[..n]))?;
            Ok(n)
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
            // Linux ABI: futex(uaddr, op, val, timeout/val2, uaddr2, val3)
            // arg1=uaddr, arg2=op, arg3=val, arg4=timeout/val2, arg5=uaddr2
            // Linux futex(uaddr, op, val, timeout|val2, uaddr2, val3).
            // val3 (arg6) is not a handler parameter; it is read from the
            // saved syscall frame. Mask off FUTEX_PRIVATE_FLAG (bit 7 = 128)
            // -- VeridianOS is single-address-space per process, so
            // private == shared.
            let cmd = (arg2 as u32) & 0x7F;
            match cmd {
                // FUTEX_WAIT: wait if *uaddr == val
                0 => futex::sys_futex_wait(arg1, arg3 as u32, arg4, 0, arg2).map(|v| v as usize),
                // FUTEX_WAKE: wake up to val waiters
                1 => futex::sys_futex_wake(arg1, arg3, 0).map(|v| v as usize),
                // FUTEX_REQUEUE: wake val waiters, requeue rest to uaddr2
                3 => futex::sys_futex_requeue(arg1, arg3, arg5, 0).map(|v| v as usize),
                // FUTEX_WAKE_OP(uaddr, val, val2 = arg4, uaddr2, encoded op = val3).
                // Passing 0 for the encoded op meant "*uaddr2 = 0" on every call.
                5 => futex::sys_futex_wake_op(arg1, arg3, arg5, arg4, syscall_arg6()?)
                    .map(|v| v as usize),
                // FUTEX_WAIT_BITSET: the bitset is val3 (arg6).
                9 => futex::sys_futex_wait(arg1, arg3 as u32, arg4, syscall_arg6()?, arg2)
                    .map(|v| v as usize),
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
            let byte_len = sample_count
                .checked_mul(2) // i16 = 2 bytes
                .ok_or(SyscallError::InvalidArgument)?;
            // Copied in through the fault-tolerant reader (N-43).
            let bytes = userspace::read_user_vec(buf_ptr, byte_len, userspace::MAX_USER_MESSAGE)?;
            let samples: alloc::vec::Vec<i16> = bytes
                .chunks_exact(2)
                .map(|b| i16::from_ne_bytes([b[0], b[1]]))
                .collect();
            crate::audio::client::with_client(|client| client.write_samples(stream_id, &samples))
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
            userspace::write_user::<[u32; 3]>(info_ptr, [info.0, info.1, info.2])?;
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

        // getrandom(buf, buflen, flags) -> bytes_written
        Syscall::Getrandom => sys_getrandom(arg1, arg2, arg3),

        // eventfd syscall -- creates VfsNode-backed fd for musl compat
        Syscall::EventfdCreate => {
            let initval = arg1 as u32;
            let flags = arg2 as u32;
            let cloexec = (flags & crate::fs::eventfd::EFD_CLOEXEC) != 0;
            // Create the internal eventfd instance
            let efd_id = crate::fs::eventfd::eventfd_create(initval, flags)? as u32;
            // Wrap as VfsNode for real fd semantics (read/write/close/epoll)
            let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
                alloc::sync::Arc::new(crate::fs::eventfd::EventFdNode::new(efd_id));
            let file = crate::fs::file::File::new(node, crate::fs::OpenFlags::read_write());
            let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
            let file_table = proc.file_table.lock();
            let fd = file_table
                .open_with_flags(alloc::sync::Arc::new(file), cloexec)
                .map_err(|_| SyscallError::OutOfMemory)?;
            Ok(fd)
        }
        // EventfdRead/EventfdWrite kept for backward compat with internal callers
        // but musl programs will use read()/write() via VfsNode path
        Syscall::EventfdRead => {
            let efd_id = arg1 as u32;
            let buf_ptr = arg2;
            validate_user_buffer(buf_ptr, 8)?;
            let val = crate::fs::eventfd::eventfd_read(efd_id)?;
            userspace::write_user::<u64>(buf_ptr, val)?;
            Ok(8)
        }
        Syscall::EventfdWrite => {
            let efd_id = arg1 as u32;
            let buf_ptr = arg2;
            validate_user_buffer(buf_ptr, 8)?;
            let val: u64 = userspace::read_user(buf_ptr)?;
            crate::fs::eventfd::eventfd_write(efd_id, val)
        }

        // timerfd syscalls -- create returns VfsNode-backed fd
        Syscall::TimerfdCreate => {
            let clockid = arg1 as u32;
            let flags = arg2 as u32;
            let cloexec = (flags & crate::fs::timerfd::TFD_CLOEXEC) != 0;
            let tfd_id = crate::fs::timerfd::timerfd_create(clockid, flags)? as u32;
            let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
                alloc::sync::Arc::new(crate::fs::timerfd::TimerFdNode::new(tfd_id));
            let file = crate::fs::file::File::new(node, crate::fs::OpenFlags::read_only());
            let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
            let file_table = proc.file_table.lock();
            let fd = file_table
                .open_with_flags(alloc::sync::Arc::new(file), cloexec)
                .map_err(|_| SyscallError::OutOfMemory)?;
            Ok(fd)
        }
        // timerfd_settime/gettime: musl passes the real fd, so we look up
        // the internal tfd_id via as_any() downcast on the VfsNode.
        Syscall::TimerfdSettime => {
            let fd = arg1;
            let flags = arg2 as u32;
            let new_ptr = arg3;
            let old_ptr = arg4;
            let tfd_id = resolve_timerfd_id(fd)?;
            // Copied in, and the previous value copied out, through the
            // fault-tolerant accessors (N-43).
            let new_spec: crate::fs::timerfd::Itimerspec = userspace::read_user(new_ptr)?;
            let mut old_spec = crate::fs::timerfd::Itimerspec::default();
            let result = crate::fs::timerfd::timerfd_settime(
                tfd_id,
                flags,
                &new_spec,
                (old_ptr != 0).then_some(&mut old_spec),
            );
            if result.is_ok() && old_ptr != 0 {
                userspace::write_user(old_ptr, old_spec)?;
            }
            result
        }
        Syscall::TimerfdGettime => {
            let fd = arg1;
            let curr_ptr = arg2;
            let tfd_id = resolve_timerfd_id(fd)?;
            validate_user_ptr_typed::<crate::fs::timerfd::Itimerspec>(curr_ptr)?;
            let spec = crate::fs::timerfd::timerfd_gettime(tfd_id)?;
            userspace::write_user(curr_ptr, spec)?;
            Ok(0)
        }

        // signalfd syscall -- creates VfsNode-backed fd
        Syscall::SignalfdCreate => {
            let fd_arg = arg1 as i32;
            let mask = arg2 as u64;
            let flags = arg3 as u32;
            let cloexec = (flags & crate::fs::signalfd::SFD_CLOEXEC) != 0;
            let sfd_id = crate::fs::signalfd::signalfd_create(fd_arg, mask, flags)? as u32;
            // If updating an existing signalfd (fd_arg != -1), return the same fd
            if fd_arg != -1 {
                return Ok(fd_arg as usize);
            }
            let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
                alloc::sync::Arc::new(crate::fs::signalfd::SignalFdNode::new(sfd_id));
            let file = crate::fs::file::File::new(node, crate::fs::OpenFlags::read_only());
            let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
            let file_table = proc.file_table.lock();
            let fd = file_table
                .open_with_flags(alloc::sync::Arc::new(file), cloexec)
                .map_err(|_| SyscallError::OutOfMemory)?;
            Ok(fd)
        }

        // sendmsg/recvmsg -- delegate to unix socket module for SCM_RIGHTS
        Syscall::SendMsg => sys_sendmsg(arg1, arg2, arg3),
        Syscall::RecvMsg => sys_recvmsg(arg1, arg2, arg3),

        // musl libc compatibility syscalls
        Syscall::Getdents64 => sys_getdents64(arg1, arg2, arg3),
        Syscall::Prlimit64 => sys_prlimit64(arg1, arg2, arg3, arg4),
        Syscall::InotifyInit1 => Err(SyscallError::NotImplemented),
        Syscall::InotifyAddWatch => Err(SyscallError::NotImplemented),
        Syscall::InotifyRmWatch => Err(SyscallError::NotImplemented),
        Syscall::Madvise => {
            // madvise is advisory -- always succeed (no-op).
            Ok(0)
        }

        // *at() syscalls -- dirfd-relative path operations for musl
        Syscall::Fchmodat => sys_fchmodat(arg1, arg2, arg3),
        Syscall::Fchownat => sys_fchownat(arg1, arg2, arg3, arg4, arg5),
        Syscall::Linkat => sys_linkat(arg1, arg2, arg3, arg4, arg5),
        Syscall::Symlinkat => sys_symlinkat(arg1, arg2, arg3),
        Syscall::Readlinkat => sys_readlinkat(arg1, arg2, arg3, arg4),
        Syscall::MemfdCreate => sys_memfd_create(arg1, arg2),
        Syscall::SetTidAddress => sys_set_tid_address(arg1),
        Syscall::SetRobustList => sys_set_robust_list(arg1, arg2),
        Syscall::ClockNanosleep => sys_clock_nanosleep(arg1, arg2, arg3, arg4),
        Syscall::Prctl => linux_compat::sys_prctl(arg1, arg2),
        Syscall::Flock => sys_flock(arg1, arg2),
        Syscall::Tkill => process::sys_tkill(arg1, arg2),
        Syscall::Tgkill => process::sys_tgkill(arg1, arg2, arg3),
        Syscall::Waitid => process::sys_waitid(arg1, arg2, arg3, arg4),
        Syscall::SigPending => signal::sys_sigpending(arg1, arg2),

        _ => Err(SyscallError::InvalidSyscall),
    }
}

/// Resolve a file descriptor to an internal timerfd ID.
///
/// Looks up the fd in the current process's file table, downcasts the
/// VfsNode to `TimerFdNode`, and returns its internal `tfd_id`.
fn resolve_timerfd_id(fd: usize) -> Result<u32, SyscallError> {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;
    let any = file.node.as_any().ok_or(SyscallError::BadFileDescriptor)?;
    let tfd_node = any
        .downcast_ref::<crate::fs::timerfd::TimerFdNode>()
        .ok_or(SyscallError::BadFileDescriptor)?;
    Ok(tfd_node.tfd_id())
}

/// Resolve a file descriptor to an internal epoll ID.
fn resolve_epoll_id(fd: usize) -> Result<u32, SyscallError> {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file = file_table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;
    let any = file.node.as_any().ok_or(SyscallError::BadFileDescriptor)?;
    let epoll_node = any
        .downcast_ref::<crate::net::epoll::EpollNode>()
        .ok_or(SyscallError::BadFileDescriptor)?;
    Ok(epoll_node.epoll_id())
}

/// `sizeof(struct epoll_event)`: packed `u32 events` + `u64 data`.
const EPOLL_EVENT_BYTES: usize = core::mem::size_of::<crate::net::epoll::EpollEvent>();

/// Decode a user `struct epoll_event`.
fn epoll_event_from_bytes(raw: &[u8; EPOLL_EVENT_BYTES]) -> crate::net::epoll::EpollEvent {
    let mut events = [0u8; 4];
    let mut data = [0u8; 8];
    events.copy_from_slice(&raw[..4]);
    data.copy_from_slice(&raw[4..]);
    crate::net::epoll::EpollEvent {
        events: u32::from_ne_bytes(events),
        data: u64::from_ne_bytes(data),
    }
}

/// Bytes of the user buffer epoll_wait may fill for `max_events` events.
/// Linux caps maxevents at INT_MAX / sizeof(struct epoll_event); the byte
/// count must not wrap (W-16). Zero is EINVAL as well.
fn epoll_events_buffer_len(max_events: usize) -> Result<usize, SyscallError> {
    if max_events == 0 || max_events > i32::MAX as usize / EPOLL_EVENT_BYTES {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(max_events * EPOLL_EVENT_BYTES)
}

/// `events` as the packed user array epoll_wait returns.
fn epoll_events_to_bytes(events: &[crate::net::epoll::EpollEvent]) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(events.len() * EPOLL_EVENT_BYTES);
    for ev in events {
        let (flags, data) = (ev.events, ev.data);
        out.extend_from_slice(&flags.to_ne_bytes());
        out.extend_from_slice(&data.to_ne_bytes());
    }
    out
}

/// getrandom syscall -- fills user buffer with cryptographically secure random
/// bytes.
///
/// # Arguments
/// - `buf_ptr`: User-space buffer to fill.
/// - `buflen`: Number of bytes to generate.
/// - `flags`: 0 for blocking (always succeeds), GRND_NONBLOCK (1) for
///   non-blocking.
fn sys_getrandom(buf_ptr: usize, buflen: usize, _flags: usize) -> SyscallResult {
    if buflen == 0 {
        return Ok(0);
    }
    // Cap at 256 bytes per call to avoid holding the RNG lock too long
    let len = buflen.min(256);
    validate_user_buffer(buf_ptr, len)?;

    let rng = crate::crypto::random::get_random();
    // Filled in a kernel buffer and copied out (N-43).
    let mut buf = [0u8; 256];
    rng.fill_bytes(&mut buf[..len])
        .map_err(|_| SyscallError::IoError)?;
    userspace::write_user_bytes(buf_ptr, &buf[..len])?;
    // The random bytes are not left on the kernel stack.
    buf.fill(0);
    Ok(len)
}

/// getdents64 syscall -- read directory entries in Linux struct linux_dirent64
/// format.
///
/// musl's readdir() uses getdents64 to read directory contents. Each entry is:
///   d_ino (u64), d_off (u64), d_reclen (u16), d_type (u8), d_name[...]
///
/// # Arguments
/// - `fd`: Directory file descriptor.
/// - `buf_ptr`: User-space buffer for dirent64 entries.
/// - `buf_size`: Size of the buffer in bytes.
///
/// # Returns
/// Number of bytes written to buf, or 0 when no more entries. A buffer too
/// small for the next entry is EINVAL.
fn sys_getdents64(fd: usize, buf_ptr: usize, buf_size: usize) -> SyscallResult {
    if buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(buf_ptr, buf_size)?;

    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file_table = proc.file_table.lock();
    let file_desc = file_table.get(fd).ok_or(SyscallError::BadFileDescriptor)?;

    let entries = file_desc
        .node
        .readdir()
        .map_err(|_| SyscallError::IoError)?;

    let pos = file_desc.tell();
    if pos >= entries.len() {
        return Ok(0);
    }

    // Records are built in a kernel buffer and copied out once (N-43).
    let (out, idx) = build_dirents64(&entries, pos, buf_size)?;

    userspace::write_user_bytes(buf_ptr, &out)?;

    // Advance file position
    if idx > pos {
        let _ = file_desc.seek(crate::fs::SeekFrom::Start(idx));
    }

    Ok(out.len())
}

/// `d_type` of a `linux_dirent64` for a node of type `node_type`.
fn dirent64_type(node_type: crate::fs::NodeType) -> u8 {
    match node_type {
        crate::fs::NodeType::File => 8,        // DT_REG
        crate::fs::NodeType::Directory => 4,   // DT_DIR
        crate::fs::NodeType::CharDevice => 2,  // DT_CHR
        crate::fs::NodeType::BlockDevice => 6, // DT_BLK
        crate::fs::NodeType::Symlink => 10,    // DT_LNK
        crate::fs::NodeType::Pipe => 1,        // DT_FIFO
        crate::fs::NodeType::Socket => 12,     // DT_SOCK
    }
}

/// Build `linux_dirent64` records for `entries[pos..]`, as many as fit in
/// `buf_size` bytes. Returns the records and the index of the first entry
/// not included (no records at all past the last entry), or EINVAL if not
/// even the first remaining record fits.
fn build_dirents64(
    entries: &[crate::fs::DirEntry],
    pos: usize,
    buf_size: usize,
) -> Result<(alloc::vec::Vec<u8>, usize), SyscallError> {
    let mut offset = 0usize;
    let mut idx = pos;
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();

    while idx < entries.len() {
        let entry = &entries[idx];
        let name_bytes = entry.name.as_bytes();
        // d_ino(8) + d_off(8) + d_reclen(2) + d_type(1) + name + NUL
        let reclen_unaligned = 8 + 8 + 2 + 1 + name_bytes.len() + 1;
        // Align to 8 bytes
        let reclen = (reclen_unaligned + 7) & !7;

        if offset + reclen > buf_size {
            break;
        }

        // d_ino (use inode from entry, default 1)
        let ino = if entry.inode == 0 {
            (idx + 1) as u64
        } else {
            entry.inode
        };
        out.extend_from_slice(&ino.to_ne_bytes());
        // d_off: the directory position of the next entry. Positions are
        // entry indices (see the seek in sys_getdents64), and musl's
        // telldir/seekdir hand d_off to lseek; this used to be the byte
        // offset of the next record in this buffer.
        out.extend_from_slice(&((idx + 1) as u64).to_ne_bytes());
        // d_reclen
        out.extend_from_slice(&(reclen as u16).to_ne_bytes());
        // d_type
        out.push(dirent64_type(entry.node_type));
        // d_name (NUL-terminated), then zero padding to reclen
        out.extend_from_slice(name_bytes);
        out.resize(offset + reclen, 0);

        offset += reclen;
        idx += 1;
    }
    if out.is_empty() && pos < entries.len() {
        // Linux: EINVAL when the buffer cannot hold the next record. An
        // empty result here used to read as end of directory, so a short
        // buffer silently truncated the listing.
        return Err(SyscallError::InvalidArgument);
    }
    Ok((out, idx))
}

/// prlimit64 syscall -- get/set resource limits for a process.
///
/// Combines getrlimit and setrlimit in one call. musl uses this for both.
///
/// # Arguments
/// - `pid`: Process ID (0 = current process).
/// - `resource`: RLIMIT_* constant.
/// - `new_rlim_ptr`: Pointer to new Rlimit (0 = don't set).
/// - `old_rlim_ptr`: Pointer to receive old Rlimit (0 = don't get).
fn sys_prlimit64(
    _pid: usize,
    resource: usize,
    new_rlim_ptr: usize,
    old_rlim_ptr: usize,
) -> SyscallResult {
    // Get old limits if requested
    if old_rlim_ptr != 0 {
        memory::sys_getrlimit(resource, old_rlim_ptr)?;
    }

    // Set new limits if requested
    if new_rlim_ptr != 0 {
        memory::sys_setrlimit(resource, new_rlim_ptr)?;
    }

    Ok(0)
}

/// fchmodat syscall -- chmod relative to a directory fd.
///
/// # Arguments
/// - `dirfd`: Directory fd (AT_FDCWD for CWD-relative).
/// - `path_ptr`: Path to the file.
/// - `mode`: Permission bits.
fn sys_fchmodat(dirfd: usize, path_ptr: usize, mode: usize) -> SyscallResult {
    let rel_path = filesystem::read_user_path(path_ptr)?;
    let abs_path = filesystem::resolve_at_path(dirfd, &rel_path)?;

    let vfs = filesystem::vfs()?;
    let node = vfs
        .resolve_path(&abs_path)
        .map_err(filesystem::map_resolve_err)?;
    filesystem::require_owner_or_root(&node)?;
    let perms = crate::fs::Permissions::from_mode(mode as u32);
    node.chmod(perms)
        .map_err(|_| SyscallError::InvalidArgument)?;
    Ok(0)
}

/// fchownat syscall -- chown relative to a directory fd.
///
/// # Arguments
/// - `dirfd`: Directory fd (AT_FDCWD for CWD-relative).
/// - `path_ptr`: Path to the file.
/// - `uid`: User ID.
/// - `gid`: Group ID.
/// - `flags`: AT_SYMLINK_NOFOLLOW, etc.
fn sys_fchownat(
    dirfd: usize,
    path_ptr: usize,
    uid: usize,
    gid: usize,
    flags: usize,
) -> SyscallResult {
    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
    let rel_path = filesystem::read_user_path(path_ptr)?;
    let abs_path = filesystem::resolve_at_path(dirfd, &rel_path)?;
    let vfs = filesystem::vfs()?;
    let node = if flags & AT_SYMLINK_NOFOLLOW != 0 {
        vfs.resolve_path_no_follow(&abs_path)
    } else {
        vfs.resolve_path(&abs_path)
    }
    .map_err(filesystem::map_resolve_err)?;
    // Same rules as chown (root only); this used to report success and
    // change nothing.
    filesystem::chown_node(&node, uid, gid)
}

/// linkat syscall -- create hard link relative to directory fds.
///
/// # Arguments
/// - `olddirfd`: Directory fd for oldpath.
/// - `oldpath_ptr`: Source path.
/// - `newdirfd`: Directory fd for newpath.
/// - `newpath_ptr`: Link destination path.
/// - `flags`: AT_SYMLINK_FOLLOW, etc.
fn sys_linkat(
    olddirfd: usize,
    oldpath_ptr: usize,
    newdirfd: usize,
    newpath_ptr: usize,
    _flags: usize,
) -> SyscallResult {
    let old_rel = filesystem::read_user_path(oldpath_ptr)?;
    let new_rel = filesystem::read_user_path(newpath_ptr)?;
    let old_abs = filesystem::resolve_at_path(olddirfd, &old_rel)?;
    let new_abs = filesystem::resolve_at_path(newdirfd, &new_rel)?;
    filesystem::require_dir_write(&new_abs)?;

    let vfs = filesystem::vfs()?;

    // Resolve old path to get target node
    let target = vfs
        .resolve_path(&old_abs)
        .map_err(filesystem::map_resolve_err)?;

    // Split new path into parent + name, create link in parent
    let (parent_path, link_name) = filesystem::split_path(&new_abs)?;
    let parent = vfs
        .resolve_path(&parent_path)
        .map_err(filesystem::map_resolve_err)?;
    parent
        .link(&link_name, target)
        .map_err(|_| SyscallError::InvalidArgument)?;
    Ok(0)
}

/// symlinkat syscall -- create symlink relative to a directory fd.
///
/// # Arguments
/// - `target_ptr`: Symlink target (what it points to).
/// - `newdirfd`: Directory fd for linkpath.
/// - `linkpath_ptr`: Path where symlink is created.
fn sys_symlinkat(target_ptr: usize, newdirfd: usize, linkpath_ptr: usize) -> SyscallResult {
    let target = filesystem::read_user_path(target_ptr)?;
    let link_rel = filesystem::read_user_path(linkpath_ptr)?;
    let link_abs = filesystem::resolve_at_path(newdirfd, &link_rel)?;
    filesystem::require_dir_write(&link_abs)?;

    let vfs = filesystem::vfs()?;

    // Split link path into parent + name, create symlink in parent
    let (parent_path, link_name) = filesystem::split_path(&link_abs)?;
    let parent = vfs
        .resolve_path(&parent_path)
        .map_err(filesystem::map_resolve_err)?;
    let node = parent
        .symlink(&link_name, &target)
        .map_err(|_| SyscallError::InvalidArgument)?;
    filesystem::own_new_node(&node);
    Ok(0)
}

/// readlinkat syscall -- read symlink target relative to a directory fd.
///
/// # Arguments
/// - `dirfd`: Directory fd (AT_FDCWD for CWD-relative).
/// - `path_ptr`: Path to the symlink.
/// - `buf_ptr`: User buffer for the target string.
/// - `buf_size`: Size of the buffer.
fn sys_readlinkat(dirfd: usize, path_ptr: usize, buf_ptr: usize, buf_size: usize) -> SyscallResult {
    if buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let rel_path = filesystem::read_user_path(path_ptr)?;
    let abs_path = filesystem::resolve_at_path(dirfd, &rel_path)?;
    validate_user_buffer(buf_ptr, buf_size)?;

    let vfs = filesystem::vfs()?;

    // readlinkat must not follow the final symlink
    let node = vfs
        .resolve_path_no_follow(&abs_path)
        .map_err(filesystem::map_resolve_err)?;
    let target = node.readlink().map_err(|_| SyscallError::InvalidArgument)?;

    let bytes = target.as_bytes();
    let copy_len = bytes.len().min(buf_size);
    userspace::write_user_bytes(buf_ptr, &bytes[..copy_len])?;
    Ok(copy_len)
}

/// memfd_create syscall -- create anonymous memory-backed file descriptor.
///
/// Returns an fd backed by anonymous memory. Used by Wayland for shared
/// buffers and by various libraries for temporary file-like objects.
///
/// # Arguments
/// - `name_ptr`: User-space pointer to name string (for debugging).
/// - `flags`: MFD_CLOEXEC (0x01), MFD_ALLOW_SEALING (0x02).
fn sys_memfd_create(_name_ptr: usize, _flags: usize) -> SyscallResult {
    // Create an anonymous memory region as a pseudo-fd via eventfd's
    // infrastructure (counter=0, non-blocking). This provides a valid fd
    // that can be mmap'd. In a full implementation this would use a
    // dedicated memfd subsystem with sealing support.
    crate::fs::eventfd::eventfd_create(0, crate::fs::eventfd::EFD_NONBLOCK)
}

/// set_tid_address syscall -- register the calling thread's clear_child_tid
/// pointer.
///
/// musl calls this during `__libc_start_main()` to register a pointer that
/// the kernel will zero and futex-wake when the thread exits. This is how
/// `pthread_join()` works: the joining thread does `futex_wait(tid_ptr)`,
/// and when the target thread exits, the kernel clears `*tid_ptr` and
/// issues `FUTEX_WAKE` to unblock the joiner.
///
/// # Arguments
/// - `tidptr`: User-space address where the kernel will write 0 on thread exit,
///   then futex-wake any waiters.
///
/// # Returns
/// The calling thread's TID.
fn sys_set_tid_address(tidptr: usize) -> SyscallResult {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;

    if tidptr != 0 {
        validate_user_pointer(tidptr, 4)?;
    }

    // Store on both the process PCB (for process-level tracking) and the
    // current thread's clear_tid field (which exit_thread() actually reads
    // to perform the zero-and-futex-wake on thread termination).
    proc.set_clear_child_tid(tidptr);
    if let Some(thread) = crate::process::current_thread() {
        thread
            .clear_tid
            .store(tidptr, core::sync::atomic::Ordering::Release);
    }

    // Return the caller's TID (Linux returns the thread's tid, but in
    // VeridianOS single-threaded processes use pid as tid)
    let tid = crate::process::current_thread()
        .map(|t| t.tid.0 as usize)
        .unwrap_or(proc.pid.0 as usize);
    Ok(tid)
}

/// set_robust_list syscall -- register robust futex list head for cleanup on
/// abnormal thread termination.
///
/// musl calls this during thread initialization. If a thread holding a
/// robust futex dies, the kernel walks the list and marks the futexes as
/// owner-died (FUTEX_OWNER_DIED) so waiting threads can recover.
///
/// # Arguments
/// - `head_ptr`: Pointer to `struct robust_list_head` in user space.
/// - `len`: Size of the structure (must match kernel expectation).
///
/// # Returns
/// 0 on success.
fn sys_set_robust_list(head_ptr: usize, len: usize) -> SyscallResult {
    // Expected size: 3 * sizeof(void*) = 24 bytes on 64-bit
    if len != 24 {
        return Err(SyscallError::InvalidArgument);
    }
    if head_ptr != 0 {
        validate_user_pointer(head_ptr, len)?;
    }

    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    proc.set_robust_list(head_ptr);
    Ok(0)
}

/// clock_nanosleep syscall -- sleep with clock selection.
///
/// Linux ABI: `clock_nanosleep(clockid, flags, request, remain)`
/// musl maps Linux 230 -> VeridianOS 354.
///
/// We ignore clockid (always use monotonic) and flags (TIMER_ABSTIME not
/// supported), delegating to the existing nanosleep implementation.
fn sys_clock_nanosleep(
    _clockid: usize,
    _flags: usize,
    req_ptr: usize,
    rem_ptr: usize,
) -> SyscallResult {
    // Delegate to nanosleep -- ignores clockid and flags for now.
    // TIMER_ABSTIME (flags=1) would require reading the clock and computing
    // relative sleep, but for MVP this is acceptable.
    time::sys_nanosleep(req_ptr, rem_ptr)
}

/// sendmsg syscall -- sends data with optional ancillary data (SCM_RIGHTS).
///
/// arg1=socket_fd, arg2=msghdr_ptr, arg3=flags.
///
/// The msghdr struct layout (matching C struct msghdr):
///   - msg_name (ptr), msg_namelen (u32)
///   - msg_iov (ptr to iovec array), msg_iovlen (i32)
///   - msg_control (ptr to cmsghdr), msg_controllen (u32)
///   - msg_flags (i32)
///
/// For SCM_RIGHTS: msg_control points to a cmsghdr with cmsg_level=SOL_SOCKET,
/// cmsg_type=SCM_RIGHTS, followed by an array of i32 file descriptors.
fn sys_sendmsg(socket_fd: usize, msghdr_ptr: usize, _flags: usize) -> SyscallResult {
    // The msghdr is copied in whole through the fault-tolerant reader
    // (N-43). In usize words: msg_name = 0, msg_namelen = 1, msg_iov = 2,
    // msg_iovlen = 3, msg_control = 4, msg_controllen = 5.
    let hdr: [usize; 7] = userspace::read_user(msghdr_ptr)?;
    let (iov_ptr, iov_len, control_ptr, control_len) = (hdr[2], hdr[3], hdr[4], hdr[5]);
    if iov_len > IOV_MAX_MSG {
        return Err(SyscallError::InvalidArgument);
    }

    // Gather data from iovec array
    let mut data = alloc::vec::Vec::new();
    if iov_len > 0 && iov_ptr != 0 {
        for i in 0..iov_len {
            let [base, len]: [usize; 2] = userspace::read_user_index(iov_ptr, i)?;
            if len > 0 && base != 0 {
                // Bounded in total, so a sender cannot make the kernel
                // buffer an arbitrary amount (N-43).
                let room = userspace::MAX_USER_MESSAGE.saturating_sub(data.len());
                data.extend_from_slice(&userspace::read_user_vec(base, len, room)?);
            }
        }
    }

    // Parse ancillary data for SCM_RIGHTS: the passed fds must be open in
    // the sender's table, and what travels is the open files themselves.
    let rights = if control_len >= CMSGHDR_SIZE && control_ptr != 0 {
        validate_user_buffer(control_ptr, control_len)?;
        let fds = parse_scm_rights(control_ptr, control_len)?;
        if fds.is_empty() {
            None
        } else {
            Some(files_for_fds(&fds)?)
        }
    } else {
        None
    };

    with_socket_fd(socket_fd, |s| match s.handle() {
        // INET sockets cannot pass files.
        SocketHandle::Inet(_) if rights.is_some() => Err(SyscallError::InvalidArgument),
        _ => s.send(&data, rights).map_err(socket_err),
    })?
}

/// An fd from an SCM_RIGHTS array as a table index; a negative fd is EBADF.
fn scm_fd_index(fd: i32) -> Result<usize, SyscallError> {
    usize::try_from(fd).map_err(|_| SyscallError::BadFileDescriptor)
}

/// Look up each fd in the caller's table (EBADF if any is negative or not
/// open).
fn files_for_fds(fds: &[i32]) -> Result<crate::net::unix_socket::ScmRights, SyscallError> {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let table = process.file_table.lock();
    let files = fds
        .iter()
        .map(|&fd| {
            table
                .get(scm_fd_index(fd)?)
                .ok_or(SyscallError::BadFileDescriptor)
        })
        .collect::<Result<alloc::vec::Vec<_>, _>>()?;
    // A Unix socket queued (directly or via other sockets) in its own
    // receive buffer keeps itself alive after every fd is closed -- the
    // cycle Linux needs a garbage collector for. Without one, passing Unix
    // sockets is refused, which rules such cycles out.
    let passes_unix_socket = files.iter().any(|f| {
        f.node
            .as_any()
            .and_then(|a| a.downcast_ref::<SocketNode>())
            .is_some_and(|s| matches!(s.handle(), SocketHandle::Unix(_)))
    });
    if passes_unix_socket {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(crate::net::unix_socket::ScmRights { files })
}

/// Linux x86_64 / LP64 `struct cmsghdr`: `size_t cmsg_len` at 0,
/// `int cmsg_level` at 8, `int cmsg_type` at 12, data at 16.
const CMSGHDR_SIZE: usize = 16;
const SOL_SOCKET_LEVEL: i32 = 1;
const SCM_RIGHTS_TYPE: i32 = 1;
/// Most fds accepted in one SCM_RIGHTS message (Linux: SCM_MAX_FD = 253).
const SCM_MAX_FDS: usize = 16;

/// Parse the fd array of the first control message at `control_ptr`
/// (`control_len` bytes of user memory). `cmsg_len` comes from user memory
/// and is checked against `control_len` before anything past the header is
/// read.
///
/// Mirrors Linux `__scm_send`/`scm_fp_copy` (review of the v0.26.0 stack,
/// PR #10): a `cmsg_len` shorter than the header or past the buffer is
/// EINVAL; a message for another level is skipped (empty result); an
/// unsupported SOL_SOCKET type (including SCM_CREDENTIALS, not modelled) is
/// EINVAL; more than `SCM_MAX_FDS` fds is EINVAL. Negative fds are returned
/// as-is so that `files_for_fds` fails the send with EBADF -- they used to
/// be dropped silently. An empty result means "no rights to pass".
fn parse_scm_rights(
    control_ptr: usize,
    control_len: usize,
) -> Result<alloc::vec::Vec<i32>, SyscallError> {
    if control_len < CMSGHDR_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    let mut hdr = [0u8; CMSGHDR_SIZE];
    userspace::read_user_bytes(control_ptr, &mut hdr)?;
    let fd_count = scm_rights_fd_count(&hdr, control_len)?;
    let mut raw = [0u8; 4 * SCM_MAX_FDS];
    let raw = &mut raw[..4 * fd_count];
    // CMSGHDR_SIZE + 4 * fd_count <= cmsg_len <= control_len.
    userspace::read_user_bytes(control_ptr + CMSGHDR_SIZE, raw)?;
    Ok(decode_scm_fds(raw))
}

/// The number of fds announced by the cmsghdr `hdr` at the start of a
/// `control_len`-byte control buffer, with the checks `parse_scm_rights`
/// documents. 0 means nothing to pass (another level, or an empty array).
fn scm_rights_fd_count(
    hdr: &[u8; CMSGHDR_SIZE],
    control_len: usize,
) -> Result<usize, SyscallError> {
    let cmsg_len = u64::from_ne_bytes([
        hdr[0], hdr[1], hdr[2], hdr[3], hdr[4], hdr[5], hdr[6], hdr[7],
    ]);
    let level = i32::from_ne_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
    let kind = i32::from_ne_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
    let cmsg_len = usize::try_from(cmsg_len).map_err(|_| SyscallError::InvalidArgument)?;
    if cmsg_len < CMSGHDR_SIZE || cmsg_len > control_len {
        return Err(SyscallError::InvalidArgument);
    }
    if level != SOL_SOCKET_LEVEL {
        return Ok(0);
    }
    if kind != SCM_RIGHTS_TYPE {
        return Err(SyscallError::InvalidArgument);
    }
    let fd_count = (cmsg_len - CMSGHDR_SIZE) / 4;
    if fd_count > SCM_MAX_FDS {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(fd_count)
}

/// The native-endian `int` fds of an SCM_RIGHTS payload (a trailing
/// partial fd is ignored).
fn decode_scm_fds(raw: &[u8]) -> alloc::vec::Vec<i32> {
    raw.chunks_exact(4)
        .map(|c| i32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// How many fds a `control_len`-byte control buffer at `control_ptr` can
/// report in one SCM_RIGHTS message (0 without a usable buffer).
fn scm_rights_room(control_ptr: usize, control_len: usize) -> usize {
    if control_ptr != 0 && control_len >= CMSGHDR_SIZE {
        (control_len - CMSGHDR_SIZE) / 4
    } else {
        0
    }
}

/// One SCM_RIGHTS cmsghdr carrying `fds`, laid out as `parse_scm_rights`
/// reads it; `cmsg_len` is the whole message length.
fn scm_rights_cmsg(fds: &[u32]) -> alloc::vec::Vec<u8> {
    let needed = CMSGHDR_SIZE + fds.len() * 4;
    let mut cmsg = alloc::vec::Vec::with_capacity(needed);
    cmsg.extend_from_slice(&(needed as u64).to_ne_bytes());
    cmsg.extend_from_slice(&SOL_SOCKET_LEVEL.to_ne_bytes());
    cmsg.extend_from_slice(&SCM_RIGHTS_TYPE.to_ne_bytes());
    for &fd in fds {
        cmsg.extend_from_slice(&(fd as i32).to_ne_bytes());
    }
    cmsg
}

/// recvmsg syscall -- receives data with optional ancillary data (SCM_RIGHTS).
///
/// arg1=socket_fd, arg2=msghdr_ptr, arg3=flags.
///
/// On return, if SCM_RIGHTS fds were received, they are written into the
/// msg_control buffer as a cmsghdr.
fn sys_recvmsg(socket_fd: usize, msghdr_ptr: usize, _flags: usize) -> SyscallResult {
    // The msghdr is copied in whole through the fault-tolerant reader
    // (N-43). In usize words: msg_name = 0, msg_namelen = 1, msg_iov = 2,
    // msg_iovlen = 3, msg_control = 4, msg_controllen = 5.
    let hdr: [usize; 7] = userspace::read_user(msghdr_ptr)?;
    let (iov_ptr, iov_len, control_ptr, control_len) = (hdr[2], hdr[3], hdr[4], hdr[5]);
    if iov_len > IOV_MAX_MSG {
        return Err(SyscallError::InvalidArgument);
    }

    let iov_at = |i| userspace::read_user_index::<[usize; 2]>(iov_ptr, i);

    // Calculate total receive buffer size from iovec
    let total_buf_len = if iov_len > 0 && iov_ptr != 0 {
        iovs_total_len(iov_len, iov_at)?
    } else {
        0
    };

    // Allocate a temporary kernel buffer to receive into
    let mut recv_buf = alloc::vec![0u8; total_buf_len.min(65536)];

    // Receive from socket
    let (received, rights) =
        with_socket_fd(socket_fd, |s| s.recv(&mut recv_buf))?.map_err(socket_err)?;

    // Scatter received data into iovec buffers
    if iov_len > 0 && iov_ptr != 0 {
        scatter_iovs(
            &recv_buf[..received],
            iov_len,
            iov_at,
            userspace::write_user_bytes,
        )?;
    }

    // Passed files become new fds in the receiver's own table -- only as
    // many as the control buffer can report; the rest are dropped (closed),
    // as Linux does when it truncates the control message.
    let mut wrote_control = false;
    let mut sent_fds = 0usize;
    let mut delivered_fds = 0usize;
    if let Some(scm) = rights {
        sent_fds = scm.files.len();
        let room = scm_rights_room(control_ptr, control_len);
        let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let table = process.file_table.lock();
        let mut fds = alloc::vec::Vec::with_capacity(scm.files.len().min(room));
        for file in scm.files.into_iter().take(room) {
            match table.open(file) {
                Ok(fd) => fds.push(fd as u32),
                Err(_) => break,
            }
        }
        if !fds.is_empty() {
            wrote_control = write_scm_rights(control_ptr, control_len, &fds, msghdr_ptr);
            if wrote_control {
                delivered_fds = fds.len();
            } else {
                // The receiver could never learn these fds: undo.
                for &fd in &fds {
                    table.close_on_rollback(fd as usize, "recvmsg");
                }
            }
        }
    }
    if !wrote_control {
        // No control data: msg_controllen = 0 so the caller does not parse
        // stale bytes.
        userspace::write_user::<usize>(msghdr_ptr + 5 * core::mem::size_of::<usize>(), 0)?;
    }
    // msg_flags is always written, so the caller never reads back what it
    // left there, and carries MSG_CTRUNC when fds were dropped (review of
    // the v0.26.0 stack, PR #10).
    userspace::write_user::<i32>(
        msghdr_ptr + MSGHDR_FLAGS_OFFSET,
        recvmsg_flags(sent_fds, delivered_fds),
    )?;

    Ok(received)
}

/// Most iovec entries one sendmsg/recvmsg accepts (Linux `UIO_MAXIOV`).
const IOV_MAX_MSG: usize = 1024;

/// `MSG_CTRUNC`: control data was discarded for lack of room.
const MSG_CTRUNC: i32 = 0x8;
/// Offset of `int msg_flags` in the LP64 `struct msghdr`.
const MSGHDR_FLAGS_OFFSET: usize = 48;

/// `msg_flags` for a recvmsg that received `sent` passed fds and could
/// deliver `delivered` of them.
fn recvmsg_flags(sent: usize, delivered: usize) -> i32 {
    if delivered < sent {
        MSG_CTRUNC
    } else {
        0
    }
}

/// Sum of the lengths of the `iov_count` iovec entries `iov_at` yields
/// (`[iov_base, iov_len]`), saturating.
fn iovs_total_len(
    iov_count: usize,
    mut iov_at: impl FnMut(usize) -> Result<[usize; 2], SyscallError>,
) -> Result<usize, SyscallError> {
    let mut total = 0usize;
    for i in 0..iov_count {
        let [_, len] = iov_at(i)?;
        total = total.saturating_add(len);
    }
    Ok(total)
}

/// Copy `data` into the iovec entries `iov_at` yields, in order, through
/// `write(base, bytes)`. Entries with a zero base or length are skipped,
/// and no entry is read once `data` is used up. Returns the bytes copied.
fn scatter_iovs(
    data: &[u8],
    iov_count: usize,
    mut iov_at: impl FnMut(usize) -> Result<[usize; 2], SyscallError>,
    mut write: impl FnMut(usize, &[u8]) -> Result<(), SyscallError>,
) -> Result<usize, SyscallError> {
    let mut offset = 0usize;
    for i in 0..iov_count {
        if offset >= data.len() {
            break;
        }
        let [base, len] = iov_at(i)?;
        if len > 0 && base != 0 {
            let copy_len = (data.len() - offset).min(len);
            write(base, &data[offset..offset + copy_len])?;
            offset += copy_len;
        }
    }
    Ok(offset)
}

/// Write SCM_RIGHTS fds into the user's msg_control buffer as one cmsghdr
/// and set msg_controllen. Returns false (writing nothing) if the buffer is
/// too small or not valid user memory.
fn write_scm_rights(
    control_ptr: usize,
    control_len: usize,
    fds: &[u32],
    msghdr_ptr: usize,
) -> bool {
    let needed = CMSGHDR_SIZE + fds.len() * 4;
    if needed > control_len {
        return false;
    }
    // Built in a kernel buffer and copied out with the fault-tolerant
    // routine; the raw unaligned stores this replaces faulted in the kernel
    // on an unmapped page (agy review of the v0.26.0 stack, PR #10).
    let cmsg = scm_rights_cmsg(fds);
    // msg_controllen is the sixth usize-sized field of the 56-byte msghdr.
    userspace::write_user_bytes(control_ptr, &cmsg).is_ok()
        && userspace::write_user::<usize>(msghdr_ptr + 5 * core::mem::size_of::<usize>(), needed)
            .is_ok()
}

/// Build a message from `len` bytes of user memory at `ptr`, choosing the
/// size tier (IPC-ARCH-02): up to `SmallMessage` size by value (a short
/// message is zero-padded), up to `MAX_BUFFERED_PAYLOAD` copied into a
/// kernel buffer, larger refused (use a shared region). The payload is
/// copied now, from the sender's address space; the old large path kept
/// the sender's virtual address and the receiver later copied from that
/// address in *its own* address space, into a buffer validated only for a
/// `SmallMessage`.
///
/// `capability` replaces the capability field of a small message when given
/// (send/call carry the validated capability, never one the sender wrote).
fn message_from_user(
    capability: Option<u64>,
    ptr: usize,
    len: usize,
) -> Result<Message, SyscallError> {
    use crate::ipc::message::BufferedMessage;

    match message_tier(len)? {
        MessageTier::Small => {
            let mut bytes = [0u8; SMALL_MESSAGE_BYTES];
            userspace::read_user_bytes(ptr, &mut bytes[..len])?;
            Ok(Message::Small(small_message_from_bytes(&bytes, capability)))
        }
        MessageTier::Buffered => {
            let mut payload = alloc::vec![0u8; len];
            userspace::read_user_bytes(ptr, &mut payload)?;
            BufferedMessage::new(capability.unwrap_or(0), 0, payload)
                .map(Message::Buffered)
                .ok_or(SyscallError::InvalidArgument)
        }
    }
}

/// `sizeof(SmallMessage)`: the largest message passed by value.
const SMALL_MESSAGE_BYTES: usize = core::mem::size_of::<SmallMessage>();

/// How `message_from_user` carries a message of a given length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageTier {
    /// By value, zero-padded to `SMALL_MESSAGE_BYTES`.
    Small,
    /// Copied into a kernel buffer.
    Buffered,
}

/// The tier for a `len`-byte message: empty and over-`MAX_BUFFERED_PAYLOAD`
/// messages are EINVAL (the latter need a shared region).
fn message_tier(len: usize) -> Result<MessageTier, SyscallError> {
    use crate::ipc::message::MAX_BUFFERED_PAYLOAD;

    match len {
        0 => Err(SyscallError::InvalidArgument),
        1..=SMALL_MESSAGE_BYTES => Ok(MessageTier::Small),
        _ if len > MAX_BUFFERED_PAYLOAD => Err(SyscallError::InvalidArgument),
        _ => Ok(MessageTier::Buffered),
    }
}

/// A small message from its bytes; `capability`, when given, replaces the
/// capability the sender wrote.
fn small_message_from_bytes(
    bytes: &[u8; SMALL_MESSAGE_BYTES],
    capability: Option<u64>,
) -> SmallMessage {
    // SAFETY: SmallMessage is repr(C) plain data (u64, u32, u32, [u64; 4])
    // without padding, so every byte pattern is a valid value;
    // read_unaligned imposes no alignment on the byte array.
    let mut msg: SmallMessage =
        unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const SmallMessage) };
    if let Some(cap) = capability {
        msg.capability = cap;
    }
    msg
}

/// Bytes of a `repr(C)` plain-data value.
fn pod_bytes<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: callers pass SmallMessage or MessageHeader, repr(C) structs of
    // integers without padding, so all size_of::<T>() bytes are initialized.
    unsafe {
        core::slice::from_raw_parts(value as *const T as *const u8, core::mem::size_of::<T>())
    }
}

/// Copy a received message into the user buffer at `buf` holding `cap`
/// bytes, through the validated user-copy routines. Returns the message's
/// full length; a buffered payload longer than the buffer is truncated
/// (the caller sees the length it missed, as with MSG_TRUNC). A large-
/// message descriptor's address belongs to the sender and is never read.
fn message_to_user(msg: &Message, buf: usize, cap: usize) -> SyscallResult {
    use crate::ipc::message::MessageHeader;

    const HEADER: usize = core::mem::size_of::<MessageHeader>();
    match msg {
        Message::Small(small) => {
            let bytes = pod_bytes(small);
            if cap < bytes.len() {
                return Err(SyscallError::InvalidArgument);
            }
            userspace::write_user_bytes(buf, bytes)?;
            Ok(bytes.len())
        }
        Message::Buffered(m) => {
            if cap < HEADER {
                return Err(SyscallError::InvalidArgument);
            }
            userspace::write_user_bytes(buf, pod_bytes(&m.header))?;
            let fits = m.payload.len().min(cap - HEADER);
            // The header write above already validated `buf` as user memory,
            // but the sum is still checked rather than trusted.
            let payload_at = buf
                .checked_add(HEADER)
                .ok_or(SyscallError::InvalidPointer)?;
            userspace::write_user_bytes(payload_at, &m.payload[..fits])?;
            Ok(HEADER + m.payload.len())
        }
        Message::Large(m) => {
            if cap < HEADER {
                return Err(SyscallError::InvalidArgument);
            }
            userspace::write_user_bytes(buf, pod_bytes(&m.header))?;
            Ok(HEADER)
        }
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

    // The capability names the destination endpoint. It used to be passed
    // on as the endpoint id -- and from there as the receiver PID -- so a
    // message went to whatever process had that number (IPC-INC-01).
    let endpoint_id = match cap_space.lookup_entry(cap_token) {
        Some((crate::cap::object::ObjectRef::Endpoint { endpoint }, _rights)) => endpoint.id(),
        _ => return Err(SyscallError::InvalidCapability),
    };
    drop(cap_space);

    // The message carries the capability that was actually validated,
    // never one the sender wrote into the struct.
    let message = message_from_user(Some(capability as u64), msg_ptr, msg_size)?;

    // Perform the actual send using the IPC sync module
    match sync_send(message, endpoint_id) {
        Ok(()) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// IPC receive system call
///
/// # Arguments
/// - endpoint: Endpoint to receive from
/// - buffer: Buffer to receive message into
/// - buffer_len: Its size in bytes (0 means `size_of::<SmallMessage>()`)
///
/// Returns the message length; see `message_to_user` for truncation.
fn sys_ipc_receive(endpoint: usize, buffer: usize, buffer_len: usize) -> SyscallResult {
    let buffer_len = if buffer_len == 0 {
        core::mem::size_of::<SmallMessage>()
    } else {
        buffer_len
    };
    validate_user_buffer(buffer, buffer_len)?;

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
    drop(cap_space);

    let message = sync_receive(endpoint as u64).map_err(SyscallError::from)?;
    message_to_user(&message, buffer, buffer_len)
}

/// IPC call (send and wait for reply)
fn sys_ipc_call(
    capability: usize,
    send_msg: usize,
    send_size: usize,
    recv_buf: usize,
    recv_size: usize,
) -> SyscallResult {
    if recv_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_buffer(recv_buf, recv_size)?;

    let message = message_from_user(Some(capability as u64), send_msg, send_size)?;
    let reply = sync_call(message, capability as u64).map_err(SyscallError::from)?;
    message_to_user(&reply, recv_buf, recv_size)
}

/// IPC reply to a previous call
fn sys_ipc_reply(caller: usize, msg_ptr: usize, msg_size: usize) -> SyscallResult {
    let message = message_from_user(None, msg_ptr, msg_size)?;

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

/// Largest IPC shared region (physically contiguous).
const MAX_SHARED_REGION_BYTES: usize = 64 * 1024 * 1024;

/// Share memory region via IPC
///
/// Creates a shared region of `size` bytes, initialized from the caller's
/// buffer at `addr` (or zeroed when `addr` is 0), and returns a memory
/// capability for it. The caller and any process it passes the capability
/// to map the region with `sys_ipc_map_memory` and see the same frames.
fn sys_ipc_share_memory(
    addr: usize,
    size: usize,
    permissions: usize,
    _target_pid: usize,
) -> SyscallResult {
    use crate::{
        cap::memory_integration::MemoryRights,
        ipc::shared_memory::{self, Permissions, SharedRegion},
    };

    if size == 0 || size > MAX_SHARED_REGION_BYTES {
        return Err(SyscallError::InvalidArgument);
    }
    if addr != 0 {
        validate_user_buffer(addr, size)?;
    }

    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;

    // Convert permissions to capability rights
    let mut rights = MemoryRights::MAP | MemoryRights::SHARE;
    if permissions & 0b001 != 0 {
        rights |= MemoryRights::READ;
    }
    if permissions & 0b010 != 0 {
        rights |= MemoryRights::WRITE;
    }
    if permissions & 0b100 != 0 {
        rights |= MemoryRights::EXECUTE;
    }

    let perms = match permissions & 0b111 {
        0b011 => Permissions::Write,
        0b100 => Permissions::Execute,
        0b101 => Permissions::ReadExecute,
        0b111 => Permissions::ReadWriteExecute,
        _ => Permissions::Read,
    };

    let region = SharedRegion::new(current_process.pid, size, perms)
        .map_err(|_| SyscallError::OutOfMemory)?;

    // Initial contents, copied a page at a time.
    if addr != 0 {
        let mut chunk = [0u8; 4096];
        let mut offset = 0;
        while offset < size {
            let len = (size - offset).min(chunk.len());
            userspace::read_user_bytes(addr + offset, &mut chunk[..len])?;
            region
                .write_at(offset, &chunk[..len])
                .map_err(|_| SyscallError::InvalidArgument)?;
            offset += len;
        }
    }

    let base = region.physical_base().as_usize();
    let region_size = region.size();
    let _region = shared_memory::register_region(region);

    let cap_space = current_process.capability_space.lock();
    match crate::cap::memory_integration::create_memory_capability(
        base,
        region_size,
        crate::cap::object::MemoryAttributes::normal(),
        rights,
        &cap_space,
    ) {
        Ok(cap) => Ok(cap.to_u64() as usize),
        Err(_) => {
            // Nobody can reach the region: drop it (frees its frames).
            if let Err(e) = shared_memory::unregister_region(base as u64) {
                // It was registered just above; failing here means the
                // registry changed underneath us.
                crate::println!(
                    "[IPC] share: unregistering region {:#x} failed: {:?}",
                    base,
                    e
                );
            }
            Err(SyscallError::OutOfMemory)
        }
    }
}

/// Map shared memory from another process
///
/// Maps the shared region named by `capability` -- its own frames, so every
/// process mapping it sees the same memory (IPC-INC-02) -- at `addr_hint`
/// (page-aligned free user space) or, when 0, where the kernel chooses.
/// Writable or executable mappings need the WRITE or EXECUTE right.
fn sys_ipc_map_memory(capability: usize, addr_hint: usize, flags: usize) -> SyscallResult {
    use crate::{
        cap::memory_integration::MemoryRights,
        ipc::shared_memory::{self, Permissions},
    };

    let current_process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_token = crate::cap::CapabilityToken::from_u64(capability as u64);

    let (object_ref, rights) = {
        let cap_space = current_process.capability_space.lock();
        if let Err(e) = crate::cap::memory_integration::check_map_permission(cap_token, &cap_space)
        {
            return Err(match e {
                crate::cap::CapError::InvalidCapability => SyscallError::InvalidArgument,
                crate::cap::CapError::InsufficientRights => SyscallError::PermissionDenied,
                _ => SyscallError::InvalidArgument,
            });
        }
        cap_space
            .lookup_entry(cap_token)
            .ok_or(SyscallError::InvalidArgument)?
    };

    let base_phys = match object_ref {
        crate::cap::object::ObjectRef::Memory { base, .. } => base,
        _ => return Err(SyscallError::InvalidArgument),
    };

    let write = flags & 0b010 != 0;
    let exec = flags & 0b100 != 0;
    if (write && !rights.contains(MemoryRights::WRITE))
        || (exec && !rights.contains(MemoryRights::EXECUTE))
    {
        return Err(SyscallError::PermissionDenied);
    }
    let perms = match (write, exec) {
        (true, true) => Permissions::ReadWriteExecute,
        (true, false) => Permissions::Write,
        (false, true) => Permissions::ReadExecute,
        (false, false) => Permissions::Read,
    };

    // Only shared regions can be mapped this way; this used to map fresh
    // frames for any memory capability, at an unvalidated address.
    let region =
        shared_memory::lookup_region(base_phys as u64).ok_or(SyscallError::InvalidArgument)?;
    let at = (addr_hint != 0).then(|| crate::mm::VirtualAddress::new(addr_hint as u64));
    let vaddr = region
        .map(current_process.pid, at, perms)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(vaddr.as_usize())
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

            // Event/timer notification fds (KDE/Wayland infrastructure)
            330 => Ok(Syscall::Getrandom),
            331 => Ok(Syscall::EventfdCreate),
            332 => Ok(Syscall::EventfdRead),
            333 => Ok(Syscall::EventfdWrite),
            334 => Ok(Syscall::TimerfdCreate),
            335 => Ok(Syscall::TimerfdSettime),
            336 => Ok(Syscall::TimerfdGettime),
            337 => Ok(Syscall::SignalfdCreate),
            338 => Ok(Syscall::SendMsg),
            339 => Ok(Syscall::RecvMsg),

            // musl libc compatibility
            340 => Ok(Syscall::Getdents64),
            341 => Ok(Syscall::Prlimit64),
            342 => Ok(Syscall::InotifyInit1),
            343 => Ok(Syscall::InotifyAddWatch),
            344 => Ok(Syscall::InotifyRmWatch),
            345 => Ok(Syscall::Madvise),
            346 => Ok(Syscall::Fchmodat),
            347 => Ok(Syscall::Fchownat),
            348 => Ok(Syscall::Linkat),
            349 => Ok(Syscall::Symlinkat),
            350 => Ok(Syscall::Readlinkat),
            351 => Ok(Syscall::MemfdCreate),
            352 => Ok(Syscall::SetTidAddress),
            353 => Ok(Syscall::SetRobustList),
            354 => Ok(Syscall::ClockNanosleep),
            355 => Ok(Syscall::Prctl),
            356 => Ok(Syscall::Flock),
            357 => Ok(Syscall::Tkill),
            358 => Ok(Syscall::Tgkill),
            359 => Ok(Syscall::Waitid),
            360 => Ok(Syscall::SigPending),

            _ => Err(()),
        }
    }
}

// ---------------------------------------------------------------------------
// POSIX Shared Memory syscall handlers
// ---------------------------------------------------------------------------

/// Read a null-terminated name string from user space (for shm/socket paths).
fn read_user_name(ptr: usize, max_len: usize) -> Result<alloc::string::String, SyscallError> {
    userspace::read_user_cstr(ptr, max_len)
}

/// `sizeof(struct sockaddr_un)`: a 2-byte family and a 108-byte path.
const SOCKADDR_UN_LEN: usize = 110;

/// The path in a `struct sockaddr_un` of `addr_len` bytes.
///
/// `sun_path` starts after the 2-byte family; it ends at the first NUL or
/// at `addr_len`, whichever is first (Linux accepts both). The family must
/// be AF_UNIX. Abstract names (leading NUL) and unnamed addresses are not
/// supported and are rejected. `bind` used to read the name from offset 0,
/// so every bind registered the family bytes as the path (review of the
/// v0.26.0 stack, PR #14).
fn parse_sockaddr_un(raw: &[u8]) -> Result<alloc::string::String, SyscallError> {
    if raw.len() <= 2 || raw.len() > SOCKADDR_UN_LEN {
        return Err(SyscallError::InvalidArgument);
    }
    if usize::from(u16::from_ne_bytes([raw[0], raw[1]])) != AF_UNIX {
        return Err(SyscallError::InvalidArgument);
    }
    let path = &raw[2..];
    let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
    if end == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    core::str::from_utf8(&path[..end])
        .map(alloc::string::String::from)
        .map_err(|_| SyscallError::InvalidArgument)
}

/// Copy a `struct sockaddr_un` of `addr_len` bytes from user memory and
/// return its path.
fn read_sockaddr_un(
    addr_ptr: usize,
    addr_len: usize,
) -> Result<alloc::string::String, SyscallError> {
    if addr_len <= 2 || addr_len > SOCKADDR_UN_LEN {
        return Err(SyscallError::InvalidArgument);
    }
    let mut raw = [0u8; SOCKADDR_UN_LEN];
    userspace::read_user_bytes(addr_ptr, &mut raw[..addr_len])?;
    parse_sockaddr_un(&raw[..addr_len])
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

use crate::net::socket_fd::{SocketHandle, SocketNode};

/// Run `f` on the socket behind `fd` in the caller's own file table.
/// Fails with EBADF for an unknown fd and ENOTSOCK for a non-socket.
pub(super) fn with_socket_fd<R>(
    fd: usize,
    f: impl FnOnce(&SocketNode) -> R,
) -> Result<R, SyscallError> {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    let node = file
        .node
        .as_any()
        .and_then(|a| a.downcast_ref::<SocketNode>())
        .ok_or(SyscallError::NotASocket)?;
    Ok(f(node))
}

/// Install a new socket in the caller's file table and return its fd. If
/// that fails the node is dropped, which closes the socket.
fn install_socket(handle: SocketHandle) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(SocketNode::new(handle));
    let file = alloc::sync::Arc::new(crate::fs::file::File::new(
        node,
        crate::fs::file::OpenFlags::read_write(),
    ));
    let fd = process
        .file_table
        .lock()
        .open(file)
        .map_err(|_| SyscallError::ResourceLimitExceeded);
    fd
}

/// Map a socket-layer error to a syscall error.
pub(super) fn socket_err(e: crate::error::KernelError) -> SyscallError {
    match e {
        crate::error::KernelError::WouldBlock => SyscallError::WouldBlock,
        crate::error::KernelError::BrokenPipe => SyscallError::BrokenPipe,
        _ => SyscallError::InvalidState,
    }
}

/// Read an INET address passed as (4 address bytes, big-endian port).
fn read_inet_addr(addr_ptr: usize) -> Result<crate::net::SocketAddr, SyscallError> {
    let mut raw = [0u8; 6];
    userspace::read_user_bytes(addr_ptr, &mut raw)?;
    Ok(inet_addr_from_bytes(&raw))
}

/// Decode an INET address passed as (4 address bytes, big-endian port).
fn inet_addr_from_bytes(raw: &[u8; 6]) -> crate::net::SocketAddr {
    crate::net::SocketAddr::v4(
        crate::net::Ipv4Address([raw[0], raw[1], raw[2], raw[3]]),
        u16::from_be_bytes([raw[4], raw[5]]),
    )
}

/// Convert user-space socket type to UnixSocketType.
fn to_unix_socket_type(
    sock_type: usize,
) -> Result<crate::net::unix_socket::UnixSocketType, SyscallError> {
    // Mask off SOCK_CLOEXEC (0x80000) and SOCK_NONBLOCK (0x800) flags
    // that musl passes alongside the base socket type.
    let base_type = sock_type & 0xF;
    match base_type {
        SOCK_STREAM => Ok(crate::net::unix_socket::UnixSocketType::Stream),
        SOCK_DGRAM => Ok(crate::net::unix_socket::UnixSocketType::Datagram),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// SYS_SOCKET_CREATE: Create a new socket and return an fd for it.
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
            let id = crate::net::unix_socket::socket_create(utype, pid)
                .map_err(|_| SyscallError::OutOfMemory)?;
            install_socket(SocketHandle::Unix(id))
        }
        AF_INET => {
            let sock_domain = crate::net::socket::SocketDomain::Inet;
            // Mask off SOCK_CLOEXEC/SOCK_NONBLOCK flags
            let base_type = sock_type & 0xF;
            let (sock_tp, proto) = match base_type {
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
            let id = crate::net::socket::create_socket(sock_domain, sock_tp, proto)
                .map_err(|_| SyscallError::OutOfMemory)?;
            install_socket(SocketHandle::Inet(id))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// SYS_SOCKET_BIND: Bind a socket to an address/path.
fn sys_socket_bind(fd: usize, addr_ptr: usize, addr_len: usize) -> SyscallResult {
    match with_socket_fd(fd, SocketNode::handle)? {
        SocketHandle::Inet(id) => {
            let addr = read_inet_addr(addr_ptr)?;
            crate::net::socket::with_socket_mut(id, |s| s.bind(addr))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidState)?;
            Ok(0)
        }
        SocketHandle::Unix(id) => {
            let path = read_sockaddr_un(addr_ptr, addr_len)?;
            crate::net::unix_socket::socket_bind(id, &path)
                .map(|()| 0)
                .map_err(|_| SyscallError::InvalidState)
        }
    }
}

/// SYS_SOCKET_LISTEN: Start listening on a bound socket.
fn sys_socket_listen(fd: usize, backlog: usize) -> SyscallResult {
    match with_socket_fd(fd, SocketNode::handle)? {
        SocketHandle::Inet(id) => {
            crate::net::socket::with_socket_mut(id, |s| s.listen(backlog))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidState)?;
            Ok(0)
        }
        SocketHandle::Unix(id) => crate::net::unix_socket::socket_listen(id, backlog)
            .map(|()| 0)
            .map_err(|_| SyscallError::InvalidState),
    }
}

/// SYS_SOCKET_CONNECT: Connect to a listening socket.
///
/// For Unix sockets, `addr_ptr` points to `struct sockaddr_un`:
///   `{ sa_family_t sun_family; char sun_path[108]; }`
/// The path starts at offset 2 (after the 2-byte family field).
fn sys_socket_connect(fd: usize, addr_ptr: usize, addr_len: usize) -> SyscallResult {
    match with_socket_fd(fd, SocketNode::handle)? {
        SocketHandle::Inet(id) => {
            let addr = read_inet_addr(addr_ptr)?;
            crate::net::socket::with_socket_mut(id, |s| s.connect(addr))
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(|_| SyscallError::InvalidState)?;
            Ok(0)
        }
        SocketHandle::Unix(id) => {
            let path = read_sockaddr_un(addr_ptr, addr_len)?;
            crate::net::unix_socket::socket_connect(id, &path)
                .map(|()| 0)
                // ENOENT: no socket bound at that path.
                .map_err(|_| SyscallError::ResourceNotFound)
        }
    }
}

/// SYS_SOCKET_ACCEPT: Accept a pending connection and return an fd for it.
///
/// Linux ABI: `accept4(fd, addr, addrlen_ptr, flags)`.
/// `addr_ptr` is optional (may be 0). When non-null, `addrlen_ptr` must
/// point at the buffer size: at most that many bytes of the peer address
/// (sockaddr_in, or an unnamed sockaddr_un for Unix sockets) are written,
/// and its full length is stored back in `*addrlen_ptr`.
fn sys_socket_accept(fd: usize, addr_ptr: usize, addrlen_ptr: usize) -> SyscallResult {
    match with_socket_fd(fd, SocketNode::handle)? {
        SocketHandle::Inet(id) => {
            let (new_sock, remote) = crate::net::socket::with_socket(id, |s| s.accept())
                .map_err(|_| SyscallError::InvalidState)?
                .map_err(socket_err)?;
            // NOTE (NET-INC-01): this registers a fresh socket of the same
            // kind rather than the accepted one, so the connection state is
            // lost; fixed with the TCP rework.
            let new_id = crate::net::socket::create_socket(
                new_sock.domain,
                new_sock.socket_type,
                new_sock.protocol,
            )
            .map_err(|_| SyscallError::OutOfMemory)?;
            let new_fd = install_socket(SocketHandle::Inet(new_id))?;
            let peer = network_ext_syscalls::sockaddr_in_bytes(&remote);
            finish_accept(new_fd, addr_ptr, addrlen_ptr, &peer)
        }
        SocketHandle::Unix(id) => {
            let (new_id, _connecting_id) =
                crate::net::unix_socket::socket_accept(id).map_err(socket_err)?;
            let new_fd = install_socket(SocketHandle::Unix(new_id))?;
            // The peer of an accepted Unix connection is reported unnamed:
            // sun_family only, length 2 (bound client paths are not
            // tracked).
            finish_accept(
                new_fd,
                addr_ptr,
                addrlen_ptr,
                &(AF_UNIX as u16).to_ne_bytes(),
            )
        }
    }
}

/// Report the peer address of an accepted connection and return its fd.
/// If the address cannot be written the caller would never learn about
/// the fd, so it is closed and the error returned, as Linux does.
fn finish_accept(new_fd: usize, addr_ptr: usize, addrlen_ptr: usize, peer: &[u8]) -> SyscallResult {
    if let Err(e) = copy_sockaddr_out(addr_ptr, addrlen_ptr, peer) {
        if let Some(p) = crate::process::current_process() {
            p.file_table.lock().close_on_rollback(new_fd, "accept");
        }
        return Err(e);
    }
    Ok(new_fd)
}

/// Copy a socket address out to user space with Linux `move_addr_to_user`
/// semantics: read `*addrlen` (a socklen_t, negative is EINVAL), copy at
/// most that many bytes of `addr`, then store the full length so the caller
/// can see truncation. `addr_ptr == 0` means the caller wants no address.
///
/// accept used to write a whole 16-byte sockaddr_in regardless of the
/// caller's buffer, overflowing a smaller one, and nothing for a Unix
/// socket (review of the v0.26.0 stack, PR #10). Both pointers go through
/// the fault-handled user-copy routines, so neither needs to be aligned.
fn copy_sockaddr_out(addr_ptr: usize, addrlen_ptr: usize, addr: &[u8]) -> Result<(), SyscallError> {
    if addr_ptr == 0 {
        return Ok(());
    }
    let len = userspace::read_user::<u32>(addrlen_ptr)? as i32;
    let len = usize::try_from(len).map_err(|_| SyscallError::InvalidArgument)?;
    userspace::write_user_bytes(addr_ptr, &addr[..len.min(addr.len())])?;
    userspace::write_user::<u32>(addrlen_ptr, addr.len() as u32)
}

/// SYS_SOCKET_SEND: Send data on a connected socket.
fn sys_socket_send(fd: usize, buf_ptr: usize, buf_len: usize) -> SyscallResult {
    let data = userspace::read_user_vec(buf_ptr, buf_len, userspace::MAX_USER_MESSAGE)?;
    with_socket_fd(fd, |s| s.send(&data, None))?.map_err(socket_err)
}

/// SYS_SOCKET_RECV: Receive data from a socket.
fn sys_socket_recv(fd: usize, buf_ptr: usize, buf_len: usize) -> SyscallResult {
    validate_user_buffer(buf_ptr, buf_len)?;
    // Received into a kernel buffer, then copied out fault-tolerantly (N-43).
    let mut kbuf = alloc::vec![0u8; buf_len.min(userspace::USER_COPY_CHUNK)];
    let n = with_socket_fd(fd, |s| s.recv(&mut kbuf))?
        .map(|(n, _)| n)
        .map_err(socket_err)?;
    userspace::write_user_bytes(buf_ptr, &kbuf[..n])?;
    Ok(n)
}

/// SYS_SOCKET_CLOSE: Close a socket fd. The socket itself closes when the
/// last open file referring to it goes away.
fn sys_socket_close(fd: usize) -> SyscallResult {
    with_socket_fd(fd, |_| ())?;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let result = process
        .file_table
        .lock()
        .close(fd)
        .map(|()| 0)
        .map_err(|_| SyscallError::BadFileDescriptor);
    result
}

/// The Unix socket type for `socketpair(domain, sock_type, protocol, sv)`:
/// AF_UNIX only, protocol 0 or PF_UNIX (as Linux accepts), and the type
/// mapped like socket() does. The type and protocol used to be ignored, so
/// a SOCK_DGRAM pair silently got stream semantics (review of the v0.26.0
/// stack, PR #10). SOCK_CLOEXEC / SOCK_NONBLOCK are masked off and not yet
/// honoured, as for socket().
fn socketpair_type(
    domain: usize,
    sock_type: usize,
    protocol: usize,
) -> Result<crate::net::unix_socket::UnixSocketType, SyscallError> {
    if domain != AF_UNIX || (protocol != 0 && protocol != AF_UNIX) {
        return Err(SyscallError::InvalidArgument);
    }
    to_unix_socket_type(sock_type)
}

/// SYS_SOCKET_PAIR: Create a connected socket pair.
///
/// # Arguments
/// - domain: AF_UNIX only
/// - sock_type: SOCK_STREAM or SOCK_DGRAM (plus flags)
/// - protocol: 0 or PF_UNIX
/// - result_ptr: user-space pointer to `int sv[2]`
fn sys_socket_pair(
    domain: usize,
    sock_type: usize,
    protocol: usize,
    result_ptr: usize,
) -> SyscallResult {
    let utype = socketpair_type(domain, sock_type, protocol)?;
    // Linux writes int sv[2] (two i32 values = 8 bytes).
    validate_user_buffer(result_ptr, 2 * core::mem::size_of::<i32>())?;

    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let (id_a, id_b) =
        crate::net::unix_socket::socketpair(utype, pid).map_err(|_| SyscallError::OutOfMemory)?;
    let fd_a = install_socket(SocketHandle::Unix(id_a));
    let fd_b = install_socket(SocketHandle::Unix(id_b));
    let (fd_a, fd_b) = match (fd_a, fd_b) {
        (Ok(a), Ok(b)) => (a, b),
        (Ok(a), Err(e)) | (Err(e), Ok(a)) => {
            if let Some(p) = crate::process::current_process() {
                p.file_table.lock().close_on_rollback(a, "socketpair");
            }
            return Err(e);
        }
        (Err(e), Err(_)) => return Err(e),
    };

    // int sv[2] through the fault-handled user copy: `sv` need not be
    // aligned or mapped. If it cannot be written the caller can never learn
    // the fds, so close them, as Linux does.
    if let Err(e) = userspace::write_user_slice::<i32>(result_ptr, &[fd_a as i32, fd_b as i32]) {
        if let Some(p) = crate::process::current_process() {
            let table = p.file_table.lock();
            table.close_on_rollback(fd_a, "socketpair");
            table.close_on_rollback(fd_b, "socketpair");
        }
        return Err(e);
    }
    Ok(0)
}

/// flock(2): a whole-file advisory lock on an open file (N-120).
///
/// Locks are keyed by the open node's identity, which is unique across
/// filesystems (inode numbers are not), and released when the open file is
/// closed for the last time or its process exits. A conflicting lock returns
/// EWOULDBLOCK even without LOCK_NB until flock waits on a wait queue (ADR
/// 0006, sprint D).
fn sys_flock(fd: usize, operation: usize) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    let op = operation as u32;
    match crate::fs::flock::flock(file.flock_key(), process.pid.0, op) {
        Ok(()) => {
            // The open file owns the lock: it is released when the file is
            // closed for the last time (File::drop), so a lock can never
            // outlive the node its key names (review of N-120).
            let owner = if op & !crate::fs::flock::LOCK_NB == crate::fs::flock::LOCK_UN {
                0
            } else {
                process.pid.0
            };
            file.flock_owner
                .store(owner, core::sync::atomic::Ordering::Release);
            Ok(0)
        }
        Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Numbers the musl remap patch relies on (review of the v0.26.0
    // stack, PR #8) ---

    #[test]
    fn missing_sixth_argument_is_an_error_not_zero() {
        // No syscall frame (host test, or an architecture that does not
        // capture it): the futex ops that need val3 must fail, not use 0.
        assert_eq!(syscall_arg6(), Err(SyscallError::NotImplemented));
    }

    #[test]
    fn sockaddr_un_path_starts_after_the_family() {
        let mut sa = alloc::vec::Vec::from((AF_UNIX as u16).to_ne_bytes());
        sa.extend_from_slice(b"/tmp/sock\0garbage");
        assert_eq!(parse_sockaddr_un(&sa).unwrap(), "/tmp/sock");
        // Without a NUL, addr_len bounds the path.
        assert_eq!(parse_sockaddr_un(&sa[..2 + 4]).unwrap(), "/tmp");
        // Unnamed, abstract, wrong family, oversized.
        assert!(parse_sockaddr_un(&sa[..2]).is_err());
        let mut abs = alloc::vec::Vec::from((AF_UNIX as u16).to_ne_bytes());
        abs.extend_from_slice(b"\0name");
        assert!(parse_sockaddr_un(&abs).is_err());
        let mut inet = alloc::vec::Vec::from(2u16.to_ne_bytes());
        inet.extend_from_slice(b"/tmp/sock\0");
        assert!(parse_sockaddr_un(&inet).is_err());
        assert!(parse_sockaddr_un(&[0u8; SOCKADDR_UN_LEN + 1]).is_err());
    }

    #[test]
    fn musl_remap_targets_have_the_expected_meaning() {
        // tools/cross/musl-patches/0001-veridian-syscall-remap.patch.
        // faccessat (Linux 269) is left unmapped: it must not be a native
        // number, so it reaches the faccessat fallback. It used to be
        // remapped to 347, native fchownat, which chowned the file.
        assert!(Syscall::try_from(269).is_err());
        assert!(linux_compat::is_faccessat(269));
        assert_eq!(Syscall::try_from(347), Ok(Syscall::Fchownat)); // Linux 260
        assert_eq!(Syscall::try_from(191), Ok(Syscall::FileFstatat)); // Linux 262
        assert_eq!(Syscall::try_from(260), Ok(Syscall::GetRlimit)); // Linux 97
        assert_eq!(Syscall::try_from(261), Ok(Syscall::SetRlimit)); // Linux 160
    }

    // --- SCM_RIGHTS control messages (Linux LP64 cmsghdr) ---

    fn cmsg(len: u64, level: i32, kind: i32, fds: &[i32]) -> alloc::vec::Vec<u8> {
        let mut b = alloc::vec::Vec::new();
        b.extend_from_slice(&len.to_ne_bytes());
        b.extend_from_slice(&level.to_ne_bytes());
        b.extend_from_slice(&kind.to_ne_bytes());
        for fd in fds {
            b.extend_from_slice(&fd.to_ne_bytes());
        }
        b
    }

    #[test]
    fn scm_rights_parses_linux_layout() {
        let b = cmsg(16 + 8, 1, 1, &[5, 7]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Ok(alloc::vec![5, 7])
        );
    }

    /// A negative fd is kept so files_for_fds fails the send with EBADF,
    /// instead of being dropped from the array (review of the v0.26.0
    /// stack, PR #10).
    #[test]
    fn scm_rights_keeps_negative_fds() {
        let b = cmsg(16 + 8, 1, 1, &[5, -1]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Ok(alloc::vec![5, -1])
        );
    }

    #[test]
    fn scm_rights_rejects_len_past_buffer() {
        // cmsg_len claims 4 fds but the validated buffer holds 1.
        let b = cmsg(16 + 16, 1, 1, &[5]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Err(SyscallError::InvalidArgument)
        );
        // Shorter than the header (Linux CMSG_OK fails: EINVAL).
        let b = cmsg(8, 1, 1, &[]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, 16),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn scm_rights_level_and_type() {
        // Linux skips control messages for other levels.
        let b = cmsg(20, 0, 1, &[5]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Ok(alloc::vec![])
        );
        // An unsupported SOL_SOCKET type is EINVAL (SCM_CREDENTIALS = 2).
        let b = cmsg(20, 1, 2, &[5]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn scm_rights_fd_count_limits() {
        // An empty SCM_RIGHTS passes nothing, as on Linux.
        let b = cmsg(16, 1, 1, &[]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Ok(alloc::vec![])
        );
        let fds = [3i32; SCM_MAX_FDS + 1];
        let b = cmsg((16 + 4 * fds.len()) as u64, 1, 1, &fds);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Err(SyscallError::InvalidArgument)
        );
    }

    fn cmsg_hdr(len: u64, level: i32, kind: i32) -> [u8; CMSGHDR_SIZE] {
        let mut h = [0u8; CMSGHDR_SIZE];
        h.copy_from_slice(&cmsg(len, level, kind, &[]));
        h
    }

    #[test]
    fn scm_rights_fd_count_from_header() {
        assert_eq!(scm_rights_fd_count(&cmsg_hdr(16 + 12, 1, 1), 64), Ok(3));
        // A trailing partial fd does not count.
        assert_eq!(scm_rights_fd_count(&cmsg_hdr(16 + 6, 1, 1), 64), Ok(1));
        // cmsg_len exactly at the buffer end is fine, one past is not.
        assert_eq!(scm_rights_fd_count(&cmsg_hdr(24, 1, 1), 24), Ok(2));
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(25, 1, 1), 24),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(15, 1, 1), 64),
            Err(SyscallError::InvalidArgument)
        );
        // Another level is skipped before its type is looked at.
        assert_eq!(scm_rights_fd_count(&cmsg_hdr(20, 41, 99), 64), Ok(0));
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(20, 1, 2), 64),
            Err(SyscallError::InvalidArgument)
        );
        let max = (16 + 4 * SCM_MAX_FDS) as u64;
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(max, 1, 1), 1024),
            Ok(SCM_MAX_FDS)
        );
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(max + 4, 1, 1), 1024),
            Err(SyscallError::InvalidArgument)
        );
        // A huge cmsg_len that the buffer length does not rule out is
        // still EINVAL (too many fds), not an overflow.
        assert_eq!(
            scm_rights_fd_count(&cmsg_hdr(u64::MAX, 1, 1), usize::MAX),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn scm_fds_decode_native_ints() {
        let mut raw = alloc::vec::Vec::new();
        for fd in [0i32, 9, -1, i32::MAX] {
            raw.extend_from_slice(&fd.to_ne_bytes());
        }
        raw.push(0xFF); // partial trailing fd
        assert_eq!(decode_scm_fds(&raw), alloc::vec![0, 9, -1, i32::MAX]);
        assert!(decode_scm_fds(&[]).is_empty());
    }

    #[test]
    fn scm_rights_room_counts_whole_fds() {
        assert_eq!(scm_rights_room(0x1000, 16), 0);
        assert_eq!(scm_rights_room(0x1000, 19), 0);
        assert_eq!(scm_rights_room(0x1000, 20), 1);
        assert_eq!(scm_rights_room(0x1000, 16 + 4 * 5 + 3), 5);
        // No buffer, or one shorter than a header.
        assert_eq!(scm_rights_room(0, 64), 0);
        assert_eq!(scm_rights_room(0x1000, 15), 0);
        assert_eq!(scm_rights_room(0x1000, 0), 0);
    }

    /// What recvmsg writes is what sendmsg parses.
    #[test]
    fn scm_rights_cmsg_round_trips() {
        let b = scm_rights_cmsg(&[3, 4, 10]);
        assert_eq!(b.len(), 16 + 12);
        assert_eq!(b, cmsg(28, 1, 1, &[3, 4, 10]));
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, b.len()),
            Ok(alloc::vec![3, 4, 10])
        );
        assert_eq!(scm_rights_cmsg(&[]), cmsg(16, 1, 1, &[]));
    }

    #[test]
    fn write_scm_rights_sets_controllen_or_writes_nothing() {
        // msghdr as 7 usize words; msg_controllen is word 5.
        let mut hdr = [usize::MAX; 7];
        let mut control = [0xEEu8; 32];
        let hdr_ptr = hdr.as_mut_ptr() as usize;
        assert!(write_scm_rights(
            control.as_mut_ptr() as usize,
            control.len(),
            &[7, 8],
            hdr_ptr
        ));
        assert_eq!(hdr[5], 24);
        assert_eq!(&control[..24], &cmsg(24, 1, 1, &[7, 8])[..]);
        assert!(control[24..].iter().all(|&b| b == 0xEE));

        // Too small: nothing written, msg_controllen untouched.
        let mut hdr = [usize::MAX; 7];
        let mut control = [0xEEu8; 23];
        assert!(!write_scm_rights(
            control.as_mut_ptr() as usize,
            control.len(),
            &[7, 8],
            hdr.as_mut_ptr() as usize
        ));
        assert_eq!(hdr[5], usize::MAX);
        assert!(control.iter().all(|&b| b == 0xEE));
    }

    #[test]
    fn scm_rights_rejects_short_control_buffer() {
        let b = cmsg(16, 1, 1, &[]);
        assert_eq!(
            parse_scm_rights(b.as_ptr() as usize, CMSGHDR_SIZE - 1),
            Err(SyscallError::InvalidArgument)
        );
    }

    // --- iovec gather/scatter ---

    fn iovs(entries: &[[usize; 2]]) -> impl FnMut(usize) -> Result<[usize; 2], SyscallError> + '_ {
        move |i| entries.get(i).copied().ok_or(SyscallError::InvalidPointer)
    }

    #[test]
    fn iovs_total_len_sums_and_saturates() {
        assert_eq!(iovs_total_len(0, iovs(&[])), Ok(0));
        assert_eq!(
            iovs_total_len(3, iovs(&[[1, 4], [0, 7], [2, 0]])),
            Ok(11),
            "a null base still counts, as before"
        );
        assert_eq!(
            iovs_total_len(2, iovs(&[[1, usize::MAX], [2, 5]])),
            Ok(usize::MAX)
        );
        // An unreadable entry fails the call.
        assert_eq!(
            iovs_total_len(2, iovs(&[[1, 4]])),
            Err(SyscallError::InvalidPointer)
        );
    }

    #[test]
    fn scatter_iovs_fills_in_order_and_skips_empty_entries() {
        let data: alloc::vec::Vec<u8> = (1..=10).collect();
        let mut writes = alloc::vec::Vec::new();
        let n = scatter_iovs(
            &data,
            4,
            iovs(&[[0x100, 3], [0, 5], [0x200, 0], [0x300, 100]]),
            |base, bytes| {
                writes.push((base, alloc::vec::Vec::from(bytes)));
                Ok(())
            },
        );
        assert_eq!(n, Ok(10));
        assert_eq!(
            writes,
            alloc::vec![
                (0x100, alloc::vec![1, 2, 3]),
                (0x300, alloc::vec![4, 5, 6, 7, 8, 9, 10]),
            ]
        );
    }

    #[test]
    fn scatter_iovs_stops_reading_entries_once_data_is_placed() {
        // Entry 1 is unreadable but never needed: 2 bytes fit in entry 0.
        let mut out = alloc::vec::Vec::new();
        assert_eq!(
            scatter_iovs(&[9, 8], 5, iovs(&[[0x100, 4]]), |_, b| {
                out.extend_from_slice(b);
                Ok(())
            }),
            Ok(2)
        );
        assert_eq!(out, [9, 8]);
        // Nothing received: no entry is read at all.
        assert_eq!(scatter_iovs(&[], 5, iovs(&[]), |_, _| Ok(())), Ok(0));
        // More data than room: the excess is dropped.
        assert_eq!(
            scatter_iovs(&[1, 2, 3], 1, iovs(&[[0x100, 2]]), |_, _| Ok(())),
            Ok(2)
        );
    }

    #[test]
    fn scatter_iovs_propagates_errors() {
        assert_eq!(
            scatter_iovs(&[1, 2, 3], 2, iovs(&[[0x100, 1]]), |_, _| Ok(())),
            Err(SyscallError::InvalidPointer)
        );
        assert_eq!(
            scatter_iovs(&[1], 1, iovs(&[[0x100, 1]]), |_, _| Err(
                SyscallError::UnmappedMemory
            )),
            Err(SyscallError::UnmappedMemory)
        );
    }

    /// recvmsg reports dropped fds with MSG_CTRUNC (review of the v0.26.0
    /// stack, PR #10).
    #[test]
    fn recvmsg_flags_report_control_truncation() {
        assert_eq!(MSG_CTRUNC, 0x8);
        assert_eq!(recvmsg_flags(0, 0), 0);
        assert_eq!(recvmsg_flags(2, 2), 0);
        assert_eq!(recvmsg_flags(3, 1), MSG_CTRUNC);
        assert_eq!(recvmsg_flags(1, 0), MSG_CTRUNC);
    }

    /// socketpair honours the type and accepts protocol 0 or PF_UNIX only
    /// (review of the v0.26.0 stack, PR #10).
    #[test]
    fn socketpair_type_maps_type_and_protocol() {
        use crate::net::unix_socket::UnixSocketType;
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_STREAM, 0),
            Ok(UnixSocketType::Stream)
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_DGRAM | 0x80000, 0),
            Ok(UnixSocketType::Datagram)
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_DGRAM, AF_UNIX),
            Ok(UnixSocketType::Datagram)
        );
        assert_eq!(
            socketpair_type(AF_UNIX, 7, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_STREAM, 6),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            socketpair_type(2, SOCK_STREAM, 0),
            Err(SyscallError::InvalidArgument)
        );
    }

    /// accept copies at most *addrlen bytes and stores the full length
    /// (review of the v0.26.0 stack, PR #10).
    #[test]
    fn sockaddr_out_honours_addrlen() {
        let addr: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
        // Room for everything; misaligned socklen_t.
        let mut out = [0u8; 16];
        let mut len = [0u8; 5];
        len[1..5].copy_from_slice(&64u32.to_ne_bytes());
        let len_ptr = len.as_mut_ptr() as usize + 1;
        assert_eq!(
            copy_sockaddr_out(out.as_mut_ptr() as usize, len_ptr, &addr),
            Ok(())
        );
        assert_eq!(out, addr);
        assert_eq!(u32::from_ne_bytes([len[1], len[2], len[3], len[4]]), 16);

        // A 1-byte buffer gets 1 byte, and learns the real length.
        let mut out = [0xEEu8; 4];
        let mut l = 1u32;
        assert_eq!(
            copy_sockaddr_out(
                out.as_mut_ptr() as usize,
                &mut l as *mut u32 as usize,
                &addr
            ),
            Ok(())
        );
        assert_eq!(out, [1, 0xEE, 0xEE, 0xEE]);
        assert_eq!(l, 16);

        // Unnamed AF_UNIX: length 2.
        let mut out = [0u8; 110];
        let mut l = 110u32;
        let unnamed = (AF_UNIX as u16).to_ne_bytes();
        assert_eq!(
            copy_sockaddr_out(
                out.as_mut_ptr() as usize,
                &mut l as *mut u32 as usize,
                &unnamed
            ),
            Ok(())
        );
        assert_eq!(l, 2);
        assert_eq!(u16::from_ne_bytes([out[0], out[1]]), AF_UNIX as u16);

        // Negative *addrlen is EINVAL; no address buffer means no copy.
        let mut l = u32::MAX;
        assert_eq!(
            copy_sockaddr_out(
                out.as_mut_ptr() as usize,
                &mut l as *mut u32 as usize,
                &addr
            ),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(copy_sockaddr_out(0, 0, &addr), Ok(()));
    }

    #[test]
    fn fd_index_rejects_negative() {
        assert_eq!(scm_fd_index(-1), Err(SyscallError::BadFileDescriptor));
        assert_eq!(scm_fd_index(i32::MIN), Err(SyscallError::BadFileDescriptor));
        assert_eq!(scm_fd_index(7), Ok(7));
    }

    // --- IPC message size tiers (IPC-ARCH-02) ---

    #[test]
    fn message_tier_boundaries() {
        use crate::ipc::message::MAX_BUFFERED_PAYLOAD;
        assert_eq!(SMALL_MESSAGE_BYTES, 48);
        assert_eq!(message_tier(0), Err(SyscallError::InvalidArgument));
        assert_eq!(message_tier(1), Ok(MessageTier::Small));
        assert_eq!(message_tier(SMALL_MESSAGE_BYTES), Ok(MessageTier::Small));
        assert_eq!(
            message_tier(SMALL_MESSAGE_BYTES + 1),
            Ok(MessageTier::Buffered)
        );
        assert_eq!(
            message_tier(MAX_BUFFERED_PAYLOAD),
            Ok(MessageTier::Buffered)
        );
        assert_eq!(
            message_tier(MAX_BUFFERED_PAYLOAD + 1),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(message_tier(usize::MAX), Err(SyscallError::InvalidArgument));
    }

    fn small_bytes(msg: &SmallMessage) -> [u8; SMALL_MESSAGE_BYTES] {
        let mut b = [0u8; SMALL_MESSAGE_BYTES];
        b.copy_from_slice(pod_bytes(msg));
        b
    }

    #[test]
    fn small_message_capability_is_replaced_only_when_given() {
        let sent = SmallMessage::new(0x1111, 7)
            .with_flags(3)
            .with_data(0, 0xAA)
            .with_data(3, 0xDD);
        let bytes = small_bytes(&sent);
        assert_eq!(small_message_from_bytes(&bytes, None), sent);
        let got = small_message_from_bytes(&bytes, Some(0x9999));
        assert_eq!(got.capability, 0x9999);
        assert_eq!((got.opcode, got.flags, got.data), (7, 3, sent.data));
    }

    #[test]
    fn message_from_user_picks_the_tier_and_pads_short_messages() {
        // A short small message is zero-padded.
        let raw = 0x0102_0304_0506_0708u64.to_ne_bytes();
        match message_from_user(Some(5), raw.as_ptr() as usize, raw.len()) {
            Ok(Message::Small(m)) => {
                assert_eq!(m.capability, 5, "the validated capability wins");
                assert_eq!((m.opcode, m.flags, m.data), (0, 0, [0; 4]));
            }
            _ => panic!("expected a small message"),
        }
        let full = small_bytes(&SmallMessage::new(1, 2).with_data(1, 9));
        match message_from_user(None, full.as_ptr() as usize, full.len()) {
            Ok(Message::Small(m)) => assert_eq!(m, SmallMessage::new(1, 2).with_data(1, 9)),
            _ => panic!("expected a small message"),
        }
        // One byte more is buffered, with the capability in the header.
        let payload = [0x5Au8; SMALL_MESSAGE_BYTES + 1];
        match message_from_user(Some(77), payload.as_ptr() as usize, payload.len()) {
            Ok(Message::Buffered(m)) => {
                assert_eq!(m.payload, payload);
                assert_eq!(m.header.capability, 77);
                assert_eq!(m.header.total_size, payload.len() as u64);
            }
            _ => panic!("expected a buffered message"),
        }
        assert!(matches!(
            message_from_user(None, payload.as_ptr() as usize, 0),
            Err(SyscallError::InvalidArgument)
        ));
    }

    #[test]
    fn message_to_user_copies_and_truncates() {
        use crate::ipc::message::{BufferedMessage, MessageHeader};
        const HEADER: usize = core::mem::size_of::<MessageHeader>();

        let small = SmallMessage::new(4, 5).with_data(2, 6);
        let mut out = [0u8; SMALL_MESSAGE_BYTES + 4];
        assert_eq!(
            message_to_user(&Message::Small(small), out.as_mut_ptr() as usize, out.len()),
            Ok(SMALL_MESSAGE_BYTES)
        );
        assert_eq!(&out[..SMALL_MESSAGE_BYTES], &small_bytes(&small)[..]);
        // A small message never goes into a shorter buffer.
        assert_eq!(
            message_to_user(&Message::Small(small), out.as_mut_ptr() as usize, 47),
            Err(SyscallError::InvalidArgument)
        );

        let payload: alloc::vec::Vec<u8> = (0..100).collect();
        let msg = Message::Buffered(BufferedMessage::new(8, 0, payload.clone()).unwrap());
        // Room for the header and 10 payload bytes: the full length is
        // reported so the receiver sees what it missed.
        let mut out = [0xEEu8; HEADER + 12];
        assert_eq!(
            message_to_user(&msg, out.as_mut_ptr() as usize, HEADER + 10),
            Ok(HEADER + 100)
        );
        assert_eq!(&out[HEADER..HEADER + 10], &payload[..10]);
        assert_eq!(&out[HEADER + 10..], &[0xEE, 0xEE]);
        assert_eq!(u64::from_ne_bytes(out[..8].try_into().unwrap()), 8);
        assert_eq!(
            message_to_user(&msg, out.as_mut_ptr() as usize, HEADER - 1),
            Err(SyscallError::InvalidArgument)
        );
    }

    // --- epoll_event layout ---

    #[test]
    fn epoll_event_is_packed_and_round_trips() {
        use crate::net::epoll::EpollEvent;
        assert_eq!(EPOLL_EVENT_BYTES, 12);
        let mut raw = [0u8; 12];
        raw[..4].copy_from_slice(&0x8000_0011u32.to_ne_bytes());
        raw[4..].copy_from_slice(&0xDEAD_BEEF_0000_0042u64.to_ne_bytes());
        let ev = epoll_event_from_bytes(&raw);
        let (events, data) = (ev.events, ev.data);
        assert_eq!((events, data), (0x8000_0011, 0xDEAD_BEEF_0000_0042));

        let second = EpollEvent { events: 1, data: 2 };
        let bytes = epoll_events_to_bytes(&[ev, second]);
        assert_eq!(bytes.len(), 24);
        assert_eq!(&bytes[..12], &raw);
        assert_eq!(&bytes[12..16], &1u32.to_ne_bytes());
        assert_eq!(&bytes[16..], &2u64.to_ne_bytes());
        assert!(epoll_events_to_bytes(&[]).is_empty());
    }

    /// W-16: the byte count for maxevents must not wrap.
    #[test]
    fn epoll_buffer_len_bounds_maxevents() {
        let cap = i32::MAX as usize / 12;
        assert_eq!(epoll_events_buffer_len(1), Ok(12));
        assert_eq!(epoll_events_buffer_len(cap), Ok(cap * 12));
        assert_eq!(
            epoll_events_buffer_len(cap + 1),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            epoll_events_buffer_len(0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            epoll_events_buffer_len(usize::MAX),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn inet_addr_is_address_then_big_endian_port() {
        let a = inet_addr_from_bytes(&[10, 0, 2, 15, 0x1F, 0x90]);
        assert_eq!(
            a,
            crate::net::SocketAddr::v4(crate::net::Ipv4Address([10, 0, 2, 15]), 8080)
        );
        let raw = [192u8, 168, 1, 1, 0, 53];
        assert_eq!(
            read_inet_addr(raw.as_ptr() as usize),
            Ok(inet_addr_from_bytes(&raw))
        );
        assert!(read_inet_addr(0).is_err());
    }

    // --- getdents64 records ---

    fn dir_entry(name: &str, node_type: crate::fs::NodeType, inode: u64) -> crate::fs::DirEntry {
        crate::fs::DirEntry {
            name: alloc::string::String::from(name),
            node_type,
            inode,
        }
    }

    /// (d_ino, d_off, d_reclen, d_type, name) of each record in `buf`.
    fn parse_dirents(buf: &[u8]) -> alloc::vec::Vec<(u64, u64, usize, u8, alloc::string::String)> {
        let mut out = alloc::vec::Vec::new();
        let mut at = 0;
        while at < buf.len() {
            let rec = &buf[at..];
            let reclen = u16::from_ne_bytes([rec[16], rec[17]]) as usize;
            let name_end = rec[19..reclen].iter().position(|&b| b == 0).unwrap();
            out.push((
                u64::from_ne_bytes(rec[..8].try_into().unwrap()),
                u64::from_ne_bytes(rec[8..16].try_into().unwrap()),
                reclen,
                rec[18],
                alloc::string::String::from_utf8(rec[19..19 + name_end].to_vec()).unwrap(),
            ));
            at += reclen;
        }
        out
    }

    #[test]
    fn dirent64_types_match_linux() {
        use crate::fs::NodeType;
        assert_eq!(dirent64_type(NodeType::Pipe), 1);
        assert_eq!(dirent64_type(NodeType::CharDevice), 2);
        assert_eq!(dirent64_type(NodeType::Directory), 4);
        assert_eq!(dirent64_type(NodeType::BlockDevice), 6);
        assert_eq!(dirent64_type(NodeType::File), 8);
        assert_eq!(dirent64_type(NodeType::Symlink), 10);
        assert_eq!(dirent64_type(NodeType::Socket), 12);
    }

    #[test]
    fn dirents64_are_aligned_terminated_and_bounded() {
        use crate::fs::NodeType;
        let entries = [
            dir_entry(".", NodeType::Directory, 0),
            dir_entry("hello.txt", NodeType::File, 42),
            dir_entry("abcd", NodeType::Symlink, 7),
        ];
        // 19 + 1 + 1 = 21 -> 24; 19 + 9 + 1 = 29 -> 32; 19 + 4 + 1 = 24.
        let (buf, next) = build_dirents64(&entries, 0, 4096).unwrap();
        assert_eq!(next, 3);
        assert_eq!(buf.len(), 24 + 32 + 24);
        let recs = parse_dirents(&buf);
        assert_eq!(recs.len(), 3);
        // A missing inode number becomes index + 1.
        assert_eq!((recs[0].0, recs[0].2, recs[0].3), (1, 24, 4));
        assert_eq!(recs[0].4, ".");
        assert_eq!((recs[1].0, recs[1].2, recs[1].3), (42, 32, 8));
        assert_eq!(recs[1].4, "hello.txt");
        assert_eq!((recs[2].0, recs[2].3), (7, 10));
        // Padding after the name is zeroed.
        assert!(buf[24 + 19 + 9..56].iter().all(|&b| b == 0));

        // A buffer that ends inside a record stops before it.
        let (buf, next) = build_dirents64(&entries, 0, 24 + 31).unwrap();
        assert_eq!((buf.len(), next), (24, 1));
        // Resuming at an index continues from there.
        let (buf, next) = build_dirents64(&entries, 2, 24).unwrap();
        assert_eq!((buf.len(), next), (24, 3));
        assert_eq!(parse_dirents(&buf)[0].4, "abcd");
        // Nothing left is end of directory; no room for even one record is
        // EINVAL, as on Linux, not a short directory.
        assert_eq!(build_dirents64(&entries, 3, 4096), Ok((alloc::vec![], 3)));
        assert_eq!(build_dirents64(&entries, 7, 4096), Ok((alloc::vec![], 7)));
        assert_eq!(
            build_dirents64(&entries, 0, 23),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            build_dirents64(&entries, 1, 31),
            Err(SyscallError::InvalidArgument)
        );
    }

    /// d_off is the position to seek to for the next entry. A directory
    /// fd's position is an entry index, and musl's telldir/seekdir pass
    /// d_off straight to lseek, so it must be an index, not a byte offset
    /// into this call's buffer.
    #[test]
    fn dirent64_d_off_is_the_next_entry_index() {
        use crate::fs::NodeType;
        let entries = [
            dir_entry("a", NodeType::File, 0),
            dir_entry("bb", NodeType::File, 0),
            dir_entry("ccc", NodeType::File, 0),
        ];
        let (buf, _) = build_dirents64(&entries, 0, 4096).unwrap();
        let offs: alloc::vec::Vec<u64> = parse_dirents(&buf).iter().map(|r| r.1).collect();
        assert_eq!(offs, [1, 2, 3]);
        // Resuming at the d_off of the first record yields the second.
        let (buf, _) = build_dirents64(&entries, offs[0] as usize, 4096).unwrap();
        let recs = parse_dirents(&buf);
        assert_eq!(recs[0].4, "bb");
        assert_eq!(recs[0].1, 2);
    }

    // --- Rate limiter (SYS-PERF-01) ---

    const HZ: u64 = 1_000_000_000;

    #[test]
    fn rate_limiter_never_wraps_below_zero() {
        let limiter = SyscallRateLimiter::with_tokens(1);
        assert!(limiter.check_at(0, HZ));
        assert!(!limiter.check_at(0, HZ));
        assert!(!limiter.check_at(0, HZ));
        assert_eq!(limiter.tokens.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn rate_limiter_concurrent_consumers_take_each_token_once() {
        extern crate std;
        use std::{sync::Arc, thread, vec::Vec};

        let limiter = Arc::new(SyscallRateLimiter::with_tokens(1_000));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let l = Arc::clone(&limiter);
                thread::spawn(move || (0..1_000).filter(|_| l.check_at(0, HZ)).count())
            })
            .collect();
        let granted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(granted, 1_000);
        assert_eq!(limiter.tokens.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn rate_limiter_refills_by_elapsed_time_not_cycles() {
        let limiter = SyscallRateLimiter::with_tokens(0);
        limiter.check_at(0, HZ); // establishes the refill epoch
                                 // 1 us at 1 GHz is 1000 ticks: that must yield REFILL_PER_SEC / 1e6
                                 // tokens, not one token per tick.
        let per_us = SyscallRateLimiter::REFILL_PER_SEC / 1_000_000;
        let before = limiter.tokens.load(Ordering::Relaxed);
        limiter.check_at(1_000, HZ);
        let refilled = limiter.tokens.load(Ordering::Relaxed) + 1 - before;
        assert_eq!(refilled, per_us, "tokens credited for 1us");
        // Sub-token intervals accumulate instead of being dropped.
        let limiter = SyscallRateLimiter::with_tokens(0);
        let tick = HZ / SyscallRateLimiter::REFILL_PER_SEC; // ticks per token
        limiter.check_at(0, HZ);
        assert!(!limiter.check_at(tick / 2, HZ));
        assert!(limiter.check_at(tick, HZ));
        // A full second refills to the burst cap, no further.
        limiter.check_at(2 * HZ, HZ);
        assert!(limiter.tokens.load(Ordering::Relaxed) <= SyscallRateLimiter::MAX_TOKENS);
        assert!(limiter.tokens.load(Ordering::Relaxed) >= SyscallRateLimiter::MAX_TOKENS - 1);
    }

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
