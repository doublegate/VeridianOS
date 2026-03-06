//! Kerberos Ticket Cache (ccache)
//!
//! Implements an in-memory credential cache for Kerberos tickets, compatible
//! with the MIT krb5 ccache binary format for serialization/deserialization.
//!
//! # Features
//!
//! - Store/lookup/remove tickets by server principal
//! - TTL-based expiry using kernel tick counts
//! - MIT krb5 ccache v4 binary format serialization
//! - Shell command helpers: kinit, klist, kdestroy
//! - Auth backend integration for ticket-based verification

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

use super::protocol::{EncryptionType, KerberosClient, KerberosTime, PrincipalName};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Ccache Constants
// ---------------------------------------------------------------------------

/// MIT krb5 ccache file format version (0x0504)
const CCACHE_VERSION: u16 = 0x0504;

/// Maximum number of cached tickets
const MAX_CACHE_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// Ccache Entry
// ---------------------------------------------------------------------------

/// A single cached Kerberos ticket.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CcacheEntry {
    /// Client principal
    pub client_principal: PrincipalName,
    /// Server principal (service/host)
    pub server_principal: PrincipalName,
    /// Session key (decrypted)
    pub session_key: Vec<u8>,
    /// Session key encryption type
    pub session_key_etype: EncryptionType,
    /// Authentication time
    pub auth_time: KerberosTime,
    /// Start time (when ticket becomes valid)
    pub start_time: KerberosTime,
    /// End time (when ticket expires)
    pub end_time: KerberosTime,
    /// Renewal end time
    pub renew_till: Option<KerberosTime>,
    /// Raw ticket data (BER-encoded Ticket)
    pub ticket_data: Vec<u8>,
    /// Ticket flags
    pub flags: u32,
}

#[cfg(feature = "alloc")]
impl CcacheEntry {
    /// Check if this ticket has expired.
    pub fn is_expired(&self) -> bool {
        self.end_time.has_expired()
    }

    /// Check if this ticket is renewable and the renewal window is still open.
    pub fn is_renewable(&self) -> bool {
        if let Some(ref renew_till) = self.renew_till {
            !renew_till.has_expired()
        } else {
            false
        }
    }

    /// Check if this entry matches a server principal.
    pub fn matches_server(&self, server: &PrincipalName) -> bool {
        self.server_principal == *server
    }

    /// Remaining lifetime in seconds (0 if expired).
    pub fn remaining_secs(&self) -> u64 {
        let now = crate::arch::timer::get_timestamp_secs();
        self.end_time.timestamp.saturating_sub(now)
    }
}

// ---------------------------------------------------------------------------
// Ccache File Format
// ---------------------------------------------------------------------------

/// MIT krb5 ccache file structure.
///
/// Format (v4):
/// ```text
/// [2 bytes] version (0x0504)
/// [2 bytes] header length
/// [N bytes] header tags
/// [principal] default principal
/// [credentials...] repeated credential entries
/// ```
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CcacheFile {
    /// File format version
    pub version: u16,
    /// Default principal
    pub default_principal: PrincipalName,
    /// Cached credentials
    pub entries: Vec<CcacheEntry>,
}

#[cfg(feature = "alloc")]
impl CcacheFile {
    /// Create a new empty ccache file.
    pub fn new(default_principal: PrincipalName) -> Self {
        Self {
            version: CCACHE_VERSION,
            default_principal,
            entries: Vec::new(),
        }
    }

    /// Serialize to MIT krb5 ccache binary format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();

        // Version (big-endian u16)
        out.extend_from_slice(&self.version.to_be_bytes());

        // Header length (v4 has a header section)
        let header_len: u16 = 0; // no extra header tags
        out.extend_from_slice(&header_len.to_be_bytes());

        // Default principal
        Self::serialize_principal(&self.default_principal, &mut out);

        // Credentials
        for entry in &self.entries {
            self.serialize_credential(entry, &mut out);
        }

