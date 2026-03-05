//! HTTP/1.1 client library for VeridianOS
//!
//! Provides an HTTP request builder, incremental response parser,
//! and client abstraction for sending requests over TCP connections.
//! Supports chunked transfer encoding, keep-alive, redirects,
//! basic authentication, cookies, and query string encoding.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default HTTP port
pub const HTTP_PORT: u16 = 80;

/// Default HTTPS port
pub const HTTPS_PORT: u16 = 443;

/// Default maximum redirects to follow
pub const DEFAULT_MAX_REDIRECTS: u8 = 10;

/// Default request timeout in milliseconds
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// HTTP version string
const HTTP_VERSION: &str = "HTTP/1.1";

/// Default User-Agent
const DEFAULT_USER_AGENT: &str = "VeridianOS/0.10.6";

// MIME type constants
/// text/html
pub const MIME_TEXT_HTML: &str = "text/html";
/// application/json
pub const MIME_APPLICATION_JSON: &str = "application/json";
/// text/plain
pub const MIME_TEXT_PLAIN: &str = "text/plain";
/// application/octet-stream
pub const MIME_APPLICATION_OCTET_STREAM: &str = "application/octet-stream";
/// application/x-www-form-urlencoded
pub const MIME_FORM_URLENCODED: &str = "application/x-www-form-urlencoded";

// ---------------------------------------------------------------------------
// HTTP Method
// ---------------------------------------------------------------------------

/// HTTP request methods
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Patch,
    Options,
}

#[cfg(feature = "alloc")]
impl HttpMethod {
    /// Return the method as an uppercase string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Options => "OPTIONS",
        }
    }
}

// ---------------------------------------------------------------------------
// URL parsing
// ---------------------------------------------------------------------------

/// Parsed URL components.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    /// Scheme: "http" or "https"
    pub scheme: String,
    /// Host name or IP address
    pub host: String,
    /// Port number (default 80 for http, 443 for https)
    pub port: u16,
    /// Path component (starts with '/')
    pub path: String,
    /// Optional query string (without leading '?')
    pub query: Option<String>,
    /// Whether this is an HTTPS URL
    pub is_https: bool,
}

#[cfg(feature = "alloc")]
impl ParsedUrl {
    /// Parse a URL string into its components.
    ///
    /// Supports `http://host[:port][/path][?query]` and
    /// `https://host[:port][/path][?query]`.
    pub fn parse(url: &str) -> Result<Self, HttpError> {
        let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
            ("https", r)
        } else if let Some(r) = url.strip_prefix("http://") {
            ("http", r)
        } else {
            return Err(HttpError::InvalidUrl);
        };

        let is_https = scheme == "https";
        let default_port: u16 = if is_https { HTTPS_PORT } else { HTTP_PORT };

        // Split host+port from path+query
        let (authority, path_and_query) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };

        // Split path and query
        let (path, query) = match path_and_query.find('?') {
            Some(idx) => (&path_and_query[..idx], Some(&path_and_query[idx + 1..])),
            None => (path_and_query, None),
        };

        // Split host and port
        let (host, port) = if let Some(colon_idx) = authority.rfind(':') {
            let port_str = &authority[colon_idx + 1..];
            if let Ok(p) = parse_u16(port_str) {
                (&authority[..colon_idx], p)
            } else {
                (authority, default_port)
            }
        } else {
            (authority, default_port)
        };

        if host.is_empty() {
            return Err(HttpError::InvalidUrl);
        }

        let path_str = if path.is_empty() {
            String::from("/")
        } else {
            String::from(path)
        };

        Ok(ParsedUrl {
            scheme: String::from(scheme),
            host: String::from(host),
            port,
            path: path_str,
            query: query.map(String::from),
            is_https,
        })
    }

    /// Return full request path including query string.
    pub fn request_path(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during HTTP operations.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// URL could not be parsed.
    InvalidUrl,
    /// Response contained an invalid status line.
    InvalidStatusLine,
    /// Response contained an invalid header.
    InvalidHeader,
    /// Chunked encoding contained an invalid chunk size.
    InvalidChunkSize,
    /// Response body exceeds maximum allowed size.
    BodyTooLarge,
    /// Too many redirects followed.
    TooManyRedirects,
    /// Connection timed out.
    Timeout,
    /// Generic parse error with a message.
    ParseError(String),
}

