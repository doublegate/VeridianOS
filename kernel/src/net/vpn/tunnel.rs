//! Virtual Tunnel Interface (TUN/TAP)
//!
//! Provides L3 (TUN) and L2 (TAP) virtual network interfaces for VPN tunneling.
//! Supports packet queuing, MTU management, route injection, and encapsulation
//! headers for outer tunnel transport.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::net::Ipv4Address;

// ── Constants ────────────────────────────────────────────────────────────────

/// Default MTU for tunnel interfaces
const DEFAULT_MTU: u16 = 1500;

/// Maximum tunnel name length
const MAX_NAME_LEN: usize = 16;

/// Maximum number of packets in a queue
const MAX_QUEUE_DEPTH: usize = 256;

/// Maximum number of tunnel interfaces
const MAX_TUNNELS: usize = 64;

/// Maximum number of routes per tunnel
const MAX_ROUTES: usize = 128;

// ── Tunnel Types ─────────────────────────────────────────────────────────────

/// Tunnel device type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunnelType {
    /// Layer 3 tunnel (IP packets only)
    #[default]
    Tun,
    /// Layer 2 tunnel (full Ethernet frames)
    Tap,
}

/// Tunnel interface state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunnelState {
    /// Interface is down / not active
    #[default]
    Down,
    /// Interface is up and operational
    Up,
}

/// Encapsulation protocol for the outer tunnel transport
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncapProtocol {
    /// UDP encapsulation (most common for VPN)
    #[default]
    Udp,
    /// TCP encapsulation (for restricted networks)
    Tcp,
}

/// Tunnel error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelError {
    /// Tunnel interface already exists
    AlreadyExists,
    /// Tunnel interface not found
    NotFound,
    /// Tunnel interface is not up
    NotUp,
    /// Tunnel interface is already up
    AlreadyUp,
    /// Packet exceeds MTU
    PacketTooLarge,
    /// Queue is full
    QueueFull,
    /// Queue is empty (no packets available)
    QueueEmpty,
    /// Maximum number of tunnels reached
    TooManyTunnels,
    /// Maximum number of routes reached
    TooManyRoutes,
    /// Route already exists
    RouteExists,
    /// Route not found
    RouteNotFound,
    /// Invalid tunnel name (empty or too long)
    InvalidName,
    /// Invalid MTU value
    InvalidMtu,
}

// ── Tunnel Statistics ────────────────────────────────────────────────────────

/// Statistics for a tunnel interface
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TunnelStats {
    /// Total packets transmitted
    pub tx_packets: u64,
    /// Total packets received
    pub rx_packets: u64,
    /// Total bytes transmitted
    pub tx_bytes: u64,
    /// Total bytes received
    pub rx_bytes: u64,
    /// Transmit errors (e.g., MTU exceeded, queue full)
    pub tx_errors: u64,
    /// Receive errors
    pub rx_errors: u64,
}

// ── Tunnel Configuration ─────────────────────────────────────────────────────

/// Configuration for a tunnel interface
#[derive(Debug, Clone, PartialEq)]
pub struct TunnelConfig {
    /// Interface name (e.g., "tun0", "tap0")
    pub name: String,
    /// Tunnel type (L3 TUN or L2 TAP)
    pub tunnel_type: TunnelType,
    /// Maximum transmission unit
    pub mtu: u16,
    /// Local IP address assigned to this tunnel
    pub local_address: Ipv4Address,
    /// Remote peer IP address
    pub peer_address: Ipv4Address,
    /// Subnet mask for the tunnel network
    pub subnet_mask: Ipv4Address,
}

impl TunnelConfig {
    /// Create a new tunnel configuration with default MTU
    pub fn new(
        name: &str,
        tunnel_type: TunnelType,
        local_address: Ipv4Address,
        peer_address: Ipv4Address,
        subnet_mask: Ipv4Address,
    ) -> Self {
        Self {
            name: String::from(name),
            tunnel_type,
            mtu: DEFAULT_MTU,
            local_address,
            peer_address,
            subnet_mask,
        }
    }
}

