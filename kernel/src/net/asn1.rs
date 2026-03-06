//! ASN.1/BER Encoding Library
//!
//! Provides ASN.1 Basic Encoding Rules (BER) serialization and deserialization
//! for use by LDAP, Kerberos, and other protocols that depend on ASN.1
//! structured data.
//!
//! # Features
//!
//! - Tag encoding/decoding with all four tag classes
//! - Definite-length BER encoding (short and long forms)
//! - Primitive types: Boolean, Integer, BigInteger, BitString, OctetString,
//!   Null, OID, UTF8String, Enumerated
//! - Constructed types: Sequence, Set
//! - Context-specific tagged values
//! - Builder API for constructing ASN.1 structures fluently
//! - Full OID encoding/decoding with base-128 variable-length encoding

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Tag classes and types
// ---------------------------------------------------------------------------

/// ASN.1 tag class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TagClass {
    /// Universal (0)
    Universal = 0,
    /// Application (1)
    Application = 1,
    /// Context-specific (2)
    ContextSpecific = 2,
    /// Private (3)
    Private = 3,
}

impl TagClass {
    /// Create from the top 2 bits of a tag byte
    fn from_byte(b: u8) -> Self {
        match (b >> 6) & 0x03 {
            0 => TagClass::Universal,
            1 => TagClass::Application,
            2 => TagClass::ContextSpecific,
            3 => TagClass::Private,
            _ => TagClass::Universal,
        }
    }
}

/// ASN.1 tag identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// BOOLEAN (1)
    Boolean,
    /// INTEGER (2)
    Integer,
    /// BIT STRING (3)
    BitString,
    /// OCTET STRING (4)
    OctetString,
    /// NULL (5)
    Null,
    /// OBJECT IDENTIFIER (6)
    ObjectIdentifier,
    /// ENUMERATED (10)
    Enumerated,
    /// UTF8String (12)
    Utf8String,
    /// PrintableString (19)
    PrintableString,
    /// IA5String (22)
    Ia5String,
    /// UTCTime (23)
    UtcTime,
    /// SEQUENCE (0x30)
    Sequence,
    /// SET (0x31)
    Set,
    /// Context-specific tag with class and number
    ContextSpecific(TagClass, u8),
}

impl Tag {
    /// Get the tag number for universal types
    fn universal_number(&self) -> Option<u8> {
        match self {
            Tag::Boolean => Some(1),
            Tag::Integer => Some(2),
            Tag::BitString => Some(3),
            Tag::OctetString => Some(4),
            Tag::Null => Some(5),
            Tag::ObjectIdentifier => Some(6),
            Tag::Enumerated => Some(10),
            Tag::Utf8String => Some(12),
            Tag::PrintableString => Some(19),
            Tag::Ia5String => Some(22),
            Tag::UtcTime => Some(23),
            Tag::Sequence => Some(16), // 0x30 = constructed + 16
            Tag::Set => Some(17),      // 0x31 = constructed + 17
            Tag::ContextSpecific(_, _) => None,
        }
    }

    /// Whether this tag represents a constructed (vs primitive) encoding
    fn is_constructed(&self) -> bool {
        matches!(self, Tag::Sequence | Tag::Set | Tag::ContextSpecific(_, _))
    }
}

// ---------------------------------------------------------------------------
// ASN.1 value types
// ---------------------------------------------------------------------------

/// ASN.1 value
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsnValue {
    /// BOOLEAN
    Boolean(bool),
    /// INTEGER (fits in i64)
    Integer(i64),
    /// INTEGER (arbitrary precision, big-endian two's complement)
    BigInteger(Vec<u8>),
    /// BIT STRING (data bytes, unused bits in last byte)
    BitString(Vec<u8>, u8),
    /// OCTET STRING
    OctetString(Vec<u8>),
    /// NULL
    Null,
    /// OBJECT IDENTIFIER (arc components)
    Oid(Vec<u32>),
    /// UTF8String
    Utf8String(String),
    /// PrintableString
    PrintableString(String),
    /// IA5String
    Ia5String(String),
    /// UTCTime (raw string)
    UtcTime(String),
    /// SEQUENCE (ordered list)
    Sequence(Vec<AsnValue>),
    /// SET (unordered collection)
    Set(Vec<AsnValue>),
    /// ENUMERATED
    Enumerated(i64),
    /// Context-specific tagged value (tag number, raw bytes)
    ContextSpecific(u8, Vec<u8>),
}

