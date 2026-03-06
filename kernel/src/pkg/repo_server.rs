//! Package Repository Hosting
//!
//! HTTP-based package repository server for VeridianOS. Hosts binary packages
//! with metadata indexing, Ed25519 signature verification, paginated search,
//! and JSON-like index generation for client consumption.
//!
//! ## HTTP Endpoints (conceptual)
//!
//! - `GET /api/packages` -- paginated package listing
//! - `GET /api/packages/{name}` -- all versions of a package
//! - `GET /api/packages/{name}/{version}` -- specific version metadata
//! - `POST /api/packages` -- upload with Ed25519 signature auth

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(feature = "alloc")]
use super::build_package::PackageSignature;
#[cfg(feature = "alloc")]
use crate::error::KernelError;

/// Package metadata stored in the repository index.
///
/// Distinct from `build_package::PackageMetadata` -- this adds
/// repository-specific fields (sha256_hash, upload_time).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct RepoPackageMeta {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub description: String,
    pub size: u64,
    pub sha256_hash: [u8; 32],
    pub dependencies: Vec<String>,
    pub upload_time: u64,
}

#[cfg(feature = "alloc")]
impl RepoPackageMeta {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            architecture: String::from("x86_64"),
            description: String::new(),
            size: 0,
            sha256_hash: [0u8; 32],
            dependencies: Vec::new(),
            upload_time: 0,
        }
    }

    /// Format metadata as a JSON-like key-value string.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        out.push_str("\"name\":\"");
        out.push_str(&self.name);
        out.push_str("\",\"version\":\"");
        out.push_str(&self.version);
        out.push_str("\",\"arch\":\"");
        out.push_str(&self.architecture);
        out.push_str("\",\"description\":\"");
        out.push_str(&self.description);
        out.push_str("\",\"size\":");
        push_u64(&mut out, self.size);
        out.push_str(",\"sha256\":\"");
        for b in &self.sha256_hash {
            push_hex_byte(&mut out, *b);
        }
        out.push_str("\",\"deps\":[");
        for (i, dep) in self.dependencies.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(dep);
            out.push('"');
        }
        out.push_str("],\"upload_time\":");
        push_u64(&mut out, self.upload_time);
        out.push('}');
        out
    }
}

/// Repository package index.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct RepoIndex {
    /// Packages keyed by name, each with a list of version entries.
    pub packages: BTreeMap<String, Vec<RepoPackageMeta>>,
    /// Tick count of last index update.
    pub last_updated: u64,
}

#[cfg(feature = "alloc")]
impl Default for RepoIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl RepoIndex {
    pub fn new() -> Self {
        Self {
            packages: BTreeMap::new(),
            last_updated: 0,
        }
    }

    /// Total number of unique package names.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Total number of individual package versions.
    pub fn version_count(&self) -> usize {
        self.packages.values().map(|v| v.len()).sum()
    }
}

/// Repository configuration.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub listen_port: u16,
    pub storage_path: String,
    /// Maximum package upload size in bytes (default 256 MB).
    pub max_package_size: u64,
    /// Whether uploads must include a valid Ed25519 signature.
    pub require_signatures: bool,
}

#[cfg(feature = "alloc")]
impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            listen_port: 8080,
            storage_path: String::from("/var/repo/packages"),
            max_package_size: 256 * 1024 * 1024,
            require_signatures: true,
        }
    }
}

/// HTTP request method (subset relevant to the repo API).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

/// Parsed HTTP request for the repository API.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct RepoRequest {
    pub method: HttpMethod,
    pub path: String,
    pub body: Vec<u8>,
}

/// HTTP response from the repository.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct RepoResponse {
    pub status: u16,
    pub body: String,
}

#[cfg(feature = "alloc")]
impl RepoResponse {
    pub fn ok(body: String) -> Self {
        Self { status: 200, body }
    }

    pub fn not_found(msg: &str) -> Self {
        Self {
            status: 404,
            body: msg.to_string(),
        }
    }

    pub fn bad_request(msg: &str) -> Self {
        Self {
            status: 400,
            body: msg.to_string(),
        }
    }

    pub fn forbidden(msg: &str) -> Self {
        Self {
            status: 403,
            body: msg.to_string(),
        }
    }
}

/// Package repository server.
///
/// Manages the package index, handles uploads with optional signature
/// verification, and provides search/listing capabilities.
#[cfg(feature = "alloc")]
pub struct RepoServer {
    pub config: RepoConfig,
    pub index: RepoIndex,
    /// Raw package data keyed by "name-version".
    packages: BTreeMap<String, Vec<u8>>,
}