// ---------------------------------------------------------------------------
// HTTP Request
// ---------------------------------------------------------------------------

/// An HTTP request ready to be serialized and sent.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method
    pub method: HttpMethod,
    /// Parsed URL
    pub url: ParsedUrl,
    /// Headers (lowercase key -> value)
    pub headers: BTreeMap<String, String>,
    /// Optional request body
    pub body: Option<Vec<u8>>,
}

#[cfg(feature = "alloc")]
impl HttpRequest {
    /// Create a new request with default headers.
    pub fn new(method: HttpMethod, url: &str) -> Result<Self, HttpError> {
        let parsed = ParsedUrl::parse(url)?;
        let mut headers = BTreeMap::new();

        // Set mandatory Host header
        if parsed.port != HTTP_PORT && parsed.port != HTTPS_PORT {
            headers.insert(
                String::from("host"),
                format!("{}:{}", parsed.host, parsed.port),
            );
        } else {
            headers.insert(String::from("host"), parsed.host.clone());
        }

        headers.insert(String::from("user-agent"), String::from(DEFAULT_USER_AGENT));
        headers.insert(String::from("accept"), String::from("*/*"));
        headers.insert(String::from("connection"), String::from("keep-alive"));

        Ok(HttpRequest {
            method,
            url: parsed,
            headers,
            body: None,
        })
    }

    /// Set a header (name is stored lowercase).
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.insert(to_lowercase(name), String::from(value));
    }

    /// Set the request body and automatically set Content-Length.
    pub fn set_body(&mut self, body: Vec<u8>) {
        let len = body.len();
        self.body = Some(body);
        self.headers
            .insert(String::from("content-length"), uint_to_string(len));
    }

    /// Set the request body from a string.
    pub fn set_body_str(&mut self, body: &str) {
        self.set_body(Vec::from(body.as_bytes()));
    }

    /// Set Content-Type header.
    pub fn set_content_type(&mut self, mime: &str) {
        self.set_header("content-type", mime);
    }

    /// Set Authorization header with Basic auth (Base64-encoded user:password).
    pub fn set_basic_auth(&mut self, user: &str, password: &str) {
        let credentials = format!("{}:{}", user, password);
        let encoded = base64_encode(credentials.as_bytes());
        self.set_header("authorization", &format!("Basic {}", encoded));
    }

    /// Set Cookie header from a list of (name, value) pairs.
    pub fn set_cookies(&mut self, cookies: &[(&str, &str)]) {
        if cookies.is_empty() {
            return;
        }
        let mut cookie_str = String::new();
        for (i, (name, value)) in cookies.iter().enumerate() {
            if i > 0 {
                cookie_str.push_str("; ");
            }
            cookie_str.push_str(name);
            cookie_str.push('=');
            cookie_str.push_str(value);
        }
        self.set_header("cookie", &cookie_str);
    }

    /// Serialize the request to bytes for sending over TCP.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        // Request line
        let request_line = format!(
            "{} {} {}\r\n",
            self.method.as_str(),
            self.url.request_path(),
            HTTP_VERSION,
        );
        buf.extend_from_slice(request_line.as_bytes());

        // Headers
        for (name, value) in &self.headers {
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(value.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }

        // End of headers
        buf.extend_from_slice(b"\r\n");

        // Body
        if let Some(ref body) = self.body {
            buf.extend_from_slice(body);
        }

        buf
    }
}

// ---------------------------------------------------------------------------
// HTTP Response
// ---------------------------------------------------------------------------

