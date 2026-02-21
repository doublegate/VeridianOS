//! Toolchain Management for VeridianOS SDK
//!
//! Provides toolchain registration, cross-compilation configuration, and linker
//! setup for building user-space packages targeting VeridianOS. Supports
//! x86_64, AArch64, and RISC-V architectures with appropriate defaults for
//! each target.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space SDK forward declarations -- see module doc TODO(user-space)
#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::error::{KernelError, KernelResult};

// ============================================================================
// VeridianTarget - compile-time target definitions
// ============================================================================

/// Describes a supported VeridianOS build target at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VeridianTarget {
    /// Target triple (e.g. "x86_64-veridian").
    pub triple: &'static str,
    /// Architecture short name (e.g. "x86_64").
    pub arch: &'static str,
    /// Architecture-specific compiler feature flags.
    pub features: &'static str,
}

impl VeridianTarget {
    /// x86_64 target with soft-float and no red zone for kernel safety.
    pub const X86_64: VeridianTarget = VeridianTarget {
        triple: "x86_64-veridian",
        arch: "x86_64",
        features: "-mmx,-sse,-sse2,-sse3,-ssse3,-sse4.1,-sse4.2,-avx,-avx2,+soft-float",
    };

    /// AArch64 target using the Cortex-A57 baseline.
    pub const AARCH64: VeridianTarget = VeridianTarget {
        triple: "aarch64-veridian",
        arch: "aarch64",
        features: "+strict-align",
    };

    /// RISC-V 64-bit target with GC extensions.
    pub const RISCV64: VeridianTarget = VeridianTarget {
        triple: "riscv64gc-veridian",
        arch: "riscv64",
        features: "+m,+a,+f,+d,+c",
    };

    /// Look up a target definition by its triple string.
    pub fn from_triple(triple: &str) -> Option<VeridianTarget> {
        match triple {
            "x86_64-veridian" => Some(Self::X86_64),
            "aarch64-veridian" => Some(Self::AARCH64),
            "riscv64gc-veridian" => Some(Self::RISCV64),
            _ => None,
        }
    }

    /// Return the target definition for the current compile-time architecture.
    pub fn current() -> VeridianTarget {
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::AARCH64
        }
        #[cfg(target_arch = "riscv64")]
        {
            Self::RISCV64
        }
    }
}

// ============================================================================
// ToolchainComponent
// ============================================================================

/// Identifies a single component within a toolchain installation.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainComponent {
    /// A compiler for the given programming language (e.g. "c", "c++", "rust").
    Compiler { language: String },
    /// A system linker.
    Linker,
    /// An assembler.
    Assembler,
    /// A debugger (e.g. GDB, LLDB).
    Debugger,
    /// A profiling tool.
    Profiler,
}

// ============================================================================
// Toolchain
// ============================================================================

/// A registered toolchain containing one or more components.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct Toolchain {
    /// Human-readable name (e.g. "veridian-gcc-13").
    pub name: String,
    /// Version string (e.g. "13.2.0").
    pub version: String,
    /// Target triple this toolchain produces code for.
    pub target_triple: String,
    /// Filesystem path to the toolchain binary directory.
    pub bin_path: String,
    /// Filesystem path to the sysroot containing headers and libraries.
    pub sysroot_path: String,
    /// Components available in this toolchain.
    pub components: Vec<ToolchainComponent>,
}

#[cfg(feature = "alloc")]
impl Toolchain {
    /// Create a new toolchain with the given identity and paths.
    pub fn new(
        name: &str,
        version: &str,
        target_triple: &str,
        bin_path: &str,
        sysroot_path: &str,
    ) -> Self {
        Self {
            name: String::from(name),
            version: String::from(version),
            target_triple: String::from(target_triple),
            bin_path: String::from(bin_path),
            sysroot_path: String::from(sysroot_path),
            components: Vec::new(),
        }
    }

    /// Add a component to this toolchain.
    pub fn add_component(&mut self, component: ToolchainComponent) {
        if !self.components.contains(&component) {
            self.components.push(component);
        }
    }

    /// Check whether a specific component type is present.
    pub fn has_component(&self, component: &ToolchainComponent) -> bool {
        self.components.contains(component)
    }
}

// ============================================================================
// ToolchainRegistry
// ============================================================================

