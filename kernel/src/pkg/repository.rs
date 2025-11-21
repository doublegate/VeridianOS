//! Package Repository Management
//!
//! Implements HTTP-based package repository fetching for VeridianOS package
//! manager. Uses the network stack for actual HTTP communication.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use super::{Dependency, PackageId, PackageMetadata, Version};

/// HTTP request type
#[derive(Debug, Clone, Copy)]
enum HttpMethod {
    Get,
    Head,
}

/// HTTP response
#[derive(Debug)]
struct HttpResponse {
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
struct HttpClient {
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
enum HttpError {
    ConnectionFailed,
    InvalidResponse,
    ResponseTooLarge,
    #[allow(dead_code)]
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
