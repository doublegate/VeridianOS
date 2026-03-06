//! Package Repository Management
//!
//! Implements HTTP-based package repository fetching for VeridianOS package
//! manager. Uses the network stack for actual HTTP communication.

// Phase 4 (package ecosystem) -- repository fetching is defined but not yet
// wired to the network stack.

use alloc::{string::String, vec::Vec};

use super::{Dependency, PackageId, PackageMetadata, Version};

/// HTTP request type
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Network API stub -- used when network stack is available
enum HttpMethod {
    Get,
    Head,
}

/// HTTP response
#[derive(Debug)]
pub struct HttpResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new() -> Self {
        Self {
            status_code: 0,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }
}

/// HTTP client for repository communication
pub struct HttpClient {
    /// Base URL for the repository
    base_url: String,
    /// Connection timeout in milliseconds
    timeout_ms: u32,
    /// User agent string
    user_agent: String,
}

impl HttpClient {
    fn new(base_url: String) -> Self {
        Self {
            base_url,
            timeout_ms: 30000, // 30 second default timeout
            user_agent: String::from("VeridianOS-PackageManager/1.0"),
        }
    }

    /// Perform HTTP GET request
    fn get(&self, path: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpMethod::Get, path, None)
    }

    /// Perform HTTP request
    fn request(
        &self,
        method: HttpMethod,
        path: &str,
        _body: Option<&[u8]>,
    ) -> Result<HttpResponse, HttpError> {
        // Build full URL
        let url = self.build_url(path);

        // Parse URL components
        let (host, port, request_path) = self.parse_url(&url)?;

        // Build HTTP request
        let request = self.build_request(method, &host, &request_path);

        // Connect and send request via network stack
        let response_data = self.send_request(&host, port, &request)?;

        // Parse HTTP response
        self.parse_response(&response_data)
    }

    fn build_url(&self, path: &str) -> String {
        let mut url = self.base_url.clone();
        if !url.ends_with('/') && !path.starts_with('/') {
            url.push('/');
        }
        url.push_str(path);
        url
    }

    fn parse_url(&self, url: &str) -> Result<(String, u16, String), HttpError> {
        // Strip protocol prefix
        let url = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or(url);

        // Split host and path
        let (host_port, path) = match url.find('/') {
            Some(idx) => (&url[..idx], &url[idx..]),
            None => (url, "/"),
        };

        // Parse host and port
        let (host, port) = match host_port.find(':') {
            Some(idx) => {
                let port_str = &host_port[idx + 1..];
                let port = port_str.parse::<u16>().unwrap_or(443);
                (&host_port[..idx], port)
            }
            None => {
                // Default to HTTPS port for security
                (host_port, 443)
            }
        };

        Ok((String::from(host), port, String::from(path)))
    }

    fn build_request(&self, method: HttpMethod, host: &str, path: &str) -> Vec<u8> {
        let method_str = match method {
            HttpMethod::Get => "GET",
            HttpMethod::Head => "HEAD",
        };

        let request = alloc::format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: application/octet-stream, \
             application/json, */*\r\nConnection: close\r\n\r\n",
            method_str,
            path,
            host,
            self.user_agent
        );

        request.into_bytes()
    }

    fn send_request(&self, host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, HttpError> {
        // Use network stack to establish TCP connection
        use crate::net::{
            socket::{Socket, SocketDomain, SocketProtocol, SocketType},
            Ipv4Address, SocketAddr,
        };

        // Resolve hostname to IP (simplified - in production would use DNS)
        let ip_addr = self.resolve_hostname(host)?;

        // Create TCP socket
        let mut socket = Socket::new(SocketDomain::Inet, SocketType::Stream, SocketProtocol::Tcp)
            .map_err(|_| HttpError::ConnectionFailed)?;

        // Connect to server
        let addr = SocketAddr::v4(Ipv4Address(ip_addr), port);
        socket
            .connect(addr)
            .map_err(|_| HttpError::ConnectionFailed)?;

        // Set receive timeout via socket options
        socket.options.recv_timeout_ms = Some(self.timeout_ms as u64);

        // Send HTTP request
        socket
            .send(request, 0)
            .map_err(|_| HttpError::NetworkError)?;

        // Receive response
        let mut response = Vec::new();
        let mut buf = [0u8; 4096];

        loop {
            match socket.recv(&mut buf, 0) {
                Ok(0) => break, // Connection closed
                Ok(n) => response.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }

            // Safety limit - max 64MB response
            if response.len() > 64 * 1024 * 1024 {
                return Err(HttpError::ResponseTooLarge);
            }
        }

        // Close socket
        let _ = socket.close();

        Ok(response)
    }

    fn resolve_hostname(&self, host: &str) -> Result<[u8; 4], HttpError> {
        // Simple hostname resolution
        // In production, would use DNS resolver

        // Check for IP address format first
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() == 4 {
            let mut ip = [0u8; 4];
            let mut valid = true;
            for (i, part) in parts.iter().enumerate() {
                match part.parse::<u8>() {
                    Ok(byte) => ip[i] = byte,
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            if valid {
                return Ok(ip);
            }
        }

        // For now, return localhost for unresolved hostnames
        // Real implementation would use DNS
        Ok([127, 0, 0, 1])
    }

    fn parse_response(&self, data: &[u8]) -> Result<HttpResponse, HttpError> {
        let mut response = HttpResponse::new();

        // Find header/body separator
        let header_end = self
            .find_header_end(data)
            .ok_or(HttpError::InvalidResponse)?;
        let header_data = &data[..header_end];
        let body_start = header_end + 4; // Skip \r\n\r\n

        // Parse status line
        let header_str =
            core::str::from_utf8(header_data).map_err(|_| HttpError::InvalidResponse)?;
        let mut lines = header_str.lines();

        let status_line = lines.next().ok_or(HttpError::InvalidResponse)?;
        let parts: Vec<&str> = status_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(HttpError::InvalidResponse);
        }
        response.status_code = parts[1].parse().unwrap_or(0);

        // Parse headers
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                response
                    .headers
                    .push((String::from(key.trim()), String::from(value.trim())));
            }
        }

        // Extract body
        if body_start < data.len() {
            response.body = data[body_start..].to_vec();
        }

        Ok(response)
    }

    fn find_header_end(&self, data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(3) {
            if &data[i..i + 4] == b"\r\n\r\n" {
                return Some(i);
            }
        }
        None
    }
}

/// HTTP errors
#[derive(Debug)]
pub enum HttpError {
    ConnectionFailed,
    InvalidResponse,
    ResponseTooLarge,
    Timeout,
    NetworkError,
}

/// Package repository
#[derive(Debug, Clone)]
pub struct Repository {
    /// Repository name
    pub name: String,
    /// Repository URL
    pub url: String,
    /// Is repository trusted
    pub trusted: bool,
    /// Package index cache
    package_cache: Vec<PackageMetadata>,
    /// Last update timestamp (Unix time)
    last_updated: u64,
}

impl Repository {
    pub fn new(name: String, url: String, trusted: bool) -> Self {
        Self {
            name,
            url,
            trusted,
            package_cache: Vec::new(),
            last_updated: 0,
        }
    }

    /// Fetch package list from repository via HTTP/HTTPS
    pub fn fetch_package_list(&self) -> Vec<PackageMetadata> {
        let client = HttpClient::new(self.url.clone());

        // Fetch package index file
        match client.get("packages.json") {
            Ok(response) if response.is_success() => self.parse_package_index(&response.body),
            Ok(_response) => {
                // HTTP error - return cached packages if available
                crate::println!("[PKG] Repository {} returned error, using cache", self.name);
                self.package_cache.clone()
            }
            Err(_e) => {
                // Network error - return cached packages
                crate::println!("[PKG] Failed to connect to repository {}", self.name);
                self.package_cache.clone()
            }
        }
    }

    /// Parse package index JSON
    fn parse_package_index(&self, data: &[u8]) -> Vec<PackageMetadata> {
        let mut packages = Vec::new();

        // Parse JSON package list (simplified JSON parser)
        let json_str = match core::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => return packages,
        };

        // Simple JSON parser for package array
        // Format: [{"name":"pkg1","version":"1.0.0",...},...]
        if !json_str.starts_with('[') {
            return packages;
        }

        // Split into package objects
        let mut depth = 0;
        let mut obj_start = 0;
        let chars: Vec<char> = json_str.chars().collect();

        for (i, &c) in chars.iter().enumerate() {
            match c {
                '{' => {
                    if depth == 1 {
                        obj_start = i;
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 1 {
                        let obj_str: String = chars[obj_start..=i].iter().collect();
                        if let Some(pkg) = self.parse_package_object(&obj_str) {
                            packages.push(pkg);
                        }
                    }
                }
                _ => {}
            }
        }

        packages
    }

    /// Parse single package JSON object
    fn parse_package_object(&self, json: &str) -> Option<PackageMetadata> {
        // Extract fields from JSON object
        let name = self.extract_json_string(json, "name")?;
        let version_str = self.extract_json_string(json, "version")?;
        let author = self.extract_json_string(json, "author").unwrap_or_default();
        let description = self
            .extract_json_string(json, "description")
            .unwrap_or_default();
        let license = self
            .extract_json_string(json, "license")
            .unwrap_or_default();

        // Parse version
        let version = self.parse_version(&version_str)?;

        // Parse dependencies array
        let dependencies = self.extract_dependencies(json);
        let conflicts = self.extract_conflicts(json);

        Some(PackageMetadata {
            name,
            version,
            author,
            description,
            license,
            dependencies,
            conflicts,
        })
    }

    fn extract_json_string(&self, json: &str, key: &str) -> Option<String> {
        let pattern = alloc::format!("\"{}\":\"", key);
        let start = json.find(&pattern)? + pattern.len();
        let end = json[start..].find('"')? + start;
        Some(String::from(&json[start..end]))
    }

    fn parse_version(&self, version_str: &str) -> Option<Version> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() >= 3 {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            let patch = parts[2].parse().ok()?;
            Some(Version::new(major, minor, patch))
        } else {
            None
        }
    }

    fn extract_dependencies(&self, json: &str) -> Vec<Dependency> {
        let mut deps = Vec::new();

        // Find dependencies array
        if let Some(start) = json.find("\"dependencies\":[") {
            let arr_start = start + "\"dependencies\":[".len();
            if let Some(arr_end) = json[arr_start..].find(']') {
                let arr_str = &json[arr_start..arr_start + arr_end];

                // Parse each dependency object
                for dep_obj in arr_str.split("},{") {
                    let dep_str = dep_obj.trim_matches(|c| c == '{' || c == '}');
                    if let (Some(name), Some(version)) = (
                        self.extract_json_string(dep_str, "name"),
                        self.extract_json_string(dep_str, "version"),
                    ) {
                        deps.push(Dependency {
                            name,
                            version_req: version,
                        });
                    }
                }
            }
        }

        deps
    }

    fn extract_conflicts(&self, json: &str) -> Vec<String> {
        let mut conflicts = Vec::new();

        // Find conflicts array
        if let Some(start) = json.find("\"conflicts\":[") {
            let arr_start = start + "\"conflicts\":[".len();
            if let Some(arr_end) = json[arr_start..].find(']') {
                let arr_str = &json[arr_start..arr_start + arr_end];

                // Parse each conflict string
                for conflict in arr_str.split(',') {
                    let name = conflict.trim().trim_matches('"');
                    if !name.is_empty() {
                        conflicts.push(String::from(name));
                    }
                }
            }
        }

        conflicts
    }

    /// Download package by ID via HTTP/HTTPS
    pub fn download_package(&self, package_id: &PackageId) -> Option<Vec<u8>> {
        let client = HttpClient::new(self.url.clone());

        // Construct package download path
        let path = alloc::format!("packages/{}.vpkg", package_id);

        match client.get(&path) {
            Ok(response) if response.is_success() => {
                crate::println!(
                    "[PKG] Downloaded {} ({} bytes)",
                    package_id,
                    response.body.len()
                );
                Some(response.body)
            }
            Ok(_response) => {
                #[cfg(target_arch = "x86_64")]
                crate::println!(
                    "[PKG] Failed to download {}: HTTP {}",
                    package_id,
                    _response.status_code
                );
                None
            }
            Err(_e) => {
                crate::println!("[PKG] Network error downloading {}", package_id);
                None
            }
        }
    }

    /// Check if package exists in repository
    pub fn has_package(&self, package_id: &PackageId) -> bool {
        self.package_cache.iter().any(|p| &p.name == package_id)
    }

    /// Get cached package metadata
    pub fn get_package(&self, package_id: &PackageId) -> Option<&PackageMetadata> {
        self.package_cache.iter().find(|p| &p.name == package_id)
    }

    /// Invalidate cache
    pub fn invalidate_cache(&mut self) {
        self.package_cache.clear();
        self.last_updated = 0;
    }

    /// Check if cache is stale (older than 1 hour)
    pub fn is_cache_stale(&self, current_time: u64) -> bool {
        current_time.saturating_sub(self.last_updated) > 3600
    }
}

impl Default for Repository {
    fn default() -> Self {
        Self::new(
            String::from("default"),
            String::from("https://packages.veridian.org"),
            true,
        )
    }
}

// ============================================================================
// Repository Index
// ============================================================================

/// Server-side repository metadata index.
///
/// Describes all packages available in a repository, signed for integrity.
/// Serialized as a simple JSON-like format for transmission.
pub struct RepositoryIndex {
    /// Index format version
    pub version: u32,
    /// Timestamp when the index was generated (seconds since epoch)
    pub generated_at: u64,
    /// Package entries in the index
    pub entries: Vec<RepositoryIndexEntry>,
    /// Ed25519 signature over the serialized entries
    pub signature: Vec<u8>,
}

/// A single entry in the repository index.
#[derive(Debug, Clone)]
pub struct RepositoryIndexEntry {
    /// Package name
    pub name: String,
    /// Package version string
    pub version: String,
    /// SHA-256 hash of the .vpkg file
    pub hash: [u8; 32],
    /// Size of the .vpkg file in bytes
    pub size: u64,
    /// Package description
    pub description: String,
    /// License identifier
    pub license: String,
}

impl RepositoryIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            version: 1,
            generated_at: crate::arch::timer::get_timestamp_secs(),
            entries: Vec::new(),
            signature: Vec::new(),
        }
    }

    /// Generate a repository index from a list of package metadata.
    pub fn generate(packages: &[super::PackageMetadata]) -> Self {
        let mut index = Self::new();
        for pkg in packages {
            index.entries.push(RepositoryIndexEntry {
                name: pkg.name.clone(),
                version: alloc::format!(
                    "{}.{}.{}",
                    pkg.version.major,
                    pkg.version.minor,
                    pkg.version.patch
                ),
                hash: [0u8; 32], // Hash populated when package file is available
                size: 0,
                description: pkg.description.clone(),
                license: pkg.license.clone(),
            });
        }
        index
    }

    /// Serialize the index to bytes (simple JSON format).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"{\"version\":");
        let version_str = alloc::format!("{}", self.version);
        buf.extend_from_slice(version_str.as_bytes());
        buf.extend_from_slice(b",\"generated_at\":");
        let ts_str = alloc::format!("{}", self.generated_at);
        buf.extend_from_slice(ts_str.as_bytes());
        buf.extend_from_slice(b",\"packages\":[");
        for (i, entry) in self.entries.iter().enumerate() {
            if i > 0 {
                buf.push(b',');
            }
            let entry_json = alloc::format!(
                "{{\"name\":\"{}\",\"version\":\"{}\",\"size\":{},\"description\":\"{}\",\"\
                 license\":\"{}\"}}",
                entry.name,
                entry.version,
                entry.size,
                entry.description,
                entry.license
            );
            buf.extend_from_slice(entry_json.as_bytes());
        }
        buf.extend_from_slice(b"]}");
        buf
    }

    /// Verify the Ed25519 signature over the index data.
    ///
    /// Uses `crate::crypto::asymmetric::Ed25519` for real verification.
    pub fn verify_signature(&self, public_key: &[u8]) -> bool {
        if self.signature.is_empty() || public_key.is_empty() {
            return false;
        }

        let content = self.to_bytes();

        // Use real Ed25519 verification
        let sig = match crate::crypto::asymmetric::Signature::from_bytes(&self.signature) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let vk = match crate::crypto::asymmetric::VerifyingKey::from_bytes(public_key) {
            Ok(v) => v,
            Err(_) => return false,
        };

        matches!(vk.verify(&content, &sig), Ok(true))
    }
}

impl Default for RepositoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Mirror Management
// ============================================================================

/// Status of a mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorStatus {
    /// Mirror is available and responding
    Online,
    /// Mirror is not responding
    Offline,
    /// Mirror status is unknown (not yet checked)
    Unknown,
}

/// Metadata about a repository mirror.
#[derive(Debug, Clone)]
pub struct MirrorMetadata {
    /// Mirror URL
    pub url: String,
    /// Priority (lower = preferred)
    pub priority: u32,
    /// Geographic region hint
    pub region: String,
    /// Timestamp of last successful sync
    pub last_sync: u64,
    /// Current mirror status
    pub status: MirrorStatus,
}

/// Manages multiple mirrors for a repository, providing failover.
pub struct MirrorManager {
    /// Available mirrors sorted by priority
    mirrors: Vec<MirrorMetadata>,
}

impl MirrorManager {
    /// Create a new mirror manager.
    pub fn new() -> Self {
        Self {
            mirrors: Vec::new(),
        }
    }

    /// Add a mirror to the manager.
    pub fn add_mirror(&mut self, mirror: MirrorMetadata) {
        self.mirrors.push(mirror);
        // Keep sorted by priority (lower first)
        self.mirrors.sort_by_key(|m| m.priority);
    }

    /// Remove a mirror by URL.
    pub fn remove_mirror(&mut self, url: &str) -> bool {
        if let Some(pos) = self.mirrors.iter().position(|m| m.url == url) {
            self.mirrors.remove(pos);
            true
        } else {
            false
        }
    }

    /// Select the best available mirror.
    ///
    /// Returns the highest-priority mirror that is not offline.
    /// Falls back to the first mirror if all are offline.
    pub fn select_best_mirror(&self) -> Option<&MirrorMetadata> {
        // Prefer online mirrors by priority, fall back to any mirror
        self.mirrors
            .iter()
            .find(|m| m.status != MirrorStatus::Offline)
            .or(self.mirrors.first())
    }

    /// Mark a mirror as offline after a failed connection.
    pub fn mark_offline(&mut self, url: &str) {
        if let Some(mirror) = self.mirrors.iter_mut().find(|m| m.url == url) {
            mirror.status = MirrorStatus::Offline;
        }
    }

    /// Mark a mirror as online after a successful connection.
    pub fn mark_online(&mut self, url: &str) {
        if let Some(mirror) = self.mirrors.iter_mut().find(|m| m.url == url) {
            mirror.status = MirrorStatus::Online;
            mirror.last_sync = crate::arch::timer::get_timestamp_secs();
        }
    }

    /// List all mirrors.
    pub fn list_mirrors(&self) -> &[MirrorMetadata] {
        &self.mirrors
    }

    /// Return the number of mirrors.
    pub fn mirror_count(&self) -> usize {
        self.mirrors.len()
    }
}

impl Default for MirrorManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Repository Configuration
// ============================================================================

/// Configuration for multi-repository management.
pub struct RepositoryConfig {
    /// Configured repositories
    repositories: Vec<RepositoryEntry>,
}

/// A single repository entry in the configuration.
#[derive(Debug, Clone)]
pub struct RepositoryEntry {
    /// Repository name
    pub name: String,
    /// Repository URL
    pub url: String,
    /// Whether this repository is enabled
    pub enabled: bool,
    /// Whether this repository is trusted
    pub trusted: bool,
    /// Priority (lower = checked first)
    pub priority: u32,
    /// Mirror manager for this repository
    pub mirrors: Vec<MirrorMetadata>,
}

impl RepositoryConfig {
    /// Create a new empty configuration.
    pub fn new() -> Self {
        Self {
            repositories: Vec::new(),
        }
    }

    /// Add a repository.
    pub fn add_repository(&mut self, entry: RepositoryEntry) {
        self.repositories.push(entry);
        self.repositories.sort_by_key(|r| r.priority);
    }

    /// Remove a repository by name.
    pub fn remove_repository(&mut self, name: &str) -> bool {
        if let Some(pos) = self.repositories.iter().position(|r| r.name == name) {
            self.repositories.remove(pos);
            true
        } else {
            false
        }
    }

    /// Enable a repository.
    pub fn enable_repository(&mut self, name: &str) -> bool {
        if let Some(repo) = self.repositories.iter_mut().find(|r| r.name == name) {
            repo.enabled = true;
            true
        } else {
            false
        }
    }

    /// Disable a repository.
    pub fn disable_repository(&mut self, name: &str) -> bool {
        if let Some(repo) = self.repositories.iter_mut().find(|r| r.name == name) {
            repo.enabled = false;
            true
        } else {
            false
        }
    }

    /// List all enabled repositories.
    pub fn enabled_repositories(&self) -> Vec<&RepositoryEntry> {
        self.repositories.iter().filter(|r| r.enabled).collect()
    }

    /// List all repositories.
    pub fn all_repositories(&self) -> &[RepositoryEntry] {
        &self.repositories
    }

    /// Get a repository by name.
    pub fn get_repository(&self, name: &str) -> Option<&RepositoryEntry> {
        self.repositories.iter().find(|r| r.name == name)
    }
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Repository Access Control
// ============================================================================

/// Policy governing who may upload packages to a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadPolicy {
    /// Anyone can upload packages.
    Open,
    /// Only uploaders whose Ed25519 public key fingerprint is in the
    /// allowed list may upload.
    Restricted,
    /// No uploads are accepted.
    Closed,
}

/// Controls which uploaders are permitted to push packages to a repository.
#[derive(Debug, Clone)]
pub struct AccessControl {
    /// SHA-256 fingerprints of allowed Ed25519 public keys.
    allowed_uploaders: Vec<[u8; 32]>,
    /// Current upload policy.
    pub upload_policy: UploadPolicy,
}

impl AccessControl {
    /// Create a new access control with the given policy.
    pub fn new(policy: UploadPolicy) -> Self {
        Self {
            allowed_uploaders: Vec::new(),
            upload_policy: policy,
        }
    }

    /// Register an uploader by their Ed25519 public key fingerprint.
    pub fn add_uploader(&mut self, key_fingerprint: [u8; 32]) {
        if !self.allowed_uploaders.contains(&key_fingerprint) {
            self.allowed_uploaders.push(key_fingerprint);
        }
    }

    /// Remove an uploader. Returns `true` if the fingerprint was present.
    pub fn remove_uploader(&mut self, key_fingerprint: &[u8; 32]) -> bool {
        if let Some(pos) = self
            .allowed_uploaders
            .iter()
            .position(|fp| fp == key_fingerprint)
        {
            self.allowed_uploaders.remove(pos);
            true
        } else {
            false
        }
    }

    /// Verify that an upload is authorized and properly signed.
    ///
    /// Checks the upload policy, uploader identity (for `Restricted`), and
    /// Ed25519 signature over the package data.
    pub fn verify_upload(
        &self,
        package_data: &[u8],
        signature: &[u8],
        uploader_key: &[u8],
    ) -> Result<(), crate::error::KernelError> {
        use crate::error::KernelError;

        // Policy gate
        match self.upload_policy {
            UploadPolicy::Open => { /* skip identity check */ }
            UploadPolicy::Closed => {
                return Err(KernelError::PermissionDenied {
                    operation: "upload package",
                });
            }
            UploadPolicy::Restricted => {
                let fingerprint = crate::crypto::hash::sha256(uploader_key);
                if !self.allowed_uploaders.contains(fingerprint.as_bytes()) {
                    return Err(KernelError::PermissionDenied {
                        operation: "upload package",
                    });
                }
            }
        }

        // Signature verification
        let sig = crate::crypto::asymmetric::Signature::from_bytes(signature).map_err(|_| {
            KernelError::PermissionDenied {
                operation: "upload package",
            }
        })?;
        let vk =
            crate::crypto::asymmetric::VerifyingKey::from_bytes(uploader_key).map_err(|_| {
                KernelError::PermissionDenied {
                    operation: "upload package",
                }
            })?;

        match vk.verify(package_data, &sig) {
            Ok(true) => Ok(()),
            _ => Err(KernelError::PermissionDenied {
                operation: "upload package",
            }),
        }
    }
}

