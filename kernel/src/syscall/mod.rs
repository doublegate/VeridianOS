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

// Scheduling policy, priority and affinity (N-221)
pub(crate) mod scheduling;

// Password checks for pam_veridian
mod veridian_auth;

// getrusage, times, sysinfo (N-223)
mod usage;

// getrlimit, setrlimit, prlimit64 (N-224)
mod limits;

// poll, ppoll, select, pselect6
mod multiplex;
use self::multiplex::{sys_poll, sys_ppoll, sys_pselect6, sys_select};

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

// System call numbers, generated from abi/syscalls.map (ADR 0009): Linux
// x86_64 numbers for the calls Linux has, private numbers from 1024 for the
// rest.
mod numbers;
pub use self::numbers::Syscall;

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
    /// No space left on the device (ENOSPC, errno 28; N-127).
    NoSpace = -112,
    /// Resource busy, e.g. already mounted (EBUSY, errno 16).
    Busy = -113,
    /// File too large (EFBIG, errno 27).
    FileTooLarge = -114,
    /// Operation not supported by this object (EOPNOTSUPP, errno 95).
    NotSupported = -115,
    /// No such device or filesystem type (ENODEV, errno 19).
    NoDevice = -116,
    /// Not an executable format (ENOEXEC, errno 8).
    ExecFormat = -117,
    /// A timed wait ran out (ETIMEDOUT, errno 110).
    TimedOut = -118,
    AddressFamilyNotSupported = -119,
    ProtocolNotSupported = -120,
    IllegalSeek = -121,
    RangeError = -122,
    NameTooLong = -123,
    AddressInUse = -124,
    ConnectionRefused = -125,
    NotConnected = -126,
    AlreadyConnected = -127,
    InProgress = -128,
    ConnectionReset = -129,
    /// ECANCELED: a timerfd's wall clock was set (TFD_TIMER_CANCEL_ON_SET).
    Canceled = -130,
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
            FsError::TooManyOpenFiles => SyscallError::ResourceLimitExceeded,
            FsError::CrossDevice => SyscallError::CrossDevice,
            // One errno per cause (N-127); no catch-all, so a new FsError
            // variant has to be given one.
            FsError::AlreadyMounted => SyscallError::Busy,
            FsError::NotMounted => SyscallError::InvalidArgument,
            FsError::UnknownFsType => SyscallError::NoDevice,
            FsError::NotSupported => SyscallError::NotSupported,
            FsError::NotASymlink => SyscallError::InvalidArgument,
            FsError::FileTooLarge => SyscallError::FileTooLarge,
            FsError::CorruptedData => SyscallError::IoError,
            FsError::SymlinkLoop => SyscallError::SymlinkLoop,
            FsError::NoSpace => SyscallError::NoSpace,
            FsError::OperationNotPermitted => SyscallError::OperationNotPermitted,
            FsError::Busy => SyscallError::Busy,
            FsError::Canceled => SyscallError::Canceled,
        },
        KernelError::OutOfMemory { .. } => SyscallError::OutOfMemory,
        KernelError::InvalidArgument { .. } => SyscallError::InvalidArgument,
        KernelError::OperationNotSupported { .. } => SyscallError::NotSupported,
        KernelError::ResourceExhausted { .. } => SyscallError::OutOfMemory,
        KernelError::UnmappedMemory { .. } | KernelError::InvalidAddress { .. } => {
            SyscallError::InvalidPointer
        }
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

    // Caller PID for audit logging. The process reference is dropped here:
    // exit and exec do not return, so an Arc held across the dispatch would
    // never be released.
    let caller_pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    // One table for every caller (ADR 0009): Linux x86_64 numbers, and
    // private numbers from 1024 for VeridianOS-only calls. An unknown
    // number is ENOSYS, as on Linux.
    let result = match Syscall::try_from(syscall_num) {
        Ok(syscall) => handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5),
        Err(()) => Err(SyscallError::InvalidSyscall),
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
            // Errors are Linux errno values (-errno), which both C
            // libraries' __syscall_ret expect. Raw VeridianOS codes were
            // misread: ResourceNotFound (-4) is EINTR to musl, which then
            // retried forever.
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
        Syscall::SchedYield => sys_yield(),
        Syscall::ExitGroup => sys_exit(arg1),
        Syscall::Fork => sys_fork(),
        Syscall::Vfork => process::sys_vfork(),
        Syscall::Execve => sys_exec(arg1, arg2, arg3),
        Syscall::Wait4 => sys_wait4(arg1 as isize, arg2, arg3, arg4),
        Syscall::Getpid => sys_getpid(),
        Syscall::Getppid => sys_getppid(),
        Syscall::ProcessSetPriority => sys_setpriority(arg1, arg2, arg3),
        Syscall::ProcessGetPriority => sys_getpriority(arg1, arg2),

        // Thread management
        Syscall::ThreadCreate => sys_thread_create(arg1, arg2, arg3, arg4),
        Syscall::Exit => sys_thread_exit(arg1),
        Syscall::ThreadJoin => sys_thread_join(arg1, arg2),
        Syscall::Gettid => sys_gettid(),
        Syscall::ThreadSetAffinity => sys_thread_setaffinity(arg1, arg2, arg3),
        Syscall::ThreadGetAffinity => sys_thread_getaffinity(arg1, arg2, arg3),
        Syscall::Clone => thread_clone::sys_thread_clone(arg1, arg2, arg3, arg4, arg5),

        // Filesystem operations
        Syscall::Open => sys_open(arg1, arg2, arg3),
        Syscall::Close => sys_close(arg1),
        Syscall::Read => sys_read(arg1, arg2, arg3),
        Syscall::Write => sys_write(arg1, arg2, arg3),
        Syscall::Lseek => sys_seek(arg1, arg2 as isize, arg3),
        Syscall::Fstat => sys_stat(arg1, arg2),
        Syscall::Ftruncate => sys_ftruncate(arg1, arg2),
        Syscall::Dup => sys_dup(arg1),
        Syscall::Dup2 => sys_dup2(arg1, arg2),
        Syscall::Pipe => sys_pipe(arg1),

        // Memory management
        Syscall::Mmap => sys_mmap(arg1, arg2, arg3, arg4, arg5),
        Syscall::Munmap => sys_munmap(arg1, arg2),
        Syscall::Mprotect => sys_mprotect(arg1, arg2, arg3),
        Syscall::Brk => sys_brk(arg1),

        // Directory operations
        Syscall::Mkdir => sys_mkdir(arg1, arg2),
        Syscall::Rmdir => sys_rmdir(arg1),
        Syscall::DirOpendir => sys_opendir(arg1),
        Syscall::DirReaddir => sys_readdir(arg1, arg2, arg3),
        Syscall::DirClosedir => sys_closedir(arg1),
        Syscall::Pipe2 => sys_pipe2(arg1, arg2),
        Syscall::Dup3 => sys_dup3(arg1, arg2, arg3),

        // Filesystem management
        Syscall::FsMount => sys_mount(arg1, arg2, arg3, arg4),
        Syscall::FsUnmount => sys_unmount(arg1),
        Syscall::Sync => sys_sync(),
        Syscall::Fsync | Syscall::Fdatasync => sys_fsync(arg1),

        // Kernel information
        Syscall::KernelGetInfo => sys_get_kernel_info(arg1),

        // Package management
        Syscall::PkgInstall => sys_pkg_install(arg1, arg2),
        Syscall::PkgRemove => sys_pkg_remove(arg1, arg2),
        Syscall::PkgQuery => sys_pkg_query(arg1, arg2),
        Syscall::PkgList => sys_pkg_list(arg1, arg2),
        Syscall::PkgUpdate => sys_pkg_update(arg1),

        // Extended process operations
        Syscall::Getcwd => sys_getcwd(arg1, arg2),
        Syscall::Chdir => sys_chdir(arg1),
        Syscall::Ioctl => sys_ioctl(arg1, arg2, arg3),
        Syscall::Kill => sys_kill(arg1, arg2),

        // Time management
        Syscall::TimeGetUptime => sys_time_get_uptime(),
        Syscall::TimeCreateTimer => sys_time_create_timer(arg1, arg2, arg3),
        Syscall::TimeCancelTimer => sys_time_cancel_timer(arg1),

        // Signal management
        Syscall::RtSigaction => sys_sigaction(arg1, arg2, arg3),
        Syscall::RtSigprocmask => sys_sigprocmask(arg1, arg2, arg3),
        Syscall::RtSigsuspend => sys_sigsuspend(arg1),
        Syscall::RtSigreturn => sys_sigreturn(arg1),

        // POSIX time syscalls
        Syscall::ClockGettime => sys_clock_gettime(arg1, arg2),
        Syscall::ClockGetres => sys_clock_getres(arg1, arg2),
        Syscall::Nanosleep => sys_nanosleep(arg1, arg2),
        Syscall::Gettimeofday => sys_gettimeofday(arg1, arg2),
        Syscall::Settimeofday => sys_settimeofday(arg1, arg2),
        Syscall::ClockSettime => sys_clock_settime(arg1, arg2),
        Syscall::Time => sys_time(arg1),

        // Identity syscalls
        Syscall::Getuid => sys_getuid(),
        Syscall::Geteuid => sys_geteuid(),
        Syscall::Getgid => sys_getgid(),
        Syscall::Getegid => sys_getegid(),
        Syscall::Setuid => sys_setuid(arg1),
        Syscall::Setgid => sys_setgid(arg1),
        Syscall::Setreuid => process::sys_setreuid(arg1, arg2),
        Syscall::Setregid => process::sys_setregid(arg1, arg2),
        Syscall::Setresuid => process::sys_setresuid(arg1, arg2, arg3),
        Syscall::Getresuid => process::sys_getresuid(arg1, arg2, arg3),
        Syscall::Setresgid => process::sys_setresgid(arg1, arg2, arg3),
        Syscall::Getresgid => process::sys_getresgid(arg1, arg2, arg3),
        Syscall::Getgroups => process::sys_getgroups(arg1, arg2),
        Syscall::Setgroups => process::sys_setgroups(arg1, arg2),
        Syscall::Fchdir => sys_fchdir(arg1),
        Syscall::Chroot => sys_chroot(arg1),
        Syscall::Utimensat => sys_utimensat(arg1, arg2, arg3, arg4),

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
        Syscall::Stat => sys_stat_path(arg1, arg2),
        Syscall::Lstat => sys_lstat(arg1, arg2),
        Syscall::Readlink => sys_readlink(arg1, arg2, arg3),
        Syscall::Access => sys_access(arg1, arg2),
        Syscall::Rename => sys_rename(arg1, arg2),
        Syscall::Link => sys_link(arg1, arg2),
        Syscall::Symlink => sys_symlink(arg1, arg2),
        Syscall::Unlink => sys_unlink(arg1),
        Syscall::Fcntl => sys_fcntl(arg1, arg2, arg3),

        // Self-hosting filesystem ops
        Syscall::Chmod => sys_chmod(arg1, arg2),
        Syscall::Fchmod => sys_fchmod(arg1, arg2),
        Syscall::Umask => sys_umask(arg1),
        Syscall::Truncate => sys_truncate(arg1, arg2),
        Syscall::Poll => sys_poll(arg1, arg2, arg3),
        Syscall::Openat => sys_openat(arg1, arg2, arg3, arg4),
        Syscall::Newfstatat => sys_fstatat(arg1, arg2, arg3, arg4),
        Syscall::Unlinkat => sys_unlinkat(arg1, arg2, arg3),
        Syscall::Mkdirat => sys_mkdirat(arg1, arg2, arg3),
        Syscall::Renameat => sys_renameat(arg1, arg2, arg3, arg4),
        Syscall::Pread64 => sys_pread(arg1, arg2, arg3, arg4),
        Syscall::Pwrite64 => sys_pwrite(arg1, arg2, arg3, arg4),
        Syscall::Chown => sys_chown(arg1, arg2, arg3),
        Syscall::Fchown => sys_fchown(arg1, arg2, arg3),
        Syscall::Mknod => sys_mknod(arg1, arg2, arg3),
        Syscall::Select => sys_select(arg1, arg2, arg3, arg4, arg5),
        // pselect6(nfds, readfds, writefds, exceptfds, timeout, sig)
        Syscall::Pselect6 => sys_pselect6(arg1, arg2, arg3, arg4, arg5, syscall_arg6()?),
        // Futex entrypoint: dispatch all futex ops (wait/wake/requeue/bitset/wake_op)
        Syscall::FutexWait => {
            futex::sys_futex_dispatch(arg1, arg2, arg3, arg4, arg5).map(|v| v as usize)
        }
        Syscall::FutexWake => futex::sys_futex_wake(arg1, arg2, arg3).map(|v| v as usize),
        Syscall::ArchPrctl => arch_prctl::sys_arch_prctl(arg1, arg2).map(|v| v as usize),
        Syscall::Uname => sys_uname(arg1),
        Syscall::ProcessGetenv => sys_getenv(arg1, arg2, arg3, arg4),

        // POSIX shared memory
        Syscall::ShmOpen => sys_shm_open(arg1, arg2, arg3),
        Syscall::ShmUnlink => sys_shm_unlink(arg1, arg2),
        Syscall::ShmTruncate => sys_shm_truncate(arg1, arg2, arg3),

        // Socket operations
        Syscall::Socket => sys_socket_create(arg1, arg2, arg3),
        Syscall::Bind => sys_socket_bind(arg1, arg2, arg3),
        Syscall::Listen => sys_socket_listen(arg1, arg2),
        Syscall::Connect => sys_socket_connect(arg1, arg2, arg3),
        Syscall::Accept => sys_socket_accept(arg1, arg2, arg3, 0),
        Syscall::Accept4 => sys_socket_accept(arg1, arg2, arg3, arg4),
        Syscall::SocketSend => sys_socket_send(arg1, arg2, arg3),
        Syscall::SocketRecv => sys_socket_recv(arg1, arg2, arg3),
        Syscall::SocketClose => sys_socket_close(arg1),
        // Linux ABI: socketpair(domain, type, protocol, sv[2])
        // arg1=domain, arg2=type, arg3=protocol, arg4=sv pointer
        Syscall::Socketpair => sys_socket_pair(arg1, arg2, arg3, arg4),

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
        Syscall::Sendto => sys_net_sendto(arg1, arg2, arg3, arg4, arg5),
        Syscall::Recvfrom => sys_net_recvfrom(arg1, arg2, arg3, arg4, arg5),
        Syscall::Getsockname => sys_net_getsockname(arg1, arg2, arg3),
        Syscall::Getpeername => sys_net_getpeername(arg1, arg2, arg3),
        Syscall::Setsockopt => sys_net_setsockopt(arg1, arg2, arg3, arg4, arg5),
        Syscall::Getsockopt => sys_net_getsockopt(arg1, arg2, arg3, arg4, arg5),

        // Resource limits (Phase 6.5)
        #[cfg(feature = "alloc")]
        Syscall::Getrlimit => limits::sys_getrlimit(arg1, arg2),
        #[cfg(feature = "alloc")]
        Syscall::Setrlimit => limits::sys_setrlimit(arg1, arg2),

        // epoll (N-234, N-245): the instance is its file.
        Syscall::EpollCreate => {
            if arg1 as i32 <= 0 {
                return Err(SyscallError::InvalidArgument);
            }
            sys_epoll_create1(0)
        }
        Syscall::EpollCreate1 => sys_epoll_create1(arg1),
        Syscall::EpollCtl => sys_epoll_ctl(arg1, arg2, arg3, arg4),
        Syscall::EpollWait => sys_epoll_wait(arg1, arg2, arg3, epoll_timeout_ms(arg4)),
        // epoll_pwait(epfd, events, maxevents, timeout, sigmask, sigsetsize)
        Syscall::EpollPwait => {
            let size = if arg5 != 0 { syscall_arg6()? } else { 0 };
            let mask = signal::begin_wait_sigmask(arg5, size)?;
            let result = sys_epoll_wait(arg1, arg2, arg3, epoll_timeout_ms(arg4));
            mask.end(&result);
            result
        }
        // epoll_pwait2: the timeout is a timespec (NULL: none).
        Syscall::EpollPwait2 => {
            let timeout = if arg4 == 0 {
                None
            } else {
                let [sec, nsec] = userspace::read_user::<[i64; 2]>(arg4)?;
                if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
                    return Err(SyscallError::InvalidArgument);
                }
                Some(
                    (sec as u64)
                        .saturating_mul(1_000_000_000)
                        .saturating_add(nsec as u64),
                )
            };
            let size = if arg5 != 0 { syscall_arg6()? } else { 0 };
            let mask = signal::begin_wait_sigmask(arg5, size)?;
            let result = sys_epoll_wait(arg1, arg2, arg3, timeout);
            mask.end(&result);
            result
        }
        // Process groups / sessions (Phase 6.5) -- delegate to existing
        // implementations which also back the older syscall numbers 176-180.
        Syscall::TcSetPgrp => sys_tcsetpgrp(arg1, arg2),
        Syscall::TcGetPgrp => sys_tcgetpgrp(arg1),
        // PTY syscalls (Phase 6.5)
        Syscall::OpenPty => pty::sys_openpty(arg1, arg2),
        Syscall::GrantPty => pty::sys_grantpt(arg1),
        Syscall::UnlockPty => pty::sys_unlockpt(arg1),
        Syscall::PtsName => pty::sys_ptsname(arg1, arg2, arg3),
        // Duplicate POSIX aliases -- delegate to the primary implementations.
        Syscall::Futex => {
            // Linux ABI: futex(uaddr, op, val, timeout/val2, uaddr2, val3)
            // arg1=uaddr, arg2=op, arg3=val, arg4=timeout/val2, arg5=uaddr2
            // Linux futex(uaddr, op, val, timeout|val2, uaddr2, val3).
            // val3 (arg6) is not a handler parameter; it is read from the
            // saved syscall frame. Mask off FUTEX_PRIVATE_FLAG (bit 7 = 128)
            // -- VeridianOS is single-address-space per process, so
            // private == shared.
            let cmd = (arg2 as u32) & 0x7F;
            // FUTEX_CLOCK_REALTIME: a WAIT_BITSET deadline is on the wall
            // clock. Linux accepts it only on waits (ENOSYS otherwise).
            let realtime = arg2 & 0x100 != 0;
            if realtime && cmd != 0 && cmd != 9 {
                return Err(SyscallError::NotImplemented);
            }
            match cmd {
                // FUTEX_WAIT: wait if *uaddr == val; the timeout is a
                // relative timespec (N-104).
                0 => {
                    let deadline = futex::linux_timeout(arg4, futex::FutexTimeout::Relative)?;
                    futex::futex_wait_until(
                        arg1,
                        arg3 as u32,
                        deadline,
                        futex::FUTEX_WAIT_BITSET_MATCH_ANY,
                    )
                    .map(|v| v as usize)
                }
                // FUTEX_WAKE: wake up to val waiters
                1 => futex::sys_futex_wake(arg1, arg3, 0).map(|v| v as usize),
                // FUTEX_REQUEUE: wake val waiters, move up to val2 (arg4)
                // of the rest to uaddr2. The count was passed as 0, so
                // nobody moved.
                3 => futex::sys_futex_requeue(arg1, arg3, arg5, arg4).map(|v| v as usize),
                // FUTEX_CMP_REQUEUE: the same, if *uaddr still equals val3.
                4 => futex::sys_futex_cmp_requeue(arg1, arg3, arg5, arg4, syscall_arg6()? as u32)
                    .map(|v| v as usize),
                // FUTEX_WAKE_OP(uaddr, val, val2 = arg4, uaddr2, encoded op = val3).
                // Passing 0 for the encoded op meant "*uaddr2 = 0" on every call.
                5 => futex::sys_futex_wake_op(arg1, arg3, arg5, arg4, syscall_arg6()?)
                    .map(|v| v as usize),
                // FUTEX_WAIT_BITSET: the bitset is val3 (arg6), and the
                // timeout is an absolute time (N-104).
                9 => {
                    let bitset = syscall_arg6()? as u32;
                    if bitset == 0 {
                        return Err(SyscallError::InvalidArgument);
                    }
                    let deadline = futex::linux_timeout(
                        arg4,
                        if realtime {
                            futex::FutexTimeout::Realtime
                        } else {
                            futex::FutexTimeout::Monotonic
                        },
                    )?;
                    futex::futex_wait_until(arg1, arg3 as u32, deadline, bitset).map(|v| v as usize)
                }
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

        // eventfd2(initval, flags), timerfd_create(clockid, flags): the
        // NONBLOCK flag is the file's, as fcntl sees it (N-232).
        Syscall::Eventfd2 => sys_eventfd2(arg1, arg2),
        // eventfd(initval): eventfd2 without flags.
        Syscall::Eventfd => sys_eventfd2(arg1, 0),
        Syscall::TimerfdCreate => {
            let flags = arg2 as u32;
            let tfd_id = crate::fs::timerfd::timerfd_create(arg1 as u32, flags)? as u32;
            install_event_file(
                alloc::sync::Arc::new(crate::fs::timerfd::TimerFdNode::new(tfd_id)),
                flags & crate::fs::timerfd::TFD_NONBLOCK != 0,
                flags & crate::fs::timerfd::TFD_CLOEXEC != 0,
            )
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

        // signalfd4(fd, mask, sizemask, flags) (N-233): the mask is a
        // sigset_t in user memory, sizemask its size (8); fd -1 makes a new
        // signalfd, an existing signalfd's fd gets the new mask.
        Syscall::Signalfd4 => sys_signalfd4(arg1, arg2, arg3, arg4),
        // signalfd(fd, mask, sizemask): signalfd4 without flags.
        Syscall::Signalfd => sys_signalfd4(arg1, arg2, arg3, 0),

        // sendmsg/recvmsg -- delegate to unix socket module for SCM_RIGHTS
        Syscall::Sendmsg => sys_sendmsg(arg1, arg2, arg3),
        Syscall::Recvmsg => sys_recvmsg(arg1, arg2, arg3),

        // musl libc compatibility syscalls
        Syscall::Getdents64 => sys_getdents64(arg1, arg2, arg3),
        #[cfg(feature = "alloc")]
        Syscall::Prlimit64 => limits::sys_prlimit64(arg1, arg2, arg3, arg4),
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
        Syscall::GetRobustList => sys_get_robust_list(arg1, arg2, arg3),
        Syscall::ClockNanosleep => time::sys_clock_nanosleep(arg1, arg2, arg3, arg4),
        Syscall::Prctl => linux_compat::sys_prctl(arg1, arg2, arg3, arg4, arg5),
        Syscall::VeridianAuth => veridian_auth::sys_veridian_auth(arg1, arg2, arg3, arg4),
        Syscall::Flock => sys_flock(arg1, arg2),
        Syscall::Tkill => process::sys_tkill(arg1, arg2),
        Syscall::Tgkill => process::sys_tgkill(arg1, arg2, arg3),
        Syscall::Waitid => process::sys_waitid(arg1, arg2, arg3, arg4, arg5),
        Syscall::RtSigpending => signal::sys_sigpending(arg1, arg2),
        Syscall::Ppoll => sys_ppoll(arg1, arg2, arg3, arg4, arg5),
        // faccessat has no flags argument; faccessat2 adds one.
        Syscall::Faccessat => sys_faccessat(arg1, arg2, arg3, 0),
        Syscall::Faccessat2 => sys_faccessat(arg1, arg2, arg3, arg4),
        // Not implemented yet; callers fall back (musl: statx -> fstatat,
        // clone3 -> clone, mremap ENOMEM -> mmap + copy).
        Syscall::Mremap => Err(SyscallError::OutOfMemory),
        #[cfg(feature = "alloc")]
        Syscall::Getrusage => usage::sys_getrusage(arg1, arg2),
        #[cfg(feature = "alloc")]
        Syscall::Sysinfo => usage::sys_sysinfo(arg1),
        #[cfg(feature = "alloc")]
        Syscall::Times => usage::sys_times(arg1),
        Syscall::Rseq
        | Syscall::Statx
        | Syscall::Clone3
        | Syscall::Fallocate
        | Syscall::Statfs
        | Syscall::Fstatfs => Err(SyscallError::NotImplemented),
        Syscall::Sigaltstack => signal::sys_sigaltstack(arg1, arg2),
        // Scheduling policy, priority and affinity (N-221).
        Syscall::SchedSetscheduler => scheduling::sys_sched_setscheduler(arg1, arg2, arg3),
        Syscall::SchedGetscheduler => scheduling::sys_sched_getscheduler(arg1),
        Syscall::SchedSetparam => scheduling::sys_sched_setparam(arg1, arg2),
        Syscall::SchedGetparam => scheduling::sys_sched_getparam(arg1, arg2),
        Syscall::SchedSetattr => scheduling::sys_sched_setattr(arg1, arg2, arg3),
        Syscall::SchedGetattr => scheduling::sys_sched_getattr(arg1, arg2, arg3, arg4),
        Syscall::SchedGetPriorityMax => scheduling::sys_sched_get_priority_max(arg1),
        Syscall::SchedGetPriorityMin => scheduling::sys_sched_get_priority_min(arg1),
        Syscall::SchedRrGetInterval => scheduling::sys_sched_rr_get_interval(arg1, arg2),
        Syscall::SchedSetaffinity => scheduling::sys_sched_setaffinity(arg1, arg2, arg3),
        Syscall::SchedGetaffinity => scheduling::sys_sched_getaffinity(arg1, arg2, arg3),
        Syscall::Getpriority => scheduling::sys_getpriority(arg1, arg2),
        Syscall::Setpriority => scheduling::sys_setpriority(arg1, arg2, arg3),

        _ => Err(SyscallError::InvalidSyscall),
    }
}

/// eventfd2(initval, flags): the NONBLOCK flag is the file's, as fcntl
/// sees it (N-232).
#[cfg(feature = "alloc")]
fn sys_eventfd2(initval: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    let efd_id = crate::fs::eventfd::eventfd_create(initval as u32, flags)? as u32;
    install_event_file(
        alloc::sync::Arc::new(crate::fs::eventfd::EventFdNode::new(efd_id)),
        flags & crate::fs::eventfd::EFD_NONBLOCK != 0,
        flags & crate::fs::eventfd::EFD_CLOEXEC != 0,
    )
}

/// signalfd4(fd, mask, sizemask, flags) (N-233): a new signalfd for the
/// signals in `*mask`, or, given a signalfd, a new mask for it.
#[cfg(feature = "alloc")]
fn sys_signalfd4(fd: usize, mask_ptr: usize, sizemask: usize, flags: usize) -> SyscallResult {
    use crate::fs::signalfd::{SignalFdNode, SFD_CLOEXEC, SFD_NONBLOCK};
    let (fd, flags) = (fd as i32, flags as u32);
    if sizemask != 8 || flags & !(SFD_NONBLOCK | SFD_CLOEXEC) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let mask: u64 = userspace::read_user(mask_ptr)?;
    if fd != -1 {
        let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let file = proc
            .file_table
            .lock()
            .get(fd as usize)
            .ok_or(SyscallError::BadFileDescriptor)?;
        let node = file
            .node
            .as_any()
            .and_then(|any| any.downcast_ref::<SignalFdNode>())
            .ok_or(SyscallError::InvalidArgument)?;
        node.set_mask(mask);
        return Ok(fd as usize);
    }
    install_event_file(
        alloc::sync::Arc::new(SignalFdNode::new(mask)),
        flags & SFD_NONBLOCK != 0,
        flags & SFD_CLOEXEC != 0,
    )
}

/// Put an event object (eventfd, timerfd, signalfd) in the calling
/// process's file table: readable and writable, O_NONBLOCK and FD_CLOEXEC
/// as the creating call's flags ask.
#[cfg(feature = "alloc")]
fn install_event_file(
    node: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    nonblock: bool,
    cloexec: bool,
) -> SyscallResult {
    let mut flags = crate::fs::OpenFlags::read_write();
    flags.nonblock = nonblock;
    let file = crate::fs::file::File::new(node, flags);
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(alloc::sync::Arc::new(file), cloexec)
        .map_err(map_kernel_error)?;
    Ok(fd)
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

/// epoll_create1(flags): EPOLL_CLOEXEC is the only flag (EINVAL otherwise).
#[cfg(feature = "alloc")]
fn sys_epoll_create1(flags: usize) -> SyscallResult {
    use crate::net::epoll::{EpollNode, EPOLL_CLOEXEC};
    if flags & !(EPOLL_CLOEXEC as usize) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let file = crate::fs::file::File::new(
        alloc::sync::Arc::new(EpollNode::new()),
        crate::fs::OpenFlags::read_write(),
    );
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(alloc::sync::Arc::new(file), flags != 0)
        .map_err(map_kernel_error)?;
    Ok(fd)
}

/// The open file behind `fd` in the calling process (EBADF if none).
fn current_file(fd: usize) -> Result<alloc::sync::Arc<crate::fs::file::File>, SyscallError> {
    if fd > i32::MAX as usize {
        return Err(SyscallError::BadFileDescriptor);
    }
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc.file_table.lock().get(fd);
    file.ok_or(SyscallError::BadFileDescriptor)
}

/// epoll_ctl(epfd, op, fd, event), with Linux's checks in Linux's order:
/// the event (EFAULT, for ADD and MOD), both descriptors (EBADF), a target
/// without readiness (EPERM), then the rest ([`crate::net::epoll::CtlError`]).
#[cfg(feature = "alloc")]
fn sys_epoll_ctl(epfd: usize, op: usize, fd: usize, event_ptr: usize) -> SyscallResult {
    use crate::net::epoll::{CtlError, EpollNode, EPOLL_CTL_ADD, EPOLL_CTL_MOD};
    let op = op as u32;
    // struct epoll_event is packed (12 bytes on x86_64): copied field by
    // field through the fault-tolerant reader rather than borrowed from
    // user memory (review of the v0.26.0 stack).
    let event = if op == EPOLL_CTL_ADD || op == EPOLL_CTL_MOD {
        let mut raw = [0u8; EPOLL_EVENT_BYTES];
        userspace::read_user_bytes(event_ptr, &mut raw)?;
        Some(epoll_event_from_bytes(&raw))
    } else {
        None
    };
    let ep_file = current_file(epfd)?;
    let target = current_file(fd)?;
    EpollNode::ctl(&ep_file, op, fd as i32, &target, event.as_ref())
        .map(|()| 0)
        .map_err(|e| match e {
            CtlError::Invalid => SyscallError::InvalidArgument,
            CtlError::Exists => SyscallError::FileExists,
            CtlError::NotFound => SyscallError::ResourceNotFound,
            CtlError::NotPollable => SyscallError::OperationNotPermitted,
            CtlError::Loop => SyscallError::SymlinkLoop,
            CtlError::NoSpace => SyscallError::NoSpace,
        })
}

/// An epoll_wait timeout in milliseconds as a wait limit: negative waits
/// without one.
fn epoll_timeout_ms(timeout: usize) -> Option<u64> {
    let ms = timeout as i32;
    (ms >= 0).then(|| ms as u64 * 1_000_000)
}

/// epoll_wait and its variants: up to `max_events` events into the user
/// array, waiting up to `timeout_ns` (`None`: no limit). maxevents is
/// checked first (EINVAL), then the array (EFAULT), then the descriptor
/// (EBADF, EINVAL if not an epoll file).
#[cfg(feature = "alloc")]
fn sys_epoll_wait(
    epfd: usize,
    events_ptr: usize,
    max_events: usize,
    timeout_ns: Option<u64>,
) -> SyscallResult {
    use crate::net::epoll::{EpollEvent, EpollNode};
    validate_user_buffer(events_ptr, epoll_events_buffer_len(max_events)?)?;
    let file = current_file(epfd)?;
    let ep = EpollNode::of(&file).ok_or(SyscallError::InvalidArgument)?;
    // Events are gathered in a kernel array (at most 1024 per call, as a
    // short count is always allowed) and copied out packed, through the
    // fault-tolerant writer (N-43).
    let mut events = alloc::vec![EpollEvent { events: 0, data: 0 }; max_events.min(1024)];
    let n = ep.wait(&mut events, timeout_ns).map_err(|e| match e {
        crate::error::KernelError::WouldBlock => SyscallError::Interrupted,
        _ => SyscallError::InvalidArgument,
    })?;
    userspace::write_user_bytes(events_ptr, &epoll_events_to_bytes(&events[..n]))?;
    Ok(n)
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
    filesystem::chmod_node(&node, mode)
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

/// memfd_create (N-229): a read-write fd on an anonymous regular file in no
/// directory, which can be sized, read, written, mapped and sealed
/// (fcntl F_ADD_SEALS) -- what Wayland clients put their buffers in. It
/// returned an eventfd registry id that was never installed as an fd.
/// Flags as on Linux: MFD_CLOEXEC, MFD_ALLOW_SEALING, MFD_NOEXEC_SEAL,
/// MFD_EXEC; names over 249 bytes are EINVAL.
fn sys_memfd_create(name_ptr: usize, flags: usize) -> SyscallResult {
    const MFD_CLOEXEC: usize = 0x1;
    const MFD_ALLOW_SEALING: usize = 0x2;
    const MFD_NOEXEC_SEAL: usize = 0x8;
    const MFD_EXEC: usize = 0x10;
    /// NAME_MAX less the "memfd:" prefix Linux shows the name with.
    const MFD_NAME_MAX: usize = 249;

    // MFD_HUGETLB (0x4) and its size bits are refused with the unknown
    // flags: there is no hugetlbfs.
    if flags & !(MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_NOEXEC_SEAL | MFD_EXEC) != 0
        || (flags & MFD_NOEXEC_SEAL != 0 && flags & MFD_EXEC != 0)
    {
        return Err(SyscallError::InvalidArgument);
    }
    let name = userspace::read_user_cstr(name_ptr, MFD_NAME_MAX)?;
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let creds = proc.credentials();
    // MFD_NOEXEC_SEAL: not executable, the mode's execute bits sealed, and
    // sealable as with MFD_ALLOW_SEALING.
    let noexec = flags & MFD_NOEXEC_SEAL != 0;
    let node = crate::fs::memfd::MemfdNode::new(
        crate::fs::Permissions::from_mode(if noexec { 0o666 } else { 0o777 }),
        creds.euid,
        creds.egid,
        flags & (MFD_ALLOW_SEALING | MFD_NOEXEC_SEAL) != 0,
        if noexec { crate::fs::seals::EXEC } else { 0 },
    );
    let file = crate::fs::file::File::new_with_path(
        node,
        crate::fs::OpenFlags::read_write(),
        alloc::format!("/memfd:{} (deleted)", name),
    );
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(alloc::sync::Arc::new(file), flags & MFD_CLOEXEC != 0)
        .map_err(map_kernel_error)?;
    Ok(fd)
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

/// set_robust_list(head, len): the calling thread's robust futex list,
/// walked when it exits (N-225). `len` must be
/// `sizeof(struct robust_list_head)`; the pointer is only stored, as Linux
/// does (a bad one ends the walk).
fn sys_set_robust_list(head_ptr: usize, len: usize) -> SyscallResult {
    use crate::process::robust_list::ROBUST_LIST_HEAD_SIZE;
    if len != ROBUST_LIST_HEAD_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    thread
        .robust_list
        .store(head_ptr, core::sync::atomic::Ordering::Release);
    Ok(0)
}

/// get_robust_list(tid, head_ptr, len_ptr): the robust list of the thread
/// `tid` (0: the caller), for a caller allowed to trace it (ESRCH, EPERM).
fn sys_get_robust_list(tid: usize, head_out: usize, len_out: usize) -> SyscallResult {
    use crate::process::robust_list::ROBUST_LIST_HEAD_SIZE;
    let tid = tid as u32 as i32;
    let head = if tid == 0 {
        let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
        thread
            .robust_list
            .load(core::sync::atomic::Ordering::Acquire)
    } else {
        if tid < 0 {
            return Err(SyscallError::ProcessNotFound);
        }
        let caller = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
        let mut found = None;
        crate::process::table::PROCESS_TABLE.for_each(|p| {
            if found.is_none() {
                if let Some(t) = p.get_thread(crate::process::ThreadId(tid as u64)) {
                    found = Some((
                        p.credentials(),
                        p.dumpable.load(core::sync::atomic::Ordering::Acquire),
                        t,
                    ));
                }
            }
        });
        let (target_creds, target_dumpable, thread) = found.ok_or(SyscallError::ProcessNotFound)?;
        if !debug::may_access(&caller.credentials(), &target_creds, target_dumpable) {
            return Err(SyscallError::OperationNotPermitted);
        }
        thread
            .robust_list
            .load(core::sync::atomic::Ordering::Acquire)
    };
    userspace::write_user(head_out, head as u64)?;
    userspace::write_user(len_out, ROBUST_LIST_HEAD_SIZE as u64)?;
    Ok(0)
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

/// Linux SOCK_NONBLOCK and SOCK_CLOEXEC: OR-ed into socket()'s type, and
/// accept4()'s flags.
const SOCK_NONBLOCK: usize = 0x800;
const SOCK_CLOEXEC: usize = 0x8_0000;

/// Descriptor flags of a new socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SocketFdFlags {
    cloexec: bool,
    nonblock: bool,
}

/// Split socket()'s type into the base type (low 4 bits) and the
/// descriptor flags. Any other bit is EINVAL, as on Linux; they were
/// masked off and lost, so a "non-blocking" socket blocked.
fn socket_type_flags(raw: usize) -> Result<(usize, SocketFdFlags), SyscallError> {
    let flags = raw & !0xF;
    if flags & !(SOCK_NONBLOCK | SOCK_CLOEXEC) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok((
        raw & 0xF,
        SocketFdFlags {
            cloexec: flags & SOCK_CLOEXEC != 0,
            nonblock: flags & SOCK_NONBLOCK != 0,
        },
    ))
}

/// Install a new socket in the caller's file table and return its fd. If
/// that fails the node is dropped, which closes the socket.
fn install_socket(handle: SocketHandle, fd_flags: SocketFdFlags) -> SyscallResult {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let node: alloc::sync::Arc<dyn crate::fs::VfsNode> =
        alloc::sync::Arc::new(SocketNode::new(handle));
    let file = alloc::sync::Arc::new(crate::fs::file::File::new(
        node,
        crate::fs::file::OpenFlags {
            nonblock: fd_flags.nonblock,
            ..crate::fs::file::OpenFlags::read_write()
        },
    ));
    let fd = process
        .file_table
        .lock()
        .open_with_flags(file, fd_flags.cloexec)
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

/// The Unix socket type for a base socket type (flags already removed by
/// `socket_type_flags`).
fn to_unix_socket_type(
    base_type: usize,
) -> Result<crate::net::unix_socket::UnixSocketType, SyscallError> {
    match base_type {
        SOCK_STREAM => Ok(crate::net::unix_socket::UnixSocketType::Stream),
        SOCK_DGRAM => Ok(crate::net::unix_socket::UnixSocketType::Datagram),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// IPPROTO_TCP and IPPROTO_UDP, the protocols AF_INET sockets accept
/// besides 0 (the type's default).
const IPPROTO_TCP: usize = 6;
const IPPROTO_UDP: usize = 17;

/// What socket(domain, type, protocol) creates, or Linux's error for it:
/// EAFNOSUPPORT for an unknown domain, EINVAL for an unknown type,
/// EPROTONOSUPPORT for a protocol the type does not use.
#[derive(Debug, PartialEq, Eq)]
enum SocketKind {
    Unix(usize),
    Inet(usize),
}

fn socket_kind(
    domain: usize,
    base_type: usize,
    protocol: usize,
) -> Result<SocketKind, SyscallError> {
    match domain {
        AF_UNIX => {
            if base_type != SOCK_STREAM && base_type != SOCK_DGRAM {
                return Err(SyscallError::InvalidArgument);
            }
            if protocol != 0 && protocol != AF_UNIX {
                return Err(SyscallError::ProtocolNotSupported);
            }
            Ok(SocketKind::Unix(base_type))
        }
        AF_INET => {
            let default = match base_type {
                SOCK_STREAM => IPPROTO_TCP,
                SOCK_DGRAM => IPPROTO_UDP,
                _ => return Err(SyscallError::InvalidArgument),
            };
            if protocol != 0 && protocol != default {
                return Err(SyscallError::ProtocolNotSupported);
            }
            Ok(SocketKind::Inet(base_type))
        }
        _ => Err(SyscallError::AddressFamilyNotSupported),
    }
}

/// SYS_socket: socket(domain, type | SOCK_NONBLOCK | SOCK_CLOEXEC, protocol).
fn sys_socket_create(domain: usize, sock_type: usize, protocol: usize) -> SyscallResult {
    let (base_type, fd_flags) = socket_type_flags(sock_type)?;
    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    match socket_kind(domain, base_type, protocol)? {
        SocketKind::Unix(base_type) => {
            let utype = to_unix_socket_type(base_type)?;
            let id = crate::net::unix_socket::socket_create(utype, pid)
                .map_err(|_| SyscallError::OutOfMemory)?;
            install_socket(SocketHandle::Unix(id), fd_flags)
        }
        SocketKind::Inet(base_type) => {
            let sock_domain = crate::net::socket::SocketDomain::Inet;
            let (sock_tp, proto) = if base_type == SOCK_STREAM {
                (
                    crate::net::socket::SocketType::Stream,
                    crate::net::socket::SocketProtocol::Tcp,
                )
            } else {
                (
                    crate::net::socket::SocketType::Dgram,
                    crate::net::socket::SocketProtocol::Udp,
                )
            };
            let id = crate::net::socket::create_socket(sock_domain, sock_tp, proto)
                .map_err(|_| SyscallError::OutOfMemory)?;
            install_socket(SocketHandle::Inet(id), fd_flags)
        }
    }
}

/// SYS_bind: Bind a socket to an address/path.
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

/// SYS_listen: Start listening on a bound socket.
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

/// SYS_connect: Connect to a listening socket.
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

/// SYS_accept: Accept a pending connection and return an fd for it.
///
/// Linux ABI: `accept4(fd, addr, addrlen_ptr, flags)`.
/// `addr_ptr` is optional (may be 0). When non-null, `addrlen_ptr` must
/// point at the buffer size: at most that many bytes of the peer address
/// (sockaddr_in, or an unnamed sockaddr_un for Unix sockets) are written,
/// and its full length is stored back in `*addrlen_ptr`.
/// accept and accept4: `flags` is accept4's SOCK_NONBLOCK | SOCK_CLOEXEC
/// for the new descriptor (0 for accept); any other bit is EINVAL.
fn sys_socket_accept(
    fd: usize,
    addr_ptr: usize,
    addrlen_ptr: usize,
    flags: usize,
) -> SyscallResult {
    let (base, fd_flags) = socket_type_flags(flags)?;
    if base != 0 {
        return Err(SyscallError::InvalidArgument);
    }
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
            let new_fd = install_socket(SocketHandle::Inet(new_id), fd_flags)?;
            let peer = network_ext_syscalls::sockaddr_in_bytes(&remote);
            finish_accept(new_fd, addr_ptr, addrlen_ptr, &peer)
        }
        SocketHandle::Unix(id) => {
            let (new_id, _connecting_id) =
                crate::net::unix_socket::socket_accept(id).map_err(socket_err)?;
            let new_fd = install_socket(SocketHandle::Unix(new_id), fd_flags)?;
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

/// The Unix socket type and descriptor flags for
/// `socketpair(domain, sock_type, protocol, sv)`, checked as socket()
/// checks them. Only AF_UNIX makes pairs: AF_INET is EOPNOTSUPP, as on
/// Linux. The type and protocol used to be ignored, so a SOCK_DGRAM pair
/// silently got stream semantics (review of the v0.26.0 stack, PR #10).
fn socketpair_type(
    domain: usize,
    sock_type: usize,
    protocol: usize,
) -> Result<(crate::net::unix_socket::UnixSocketType, SocketFdFlags), SyscallError> {
    let (base_type, fd_flags) = socket_type_flags(sock_type)?;
    match socket_kind(domain, base_type, protocol)? {
        SocketKind::Unix(base_type) => Ok((to_unix_socket_type(base_type)?, fd_flags)),
        SocketKind::Inet(_) => Err(SyscallError::NotSupported),
    }
}

/// SYS_socketpair: Create a connected socket pair.
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
    let (utype, fd_flags) = socketpair_type(domain, sock_type, protocol)?;
    // Linux writes int sv[2] (two i32 values = 8 bytes).
    validate_user_buffer(result_ptr, 2 * core::mem::size_of::<i32>())?;

    let pid = crate::process::current_process()
        .map(|p| p.pid.0)
        .unwrap_or(0);

    let (id_a, id_b) =
        crate::net::unix_socket::socketpair(utype, pid).map_err(|_| SyscallError::OutOfMemory)?;
    let fd_a = install_socket(SocketHandle::Unix(id_a), fd_flags);
    let fd_b = install_socket(SocketHandle::Unix(id_b), fd_flags);
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

/// flock(fd, operation): a whole-file lock owned by the open file (dups and
/// fork children share it; another open of the file does not), released at
/// its last close. Without LOCK_NB a conflicting request sleeps until the
/// lock is granted, or fails with EINTR if a signal must be handled first
/// (N-207).
fn sys_flock(fd: usize, operation: usize) -> SyscallResult {
    use crate::fs::flock::{flock, LOCK_EX, LOCK_NB, LOCK_SH};
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = process
        .file_table
        .lock()
        .get(fd)
        .ok_or(SyscallError::BadFileDescriptor)?;
    drop(process);
    let op = operation as u32;
    let key = file.flock_key();
    let owner = alloc::sync::Arc::as_ptr(&file) as u64;
    // The open file notes that it uses flock before it can hold a lock,
    // and never forgets it, so its last close (File::drop) always releases
    // whatever it holds. Recording "held" after the fact raced with another
    // thread's flock on the same file and could leave a lock no close
    // would release.
    if matches!(op & !LOCK_NB, LOCK_SH | LOCK_EX) {
        file.flock_owner
            .store(owner, core::sync::atomic::Ordering::Release);
    }
    let result = match flock(key, owner, op) {
        #[cfg(feature = "alloc")]
        Err(crate::error::KernelError::WouldBlock)
            if op & LOCK_NB == 0 && crate::sched::dispatch::current_owner().is_some() =>
        {
            crate::sched::dispatch::wait_event(&crate::fs::flock::FLOCK_WAITERS, None, || {
                flock(key, owner, op | LOCK_NB).is_ok()
            })
            .map_err(|_| SyscallError::Interrupted)
        }
        Err(crate::error::KernelError::WouldBlock) => Err(SyscallError::WouldBlock),
        Err(_) => Err(SyscallError::InvalidArgument),
        Ok(()) => Ok(()),
    };
    result.map(|()| 0)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The generated table (ADR 0009): every call decodes from its own
    /// number, Linux calls are below 1024 and VeridianOS calls from 1024.
    #[test]
    fn syscall_numbers_round_trip() {
        for &call in numbers::ALL {
            let n = call as usize;
            assert_eq!(Syscall::try_from(n), Ok(call), "number {}", n);
            assert_eq!(call.linux_name().is_some(), n < 1024, "{:?}", call);
        }
        assert!(Syscall::try_from(9999).is_err());
        assert!(Syscall::try_from(1023).is_err());
    }

    /// Linux numbers that the old musl remap or the old Linux translation
    /// table sent to the wrong call now reach the right one.
    #[test]
    fn linux_numbers_reach_the_linux_call() {
        let cases: &[(usize, Syscall)] = &[
            (60, Syscall::Exit),
            (231, Syscall::ExitGroup),
            (73, Syscall::Flock),
            (74, Syscall::Fsync),
            (76, Syscall::Truncate),
            (77, Syscall::Ftruncate),
            (97, Syscall::Getrlimit),
            (110, Syscall::Getppid),
            (121, Syscall::Getpgid),
            (127, Syscall::RtSigpending),
            (130, Syscall::RtSigsuspend),
            (131, Syscall::Sigaltstack),
            (157, Syscall::Prctl),
            (160, Syscall::Setrlimit),
            (233, Syscall::EpollCtl),
            (260, Syscall::Fchownat),
            (262, Syscall::Newfstatat),
            (269, Syscall::Faccessat),
            (271, Syscall::Ppoll),
            (281, Syscall::EpollPwait),
            (288, Syscall::Accept4),
            (291, Syscall::EpollCreate1),
            (292, Syscall::Dup3),
            (293, Syscall::Pipe2),
            (294, Syscall::InotifyInit1),
            (439, Syscall::Faccessat2),
        ];
        for &(n, call) in cases {
            assert_eq!(Syscall::try_from(n), Ok(call), "Linux {}", n);
        }
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

    /// socketpair honours the type, the descriptor flags and the protocol
    /// (review of the v0.26.0 stack, PR #10).
    #[test]
    fn socketpair_type_maps_type_and_protocol() {
        use crate::net::unix_socket::UnixSocketType;
        let none = SocketFdFlags::default();
        let cloexec = SocketFdFlags {
            cloexec: true,
            nonblock: false,
        };
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_STREAM, 0),
            Ok((UnixSocketType::Stream, none))
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC, 0),
            Ok((UnixSocketType::Datagram, cloexec))
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_DGRAM, AF_UNIX),
            Ok((UnixSocketType::Datagram, none))
        );
        assert_eq!(
            socketpair_type(AF_UNIX, 7, 0),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            socketpair_type(AF_UNIX, SOCK_STREAM, 6),
            Err(SyscallError::ProtocolNotSupported)
        );
        assert_eq!(
            socketpair_type(AF_INET, SOCK_STREAM, 0),
            Err(SyscallError::NotSupported)
        );
        assert_eq!(
            socketpair_type(99, SOCK_STREAM, 0),
            Err(SyscallError::AddressFamilyNotSupported)
        );
    }

    /// socket() keeps SOCK_NONBLOCK / SOCK_CLOEXEC instead of masking them
    /// off, rejects other type bits, and checks the protocol.
    #[test]
    fn socket_type_flags_and_protocol_are_checked() {
        assert_eq!(
            socket_type_flags(SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC),
            Ok((
                SOCK_STREAM,
                SocketFdFlags {
                    cloexec: true,
                    nonblock: true
                }
            ))
        );
        assert_eq!(
            socket_type_flags(SOCK_STREAM | 0x100),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            socket_kind(AF_INET, SOCK_STREAM, 6),
            Ok(SocketKind::Inet(SOCK_STREAM))
        );
        assert_eq!(
            socket_kind(AF_INET, SOCK_DGRAM, 17),
            Ok(SocketKind::Inet(SOCK_DGRAM))
        );
        assert_eq!(
            socket_kind(AF_INET, SOCK_DGRAM, 6),
            Err(SyscallError::ProtocolNotSupported)
        );
        assert_eq!(
            socket_kind(AF_UNIX, SOCK_STREAM, 0),
            Ok(SocketKind::Unix(SOCK_STREAM))
        );
        assert_eq!(
            socket_kind(10, SOCK_STREAM, 0),
            Err(SyscallError::AddressFamilyNotSupported)
        );
        // accept4 takes only the two descriptor flags.
        assert_eq!(socket_type_flags(SOCK_CLOEXEC).map(|(b, _)| b), Ok(0));
        assert_eq!(
            socket_type_flags(SOCK_STREAM).map(|(b, _)| b),
            Ok(SOCK_STREAM)
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
        let result = Syscall::try_from(1024);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Syscall::IpcSend);
    }

    #[test]
    fn test_syscall_try_from_ipc_receive() {
        assert_eq!(Syscall::try_from(1025).unwrap(), Syscall::IpcReceive);
    }

    #[test]
    fn test_syscall_try_from_ipc_call() {
        assert_eq!(Syscall::try_from(1026).unwrap(), Syscall::IpcCall);
    }

    #[test]
    fn test_syscall_try_from_ipc_reply() {
        assert_eq!(Syscall::try_from(1027).unwrap(), Syscall::IpcReply);
    }

    #[test]
    fn test_syscall_try_from_process_yield() {
        assert_eq!(Syscall::try_from(24).unwrap(), Syscall::SchedYield);
    }

    #[test]
    fn test_syscall_try_from_process_exit() {
        assert_eq!(Syscall::try_from(231).unwrap(), Syscall::ExitGroup);
    }

    #[test]
    fn test_syscall_try_from_process_fork() {
        assert_eq!(Syscall::try_from(57).unwrap(), Syscall::Fork);
    }

    #[test]
    fn test_syscall_try_from_process_getpid() {
        assert_eq!(Syscall::try_from(39).unwrap(), Syscall::Getpid);
    }

    #[test]
    fn test_syscall_try_from_memory_map() {
        assert_eq!(Syscall::try_from(9).unwrap(), Syscall::Mmap);
    }

    #[test]
    fn test_syscall_try_from_capability_grant() {
        assert_eq!(Syscall::try_from(1054).unwrap(), Syscall::CapabilityGrant);
    }

    #[test]
    fn test_syscall_try_from_thread_create() {
        assert_eq!(Syscall::try_from(1064).unwrap(), Syscall::ThreadCreate);
    }

    #[test]
    fn test_syscall_try_from_file_open() {
        assert_eq!(Syscall::try_from(2).unwrap(), Syscall::Open);
    }

    #[test]
    fn test_syscall_try_from_dir_mkdir() {
        assert_eq!(Syscall::try_from(83).unwrap(), Syscall::Mkdir);
    }

    #[test]
    fn test_syscall_try_from_fs_mount() {
        assert_eq!(Syscall::try_from(1094).unwrap(), Syscall::FsMount);
    }

    #[test]
    fn test_syscall_try_from_kernel_get_info() {
        assert_eq!(Syscall::try_from(1104).unwrap(), Syscall::KernelGetInfo);
    }

    #[test]
    fn test_syscall_try_from_invalid() {
        assert!(Syscall::try_from(999).is_err());
    }

    #[test]
    fn test_syscall_try_from_gap_value() {
        // Linux numbers with no call yet, and the gap below the private
        // range, are not decoded.
        assert!(Syscall::try_from(26).is_err()); // msync
        assert!(Syscall::try_from(155).is_err()); // pivot_root
        assert!(Syscall::try_from(500).is_err());
        assert!(Syscall::try_from(1023).is_err());
    }

    // --- Syscall round-trip tests ---

    #[test]
    fn test_all_ipc_syscalls() {
        let ipc_syscalls = [
            (1024, Syscall::IpcSend),
            (1025, Syscall::IpcReceive),
            (1026, Syscall::IpcCall),
            (1027, Syscall::IpcReply),
            (1028, Syscall::IpcCreateEndpoint),
            (1029, Syscall::IpcBindEndpoint),
            (1030, Syscall::IpcShareMemory),
            (1031, Syscall::IpcMapMemory),
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
            (24, Syscall::SchedYield),
            (231, Syscall::ExitGroup),
            (57, Syscall::Fork),
            (59, Syscall::Execve),
            (61, Syscall::Wait4),
            (39, Syscall::Getpid),
            (110, Syscall::Getppid),
            (1041, Syscall::ProcessSetPriority),
            (1042, Syscall::ProcessGetPriority),
        ];

        for (num, expected) in &proc_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_thread_syscalls() {
        let thread_syscalls = [
            (1064, Syscall::ThreadCreate),
            (60, Syscall::Exit),
            (1066, Syscall::ThreadJoin),
            (186, Syscall::Gettid),
            (1068, Syscall::ThreadSetAffinity),
            (1069, Syscall::ThreadGetAffinity),
        ];

        for (num, expected) in &thread_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_file_syscalls() {
        let file_syscalls = [
            (2, Syscall::Open),
            (3, Syscall::Close),
            (0, Syscall::Read),
            (1, Syscall::Write),
            (8, Syscall::Lseek),
            (5, Syscall::Fstat),
            (77, Syscall::Ftruncate),
        ];

        for (num, expected) in &file_syscalls {
            assert_eq!(Syscall::try_from(*num).unwrap(), *expected);
        }
    }

    #[test]
    fn test_all_dir_syscalls() {
        let dir_syscalls = [
            (83, Syscall::Mkdir),
            (84, Syscall::Rmdir),
            (1086, Syscall::DirOpendir),
            (1087, Syscall::DirReaddir),
            (1088, Syscall::DirClosedir),
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