// ---------------------------------------------------------------------------
// Length encoding/decoding
// ---------------------------------------------------------------------------

/// Encode a BER definite-length value.
///
/// Short form: lengths 0..=127 use a single byte.
/// Long form: lengths >= 128 use one byte for the count of length bytes,
/// followed by that many bytes encoding the length in big-endian.
#[cfg(feature = "alloc")]
pub fn encode_length(length: usize, out: &mut Vec<u8>) {
    if length < 128 {
        out.push(length as u8);
    } else if length <= 0xFF {
        out.push(0x81);
        out.push(length as u8);
    } else if length <= 0xFFFF {
        out.push(0x82);
        out.push((length >> 8) as u8);
        out.push(length as u8);
    } else if length <= 0xFF_FFFF {
        out.push(0x83);
        out.push((length >> 16) as u8);
        out.push((length >> 8) as u8);
        out.push(length as u8);
    } else {
        out.push(0x84);
        out.push((length >> 24) as u8);
        out.push((length >> 16) as u8);
        out.push((length >> 8) as u8);
        out.push(length as u8);
    }
}

/// Decode a BER definite-length value.
///
/// Returns `(length, bytes_consumed)` or an error if the data is truncated.
pub fn decode_length(data: &[u8]) -> Result<(usize, usize), KernelError> {
    if data.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "asn1_length",
            value: "truncated",
        });
    }

    let first = data[0];
    if first < 128 {
        Ok((first as usize, 1))
    } else {
        let num_bytes = (first & 0x7F) as usize;
        if num_bytes == 0 || num_bytes > 4 {
            return Err(KernelError::InvalidArgument {
                name: "asn1_length",
                value: "unsupported length encoding",
            });
        }
        if data.len() < 1 + num_bytes {
            return Err(KernelError::InvalidArgument {
                name: "asn1_length",
                value: "truncated long form",
            });
        }
        let mut length: usize = 0;
        for i in 0..num_bytes {
            length = (length << 8) | (data[1 + i] as usize);
        }
        Ok((length, 1 + num_bytes))
    }
}

// ---------------------------------------------------------------------------
// Tag encoding/decoding
// ---------------------------------------------------------------------------

/// Encode a tag byte (or bytes for high-tag-number form).
///
/// For tags 0..=30, a single byte suffices:
///   `[class << 6 | constructed << 5 | tag_number]`
///
/// High-tag-number form (tag >= 31) is supported but uncommon.
#[cfg(feature = "alloc")]
pub fn encode_tag(class: TagClass, constructed: bool, number: u8, out: &mut Vec<u8>) {
    let class_bits = (class as u8) << 6;
    let constructed_bit = if constructed { 0x20 } else { 0 };

    if number < 31 {
        out.push(class_bits | constructed_bit | number);
    } else {
        out.push(class_bits | constructed_bit | 0x1F);
        // Base-128 encoding for tag number >= 31
        encode_base128(number as u32, out);
    }
}

