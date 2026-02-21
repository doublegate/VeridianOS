//! Package Ecosystem Definitions
//!
//! Defines the VeridianOS package ecosystem: base system packages, essential
//! applications, and architecture-specific driver packages. These are
//! specifications describing what the ecosystem WILL contain, not compiled
//! software.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space API forward declarations -- see NOTE above
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A named set of related packages (e.g. "base-system", "dev-tools").
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PackageSet {
    /// Set name (e.g. "base-system")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Packages belonging to this set
    pub packages: Vec<PackageDefinition>,
}

/// Definition of a single package within a set.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PackageDefinition {
    /// Package name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Functional category
    pub category: PackageCategory,
    /// Whether this package is essential for a minimal installation
    pub essential: bool,
}

/// Functional categories for packages in the ecosystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageCategory {
    /// Core base system packages (kernel, init, shell)
    Base,
    /// Core user-space utilities (ls, cat, cp, etc.)
    CoreUtils,
    /// Development tools (compilers, debuggers, build systems)
    DevTools,
    /// System libraries (libc, runtime support)
    SystemLibs,
    /// Text editors
    TextEditor,
    /// File management tools
    FileManager,
    /// Networking utilities
    NetworkTools,
    /// System monitoring tools
    SystemMonitor,
    /// Graphics/display drivers
    GraphicsDriver,
    /// Network interface drivers
    NetworkDriver,
    /// Storage device drivers
    StorageDriver,
    /// Input device drivers
    InputDriver,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a `PackageDefinition` with the given fields.
#[cfg(feature = "alloc")]
fn pkg(
    name: &str,
    description: &str,
    category: PackageCategory,
    essential: bool,
) -> PackageDefinition {
    PackageDefinition {
        name: String::from(name),
        description: String::from(description),
        category,
        essential,
    }
}

/// Create a `PackageSet` with the given fields.
#[cfg(feature = "alloc")]
fn pkgset(name: &str, description: &str, packages: Vec<PackageDefinition>) -> PackageSet {
    PackageSet {
        name: String::from(name),
        description: String::from(description),
        packages,
    }
}

// ---------------------------------------------------------------------------
// Base System
// ---------------------------------------------------------------------------

/// Returns package sets for the minimal base system.
///
/// Sets:
/// - `base-system`: kernel, init, shell, coreutils
/// - `dev-tools`: compiler toolchain, build tools
/// - `system-libs`: core libraries
#[cfg(feature = "alloc")]
pub fn get_base_system_packages() -> Vec<PackageSet> {
    vec![
        pkgset(
            "base-system",
            "Core operating system packages required for a minimal installation",
            vec![
                pkg(
                    "kernel",
                    "VeridianOS microkernel",
                    PackageCategory::Base,
                    true,
                ),
                pkg(
                    "init",
                    "System init process (PID 1)",
                    PackageCategory::Base,
                    true,
                ),
                pkg("vsh", "Veridian shell", PackageCategory::Base, true),
                pkg(
                    "ls",
                    "List directory contents",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "cat",
                    "Concatenate and display files",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "cp",
                    "Copy files and directories",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "mv",
                    "Move or rename files",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "rm",
                    "Remove files and directories",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "mkdir",
                    "Create directories",
                    PackageCategory::CoreUtils,
                    true,
                ),
                pkg(
                    "chmod",
                    "Change file permissions",
                    PackageCategory::CoreUtils,
                    true,
                ),
            ],
        ),
        pkgset(
            "dev-tools",
            "Development toolchain and build utilities",
            vec![
                pkg(
                    "gcc-veridian",
                    "GCC cross-compiler targeting VeridianOS",
                    PackageCategory::DevTools,
                    false,
                ),
                pkg(
                    "binutils-veridian",
                    "GNU binutils for VeridianOS targets",
                    PackageCategory::DevTools,
                    false,
                ),
                pkg(
                    "make",
                    "GNU Make build system",
                    PackageCategory::DevTools,
                    false,
                ),
                pkg(
                    "pkg-config",
                    "Helper tool for compiling against libraries",
                    PackageCategory::DevTools,
                    false,
                ),
                pkg(
                    "cmake",
                    "Cross-platform build system generator",
                    PackageCategory::DevTools,
                    false,
                ),
            ],
        ),
        pkgset(
            "system-libs",
            "Core system libraries",
            vec![
                pkg(
                    "libc-veridian",
                    "C standard library for VeridianOS",
                    PackageCategory::SystemLibs,
                    true,
                ),
                pkg(
                    "libveridian",
                    "VeridianOS system call wrapper library",
                    PackageCategory::SystemLibs,
                    true,
                ),
                pkg(
                    "libcrypto-veridian",
                    "Cryptographic library with post-quantum support",
                    PackageCategory::SystemLibs,
                    false,
                ),
            ],
        ),
    ]
}

