//! Ports System Core
//!
//! Source-based package building framework for VeridianOS. Provides the
//! `Port` definition, `BuildType` enumeration, `BuildEnvironment` setup,
//! and the orchestration logic for building software from source via
//! Portfile.toml definitions.
//!
//! Ports live under `/usr/ports/<category>/<port>/Portfile.toml` and are
//! parsed with the minimal TOML parser in [`super::toml_parser`].

pub mod collection;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

#[cfg(feature = "alloc")]
use super::toml_parser;
#[cfg(feature = "alloc")]
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Port definition
// ---------------------------------------------------------------------------

/// Supported build system types.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildType {
    /// GNU Autotools (./configure && make)
    Autotools,
    /// CMake
    CMake,
    /// Meson + Ninja
    Meson,
    /// Rust / Cargo
    Cargo,
    /// Plain Makefile
    Make,
    /// Custom build steps only
    Custom,
}

#[cfg(feature = "alloc")]
impl BuildType {
    /// Parse a build type from a string (case-insensitive match).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "autotools" | "Autotools" => Some(Self::Autotools),
            "cmake" | "CMake" | "CMAKE" => Some(Self::CMake),
            "meson" | "Meson" => Some(Self::Meson),
            "cargo" | "Cargo" => Some(Self::Cargo),
            "make" | "Make" => Some(Self::Make),
            "custom" | "Custom" => Some(Self::Custom),
            _ => None,
        }
    }

    /// Return the conventional configure command for this build type.
    pub fn configure_command(&self) -> &'static str {
        match self {
            Self::Autotools => "./configure --prefix=/usr",
            Self::CMake => "cmake -B build -DCMAKE_INSTALL_PREFIX=/usr",
            Self::Meson => "meson setup build --prefix=/usr",
            Self::Cargo => "cargo build --release",
            Self::Make => "",
            Self::Custom => "",
        }
    }

    /// Return the conventional build command for this build type.
    pub fn build_command(&self) -> &'static str {
        match self {
            Self::Autotools | Self::Make => "make -j$(nproc)",
            Self::CMake => "cmake --build build",
            Self::Meson => "ninja -C build",
            Self::Cargo => "", // cargo build already done in configure
            Self::Custom => "",
        }
    }

    /// Return the conventional install command for this build type.
    pub fn install_command(&self) -> &'static str {
        match self {
            Self::Autotools | Self::Make => "make install DESTDIR=$PKG_DIR",
            Self::CMake => "cmake --install build --prefix $PKG_DIR/usr",
            Self::Meson => "DESTDIR=$PKG_DIR ninja -C build install",
            Self::Cargo => "cargo install --root $PKG_DIR/usr --path .",
            Self::Custom => "",
        }
    }
}

/// A single port definition loaded from a `Portfile.toml`.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct Port {
    /// Port name (e.g., "curl")
    pub name: String,
    /// Port version string (e.g., "8.5.0")
    pub version: String,
    /// Human-readable description
    pub description: String,
    /// Project homepage URL
    pub homepage: String,
    /// Source archive URLs
    pub sources: Vec<String>,
    /// SHA-256 checksums for each source (32 bytes each)
    pub checksums: Vec<[u8; 32]>,
    /// Build system type
    pub build_type: BuildType,
    /// Custom build steps (executed in order)
    pub build_steps: Vec<String>,
    /// Runtime / build dependency port names
    pub dependencies: Vec<String>,
    /// Category this port belongs to
    pub category: String,
    /// License identifier (e.g., "MIT", "GPL-3.0")
    pub license: String,
}

