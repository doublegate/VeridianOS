//! IEEE 802.1Q VLAN tagging support
//!
//! Provides VLAN tag insertion/stripping and interface management
//! for 802.1Q virtual LAN segmentation.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use spin::Mutex;

use crate::sync::once_lock::OnceLock;

/// IEEE 802.1Q Tag Protocol Identifier
pub const TPID_8021Q: u16 = 0x8100;

/// Minimum Ethernet frame size (dst MAC + src MAC + EtherType)
const ETH_HEADER_MIN: usize = 14;

/// Size of an 802.1Q tag in bytes (TPID + TCI)
const VLAN_TAG_SIZE: usize = 4;

/// Maximum valid VLAN ID
const VLAN_ID_MAX: u16 = 4094;

// ── 802.1Q Tag ──────────────────────────────────────────────────────────────

/// IEEE 802.1Q VLAN tag
///
/// Layout (network byte order):
///   TPID: 16 bits (0x8100 for 802.1Q)
///   TCI:  16 bits = PCP (3) + DEI (1) + VID (12)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VlanTag {
    /// Tag Protocol Identifier (always 0x8100 for 802.1Q)
    pub tpid: u16,
    /// Tag Control Information (PCP + DEI + VID)
    pub tci: u16,
}

impl VlanTag {
    /// Create a new VLAN tag with the given parameters.
    ///
    /// - `vid`: VLAN Identifier (0-4094, 12 bits)
    /// - `pcp`: Priority Code Point (0-7, 3 bits)
    /// - `dei`: Drop Eligible Indicator
    pub fn new(vid: u16, pcp: u8, dei: bool) -> Self {
        let tci = ((pcp as u16 & 0x07) << 13) | (if dei { 1u16 << 12 } else { 0 }) | (vid & 0x0FFF);
        Self {
            tpid: TPID_8021Q,
            tci,
        }
    }

    /// Extract the 12-bit VLAN Identifier from TCI.
    pub fn vid(&self) -> u16 {
        self.tci & 0x0FFF
    }

    /// Extract the 3-bit Priority Code Point from TCI.
    pub fn pcp(&self) -> u8 {
        ((self.tci >> 13) & 0x07) as u8
    }

    /// Extract the Drop Eligible Indicator bit from TCI.
    pub fn dei(&self) -> bool {
        (self.tci >> 12) & 0x01 != 0
    }

    /// Serialize the tag to 4 bytes in network byte order.
    pub fn to_bytes(&self) -> [u8; VLAN_TAG_SIZE] {
        let tpid = self.tpid.to_be_bytes();
        let tci = self.tci.to_be_bytes();
        [tpid[0], tpid[1], tci[0], tci[1]]
    }

    /// Parse a VLAN tag from 4 bytes in network byte order.
    pub fn from_bytes(bytes: &[u8; VLAN_TAG_SIZE]) -> Self {
        let tpid = u16::from_be_bytes([bytes[0], bytes[1]]);
        let tci = u16::from_be_bytes([bytes[2], bytes[3]]);
        Self { tpid, tci }
    }
}

// ── Tag insertion / stripping ───────────────────────────────────────────────