impl Default for AccessControl {
    fn default() -> Self {
        Self::new(UploadPolicy::Restricted)
    }
}

// ============================================================================
// Package Security Scanning
// ============================================================================

/// Classification of a malware detection pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternType {
    /// File path pattern (e.g. accessing sensitive system files).
    SuspiciousPath,
    /// Requesting dangerous capabilities.
    ExcessiveCapability,
    /// File matches a known malware hash.
    KnownBadHash,
}

/// Severity level for security findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Low-risk finding.
    Low,
    /// Medium-risk finding.
    Medium,
    /// High-risk finding.
    High,
    /// Critical-risk finding.
    Critical,
}

/// A pattern used to detect suspicious content in a package.
#[derive(Debug, Clone)]
pub struct MalwarePattern {
    /// What kind of pattern this is.
    pub pattern_type: PatternType,
    /// Human-readable description.
    pub description: String,
    /// Severity if matched.
    pub severity: Severity,
    /// The actual pattern to match (path substring, capability name, or hex
    /// hash).
    pub pattern: String,
}

/// A security finding produced by the scanner.
#[derive(Debug, Clone)]
pub struct MalwareFinding {
    /// Severity of the finding.
    pub severity: Severity,
    /// Description of the issue.
    pub description: String,
    /// Which file triggered the match.
    pub file_path: String,
    /// The pattern that was matched.
    pub pattern_matched: String,
}