        out
    }

    /// Deserialize from MIT krb5 ccache binary format.
    pub fn deserialize(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 4 {
            return Err(KernelError::InvalidArgument {
                name: "ccache",
                value: "too short",
            });
        }

        let mut pos = 0;

        // Version
        let version = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        if version != CCACHE_VERSION {
            return Err(KernelError::InvalidArgument {
                name: "ccache_version",
                value: "unsupported version",
            });
        }

        // Header length
        let header_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        // Skip header tags
        if pos + header_len > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_header",
                value: "truncated",
            });
        }
        pos += header_len;

        // Default principal
        let (default_principal, consumed) = Self::deserialize_principal(data, pos)?;
        pos += consumed;

        // Credentials
        let mut entries = Vec::new();
        while pos < data.len() {
            match Self::deserialize_credential(data, pos) {
                Ok((entry, consumed)) => {
                    entries.push(entry);
                    pos += consumed;
                }
                Err(_) => break,
            }
        }

        Ok(Self {
            version,
            default_principal,
            entries,
        })
    }

    /// Serialize a principal name.
    fn serialize_principal(principal: &PrincipalName, out: &mut Vec<u8>) {
        // name_type (u32 big-endian)
        out.extend_from_slice(&(principal.name_type as u32).to_be_bytes());
        // num_components (u32 big-endian)
        out.extend_from_slice(&(principal.name_string.len() as u32).to_be_bytes());
        // Each component: [u32 length] [bytes]
        for component in &principal.name_string {
            out.extend_from_slice(&(component.len() as u32).to_be_bytes());
            out.extend_from_slice(component.as_bytes());
        }
    }

    /// Deserialize a principal name.
    fn deserialize_principal(
        data: &[u8],
        start: usize,
    ) -> Result<(PrincipalName, usize), KernelError> {
        let mut pos = start;

        if pos + 8 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_principal",
                value: "truncated",
            });
        }

        let name_type_val =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        let num_components =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let mut name_string = Vec::new();
        for _ in 0..num_components {
            if pos + 4 > data.len() {
                return Err(KernelError::InvalidArgument {
                    name: "ccache_component",
                    value: "truncated",
                });
            }
            let comp_len =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            pos += 4;

            if pos + comp_len > data.len() {
                return Err(KernelError::InvalidArgument {
                    name: "ccache_component_data",
                    value: "truncated",
                });
            }
            let s = core::str::from_utf8(&data[pos..pos + comp_len]).map_err(|_| {
                KernelError::InvalidArgument {
                    name: "ccache_component",
                    value: "invalid utf8",
                }
            })?;
            name_string.push(String::from(s));
            pos += comp_len;
        }

        let name_type = match name_type_val {
            1 => super::protocol::NameType::Principal,
            2 => super::protocol::NameType::SrvInst,
            3 => super::protocol::NameType::SrvHst,
            _ => super::protocol::NameType::Principal,
        };

        Ok((
            PrincipalName {
                name_type,
                name_string,
            },
            pos - start,
        ))
    }

    /// Serialize a credential entry.
    fn serialize_credential(&self, entry: &CcacheEntry, out: &mut Vec<u8>) {
        // Client principal
        Self::serialize_principal(&entry.client_principal, out);
        // Server principal
        Self::serialize_principal(&entry.server_principal, out);
        // Session key: [u16 etype] [u32 len] [bytes]
        out.extend_from_slice(&(entry.session_key_etype as u16).to_be_bytes());
        out.extend_from_slice(&(entry.session_key.len() as u32).to_be_bytes());
        out.extend_from_slice(&entry.session_key);
        // Times: auth_time, start_time, end_time, renew_till (each u32)
        out.extend_from_slice(&(entry.auth_time.timestamp as u32).to_be_bytes());
        out.extend_from_slice(&(entry.start_time.timestamp as u32).to_be_bytes());
        out.extend_from_slice(&(entry.end_time.timestamp as u32).to_be_bytes());
        let renew = entry.renew_till.map_or(0u32, |t| t.timestamp as u32);
        out.extend_from_slice(&renew.to_be_bytes());
        // Flags (u32)
        out.extend_from_slice(&entry.flags.to_be_bytes());
        // Ticket data: [u32 len] [bytes]
        out.extend_from_slice(&(entry.ticket_data.len() as u32).to_be_bytes());
        out.extend_from_slice(&entry.ticket_data);
    }

    /// Deserialize a credential entry.
    fn deserialize_credential(
        data: &[u8],
        start: usize,
    ) -> Result<(CcacheEntry, usize), KernelError> {
        let mut pos = start;

        // Client principal
        let (client_principal, consumed) = Self::deserialize_principal(data, pos)?;
        pos += consumed;

        // Server principal
        let (server_principal, consumed) = Self::deserialize_principal(data, pos)?;
        pos += consumed;

        // Session key
        if pos + 6 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated key",
            });
        }
        let etype_val = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;
        let key_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + key_len > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated key data",
            });
        }
        let session_key = data[pos..pos + key_len].to_vec();
        pos += key_len;

        // Times (4 x u32)
        if pos + 16 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated times",
            });
        }
        let auth_time =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        let start_time =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        let end_time = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;
        let renew_till_val =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // Flags
        if pos + 4 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated flags",
            });
        }
        let flags = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // Ticket data
        if pos + 4 > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated ticket len",
            });
        }
        let ticket_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + ticket_len > data.len() {
            return Err(KernelError::InvalidArgument {
                name: "ccache_cred",
                value: "truncated ticket data",
            });
        }
        let ticket_data = data[pos..pos + ticket_len].to_vec();
        pos += ticket_len;

        let session_key_etype =
            EncryptionType::from_i64(etype_val as i64).unwrap_or(EncryptionType::Aes256CtsHmacSha1);

        let renew_till = if renew_till_val > 0 {
            Some(KerberosTime::from_timestamp(renew_till_val as u64))
        } else {
            None
        };

        Ok((
            CcacheEntry {
                client_principal,
                server_principal,
                session_key,
                session_key_etype,
                auth_time: KerberosTime::from_timestamp(auth_time as u64),
                start_time: KerberosTime::from_timestamp(start_time as u64),
                end_time: KerberosTime::from_timestamp(end_time as u64),
                renew_till,
                ticket_data,
                flags,
            },
            pos - start,
        ))
    }
}

