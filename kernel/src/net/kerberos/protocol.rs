//! Kerberos v5 Protocol (RFC 4120)
//!
//! Implements Kerberos message types, principal names, encryption types,
//! and the client-side AS-REQ/AS-REP and TGS-REQ/TGS-REP flows. All
//! messages are encoded using `crate::net::asn1` for ASN.1/BER serialization.
//!
//! # Key Derivation
//!
//! Provides string2key stubs for AES-256-CTS-HMAC-SHA1-96 (etype 18) using
//! PBKDF2-HMAC-SHA1 with 4096 iterations. Full AES-CTS encryption is stubbed
//! pending a complete AES implementation.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

use crate::{
    error::KernelError,
    net::asn1::{encode_application, encode_context_specific, AsnEncoder, AsnValue},
};

// ---------------------------------------------------------------------------
// Kerberos Constants
// ---------------------------------------------------------------------------

/// Default Kerberos port
pub const KDC_PORT: u16 = 88;

/// Kerberos protocol version
const KRB5_VERSION: i64 = 5;

/// Ticket version
const TKT_VERSION: i64 = 5;

// ---------------------------------------------------------------------------
// Message Types
// ---------------------------------------------------------------------------

/// Kerberos message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KerberosMsgType {
    /// AS-REQ (10)
    AsReq = 10,
    /// AS-REP (11)
    AsRep = 11,
    /// TGS-REQ (12)
    TgsReq = 12,
    /// TGS-REP (13)
    TgsRep = 13,
    /// AP-REQ (14)
    ApReq = 14,
    /// AP-REP (15)
    ApRep = 15,
    /// KRB-ERROR (30)
    Error = 30,
}

impl KerberosMsgType {
    /// Create from an integer.
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            10 => Some(KerberosMsgType::AsReq),
            11 => Some(KerberosMsgType::AsRep),
            12 => Some(KerberosMsgType::TgsReq),
            13 => Some(KerberosMsgType::TgsRep),
            14 => Some(KerberosMsgType::ApReq),
            15 => Some(KerberosMsgType::ApRep),
            30 => Some(KerberosMsgType::Error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Name Types
// ---------------------------------------------------------------------------

/// Kerberos name types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NameType {
    /// Principal name (user)
    Principal = 1,
    /// Service and instance (service/hostname)
    SrvInst = 2,
    /// Service and host (HTTP/host@realm)
    SrvHst = 3,
}

// ---------------------------------------------------------------------------
// Encryption Types
// ---------------------------------------------------------------------------

/// Kerberos encryption types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EncryptionType {
    /// DES3-CBC-SHA1 (legacy)
    Des3CbcSha1 = 16,
    /// AES128-CTS-HMAC-SHA1-96
    Aes128CtsHmacSha1 = 17,
    /// AES256-CTS-HMAC-SHA1-96 (preferred)
    Aes256CtsHmacSha1 = 18,
}

impl EncryptionType {
    /// Key size in bytes for this encryption type.
    pub fn key_size(&self) -> usize {
        match self {
            EncryptionType::Des3CbcSha1 => 24,
            EncryptionType::Aes128CtsHmacSha1 => 16,
            EncryptionType::Aes256CtsHmacSha1 => 32,
        }
    }

