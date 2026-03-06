//! Container Networking - virtual Ethernet pairs, bridge, NAT masquerade, ARP
//! proxy.

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use crate::error::KernelError;

/// Virtual Ethernet interface state.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VethEndpoint {
    /// Interface name.
    pub name: String,
    /// Peer interface name.
    pub peer_name: String,
    /// MAC address (6 bytes).
    pub mac: [u8; 6],
    /// IPv4 address (network byte order).
    pub ipv4_addr: u32,
    /// IPv4 subnet mask (network byte order).
    pub ipv4_mask: u32,
    /// MTU in bytes (default 1500).
    pub mtu: u16,
    /// Whether the interface is up.
    pub is_up: bool,
    /// Namespace ID this endpoint belongs to (0 = host).
    pub namespace_id: u64,
}

/// A virtual Ethernet pair.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct VethPair {
    /// Host-side endpoint.
    pub host: VethEndpoint,
    /// Container-side endpoint.
    pub container: VethEndpoint,
}

/// NAT port mapping entry.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatPortMapping {
    /// External (host) port.
    pub external_port: u16,
    /// Internal (container) port.
    pub internal_port: u16,
    /// Protocol: 6=TCP, 17=UDP.
    pub protocol: u8,
    /// Container IPv4 address.
    pub container_ip: u32,
}

/// NAT masquerade table for outbound SNAT and inbound port forwarding.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct NatTable {
    /// Host external IP address.
    pub host_ip: u32,
    /// Port mappings for inbound (DNAT).
    pub port_mappings: Vec<NatPortMapping>,
    /// Whether SNAT masquerade is enabled.
    pub masquerade_enabled: bool,
}

#[cfg(feature = "alloc")]
impl NatTable {
    pub fn new(host_ip: u32) -> Self {
        Self {
            host_ip,
            port_mappings: Vec::new(),
            masquerade_enabled: false,
        }
    }

    /// Enable SNAT masquerade for outbound traffic.
    pub fn enable_masquerade(&mut self) {
        self.masquerade_enabled = true;
    }

    /// Add a port mapping for inbound traffic.
    pub fn add_port_mapping(&mut self, mapping: NatPortMapping) -> Result<(), KernelError> {
        // Check for duplicate external port + protocol
        for existing in &self.port_mappings {
            if existing.external_port == mapping.external_port
                && existing.protocol == mapping.protocol
            {
                return Err(KernelError::AlreadyExists {
                    resource: "nat port mapping",
                    id: mapping.external_port as u64,
                });
            }
        }
        self.port_mappings.push(mapping);
        Ok(())
    }

    /// Remove a port mapping.
    pub fn remove_port_mapping(&mut self, external_port: u16, protocol: u8) -> bool {
        let before = self.port_mappings.len();
        self.port_mappings
            .retain(|m| !(m.external_port == external_port && m.protocol == protocol));
        self.port_mappings.len() < before
    }

    /// Look up a port mapping for inbound traffic.
    pub fn lookup_inbound(&self, external_port: u16, protocol: u8) -> Option<&NatPortMapping> {
        self.port_mappings
            .iter()
            .find(|m| m.external_port == external_port && m.protocol == protocol)
    }

    /// Apply SNAT: rewrite source IP to host IP for outbound packets.
    pub fn snat_rewrite(&self, _src_ip: u32) -> Option<u32> {
        if self.masquerade_enabled {
            Some(self.host_ip)
        } else {
            None
        }
    }
}

/// ARP proxy entry for container IPs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpProxyEntry {
    /// IPv4 address to proxy.
    pub ip: u32,
    /// MAC address to respond with.
    pub mac: [u8; 6],
}

/// Bridge configuration for container networking.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct VethBridge {
    /// Bridge name.
    pub name: String,
    /// Bridge IPv4 address (gateway).
    pub bridge_ip: u32,
    /// Bridge subnet mask.
    pub subnet_mask: u32,
    /// Attached veth host-side endpoint names.
    pub attached_interfaces: Vec<String>,
    /// ARP proxy entries.
    pub arp_proxy_entries: Vec<ArpProxyEntry>,
    /// NAT table.
    pub nat: NatTable,
}

#[cfg(feature = "alloc")]
impl VethBridge {
    pub fn new(name: &str, bridge_ip: u32, subnet_mask: u32) -> Self {
        Self {
            name: String::from(name),
            bridge_ip,
            subnet_mask,
            attached_interfaces: Vec::new(),
            arp_proxy_entries: Vec::new(),
            nat: NatTable::new(bridge_ip),
        }
    }

    /// Attach a host-side veth endpoint to the bridge.
    pub fn attach(&mut self, interface_name: &str) {
        if !self.attached_interfaces.iter().any(|n| n == interface_name) {
            self.attached_interfaces.push(String::from(interface_name));
        }
    }

    /// Detach an interface from the bridge.
    pub fn detach(&mut self, interface_name: &str) {
        self.attached_interfaces.retain(|n| n != interface_name);
    }

    /// Add an ARP proxy entry.
    pub fn add_arp_proxy(&mut self, entry: ArpProxyEntry) {
        self.arp_proxy_entries.push(entry);
    }

    /// Look up an ARP proxy entry by IP.
    pub fn arp_lookup(&self, ip: u32) -> Option<&ArpProxyEntry> {
        self.arp_proxy_entries.iter().find(|e| e.ip == ip)
    }

    /// Check if an IP is within the bridge subnet.
    pub fn in_subnet(&self, ip: u32) -> bool {
        (ip & self.subnet_mask) == (self.bridge_ip & self.subnet_mask)
    }

    pub fn attached_count(&self) -> usize {
        self.attached_interfaces.len()
    }
}

static NEXT_VETH_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a deterministic MAC address from a veth pair ID.
pub fn generate_veth_mac(veth_id: u64) -> [u8; 6] {
    [
        0x02, // locally administered
        0x42,
        ((veth_id >> 24) & 0xff) as u8,
        ((veth_id >> 16) & 0xff) as u8,
        ((veth_id >> 8) & 0xff) as u8,
        (veth_id & 0xff) as u8,
    ]
}

/// Create a veth pair with generated MACs.
#[cfg(feature = "alloc")]
pub fn create_veth_pair(host_name: &str, container_name: &str, namespace_id: u64) -> VethPair {
    let id = NEXT_VETH_ID.fetch_add(1, Ordering::Relaxed);
    let host_mac = generate_veth_mac(id);
    // Container MAC: flip one bit to differentiate
    let mut container_mac = generate_veth_mac(id);
    container_mac[5] ^= 0x01;

    VethPair {
        host: VethEndpoint {
            name: String::from(host_name),
            peer_name: String::from(container_name),
            mac: host_mac,
            ipv4_addr: 0,
            ipv4_mask: 0,
            mtu: 1500,
            is_up: false,
            namespace_id: 0,
        },
        container: VethEndpoint {
            name: String::from(container_name),
            peer_name: String::from(host_name),
            mac: container_mac,
            ipv4_addr: 0,
            ipv4_mask: 0,
            mtu: 1500,
            is_up: false,
            namespace_id,
        },
    }
}
