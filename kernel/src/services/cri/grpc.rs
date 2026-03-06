//! Minimal gRPC transport over HTTP/2
//!
//! Provides HTTP/2 frame parsing, HPACK static table, and gRPC message
//! framing for container runtime interface communication.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// HTTP/2 Frame Types
// ---------------------------------------------------------------------------

/// HTTP/2 frame type identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FrameType {
    /// DATA frame (type 0x0)
    Data,
    /// HEADERS frame (type 0x1)
    Headers,
    /// SETTINGS frame (type 0x4)
    Settings,
    /// WINDOW_UPDATE frame (type 0x8)
    WindowUpdate,
    /// GOAWAY frame (type 0x7)
    GoAway,
    /// Unknown frame type
    Unknown(u8),
}

impl FrameType {
    /// Convert a raw byte to a frame type.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x0 => FrameType::Data,
            0x1 => FrameType::Headers,
            0x4 => FrameType::Settings,
            0x7 => FrameType::GoAway,
            0x8 => FrameType::WindowUpdate,
            other => FrameType::Unknown(other),
        }
    }

    /// Convert frame type to its wire byte.
    pub fn to_byte(self) -> u8 {
        match self {
            FrameType::Data => 0x0,
            FrameType::Headers => 0x1,
            FrameType::Settings => 0x4,
            FrameType::GoAway => 0x7,
            FrameType::WindowUpdate => 0x8,
            FrameType::Unknown(b) => b,
        }
    }
}

/// HTTP/2 frame header + payload.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Http2Frame {
    /// Length of the payload (24-bit).
    pub length: u32,
    /// Frame type.
    pub frame_type: FrameType,
    /// Frame flags.
    pub flags: u8,
    /// Stream identifier (31-bit).
    pub stream_id: u32,
    /// Frame payload.
    pub payload: Vec<u8>,
}

impl Http2Frame {
    /// HTTP/2 frame header size in bytes.
    pub const HEADER_SIZE: usize = 9;

    /// Parse an HTTP/2 frame from raw bytes.
    ///
    /// Returns the frame and the number of bytes consumed, or None if
    /// the buffer is too small.
    pub fn parse(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < Self::HEADER_SIZE {
            return None;
        }

        let length = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32);
        let frame_type = FrameType::from_byte(data[3]);
        let flags = data[4];
        let stream_id = ((data[5] as u32 & 0x7F) << 24)
            | ((data[6] as u32) << 16)
            | ((data[7] as u32) << 8)
            | (data[8] as u32);

        let total_len = Self::HEADER_SIZE + length as usize;
        if data.len() < total_len {
            return None;
        }

        let payload = data[Self::HEADER_SIZE..total_len].to_vec();