// ── Encapsulation Header ─────────────────────────────────────────────────────

/// Outer encapsulation header for tunnel packets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncapsulationHeader {
    /// Outer source IP address
    pub src_ip: Ipv4Address,
    /// Outer destination IP address
    pub dst_ip: Ipv4Address,
    /// Transport protocol (UDP or TCP)
    pub protocol: EncapProtocol,
    /// Outer source port
    pub src_port: u16,
    /// Outer destination port
    pub dst_port: u16,
}

impl EncapsulationHeader {
    /// Create a new encapsulation header
    pub fn new(
        src_ip: Ipv4Address,
        dst_ip: Ipv4Address,
        protocol: EncapProtocol,
        src_port: u16,
        dst_port: u16,
    ) -> Self {
        Self {
            src_ip,
            dst_ip,
            protocol,
            src_port,
            dst_port,
        }
    }

    /// Serialise the encapsulation header to bytes (20-byte pseudo-header)
    pub fn to_bytes(&self) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[0..4].copy_from_slice(&self.src_ip.0);
        buf[4..8].copy_from_slice(&self.dst_ip.0);
        buf[8] = match self.protocol {
            EncapProtocol::Udp => 17,
            EncapProtocol::Tcp => 6,
        };
        buf[9] = 0; // reserved
        buf[10..12].copy_from_slice(&self.src_port.to_be_bytes());
        buf[12..14].copy_from_slice(&self.dst_port.to_be_bytes());
        // bytes 14..20 reserved / padding
        buf
    }

    /// Overhead in bytes added by the encapsulation
    pub fn overhead(&self) -> u16 {
        // IP header (20) + UDP/TCP header (8 for UDP, 20 for TCP)
        match self.protocol {
            EncapProtocol::Udp => 28,
            EncapProtocol::Tcp => 40,
        }
    }
}

// ── Tunnel Interface ─────────────────────────────────────────────────────────

/// Virtual tunnel network interface
#[derive(Debug, PartialEq)]
pub struct TunnelInterface {
    /// Configuration for this tunnel
    config: TunnelConfig,
    /// Current interface state
    state: TunnelState,
    /// Transmit packet queue
    tx_queue: Vec<Vec<u8>>,
    /// Receive packet queue
    rx_queue: Vec<Vec<u8>>,
    /// Interface statistics
    stats: TunnelStats,
    /// Optional encapsulation header for outer transport
    encap: Option<EncapsulationHeader>,
}

impl TunnelInterface {
    /// Create a new tunnel interface from configuration
    pub fn create(config: TunnelConfig) -> Result<Self, TunnelError> {
        if config.name.is_empty() || config.name.len() > MAX_NAME_LEN {
            return Err(TunnelError::InvalidName);
        }
        if config.mtu == 0 {
            return Err(TunnelError::InvalidMtu);
        }

        Ok(Self {
            config,
            state: TunnelState::Down,
            tx_queue: Vec::new(),
            rx_queue: Vec::new(),
            stats: TunnelStats::default(),
            encap: None,
        })
    }

    /// Bring the interface up
    pub fn bring_up(&mut self) -> Result<(), TunnelError> {
        if self.state == TunnelState::Up {
            return Err(TunnelError::AlreadyUp);
        }
        self.state = TunnelState::Up;
        Ok(())
    }

    /// Bring the interface down and flush queues
    pub fn bring_down(&mut self) {
        self.state = TunnelState::Down;
        self.tx_queue.clear();
        self.rx_queue.clear();
    }

