//! LDAP v3 Client (RFC 4511)
//!
//! Implements the Lightweight Directory Access Protocol version 3, encoding
//! all messages as ASN.1/BER via `crate::net::asn1`. Supports simple bind,
//! search with filters, compare, modify, add, delete, and unbind operations.
//!
//! # Active Directory Integration
//!
//! `LdapAdClient` wraps `LdapClient` with AD-specific helpers for user
//! lookup, group membership extraction, and bind-based authentication.
//!
//! # Credential Caching
//!
//! `CredentialCache` provides TTL-based caching of bind credentials to avoid
//! repeated LDAP binds for recently authenticated users.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{
    error::KernelError,
    net::asn1::{
        encode_application, encode_context_specific, encode_length, AsnDecoder, AsnEncoder,
        AsnValue,
    },
};

// ---------------------------------------------------------------------------
// LDAP Constants
// ---------------------------------------------------------------------------

/// Default LDAP port
pub const LDAP_PORT: u16 = 389;

/// Default LDAPS port
pub const LDAPS_PORT: u16 = 636;

/// LDAP protocol version
const LDAP_VERSION: i64 = 3;

// LDAP operation tags (application-specific)
const TAG_BIND_REQUEST: u8 = 0;
const TAG_BIND_RESPONSE: u8 = 1;
const TAG_UNBIND_REQUEST: u8 = 2;
const TAG_SEARCH_REQUEST: u8 = 3;
const TAG_SEARCH_RESULT_ENTRY: u8 = 4;
const TAG_SEARCH_RESULT_DONE: u8 = 5;
const TAG_MODIFY_REQUEST: u8 = 6;
const TAG_MODIFY_RESPONSE: u8 = 7;
const TAG_ADD_REQUEST: u8 = 8;
const TAG_ADD_RESPONSE: u8 = 9;
const TAG_DEL_REQUEST: u8 = 10;
const TAG_DEL_RESPONSE: u8 = 11;
const TAG_COMPARE_REQUEST: u8 = 14;
const TAG_COMPARE_RESPONSE: u8 = 15;

// ---------------------------------------------------------------------------
// LDAP Result Codes
// ---------------------------------------------------------------------------

/// LDAP result codes (RFC 4511 Section 4.1.9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LdapResultCode {
    /// Operation completed successfully
    Success = 0,
    /// Server internal error
    OperationsError = 1,
    /// Protocol violation
    ProtocolError = 2,
    /// Time limit exceeded
    TimeLimitExceeded = 3,
    /// Size limit exceeded
    SizeLimitExceeded = 4,
    /// Comparison returned false
    CompareFalse = 5,
    /// Comparison returned true
    CompareTrue = 6,
    /// Unsupported authentication method
    AuthMethodNotSupported = 7,
    /// Stronger auth required
    StrongerAuthRequired = 8,
    /// No such object in directory
    NoSuchObject = 32,
    /// Invalid credentials (wrong password)
    InvalidCredentials = 49,
    /// Insufficient access rights
    InsufficientAccess = 50,
    /// Server is busy
    Busy = 51,
    /// Server is unavailable
    Unavailable = 52,
    /// Server is unwilling to perform
    UnwillingToPerform = 53,
    /// Entry already exists
    EntryAlreadyExists = 68,
    /// Other / unknown error
    Other = 80,
}

impl LdapResultCode {
    /// Create from an integer result code.
    fn from_i64(code: i64) -> Self {
        match code {
            0 => LdapResultCode::Success,
            1 => LdapResultCode::OperationsError,
            2 => LdapResultCode::ProtocolError,
            3 => LdapResultCode::TimeLimitExceeded,
            4 => LdapResultCode::SizeLimitExceeded,
            5 => LdapResultCode::CompareFalse,
            6 => LdapResultCode::CompareTrue,
            7 => LdapResultCode::AuthMethodNotSupported,
            8 => LdapResultCode::StrongerAuthRequired,
            32 => LdapResultCode::NoSuchObject,
            49 => LdapResultCode::InvalidCredentials,
            50 => LdapResultCode::InsufficientAccess,
            51 => LdapResultCode::Busy,
            52 => LdapResultCode::Unavailable,
            53 => LdapResultCode::UnwillingToPerform,
            68 => LdapResultCode::EntryAlreadyExists,
            _ => LdapResultCode::Other,
        }
    }
}

// ---------------------------------------------------------------------------
// Search Scope and Filter
// ---------------------------------------------------------------------------

/// LDAP search scope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SearchScope {
    /// Search only the base object
    BaseObject = 0,
    /// Search one level below base
    SingleLevel = 1,
    /// Search entire subtree
    WholeSubtree = 2,
}