        Some((
            Http2Frame {
                length,
                frame_type,
                flags,
                stream_id,
                payload,
            },
            total_len,
        ))
    }

    /// Serialize this frame into bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::HEADER_SIZE + self.payload.len());
        // Length (24-bit)
        buf.push(((self.length >> 16) & 0xFF) as u8);
        buf.push(((self.length >> 8) & 0xFF) as u8);
        buf.push((self.length & 0xFF) as u8);
        // Type
        buf.push(self.frame_type.to_byte());
        // Flags
        buf.push(self.flags);
        // Stream ID (31-bit, R bit = 0)
        buf.push(((self.stream_id >> 24) & 0x7F) as u8);
        buf.push(((self.stream_id >> 16) & 0xFF) as u8);
        buf.push(((self.stream_id >> 8) & 0xFF) as u8);
        buf.push((self.stream_id & 0xFF) as u8);
        // Payload
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Create a DATA frame.
    pub fn data(stream_id: u32, payload: Vec<u8>, end_stream: bool) -> Self {
        let flags = if end_stream { 0x01 } else { 0x00 };
        Http2Frame {
            length: payload.len() as u32,
            frame_type: FrameType::Data,
            flags,
            stream_id,
            payload,
        }
    }

    /// Create a SETTINGS frame.
    pub fn settings(stream_id: u32, payload: Vec<u8>) -> Self {
        Http2Frame {
            length: payload.len() as u32,
            frame_type: FrameType::Settings,
            flags: 0,
            stream_id,
            payload,
        }
    }

    /// Create a SETTINGS ACK frame.
    pub fn settings_ack() -> Self {
        Http2Frame {
            length: 0,
            frame_type: FrameType::Settings,
            flags: 0x01,
            stream_id: 0,
            payload: Vec::new(),
        }
    }

    /// Create a WINDOW_UPDATE frame.
    pub fn window_update(stream_id: u32, increment: u32) -> Self {
        let payload = alloc::vec![
            ((increment >> 24) & 0x7F) as u8,
            ((increment >> 16) & 0xFF) as u8,
            ((increment >> 8) & 0xFF) as u8,
            (increment & 0xFF) as u8,
        ];
        Http2Frame {
            length: 4,
            frame_type: FrameType::WindowUpdate,
            flags: 0,
            stream_id,
            payload,
        }
    }

    /// Create a GOAWAY frame.
    pub fn goaway(last_stream_id: u32, error_code: u32) -> Self {
        let payload = alloc::vec![
            ((last_stream_id >> 24) & 0x7F) as u8,
            ((last_stream_id >> 16) & 0xFF) as u8,
            ((last_stream_id >> 8) & 0xFF) as u8,
            (last_stream_id & 0xFF) as u8,
            ((error_code >> 24) & 0xFF) as u8,
            ((error_code >> 16) & 0xFF) as u8,
            ((error_code >> 8) & 0xFF) as u8,
            (error_code & 0xFF) as u8,
        ];
        Http2Frame {
            length: 8,
            frame_type: FrameType::GoAway,
            flags: 0,
            stream_id: 0,
            payload,
        }
    }

    /// Check if END_STREAM flag is set.
    pub fn is_end_stream(&self) -> bool {
        self.flags & 0x01 != 0
    }

    /// Check if END_HEADERS flag is set (for HEADERS frames).
    pub fn is_end_headers(&self) -> bool {
        self.flags & 0x04 != 0
    }
}

// ---------------------------------------------------------------------------
// HPACK Static Table (RFC 7541, Appendix A)
// ---------------------------------------------------------------------------

/// HPACK static table: first 61 entries as (name, value) tuples.
pub const HPACK_STATIC_TABLE: &[(&str, &str); 61] = &[
    // Index 1
    (":authority", ""),
    // Index 2
    (":method", "GET"),
    // Index 3
    (":method", "POST"),
    // Index 4
    (":path", "/"),
    // Index 5
    (":path", "/index.html"),
    // Index 6
    (":scheme", "http"),
    // Index 7
    (":scheme", "https"),
    // Index 8
    (":status", "200"),
    // Index 9
    (":status", "204"),
    // Index 10
    (":status", "206"),
    // Index 11
    (":status", "304"),
    // Index 12
    (":status", "400"),
    // Index 13
    (":status", "404"),
    // Index 14
    (":status", "500"),
    // Index 15
    ("accept-charset", ""),
    // Index 16
    ("accept-encoding", "gzip, deflate"),
    // Index 17
    ("accept-language", ""),
    // Index 18
    ("accept-ranges", ""),
    // Index 19
    ("accept", ""),
    // Index 20
    ("access-control-allow-origin", ""),
    // Index 21
    ("age", ""),
    // Index 22
    ("allow", ""),
    // Index 23
    ("authorization", ""),
    // Index 24
    ("cache-control", ""),
    // Index 25
    ("content-disposition", ""),
    // Index 26
    ("content-encoding", ""),
    // Index 27
    ("content-language", ""),
    // Index 28
    ("content-length", ""),
    // Index 29
    ("content-location", ""),
    // Index 30
    ("content-range", ""),
    // Index 31
    ("content-type", ""),
    // Index 32
    ("cookie", ""),
    // Index 33
    ("date", ""),
    // Index 34
    ("etag", ""),
    // Index 35
    ("expect", ""),
    // Index 36
    ("expires", ""),
    // Index 37
    ("from", ""),
    // Index 38
    ("host", ""),
    // Index 39
    ("if-match", ""),
    // Index 40
    ("if-modified-since", ""),
    // Index 41
    ("if-none-match", ""),
    // Index 42
    ("if-range", ""),
    // Index 43
    ("if-unmodified-since", ""),
    // Index 44
    ("last-modified", ""),
    // Index 45
    ("link", ""),
    // Index 46
    ("location", ""),
    // Index 47
    ("max-forwards", ""),
    // Index 48
    ("proxy-authenticate", ""),
    // Index 49
    ("proxy-authorization", ""),
    // Index 50
    ("range", ""),
    // Index 51
    ("referer", ""),
    // Index 52
    ("refresh", ""),
    // Index 53
    ("retry-after", ""),
    // Index 54
    ("server", ""),
    // Index 55
    ("set-cookie", ""),
    // Index 56
    ("strict-transport-security", ""),
    // Index 57
    ("transfer-encoding", ""),
    // Index 58
    ("user-agent", ""),
    // Index 59
    ("vary", ""),
    // Index 60
    ("via", ""),
    // Index 61
    ("www-authenticate", ""),
];

