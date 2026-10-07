//! Kernel version information
//!
//! Provides compile-time version metadata including semantic version,
//! git hash, and build timestamp. Accessible via the `SYS_VERSION` syscall.

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KernelVersionInfo {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub git_hash: [u8; 40],
    /// Explicit padding (always zero), so copying the struct to user space
    /// discloses no uninitialised kernel stack bytes (N-43).
    pub _pad: [u8; 2],
    pub build_timestamp: u64,
    pub supported_archs: u64,
}

const _: () = {
    assert!(core::mem::offset_of!(KernelVersionInfo, _pad) == 46);
    assert!(core::mem::offset_of!(KernelVersionInfo, build_timestamp) == 48);
    assert!(core::mem::size_of::<KernelVersionInfo>() == 64);
};

/// Returns the kernel version information.
pub fn get_version_info() -> KernelVersionInfo {
    // These values would typically be populated by the build script.
    let git_hash_str = env!("GIT_HASH", "0000000000000000000000000000000000000000");
    let mut git_hash = [0u8; 40];
    git_hash.copy_from_slice(git_hash_str.as_bytes());

    KernelVersionInfo {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0),
        patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0),
        git_hash,
        _pad: [0; 2],
        build_timestamp: env!("BUILD_TIMESTAMP").parse().unwrap_or(0),
        supported_archs: (1 << 0) | (1 << 1) | (1 << 2), // x86_64, AArch64, RISC-V
    }
}