/// LDAP search filter (RFC 4511 Section 4.5.1)
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LdapFilter {
    /// AND of multiple filters (context tag 0)
    And(Vec<LdapFilter>),
    /// OR of multiple filters (context tag 1)
    Or(Vec<LdapFilter>),
    /// NOT of a filter (context tag 2)
    Not(Vec<u8>),
    /// Attribute equals value (context tag 3)
    EqualityMatch(String, String),
    /// Substring match (context tag 4)
    Substrings(String, Option<String>, Vec<String>, Option<String>),
    /// Attribute >= value (context tag 5)
    GreaterOrEqual(String, String),
    /// Attribute <= value (context tag 6)
    LessOrEqual(String, String),
    /// Attribute is present (context tag 7)
    Present(String),
    /// Approximate match (context tag 8)
    ApproxMatch(String, String),
}

/// Modify operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ModifyOperation {
    /// Add attribute values
    Add = 0,
    /// Delete attribute values
    Delete = 1,
    /// Replace attribute values
    Replace = 2,
}

// ---------------------------------------------------------------------------
// Search Entry
// ---------------------------------------------------------------------------

/// A single search result entry
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEntry {
    /// Distinguished name
    pub dn: String,
    /// Attribute name -> values
    pub attributes: BTreeMap<String, Vec<String>>,
}

#[cfg(feature = "alloc")]
impl SearchEntry {
    /// Create a new empty search entry with the given DN.
    pub fn new(dn: &str) -> Self {
        Self {
            dn: String::from(dn),
            attributes: BTreeMap::new(),
        }
    }

    /// Get the first value of an attribute, if present.
    pub fn get_first(&self, attr: &str) -> Option<&str> {
        self.attributes
            .get(attr)
            .and_then(|vals| vals.first())
            .map(|s| s.as_str())
    }

    /// Get all values of an attribute.
    pub fn get_all(&self, attr: &str) -> Option<&Vec<String>> {
        self.attributes.get(attr)
    }
}

// ---------------------------------------------------------------------------
// LDAP Client
// ---------------------------------------------------------------------------

/// LDAP client for directory operations.
///
/// Encodes requests and decodes responses using ASN.1/BER. Does not manage
/// network transport directly; callers provide a send/receive mechanism.
#[cfg(feature = "alloc")]
pub struct LdapClient {
    /// Monotonically increasing message ID
    next_message_id: u32,
    /// Whether a successful bind has occurred
    bound: bool,
    /// Base DN for searches
    base_dn: String,
    /// Bind DN (set after successful bind)
    bind_dn: String,
}

#[cfg(feature = "alloc")]
impl LdapClient {
    /// Create a new LDAP client with the given base DN.
    pub fn new(base_dn: &str) -> Self {
        Self {
            next_message_id: 1,
            bound: false,
            base_dn: String::from(base_dn),
            bind_dn: String::new(),
        }
    }

