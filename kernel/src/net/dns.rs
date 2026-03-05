//! DNS resolver for VeridianOS
//!
//! Provides DNS name resolution with caching, label compression,
//! and support for common record types (A, AAAA, CNAME, MX, TXT, PTR, SRV).

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use spin::Mutex;

use super::Ipv4Address;

// ---------------------------------------------------------------------------
// DNS constants
// ---------------------------------------------------------------------------

/// DNS default port
pub const DNS_PORT: u16 = 53;

/// Maximum DNS message size (UDP)
const MAX_DNS_MSG_SIZE: usize = 512;

/// Maximum label length
const MAX_LABEL_LEN: usize = 63;

/// Maximum domain name length
const MAX_NAME_LEN: usize = 255;

/// Maximum DNS cache entries
const MAX_CACHE_ENTRIES: usize = 256;

/// Maximum nameservers
const MAX_NAMESERVERS: usize = 3;

/// Label pointer mask (top 2 bits = 11)
const LABEL_POINTER_MASK: u8 = 0xC0;

// ---------------------------------------------------------------------------
// DNS record types
// ---------------------------------------------------------------------------

/// DNS record type codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum DnsRecordType {
    /// IPv4 address
    A = 1,
    /// Name server
    NS = 2,
    /// Canonical name (alias)
    CNAME = 5,
    /// Start of authority
    SOA = 6,
    /// Domain name pointer (reverse lookup)
    PTR = 12,
    /// Mail exchange
    MX = 15,
    /// Text record
    TXT = 16,
    /// IPv6 address
    AAAA = 28,
    /// Service locator
    SRV = 33,
    /// Unknown type
    Unknown = 0,
}

impl DnsRecordType {
    pub fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::A,
            2 => Self::NS,
            5 => Self::CNAME,
            6 => Self::SOA,
            12 => Self::PTR,
            15 => Self::MX,
            16 => Self::TXT,
            28 => Self::AAAA,
            33 => Self::SRV,
            _ => Self::Unknown,
        }
    }

    pub fn to_u16(self) -> u16 {
        self as u16
    }
}

/// DNS class codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DnsClass {
    /// Internet
    IN = 1,
    /// Any class
    ANY = 255,
}

impl DnsClass {
    pub fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::IN,
            255 => Self::ANY,
            _ => Self::IN,
        }
    }
}

// ---------------------------------------------------------------------------
// DNS error type
// ---------------------------------------------------------------------------

/// DNS-specific errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    /// Message too short to parse
    MessageTooShort,
    /// Invalid label in domain name
    InvalidLabel,
    /// Name exceeds maximum length
    NameTooLong,
    /// Compression pointer loop detected
    CompressionLoop,
    /// Buffer too small for serialization
    BufferTooSmall,
    /// Server returned an error response code
    ServerError(DnsResponseCode),
    /// No nameservers configured
    NoNameservers,
    /// All nameservers timed out
    Timeout,
    /// No records found
    NotFound,
    /// Invalid message format
    InvalidFormat,
    /// Cache is full and eviction failed
    CacheFull,
}

/// DNS response codes (RCODE)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DnsResponseCode {
    NoError = 0,
    FormatError = 1,
    ServerFailure = 2,
    NameError = 3,
    NotImplemented = 4,
    Refused = 5,
}

impl DnsResponseCode {
    pub fn from_u8(val: u8) -> Self {
        match val & 0x0F {
            0 => Self::NoError,
            1 => Self::FormatError,
            2 => Self::ServerFailure,
            3 => Self::NameError,
            4 => Self::NotImplemented,
            5 => Self::Refused,
            _ => Self::ServerFailure,
        }
    }
}

// ---------------------------------------------------------------------------
// DNS message header
// ---------------------------------------------------------------------------

/// DNS message header (12 bytes)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsHeader {
    /// Transaction ID
    pub id: u16,
    /// QR: 0=query, 1=response
    pub qr: bool,
    /// Opcode: 0=standard query
    pub opcode: u8,
    /// Authoritative answer
    pub aa: bool,
    /// Message truncated
    pub tc: bool,
    /// Recursion desired
    pub rd: bool,
    /// Recursion available
    pub ra: bool,
    /// Response code
    pub rcode: DnsResponseCode,
    /// Question count
    pub qdcount: u16,
    /// Answer count
    pub ancount: u16,
    /// Authority count
    pub nscount: u16,
    /// Additional count
    pub arcount: u16,
}

impl DnsHeader {
    pub const SIZE: usize = 12;

    /// Create a new query header
    pub fn new_query(id: u16) -> Self {
        Self {
            id,
            qr: false,
            opcode: 0,
            aa: false,
            tc: false,
            rd: true,
            ra: false,
            rcode: DnsResponseCode::NoError,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        }
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self, buf: &mut [u8]) -> Result<usize, DnsError> {
        if buf.len() < Self::SIZE {
            return Err(DnsError::BufferTooSmall);
        }

        buf[0..2].copy_from_slice(&self.id.to_be_bytes());

        let mut flags: u16 = 0;
        if self.qr {
            flags |= 1 << 15;
        }
        flags |= (self.opcode as u16 & 0x0F) << 11;
        if self.aa {
            flags |= 1 << 10;
        }
        if self.tc {
            flags |= 1 << 9;
        }
        if self.rd {
            flags |= 1 << 8;
        }
        if self.ra {
            flags |= 1 << 7;
        }
        flags |= self.rcode as u16 & 0x0F;

        buf[2..4].copy_from_slice(&flags.to_be_bytes());
        buf[4..6].copy_from_slice(&self.qdcount.to_be_bytes());
        buf[6..8].copy_from_slice(&self.ancount.to_be_bytes());
        buf[8..10].copy_from_slice(&self.nscount.to_be_bytes());
        buf[10..12].copy_from_slice(&self.arcount.to_be_bytes());

        Ok(Self::SIZE)
    }

