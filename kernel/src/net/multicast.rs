//! Multicast group management with IGMP/MLD protocol support
//!
//! Provides IPv4 multicast via IGMPv2 and IPv6 multicast via MLDv2,
//! including group join/leave, periodic report generation, and
//! query response handling.

#![allow(dead_code)] // Phase 7.5 network stack -- functions called as stack matures

use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use crate::sync::once_lock::OnceLock;

// ============================================================================
// Constants
// ============================================================================

// IGMPv2 message types
/// Membership Query
pub const IGMP_MEMBERSHIP_QUERY: u8 = 0x11;
/// IGMPv2 Membership Report
pub const IGMP_MEMBERSHIP_REPORT: u8 = 0x16;
/// Leave Group
pub const IGMP_LEAVE_GROUP: u8 = 0x17;

// MLDv2 message types (ICMPv6 types)
/// Multicast Listener Query
pub const MLD_QUERY: u8 = 130;
/// MLDv2 Multicast Listener Report
pub const MLD_REPORT_V2: u8 = 143;

// MLDv2 multicast address record types
/// Current-State Record: Include mode
pub const MLD_RECORD_IS_IN: u8 = 1;
/// Current-State Record: Exclude mode
pub const MLD_RECORD_IS_EX: u8 = 2;
/// Filter-Mode-Change: to Include
pub const MLD_RECORD_TO_IN: u8 = 3;
/// Filter-Mode-Change: to Exclude
pub const MLD_RECORD_TO_EX: u8 = 4;
/// Source-List-Change: allow new sources
pub const MLD_RECORD_ALLOW: u8 = 5;
/// Source-List-Change: block old sources
pub const MLD_RECORD_BLOCK: u8 = 6;

/// Default unsolicited report interval in ticks
const UNSOLICITED_REPORT_INTERVAL: u64 = 1000;

// ============================================================================
// Error Type
// ============================================================================

/// Errors from multicast operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulticastError {
    /// Address is not in the multicast range
    InvalidAddress,
    /// Group not found in the membership table
    GroupNotFound,
    /// Already a member of this group
    AlreadyMember,
    /// Maximum number of groups reached
    GroupLimitReached,
    /// Serialization/deserialization failure
    MalformedMessage,
    /// Manager not initialized
    NotInitialized,
}

// ============================================================================
// Multicast Group Addresses
// ============================================================================

/// IPv4 multicast group identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MulticastGroup {
    /// IPv4 multicast address (must be in 224.0.0.0/4)
    pub address: [u8; 4],
    /// Network interface index
    pub interface_index: u32,
}

impl MulticastGroup {
    /// Create a new multicast group, validating the address range.
    pub fn new(address: [u8; 4], interface_index: u32) -> Result<Self, MulticastError> {
        if !is_ipv4_multicast(&address) {
            return Err(MulticastError::InvalidAddress);
        }
        Ok(Self {
            address,
            interface_index,
        })
    }
}

/// IPv6 multicast group identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MulticastGroupV6 {
    /// IPv6 multicast address (must be in ff00::/8)
    pub address: [u8; 16],
    /// Network interface index
    pub interface_index: u32,
}

impl MulticastGroupV6 {
    /// Create a new IPv6 multicast group, validating the address range.
    pub fn new(address: [u8; 16], interface_index: u32) -> Result<Self, MulticastError> {
        if !is_ipv6_multicast(&address) {
            return Err(MulticastError::InvalidAddress);
        }
        Ok(Self {
            address,
            interface_index,
        })
    }
}

/// Check if an IPv4 address is in the multicast range (224.0.0.0/4).
pub fn is_ipv4_multicast(addr: &[u8; 4]) -> bool {
    addr[0] & 0xF0 == 224
}

/// Check if an IPv6 address is in the multicast range (ff00::/8).
pub fn is_ipv6_multicast(addr: &[u8; 16]) -> bool {
    addr[0] == 0xFF
}

// ============================================================================
// IGMPv2 Message
// ============================================================================

/// IGMPv2 message (8 bytes on the wire)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgmpMessage {
    /// Message type (0x11 = Query, 0x16 = Report, 0x17 = Leave)
    pub msg_type: u8,
    /// Maximum response time (in 1/10 second units)
    pub max_resp_time: u8,
    /// Internet checksum over the entire IGMP message
    pub checksum: u16,
    /// Group address (0.0.0.0 for general queries)
    pub group_address: [u8; 4],
}