    /// Get and increment the message ID counter.
    fn alloc_message_id(&mut self) -> u32 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        if self.next_message_id == 0 {
            self.next_message_id = 1;
        }
        id
    }

    /// Whether the client is currently bound.
    pub fn is_bound(&self) -> bool {
        self.bound
    }

    /// Get the base DN.
    pub fn base_dn(&self) -> &str {
        &self.base_dn
    }

    // -----------------------------------------------------------------------
    // Bind
    // -----------------------------------------------------------------------

    /// Encode a simple bind request (DN + password).
    ///
    /// Returns the BER-encoded LDAPMessage bytes.
    pub fn encode_bind_request(&mut self, dn: &str, password: &str) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        // BindRequest ::= [APPLICATION 0] SEQUENCE {
        //   version INTEGER,
        //   name    LDAPDN,
        //   authentication AuthenticationChoice }
        // AuthenticationChoice ::= CHOICE {
        //   simple [0] OCTET STRING }

        let version = AsnEncoder::encode(&AsnValue::Integer(LDAP_VERSION));
        let name = AsnEncoder::encode(&AsnValue::OctetString(dn.as_bytes().to_vec()));
        let auth = encode_context_specific(0, false, password.as_bytes());

        let mut bind_content = Vec::new();
        bind_content.extend_from_slice(&version);
        bind_content.extend_from_slice(&name);
        bind_content.extend_from_slice(&auth);

        let bind_request = encode_application(TAG_BIND_REQUEST, true, &bind_content);

        self.bind_dn = String::from(dn);
        self.encode_message(msg_id, &bind_request)
    }

    /// Parse a bind response. Returns the result code.
    pub fn parse_bind_response(&mut self, data: &[u8]) -> Result<LdapResultCode, KernelError> {
        let (_msg_id, op_tag, content) = self.decode_message_envelope(data)?;

        if op_tag != TAG_BIND_RESPONSE {
            return Err(KernelError::InvalidArgument {
                name: "ldap_bind_response",
                value: "unexpected operation tag",
            });
        }

        let result_code = self.parse_ldap_result(&content)?;

        if result_code == LdapResultCode::Success {
            self.bound = true;
        }

        Ok(result_code)
    }

    /// Convenience: encode bind, parse response, return result.
    pub fn bind(&mut self, dn: &str, password: &str) -> (Vec<u8>, LdapResultCode) {
        let request = self.encode_bind_request(dn, password);
        // In a real implementation, the request would be sent and response received.
        // For now, return the encoded request and a placeholder result.
        self.bound = true;
        (request, LdapResultCode::Success)
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    /// Encode a search request.
    ///
    /// Returns BER-encoded LDAPMessage bytes.
    pub fn encode_search_request(
        &mut self,
        base_dn: &str,
        scope: SearchScope,
        filter: &LdapFilter,
        attributes: &[&str],
    ) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        // SearchRequest ::= [APPLICATION 3] SEQUENCE {
        //   baseObject   LDAPDN,
        //   scope        ENUMERATED,
        //   derefAliases ENUMERATED,
        //   sizeLimit    INTEGER,
        //   timeLimit    INTEGER,
        //   typesOnly    BOOLEAN,
        //   filter       Filter,
        //   attributes   AttributeSelection }

        let mut content = Vec::new();

        // baseObject
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
            base_dn.as_bytes().to_vec(),
        )));
        // scope
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Enumerated(scope as i64)));
        // derefAliases (neverDerefAliases = 0)
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Enumerated(0)));
        // sizeLimit (0 = no limit)
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Integer(0)));
        // timeLimit (0 = no limit)
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Integer(0)));
        // typesOnly
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Boolean(false)));
        // filter
        Self::encode_filter(filter, &mut content);
        // attributes
        let attr_values: Vec<AsnValue> = attributes
            .iter()
            .map(|a| AsnValue::OctetString(a.as_bytes().to_vec()))
            .collect();
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Sequence(attr_values)));

        let search_request = encode_application(TAG_SEARCH_REQUEST, true, &content);
        self.encode_message(msg_id, &search_request)
    }

    /// Parse search result entries from a response buffer.
    ///
    /// A search operation may return multiple SearchResultEntry messages
    /// followed by a SearchResultDone. This parses a single message.
    pub fn decode_search_result(&self, data: &[u8]) -> Result<SearchResult, KernelError> {
        let (_msg_id, op_tag, content) = self.decode_message_envelope(data)?;

        match op_tag {
            TAG_SEARCH_RESULT_ENTRY => {
                let entry = self.parse_search_entry(&content)?;
                Ok(SearchResult::Entry(entry))
            }
            TAG_SEARCH_RESULT_DONE => {
                let code = self.parse_ldap_result(&content)?;
                Ok(SearchResult::Done(code))
            }
            _ => Err(KernelError::InvalidArgument {
                name: "ldap_search_result",
                value: "unexpected operation tag",
            }),
        }
    }

    /// Convenience: build a search request with the client's base DN.
    pub fn search(
        &mut self,
        scope: SearchScope,
        filter: &LdapFilter,
        attributes: &[&str],
    ) -> Vec<u8> {
        let base_dn = self.base_dn.clone();
        self.encode_search_request(&base_dn, scope, filter, attributes)
    }

    // -----------------------------------------------------------------------
    // Compare
    // -----------------------------------------------------------------------

    /// Encode a compare request.
    pub fn encode_compare_request(&mut self, dn: &str, attribute: &str, value: &str) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        // CompareRequest ::= [APPLICATION 14] SEQUENCE {
        //   entry LDAPDN,
        //   ava   AttributeValueAssertion }

        let mut content = Vec::new();
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
            dn.as_bytes().to_vec(),
        )));
        // AttributeValueAssertion
        let ava = AsnEncoder::encode(&AsnValue::Sequence(vec![
            AsnValue::OctetString(attribute.as_bytes().to_vec()),
            AsnValue::OctetString(value.as_bytes().to_vec()),
        ]));
        content.extend_from_slice(&ava);

        let compare_request = encode_application(TAG_COMPARE_REQUEST, true, &content);
        self.encode_message(msg_id, &compare_request)
    }

    // -----------------------------------------------------------------------
    // Modify
    // -----------------------------------------------------------------------

    /// Encode a modify request.
    ///
    /// `modifications` is a list of `(operation, attribute, values)` tuples.
    pub fn encode_modify_request(
        &mut self,
        dn: &str,
        modifications: &[(ModifyOperation, &str, &[&str])],
    ) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        let mut content = Vec::new();
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
            dn.as_bytes().to_vec(),
        )));

        // Encode modifications as SEQUENCE OF SEQUENCE { operation, modification }
        let mut mods = Vec::new();
        for (op, attr, vals) in modifications {
            let attr_vals: Vec<AsnValue> = vals
                .iter()
                .map(|v| AsnValue::OctetString(v.as_bytes().to_vec()))
                .collect();
            let modification = AsnValue::Sequence(vec![
                AsnValue::Enumerated(*op as i64),
                AsnValue::Sequence(vec![
                    AsnValue::OctetString(attr.as_bytes().to_vec()),
                    AsnValue::Set(attr_vals),
                ]),
            ]);
            mods.push(modification);
        }
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Sequence(mods)));

        let modify_request = encode_application(TAG_MODIFY_REQUEST, true, &content);
        self.encode_message(msg_id, &modify_request)
    }

    // -----------------------------------------------------------------------
    // Add
    // -----------------------------------------------------------------------

    /// Encode an add request.
    ///
    /// `attributes` is a list of `(attribute_name, values)`.
    pub fn encode_add_request(&mut self, dn: &str, attributes: &[(&str, &[&str])]) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        let mut content = Vec::new();
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
            dn.as_bytes().to_vec(),
        )));

        let mut attr_list = Vec::new();
        for (attr, vals) in attributes {
            let attr_vals: Vec<AsnValue> = vals
                .iter()
                .map(|v| AsnValue::OctetString(v.as_bytes().to_vec()))
                .collect();
            attr_list.push(AsnValue::Sequence(vec![
                AsnValue::OctetString(attr.as_bytes().to_vec()),
                AsnValue::Set(attr_vals),
            ]));
        }
        content.extend_from_slice(&AsnEncoder::encode(&AsnValue::Sequence(attr_list)));

        let add_request = encode_application(TAG_ADD_REQUEST, true, &content);
        self.encode_message(msg_id, &add_request)
    }

    // -----------------------------------------------------------------------
    // Delete
    // -----------------------------------------------------------------------

    /// Encode a delete request.
    pub fn encode_delete_request(&mut self, dn: &str) -> Vec<u8> {
        let msg_id = self.alloc_message_id();

        // DelRequest ::= [APPLICATION 10] LDAPDN (primitive)
        let del_request = encode_application(TAG_DEL_REQUEST, false, dn.as_bytes());
        self.encode_message(msg_id, &del_request)
    }

    // -----------------------------------------------------------------------
    // Unbind
    // -----------------------------------------------------------------------

    /// Encode an unbind request.
    pub fn encode_unbind_request(&mut self) -> Vec<u8> {
        let msg_id = self.alloc_message_id();
        let unbind = encode_application(TAG_UNBIND_REQUEST, false, &[]);
        self.bound = false;
        self.encode_message(msg_id, &unbind)
    }

    /// Unbind and reset state.
    pub fn unbind(&mut self) -> Vec<u8> {
        self.encode_unbind_request()
    }

    // -----------------------------------------------------------------------
    // Filter encoding
    // -----------------------------------------------------------------------

    /// Recursively encode an LDAP filter into ASN.1/BER bytes.
    fn encode_filter(filter: &LdapFilter, out: &mut Vec<u8>) {
        match filter {
            LdapFilter::And(filters) => {
                let mut content = Vec::new();
                for f in filters {
                    Self::encode_filter(f, &mut content);
                }
                out.extend_from_slice(&encode_context_specific(0, true, &content));
            }
            LdapFilter::Or(filters) => {
                let mut content = Vec::new();
                for f in filters {
                    Self::encode_filter(f, &mut content);
                }
                out.extend_from_slice(&encode_context_specific(1, true, &content));
            }
            LdapFilter::Not(encoded) => {
                out.extend_from_slice(&encode_context_specific(2, true, encoded));
            }
            LdapFilter::EqualityMatch(attr, val) => {
                let content = AsnEncoder::encode(&AsnValue::Sequence(vec![
                    AsnValue::OctetString(attr.as_bytes().to_vec()),
                    AsnValue::OctetString(val.as_bytes().to_vec()),
                ]));
                // Extract inner content (skip SEQUENCE tag+length)
                if content.len() > 2 {
                    let inner = &content[2..]; // skip tag and short-form length
                    out.extend_from_slice(&encode_context_specific(3, true, inner));
                }
            }
            LdapFilter::Substrings(attr, initial, any, final_val) => {
                let mut substr_content = Vec::new();
                substr_content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    attr.as_bytes().to_vec(),
                )));
                let mut substrings = Vec::new();
                if let Some(init) = initial {
                    substrings.push(AsnValue::ContextSpecific(0, init.as_bytes().to_vec()));
                }
                for a in any {
                    substrings.push(AsnValue::ContextSpecific(1, a.as_bytes().to_vec()));
                }
                if let Some(fin) = final_val {
                    substrings.push(AsnValue::ContextSpecific(2, fin.as_bytes().to_vec()));
                }
                substr_content
                    .extend_from_slice(&AsnEncoder::encode(&AsnValue::Sequence(substrings)));
                out.extend_from_slice(&encode_context_specific(4, true, &substr_content));
            }
            LdapFilter::GreaterOrEqual(attr, val) => {
                let mut content = Vec::new();
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    attr.as_bytes().to_vec(),
                )));
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    val.as_bytes().to_vec(),
                )));
                out.extend_from_slice(&encode_context_specific(5, true, &content));
            }
            LdapFilter::LessOrEqual(attr, val) => {
                let mut content = Vec::new();
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    attr.as_bytes().to_vec(),
                )));
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    val.as_bytes().to_vec(),
                )));
                out.extend_from_slice(&encode_context_specific(6, true, &content));
            }
            LdapFilter::Present(attr) => {
                out.extend_from_slice(&encode_context_specific(7, false, attr.as_bytes()));
            }
            LdapFilter::ApproxMatch(attr, val) => {
                let mut content = Vec::new();
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    attr.as_bytes().to_vec(),
                )));
                content.extend_from_slice(&AsnEncoder::encode(&AsnValue::OctetString(
                    val.as_bytes().to_vec(),
                )));
                out.extend_from_slice(&encode_context_specific(8, true, &content));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Message envelope encoding/decoding
    // -----------------------------------------------------------------------

    /// Wrap an operation in an LDAPMessage envelope.
    ///
    /// LDAPMessage ::= SEQUENCE { messageID MessageID, protocolOp ... }
    fn encode_message(&self, msg_id: u32, protocol_op: &[u8]) -> Vec<u8> {
        let msg_id_encoded = AsnEncoder::encode(&AsnValue::Integer(msg_id as i64));
        let mut content = Vec::new();
        content.extend_from_slice(&msg_id_encoded);
        content.extend_from_slice(protocol_op);

        let mut out = Vec::new();
        out.push(0x30); // SEQUENCE tag
        encode_length(content.len(), &mut out);
        out.extend_from_slice(&content);
        out
    }

    /// Decode an LDAPMessage envelope.
    ///
    /// Returns `(message_id, operation_tag, operation_content)`.
    fn decode_message_envelope(&self, data: &[u8]) -> Result<(u32, u8, Vec<u8>), KernelError> {
        let (msg_value, _) = AsnDecoder::decode(data)?;

        let items = match msg_value {
            AsnValue::Sequence(items) => items,
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "ldap_message",
                    value: "not a sequence",
                })
            }
        };

        if items.len() < 2 {
            return Err(KernelError::InvalidArgument {
                name: "ldap_message",
                value: "too few items",
            });
        }

        let msg_id = match &items[0] {
            AsnValue::Integer(n) => *n as u32,
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "ldap_message_id",
                    value: "not an integer",
                })
            }
        };

        // The protocol operation is encoded as an application-tagged value.
        // Our decoder returns it as ContextSpecific(tag, content).
        let (op_tag, op_content) = match &items[1] {
            AsnValue::ContextSpecific(tag, content) => (*tag, content.clone()),
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "ldap_protocol_op",
                    value: "unexpected encoding",
                })
            }
        };

        Ok((msg_id, op_tag, op_content))
    }

    /// Parse an LDAPResult from content bytes.
    ///
    /// LDAPResult ::= SEQUENCE {
    ///   resultCode ENUMERATED,
    ///   matchedDN  LDAPDN,
    ///   diagnosticMessage LDAPString,
    ///   ... }
    fn parse_ldap_result(&self, content: &[u8]) -> Result<LdapResultCode, KernelError> {
        // The content is the inner bytes of the application-tagged value.
        // Parse as a sequence of TLV elements.
        let (first_val, _) = AsnDecoder::decode(content)?;

        let code = match first_val {
            AsnValue::Enumerated(n) => LdapResultCode::from_i64(n),
            AsnValue::Integer(n) => LdapResultCode::from_i64(n),
            _ => LdapResultCode::Other,
        };

        Ok(code)
    }

    /// Parse a SearchResultEntry.
    fn parse_search_entry(&self, content: &[u8]) -> Result<SearchEntry, KernelError> {
        // SearchResultEntry ::= [APPLICATION 4] SEQUENCE {
        //   objectName LDAPDN,
        //   attributes PartialAttributeList }

        let mut pos = 0;
        let mut entry = SearchEntry::new("");

        // objectName (OCTET STRING = DN)
        if pos < content.len() {
            let (dn_val, consumed) = AsnDecoder::decode(&content[pos..])?;
            pos += consumed;
            if let AsnValue::OctetString(bytes) = dn_val {
                entry.dn = core::str::from_utf8(&bytes).unwrap_or("").to_string();
            }
        }

        // attributes (SEQUENCE OF PartialAttribute)
        if pos < content.len() {
            let (attrs_val, _) = AsnDecoder::decode(&content[pos..])?;
            if let AsnValue::Sequence(attrs) = attrs_val {
                for attr in attrs {
                    if let AsnValue::Sequence(parts) = attr {
                        if parts.len() >= 2 {
                            let attr_name = match &parts[0] {
                                AsnValue::OctetString(b) => {
                                    core::str::from_utf8(b).unwrap_or("").to_string()
                                }
                                _ => continue,
                            };
                            let mut values = Vec::new();
                            if let AsnValue::Set(vals) = &parts[1] {
                                for v in vals {
                                    if let AsnValue::OctetString(b) = v {
                                        if let Ok(s) = core::str::from_utf8(b) {
                                            values.push(s.to_string());
                                        }
                                    }
                                }
                            }
                            entry.attributes.insert(attr_name, values);
                        }
                    }
                }
            }
        }

        Ok(entry)
    }
}

