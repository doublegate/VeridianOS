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

    // Fault-tolerant copy (N-43); the struct has no implicit padding.
    crate::syscall::userspace::write_user(buf, version_info)?;

    Ok(size_of::<KernelVersionInfo>())
}

/// Linux utsname layout: six 65-byte fields (sysname, nodename, release,
/// version, machine, domainname). Total size = 390 bytes.
/// musl allocates all 6 fields; we must write all of them.
const UTSNAME_LENGTH: usize = 65;
const UTSNAME_SIZE: usize = UTSNAME_LENGTH * 6;

/// The utsname a process with system name `sysname` sees: sysname,
/// nodename, release, version, machine and domainname, each NUL-padded to
/// 65 bytes (ADR 0021).
fn utsname_bytes(sysname: &str) -> [u8; UTSNAME_SIZE] {
    use crate::utils::uname;

    let mut out = [0u8; UTSNAME_SIZE];
    // The fields hold their value and at least one NUL.
    let fields = [
        sysname,
        uname::NODENAME,
        uname::RELEASE,
        uname::VERSION,
        uname::machine(),
        // domainname: no NIS domain
        "",
    ];
    for (i, value) in fields.iter().enumerate() {
        let len = core::cmp::min(value.len(), UTSNAME_LENGTH - 1);
        let at = i * UTSNAME_LENGTH;
        out[at..at + len].copy_from_slice(&value.as_bytes()[..len]);
    }
    out
}

/// Fill a user-space utsname struct with system identification.
///
/// # Arguments
/// * `buf` - Pointer to a user-space utsname struct (390 bytes).
pub fn sys_uname(buf: usize) -> SyscallResult {
    validate_user_buffer(buf, UTSNAME_SIZE)?;
    // Built in a kernel buffer and copied out once (N-43).
    let out = utsname_bytes(crate::utils::uname::SYSNAME);
    super::userspace::write_user_bytes(buf, &out)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(out: &[u8; UTSNAME_SIZE], i: usize) -> &str {
        let f = &out[i * UTSNAME_LENGTH..(i + 1) * UTSNAME_LENGTH];
        let end = f.iter().position(|&b| b == 0).unwrap();
        assert!(f[end..].iter().all(|&b| b == 0), "field {i} is NUL-padded");
        core::str::from_utf8(&f[..end]).unwrap()
    }

    #[test]
    fn uname_reports_a_linux_release_and_the_veridian_version() {
        let out = utsname_bytes("VeridianOS");
        assert_eq!(field(&out, 0), "VeridianOS");
        assert_eq!(field(&out, 1), "veridian");
        assert_eq!(field(&out, 2), "7.3.0-veridian");
        assert_eq!(
            field(&out, 3),
            concat!("#1 SMP VeridianOS ", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(field(&out, 4), crate::utils::uname::machine());
        assert_eq!(field(&out, 5), "");
    }
}