/// Scans packages for suspicious paths, excessive capabilities, and known-bad
/// hashes.
#[derive(Debug, Clone)]
pub struct SecurityScanner {
    /// Registered detection patterns.
    patterns: Vec<MalwarePattern>,
}

impl SecurityScanner {
    /// Create a new scanner pre-loaded with default suspicious patterns.
    pub fn new() -> Self {
        let mut scanner = Self {
            patterns: Vec::new(),
        };
        scanner.add_default_patterns();
        scanner
    }

    /// Populate the scanner with well-known suspicious patterns.
    pub fn add_default_patterns(&mut self) {
        // High-severity sensitive file paths
        let sensitive_paths = [
            "/etc/shadow",
            "/etc/passwd",
            "/dev/mem",
            "/dev/kmem",
            "/proc/kcore",
        ];
        for path in &sensitive_paths {
            self.patterns.push(MalwarePattern {
                pattern_type: PatternType::SuspiciousPath,
                description: alloc::format!("Access to sensitive path: {}", path),
                severity: Severity::High,
                pattern: String::from(*path),
            });
        }

        // Medium-severity capability requests
        let dangerous_caps = ["CAP_SYS_ADMIN", "CAP_NET_RAW", "CAP_SYS_PTRACE"];
        for cap in &dangerous_caps {
            self.patterns.push(MalwarePattern {
                pattern_type: PatternType::ExcessiveCapability,
                description: alloc::format!("Excessive capability request: {}", cap),
                severity: Severity::Medium,
                pattern: String::from(*cap),
            });
        }

        // Medium-severity permission patterns
        let perm_patterns = ["setuid", "world-writable"];
        for pat in &perm_patterns {
            self.patterns.push(MalwarePattern {
                pattern_type: PatternType::SuspiciousPath,
                description: alloc::format!("Suspicious permission pattern: {}", pat),
                severity: Severity::Medium,
                pattern: String::from(*pat),
            });
        }
    }