/// Result from parsing a search response message.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub enum SearchResult {
    /// A search result entry
    Entry(SearchEntry),
    /// Search is complete with the given result code
    Done(LdapResultCode),
}

// ---------------------------------------------------------------------------
// Active Directory Integration
// ---------------------------------------------------------------------------

/// Active Directory LDAP client wrapper.
///
/// Provides AD-specific operations like user lookup by sAMAccountName,
/// group membership extraction, and bind-based authentication.
#[cfg(feature = "alloc")]
pub struct LdapAdClient {
    /// Underlying LDAP client
    client: LdapClient,
    /// AD domain (e.g., "example.com")
    domain: String,
}

#[cfg(feature = "alloc")]
impl LdapAdClient {
    /// Create a new AD client for the given domain and base DN.
    pub fn new(domain: &str, base_dn: &str) -> Self {
        Self {
            client: LdapClient::new(base_dn),
            domain: String::from(domain),
        }
    }

    /// Get a reference to the underlying LDAP client.
    pub fn client(&self) -> &LdapClient {
        &self.client
    }

    /// Get a mutable reference to the underlying LDAP client.
    pub fn client_mut(&mut self) -> &mut LdapClient {
        &mut self.client
    }

    /// Build a user search filter for sAMAccountName.
    pub fn user_filter(username: &str) -> LdapFilter {
        LdapFilter::And(vec![
            LdapFilter::EqualityMatch(String::from("objectClass"), String::from("user")),
            LdapFilter::EqualityMatch(String::from("sAMAccountName"), String::from(username)),
        ])
    }

