//! VeridianOS target specification for the Rust compiler.
//!
//! This module contains:
//!
//! 1. Compile-time constants describing the target platform (analogous to
//!    `std::env::consts`)
//! 2. Complete target specification JSON for all three architectures, usable
//!    with `rustc --target <file>.json`
//! 3. Platform capability constants (limits, sizes, features)
//! 4. Target registration documentation for adding VeridianOS as a built-in
//!    target in the Rust compiler
//!
//! # Registering the Target
//!
//! There are two approaches for using a custom target with `rustc`:
//!
//! ## Approach 1: Target JSON file (development)
//!
//! Place the target JSON file in the project and pass it to `rustc`:
//!
//! ```text
//! $ rustc --target path/to/x86_64-unknown-veridian.json \
//!     -Z build-std=core,alloc main.rs
//! ```
//!
//! Or in `.cargo/config.toml`:
//!
//! ```toml
//! [build]
//! target = "targets/x86_64-unknown-veridian.json"
//!
//! [unstable]
//! build-std = ["core", "compiler_builtins", "alloc"]
//! ```
//!
//! ## Approach 2: Built-in target (upstream)
//!
//! To add VeridianOS as a built-in target in the Rust compiler:
//!
//! 1. Add `veridian` to `rustc_target::spec::OperatingSystem` enum
//! 2. Create `rustc_target/src/spec/targets/x86_64_unknown_veridian.rs`
//!    implementing `Target::target()` with the specification below
//! 3. Register the target in `rustc_target/src/spec/mod.rs` in the
//!    `supported_targets!` macro
//! 4. Add VeridianOS support to `library/std/src/sys/` with a new platform
//!    module (this crate serves as the reference implementation)
//! 5. Update `compiler/rustc_target/src/spec/base/` with a VeridianOS base spec
//!    if multiple architectures share common settings

// ============================================================================
// OS identification
// ============================================================================

/// The operating system name: `"veridian"`.
pub const OS: &str = "veridian";

/// The operating system family: `"unix"`.
///
/// VeridianOS follows Unix/POSIX conventions for its user-space ABI.
pub const FAMILY: &str = "unix";

// ============================================================================
// Architecture
// ============================================================================

/// The CPU architecture.
#[cfg(target_arch = "x86_64")]
pub const ARCH: &str = "x86_64";

/// The CPU architecture.
#[cfg(target_arch = "aarch64")]
pub const ARCH: &str = "aarch64";

/// The CPU architecture.
#[cfg(target_arch = "riscv64")]
pub const ARCH: &str = "riscv64";

/// Pointer width in bits.
pub const PTR_WIDTH: usize = core::mem::size_of::<usize>() * 8;

// ============================================================================
// Binary format
// ============================================================================

/// The executable file suffix (empty on Unix-like systems).
pub const EXE_SUFFIX: &str = "";

/// The shared library prefix.
pub const DLL_PREFIX: &str = "lib";

/// The shared library suffix.
pub const DLL_SUFFIX: &str = ".so";

/// The static library prefix.
pub const STATICLIB_PREFIX: &str = "lib";

/// The static library suffix.
pub const STATICLIB_SUFFIX: &str = ".a";

// ============================================================================
// Endianness
// ============================================================================

/// Target endianness.
#[cfg(target_endian = "little")]
pub const ENDIAN: &str = "little";

/// Target endianness.
#[cfg(target_endian = "big")]
pub const ENDIAN: &str = "big";

// ============================================================================
// Target triple constants
// ============================================================================

/// Target triple for x86_64.
pub const TARGET_TRIPLE_X86_64: &str = "x86_64-unknown-veridian";

/// Target triple for AArch64.
pub const TARGET_TRIPLE_AARCH64: &str = "aarch64-unknown-veridian";

/// Target triple for RISC-V 64.
pub const TARGET_TRIPLE_RISCV64: &str = "riscv64gc-unknown-veridian";

/// The vendor field (unknown).
pub const VENDOR: &str = "unknown";