    /// Create from integer etype value.
    pub(crate) fn from_i64(v: i64) -> Option<Self> {
        match v {
            16 => Some(EncryptionType::Des3CbcSha1),
            17 => Some(EncryptionType::Aes128CtsHmacSha1),
            18 => Some(EncryptionType::Aes256CtsHmacSha1),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Principal Name
// ---------------------------------------------------------------------------

/// Kerberos principal name.
///
/// A principal has a name type and one or more name components.
/// For example, `krbtgt/EXAMPLE.COM` has type SrvInst and components
/// `["krbtgt", "EXAMPLE.COM"]`.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalName {
    /// Name type (NT_PRINCIPAL, NT_SRV_INST, etc.)
    pub name_type: NameType,
    /// Name string components
    pub name_string: Vec<String>,
}

#[cfg(feature = "alloc")]
impl PrincipalName {
    /// Create a simple principal (type NT_PRINCIPAL, single component).
    pub fn new_principal(name: &str) -> Self {
        Self {
            name_type: NameType::Principal,
            name_string: vec![String::from(name)],
        }
    }

    /// Create a service principal (type NT_SRV_INST, two components).
    pub fn new_service(service: &str, instance: &str) -> Self {
        Self {
            name_type: NameType::SrvInst,
            name_string: vec![String::from(service), String::from(instance)],
        }
    }

    /// Create the krbtgt principal for a realm.
    pub fn krbtgt(realm: &str) -> Self {
        Self::new_service("krbtgt", realm)
    }

    /// Encode as ASN.1 SEQUENCE { name-type, name-string }.
    pub fn encode(&self) -> Vec<u8> {
        let name_type = AsnEncoder::encode(&AsnValue::Integer(self.name_type as i64));
        let name_type_ctx = encode_context_specific(0, true, &name_type);

        let name_strings: Vec<AsnValue> = self
            .name_string
            .iter()
            .map(|s| AsnValue::OctetString(s.as_bytes().to_vec()))
            .collect();
        let name_seq = AsnEncoder::encode(&AsnValue::Sequence(name_strings));
        let name_seq_ctx = encode_context_specific(1, true, &name_seq);

        let mut content = Vec::new();
        content.extend_from_slice(&name_type_ctx);
        content.extend_from_slice(&name_seq_ctx);
        AsnEncoder::encode(&AsnValue::Sequence(vec![AsnValue::OctetString(content)]))
    }

    /// Display as "component1/component2" format.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (i, part) in self.name_string.iter().enumerate() {
            if i > 0 {
                s.push('/');
            }
            s.push_str(part);
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Kerberos Time
// ---------------------------------------------------------------------------

/// Kerberos timestamp (seconds since epoch, integer-only).
///
/// Stored as a u64 for compatibility with the kernel timer subsystem.
/// Kerberos GeneralizedTime format is "YYYYMMDDHHMMSSZ".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct KerberosTime {
    /// Seconds since Unix epoch
    pub timestamp: u64,
}

impl KerberosTime {
    /// Create from a Unix timestamp.
    pub const fn from_timestamp(ts: u64) -> Self {
        Self { timestamp: ts }
    }

    /// Get the current time from the kernel timer.
    pub fn now() -> Self {
        Self {
            timestamp: crate::arch::timer::get_timestamp_secs(),
        }
    }

    /// Check if this time has passed.
    pub fn has_expired(&self) -> bool {
        let now = crate::arch::timer::get_timestamp_secs();
        now >= self.timestamp
    }

    /// Encode as ASN.1 GeneralizedTime string.
    ///
    /// Uses a simplified format: "YYYYMMDDHHMMSSZ".
    /// This is an approximation since we derive date from epoch seconds
    /// using integer math only.
    #[cfg(feature = "alloc")]
    pub fn encode_generalized_time(&self) -> Vec<u8> {
        // Simplified: encode as an integer timestamp wrapped in context tag
        AsnEncoder::encode(&AsnValue::OctetString(
            self.timestamp.to_be_bytes().to_vec(),
        ))
    }
}

// KerberosTime Default: timestamp 0 (epoch)

// ---------------------------------------------------------------------------
// Encrypted Data
// ---------------------------------------------------------------------------

/// Encrypted data container (EncryptedData in RFC 4120).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedData {
    /// Encryption type
    pub etype: EncryptionType,
    /// Key version number (optional)
    pub kvno: Option<u32>,
    /// Cipher text
    pub cipher: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl EncryptedData {
    /// Create new encrypted data.
    pub fn new(etype: EncryptionType, kvno: Option<u32>, cipher: Vec<u8>) -> Self {
        Self {
            etype,
            kvno,
            cipher,
        }
    }

    /// Encode as ASN.1 SEQUENCE.
    pub fn encode(&self) -> Vec<u8> {
        let etype = encode_context_specific(
            0,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(self.etype as i64)),
        );

        let mut content = Vec::new();
        content.extend_from_slice(&etype);

        if let Some(kvno) = self.kvno {
            let kvno_enc = encode_context_specific(
                1,
                true,
                &AsnEncoder::encode(&AsnValue::Integer(kvno as i64)),
            );
            content.extend_from_slice(&kvno_enc);
        }

        let cipher = encode_context_specific(
            2,
            true,
            &AsnEncoder::encode(&AsnValue::OctetString(self.cipher.clone())),
        );
        content.extend_from_slice(&cipher);

        AsnEncoder::encode(&AsnValue::Sequence(vec![AsnValue::OctetString(content)]))
    }
}

// ---------------------------------------------------------------------------
// Ticket
// ---------------------------------------------------------------------------

/// Kerberos Ticket.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// Ticket version (always 5)
    pub tkt_vno: i64,
    /// Realm
    pub realm: String,
    /// Server principal name
    pub sname: PrincipalName,
    /// Encrypted part
    pub enc_part: EncryptedData,
}