    /// Enqueue a packet for transmission
    pub fn send_packet(&mut self, data: &[u8]) -> Result<(), TunnelError> {
        if self.state != TunnelState::Up {
            return Err(TunnelError::NotUp);
        }
        if data.len() > self.config.mtu as usize {
            self.stats.tx_errors += 1;
            return Err(TunnelError::PacketTooLarge);
        }
        if self.tx_queue.len() >= MAX_QUEUE_DEPTH {
            self.stats.tx_errors += 1;
            return Err(TunnelError::QueueFull);
        }

        self.stats.tx_packets += 1;
        self.stats.tx_bytes += data.len() as u64;
        self.tx_queue.push(data.to_vec());
        Ok(())
    }

    /// Dequeue a received packet
    pub fn receive_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        if self.state != TunnelState::Up {
            return Err(TunnelError::NotUp);
        }
        if self.rx_queue.is_empty() {
            return Err(TunnelError::QueueEmpty);
        }

        let pkt = self.rx_queue.remove(0);
        self.stats.rx_packets += 1;
        self.stats.rx_bytes += pkt.len() as u64;
        Ok(pkt)
    }

    /// Enqueue a packet into the receive queue (called by the VPN transport
    /// layer)
    pub fn inject_rx(&mut self, data: Vec<u8>) -> Result<(), TunnelError> {
        if self.rx_queue.len() >= MAX_QUEUE_DEPTH {
            self.stats.rx_errors += 1;
            return Err(TunnelError::QueueFull);
        }
        self.rx_queue.push(data);
        Ok(())
    }

    /// Dequeue a packet from the transmit queue (called by the VPN transport
    /// layer)
    pub fn drain_tx(&mut self) -> Option<Vec<u8>> {
        if self.tx_queue.is_empty() {
            None
        } else {
            Some(self.tx_queue.remove(0))
        }
    }

    /// Set the MTU for this interface
    pub fn set_mtu(&mut self, mtu: u16) -> Result<(), TunnelError> {
        if mtu == 0 {
            return Err(TunnelError::InvalidMtu);
        }
        self.config.mtu = mtu;
        Ok(())
    }

    /// Get the current MTU
    pub fn get_mtu(&self) -> u16 {
        self.config.mtu
    }

    /// Get interface statistics
    pub fn get_stats(&self) -> TunnelStats {
        self.stats
    }

    /// Get the interface name
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Get the tunnel type
    pub fn tunnel_type(&self) -> TunnelType {
        self.config.tunnel_type
    }

    /// Get the current state
    pub fn state(&self) -> TunnelState {
        self.state
    }

    /// Get the configuration
    pub fn config(&self) -> &TunnelConfig {
        &self.config
    }

    /// Set the encapsulation header
    pub fn set_encap(&mut self, encap: EncapsulationHeader) {
        self.encap = Some(encap);
    }

    /// Get the encapsulation header
    pub fn encap(&self) -> Option<&EncapsulationHeader> {
        self.encap.as_ref()
    }
}

// ── Route Injection ──────────────────────────────────────────────────────────

/// A route entry that directs traffic through a tunnel
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRoute {
    /// Destination network address
    pub destination: Ipv4Address,
    /// Subnet mask / prefix length (CIDR notation)
    pub prefix_len: u8,
    /// Name of the tunnel interface this route uses
    pub tunnel_name: String,
    /// Metric / priority (lower = preferred)
    pub metric: u32,
}

/// Route injection manager for tunnel interfaces
pub struct RouteInjection {
    /// Active tunnel routes
    routes: Vec<TunnelRoute>,
}

impl Default for RouteInjection {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteInjection {
    /// Create a new route injection manager
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Add a route through a tunnel interface
    pub fn add_route(
        &mut self,
        destination: Ipv4Address,
        prefix_len: u8,
        tunnel_name: &str,
        metric: u32,
    ) -> Result<(), TunnelError> {
        // Check for duplicate
        for r in &self.routes {
            if r.destination == destination
                && r.prefix_len == prefix_len
                && r.tunnel_name == tunnel_name
            {
                return Err(TunnelError::RouteExists);
            }
        }

        if self.routes.len() >= MAX_ROUTES {
            return Err(TunnelError::TooManyRoutes);
        }

        self.routes.push(TunnelRoute {
            destination,
            prefix_len,
            tunnel_name: String::from(tunnel_name),
            metric,
        });
        Ok(())
    }

