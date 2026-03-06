//! Firewall rule matching and evaluation
//!
//! Provides rule definitions with match criteria (source/dest IP with CIDR,
//! port ranges, protocol, TCP flags, connection state) and actions
//! (Accept, Drop, Reject, Log, Jump, Masquerade, SNAT, DNAT).
//! CIDR matching uses bitmask comparison for efficient subnet checks.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::conntrack::ConntrackState;
use crate::net::{Ipv4Address, Port};

// ============================================================================
// Protocol
// ============================================================================

/// IP protocol for rule matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    /// Match any protocol
    #[default]
    Any,
    /// TCP (protocol 6)
    Tcp,
    /// UDP (protocol 17)
    Udp,
    /// ICMP (protocol 1)
    Icmp,
    /// ICMPv6 (protocol 58)
    Icmpv6,
}

// ============================================================================
// TCP Flags
// ============================================================================

/// TCP flag bitmask for matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TcpFlags {
    /// Flags that must be set
    pub mask: u8,
    /// Expected value after masking
    pub value: u8,
}

impl TcpFlags {
    pub const SYN: u8 = 0x02;
    pub const ACK: u8 = 0x10;
    pub const FIN: u8 = 0x01;
    pub const RST: u8 = 0x04;
    pub const PSH: u8 = 0x08;
    pub const URG: u8 = 0x20;

    /// Create a new TCP flags match
    pub const fn new(mask: u8, value: u8) -> Self {
        Self { mask, value }
    }

    /// Match SYN-only packets (SYN set, ACK cleared)
    pub const fn syn_only() -> Self {
        Self {
            mask: Self::SYN | Self::ACK,
            value: Self::SYN,
        }
    }

    /// Match established connection packets (ACK set)
    pub const fn established() -> Self {
        Self {
            mask: Self::ACK,
            value: Self::ACK,
        }
    }

    /// Check if the given flags match this criteria
    pub fn matches(&self, flags: u8) -> bool {
        (flags & self.mask) == self.value
    }
}

// ============================================================================
// Port Range
// ============================================================================

/// A range of ports for matching (inclusive on both ends)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRange {
    pub start: Port,
    pub end: Port,
}

impl PortRange {
    /// Create a range matching a single port
    pub const fn single(port: Port) -> Self {
        Self {
            start: port,
            end: port,
        }
    }

    /// Create a range of ports (inclusive)
    pub const fn range(start: Port, end: Port) -> Self {
        Self { start, end }
    }

    /// Create a range matching any port
    pub const fn any() -> Self {
        Self {
            start: 0,
            end: 65535,
        }
    }

    /// Check if a port falls within this range
    pub fn contains(&self, port: Port) -> bool {
        port >= self.start && port <= self.end
    }
}

impl Default for PortRange {
    fn default() -> Self {
        Self::any()
    }
}

// ============================================================================
// CIDR Address
// ============================================================================

/// IPv4 address with CIDR prefix length for subnet matching
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CidrAddress {
    /// Base address
    pub address: Ipv4Address,
    /// Prefix length (0-32)
    pub prefix_len: u8,
}

impl CidrAddress {
    /// Create a CIDR address
    pub const fn new(address: Ipv4Address, prefix_len: u8) -> Self {
        Self {
            address,
            prefix_len,
        }
    }

    /// Create a CIDR that matches any address (0.0.0.0/0)
    pub const fn any() -> Self {
        Self {
            address: Ipv4Address::ANY,
            prefix_len: 0,
        }
    }

    /// Create a CIDR that matches a single host (/32)
    pub const fn host(address: Ipv4Address) -> Self {
        Self {
            address,
            prefix_len: 32,
        }
    }

    /// Compute the subnet mask as a u32
    fn mask(&self) -> u32 {
        if self.prefix_len == 0 {
            0
        } else if self.prefix_len >= 32 {
            0xFFFF_FFFF
        } else {
            0xFFFF_FFFF << (32 - self.prefix_len)
        }
    }