/// Registry of known toolchains, keyed by name.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ToolchainRegistry {
    /// All registered toolchains.
    toolchains: BTreeMap<String, Toolchain>,
    /// Name of the default toolchain, if set.
    default_name: Option<String>,
}

#[cfg(feature = "alloc")]
impl ToolchainRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            toolchains: BTreeMap::new(),
            default_name: None,
        }
    }

    /// Register a toolchain. Returns an error if a toolchain with the same
    /// name is already registered.
    pub fn register(&mut self, toolchain: Toolchain) -> KernelResult<()> {
        if self.toolchains.contains_key(&toolchain.name) {
            return Err(KernelError::AlreadyExists {
                resource: "toolchain",
                id: 0,
            });
        }
        self.toolchains.insert(toolchain.name.clone(), toolchain);
        Ok(())
    }

    /// Look up a toolchain by name.
    pub fn get(&self, name: &str) -> Option<&Toolchain> {
        self.toolchains.get(name)
    }

    /// Return the names of all registered toolchains.
    pub fn list(&self) -> Vec<&str> {
        self.toolchains.keys().map(|k| k.as_str()).collect()
    }

    /// Remove a toolchain by name. Returns an error if it does not exist.
    pub fn remove(&mut self, name: &str) -> KernelResult<Toolchain> {
        // Clear default if it points to the removed toolchain.
        if self.default_name.as_deref() == Some(name) {
            self.default_name = None;
        }
        self.toolchains.remove(name).ok_or(KernelError::NotFound {
            resource: "toolchain",
            id: 0,
        })
    }

    /// Set the default toolchain by name. The toolchain must already be
    /// registered.
    pub fn set_default(&mut self, name: &str) -> KernelResult<()> {
        if !self.toolchains.contains_key(name) {
            return Err(KernelError::NotFound {
                resource: "toolchain",
                id: 0,
            });
        }
        self.default_name = Some(String::from(name));
        Ok(())
    }

    /// Return a reference to the default toolchain, if one has been set.
    pub fn get_default(&self) -> Option<&Toolchain> {
        self.default_name
            .as_deref()
            .and_then(|name| self.toolchains.get(name))
    }
}

#[cfg(feature = "alloc")]
impl Default for ToolchainRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// CrossCompilerConfig
// ============================================================================

/// Cross-compilation tool paths for a specific target.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CrossCompilerConfig {
    /// Path to the C compiler.
    pub cc: String,
    /// Path to the C++ compiler.
    pub cxx: String,
    /// Path to the archiver.
    pub ar: String,
    /// Path to the linker.
    pub ld: String,
    /// Path to the ranlib tool.
    pub ranlib: String,
    /// Path to the strip tool.
    pub strip: String,
}

#[cfg(feature = "alloc")]
impl CrossCompilerConfig {
    /// Produce a cross-compiler configuration with sensible defaults for the
    /// given target triple. Uses GNU-style tool naming conventions.
    pub fn for_target(target: &str) -> Self {
        let prefix = match target {
            "x86_64-veridian" | "x86_64-unknown-none" => "x86_64-veridian",
            "aarch64-veridian" | "aarch64-unknown-none" => "aarch64-veridian",
            "riscv64gc-veridian" | "riscv64gc-unknown-none-elf" => "riscv64gc-veridian",
            other => other,
        };
        let sysroot = super::get_sysroot();

        Self {
            cc: format!("{}/bin/{}-gcc", sysroot, prefix),
            cxx: format!("{}/bin/{}-g++", sysroot, prefix),
            ar: format!("{}/bin/{}-ar", sysroot, prefix),
            ld: format!("{}/bin/{}-ld", sysroot, prefix),
            ranlib: format!("{}/bin/{}-ranlib", sysroot, prefix),
            strip: format!("{}/bin/{}-strip", sysroot, prefix),
        }
    }

    /// Convert the configuration into environment variable key-value pairs
    /// suitable for passing to a build system.
    pub fn to_env_vars(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert(String::from("CC"), self.cc.clone());
        env.insert(String::from("CXX"), self.cxx.clone());
        env.insert(String::from("AR"), self.ar.clone());
        env.insert(String::from("LD"), self.ld.clone());
        env.insert(String::from("RANLIB"), self.ranlib.clone());
        env.insert(String::from("STRIP"), self.strip.clone());
        env
    }
}

