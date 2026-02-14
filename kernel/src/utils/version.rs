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
    pub build_timestamp: u64,
    pub supported_archs: u64,
}

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
        build_timestamp: env!("BUILD_TIMESTAMP").parse().unwrap_or(0),
        supported_archs: (1 << 0) | (1 << 1) | (1 << 2), // x86_64, AArch64, RISC-V
    }
}
