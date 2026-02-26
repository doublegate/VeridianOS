//! SDK Core Types for VeridianOS Package Development
//!
//! Provides toolchain information, build target configuration, and sysroot
//! management for building VeridianOS packages. These types define the SDK
//! contract for user-space library and application development.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space SDK forward declarations -- see module doc TODO(user-space)

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

pub mod generator;
pub mod pkg_config;
pub mod syscall_api;
pub mod toolchain;

/// Return the sysroot path for the VeridianOS SDK.
pub fn get_sysroot() -> &'static str {
    "/usr/veridian"
}

/// Return the target triple for the current architecture.
pub fn get_target_triple() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-veridian"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-veridian"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64gc-veridian"
    }
}

/// Build target specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildTarget {
    /// Build for the host architecture.
    Native,
    /// Cross-compile for the specified target triple.
    #[cfg(feature = "alloc")]
    Cross(String),
}

/// Information about the active toolchain.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ToolchainInfo {
    /// Path to the compiler binary.
    pub compiler_path: String,
    /// Path to the linker binary.
    pub linker_path: String,
    /// Target triple (e.g. "x86_64-veridian").
    pub target_triple: String,
    /// Sysroot directory containing headers and libraries.
    pub sysroot_path: String,
    /// Toolchain version string.
    pub version: String,
}

#[cfg(feature = "alloc")]
impl ToolchainInfo {
    /// Return toolchain information for the current architecture.
    pub fn current() -> Self {
        let triple = String::from(get_target_triple());
        let sysroot = String::from(get_sysroot());

        Self {
            compiler_path: alloc::format!("{}/bin/{}-gcc", sysroot, triple),
            linker_path: alloc::format!("{}/bin/{}-ld", sysroot, triple),
            target_triple: triple,
            sysroot_path: sysroot,
            version: String::from("0.4.0"),
        }
    }
}

/// SDK configuration controlling compiler and linker flags.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SdkConfig {
    /// Default C compiler flags.
    pub default_cflags: Vec<String>,
    /// Default linker flags.
    pub default_ldflags: Vec<String>,
    /// Sysroot directory.
    pub sysroot: String,
    /// Include search paths.
    pub include_paths: Vec<String>,
    /// Library search paths.
    pub lib_paths: Vec<String>,
}

#[cfg(feature = "alloc")]
impl SdkConfig {
    /// Create a new SDK configuration with sensible defaults.
    pub fn new() -> Self {
        Self::for_target(BuildTarget::Native)
    }

    /// Create an SDK configuration for a specific build target.
    pub fn for_target(target: BuildTarget) -> Self {
        let sysroot = String::from(get_sysroot());

        let triple = match &target {
            BuildTarget::Native => String::from(get_target_triple()),
            BuildTarget::Cross(t) => t.clone(),
        };

        let include_base = alloc::format!("{}/include", sysroot);
        let lib_base = alloc::format!("{}/lib/{}", sysroot, triple);

        Self {
            default_cflags: vec![
                String::from("-ffreestanding"),
                String::from("-nostdlib"),
                alloc::format!("--sysroot={}", sysroot),
                alloc::format!("--target={}", triple),
            ],
            default_ldflags: vec![
                String::from("-nostdlib"),
                alloc::format!("-L{}", lib_base),
                String::from("-lveridian"),
            ],
            sysroot,
            include_paths: vec![
                include_base.clone(),
                alloc::format!("{}/veridian", include_base),
            ],
            lib_paths: vec![lib_base],
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for SdkConfig {
    fn default() -> Self {
        Self::new()
    }
}