// ---------------------------------------------------------------------------
// Ticket Cache (in-memory)
// ---------------------------------------------------------------------------

/// In-memory Kerberos ticket cache.
///
/// Provides store/lookup/remove operations with automatic expiry purging.
#[cfg(feature = "alloc")]
pub struct TicketCache {
    /// Default principal (the authenticated user)
    default_principal: Option<PrincipalName>,
    /// Cached ticket entries
    entries: Vec<CcacheEntry>,
}

#[cfg(feature = "alloc")]
impl Default for TicketCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl TicketCache {
    /// Create a new empty ticket cache.
    pub fn new() -> Self {
        Self {
            default_principal: None,
            entries: Vec::new(),
        }
    }

    /// Set the default principal.
    pub fn set_default_principal(&mut self, principal: PrincipalName) {
        self.default_principal = Some(principal);
    }

    /// Get the default principal.
    pub fn default_principal(&self) -> Option<&PrincipalName> {
        self.default_principal.as_ref()
    }

    /// Store a ticket in the cache.
    ///
    /// If a ticket for the same server principal already exists, it is
    /// replaced.
    pub fn store_ticket(&mut self, entry: CcacheEntry) {
        // Replace existing entry for the same server principal
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.server_principal == entry.server_principal)
        {
            *existing = entry;
            return;
        }

        // Evict oldest if at capacity
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.entries.remove(0);
        }

        self.entries.push(entry);
    }

    /// Look up a ticket by server principal.
    pub fn lookup_ticket(&self, server: &PrincipalName) -> Option<&CcacheEntry> {
        self.entries
            .iter()
            .find(|e| e.matches_server(server) && !e.is_expired())
    }

    /// Remove a ticket by server principal.
    pub fn remove_ticket(&mut self, server: &PrincipalName) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|e| !e.matches_server(server));
        self.entries.len() < initial_len
    }

    /// Remove all expired tickets.
    pub fn purge_expired(&mut self) -> usize {
        let initial_len = self.entries.len();
        self.entries.retain(|e| !e.is_expired());
        initial_len - self.entries.len()
    }

    /// List all cached tickets with metadata.
    pub fn list_tickets(&self) -> &[CcacheEntry] {
        &self.entries
    }

    /// Get the number of cached tickets.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries (kdestroy).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.default_principal = None;
    }

    /// Export to ccache binary format.
    pub fn serialize(&self) -> Option<Vec<u8>> {
        let principal = self.default_principal.as_ref()?;
        let file = CcacheFile {
            version: CCACHE_VERSION,
            default_principal: principal.clone(),
            entries: self.entries.clone(),
        };
        Some(file.serialize())
    }

    /// Import from ccache binary format.
    pub fn deserialize(data: &[u8]) -> Result<Self, KernelError> {
        let file = CcacheFile::deserialize(data)?;
        Ok(Self {
            default_principal: Some(file.default_principal),
            entries: file.entries,
        })
    }
}

