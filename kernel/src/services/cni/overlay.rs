//! VXLAN Overlay Network
//!
//! Provides VXLAN tunnel encapsulation/decapsulation, forwarding database
//! (FDB) MAC learning, and ARP proxy for cross-host container networking.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

// ---------------------------------------------------------------------------
// VXLAN Types
// ---------------------------------------------------------------------------

/// VXLAN tunnel configuration.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VxlanTunnel {
    /// VXLAN Network Identifier (24-bit, 0..16777215).
    pub vni: u32,
    /// Local tunnel endpoint IP.
    pub local_ip: u32,
    /// Remote tunnel endpoint IP.
    pub remote_ip: u32,
    /// UDP destination port (default 4789).
    pub port: u16,
    /// MTU for the tunnel interface.
    pub mtu: u16,
}

impl VxlanTunnel {
    /// Default VXLAN UDP port.
    pub const DEFAULT_PORT: u16 = 4789;
    /// Default tunnel MTU (1500 - 50 bytes VXLAN overhead).
    pub const DEFAULT_MTU: u16 = 1450;

    /// Create a new VXLAN tunnel.
    pub fn new(vni: u32, local_ip: u32, remote_ip: u32) -> Self {
        VxlanTunnel {
            vni: vni & 0x00FF_FFFF, // mask to 24 bits
            local_ip,
            remote_ip,
            port: Self::DEFAULT_PORT,
            mtu: Self::DEFAULT_MTU,
        }
    }
}

/// VXLAN header (8 bytes).
///
/// Wire format:
///   [flags(1)][reserved(3)][vni(3)][reserved(1)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub struct VxlanHeader {
    /// Flags (bit 3 = VNI valid).
    pub flags: u8,
    /// VXLAN Network Identifier (24-bit).
    pub vni: u32,
}

impl VxlanHeader {
    /// VXLAN header size in bytes.
    pub const SIZE: usize = 8;
    /// Flag indicating VNI is valid.
    pub const FLAG_VNI_VALID: u8 = 0x08;

    /// Create a new VXLAN header.
    pub fn new(vni: u32) -> Self {
        VxlanHeader {
            flags: Self::FLAG_VNI_VALID,
            vni: vni & 0x00FF_FFFF,
        }
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0] = self.flags;
        // bytes 1-3 reserved
        buf[4] = ((self.vni >> 16) & 0xFF) as u8;
        buf[5] = ((self.vni >> 8) & 0xFF) as u8;
        buf[6] = (self.vni & 0xFF) as u8;
        // byte 7 reserved
        buf
    }

    /// Parse from bytes.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::SIZE {
            return None;
        }
        let flags = data[0];
        let vni = ((data[4] as u32) << 16) | ((data[5] as u32) << 8) | (data[6] as u32);
        Some(VxlanHeader { flags, vni })
    }
}

// ---------------------------------------------------------------------------
// Forwarding Database
// ---------------------------------------------------------------------------

/// MAC address (6 bytes).
pub type MacAddress = [u8; 6];

/// FDB entry: maps MAC to remote VTEP IP.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FdbEntry {
    /// Destination MAC address.
    pub mac: MacAddress,
    /// Remote VTEP IP address.
    pub vtep_ip: u32,
    /// Last seen tick (for aging).
    pub last_seen: u64,
    /// Whether this is a static (non-aging) entry.
    pub is_static: bool,
}

// ---------------------------------------------------------------------------
// ARP Proxy Entry
// ---------------------------------------------------------------------------

/// ARP proxy entry: maps IP to MAC for remote containers.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ArpProxyEntry {
    /// Container IP address.
    pub ip: u32,
    /// Container MAC address.
    pub mac: MacAddress,
    /// Remote VTEP IP.
    pub vtep_ip: u32,
}

// ---------------------------------------------------------------------------
// VXLAN Overlay
// ---------------------------------------------------------------------------

/// VXLAN overlay error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum VxlanError {
    /// Tunnel not found.
    TunnelNotFound(u32),
    /// Tunnel already exists.
    TunnelExists(u32),
    /// Packet too small.
    PacketTooSmall,
    /// Invalid VXLAN header.
    InvalidHeader,
    /// VNI mismatch.
    VniMismatch { expected: u32, got: u32 },
    /// FDB lookup miss (no entry for MAC).
    FdbMiss,
}

