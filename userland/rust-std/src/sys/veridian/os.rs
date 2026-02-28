//! OS-specific utilities for VeridianOS.
//!
//! Provides environment variable access, command-line argument parsing,
//! current directory operations, and other OS-level utilities.
//!
//! # Environment Variables
//!
//! VeridianOS passes environment variables on the initial stack following
//! the Linux ABI convention: after argv comes a NULL-terminated envp array.
//! The kernel also provides `SYS_PROCESS_GETENV (205)` for direct lookup.
//!
//! # Arguments
//!
//! Command-line arguments follow the standard C ABI: argc, argv on the stack.

extern crate alloc;
use alloc::{string::String, vec::Vec};

use super::{
    path::{OsStr, OsString, Path, PathBuf},
    syscall0, syscall_result, SyscallError, SYS_GETEGID, SYS_GETEUID, SYS_GETGID, SYS_GETUID,
};

// ============================================================================
// Identity Operations
// ============================================================================

/// Get the real user ID.
pub fn getuid() -> usize {
    unsafe { syscall0(SYS_GETUID) as usize }
}

/// Get the effective user ID.
pub fn geteuid() -> usize {
    unsafe { syscall0(SYS_GETEUID) as usize }
}

/// Get the real group ID.
pub fn getgid() -> usize {
    unsafe { syscall0(SYS_GETGID) as usize }
}

/// Get the effective group ID.
pub fn getegid() -> usize {
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
        let entry = unsafe { *envp.add(i) };
        if entry.is_null() {
            break;
        }

        // Compare name prefix.
        let mut matches = true;
        for (j, &name_byte) in name.iter().enumerate() {
            let entry_byte = unsafe { *entry.add(j) };
            if entry_byte != name_byte {
                matches = false;
                break;
            }
        }

        if matches {
            let sep = unsafe { *entry.add(name.len()) };
            if sep == b'=' {
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
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

/// Look up an environment variable by name via the kernel's
/// `SYS_PROCESS_GETENV (205)` syscall.
///
/// Returns the value as an `OsString`, or `None` if not found.
pub fn getenv(name: &OsStr) -> Option<OsString> {
    use super::{syscall4, SYS_PROCESS_GETENV};

    let mut buf = [0u8; 4096];
    let name_bytes = name.as_bytes();

    let ret = unsafe {
        syscall4(
            SYS_PROCESS_GETENV,
            name_bytes.as_ptr() as usize,
            name_bytes.len(),
            buf.as_mut_ptr() as usize,
            buf.len(),
        )
    };

    match syscall_result(ret) {
        Ok(len) => Some(OsString::from_vec(buf[..len].to_vec())),
        Err(_) => None,
    }
}

/// Look up an environment variable by name (string convenience).
pub fn var(name: &str) -> Option<OsString> {
    getenv(OsStr::new(name))
}

/// Look up an environment variable and return it as a `String`.
pub fn var_string(name: &str) -> Option<String> {
    var(name).and_then(|os| os.into_string().ok())
}

/// Read all environment variables from the process's envp.
///
/// This requires access to the raw envp pointer, which is typically
/// passed to `main` or stored by the CRT startup code.  Since we are
/// `no_std`, this function takes the envp pointer as an argument.
///
/// Each entry is returned as a `(key, value)` pair of `OsString`.
///
/// # Safety
/// `envp` must point to a valid null-terminated array of
/// null-terminated "KEY=VALUE" strings.
pub unsafe fn vars_from_envp(envp: *const *const u8) -> Vec<(OsString, OsString)> {
    let mut result = Vec::new();
    if envp.is_null() {
        return result;
    }

    let mut i = 0;
    loop {
        let entry = unsafe { *envp.add(i) };
        if entry.is_null() {
            break;
        }

        let len = unsafe { c_strlen(entry) };
        let bytes = unsafe { core::slice::from_raw_parts(entry, len) };

        // Find the '=' separator.
        if let Some(eq_pos) = bytes.iter().position(|&b| b == b'=') {
            let key = OsString::from_vec(bytes[..eq_pos].to_vec());
            let val = OsString::from_vec(bytes[eq_pos + 1..].to_vec());
            result.push((key, val));
        }

        i += 1;
    }

    result
}

/// Read command-line arguments from the process's argv.
///
/// # Safety
/// `argc` must be the argument count and `argv` must point to a valid
/// array of `argc` null-terminated string pointers.
pub unsafe fn args_from_argv(argc: usize, argv: *const *const u8) -> Vec<OsString> {
    let mut result = Vec::with_capacity(argc);
    for i in 0..argc {
        let ptr = unsafe { *argv.add(i) };
        if ptr.is_null() {
            break;
        }
        let len = unsafe { c_strlen(ptr) };
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        result.push(OsString::from_vec(bytes.to_vec()));
    }
    result
}

// ============================================================================
// Current directory
// ============================================================================

/// Get the current working directory.
pub fn current_dir() -> Result<PathBuf, SyscallError> {
    super::process::current_dir()
}

/// Set the current working directory.
pub fn set_current_dir(path: &Path) -> Result<(), SyscallError> {
    super::process::set_current_dir(path)
}

// ============================================================================
// Home directory
// ============================================================================

/// Get the home directory for the current user.
///
/// Checks the `HOME` environment variable first.  Falls back to `/root`
/// for uid 0 or `/home/<user>` otherwise (simplified, no /etc/passwd).
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = var("HOME") {
        return Some(PathBuf::from(home));
    }
    // Fallback based on uid.
    let uid = getuid();
    if uid == 0 {
        Some(PathBuf::from_str("/root"))
    } else {
        // Without /etc/passwd parsing, we cannot determine the username.
        None
    }
}

// ============================================================================
// Kernel Info
// ============================================================================

/// Get kernel information.
///
/// Writes kernel info (version, etc.) to the provided buffer.
pub fn kernel_info(buf: *mut u8) -> Result<usize, SyscallError> {
    use super::{syscall1, SYS_KERNEL_GET_INFO};

    let ret = unsafe { syscall1(SYS_KERNEL_GET_INFO, buf as usize) };
    syscall_result(ret)
}

// ============================================================================
// Hostname
// ============================================================================

/// Get the hostname via the kernel's uname syscall.
///
/// Returns the hostname as a byte vector on success.
pub fn hostname() -> Result<OsString, SyscallError> {
    use super::{syscall1, SYS_PROCESS_UNAME};

    // uname fills a utsname structure.  We only need the nodename field.
    // The exact layout matches the kernel's UtsName.  For simplicity we
    // read the full struct and extract the hostname.
    #[repr(C)]
    struct Utsname {
        sysname: [u8; 65],
        nodename: [u8; 65],
        release: [u8; 65],
        version: [u8; 65],
        machine: [u8; 65],
    }

    let mut buf = Utsname {
        sysname: [0; 65],
        nodename: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
    };

    let ret = unsafe { syscall1(SYS_PROCESS_UNAME, &mut buf as *mut Utsname as usize) };
    syscall_result(ret)?;

    // Find the NUL terminator in nodename.
    let len = buf
        .nodename
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(buf.nodename.len());
    Ok(OsString::from_vec(buf.nodename[..len].to_vec()))
}