/// Decode a tag from the data stream.
///
/// Returns `(class, constructed, tag_number, bytes_consumed)`.
pub fn decode_tag(data: &[u8]) -> Result<(TagClass, bool, u8, usize), KernelError> {
    if data.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "asn1_tag",
            value: "truncated",
        });
    }

    let first = data[0];
    let class = TagClass::from_byte(first);
    let constructed = (first & 0x20) != 0;
    let low_bits = first & 0x1F;

    if low_bits < 31 {
        Ok((class, constructed, low_bits, 1))
    } else {
        // High-tag-number form
        let mut number: u32 = 0;
        let mut pos = 1;
        loop {
            if pos >= data.len() {
                return Err(KernelError::InvalidArgument {
                    name: "asn1_tag",
                    value: "truncated high tag",
                });
            }
            let b = data[pos];
            number = (number << 7) | ((b & 0x7F) as u32);
            pos += 1;
            if b & 0x80 == 0 {
                break;
            }
            if number > 255 {
                return Err(KernelError::InvalidArgument {
                    name: "asn1_tag",
                    value: "tag number too large",
                });
            }
        }
        Ok((class, constructed, number as u8, pos))
    }
}

// ---------------------------------------------------------------------------
// OID encoding/decoding
// ---------------------------------------------------------------------------

/// Encode an OID to BER content bytes.
///
/// The first two arcs are combined as `first * 40 + second`.
/// Subsequent arcs use base-128 variable-length encoding.
#[cfg(feature = "alloc")]
fn encode_oid(arcs: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    if arcs.len() < 2 {
        return out;
    }

    // First two components combined
    let first_byte = arcs[0].saturating_mul(40).saturating_add(arcs[1]);
    encode_base128(first_byte, &mut out);

    for &arc in &arcs[2..] {
        encode_base128(arc, &mut out);
    }

    out
}

/// Decode OID content bytes into arc components.
#[cfg(feature = "alloc")]
fn decode_oid(data: &[u8]) -> Result<Vec<u32>, KernelError> {
    if data.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "asn1_oid",
            value: "empty",
        });
    }

    let mut arcs = Vec::new();
    let mut pos = 0;

    // Decode first byte (combines first two arcs)
    let (first_combined, consumed) = decode_base128(data, pos)?;
    pos += consumed;

    if first_combined < 40 {
        arcs.push(0);
        arcs.push(first_combined);
    } else if first_combined < 80 {
        arcs.push(1);
        arcs.push(first_combined - 40);
    } else {
        arcs.push(2);
        arcs.push(first_combined - 80);
    }

    // Decode remaining arcs
    while pos < data.len() {
        let (arc, consumed) = decode_base128(data, pos)?;
        pos += consumed;
        arcs.push(arc);
    }

    Ok(arcs)
}

// ---------------------------------------------------------------------------
// Base-128 variable-length integer encoding
// ---------------------------------------------------------------------------

/// Encode a u32 in base-128 variable-length format.
/// Each byte has bit 7 set except the last.
#[cfg(feature = "alloc")]
fn encode_base128(mut value: u32, out: &mut Vec<u8>) {
    if value == 0 {
        out.push(0);
        return;
    }

    // Collect bytes in reverse order
    let mut bytes = [0u8; 5];
    let mut count = 0;
    while value > 0 {
        bytes[count] = (value & 0x7F) as u8;
        value >>= 7;
        count += 1;
    }

    // Write in correct order with continuation bits
    for i in (0..count).rev() {
        let b = bytes[i];
        if i > 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
        }
    }
}

/// Decode a base-128 variable-length integer.
/// Returns `(value, bytes_consumed)`.
fn decode_base128(data: &[u8], start: usize) -> Result<(u32, usize), KernelError> {
    let mut value: u32 = 0;
    let mut pos = start;

    loop {
        if pos >= data.len() {
            return Err(KernelError::InvalidArgument {
                name: "asn1_base128",
                value: "truncated",
            });
        }
        let b = data[pos];
        value = value
            .checked_mul(128)
            .and_then(|v| v.checked_add((b & 0x7F) as u32))
            .ok_or(KernelError::InvalidArgument {
                name: "asn1_base128",
                value: "overflow",
            })?;
        pos += 1;
        if b & 0x80 == 0 {
            break;
        }
    }

    Ok((value, pos - start))
}

