//! OS-specific utilities for VeridianOS.
//!
//! Provides environment variable access, command-line argument parsing,
//! and other OS-level utilities.
//!
//! # Environment Variables
//!
//! VeridianOS passes environment variables on the initial stack following
//! the Linux ABI convention: after argv comes a NULL-terminated envp array.
//! This module caches them for fast lookup.
//!
//! # Arguments
//!
//! Command-line arguments follow the standard C ABI: argc, argv on the stack.

use super::{
    syscall0, syscall_result, SyscallError, SYS_GETEGID, SYS_GETEUID, SYS_GETGID, SYS_GETUID,
};

// ============================================================================
// Identity Operations
// ============================================================================

/// Get the real user ID.
pub fn getuid() -> usize {
    // SAFETY: getuid never fails.
    unsafe { syscall0(SYS_GETUID) as usize }
}

/// Get the effective user ID.
pub fn geteuid() -> usize {
    // SAFETY: geteuid never fails.
    unsafe { syscall0(SYS_GETEUID) as usize }
}

/// Get the real group ID.
pub fn getgid() -> usize {
    // SAFETY: getgid never fails.
    unsafe { syscall0(SYS_GETGID) as usize }
}

/// Get the effective group ID.
pub fn getegid() -> usize {
    // SAFETY: getegid never fails.
    unsafe { syscall0(SYS_GETEGID) as usize }
}

// ============================================================================
// Environment Variables
// ============================================================================

/// Search for an environment variable in a null-terminated envp array.
///
/// # Arguments
/// - `envp`: Pointer to a null-terminated array of "KEY=VALUE\0" C strings
/// - `name`: The variable name to look for (as a byte slice, no '=')
///
/// # Returns
/// Pointer to the value portion (after '=') if found, or null.
///
/// # Safety
/// Caller must ensure `envp` points to a valid null-terminated array
/// of null-terminated strings.
pub unsafe fn getenv_from_envp(envp: *const *const u8, name: &[u8]) -> *const u8 {
    if envp.is_null() {
        return core::ptr::null();
    }

    let mut i = 0;
    loop {
        // SAFETY: Caller guarantees envp is a valid null-terminated array.
        let entry = unsafe { *envp.add(i) };
        if entry.is_null() {
            break;
        }

        // Compare name prefix
        let mut matches = true;
        for (j, &name_byte) in name.iter().enumerate() {
            // SAFETY: entry is a valid null-terminated string from envp.
            let entry_byte = unsafe { *entry.add(j) };
            if entry_byte != name_byte {
                matches = false;
                break;
            }
        }

        if matches {
            // Check that the character after the name is '='
            // SAFETY: We checked all name bytes matched, so entry is valid up to name.len().
            let sep = unsafe { *entry.add(name.len()) };
            if sep == b'=' {
                // SAFETY: Return pointer to the byte after '='.
                return unsafe { entry.add(name.len() + 1) };
            }
        }

        i += 1;
    }

    core::ptr::null()
}

/// Get the length of a null-terminated C string.
///
/// # Safety
/// Caller must ensure `s` points to a valid null-terminated string.
pub unsafe fn c_strlen(s: *const u8) -> usize {
    let mut len = 0;
    // SAFETY: Caller guarantees s is a valid null-terminated string.
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

// ============================================================================
// Kernel Info
// ============================================================================

/// Get kernel information.
///
/// Writes kernel info (version, etc.) to the provided buffer.
pub fn kernel_info(buf: *mut u8) -> Result<usize, SyscallError> {
    use super::{syscall1, SYS_KERNEL_GET_INFO};

    // SAFETY: Caller must provide a valid buffer pointer.
    let ret = unsafe { syscall1(SYS_KERNEL_GET_INFO, buf as usize) };
    syscall_result(ret)
}
