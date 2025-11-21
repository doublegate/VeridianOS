//! Syscall for getting kernel information.

use core::mem::size_of;

use crate::{
    syscall::{SyscallError, SyscallResult},
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
    if buf == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let version_info = get_version_info();

    // Copy the version info to the user buffer
    unsafe {
        let user_buf = buf as *mut KernelVersionInfo;
        *user_buf = version_info;
    }

    Ok(size_of::<KernelVersionInfo>())
}