    /// Parse header from bytes
    pub fn from_bytes(buf: &[u8]) -> Result<Self, DnsError> {
        if buf.len() < Self::SIZE {
            return Err(DnsError::MessageTooShort);
        }

        let id = u16::from_be_bytes([buf[0], buf[1]]);
        let flags = u16::from_be_bytes([buf[2], buf[3]]);

        Ok(Self {
            id,
            qr: (flags >> 15) & 1 == 1,
            opcode: ((flags >> 11) & 0x0F) as u8,
            aa: (flags >> 10) & 1 == 1,
            tc: (flags >> 9) & 1 == 1,
            rd: (flags >> 8) & 1 == 1,
            ra: (flags >> 7) & 1 == 1,
            rcode: DnsResponseCode::from_u8((flags & 0x0F) as u8),
            qdcount: u16::from_be_bytes([buf[4], buf[5]]),
            ancount: u16::from_be_bytes([buf[6], buf[7]]),
            nscount: u16::from_be_bytes([buf[8], buf[9]]),
            arcount: u16::from_be_bytes([buf[10], buf[11]]),
        })
    }
}

// ---------------------------------------------------------------------------
// DNS question
// ---------------------------------------------------------------------------

/// DNS question section entry
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct DnsQuestion {
    pub qname: String,
    pub qtype: DnsRecordType,
    pub qclass: DnsClass,
}

// ---------------------------------------------------------------------------
// DNS record data
// ---------------------------------------------------------------------------