#[cfg(feature = "alloc")]
impl RepoServer {
    /// Initialize a new repository server with default configuration.
    pub fn init() -> Self {
        Self::with_config(RepoConfig::default())
    }

    /// Initialize with custom configuration.
    pub fn with_config(config: RepoConfig) -> Self {
        Self {
            config,
            index: RepoIndex::new(),
            packages: BTreeMap::new(),
        }
    }

    /// Register a package in the index.
    ///
    /// If `require_signatures` is enabled, the caller must provide a valid
    /// signature. The package data is stored in the internal map.
    pub fn add_package(
        &mut self,
        meta: RepoPackageMeta,
        data: Vec<u8>,
        signature: Option<&PackageSignature>,
    ) -> Result<(), KernelError> {
        // Size check
        if data.len() as u64 > self.config.max_package_size {
            return Err(KernelError::InvalidArgument {
                name: "package_data",
                value: "exceeds max_package_size",
            });
        }

        // Signature verification
        if self.config.require_signatures {
            if let Some(sig) = signature {
                if !self.verify_upload(sig, &data) {
                    return Err(KernelError::PermissionDenied {
                        operation: "package_upload",
                    });
                }
            } else {
                return Err(KernelError::PermissionDenied {
                    operation: "package_upload",
                });
            }
        }

        let key = package_key(&meta.name, &meta.version);
        self.packages.insert(key, data);

        let entry = self
            .index
            .packages
            .entry(meta.name.clone())
            .or_insert_with(Vec::new);

        // Replace existing version or append
        if let Some(pos) = entry.iter().position(|e| e.version == meta.version) {
            entry[pos] = meta;
        } else {
            entry.push(meta);
        }

        self.index.last_updated += 1;
        Ok(())
    }

