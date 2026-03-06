//! Git Network Transport
//!
//! Implements the Git smart HTTP protocol for clone, fetch, push, and pull
//! operations. Wraps the existing `net::http` and `net::tls` modules.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::objects::ObjectId;

/// Transport protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportProtocol {
    /// Git smart HTTP/HTTPS
    Http,
    /// Git protocol (git://)
    Git,
    /// SSH (not yet implemented)
    Ssh,
}

/// Remote repository reference
#[derive(Debug, Clone)]
pub(crate) struct RemoteRef {
    pub(crate) id: ObjectId,
    pub(crate) name: String,
}

/// Remote connection configuration
#[derive(Debug, Clone)]
pub(crate) struct RemoteConfig {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) protocol: TransportProtocol,
}

impl RemoteConfig {
    pub(crate) fn new(name: &str, url: &str) -> Self {
        let protocol = if url.starts_with("https://") || url.starts_with("http://") {
            TransportProtocol::Http
        } else if url.starts_with("git://") {
            TransportProtocol::Git
        } else if url.starts_with("ssh://") || url.contains('@') {
            TransportProtocol::Ssh
        } else {
            TransportProtocol::Http
        };

        Self {
            name: name.to_string(),
            url: url.to_string(),
            protocol,
        }
    }

    /// Get info/refs URL for smart HTTP
    pub(crate) fn info_refs_url(&self) -> String {
        let base = self.url.trim_end_matches('/');
        alloc::format!("{}/info/refs?service=git-upload-pack", base)
    }

    /// Get upload-pack URL
    pub(crate) fn upload_pack_url(&self) -> String {
        let base = self.url.trim_end_matches('/');
        alloc::format!("{}/git-upload-pack", base)
    }

    /// Get receive-pack URL
    pub(crate) fn receive_pack_url(&self) -> String {
        let base = self.url.trim_end_matches('/');
        alloc::format!("{}/git-receive-pack", base)
    }
}

/// Git pkt-line protocol helpers
pub mod pktline {
    use alloc::vec::Vec;

    /// Encode a pkt-line (4-char hex length prefix + data)
    pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
        let len = data.len() + 4;
        let mut buf = Vec::with_capacity(len);
        buf.extend_from_slice(alloc::format!("{:04x}", len).as_bytes());
        buf.extend_from_slice(data);
        buf
    }

    /// Encode a flush packet (0000)
    pub(crate) fn flush() -> Vec<u8> {
        b"0000".to_vec()
    }

    /// Encode a delimiter packet (0001)
    pub(crate) fn delim() -> Vec<u8> {
        b"0001".to_vec()
    }

    /// Decode a pkt-line from a buffer, returning (line_data, bytes_consumed)
    pub(crate) fn decode(data: &[u8]) -> Option<(Vec<u8>, usize)> {
        if data.len() < 4 {
            return None;
        }

        let len_str = core::str::from_utf8(&data[..4]).ok()?;
        let len = usize::from_str_radix(len_str, 16).ok()?;

        if len == 0 {
            return Some((Vec::new(), 4)); // Flush
        }
        if len == 1 {
            return Some((Vec::new(), 4)); // Delimiter
        }
        if len < 4 || data.len() < len {
            return None;
        }

        let line_data = data[4..len].to_vec();
        Some((line_data, len))
    }

    /// Parse all pkt-lines from a buffer
    pub(crate) fn parse_all(mut data: &[u8]) -> Vec<Vec<u8>> {
        let mut lines = Vec::new();
        while !data.is_empty() {
            match decode(data) {
                Some((line, consumed)) => {
                    if !line.is_empty() {
                        lines.push(line);
                    }
                    data = &data[consumed..];
                }
                None => break,
            }
        }
        lines
    }
}

/// Parse server advertisement (info/refs response)
pub(crate) fn parse_refs_advertisement(data: &[u8]) -> Vec<RemoteRef> {
    let mut refs = Vec::new();

    let lines = pktline::parse_all(data);
    for line in &lines {
        let text = match core::str::from_utf8(line) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Skip service declaration line
        if text.starts_with('#') {
            continue;
        }

        // Format: "sha1 refname\0capabilities" or "sha1 refname"
        let text = text.split('\0').next().unwrap_or(text);
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let hex = parts[0].trim();
            let name = parts[1].trim();
            if hex.len() == 40 {
                if let Some(id) = ObjectId::from_hex(hex) {
                    refs.push(RemoteRef {
                        id,
                        name: name.to_string(),
                    });
                }
            }
        }
    }

    refs
}

/// Build a want/have negotiation request
pub(crate) fn build_want_request(wants: &[ObjectId], haves: &[ObjectId]) -> Vec<u8> {
    let mut buf = Vec::new();

    for (i, want) in wants.iter().enumerate() {
        let line = if i == 0 {
            alloc::format!("want {} multi_ack_detailed side-band-64k ofs-delta\n", want)
        } else {
            alloc::format!("want {}\n", want)
        };
        buf.extend_from_slice(&pktline::encode(line.as_bytes()));
    }

    buf.extend_from_slice(&pktline::flush());

    for have in haves {
        let line = alloc::format!("have {}\n", have);
        buf.extend_from_slice(&pktline::encode(line.as_bytes()));
    }

    let done = b"done\n";
    buf.extend_from_slice(&pktline::encode(done));

    buf
}