/// Generate a complete set of cross-compilation environment variables for the
/// given target, including compiler paths, flags, and pkg-config hints.
#[cfg(feature = "alloc")]
pub fn generate_cross_env(target: &str) -> BTreeMap<String, String> {
    let cross = CrossCompilerConfig::for_target(target);
    let mut env = cross.to_env_vars();

    let sysroot = super::get_sysroot();
    let target_info = VeridianTarget::from_triple(target);

    // Common compiler flags
    env.insert(
        String::from("CFLAGS"),
        format!("--sysroot={} -ffreestanding -nostdlib", sysroot),
    );
    env.insert(
        String::from("CXXFLAGS"),
        format!("--sysroot={} -ffreestanding -nostdlib", sysroot),
    );
    env.insert(
        String::from("LDFLAGS"),
        format!("-L{}/lib/{} -nostdlib", sysroot, target),
    );

    // pkg-config integration
    env.insert(
        String::from("PKG_CONFIG_SYSROOT_DIR"),
        String::from(sysroot),
    );
    env.insert(
        String::from("PKG_CONFIG_PATH"),
        format!("{}/lib/{}/pkgconfig", sysroot, target),
    );

    // Architecture-specific features
    if let Some(info) = target_info {
        env.insert(String::from("VERIDIAN_ARCH"), String::from(info.arch));
        env.insert(String::from("VERIDIAN_TARGET"), String::from(info.triple));
        env.insert(
            String::from("VERIDIAN_FEATURES"),
            String::from(info.features),
        );
    }

    env
}

// ============================================================================
// LinkerConfig
// ============================================================================

/// Linker configuration for a specific target architecture.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct LinkerConfig {
    /// Path to the linker binary.
    pub linker_path: String,
    /// Optional path to a custom linker script.
    pub linker_script: Option<String>,
    /// Library search paths passed via `-L`.
    pub search_paths: Vec<String>,
    /// Additional linker flags.
    pub flags: Vec<String>,
}

#[cfg(feature = "alloc")]
impl LinkerConfig {
    /// Produce a linker configuration with sensible defaults for the given
    /// target triple.
    pub fn for_target(target: &str) -> Self {
        let sysroot = super::get_sysroot();
        let cross = CrossCompilerConfig::for_target(target);
        let lib_dir = format!("{}/lib/{}", sysroot, target);

        let mut flags = vec![String::from("--gc-sections")];

        // Architecture-specific linker flags
        match target {
            "x86_64-veridian" | "x86_64-unknown-none" => {
                flags.push(String::from("-z"));
                flags.push(String::from("max-page-size=4096"));
            }
            "aarch64-veridian" | "aarch64-unknown-none" => {
                flags.push(String::from("-z"));
                flags.push(String::from("max-page-size=65536"));
            }
            "riscv64gc-veridian" | "riscv64gc-unknown-none-elf" => {
                flags.push(String::from("-z"));
                flags.push(String::from("max-page-size=4096"));
            }
            _ => {}
        }

        Self {
            linker_path: cross.ld,
            linker_script: None,
            search_paths: vec![lib_dir],
            flags,
        }
    }
}

// ============================================================================
// Linker Script Generation
// ============================================================================

