//! Connection tracking (conntrack) for stateful packet inspection
//!
//! Tracks network connections using 5-tuple keys (src_ip, dst_ip, src_port,
//! dst_port, protocol). Maintains TCP state machine for accurate connection
//! lifecycle tracking. Supports garbage collection of expired entries and
//! enforces a maximum entry limit to prevent resource exhaustion.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::rules::Protocol;
use crate::{error::KernelError, net::Ipv4Address, sync::once_lock::GlobalState};

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of connection tracking entries
const MAX_CONNTRACK_ENTRIES: usize = 65536;

/// Default timeout for established TCP connections (in ticks, ~7200s at 1Hz)
const TCP_ESTABLISHED_TIMEOUT: u64 = 7200;

/// Default timeout for new/half-open connections (in ticks, ~120s)
const TCP_NEW_TIMEOUT: u64 = 120;

/// Default timeout for TIME_WAIT state (in ticks, ~120s = 2*MSL)
const TCP_TIME_WAIT_TIMEOUT: u64 = 120;

/// Default timeout for UDP connections (in ticks, ~30s)
const UDP_TIMEOUT: u64 = 30;

/// Default timeout for ICMP entries (in ticks, ~30s)
const ICMP_TIMEOUT: u64 = 30;

// ============================================================================
// Connection Tracking Key
// ============================================================================

/// 5-tuple identifying a unique connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConntrackKey {
    /// Source IP address
    pub src_ip: Ipv4Address,
    /// Destination IP address
    pub dst_ip: Ipv4Address,
    /// Source port (0 for ICMP)
    pub src_port: u16,
    /// Destination port (0 for ICMP)
    pub dst_port: u16,
    /// IP protocol
    pub protocol: u8,
}

impl ConntrackKey {
    /// Create a new connection tracking key
    pub fn new(
        src_ip: Ipv4Address,
        dst_ip: Ipv4Address,
        src_port: u16,
        dst_port: u16,
        protocol: u8,
    ) -> Self {
        Self {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
        }
    }

    /// Create the reverse key (for tracking reply direction)
    pub fn reverse(&self) -> Self {
        Self {
            src_ip: self.dst_ip,
            dst_ip: self.src_ip,
            src_port: self.dst_port,
            dst_port: self.src_port,
            protocol: self.protocol,
        }
    }

    /// Protocol number for TCP
    pub const PROTO_TCP: u8 = 6;
    /// Protocol number for UDP
    pub const PROTO_UDP: u8 = 17;
    /// Protocol number for ICMP
    pub const PROTO_ICMP: u8 = 1;

    /// Convert from rules::Protocol to protocol number
    pub fn protocol_to_num(proto: Protocol) -> u8 {
        match proto {
            Protocol::Tcp => Self::PROTO_TCP,
            Protocol::Udp => Self::PROTO_UDP,
            Protocol::Icmp => Self::PROTO_ICMP,
            Protocol::Icmpv6 => 58,
            Protocol::Any => 0,
        }
    }
}

// ============================================================================
// Connection State
// ============================================================================

/// High-level connection tracking state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConntrackState {
    /// First packet of a new connection
    #[default]
    New,
    /// Connection has seen traffic in both directions
    Established,
    /// Related to an existing connection (e.g., ICMP error, FTP data)
    Related,
    /// Invalid or unexpected packet
    Invalid,
    /// TCP TIME_WAIT state
    TimeWait,
}

/// Detailed TCP connection state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TcpConnState {
    /// No connection
    #[default]
    None,
    /// SYN sent by originator
    SynSent,
    /// SYN-ACK received (SYN sent by responder)
    SynRecv,
    /// Three-way handshake complete
    Established,
    /// FIN sent by originator
    FinWait1,
    /// FIN acknowledged by responder
    FinWait2,
    /// Both sides closing
    Closing,
    /// FIN received while in ESTABLISHED
    CloseWait,
    /// FIN sent after CLOSE_WAIT
    LastAck,
    /// Waiting for old duplicates to expire
    TimeWait,
    /// Connection fully closed
    Closed,
}