    /// Check if an address matches this CIDR block
    pub fn matches(&self, addr: &Ipv4Address) -> bool {
        let mask = self.mask();
        (self.address.to_u32() & mask) == (addr.to_u32() & mask)
    }
}

impl Default for CidrAddress {
    fn default() -> Self {
        Self::any()
    }
}

// ============================================================================
// Match Criteria
// ============================================================================

/// Criteria for matching packets against a firewall rule
#[derive(Debug, Clone, Default)]
pub struct MatchCriteria {
    /// Source IP with CIDR mask (None = match any)
    pub src_ip: Option<CidrAddress>,
    /// Destination IP with CIDR mask (None = match any)
    pub dst_ip: Option<CidrAddress>,
    /// Source port range (None = match any)
    pub src_port: Option<PortRange>,
    /// Destination port range (None = match any)
    pub dst_port: Option<PortRange>,
    /// IP protocol (Any = match all protocols)
    pub protocol: Protocol,
    /// TCP flags to match (None = don't check flags)
    pub tcp_flags: Option<TcpFlags>,
    /// Connection tracking state (None = don't check state)
    pub conn_state: Option<ConntrackState>,
    /// Negate source IP match
    pub negate_src: bool,
    /// Negate destination IP match
    pub negate_dst: bool,
}

impl MatchCriteria {
    /// Create criteria matching everything
    pub fn new() -> Self {
        Self::default()
    }

    /// Set source IP CIDR
    pub fn with_src_ip(mut self, cidr: CidrAddress) -> Self {
        self.src_ip = Some(cidr);
        self
    }

    /// Set destination IP CIDR
    pub fn with_dst_ip(mut self, cidr: CidrAddress) -> Self {
        self.dst_ip = Some(cidr);
        self
    }

    /// Set source port range
    pub fn with_src_port(mut self, range: PortRange) -> Self {
        self.src_port = Some(range);
        self
    }

    /// Set destination port range
    pub fn with_dst_port(mut self, range: PortRange) -> Self {
        self.dst_port = Some(range);
        self
    }

    /// Set protocol
    pub fn with_protocol(mut self, proto: Protocol) -> Self {
        self.protocol = proto;
        self
    }

    /// Set TCP flags match
    pub fn with_tcp_flags(mut self, flags: TcpFlags) -> Self {
        self.tcp_flags = Some(flags);
        self
    }

    /// Set connection state match
    pub fn with_conn_state(mut self, state: ConntrackState) -> Self {
        self.conn_state = Some(state);
        self
    }
}

// ============================================================================
// Rule Actions
// ============================================================================

/// Action to take when a rule matches
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RuleAction {
    /// Allow the packet
    #[default]
    Accept,
    /// Silently drop the packet
    Drop,
    /// Drop and send ICMP unreachable
    Reject,
    /// Log the packet and continue evaluation
    Log,
    /// Jump to another chain
    Jump(String),
    /// Return from current chain to caller
    Return,
    /// Source NAT with masquerading (use outgoing interface address)
    Masquerade,
    /// Source NAT to a specific address
    Snat(Ipv4Address),
    /// Destination NAT to a specific address and port
    Dnat(Ipv4Address, Port),
}

// ============================================================================
// Packet Metadata
// ============================================================================

/// Extracted packet metadata used for rule evaluation
///
/// This avoids passing raw packet bytes -- the caller extracts relevant
/// header fields before calling the firewall engine.
#[derive(Debug, Clone)]
pub struct PacketMetadata {
    /// Source IPv4 address
    pub src_ip: Ipv4Address,
    /// Destination IPv4 address
    pub dst_ip: Ipv4Address,
    /// Source port (0 for ICMP)
    pub src_port: Port,
    /// Destination port (0 for ICMP)
    pub dst_port: Port,
    /// IP protocol
    pub protocol: Protocol,
    /// TCP flags (raw byte)
    pub tcp_flags: u8,
    /// Connection tracking state (if known)
    pub conn_state: Option<ConntrackState>,
    /// Total packet length in bytes
    pub packet_len: u16,
}

