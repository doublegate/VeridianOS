//! Package Configuration Support
//!
//! Provides pkg-config compatible metadata generation for installed packages,
//! allowing build systems to discover compiler and linker flags required to
//! use a given library.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{format, string::String, vec::Vec};

/// A single include search path.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct IncludePath {
    /// Filesystem path to the include directory.
    pub path: String,
    /// Whether this is a system include path (uses `-isystem` instead of `-I`).
    pub system: bool,
}

/// A library dependency.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct LibraryPath {
    /// Directory containing the library.
    pub path: String,
    /// Library name (without `lib` prefix or file extension).
    pub name: String,
}

/// Package configuration metadata, compatible with the `pkg-config` format.
///
/// Stores the compiler and linker flags needed to build against a particular
/// installed library package.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PkgConfig {
    /// Package name.
    pub name: String,
    /// Package version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// Include search directories.
    pub include_dirs: Vec<String>,
    /// Library search directories.
    pub lib_dirs: Vec<String>,
    /// Libraries to link against (short names, e.g. "veridian").
    pub libs: Vec<String>,
    /// Additional compiler flags.
    pub cflags: Vec<String>,
}

#[cfg(feature = "alloc")]
impl PkgConfig {
    /// Generate a pkg-config compatible `.pc` file contents.
    ///
    /// The output follows the standard pkg-config format with `Name`,
    /// `Version`, `Description`, `Cflags`, and `Libs` fields.
    pub fn generate_pkg_config(&self) -> String {
        let sysroot = super::get_sysroot();

        let mut output = String::new();

        // Variable definitions
        output.push_str(&format!("prefix={}\n", sysroot));
        output.push_str("exec_prefix=${prefix}\n");
        output.push_str("libdir=${exec_prefix}/lib\n");
        output.push_str("includedir=${prefix}/include\n");
        output.push('\n');

        // Metadata fields
        output.push_str(&format!("Name: {}\n", self.name));
        output.push_str(&format!("Version: {}\n", self.version));
        output.push_str(&format!("Description: {}\n", self.description));

        // Cflags line
        let mut cflags_parts: Vec<String> = Vec::new();
        for dir in &self.include_dirs {
            cflags_parts.push(format!("-I{}", dir));
        }
        for flag in &self.cflags {
            cflags_parts.push(flag.clone());
        }
        if !cflags_parts.is_empty() {
            output.push_str("Cflags:");
            for part in &cflags_parts {
                output.push(' ');
                output.push_str(part);
            }
            output.push('\n');
        }

        // Libs line
        let mut libs_parts: Vec<String> = Vec::new();
        for dir in &self.lib_dirs {
            libs_parts.push(format!("-L{}", dir));
        }
        for lib in &self.libs {
            libs_parts.push(format!("-l{}", lib));
        }
        if !libs_parts.is_empty() {
            output.push_str("Libs:");
            for part in &libs_parts {
                output.push(' ');
                output.push_str(part);
            }
            output.push('\n');
        }

        output
    }

    /// Look up the pkg-config metadata for an installed package by name.
    ///
    /// In a full implementation this would query the VFS for `.pc` files under
    /// the sysroot. Currently returns built-in configurations for core
    /// VeridianOS libraries.
    pub fn find_package(name: &str) -> Option<PkgConfig> {
        let sysroot = super::get_sysroot();
        let triple = super::get_target_triple();

        match name {
            "veridian" => Some(PkgConfig {
                name: String::from("veridian"),
                version: String::from("0.4.0"),
                description: String::from("VeridianOS core system library"),
                include_dirs: alloc::vec![
                    format!("{}/include", sysroot),
                    format!("{}/include/veridian", sysroot),
                ],
                lib_dirs: alloc::vec![format!("{}/lib/{}", sysroot, triple)],
                libs: alloc::vec![String::from("veridian")],
                cflags: alloc::vec![
                    String::from("-ffreestanding"),
                    format!("--target={}", triple),
                ],
            }),
            "veridian-ipc" => Some(PkgConfig {
                name: String::from("veridian-ipc"),
                version: String::from("0.4.0"),
                description: String::from("VeridianOS IPC library"),
                include_dirs: alloc::vec![format!("{}/include/veridian/ipc", sysroot)],
                lib_dirs: alloc::vec![format!("{}/lib/{}", sysroot, triple)],
                libs: alloc::vec![String::from("veridian-ipc"), String::from("veridian"),],
                cflags: alloc::vec![format!("--target={}", triple)],
            }),
            "veridian-cap" => Some(PkgConfig {
                name: String::from("veridian-cap"),
                version: String::from("0.4.0"),
                description: String::from("VeridianOS capability library"),
                include_dirs: alloc::vec![format!("{}/include/veridian/cap", sysroot)],
                lib_dirs: alloc::vec![format!("{}/lib/{}", sysroot, triple)],
                libs: alloc::vec![String::from("veridian-cap"), String::from("veridian"),],
                cflags: alloc::vec![format!("--target={}", triple)],
            }),
            _ => {
                // TODO(future): query VFS for /usr/veridian/lib/pkgconfig/<name>.pc
                None
            }
        }
    }

    /// Create `IncludePath` entries from the stored include directories.
    pub fn include_paths(&self) -> Vec<IncludePath> {
        self.include_dirs
            .iter()
            .map(|p| IncludePath {
                path: p.clone(),
                system: false,
            })
            .collect()
    }

    /// Create `LibraryPath` entries from the stored library directories and
    /// names.
    pub fn library_paths(&self) -> Vec<LibraryPath> {
        let mut result = Vec::new();
        for dir in &self.lib_dirs {
            for lib in &self.libs {
                result.push(LibraryPath {
                    path: dir.clone(),
                    name: lib.clone(),
                });
            }
        }
        result
    }
}

#[cfg(feature = "alloc")]
impl core::fmt::Display for PkgConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.generate_pkg_config())
    }
}

#[cfg(feature = "alloc")]
impl core::fmt::Display for IncludePath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.system {
            write!(f, "-isystem {}", self.path)
        } else {
            write!(f, "-I{}", self.path)
        }
    }
}

#[cfg(feature = "alloc")]
impl core::fmt::Display for LibraryPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "-L{} -l{}", self.path, self.name)
    }
}