/// DNS resource record data
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub enum DnsRecordData {
    /// A record: IPv4 address
    A(Ipv4Address),
    /// AAAA record: IPv6 address (16 bytes)
    AAAA([u8; 16]),
    /// CNAME record: canonical name
    CNAME(String),
    /// MX record: preference + exchange
    MX { preference: u16, exchange: String },
    /// TXT record: text data
    TXT(String),
    /// PTR record: domain name
    PTR(String),
    /// SRV record: priority, weight, port, target
    SRV {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    /// NS record: nameserver
    NS(String),
    /// Raw/unknown record data
    Raw(Vec<u8>),
}

/// DNS resource record
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
pub struct DnsRecord {
    pub name: String,
    pub rtype: DnsRecordType,
    pub rclass: DnsClass,
    pub ttl: u32,
    pub data: DnsRecordData,
}

// ---------------------------------------------------------------------------
// Label encoding / decoding
// ---------------------------------------------------------------------------

/// Encode a dotted domain name into DNS label format.
///
/// Example: "www.example.com" -> [3, 'w', 'w', 'w', 7, 'e', 'x', 'a', 'm', 'p',
/// 'l', 'e', 3, 'c', 'o', 'm', 0]
pub fn encode_name(name: &str, buf: &mut [u8]) -> Result<usize, DnsError> {
    let mut pos = 0;

    if name.is_empty() {
        if buf.is_empty() {
            return Err(DnsError::BufferTooSmall);
        }
        buf[0] = 0;
        return Ok(1);
    }

    for label in name.split('.') {
        let len = label.len();
        if len == 0 {
            continue;
        }
        if len > MAX_LABEL_LEN {
            return Err(DnsError::InvalidLabel);
        }
        if pos + 1 + len >= buf.len() {
            return Err(DnsError::BufferTooSmall);
        }
        buf[pos] = len as u8;
        pos += 1;
        buf[pos..pos + len].copy_from_slice(label.as_bytes());
        pos += len;
    }

    if pos >= MAX_NAME_LEN {
        return Err(DnsError::NameTooLong);
    }

    if pos >= buf.len() {
        return Err(DnsError::BufferTooSmall);
    }
    buf[pos] = 0; // null terminator
    pos += 1;

    Ok(pos)
}

/// Decode a DNS label sequence from a message buffer, handling pointer
/// compression.
///
/// Returns (decoded name string, number of bytes consumed at `offset`).
#[cfg(feature = "alloc")]
pub fn decode_name(msg: &[u8], offset: usize) -> Result<(String, usize), DnsError> {
    let mut name = String::new();
    let mut pos = offset;
    let mut consumed = 0;
    let mut followed_pointer = false;
    let mut jumps = 0;
    const MAX_JUMPS: usize = 16;

    loop {
        if pos >= msg.len() {
            return Err(DnsError::MessageTooShort);
        }

        let len_byte = msg[pos];

        if len_byte == 0 {
            // End of name
            if !followed_pointer {
                consumed = pos - offset + 1;
            }
            break;
        }

        if len_byte & LABEL_POINTER_MASK == LABEL_POINTER_MASK {
            // Pointer compression: 2-byte pointer
            if pos + 1 >= msg.len() {
                return Err(DnsError::MessageTooShort);
            }
            if !followed_pointer {
                consumed = pos - offset + 2;
                followed_pointer = true;
            }
            let ptr_offset =
                (((len_byte & !LABEL_POINTER_MASK) as usize) << 8) | (msg[pos + 1] as usize);
            jumps += 1;
            if jumps > MAX_JUMPS {
                return Err(DnsError::CompressionLoop);
            }
            pos = ptr_offset;
            continue;
        }

        // Regular label
        let label_len = len_byte as usize;
        pos += 1;
        if pos + label_len > msg.len() {
            return Err(DnsError::MessageTooShort);
        }

        if !name.is_empty() {
            name.push('.');
        }
        for &b in &msg[pos..pos + label_len] {
            name.push(b as char);
        }
        pos += label_len;
    }

    if consumed == 0 && !followed_pointer {
        consumed = 1; // just the null terminator for root name
    }

    Ok((name, consumed))
}

// ---------------------------------------------------------------------------
// DNS message serialization
// ---------------------------------------------------------------------------

/// Build a DNS query message for the given name and record type.
///
/// Returns the number of bytes written to `buf`.
pub fn build_query(
    buf: &mut [u8],
    id: u16,
    name: &str,
    rtype: DnsRecordType,
) -> Result<usize, DnsError> {
    let header = DnsHeader::new_query(id);
    let mut pos = header.to_bytes(buf)?;

    // Encode question name
    pos += encode_name(name, &mut buf[pos..])?;

    // QTYPE (2 bytes)
    if pos + 4 > buf.len() {
        return Err(DnsError::BufferTooSmall);
    }
    buf[pos..pos + 2].copy_from_slice(&rtype.to_u16().to_be_bytes());
    pos += 2;

    // QCLASS = IN (2 bytes)
    buf[pos..pos + 2].copy_from_slice(&DnsClass::IN.to_u16().to_be_bytes());
    pos += 2;

    Ok(pos)
}

impl DnsClass {
    pub fn to_u16(self) -> u16 {
        self as u16
    }
}

/// Parse a DNS response message and extract resource records.
#[cfg(feature = "alloc")]
pub fn parse_response(msg: &[u8]) -> Result<(DnsHeader, Vec<DnsRecord>), DnsError> {
    let header = DnsHeader::from_bytes(msg)?;

    if !header.qr {
        return Err(DnsError::InvalidFormat);
    }
    if header.rcode as u8 != 0 {
        return Err(DnsError::ServerError(header.rcode));
    }

    let mut pos = DnsHeader::SIZE;

    // Skip question section
    for _ in 0..header.qdcount {
        let (_qname, consumed) = decode_name(msg, pos)?;
        pos += consumed;
        pos += 4; // QTYPE + QCLASS
        if pos > msg.len() {
            return Err(DnsError::MessageTooShort);
        }
    }

    // Parse answer + authority + additional sections
    let total_rr = header.ancount as usize + header.nscount as usize + header.arcount as usize;
    let mut records = Vec::with_capacity(total_rr);

    for _ in 0..total_rr {
        if pos >= msg.len() {
            break;
        }
        let (name, consumed) = decode_name(msg, pos)?;
        pos += consumed;

        if pos + 10 > msg.len() {
            return Err(DnsError::MessageTooShort);
        }

        let rtype = DnsRecordType::from_u16(u16::from_be_bytes([msg[pos], msg[pos + 1]]));
        let rclass = DnsClass::from_u16(u16::from_be_bytes([msg[pos + 2], msg[pos + 3]]));
        let ttl = u32::from_be_bytes([msg[pos + 4], msg[pos + 5], msg[pos + 6], msg[pos + 7]]);
        let rdlength = u16::from_be_bytes([msg[pos + 8], msg[pos + 9]]) as usize;
        pos += 10;

        if pos + rdlength > msg.len() {
            return Err(DnsError::MessageTooShort);
        }

        let data = parse_rdata(msg, pos, rdlength, rtype)?;
        pos += rdlength;

        records.push(DnsRecord {
            name,
            rtype,
            rclass,
            ttl,
            data,
        });
    }

    Ok((header, records))
}

/// Parse resource record data based on record type.
#[cfg(feature = "alloc")]
fn parse_rdata(
    msg: &[u8],
    offset: usize,
    rdlength: usize,
    rtype: DnsRecordType,
) -> Result<DnsRecordData, DnsError> {
    match rtype {
        DnsRecordType::A => {
            if rdlength != 4 {
                return Err(DnsError::InvalidFormat);
            }
            Ok(DnsRecordData::A(Ipv4Address::new(
                msg[offset],
                msg[offset + 1],
                msg[offset + 2],
                msg[offset + 3],
            )))
        }
        DnsRecordType::AAAA => {
            if rdlength != 16 {
                return Err(DnsError::InvalidFormat);
            }
            let mut addr = [0u8; 16];
            addr.copy_from_slice(&msg[offset..offset + 16]);
            Ok(DnsRecordData::AAAA(addr))
        }
        DnsRecordType::CNAME | DnsRecordType::PTR | DnsRecordType::NS => {
            let (name, _) = decode_name(msg, offset)?;
            match rtype {
                DnsRecordType::CNAME => Ok(DnsRecordData::CNAME(name)),
                DnsRecordType::PTR => Ok(DnsRecordData::PTR(name)),
                DnsRecordType::NS => Ok(DnsRecordData::NS(name)),
                _ => unreachable!(),
            }
        }
        DnsRecordType::MX => {
            if rdlength < 3 {
                return Err(DnsError::InvalidFormat);
            }
            let preference = u16::from_be_bytes([msg[offset], msg[offset + 1]]);
            let (exchange, _) = decode_name(msg, offset + 2)?;
            Ok(DnsRecordData::MX {
                preference,
                exchange,
            })
        }
        DnsRecordType::TXT => {
            // TXT records: one or more length-prefixed strings
            let mut text = String::new();
            let mut pos = offset;
            let end = offset + rdlength;
            while pos < end {
                let txt_len = msg[pos] as usize;
                pos += 1;
                if pos + txt_len > end {
                    return Err(DnsError::InvalidFormat);
                }
                for &b in &msg[pos..pos + txt_len] {
                    text.push(b as char);
                }
                pos += txt_len;
            }
            Ok(DnsRecordData::TXT(text))
        }
        DnsRecordType::SRV => {
            if rdlength < 7 {
                return Err(DnsError::InvalidFormat);
            }
            let priority = u16::from_be_bytes([msg[offset], msg[offset + 1]]);
            let weight = u16::from_be_bytes([msg[offset + 2], msg[offset + 3]]);
            let port = u16::from_be_bytes([msg[offset + 4], msg[offset + 5]]);
            let (target, _) = decode_name(msg, offset + 6)?;
            Ok(DnsRecordData::SRV {
                priority,
                weight,
                port,
                target,
            })
        }
        _ => {
            let mut raw = vec![0u8; rdlength];
            raw.copy_from_slice(&msg[offset..offset + rdlength]);
            Ok(DnsRecordData::Raw(raw))
        }
    }
}

// ---------------------------------------------------------------------------
// DNS cache
// ---------------------------------------------------------------------------

/// A cached DNS record entry
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
struct CacheEntry {
    record: DnsRecord,
    expires_at: u64,
    last_used: u64,
}

/// DNS cache with LRU eviction and TTL-based expiry
#[cfg(feature = "alloc")]
pub struct DnsCache {
    /// Entries keyed by (name, record type)
    entries: BTreeMap<(String, u16), Vec<CacheEntry>>,
    /// Total number of individual entries
    count: usize,
    /// Maximum entries
    max_entries: usize,
    /// Monotonic clock counter (incremented on each access)
    clock: u64,
}

#[cfg(feature = "alloc")]
impl Default for DnsCache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            count: 0,
            max_entries: MAX_CACHE_ENTRIES,
            clock: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl DnsCache {
    /// Create a new DNS cache
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached record by name and type
    pub fn lookup(&mut self, name: &str, rtype: DnsRecordType) -> Option<Vec<DnsRecord>> {
        self.clock += 1;
        let now = self.clock;

        let key = (String::from(name), rtype.to_u16());
        let entries = self.entries.get_mut(&key)?;

        // Remove expired entries
        let before_len = entries.len();
        entries.retain(|e| e.expires_at > now);
        self.count -= before_len - entries.len();

        if entries.is_empty() {
            self.entries.remove(&key);
            return None;
        }

        // Update last_used for all matching entries
        for entry in entries.iter_mut() {
            entry.last_used = now;
        }

        Some(entries.iter().map(|e| e.record.clone()).collect())
    }

    /// Insert a record into the cache with the given TTL
    pub fn insert(&mut self, name: &str, record: DnsRecord, ttl: u32) {
        self.clock += 1;
        let now = self.clock;

        // Evict expired entries first
        self.evict_expired(now);

        // If still at capacity, evict LRU
        while self.count >= self.max_entries {
            self.evict_lru();
        }

        let key = (String::from(name), record.rtype.to_u16());
        let entry = CacheEntry {
            record,
            expires_at: now + ttl as u64,
            last_used: now,
        };

        self.entries.entry(key).or_default().push(entry);
        self.count += 1;
    }

    /// Remove all expired entries
    pub fn evict_expired(&mut self, now: u64) {
        let mut keys_to_remove = Vec::new();

        for (key, entries) in self.entries.iter_mut() {
            let before = entries.len();
            entries.retain(|e| e.expires_at > now);
            self.count -= before - entries.len();
            if entries.is_empty() {
                keys_to_remove.push(key.clone());
            }
        }

        for key in keys_to_remove {
            self.entries.remove(&key);
        }
    }

    /// Evict the least recently used entry
    fn evict_lru(&mut self) {
        let mut oldest_key: Option<(String, u16)> = None;
        let mut oldest_time = u64::MAX;

        for (key, entries) in &self.entries {
            for entry in entries {
                if entry.last_used < oldest_time {
                    oldest_time = entry.last_used;
                    oldest_key = Some(key.clone());
                }
            }
        }

        if let Some(key) = oldest_key {
            if let Some(entries) = self.entries.get_mut(&key) {
                // Remove the oldest entry from this key's list
                if let Some(idx) = entries.iter().position(|e| e.last_used == oldest_time) {
                    entries.remove(idx);
                    self.count -= 1;
                }
                if entries.is_empty() {
                    self.entries.remove(&key);
                }
            }
        }
    }

    /// Return the number of cached entries
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
        self.count = 0;
    }
}