/// VXLAN overlay network manager.
#[derive(Debug)]
#[allow(dead_code)]
pub struct VxlanOverlay {
    /// Active tunnels keyed by VNI.
    tunnels: BTreeMap<u32, VxlanTunnel>,
    /// Forwarding database: MAC -> FDB entry.
    fdb: BTreeMap<MacAddress, FdbEntry>,
    /// ARP proxy table: IP -> ARP entry.
    arp_proxy_table: BTreeMap<u32, ArpProxyEntry>,
    /// FDB aging timeout in ticks.
    fdb_age_timeout: u64,
}

impl Default for VxlanOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl VxlanOverlay {
    /// Default FDB aging timeout: 300 ticks (5 minutes at 1 tick/sec).
    pub const DEFAULT_FDB_AGE: u64 = 300;

    /// Create a new VXLAN overlay manager.
    pub fn new() -> Self {
        VxlanOverlay {
            tunnels: BTreeMap::new(),
            fdb: BTreeMap::new(),
            arp_proxy_table: BTreeMap::new(),
            fdb_age_timeout: Self::DEFAULT_FDB_AGE,
        }
    }

    /// Add a tunnel.
    pub fn add_tunnel(&mut self, tunnel: VxlanTunnel) -> Result<(), VxlanError> {
        if self.tunnels.contains_key(&tunnel.vni) {
            return Err(VxlanError::TunnelExists(tunnel.vni));
        }
        self.tunnels.insert(tunnel.vni, tunnel);
        Ok(())
    }

    /// Remove a tunnel.
    pub fn remove_tunnel(&mut self, vni: u32) -> Result<(), VxlanError> {
        self.tunnels
            .remove(&vni)
            .map(|_| ())
            .ok_or(VxlanError::TunnelNotFound(vni))
    }

    /// Get a tunnel by VNI.
    pub fn get_tunnel(&self, vni: u32) -> Option<&VxlanTunnel> {
        self.tunnels.get(&vni)
    }

    /// Encapsulate a frame in a VXLAN header.
    ///
    /// Returns the VXLAN-encapsulated packet (header + original frame).
    pub fn encapsulate(&self, vni: u32, inner_frame: &[u8]) -> Result<Vec<u8>, VxlanError> {
        if !self.tunnels.contains_key(&vni) {
            return Err(VxlanError::TunnelNotFound(vni));
        }

        let header = VxlanHeader::new(vni);
        let header_bytes = header.to_bytes();

        let mut packet = Vec::with_capacity(VxlanHeader::SIZE + inner_frame.len());
        packet.extend_from_slice(&header_bytes);
        packet.extend_from_slice(inner_frame);
        Ok(packet)
    }

    /// Decapsulate a VXLAN packet.
    ///
    /// Returns (VNI, inner frame).
    pub fn decapsulate(&self, packet: &[u8]) -> Result<(u32, Vec<u8>), VxlanError> {
        let header = VxlanHeader::from_bytes(packet).ok_or(VxlanError::PacketTooSmall)?;

        if header.flags & VxlanHeader::FLAG_VNI_VALID == 0 {
            return Err(VxlanError::InvalidHeader);
        }

        if !self.tunnels.contains_key(&header.vni) {
            return Err(VxlanError::TunnelNotFound(header.vni));
        }

        let inner = packet[VxlanHeader::SIZE..].to_vec();
        Ok((header.vni, inner))
    }

    /// Learn a MAC address from an incoming frame.
    pub fn fdb_learn(&mut self, mac: MacAddress, vtep_ip: u32, current_tick: u64) {
        let entry = self.fdb.entry(mac).or_insert(FdbEntry {
            mac,
            vtep_ip,
            last_seen: current_tick,
            is_static: false,
        });
        entry.vtep_ip = vtep_ip;
        entry.last_seen = current_tick;
    }

    /// Add a static FDB entry.
    pub fn fdb_add_static(&mut self, mac: MacAddress, vtep_ip: u32) {
        self.fdb.insert(
            mac,
            FdbEntry {
                mac,
                vtep_ip,
                last_seen: 0,
                is_static: true,
            },
        );
    }

    /// Look up a MAC address in the FDB.
    pub fn fdb_lookup(&self, mac: &MacAddress) -> Option<&FdbEntry> {
        self.fdb.get(mac)
    }

    /// Remove aged-out FDB entries.
    pub fn fdb_age(&mut self, current_tick: u64) -> usize {
        let timeout = self.fdb_age_timeout;
        let before = self.fdb.len();
        self.fdb.retain(|_, entry| {
            entry.is_static || current_tick.saturating_sub(entry.last_seen) < timeout
        });
        before - self.fdb.len()
    }