impl IgmpMessage {
    /// Size of a serialized IGMP message in bytes.
    pub const WIRE_SIZE: usize = 8;

    /// Create a new IGMP message with checksum computed automatically.
    pub fn new(msg_type: u8, max_resp_time: u8, group_address: [u8; 4]) -> Self {
        let mut msg = Self {
            msg_type,
            max_resp_time,
            checksum: 0,
            group_address,
        };
        msg.checksum = msg.compute_checksum();
        msg
    }

    /// Serialize the message to bytes.
    pub fn to_bytes(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0] = self.msg_type;
        buf[1] = self.max_resp_time;
        buf[2] = (self.checksum >> 8) as u8;
        buf[3] = self.checksum as u8;
        buf[4..8].copy_from_slice(&self.group_address);
        buf
    }

    /// Deserialize from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, MulticastError> {
        if data.len() < Self::WIRE_SIZE {
            return Err(MulticastError::MalformedMessage);
        }
        Ok(Self {
            msg_type: data[0],
            max_resp_time: data[1],
            checksum: u16::from_be_bytes([data[2], data[3]]),
            group_address: [data[4], data[5], data[6], data[7]],
        })
    }

    /// Compute the Internet checksum over the IGMP message.
    pub fn compute_checksum(&self) -> u16 {
        let mut bytes = self.to_bytes();
        // Zero out the checksum field before computing
        bytes[2] = 0;
        bytes[3] = 0;
        internet_checksum(&bytes)
    }

    /// Verify the message checksum.
    pub fn verify_checksum(&self) -> bool {
        let bytes = self.to_bytes();
        internet_checksum(&bytes) == 0
    }
}

// ============================================================================
// MLDv2 Message
// ============================================================================

/// MLDv2 message header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MldMessage {
    /// Message type (130 = Query, 143 = Report)
    pub msg_type: u8,
    /// Code (subtype, typically 0)
    pub code: u8,
    /// Checksum (computed over pseudo-header + message)
    pub checksum: u16,
    /// Maximum response delay (queries) or reserved (reports)
    pub max_resp_delay: u16,
    /// Reserved field
    pub reserved: u16,
    /// Multicast address (queries) or zero (reports)
    pub multicast_address: [u8; 16],
}

impl MldMessage {
    /// Minimum header size in bytes.
    pub const HEADER_SIZE: usize = 24;

    /// Create a new MLD query message.
    pub fn new_query(max_resp_delay: u16, multicast_address: [u8; 16]) -> Self {
        Self {
            msg_type: MLD_QUERY,
            code: 0,
            checksum: 0,
            max_resp_delay,
            reserved: 0,
            multicast_address,
        }
    }

    /// Create a new MLDv2 report message.
    pub fn new_report() -> Self {
        Self {
            msg_type: MLD_REPORT_V2,
            code: 0,
            checksum: 0,
            max_resp_delay: 0,
            reserved: 0,
            multicast_address: [0; 16],
        }
    }

    /// Serialize the message header to bytes.
    pub fn to_bytes(&self) -> [u8; Self::HEADER_SIZE] {
        let mut buf = [0u8; Self::HEADER_SIZE];
        buf[0] = self.msg_type;
        buf[1] = self.code;
        buf[2] = (self.checksum >> 8) as u8;
        buf[3] = self.checksum as u8;
        buf[4] = (self.max_resp_delay >> 8) as u8;
        buf[5] = self.max_resp_delay as u8;
        buf[6] = (self.reserved >> 8) as u8;
        buf[7] = self.reserved as u8;
        buf[8..24].copy_from_slice(&self.multicast_address);
        buf
    }

    /// Deserialize from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, MulticastError> {
        if data.len() < Self::HEADER_SIZE {
            return Err(MulticastError::MalformedMessage);
        }
        let mut addr = [0u8; 16];
        addr.copy_from_slice(&data[8..24]);
        Ok(Self {
            msg_type: data[0],
            code: data[1],
            checksum: u16::from_be_bytes([data[2], data[3]]),
            max_resp_delay: u16::from_be_bytes([data[4], data[5]]),
            reserved: u16::from_be_bytes([data[6], data[7]]),
            multicast_address: addr,
        })
    }
}