/// An HTTP response.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP version string (e.g. "HTTP/1.1")
    pub version: String,
    /// Status code (e.g. 200, 404)
    pub status_code: u16,
    /// Reason phrase (e.g. "OK", "Not Found")
    pub reason: String,
    /// Response headers (lowercase key -> value)
    pub headers: BTreeMap<String, String>,
    /// Response body
    pub body: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl HttpResponse {
    /// Check if this is a redirect status (301, 302, 307, 308).
    pub fn is_redirect(&self) -> bool {
        matches!(self.status_code, 301 | 302 | 307 | 308)
    }

    /// Get the Location header for redirects.
    pub fn redirect_location(&self) -> Option<&String> {
        self.headers.get("location")
    }

    /// Check if the connection should be kept alive.
    pub fn is_keep_alive(&self) -> bool {
        if let Some(conn) = self.headers.get("connection") {
            let lower = to_lowercase(conn);
            lower.contains("keep-alive")
        } else {
            // HTTP/1.1 defaults to keep-alive
            self.version.contains("1.1")
        }
    }

    /// Get the Content-Type header value.
    pub fn content_type(&self) -> Option<&String> {
        self.headers.get("content-type")
    }

    /// Get the Content-Length value if present.
    pub fn content_length(&self) -> Option<usize> {
        self.headers
            .get("content-length")
            .and_then(|v| parse_usize(v).ok())
    }

    /// Get Content-Encoding header value.
    pub fn content_encoding(&self) -> ContentEncoding {
        match self.headers.get("content-encoding") {
            Some(v) if to_lowercase(v).contains("gzip") => ContentEncoding::Gzip,
            Some(v) if to_lowercase(v).contains("deflate") => ContentEncoding::Deflate,
            _ => ContentEncoding::Identity,
        }
    }

    /// Return the body interpreted as a UTF-8 string, if valid.
    pub fn body_as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.body).ok()
    }
}

// ---------------------------------------------------------------------------
// Content Encoding
// ---------------------------------------------------------------------------

/// Content-Encoding types.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEncoding {
    /// No encoding (identity)
    Identity,
    /// Gzip compression (stub -- caller must decompress)
    Gzip,
    /// Deflate compression (stub -- caller must decompress)
    Deflate,
}

// ---------------------------------------------------------------------------
// Response Parser (incremental / state machine)
// ---------------------------------------------------------------------------

/// Current state of the incremental response parser.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseState {
    /// Awaiting or parsing the status line.
    StatusLine,
    /// Parsing headers.
    Headers,
    /// Reading a fixed-length body (Content-Length).
    Body,
    /// Reading a chunked body.
    ChunkedBody,
    /// Parsing is complete.
    Complete,
}

/// Incremental HTTP response parser.
///
/// Feed bytes via `feed()`. When `state()` returns `ParseState::Complete`,
/// call `take_response()` to extract the finished `HttpResponse`.
#[cfg(feature = "alloc")]
pub struct ResponseParser {
    state: ParseState,
    buffer: Vec<u8>,
    version: String,
    status_code: u16,
    reason: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    content_length: Option<usize>,
    chunked: bool,
    /// Remaining bytes in the current chunk (for chunked encoding).
    chunk_remaining: usize,
    /// Whether we have finished reading the chunk size line for this chunk.
    chunk_size_parsed: bool,
}

#[cfg(feature = "alloc")]
impl Default for ResponseParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl ResponseParser {
    /// Create a new response parser.
    pub fn new() -> Self {
        ResponseParser {
            state: ParseState::StatusLine,
            buffer: Vec::new(),
            version: String::new(),
            status_code: 0,
            reason: String::new(),
            headers: BTreeMap::new(),
            body: Vec::new(),
            content_length: None,
            chunked: false,
            chunk_remaining: 0,
            chunk_size_parsed: false,
        }
    }

    /// Return the current parse state.
    pub fn state(&self) -> &ParseState {
        &self.state
    }

    /// Feed bytes into the parser, advancing the state machine.
    pub fn feed(&mut self, data: &[u8]) -> Result<(), HttpError> {
        self.buffer.extend_from_slice(data);

        loop {
            match self.state {
                ParseState::StatusLine => {
                    if !self.try_parse_status_line()? {
                        return Ok(());
                    }
                }
                ParseState::Headers => {
                    if !self.try_parse_headers()? {
                        return Ok(());
                    }
                }
                ParseState::Body => {
                    self.try_parse_body();
                    return Ok(());
                }
                ParseState::ChunkedBody => {
                    if !self.try_parse_chunked()? {
                        return Ok(());
                    }
                }
                ParseState::Complete => return Ok(()),
            }
        }
    }

