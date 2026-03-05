//! mDNS/DNS-SD: Multicast DNS and Service Discovery for VeridianOS
//!
//! Implements RFC 6762 (Multicast DNS) and RFC 6763 (DNS-Based Service
//! Discovery) for zero-configuration networking on the `.local` domain.
//!
//! Features:
//! - mDNS query/response on 224.0.0.251:5353 (IPv4) / [ff02::fb]:5353 (IPv6)
//! - One-shot and continuous query modes
//! - Known-answer suppression
//! - Conflict resolution: probe (3x, 250ms) then announce (2x, 1s)
//! - DNS-SD service registration, browsing, and deregistration
//! - PTR/SRV/TXT record handling for service instances
//! - TTL-based cache with expiry
//! - Goodbye packets (TTL=0) on shutdown

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ============================================================================
// Constants
// ============================================================================

/// mDNS multicast IPv4 address: 224.0.0.251
pub const MDNS_IPV4_ADDR: [u8; 4] = [224, 0, 0, 251];

/// mDNS multicast IPv6 address: ff02::fb
pub const MDNS_IPV6_ADDR: [u8; 16] = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfb];

/// mDNS port
pub const MDNS_PORT: u16 = 5353;

/// Default TTL for unique records (host addresses, SRV)
pub const TTL_UNIQUE: u32 = 120;

/// Default TTL for shared records (PTR, service types)
pub const TTL_SHARED: u32 = 4500;

/// Probe interval in milliseconds
pub const PROBE_INTERVAL_MS: u64 = 250;

/// Number of probe attempts before claiming a name
pub const PROBE_COUNT: u8 = 3;

/// Announce interval in milliseconds
pub const ANNOUNCE_INTERVAL_MS: u64 = 1000;

/// Number of announcements after claiming a name
pub const ANNOUNCE_COUNT: u8 = 2;

/// Maximum mDNS message size (same as DNS over UDP)
const MAX_MDNS_MSG_SIZE: usize = 512;

/// Maximum label length (per DNS spec)
const MAX_LABEL_LEN: usize = 63;

/// Maximum domain name length
const MAX_NAME_LEN: usize = 255;

/// Maximum cached entries
const MAX_CACHE_ENTRIES: usize = 512;

/// Maximum registered services
const MAX_SERVICES: usize = 64;

/// Maximum TXT record pairs
const MAX_TXT_PAIRS: usize = 16;

/// QU (unicast response) bit in class field
const QU_BIT: u16 = 0x8000;

/// Cache-flush bit in class field (for responses)
const CACHE_FLUSH_BIT: u16 = 0x8000;

/// `.local` suffix
pub const LOCAL_SUFFIX: &str = ".local";

// ============================================================================
// DNS Record Types (local definitions to avoid coupling)
// ============================================================================

/// DNS record type codes used by mDNS/DNS-SD
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum MdnsRecordType {
    /// IPv4 address
    A = 1,
    /// Domain name pointer (reverse lookup / service enumeration)
    PTR = 12,
    /// Text record (key=value metadata)
    TXT = 16,
    /// IPv6 address
    AAAA = 28,
    /// Service locator (priority, weight, port, target)
    SRV = 33,
    /// Any type (query wildcard)
    ANY = 255,
    /// Unknown / unsupported
    Unknown = 0,
}

impl MdnsRecordType {
    pub fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::A,
            12 => Self::PTR,
            16 => Self::TXT,
            28 => Self::AAAA,
            33 => Self::SRV,
            255 => Self::ANY,
            _ => Self::Unknown,
        }
    }

    pub fn to_u16(self) -> u16 {
        self as u16
    }
}

/// DNS class (IN = Internet)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MdnsClass {
    /// Internet class
    IN = 1,
    /// Any class (query wildcard)
    ANY = 255,
}

impl MdnsClass {
    pub fn from_u16(val: u16) -> Self {
        match val & !QU_BIT {
            1 => Self::IN,
            255 => Self::ANY,
            _ => Self::IN,
        }
    }
}

// ============================================================================
// Error Type
// ============================================================================

/// Errors from mDNS operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdnsError {
    /// Message too short to parse
    MessageTooShort,
    /// Invalid label in domain name
    InvalidLabel,
    /// Name exceeds maximum length
    NameTooLong,
    /// Buffer too small for serialization
    BufferTooSmall,
    /// Service limit reached
    TooManyServices,
    /// Name conflict detected during probing
    NameConflict,
    /// Cache is full
    CacheFull,
    /// Invalid service type format
    InvalidServiceType,
    /// TXT record too large
    TxtTooLarge,
    /// Record not found
    NotFound,
    /// Invalid message format
    InvalidFormat,
}

// ============================================================================
// Record data types
// ============================================================================

/// SRV record data (RFC 2782)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct SrvRecord {
    /// Priority (lower = preferred)
    pub priority: u16,
    /// Weight for load balancing among same-priority targets
    pub weight: u16,
    /// Port number
    pub port: u16,
    /// Target hostname
    pub target: String,
}

/// TXT record: collection of key=value pairs
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct TxtRecord {
    /// Key-value pairs
    pub entries: Vec<TxtEntry>,
}

/// Single TXT record entry
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct TxtEntry {
    /// Key (before '=')
    pub key: String,
    /// Value (after '='), empty if boolean key
    pub value: String,
}

#[cfg(feature = "alloc")]
impl Default for TxtRecord {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl TxtRecord {
    /// Create an empty TXT record
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a key=value pair
    pub fn add(&mut self, key: &str, value: &str) -> Result<(), MdnsError> {
        if self.entries.len() >= MAX_TXT_PAIRS {
            return Err(MdnsError::TxtTooLarge);
        }
        // Each entry is length-prefixed; key=value must fit in 255 bytes
        if key.len() + 1 + value.len() > 255 {
            return Err(MdnsError::TxtTooLarge);
        }
        self.entries.push(TxtEntry {
            key: String::from(key),
            value: String::from(value),
        });
        Ok(())
    }

