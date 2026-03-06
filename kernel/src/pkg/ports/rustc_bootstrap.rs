//! Rustc Bootstrap (Stage 0 -> Stage 1 -> Stage 2)
//!
//! Bootstraps the Rust compiler natively on VeridianOS using a cross-compiled
//! rustc (from Phase 6.5) as the seed compiler. Manages the multi-stage
//! build process and config.toml generation.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::error::KernelError;

/// Bootstrap stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapStage {
    /// Using cross-compiled seed compiler
    Stage0,
    /// First native build (compiled by seed)
    Stage1,
    /// Self-hosted build (compiled by Stage 1)
    Stage2,
}

impl BootstrapStage {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Stage0 => Some(Self::Stage1),
            Self::Stage1 => Some(Self::Stage2),
            Self::Stage2 => None,
        }
    }
}

/// Rustc bootstrap configuration
#[derive(Debug, Clone)]
pub struct RustcBootstrapConfig {
    pub rustc_version: String,
    pub target_triple: String,
    pub seed_rustc_path: String,
    pub seed_cargo_path: String,
    pub llvm_root: String,
    pub install_prefix: String,
    pub enable_docs: bool,
    pub enable_extended: bool,
    pub current_stage: BootstrapStage,
}

impl Default for RustcBootstrapConfig {
    fn default() -> Self {
        Self {
            rustc_version: "1.93.1".to_string(),
            target_triple: "x86_64-unknown-veridian".to_string(),
            seed_rustc_path: "/usr/local/bin/rustc".to_string(),
            seed_cargo_path: "/usr/local/bin/cargo".to_string(),
            llvm_root: "/usr/local".to_string(),
            install_prefix: "/usr/local".to_string(),
            enable_docs: false,
            enable_extended: true,
            current_stage: BootstrapStage::Stage0,
        }
    }
}

impl RustcBootstrapConfig {
    /// Generate config.toml for x.py
    pub fn generate_config_toml(&self) -> String {
        let mut config = String::new();

        config.push_str("[build]\n");
        config.push_str(&alloc::format!("host = [\"{}\"]\n", self.target_triple));
        config.push_str(&alloc::format!("target = [\"{}\"]\n", self.target_triple));
        config.push_str(&alloc::format!("cargo = \"{}\"\n", self.seed_cargo_path));
        config.push_str(&alloc::format!("rustc = \"{}\"\n", self.seed_rustc_path));
        config.push_str("docs = false\n");
        config.push_str("extended = true\n");
        config.push_str("tools = [\"cargo\"]\n");
        config.push_str("vendor = true\n");
        config.push('\n');

        config.push_str("[llvm]\n");
        config.push_str("link-shared = false\n");
        config.push_str("targets = \"X86;AArch64;RISCV\"\n");
        config.push('\n');

        config.push_str("[rust]\n");
        config.push_str("codegen-units = 1\n");
        config.push_str("lto = \"thin\"\n");
        config.push_str("debug = false\n");
        config.push_str("optimize = true\n");
        config.push_str("channel = \"nightly\"\n");
        config.push('\n');

        config.push_str(&alloc::format!("[target.{}]\n", self.target_triple));
        config.push_str(&alloc::format!(
            "llvm-config = \"{}/bin/llvm-config\"\n",
            self.llvm_root
        ));
        config.push('\n');

        config.push_str("[install]\n");
        config.push_str(&alloc::format!("prefix = \"{}\"\n", self.install_prefix));

        config
    }

    /// Generate x.py build command for a given stage
    pub fn build_command(&self, stage: BootstrapStage) -> String {
        let stage_num = match stage {
            BootstrapStage::Stage0 => 0,
            BootstrapStage::Stage1 => 1,
            BootstrapStage::Stage2 => 2,
        };
        alloc::format!("python3 x.py build --stage {}", stage_num)
    }

    /// Generate x.py install command
    pub fn install_command(&self) -> String {
        "python3 x.py install".to_string()
    }

    /// Advance to next stage
    pub fn advance_stage(&mut self) -> Result<(), KernelError> {
        match self.current_stage.next() {
            Some(next) => {
                self.current_stage = next;
                Ok(())
            }
            None => Err(KernelError::InvalidArgument {
                name: "stage",
                value: "already at final stage",
            }),
        }
    }