impl Default for PacketMetadata {
    fn default() -> Self {
        Self {
            src_ip: Ipv4Address::ANY,
            dst_ip: Ipv4Address::ANY,
            src_port: 0,
            dst_port: 0,
            protocol: Protocol::default(),
            tcp_flags: 0,
            conn_state: None,
            packet_len: 0,
        }
    }
}

// ============================================================================
// Firewall Rule
// ============================================================================

/// A single firewall rule with match criteria, action, and counters
#[derive(Debug, Clone)]
pub struct FirewallRule {
    /// Unique rule identifier
    pub id: u64,
    /// Priority (lower = evaluated first within a chain)
    pub priority: u32,
    /// Match criteria
    pub criteria: MatchCriteria,
    /// Action to take on match
    pub action: RuleAction,
    /// Packet counter
    pub packets: u64,
    /// Byte counter
    pub bytes: u64,
    /// Whether this rule is active
    pub enabled: bool,
    /// Optional comment/description
    pub comment: String,
}

impl FirewallRule {
    /// Create a new rule with the given criteria and action
    pub fn new(id: u64, criteria: MatchCriteria, action: RuleAction) -> Self {
        Self {
            id,
            priority: 0,
            criteria,
            action,
            packets: 0,
            bytes: 0,
            enabled: true,
            comment: String::new(),
        }
    }

    /// Set the rule priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set the rule comment
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = String::from(comment);
        self
    }

    /// Check if this rule matches the given packet metadata
    pub fn matches_packet(&self, meta: &PacketMetadata) -> bool {
        // Protocol check
        if self.criteria.protocol != Protocol::Any && self.criteria.protocol != meta.protocol {
            return false;
        }

        // Source IP check
        if let Some(ref cidr) = self.criteria.src_ip {
            let matches = cidr.matches(&meta.src_ip);
            if matches == self.criteria.negate_src {
                return false;
            }
        }

        // Destination IP check
        if let Some(ref cidr) = self.criteria.dst_ip {
            let matches = cidr.matches(&meta.dst_ip);
            if matches == self.criteria.negate_dst {
                return false;
            }
        }

        // Source port check
        if let Some(ref range) = self.criteria.src_port {
            if !range.contains(meta.src_port) {
                return false;
            }
        }

        // Destination port check
        if let Some(ref range) = self.criteria.dst_port {
            if !range.contains(meta.dst_port) {
                return false;
            }
        }

        // TCP flags check
        if let Some(ref flags) = self.criteria.tcp_flags {
            if !flags.matches(meta.tcp_flags) {
                return false;
            }
        }

        // Connection state check
        if let Some(ref expected_state) = self.criteria.conn_state {
            match meta.conn_state {
                Some(ref actual_state) if actual_state == expected_state => {}
                _ => return false,
            }
        }

        true
    }

    /// Reset packet/byte counters
    pub fn reset_counters(&mut self) {
        self.packets = 0;
        self.bytes = 0;
    }
}

// ============================================================================
// Rule Engine
// ============================================================================

/// Manages all firewall rules and provides lookup by ID
pub struct RuleEngine {
    /// All rules indexed by ID
    rules: BTreeMap<u64, FirewallRule>,
    /// Next available rule ID
    next_id: u64,
}