// ============================================================================
// NAT Info (stored in conntrack entries)
// ============================================================================

/// NAT information associated with a connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatInfo {
    /// Original source before SNAT
    pub original_src_ip: Ipv4Address,
    pub original_src_port: u16,
    /// Translated source after SNAT
    pub translated_src_ip: Ipv4Address,
    pub translated_src_port: u16,
    /// Original destination before DNAT
    pub original_dst_ip: Ipv4Address,
    pub original_dst_port: u16,
    /// Translated destination after DNAT
    pub translated_dst_ip: Ipv4Address,
    pub translated_dst_port: u16,
}

// ============================================================================
// Connection Tracking Entry
// ============================================================================

/// A single connection tracking entry
#[derive(Debug, Clone)]
pub struct ConntrackEntry {
    /// Connection 5-tuple key
    pub key: ConntrackKey,
    /// High-level connection state
    pub state: ConntrackState,
    /// Detailed TCP state (only meaningful for TCP)
    pub tcp_state: TcpConnState,
    /// Timeout in ticks (entry expires when current_tick >= last_seen +
    /// timeout)
    pub timeout_ticks: u64,
    /// Number of packets seen
    pub packet_count: u64,
    /// Number of bytes seen
    pub byte_count: u64,
    /// Tick counter when entry was last updated
    pub last_seen: u64,
    /// Optional NAT translation info
    pub nat_info: Option<NatInfo>,
    /// Whether reply traffic has been seen
    pub reply_seen: bool,
}

impl ConntrackEntry {
    /// Create a new conntrack entry for a first-seen packet
    pub fn new(key: ConntrackKey, protocol: u8) -> Self {
        let timeout = match protocol {
            ConntrackKey::PROTO_TCP => TCP_NEW_TIMEOUT,
            ConntrackKey::PROTO_UDP => UDP_TIMEOUT,
            ConntrackKey::PROTO_ICMP => ICMP_TIMEOUT,
            _ => UDP_TIMEOUT,
        };

        Self {
            key,
            state: ConntrackState::New,
            tcp_state: if protocol == ConntrackKey::PROTO_TCP {
                TcpConnState::SynSent
            } else {
                TcpConnState::None
            },
            timeout_ticks: timeout,
            packet_count: 1,
            byte_count: 0,
            last_seen: 0,
            nat_info: None,
            reply_seen: false,
        }
    }

    /// Check if this entry has expired
    pub fn is_expired(&self, current_tick: u64) -> bool {
        current_tick >= self.last_seen + self.timeout_ticks
    }

    /// Update the last-seen timestamp and packet counters
    pub fn update(&mut self, current_tick: u64, bytes: u64) {
        self.last_seen = current_tick;
        self.packet_count += 1;
        self.byte_count += bytes;
    }

    /// Mark that reply traffic has been seen, promoting to Established
    pub fn mark_reply_seen(&mut self) {
        if !self.reply_seen {
            self.reply_seen = true;
            if self.state == ConntrackState::New {
                self.state = ConntrackState::Established;
                if self.key.protocol == ConntrackKey::PROTO_TCP {
                    self.timeout_ticks = TCP_ESTABLISHED_TIMEOUT;
                }
            }
        }
    }
}

// ============================================================================
// TCP State Machine
// ============================================================================

/// TCP flag constants for state machine transitions
const TCP_SYN: u8 = 0x02;
const TCP_ACK: u8 = 0x10;
const TCP_FIN: u8 = 0x01;
const TCP_RST: u8 = 0x04;