    /// Encode TXT record to wire format (length-prefixed strings)
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        for entry in &self.entries {
            let s = if entry.value.is_empty() {
                entry.key.clone()
            } else {
                let mut s = entry.key.clone();
                s.push('=');
                s.push_str(&entry.value);
                s
            };
            let len = s.len();
            if len <= 255 {
                buf.push(len as u8);
                buf.extend_from_slice(s.as_bytes());
            }
        }
        // RFC 6763: empty TXT record must contain single zero byte
        if buf.is_empty() {
            buf.push(0);
        }
        buf
    }

    /// Decode TXT record from wire format
    pub fn decode(data: &[u8]) -> Result<Self, MdnsError> {
        let mut entries = Vec::new();
        let mut pos = 0;
        while pos < data.len() {
            let len = data[pos] as usize;
            pos += 1;
            if len == 0 {
                continue;
            }
            if pos + len > data.len() {
                return Err(MdnsError::InvalidFormat);
            }
            let s = core::str::from_utf8(&data[pos..pos + len])
                .map_err(|_| MdnsError::InvalidFormat)?;
            pos += len;
            if let Some(eq_pos) = s.find('=') {
                entries.push(TxtEntry {
                    key: String::from(&s[..eq_pos]),
                    value: String::from(&s[eq_pos + 1..]),
                });
            } else {
                entries.push(TxtEntry {
                    key: String::from(s),
                    value: String::new(),
                });
            }
        }
        Ok(Self { entries })
    }

    /// Look up a value by key
    pub fn get(&self, key: &str) -> Option<&str> {
        for entry in &self.entries {
            if entry.key == key {
                return Some(&entry.value);
            }
        }
        None
    }
}

// ============================================================================
// mDNS Resource Record
// ============================================================================

/// An mDNS resource record
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct MdnsRecord {
    /// Fully-qualified domain name
    pub name: String,
    /// Record type
    pub rtype: MdnsRecordType,
    /// Cache-flush (unique record)
    pub cache_flush: bool,
    /// Time-to-live in seconds
    pub ttl: u32,
    /// Record data (wire format)
    pub rdata: Vec<u8>,
}

/// An mDNS question
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct MdnsQuestion {
    /// Queried name
    pub name: String,
    /// Query type
    pub qtype: MdnsRecordType,
    /// Unicast response requested (QU bit)
    pub unicast: bool,
}

// ============================================================================
// Service Type Parsing
// ============================================================================

/// A parsed DNS-SD service type
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ServiceType {
    /// Service name (e.g., "http")
    pub service: String,
    /// Protocol ("tcp" or "udp")
    pub protocol: String,
    /// Domain (e.g., "local")
    pub domain: String,
}

#[cfg(feature = "alloc")]
impl ServiceType {
    /// Parse a service type string like "_http._tcp.local"
    pub fn parse(s: &str) -> Result<Self, MdnsError> {
        let s = s.trim_end_matches('.');
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() < 3 {
            return Err(MdnsError::InvalidServiceType);
        }
        let service_part = parts[0];
        let proto_part = parts[1];
        let mut domain = String::new();
        for (i, part) in parts[2..].iter().enumerate() {
            if i > 0 {
                domain.push('.');
            }
            domain.push_str(part);
        }

        // Service must start with '_'
        if !service_part.starts_with('_') || service_part.len() < 2 {
            return Err(MdnsError::InvalidServiceType);
        }
        // Protocol must be _tcp or _udp
        if proto_part != "_tcp" && proto_part != "_udp" {
            return Err(MdnsError::InvalidServiceType);
        }

        Ok(Self {
            service: String::from(&service_part[1..]),
            protocol: String::from(&proto_part[1..]),
            domain,
        })
    }

    /// Format as DNS-SD service type string
    pub fn to_service_string(&self) -> String {
        let mut s = String::from("_");
        s.push_str(&self.service);
        s.push_str("._");
        s.push_str(&self.protocol);
        s.push('.');
        s.push_str(&self.domain);
        s
    }
}

// ============================================================================
// DNS-SD Service Instance
// ============================================================================

/// A registered DNS-SD service instance
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ServiceInstance {
    /// Human-readable instance name (e.g., "My Web Server")
    pub instance_name: String,
    /// Service type (e.g., "_http._tcp")
    pub service_type: ServiceType,
    /// Port number
    pub port: u16,
    /// Target hostname (defaults to local hostname)
    pub target: String,
    /// Priority for SRV record
    pub priority: u16,
    /// Weight for SRV record
    pub weight: u16,
    /// TXT record metadata
    pub txt: TxtRecord,
}

#[cfg(feature = "alloc")]
impl ServiceInstance {
    /// Construct the full service instance name
    /// e.g., "My Web Server._http._tcp.local"
    pub fn full_name(&self) -> String {
        let mut name = self.instance_name.clone();
        name.push('.');
        name.push_str(&self.service_type.to_service_string());
        name
    }

    /// Build the PTR record name for this service type
    pub fn ptr_name(&self) -> String {
        self.service_type.to_service_string()
    }

    /// Build an SRV record for this instance
    pub fn to_srv(&self) -> SrvRecord {
        SrvRecord {
            priority: self.priority,
            weight: self.weight,
            port: self.port,
            target: self.target.clone(),
        }
    }

    /// Encode SRV rdata to wire format: priority(2) + weight(2) + port(2) +
    /// target(variable)
    pub fn encode_srv_rdata(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.priority.to_be_bytes());
        buf.extend_from_slice(&self.weight.to_be_bytes());
        buf.extend_from_slice(&self.port.to_be_bytes());
        // Target as DNS name labels
        buf.extend_from_slice(&encode_dns_name(&self.target));
        buf
    }
}

// ============================================================================
// Cache Entry
// ============================================================================

/// A cached mDNS record with expiry tracking
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct CacheEntry {
    /// The resource record
    pub record: MdnsRecord,
    /// Tick count when this entry was inserted
    pub inserted_tick: u64,
    /// TTL in seconds at insertion time
    pub original_ttl: u32,
}