/// Generate a basic VeridianOS linker script for the given target architecture.
///
/// The script defines entry points and section layout appropriate for each
/// supported architecture:
/// - x86_64: kernel mapped at `0xFFFFFFFF80100000` (higher-half)
/// - aarch64: loaded at `0x40080000` (QEMU virt)
/// - riscv64: loaded at `0x80200000` (OpenSBI payload)
#[cfg(feature = "alloc")]
pub fn generate_linker_script(target: &str) -> String {
    let (entry, origin) = match target {
        "x86_64-veridian" | "x86_64-unknown-none" => ("_start", "0xFFFFFFFF80100000"),
        "aarch64-veridian" | "aarch64-unknown-none" => ("_start", "0x40080000"),
        "riscv64gc-veridian" | "riscv64gc-unknown-none-elf" => ("_start", "0x80200000"),
        _ => ("_start", "0x100000"),
    };

    let mut s = String::new();
    s.push_str("/* VeridianOS linker script - auto-generated */\n");
    s.push_str(&format!("ENTRY({})\n\n", entry));
    s.push_str("SECTIONS\n{\n");
    s.push_str(&format!("    . = {};\n\n", origin));

    // .text section
    s.push_str("    .text : ALIGN(4096)\n    {\n");
    s.push_str("        _text_start = .;\n");
    s.push_str("        *(.text.boot)\n");
    s.push_str("        *(.text .text.*)\n");
    s.push_str("        _text_end = .;\n");
    s.push_str("    }\n\n");

    // .rodata section
    s.push_str("    .rodata : ALIGN(4096)\n    {\n");
    s.push_str("        _rodata_start = .;\n");
    s.push_str("        *(.rodata .rodata.*)\n");
    s.push_str("        _rodata_end = .;\n");
    s.push_str("    }\n\n");

    // .data section
    s.push_str("    .data : ALIGN(4096)\n    {\n");
    s.push_str("        _data_start = .;\n");
    s.push_str("        *(.data .data.*)\n");
    s.push_str("        _data_end = .;\n");
    s.push_str("    }\n\n");

    // .bss section
    s.push_str("    .bss : ALIGN(4096)\n    {\n");
    s.push_str("        _bss_start = .;\n");
    s.push_str("        *(.bss .bss.*)\n");
    s.push_str("        *(COMMON)\n");
    s.push_str("        _bss_end = .;\n");
    s.push_str("    }\n\n");

    s.push_str("    _kernel_end = .;\n\n");
    s.push_str("    /DISCARD/ :\n    {\n");
    s.push_str("        *(.comment)\n");
    s.push_str("        *(.note.*)\n");
    s.push_str("        *(.eh_frame*)\n");
    s.push_str("    }\n");
    s.push_str("}\n");

    s
}

// ============================================================================
// CMake Toolchain File Generation
// ============================================================================

/// Generates a CMake toolchain file for cross-compiling to VeridianOS.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CMakeToolchainFile {
    /// Target triple to generate the toolchain file for.
    pub target: String,
}

#[cfg(feature = "alloc")]
impl CMakeToolchainFile {
    /// Create a new generator for the given target triple.
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_string(),
        }
    }

    /// Generate the contents of a CMake toolchain file.
    ///
    /// The output sets `CMAKE_SYSTEM_NAME`, `CMAKE_SYSTEM_PROCESSOR`,
    /// `CMAKE_C_COMPILER`, `CMAKE_CXX_COMPILER`, `CMAKE_FIND_ROOT_PATH`,
    /// and related variables so that CMake can cross-compile for VeridianOS.
    pub fn generate(&self) -> String {
        let cross = CrossCompilerConfig::for_target(&self.target);
        let sysroot = super::get_sysroot();

        let processor = match self.target.as_str() {
            "x86_64-veridian" | "x86_64-unknown-none" => "x86_64",
            "aarch64-veridian" | "aarch64-unknown-none" => "aarch64",
            "riscv64gc-veridian" | "riscv64gc-unknown-none-elf" => "riscv64",
            _ => "unknown",
        };

        let mut s = String::new();
        s.push_str("# VeridianOS CMake toolchain file - auto-generated\n\n");
        s.push_str("set(CMAKE_SYSTEM_NAME VeridianOS)\n");
        s.push_str(&format!("set(CMAKE_SYSTEM_PROCESSOR {})\n\n", processor));

        s.push_str(&format!("set(CMAKE_C_COMPILER {})\n", cross.cc));
        s.push_str(&format!("set(CMAKE_CXX_COMPILER {})\n", cross.cxx));
        s.push_str(&format!("set(CMAKE_AR {})\n", cross.ar));
        s.push_str(&format!("set(CMAKE_RANLIB {})\n", cross.ranlib));
        s.push_str(&format!("set(CMAKE_STRIP {})\n", cross.strip));
        s.push_str(&format!("set(CMAKE_LINKER {})\n\n", cross.ld));

        s.push_str(&format!("set(CMAKE_SYSROOT {})\n", sysroot));
        s.push_str(&format!("set(CMAKE_FIND_ROOT_PATH {})\n\n", sysroot));

        s.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)\n");
        s.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)\n");
        s.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)\n");
        s.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)\n\n");

        s.push_str("set(CMAKE_C_FLAGS_INIT \"-ffreestanding -nostdlib\")\n");
        s.push_str("set(CMAKE_CXX_FLAGS_INIT \"-ffreestanding -nostdlib\")\n");

        s
    }
}