/// Update the TCP connection state based on observed flags
///
/// `is_reply` indicates whether this packet is from the responder (reply
/// direction).
pub fn update_tcp_state(entry: &mut ConntrackEntry, tcp_flags: u8, is_reply: bool) {
    let has_syn = tcp_flags & TCP_SYN != 0;
    let has_ack = tcp_flags & TCP_ACK != 0;
    let has_fin = tcp_flags & TCP_FIN != 0;
    let has_rst = tcp_flags & TCP_RST != 0;

    // RST immediately closes the connection
    if has_rst {
        entry.tcp_state = TcpConnState::Closed;
        entry.state = ConntrackState::Invalid;
        entry.timeout_ticks = TCP_NEW_TIMEOUT;
        return;
    }

    entry.tcp_state = match entry.tcp_state {
        TcpConnState::None => {
            if has_syn && !has_ack {
                TcpConnState::SynSent
            } else {
                TcpConnState::None
            }
        }
        TcpConnState::SynSent => {
            if is_reply && has_syn && has_ack {
                entry.mark_reply_seen();
                TcpConnState::SynRecv
            } else {
                TcpConnState::SynSent
            }
        }
        TcpConnState::SynRecv => {
            if !is_reply && has_ack {
                entry.state = ConntrackState::Established;
                entry.timeout_ticks = TCP_ESTABLISHED_TIMEOUT;
                TcpConnState::Established
            } else {
                TcpConnState::SynRecv
            }
        }
        TcpConnState::Established => {
            if has_fin {
                if is_reply {
                    TcpConnState::CloseWait
                } else {
                    TcpConnState::FinWait1
                }
            } else {
                TcpConnState::Established
            }
        }
        TcpConnState::FinWait1 => {
            if is_reply && has_fin && has_ack {
                entry.timeout_ticks = TCP_TIME_WAIT_TIMEOUT;
                entry.state = ConntrackState::TimeWait;
                TcpConnState::TimeWait
            } else if is_reply && has_ack {
                TcpConnState::FinWait2
            } else if is_reply && has_fin {
                TcpConnState::Closing
            } else {
                TcpConnState::FinWait1
            }
        }
        TcpConnState::FinWait2 => {
            if is_reply && has_fin {
                entry.timeout_ticks = TCP_TIME_WAIT_TIMEOUT;
                entry.state = ConntrackState::TimeWait;
                TcpConnState::TimeWait
            } else {
                TcpConnState::FinWait2
            }
        }
        TcpConnState::Closing => {
            if has_ack {
                entry.timeout_ticks = TCP_TIME_WAIT_TIMEOUT;
                entry.state = ConntrackState::TimeWait;
                TcpConnState::TimeWait
            } else {
                TcpConnState::Closing
            }
        }
        TcpConnState::CloseWait => {
            if !is_reply && has_fin {
                TcpConnState::LastAck
            } else {
                TcpConnState::CloseWait
            }
        }
        TcpConnState::LastAck => {
            if is_reply && has_ack {
                entry.state = ConntrackState::TimeWait;
                entry.timeout_ticks = TCP_TIME_WAIT_TIMEOUT;
                TcpConnState::Closed
            } else {
                TcpConnState::LastAck
            }
        }
        TcpConnState::TimeWait | TcpConnState::Closed => entry.tcp_state,
    };
}

// ============================================================================
// Connection Tracking Table
// ============================================================================

/// Connection tracking table managing all active connections
pub struct ConntrackTable {
    /// Active connections indexed by 5-tuple key
    entries: BTreeMap<ConntrackKey, ConntrackEntry>,
    /// Maximum number of entries
    max_entries: usize,
    /// Current tick counter (monotonically increasing)
    current_tick: u64,
    /// Total entries created over lifetime
    total_created: u64,
    /// Total entries expired/garbage-collected
    total_expired: u64,
}