#[cfg(feature = "alloc")]
impl CacheEntry {
    /// Check if this entry has expired given the current tick and
    /// ticks-per-second
    pub fn is_expired(&self, current_tick: u64, ticks_per_sec: u64) -> bool {
        if ticks_per_sec == 0 {
            return false;
        }
        let elapsed_ticks = current_tick.saturating_sub(self.inserted_tick);
        let elapsed_secs = elapsed_ticks / ticks_per_sec;
        elapsed_secs >= self.original_ttl as u64
    }

    /// Remaining TTL in seconds
    pub fn remaining_ttl(&self, current_tick: u64, ticks_per_sec: u64) -> u32 {
        if ticks_per_sec == 0 {
            return self.original_ttl;
        }
        let elapsed_ticks = current_tick.saturating_sub(self.inserted_tick);
        let elapsed_secs = elapsed_ticks / ticks_per_sec;
        self.original_ttl.saturating_sub(elapsed_secs as u32)
    }
}

// ============================================================================
// Probe State Machine
// ============================================================================

/// State of a name-claiming probe sequence
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeState {
    /// Not yet started
    Idle,
    /// Sending probe queries (count, next_send_tick)
    Probing { sent: u8, next_tick: u64 },
    /// Sending announcements
    Announcing { sent: u8, next_tick: u64 },
    /// Name successfully claimed
    Claimed,
    /// Conflict detected, must choose new name
    Conflict,
}

impl ProbeState {
    /// Whether the name is fully claimed and usable
    pub fn is_claimed(&self) -> bool {
        matches!(self, ProbeState::Claimed)
    }

    /// Whether a conflict was detected
    pub fn is_conflict(&self) -> bool {
        matches!(self, ProbeState::Conflict)
    }
}

// ============================================================================
// mDNS Responder
// ============================================================================

/// Core mDNS responder managing registered services and cached records
#[cfg(feature = "alloc")]
pub struct MdnsResponder {
    /// Our hostname (without .local suffix)
    hostname: String,
    /// Registered service instances
    services: Vec<ServiceInstance>,
    /// Record cache (name -> list of records)
    cache: BTreeMap<String, Vec<CacheEntry>>,
    /// Probe state per name being claimed
    probe_states: BTreeMap<String, ProbeState>,
    /// Monotonic tick counter reference (ticks per second)
    ticks_per_sec: u64,
    /// Host IPv4 address (4 bytes)
    host_ipv4: [u8; 4],
    /// Host IPv6 address (16 bytes)
    host_ipv6: [u8; 16],
}

#[cfg(feature = "alloc")]
impl MdnsResponder {
    /// Create a new mDNS responder
    pub fn new(hostname: &str, ticks_per_sec: u64) -> Self {
        Self {
            hostname: String::from(hostname),
            services: Vec::new(),
            cache: BTreeMap::new(),
            probe_states: BTreeMap::new(),
            ticks_per_sec,
            host_ipv4: [0; 4],
            host_ipv6: [0; 16],
        }
    }

    /// Set the host IPv4 address
    pub fn set_ipv4(&mut self, addr: [u8; 4]) {
        self.host_ipv4 = addr;
    }

    /// Set the host IPv6 address
    pub fn set_ipv6(&mut self, addr: [u8; 16]) {
        self.host_ipv6 = addr;
    }

    /// Get our fully-qualified hostname (hostname.local)
    pub fn fqdn(&self) -> String {
        let mut name = self.hostname.clone();
        name.push_str(LOCAL_SUFFIX);
        name
    }

    // ---- Service Registration ----

    /// Register a new service instance
    pub fn register_service(&mut self, svc: ServiceInstance) -> Result<(), MdnsError> {
        if self.services.len() >= MAX_SERVICES {
            return Err(MdnsError::TooManyServices);
        }
        // Start probing for the service name
        let full_name = svc.full_name();
        self.probe_states.insert(full_name, ProbeState::Idle);
        self.services.push(svc);
        Ok(())
    }

    /// Deregister a service by instance name, returning goodbye records
    pub fn deregister_service(&mut self, instance_name: &str) -> Vec<MdnsRecord> {
        let mut goodbyes = Vec::new();
        let mut names_to_remove = Vec::new();

        self.services.retain(|svc| {
            if svc.instance_name == instance_name {
                // Generate goodbye packets (TTL=0) for PTR, SRV, TXT
                goodbyes.push(MdnsRecord {
                    name: svc.ptr_name(),
                    rtype: MdnsRecordType::PTR,
                    cache_flush: false,
                    ttl: 0,
                    rdata: encode_dns_name(&svc.full_name()),
                });
                goodbyes.push(MdnsRecord {
                    name: svc.full_name(),
                    rtype: MdnsRecordType::SRV,
                    cache_flush: true,
                    ttl: 0,
                    rdata: svc.encode_srv_rdata(),
                });
                goodbyes.push(MdnsRecord {
                    name: svc.full_name(),
                    rtype: MdnsRecordType::TXT,
                    cache_flush: true,
                    ttl: 0,
                    rdata: svc.txt.encode(),
                });
                names_to_remove.push(svc.full_name());
                false // remove from list
            } else {
                true // keep
            }
        });

        for name in &names_to_remove {
            self.probe_states.remove(name);
        }

        goodbyes
    }

    /// Browse for services of a given type in our cache
    pub fn browse_services(&self, service_type: &str) -> Vec<&ServiceInstance> {
        // Check our own registered services
        let mut results = Vec::new();
        for svc in &self.services {
            if svc.service_type.to_service_string() == service_type {
                results.push(svc);
            }
        }
        results
    }

    /// Look up a service instance by full name
    pub fn lookup_service(&self, full_name: &str) -> Option<&ServiceInstance> {
        self.services
            .iter()
            .find(|svc| svc.full_name() == full_name)
    }

    // ---- Probing & Announcing ----

    /// Start probing for a name at the given tick
    pub fn start_probe(&mut self, name: &str, current_tick: u64) {
        let interval_ticks = (PROBE_INTERVAL_MS * self.ticks_per_sec) / 1000;
        self.probe_states.insert(
            String::from(name),
            ProbeState::Probing {
                sent: 1, // First probe is sent immediately on start
                next_tick: current_tick + interval_ticks,
            },
        );
    }