// ---------------------------------------------------------------------------
// ASN.1 Encoder
// ---------------------------------------------------------------------------

/// Encodes an `AsnValue` tree into BER-encoded bytes.
#[cfg(feature = "alloc")]
pub struct AsnEncoder;

#[cfg(feature = "alloc")]
impl AsnEncoder {
    /// Encode an ASN.1 value to BER bytes.
    pub fn encode(value: &AsnValue) -> Vec<u8> {
        let mut out = Vec::new();
        Self::encode_value(value, &mut out);
        out
    }

    fn encode_value(value: &AsnValue, out: &mut Vec<u8>) {
        match value {
            AsnValue::Boolean(b) => {
                out.push(0x01); // tag
                out.push(0x01); // length
                out.push(if *b { 0xFF } else { 0x00 });
            }
            AsnValue::Integer(n) => {
                let content = Self::encode_integer(*n);
                out.push(0x02); // tag
                encode_length(content.len(), out);
                out.extend_from_slice(&content);
            }
            AsnValue::BigInteger(bytes) => {
                out.push(0x02); // tag
                encode_length(bytes.len(), out);
                out.extend_from_slice(bytes);
            }
            AsnValue::BitString(data, unused_bits) => {
                out.push(0x03); // tag
                encode_length(data.len() + 1, out);
                out.push(*unused_bits);
                out.extend_from_slice(data);
            }
            AsnValue::OctetString(data) => {
                out.push(0x04); // tag
                encode_length(data.len(), out);
                out.extend_from_slice(data);
            }
            AsnValue::Null => {
                out.push(0x05); // tag
                out.push(0x00); // length
            }
            AsnValue::Oid(arcs) => {
                let content = encode_oid(arcs);
                out.push(0x06); // tag
                encode_length(content.len(), out);
                out.extend_from_slice(&content);
            }
            AsnValue::Utf8String(s) => {
                out.push(0x0C); // tag 12
                encode_length(s.len(), out);
                out.extend_from_slice(s.as_bytes());
            }
            AsnValue::PrintableString(s) => {
                out.push(0x13); // tag 19
                encode_length(s.len(), out);
                out.extend_from_slice(s.as_bytes());
            }
            AsnValue::Ia5String(s) => {
                out.push(0x16); // tag 22
                encode_length(s.len(), out);
                out.extend_from_slice(s.as_bytes());
            }
            AsnValue::UtcTime(s) => {
                out.push(0x17); // tag 23
                encode_length(s.len(), out);
                out.extend_from_slice(s.as_bytes());
            }
            AsnValue::Sequence(items) => {
                let mut content = Vec::new();
                for item in items {
                    Self::encode_value(item, &mut content);
                }
                out.push(0x30); // tag (constructed)
                encode_length(content.len(), out);
                out.extend_from_slice(&content);
            }
            AsnValue::Set(items) => {
                let mut content = Vec::new();
                for item in items {
                    Self::encode_value(item, &mut content);
                }
                out.push(0x31); // tag (constructed)
                encode_length(content.len(), out);
                out.extend_from_slice(&content);
            }
            AsnValue::Enumerated(n) => {
                let content = Self::encode_integer(*n);
                out.push(0x0A); // tag 10
                encode_length(content.len(), out);
                out.extend_from_slice(&content);
            }
            AsnValue::ContextSpecific(number, data) => {
                // Context-specific, constructed
                encode_tag(TagClass::ContextSpecific, true, *number, out);
                encode_length(data.len(), out);
                out.extend_from_slice(data);
            }
        }
    }