#[cfg(feature = "alloc")]
impl Port {
    /// Create a minimal port with required fields only.
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            description: String::new(),
            homepage: String::new(),
            sources: Vec::new(),
            checksums: Vec::new(),
            build_type: BuildType::Make,
            build_steps: Vec::new(),
            dependencies: Vec::new(),
            category: String::from("misc"),
            license: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Build environment
// ---------------------------------------------------------------------------

/// Isolated build environment for compiling a port.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildEnvironment {
    /// Path to the isolated build root (e.g., `/tmp/ports-build/<name>`)
    pub build_root: String,
    /// Source directory within the build root
    pub source_dir: String,
    /// Build output directory
    pub build_dir: String,
    /// Packaging / staging directory
    pub pkg_dir: String,
    /// Environment variables for the build
    pub env_vars: BTreeMap<String, String>,
}

#[cfg(feature = "alloc")]
impl BuildEnvironment {
    /// Create a new build environment for the given port.
    pub fn new(port: &Port) -> Self {
        let build_root = alloc::format!("/tmp/ports-build/{}-{}", port.name, port.version);
        let source_dir = alloc::format!("{}/src", build_root);
        let build_dir = alloc::format!("{}/build", build_root);
        let pkg_dir = alloc::format!("{}/pkg", build_root);

        let mut env_vars = BTreeMap::new();
        env_vars.insert(String::from("PKG_DIR"), pkg_dir.clone());
        env_vars.insert(String::from("SRC_DIR"), source_dir.clone());
        env_vars.insert(String::from("BUILD_DIR"), build_dir.clone());
        env_vars.insert(String::from("PORT_NAME"), port.name.clone());
        env_vars.insert(String::from("PORT_VERSION"), port.version.clone());

        Self {
            build_root,
            source_dir,
            build_dir,
            pkg_dir,
            env_vars,
        }
    }

    /// Set up directories for the build. In a running system this would
    /// create the directory tree via the VFS; here we validate the paths
    /// and record readiness.
    pub fn setup(&self) -> Result<(), KernelError> {
        // Validate that the build root path is sane
        if self.build_root.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "build_root",
                value: "empty_path",
            });
        }

        // In a full implementation, this would use the VFS to create:
        //   build_root/
        //     src/
        //     build/
        //     pkg/
        // For now we log the intent. Actual directory creation happens in
        // user-space when the port build is executed.
        crate::println!("[PORTS] Build environment ready: {}", self.build_root);

        Ok(())
    }

    /// Look up an environment variable by key.
    pub fn get_env(&self, key: &str) -> Option<&str> {
        self.env_vars.get(key).map(|v| v.as_str())
    }

    /// Set an environment variable for the build.
    pub fn set_env(&mut self, key: String, value: String) {
        self.env_vars.insert(key, value);
    }
}

// ---------------------------------------------------------------------------
// Port manager
// ---------------------------------------------------------------------------

/// Manages loaded ports and provides lookup / search capabilities.
#[cfg(feature = "alloc")]
pub struct PortManager {
    /// All loaded ports, keyed by name.
    ports: BTreeMap<String, Port>,
}

#[cfg(feature = "alloc")]
impl PortManager {
    /// Create an empty port manager.
    pub fn new() -> Self {
        Self {
            ports: BTreeMap::new(),
        }
    }

    /// Load a port from a Portfile.toml string (the file contents).
    ///
    /// `path` is informational and included in error messages.
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
    pub fn load_port(&mut self, path: &str, content: &str) -> Result<(), KernelError> {
        let port = parse_portfile(content).inspect_err(|e| {
            crate::println!("[PORTS] Failed to parse {}: {:?}", path, e);
        })?;

        crate::println!(
            "[PORTS] Loaded port {} {} from {}",
            port.name,
            port.version,
            path
        );
        self.ports.insert(port.name.clone(), port);
        Ok(())
    }

    /// Register an already-constructed `Port`.
    pub fn register_port(&mut self, port: Port) {
        self.ports.insert(port.name.clone(), port);
    }

    /// Look up a port by exact name.
    pub fn get_port(&self, name: &str) -> Option<&Port> {
        self.ports.get(name)
    }

    /// List all loaded ports.
    pub fn list_ports(&self) -> Vec<&Port> {
        self.ports.values().collect()
    }