// ---------------------------------------------------------------------------
// Shell Command Helpers
// ---------------------------------------------------------------------------

/// Perform a kinit-like operation: derive key, request TGT, store in cache.
///
/// Returns the encoded AS-REQ bytes (caller must send to KDC and feed
/// the response back via `process_kinit_response`).
#[cfg(feature = "alloc")]
pub fn kinit_command(
    username: &str,
    realm: &str,
    password: &str,
    cache: &mut TicketCache,
) -> Vec<u8> {
    let mut client = KerberosClient::new(username, realm, password);
    let principal = PrincipalName::new_principal(username);
    cache.set_default_principal(principal);
    client.request_tgt()
}

/// Format cached tickets for display (klist).
#[cfg(feature = "alloc")]
pub fn klist_command(cache: &TicketCache) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(principal) = cache.default_principal() {
        let mut header = String::from("Default principal: ");
        header.push_str(&principal.to_text());
        lines.push(header);
    } else {
        lines.push(String::from("No default principal"));
    }

    lines.push(String::new());

    if cache.is_empty() {
        lines.push(String::from("No cached tickets"));
        return lines;
    }

    lines.push(String::from(
        "  Server                          Expires         Flags",
    ));
    lines.push(String::from(
        "  ------                          -------         -----",
    ));

    for entry in cache.list_tickets() {
        let server = entry.server_principal.to_text();
        let remaining = entry.remaining_secs();
        let expired_marker = if entry.is_expired() { " [EXPIRED]" } else { "" };

        let mut line = String::from("  ");
        line.push_str(&server);

        // Pad to alignment
        let pad = if server.len() < 32 {
            32 - server.len()
        } else {
            2
        };
        for _ in 0..pad {
            line.push(' ');
        }

        // Remaining time
        let hours = remaining / 3600;
        let minutes = (remaining % 3600) / 60;
        let mut time_str = String::new();
        // Manual integer-to-string formatting
        push_u64(&mut time_str, hours);
        time_str.push('h');
        push_u64(&mut time_str, minutes);
        time_str.push('m');
        line.push_str(&time_str);
        line.push_str(expired_marker);

        lines.push(line);
    }

    lines
}

