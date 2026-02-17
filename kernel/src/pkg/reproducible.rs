//! Reproducible Builds Infrastructure
//!
//! Ensures deterministic build outputs by normalizing build environments,
//! recording build inputs/outputs, and verifying reproducibility across
//! independent builds.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

#[cfg(feature = "alloc")]
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Captures the complete build environment state for reproducibility.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildSnapshot {
    /// Compiler/toolchain version string (e.g., "rustc 1.93.0-nightly")
    pub toolchain_version: String,
    /// Sorted environment variables present at build time
    pub env_vars: BTreeMap<String, String>,
    /// If set, replaces real timestamps for reproducibility
    pub timestamp_override: Option<u64>,
    /// (filename, SHA-256 hash) pairs for source files
    pub source_hashes: Vec<(String, [u8; 32])>,
    /// Target triple (e.g., "x86_64-veridian")
    pub target_triple: String,
}

#[cfg(feature = "alloc")]
impl BuildSnapshot {
    /// Create a new empty snapshot.
    pub fn new() -> Self {
        Self {
            toolchain_version: String::new(),
            env_vars: BTreeMap::new(),
            timestamp_override: None,
            source_hashes: Vec::new(),
            target_triple: String::new(),
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for BuildSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// Records the complete inputs and outputs of a single build.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildManifest {
    /// Port name that was built
    pub port_name: String,
    /// Port version that was built
    pub port_version: String,
    /// All build inputs
    pub inputs: BuildInputs,
    /// All build outputs
    pub outputs: BuildOutputs,
    /// Wall-clock build duration in milliseconds
    pub build_duration_ms: u64,
}

/// Input specification for a build.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildInputs {
    /// SHA-256 hashes of source files
    pub source_hashes: Vec<(String, [u8; 32])>,
    /// Environment snapshot at build time
    pub env_snapshot: BuildSnapshot,
    /// Dependency name -> version mapping
    pub dependency_versions: BTreeMap<String, String>,
}

#[cfg(feature = "alloc")]
impl BuildInputs {
    /// Create empty build inputs.
    pub fn new() -> Self {
        Self {
            source_hashes: Vec::new(),
            env_snapshot: BuildSnapshot::new(),
            dependency_versions: BTreeMap::new(),
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for BuildInputs {
    fn default() -> Self {
        Self::new()
    }
}

/// Output specification for a build.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildOutputs {
    /// (filename, SHA-256 hash) pairs for output files
    pub file_hashes: Vec<(String, [u8; 32])>,
    /// Total output size in bytes
    pub total_size: u64,
    /// Number of output files
    pub file_count: usize,
}

#[cfg(feature = "alloc")]
impl BuildOutputs {
    /// Create empty build outputs.
    pub fn new() -> Self {
        Self {
            file_hashes: Vec::new(),
            total_size: 0,
            file_count: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for BuildOutputs {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of comparing two build manifests for reproducibility.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ReproducibilityResult {
    /// Files whose hashes match between both builds
    pub matching_files: Vec<String>,
    /// Files present in both builds with differing hashes: (path, hash_a,
    /// hash_b)
    pub differing_files: Vec<(String, [u8; 32], [u8; 32])>,
    /// Files present only in build B (missing from A)
    pub missing_in_a: Vec<String>,
    /// Files present only in build A (missing from B)
    pub missing_in_b: Vec<String>,
}

#[cfg(feature = "alloc")]
impl ReproducibilityResult {
    /// Returns true if the builds are fully reproducible (all outputs match).
    pub fn is_reproducible(&self) -> bool {
        self.differing_files.is_empty()
            && self.missing_in_a.is_empty()
            && self.missing_in_b.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

/// Normalize a build environment for reproducibility.
///
/// - Sets `SOURCE_DATE_EPOCH` to "0" to eliminate timestamp variation
/// - Removes locale-dependent variables (`LC_ALL`, `LANG`, `LANGUAGE`) and sets
///   them to "C" for deterministic collation
/// - Sets `TZ=UTC` for timezone consistency
/// - Canonicalizes paths by stripping trailing slashes and collapsing double
///   slashes
#[cfg(feature = "alloc")]
pub fn normalize_environment(env: &mut super::ports::BuildEnvironment) {
    // Zero out timestamp-dependent variables
    env.env_vars
        .insert(String::from("SOURCE_DATE_EPOCH"), String::from("0"));

    // Set locale to "C" for deterministic collation/formatting
    env.env_vars
        .insert(String::from("LC_ALL"), String::from("C"));
    env.env_vars.insert(String::from("LANG"), String::from("C"));
    env.env_vars
        .insert(String::from("LANGUAGE"), String::from("C"));

    // Set timezone to UTC
    env.env_vars.insert(String::from("TZ"), String::from("UTC"));

    // Canonicalize path values: strip trailing slashes, collapse double slashes
    let keys: Vec<String> = env.env_vars.keys().cloned().collect();
    for key in keys {
        if let Some(val) = env.env_vars.get_mut(&key) {
            *val = canonicalize_path_value(val);
        }
    }
}

/// Canonicalize a path string: collapse double slashes and strip trailing
/// slash.
#[cfg(feature = "alloc")]
fn canonicalize_path_value(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut prev_slash = false;

    for ch in path.chars() {
        if ch == '/' {
            if !prev_slash {
                result.push('/');
            }
            prev_slash = true;
        } else {
            prev_slash = false;
            result.push(ch);
        }
    }

    // Strip trailing slash unless the path is exactly "/"
    if result.len() > 1 && result.ends_with('/') {
        result.pop();
    }

    result
}

/// Create a build manifest recording all inputs and outputs.
///
/// Reads the port's source checksums as inputs and attempts to walk
/// `output_dir` via the VFS to compute output file hashes. If the VFS
/// is unavailable, outputs are recorded as empty.
#[cfg(feature = "alloc")]
pub fn create_build_manifest(
    port: &super::ports::Port,
    env: &super::ports::BuildEnvironment,
    output_dir: &str,
) -> Result<BuildManifest, KernelError> {
    // Build inputs from port checksums
    let mut source_hashes = Vec::new();
    for (i, checksum) in port.checksums.iter().enumerate() {
        let name = if i < port.sources.len() {
            port.sources[i].clone()
        } else {
            alloc::format!("source-{}", i)
        };
        source_hashes.push((name, *checksum));
    }

    // Build environment snapshot
    let env_snapshot = BuildSnapshot {
        toolchain_version: env
            .get_env("RUSTC_VERSION")
            .map(String::from)
            .unwrap_or_default(),
        env_vars: env.env_vars.clone(),
        timestamp_override: Some(0), // Reproducible builds always override
        source_hashes: source_hashes.clone(),
        target_triple: env.get_env("TARGET").map(String::from).unwrap_or_default(),
    };

    // Dependency versions from environment
    let mut dependency_versions = BTreeMap::new();
    for dep in &port.dependencies {
        dependency_versions.insert(dep.clone(), String::from("*"));
    }

    let inputs = BuildInputs {
        source_hashes,
        env_snapshot,
        dependency_versions,
    };

    // Compute outputs by walking the output directory via VFS
    let outputs = compute_outputs(output_dir);

    Ok(BuildManifest {
        port_name: port.name.clone(),
        port_version: port.version.clone(),
        inputs,
        outputs,
        build_duration_ms: 0, // Actual timing comes from the build runner
    })
}

/// Walk an output directory via VFS and hash all files found.
///
/// Returns empty outputs if the VFS is unavailable or the path does
/// not exist.
#[cfg(feature = "alloc")]
fn compute_outputs(output_dir: &str) -> BuildOutputs {
    let mut outputs = BuildOutputs::new();

    let vfs_lock = match crate::fs::try_get_vfs() {
        Some(lock) => lock,
        None => {
            crate::println!(
                "[REPRO] VFS unavailable, recording empty outputs for {}",
                output_dir
            );
            return outputs;
        }
    };

    let vfs = vfs_lock.read();

    // Try to resolve the output directory
    let dir_node = match vfs.resolve_path(output_dir) {
        Ok(node) => node,
        Err(_) => {
            crate::println!(
                "[REPRO] Output directory not found: {}, recording empty outputs",
                output_dir
            );
            return outputs;
        }
    };

    // Read directory entries
    let entries = match dir_node.readdir() {
        Ok(entries) => entries,
        Err(_) => {
            return outputs;
        }
    };

    for entry in &entries {
        // Skip . and .. entries
        if entry.name == "." || entry.name == ".." {
            continue;
        }

        if entry.node_type == crate::fs::NodeType::File {
            let file_path = alloc::format!("{}/{}", output_dir, entry.name);
            if let Ok(node) = vfs.resolve_path(&file_path) {
                if let Ok(metadata) = node.metadata() {
                    let size = metadata.size;
                    // Read file contents to hash
                    let mut buf = vec![0u8; size];
                    if let Ok(bytes_read) = node.read(0, &mut buf) {
                        buf.truncate(bytes_read);
                        let hash = crate::crypto::hash::sha256(&buf);
                        outputs
                            .file_hashes
                            .push((entry.name.clone(), *hash.as_bytes()));
                        outputs.total_size += size as u64;
                        outputs.file_count += 1;
                    }
                }
            }
        }
    }

    outputs
}

/// Compare two build manifests and produce a reproducibility report.
///
/// Walks through all output files in both manifests, categorizing each
/// file as matching, differing, or missing from one side.
#[cfg(feature = "alloc")]
pub fn verify_reproducible(a: &BuildManifest, b: &BuildManifest) -> ReproducibilityResult {
    let map_a: BTreeMap<&str, &[u8; 32]> = a
        .outputs
        .file_hashes
        .iter()
        .map(|(name, hash)| (name.as_str(), hash))
        .collect();

    let map_b: BTreeMap<&str, &[u8; 32]> = b
        .outputs
        .file_hashes
        .iter()
        .map(|(name, hash)| (name.as_str(), hash))
        .collect();

    let mut matching_files = Vec::new();
    let mut differing_files = Vec::new();
    let mut missing_in_a = Vec::new();
    let mut missing_in_b = Vec::new();

    // Check all files in A
    for (name, hash_a) in &map_a {
        match map_b.get(name) {
            Some(hash_b) => {
                if hash_a == hash_b {
                    matching_files.push(String::from(*name));
                } else {
                    differing_files.push((String::from(*name), **hash_a, **hash_b));
                }
            }
            None => {
                missing_in_b.push(String::from(*name));
            }
        }
    }

    // Check for files only in B
    for name in map_b.keys() {
        if !map_a.contains_key(name) {
            missing_in_a.push(String::from(*name));
        }
    }

    ReproducibilityResult {
        matching_files,
        differing_files,
        missing_in_a,
        missing_in_b,
    }
}

/// Serialize a build manifest to a simple text format for VFS storage.
///
/// Format:
/// ```text
/// PORT={name}
/// VERSION={version}
/// TOOLCHAIN={toolchain_version}
/// TARGET={target_triple}
/// DURATION_MS={duration}
/// INPUT_COUNT={count}
/// INPUT:{filename}={hex_hash}
/// ...
/// OUTPUT_COUNT={count}
/// OUTPUT:{filename}={hex_hash}
/// ...
/// TOTAL_SIZE={size}
/// ```
#[cfg(feature = "alloc")]
pub fn serialize_manifest(manifest: &BuildManifest) -> Vec<u8> {
    let mut out = String::new();

    out.push_str("PORT=");
    out.push_str(&manifest.port_name);
    out.push('\n');

    out.push_str("VERSION=");
    out.push_str(&manifest.port_version);
    out.push('\n');

    out.push_str("TOOLCHAIN=");
    out.push_str(&manifest.inputs.env_snapshot.toolchain_version);
    out.push('\n');

    out.push_str("TARGET=");
    out.push_str(&manifest.inputs.env_snapshot.target_triple);
    out.push('\n');

    out.push_str("DURATION_MS=");
    push_u64(&mut out, manifest.build_duration_ms);
    out.push('\n');

    // Input hashes
    out.push_str("INPUT_COUNT=");
    push_usize(&mut out, manifest.inputs.source_hashes.len());
    out.push('\n');

    for (name, hash) in &manifest.inputs.source_hashes {
        out.push_str("INPUT:");
        out.push_str(name);
        out.push('=');
        out.push_str(&bytes_to_hex(hash));
        out.push('\n');
    }

    // Output hashes
    out.push_str("OUTPUT_COUNT=");
    push_usize(&mut out, manifest.outputs.file_hashes.len());
    out.push('\n');

    for (name, hash) in &manifest.outputs.file_hashes {
        out.push_str("OUTPUT:");
        out.push_str(name);
        out.push('=');
        out.push_str(&bytes_to_hex(hash));
        out.push('\n');
    }

    out.push_str("TOTAL_SIZE=");
    push_u64(&mut out, manifest.outputs.total_size);
    out.push('\n');

    out.into_bytes()
}

/// Convert a byte slice to a lowercase hex string.
#[cfg(feature = "alloc")]
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let high = HEX_CHARS[(b >> 4) as usize];
        let low = HEX_CHARS[(b & 0x0f) as usize];
        hex.push(high as char);
        hex.push(low as char);
    }
    hex
}

/// Hex character lookup table.
#[cfg(feature = "alloc")]
const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Append a `u64` as decimal to a string (no_std helper).
#[cfg(feature = "alloc")]
fn push_u64(s: &mut String, value: u64) {
    use core::fmt::Write;
    let _ = write!(s, "{}", value);
}

/// Append a `usize` as decimal to a string (no_std helper).
#[cfg(feature = "alloc")]
fn push_usize(s: &mut String, value: usize) {
    use core::fmt::Write;
    let _ = write!(s, "{}", value);
}