    /// Remove a route
    pub fn remove_route(
        &mut self,
        destination: Ipv4Address,
        prefix_len: u8,
        tunnel_name: &str,
    ) -> Result<(), TunnelError> {
        let idx = self
            .routes
            .iter()
            .position(|r| {
                r.destination == destination
                    && r.prefix_len == prefix_len
                    && r.tunnel_name == tunnel_name
            })
            .ok_or(TunnelError::RouteNotFound)?;

        self.routes.remove(idx);
        Ok(())
    }

    /// Get all routes
    pub fn get_routes(&self) -> &[TunnelRoute] {
        &self.routes
    }

    /// Find the best route for a given destination IP
    pub fn lookup(&self, dst: &Ipv4Address) -> Option<&TunnelRoute> {
        let mut best: Option<&TunnelRoute> = None;
        let dst_u32 = dst.to_u32();

        for route in &self.routes {
            let mask = if route.prefix_len == 0 {
                0u32
            } else {
                u32::MAX << (32 - route.prefix_len)
            };
            let net = route.destination.to_u32() & mask;
            if (dst_u32 & mask) == net {
                match best {
                    None => best = Some(route),
                    Some(b) => {
                        // Prefer longer prefix, then lower metric
                        if route.prefix_len > b.prefix_len
                            || (route.prefix_len == b.prefix_len && route.metric < b.metric)
                        {
                            best = Some(route);
                        }
                    }
                }
            }
        }

        best
    }
}

// ── Tunnel Manager ───────────────────────────────────────────────────────────

/// Manager for all tunnel interfaces on the system
pub struct TunnelManager {
    /// Map of tunnel name -> tunnel interface
    tunnels: BTreeMap<String, TunnelInterface>,
    /// Route injection table
    routes: RouteInjection,
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelManager {
    /// Create a new tunnel manager
    pub fn new() -> Self {
        Self {
            tunnels: BTreeMap::new(),
            routes: RouteInjection::new(),
        }
    }

    /// Create and register a new tunnel interface
    pub fn create_tunnel(&mut self, config: TunnelConfig) -> Result<(), TunnelError> {
        if self.tunnels.len() >= MAX_TUNNELS {
            return Err(TunnelError::TooManyTunnels);
        }
        if self.tunnels.contains_key(&config.name) {
            return Err(TunnelError::AlreadyExists);
        }

        let name = config.name.clone();
        let iface = TunnelInterface::create(config)?;
        self.tunnels.insert(name, iface);
        Ok(())
    }

    /// Destroy (remove) a tunnel interface
    pub fn destroy_tunnel(&mut self, name: &str) -> Result<(), TunnelError> {
        self.tunnels
            .remove(name)
            .map(|_| ())
            .ok_or(TunnelError::NotFound)
    }

    /// Get a reference to a tunnel interface
    pub fn get_tunnel(&self, name: &str) -> Option<&TunnelInterface> {
        self.tunnels.get(name)
    }

    /// Get a mutable reference to a tunnel interface
    pub fn get_tunnel_mut(&mut self, name: &str) -> Option<&mut TunnelInterface> {
        self.tunnels.get_mut(name)
    }

    /// List all tunnel interface names
    pub fn list_tunnels(&self) -> Vec<&str> {
        self.tunnels.keys().map(|k| k.as_str()).collect()
    }

    /// Get the number of tunnels
    pub fn tunnel_count(&self) -> usize {
        self.tunnels.len()
    }

    /// Access the route injection table
    pub fn routes(&self) -> &RouteInjection {
        &self.routes
    }

    /// Access the route injection table mutably
    pub fn routes_mut(&mut self) -> &mut RouteInjection {
        &mut self.routes
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(name: &str) -> TunnelConfig {
        TunnelConfig::new(
            name,
            TunnelType::Tun,
            Ipv4Address::new(10, 0, 0, 1),
            Ipv4Address::new(10, 0, 0, 2),
            Ipv4Address::new(255, 255, 255, 0),
        )
    }

    #[test]
    fn test_tunnel_create() {
        let config = test_config("tun0");
        let iface = TunnelInterface::create(config).unwrap();
        assert_eq!(iface.name(), "tun0");
        assert_eq!(iface.tunnel_type(), TunnelType::Tun);
        assert_eq!(iface.state(), TunnelState::Down);
        assert_eq!(iface.get_mtu(), DEFAULT_MTU);
    }

    #[test]
    fn test_tunnel_create_invalid_name() {
        let mut config = test_config("tun0");
        config.name = String::new();
        assert_eq!(
            TunnelInterface::create(config),
            Err(TunnelError::InvalidName)
        );
    }

    #[test]
    fn test_tunnel_bring_up_down() {
        let config = test_config("tun0");
        let mut iface = TunnelInterface::create(config).unwrap();

        assert!(iface.bring_up().is_ok());
        assert_eq!(iface.state(), TunnelState::Up);

        // Double bring_up is an error
        assert_eq!(iface.bring_up(), Err(TunnelError::AlreadyUp));

        iface.bring_down();
        assert_eq!(iface.state(), TunnelState::Down);
    }

    #[test]
    fn test_tunnel_send_requires_up() {
        let config = test_config("tun0");
        let mut iface = TunnelInterface::create(config).unwrap();

        assert_eq!(iface.send_packet(&[1, 2, 3]), Err(TunnelError::NotUp));
    }

    #[test]
    fn test_tunnel_send_receive() {
        let config = test_config("tun0");
        let mut iface = TunnelInterface::create(config).unwrap();
        iface.bring_up().unwrap();

        let data = [0xAA; 100];
        assert!(iface.send_packet(&data).is_ok());

        // TX queue has one packet, RX queue is empty
        let pkt = iface.drain_tx().unwrap();
        assert_eq!(pkt.len(), 100);
        assert!(iface.drain_tx().is_none());

        // Inject into RX queue
        iface.inject_rx(pkt).unwrap();
        let received = iface.receive_packet().unwrap();
        assert_eq!(received.len(), 100);
        assert_eq!(received[0], 0xAA);
    }

    #[test]
    fn test_tunnel_mtu_enforcement() {
        let mut config = test_config("tun0");
        config.mtu = 64;
        let mut iface = TunnelInterface::create(config).unwrap();
        iface.bring_up().unwrap();

        // Packet within MTU
        assert!(iface.send_packet(&[0u8; 64]).is_ok());

        // Packet exceeding MTU
        assert_eq!(
            iface.send_packet(&[0u8; 65]),
            Err(TunnelError::PacketTooLarge)
        );

        let stats = iface.get_stats();
        assert_eq!(stats.tx_packets, 1);
        assert_eq!(stats.tx_errors, 1);
    }

    #[test]
    fn test_tunnel_set_mtu() {
        let config = test_config("tun0");
        let mut iface = TunnelInterface::create(config).unwrap();

        assert!(iface.set_mtu(9000).is_ok());
        assert_eq!(iface.get_mtu(), 9000);

        assert_eq!(iface.set_mtu(0), Err(TunnelError::InvalidMtu));
    }

    #[test]
    fn test_tunnel_stats() {
        let config = test_config("tun0");
        let mut iface = TunnelInterface::create(config).unwrap();
        iface.bring_up().unwrap();

        iface.send_packet(&[1u8; 50]).unwrap();
        iface.send_packet(&[2u8; 100]).unwrap();
        iface.inject_rx(alloc::vec![3u8; 75]).unwrap();
        iface.receive_packet().unwrap();

        let stats = iface.get_stats();
        assert_eq!(stats.tx_packets, 2);
        assert_eq!(stats.tx_bytes, 150);
        assert_eq!(stats.rx_packets, 1);
        assert_eq!(stats.rx_bytes, 75);
        assert_eq!(stats.tx_errors, 0);
        assert_eq!(stats.rx_errors, 0);
    }

    #[test]
    fn test_tunnel_manager_create_destroy() {
        let mut mgr = TunnelManager::new();

        mgr.create_tunnel(test_config("tun0")).unwrap();
        mgr.create_tunnel(test_config("tun1")).unwrap();
        assert_eq!(mgr.tunnel_count(), 2);

        // Duplicate name
        assert_eq!(
            mgr.create_tunnel(test_config("tun0")),
            Err(TunnelError::AlreadyExists)
        );

        mgr.destroy_tunnel("tun0").unwrap();
        assert_eq!(mgr.tunnel_count(), 1);

        // Not found
        assert_eq!(mgr.destroy_tunnel("tun0"), Err(TunnelError::NotFound));
    }

    #[test]
    fn test_tunnel_manager_list() {
        let mut mgr = TunnelManager::new();
        mgr.create_tunnel(test_config("tun0")).unwrap();
        mgr.create_tunnel(test_config("tun1")).unwrap();

        let names = mgr.list_tunnels();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"tun0"));
        assert!(names.contains(&"tun1"));
    }