    /// Advance probe/announce state machine; returns names that need packets
    /// sent
    pub fn tick_probes(&mut self, current_tick: u64) -> Vec<(String, ProbeState)> {
        let mut actions = Vec::new();
        let probe_interval = (PROBE_INTERVAL_MS * self.ticks_per_sec) / 1000;
        let announce_interval = (ANNOUNCE_INTERVAL_MS * self.ticks_per_sec) / 1000;

        for (name, state) in self.probe_states.iter_mut() {
            match *state {
                ProbeState::Probing { sent, next_tick } if current_tick >= next_tick => {
                    if sent + 1 >= PROBE_COUNT {
                        // Probing complete, start announcing
                        *state = ProbeState::Announcing {
                            sent: 0,
                            next_tick: current_tick + announce_interval,
                        };
                    } else {
                        *state = ProbeState::Probing {
                            sent: sent + 1,
                            next_tick: current_tick + probe_interval,
                        };
                    }
                    actions.push((name.clone(), *state));
                }
                ProbeState::Announcing { sent, next_tick } if current_tick >= next_tick => {
                    if sent + 1 >= ANNOUNCE_COUNT {
                        *state = ProbeState::Claimed;
                    } else {
                        *state = ProbeState::Announcing {
                            sent: sent + 1,
                            next_tick: current_tick + announce_interval,
                        };
                    }
                    actions.push((name.clone(), *state));
                }
                _ => {}
            }
        }
        actions
    }

    /// Mark a name as conflicted
    pub fn mark_conflict(&mut self, name: &str) {
        if let Some(state) = self.probe_states.get_mut(name) {
            *state = ProbeState::Conflict;
        }
    }

    // ---- Query Handling ----

    /// Check if a question matches any of our registered records
    pub fn has_answer(&self, question: &MdnsQuestion) -> bool {
        let qname = &question.name;
        let fqdn = self.fqdn();

        match question.qtype {
            MdnsRecordType::A | MdnsRecordType::ANY => {
                if qname == &fqdn {
                    return true;
                }
            }
            MdnsRecordType::AAAA => {
                if qname == &fqdn {
                    return true;
                }
            }
            MdnsRecordType::PTR => {
                for svc in &self.services {
                    if qname == &svc.ptr_name() {
                        return true;
                    }
                }
            }
            MdnsRecordType::SRV | MdnsRecordType::TXT => {
                for svc in &self.services {
                    if qname == &svc.full_name() {
                        return true;
                    }
                }
            }
            _ => {}
        }
        false
    }

    /// Generate answer records for a given question
    pub fn answer(&self, question: &MdnsQuestion) -> Vec<MdnsRecord> {
        let mut answers = Vec::new();
        let qname = &question.name;
        let fqdn = self.fqdn();

        // A record for our hostname
        if (question.qtype == MdnsRecordType::A || question.qtype == MdnsRecordType::ANY)
            && qname == &fqdn
        {
            answers.push(MdnsRecord {
                name: fqdn.clone(),
                rtype: MdnsRecordType::A,
                cache_flush: true,
                ttl: TTL_UNIQUE,
                rdata: self.host_ipv4.to_vec(),
            });
        }

        // AAAA record for our hostname
        if (question.qtype == MdnsRecordType::AAAA || question.qtype == MdnsRecordType::ANY)
            && qname == &fqdn
        {
            answers.push(MdnsRecord {
                name: fqdn.clone(),
                rtype: MdnsRecordType::AAAA,
                cache_flush: true,
                ttl: TTL_UNIQUE,
                rdata: self.host_ipv6.to_vec(),
            });
        }

        // PTR records for service enumeration
        if question.qtype == MdnsRecordType::PTR || question.qtype == MdnsRecordType::ANY {
            for svc in &self.services {
                if qname == &svc.ptr_name() {
                    answers.push(MdnsRecord {
                        name: svc.ptr_name(),
                        rtype: MdnsRecordType::PTR,
                        cache_flush: false,
                        ttl: TTL_SHARED,
                        rdata: encode_dns_name(&svc.full_name()),
                    });
                }
            }
        }

        // SRV records for service instances
        if question.qtype == MdnsRecordType::SRV || question.qtype == MdnsRecordType::ANY {
            for svc in &self.services {
                if qname == &svc.full_name() {
                    answers.push(MdnsRecord {
                        name: svc.full_name(),
                        rtype: MdnsRecordType::SRV,
                        cache_flush: true,
                        ttl: TTL_UNIQUE,
                        rdata: svc.encode_srv_rdata(),
                    });
                }
            }
        }

        // TXT records for service instances
        if question.qtype == MdnsRecordType::TXT || question.qtype == MdnsRecordType::ANY {
            for svc in &self.services {
                if qname == &svc.full_name() {
                    answers.push(MdnsRecord {
                        name: svc.full_name(),
                        rtype: MdnsRecordType::TXT,
                        cache_flush: true,
                        ttl: TTL_UNIQUE,
                        rdata: svc.txt.encode(),
                    });
                }
            }
        }

        answers
    }

    // ---- Known-Answer Suppression ----

    /// Filter out answers that the querier already knows (known-answer
    /// suppression) Per RFC 6762 section 7.1: if the querier includes a
    /// known answer with TTL >= 50% of our TTL, we suppress that answer.
    pub fn suppress_known_answers(
        &self,
        answers: &mut Vec<MdnsRecord>,
        known_answers: &[MdnsRecord],
    ) {
        answers.retain(|answer| {
            !known_answers.iter().any(|ka| {
                ka.name == answer.name
                    && ka.rtype == answer.rtype
                    && ka.rdata == answer.rdata
                    && ka.ttl >= answer.ttl / 2
            })
        });
    }

    // ---- Conflict Detection ----