/// Helper: push a u64 value as decimal digits to a String.
#[cfg(feature = "alloc")]
fn push_u64(s: &mut String, mut val: u64) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut count = 0;
    while val > 0 {
        digits[count] = (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    for i in (0..count).rev() {
        s.push((b'0' + digits[i]) as char);
    }
}

/// Destroy all cached tickets (kdestroy).
#[cfg(feature = "alloc")]
pub fn kdestroy_command(cache: &mut TicketCache) {
    cache.clear();
}

// ---------------------------------------------------------------------------
// Kerberos Auth Backend
// ---------------------------------------------------------------------------

/// Authentication backend that verifies credentials via Kerberos ticket
/// presence in the cache.
#[cfg(feature = "alloc")]
pub struct KerberosAuthBackend {
    /// Realm for this backend
    realm: String,
}

#[cfg(feature = "alloc")]
impl KerberosAuthBackend {
    /// Create a new Kerberos auth backend.
    pub fn new(realm: &str) -> Self {
        Self {
            realm: String::from(realm),
        }
    }

    /// Check if a user has a valid TGT in the cache.
    pub fn has_valid_ticket(&self, cache: &TicketCache, username: &str) -> bool {
        let krbtgt = PrincipalName::krbtgt(&self.realm);
        if let Some(entry) = cache.lookup_ticket(&krbtgt) {
            // Verify the client principal matches
            if entry
                .client_principal
                .name_string
                .first()
                .map(|s| s.as_str())
                == Some(username)
            {
                return !entry.is_expired();
            }
        }
        false
    }

    /// Get the realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_test_entry(server: &str, end_secs: u64) -> CcacheEntry {
        CcacheEntry {
            client_principal: PrincipalName::new_principal("testuser"),
            server_principal: PrincipalName::new_service("krbtgt", server),
            session_key: vec![0x42; 32],
            session_key_etype: EncryptionType::Aes256CtsHmacSha1,
            auth_time: KerberosTime::from_timestamp(1000),
            start_time: KerberosTime::from_timestamp(1000),
            end_time: KerberosTime::from_timestamp(end_secs),
            renew_till: None,
            ticket_data: vec![0xDE, 0xAD],
            flags: 0x4000_0000,
        }
    }

    #[test]
    fn test_ticket_cache_store_lookup() {
        let mut cache = TicketCache::new();
        let entry = make_test_entry("EXAMPLE.COM", u64::MAX);
        cache.store_ticket(entry);

        let server = PrincipalName::new_service("krbtgt", "EXAMPLE.COM");
        let found = cache.lookup_ticket(&server);
        assert!(found.is_some());
        assert_eq!(found.unwrap().session_key.len(), 32);
    }

    #[test]
    fn test_ticket_cache_remove() {
        let mut cache = TicketCache::new();
        cache.store_ticket(make_test_entry("EXAMPLE.COM", u64::MAX));

        let server = PrincipalName::new_service("krbtgt", "EXAMPLE.COM");
        assert!(cache.remove_ticket(&server));
        assert!(cache.is_empty());
    }

    #[test]
    fn test_ticket_cache_replace_existing() {
        let mut cache = TicketCache::new();
        // Use far-future timestamps to avoid expiration in test
        cache.store_ticket(make_test_entry("EXAMPLE.COM", u64::MAX / 2));
        cache.store_ticket(make_test_entry("EXAMPLE.COM", u64::MAX / 2 + 1));

        assert_eq!(cache.len(), 1);
        let server = PrincipalName::new_service("krbtgt", "EXAMPLE.COM");
        let found = cache.lookup_ticket(&server).unwrap();
        assert_eq!(found.end_time.timestamp, u64::MAX / 2 + 1);
    }

    #[test]
    fn test_ticket_cache_clear() {
        let mut cache = TicketCache::new();
        cache.set_default_principal(PrincipalName::new_principal("alice"));
        cache.store_ticket(make_test_entry("EXAMPLE.COM", u64::MAX));
        cache.store_ticket(make_test_entry("OTHER.COM", u64::MAX));

        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.default_principal().is_none());
    }

    #[test]
    fn test_ccache_serialize_deserialize() {
        let principal = PrincipalName::new_principal("alice");
        let mut file = CcacheFile::new(principal.clone());
        file.entries.push(make_test_entry("EXAMPLE.COM", 5000));

        let serialized = file.serialize();
        assert!(!serialized.is_empty());

        let deserialized = CcacheFile::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.version, CCACHE_VERSION);
        assert_eq!(deserialized.default_principal.name_string[0], "alice");
        assert_eq!(deserialized.entries.len(), 1);
    }

    #[test]
    fn test_ccache_roundtrip_multiple_entries() {
        let principal = PrincipalName::new_principal("bob");
        let mut file = CcacheFile::new(principal);
        file.entries.push(make_test_entry("REALM1.COM", 5000));
        file.entries.push(make_test_entry("REALM2.COM", 6000));

        let serialized = file.serialize();
        let deserialized = CcacheFile::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.entries.len(), 2);
        assert_eq!(
            deserialized.entries[0].server_principal.name_string[1],
            "REALM1.COM"
        );
        assert_eq!(
            deserialized.entries[1].server_principal.name_string[1],
            "REALM2.COM"
        );
    }

    #[test]
    fn test_kinit_produces_bytes() {
        let mut cache = TicketCache::new();
        let req = kinit_command("alice", "EXAMPLE.COM", "password", &mut cache);
        assert!(!req.is_empty());
        assert!(cache.default_principal().is_some());
    }

    #[test]
    fn test_klist_empty_cache() {
        let cache = TicketCache::new();
        let lines = klist_command(&cache);
        assert!(lines.iter().any(|l| l.contains("No cached tickets")));
    }

    #[test]
    fn test_kdestroy() {
        let mut cache = TicketCache::new();
        cache.store_ticket(make_test_entry("EXAMPLE.COM", u64::MAX));
        kdestroy_command(&mut cache);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_auth_backend() {
        let backend = KerberosAuthBackend::new("EXAMPLE.COM");
        assert_eq!(backend.realm(), "EXAMPLE.COM");

        let cache = TicketCache::new();
        assert!(!backend.has_valid_ticket(&cache, "alice"));
    }
}