/// Look up an HPACK static table entry by 1-based index.
pub fn hpack_static_lookup(index: usize) -> Option<(&'static str, &'static str)> {
    if index == 0 || index > HPACK_STATIC_TABLE.len() {
        return None;
    }
    Some(HPACK_STATIC_TABLE[index - 1])
}

/// Find the index of a header name in the HPACK static table.
pub fn hpack_static_find_name(name: &str) -> Option<usize> {
    for (i, (n, _)) in HPACK_STATIC_TABLE.iter().enumerate() {
        if *n == name {
            return Some(i + 1);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// gRPC Message Framing
// ---------------------------------------------------------------------------

/// gRPC message: 1-byte compressed flag + 4-byte length + payload.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GrpcMessage {
    /// Service name (e.g., "runtime.v1.RuntimeService").
    pub service: String,
    /// Method name (e.g., "RunPodSandbox").
    pub method: String,
    /// Raw protobuf-encoded payload.
    pub payload: Vec<u8>,
}

impl GrpcMessage {
    /// Create a new gRPC message.
    pub fn new(service: String, method: String, payload: Vec<u8>) -> Self {
        GrpcMessage {
            service,
            method,
            payload,
        }
    }

    /// Encode the payload into gRPC wire format (length-prefixed message).
    ///
    /// Format: [compressed(1)] [length(4)] [message(N)]
    pub fn encode_payload(&self) -> Vec<u8> {
        let len = self.payload.len() as u32;
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        // Compressed flag: 0 (not compressed)
        buf.push(0);
        // Message length (big-endian u32)
        buf.push(((len >> 24) & 0xFF) as u8);
        buf.push(((len >> 16) & 0xFF) as u8);
        buf.push(((len >> 8) & 0xFF) as u8);
        buf.push((len & 0xFF) as u8);
        // Payload
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode a gRPC length-prefixed message from raw bytes.
    ///
    /// Returns the decompressed payload and the number of bytes consumed,
    /// or None if the buffer is too small.
    pub fn decode_payload(data: &[u8]) -> Option<(Vec<u8>, usize)> {
        if data.len() < 5 {
            return None;
        }

        let _compressed = data[0];
        let length = ((data[1] as u32) << 24)
            | ((data[2] as u32) << 16)
            | ((data[3] as u32) << 8)
            | (data[4] as u32);

        let total = 5 + length as usize;
        if data.len() < total {
            return None;
        }

        let payload = data[5..total].to_vec();
        Some((payload, total))
    }

    /// Build the HTTP/2 path for this gRPC call.
    ///
    /// Format: `/{service}/{method}`
    pub fn path(&self) -> String {
        let mut p = String::with_capacity(2 + self.service.len() + self.method.len());
        p.push('/');
        p.push_str(&self.service);
        p.push('/');
        p.push_str(&self.method);
        p
    }
}

// ---------------------------------------------------------------------------
// gRPC Transport (Unix socket abstraction)
// ---------------------------------------------------------------------------

/// gRPC status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum GrpcStatus {
    /// Success.
    Ok,
    /// The operation was cancelled.
    Cancelled,
    /// Unknown error.
    Unknown,
    /// Invalid argument.
    InvalidArgument,
    /// Deadline exceeded.
    DeadlineExceeded,
    /// Resource not found.
    NotFound,
    /// Resource already exists.
    AlreadyExists,
    /// Permission denied.
    PermissionDenied,
    /// Resource exhausted.
    ResourceExhausted,
    /// Unimplemented operation.
    Unimplemented,
    /// Internal error.
    Internal,
    /// Service unavailable.
    Unavailable,
}

impl GrpcStatus {
    /// Convert to integer code.
    pub fn code(self) -> u32 {
        match self {
            GrpcStatus::Ok => 0,
            GrpcStatus::Cancelled => 1,
            GrpcStatus::Unknown => 2,
            GrpcStatus::InvalidArgument => 3,
            GrpcStatus::DeadlineExceeded => 4,
            GrpcStatus::NotFound => 5,
            GrpcStatus::AlreadyExists => 6,
            GrpcStatus::PermissionDenied => 7,
            GrpcStatus::ResourceExhausted => 8,
            GrpcStatus::Unimplemented => 12,
            GrpcStatus::Internal => 13,
            GrpcStatus::Unavailable => 14,
        }
    }

    /// Convert from integer code.
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => GrpcStatus::Ok,
            1 => GrpcStatus::Cancelled,
            3 => GrpcStatus::InvalidArgument,
            4 => GrpcStatus::DeadlineExceeded,
            5 => GrpcStatus::NotFound,
            6 => GrpcStatus::AlreadyExists,
            7 => GrpcStatus::PermissionDenied,
            8 => GrpcStatus::ResourceExhausted,
            12 => GrpcStatus::Unimplemented,
            13 => GrpcStatus::Internal,
            14 => GrpcStatus::Unavailable,
            _ => GrpcStatus::Unknown,
        }
    }
}