/// MLDv2 multicast address record
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MldAddressRecord {
    /// Record type (IS_IN, IS_EX, TO_IN, TO_EX, ALLOW, BLOCK)
    pub record_type: u8,
    /// Auxiliary data length (in 32-bit words)
    pub aux_data_len: u8,
    /// Multicast address
    pub multicast_address: [u8; 16],
    /// Source addresses
    pub source_addresses: Vec<[u8; 16]>,
}

impl MldAddressRecord {
    /// Create a new address record for a group.
    pub fn new(record_type: u8, multicast_address: [u8; 16]) -> Self {
        Self {
            record_type,
            aux_data_len: 0,
            multicast_address,
            source_addresses: Vec::new(),
        }
    }

    /// Serialized size in bytes.
    pub fn wire_size(&self) -> usize {
        // 4 bytes header + 16 bytes mcast addr + 16 * num_sources
        4 + 16 + self.source_addresses.len() * 16
    }

    /// Serialize to bytes (appended to the provided buffer).
    pub fn serialize_into(&self, buf: &mut Vec<u8>) {
        buf.push(self.record_type);
        buf.push(self.aux_data_len);
        let num_sources = self.source_addresses.len() as u16;
        buf.push((num_sources >> 8) as u8);
        buf.push(num_sources as u8);
        buf.extend_from_slice(&self.multicast_address);
        for src in &self.source_addresses {
            buf.extend_from_slice(src);
        }
    }
}

// ============================================================================
// Checksum
// ============================================================================

/// Compute the Internet checksum (RFC 1071) over a byte slice.
pub fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    let len = data.len();

    // Sum 16-bit words
    while i + 1 < len {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }

    // Handle odd trailing byte
    if i < len {
        sum += (data[i] as u32) << 8;
    }

    // Fold 32-bit sum to 16 bits
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

// ============================================================================
// Group State and Manager
// ============================================================================

/// State of a joined multicast group
#[derive(Debug, Clone)]
pub struct GroupState {
    /// Number of local members (sockets) in this group
    pub members: u32,
    /// Tick count of last report sent
    pub last_report: u64,
    /// Remaining ticks until next unsolicited report
    pub timer: u64,
    /// Interface index the group is joined on
    pub interface_index: u32,
}

/// Outgoing message produced by the manager (for the network layer to send)
#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    /// Send an IGMPv2 message for an IPv4 group
    Igmp(IgmpMessage),
    /// Send an MLDv2 report with address records for IPv6 groups
    MldReport(Vec<MldAddressRecord>),
}

/// Manages multicast group memberships for IPv4 and IPv6.
#[derive(Default)]
pub struct MulticastManager {
    /// IPv4 groups keyed by group address
    groups_v4: BTreeMap<[u8; 4], GroupState>,
    /// IPv6 groups keyed by group address
    groups_v6: BTreeMap<[u8; 16], GroupState>,
    /// Current tick counter
    current_tick: u64,
    /// Pending outgoing messages
    outbox: Vec<OutgoingMessage>,
}

impl MulticastManager {
    /// Create a new, empty multicast manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Join an IPv4 multicast group. Sends an immediate IGMP report.
    pub fn join_group(&mut self, group: MulticastGroup) -> Result<(), MulticastError> {
        if !is_ipv4_multicast(&group.address) {
            return Err(MulticastError::InvalidAddress);
        }

        if let Some(state) = self.groups_v4.get_mut(&group.address) {
            state.members += 1;
            return Ok(());
        }

        let state = GroupState {
            members: 1,
            last_report: self.current_tick,
            timer: UNSOLICITED_REPORT_INTERVAL,
            interface_index: group.interface_index,
        };
        self.groups_v4.insert(group.address, state);

        // Send immediate membership report
        let report = IgmpMessage::new(IGMP_MEMBERSHIP_REPORT, 0, group.address);
        self.outbox.push(OutgoingMessage::Igmp(report));

        Ok(())
    }

    /// Leave an IPv4 multicast group. Sends a leave message when last member
    /// leaves.
    pub fn leave_group(&mut self, group: MulticastGroup) -> Result<(), MulticastError> {
        if !is_ipv4_multicast(&group.address) {
            return Err(MulticastError::InvalidAddress);
        }

        let state = self
            .groups_v4
            .get_mut(&group.address)
            .ok_or(MulticastError::GroupNotFound)?;

        state.members = state.members.saturating_sub(1);

        if state.members == 0 {
            self.groups_v4.remove(&group.address);
            let leave = IgmpMessage::new(IGMP_LEAVE_GROUP, 0, group.address);
            self.outbox.push(OutgoingMessage::Igmp(leave));
        }

        Ok(())
    }

