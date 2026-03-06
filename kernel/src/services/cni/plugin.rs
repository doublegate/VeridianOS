//! CNI Plugin Framework
//!
//! Provides a plugin-based container networking interface with bridge
//! configuration, veth pair creation, and NAT setup.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// CNI Config
// ---------------------------------------------------------------------------

/// CNI plugin configuration.
#[derive(Debug, Clone)]
pub struct CniConfig {
    /// CNI specification version.
    pub cni_version: String,
    /// Network name.
    pub name: String,
    /// Plugin type name.
    pub type_name: String,
    /// Bridge device name (for bridge plugin).
    pub bridge: String,
    /// Subnet in CIDR notation (e.g., "10.244.0.0/24").
    pub subnet: String,
    /// Gateway address (e.g., "10.244.0.1").
    pub gateway: String,
    /// Additional plugin-specific options.
    pub options: BTreeMap<String, String>,
}

impl Default for CniConfig {
    fn default() -> Self {
        CniConfig {
            cni_version: String::from("1.0.0"),
            name: String::from("veridian-net"),
            type_name: String::from("bridge"),
            bridge: String::from("cni0"),
            subnet: String::from("10.244.0.0/24"),
            gateway: String::from("10.244.0.1"),
            options: BTreeMap::new(),
        }
    }
}

impl CniConfig {
    /// Parse a CNI config from a simple key=value format.
    ///
    /// Each line is `key=value`. Recognized keys:
    /// cniVersion, name, type, bridge, subnet, gateway.
    pub fn from_key_value(input: &str) -> Self {
        let mut config = CniConfig::default();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "cniVersion" => config.cni_version = String::from(value),
                    "name" => config.name = String::from(value),
                    "type" => config.type_name = String::from(value),
                    "bridge" => config.bridge = String::from(value),
                    "subnet" => config.subnet = String::from(value),
                    "gateway" => config.gateway = String::from(value),
                    _ => {
                        config
                            .options
                            .insert(String::from(key), String::from(value));
                    }
                }
            }
        }
        config
    }
}

// ---------------------------------------------------------------------------
// CNI Result
// ---------------------------------------------------------------------------

/// A network interface in the CNI result.
#[derive(Debug, Clone)]
pub struct CniInterface {
    /// Interface name (e.g., "eth0").
    pub name: String,
    /// MAC address (e.g., "02:42:ac:11:00:02").
    pub mac: String,
    /// Whether this is inside the container sandbox.
    pub sandbox: bool,
}

/// An IP address assignment in the CNI result.
#[derive(Debug, Clone)]
pub struct CniIpConfig {
    /// IP address with prefix (e.g., "10.244.0.5/24").
    pub address: String,
    /// Gateway address.
    pub gateway: String,
    /// Interface index this IP is assigned to.
    pub interface_idx: usize,
}

/// A route in the CNI result.
#[derive(Debug, Clone)]
pub struct CniRoute {
    /// Destination CIDR (e.g., "0.0.0.0/0" for default).
    pub dst: String,
    /// Gateway address.
    pub gw: String,
}

/// DNS configuration in the CNI result.
#[derive(Debug, Clone, Default)]
pub struct CniDns {
    /// DNS nameservers.
    pub nameservers: Vec<String>,
    /// Search domains.
    pub search: Vec<String>,
}

/// Result returned from a CNI plugin operation.
#[derive(Debug, Clone)]
pub struct CniResult {
    /// CNI version.
    pub cni_version: String,
    /// Created interfaces.
    pub interfaces: Vec<CniInterface>,
    /// Assigned IP addresses.
    pub ips: Vec<CniIpConfig>,
    /// Routes added.
    pub routes: Vec<CniRoute>,
    /// DNS configuration.
    pub dns: CniDns,
}

