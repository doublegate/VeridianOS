//! Package management system calls
//!
//! Provides kernel-side implementation of package management operations
//! for user-space package tools. All operations delegate to the
//! `pkg::PackageManager` singleton after validating arguments and
//! checking capabilities.

use super::{SyscallError, SyscallResult};

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

/// Read a null-terminated string from a user-space pointer.
///
/// Returns the string contents up to `max_len` bytes.
fn read_user_string(ptr: usize, max_len: usize) -> Result<String, SyscallError> {
    if ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // SAFETY: ptr was validated as non-zero above. We read bytes one at a
    // time from the user-space pointer until we find a null terminator or
    // reach the max_len limit. The caller must provide a valid, null-
    // terminated string in mapped user memory.
    let bytes = unsafe {
        let mut buf = Vec::new();
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

    let s = core::str::from_utf8(&bytes).map_err(|_| SyscallError::InvalidArgument)?;
    Ok(String::from(s))
}

/// Install a package by name (SYS_PKG_INSTALL = 90)
///
/// # Arguments
/// - `name_ptr`: Pointer to null-terminated package name string
/// - `name_len`: Maximum length of name string (unused, reads until null)
///
/// # Returns
/// 0 on success
pub fn sys_pkg_install(name_ptr: usize, _name_len: usize) -> SyscallResult {
    let name = read_user_string(name_ptr, 256)?;

    // Check that the calling process has appropriate capabilities
    // Package installation is a privileged operation
    let current = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current.capability_space.lock();
    let has_perm = {
        let mut found = false;
        #[cfg(feature = "alloc")]
        {
            let _ = cap_space.iter_capabilities(|entry| {
                if entry.rights.contains(crate::cap::Rights::WRITE)
                    && entry.rights.contains(crate::cap::Rights::CREATE)
                {
                    found = true;
                    return false;
                }
                true
            });
        }
        found
    };
    if !has_perm {
        return Err(SyscallError::PermissionDenied);
    }

    match crate::pkg::with_package_manager(|mgr| mgr.install(name.clone(), String::from("*"))) {
        Some(Ok(())) => Ok(0),
        Some(Err(_)) => Err(SyscallError::InvalidState),
        None => Err(SyscallError::InvalidState),
    }
}

/// Remove a package (SYS_PKG_REMOVE = 91)
///
/// # Arguments
/// - `name_ptr`: Pointer to null-terminated package name string
/// - `name_len`: Maximum length of name string (unused, reads until null)
///
/// # Returns
/// 0 on success
pub fn sys_pkg_remove(name_ptr: usize, _name_len: usize) -> SyscallResult {
    let name = read_user_string(name_ptr, 256)?;

    // Check capabilities -- removal is a privileged operation
    let current = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current.capability_space.lock();
    let has_perm = {
        let mut found = false;
        #[cfg(feature = "alloc")]
        {
            let _ = cap_space.iter_capabilities(|entry| {
                if entry.rights.contains(crate::cap::Rights::WRITE)
                    && entry.rights.contains(crate::cap::Rights::CREATE)
                {
                    found = true;
                    return false;
                }
                true
            });
        }
        found
    };
    if !has_perm {
        return Err(SyscallError::PermissionDenied);
    }

    match crate::pkg::with_package_manager(|mgr| mgr.remove(&name)) {
        Some(Ok(())) => Ok(0),
        Some(Err(_)) => Err(SyscallError::ResourceNotFound),
        None => Err(SyscallError::InvalidState),
    }
}

/// Query package information (SYS_PKG_QUERY = 92)
///
/// # Arguments
/// - `name_ptr`: Pointer to null-terminated package name string
/// - `info_buf`: Pointer to buffer for writing package info
///
/// # Returns
/// 1 if package is installed, 0 if not found
pub fn sys_pkg_query(name_ptr: usize, _info_buf: usize) -> SyscallResult {
    let name = read_user_string(name_ptr, 256)?;

    match crate::pkg::with_package_manager(|mgr| mgr.is_installed(&name)) {
        Some(true) => Ok(1),
        Some(false) => Ok(0),
        None => Err(SyscallError::InvalidState),
    }
}

/// List installed packages (SYS_PKG_LIST = 93)
///
/// # Arguments
/// - `buf_ptr`: Pointer to buffer for writing package count
/// - `buf_size`: Size of the buffer
///
/// # Returns
/// Number of installed packages
pub fn sys_pkg_list(buf_ptr: usize, _buf_size: usize) -> SyscallResult {
    let count = match crate::pkg::with_package_manager(|mgr| mgr.list_installed().len()) {
        Some(n) => n,
        None => return Err(SyscallError::InvalidState),
    };

    // If a buffer was provided, write the count there
    if buf_ptr != 0 {
        // SAFETY: buf_ptr was validated as non-zero. The caller must provide
        // a valid, writable pointer to a usize-sized buffer in mapped user
        // memory. We write a single usize value (the package count).
        unsafe {
            let out = buf_ptr as *mut usize;
            *out = count;
        }
    }

    Ok(count)
}

/// Update repository index (SYS_PKG_UPDATE = 94)
///
/// # Arguments
/// - `flags`: Reserved for future use (must be 0)
///
/// # Returns
/// 0 on success
pub fn sys_pkg_update(_flags: usize) -> SyscallResult {
    // Check capabilities -- updating repos is a privileged operation
    let current = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let cap_space = current.capability_space.lock();
    let has_perm = {
        let mut found = false;
        #[cfg(feature = "alloc")]
        {
            let _ = cap_space.iter_capabilities(|entry| {
                if entry.rights.contains(crate::cap::Rights::WRITE) {
                    found = true;
                    return false;
                }
                true
            });
        }
        found
    };
    if !has_perm {
        return Err(SyscallError::PermissionDenied);
    }

    match crate::pkg::with_package_manager(|mgr| mgr.update()) {
        Some(Ok(())) => Ok(0),
        Some(Err(_)) => Err(SyscallError::InvalidState),
        None => Err(SyscallError::InvalidState),
    }
}