    /// Remove a package (specific version) from the index and storage.
    pub fn remove_package(&mut self, name: &str, version: &str) -> Result<(), KernelError> {
        let key = package_key(name, version);
        self.packages.remove(&key);

        if let Some(versions) = self.index.packages.get_mut(name) {
            versions.retain(|v| v.version != version);
            if versions.is_empty() {
                self.index.packages.remove(name);
            }
            self.index.last_updated += 1;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "package",
                id: 0,
            })
        }
    }

    /// Search packages by substring match on name.
    pub fn search(&self, pattern: &str) -> Vec<&RepoPackageMeta> {
        let mut results = Vec::new();
        for versions in self.index.packages.values() {
            for meta in versions {
                if meta.name.contains(pattern) {
                    results.push(meta);
                }
            }
        }
        results
    }

    /// Get metadata for a specific package name and version.
    pub fn get_package_info(&self, name: &str, version: &str) -> Option<&RepoPackageMeta> {
        self.index
            .packages
            .get(name)?
            .iter()
            .find(|m| m.version == version)
    }

    /// List all versions for a given package name.
    pub fn list_versions(&self, name: &str) -> Option<&Vec<RepoPackageMeta>> {
        self.index.packages.get(name)
    }

    /// Paginated listing of all packages.
    ///
    /// Returns at most `page_size` entries starting from `offset`.
    pub fn list_packages(&self, offset: usize, page_size: usize) -> Vec<&RepoPackageMeta> {
        let mut all: Vec<&RepoPackageMeta> = Vec::new();
        for versions in self.index.packages.values() {
            for meta in versions {
                all.push(meta);
            }
        }
        // Apply pagination
        all.into_iter().skip(offset).take(page_size).collect()
    }

    /// Generate a JSON-like index string for HTTP serving.
    pub fn generate_index_json(&self) -> String {
        let mut out = String::from("{\"packages\":[");
        let mut first = true;
        for versions in self.index.packages.values() {
            for meta in versions {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&meta.to_json());
            }
        }
        out.push_str("],\"last_updated\":");
        push_u64(&mut out, self.index.last_updated);
        out.push('}');
        out
    }

    /// Verify an upload signature using Ed25519 marker check.
    pub fn verify_upload(&self, signature: &PackageSignature, archive_data: &[u8]) -> bool {
        // Delegate to PackageSignature's verification with a placeholder key
        let public_key = [0u8; 32];
        signature.verify(archive_data, &public_key)
    }

    /// Route an HTTP request to the appropriate handler.
    pub fn handle_request(&self, req: &RepoRequest) -> RepoResponse {
        match req.method {
            HttpMethod::Get => self.handle_get(&req.path),
            HttpMethod::Post => {
                // POST /api/packages -- upload not fully implemented here
                RepoResponse::bad_request("upload requires multipart body parsing")
            }
        }
    }

    fn handle_get(&self, path: &str) -> RepoResponse {
        // GET /api/packages
        if path == "/api/packages" {
            return RepoResponse::ok(self.generate_index_json());
        }

        // GET /api/packages/{name}
        // GET /api/packages/{name}/{version}
        if let Some(rest) = path.strip_prefix("/api/packages/") {
            let parts: Vec<&str> = rest.splitn(2, '/').collect();
            match parts.len() {
                1 => {
                    let name = parts[0];
                    if let Some(versions) = self.list_versions(name) {
                        let mut out = String::from("[");
                        for (i, meta) in versions.iter().enumerate() {
                            if i > 0 {
                                out.push(',');
                            }
                            out.push_str(&meta.to_json());
                        }
                        out.push(']');
                        RepoResponse::ok(out)
                    } else {
                        RepoResponse::not_found("package not found")
                    }
                }
                2 => {
                    let (name, version) = (parts[0], parts[1]);
                    if let Some(meta) = self.get_package_info(name, version) {
                        RepoResponse::ok(meta.to_json())
                    } else {
                        RepoResponse::not_found("version not found")
                    }
                }
                _ => RepoResponse::not_found("invalid path"),
            }
        } else {
            RepoResponse::not_found("unknown endpoint")
        }
    }

    /// Get the raw package data for download.
    pub fn get_package_data(&self, name: &str, version: &str) -> Option<&Vec<u8>> {
        let key = package_key(name, version);
        self.packages.get(&key)
    }

    /// Total stored package data in bytes.
    pub fn total_storage_bytes(&self) -> u64 {
        self.packages.values().map(|d| d.len() as u64).sum()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a storage key from package name and version.
#[cfg(feature = "alloc")]
fn package_key(name: &str, version: &str) -> String {
    let mut key = String::from(name);
    key.push('-');
    key.push_str(version);
    key
}

/// Append a u64 as decimal digits to a string (no formatting crate needed).
#[cfg(feature = "alloc")]
fn push_u64(out: &mut String, mut val: u64) {
    if val == 0 {
        out.push('0');
        return;
    }
    let start = out.len();
    while val > 0 {
        let digit = (val % 10) as u8 + b'0';
        out.push(digit as char);
        val /= 10;
    }
    // Reverse the digits we just pushed
    let bytes = unsafe { out.as_bytes_mut() };
    bytes[start..].reverse();
}

/// Append a single byte as two hex characters.
#[cfg(feature = "alloc")]
fn push_hex_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(HEX[(byte >> 4) as usize] as char);
    out.push(HEX[(byte & 0x0F) as usize] as char);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_meta(name: &str, version: &str) -> RepoPackageMeta {
        let mut m = RepoPackageMeta::new(name, version);
        m.description = String::from("test package");
        m.size = 1024;
        m
    }

    fn signed_sig() -> PackageSignature {
        let mut sig = PackageSignature::new("test-signer");
        sig.sign(&[], &[0u8; 32]);
        sig
    }

    #[test]
    fn test_repo_server_init() {
        let server = RepoServer::init();
        assert_eq!(server.config.listen_port, 8080);
        assert_eq!(server.index.package_count(), 0);
    }

    #[test]
    fn test_add_package_with_signature() {
        let mut server = RepoServer::init();
        let meta = make_meta("hello", "1.0.0");
        let sig = signed_sig();
        assert!(server.add_package(meta, vec![1, 2, 3], Some(&sig)).is_ok());
        assert_eq!(server.index.package_count(), 1);
    }

    #[test]
    fn test_add_package_no_sig_rejected() {
        let mut server = RepoServer::init();
        let meta = make_meta("hello", "1.0.0");
        assert!(server.add_package(meta, vec![1, 2, 3], None).is_err());
    }

    #[test]
    fn test_add_package_no_sig_allowed() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        let meta = make_meta("hello", "1.0.0");
        assert!(server.add_package(meta, vec![1, 2, 3], None).is_ok());
    }

    #[test]
    fn test_remove_package() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        let meta = make_meta("hello", "1.0.0");
        server.add_package(meta, vec![1], None).unwrap();
        assert!(server.remove_package("hello", "1.0.0").is_ok());
        assert_eq!(server.index.package_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent() {
        let server_config = RepoConfig {
            require_signatures: false,
            ..RepoConfig::default()
        };
        let mut server = RepoServer::with_config(server_config);
        assert!(server.remove_package("nope", "1.0.0").is_err());
    }

    #[test]
    fn test_search_packages() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("libfoo", "1.0.0"), vec![1], None)
            .unwrap();
        server
            .add_package(make_meta("libbar", "2.0.0"), vec![2], None)
            .unwrap();
        server
            .add_package(make_meta("hello", "1.0.0"), vec![3], None)
            .unwrap();

        let results = server.search("lib");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_get_package_info() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("hello", "1.0.0"), vec![1], None)
            .unwrap();
        let info = server.get_package_info("hello", "1.0.0");
        assert!(info.is_some());
        assert_eq!(info.unwrap().size, 1024);
    }

    #[test]
    fn test_list_packages_paginated() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        for i in 0..5 {
            let name = alloc::format!("pkg{}", i);
            server
                .add_package(make_meta(&name, "1.0.0"), vec![1], None)
                .unwrap();
        }
        let page1 = server.list_packages(0, 3);
        assert_eq!(page1.len(), 3);
        let page2 = server.list_packages(3, 3);
        assert_eq!(page2.len(), 2);
    }

    #[test]
    fn test_generate_index_json() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("hello", "1.0.0"), vec![1], None)
            .unwrap();
        let json = server.generate_index_json();
        assert!(json.contains("\"name\":\"hello\""));
        assert!(json.contains("\"version\":\"1.0.0\""));
    }

    #[test]
    fn test_handle_get_packages() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("hello", "1.0.0"), vec![1], None)
            .unwrap();

        let req = RepoRequest {
            method: HttpMethod::Get,
            path: String::from("/api/packages"),
            body: Vec::new(),
        };
        let resp = server.handle_request(&req);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("hello"));
    }

    #[test]
    fn test_handle_get_package_versions() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("hello", "1.0.0"), vec![1], None)
            .unwrap();

        let req = RepoRequest {
            method: HttpMethod::Get,
            path: String::from("/api/packages/hello"),
            body: Vec::new(),
        };
        let resp = server.handle_request(&req);
        assert_eq!(resp.status, 200);

        let req_missing = RepoRequest {
            method: HttpMethod::Get,
            path: String::from("/api/packages/nonexistent"),
            body: Vec::new(),
        };
        let resp_missing = server.handle_request(&req_missing);
        assert_eq!(resp_missing.status, 404);
    }

    #[test]
    fn test_handle_get_specific_version() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("hello", "2.0.0"), vec![1], None)
            .unwrap();

        let req = RepoRequest {
            method: HttpMethod::Get,
            path: String::from("/api/packages/hello/2.0.0"),
            body: Vec::new(),
        };
        let resp = server.handle_request(&req);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("2.0.0"));
    }

    #[test]
    fn test_total_storage_bytes() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        server
            .add_package(make_meta("a", "1.0.0"), vec![0; 100], None)
            .unwrap();
        server
            .add_package(make_meta("b", "1.0.0"), vec![0; 200], None)
            .unwrap();
        assert_eq!(server.total_storage_bytes(), 300);
    }

    #[test]
    fn test_replace_existing_version() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        let mut server = RepoServer::with_config(config);
        let mut m1 = make_meta("hello", "1.0.0");
        m1.size = 100;
        server.add_package(m1, vec![1], None).unwrap();

        let mut m2 = make_meta("hello", "1.0.0");
        m2.size = 200;
        server.add_package(m2, vec![2], None).unwrap();

        // Should have replaced, not duplicated
        assert_eq!(server.index.version_count(), 1);
        let info = server.get_package_info("hello", "1.0.0").unwrap();
        assert_eq!(info.size, 200);
    }

    #[test]
    fn test_package_too_large() {
        let mut config = RepoConfig::default();
        config.require_signatures = false;
        config.max_package_size = 10;
        let mut server = RepoServer::with_config(config);
        let meta = make_meta("big", "1.0.0");
        assert!(server.add_package(meta, vec![0; 100], None).is_err());
    }

    #[test]
    fn test_repo_meta_to_json() {
        let mut m = RepoPackageMeta::new("test", "1.0.0");
        m.dependencies.push(String::from("libfoo"));
        let json = m.to_json();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"deps\":[\"libfoo\"]"));
    }
}
