//! Thread operations for VeridianOS.
//!
//! Maps Rust threading operations to VeridianOS syscalls:
//! - `clone` -> SYS_THREAD_CLONE (46)
//! - `futex_wait` -> SYS_FUTEX_WAIT (201)
//! - `futex_wake` -> SYS_FUTEX_WAKE (202)
//! - `thread_exit` -> SYS_THREAD_EXIT (41)
//! - `gettid` -> SYS_THREAD_GETTID (43)
//!
//! Thread creation uses `clone` with shared address space flags.
//! Synchronization primitives (mutex, condvar) can be built on top
//! of the futex interface.

use super::{
    syscall0, syscall1, syscall3, syscall5, syscall_result, SyscallError, SYS_FUTEX_WAIT,
    SYS_FUTEX_WAKE, SYS_THREAD_CLONE, SYS_THREAD_EXIT, SYS_THREAD_GETTID,
};

// ============================================================================
// Clone flags (must match kernel definitions)
// ============================================================================

/// Share virtual memory with parent.
pub const CLONE_VM: usize = 0x0000_0100;
/// Share filesystem info.
pub const CLONE_FS: usize = 0x0000_0200;
/// Share file descriptors.
pub const CLONE_FILES: usize = 0x0000_0400;
/// Share signal handlers.
pub const CLONE_SIGHAND: usize = 0x0000_0800;
/// Same thread group as parent.
pub const CLONE_THREAD: usize = 0x0001_0000;
/// Set TLS for new thread.
pub const CLONE_SETTLS: usize = 0x0008_0000;
/// Store child TID at parent_tidptr in parent.
pub const CLONE_PARENT_SETTID: usize = 0x0010_0000;
/// Clear child TID at child_tidptr on exit (for futex wake).
pub const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
/// Store child TID at child_tidptr in child.
pub const CLONE_CHILD_SETTID: usize = 0x0100_0000;

/// Default flags for creating a pthread-like thread.
pub const THREAD_FLAGS: usize =
    CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SETTLS;

// ============================================================================
// Futex operations
// ============================================================================

/// Futex wait: sleep if `*uaddr == expected`.
pub const FUTEX_WAIT: usize = 0;
/// Futex wake: wake up to `count` waiters on `uaddr`.
pub const FUTEX_WAKE: usize = 1;

// ============================================================================
// Thread Operations
// ============================================================================

/// Create a new thread via clone.
///
/// # Arguments
/// - `flags`: Clone flags (use THREAD_FLAGS for standard threads)
/// - `stack`: Top of the new thread's stack
/// - `parent_tidptr`: Where to store child TID in parent (or null)
/// - `child_tidptr`: Where to store child TID in child (or null)
/// - `tls`: Thread-local storage pointer (or null)
///
/// # Returns
/// Thread ID of the new thread in the parent, 0 in the child.
pub fn clone(
    flags: usize,
    stack: *mut u8,
    parent_tidptr: *mut i32,
    child_tidptr: *mut i32,
    tls: usize,
) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid stack pointer and valid (or null)
    // tid pointers. The kernel validates all pointer arguments.
    let ret = unsafe {
        syscall5(
            SYS_THREAD_CLONE,
            flags,
            stack as usize,
            parent_tidptr as usize,
            child_tidptr as usize,
            tls,
        )
    };
    syscall_result(ret)
}

/// Exit the current thread.
///
/// This function does not return.
pub fn thread_exit(status: usize) -> ! {
    // SAFETY: thread_exit terminates the calling thread.
    unsafe {
        syscall1(SYS_THREAD_EXIT, status);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Get the current thread ID.
pub fn gettid() -> usize {
    // SAFETY: gettid never fails.
    unsafe { syscall0(SYS_THREAD_GETTID) as usize }
}

/// Futex wait: block the calling thread if `*uaddr == expected`.
///
/// # Arguments
/// - `uaddr`: Pointer to the futex word (must be aligned to i32)
/// - `expected`: Expected value at *uaddr
/// - `timeout_ms`: Timeout in milliseconds (0 = no timeout)
///
/// # Returns
/// 0 on success (woken up), error on failure.
pub fn futex_wait(uaddr: *const i32, expected: i32, timeout_ms: u64) -> Result<usize, SyscallError> {
    let timeout_ptr = if timeout_ms > 0 {
        &timeout_ms as *const u64 as usize
    } else {
        0
    };
    let timeout_size = if timeout_ms > 0 {
        core::mem::size_of::<u64>()
    } else {
        0
    };
    // SAFETY: Caller must provide a valid pointer to an aligned i32.
    let ret = unsafe {
        syscall5(
            SYS_FUTEX_WAIT,
            uaddr as usize,
            expected as usize,
            timeout_ptr,
            timeout_size,
            0,
        )
    };
    syscall_result(ret)
}

/// Futex wake: wake up to `count` threads waiting on `uaddr`.
///
/// # Arguments
/// - `uaddr`: Pointer to the futex word
/// - `count`: Maximum number of waiters to wake
///
/// # Returns
/// Number of threads woken.
pub fn futex_wake(uaddr: *const i32, count: i32) -> Result<usize, SyscallError> {
    // SAFETY: Caller must provide a valid pointer to an aligned i32.
    let ret = unsafe { syscall3(SYS_FUTEX_WAKE, uaddr as usize, count as usize, 0) };
    syscall_result(ret)
}