    /// Check if an incoming record conflicts with any of our registrations
    pub fn detect_conflict(&self, record: &MdnsRecord) -> bool {
        let fqdn = self.fqdn();

        // Check hostname conflict
        if record.name == fqdn
            && (record.rtype == MdnsRecordType::A || record.rtype == MdnsRecordType::AAAA)
            && record.rdata != self.host_ipv4.to_vec()
            && record.rdata != self.host_ipv6.to_vec()
        {
            return true;
        }

        // Check service name conflicts
        for svc in &self.services {
            if record.name == svc.full_name()
                && record.rtype == MdnsRecordType::SRV
                && record.rdata != svc.encode_srv_rdata()
            {
                return true;
            }
        }

        false
    }

    // ---- Cache Management ----

    /// Insert a record into the cache
    pub fn cache_insert(&mut self, record: MdnsRecord, current_tick: u64) -> Result<(), MdnsError> {
        // Goodbye packet (TTL=0): remove from cache
        if record.ttl == 0 {
            self.cache_remove(&record.name, record.rtype);
            return Ok(());
        }

        let ttl = record.ttl;
        let entry = CacheEntry {
            original_ttl: ttl,
            inserted_tick: current_tick,
            record,
        };

        // Check total cache size before inserting
        let total: usize = self.cache.values().map(|v| v.len()).sum();

        let entries = self.cache.entry(entry.record.name.clone()).or_default();

        // If cache-flush bit is set, remove all records of the same type
        if entry.record.cache_flush {
            entries.retain(|e| e.record.rtype != entry.record.rtype);
        }

        // Replace existing identical record or add new
        if let Some(existing) = entries
            .iter_mut()
            .find(|e| e.record.rtype == entry.record.rtype && e.record.rdata == entry.record.rdata)
        {
            *existing = entry;
        } else {
            if total >= MAX_CACHE_ENTRIES {
                return Err(MdnsError::CacheFull);
            }
            entries.push(entry);
        }

        Ok(())
    }

    /// Remove records from cache by name and type
    pub fn cache_remove(&mut self, name: &str, rtype: MdnsRecordType) {
        if let Some(entries) = self.cache.get_mut(name) {
            entries.retain(|e| e.record.rtype != rtype);
            if entries.is_empty() {
                self.cache.remove(name);
            }
        }
    }