impl Default for CniResult {
    fn default() -> Self {
        CniResult {
            cni_version: String::from("1.0.0"),
            interfaces: Vec::new(),
            ips: Vec::new(),
            routes: Vec::new(),
            dns: CniDns::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// CNI Plugin Trait
// ---------------------------------------------------------------------------

/// CNI plugin error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CniError {
    /// Configuration error.
    InvalidConfig(String),
    /// Network setup failed.
    SetupFailed(String),
    /// Network teardown failed.
    TeardownFailed(String),
    /// Plugin not found.
    PluginNotFound(String),
    /// Address exhaustion.
    NoAddressAvailable,
}

/// Trait for CNI plugin implementations.
pub trait CniPlugin {
    /// Add a container to the network.
    fn add(&mut self, container_id: &str, config: &CniConfig) -> Result<CniResult, CniError>;

    /// Remove a container from the network.
    fn del(&mut self, container_id: &str, config: &CniConfig) -> Result<(), CniError>;

    /// Check that a container's networking is correct.
    fn check(&self, container_id: &str, config: &CniConfig) -> Result<(), CniError>;

    /// Return supported CNI versions.
    fn version(&self) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Bridge Plugin
// ---------------------------------------------------------------------------

/// Veth pair representing a container-to-bridge link.
#[derive(Debug, Clone)]
pub struct VethPair {
    /// Host-side veth name.
    pub host_name: String,
    /// Container-side veth name.
    pub container_name: String,
    /// Container ID.
    pub container_id: String,
    /// Assigned IP address.
    pub ip_address: String,
    /// MAC address.
    pub mac_address: String,
}

/// Bridge plugin: creates veth pairs and attaches them to a bridge device.
#[derive(Debug)]
pub struct BridgePlugin {
    /// Bridge device name.
    bridge_name: String,
    /// Active veth pairs keyed by container ID.
    veth_pairs: BTreeMap<String, VethPair>,
    /// Next IP host part to assign.
    next_host_part: u32,
    /// Next veth index for naming.
    next_veth_idx: u32,
}

impl Default for BridgePlugin {
    fn default() -> Self {
        Self::new(String::from("cni0"))
    }
}

impl BridgePlugin {
    /// Create a new bridge plugin.
    pub fn new(bridge_name: String) -> Self {
        BridgePlugin {
            bridge_name,
            veth_pairs: BTreeMap::new(),
            next_host_part: 2, // .1 is gateway
            next_veth_idx: 0,
        }
    }

    /// Create a veth pair for a container.
    fn setup_veth(&mut self, container_id: &str) -> VethPair {
        let idx = self.next_veth_idx;
        self.next_veth_idx += 1;

        let host_name = alloc::format!("veth{:04x}", idx);
        let container_name = String::from("eth0");

        // Generate deterministic MAC
        let mac_address =
            alloc::format!("02:42:ac:11:{:02x}:{:02x}", (idx >> 8) & 0xFF, idx & 0xFF);

        VethPair {
            host_name,
            container_name,
            container_id: String::from(container_id),
            ip_address: String::new(), // filled in by add()
            mac_address,
        }
    }

    /// Attach a veth to the bridge (conceptual).
    fn attach_to_bridge(&self, _veth: &VethPair) -> Result<(), CniError> {
        // In a real implementation this would call netlink
        Ok(())
    }

    /// Configure NAT for outbound traffic (conceptual).
    fn configure_nat(&self, _subnet: &str) -> Result<(), CniError> {
        // In a real implementation this would set up iptables/nftables rules
        Ok(())
    }

    /// Allocate the next IP address from the subnet.
    fn allocate_ip(&mut self, config: &CniConfig) -> Result<String, CniError> {
        // Parse subnet base (simplified: assume /24 with "x.y.z.0/24" format)
        let subnet = &config.subnet;
        let slash_pos = subnet.find('/').unwrap_or(subnet.len());
        let base = &subnet[..slash_pos];

        // Find last dot to replace host part
        if let Some(last_dot) = base.rfind('.') {
            let prefix = &base[..last_dot];
            let host_part = self.next_host_part;
            if host_part > 254 {
                return Err(CniError::NoAddressAvailable);
            }
            self.next_host_part += 1;
            Ok(alloc::format!("{}.{}/24", prefix, host_part))
        } else {
            Err(CniError::InvalidConfig(String::from(
                "invalid subnet format",
            )))
        }
    }

    /// Get the number of active veth pairs.
    pub fn active_count(&self) -> usize {
        self.veth_pairs.len()
    }
}

impl CniPlugin for BridgePlugin {
    fn add(&mut self, container_id: &str, config: &CniConfig) -> Result<CniResult, CniError> {
        // Check for duplicate
        if self.veth_pairs.contains_key(container_id) {
            return Err(CniError::SetupFailed(String::from(
                "container already attached",
            )));
        }

        let mut veth = self.setup_veth(container_id);
        let ip_address = self.allocate_ip(config)?;
        veth.ip_address = ip_address.clone();

        self.attach_to_bridge(&veth)?;
        self.configure_nat(&config.subnet)?;

        let result = CniResult {
            cni_version: config.cni_version.clone(),
            interfaces: alloc::vec![
                CniInterface {
                    name: veth.host_name.clone(),
                    mac: String::new(),
                    sandbox: false,
                },
                CniInterface {
                    name: veth.container_name.clone(),
                    mac: veth.mac_address.clone(),
                    sandbox: true,
                },
            ],
            ips: alloc::vec![CniIpConfig {
                address: ip_address,
                gateway: config.gateway.clone(),
                interface_idx: 1,
            }],
            routes: alloc::vec![CniRoute {
                dst: String::from("0.0.0.0/0"),
                gw: config.gateway.clone(),
            }],
            dns: CniDns::default(),
        };

        self.veth_pairs.insert(String::from(container_id), veth);
        Ok(result)
    }

    fn del(&mut self, container_id: &str, _config: &CniConfig) -> Result<(), CniError> {
        if self.veth_pairs.remove(container_id).is_none() {
            return Err(CniError::TeardownFailed(String::from(
                "container not found",
            )));
        }
        Ok(())
    }

    fn check(&self, container_id: &str, _config: &CniConfig) -> Result<(), CniError> {
        if self.veth_pairs.contains_key(container_id) {
            Ok(())
        } else {
            Err(CniError::SetupFailed(String::from(
                "container not attached",
            )))
        }
    }

    fn version(&self) -> Vec<String> {
        alloc::vec![
            String::from("0.3.0"),
            String::from("0.3.1"),
            String::from("0.4.0"),
            String::from("1.0.0"),
        ]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;

    use super::*;

    fn default_config() -> CniConfig {
        CniConfig::default()
    }

    #[test]
    fn test_cni_config_default() {
        let config = CniConfig::default();
        assert_eq!(config.type_name, "bridge");
        assert_eq!(config.subnet, "10.244.0.0/24");
    }

    #[test]
    fn test_cni_config_parse() {
        let input = "name=my-net\ntype=bridge\nsubnet=10.0.0.0/16\ngateway=10.0.0.1\n";
        let config = CniConfig::from_key_value(input);
        assert_eq!(config.name, "my-net");
        assert_eq!(config.subnet, "10.0.0.0/16");
        assert_eq!(config.gateway, "10.0.0.1");
    }

    #[test]
    fn test_bridge_add() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        let result = plugin.add("container1", &config).unwrap();
        assert_eq!(result.interfaces.len(), 2);
        assert_eq!(result.ips.len(), 1);
        assert!(result.ips[0].address.contains("10.244.0."));
        assert_eq!(result.routes.len(), 1);
    }

    #[test]
    fn test_bridge_add_duplicate() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        plugin.add("c1", &config).unwrap();
        let result = plugin.add("c1", &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_bridge_del() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        plugin.add("c1", &config).unwrap();
        plugin.del("c1", &config).unwrap();
        assert_eq!(plugin.active_count(), 0);
    }

    #[test]
    fn test_bridge_del_not_found() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        assert!(plugin.del("nonexistent", &config).is_err());
    }

    #[test]
    fn test_bridge_check() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        plugin.add("c1", &config).unwrap();
        assert!(plugin.check("c1", &config).is_ok());
        assert!(plugin.check("c2", &config).is_err());
    }

    #[test]
    fn test_bridge_version() {
        let plugin = BridgePlugin::default();
        let versions = plugin.version();
        assert!(versions.contains(&String::from("1.0.0")));
    }

    #[test]
    fn test_multiple_containers_get_different_ips() {
        let mut plugin = BridgePlugin::default();
        let config = default_config();
        let r1 = plugin.add("c1", &config).unwrap();
        let r2 = plugin.add("c2", &config).unwrap();
        assert_ne!(r1.ips[0].address, r2.ips[0].address);
    }

    #[test]
    fn test_config_parse_with_comments() {
        let input = "# comment\nname=test\n\ntype=bridge\n";
        let config = CniConfig::from_key_value(input);
        assert_eq!(config.name, "test");
        assert_eq!(config.type_name, "bridge");
    }
}