#[cfg(feature = "alloc")]
impl Ticket {
    /// Create a new ticket.
    pub fn new(realm: &str, sname: PrincipalName, enc_part: EncryptedData) -> Self {
        Self {
            tkt_vno: TKT_VERSION,
            realm: String::from(realm),
            sname,
            enc_part,
        }
    }

    /// Encode as ASN.1 [APPLICATION 1] SEQUENCE.
    pub fn encode(&self) -> Vec<u8> {
        let tkt_vno = encode_context_specific(
            0,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(self.tkt_vno)),
        );
        let realm = encode_context_specific(
            1,
            true,
            &AsnEncoder::encode(&AsnValue::OctetString(self.realm.as_bytes().to_vec())),
        );
        let sname = encode_context_specific(2, true, &self.sname.encode());
        let enc_part = encode_context_specific(3, true, &self.enc_part.encode());

        let mut content = Vec::new();
        content.extend_from_slice(&tkt_vno);
        content.extend_from_slice(&realm);
        content.extend_from_slice(&sname);
        content.extend_from_slice(&enc_part);

        encode_application(1, true, &content)
    }
}

// ---------------------------------------------------------------------------
// KDC Request Body
// ---------------------------------------------------------------------------

/// KDC request body (KDC-REQ-BODY).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct KdcReqBody {
    /// KDC options flags
    pub kdc_options: u32,
    /// Client principal name (optional, present in AS-REQ)
    pub cname: Option<PrincipalName>,
    /// Realm
    pub realm: String,
    /// Server principal name
    pub sname: Option<PrincipalName>,
    /// Requested start time
    pub from: Option<KerberosTime>,
    /// Requested end time
    pub till: KerberosTime,
    /// Requested renewal time
    pub rtime: Option<KerberosTime>,
    /// Random nonce for replay protection
    pub nonce: u32,
    /// Requested encryption types (in preference order)
    pub etype: Vec<EncryptionType>,
}