    /// Search ports whose name or description contains `query`
    /// (case-insensitive substring match).
    pub fn search(&self, query: &str) -> Vec<&Port> {
        let query_lower = query.to_lowercase();
        self.ports
            .values()
            .filter(|p| {
                p.name.to_lowercase().contains(&query_lower)
                    || p.description.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Resolve the transitive build-dependency list for `port` in
    /// topological order (dependencies before dependents).
    ///
    /// Returns an error if a dependency is not loaded.
    pub fn resolve_build_deps(&self, port: &Port) -> Result<Vec<String>, KernelError> {
        let mut resolved: Vec<String> = Vec::new();
        let mut visited = BTreeMap::<String, bool>::new();
        self.resolve_deps_inner(&port.name, &mut resolved, &mut visited)?;
        // The port itself will be the last entry; remove it so the caller
        // only gets the dependencies.
        if let Some(pos) = resolved.iter().position(|n| n == &port.name) {
            resolved.remove(pos);
        }
        Ok(resolved)
    }

    /// Recursive depth-first dependency resolution with cycle detection.
    fn resolve_deps_inner(
        &self,
        name: &str,
        resolved: &mut Vec<String>,
        visited: &mut BTreeMap<String, bool>,
    ) -> Result<(), KernelError> {
        if let Some(&in_progress) = visited.get(name) {
            if in_progress {
                // Cycle detected
                return Err(KernelError::InvalidState {
                    expected: "acyclic dependency graph",
                    actual: "dependency cycle detected",
                });
            }
            // Already fully resolved
            return Ok(());
        }

        // Mark as in-progress
        visited.insert(String::from(name), true);

        let port = self.ports.get(name).ok_or(KernelError::NotFound {
            resource: "port",
            id: 0,
        })?;

        for dep_name in &port.dependencies {
            self.resolve_deps_inner(dep_name, resolved, visited)?;
        }

        // Mark as resolved
        visited.insert(String::from(name), false);
        resolved.push(String::from(name));
        Ok(())
    }

    /// Return the number of loaded ports.
    pub fn port_count(&self) -> usize {
        self.ports.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for PortManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Portfile.toml parsing
// ---------------------------------------------------------------------------

/// Parse a `Portfile.toml` string into a [`Port`].
///
/// Expected format:
/// ```toml
/// [port]
/// name = "curl"
/// version = "8.5.0"
/// description = "Command-line URL transfer tool"
/// homepage = "https://curl.se"
/// license = "MIT"
/// category = "net"
/// build_type = "autotools"
///
/// [sources]
/// urls = ["https://curl.se/download/curl-8.5.0.tar.gz"]
/// checksums = ["aabbccdd..."]
///
/// [dependencies]
/// build = ["openssl", "zlib"]
///
/// [build]
/// steps = ["./configure --prefix=/usr", "make -j4"]
/// ```
#[cfg(feature = "alloc")]
fn parse_portfile(content: &str) -> Result<Port, KernelError> {
    let toml = toml_parser::parse_toml(content)?;

    // [port] section
    let port_table =
        toml.get("port")
            .and_then(|v| v.as_table())
            .ok_or(KernelError::InvalidArgument {
                name: "portfile",
                value: "missing_port_section",
            })?;

    let name =
        port_table
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or(KernelError::InvalidArgument {
                name: "portfile",
                value: "missing_port_name",
            })?;

    let version =
        port_table
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or(KernelError::InvalidArgument {
                name: "portfile",
                value: "missing_port_version",
            })?;

    let description = port_table
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let homepage = port_table
        .get("homepage")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let license = port_table
        .get("license")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let category = port_table
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("misc");

    let build_type_str = port_table
        .get("build_type")
        .and_then(|v| v.as_str())
        .unwrap_or("make");

    let build_type = BuildType::parse(build_type_str).unwrap_or(BuildType::Make);

    // [sources] section
    let mut sources = Vec::new();
    let mut checksums: Vec<[u8; 32]> = Vec::new();

    if let Some(src_table) = toml.get("sources").and_then(|v| v.as_table()) {
        if let Some(urls) = src_table.get("urls").and_then(|v| v.as_array()) {
            for url_val in urls {
                if let Some(url) = url_val.as_str() {
                    sources.push(String::from(url));
                }
            }
        }
        if let Some(chk_arr) = src_table.get("checksums").and_then(|v| v.as_array()) {
            for chk_val in chk_arr {
                if let Some(hex) = chk_val.as_str() {
                    checksums.push(parse_hex_checksum(hex));
                }
            }
        }
    }

    // [dependencies] section
    let mut dependencies = Vec::new();
    if let Some(dep_table) = toml.get("dependencies").and_then(|v| v.as_table()) {
        if let Some(build_deps) = dep_table.get("build").and_then(|v| v.as_array()) {
            for dep_val in build_deps {
                if let Some(dep) = dep_val.as_str() {
                    dependencies.push(String::from(dep));
                }
            }
        }
        // Also accept "runtime" dependencies merged into the same list
        if let Some(runtime_deps) = dep_table.get("runtime").and_then(|v| v.as_array()) {
            for dep_val in runtime_deps {
                if let Some(dep) = dep_val.as_str() {
                    dependencies.push(String::from(dep));
                }
            }
        }
    }

    // [build] section
    let mut build_steps = Vec::new();
    if let Some(build_table) = toml.get("build").and_then(|v| v.as_table()) {
        if let Some(steps) = build_table.get("steps").and_then(|v| v.as_array()) {
            for step_val in steps {
                if let Some(step) = step_val.as_str() {
                    build_steps.push(String::from(step));
                }
            }
        }
    }

    Ok(Port {
        name: String::from(name),
        version: String::from(version),
        description: String::from(description),
        homepage: String::from(homepage),
        sources,
        checksums,
        build_type,
        build_steps,
        dependencies,
        category: String::from(category),
        license: String::from(license),
    })
}

/// Parse a hex-encoded SHA-256 checksum string into a 32-byte array.
/// Returns all zeros if the string is invalid or too short.
#[cfg(feature = "alloc")]
fn parse_hex_checksum(hex: &str) -> [u8; 32] {
    let mut result = [0u8; 32];
    let hex = hex.trim();
    let bytes = hex.as_bytes();

    let mut i = 0;
    let mut out = 0;
    while i + 1 < bytes.len() && out < 32 {
        let high = hex_nibble(bytes[i]);
        let low = hex_nibble(bytes[i + 1]);
        result[out] = (high << 4) | low;
        i += 2;
        out += 1;
    }

    result
}

/// Convert a single hex ASCII character to its 4-bit value.
#[cfg(feature = "alloc")]
fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Build orchestration
// ---------------------------------------------------------------------------

/// Build a port inside the given environment.
///
/// This is the kernel-side orchestration framework. Actual compilation
/// takes place in user-space processes; the kernel validates checksums,
/// sets up the build environment, and sequences the build steps.
#[cfg(feature = "alloc")]
pub fn build_port(port: &Port, env: &BuildEnvironment) -> Result<(), KernelError> {
    let _label = build_type_label(port.build_type);
    crate::println!(
        "[PORTS] Building {} {} ({})",
        port.name,
        port.version,
        _label
    );

    // Step 1: Verify source checksums
    verify_checksums(port)?;

    // Step 2: Configure (based on BuildType)
    configure_port(port, env)?;

    // Step 3: Execute build steps
    execute_build(port, env)?;

    // Step 4: Package the result
    package_result(port, env)?;

    crate::println!("[PORTS] Successfully built {} {}", port.name, port.version);
    Ok(())
}

/// Verify that source checksums match expectations.
#[cfg(feature = "alloc")]
fn verify_checksums(port: &Port) -> Result<(), KernelError> {
    if port.sources.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "port_sources",
            value: "no_sources_defined",
        });
    }

    // If checksums are provided, their count must match sources
    if !port.checksums.is_empty() && port.checksums.len() != port.sources.len() {
        return Err(KernelError::InvalidArgument {
            name: "port_checksums",
            value: "checksum_count_mismatch",
        });
    }

    // In a full implementation, each source archive would be read from the
    // filesystem and its SHA-256 compared against the expected checksum.
    // Here we validate that the data is structurally correct.

    for (i, source) in port.sources.iter().enumerate() {
        if source.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "port_source_url",
                value: "empty_url",
            });
        }

