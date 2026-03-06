//! L7 (Application Layer) Load Balancer
//!
//! Provides HTTP-aware load balancing with path/host-based routing,
//! rate limiting, and sticky sessions.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use super::l4::{Backend, LbAlgorithm};

// ---------------------------------------------------------------------------
// HTTP Route
// ---------------------------------------------------------------------------

/// An HTTP routing rule.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HttpRoute {
    /// Path prefix to match (e.g., "/api/v1").
    pub path_prefix: String,
    /// Host header to match (empty = any).
    pub host: String,
    /// Required headers (all must match).
    pub headers: BTreeMap<String, String>,
    /// Name of the backend group to route to.
    pub backend_group: String,
}

impl HttpRoute {
    /// Check if a request matches this route.
    pub fn matches(&self, path: &str, host: &str, headers: &BTreeMap<String, String>) -> bool {
        // Check path prefix
        if !path.starts_with(&self.path_prefix) {
            return false;
        }
        // Check host (empty = any)
        if !self.host.is_empty() && self.host != host {
            return false;
        }
        // Check required headers
        for (key, value) in &self.headers {
            match headers.get(key) {
                Some(v) if v == value => {}
                _ => return false,
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Backend Group
// ---------------------------------------------------------------------------

/// A named group of backends.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BackendGroup {
    /// Group name.
    pub name: String,
    /// Backends in this group.
    pub backends: Vec<Backend>,
    /// Load balancing algorithm for this group.
    pub algorithm: LbAlgorithm,
    /// Round-robin counter.
    rr_counter: u64,
}

impl BackendGroup {
    /// Create a new backend group.
    pub fn new(name: String, algorithm: LbAlgorithm) -> Self {
        BackendGroup {
            name,
            backends: Vec::new(),
            algorithm,
            rr_counter: 0,
        }
    }

    /// Select a healthy backend.
    pub fn select(&mut self) -> Option<(String, u16)> {
        let healthy: Vec<usize> = self
            .backends
            .iter()
            .enumerate()
            .filter(|(_, b)| b.healthy)
            .map(|(i, _)| i)
            .collect();

        if healthy.is_empty() {
            return None;
        }

        let idx = match self.algorithm {
            LbAlgorithm::RoundRobin | LbAlgorithm::WeightedRoundRobin => {
                let i = (self.rr_counter as usize) % healthy.len();
                self.rr_counter += 1;
                healthy[i]
            }
            LbAlgorithm::LeastConnections => {
                let mut min_idx = healthy[0];
                let mut min_conn = self.backends[healthy[0]].active_connections;
                for &h in &healthy[1..] {
                    if self.backends[h].active_connections < min_conn {
                        min_conn = self.backends[h].active_connections;
                        min_idx = h;
                    }
                }
                min_idx
            }
            _ => {
                let i = (self.rr_counter as usize) % healthy.len();
                self.rr_counter += 1;
                healthy[i]
            }
        };

        self.backends[idx].active_connections += 1;
        self.backends[idx].total_requests += 1;
        Some((self.backends[idx].address.clone(), self.backends[idx].port))
    }
}

// ---------------------------------------------------------------------------
// L7 Rule
// ---------------------------------------------------------------------------

/// A set of L7 routing rules.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct L7Rule {
    /// Ordered routes (first match wins).
    pub routes: Vec<HttpRoute>,
    /// Default backend group (if no route matches).
    pub default_backend: String,
}

// ---------------------------------------------------------------------------
// Rate Limiting
// ---------------------------------------------------------------------------

/// Token bucket rate limiter.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RateLimit {
    /// Maximum requests per second (integer).
    pub requests_per_second: u32,
    /// Burst size.
    pub burst: u32,
    /// Current available tokens.
    pub current_tokens: u32,
    /// Tick when tokens were last refilled.
    pub last_refill_tick: u64,
}

impl RateLimit {
    /// Create a new rate limiter.
    pub fn new(requests_per_second: u32, burst: u32) -> Self {
        RateLimit {
            requests_per_second,
            burst,
            current_tokens: burst,
            last_refill_tick: 0,
        }
    }

    /// Try to consume a token. Returns true if allowed.
    pub fn allow(&mut self, current_tick: u64) -> bool {
        self.refill(current_tick);
        if self.current_tokens > 0 {
            self.current_tokens -= 1;
            true
        } else {
            false
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self, current_tick: u64) {
        if current_tick <= self.last_refill_tick {
            return;
        }
        let elapsed = current_tick - self.last_refill_tick;
        let new_tokens = elapsed.saturating_mul(self.requests_per_second as u64);
        // Cap at burst, but clamp to u32 first
        let clamped = if new_tokens > self.burst as u64 {
            self.burst
        } else {
            new_tokens as u32
        };
        self.current_tokens = self.current_tokens.saturating_add(clamped).min(self.burst);
        self.last_refill_tick = current_tick;
    }
}

// ---------------------------------------------------------------------------
// Sticky Sessions
// ---------------------------------------------------------------------------

/// Sticky session manager (cookie-based affinity).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StickySession {
    /// Cookie name used for session affinity.
    pub cookie_name: String,
    /// Session ID -> backend index mapping.
    pub backend_map: BTreeMap<String, usize>,
}

impl StickySession {
    /// Create a new sticky session manager.
    pub fn new(cookie_name: String) -> Self {
        StickySession {
            cookie_name,
            backend_map: BTreeMap::new(),
        }
    }