    /// Encode a signed integer in minimal two's complement form.
    fn encode_integer(value: i64) -> Vec<u8> {
        if value == 0 {
            return vec![0x00];
        }

        let bytes = value.to_be_bytes();
        let mut start = 0;

        if value > 0 {
            // Skip leading 0x00 bytes, but keep one if the next byte has bit 7 set
            while start < 7 && bytes[start] == 0x00 {
                start += 1;
            }
            // If high bit is set, prepend a 0x00 for positive numbers
            if bytes[start] & 0x80 != 0 {
                let mut result = vec![0x00];
                result.extend_from_slice(&bytes[start..]);
                return result;
            }
        } else {
            // Negative: skip leading 0xFF bytes, but keep one if next byte has bit 7 clear
            while start < 7 && bytes[start] == 0xFF {
                start += 1;
            }
            if bytes[start] & 0x80 == 0 {
                let mut result = vec![0xFF];
                result.extend_from_slice(&bytes[start..]);
                return result;
            }
        }

        bytes[start..].to_vec()
    }
}

// ---------------------------------------------------------------------------
// ASN.1 Decoder
// ---------------------------------------------------------------------------

/// Decodes BER-encoded bytes into an `AsnValue` tree.
#[cfg(feature = "alloc")]
pub struct AsnDecoder;

#[cfg(feature = "alloc")]
impl AsnDecoder {
    /// Decode a single ASN.1 TLV from the beginning of `data`.
    ///
    /// Returns `(value, bytes_consumed)`.
    pub fn decode(data: &[u8]) -> Result<(AsnValue, usize), KernelError> {
        if data.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "asn1_decode",
                value: "empty input",
            });
        }

        let (class, constructed, tag_number, tag_len) = decode_tag(data)?;
        let (content_len, len_bytes) = decode_length(&data[tag_len..])?;
        let header_len = tag_len + len_bytes;

        if data.len() < header_len + content_len {
            return Err(KernelError::InvalidArgument {
                name: "asn1_decode",
                value: "truncated content",
            });
        }

        let content = &data[header_len..header_len + content_len];
        let total = header_len + content_len;

        // Context-specific tags
        if class == TagClass::ContextSpecific || class == TagClass::Application {
            return Ok((
                AsnValue::ContextSpecific(tag_number, content.to_vec()),
                total,
            ));
        }

        // Universal tags
        let value = if constructed {
            match tag_number {
                16 => {
                    // SEQUENCE
                    let items = Self::decode_sequence(content)?;
                    AsnValue::Sequence(items)
                }
                17 => {
                    // SET
                    let items = Self::decode_sequence(content)?;
                    AsnValue::Set(items)
                }
                _ => AsnValue::ContextSpecific(tag_number, content.to_vec()),
            }
        } else {
            match tag_number {
                1 => {
                    // BOOLEAN
                    if content.is_empty() {
                        return Err(KernelError::InvalidArgument {
                            name: "asn1_boolean",
                            value: "empty",
                        });
                    }
                    AsnValue::Boolean(content[0] != 0)
                }
                2 => {
                    // INTEGER
                    Self::decode_integer(content)?
                }
                3 => {
                    // BIT STRING
                    if content.is_empty() {
                        return Err(KernelError::InvalidArgument {
                            name: "asn1_bitstring",
                            value: "empty",
                        });
                    }
                    let unused_bits = content[0];
                    AsnValue::BitString(content[1..].to_vec(), unused_bits)
                }
                4 => {
                    // OCTET STRING
                    AsnValue::OctetString(content.to_vec())
                }
                5 => {
                    // NULL
                    AsnValue::Null
                }
                6 => {
                    // OID
                    let arcs = decode_oid(content)?;
                    AsnValue::Oid(arcs)
                }
                10 => {
                    // ENUMERATED
                    match Self::decode_integer(content)? {
                        AsnValue::Integer(n) => AsnValue::Enumerated(n),
                        AsnValue::BigInteger(b) => {
                            // Treat as small if possible
                            let n = Self::big_to_i64(&b);
                            AsnValue::Enumerated(n)
                        }
                        _ => {
                            return Err(KernelError::InvalidArgument {
                                name: "asn1_enumerated",
                                value: "unexpected decode",
                            })
                        }
                    }
                }
                12 => {
                    // UTF8String
                    let s = core::str::from_utf8(content).map_err(|_| {
                        KernelError::InvalidArgument {
                            name: "asn1_utf8string",
                            value: "invalid utf8",
                        }
                    })?;
                    AsnValue::Utf8String(String::from(s))
                }
                19 => {
                    // PrintableString
                    let s = core::str::from_utf8(content).map_err(|_| {
                        KernelError::InvalidArgument {
                            name: "asn1_printablestring",
                            value: "invalid utf8",
                        }
                    })?;
                    AsnValue::PrintableString(String::from(s))
                }
                22 => {
                    // IA5String
                    let s = core::str::from_utf8(content).map_err(|_| {
                        KernelError::InvalidArgument {
                            name: "asn1_ia5string",
                            value: "invalid utf8",
                        }
                    })?;
                    AsnValue::Ia5String(String::from(s))
                }
                23 => {
                    // UTCTime
                    let s = core::str::from_utf8(content).map_err(|_| {
                        KernelError::InvalidArgument {
                            name: "asn1_utctime",
                            value: "invalid utf8",
                        }
                    })?;
                    AsnValue::UtcTime(String::from(s))
                }
                _ => AsnValue::OctetString(content.to_vec()),
            }
        };

        Ok((value, total))
    }

    /// Decode a sequence of TLV items from concatenated content bytes.
    fn decode_sequence(data: &[u8]) -> Result<Vec<AsnValue>, KernelError> {
        let mut items = Vec::new();
        let mut pos = 0;
        while pos < data.len() {
            let (value, consumed) = Self::decode(&data[pos..])?;
            items.push(value);
            pos += consumed;
        }
        Ok(items)
    }

    /// Decode an integer from content bytes.
    ///
    /// If the value fits in i64, returns `AsnValue::Integer`.
    /// Otherwise returns `AsnValue::BigInteger`.
    fn decode_integer(content: &[u8]) -> Result<AsnValue, KernelError> {
        if content.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "asn1_integer",
                value: "empty",
            });
        }

        // Try to fit in i64 (up to 8 bytes)
        if content.len() <= 8 {
            let negative = content[0] & 0x80 != 0;
            let mut value: i64 = if negative { -1 } else { 0 };
            for &b in content {
                value = (value << 8) | (b as i64);
            }
            Ok(AsnValue::Integer(value))
        } else {
            Ok(AsnValue::BigInteger(content.to_vec()))
        }
    }

    /// Convert a BigInteger byte slice to i64 (truncating if too large).
    fn big_to_i64(bytes: &[u8]) -> i64 {
        if bytes.is_empty() {
            return 0;
        }
        let negative = bytes[0] & 0x80 != 0;
        let mut value: i64 = if negative { -1 } else { 0 };
        let start = if bytes.len() > 8 { bytes.len() - 8 } else { 0 };
        for &b in &bytes[start..] {
            value = (value << 8) | (b as i64);
        }
        value
    }
}