    /// Check whether the given IPv4 address is a currently-joined group.
    pub fn is_member(&self, address: &[u8; 4]) -> bool {
        self.groups_v4.contains_key(address)
    }

    /// List all currently-joined IPv4 multicast groups.
    pub fn list_groups(&self) -> Vec<MulticastGroup> {
        self.groups_v4
            .iter()
            .map(|(addr, state)| MulticastGroup {
                address: *addr,
                interface_index: state.interface_index,
            })
            .collect()
    }

    /// Join an IPv6 multicast group. Sends an immediate MLDv2 report.
    pub fn join_group_v6(&mut self, group: MulticastGroupV6) -> Result<(), MulticastError> {
        if !is_ipv6_multicast(&group.address) {
            return Err(MulticastError::InvalidAddress);
        }

        if let Some(state) = self.groups_v6.get_mut(&group.address) {
            state.members += 1;
            return Ok(());
        }

        let state = GroupState {
            members: 1,
            last_report: self.current_tick,
            timer: UNSOLICITED_REPORT_INTERVAL,
            interface_index: group.interface_index,
        };
        self.groups_v6.insert(group.address, state);

        // Send immediate MLDv2 IS_EX report (no sources = join all)
        let record = MldAddressRecord::new(MLD_RECORD_IS_EX, group.address);
        self.outbox
            .push(OutgoingMessage::MldReport(alloc::vec![record]));

        Ok(())
    }

    /// Leave an IPv6 multicast group. Sends a TO_IN record when last member
    /// leaves.
    pub fn leave_group_v6(&mut self, group: MulticastGroupV6) -> Result<(), MulticastError> {
        if !is_ipv6_multicast(&group.address) {
            return Err(MulticastError::InvalidAddress);
        }

        let state = self
            .groups_v6
            .get_mut(&group.address)
            .ok_or(MulticastError::GroupNotFound)?;

        state.members = state.members.saturating_sub(1);

        if state.members == 0 {
            self.groups_v6.remove(&group.address);
            let record = MldAddressRecord::new(MLD_RECORD_TO_IN, group.address);
            self.outbox
                .push(OutgoingMessage::MldReport(alloc::vec![record]));
        }

        Ok(())
    }

    /// Check whether the given IPv6 address is a currently-joined group.
    pub fn is_member_v6(&self, address: &[u8; 16]) -> bool {
        self.groups_v6.contains_key(address)
    }

    /// Handle an incoming IGMP query by resetting report timers.
    pub fn handle_query(&mut self, query: &IgmpMessage) {
        let max_resp = query.max_resp_time as u64 * 100; // Convert to ticks (~100ms units)
        if query.group_address == [0, 0, 0, 0] {
            // General query: reset all group timers
            for state in self.groups_v4.values_mut() {
                state.timer = max_resp.min(state.timer);
            }
        } else if let Some(state) = self.groups_v4.get_mut(&query.group_address) {
            // Group-specific query
            state.timer = max_resp.min(state.timer);
        }
    }

    /// Advance the tick counter and generate unsolicited reports for groups
    /// whose timers have expired.
    pub fn tick(&mut self) {
        self.current_tick += 1;

        // Check IPv4 group timers
        let mut reports_v4 = Vec::new();
        for (addr, state) in self.groups_v4.iter_mut() {
            if state.timer > 0 {
                state.timer -= 1;
            }
            if state.timer == 0 {
                state.timer = UNSOLICITED_REPORT_INTERVAL;
                state.last_report = self.current_tick;
                reports_v4.push(*addr);
            }
        }
        for addr in reports_v4 {
            let report = IgmpMessage::new(IGMP_MEMBERSHIP_REPORT, 0, addr);
            self.outbox.push(OutgoingMessage::Igmp(report));
        }

        // Check IPv6 group timers
        let mut reports_v6 = Vec::new();
        for (addr, state) in self.groups_v6.iter_mut() {
            if state.timer > 0 {
                state.timer -= 1;
            }
            if state.timer == 0 {
                state.timer = UNSOLICITED_REPORT_INTERVAL;
                state.last_report = self.current_tick;
                reports_v6.push(*addr);
            }
        }
        if !reports_v6.is_empty() {
            let records: Vec<MldAddressRecord> = reports_v6
                .into_iter()
                .map(|addr| MldAddressRecord::new(MLD_RECORD_IS_EX, addr))
                .collect();
            self.outbox.push(OutgoingMessage::MldReport(records));
        }
    }