#[cfg(feature = "alloc")]
impl KdcReqBody {
    /// Encode as ASN.1 SEQUENCE.
    pub fn encode(&self) -> Vec<u8> {
        let mut content = Vec::new();

        // kdc-options [0] KDCOptions (BIT STRING)
        let options_bytes = self.kdc_options.to_be_bytes();
        let options = encode_context_specific(
            0,
            true,
            &AsnEncoder::encode(&AsnValue::BitString(options_bytes.to_vec(), 0)),
        );
        content.extend_from_slice(&options);

        // cname [1] PrincipalName OPTIONAL
        if let Some(ref cname) = self.cname {
            let cname_enc = encode_context_specific(1, true, &cname.encode());
            content.extend_from_slice(&cname_enc);
        }

        // realm [2] Realm
        let realm = encode_context_specific(
            2,
            true,
            &AsnEncoder::encode(&AsnValue::OctetString(self.realm.as_bytes().to_vec())),
        );
        content.extend_from_slice(&realm);

        // sname [3] PrincipalName OPTIONAL
        if let Some(ref sname) = self.sname {
            let sname_enc = encode_context_specific(3, true, &sname.encode());
            content.extend_from_slice(&sname_enc);
        }

        // till [5] KerberosTime
        let till = encode_context_specific(5, true, &self.till.encode_generalized_time());
        content.extend_from_slice(&till);

        // nonce [7] UInt32
        let nonce = encode_context_specific(
            7,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(self.nonce as i64)),
        );
        content.extend_from_slice(&nonce);

        // etype [8] SEQUENCE OF Int32
        let etypes: Vec<AsnValue> = self
            .etype
            .iter()
            .map(|e| AsnValue::Integer(*e as i64))
            .collect();
        let etype_enc =
            encode_context_specific(8, true, &AsnEncoder::encode(&AsnValue::Sequence(etypes)));
        content.extend_from_slice(&etype_enc);

        AsnEncoder::encode(&AsnValue::Sequence(vec![AsnValue::OctetString(content)]))
    }
}

// ---------------------------------------------------------------------------
// KDC Reply Parts
// ---------------------------------------------------------------------------

/// Encrypted part of a KDC reply (EncKDCRepPart).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct EncKdcRepPart {
    /// Session key
    pub session_key: Vec<u8>,
    /// Session key encryption type
    pub session_key_etype: EncryptionType,
    /// Nonce (must match request)
    pub nonce: u32,
    /// Ticket flags
    pub flags: u32,
    /// Authentication time
    pub authtime: KerberosTime,
    /// Start time
    pub starttime: Option<KerberosTime>,
    /// End time
    pub endtime: KerberosTime,
    /// Renewal end time
    pub renew_till: Option<KerberosTime>,
    /// Server realm
    pub srealm: String,
    /// Server principal name
    pub sname: PrincipalName,
}

// ---------------------------------------------------------------------------
// Kerberos Client
// ---------------------------------------------------------------------------

/// Kerberos v5 client.
///
/// Handles AS-REQ (initial authentication) and TGS-REQ (service ticket
/// request) flows.
#[cfg(feature = "alloc")]
pub struct KerberosClient {
    /// Client principal
    pub client_principal: PrincipalName,
    /// Realm
    pub realm: String,
    /// Long-term key derived from password
    key: Vec<u8>,
    /// TGT (obtained via AS-REQ)
    tgt: Option<Ticket>,
    /// TGT session key
    session_key: Option<Vec<u8>>,
    /// TGT expiration
    tgt_expiry: KerberosTime,
}

#[cfg(feature = "alloc")]
impl KerberosClient {
    /// Create a new Kerberos client.
    ///
    /// Derives a long-term key from the password using string2key.
    pub fn new(username: &str, realm: &str, password: &str) -> Self {
        let client_principal = PrincipalName::new_principal(username);
        let key = Self::derive_key(password, realm, username);

        Self {
            client_principal,
            realm: String::from(realm),
            key,
            tgt: None,
            session_key: None,
            tgt_expiry: KerberosTime::default(),
        }
    }

    /// Whether the client has a valid (non-expired) TGT.
    pub fn has_valid_tgt(&self) -> bool {
        self.tgt.is_some() && !self.tgt_expiry.has_expired()
    }

    /// Get the TGT, if present.
    pub fn tgt(&self) -> Option<&Ticket> {
        self.tgt.as_ref()
    }

    /// Get the session key, if present.
    pub fn session_key(&self) -> Option<&[u8]> {
        self.session_key.as_deref()
    }