// ---------------------------------------------------------------------------
// Builder API
// ---------------------------------------------------------------------------

/// Fluent builder for constructing ASN.1 values.
///
/// # Example
///
/// ```ignore
/// let data = AsnBuilder::new()
///     .sequence(|s| {
///         s.integer(42)
///          .octet_string(b"hello")
///          .boolean(true)
///     })
///     .build();
/// ```
#[cfg(feature = "alloc")]
pub struct AsnBuilder {
    items: Vec<AsnValue>,
}

#[cfg(feature = "alloc")]
impl Default for AsnBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl AsnBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add a BOOLEAN value.
    pub fn boolean(mut self, value: bool) -> Self {
        self.items.push(AsnValue::Boolean(value));
        self
    }

    /// Add an INTEGER value.
    pub fn integer(mut self, value: i64) -> Self {
        self.items.push(AsnValue::Integer(value));
        self
    }

    /// Add a BigInteger value (raw bytes, two's complement).
    pub fn big_integer(mut self, bytes: &[u8]) -> Self {
        self.items.push(AsnValue::BigInteger(bytes.to_vec()));
        self
    }

    /// Add a BIT STRING value.
    pub fn bit_string(mut self, data: &[u8], unused_bits: u8) -> Self {
        self.items
            .push(AsnValue::BitString(data.to_vec(), unused_bits));
        self
    }

    /// Add an OCTET STRING value.
    pub fn octet_string(mut self, data: &[u8]) -> Self {
        self.items.push(AsnValue::OctetString(data.to_vec()));
        self
    }

    /// Add a NULL value.
    pub fn null(mut self) -> Self {
        self.items.push(AsnValue::Null);
        self
    }

    /// Add an OBJECT IDENTIFIER value.
    pub fn oid(mut self, arcs: &[u32]) -> Self {
        self.items.push(AsnValue::Oid(arcs.to_vec()));
        self
    }

    /// Add a UTF8String value.
    pub fn utf8_string(mut self, s: &str) -> Self {
        self.items.push(AsnValue::Utf8String(String::from(s)));
        self
    }

    /// Add an ENUMERATED value.
    pub fn enumerated(mut self, value: i64) -> Self {
        self.items.push(AsnValue::Enumerated(value));
        self
    }

    /// Add a context-specific tagged value.
    pub fn context_specific(mut self, number: u8, data: &[u8]) -> Self {
        self.items
            .push(AsnValue::ContextSpecific(number, data.to_vec()));
        self
    }

    /// Add a context-specific tagged value wrapping an encoded ASN.1 value.
    pub fn context_specific_value(mut self, number: u8, value: &AsnValue) -> Self {
        let encoded = AsnEncoder::encode(value);
        self.items.push(AsnValue::ContextSpecific(number, encoded));
        self
    }

    /// Add a nested SEQUENCE built with a closure.
    pub fn sequence<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AsnBuilder) -> AsnBuilder,
    {
        let inner = f(AsnBuilder::new());
        self.items.push(AsnValue::Sequence(inner.items));
        self
    }

    /// Add a nested SET built with a closure.
    pub fn set<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AsnBuilder) -> AsnBuilder,
    {
        let inner = f(AsnBuilder::new());
        self.items.push(AsnValue::Set(inner.items));
        self
    }

    /// Add a raw pre-built AsnValue.
    pub fn value(mut self, v: AsnValue) -> Self {
        self.items.push(v);
        self
    }

    /// Build the accumulated items.
    ///
    /// If there is exactly one item, returns its encoded form.
    /// If there are multiple items, wraps them in a SEQUENCE.
    pub fn build(self) -> Vec<u8> {
        if self.items.len() == 1 {
            AsnEncoder::encode(&self.items[0])
        } else {
            AsnEncoder::encode(&AsnValue::Sequence(self.items))
        }
    }

    /// Build and return the items as an `AsnValue::Sequence`.
    pub fn build_value(self) -> AsnValue {
        if self.items.len() == 1 {
            self.items.into_iter().next().unwrap_or(AsnValue::Null)
        } else {
            AsnValue::Sequence(self.items)
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: encode a context-specific wrapper around encoded content
// ---------------------------------------------------------------------------

/// Encode a context-specific tagged wrapper.
///
/// Produces `[tag_byte, length, content...]`.
#[cfg(feature = "alloc")]
pub fn encode_context_specific(number: u8, constructed: bool, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_tag(TagClass::ContextSpecific, constructed, number, &mut out);
    encode_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

/// Encode an application-tagged wrapper.
#[cfg(feature = "alloc")]
pub fn encode_application(number: u8, constructed: bool, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_tag(TagClass::Application, constructed, number, &mut out);
    encode_length(content.len(), &mut out);
    out.extend_from_slice(content);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_length_short() {
        let mut out = Vec::new();
        encode_length(0, &mut out);
        assert_eq!(out, [0x00]);

        let mut out = Vec::new();
        encode_length(127, &mut out);
        assert_eq!(out, [0x7F]);
    }

    #[test]
    fn test_encode_length_long() {
        let mut out = Vec::new();
        encode_length(128, &mut out);
        assert_eq!(out, [0x81, 0x80]);

        let mut out = Vec::new();
        encode_length(256, &mut out);
        assert_eq!(out, [0x82, 0x01, 0x00]);
    }

    #[test]
    fn test_decode_length_short() {
        let data = [0x05];
        let (len, consumed) = decode_length(&data).unwrap();
        assert_eq!(len, 5);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_decode_length_long() {
        let data = [0x82, 0x01, 0x00];
        let (len, consumed) = decode_length(&data).unwrap();
        assert_eq!(len, 256);
        assert_eq!(consumed, 3);
    }

    #[test]
    fn test_encode_decode_boolean() {
        let val = AsnValue::Boolean(true);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, consumed) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded, AsnValue::Boolean(true));
    }

    #[test]
    fn test_encode_decode_integer_positive() {
        let val = AsnValue::Integer(42);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Integer(42));
    }

    #[test]
    fn test_encode_decode_integer_negative() {
        let val = AsnValue::Integer(-128);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Integer(-128));
    }

    #[test]
    fn test_encode_decode_integer_zero() {
        let val = AsnValue::Integer(0);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Integer(0));
    }

    #[test]
    fn test_encode_decode_null() {
        let val = AsnValue::Null;
        let encoded = AsnEncoder::encode(&val);
        assert_eq!(encoded, [0x05, 0x00]);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Null);
    }

    #[test]
    fn test_encode_decode_octet_string() {
        let val = AsnValue::OctetString(vec![0x01, 0x02, 0x03]);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_utf8_string() {
        let val = AsnValue::Utf8String(String::from("hello"));
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Utf8String(String::from("hello")));
    }

    #[test]
    fn test_encode_decode_oid() {
        // OID 1.2.840.113549.1.1.1 (RSA encryption)
        let val = AsnValue::Oid(vec![1, 2, 840, 113549, 1, 1, 1]);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_sequence() {
        let val = AsnValue::Sequence(vec![
            AsnValue::Integer(1),
            AsnValue::Boolean(true),
            AsnValue::OctetString(vec![0xAB]),
        ]);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_encode_decode_enumerated() {
        let val = AsnValue::Enumerated(3);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, AsnValue::Enumerated(3));
    }

    #[test]
    fn test_encode_decode_bit_string() {
        let val = AsnValue::BitString(vec![0xFF, 0x80], 1);
        let encoded = AsnEncoder::encode(&val);
        let (decoded, _) = AsnDecoder::decode(&encoded).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_builder_single_value() {
        let data = AsnBuilder::new().integer(42).build();
        let (decoded, _) = AsnDecoder::decode(&data).unwrap();
        assert_eq!(decoded, AsnValue::Integer(42));
    }

    #[test]
    fn test_builder_sequence() {
        let data = AsnBuilder::new()
            .sequence(|s| s.integer(1).boolean(false).null())
            .build();
        let (decoded, _) = AsnDecoder::decode(&data).unwrap();
        match decoded {
            AsnValue::Sequence(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], AsnValue::Integer(1));
                assert_eq!(items[1], AsnValue::Boolean(false));
                assert_eq!(items[2], AsnValue::Null);
            }
            _ => panic!("expected sequence"),
        }
    }

    #[test]
    fn test_decode_truncated_returns_error() {
        let data: &[u8] = &[0x02, 0x05, 0x01]; // integer with length 5 but only 1 byte of content
        let result = AsnDecoder::decode(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_tag_encode_decode_roundtrip() {
        let mut out = Vec::new();
        encode_tag(TagClass::Application, true, 3, &mut out);
        let (class, constructed, number, _) = decode_tag(&out).unwrap();
        assert_eq!(class, TagClass::Application);
        assert!(constructed);
        assert_eq!(number, 3);
    }
}