    /// Register an additional detection pattern.
    pub fn add_pattern(&mut self, pattern: MalwarePattern) {
        self.patterns.push(pattern);
    }

    /// Scan a list of file paths against `SuspiciousPath` patterns.
    pub fn scan_package_paths(&self, file_paths: &[&str]) -> Vec<MalwareFinding> {
        let mut findings = Vec::new();
        for path in file_paths {
            for pat in &self.patterns {
                if pat.pattern_type != PatternType::SuspiciousPath {
                    continue;
                }
                if path.contains(pat.pattern.as_str()) {
                    findings.push(MalwareFinding {
                        severity: pat.severity,
                        description: pat.description.clone(),
                        file_path: String::from(*path),
                        pattern_matched: pat.pattern.clone(),
                    });
                }
            }
        }
        findings
    }

    /// Scan requested capabilities against `ExcessiveCapability` patterns.
    pub fn scan_capabilities(&self, requested_caps: &[&str]) -> Vec<MalwareFinding> {
        let mut findings = Vec::new();
        for cap in requested_caps {
            for pat in &self.patterns {
                if pat.pattern_type != PatternType::ExcessiveCapability {
                    continue;
                }
                if *cap == pat.pattern.as_str() {
                    findings.push(MalwareFinding {
                        severity: pat.severity,
                        description: pat.description.clone(),
                        file_path: String::new(),
                        pattern_matched: pat.pattern.clone(),
                    });
                }
            }
        }
        findings
    }
}