    /// Verify that a rustc binary at the given path produces expected output
    pub fn verify_rustc(&self, version_output: &str) -> bool {
        version_output.contains("rustc") && version_output.contains(&self.rustc_version)
    }

    /// Check if Stage 2 output matches Stage 1 (reproducibility)
    pub fn check_reproducibility(&self, stage1_hash: &[u8; 32], stage2_hash: &[u8; 32]) -> bool {
        stage1_hash == stage2_hash
    }
}

/// Rustdoc generation configuration
#[derive(Debug, Clone)]
pub struct RustdocConfig {
    pub crate_name: String,
    pub source_dir: String,
    pub output_dir: String,
    pub features: Vec<String>,
}

impl RustdocConfig {
    pub fn new(crate_name: &str, source_dir: &str) -> Self {
        Self {
            crate_name: crate_name.to_string(),
            source_dir: source_dir.to_string(),
            output_dir: alloc::format!("{}/target/doc", source_dir),
            features: Vec::new(),
        }
    }

    pub fn build_command(&self) -> String {
        let mut cmd = String::from("cargo doc --no-deps");
        if !self.features.is_empty() {
            cmd.push_str(&alloc::format!(" --features {}", self.features.join(",")));
        }
        cmd
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootstrap_stage_next() {
        assert_eq!(BootstrapStage::Stage0.next(), Some(BootstrapStage::Stage1));
        assert_eq!(BootstrapStage::Stage1.next(), Some(BootstrapStage::Stage2));
        assert_eq!(BootstrapStage::Stage2.next(), None);
    }

    #[test]
    fn test_default_config() {
        let config = RustcBootstrapConfig::default();
        assert_eq!(config.current_stage, BootstrapStage::Stage0);
        assert!(config.target_triple.contains("veridian"));
    }

    #[test]
    fn test_generate_config_toml() {
        let config = RustcBootstrapConfig::default();
        let toml = config.generate_config_toml();
        assert!(toml.contains("[build]"));
        assert!(toml.contains("[llvm]"));
        assert!(toml.contains("[rust]"));
        assert!(toml.contains("[install]"));
        assert!(toml.contains("veridian"));
    }

    #[test]
    fn test_build_command() {
        let config = RustcBootstrapConfig::default();
        let cmd = config.build_command(BootstrapStage::Stage1);
        assert!(cmd.contains("--stage 1"));
    }

    #[test]
    fn test_advance_stage() {
        let mut config = RustcBootstrapConfig::default();
        assert_eq!(config.current_stage, BootstrapStage::Stage0);
        config.advance_stage().unwrap();
        assert_eq!(config.current_stage, BootstrapStage::Stage1);
        config.advance_stage().unwrap();
        assert_eq!(config.current_stage, BootstrapStage::Stage2);
        assert!(config.advance_stage().is_err());
    }

    #[test]
    fn test_verify_rustc() {
        let config = RustcBootstrapConfig::default();
        assert!(config.verify_rustc("rustc 1.93.1 (abcdef 2026-01-01)"));
        assert!(!config.verify_rustc("gcc 14.2.0"));
    }

    #[test]
    fn test_reproducibility() {
        let config = RustcBootstrapConfig::default();
        let hash1 = [0xAB; 32];
        let hash2 = [0xAB; 32];
        let hash3 = [0xCD; 32];
        assert!(config.check_reproducibility(&hash1, &hash2));
        assert!(!config.check_reproducibility(&hash1, &hash3));
    }

    #[test]
    fn test_rustdoc_config() {
        let config = RustdocConfig::new("mylib", "/src/mylib");
        assert_eq!(config.crate_name, "mylib");
        assert!(config.output_dir.contains("target/doc"));
    }

    #[test]
    fn test_rustdoc_command() {
        let config = RustdocConfig::new("mylib", "/src/mylib");
        let cmd = config.build_command();
        assert!(cmd.contains("cargo doc"));
    }

    #[test]
    fn test_rustdoc_features() {
        let mut config = RustdocConfig::new("mylib", "/src/mylib");
        config.features.push("alloc".to_string());
        let cmd = config.build_command();
        assert!(cmd.contains("--features alloc"));
    }

    #[test]
    fn test_install_command() {
        let config = RustcBootstrapConfig::default();
        let cmd = config.install_command();
        assert!(cmd.contains("x.py install"));
    }
}
