//! SDK Generator for VeridianOS Package Development
//!
//! Provides an SDK packaging framework for creating distributable development
//! kits containing headers, libraries, tools, and documentation. The generator
//! validates SDK manifests and produces serialized SDK packages suitable for
//! distribution.
//!
//! TODO(user-space): Actual file collection requires a functional VFS layer.
//! Currently the generator validates manifests and produces placeholder package
//! bytes.

#[cfg(feature = "alloc")]
use alloc::{format, string::String, vec::Vec};

use crate::error::{KernelError, KernelResult};

// ============================================================================
// SdkComponent
// ============================================================================

/// Identifies a category of files within an SDK package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SdkComponent {
    /// C/C++ header files for the VeridianOS API.
    Headers,
    /// Static libraries (.a) for linking.
    StaticLibs,
    /// Shared/dynamic libraries (.so) for runtime linking.
    SharedLibs,
    /// Build tools and utilities (compilers, linkers, etc.).
    Tools,
    /// API documentation and guides.
    Documentation,
    /// Example programs and code snippets.
    Examples,
}

#[allow(dead_code)]
impl SdkComponent {
    /// Return a short identifier for this component.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::StaticLibs => "static-libs",
            Self::SharedLibs => "shared-libs",
            Self::Tools => "tools",
            Self::Documentation => "documentation",
            Self::Examples => "examples",
        }
    }

    /// Return a human-readable description of this component.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Headers => "C/C++ header files for the VeridianOS API",
            Self::StaticLibs => "Static libraries for compile-time linking",
            Self::SharedLibs => "Shared libraries for runtime linking",
            Self::Tools => "Build tools and utilities",
            Self::Documentation => "API documentation and developer guides",
            Self::Examples => "Example programs and code snippets",
        }
    }
}

// ============================================================================
// SdkManifest
// ============================================================================

/// Describes the contents and metadata of an SDK package.
///
/// The manifest must contain at least one component and one target architecture
/// to be considered valid.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SdkManifest {
    /// SDK version string (semver).
    pub version: String,
    /// Target architectures this SDK supports (e.g. "x86_64-veridian").
    pub target_archs: Vec<String>,
    /// Components included in this SDK package.
    pub components: Vec<SdkComponent>,
    /// Total size of all included files in bytes.
    pub total_size: u64,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl SdkManifest {
    /// Create a new empty SDK manifest with the given version.
    pub fn new(version: &str) -> Self {
        Self {
            version: String::from(version),
            target_archs: Vec::new(),
            components: Vec::new(),
            total_size: 0,
        }
    }

    /// Add a component to the manifest if not already present.
    pub fn add_component(&mut self, component: SdkComponent) {
        if !self.components.contains(&component) {
            self.components.push(component);
        }
    }

    /// Add a target architecture to the manifest if not already present.
    pub fn add_target_arch(&mut self, arch: &str) {
        let arch_string = String::from(arch);
        if !self.target_archs.contains(&arch_string) {
            self.target_archs.push(arch_string);
        }
    }

    /// Check whether the manifest includes the given component.
    pub fn has_component(&self, component: SdkComponent) -> bool {
        self.components.contains(&component)
    }

    /// Validate that the manifest has at least one component and one target.
    pub fn validate(&self) -> KernelResult<()> {
        if self.components.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "components",
                value: "manifest must contain at least one component",
            });
        }
        if self.target_archs.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "target_archs",
                value: "manifest must target at least one architecture",
            });
        }
        Ok(())
    }
}

// ============================================================================
// SdkPackageSpec
// ============================================================================

/// Full specification for building an SDK package, combining the manifest with
/// the paths to include.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SdkPackageSpec {
    /// The SDK manifest describing version, targets, and components.
    pub manifest: SdkManifest,
    /// Paths to header files to include.
    pub header_paths: Vec<String>,
    /// Paths to library files to include.
    pub lib_paths: Vec<String>,
    /// Paths to tool binaries to include.
    pub tool_paths: Vec<String>,
    /// Paths to documentation files to include.
    pub doc_paths: Vec<String>,
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
impl SdkPackageSpec {
    /// Create a new package spec from a manifest with empty path lists.
    pub fn new(manifest: SdkManifest) -> Self {
        Self {
            manifest,
            header_paths: Vec::new(),
            lib_paths: Vec::new(),
            tool_paths: Vec::new(),
            doc_paths: Vec::new(),
        }
    }