// ============================================================================
// x86_64 kernel target specification
// ============================================================================

/// Complete target specification JSON for `x86_64-unknown-veridian` (kernel).
///
/// This can be written to a `.json` file and passed to `rustc` via
/// `--target`.  It is also the reference for implementing the target
/// in `rustc_target::spec`.
///
/// # Key Settings
///
/// | Field | Value | Rationale |
/// |-------|-------|-----------|
/// | `arch` | `"x86_64"` | Primary architecture |
/// | `os` | `"veridian"` | VeridianOS identifier |
/// | `panic-strategy` | `"abort"` | No unwinding in kernel/user |
/// | `disable-redzone` | `true` | Required for interrupt safety |
/// | `code-model` | `"kernel"` | Kernel linked at 0xFFFF... |
/// | `relocation-model` | `"static"` | No dynamic linker for kernel |
/// | `features` | `"-mmx,-sse,-sse2,+soft-float"` | No FPU in kernel mode |
/// | `linker-flavor` | `"ld.lld"` | LLVM linker for cross-compilation |
/// | `exe-suffix` | `""` | No .exe extension |
/// | `has-thread-local` | `true` | TLS via `fs` segment |
/// | `position-independent-executables` | `false` | Static kernel binary |
pub const X86_64_KERNEL_TARGET_SPEC: &str = r#"{
    "llvm-target": "x86_64-unknown-none",
    "arch": "x86_64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "data-layout": "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
    "os": "veridian",
    "vendor": "unknown",
    "env": "",
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "disable-redzone": true,
    "code-model": "kernel",
    "relocation-model": "static",
    "features": "-mmx,-sse,-sse2,+soft-float",
    "executables": true,
    "exe-suffix": "",
    "has-thread-local": true,
    "pre-link-args": {
        "ld.lld": [
            "--gc-sections",
            "-z", "max-page-size=4096"
        ]
    },
    "position-independent-executables": false,
    "static-position-independent-executables": false,
    "needs-plt": false,
    "frame-pointer": "always",
    "supported-sanitizers": [],
    "is-like-none": true,
    "max-atomic-width": 64,
    "emit-debug-gdb-scripts": false,
    "dynamic-linking": false
}"#;

// ============================================================================
// x86_64 user-space target specification
// ============================================================================

/// Target specification for x86_64 user-space programs on VeridianOS.
///
/// This variant differs from the kernel target:
/// - Uses `"small"` code model (not kernel)
/// - Enables SSE/SSE2 for user-space floating point
/// - Red zone is allowed (no interrupts in user code)
/// - Frame pointer can be omitted for optimization
pub const X86_64_USER_TARGET_SPEC: &str = r#"{
    "llvm-target": "x86_64-unknown-none",
    "arch": "x86_64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "data-layout": "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128",
    "os": "veridian",
    "vendor": "unknown",
    "env": "",
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "disable-redzone": false,
    "code-model": "small",
    "relocation-model": "static",
    "features": "+sse,+sse2",
    "executables": true,
    "exe-suffix": "",
    "has-thread-local": true,
    "pre-link-args": {
        "ld.lld": [
            "--gc-sections",
            "-z", "max-page-size=4096"
        ]
    },
    "position-independent-executables": false,
    "static-position-independent-executables": false,
    "needs-plt": false,
    "frame-pointer": "may-omit",
    "supported-sanitizers": [],
    "is-like-none": true,
    "max-atomic-width": 64,
    "emit-debug-gdb-scripts": false,
    "dynamic-linking": false
}"#;

// ============================================================================
// AArch64 target specification
// ============================================================================