// ---------------------------------------------------------------------------
// Hosts file entry
// ---------------------------------------------------------------------------

/// Static host entry (from /etc/hosts)
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct HostEntry {
    pub name: String,
    pub addr: Ipv4Address,
}

// ---------------------------------------------------------------------------
// DNS resolver
// ---------------------------------------------------------------------------

/// DNS resolver with caching and multiple nameserver support
#[cfg(feature = "alloc")]
pub struct DnsResolver {
    /// Configured nameservers (up to MAX_NAMESERVERS)
    nameservers: Vec<Ipv4Address>,
    /// DNS cache
    cache: DnsCache,
    /// Static hosts entries
    hosts: Vec<HostEntry>,
    /// Next transaction ID
    next_id: u16,
}

#[cfg(feature = "alloc")]
impl Default for DnsResolver {
    fn default() -> Self {
        let mut resolver = Self {
            nameservers: Vec::new(),
            cache: DnsCache::new(),
            hosts: Vec::new(),
            next_id: 1,
        };

        // Add default localhost entry
        resolver.hosts.push(HostEntry {
            name: String::from("localhost"),
            addr: Ipv4Address::LOCALHOST,
        });

        resolver
    }
}

#[cfg(feature = "alloc")]
impl DnsResolver {
    /// Create a new DNS resolver with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a nameserver address
    pub fn add_nameserver(&mut self, addr: Ipv4Address) {
        if self.nameservers.len() < MAX_NAMESERVERS {
            self.nameservers.push(addr);
        }
    }