    /// Get the backend index for a session.
    pub fn get_backend(&self, session_id: &str) -> Option<usize> {
        self.backend_map.get(session_id).copied()
    }

    /// Set the backend for a session.
    pub fn set_backend(&mut self, session_id: String, backend_idx: usize) {
        self.backend_map.insert(session_id, backend_idx);
    }

    /// Remove a session.
    pub fn remove_session(&mut self, session_id: &str) {
        self.backend_map.remove(session_id);
    }

    /// Get the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.backend_map.len()
    }
}

// ---------------------------------------------------------------------------
// L7 Error
// ---------------------------------------------------------------------------

/// L7 load balancer error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum L7Error {
    /// No route matched.
    NoRouteMatch,
    /// Backend group not found.
    BackendGroupNotFound(String),
    /// No healthy backend.
    NoHealthyBackend,
    /// Rate limit exceeded.
    RateLimitExceeded,
}

// ---------------------------------------------------------------------------
// L7 Load Balancer
// ---------------------------------------------------------------------------

/// L7 Load Balancer implementation.
#[derive(Debug)]
#[allow(dead_code)]
pub struct L7LoadBalancer {
    /// Routing rules.
    rules: Vec<L7Rule>,
    /// Backend groups.
    backend_groups: BTreeMap<String, BackendGroup>,
    /// Rate limiters keyed by identifier (e.g., client IP).
    rate_limiters: BTreeMap<String, RateLimit>,
    /// Sticky session manager.
    sticky_sessions: StickySession,
    /// Default rate limit config.
    default_rps: u32,
    /// Default burst.
    default_burst: u32,
}

impl Default for L7LoadBalancer {
    fn default() -> Self {
        Self::new()
    }
}

impl L7LoadBalancer {
    /// Create a new L7 load balancer.
    pub fn new() -> Self {
        L7LoadBalancer {
            rules: Vec::new(),
            backend_groups: BTreeMap::new(),
            rate_limiters: BTreeMap::new(),
            sticky_sessions: StickySession::new(String::from("VERIDIAN_SESSION")),
            default_rps: 100,
            default_burst: 200,
        }
    }

    /// Add a routing rule.
    pub fn add_rule(&mut self, rule: L7Rule) {
        self.rules.push(rule);
    }

    /// Add a backend group.
    pub fn add_backend_group(&mut self, group: BackendGroup) {
        self.backend_groups.insert(group.name.clone(), group);
    }

    /// Route an HTTP request to a backend.
    pub fn route_request(
        &mut self,
        path: &str,
        host: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<(String, u16), L7Error> {
        // Find matching route
        let mut target_group = None;

        for rule in &self.rules {
            for route in &rule.routes {
                if route.matches(path, host, headers) {
                    target_group = Some(route.backend_group.clone());
                    break;
                }
            }
            if target_group.is_some() {
                break;
            }
            // Use default backend from rule if no route matched
            if target_group.is_none() && !rule.default_backend.is_empty() {
                target_group = Some(rule.default_backend.clone());
            }
        }

        let group_name = target_group.ok_or(L7Error::NoRouteMatch)?;

        let group = self
            .backend_groups
            .get_mut(&group_name)
            .ok_or_else(|| L7Error::BackendGroupNotFound(group_name.clone()))?;

        group.select().ok_or(L7Error::NoHealthyBackend)
    }

    /// Check rate limit for a client.
    pub fn check_rate_limit(&mut self, client_id: &str, current_tick: u64) -> Result<(), L7Error> {
        let limiter = self
            .rate_limiters
            .entry(String::from(client_id))
            .or_insert_with(|| RateLimit::new(self.default_rps, self.default_burst));

        if limiter.allow(current_tick) {
            Ok(())
        } else {
            Err(L7Error::RateLimitExceeded)
        }
    }

    /// Get sticky backend for a session.
    pub fn get_sticky_backend(&self, session_id: &str) -> Option<usize> {
        self.sticky_sessions.get_backend(session_id)
    }

    /// Set sticky backend for a session.
    pub fn set_sticky_backend(&mut self, session_id: String, backend_idx: usize) {
        self.sticky_sessions.set_backend(session_id, backend_idx);
    }

    /// Get the number of backend groups.
    pub fn backend_group_count(&self) -> usize {
        self.backend_groups.len()
    }

    /// Get the number of routing rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_lb() -> L7LoadBalancer {
        let mut lb = L7LoadBalancer::new();

        let mut group = BackendGroup::new(String::from("api-backends"), LbAlgorithm::RoundRobin);
        group
            .backends
            .push(Backend::new(String::from("10.0.0.1"), 8080, 1));
        group
            .backends
            .push(Backend::new(String::from("10.0.0.2"), 8080, 1));
        lb.add_backend_group(group);

        let mut default_group =
            BackendGroup::new(String::from("default-backends"), LbAlgorithm::RoundRobin);
        default_group
            .backends
            .push(Backend::new(String::from("10.0.1.1"), 80, 1));
        lb.add_backend_group(default_group);

        lb.add_rule(L7Rule {
            routes: vec![HttpRoute {
                path_prefix: String::from("/api/"),
                host: String::new(),
                headers: BTreeMap::new(),
                backend_group: String::from("api-backends"),
            }],
            default_backend: String::from("default-backends"),
        });

        lb
    }