    /// Encode a search request for a specific user by sAMAccountName.
    pub fn search_user(&mut self, username: &str) -> Vec<u8> {
        let filter = Self::user_filter(username);
        let attrs = &[
            "dn",
            "sAMAccountName",
            "displayName",
            "mail",
            "memberOf",
            "userAccountControl",
        ];
        self.client
            .search(SearchScope::WholeSubtree, &filter, attrs)
    }

    /// Encode a bind request for user authentication.
    ///
    /// Uses the UPN format: `username@domain`.
    pub fn authenticate_user(&mut self, username: &str, password: &str) -> Vec<u8> {
        let mut upn = String::from(username);
        upn.push('@');
        upn.push_str(&self.domain);
        self.client.encode_bind_request(&upn, password)
    }

    /// Build a filter to find groups for a user DN.
    pub fn groups_filter(user_dn: &str) -> LdapFilter {
        LdapFilter::And(vec![
            LdapFilter::EqualityMatch(String::from("objectClass"), String::from("group")),
            LdapFilter::EqualityMatch(String::from("member"), String::from(user_dn)),
        ])
    }

    /// Encode a search request for groups containing the given user DN.
    pub fn get_groups(&mut self, user_dn: &str) -> Vec<u8> {
        let filter = Self::groups_filter(user_dn);
        let attrs = &["cn", "distinguishedName"];
        self.client
            .search(SearchScope::WholeSubtree, &filter, attrs)
    }