    /// Add a header file path.
    pub fn add_header_path(&mut self, path: &str) {
        self.header_paths.push(String::from(path));
    }

    /// Add a library file path.
    pub fn add_lib_path(&mut self, path: &str) {
        self.lib_paths.push(String::from(path));
    }

    /// Add a tool binary path.
    pub fn add_tool_path(&mut self, path: &str) {
        self.tool_paths.push(String::from(path));
    }

    /// Add a documentation file path.
    pub fn add_doc_path(&mut self, path: &str) {
        self.doc_paths.push(String::from(path));
    }
}

// ============================================================================
// SDK Generation
// ============================================================================

/// Generate an SDK package from the given specification.
///
/// Validates the manifest, then collects headers, libraries, tools, and
/// documentation into a serialized package byte stream.
///
/// TODO(user-space): Actual file collection from `/usr/include/veridian/`,
/// `/usr/lib/`, and the toolchain registry requires a functional VFS. This
/// currently produces a placeholder package containing only the manifest
/// metadata and generated pkg-config content.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub fn generate_sdk(spec: &SdkPackageSpec) -> KernelResult<Vec<u8>> {
    // Step 1: Validate the manifest
    spec.manifest.validate()?;

    let mut package_bytes: Vec<u8> = Vec::new();

    // Step 2: Write a simple header identifying this as an SDK package
    // Magic: "VSDK" (VeridianOS SDK)
    package_bytes.extend_from_slice(b"VSDK");

    // Version string length + data
    let version_bytes = spec.manifest.version.as_bytes();
    package_bytes.extend_from_slice(&(version_bytes.len() as u32).to_le_bytes());
    package_bytes.extend_from_slice(version_bytes);

    // Number of components
    package_bytes.extend_from_slice(&(spec.manifest.components.len() as u32).to_le_bytes());

    // Number of target architectures
    package_bytes.extend_from_slice(&(spec.manifest.target_archs.len() as u32).to_le_bytes());

    // Step 3: Collect headers from /usr/include/veridian/
    // TODO(user-space): Read actual header files via VFS
    let header_count = spec.header_paths.len() as u32;
    package_bytes.extend_from_slice(&header_count.to_le_bytes());

    // Step 4: Collect libraries from /usr/lib/
    // TODO(user-space): Read actual library files via VFS
    let lib_count = spec.lib_paths.len() as u32;
    package_bytes.extend_from_slice(&lib_count.to_le_bytes());

    // Step 5: Package tools from toolchain registry
    // TODO(user-space): Enumerate tools from registered toolchains
    let tool_count = spec.tool_paths.len() as u32;
    package_bytes.extend_from_slice(&tool_count.to_le_bytes());

    // Step 6: Generate pkg-config content for each component
    for component in &spec.manifest.components {
        let pc_content = generate_pkg_config_content(
            component.as_str(),
            &spec.manifest.version,
            component.description(),
            component.as_str(),
        );
        let pc_bytes = pc_content.as_bytes();
        package_bytes.extend_from_slice(&(pc_bytes.len() as u32).to_le_bytes());
        package_bytes.extend_from_slice(pc_bytes);
    }

    Ok(package_bytes)
}

/// Generate standard pkg-config `.pc` file content for an SDK component.
///
/// Produces output compatible with the `pkg-config` tool, defining the
/// prefix, include, and library paths for a named component.
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub fn generate_pkg_config_content(
    name: &str,
    version: &str,
    description: &str,
    lib_name: &str,
) -> String {
    let sysroot = super::get_sysroot();

    let mut output = String::new();
    output.push_str(&format!("prefix={}\n", sysroot));
    output.push_str("exec_prefix=${prefix}\n");
    output.push_str("libdir=${exec_prefix}/lib\n");
    output.push_str("includedir=${prefix}/include\n");
    output.push('\n');
    output.push_str(&format!("Name: {}\n", name));
    output.push_str(&format!("Version: {}\n", version));
    output.push_str(&format!("Description: {}\n", description));
    output.push_str(&format!("Cflags: -I{}/include/veridian\n", sysroot));
    output.push_str(&format!("Libs: -L{}/lib -l{}\n", sysroot, lib_name));

    output
}