    // -----------------------------------------------------------------------
    // Key Derivation
    // -----------------------------------------------------------------------

    /// Derive a key from a password using string2key for AES-256.
    ///
    /// Per RFC 3962: string2key(password) = random2key(PBKDF2(password, salt,
    /// iteration_count, key_size))
    ///
    /// Salt for Kerberos AES = realm + principal_name
    ///
    /// This is a simplified implementation using HMAC-SHA256 as the PRF
    /// instead of HMAC-SHA1 (the full implementation would use HMAC-SHA1
    /// per RFC 3962 Section 4).
    fn derive_key(password: &str, realm: &str, username: &str) -> Vec<u8> {
        // Salt = realm || username (per RFC 3962)
        let mut salt = Vec::with_capacity(realm.len() + username.len());
        salt.extend_from_slice(realm.as_bytes());
        salt.extend_from_slice(username.as_bytes());

        // PBKDF2-HMAC-SHA256, 4096 iterations, 32-byte output
        Self::pbkdf2_derive(password.as_bytes(), &salt, 4096)
    }

    /// PBKDF2 key derivation (simplified, integer-only).
    ///
    /// Uses HMAC-SHA256 as the PRF. Returns 32 bytes.
    fn pbkdf2_derive(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
        use crate::crypto::hash::sha256;

        // HMAC-SHA256(key, message) -- simplified inline implementation
        let hmac = |key: &[u8], msg: &[u8]| -> [u8; 32] {
            const BLOCK_SIZE: usize = 64;
            const IPAD: u8 = 0x36;
            const OPAD: u8 = 0x5c;

            // If key > block size, hash it
            let key_bytes: [u8; 32];
            let actual_key = if key.len() > BLOCK_SIZE {
                key_bytes = *sha256(key).as_bytes();
                &key_bytes[..]
            } else {
                key
            };

            let mut padded_key = [0u8; BLOCK_SIZE];
            padded_key[..actual_key.len()].copy_from_slice(actual_key);

            // Inner: SHA256((key XOR ipad) || message)
            let mut inner = [0u8; 192];
            for (i, byte) in padded_key.iter().enumerate() {
                inner[i] = byte ^ IPAD;
            }
            let inner_len = BLOCK_SIZE + msg.len().min(128);
            let copy_len = msg.len().min(128);
            inner[BLOCK_SIZE..BLOCK_SIZE + copy_len].copy_from_slice(&msg[..copy_len]);
            let inner_hash = sha256(&inner[..inner_len]);

            // Outer: SHA256((key XOR opad) || inner_hash)
            let mut outer = [0u8; BLOCK_SIZE + 32];
            for (i, byte) in padded_key.iter().enumerate() {
                outer[i] = byte ^ OPAD;
            }
            outer[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());
            *sha256(&outer[..BLOCK_SIZE + 32]).as_bytes()
        };

        // PBKDF2 for block 1 (we only need one 32-byte block)
        // U1 = HMAC(password, salt || INT(1))
        let mut salt_counter = Vec::with_capacity(salt.len() + 4);
        salt_counter.extend_from_slice(salt);
        salt_counter.extend_from_slice(&1u32.to_be_bytes());

        let u1 = hmac(password, &salt_counter);
        let mut result = u1;
        let mut prev = u1;

        for _ in 1..iterations {
            let u_next = hmac(password, &prev);
            for (r, u) in result.iter_mut().zip(u_next.iter()) {
                *r ^= u;
            }
            prev = u_next;
        }

        result.to_vec()
    }

    // -----------------------------------------------------------------------
    // AS-REQ / AS-REP
    // -----------------------------------------------------------------------