    /// Look up cached records by name and type
    pub fn cache_lookup(
        &self,
        name: &str,
        rtype: MdnsRecordType,
        current_tick: u64,
    ) -> Vec<&CacheEntry> {
        match self.cache.get(name) {
            Some(entries) => entries
                .iter()
                .filter(|e| {
                    (rtype == MdnsRecordType::ANY || e.record.rtype == rtype)
                        && !e.is_expired(current_tick, self.ticks_per_sec)
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Evict expired entries from the cache
    pub fn cache_evict_expired(&mut self, current_tick: u64) {
        let tps = self.ticks_per_sec;
        self.cache.retain(|_name, entries| {
            entries.retain(|e| !e.is_expired(current_tick, tps));
            !entries.is_empty()
        });
    }

    /// Get the number of cached entries
    pub fn cache_size(&self) -> usize {
        self.cache.values().map(|v| v.len()).sum()
    }
}

// ============================================================================
// Name Resolution Helpers
// ============================================================================

/// Check if a name is in the `.local` domain
pub fn is_local_name(name: &str) -> bool {
    // Check suffix ".local" or ".local."
    let name_trimmed = name.strip_suffix('.').unwrap_or(name);
    if name_trimmed.len() < 5 {
        return false;
    }
    // Case-insensitive check for ".local" suffix or bare "local"
    let bytes = name_trimmed.as_bytes();
    let len = bytes.len();
    if len >= 6 {
        let suffix = &bytes[len - 6..];
        suffix.eq_ignore_ascii_case(b".local")
    } else {
        // Exactly "local" (len == 5)
        bytes.eq_ignore_ascii_case(b"local")
    }
}

/// Check if a name is a reverse lookup address (.arpa)
pub fn is_reverse_lookup(name: &str) -> bool {
    let name_trimmed = name.trim_end_matches('.');
    name_trimmed.ends_with(".in-addr.arpa") || name_trimmed.ends_with(".ip6.arpa")
}

/// Build a reverse lookup name for an IPv4 address
#[cfg(feature = "alloc")]
pub fn ipv4_reverse_name(addr: [u8; 4]) -> String {
    let mut name = String::new();
    // Reverse octets
    for i in (0..4).rev() {
        if !name.is_empty() {
            name.push('.');
        }
        // Manual integer formatting to avoid format! in no_std hot paths
        let mut val = addr[i];
        if val >= 100 {
            name.push((b'0' + val / 100) as char);
            val %= 100;
            name.push((b'0' + val / 10) as char);
            name.push((b'0' + val % 10) as char);
        } else if val >= 10 {
            name.push((b'0' + val / 10) as char);
            name.push((b'0' + val % 10) as char);
        } else {
            name.push((b'0' + val) as char);
        }
    }
    name.push_str(".in-addr.arpa");
    name
}

// ============================================================================
// DNS Name Encoding/Decoding
// ============================================================================

/// Encode a domain name to DNS wire format (label-length prefixed)
#[cfg(feature = "alloc")]
pub fn encode_dns_name(name: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    let name = name.trim_end_matches('.');
    if name.is_empty() {
        buf.push(0);
        return buf;
    }
    for label in name.split('.') {
        let len = label.len();
        if len > MAX_LABEL_LEN {
            // Truncate oversized labels
            buf.push(MAX_LABEL_LEN as u8);
            buf.extend_from_slice(&label.as_bytes()[..MAX_LABEL_LEN]);
        } else {
            buf.push(len as u8);
            buf.extend_from_slice(label.as_bytes());
        }
    }
    buf.push(0); // Root label
    buf
}

/// Decode a DNS wire-format name from a buffer at the given offset.
/// Returns the decoded name and the number of bytes consumed from `offset`.
#[cfg(feature = "alloc")]
pub fn decode_dns_name(buf: &[u8], offset: usize) -> Result<(String, usize), MdnsError> {
    let mut name = String::new();
    let mut pos = offset;
    let mut consumed = 0;
    let mut followed_pointer = false;
    let mut jumps = 0;

    loop {
        if pos >= buf.len() {
            return Err(MdnsError::MessageTooShort);
        }
        let len = buf[pos] as usize;

        if len == 0 {
            if !followed_pointer {
                consumed = pos - offset + 1;
            }
            break;
        }

        // Compression pointer
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= buf.len() {
                return Err(MdnsError::MessageTooShort);
            }
            if !followed_pointer {
                consumed = pos - offset + 2;
                followed_pointer = true;
            }
            let ptr = ((len & 0x3F) << 8) | (buf[pos + 1] as usize);
            pos = ptr;
            jumps += 1;
            if jumps > MAX_NAME_LEN {
                return Err(MdnsError::InvalidLabel);
            }
            continue;
        }

        if len > MAX_LABEL_LEN {
            return Err(MdnsError::InvalidLabel);
        }
        pos += 1;
        if pos + len > buf.len() {
            return Err(MdnsError::MessageTooShort);
        }

        if !name.is_empty() {
            name.push('.');
        }
        let label =
            core::str::from_utf8(&buf[pos..pos + len]).map_err(|_| MdnsError::InvalidLabel)?;
        name.push_str(label);
        pos += len;

        if name.len() > MAX_NAME_LEN {
            return Err(MdnsError::NameTooLong);
        }
    }

    Ok((name, consumed))
}

/// Build an mDNS probe query message for the given name (ANY type, QU bit set)
#[cfg(feature = "alloc")]
pub fn build_probe_query(name: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(MAX_MDNS_MSG_SIZE);

    // Header: ID=0, QR=0 (query), QDCOUNT=1
    buf.extend_from_slice(&[0u8; 2]); // ID = 0 for mDNS
    buf.extend_from_slice(&[0x00, 0x00]); // Flags: standard query
    buf.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT = 1
    buf.extend_from_slice(&[0u8; 2]); // ANCOUNT = 0
    buf.extend_from_slice(&[0u8; 2]); // NSCOUNT = 0
    buf.extend_from_slice(&[0u8; 2]); // ARCOUNT = 0

    // Question: name, type=ANY, class=IN|QU
    buf.extend_from_slice(&encode_dns_name(name));
    buf.extend_from_slice(&MdnsRecordType::ANY.to_u16().to_be_bytes());
    let class_qu = MdnsClass::IN as u16 | QU_BIT;
    buf.extend_from_slice(&class_qu.to_be_bytes());

    buf
}

// ============================================================================
// Trait: eq_ignore_ascii_case for byte slices
// ============================================================================

trait EqIgnoreAsciiCase {
    fn eq_ignore_ascii_case(&self, other: &[u8]) -> bool;
}

impl EqIgnoreAsciiCase for [u8] {
    fn eq_ignore_ascii_case(&self, other: &[u8]) -> bool {
        if self.len() != other.len() {
            return false;
        }
        for i in 0..self.len() {
            if !self[i].eq_ignore_ascii_case(&other[i]) {
                return false;
            }
        }
        true
    }
}

impl EqIgnoreAsciiCase for str {
    fn eq_ignore_ascii_case(&self, other: &[u8]) -> bool {
        self.as_bytes().eq_ignore_ascii_case(other)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // ---- Service type parsing ----

    #[test]
    fn test_parse_http_tcp_local() {
        let st = ServiceType::parse("_http._tcp.local").unwrap();
        assert_eq!(st.service, "http");
        assert_eq!(st.protocol, "tcp");
        assert_eq!(st.domain, "local");
    }

    #[test]
    fn test_parse_ssh_tcp_local() {
        let st = ServiceType::parse("_ssh._tcp.local").unwrap();
        assert_eq!(st.service, "ssh");
        assert_eq!(st.protocol, "tcp");
        assert_eq!(st.domain, "local");
    }

    #[test]
    fn test_parse_udp_service() {
        let st = ServiceType::parse("_tftp._udp.local").unwrap();
        assert_eq!(st.protocol, "udp");
    }

    #[test]
    fn test_parse_invalid_no_underscore() {
        assert_eq!(
            ServiceType::parse("http._tcp.local"),
            Err(MdnsError::InvalidServiceType)
        );
    }

    #[test]
    fn test_parse_invalid_protocol() {
        assert_eq!(
            ServiceType::parse("_http._sctp.local"),
            Err(MdnsError::InvalidServiceType)
        );
    }

    #[test]
    fn test_service_type_roundtrip() {
        let st = ServiceType::parse("_http._tcp.local").unwrap();
        assert_eq!(st.to_service_string(), "_http._tcp.local");
    }

    // ---- TXT record encoding/decoding ----

    #[test]
    fn test_txt_encode_decode() {
        let mut txt = TxtRecord::new();
        txt.add("path", "/index.html").unwrap();
        txt.add("version", "1").unwrap();

        let encoded = txt.encode();
        let decoded = TxtRecord::decode(&encoded).unwrap();

        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.get("path"), Some("/index.html"));
        assert_eq!(decoded.get("version"), Some("1"));
    }

    #[test]
    fn test_txt_empty() {
        let txt = TxtRecord::new();
        let encoded = txt.encode();
        assert_eq!(encoded, vec![0u8]); // Single zero byte per RFC 6763
    }

    #[test]
    fn test_txt_boolean_key() {
        let mut txt = TxtRecord::new();
        txt.add("paper", "").unwrap();
        let encoded = txt.encode();
        let decoded = TxtRecord::decode(&encoded).unwrap();
        assert_eq!(decoded.entries[0].key, "paper");
        assert_eq!(decoded.entries[0].value, "");
    }

    // ---- SRV record ----

    #[test]
    fn test_srv_record_construction() {
        let svc = ServiceInstance {
            instance_name: String::from("My Web Server"),
            service_type: ServiceType::parse("_http._tcp.local").unwrap(),
            port: 8080,
            target: String::from("myhost.local"),
            priority: 0,
            weight: 0,
            txt: TxtRecord::new(),
        };
        let srv = svc.to_srv();
        assert_eq!(srv.port, 8080);
        assert_eq!(srv.priority, 0);
        assert_eq!(srv.target, "myhost.local");
    }

    #[test]
    fn test_srv_rdata_encoding() {
        let svc = ServiceInstance {
            instance_name: String::from("Test"),
            service_type: ServiceType::parse("_http._tcp.local").unwrap(),
            port: 80,
            target: String::from("host.local"),
            priority: 10,
            weight: 20,
            txt: TxtRecord::new(),
        };
        let rdata = svc.encode_srv_rdata();
        // priority=10 (2 bytes) + weight=20 (2 bytes) + port=80 (2 bytes) + name
        assert_eq!(rdata[0], 0);
        assert_eq!(rdata[1], 10); // priority
        assert_eq!(rdata[2], 0);
        assert_eq!(rdata[3], 20); // weight
        assert_eq!(rdata[4], 0);
        assert_eq!(rdata[5], 80); // port
                                  // Followed by encoded "host.local"
        assert_eq!(rdata[6], 4); // "host" label length
    }

    // ---- PTR record for service enumeration ----

    #[test]
    fn test_ptr_name_for_service() {
        let svc = ServiceInstance {
            instance_name: String::from("My Printer"),
            service_type: ServiceType::parse("_ipp._tcp.local").unwrap(),
            port: 631,
            target: String::from("printer.local"),
            priority: 0,
            weight: 0,
            txt: TxtRecord::new(),
        };
        assert_eq!(svc.ptr_name(), "_ipp._tcp.local");
        assert_eq!(svc.full_name(), "My Printer._ipp._tcp.local");
    }

    // ---- Conflict detection ----

    #[test]
    fn test_conflict_same_name_different_data() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        resp.set_ipv4([192, 168, 1, 10]);

        let conflicting = MdnsRecord {
            name: String::from("myhost.local"),
            rtype: MdnsRecordType::A,
            cache_flush: true,
            ttl: TTL_UNIQUE,
            rdata: vec![192, 168, 1, 99], // Different IP
        };
        assert!(resp.detect_conflict(&conflicting));
    }

    #[test]
    fn test_no_conflict_same_data() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        resp.set_ipv4([192, 168, 1, 10]);

        let same = MdnsRecord {
            name: String::from("myhost.local"),
            rtype: MdnsRecordType::A,
            cache_flush: true,
            ttl: TTL_UNIQUE,
            rdata: vec![192, 168, 1, 10], // Same IP
        };
        assert!(!resp.detect_conflict(&same));
    }

    // ---- Known-answer suppression ----

    #[test]
    fn test_known_answer_suppression() {
        let resp = MdnsResponder::new("myhost", 1000);

        let mut answers = vec![MdnsRecord {
            name: String::from("_http._tcp.local"),
            rtype: MdnsRecordType::PTR,
            cache_flush: false,
            ttl: TTL_SHARED,
            rdata: encode_dns_name("Server._http._tcp.local"),
        }];

        let known = vec![MdnsRecord {
            name: String::from("_http._tcp.local"),
            rtype: MdnsRecordType::PTR,
            cache_flush: false,
            ttl: TTL_SHARED, // >= 50% of our TTL
            rdata: encode_dns_name("Server._http._tcp.local"),
        }];

        resp.suppress_known_answers(&mut answers, &known);
        assert!(answers.is_empty(), "Answer should be suppressed");
    }

    #[test]
    fn test_known_answer_not_suppressed_low_ttl() {
        let resp = MdnsResponder::new("myhost", 1000);

        let mut answers = vec![MdnsRecord {
            name: String::from("_http._tcp.local"),
            rtype: MdnsRecordType::PTR,
            cache_flush: false,
            ttl: TTL_SHARED,
            rdata: encode_dns_name("Server._http._tcp.local"),
        }];

        let known = vec![MdnsRecord {
            name: String::from("_http._tcp.local"),
            rtype: MdnsRecordType::PTR,
            cache_flush: false,
            ttl: 1, // Much less than 50% of TTL_SHARED
            rdata: encode_dns_name("Server._http._tcp.local"),
        }];

        resp.suppress_known_answers(&mut answers, &known);
        assert_eq!(
            answers.len(),
            1,
            "Answer should NOT be suppressed with low TTL"
        );
    }

    // ---- TTL expiry ----

    #[test]
    fn test_cache_entry_expiry() {
        let entry = CacheEntry {
            record: MdnsRecord {
                name: String::from("host.local"),
                rtype: MdnsRecordType::A,
                cache_flush: true,
                ttl: 120,
                rdata: vec![10, 0, 0, 1],
            },
            inserted_tick: 0,
            original_ttl: 120,
        };

        // 1000 ticks/sec, 119 seconds elapsed: not expired
        assert!(!entry.is_expired(119_000, 1000));
        assert_eq!(entry.remaining_ttl(119_000, 1000), 1);

        // 120 seconds elapsed: expired
        assert!(entry.is_expired(120_000, 1000));
        assert_eq!(entry.remaining_ttl(120_000, 1000), 0);
    }

    // ---- Goodbye packet generation ----

    #[test]
    fn test_goodbye_packets_on_deregister() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        let mut txt = TxtRecord::new();
        txt.add("path", "/").unwrap();

        let svc = ServiceInstance {
            instance_name: String::from("WebSrv"),
            service_type: ServiceType::parse("_http._tcp.local").unwrap(),
            port: 80,
            target: String::from("myhost.local"),
            priority: 0,
            weight: 0,
            txt,
        };
        resp.register_service(svc).unwrap();

        let goodbyes = resp.deregister_service("WebSrv");
        assert_eq!(goodbyes.len(), 3); // PTR, SRV, TXT
        for g in &goodbyes {
            assert_eq!(g.ttl, 0, "Goodbye packets must have TTL=0");
        }
        assert_eq!(goodbyes[0].rtype, MdnsRecordType::PTR);
        assert_eq!(goodbyes[1].rtype, MdnsRecordType::SRV);
        assert_eq!(goodbyes[2].rtype, MdnsRecordType::TXT);
    }

