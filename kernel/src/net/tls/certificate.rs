//! X.509 Certificate Parsing and Validation
//!
//! Provides simplified X.509 certificate parsing (DER-encoded) and
//! a trust store for root CA chain validation.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

// ============================================================================
// ASN.1 DER Parsing
// ============================================================================

/// ASN.1 tag constants
const ASN1_SEQUENCE: u8 = 0x30;
const ASN1_SET: u8 = 0x31;
const ASN1_OID: u8 = 0x06;
const ASN1_UTF8STRING: u8 = 0x0C;
const ASN1_PRINTABLESTRING: u8 = 0x13;
const ASN1_BIT_STRING: u8 = 0x03;

/// Parse ASN.1 DER tag and length. Returns (tag, content_start,
/// content_length).
pub(crate) fn asn1_parse_tlv(data: &[u8]) -> Option<(u8, usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let tag = data[0];
    if data.len() < 2 {
        return None;
    }

    let (content_start, content_len) = if data[1] & 0x80 == 0 {
        // Short form
        (2, data[1] as usize)
    } else {
        let num_len_bytes = (data[1] & 0x7F) as usize;
        if num_len_bytes == 0 || num_len_bytes > 4 || data.len() < 2 + num_len_bytes {
            return None;
        }
        let mut len: usize = 0;
        for i in 0..num_len_bytes {
            len = (len << 8) | (data[2 + i] as usize);
        }
        (2 + num_len_bytes, len)
    };

    if content_start + content_len > data.len() {
        return None;
    }

    Some((tag, content_start, content_len))
}

/// Extract a Common Name (OID 2.5.4.3) from an X.501 Name sequence
fn extract_cn(name_data: &[u8]) -> Vec<u8> {
    // OID for commonName: 2.5.4.3 = 55 04 03
    let cn_oid: [u8; 3] = [0x55, 0x04, 0x03];

    let mut pos = 0;
    while pos < name_data.len() {
        if let Some((_tag, start, len)) = asn1_parse_tlv(&name_data[pos..]) {
            let inner = &name_data[pos + start..pos + start + len];
            // Search for CN OID within this SET
            if let Some(idx) = find_subsequence(inner, &cn_oid) {
                // The value follows the OID TLV
                let after_oid = idx + cn_oid.len();
                if after_oid < inner.len() {
                    if let Some((_vtag, vstart, vlen)) = asn1_parse_tlv(&inner[after_oid..]) {
                        let value = &inner[after_oid + vstart..after_oid + vstart + vlen];
                        return value.to_vec();
                    }
                }
            }
            pos += start + len;
        } else {
            break;
        }
    }

    Vec::new()
}

/// Find a subsequence within a byte slice
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    for i in 0..=haystack.len() - needle.len() {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i + needle.len());
        }
    }
    None
}

// ============================================================================
// X.509 Certificate
// ============================================================================

/// Simplified X.509 certificate representation
#[derive(Debug, Clone)]
pub struct X509Certificate {
    /// Raw DER-encoded certificate bytes
    pub raw: Vec<u8>,
    /// Subject common name (extracted from DER)
    pub subject_cn: Vec<u8>,
    /// Issuer common name (extracted from DER)
    pub issuer_cn: Vec<u8>,
    /// Subject public key bytes (raw)
    pub public_key: Vec<u8>,
    /// Not-before timestamp (Unix epoch seconds, 0 if unparsed)
    pub not_before: u64,
    /// Not-after timestamp (Unix epoch seconds, 0 if unparsed)
    pub not_after: u64,
    /// Is this a CA certificate?
    pub is_ca: bool,
}

impl X509Certificate {
    /// Parse a simplified X.509 certificate from DER-encoded bytes.
    ///
    /// This is a best-effort parser that extracts subject/issuer CN and
    /// the public key. Full ASN.1 validation is beyond scope.
    pub fn from_der(data: &[u8]) -> Option<Self> {
        // Outer SEQUENCE
        let (tag, start, len) = asn1_parse_tlv(data)?;
        if tag != ASN1_SEQUENCE {
            return None;
        }
        let cert_content = &data[start..start + len];

        // TBSCertificate (first SEQUENCE inside)
        let (tbs_tag, tbs_start, tbs_len) = asn1_parse_tlv(cert_content)?;
        if tbs_tag != ASN1_SEQUENCE {
            return None;
        }
        let tbs = &cert_content[tbs_start..tbs_start + tbs_len];

        // Skip version (context [0]) + serial number + signature algorithm
        // Then find issuer and subject sequences
        // This is simplified: we scan for CN OIDs in the TBS data

        let issuer_cn = extract_cn(tbs);

        // Subject is typically after issuer -- scan from a later offset
        let subject_cn = if tbs.len() > 100 {
            let second_half = &tbs[tbs.len() / 3..];
            let cn = extract_cn(second_half);
            if cn.is_empty() {
                issuer_cn.clone()
            } else {
                cn
            }
        } else {
            issuer_cn.clone()
        };

        // Extract public key (look for BIT STRING after SubjectPublicKeyInfo SEQUENCE)
        let mut public_key = Vec::new();
        let mut scan_pos = 0;
        while scan_pos < tbs.len() {
            if tbs[scan_pos] == ASN1_BIT_STRING {
                if let Some((_, bs_start, bs_len)) = asn1_parse_tlv(&tbs[scan_pos..]) {
                    if bs_len > 1 {
                        // Skip the "unused bits" byte
                        public_key =
                            tbs[scan_pos + bs_start + 1..scan_pos + bs_start + bs_len].to_vec();
                    }
                    break;
                }
            }
            scan_pos += 1;
        }

        Some(Self {
            raw: data.to_vec(),
            subject_cn,
            issuer_cn,
            public_key,
            not_before: 0,
            not_after: 0,
            is_ca: false,
        })
    }
}

// ============================================================================
// Trust Store
// ============================================================================

/// Trust anchor store for root CAs
pub struct TrustStore {
    /// Trusted root CA certificates (subject CN -> certificate)
    anchors: Vec<X509Certificate>,
}

impl Default for TrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore {
    /// Create an empty trust store
    pub fn new() -> Self {
        Self {
            anchors: Vec::new(),
        }
    }

    /// Add a trusted root CA certificate
    pub fn add_anchor(&mut self, cert: X509Certificate) {
        self.anchors.push(cert);
    }

    /// Validate a certificate chain against the trust store.
    ///
    /// Returns true if the chain can be verified back to a trusted anchor
    /// via basic issuer/subject matching.
    pub fn validate_chain(&self, chain: &[X509Certificate]) -> bool {
        if chain.is_empty() {
            return false;
        }

        // Walk the chain: each cert's issuer should match the next cert's subject
        for i in 0..chain.len().saturating_sub(1) {
            if chain[i].issuer_cn != chain[i + 1].subject_cn {
                return false;
            }
        }

        // The last cert in the chain should be issued by a trusted anchor
        let root_issuer = &chain[chain.len() - 1].issuer_cn;
        self.anchors
            .iter()
            .any(|anchor| &anchor.subject_cn == root_issuer)
    }

    /// Number of trust anchors
    pub fn anchor_count(&self) -> usize {
        self.anchors.len()
    }
}