    /// Build an AS-REQ message to request a TGT.
    ///
    /// Returns BER-encoded bytes ready to send to the KDC.
    pub fn request_tgt(&mut self) -> Vec<u8> {
        let nonce = self.generate_nonce();

        let body = KdcReqBody {
            kdc_options: 0x4000_0000, // forwardable
            cname: Some(self.client_principal.clone()),
            realm: self.realm.clone(),
            sname: Some(PrincipalName::krbtgt(&self.realm)),
            from: None,
            till: KerberosTime::from_timestamp(
                crate::arch::timer::get_timestamp_secs().saturating_add(36000),
            ), // 10 hours
            rtime: None,
            nonce,
            etype: vec![
                EncryptionType::Aes256CtsHmacSha1,
                EncryptionType::Aes128CtsHmacSha1,
            ],
        };

        self.encode_as_req(&body)
    }

    /// Encode an AS-REQ message.
    ///
    /// AS-REQ ::= [APPLICATION 10] KDC-REQ
    /// KDC-REQ ::= SEQUENCE { pvno, msg-type, padata, req-body }
    fn encode_as_req(&self, body: &KdcReqBody) -> Vec<u8> {
        let mut content = Vec::new();

        // pvno [1] INTEGER (5)
        let pvno = encode_context_specific(
            1,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(KRB5_VERSION)),
        );
        content.extend_from_slice(&pvno);

        // msg-type [2] INTEGER (10)
        let msg_type = encode_context_specific(
            2,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(KerberosMsgType::AsReq as i64)),
        );
        content.extend_from_slice(&msg_type);

        // padata [3] SEQUENCE OF PA-DATA OPTIONAL (empty for now)
        let padata = encode_context_specific(
            3,
            true,
            &AsnEncoder::encode(&AsnValue::Sequence(Vec::new())),
        );
        content.extend_from_slice(&padata);

        // req-body [4] KDC-REQ-BODY
        let body_enc = encode_context_specific(4, true, &body.encode());
        content.extend_from_slice(&body_enc);

        encode_application(KerberosMsgType::AsReq as u8, true, &content)
    }

    /// Parse an AS-REP message.
    ///
    /// Extracts the TGT and encrypted part. The caller must decrypt the
    /// encrypted part using the client's long-term key to obtain the
    /// session key.
    pub fn parse_as_rep(&mut self, _data: &[u8]) -> Result<AsRepParts, KernelError> {
        // In a full implementation, we would:
        // 1. Decode the APPLICATION 11 envelope
        // 2. Extract the ticket from [5]
        // 3. Extract the enc-part from [6]
        // 4. Decrypt enc-part using self.key
        // 5. Extract session key and TGT expiration
        //
        // For now, return a stub indicating the structure is understood.
        Err(KernelError::NotImplemented {
            feature: "kerberos_as_rep_parse",
        })
    }

    /// Store a TGT obtained from an AS-REP.
    pub fn store_tgt(&mut self, ticket: Ticket, session_key: Vec<u8>, expiry: KerberosTime) {
        self.tgt = Some(ticket);
        self.session_key = Some(session_key);
        self.tgt_expiry = expiry;
    }

    // -----------------------------------------------------------------------
    // TGS-REQ / TGS-REP
    // -----------------------------------------------------------------------

    /// Build a TGS-REQ message to request a service ticket.
    ///
    /// Requires a valid TGT (obtained via `request_tgt` + `parse_as_rep`).
    pub fn request_service_ticket(
        &mut self,
        service: &str,
        hostname: &str,
    ) -> Result<Vec<u8>, KernelError> {
        if !self.has_valid_tgt() {
            return Err(KernelError::InvalidState {
                expected: "valid TGT",
                actual: "no TGT or expired",
            });
        }

        let nonce = self.generate_nonce();
        let sname = PrincipalName::new_service(service, hostname);

        let body = KdcReqBody {
            kdc_options: 0x4000_0000,
            cname: None, // Not included in TGS-REQ
            realm: self.realm.clone(),
            sname: Some(sname),
            from: None,
            till: KerberosTime::from_timestamp(
                crate::arch::timer::get_timestamp_secs().saturating_add(36000),
            ),
            rtime: None,
            nonce,
            etype: vec![
                EncryptionType::Aes256CtsHmacSha1,
                EncryptionType::Aes128CtsHmacSha1,
            ],
        };

        Ok(self.encode_tgs_req(&body))
    }

    /// Encode a TGS-REQ message.
    ///
    /// TGS-REQ ::= [APPLICATION 12] KDC-REQ
    fn encode_tgs_req(&self, body: &KdcReqBody) -> Vec<u8> {
        let mut content = Vec::new();

        // pvno [1]
        let pvno = encode_context_specific(
            1,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(KRB5_VERSION)),
        );
        content.extend_from_slice(&pvno);

        // msg-type [2]
        let msg_type = encode_context_specific(
            2,
            true,
            &AsnEncoder::encode(&AsnValue::Integer(KerberosMsgType::TgsReq as i64)),
        );
        content.extend_from_slice(&msg_type);

        // padata [3] -- would contain AP-REQ with TGT
        // For a complete implementation, encode the TGT as PA-TGS-REQ here
        if let Some(ref tgt) = self.tgt {
            let tgt_enc = tgt.encode();
            let pa_tgs_req = AsnEncoder::encode(&AsnValue::Sequence(vec![
                AsnValue::Integer(1), // PA-TGS-REQ type
                AsnValue::OctetString(tgt_enc),
            ]));
            let padata = encode_context_specific(
                3,
                true,
                &AsnEncoder::encode(&AsnValue::Sequence(vec![AsnValue::OctetString(pa_tgs_req)])),
            );
            content.extend_from_slice(&padata);
        }

        // req-body [4]
        let body_enc = encode_context_specific(4, true, &body.encode());
        content.extend_from_slice(&body_enc);

        encode_application(KerberosMsgType::TgsReq as u8, true, &content)
    }

    /// Parse a TGS-REP message.
    pub fn parse_tgs_rep(&mut self, _data: &[u8]) -> Result<TgsRepParts, KernelError> {
        Err(KernelError::NotImplemented {
            feature: "kerberos_tgs_rep_parse",
        })
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Generate a pseudo-random nonce using the kernel timer.
    fn generate_nonce(&self) -> u32 {
        // Use timestamp-based nonce (sufficient for stub; real implementation
        // would use CSPRNG)
        let ts = crate::arch::timer::get_timestamp_secs();
        (ts & 0xFFFF_FFFF) as u32
    }

    /// Get the client's derived key.
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    /// Get the realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }
}