/// gRPC response.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GrpcResponse {
    /// Status code.
    pub status: GrpcStatus,
    /// Response payload.
    pub payload: Vec<u8>,
    /// Status message (optional).
    pub message: String,
}

impl GrpcResponse {
    /// Create a successful response.
    pub fn ok(payload: Vec<u8>) -> Self {
        GrpcResponse {
            status: GrpcStatus::Ok,
            payload,
            message: String::new(),
        }
    }

    /// Create an error response.
    pub fn error(status: GrpcStatus, message: String) -> Self {
        GrpcResponse {
            status,
            payload: Vec::new(),
            message,
        }
    }
}

/// Unix socket-based gRPC transport.
#[derive(Debug)]
#[allow(dead_code)]
pub struct GrpcTransport {
    /// Socket path for the CRI endpoint.
    socket_path: String,
    /// Next stream ID (odd for client-initiated).
    next_stream_id: u32,
    /// Maximum frame size.
    max_frame_size: u32,
    /// Connection window size.
    window_size: u32,
}

impl GrpcTransport {
    /// Default maximum HTTP/2 frame size (16 KiB).
    pub const DEFAULT_MAX_FRAME_SIZE: u32 = 16384;
    /// Default window size (64 KiB).
    pub const DEFAULT_WINDOW_SIZE: u32 = 65535;

    /// Create a new gRPC transport targeting a Unix socket.
    pub fn new(socket_path: String) -> Self {
        GrpcTransport {
            socket_path,
            next_stream_id: 1,
            max_frame_size: Self::DEFAULT_MAX_FRAME_SIZE,
            window_size: Self::DEFAULT_WINDOW_SIZE,
        }
    }

