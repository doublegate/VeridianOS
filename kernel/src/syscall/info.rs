//! Syscall for getting kernel information.

use core::mem::size_of;

#[allow(unused_imports)]
use crate::{
    syscall::{validate_user_buffer, validate_user_ptr_typed, SyscallError, SyscallResult},
    utils::version::{get_version_info, KernelVersionInfo},
};

/// Copies kernel version information to a user-provided buffer.
///
/// # Arguments
/// * `buf` - A pointer to a `KernelVersionInfo` struct in userspace.
///
/// # Returns
/// A `SyscallResult` indicating the outcome of the operation.
pub fn sys_get_kernel_info(buf: usize) -> SyscallResult {
    // Validate buffer is in user space and properly aligned for KernelVersionInfo
    validate_user_ptr_typed::<KernelVersionInfo>(buf)?;

    let version_info = get_version_info();

    // Copy the version info to the user buffer
    // SAFETY: buf was validated as non-null, in user-space, properly sized,
    // and aligned for KernelVersionInfo above. The struct is Copy and
    // repr(C), so the write through the pointer is well-defined.
    unsafe {
        let user_buf = buf as *mut KernelVersionInfo;
        *user_buf = version_info;
    }

    Ok(size_of::<KernelVersionInfo>())
}

/// POSIX uname() layout: five 65-byte fields (sysname, nodename, release,
/// version, machine). Total size = 325 bytes.
const UTSNAME_LENGTH: usize = 65;
const UTSNAME_SIZE: usize = UTSNAME_LENGTH * 5;

/// Fill a user-space utsname struct with system identification.
///
/// # Arguments
/// * `buf` - Pointer to a user-space utsname struct (325 bytes).
pub fn sys_uname(buf: usize) -> SyscallResult {
    validate_user_buffer(buf, UTSNAME_SIZE)?;

    let user_buf = buf as *mut u8;

    // Helper: write a string into a fixed-size field, NUL-padded.
    let write_field = |offset: usize, value: &[u8]| {
        let field =
            unsafe { core::slice::from_raw_parts_mut(user_buf.add(offset), UTSNAME_LENGTH) };
        let len = core::cmp::min(value.len(), UTSNAME_LENGTH - 1);
        field[..len].copy_from_slice(&value[..len]);
        // Zero the rest
        for byte in &mut field[len..] {
            *byte = 0;
        }
    };

    // Field offsets: sysname=0, nodename=65, release=130, version=195, machine=260
    write_field(0, b"VeridianOS");
    write_field(UTSNAME_LENGTH, b"veridian");
    write_field(UTSNAME_LENGTH * 2, b"0.5.0");
    write_field(UTSNAME_LENGTH * 3, b"#1 SMP");

    #[cfg(target_arch = "x86_64")]
    write_field(UTSNAME_LENGTH * 4, b"x86_64");
    #[cfg(target_arch = "aarch64")]
    write_field(UTSNAME_LENGTH * 4, b"aarch64");
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    write_field(UTSNAME_LENGTH * 4, b"riscv64");

    Ok(0)
}