/// Build a push request
pub(crate) fn build_push_request(old_id: &ObjectId, new_id: &ObjectId, ref_name: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    let line = alloc::format!("{} {} {}\0report-status\n", old_id, new_id, ref_name);
    buf.extend_from_slice(&pktline::encode(line.as_bytes()));
    buf.extend_from_slice(&pktline::flush());
    buf
}

/// Packfile header
#[derive(Debug, Clone, Copy)]
pub(crate) struct PackHeader {
    pub(crate) version: u32,
    pub(crate) num_objects: u32,
}

/// Parse packfile header ("PACK" + version + object count)
pub(crate) fn parse_pack_header(data: &[u8]) -> Option<PackHeader> {
    if data.len() < 12 {
        return None;
    }
    if &data[0..4] != b"PACK" {
        return None;
    }

    let version = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let num_objects = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    Some(PackHeader {
        version,
        num_objects,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_config_http() {
        let remote = RemoteConfig::new("origin", "https://github.com/user/repo.git");
        assert_eq!(remote.protocol, TransportProtocol::Http);
    }

    #[test]
    fn test_remote_config_git() {
        let remote = RemoteConfig::new("origin", "git://github.com/user/repo.git");
        assert_eq!(remote.protocol, TransportProtocol::Git);
    }

    #[test]
    fn test_remote_config_ssh() {
        let remote = RemoteConfig::new("origin", "ssh://git@github.com/user/repo.git");
        assert_eq!(remote.protocol, TransportProtocol::Ssh);
    }

    #[test]
    fn test_remote_urls() {
        let remote = RemoteConfig::new("origin", "https://example.com/repo");
        assert!(remote.info_refs_url().contains("info/refs"));
        assert!(remote.upload_pack_url().contains("git-upload-pack"));
        assert!(remote.receive_pack_url().contains("git-receive-pack"));
    }

    #[test]
    fn test_pktline_encode() {
        let encoded = pktline::encode(b"hello\n");
        assert_eq!(&encoded[..4], b"000a");
        assert_eq!(&encoded[4..], b"hello\n");
    }

    #[test]
    fn test_pktline_flush() {
        assert_eq!(pktline::flush(), b"0000");
    }

    #[test]
    fn test_pktline_delim() {
        assert_eq!(pktline::delim(), b"0001");
    }

    #[test]
    fn test_pktline_decode() {
        let data = b"000ahello\n";
        let (line, consumed) = pktline::decode(data).unwrap();
        assert_eq!(&line, b"hello\n");
        assert_eq!(consumed, 10);
    }

    #[test]
    fn test_pktline_decode_flush() {
        let data = b"0000";
        let (line, consumed) = pktline::decode(data).unwrap();
        assert!(line.is_empty());
        assert_eq!(consumed, 4);
    }

    #[test]
    fn test_pktline_decode_too_short() {
        assert!(pktline::decode(b"00").is_none());
    }

    #[test]
    fn test_parse_pack_header() {
        let mut data = Vec::new();
        data.extend_from_slice(b"PACK");
        data.extend_from_slice(&2u32.to_be_bytes());
        data.extend_from_slice(&42u32.to_be_bytes());

        let header = parse_pack_header(&data).unwrap();
        assert_eq!(header.version, 2);
        assert_eq!(header.num_objects, 42);
    }

    #[test]
    fn test_parse_pack_header_invalid() {
        assert!(parse_pack_header(b"NOTPACK").is_none());
        assert!(parse_pack_header(b"PAC").is_none());
    }

    #[test]
    fn test_build_want_request() {
        let want = ObjectId::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
        let req = build_want_request(&[want], &[]);
        assert!(!req.is_empty());
        // Should contain "want" and "done"
        let text = String::from_utf8_lossy(&req);
        assert!(text.contains("want"));
        assert!(text.contains("done"));
    }

    #[test]
    fn test_build_push_request() {
        let old = ObjectId::ZERO;
        let new = ObjectId::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
        let req = build_push_request(&old, &new, "refs/heads/main");
        let text = String::from_utf8_lossy(&req);
        assert!(text.contains("refs/heads/main"));
    }

    #[test]
    fn test_parse_refs_advertisement() {
        let hex = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let line = alloc::format!("{} refs/heads/main\0multi_ack\n", hex);
        let pkt = pktline::encode(line.as_bytes());

        let refs = parse_refs_advertisement(&pkt);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "refs/heads/main");
        assert_eq!(refs[0].id.to_hex(), hex);
    }

    #[test]
    fn test_pktline_parse_all() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&pktline::encode(b"line1\n"));
        buf.extend_from_slice(&pktline::encode(b"line2\n"));
        buf.extend_from_slice(&pktline::flush());

        let lines = pktline::parse_all(&buf);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_transport_protocol_eq() {
        assert_eq!(TransportProtocol::Http, TransportProtocol::Http);
        assert_ne!(TransportProtocol::Http, TransportProtocol::Git);
    }
}
