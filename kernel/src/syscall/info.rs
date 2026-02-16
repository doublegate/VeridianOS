//! Syscall for getting kernel information.

use core::mem::size_of;

#[allow(unused_imports)]
use crate::{
    syscall::{validate_user_ptr_typed, SyscallError, SyscallResult},
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