    // ---- Service registration and lookup ----

    #[test]
    fn test_register_and_lookup_service() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        let svc = ServiceInstance {
            instance_name: String::from("My SSH"),
            service_type: ServiceType::parse("_ssh._tcp.local").unwrap(),
            port: 22,
            target: String::from("myhost.local"),
            priority: 0,
            weight: 0,
            txt: TxtRecord::new(),
        };
        resp.register_service(svc).unwrap();

        let found = resp.browse_services("_ssh._tcp.local");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].port, 22);

        let by_name = resp.lookup_service("My SSH._ssh._tcp.local");
        assert!(by_name.is_some());
    }

    // ---- .local suffix detection ----

    #[test]
    fn test_is_local_name() {
        assert!(is_local_name("myhost.local"));
        assert!(is_local_name("myhost.local."));
        assert!(is_local_name("sub.myhost.local"));
        assert!(!is_local_name("myhost.com"));
        assert!(!is_local_name("myhost.localhost"));
    }

    // ---- Probe message construction ----

    #[test]
    fn test_build_probe_query() {
        let query = build_probe_query("myhost.local");
        // Check header: ID=0, flags=0, QDCOUNT=1, others=0
        assert_eq!(query[0], 0); // ID high
        assert_eq!(query[1], 0); // ID low
        assert_eq!(query[4], 0); // QDCOUNT high
        assert_eq!(query[5], 1); // QDCOUNT low = 1
        assert_eq!(query[6], 0); // ANCOUNT high
        assert_eq!(query[7], 0); // ANCOUNT low

        // Question section starts at offset 12
        // First label: "myhost" (length 6)
        assert_eq!(query[12], 6);
        assert_eq!(&query[13..19], b"myhost");
        // Second label: "local" (length 5)
        assert_eq!(query[19], 5);
        assert_eq!(&query[20..25], b"local");
        // Root label
        assert_eq!(query[25], 0);
        // Type = ANY (255)
        assert_eq!(query[26], 0);
        assert_eq!(query[27], 255);
        // Class = IN | QU bit (0x8001)
        assert_eq!(query[28], 0x80);
        assert_eq!(query[29], 0x01);
    }

    // ---- DNS name encoding ----

    #[test]
    fn test_encode_dns_name() {
        let encoded = encode_dns_name("myhost.local");
        assert_eq!(encoded[0], 6); // "myhost" length
        assert_eq!(&encoded[1..7], b"myhost");
        assert_eq!(encoded[7], 5); // "local" length
        assert_eq!(&encoded[8..13], b"local");
        assert_eq!(encoded[13], 0); // root
    }

    #[test]
    fn test_decode_dns_name() {
        let encoded = encode_dns_name("test.local");
        let (name, consumed) = decode_dns_name(&encoded, 0).unwrap();
        assert_eq!(name, "test.local");
        assert_eq!(consumed, encoded.len());
    }

    // ---- Reverse lookup ----

    #[test]
    fn test_is_reverse_lookup() {
        assert!(is_reverse_lookup("1.168.192.in-addr.arpa"));
        assert!(is_reverse_lookup("8.b.d.0.1.0.0.2.ip6.arpa"));
        assert!(!is_reverse_lookup("myhost.local"));
    }

    #[test]
    fn test_ipv4_reverse_name() {
        let name = ipv4_reverse_name([192, 168, 1, 10]);
        assert_eq!(name, "10.1.168.192.in-addr.arpa");
    }

    // ---- Cache operations ----

    #[test]
    fn test_cache_insert_and_lookup() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        let record = MdnsRecord {
            name: String::from("other.local"),
            rtype: MdnsRecordType::A,
            cache_flush: false,
            ttl: 120,
            rdata: vec![10, 0, 0, 2],
        };
        resp.cache_insert(record, 0).unwrap();

        let results = resp.cache_lookup("other.local", MdnsRecordType::A, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].record.rdata, vec![10, 0, 0, 2]);
    }

    #[test]
    fn test_cache_goodbye_removes() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        let record = MdnsRecord {
            name: String::from("gone.local"),
            rtype: MdnsRecordType::A,
            cache_flush: false,
            ttl: 120,
            rdata: vec![10, 0, 0, 3],
        };
        resp.cache_insert(record, 0).unwrap();
        assert_eq!(resp.cache_size(), 1);

        // Goodbye packet
        let goodbye = MdnsRecord {
            name: String::from("gone.local"),
            rtype: MdnsRecordType::A,
            cache_flush: false,
            ttl: 0,
            rdata: vec![10, 0, 0, 3],
        };
        resp.cache_insert(goodbye, 1000).unwrap();
        assert_eq!(resp.cache_size(), 0);
    }

    // ---- Probe state machine ----

    #[test]
    fn test_probe_state_machine() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        resp.start_probe("myhost.local", 0);

        // Probe interval = 250ms * 1000 tps = 250 ticks
        let actions = resp.tick_probes(250);
        assert_eq!(actions.len(), 1);

        let actions = resp.tick_probes(500);
        assert_eq!(actions.len(), 1);
        // After 3rd probe -> Announcing
        match actions[0].1 {
            ProbeState::Announcing { .. } => {}
            _ => panic!("Expected Announcing state after 3 probes"),
        }
    }

    // ---- Query answering ----

    #[test]
    fn test_answer_a_record() {
        let mut resp = MdnsResponder::new("myhost", 1000);
        resp.set_ipv4([192, 168, 1, 42]);

        let q = MdnsQuestion {
            name: String::from("myhost.local"),
            qtype: MdnsRecordType::A,
            unicast: false,
        };

        assert!(resp.has_answer(&q));
        let answers = resp.answer(&q);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].rtype, MdnsRecordType::A);
        assert_eq!(answers[0].rdata, vec![192, 168, 1, 42]);
        assert_eq!(answers[0].ttl, TTL_UNIQUE);
    }
}