    /// Add a static host entry
    pub fn add_host(&mut self, name: &str, addr: Ipv4Address) {
        self.hosts.push(HostEntry {
            name: String::from(name),
            addr,
        });
    }

    /// Parse resolv.conf content to extract nameserver lines
    pub fn parse_resolv_conf(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(addr_str) = line.strip_prefix("nameserver") {
                let addr_str = addr_str.trim();
                if let Some(addr) = parse_ipv4(addr_str) {
                    self.add_nameserver(addr);
                }
            }
        }
    }

    /// Parse hosts file content
    pub fn parse_hosts(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            // Split on whitespace: first token is IP, rest are hostnames
            let mut parts = line.split_whitespace();
            if let Some(addr_str) = parts.next() {
                if let Some(addr) = parse_ipv4(addr_str) {
                    for hostname in parts {
                        if hostname.starts_with('#') {
                            break;
                        }
                        self.add_host(hostname, addr);
                    }
                }
            }
        }
    }

    /// Look up a name in the static hosts table
    pub fn lookup_hosts(&self, name: &str) -> Option<Ipv4Address> {
        for entry in &self.hosts {
            if entry.name == name {
                return Some(entry.addr);
            }
        }
        None
    }

    /// Resolve a domain name to DNS records.
    ///
    /// Check order: hosts file -> cache -> network query
    pub fn resolve(
        &mut self,
        name: &str,
        rtype: DnsRecordType,
    ) -> Result<Vec<DnsRecord>, DnsError> {
        // Check static hosts for A records
        if rtype == DnsRecordType::A {
            if let Some(addr) = self.lookup_hosts(name) {
                return Ok(vec![DnsRecord {
                    name: String::from(name),
                    rtype: DnsRecordType::A,
                    rclass: DnsClass::IN,
                    ttl: 0,
                    data: DnsRecordData::A(addr),
                }]);
            }
        }

        // Check cache
        if let Some(records) = self.cache.lookup(name, rtype) {
            return Ok(records);
        }

        // Build query packet
        let mut query_buf = [0u8; MAX_DNS_MSG_SIZE];
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let query_len = build_query(&mut query_buf, id, name, rtype)?;

        // Try each nameserver
        if self.nameservers.is_empty() {
            return Err(DnsError::NoNameservers);
        }

        let mut last_err = DnsError::Timeout;

        for ns_idx in 0..self.nameservers.len() {
            let _ns_addr = self.nameservers[ns_idx];

            // In a real implementation, we would send the UDP packet via
            // the socket layer and wait for a response. For now, we prepare
            // the query and return an error indicating no response.
            //
            // Future: udp::send_to(ns_addr, DNS_PORT, &query_buf[..query_len])
            //         udp::recv_from(timeout) -> response
            let _ = query_len;

            // Placeholder: would parse response here
            // let (header, records) = parse_response(&response_buf)?;
            // Cache results and return

            last_err = DnsError::Timeout;
        }

        Err(last_err)
    }

    /// Insert records into the cache (for use when response is received)
    pub fn cache_records(&mut self, records: &[DnsRecord]) {
        for record in records {
            self.cache.insert(&record.name, record.clone(), record.ttl);
        }
    }

    /// Get cache statistics
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Clear the DNS cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

// ---------------------------------------------------------------------------
// Helper: parse IPv4 address from string
// ---------------------------------------------------------------------------