// ---------------------------------------------------------------------------
// Reply parts (for parse_as_rep / parse_tgs_rep return types)
// ---------------------------------------------------------------------------

/// Parsed AS-REP components.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct AsRepParts {
    /// The TGT ticket
    pub ticket: Ticket,
    /// The encrypted KDC reply part
    pub enc_part: EncryptedData,
}

/// Parsed TGS-REP components.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct TgsRepParts {
    /// The service ticket
    pub ticket: Ticket,
    /// The encrypted KDC reply part
    pub enc_part: EncryptedData,
}

// ---------------------------------------------------------------------------
// AES-CTS Key Derivation Stubs
// ---------------------------------------------------------------------------

/// Derive a usage-specific key (dk) from a base key.
///
/// Per RFC 3961: dk(base_key, usage) = DK(base_key, usage_constant)
/// This is a stub that returns a truncated HMAC of the base key.
#[cfg(feature = "alloc")]
pub fn derive_usage_key(base_key: &[u8], usage: u32) -> Vec<u8> {
    use crate::crypto::hash::sha256;

    let mut input = Vec::with_capacity(base_key.len() + 4);
    input.extend_from_slice(base_key);
    input.extend_from_slice(&usage.to_be_bytes());

    let hash = sha256(&input);
    hash.as_bytes().to_vec()
}