    /// Add an ARP proxy entry.
    pub fn arp_proxy_add(&mut self, ip: u32, mac: MacAddress, vtep_ip: u32) {
        self.arp_proxy_table
            .insert(ip, ArpProxyEntry { ip, mac, vtep_ip });
    }

    /// Respond to an ARP request for a remote container.
    ///
    /// Returns the MAC address if we can proxy-respond.
    pub fn arp_proxy(&self, target_ip: u32) -> Option<MacAddress> {
        self.arp_proxy_table.get(&target_ip).map(|e| e.mac)
    }

    /// Get tunnel count.
    pub fn tunnel_count(&self) -> usize {
        self.tunnels.len()
    }

    /// Get FDB entry count.
    pub fn fdb_count(&self) -> usize {
        self.fdb.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> u32 {
        ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | (d as u32)
    }

    #[test]
    fn test_vxlan_header_roundtrip() {
        let header = VxlanHeader::new(12345);
        let bytes = header.to_bytes();
        let parsed = VxlanHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.vni, 12345);
        assert_eq!(parsed.flags, VxlanHeader::FLAG_VNI_VALID);
    }

    #[test]
    fn test_vxlan_header_too_small() {
        assert!(VxlanHeader::from_bytes(&[0u8; 4]).is_none());
    }

    #[test]
    fn test_add_remove_tunnel() {
        let mut overlay = VxlanOverlay::new();
        let tunnel = VxlanTunnel::new(100, ip(10, 0, 0, 1), ip(10, 0, 0, 2));
        overlay.add_tunnel(tunnel).unwrap();
        assert_eq!(overlay.tunnel_count(), 1);
        overlay.remove_tunnel(100).unwrap();
        assert_eq!(overlay.tunnel_count(), 0);
    }

    #[test]
    fn test_duplicate_tunnel() {
        let mut overlay = VxlanOverlay::new();
        let t1 = VxlanTunnel::new(100, ip(10, 0, 0, 1), ip(10, 0, 0, 2));
        let t2 = VxlanTunnel::new(100, ip(10, 0, 0, 1), ip(10, 0, 0, 3));
        overlay.add_tunnel(t1).unwrap();
        assert_eq!(overlay.add_tunnel(t2), Err(VxlanError::TunnelExists(100)));
    }

    #[test]
    fn test_encap_decap() {
        let mut overlay = VxlanOverlay::new();
        overlay
            .add_tunnel(VxlanTunnel::new(42, ip(10, 0, 0, 1), ip(10, 0, 0, 2)))
            .unwrap();

        let frame = [0xDE, 0xAD, 0xBE, 0xEF];
        let packet = overlay.encapsulate(42, &frame).unwrap();
        assert_eq!(packet.len(), 8 + 4);

        let (vni, inner) = overlay.decapsulate(&packet).unwrap();
        assert_eq!(vni, 42);
        assert_eq!(inner, frame);
    }

    #[test]
    fn test_encap_unknown_vni() {
        let overlay = VxlanOverlay::new();
        assert_eq!(
            overlay.encapsulate(999, &[1, 2, 3]),
            Err(VxlanError::TunnelNotFound(999))
        );
    }

    #[test]
    fn test_fdb_learn_and_lookup() {
        let mut overlay = VxlanOverlay::new();
        let mac = [0x02, 0x42, 0xAC, 0x11, 0x00, 0x02];
        overlay.fdb_learn(mac, ip(10, 0, 0, 5), 100);
        let entry = overlay.fdb_lookup(&mac).unwrap();
        assert_eq!(entry.vtep_ip, ip(10, 0, 0, 5));
        assert!(!entry.is_static);
    }

    #[test]
    fn test_fdb_aging() {
        let mut overlay = VxlanOverlay::new();
        let mac = [0x02, 0x42, 0xAC, 0x11, 0x00, 0x03];
        overlay.fdb_learn(mac, ip(10, 0, 0, 5), 100);
        // Not aged yet
        assert_eq!(overlay.fdb_age(200), 0);
        // Aged out
        assert_eq!(overlay.fdb_age(500), 1);
        assert_eq!(overlay.fdb_count(), 0);
    }

    #[test]
    fn test_arp_proxy() {
        let mut overlay = VxlanOverlay::new();
        let mac = [0x02, 0x42, 0xAC, 0x11, 0x00, 0x04];
        let container_ip = ip(10, 244, 1, 5);
        overlay.arp_proxy_add(container_ip, mac, ip(10, 0, 0, 2));
        let resolved = overlay.arp_proxy(container_ip).unwrap();
        assert_eq!(resolved, mac);
        assert!(overlay.arp_proxy(ip(10, 244, 1, 99)).is_none());
    }
}