/// Complete target specification JSON for `aarch64-unknown-veridian`.
///
/// # Key Differences from x86_64
///
/// - Uses `aarch64-unknown-none` as the LLVM target
/// - No red zone concept on AArch64
/// - NEON/FP is standard on AArch64 (no soft-float needed)
/// - Standard code model (no kernel code model on AArch64)
/// - Supports 128-bit atomics (via LDXP/STXP or LSE)
pub const AARCH64_TARGET_SPEC: &str = r#"{
    "llvm-target": "aarch64-unknown-none",
    "arch": "aarch64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "data-layout": "e-m:e-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128-Fn32",
    "os": "veridian",
    "vendor": "unknown",
    "env": "",
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "disable-redzone": true,
    "code-model": "small",
    "relocation-model": "static",
    "features": "+v8a,+strict-align",
    "executables": true,
    "exe-suffix": "",
    "has-thread-local": true,
    "pre-link-args": {
        "ld.lld": [
            "--gc-sections",
            "-z", "max-page-size=4096"
        ]
    },
    "position-independent-executables": false,
    "static-position-independent-executables": false,
    "needs-plt": false,
    "frame-pointer": "always",
    "supported-sanitizers": [],
    "is-like-none": true,
    "max-atomic-width": 128,
    "emit-debug-gdb-scripts": false,
    "dynamic-linking": false
}"#;

// ============================================================================
// RISC-V 64 target specification
// ============================================================================

/// Complete target specification JSON for `riscv64gc-unknown-veridian`.
///
/// # Key Differences
///
/// - Uses `riscv64-unknown-none-elf` as the LLVM target
/// - Supports 64-bit atomics (max-atomic-width = 64)
/// - Uses RV64GC ISA: M (multiply), A (atomics), F (float), D (double), C
///   (compressed)
/// - Code model "medium" for larger programs
pub const RISCV64_TARGET_SPEC: &str = r#"{
    "llvm-target": "riscv64-unknown-none-elf",
    "arch": "riscv64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "data-layout": "e-m:e-p:64:64-i64:64-i128:128-n32:64-S128",
    "os": "veridian",
    "vendor": "unknown",
    "env": "",
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "disable-redzone": true,
    "code-model": "medium",
    "relocation-model": "static",
    "features": "+m,+a,+f,+d,+c",
    "executables": true,
    "exe-suffix": "",
    "has-thread-local": true,
    "pre-link-args": {
        "ld.lld": [
            "--gc-sections",
            "-z", "max-page-size=4096"
        ]
    },
    "position-independent-executables": false,
    "static-position-independent-executables": false,
    "needs-plt": false,
    "frame-pointer": "always",
    "supported-sanitizers": [],
    "is-like-none": true,
    "max-atomic-width": 64,
    "emit-debug-gdb-scripts": false,
    "dynamic-linking": false,
    "atomic-cas": true
}"#;

// ============================================================================
// Platform capability constants
// ============================================================================

/// Maximum path length (POSIX PATH_MAX equivalent).
pub const PATH_MAX: usize = 4096;

/// Maximum file name component length.
pub const NAME_MAX: usize = 255;

/// Page size in bytes.
///
/// All three supported architectures use 4 KiB base pages.
pub const PAGE_SIZE: usize = 4096;

/// Maximum number of open file descriptors per process.
pub const OPEN_MAX: usize = 256;

/// Maximum argument length for exec (POSIX ARG_MAX).
pub const ARG_MAX: usize = 131072; // 128 KiB

/// Maximum number of arguments to `execve`.
pub const MAX_ARGS: usize = 32768;

/// Maximum number of environment variables.
pub const ENV_MAX: usize = 32768;

/// Clock ticks per second (POSIX CLK_TCK / _SC_CLK_TCK).
pub const CLK_TCK: usize = 100;

/// Minimum supported stack size for threads (64 KiB).
pub const MIN_STACK_SIZE: usize = 64 * 1024;

/// Default stack size for new threads (2 MiB).
pub const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;

/// Maximum number of processes per user.
pub const CHILD_MAX: usize = 1024;

/// Maximum number of supplementary group IDs.
pub const NGROUPS_MAX: usize = 65536;

/// Maximum number of symbolic link traversals in path resolution.
pub const SYMLOOP_MAX: usize = 40;

/// Maximum length of a hostname.
pub const HOST_NAME_MAX: usize = 64;