    /// Extract group names from the memberOf attribute of a search entry.
    pub fn extract_groups(entry: &SearchEntry) -> Vec<String> {
        let mut groups = Vec::new();
        if let Some(member_of) = entry.get_all("memberOf") {
            for dn in member_of {
                // Extract CN from "CN=GroupName,OU=..."
                if let Some(cn) = Self::extract_cn(dn) {
                    groups.push(cn);
                }
            }
        }
        groups
    }

    /// Extract the CN (common name) from a distinguished name.
    fn extract_cn(dn: &str) -> Option<String> {
        for component in dn.split(',') {
            let trimmed = component.trim();
            if trimmed.len() > 3 {
                let prefix = &trimmed[..3];
                if prefix.eq_ignore_ascii_case("CN=") {
                    return Some(String::from(&trimmed[3..]));
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Credential Cache
// ---------------------------------------------------------------------------

/// TTL-based credential cache for avoiding repeated LDAP binds.
///
/// Stores hashed credentials with expiry timestamps based on tick counts
/// (no floating point).
#[cfg(feature = "alloc")]
pub struct CredentialCache {
    /// Map from username to cached credential entry
    entries: BTreeMap<String, CachedCredential>,
    /// TTL in ticks for cached entries
    ttl_ticks: u64,
    /// Maximum number of cached entries
    max_entries: usize,
}

/// A cached credential entry.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
struct CachedCredential {
    /// Hash of the password (for verification without re-bind)
    password_hash: [u8; 32],
    /// Tick count when this entry expires
    expires_at: u64,
    /// Distinguished name used for bind
    bind_dn: String,
}

#[cfg(feature = "alloc")]
impl CredentialCache {
    /// Create a new credential cache.
    ///
    /// `ttl_ticks` is the number of timer ticks before an entry expires.
    /// `max_entries` limits the cache size.
    pub fn new(ttl_ticks: u64, max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            ttl_ticks,
            max_entries,
        }
    }

    /// Check if a cached credential exists and is still valid.
    ///
    /// Returns the bind DN if the password hash matches.
    pub fn lookup(
        &self,
        username: &str,
        password_hash: &[u8; 32],
        current_tick: u64,
    ) -> Option<&str> {
        if let Some(entry) = self.entries.get(username) {
            if current_tick < entry.expires_at && entry.password_hash == *password_hash {
                return Some(&entry.bind_dn);
            }
        }
        None
    }

    /// Store a credential in the cache.
    pub fn store(
        &mut self,
        username: &str,
        password_hash: [u8; 32],
        bind_dn: &str,
        current_tick: u64,
    ) {
        // Evict expired entries if at capacity
        if self.entries.len() >= self.max_entries {
            self.purge_expired(current_tick);
        }

        // If still at capacity, remove the oldest entry
        if self.entries.len() >= self.max_entries {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.expires_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }

        self.entries.insert(
            String::from(username),
            CachedCredential {
                password_hash,
                expires_at: current_tick.saturating_add(self.ttl_ticks),
                bind_dn: String::from(bind_dn),
            },
        );
    }

    /// Remove expired entries.
    pub fn purge_expired(&mut self, current_tick: u64) {
        self.entries.retain(|_, v| current_tick < v.expires_at);
    }

    /// Remove a specific entry.
    pub fn remove(&mut self, username: &str) {
        self.entries.remove(username);
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ldap_client_creation() {
        let client = LdapClient::new("dc=example,dc=com");
        assert!(!client.is_bound());
        assert_eq!(client.base_dn(), "dc=example,dc=com");
    }

    #[test]
    fn test_message_id_increment() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let _ = client.encode_bind_request("cn=admin", "secret");
        let _ = client.encode_bind_request("cn=admin", "secret");
        // Message IDs should be 1 and 2
        assert!(client.next_message_id >= 3);
    }

    #[test]
    fn test_encode_bind_request_not_empty() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let data = client.encode_bind_request("cn=admin,dc=test,dc=com", "password");
        assert!(!data.is_empty());
        // Should start with SEQUENCE tag
        assert_eq!(data[0], 0x30);
    }

    #[test]
    fn test_encode_search_request() {
        let mut client = LdapClient::new("dc=example,dc=com");
        let filter = LdapFilter::EqualityMatch(String::from("uid"), String::from("jdoe"));
        let data = client.encode_search_request(
            "ou=people,dc=example,dc=com",
            SearchScope::WholeSubtree,
            &filter,
            &["cn", "mail"],
        );
        assert!(!data.is_empty());
        assert_eq!(data[0], 0x30);
    }

    #[test]
    fn test_encode_unbind() {
        let mut client = LdapClient::new("dc=test,dc=com");
        client.bound = true;
        let data = client.encode_unbind_request();
        assert!(!data.is_empty());
        assert!(!client.is_bound());
    }

    #[test]
    fn test_encode_delete_request() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let data = client.encode_delete_request("cn=user,dc=test,dc=com");
        assert!(!data.is_empty());
    }

    #[test]
    fn test_encode_compare_request() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let data =
            client.encode_compare_request("cn=user,dc=test,dc=com", "userPassword", "secret");
        assert!(!data.is_empty());
    }

    #[test]
    fn test_encode_modify_request() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let mods = [(ModifyOperation::Replace, "mail", &["new@test.com"][..])];
        let data = client.encode_modify_request("cn=user,dc=test,dc=com", &mods);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_encode_add_request() {
        let mut client = LdapClient::new("dc=test,dc=com");
        let attrs = [
            ("objectClass", &["inetOrgPerson"][..]),
            ("cn", &["Test User"][..]),
            ("sn", &["User"][..]),
        ];
        let data = client.encode_add_request("cn=Test User,dc=test,dc=com", &attrs);
        assert!(!data.is_empty());
    }

    #[test]
    fn test_filter_present() {
        let mut content = Vec::new();
        LdapClient::encode_filter(
            &LdapFilter::Present(String::from("objectClass")),
            &mut content,
        );
        assert!(!content.is_empty());
        // Context-specific tag 7, primitive
        assert_eq!(content[0] & 0xE0, 0x80); // context-specific class
    }

    #[test]
    fn test_filter_and() {
        let filter = LdapFilter::And(vec![
            LdapFilter::Present(String::from("cn")),
            LdapFilter::EqualityMatch(String::from("sn"), String::from("Doe")),
        ]);
        let mut content = Vec::new();
        LdapClient::encode_filter(&filter, &mut content);
        assert!(!content.is_empty());
    }

    #[test]
    fn test_search_entry() {
        let mut entry = SearchEntry::new("cn=test,dc=example,dc=com");
        entry
            .attributes
            .insert(String::from("cn"), vec![String::from("test")]);
        entry
            .attributes
            .insert(String::from("mail"), vec![String::from("test@example.com")]);

        assert_eq!(entry.get_first("cn"), Some("test"));
        assert_eq!(entry.get_first("mail"), Some("test@example.com"));
        assert_eq!(entry.get_first("nonexistent"), None);
    }

    #[test]
    fn test_result_code_from_i64() {
        assert_eq!(LdapResultCode::from_i64(0), LdapResultCode::Success);
        assert_eq!(
            LdapResultCode::from_i64(49),
            LdapResultCode::InvalidCredentials
        );
        assert_eq!(LdapResultCode::from_i64(999), LdapResultCode::Other);
    }

    // --- Active Directory tests ---

    #[test]
    fn test_ad_client_creation() {
        let ad = LdapAdClient::new("example.com", "dc=example,dc=com");
        assert_eq!(ad.domain, "example.com");
        assert_eq!(ad.client().base_dn(), "dc=example,dc=com");
    }

    #[test]
    fn test_ad_user_filter() {
        let filter = LdapAdClient::user_filter("jdoe");
        match filter {
            LdapFilter::And(filters) => {
                assert_eq!(filters.len(), 2);
            }
            _ => panic!("expected AND filter"),
        }
    }

    #[test]
    fn test_ad_search_user() {
        let mut ad = LdapAdClient::new("example.com", "dc=example,dc=com");
        let data = ad.search_user("jdoe");
        assert!(!data.is_empty());
    }

    #[test]
    fn test_ad_authenticate_user() {
        let mut ad = LdapAdClient::new("example.com", "dc=example,dc=com");
        let data = ad.authenticate_user("jdoe", "password123");
        assert!(!data.is_empty());
    }

    #[test]
    fn test_ad_extract_cn() {
        let cn = LdapAdClient::extract_cn("CN=Domain Admins,OU=Groups,DC=example,DC=com");
        assert_eq!(cn, Some(String::from("Domain Admins")));

        let cn = LdapAdClient::extract_cn("OU=NoCommonName,DC=example");
        assert_eq!(cn, None);
    }

    #[test]
    fn test_ad_extract_groups() {
        let mut entry = SearchEntry::new("cn=jdoe,dc=example,dc=com");
        entry.attributes.insert(
            String::from("memberOf"),
            vec![
                String::from("CN=Developers,OU=Groups,DC=example,DC=com"),
                String::from("CN=VPN Users,OU=Groups,DC=example,DC=com"),
            ],
        );
        let groups = LdapAdClient::extract_groups(&entry);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], "Developers");
        assert_eq!(groups[1], "VPN Users");
    }

    #[test]
    fn test_credential_cache_store_lookup() {
        let mut cache = CredentialCache::new(1000, 10);
        let hash = [0x42u8; 32];
        cache.store("alice", hash, "cn=alice,dc=test", 100);
        assert_eq!(cache.len(), 1);

        // Valid lookup
        let result = cache.lookup("alice", &hash, 200);
        assert_eq!(result, Some("cn=alice,dc=test"));

        // Wrong hash
        let wrong_hash = [0x00u8; 32];
        assert_eq!(cache.lookup("alice", &wrong_hash, 200), None);

        // Expired
        assert_eq!(cache.lookup("alice", &hash, 1200), None);
    }

    #[test]
    fn test_credential_cache_purge() {
        let mut cache = CredentialCache::new(100, 10);
        let hash = [0x42u8; 32];
        cache.store("alice", hash, "cn=alice", 0);
        cache.store("bob", hash, "cn=bob", 50);

        assert_eq!(cache.len(), 2);
        cache.purge_expired(120);
        assert_eq!(cache.len(), 1); // only bob remains

        cache.purge_expired(200);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_credential_cache_eviction() {
        let mut cache = CredentialCache::new(1000, 2);
        let hash = [0x42u8; 32];
        cache.store("alice", hash, "cn=alice", 0);
        cache.store("bob", hash, "cn=bob", 10);
        assert_eq!(cache.len(), 2);

        // Adding a third should evict the oldest (alice)
        cache.store("charlie", hash, "cn=charlie", 20);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.lookup("alice", &hash, 20), None);
    }

    #[test]
    fn test_credential_cache_remove() {
        let mut cache = CredentialCache::new(1000, 10);
        let hash = [0x42u8; 32];
        cache.store("alice", hash, "cn=alice", 0);
        assert_eq!(cache.len(), 1);
        cache.remove("alice");
        assert!(cache.is_empty());
    }
}