    /// Try to parse the status line from the buffer.
    /// Returns `true` if the status line was found and parsed.
    fn try_parse_status_line(&mut self) -> Result<bool, HttpError> {
        let line_end = match find_crlf(&self.buffer) {
            Some(pos) => pos,
            None => return Ok(false),
        };

        let line = match core::str::from_utf8(&self.buffer[..line_end]) {
            Ok(s) => s,
            Err(_) => return Err(HttpError::InvalidStatusLine),
        };

        // Parse "HTTP/1.1 200 OK"
        let mut parts = line.splitn(3, ' ');
        let version = parts.next().ok_or(HttpError::InvalidStatusLine)?;
        let status_str = parts.next().ok_or(HttpError::InvalidStatusLine)?;
        let reason = parts.next().unwrap_or("");

        self.version = String::from(version);
        self.status_code = parse_u16(status_str).map_err(|_| HttpError::InvalidStatusLine)?;
        self.reason = String::from(reason);

        // Consume the line + CRLF
        let new_start = line_end + 2;
        self.buffer = self.buffer[new_start..].to_vec();
        self.state = ParseState::Headers;
        Ok(true)
    }

    /// Try to parse headers from the buffer.
    /// Returns `true` when all headers have been consumed (empty line found).
    fn try_parse_headers(&mut self) -> Result<bool, HttpError> {
        loop {
            let line_end = match find_crlf(&self.buffer) {
                Some(pos) => pos,
                None => return Ok(false),
            };

            if line_end == 0 {
                // Empty line -- end of headers
                self.buffer = self.buffer[2..].to_vec();
                self.determine_body_mode();
                return Ok(true);
            }

            let line = match core::str::from_utf8(&self.buffer[..line_end]) {
                Ok(s) => s,
                Err(_) => return Err(HttpError::InvalidHeader),
            };

            if let Some(colon_pos) = line.find(':') {
                let name = to_lowercase(&line[..colon_pos]);
                let value = line[colon_pos + 1..].trim_start();
                self.headers.insert(name, String::from(value));
            }

            let new_start = line_end + 2;
            self.buffer = self.buffer[new_start..].to_vec();
        }
    }

    /// Determine whether to read a fixed-length body or chunked body.
    fn determine_body_mode(&mut self) {
        // Check for chunked transfer encoding
        if let Some(te) = self.headers.get("transfer-encoding") {
            if to_lowercase(te).contains("chunked") {
                self.chunked = true;
                self.state = ParseState::ChunkedBody;
                return;
            }
        }

        // Check for Content-Length
        if let Some(cl) = self.headers.get("content-length") {
            if let Ok(len) = parse_usize(cl) {
                self.content_length = Some(len);
                if len == 0 {
                    self.state = ParseState::Complete;
                } else {
                    self.state = ParseState::Body;
                }
                return;
            }
        }

        // No body indication -- treat as complete (e.g., HEAD response)
        self.state = ParseState::Complete;
    }

    /// Try to read a Content-Length body.
    fn try_parse_body(&mut self) {
        if let Some(expected) = self.content_length {
            if self.buffer.len() >= expected {
                self.body = self.buffer[..expected].to_vec();
                self.buffer = self.buffer[expected..].to_vec();
                self.state = ParseState::Complete;
            }
            // else: need more data
        }
    }

    /// Try to parse chunked transfer-encoded body.
    /// Returns `true` when all chunks have been read (0-size terminator).
    fn try_parse_chunked(&mut self) -> Result<bool, HttpError> {
        loop {
            if !self.chunk_size_parsed {
                // Read chunk size line
                let line_end = match find_crlf(&self.buffer) {
                    Some(pos) => pos,
                    None => return Ok(false),
                };

                let size_line = match core::str::from_utf8(&self.buffer[..line_end]) {
                    Ok(s) => s,
                    Err(_) => return Err(HttpError::InvalidChunkSize),
                };

                // Chunk size may have extensions after ';' -- ignore them
                let size_str = match size_line.find(';') {
                    Some(idx) => &size_line[..idx],
                    None => size_line,
                };

                let chunk_size =
                    parse_hex_usize(size_str.trim()).map_err(|_| HttpError::InvalidChunkSize)?;

                self.buffer = self.buffer[line_end + 2..].to_vec();

                if chunk_size == 0 {
                    // Terminal chunk -- consume trailing CRLF if present
                    if self.buffer.len() >= 2 && self.buffer[0] == b'\r' && self.buffer[1] == b'\n'
                    {
                        self.buffer = self.buffer[2..].to_vec();
                    }
                    self.state = ParseState::Complete;
                    return Ok(true);
                }

                self.chunk_remaining = chunk_size;
                self.chunk_size_parsed = true;
            }

            // Read chunk data
            if self.buffer.len() < self.chunk_remaining {
                return Ok(false);
            }

            self.body
                .extend_from_slice(&self.buffer[..self.chunk_remaining]);
            self.buffer = self.buffer[self.chunk_remaining..].to_vec();
            self.chunk_remaining = 0;
            self.chunk_size_parsed = false;

            // Consume trailing CRLF after chunk data
            if self.buffer.len() >= 2 && self.buffer[0] == b'\r' && self.buffer[1] == b'\n' {
                self.buffer = self.buffer[2..].to_vec();
            } else if self.buffer.len() < 2 {
                return Ok(false);
            }
        }
    }