    /// Get the socket path.
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }

    /// Allocate the next client stream ID (always odd).
    pub fn next_stream(&mut self) -> u32 {
        let id = self.next_stream_id;
        self.next_stream_id += 2;
        id
    }

    /// Build a DATA frame containing a gRPC-encoded message.
    pub fn build_request_frame(&mut self, msg: &GrpcMessage) -> Http2Frame {
        let stream_id = self.next_stream();
        let encoded = msg.encode_payload();
        Http2Frame::data(stream_id, encoded, true)
    }

    /// Parse a gRPC response from an HTTP/2 DATA frame.
    pub fn parse_response_frame(frame: &Http2Frame) -> Option<GrpcResponse> {
        if frame.frame_type != FrameType::Data {
            return None;
        }

        match GrpcMessage::decode_payload(&frame.payload) {
            Some((payload, _)) => Some(GrpcResponse::ok(payload)),
            None => Some(GrpcResponse::error(
                GrpcStatus::Internal,
                String::from("failed to decode gRPC payload"),
            )),
        }
    }

    /// Get the current window size.
    pub fn window_size(&self) -> u32 {
        self.window_size
    }

    /// Update window size by a delta.
    pub fn update_window(&mut self, delta: u32) {
        self.window_size = self.window_size.saturating_add(delta);
    }

    /// Consume window for a send operation.
    pub fn consume_window(&mut self, amount: u32) -> bool {
        if self.window_size >= amount {
            self.window_size -= amount;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_frame_type_roundtrip() {
        for byte in 0..=8u8 {
            let ft = FrameType::from_byte(byte);
            assert_eq!(ft.to_byte(), byte);
        }
    }

    #[test]
    fn test_http2_frame_parse_too_small() {
        let data = [0u8; 5];
        assert!(Http2Frame::parse(&data).is_none());
    }

    #[test]
    fn test_http2_frame_parse_roundtrip() {
        let original = Http2Frame::data(3, vec![0xDE, 0xAD, 0xBE, 0xEF], true);
        let serialized = original.serialize();
        let (parsed, consumed) = Http2Frame::parse(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(parsed.stream_id, 3);
        assert_eq!(parsed.frame_type, FrameType::Data);
        assert!(parsed.is_end_stream());
        assert_eq!(parsed.payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_http2_settings_ack() {
        let frame = Http2Frame::settings_ack();
        assert_eq!(frame.frame_type, FrameType::Settings);
        assert_eq!(frame.flags, 0x01);
        assert_eq!(frame.stream_id, 0);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn test_http2_window_update() {
        let frame = Http2Frame::window_update(1, 32768);
        assert_eq!(frame.frame_type, FrameType::WindowUpdate);
        assert_eq!(frame.length, 4);
        let val = ((frame.payload[0] as u32 & 0x7F) << 24)
            | ((frame.payload[1] as u32) << 16)
            | ((frame.payload[2] as u32) << 8)
            | (frame.payload[3] as u32);
        assert_eq!(val, 32768);
    }

    #[test]
    fn test_http2_goaway() {
        let frame = Http2Frame::goaway(5, 0);
        assert_eq!(frame.frame_type, FrameType::GoAway);
        assert_eq!(frame.length, 8);
        assert_eq!(frame.stream_id, 0);
    }

    #[test]
    fn test_hpack_static_lookup() {
        let (name, val) = hpack_static_lookup(2).unwrap();
        assert_eq!(name, ":method");
        assert_eq!(val, "GET");
        assert!(hpack_static_lookup(0).is_none());
        assert!(hpack_static_lookup(62).is_none());
    }

    #[test]
    fn test_hpack_static_find_name() {
        assert_eq!(hpack_static_find_name(":authority"), Some(1));
        assert_eq!(hpack_static_find_name(":method"), Some(2));
        assert_eq!(hpack_static_find_name("content-type"), Some(31));
        assert!(hpack_static_find_name("x-custom").is_none());
    }

    #[test]
    fn test_grpc_message_encode_decode() {
        let msg = GrpcMessage::new(
            String::from("runtime.v1.RuntimeService"),
            String::from("RunPodSandbox"),
            vec![1, 2, 3, 4],
        );
        let encoded = msg.encode_payload();
        assert_eq!(encoded[0], 0); // not compressed
        let decoded_len = ((encoded[1] as u32) << 24)
            | ((encoded[2] as u32) << 16)
            | ((encoded[3] as u32) << 8)
            | (encoded[4] as u32);
        assert_eq!(decoded_len, 4);

        let (payload, consumed) = GrpcMessage::decode_payload(&encoded).unwrap();
        assert_eq!(payload, vec![1, 2, 3, 4]);
        assert_eq!(consumed, 9);
    }

    #[test]
    fn test_grpc_message_path() {
        let msg = GrpcMessage::new(
            String::from("runtime.v1.RuntimeService"),
            String::from("RunPodSandbox"),
            Vec::new(),
        );
        assert_eq!(msg.path(), "/runtime.v1.RuntimeService/RunPodSandbox");
    }

    #[test]
    fn test_grpc_status_roundtrip() {
        for code in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14] {
            let status = GrpcStatus::from_code(code);
            assert_eq!(status.code(), code);
        }
    }

    #[test]
    fn test_grpc_transport_stream_ids() {
        let mut transport = GrpcTransport::new(String::from("/run/cri.sock"));
        assert_eq!(transport.next_stream(), 1);
        assert_eq!(transport.next_stream(), 3);
        assert_eq!(transport.next_stream(), 5);
    }
}