    #[test]
    fn test_route_injection_add_remove() {
        let mut routes = RouteInjection::new();

        routes
            .add_route(Ipv4Address::new(10, 0, 0, 0), 24, "tun0", 100)
            .unwrap();
        assert_eq!(routes.get_routes().len(), 1);

        // Duplicate route
        assert_eq!(
            routes.add_route(Ipv4Address::new(10, 0, 0, 0), 24, "tun0", 100),
            Err(TunnelError::RouteExists)
        );

        routes
            .remove_route(Ipv4Address::new(10, 0, 0, 0), 24, "tun0")
            .unwrap();
        assert_eq!(routes.get_routes().len(), 0);

        assert_eq!(
            routes.remove_route(Ipv4Address::new(10, 0, 0, 0), 24, "tun0"),
            Err(TunnelError::RouteNotFound)
        );
    }

    #[test]
    fn test_route_lookup_longest_prefix() {
        let mut routes = RouteInjection::new();

        // Default route via tun0
        routes
            .add_route(Ipv4Address::new(0, 0, 0, 0), 0, "tun0", 100)
            .unwrap();
        // More specific route via tun1
        routes
            .add_route(Ipv4Address::new(10, 0, 0, 0), 24, "tun1", 100)
            .unwrap();

        // 10.0.0.5 matches both, but /24 is longer prefix
        let route = routes.lookup(&Ipv4Address::new(10, 0, 0, 5)).unwrap();
        assert_eq!(route.tunnel_name, "tun1");

        // 192.168.1.1 only matches default
        let route = routes.lookup(&Ipv4Address::new(192, 168, 1, 1)).unwrap();
        assert_eq!(route.tunnel_name, "tun0");
    }

    #[test]
    fn test_encap_header() {
        let hdr = EncapsulationHeader::new(
            Ipv4Address::new(192, 168, 1, 1),
            Ipv4Address::new(203, 0, 113, 1),
            EncapProtocol::Udp,
            12345,
            1194,
        );

        assert_eq!(hdr.overhead(), 28);

        let bytes = hdr.to_bytes();
        assert_eq!(&bytes[0..4], &[192, 168, 1, 1]);
        assert_eq!(&bytes[4..8], &[203, 0, 113, 1]);
        assert_eq!(bytes[8], 17); // UDP protocol number

        let tcp_hdr = EncapsulationHeader::new(
            Ipv4Address::new(10, 0, 0, 1),
            Ipv4Address::new(10, 0, 0, 2),
            EncapProtocol::Tcp,
            443,
            443,
        );
        assert_eq!(tcp_hdr.overhead(), 40);
        assert_eq!(tcp_hdr.to_bytes()[8], 6); // TCP protocol number
    }
}