/// Maximum length of a login name.
pub const LOGIN_NAME_MAX: usize = 256;

/// Maximum length of a terminal device name.
pub const TTY_NAME_MAX: usize = 32;

/// Pipe buffer capacity in bytes.
pub const PIPE_BUF: usize = 4096;

// ============================================================================
// Runtime target information
// ============================================================================

/// Runtime information about the current target.
#[derive(Debug, Clone, Copy)]
pub struct TargetInfo {
    /// Target triple string.
    pub triple: &'static str,
    /// Architecture name.
    pub arch: &'static str,
    /// Pointer width in bits.
    pub pointer_width: u32,
    /// Page size in bytes.
    pub page_size: usize,
    /// Maximum supported atomic width in bits.
    pub max_atomic_width: u32,
    /// Whether thread-local storage is available.
    pub has_tls: bool,
    /// Endianness.
    pub endian: Endian,
}

/// Byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    /// Little-endian byte order.
    Little,
    /// Big-endian byte order.
    Big,
}

/// Returns information about the current compilation target.
///
/// This is resolved at compile time based on `cfg(target_arch)`.
pub const fn current_target() -> TargetInfo {
    #[cfg(target_arch = "x86_64")]
    {
        TargetInfo {
            triple: TARGET_TRIPLE_X86_64,
            arch: "x86_64",
            pointer_width: 64,
            page_size: 4096,
            max_atomic_width: 64,
            has_tls: true,
            endian: Endian::Little,
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        TargetInfo {
            triple: TARGET_TRIPLE_AARCH64,
            arch: "aarch64",
            pointer_width: 64,
            page_size: 4096,
            max_atomic_width: 128,
            has_tls: true,
            endian: Endian::Little,
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        TargetInfo {
            triple: TARGET_TRIPLE_RISCV64,
            arch: "riscv64",
            pointer_width: 64,
            page_size: 4096,
            max_atomic_width: 64,
            has_tls: true,
            endian: Endian::Little,
        }
    }
}

/// Get the system page size.
///
/// All VeridianOS architectures currently use 4 KiB pages.
/// This function exists for forward compatibility with 16K/64K pages
/// on AArch64.
#[inline]
pub const fn page_size() -> usize {
    current_target().page_size
}

/// Get the maximum supported atomic width in bits.
#[inline]
pub const fn max_atomic_width() -> u32 {
    current_target().max_atomic_width
}

// ============================================================================
// Feature detection
// ============================================================================

/// CPU feature flags that can be queried at compile time.
///
/// On x86_64 these correspond to CPUID features; on AArch64 to ID register
/// fields; on RISC-V to ISA extensions.
///
/// Currently compile-time only -- runtime detection requires either
/// privileged register access or kernel `AT_HWCAP` auxiliary vector support.
#[derive(Debug, Clone, Copy)]
pub struct CpuFeatures {
    /// SSE2 available (x86_64; always true for user-space targets).
    pub sse2: bool,
    /// AVX available.
    pub avx: bool,
    /// AVX2 available.
    pub avx2: bool,
    /// NEON/ASIMD available (AArch64; always true for v8+).
    pub neon: bool,
    /// RDRAND available (x86_64).
    pub rdrand: bool,
    /// Atomics extension (AArch64 LSE).
    pub atomics: bool,
    /// CRC32 instructions available.
    pub crc32: bool,
}

/// Compile-time CPU feature detection based on target configuration.
///
/// For user-space programs, SSE2 is always available on x86_64 and NEON
/// is always available on AArch64 v8+.
pub const fn compile_time_features() -> CpuFeatures {
    CpuFeatures {
        sse2: cfg!(target_arch = "x86_64"),
        avx: false, // Conservative; runtime detection needed
        avx2: false,
        neon: cfg!(target_arch = "aarch64"),
        rdrand: false, // Must check CPUID at runtime
        atomics: cfg!(target_arch = "aarch64"),
        crc32: false,
    }
}
