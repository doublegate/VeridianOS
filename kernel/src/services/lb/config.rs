//! Load Balancer Configuration
//!
//! Provides configuration parsing and shell command integration
//! for the L4/L7 load balancers.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use super::l4::LbAlgorithm;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// VIP configuration entry.
#[derive(Debug, Clone)]
pub struct VipConfig {
    /// VIP address.
    pub address: String,
    /// VIP port.
    pub port: u16,
    /// Algorithm name.
    pub algorithm: LbAlgorithm,
    /// Backend addresses (addr:port).
    pub backends: Vec<String>,
}

/// Route configuration entry.
#[derive(Debug, Clone)]
pub struct RouteConfig {
    /// Path prefix.
    pub path_prefix: String,
    /// Host header.
    pub host: String,
    /// Backend group name.
    pub backend_group: String,
}

/// Load balancer configuration.
#[derive(Debug, Clone)]
pub struct LbConfig {
    /// VIP definitions.
    pub vips: Vec<VipConfig>,
    /// Route definitions.
    pub routes: Vec<RouteConfig>,
    /// Health check interval in ticks.
    pub health_check_interval: u64,
    /// Default rate limit (requests per second).
    pub rate_limit_rps: u32,
    /// Default rate limit burst.
    pub rate_limit_burst: u32,
}

impl Default for LbConfig {
    fn default() -> Self {
        LbConfig {
            vips: Vec::new(),
            routes: Vec::new(),
            health_check_interval: 30,
            rate_limit_rps: 100,
            rate_limit_burst: 200,
        }
    }
}

/// Config parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Invalid configuration line.
    InvalidLine(String),
    /// Missing required field.
    MissingField(String),
    /// Invalid port number.
    InvalidPort(String),
    /// Unknown algorithm.
    UnknownAlgorithm(String),
}

/// Parse a load balancer configuration from key=value format.
///
/// Recognized keys:
/// - `health_check_interval=N`
/// - `rate_limit_rps=N`
/// - `rate_limit_burst=N`
/// - `vip=addr:port:algorithm` (e.g., `vip=10.96.0.1:80:roundrobin`)
/// - `backend=vip_addr:vip_port:backend_addr:backend_port`
/// - `route=path_prefix:host:backend_group`
pub fn parse_config(input: &str) -> Result<LbConfig, ConfigError> {
    let mut config = LbConfig::default();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "health_check_interval" => {
                    config.health_check_interval = parse_u64(value)?;
                }
                "rate_limit_rps" => {
                    config.rate_limit_rps = parse_u32(value)?;
                }
                "rate_limit_burst" => {
                    config.rate_limit_burst = parse_u32(value)?;
                }
                "vip" => {
                    let vip = parse_vip_config(value)?;
                    config.vips.push(vip);
                }
                "route" => {
                    let route = parse_route_config(value)?;
                    config.routes.push(route);
                }
                _ => {
                    // Unknown keys are ignored
                }
            }
        }
    }

    Ok(config)
}

/// Parse a VIP config: "addr:port:algorithm"
fn parse_vip_config(value: &str) -> Result<VipConfig, ConfigError> {
    let parts: Vec<&str> = value.splitn(3, ':').collect();
    if parts.len() < 3 {
        return Err(ConfigError::MissingField(String::from(
            "vip requires addr:port:algorithm",
        )));
    }

    let address = String::from(parts[0]);
    let port = parse_u16(parts[1])?;
    let algorithm = parse_algorithm(parts[2])?;

    Ok(VipConfig {
        address,
        port,
        algorithm,
        backends: Vec::new(),
    })
}

/// Parse a route config: "path_prefix:host:backend_group"
fn parse_route_config(value: &str) -> Result<RouteConfig, ConfigError> {
    let parts: Vec<&str> = value.splitn(3, ':').collect();
    if parts.len() < 3 {
        return Err(ConfigError::MissingField(String::from(
            "route requires path:host:group",
        )));
    }

    Ok(RouteConfig {
        path_prefix: String::from(parts[0]),
        host: String::from(parts[1]),
        backend_group: String::from(parts[2]),
    })
}

/// Parse algorithm name.
fn parse_algorithm(s: &str) -> Result<LbAlgorithm, ConfigError> {
    match s {
        "roundrobin" | "rr" => Ok(LbAlgorithm::RoundRobin),
        "leastconn" | "lc" => Ok(LbAlgorithm::LeastConnections),
        "weighted" | "wrr" => Ok(LbAlgorithm::WeightedRoundRobin),
        "random" => Ok(LbAlgorithm::Random),
        "iphash" => Ok(LbAlgorithm::IpHash),
        _ => Err(ConfigError::UnknownAlgorithm(String::from(s))),
    }
}

/// Parse u64 from string.
fn parse_u64(s: &str) -> Result<u64, ConfigError> {
    let mut result: u64 = 0;
    for ch in s.bytes() {
        if !ch.is_ascii_digit() {
            return Err(ConfigError::InvalidLine(String::from(s)));
        }
        result = result
            .checked_mul(10)
            .and_then(|r| r.checked_add((ch - b'0') as u64))
            .ok_or_else(|| ConfigError::InvalidLine(String::from(s)))?;
    }
    Ok(result)
}

/// Parse u32 from string.
fn parse_u32(s: &str) -> Result<u32, ConfigError> {
    let v = parse_u64(s)?;
    if v > u32::MAX as u64 {
        return Err(ConfigError::InvalidLine(String::from(s)));
    }
    Ok(v as u32)
}

/// Parse u16 from string.
fn parse_u16(s: &str) -> Result<u16, ConfigError> {
    let v = parse_u64(s)?;
    if v > u16::MAX as u64 {
        return Err(ConfigError::InvalidPort(String::from(s)));
    }
    Ok(v as u16)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        let config = parse_config("").unwrap();
        assert!(config.vips.is_empty());
        assert_eq!(config.health_check_interval, 30);
    }

    #[test]
    fn test_parse_basic_config() {
        let input = "\
health_check_interval=60
rate_limit_rps=50
rate_limit_burst=100
vip=10.96.0.1:80:roundrobin
route=/api:example.com:api-group
";
        let config = parse_config(input).unwrap();
        assert_eq!(config.health_check_interval, 60);
        assert_eq!(config.rate_limit_rps, 50);
        assert_eq!(config.vips.len(), 1);
        assert_eq!(config.vips[0].port, 80);
        assert_eq!(config.vips[0].algorithm, LbAlgorithm::RoundRobin);
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].path_prefix, "/api");
    }

    #[test]
    fn test_parse_algorithms() {
        assert_eq!(
            parse_algorithm("roundrobin").unwrap(),
            LbAlgorithm::RoundRobin
        );
        assert_eq!(
            parse_algorithm("leastconn").unwrap(),
            LbAlgorithm::LeastConnections
        );
        assert_eq!(parse_algorithm("iphash").unwrap(), LbAlgorithm::IpHash);
        assert!(parse_algorithm("unknown").is_err());
    }

    #[test]
    fn test_parse_with_comments() {
        let input = "# comment\nhealth_check_interval=10\n# another\n";
        let config = parse_config(input).unwrap();
        assert_eq!(config.health_check_interval, 10);
    }
}