    /// Drain all pending outgoing messages.
    pub fn drain_outbox(&mut self) -> Vec<OutgoingMessage> {
        core::mem::take(&mut self.outbox)
    }

    /// Return count of joined IPv4 groups.
    pub fn group_count_v4(&self) -> usize {
        self.groups_v4.len()
    }

    /// Return count of joined IPv6 groups.
    pub fn group_count_v6(&self) -> usize {
        self.groups_v6.len()
    }
}

// ============================================================================
// Global Manager
// ============================================================================

static MULTICAST_MANAGER: OnceLock<Mutex<MulticastManager>> = OnceLock::new();

/// Initialize the global multicast manager.
pub fn init() -> Result<(), MulticastError> {
    MULTICAST_MANAGER
        .set(Mutex::new(MulticastManager::new()))
        .map_err(|_| MulticastError::NotInitialized)
}

/// Access the global multicast manager.
pub fn with_manager<R, F: FnOnce(&mut MulticastManager) -> R>(f: F) -> Result<R, MulticastError> {
    let lock = MULTICAST_MANAGER
        .get()
        .ok_or(MulticastError::NotInitialized)?;
    let mut manager = lock.lock();
    Ok(f(&mut manager))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_multicast_validation() {
        assert!(is_ipv4_multicast(&[224, 0, 0, 1]));
        assert!(is_ipv4_multicast(&[239, 255, 255, 255]));
        assert!(!is_ipv4_multicast(&[192, 168, 1, 1]));
        assert!(!is_ipv4_multicast(&[10, 0, 0, 1]));
    }

    #[test]
    fn test_ipv6_multicast_validation() {
        assert!(is_ipv6_multicast(&[
            0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
        assert!(!is_ipv6_multicast(&[
            0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
    }

    #[test]
    fn test_multicast_group_new_valid() {
        let group = MulticastGroup::new([224, 0, 0, 1], 0);
        assert!(group.is_ok());
    }

    #[test]
    fn test_multicast_group_new_invalid() {
        let group = MulticastGroup::new([192, 168, 1, 1], 0);
        assert_eq!(group, Err(MulticastError::InvalidAddress));
    }

    #[test]
    fn test_igmp_message_serialize_roundtrip() {
        let msg = IgmpMessage::new(IGMP_MEMBERSHIP_REPORT, 0, [224, 0, 0, 1]);
        let bytes = msg.to_bytes();
        let parsed = IgmpMessage::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.msg_type, IGMP_MEMBERSHIP_REPORT);
        assert_eq!(parsed.group_address, [224, 0, 0, 1]);
        assert_eq!(parsed.checksum, msg.checksum);
    }

    #[test]
    fn test_igmp_checksum_verifies() {
        let msg = IgmpMessage::new(IGMP_MEMBERSHIP_QUERY, 100, [224, 0, 0, 1]);
        assert!(msg.verify_checksum());
    }

    #[test]
    fn test_igmp_bad_checksum() {
        let mut msg = IgmpMessage::new(IGMP_MEMBERSHIP_REPORT, 0, [224, 0, 0, 1]);
        msg.checksum = msg.checksum.wrapping_add(1); // Corrupt it
        assert!(!msg.verify_checksum());
    }

    #[test]
    fn test_igmp_from_bytes_too_short() {
        let short = [0u8; 4];
        assert_eq!(
            IgmpMessage::from_bytes(&short),
            Err(MulticastError::MalformedMessage)
        );
    }

    #[test]
    fn test_internet_checksum_rfc_example() {
        // RFC 1071 example: 0x0001 + 0xf203 + ... (simplified test)
        let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        let cksum = internet_checksum(&data);
        // Verify: applying checksum to data+checksum should yield 0
        let mut verify = data.to_vec();
        verify.push((cksum >> 8) as u8);
        verify.push(cksum as u8);
        assert_eq!(internet_checksum(&verify), 0);
    }

    #[test]
    fn test_manager_join_leave() {
        let mut mgr = MulticastManager::new();
        let group = MulticastGroup::new([224, 0, 0, 1], 0).unwrap();

        assert!(!mgr.is_member(&group.address));

        mgr.join_group(group).unwrap();
        assert!(mgr.is_member(&group.address));
        assert_eq!(mgr.group_count_v4(), 1);

        // Should generate an IGMP report
        let msgs = mgr.drain_outbox();
        assert_eq!(msgs.len(), 1);

        mgr.leave_group(group).unwrap();
        assert!(!mgr.is_member(&group.address));
        assert_eq!(mgr.group_count_v4(), 0);

        // Should generate an IGMP leave
        let msgs = mgr.drain_outbox();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_manager_multiple_members() {
        let mut mgr = MulticastManager::new();
        let group = MulticastGroup::new([224, 0, 0, 5], 0).unwrap();

        mgr.join_group(group).unwrap();
        mgr.join_group(group).unwrap(); // Second join increments member count

        // One report for the initial join only
        let msgs = mgr.drain_outbox();
        assert_eq!(msgs.len(), 1);

        // First leave decrements count but group remains
        mgr.leave_group(group).unwrap();
        assert!(mgr.is_member(&group.address));
        assert!(mgr.drain_outbox().is_empty()); // No leave message yet

        // Second leave removes the group
        mgr.leave_group(group).unwrap();
        assert!(!mgr.is_member(&group.address));
        assert_eq!(mgr.drain_outbox().len(), 1); // Leave message now
    }

    #[test]
    fn test_manager_list_groups() {
        let mut mgr = MulticastManager::new();
        let g1 = MulticastGroup::new([224, 0, 0, 1], 0).unwrap();
        let g2 = MulticastGroup::new([239, 1, 2, 3], 1).unwrap();

        mgr.join_group(g1).unwrap();
        mgr.join_group(g2).unwrap();

        let groups = mgr.list_groups();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_manager_leave_unknown_group() {
        let mut mgr = MulticastManager::new();
        let group = MulticastGroup::new([224, 0, 0, 1], 0).unwrap();
        assert_eq!(mgr.leave_group(group), Err(MulticastError::GroupNotFound));
    }

    #[test]
    fn test_manager_handle_query() {
        let mut mgr = MulticastManager::new();
        let group = MulticastGroup::new([224, 0, 0, 1], 0).unwrap();
        mgr.join_group(group).unwrap();
        mgr.drain_outbox(); // Clear join report

        // Send a general query with 10 second max resp time
        let query = IgmpMessage::new(IGMP_MEMBERSHIP_QUERY, 100, [0, 0, 0, 0]);
        mgr.handle_query(&query);

        // Timer should be capped at max_resp_time * 100 = 10000
        let state = mgr.groups_v4.get(&group.address).unwrap();
        assert!(state.timer <= 10000);
    }

    #[test]
    fn test_manager_tick_generates_report() {
        let mut mgr = MulticastManager::new();
        let group = MulticastGroup::new([224, 0, 0, 1], 0).unwrap();
        mgr.join_group(group).unwrap();
        mgr.drain_outbox(); // Clear join report

        // Set timer to 1 so next tick triggers a report
        mgr.groups_v4.get_mut(&group.address).unwrap().timer = 1;
        mgr.tick();

        let msgs = mgr.drain_outbox();
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_mld_message_roundtrip() {
        let msg =
            MldMessage::new_query(1000, [0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        let bytes = msg.to_bytes();
        let parsed = MldMessage::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.msg_type, MLD_QUERY);
        assert_eq!(parsed.max_resp_delay, 1000);
        assert_eq!(parsed.multicast_address[0], 0xFF);
    }

    #[test]
    fn test_mld_address_record_serialize() {
        let record = MldAddressRecord::new(
            MLD_RECORD_IS_EX,
            [0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        );
        let mut buf = Vec::new();
        record.serialize_into(&mut buf);
        assert_eq!(buf.len(), record.wire_size());
        assert_eq!(buf[0], MLD_RECORD_IS_EX);
    }

    #[test]
    fn test_manager_v6_join_leave() {
        let mut mgr = MulticastManager::new();
        let addr = [0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let group = MulticastGroupV6::new(addr, 0).unwrap();

        mgr.join_group_v6(group).unwrap();
        assert!(mgr.is_member_v6(&addr));
        assert_eq!(mgr.group_count_v6(), 1);

        let msgs = mgr.drain_outbox();
        assert_eq!(msgs.len(), 1);

        mgr.leave_group_v6(group).unwrap();
        assert!(!mgr.is_member_v6(&addr));
        assert_eq!(mgr.drain_outbox().len(), 1);
    }
}
