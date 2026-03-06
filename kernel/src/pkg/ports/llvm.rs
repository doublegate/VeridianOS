//! LLVM Native Build Port
//!
//! Portfile for building LLVM 19 natively on VeridianOS. Handles CMake
//! configuration, memory validation, single-threaded linking, and
//! installation to the system toolchain directory.

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{
    error::KernelError,
    pkg::{build_system::BuildConfig, ports::BuildType},
};

/// LLVM version
pub const LLVM_VERSION: &str = "19.1.0";

/// Minimum memory required for LLVM build (4GB)
pub const LLVM_MIN_MEMORY_MB: u64 = 4096;

/// LLVM build configuration
pub struct LlvmPortConfig {
    pub version: String,
    pub targets: Vec<String>,
    pub enable_assertions: bool,
    pub single_threaded_link: bool,
    pub build_type: String,
    pub install_prefix: String,
}

impl Default for LlvmPortConfig {
    fn default() -> Self {
        Self {
            version: LLVM_VERSION.to_string(),
            targets: vec![
                "X86".to_string(),
                "AArch64".to_string(),
                "RISCV".to_string(),
            ],
            enable_assertions: false,
            single_threaded_link: true,
            build_type: "Release".to_string(),
            install_prefix: "/usr/local".to_string(),
        }
    }
}

impl LlvmPortConfig {
    /// Generate CMake arguments
    pub fn cmake_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        args.push(alloc::format!(
            "-DCMAKE_INSTALL_PREFIX={}",
            self.install_prefix
        ));
        args.push(alloc::format!("-DCMAKE_BUILD_TYPE={}", self.build_type));
        args.push(alloc::format!(
            "-DLLVM_TARGETS_TO_BUILD={}",
            self.targets.join(";")
        ));

        if self.enable_assertions {
            args.push("-DLLVM_ENABLE_ASSERTIONS=ON".to_string());
        } else {
            args.push("-DLLVM_ENABLE_ASSERTIONS=OFF".to_string());
        }

        // Optimize TableGen for faster builds
        args.push("-DLLVM_OPTIMIZED_TABLEGEN=ON".to_string());

        // Single-threaded link to reduce memory usage
        if self.single_threaded_link {
            args.push("-DLLVM_PARALLEL_LINK_JOBS=1".to_string());
        }

        // Disable unused projects
        args.push("-DLLVM_ENABLE_PROJECTS=clang;lld".to_string());
        args.push("-DLLVM_ENABLE_RUNTIMES=".to_string());

        // Use lld for faster linking
        args.push("-DLLVM_USE_LINKER=lld".to_string());

        // Minimize build size
        args.push("-DLLVM_BUILD_DOCS=OFF".to_string());
        args.push("-DLLVM_BUILD_TESTS=OFF".to_string());
        args.push("-DLLVM_INCLUDE_TESTS=OFF".to_string());
        args.push("-DLLVM_INCLUDE_EXAMPLES=OFF".to_string());
        args.push("-DLLVM_INCLUDE_BENCHMARKS=OFF".to_string());

        args
    }

    /// Generate BuildConfig for the build orchestrator
    pub fn to_build_config(&self) -> BuildConfig {
        let mut config = BuildConfig::new("llvm", &self.version);
        config.source_url = alloc::format!(
            "https://github.com/llvm/llvm-project/releases/download/llvmorg-{}/llvm-project-{}.src.tar.xz",
            self.version, self.version
        );
        config.build_type = BuildType::CMake;
        config.configure_flags = self.cmake_args();
        config.install_prefix = self.install_prefix.clone();
        config
    }

    /// Validate system has enough resources
    pub fn validate_resources(&self, available_memory_mb: u64) -> Result<(), KernelError> {
        if available_memory_mb < LLVM_MIN_MEMORY_MB {
            return Err(KernelError::OutOfMemory {
                requested: LLVM_MIN_MEMORY_MB as usize,
                available: available_memory_mb as usize,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LlvmPortConfig::default();
        assert_eq!(config.version, LLVM_VERSION);
        assert!(config.targets.contains(&"X86".to_string()));
        assert!(config.single_threaded_link);
    }

    #[test]
    fn test_cmake_args() {
        let config = LlvmPortConfig::default();
        let args = config.cmake_args();
        assert!(args.iter().any(|a| a.contains("CMAKE_INSTALL_PREFIX")));
        assert!(args.iter().any(|a| a.contains("LLVM_TARGETS_TO_BUILD")));
        assert!(args.iter().any(|a| a.contains("OPTIMIZED_TABLEGEN")));
    }

    #[test]
    fn test_cmake_args_assertions() {
        let mut config = LlvmPortConfig::default();
        config.enable_assertions = true;
        let args = config.cmake_args();
        assert!(args.iter().any(|a| a == "-DLLVM_ENABLE_ASSERTIONS=ON"));
    }

    #[test]
    fn test_to_build_config() {
        let config = LlvmPortConfig::default();
        let build = config.to_build_config();
        assert_eq!(build.name, "llvm");
        assert_eq!(build.build_type, BuildType::CMake);
        assert!(build.source_url.contains("llvm-project"));
    }

    #[test]
    fn test_validate_resources_ok() {
        let config = LlvmPortConfig::default();
        assert!(config.validate_resources(8192).is_ok());
    }

    #[test]
    fn test_validate_resources_oom() {
        let config = LlvmPortConfig::default();
        assert!(config.validate_resources(2048).is_err());
    }

    #[test]
    fn test_version() {
        assert!(!LLVM_VERSION.is_empty());
    }

    #[test]
    fn test_min_memory() {
        assert!(LLVM_MIN_MEMORY_MB >= 4096);
    }
}