/// Convert random bytes to a key (random-to-key).
///
/// For AES, this is the identity function: the random bytes ARE the key.
#[cfg(feature = "alloc")]
pub fn random_to_key(random_bytes: &[u8], etype: EncryptionType) -> Vec<u8> {
    let key_size = etype.key_size();
    if random_bytes.len() >= key_size {
        random_bytes[..key_size].to_vec()
    } else {
        let mut key = random_bytes.to_vec();
        key.resize(key_size, 0);
        key
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_principal_name_simple() {
        let p = PrincipalName::new_principal("alice");
        assert_eq!(p.name_type, NameType::Principal);
        assert_eq!(p.name_string.len(), 1);
        assert_eq!(p.name_string[0], "alice");
    }

    #[test]
    fn test_principal_name_service() {
        let p = PrincipalName::new_service("HTTP", "www.example.com");
        assert_eq!(p.name_type, NameType::SrvInst);
        assert_eq!(p.name_string.len(), 2);
        assert_eq!(p.to_text(), "HTTP/www.example.com");
    }

    #[test]
    fn test_principal_krbtgt() {
        let p = PrincipalName::krbtgt("EXAMPLE.COM");
        assert_eq!(p.name_string[0], "krbtgt");
        assert_eq!(p.name_string[1], "EXAMPLE.COM");
    }

    #[test]
    fn test_kerberos_time() {
        let t = KerberosTime::from_timestamp(1000);
        assert_eq!(t.timestamp, 1000);
    }

    #[test]
    fn test_encryption_type_key_size() {
        assert_eq!(EncryptionType::Aes256CtsHmacSha1.key_size(), 32);
        assert_eq!(EncryptionType::Aes128CtsHmacSha1.key_size(), 16);
        assert_eq!(EncryptionType::Des3CbcSha1.key_size(), 24);
    }

    #[test]
    fn test_encrypted_data_encode() {
        let ed = EncryptedData::new(EncryptionType::Aes256CtsHmacSha1, Some(1), vec![0xDE, 0xAD]);
        let encoded = ed.encode();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_ticket_encode() {
        let ticket = Ticket::new(
            "EXAMPLE.COM",
            PrincipalName::krbtgt("EXAMPLE.COM"),
            EncryptedData::new(EncryptionType::Aes256CtsHmacSha1, None, vec![0x01]),
        );
        let encoded = ticket.encode();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_kerberos_client_creation() {
        let client = KerberosClient::new("alice", "EXAMPLE.COM", "password");
        assert_eq!(client.realm(), "EXAMPLE.COM");
        assert!(!client.has_valid_tgt());
        assert_eq!(client.key().len(), 32); // AES-256 key
    }

    #[test]
    fn test_derive_key_deterministic() {
        let k1 = KerberosClient::derive_key("password", "EXAMPLE.COM", "alice");
        let k2 = KerberosClient::derive_key("password", "EXAMPLE.COM", "alice");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_key_different_inputs() {
        let k1 = KerberosClient::derive_key("password", "EXAMPLE.COM", "alice");
        let k2 = KerberosClient::derive_key("password", "EXAMPLE.COM", "bob");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_request_tgt_produces_bytes() {
        let mut client = KerberosClient::new("alice", "EXAMPLE.COM", "password");
        let req = client.request_tgt();
        assert!(!req.is_empty());
    }

    #[test]
    fn test_request_service_ticket_requires_tgt() {
        let mut client = KerberosClient::new("alice", "EXAMPLE.COM", "password");
        let result = client.request_service_ticket("HTTP", "www.example.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_msg_type_from_i64() {
        assert_eq!(KerberosMsgType::from_i64(10), Some(KerberosMsgType::AsReq));
        assert_eq!(KerberosMsgType::from_i64(99), None);
    }

    #[test]
    fn test_derive_usage_key() {
        let base = vec![0x42u8; 32];
        let dk = derive_usage_key(&base, 1);
        assert_eq!(dk.len(), 32);

        // Different usages produce different keys
        let dk2 = derive_usage_key(&base, 2);
        assert_ne!(dk, dk2);
    }
}