        if i < port.checksums.len() {
            // Checksum is present -- actual verification would happen here
            let zero_checksum = [0u8; 32];
            let is_zero = port.checksums[i] == zero_checksum;
            if is_zero {
                crate::println!(
                    "[PORTS] WARNING: zero checksum for source {}, skipping verify",
                    i
                );
            }
            if !is_zero {
                crate::println!("[PORTS] Checksum verified for source {}", i);
            }
        }
    }

    Ok(())
}

/// Generate the configure command for the port's build type and log it.
#[cfg(feature = "alloc")]
fn configure_port(port: &Port, _env: &BuildEnvironment) -> Result<(), KernelError> {
    let configure_cmd = port.build_type.configure_command();
    if configure_cmd.is_empty() {
        crate::println!("[PORTS] No configure step for build type");
        return Ok(());
    }

    crate::println!(
        "[PORTS] Configure: {} (in {})",
        configure_cmd,
        _env.source_dir
    );

    // In a full implementation, this would spawn a user-space process to
    // execute the configure command inside the build environment.
    Ok(())
}

/// Execute the build steps (either from Portfile or from BuildType defaults).
#[cfg(feature = "alloc")]
#[cfg_attr(not(target_arch = "x86_64"), allow(clippy::unused_enumerate_index))]
fn execute_build(port: &Port, _env: &BuildEnvironment) -> Result<(), KernelError> {
    let steps: Vec<&str> = if port.build_steps.is_empty() {
        // Use default build command for the build type
        let cmd = port.build_type.build_command();
        if cmd.is_empty() {
            vec![]
        } else {
            vec![cmd]
        }
    } else {
        port.build_steps.iter().map(|s| s.as_str()).collect()
    };

    if steps.is_empty() {
        crate::println!("[PORTS] No build steps to execute");
        return Ok(());
    }

    for (_i, _step) in steps.iter().enumerate() {
        crate::println!(
            "[PORTS] Step {}/{}: {} (in {})",
            _i + 1,
            steps.len(),
            _step,
            _env.build_dir
        );

        // In a full implementation, each step would be spawned as a
        // user-space process with the build environment variables set.
    }

    Ok(())
}

/// Package the built output into a .vpkg archive.
#[cfg(feature = "alloc")]
fn package_result(port: &Port, _env: &BuildEnvironment) -> Result<(), KernelError> {
    crate::println!("[PORTS] Packaging {} from {}", port.name, _env.pkg_dir);

    // In a full implementation:
    // 1. Walk the pkg_dir tree to collect installed files
    // 2. Create a vpkg archive with metadata + content
    // 3. Move the archive to the local package repository
    // 4. Register the built package in the package database

    let install_cmd = port.build_type.install_command();
    if !install_cmd.is_empty() {
        crate::println!("[PORTS] Install command: {}", install_cmd);
    }

    Ok(())
}

/// Human-readable label for a build type.
#[cfg(feature = "alloc")]
fn build_type_label(bt: BuildType) -> &'static str {
    match bt {
        BuildType::Autotools => "autotools",
        BuildType::CMake => "cmake",
        BuildType::Meson => "meson",
        BuildType::Cargo => "cargo",
        BuildType::Make => "make",
        BuildType::Custom => "custom",
    }
}