impl ConntrackTable {
    /// Create a new connection tracking table
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            max_entries: MAX_CONNTRACK_ENTRIES,
            current_tick: 0,
            total_created: 0,
            total_expired: 0,
        }
    }

    /// Create with a custom maximum entry count
    pub fn with_max_entries(max: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            max_entries: max,
            current_tick: 0,
            total_created: 0,
            total_expired: 0,
        }
    }

    /// Advance the tick counter
    pub fn tick(&mut self) {
        self.current_tick += 1;
    }

    /// Set the current tick counter
    pub fn set_tick(&mut self, tick: u64) {
        self.current_tick = tick;
    }

    /// Number of active entries
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Look up an entry by key
    pub fn lookup(&self, key: &ConntrackKey) -> Option<&ConntrackEntry> {
        self.entries.get(key)
    }

    /// Look up an entry mutably
    pub fn lookup_mut(&mut self, key: &ConntrackKey) -> Option<&mut ConntrackEntry> {
        self.entries.get_mut(key)
    }

    /// Insert or update a connection tracking entry
    ///
    /// Returns the classified state for this packet.
    pub fn track_packet(&mut self, key: ConntrackKey, bytes: u64, tcp_flags: u8) -> ConntrackState {
        let current_tick = self.current_tick;

        // Check forward direction
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.update(current_tick, bytes);
            if key.protocol == ConntrackKey::PROTO_TCP {
                update_tcp_state(entry, tcp_flags, false);
            }
            return entry.state;
        }

        // Check reverse direction (reply packet)
        let reverse_key = key.reverse();
        if let Some(entry) = self.entries.get_mut(&reverse_key) {
            entry.update(current_tick, bytes);
            entry.mark_reply_seen();
            if reverse_key.protocol == ConntrackKey::PROTO_TCP {
                update_tcp_state(entry, tcp_flags, true);
            }
            return entry.state;
        }

        // New connection
        if self.entries.len() >= self.max_entries {
            // Table full -- run garbage collection and try again
            self.gc();
            if self.entries.len() >= self.max_entries {
                return ConntrackState::Invalid;
            }
        }

        let mut entry = ConntrackEntry::new(key, key.protocol);
        entry.last_seen = current_tick;
        entry.byte_count = bytes;
        self.entries.insert(key, entry);
        self.total_created += 1;
        ConntrackState::New
    }

    /// Remove an entry by key
    pub fn remove(&mut self, key: &ConntrackKey) -> Option<ConntrackEntry> {
        self.entries.remove(key)
    }

    /// Garbage collect expired entries
    pub fn gc(&mut self) -> usize {
        let current_tick = self.current_tick;
        let before = self.entries.len();

        // Collect expired keys
        let expired_keys: Vec<ConntrackKey> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.is_expired(current_tick))
            .map(|(key, _)| *key)
            .collect();

        for key in &expired_keys {
            self.entries.remove(key);
        }

        let removed = before - self.entries.len();
        self.total_expired += removed as u64;
        removed
    }

    /// Classify a packet based on existing connection state
    pub fn classify_packet(&self, key: &ConntrackKey) -> ConntrackState {
        // Check forward direction
        if let Some(entry) = self.entries.get(key) {
            if entry.is_expired(self.current_tick) {
                return ConntrackState::Invalid;
            }
            return entry.state;
        }

        // Check reverse direction
        let reverse = key.reverse();
        if let Some(entry) = self.entries.get(&reverse) {
            if entry.is_expired(self.current_tick) {
                return ConntrackState::Invalid;
            }
            if entry.reply_seen {
                return ConntrackState::Established;
            }
            return ConntrackState::New;
        }

        ConntrackState::New
    }

    /// Get statistics
    pub fn stats(&self) -> ConntrackStats {
        ConntrackStats {
            active_entries: self.entries.len() as u64,
            max_entries: self.max_entries as u64,
            total_created: self.total_created,
            total_expired: self.total_expired,
        }
    }
}

impl Default for ConntrackTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection tracking statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct ConntrackStats {
    pub active_entries: u64,
    pub max_entries: u64,
    pub total_created: u64,
    pub total_expired: u64,
}