impl Default for SecurityScanner {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Vulnerability Tracking
// ============================================================================

/// A vulnerability advisory for one or more packages.
#[derive(Debug, Clone)]
pub struct VulnerabilityAdvisory {
    /// CVE or advisory identifier (e.g. "CVE-2024-12345").
    pub id: String,
    /// Affected packages as `(package_name, affected_version_range)` pairs.
    pub affected_packages: Vec<(String, String)>,
    /// Severity of the vulnerability.
    pub severity: Severity,
    /// Human-readable description.
    pub description: String,
    /// Version that fixes the vulnerability, if known.
    pub fixed_version: Option<String>,
}

/// Database of known vulnerability advisories.
#[derive(Debug, Clone)]
pub struct VulnerabilityDatabase {
    /// All registered advisories.
    advisories: Vec<VulnerabilityAdvisory>,
}

impl VulnerabilityDatabase {
    /// Create an empty vulnerability database.
    pub fn new() -> Self {
        Self {
            advisories: Vec::new(),
        }
    }

    /// Add an advisory to the database.
    pub fn add_advisory(&mut self, advisory: VulnerabilityAdvisory) {
        self.advisories.push(advisory);
    }

    /// Check whether an installed package has known vulnerabilities.
    ///
    /// Uses simple equality matching on the package name and substring
    /// matching on the version range string.
    pub fn check_package(&self, name: &str, version: &str) -> Vec<&VulnerabilityAdvisory> {
        self.advisories
            .iter()
            .filter(|adv| {
                adv.affected_packages.iter().any(|(pkg_name, ver_range)| {
                    pkg_name == name
                        && (ver_range == "*"
                            || ver_range == version
                            || version.contains(ver_range.as_str()))
                })
            })
            .collect()
    }

    /// Batch-check a set of installed packages.
    ///
    /// Takes `(name, version)` pairs and returns matching advisories together
    /// with the affected package name.
    pub fn check_installed<'a>(
        &'a self,
        installed: &'a [(String, String)],
    ) -> Vec<(&'a str, &'a VulnerabilityAdvisory)> {
        let mut results = Vec::new();
        for (name, version) in installed {
            for adv in self.check_package(name, version) {
                results.push((name.as_str(), adv));
            }
        }
        results
    }

    /// Total number of advisories in the database.
    pub fn advisory_count(&self) -> usize {
        self.advisories.len()
    }

    /// Number of `Critical`-severity advisories.
    pub fn critical_count(&self) -> usize {
        self.advisories
            .iter()
            .filter(|a| a.severity == Severity::Critical)
            .count()
    }
}