/// Parse a dotted-decimal IPv4 address string
fn parse_ipv4(s: &str) -> Option<Ipv4Address> {
    let mut octets = [0u8; 4];
    let mut count = 0;

    for part in s.split('.') {
        if count >= 4 {
            return None;
        }
        let val: u16 = {
            let mut n: u16 = 0;
            for &b in part.as_bytes() {
                if !b.is_ascii_digit() {
                    return None;
                }
                n = n.checked_mul(10)?.checked_add((b - b'0') as u16)?;
            }
            n
        };
        if val > 255 {
            return None;
        }
        octets[count] = val as u8;
        count += 1;
    }

    if count != 4 {
        return None;
    }
    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

// ---------------------------------------------------------------------------
// Global resolver instance
// ---------------------------------------------------------------------------

#[cfg(feature = "alloc")]
static DNS_RESOLVER: crate::sync::once_lock::GlobalState<Mutex<DnsResolver>> =
    crate::sync::once_lock::GlobalState::new();

/// Initialize the global DNS resolver
#[cfg(feature = "alloc")]
pub fn init() -> Result<(), DnsError> {
    let resolver = DnsResolver::new();
    DNS_RESOLVER
        .init(Mutex::new(resolver))
        .map_err(|_| DnsError::InvalidFormat)?;
    Ok(())
}

/// Resolve a domain name using the global resolver
#[cfg(feature = "alloc")]
pub fn resolve(name: &str, rtype: DnsRecordType) -> Result<Vec<DnsRecord>, DnsError> {
    DNS_RESOLVER
        .with(|lock| {
            let mut resolver = lock.lock();
            resolver.resolve(name, rtype)
        })
        .unwrap_or(Err(DnsError::NoNameservers))
}

/// Add a nameserver to the global resolver
#[cfg(feature = "alloc")]
pub fn add_nameserver(addr: Ipv4Address) {
    DNS_RESOLVER.with(|lock| {
        let mut resolver = lock.lock();
        resolver.add_nameserver(addr);
    });
}

/// Add a host entry to the global resolver
#[cfg(feature = "alloc")]
pub fn add_host(name: &str, addr: Ipv4Address) {
    DNS_RESOLVER.with(|lock| {
        let mut resolver = lock.lock();
        resolver.add_host(name, addr);
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use alloc::vec;

    #[test]
    fn test_dns_record_type_roundtrip() {
        let types = [
            DnsRecordType::A,
            DnsRecordType::AAAA,
            DnsRecordType::CNAME,
            DnsRecordType::MX,
            DnsRecordType::TXT,
            DnsRecordType::PTR,
            DnsRecordType::SRV,
            DnsRecordType::NS,
        ];
        for t in &types {
            assert_eq!(DnsRecordType::from_u16(t.to_u16()), *t);
        }
    }

    #[test]
    fn test_dns_record_type_unknown() {
        assert_eq!(DnsRecordType::from_u16(999), DnsRecordType::Unknown);
    }

    #[test]
    fn test_encode_name_simple() {
        let mut buf = [0u8; 64];
        let len = encode_name("www.example.com", &mut buf).unwrap();
        assert_eq!(len, 17);
        assert_eq!(buf[0], 3); // "www" length
        assert_eq!(&buf[1..4], b"www");
        assert_eq!(buf[4], 7); // "example" length
        assert_eq!(&buf[5..12], b"example");
        assert_eq!(buf[12], 3); // "com" length
        assert_eq!(&buf[13..16], b"com");
        assert_eq!(buf[16], 0); // null terminator
    }

    #[test]
    fn test_encode_name_single_label() {
        let mut buf = [0u8; 64];
        let len = encode_name("localhost", &mut buf).unwrap();
        assert_eq!(len, 11);
        assert_eq!(buf[0], 9);
        assert_eq!(&buf[1..10], b"localhost");
        assert_eq!(buf[10], 0);
    }

    #[test]
    fn test_encode_name_empty() {
        let mut buf = [0u8; 64];
        let len = encode_name("", &mut buf).unwrap();
        assert_eq!(len, 1);
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn test_encode_name_buffer_too_small() {
        let mut buf = [0u8; 3];
        let result = encode_name("www.example.com", &mut buf);
        assert_eq!(result, Err(DnsError::BufferTooSmall));
    }

    #[test]
    fn test_encode_name_label_too_long() {
        let long_label = "a".repeat(64);
        let mut buf = [0u8; 128];
        let result = encode_name(&long_label, &mut buf);
        assert_eq!(result, Err(DnsError::InvalidLabel));
    }

    #[test]
    fn test_decode_name_simple() {
        // Manually encoded "www.example.com"
        let msg = [
            3, b'w', b'w', b'w', 7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm',
            0,
        ];
        let (name, consumed) = decode_name(&msg, 0).unwrap();
        assert_eq!(name, "www.example.com");
        assert_eq!(consumed, 17);
    }

    #[test]
    fn test_decode_name_with_pointer() {
        // Message: offset 0 = "example.com", offset 13 = pointer to 0
        let mut msg = vec![
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        // At offset 13: "www" label then pointer to offset 0
        msg.extend_from_slice(&[3, b'w', b'w', b'w', 0xC0, 0x00]);

        let (name, consumed) = decode_name(&msg, 13).unwrap();
        assert_eq!(name, "www.example.com");
        assert_eq!(consumed, 6); // 1+3 (label) + 2 (pointer)
    }

    #[test]
    fn test_decode_name_compression_loop() {
        // Two pointers pointing at each other
        let msg = [0xC0, 0x02, 0xC0, 0x00];
        let result = decode_name(&msg, 0);
        assert_eq!(result, Err(DnsError::CompressionLoop));
    }

    #[test]
    fn test_header_roundtrip() {
        let header = DnsHeader {
            id: 0x1234,
            qr: true,
            opcode: 0,
            aa: true,
            tc: false,
            rd: true,
            ra: true,
            rcode: DnsResponseCode::NoError,
            qdcount: 1,
            ancount: 2,
            nscount: 0,
            arcount: 0,
        };

        let mut buf = [0u8; 12];
        let len = header.to_bytes(&mut buf).unwrap();
        assert_eq!(len, 12);

        let parsed = DnsHeader::from_bytes(&buf).unwrap();
        assert_eq!(parsed.id, 0x1234);
        assert!(parsed.qr);
        assert!(parsed.aa);
        assert!(!parsed.tc);
        assert!(parsed.rd);
        assert!(parsed.ra);
        assert_eq!(parsed.qdcount, 1);
        assert_eq!(parsed.ancount, 2);
    }

    #[test]
    fn test_header_too_short() {
        let buf = [0u8; 6];
        assert_eq!(DnsHeader::from_bytes(&buf), Err(DnsError::MessageTooShort));
    }

    #[test]
    fn test_build_query() {
        let mut buf = [0u8; 512];
        let len = build_query(&mut buf, 0xABCD, "example.com", DnsRecordType::A).unwrap();

        // Header (12) + name (13: 7+example+3+com+0) + qtype(2) + qclass(2) = 29
        assert_eq!(len, 29);

        let header = DnsHeader::from_bytes(&buf).unwrap();
        assert_eq!(header.id, 0xABCD);
        assert!(!header.qr);
        assert!(header.rd);
        assert_eq!(header.qdcount, 1);
    }

    #[test]
    fn test_parse_response_a_record() {
        // Build a minimal DNS response with one A record
        let mut msg = vec![0u8; 512];
        let mut pos = 0;

        // Header: id=1, QR=1, RD=1, RA=1, QDCOUNT=1, ANCOUNT=1
        msg[0..2].copy_from_slice(&1u16.to_be_bytes()); // ID
        msg[2..4].copy_from_slice(&0x8180u16.to_be_bytes()); // Flags: QR+RD+RA
        msg[4..6].copy_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        msg[6..8].copy_from_slice(&1u16.to_be_bytes()); // ANCOUNT
        msg[8..10].copy_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        msg[10..12].copy_from_slice(&0u16.to_be_bytes()); // ARCOUNT
        pos = 12;

        // Question: example.com A IN
        let name_bytes = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        msg[pos..pos + 13].copy_from_slice(&name_bytes);
        pos += 13;
        msg[pos..pos + 2].copy_from_slice(&1u16.to_be_bytes()); // QTYPE=A
        pos += 2;
        msg[pos..pos + 2].copy_from_slice(&1u16.to_be_bytes()); // QCLASS=IN
        pos += 2;

        // Answer: pointer to name at offset 12, TYPE=A, CLASS=IN, TTL=300, RDLENGTH=4,
        // RDATA=93.184.216.34
        msg[pos] = 0xC0;
        msg[pos + 1] = 0x0C; // pointer to offset 12
        pos += 2;
        msg[pos..pos + 2].copy_from_slice(&1u16.to_be_bytes()); // TYPE=A
        pos += 2;
        msg[pos..pos + 2].copy_from_slice(&1u16.to_be_bytes()); // CLASS=IN
        pos += 2;
        msg[pos..pos + 4].copy_from_slice(&300u32.to_be_bytes()); // TTL
        pos += 4;
        msg[pos..pos + 2].copy_from_slice(&4u16.to_be_bytes()); // RDLENGTH
        pos += 2;
        msg[pos..pos + 4].copy_from_slice(&[93, 184, 216, 34]); // RDATA
        pos += 4;

        let (header, records) = parse_response(&msg[..pos]).unwrap();
        assert_eq!(header.ancount, 1);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "example.com");
        assert_eq!(records[0].rtype, DnsRecordType::A);
        assert_eq!(records[0].ttl, 300);
        if let DnsRecordData::A(addr) = &records[0].data {
            assert_eq!(addr.0, [93, 184, 216, 34]);
        } else {
            panic!("Expected A record data");
        }
    }

    #[test]
    fn test_cache_insert_and_lookup() {
        let mut cache = DnsCache::new();

        let record = DnsRecord {
            name: String::from("example.com"),
            rtype: DnsRecordType::A,
            rclass: DnsClass::IN,
            ttl: 300,
            data: DnsRecordData::A(Ipv4Address::new(93, 184, 216, 34)),
        };

        cache.insert("example.com", record, 300);
        assert_eq!(cache.len(), 1);

        let result = cache.lookup("example.com", DnsRecordType::A);
        assert!(result.is_some());
        let records = result.unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn test_cache_expiry() {
        let mut cache = DnsCache::new();

        let record = DnsRecord {
            name: String::from("expire.test"),
            rtype: DnsRecordType::A,
            rclass: DnsClass::IN,
            ttl: 1, // TTL = 1 tick
            data: DnsRecordData::A(Ipv4Address::new(1, 2, 3, 4)),
        };

        cache.insert("expire.test", record, 1);
        assert_eq!(cache.len(), 1);

        // Advance clock past expiry by doing many lookups
        for _ in 0..5 {
            let _ = cache.lookup("other.name", DnsRecordType::A);
        }

        // The entry should now be expired
        let result = cache.lookup("expire.test", DnsRecordType::A);
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = DnsCache::new();
        // Reduce max for test
        cache.max_entries = 4;

        // Insert 4 entries
        for i in 0..4u8 {
            let name = alloc::format!("host{}.test", i);
            let record = DnsRecord {
                name: name.clone(),
                rtype: DnsRecordType::A,
                rclass: DnsClass::IN,
                ttl: 1000,
                data: DnsRecordData::A(Ipv4Address::new(10, 0, 0, i)),
            };
            cache.insert(&name, record, 1000);
        }
        assert_eq!(cache.len(), 4);

        // Access host1 and host3 to make them recently used
        let _ = cache.lookup("host1.test", DnsRecordType::A);
        let _ = cache.lookup("host3.test", DnsRecordType::A);

        // Insert a 5th entry - should evict host0 (least recently used)
        let record = DnsRecord {
            name: String::from("host4.test"),
            rtype: DnsRecordType::A,
            rclass: DnsClass::IN,
            ttl: 1000,
            data: DnsRecordData::A(Ipv4Address::new(10, 0, 0, 4)),
        };
        cache.insert("host4.test", record, 1000);

        // host0 should be evicted (it was inserted first and never accessed again)
        assert_eq!(cache.len(), 4);
        assert!(cache.lookup("host0.test", DnsRecordType::A).is_none());
        assert!(cache.lookup("host1.test", DnsRecordType::A).is_some());
    }

    #[test]
    fn test_cache_miss() {
        let mut cache = DnsCache::new();
        let result = cache.lookup("nonexistent.com", DnsRecordType::A);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_ipv4() {
        assert_eq!(
            parse_ipv4("192.168.1.1"),
            Some(Ipv4Address::new(192, 168, 1, 1))
        );
        assert_eq!(parse_ipv4("0.0.0.0"), Some(Ipv4Address::new(0, 0, 0, 0)));
        assert_eq!(
            parse_ipv4("255.255.255.255"),
            Some(Ipv4Address::new(255, 255, 255, 255))
        );
        assert_eq!(parse_ipv4("256.0.0.1"), None);
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("abc"), None);
        assert_eq!(parse_ipv4(""), None);
    }

    #[test]
    fn test_resolver_hosts_lookup() {
        let mut resolver = DnsResolver::new();
        resolver.add_host("myhost.local", Ipv4Address::new(10, 0, 0, 1));

        assert_eq!(
            resolver.lookup_hosts("localhost"),
            Some(Ipv4Address::LOCALHOST)
        );
        assert_eq!(
            resolver.lookup_hosts("myhost.local"),
            Some(Ipv4Address::new(10, 0, 0, 1))
        );
        assert_eq!(resolver.lookup_hosts("unknown.host"), None);
    }

    #[test]
    fn test_resolver_parse_resolv_conf() {
        let mut resolver = DnsResolver::new();
        let content = "\
# DNS config
nameserver 8.8.8.8
nameserver 8.8.4.4
# nameserver 1.1.1.1
nameserver 9.9.9.9
";
        resolver.parse_resolv_conf(content);
        assert_eq!(resolver.nameservers.len(), 3);
        assert_eq!(resolver.nameservers[0], Ipv4Address::new(8, 8, 8, 8));
        assert_eq!(resolver.nameservers[1], Ipv4Address::new(8, 8, 4, 4));
        assert_eq!(resolver.nameservers[2], Ipv4Address::new(9, 9, 9, 9));
    }

    #[test]
    fn test_resolver_parse_hosts_file() {
        let mut resolver = DnsResolver::new();
        let content = "\
127.0.0.1   localhost loopback
192.168.1.100   myserver.local myserver
# comment line
10.0.0.1   gateway.local
";
        resolver.parse_hosts(content);
        assert_eq!(
            resolver.lookup_hosts("myserver.local"),
            Some(Ipv4Address::new(192, 168, 1, 100))
        );
        assert_eq!(
            resolver.lookup_hosts("myserver"),
            Some(Ipv4Address::new(192, 168, 1, 100))
        );
        assert_eq!(
            resolver.lookup_hosts("gateway.local"),
            Some(Ipv4Address::new(10, 0, 0, 1))
        );
    }

    #[test]
    fn test_resolver_hosts_before_network() {
        let mut resolver = DnsResolver::new();

        // Should resolve localhost from hosts without needing nameservers
        let result = resolver.resolve("localhost", DnsRecordType::A);
        assert!(result.is_ok());
        let records = result.unwrap();
        assert_eq!(records.len(), 1);
        if let DnsRecordData::A(addr) = &records[0].data {
            assert_eq!(*addr, Ipv4Address::LOCALHOST);
        } else {
            panic!("Expected A record");
        }
    }

    #[test]
    fn test_response_code_parse() {
        assert_eq!(DnsResponseCode::from_u8(0), DnsResponseCode::NoError);
        assert_eq!(DnsResponseCode::from_u8(3), DnsResponseCode::NameError);
        assert_eq!(DnsResponseCode::from_u8(5), DnsResponseCode::Refused);
        assert_eq!(
            DnsResponseCode::from_u8(0xFF),
            DnsResponseCode::ServerFailure
        );
    }

    #[test]
    fn test_dns_class_roundtrip() {
        assert_eq!(DnsClass::from_u16(1), DnsClass::IN);
        assert_eq!(DnsClass::from_u16(255), DnsClass::ANY);
        assert_eq!(DnsClass::IN.to_u16(), 1);
    }
}