    #[test]
    fn test_route_by_path() {
        let mut lb = make_lb();
        let headers = BTreeMap::new();
        let (addr, port) = lb
            .route_request("/api/v1/pods", "example.com", &headers)
            .unwrap();
        assert_eq!(port, 8080);
        assert!(addr.starts_with("10.0.0."));
    }

    #[test]
    fn test_route_default_backend() {
        let mut lb = make_lb();
        let headers = BTreeMap::new();
        let (addr, port) = lb
            .route_request("/other/page", "example.com", &headers)
            .unwrap();
        assert_eq!(addr, "10.0.1.1");
        assert_eq!(port, 80);
    }

    #[test]
    fn test_route_with_host_match() {
        let mut lb = L7LoadBalancer::new();
        let mut group = BackendGroup::new(String::from("host-group"), LbAlgorithm::RoundRobin);
        group
            .backends
            .push(Backend::new(String::from("10.0.0.5"), 443, 1));
        lb.add_backend_group(group);

        lb.add_rule(L7Rule {
            routes: vec![HttpRoute {
                path_prefix: String::from("/"),
                host: String::from("api.example.com"),
                headers: BTreeMap::new(),
                backend_group: String::from("host-group"),
            }],
            default_backend: String::new(),
        });

        let headers = BTreeMap::new();
        assert!(lb
            .route_request("/", "other.example.com", &headers)
            .is_err());
        let result = lb.route_request("/", "api.example.com", &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_rate_limit_allow() {
        let mut lb = make_lb();
        assert!(lb.check_rate_limit("client-1", 100).is_ok());
    }

    #[test]
    fn test_rate_limit_exceeded() {
        let mut lb = L7LoadBalancer::new();
        lb.default_rps = 1;
        lb.default_burst = 1;

        lb.check_rate_limit("client-1", 100).unwrap();
        assert_eq!(
            lb.check_rate_limit("client-1", 100),
            Err(L7Error::RateLimitExceeded)
        );
    }

    #[test]
    fn test_rate_limit_refill() {
        let mut lb = L7LoadBalancer::new();
        lb.default_rps = 1;
        lb.default_burst = 2;

        lb.check_rate_limit("c1", 100).unwrap();
        lb.check_rate_limit("c1", 100).unwrap();
        assert!(lb.check_rate_limit("c1", 100).is_err());
        // After 1 tick, should get 1 more token
        assert!(lb.check_rate_limit("c1", 101).is_ok());
    }

    #[test]
    fn test_sticky_session() {
        let mut lb = make_lb();
        lb.set_sticky_backend(String::from("sess-123"), 0);
        assert_eq!(lb.get_sticky_backend("sess-123"), Some(0));
        assert_eq!(lb.get_sticky_backend("sess-999"), None);
    }

    #[test]
    fn test_http_route_matching() {
        let route = HttpRoute {
            path_prefix: String::from("/api/"),
            host: String::from("example.com"),
            headers: BTreeMap::new(),
            backend_group: String::from("test"),
        };
        let empty_headers = BTreeMap::new();
        assert!(route.matches("/api/v1", "example.com", &empty_headers));
        assert!(!route.matches("/web/", "example.com", &empty_headers));
        assert!(!route.matches("/api/v1", "other.com", &empty_headers));
    }

    #[test]
    fn test_http_route_header_matching() {
        let mut required = BTreeMap::new();
        required.insert(String::from("x-version"), String::from("v2"));
        let route = HttpRoute {
            path_prefix: String::from("/"),
            host: String::new(),
            headers: required,
            backend_group: String::from("test"),
        };

        let mut headers = BTreeMap::new();
        headers.insert(String::from("x-version"), String::from("v2"));
        assert!(route.matches("/", "", &headers));

        let empty_headers = BTreeMap::new();
        assert!(!route.matches("/", "", &empty_headers));
    }

    #[test]
    fn test_backend_group_select() {
        let mut group = BackendGroup::new(String::from("test"), LbAlgorithm::RoundRobin);
        group
            .backends
            .push(Backend::new(String::from("10.0.0.1"), 80, 1));
        let result = group.select();
        assert!(result.is_some());
    }

    #[test]
    fn test_no_route_match() {
        let mut lb = L7LoadBalancer::new();
        let headers = BTreeMap::new();
        assert_eq!(
            lb.route_request("/", "", &headers),
            Err(L7Error::NoRouteMatch)
        );
    }
}