    /// Extract the completed response. Returns `None` if parsing is not
    /// complete.
    pub fn take_response(&mut self) -> Option<HttpResponse> {
        if self.state != ParseState::Complete {
            return None;
        }

        Some(HttpResponse {
            version: core::mem::take(&mut self.version),
            status_code: self.status_code,
            reason: core::mem::take(&mut self.reason),
            headers: core::mem::take(&mut self.headers),
            body: core::mem::take(&mut self.body),
        })
    }
}

// ---------------------------------------------------------------------------
// HTTP Client
// ---------------------------------------------------------------------------

/// HTTP client with configurable defaults.
///
/// The client does not perform actual I/O. Instead, `prepare_request()` returns
/// serialized bytes to send over TCP, and `parse_response()` accepts received
/// bytes and returns an `HttpResponse` when complete.
#[cfg(feature = "alloc")]
pub struct HttpClient {
    /// Default headers applied to every request.
    pub default_headers: BTreeMap<String, String>,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum number of redirects to follow.
    pub max_redirects: u8,
}

#[cfg(feature = "alloc")]
impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl HttpClient {
    /// Create a new HTTP client with default settings.
    pub fn new() -> Self {
        let mut default_headers = BTreeMap::new();
        default_headers.insert(String::from("user-agent"), String::from(DEFAULT_USER_AGENT));
        default_headers.insert(String::from("accept"), String::from("*/*"));

        HttpClient {
            default_headers,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_redirects: DEFAULT_MAX_REDIRECTS,
        }
    }

    /// Set a default header that will be applied to every request.
    pub fn set_default_header(&mut self, name: &str, value: &str) {
        self.default_headers
            .insert(to_lowercase(name), String::from(value));
    }

    /// Prepare a request: apply default headers and serialize to bytes.
    pub fn prepare_request(&self, request: &mut HttpRequest) -> Vec<u8> {
        // Apply default headers (request headers take precedence)
        for (name, value) in &self.default_headers {
            if !request.headers.contains_key(name) {
                request.headers.insert(name.clone(), value.clone());
            }
        }
        request.serialize()
    }

    /// Create a new response parser for receiving response data.
    pub fn create_parser(&self) -> ResponseParser {
        ResponseParser::new()
    }

    /// Convenience: build and serialize a GET request.
    pub fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        let mut req = HttpRequest::new(HttpMethod::Get, url)?;
        Ok(self.prepare_request(&mut req))
    }

    /// Convenience: build and serialize a POST request with a body.
    pub fn post(&self, url: &str, body: &[u8], content_type: &str) -> Result<Vec<u8>, HttpError> {
        let mut req = HttpRequest::new(HttpMethod::Post, url)?;
        req.set_content_type(content_type);
        req.set_body(Vec::from(body));
        Ok(self.prepare_request(&mut req))
    }

    /// Process a redirect response: returns a new request to the redirect
    /// location, or `None` if not a redirect. Decrements `remaining_redirects`.
    pub fn follow_redirect(
        &self,
        response: &HttpResponse,
        original_url: &str,
        remaining_redirects: &mut u8,
    ) -> Result<Option<HttpRequest>, HttpError> {
        if !response.is_redirect() {
            return Ok(None);
        }

        if *remaining_redirects == 0 {
            return Err(HttpError::TooManyRedirects);
        }
        *remaining_redirects -= 1;

        let location = match response.redirect_location() {
            Some(loc) => loc.clone(),
            None => return Ok(None),
        };

        // Handle relative URLs by prepending scheme + host from original
        let full_url = if location.starts_with("http://") || location.starts_with("https://") {
            location
        } else {
            let orig = ParsedUrl::parse(original_url)?;
            let scheme = &orig.scheme;
            if orig.port != HTTP_PORT && orig.port != HTTPS_PORT {
                format!("{}://{}:{}{}", scheme, orig.host, orig.port, location)
            } else {
                format!("{}://{}{}", scheme, orig.host, location)
            }
        };

        // 307/308 should preserve the original method; 301/302 change to GET.
        // Simplified: always use GET for redirects.
        let method = HttpMethod::Get;

        let req = HttpRequest::new(method, &full_url)?;
        Ok(Some(req))
    }
}