impl Default for VulnerabilityDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // ---- HttpResponse ----

    #[test]
    fn test_http_response_new() {
        let r = HttpResponse::new();
        assert_eq!(r.status_code, 0);
        assert!(r.headers.is_empty());
        assert!(r.body.is_empty());
    }

    #[test]
    fn test_http_response_is_success() {
        let mut r = HttpResponse::new();
        r.status_code = 200;
        assert!(r.is_success());
        r.status_code = 299;
        assert!(r.is_success());
        r.status_code = 300;
        assert!(!r.is_success());
        r.status_code = 199;
        assert!(!r.is_success());
        r.status_code = 404;
        assert!(!r.is_success());
    }

    // ---- HttpClient URL building / parsing ----

    #[test]
    fn test_http_client_build_url_with_trailing_slash() {
        let client = HttpClient::new(String::from("https://example.com/"));
        let url = client.build_url("packages.json");
        assert_eq!(url, "https://example.com/packages.json");
    }

    #[test]
    fn test_http_client_build_url_without_trailing_slash() {
        let client = HttpClient::new(String::from("https://example.com"));
        let url = client.build_url("packages.json");
        assert_eq!(url, "https://example.com/packages.json");
    }

    #[test]
    fn test_http_client_build_url_path_with_leading_slash() {
        let client = HttpClient::new(String::from("https://example.com"));
        let url = client.build_url("/packages.json");
        assert_eq!(url, "https://example.com/packages.json");
    }

    #[test]
    fn test_http_client_parse_url_https() {
        let client = HttpClient::new(String::from("https://example.com"));
        let (host, port, path) = client.parse_url("https://example.com/path").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/path");
    }

    #[test]
    fn test_http_client_parse_url_http() {
        let client = HttpClient::new(String::from("http://example.com"));
        let (host, port, path) = client.parse_url("http://example.com/path").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443); // Defaults to 443
        assert_eq!(path, "/path");
    }

    #[test]
    fn test_http_client_parse_url_with_port() {
        let client = HttpClient::new(String::from(""));
        let (host, port, path) = client.parse_url("https://example.com:8080/api").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8080);
        assert_eq!(path, "/api");
    }

    #[test]
    fn test_http_client_parse_url_no_path() {
        let client = HttpClient::new(String::from(""));
        let (host, port, path) = client.parse_url("https://example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/");
    }

    #[test]
    fn test_http_client_build_request_get() {
        let client = HttpClient::new(String::from(""));
        let req = client.build_request(HttpMethod::Get, "example.com", "/index.html");
        let req_str = core::str::from_utf8(&req).unwrap();
        assert!(req_str.starts_with("GET /index.html HTTP/1.1\r\n"));
        assert!(req_str.contains("Host: example.com"));
    }

    #[test]
    fn test_http_client_build_request_head() {
        let client = HttpClient::new(String::from(""));
        let req = client.build_request(HttpMethod::Head, "example.com", "/");
        let req_str = core::str::from_utf8(&req).unwrap();
        assert!(req_str.starts_with("HEAD / HTTP/1.1\r\n"));
    }

    #[test]
    fn test_http_client_resolve_hostname_ip() {
        let client = HttpClient::new(String::from(""));
        let ip = client.resolve_hostname("192.168.1.10").unwrap();
        assert_eq!(ip, [192, 168, 1, 10]);
    }

    #[test]
    fn test_http_client_resolve_hostname_fallback() {
        let client = HttpClient::new(String::from(""));
        let ip = client.resolve_hostname("example.com").unwrap();
        assert_eq!(ip, [127, 0, 0, 1]);
    }

    #[test]
    fn test_http_client_find_header_end() {
        let client = HttpClient::new(String::from(""));
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\nbody";
        let pos = client.find_header_end(data);
        assert!(pos.is_some());
        // Verify that the data at pos..pos+4 is \r\n\r\n
        let p = pos.unwrap();
        assert_eq!(&data[p..p + 4], b"\r\n\r\n");
    }

    #[test]
    fn test_http_client_find_header_end_missing() {
        let client = HttpClient::new(String::from(""));
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n";
        assert!(client.find_header_end(data).is_none());
    }

    #[test]
    fn test_http_client_parse_response() {
        let client = HttpClient::new(String::from(""));
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello world";
        let response = client.parse_response(raw).unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.headers.len(), 1);
        assert_eq!(response.headers[0].0, "Content-Type");
        assert_eq!(response.headers[0].1, "text/plain");
        assert_eq!(response.body, b"hello world");
    }

    // ---- Repository ----

    #[test]
    fn test_repository_new() {
        let repo = Repository::new(String::from("test"), String::from("https://test.org"), true);
        assert_eq!(repo.name, "test");
        assert_eq!(repo.url, "https://test.org");
        assert!(repo.trusted);
        assert!(repo.package_cache.is_empty());
        assert_eq!(repo.last_updated, 0);
    }

    #[test]
    fn test_repository_default() {
        let repo = Repository::default();
        assert_eq!(repo.name, "default");
        assert!(repo.trusted);
    }

    #[test]
    fn test_repository_has_package_empty() {
        let repo = Repository::default();
        assert!(!repo.has_package(&String::from("nonexistent")));
    }

    #[test]
    fn test_repository_get_package_none() {
        let repo = Repository::default();
        assert!(repo.get_package(&String::from("nonexistent")).is_none());
    }

    #[test]
    fn test_repository_invalidate_cache() {
        let mut repo = Repository::default();
        repo.last_updated = 1000;
        repo.invalidate_cache();
        assert!(repo.package_cache.is_empty());
        assert_eq!(repo.last_updated, 0);
    }

    #[test]
    fn test_repository_is_cache_stale() {
        let repo = Repository::default();
        // last_updated = 0, current_time = 3601 -> stale
        assert!(repo.is_cache_stale(3601));
        // current_time = 3600 -> not stale (exactly 1 hour)
        assert!(!repo.is_cache_stale(3600));
        // current_time = 100 -> not stale
        assert!(!repo.is_cache_stale(100));
    }

    // ---- Repository JSON parsing ----

    #[test]
    fn test_repository_extract_json_string() {
        let repo = Repository::default();
        let json = r#"{"name":"curl","version":"8.5.0"}"#;
        assert_eq!(
            repo.extract_json_string(json, "name"),
            Some(String::from("curl"))
        );
        assert_eq!(
            repo.extract_json_string(json, "version"),
            Some(String::from("8.5.0"))
        );
        assert_eq!(repo.extract_json_string(json, "missing"), None);
    }

    #[test]
    fn test_repository_parse_version() {
        let repo = Repository::default();
        let v = repo.parse_version("1.2.3").unwrap();
        assert_eq!(v, Version::new(1, 2, 3));
    }

    #[test]
    fn test_repository_parse_version_too_short() {
        let repo = Repository::default();
        assert!(repo.parse_version("1.2").is_none());
    }

    #[test]
    fn test_repository_parse_version_invalid() {
        let repo = Repository::default();
        assert!(repo.parse_version("abc").is_none());
    }

    #[test]
    fn test_repository_extract_conflicts() {
        let repo = Repository::default();
        let json = r#"{"conflicts":["pkg-b","pkg-c"]}"#;
        let conflicts = repo.extract_conflicts(json);
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0], "pkg-b");
        assert_eq!(conflicts[1], "pkg-c");
    }

    #[test]
    fn test_repository_extract_conflicts_empty() {
        let repo = Repository::default();
        let json = r#"{"name":"test"}"#;
        let conflicts = repo.extract_conflicts(json);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_repository_extract_dependencies() {
        let repo = Repository::default();
        let json = r#"{"dependencies":[{"name":"openssl","version":"1.0.0"}]}"#;
        let deps = repo.extract_dependencies(json);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "openssl");
        assert_eq!(deps[0].version_req, "1.0.0");
    }

    #[test]
    fn test_repository_parse_package_index_empty_array() {
        let repo = Repository::default();
        let data = b"[]";
        let pkgs = repo.parse_package_index(data);
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_repository_parse_package_index_not_array() {
        let repo = Repository::default();
        let data = b"{}";
        let pkgs = repo.parse_package_index(data);
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_repository_parse_package_index_invalid_utf8() {
        let repo = Repository::default();
        let data: &[u8] = &[0xFF, 0xFE];
        let pkgs = repo.parse_package_index(data);
        assert!(pkgs.is_empty());
    }

    // ---- RepositoryIndex ----

    #[test]
    fn test_repository_index_generate() {
        let pkgs = vec![PackageMetadata {
            name: String::from("test-pkg"),
            version: Version::new(1, 0, 0),
            author: String::from("author"),
            description: String::from("desc"),
            license: String::from("MIT"),
            dependencies: vec![],
            conflicts: vec![],
        }];
        let index = RepositoryIndex::generate(&pkgs);
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].name, "test-pkg");
        assert_eq!(index.entries[0].version, "1.0.0");
        assert_eq!(index.entries[0].description, "desc");
        assert_eq!(index.entries[0].license, "MIT");
    }

    #[test]
    fn test_repository_index_to_bytes() {
        let pkgs = vec![PackageMetadata {
            name: String::from("a"),
            version: Version::new(0, 1, 0),
            author: String::new(),
            description: String::from("d"),
            license: String::from("MIT"),
            dependencies: vec![],
            conflicts: vec![],
        }];
        let index = RepositoryIndex::generate(&pkgs);
        let bytes = index.to_bytes();
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("\"name\":\"a\""));
        assert!(s.contains("\"version\":\"0.1.0\""));
        assert!(s.contains("\"license\":\"MIT\""));
    }

    #[test]
    fn test_repository_index_verify_signature_empty() {
        let index = RepositoryIndex::generate(&[]);
        // Empty signature -> false
        assert!(!index.verify_signature(&[1, 2, 3]));
        // Empty public key -> false
        let mut index2 = RepositoryIndex::generate(&[]);
        index2.signature = vec![1, 2, 3];
        assert!(!index2.verify_signature(&[]));
    }

    // ---- MirrorManager ----

    #[test]
    fn test_mirror_manager_new() {
        let mm = MirrorManager::new();
        assert_eq!(mm.mirror_count(), 0);
        assert!(mm.list_mirrors().is_empty());
    }

    #[test]
    fn test_mirror_manager_add_and_sort() {
        let mut mm = MirrorManager::new();
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror2.com"),
            priority: 20,
            region: String::from("us"),
            last_sync: 0,
            status: MirrorStatus::Unknown,
        });
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror1.com"),
            priority: 10,
            region: String::from("eu"),
            last_sync: 0,
            status: MirrorStatus::Unknown,
        });
        assert_eq!(mm.mirror_count(), 2);
        // Should be sorted by priority (lower first)
        assert_eq!(mm.list_mirrors()[0].url, "https://mirror1.com");
        assert_eq!(mm.list_mirrors()[1].url, "https://mirror2.com");
    }

    #[test]
    fn test_mirror_manager_remove() {
        let mut mm = MirrorManager::new();
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror1.com"),
            priority: 10,
            region: String::new(),
            last_sync: 0,
            status: MirrorStatus::Online,
        });
        assert!(mm.remove_mirror("https://mirror1.com"));
        assert!(!mm.remove_mirror("https://nonexistent.com"));
        assert_eq!(mm.mirror_count(), 0);
    }

    #[test]
    fn test_mirror_manager_select_best() {
        let mut mm = MirrorManager::new();
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror1.com"),
            priority: 10,
            region: String::new(),
            last_sync: 0,
            status: MirrorStatus::Offline,
        });
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror2.com"),
            priority: 20,
            region: String::new(),
            last_sync: 0,
            status: MirrorStatus::Online,
        });
        // Should skip offline mirror1 and return online mirror2
        let best = mm.select_best_mirror().unwrap();
        assert_eq!(best.url, "https://mirror2.com");
    }

    #[test]
    fn test_mirror_manager_select_best_all_offline() {
        let mut mm = MirrorManager::new();
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror1.com"),
            priority: 10,
            region: String::new(),
            last_sync: 0,
            status: MirrorStatus::Offline,
        });
        // Falls back to first mirror when all offline
        let best = mm.select_best_mirror().unwrap();
        assert_eq!(best.url, "https://mirror1.com");
    }

    #[test]
    fn test_mirror_manager_select_best_empty() {
        let mm = MirrorManager::new();
        assert!(mm.select_best_mirror().is_none());
    }

    #[test]
    fn test_mirror_manager_mark_offline() {
        let mut mm = MirrorManager::new();
        mm.add_mirror(MirrorMetadata {
            url: String::from("https://mirror1.com"),
            priority: 10,
            region: String::new(),
            last_sync: 0,
            status: MirrorStatus::Online,
        });
        mm.mark_offline("https://mirror1.com");
        assert_eq!(mm.list_mirrors()[0].status, MirrorStatus::Offline);
    }

    // ---- RepositoryConfig ----

    #[test]
    fn test_repo_config_new() {
        let cfg = RepositoryConfig::new();
        assert!(cfg.all_repositories().is_empty());
        assert!(cfg.enabled_repositories().is_empty());
    }

    #[test]
    fn test_repo_config_add_and_sort() {
        let mut cfg = RepositoryConfig::new();
        cfg.add_repository(RepositoryEntry {
            name: String::from("b"),
            url: String::from("https://b.org"),
            enabled: true,
            trusted: true,
            priority: 20,
            mirrors: vec![],
        });
        cfg.add_repository(RepositoryEntry {
            name: String::from("a"),
            url: String::from("https://a.org"),
            enabled: true,
            trusted: false,
            priority: 10,
            mirrors: vec![],
        });
        assert_eq!(cfg.all_repositories().len(), 2);
        assert_eq!(cfg.all_repositories()[0].name, "a");
    }

    #[test]
    fn test_repo_config_remove() {
        let mut cfg = RepositoryConfig::new();
        cfg.add_repository(RepositoryEntry {
            name: String::from("test"),
            url: String::from("https://test.org"),
            enabled: true,
            trusted: true,
            priority: 10,
            mirrors: vec![],
        });
        assert!(cfg.remove_repository("test"));
        assert!(!cfg.remove_repository("test"));
        assert!(cfg.all_repositories().is_empty());
    }

    #[test]
    fn test_repo_config_enable_disable() {
        let mut cfg = RepositoryConfig::new();
        cfg.add_repository(RepositoryEntry {
            name: String::from("test"),
            url: String::from("https://test.org"),
            enabled: false,
            trusted: true,
            priority: 10,
            mirrors: vec![],
        });
        assert!(cfg.enabled_repositories().is_empty());
        assert!(cfg.enable_repository("test"));
        assert_eq!(cfg.enabled_repositories().len(), 1);
        assert!(cfg.disable_repository("test"));
        assert!(cfg.enabled_repositories().is_empty());
        assert!(!cfg.enable_repository("nonexistent"));
        assert!(!cfg.disable_repository("nonexistent"));
    }

    #[test]
    fn test_repo_config_get_repository() {
        let mut cfg = RepositoryConfig::new();
        cfg.add_repository(RepositoryEntry {
            name: String::from("test"),
            url: String::from("https://test.org"),
            enabled: true,
            trusted: true,
            priority: 10,
            mirrors: vec![],
        });
        assert!(cfg.get_repository("test").is_some());
        assert!(cfg.get_repository("other").is_none());
    }

    // ---- AccessControl ----

    #[test]
    fn test_access_control_new() {
        let ac = AccessControl::new(UploadPolicy::Open);
        assert_eq!(ac.upload_policy, UploadPolicy::Open);
    }

    #[test]
    fn test_access_control_add_remove_uploader() {
        let mut ac = AccessControl::new(UploadPolicy::Restricted);
        let fp = [0u8; 32];
        ac.add_uploader(fp);
        // Adding duplicate should not create duplicates
        ac.add_uploader(fp);
        assert!(ac.remove_uploader(&fp));
        assert!(!ac.remove_uploader(&fp));
    }

    #[test]
    fn test_access_control_default() {
        let ac = AccessControl::default();
        assert_eq!(ac.upload_policy, UploadPolicy::Restricted);
    }

    // ---- SecurityScanner ----

    #[test]
    fn test_security_scanner_default_patterns() {
        let scanner = SecurityScanner::new();
        // Should have default patterns loaded
        assert!(!scanner.patterns.is_empty());
    }

    #[test]
    fn test_security_scanner_scan_paths_match() {
        let scanner = SecurityScanner::new();
        let paths = &["/etc/shadow"];
        let findings = scanner.scan_package_paths(paths);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn test_security_scanner_scan_paths_no_match() {
        let scanner = SecurityScanner::new();
        let paths = &["/usr/bin/hello"];
        let findings = scanner.scan_package_paths(paths);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_security_scanner_scan_capabilities() {
        let scanner = SecurityScanner::new();
        let caps = &["CAP_SYS_ADMIN"];
        let findings = scanner.scan_capabilities(caps);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, Severity::Medium);
    }

    #[test]
    fn test_security_scanner_scan_capabilities_no_match() {
        let scanner = SecurityScanner::new();
        let caps = &["CAP_READ"];
        let findings = scanner.scan_capabilities(caps);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_security_scanner_add_pattern() {
        let mut scanner = SecurityScanner::new();
        let initial_count = scanner.patterns.len();
        scanner.add_pattern(MalwarePattern {
            pattern_type: PatternType::KnownBadHash,
            description: String::from("bad hash"),
            severity: Severity::Critical,
            pattern: String::from("deadbeef"),
        });
        assert_eq!(scanner.patterns.len(), initial_count + 1);
    }

    // ---- Severity ordering ----

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    // ---- VulnerabilityDatabase ----

    #[test]
    fn test_vuln_db_new() {
        let db = VulnerabilityDatabase::new();
        assert_eq!(db.advisory_count(), 0);
        assert_eq!(db.critical_count(), 0);
    }

    #[test]
    fn test_vuln_db_add_and_check() {
        let mut db = VulnerabilityDatabase::new();
        db.add_advisory(VulnerabilityAdvisory {
            id: String::from("CVE-2024-0001"),
            affected_packages: vec![(String::from("openssl"), String::from("1.0.0"))],
            severity: Severity::Critical,
            description: String::from("buffer overflow"),
            fixed_version: Some(String::from("1.0.1")),
        });
        assert_eq!(db.advisory_count(), 1);
        assert_eq!(db.critical_count(), 1);

        let results = db.check_package("openssl", "1.0.0");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "CVE-2024-0001");

        let results = db.check_package("openssl", "1.0.1");
        assert!(results.is_empty());
    }

    #[test]
    fn test_vuln_db_wildcard() {
        let mut db = VulnerabilityDatabase::new();
        db.add_advisory(VulnerabilityAdvisory {
            id: String::from("CVE-2024-0002"),
            affected_packages: vec![(String::from("curl"), String::from("*"))],
            severity: Severity::High,
            description: String::from("all versions"),
            fixed_version: None,
        });
        let results = db.check_package("curl", "999.0.0");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_vuln_db_check_installed() {
        let mut db = VulnerabilityDatabase::new();
        db.add_advisory(VulnerabilityAdvisory {
            id: String::from("CVE-2024-0003"),
            affected_packages: vec![(String::from("pkg-a"), String::from("1.0.0"))],
            severity: Severity::Medium,
            description: String::from("test"),
            fixed_version: None,
        });
        let installed = vec![
            (String::from("pkg-a"), String::from("1.0.0")),
            (String::from("pkg-b"), String::from("2.0.0")),
        ];
        let results = db.check_installed(&installed);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "pkg-a");
    }
}