impl RuleEngine {
    /// Create a new empty rule engine
    pub fn new() -> Self {
        Self {
            rules: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Add a rule and return its assigned ID
    pub fn add_rule(&mut self, mut rule: FirewallRule) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        rule.id = id;
        self.rules.insert(id, rule);
        id
    }

    /// Remove a rule by ID
    pub fn remove_rule(&mut self, id: u64) -> Option<FirewallRule> {
        self.rules.remove(&id)
    }

    /// Get a rule by ID (immutable)
    pub fn get_rule(&self, id: u64) -> Option<&FirewallRule> {
        self.rules.get(&id)
    }

    /// Get a rule by ID (mutable)
    pub fn get_rule_mut(&mut self, id: u64) -> Option<&mut FirewallRule> {
        self.rules.get_mut(&id)
    }

    /// Number of rules
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Evaluate a packet against a list of rule IDs, returning the first
    /// matching rule's action
    pub fn evaluate(&mut self, rule_ids: &[u64], metadata: &PacketMetadata) -> Option<RuleAction> {
        for &id in rule_ids {
            if let Some(rule) = self.rules.get_mut(&id) {
                if rule.enabled && rule.matches_packet(metadata) {
                    rule.packets += 1;
                    rule.bytes += metadata.packet_len as u64;
                    return Some(rule.action.clone());
                }
            }
        }
        None
    }

    /// Get all rules sorted by priority
    pub fn rules_by_priority(&self) -> Vec<&FirewallRule> {
        let mut rules: Vec<&FirewallRule> = self.rules.values().collect();
        rules.sort_by_key(|r| r.priority);
        rules
    }
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata() -> PacketMetadata {
        PacketMetadata {
            src_ip: Ipv4Address::new(192, 168, 1, 100),
            dst_ip: Ipv4Address::new(10, 0, 0, 1),
            src_port: 12345,
            dst_port: 80,
            protocol: Protocol::Tcp,
            tcp_flags: TcpFlags::SYN,
            conn_state: Some(ConntrackState::New),
            packet_len: 64,
        }
    }

    #[test]
    fn test_protocol_default() {
        assert_eq!(Protocol::default(), Protocol::Any);
    }

    #[test]
    fn test_port_range_single() {
        let range = PortRange::single(80);
        assert!(range.contains(80));
        assert!(!range.contains(81));
        assert!(!range.contains(79));
    }

    #[test]
    fn test_port_range_range() {
        let range = PortRange::range(1024, 65535);
        assert!(!range.contains(80));
        assert!(range.contains(1024));
        assert!(range.contains(50000));
        assert!(range.contains(65535));
    }

    #[test]
    fn test_port_range_any() {
        let range = PortRange::any();
        assert!(range.contains(0));
        assert!(range.contains(80));
        assert!(range.contains(65535));
    }

    #[test]
    fn test_cidr_matches_slash32() {
        let cidr = CidrAddress::host(Ipv4Address::new(192, 168, 1, 1));
        assert!(cidr.matches(&Ipv4Address::new(192, 168, 1, 1)));
        assert!(!cidr.matches(&Ipv4Address::new(192, 168, 1, 2)));
    }

    #[test]
    fn test_cidr_matches_slash24() {
        let cidr = CidrAddress::new(Ipv4Address::new(192, 168, 1, 0), 24);
        assert!(cidr.matches(&Ipv4Address::new(192, 168, 1, 0)));
        assert!(cidr.matches(&Ipv4Address::new(192, 168, 1, 255)));
        assert!(!cidr.matches(&Ipv4Address::new(192, 168, 2, 1)));
    }

    #[test]
    fn test_cidr_matches_slash0() {
        let cidr = CidrAddress::any();
        assert!(cidr.matches(&Ipv4Address::new(1, 2, 3, 4)));
        assert!(cidr.matches(&Ipv4Address::new(255, 255, 255, 255)));
    }

    #[test]
    fn test_cidr_matches_slash16() {
        let cidr = CidrAddress::new(Ipv4Address::new(10, 0, 0, 0), 16);
        assert!(cidr.matches(&Ipv4Address::new(10, 0, 0, 1)));
        assert!(cidr.matches(&Ipv4Address::new(10, 0, 255, 255)));
        assert!(!cidr.matches(&Ipv4Address::new(10, 1, 0, 1)));
    }

    #[test]
    fn test_tcp_flags_syn_only() {
        let flags = TcpFlags::syn_only();
        assert!(flags.matches(TcpFlags::SYN));
        assert!(!flags.matches(TcpFlags::SYN | TcpFlags::ACK));
        assert!(!flags.matches(TcpFlags::ACK));
    }

    #[test]
    fn test_tcp_flags_established() {
        let flags = TcpFlags::established();
        assert!(flags.matches(TcpFlags::ACK));
        assert!(flags.matches(TcpFlags::SYN | TcpFlags::ACK));
        assert!(!flags.matches(TcpFlags::SYN));
    }

    #[test]
    fn test_rule_matches_all() {
        let rule = FirewallRule::new(1, MatchCriteria::new(), RuleAction::Accept);
        let meta = test_metadata();
        assert!(rule.matches_packet(&meta));
    }

    #[test]
    fn test_rule_matches_src_ip() {
        let criteria = MatchCriteria::new()
            .with_src_ip(CidrAddress::new(Ipv4Address::new(192, 168, 1, 0), 24));
        let rule = FirewallRule::new(1, criteria, RuleAction::Accept);
        let meta = test_metadata();
        assert!(rule.matches_packet(&meta));

        let mut meta2 = test_metadata();
        meta2.src_ip = Ipv4Address::new(10, 0, 0, 5);
        assert!(!rule.matches_packet(&meta2));
    }

    #[test]
    fn test_rule_matches_dst_port() {
        let criteria = MatchCriteria::new()
            .with_protocol(Protocol::Tcp)
            .with_dst_port(PortRange::single(80));
        let rule = FirewallRule::new(1, criteria, RuleAction::Accept);
        let meta = test_metadata();
        assert!(rule.matches_packet(&meta));

        let mut meta2 = test_metadata();
        meta2.dst_port = 443;
        assert!(!rule.matches_packet(&meta2));
    }

    #[test]
    fn test_rule_matches_protocol() {
        let criteria = MatchCriteria::new().with_protocol(Protocol::Udp);
        let rule = FirewallRule::new(1, criteria, RuleAction::Drop);
        let meta = test_metadata(); // TCP
        assert!(!rule.matches_packet(&meta));
    }

    #[test]
    fn test_rule_matches_conn_state() {
        let criteria = MatchCriteria::new().with_conn_state(ConntrackState::Established);
        let rule = FirewallRule::new(1, criteria, RuleAction::Accept);
        let meta = test_metadata(); // New
        assert!(!rule.matches_packet(&meta));

        let mut meta2 = test_metadata();
        meta2.conn_state = Some(ConntrackState::Established);
        assert!(rule.matches_packet(&meta2));
    }

    #[test]
    fn test_rule_engine_add_evaluate() {
        let mut engine = RuleEngine::new();
        let criteria = MatchCriteria::new()
            .with_protocol(Protocol::Tcp)
            .with_dst_port(PortRange::single(80));
        let rule = FirewallRule::new(0, criteria, RuleAction::Accept);
        let id = engine.add_rule(rule);
        assert_eq!(id, 1);
        assert_eq!(engine.rule_count(), 1);

        let meta = test_metadata();
        let action = engine.evaluate(&[id], &meta);
        assert_eq!(action, Some(RuleAction::Accept));

        // Check counters
        let r = engine.get_rule(id).unwrap();
        assert_eq!(r.packets, 1);
        assert_eq!(r.bytes, 64);
    }

    #[test]
    fn test_rule_engine_no_match() {
        let mut engine = RuleEngine::new();
        let criteria = MatchCriteria::new().with_protocol(Protocol::Udp);
        let rule = FirewallRule::new(0, criteria, RuleAction::Drop);
        let id = engine.add_rule(rule);

        let meta = test_metadata(); // TCP
        let action = engine.evaluate(&[id], &meta);
        assert_eq!(action, None);
    }

    #[test]
    fn test_rule_engine_remove() {
        let mut engine = RuleEngine::new();
        let rule = FirewallRule::new(0, MatchCriteria::new(), RuleAction::Drop);
        let id = engine.add_rule(rule);
        assert_eq!(engine.rule_count(), 1);

        engine.remove_rule(id);
        assert_eq!(engine.rule_count(), 0);
    }

    #[test]
    fn test_rule_disabled() {
        let mut engine = RuleEngine::new();
        let mut rule = FirewallRule::new(0, MatchCriteria::new(), RuleAction::Drop);
        rule.enabled = false;
        let id = engine.add_rule(rule);

        let meta = test_metadata();
        let action = engine.evaluate(&[id], &meta);
        assert_eq!(action, None);
    }
}