// ============================================================================
// Global State
// ============================================================================

static CONNTRACK_TABLE: GlobalState<spin::Mutex<ConntrackTable>> = GlobalState::new();

/// Initialize the connection tracking subsystem
pub fn init() -> Result<(), KernelError> {
    CONNTRACK_TABLE
        .init(spin::Mutex::new(ConntrackTable::new()))
        .map_err(|_| KernelError::InvalidAddress { addr: 0 })?;
    Ok(())
}

/// Access the global conntrack table
pub fn with_conntrack<R, F: FnOnce(&mut ConntrackTable) -> R>(f: F) -> Option<R> {
    CONNTRACK_TABLE.with(|lock| {
        let mut table = lock.lock();
        f(&mut table)
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp_key() -> ConntrackKey {
        ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 100),
            Ipv4Address::new(10, 0, 0, 1),
            12345,
            80,
            ConntrackKey::PROTO_TCP,
        )
    }

    fn udp_key() -> ConntrackKey {
        ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 100),
            Ipv4Address::new(10, 0, 0, 1),
            5000,
            53,
            ConntrackKey::PROTO_UDP,
        )
    }

    #[test]
    fn test_conntrack_key_reverse() {
        let key = tcp_key();
        let rev = key.reverse();
        assert_eq!(rev.src_ip, key.dst_ip);
        assert_eq!(rev.dst_ip, key.src_ip);
        assert_eq!(rev.src_port, key.dst_port);
        assert_eq!(rev.dst_port, key.src_port);
        assert_eq!(rev.protocol, key.protocol);
    }

    #[test]
    fn test_conntrack_state_default() {
        assert_eq!(ConntrackState::default(), ConntrackState::New);
    }

    #[test]
    fn test_conntrack_entry_new_tcp() {
        let entry = ConntrackEntry::new(tcp_key(), ConntrackKey::PROTO_TCP);
        assert_eq!(entry.state, ConntrackState::New);
        assert_eq!(entry.tcp_state, TcpConnState::SynSent);
        assert_eq!(entry.timeout_ticks, TCP_NEW_TIMEOUT);
        assert!(!entry.reply_seen);
    }

    #[test]
    fn test_conntrack_entry_new_udp() {
        let entry = ConntrackEntry::new(udp_key(), ConntrackKey::PROTO_UDP);
        assert_eq!(entry.state, ConntrackState::New);
        assert_eq!(entry.tcp_state, TcpConnState::None);
        assert_eq!(entry.timeout_ticks, UDP_TIMEOUT);
    }

    #[test]
    fn test_conntrack_entry_expired() {
        let mut entry = ConntrackEntry::new(tcp_key(), ConntrackKey::PROTO_TCP);
        entry.last_seen = 100;
        entry.timeout_ticks = 50;
        assert!(entry.is_expired(151));
        assert!(!entry.is_expired(149));
        assert!(entry.is_expired(150));
    }

    #[test]
    fn test_conntrack_table_track_new() {
        let mut table = ConntrackTable::new();
        let state = table.track_packet(tcp_key(), 64, TCP_SYN);
        assert_eq!(state, ConntrackState::New);
        assert_eq!(table.entry_count(), 1);
    }

    #[test]
    fn test_conntrack_table_track_reply() {
        let mut table = ConntrackTable::new();
        let key = tcp_key();

        // Original SYN
        table.track_packet(key, 64, TCP_SYN);

        // Reply SYN-ACK
        let rev = key.reverse();
        let state = table.track_packet(rev, 64, TCP_SYN | TCP_ACK);
        assert_eq!(state, ConntrackState::Established);
        assert_eq!(table.entry_count(), 1); // Still one entry (reverse lookup)
    }

    #[test]
    fn test_conntrack_table_max_entries() {
        let mut table = ConntrackTable::with_max_entries(2);
        let k1 = ConntrackKey::new(
            Ipv4Address::new(10, 0, 0, 1),
            Ipv4Address::new(10, 0, 0, 2),
            1000,
            80,
            ConntrackKey::PROTO_TCP,
        );
        let k2 = ConntrackKey::new(
            Ipv4Address::new(10, 0, 0, 3),
            Ipv4Address::new(10, 0, 0, 4),
            1001,
            80,
            ConntrackKey::PROTO_TCP,
        );
        let k3 = ConntrackKey::new(
            Ipv4Address::new(10, 0, 0, 5),
            Ipv4Address::new(10, 0, 0, 6),
            1002,
            80,
            ConntrackKey::PROTO_TCP,
        );

        table.track_packet(k1, 64, TCP_SYN);
        table.track_packet(k2, 64, TCP_SYN);

        // Table full, no expired entries -> Invalid
        let state = table.track_packet(k3, 64, TCP_SYN);
        assert_eq!(state, ConntrackState::Invalid);
        assert_eq!(table.entry_count(), 2);
    }

    #[test]
    fn test_conntrack_table_gc() {
        let mut table = ConntrackTable::new();
        let key = tcp_key();

        table.track_packet(key, 64, TCP_SYN);
        assert_eq!(table.entry_count(), 1);

        // Advance past timeout
        table.set_tick(TCP_NEW_TIMEOUT + 1);
        let removed = table.gc();
        assert_eq!(removed, 1);
        assert_eq!(table.entry_count(), 0);
    }

    #[test]
    fn test_conntrack_classify_unknown() {
        let table = ConntrackTable::new();
        let state = table.classify_packet(&tcp_key());
        assert_eq!(state, ConntrackState::New);
    }

    #[test]
    fn test_tcp_state_full_handshake() {
        let mut entry = ConntrackEntry::new(tcp_key(), ConntrackKey::PROTO_TCP);

        // Client SYN
        assert_eq!(entry.tcp_state, TcpConnState::SynSent);

        // Server SYN-ACK
        update_tcp_state(&mut entry, TCP_SYN | TCP_ACK, true);
        assert_eq!(entry.tcp_state, TcpConnState::SynRecv);

        // Client ACK
        update_tcp_state(&mut entry, TCP_ACK, false);
        assert_eq!(entry.tcp_state, TcpConnState::Established);
        assert_eq!(entry.state, ConntrackState::Established);
    }

    #[test]
    fn test_tcp_state_rst() {
        let mut entry = ConntrackEntry::new(tcp_key(), ConntrackKey::PROTO_TCP);
        update_tcp_state(&mut entry, TCP_RST, false);
        assert_eq!(entry.tcp_state, TcpConnState::Closed);
        assert_eq!(entry.state, ConntrackState::Invalid);
    }

    #[test]
    fn test_tcp_state_fin_close() {
        let mut entry = ConntrackEntry::new(tcp_key(), ConntrackKey::PROTO_TCP);

        // Establish first
        update_tcp_state(&mut entry, TCP_SYN | TCP_ACK, true);
        update_tcp_state(&mut entry, TCP_ACK, false);
        assert_eq!(entry.tcp_state, TcpConnState::Established);

        // Client FIN
        update_tcp_state(&mut entry, TCP_FIN, false);
        assert_eq!(entry.tcp_state, TcpConnState::FinWait1);

        // Server FIN+ACK
        update_tcp_state(&mut entry, TCP_FIN | TCP_ACK, true);
        assert_eq!(entry.tcp_state, TcpConnState::TimeWait);
        assert_eq!(entry.state, ConntrackState::TimeWait);
    }

    #[test]
    fn test_conntrack_stats() {
        let mut table = ConntrackTable::new();
        table.track_packet(tcp_key(), 64, TCP_SYN);
        let stats = table.stats();
        assert_eq!(stats.active_entries, 1);
        assert_eq!(stats.total_created, 1);
        assert_eq!(stats.max_entries, MAX_CONNTRACK_ENTRIES as u64);
    }
}