/// Check whether an Ethernet frame carries an 802.1Q tag.
///
/// Inspects the EtherType field at byte offset 12.
pub fn has_vlan_tag(frame: &[u8]) -> bool {
    if frame.len() < ETH_HEADER_MIN {
        return false;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    ethertype == TPID_8021Q
}

/// Insert an 802.1Q tag into an Ethernet frame after the source MAC address.
///
/// Returns a new frame with the 4-byte tag inserted at offset 12.
pub fn insert_tag(frame: &[u8], tag: VlanTag) -> Vec<u8> {
    if frame.len() < ETH_HEADER_MIN {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity(frame.len() + VLAN_TAG_SIZE);
    // dst MAC (6) + src MAC (6)
    out.extend_from_slice(&frame[..12]);
    // 802.1Q tag (4)
    out.extend_from_slice(&tag.to_bytes());
    // original EtherType + payload
    out.extend_from_slice(&frame[12..]);
    out
}

/// Strip an 802.1Q tag from an Ethernet frame.
///
/// Returns `(Some(tag), inner_frame)` if the frame is tagged, or
/// `(None, original_frame)` if not.
pub fn strip_tag(frame: &[u8]) -> (Option<VlanTag>, Vec<u8>) {
    if !has_vlan_tag(frame) || frame.len() < ETH_HEADER_MIN + VLAN_TAG_SIZE {
        return (None, frame.to_vec());
    }
    let tag_bytes: [u8; VLAN_TAG_SIZE] = [frame[12], frame[13], frame[14], frame[15]];
    let tag = VlanTag::from_bytes(&tag_bytes);

    let mut out = Vec::with_capacity(frame.len() - VLAN_TAG_SIZE);
    // dst MAC (6) + src MAC (6)
    out.extend_from_slice(&frame[..12]);
    // skip 4-byte tag, copy remaining (original EtherType + payload)
    out.extend_from_slice(&frame[16..]);
    (Some(tag), out)
}

// ── VLAN interface / mode ───────────────────────────────────────────────────

/// VLAN operating mode for a port/interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VlanMode {
    /// Access port: all frames belong to a single VLAN.
    /// Tags outgoing frames, strips incoming tags.
    Access(u16),
    /// Trunk port: carries multiple VLANs.
    /// Passes tagged frames for allowed VIDs.
    Trunk(Vec<u16>),
}

/// A VLAN interface bound to a parent network device.
#[derive(Debug, Clone)]
pub struct VlanInterface {
    /// Name of the underlying physical NIC.
    pub parent_device: String,
    /// VLAN ID (1-4094).
    pub vid: u16,
    /// Port mode (access or trunk).
    pub mode: VlanMode,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Errors from VLAN operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VlanError {
    /// VLAN ID out of valid range (1-4094).
    InvalidVid(u16),
    /// A VLAN with this (parent, vid) already exists.
    AlreadyExists,
    /// No VLAN found for the given (parent, vid).
    NotFound,
}

// ── VLAN manager ────────────────────────────────────────────────────────────

/// Manages VLAN interfaces across all network devices.
#[derive(Debug)]
pub struct VlanManager {
    interfaces: Vec<VlanInterface>,
}

impl VlanManager {
    /// Create a new empty VLAN manager.
    pub fn new() -> Self {
        Self {
            interfaces: Vec::new(),
        }
    }

    /// Create a VLAN interface on the given parent device.
    pub fn create_vlan(&mut self, parent: &str, vid: u16, mode: VlanMode) -> Result<(), VlanError> {
        if vid == 0 || vid > VLAN_ID_MAX {
            return Err(VlanError::InvalidVid(vid));
        }
        // Check for duplicates
        let exists = self
            .interfaces
            .iter()
            .any(|i| i.parent_device == parent && i.vid == vid);
        if exists {
            return Err(VlanError::AlreadyExists);
        }
        self.interfaces.push(VlanInterface {
            parent_device: String::from(parent),
            vid,
            mode,
        });
        Ok(())
    }

    /// Delete a VLAN interface.
    pub fn delete_vlan(&mut self, parent: &str, vid: u16) -> Result<(), VlanError> {
        let pos = self
            .interfaces
            .iter()
            .position(|i| i.parent_device == parent && i.vid == vid);
        match pos {
            Some(idx) => {
                self.interfaces.remove(idx);
                Ok(())
            }
            None => Err(VlanError::NotFound),
        }
    }

    /// List all configured VLAN interfaces.
    pub fn list_vlans(&self) -> Vec<VlanInterface> {
        self.interfaces.clone()
    }

    /// Process an incoming (ingress) frame on a parent device.
    ///
    /// Returns `Some((vid, untagged_frame))` if the frame should be accepted,
    /// or `None` if it should be dropped.
    pub fn process_ingress(&self, parent: &str, frame: &[u8]) -> Option<(u16, Vec<u8>)> {
        let matching: Vec<&VlanInterface> = self
            .interfaces
            .iter()
            .filter(|i| i.parent_device == parent)
            .collect();

        if matching.is_empty() {
            return None;
        }

        if has_vlan_tag(frame) {
            // Tagged frame -- extract VID and check trunk ports
            let (tag_opt, inner) = strip_tag(frame);
            let tag = tag_opt?;
            let vid = tag.vid();

            for iface in &matching {
                match &iface.mode {
                    VlanMode::Access(access_vid) => {
                        if vid == *access_vid {
                            return Some((vid, inner));
                        }
                    }
                    VlanMode::Trunk(allowed) => {
                        if allowed.contains(&vid) {
                            return Some((vid, inner));
                        }
                    }
                }
            }
            None
        } else {
            // Untagged frame -- assign to the access VLAN if one exists
            for iface in &matching {
                if let VlanMode::Access(access_vid) = &iface.mode {
                    return Some((*access_vid, frame.to_vec()));
                }
            }
            None
        }
    }

    /// Process an outgoing (egress) frame for a given VLAN on a parent device.
    ///
    /// For access ports the frame is sent untagged (already stripped).
    /// For trunk ports the frame is sent with an 802.1Q tag.
    pub fn process_egress(&self, parent: &str, vid: u16, frame: &[u8]) -> Vec<u8> {
        let iface = self
            .interfaces
            .iter()
            .find(|i| i.parent_device == parent && i.vid == vid);

        match iface {
            Some(i) => match &i.mode {
                VlanMode::Access(_) => {
                    // Access port: send untagged
                    frame.to_vec()
                }
                VlanMode::Trunk(_) => {
                    // Trunk port: insert 802.1Q tag
                    let tag = VlanTag::new(vid, 0, false);
                    insert_tag(frame, tag)
                }
            },
            None => {
                // No matching interface -- pass through unmodified
                frame.to_vec()
            }
        }
    }
}

impl Default for VlanManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Global manager ──────────────────────────────────────────────────────────

static VLAN_MANAGER: OnceLock<Mutex<VlanManager>> = OnceLock::new();

/// Initialize the global VLAN manager.
pub fn init() {
    let _ = VLAN_MANAGER.set(Mutex::new(VlanManager::new()));
}

/// Access the global VLAN manager under a lock.
pub fn with_manager<R, F: FnOnce(&mut VlanManager) -> R>(f: F) -> Option<R> {
    VLAN_MANAGER.get().map(|m| {
        let mut guard = m.lock();
        f(&mut guard)
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use alloc::vec;

    #[test]
    fn test_vlan_tag_new() {
        let tag = VlanTag::new(100, 5, true);
        assert_eq!(tag.vid(), 100);
        assert_eq!(tag.pcp(), 5);
        assert!(tag.dei());
        assert_eq!(tag.tpid, TPID_8021Q);
    }

    #[test]
    fn test_vlan_tag_fields_zero() {
        let tag = VlanTag::new(0, 0, false);
        assert_eq!(tag.vid(), 0);
        assert_eq!(tag.pcp(), 0);
        assert!(!tag.dei());
    }

    #[test]
    fn test_vlan_tag_max_vid() {
        let tag = VlanTag::new(4095, 7, true);
        assert_eq!(tag.vid(), 4095);
        assert_eq!(tag.pcp(), 7);
        assert!(tag.dei());
    }

    #[test]
    fn test_vlan_tag_roundtrip_bytes() {
        let original = VlanTag::new(42, 3, false);
        let bytes = original.to_bytes();
        let parsed = VlanTag::from_bytes(&bytes);
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_has_vlan_tag_true() {
        // 12 bytes MACs + 0x8100 EtherType
        let mut frame = vec![0u8; 14];
        frame[12] = 0x81;
        frame[13] = 0x00;
        assert!(has_vlan_tag(&frame));
    }

    #[test]
    fn test_has_vlan_tag_false() {
        // Normal IPv4 EtherType (0x0800)
        let mut frame = vec![0u8; 14];
        frame[12] = 0x08;
        frame[13] = 0x00;
        assert!(!has_vlan_tag(&frame));
    }

    #[test]
    fn test_has_vlan_tag_short_frame() {
        let frame = vec![0u8; 10];
        assert!(!has_vlan_tag(&frame));
    }

    #[test]
    fn test_insert_and_strip_tag() {
        // Build a minimal Ethernet frame: 6 dst + 6 src + 2 EtherType + 4 payload
        let mut frame = vec![0xAA; 6]; // dst
        frame.extend_from_slice(&[0xBB; 6]); // src
        frame.extend_from_slice(&[0x08, 0x00]); // IPv4
        frame.extend_from_slice(&[1, 2, 3, 4]); // payload

        let tag = VlanTag::new(200, 2, false);
        let tagged = insert_tag(&frame, tag);

        // Tagged frame should be 4 bytes longer
        assert_eq!(tagged.len(), frame.len() + 4);
        assert!(has_vlan_tag(&tagged));

        // Strip and verify round-trip
        let (stripped_tag, inner) = strip_tag(&tagged);
        assert_eq!(stripped_tag, Some(tag));
        assert_eq!(inner, frame);
    }

    #[test]
    fn test_strip_untagged_frame() {
        let frame = vec![0u8; 20];
        let (tag, inner) = strip_tag(&frame);
        assert!(tag.is_none());
        assert_eq!(inner, frame);
    }

    #[test]
    fn test_manager_create_and_list() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 10, VlanMode::Access(10)).unwrap();
        mgr.create_vlan("eth0", 20, VlanMode::Trunk(vec![20, 30]))
            .unwrap();

        let vlans = mgr.list_vlans();
        assert_eq!(vlans.len(), 2);
        assert_eq!(vlans[0].vid, 10);
        assert_eq!(vlans[1].vid, 20);
    }

    #[test]
    fn test_manager_duplicate_error() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 10, VlanMode::Access(10)).unwrap();
        let err = mgr
            .create_vlan("eth0", 10, VlanMode::Access(10))
            .unwrap_err();
        assert_eq!(err, VlanError::AlreadyExists);
    }

    #[test]
    fn test_manager_invalid_vid() {
        let mut mgr = VlanManager::new();
        assert_eq!(
            mgr.create_vlan("eth0", 0, VlanMode::Access(0)),
            Err(VlanError::InvalidVid(0))
        );
        assert_eq!(
            mgr.create_vlan("eth0", 4095, VlanMode::Access(4095)),
            Err(VlanError::InvalidVid(4095))
        );
    }

    #[test]
    fn test_manager_delete() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 10, VlanMode::Access(10)).unwrap();
        mgr.delete_vlan("eth0", 10).unwrap();
        assert!(mgr.list_vlans().is_empty());
        assert_eq!(mgr.delete_vlan("eth0", 10), Err(VlanError::NotFound));
    }

    #[test]
    fn test_ingress_access_untagged() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 10, VlanMode::Access(10)).unwrap();

        // Untagged frame on access port -> assigned to VLAN 10
        let mut frame = vec![0u8; 18]; // 14 header + 4 payload
        frame[12] = 0x08;
        frame[13] = 0x00;
        let result = mgr.process_ingress("eth0", &frame);
        assert!(result.is_some());
        let (vid, inner) = result.unwrap();
        assert_eq!(vid, 10);
        assert_eq!(inner, frame);
    }

    #[test]
    fn test_ingress_trunk_tagged() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 20, VlanMode::Trunk(vec![20, 30]))
            .unwrap();

        // Build tagged frame with VID=20
        let mut frame = vec![0u8; 18];
        frame[12] = 0x08;
        frame[13] = 0x00;
        let tag = VlanTag::new(20, 0, false);
        let tagged = insert_tag(&frame, tag);

        let result = mgr.process_ingress("eth0", &tagged);
        assert!(result.is_some());
        let (vid, inner) = result.unwrap();
        assert_eq!(vid, 20);
        assert_eq!(inner, frame);
    }

    #[test]
    fn test_ingress_trunk_disallowed_vid() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 20, VlanMode::Trunk(vec![20, 30]))
            .unwrap();

        // Build tagged frame with VID=99 (not in allowed list)
        let mut frame = vec![0u8; 18];
        frame[12] = 0x08;
        frame[13] = 0x00;
        let tag = VlanTag::new(99, 0, false);
        let tagged = insert_tag(&frame, tag);

        let result = mgr.process_ingress("eth0", &tagged);
        assert!(result.is_none());
    }

    #[test]
    fn test_egress_access_untagged() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 10, VlanMode::Access(10)).unwrap();

        let frame = vec![0u8; 18];
        let out = mgr.process_egress("eth0", 10, &frame);
        // Access port: output is untagged
        assert_eq!(out, frame);
    }

    #[test]
    fn test_egress_trunk_tagged() {
        let mut mgr = VlanManager::new();
        mgr.create_vlan("eth0", 20, VlanMode::Trunk(vec![20, 30]))
            .unwrap();

        let mut frame = vec![0u8; 18];
        frame[12] = 0x08;
        frame[13] = 0x00;
        let out = mgr.process_egress("eth0", 20, &frame);
        // Trunk port: output is tagged
        assert!(has_vlan_tag(&out));
        assert_eq!(out.len(), frame.len() + 4);
    }
}