// ---------------------------------------------------------------------------
// Query string encoding (percent-encoding)
// ---------------------------------------------------------------------------

/// Percent-encode a string for use in URL query parameters.
///
/// Encodes all characters except unreserved characters (A-Z, a-z, 0-9, '-',
/// '_', '.', '~').
#[cfg(feature = "alloc")]
pub fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if is_unreserved(byte) {
            encoded.push(byte as char);
        } else if byte == b' ' {
            encoded.push('+');
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0F));
        }
    }
    encoded
}

/// Encode a list of key-value pairs into a query string.
///
/// Returns `key1=value1&key2=value2&...` with percent-encoding applied.
#[cfg(feature = "alloc")]
pub fn encode_query_string(params: &[(&str, &str)]) -> String {
    let mut result = String::new();
    for (i, (key, value)) in params.iter().enumerate() {
        if i > 0 {
            result.push('&');
        }
        result.push_str(&percent_encode(key));
        result.push('=');
        result.push_str(&percent_encode(value));
    }
    result
}

/// Construct a Basic Authentication header value.
///
/// Returns `"Basic <base64(user:password)>"`.
#[cfg(feature = "alloc")]
pub fn basic_auth_header(user: &str, password: &str) -> String {
    let credentials = format!("{}:{}", user, password);
    let encoded = base64_encode(credentials.as_bytes());
    format!("Basic {}", encoded)
}