// ---------------------------------------------------------------------------
// Essential Applications
// ---------------------------------------------------------------------------

/// Returns package sets for essential user-facing applications.
///
/// Sets:
/// - `editors`: text editing
/// - `file-management`: file management tools
/// - `network-tools`: networking utilities
/// - `system-monitor`: system monitoring
#[cfg(feature = "alloc")]
pub fn get_essential_apps() -> Vec<PackageSet> {
    vec![
        pkgset(
            "editors",
            "Text editors",
            vec![pkg(
                "ve",
                "Veridian Editor -- terminal-based text editor",
                PackageCategory::TextEditor,
                false,
            )],
        ),
        pkgset(
            "file-management",
            "File management utilities",
            vec![pkg(
                "vfm",
                "Veridian File Manager -- TUI file browser",
                PackageCategory::FileManager,
                false,
            )],
        ),
        pkgset(
            "network-tools",
            "Networking utilities",
            vec![
                pkg(
                    "vcurl",
                    "HTTP client for fetching URLs",
                    PackageCategory::NetworkTools,
                    false,
                ),
                pkg(
                    "vping",
                    "ICMP ping utility",
                    PackageCategory::NetworkTools,
                    false,
                ),
                pkg(
                    "vdns",
                    "DNS lookup utility",
                    PackageCategory::NetworkTools,
                    false,
                ),
            ],
        ),
        pkgset(
            "system-monitor",
            "System monitoring and process management",
            vec![
                pkg(
                    "vtop",
                    "Interactive process viewer",
                    PackageCategory::SystemMonitor,
                    false,
                ),
                pkg(
                    "vps",
                    "Process status listing",
                    PackageCategory::SystemMonitor,
                    false,
                ),
            ],
        ),
    ]
}

// ---------------------------------------------------------------------------
// Driver Packages
// ---------------------------------------------------------------------------

/// Returns architecture-specific driver package sets.
///
/// Each driver set includes VirtIO drivers (available on all architectures)
/// plus native hardware drivers specific to the given architecture.
///
/// Supported architecture strings: `"x86_64"`, `"aarch64"`, `"riscv64"`.
#[cfg(feature = "alloc")]
pub fn get_driver_packages(arch: &str) -> Vec<PackageSet> {
    let mut sets = Vec::new();

    // -- Graphics drivers ---------------------------------------------------
    let mut graphics = vec![pkg(
        "virtio-gpu",
        "VirtIO GPU driver",
        PackageCategory::GraphicsDriver,
        false,
    )];
    if arch == "x86_64" {
        graphics.push(pkg(
            "i915",
            "Intel integrated graphics driver",
            PackageCategory::GraphicsDriver,
            false,
        ));
    }
    sets.push(pkgset(
        "graphics-drv",
        "Graphics and display drivers",
        graphics,
    ));

    // -- Network drivers ----------------------------------------------------
    let mut network = vec![pkg(
        "virtio-net",
        "VirtIO network driver",
        PackageCategory::NetworkDriver,
        false,
    )];
    if arch == "x86_64" {
        network.push(pkg(
            "e1000",
            "Intel Gigabit Ethernet driver",
            PackageCategory::NetworkDriver,
            false,
        ));
    }
    sets.push(pkgset("network-drv", "Network interface drivers", network));

    // -- Storage drivers ----------------------------------------------------
    let mut storage = vec![pkg(
        "virtio-blk",
        "VirtIO block storage driver",
        PackageCategory::StorageDriver,
        false,
    )];
    if arch == "x86_64" {
        storage.push(pkg(
            "nvme",
            "NVMe solid-state drive driver",
            PackageCategory::StorageDriver,
            false,
        ));
    }
    if arch == "aarch64" {
        storage.push(pkg(
            "sd-mmc",
            "SD/MMC card driver",
            PackageCategory::StorageDriver,
            false,
        ));
    }
    sets.push(pkgset("storage-drv", "Storage device drivers", storage));

    // -- Input drivers ------------------------------------------------------
    let mut input: Vec<PackageDefinition> = Vec::new();
    if arch == "x86_64" {
        input.push(pkg(
            "ps2-keyboard",
            "PS/2 keyboard driver",
            PackageCategory::InputDriver,
            false,
        ));
    }
    input.push(pkg(
        "usb-hid",
        "USB Human Interface Device driver",
        PackageCategory::InputDriver,
        false,
    ));
    sets.push(pkgset("input-drv", "Input device drivers", input));

    sets
}