/// Build a Cookie header value from a list of (name, value) pairs.
#[cfg(feature = "alloc")]
pub fn build_cookie_header(cookies: &[(&str, &str)]) -> String {
    let mut result = String::new();
    for (i, (name, value)) in cookies.iter().enumerate() {
        if i > 0 {
            result.push_str("; ");
        }
        result.push_str(name);
        result.push('=');
        result.push_str(value);
    }
    result
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Find the position of the first `\r\n` in a byte slice.
fn find_crlf(data: &[u8]) -> Option<usize> {
    if data.len() < 2 {
        return None;
    }
    let limit = data.len() - 1;
    let mut i = 0;
    while i < limit {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Convert an ASCII string to lowercase.
#[cfg(feature = "alloc")]
fn to_lowercase(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            result.push((c as u8 + 32) as char);
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse a `&str` as a `u16`.
fn parse_u16(s: &str) -> Result<u16, ()> {
    let mut val: u16 = 0;
    for byte in s.bytes() {
        if !byte.is_ascii_digit() {
            return Err(());
        }
        val = val.checked_mul(10).ok_or(())?;
        val = val.checked_add((byte - b'0') as u16).ok_or(())?;
    }
    Ok(val)
}

/// Parse a `&str` as a `usize`.
fn parse_usize(s: &str) -> Result<usize, ()> {
    let s = s.trim();
    let mut val: usize = 0;
    for byte in s.bytes() {
        if !byte.is_ascii_digit() {
            return Err(());
        }
        val = val.checked_mul(10).ok_or(())?;
        val = val.checked_add((byte - b'0') as usize).ok_or(())?;
    }
    Ok(val)
}

/// Parse a hexadecimal string as a `usize`.
fn parse_hex_usize(s: &str) -> Result<usize, ()> {
    let mut val: usize = 0;
    for byte in s.bytes() {
        let digit = match byte {
            b'0'..=b'9' => (byte - b'0') as usize,
            b'a'..=b'f' => (byte - b'a') as usize + 10,
            b'A'..=b'F' => (byte - b'A') as usize + 10,
            _ => return Err(()),
        };
        val = val.checked_mul(16).ok_or(())?;
        val = val.checked_add(digit).ok_or(())?;
    }
    Ok(val)
}

/// Convert a `usize` to a decimal `String`.
#[cfg(feature = "alloc")]
fn uint_to_string(mut n: usize) -> String {
    if n == 0 {
        return String::from("0");
    }
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    digits.reverse();
    // Safety: digits are ASCII
    String::from_utf8(digits).unwrap_or_else(|_| String::from("0"))
}

/// Check if a byte is an unreserved URL character.
fn is_unreserved(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

/// Return the hex digit character for a 4-bit value.
fn hex_digit(val: u8) -> char {
    let nibble = val & 0x0F;
    if nibble < 10 {
        (b'0' + nibble) as char
    } else {
        (b'A' + nibble - 10) as char
    }
}

/// Minimal Base64 encoder (no padding configurable -- always pads).
#[cfg(feature = "alloc")]
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;

    while i + 2 < data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        result.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        result.push(TABLE[(n & 0x3F) as usize] as char);
        i += 3;
    }

    let remaining = data.len() - i;
    if remaining == 1 {
        let n = (data[i] as u32) << 16;
        result.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        result.push('=');
        result.push('=');
    } else if remaining == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        result.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        result.push('=');
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -- URL parsing --

    #[test]
    fn test_url_parse_http() {
        let url = ParsedUrl::parse("http://example.com").unwrap();
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/");
        assert_eq!(url.query, None);
        assert!(!url.is_https);
    }

    #[test]
    fn test_url_parse_https() {
        let url = ParsedUrl::parse("https://secure.example.com").unwrap();
        assert_eq!(url.scheme, "https");
        assert_eq!(url.host, "secure.example.com");
        assert_eq!(url.port, 443);
        assert!(url.is_https);
    }

    #[test]
    fn test_url_parse_with_port() {
        let url = ParsedUrl::parse("http://localhost:8080/api").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, 8080);
        assert_eq!(url.path, "/api");
    }

    #[test]
    fn test_url_parse_with_path() {
        let url = ParsedUrl::parse("http://example.com/path/to/resource").unwrap();
        assert_eq!(url.path, "/path/to/resource");
    }

    #[test]
    fn test_url_parse_with_query() {
        let url = ParsedUrl::parse("http://example.com/search?q=rust&page=1").unwrap();
        assert_eq!(url.path, "/search");
        assert_eq!(url.query, Some(String::from("q=rust&page=1")));
        assert_eq!(url.request_path(), "/search?q=rust&page=1");
    }

    #[test]
    fn test_url_parse_invalid() {
        assert_eq!(
            ParsedUrl::parse("ftp://example.com"),
            Err(HttpError::InvalidUrl)
        );
    }

    // -- Request serialization --

    #[test]
    fn test_request_get_serialization() {
        let req = HttpRequest::new(HttpMethod::Get, "http://example.com/index.html").unwrap();
        let bytes = req.serialize();
        let text = core::str::from_utf8(&bytes).unwrap();

        assert!(text.starts_with("GET /index.html HTTP/1.1\r\n"));
        assert!(text.contains("host: example.com\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_request_post_with_body() {
        let mut req = HttpRequest::new(HttpMethod::Post, "http://example.com/api").unwrap();
        req.set_content_type(MIME_APPLICATION_JSON);
        req.set_body_str("{\"key\":\"value\"}");
        let bytes = req.serialize();
        let text = core::str::from_utf8(&bytes).unwrap();

        assert!(text.starts_with("POST /api HTTP/1.1\r\n"));
        assert!(text.contains("content-type: application/json\r\n"));
        assert!(text.contains("content-length: 15\r\n"));
        assert!(text.ends_with("{\"key\":\"value\"}"));
    }

    // -- Response status line parsing --

    #[test]
    fn test_response_status_line() {
        let mut parser = ResponseParser::new();
        parser.feed(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(resp.version, "HTTP/1.1");
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.reason, "OK");
    }

    // -- Header parsing --

    #[test]
    fn test_response_single_header() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(
            resp.headers.get("content-type"),
            Some(&String::from("text/html"))
        );
    }

    #[test]
    fn test_response_multiple_headers() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(
            resp.headers.get("content-type"),
            Some(&String::from("text/html"))
        );
        assert_eq!(resp.headers.get("content-length"), Some(&String::from("5")));
        assert_eq!(resp.body, b"hello");
    }

    #[test]
    fn test_response_case_insensitive_headers() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nCONTENT-TYPE: text/plain\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(
            resp.headers.get("content-type"),
            Some(&String::from("text/plain"))
        );
    }

    // -- Body reading --

    #[test]
    fn test_content_length_body() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHello World")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(resp.body, b"Hello World");
        assert_eq!(resp.body_as_str(), Some("Hello World"));
    }

    #[test]
    fn test_chunked_transfer_decoding() {
        let mut parser = ResponseParser::new();
        parser
            .feed(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                  5\r\nHello\r\n6\r\n World\r\n0\r\n\r\n",
            )
            .unwrap();
        assert_eq!(parser.state(), &ParseState::Complete);
        let resp = parser.take_response().unwrap();
        assert_eq!(resp.body, b"Hello World");
    }

    // -- Redirect detection --

    #[test]
    fn test_redirect_301() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 301 Moved Permanently\r\nLocation: http://new.example.com/\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert!(resp.is_redirect());
        assert_eq!(
            resp.redirect_location(),
            Some(&String::from("http://new.example.com/"))
        );
    }

    #[test]
    fn test_redirect_302() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 302 Found\r\nLocation: /new-path\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert!(resp.is_redirect());
        assert_eq!(resp.status_code, 302);
    }

    #[test]
    fn test_redirect_307() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /temp\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert!(resp.is_redirect());
        assert_eq!(resp.status_code, 307);
    }

    // -- Query string encoding --

    #[test]
    fn test_query_string_encoding() {
        let qs = encode_query_string(&[("key", "value"), ("name", "hello world")]);
        assert_eq!(qs, "key=value&name=hello+world");
    }

    #[test]
    fn test_percent_encode_special_chars() {
        let encoded = percent_encode("a&b=c d");
        assert_eq!(encoded, "a%26b%3Dc+d");
    }

    // -- Keep-alive --

    #[test]
    fn test_keep_alive_detection() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        assert!(resp.is_keep_alive());
    }

    #[test]
    fn test_http11_default_keep_alive() {
        let mut parser = ResponseParser::new();
        parser
            .feed(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        let resp = parser.take_response().unwrap();
        // HTTP/1.1 defaults to keep-alive
        assert!(resp.is_keep_alive());
    }

    // -- Basic auth --

    #[test]
    fn test_basic_auth_header() {
        let header = basic_auth_header("user", "pass");
        // "user:pass" -> base64 "dXNlcjpwYXNz"
        assert_eq!(header, "Basic dXNlcjpwYXNz");
    }

    // -- Cookie header --

    #[test]
    fn test_cookie_header_construction() {
        let cookie = build_cookie_header(&[("session", "abc123"), ("lang", "en")]);
        assert_eq!(cookie, "session=abc123; lang=en");
    }

    // -- Method serialization --

    #[test]
    fn test_method_as_str() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
        assert_eq!(HttpMethod::Put.as_str(), "PUT");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
        assert_eq!(HttpMethod::Head.as_str(), "HEAD");
        assert_eq!(HttpMethod::Patch.as_str(), "PATCH");
        assert_eq!(HttpMethod::Options.as_str(), "OPTIONS");
    }

    // -- Empty response --

    #[test]
    fn test_empty_response_no_body() {
        let mut parser = ResponseParser::new();
        parser.feed(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        let resp = parser.take_response().unwrap();
        assert_eq!(resp.status_code, 204);
        assert!(resp.body.is_empty());
    }

    // -- Incremental parsing --

    #[test]
    fn test_incremental_parsing() {
        let mut parser = ResponseParser::new();

        // Feed status line only
        parser.feed(b"HTTP/1.1 200 OK\r\n").unwrap();
        assert_eq!(parser.state(), &ParseState::Headers);

        // Feed headers
        parser.feed(b"Content-Length: 5\r\n\r\n").unwrap();
        assert_eq!(parser.state(), &ParseState::Body);

        // Feed partial body
        parser.feed(b"Hel").unwrap();
        assert_eq!(parser.state(), &ParseState::Body);

        // Feed rest of body
        parser.feed(b"lo").unwrap();
        assert_eq!(parser.state(), &ParseState::Complete);

        let resp = parser.take_response().unwrap();
        assert_eq!(resp.body, b"Hello");
    }

    // -- Base64 encoder --

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    // -- Hex parsing --

    #[test]
    fn test_parse_hex_usize() {
        assert_eq!(parse_hex_usize("0"), Ok(0));
        assert_eq!(parse_hex_usize("5"), Ok(5));
        assert_eq!(parse_hex_usize("a"), Ok(10));
        assert_eq!(parse_hex_usize("1F"), Ok(31));
        assert_eq!(parse_hex_usize("ff"), Ok(255));
    }
}
