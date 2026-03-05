//! QUIC Protocol Implementation (RFC 9000)
//!
//! Provides QUIC transport protocol support including:
//! - Variable-length integer encoding (RFC 9000 Section 16)
//! - Long and short header packet formats
//! - Frame encoding/decoding (ACK, STREAM, CRYPTO, etc.)
//! - Connection management with state machine
//! - Bidirectional and unidirectional stream multiplexing
//! - Loss detection and recovery (RFC 9002)
//!
//! All arithmetic is integer-only (no floating point) for `no_std`
//! compatibility.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// QUIC version 1 (RFC 9000)
pub const QUIC_V1: u32 = 0x00000001;

/// Maximum connection ID length (bytes)
const MAX_CID_LEN: usize = 20;

/// Default idle timeout in milliseconds (30 seconds)
const DEFAULT_IDLE_TIMEOUT_MS: u64 = 30_000;

/// Default initial max data (bytes)
const DEFAULT_INITIAL_MAX_DATA: u64 = 1_048_576; // 1 MB

/// Default initial max stream data (bytes)
const DEFAULT_INITIAL_MAX_STREAM_DATA: u64 = 262_144; // 256 KB

/// Packet threshold for loss detection (RFC 9002 Section 6.1.1)
const PACKET_THRESHOLD: u64 = 3;

/// Time threshold numerator for loss detection (9/8 of RTT)
const TIME_THRESHOLD_NUM: u64 = 9;
const TIME_THRESHOLD_DEN: u64 = 8;

/// Minimum PTO in microseconds (1ms)
const PTO_MIN_US: u64 = 1_000;

/// Initial RTT estimate in microseconds (333ms per RFC 9002)
const INITIAL_RTT_US: u64 = 333_000;

/// SRTT/RTTVAR fixed-point shift (same as TCP Jacobson)
const SRTT_SHIFT: u32 = 3;
const RTTVAR_SHIFT: u32 = 2;

// ---------------------------------------------------------------------------
// QUIC Error
// ---------------------------------------------------------------------------

/// QUIC transport error codes (RFC 9000 Section 20)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum QuicError {
    NoError = 0x00,
    InternalError = 0x01,
    ConnectionRefused = 0x02,
    FlowControlError = 0x03,
    StreamLimitError = 0x04,
    StreamStateError = 0x05,
    FinalSizeError = 0x06,
    FrameEncodingError = 0x07,
    TransportParameterError = 0x08,
    ConnectionIdLimitError = 0x09,
    ProtocolViolation = 0x0A,
    InvalidToken = 0x0B,
    ApplicationError = 0x0C,
    CryptoBufferExceeded = 0x0D,
    KeyUpdateError = 0x0E,
    AeadLimitReached = 0x0F,
    NoViablePath = 0x10,
    /// Crypto-related errors 0x0100-0x01FF
    CryptoError = 0x0100,
    /// Buffer too small for encoding/decoding
    BufferTooSmall = 0xFFFF_0001,
    /// Invalid packet format
    InvalidPacket = 0xFFFF_0002,
    /// Invalid frame format
    InvalidFrame = 0xFFFF_0003,
    /// Connection not found
    ConnectionNotFound = 0xFFFF_0004,
    /// Stream not found
    StreamNotFound = 0xFFFF_0005,
}

impl QuicError {
    pub fn as_u64(self) -> u64 {
        self as u64
    }

    pub fn from_u64(val: u64) -> Self {
        match val {
            0x00 => Self::NoError,
            0x01 => Self::InternalError,
            0x02 => Self::ConnectionRefused,
            0x03 => Self::FlowControlError,
            0x04 => Self::StreamLimitError,
            0x05 => Self::StreamStateError,
            0x06 => Self::FinalSizeError,
            0x07 => Self::FrameEncodingError,
            0x08 => Self::TransportParameterError,
            0x09 => Self::ConnectionIdLimitError,
            0x0A => Self::ProtocolViolation,
            0x0B => Self::InvalidToken,
            0x0C => Self::ApplicationError,
            0x0D => Self::CryptoBufferExceeded,
            0x0E => Self::KeyUpdateError,
            0x0F => Self::AeadLimitReached,
            0x10 => Self::NoViablePath,
            0x0100 => Self::CryptoError,
            _ => Self::InternalError,
        }
    }
}

pub type QuicResult<T> = Result<T, QuicError>;

// ---------------------------------------------------------------------------
// Variable-Length Integer Encoding (RFC 9000 Section 16)
// ---------------------------------------------------------------------------

/// Encode a variable-length integer into `buf`, returning bytes written.
///
/// Values 0..63 use 1 byte, 64..16383 use 2 bytes,
/// 16384..1073741823 use 4 bytes, larger use 8 bytes.
pub fn encode_varint(value: u64, buf: &mut [u8]) -> QuicResult<usize> {
    if value <= 63 {
        if buf.is_empty() {
            return Err(QuicError::BufferTooSmall);
        }
        buf[0] = value as u8;
        Ok(1)
    } else if value <= 16383 {
        if buf.len() < 2 {
            return Err(QuicError::BufferTooSmall);
        }
        let v = (value as u16) | 0x4000;
        buf[0..2].copy_from_slice(&v.to_be_bytes());
        Ok(2)
    } else if value <= 1_073_741_823 {
        if buf.len() < 4 {
            return Err(QuicError::BufferTooSmall);
        }
        let v = (value as u32) | 0x8000_0000;
        buf[0..4].copy_from_slice(&v.to_be_bytes());
        Ok(4)
    } else if value <= 4_611_686_018_427_387_903 {
        if buf.len() < 8 {
            return Err(QuicError::BufferTooSmall);
        }
        let v = value | 0xC000_0000_0000_0000;
        buf[0..8].copy_from_slice(&v.to_be_bytes());
        Ok(8)
    } else {
        Err(QuicError::FrameEncodingError)
    }
}

/// Decode a variable-length integer from `buf`, returning (value,
/// bytes_consumed).
pub fn decode_varint(buf: &[u8]) -> QuicResult<(u64, usize)> {
    if buf.is_empty() {
        return Err(QuicError::BufferTooSmall);
    }
    let prefix = buf[0] >> 6;
    match prefix {
        0 => Ok((buf[0] as u64, 1)),
        1 => {
            if buf.len() < 2 {
                return Err(QuicError::BufferTooSmall);
            }
            let mut bytes = [0u8; 2];
            bytes.copy_from_slice(&buf[0..2]);
            let v = u16::from_be_bytes(bytes) & 0x3FFF;
            Ok((v as u64, 2))
        }
        2 => {
            if buf.len() < 4 {
                return Err(QuicError::BufferTooSmall);
            }
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&buf[0..4]);
            let v = u32::from_be_bytes(bytes) & 0x3FFF_FFFF;
            Ok((v as u64, 4))
        }
        3 => {
            if buf.len() < 8 {
                return Err(QuicError::BufferTooSmall);
            }
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&buf[0..8]);
            let v = u64::from_be_bytes(bytes) & 0x3FFF_FFFF_FFFF_FFFF;
            Ok((v, 8))
        }
        _ => unreachable!(),
    }
}

/// Return the number of bytes needed to encode `value` as a varint.
pub fn varint_len(value: u64) -> usize {
    if value <= 63 {
        1
    } else if value <= 16383 {
        2
    } else if value <= 1_073_741_823 {
        4
    } else {
        8
    }
}

// ---------------------------------------------------------------------------
// Connection ID
// ---------------------------------------------------------------------------

/// A QUIC Connection ID (0-20 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionId {
    pub len: u8,
    pub bytes: [u8; MAX_CID_LEN],
}

impl ConnectionId {
    pub const EMPTY: Self = Self {
        len: 0,
        bytes: [0u8; MAX_CID_LEN],
    };

    pub fn new(data: &[u8]) -> QuicResult<Self> {
        if data.len() > MAX_CID_LEN {
            return Err(QuicError::ConnectionIdLimitError);
        }
        let mut bytes = [0u8; MAX_CID_LEN];
        bytes[..data.len()].copy_from_slice(data);
        Ok(Self {
            len: data.len() as u8,
            bytes,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// Generate a connection ID from a simple seed (deterministic, for
    /// testing).
    pub fn generate(seed: u64, length: u8) -> QuicResult<Self> {
        if length as usize > MAX_CID_LEN {
            return Err(QuicError::ConnectionIdLimitError);
        }
        let mut bytes = [0u8; MAX_CID_LEN];
        let seed_bytes = seed.to_le_bytes();
        for i in 0..length as usize {
            bytes[i] = seed_bytes[i % 8];
        }
        Ok(Self { len: length, bytes })
    }
}

// ---------------------------------------------------------------------------
// Packet Number Encoding
// ---------------------------------------------------------------------------

/// Encode a packet number using the fewest bytes needed to represent the
/// difference from the largest acknowledged packet number.
///
/// Returns (encoded value, byte length 1-4).
pub fn encode_packet_number(full_pn: u64, largest_acked: u64) -> (u32, usize) {
    let num_unacked = if full_pn > largest_acked {
        full_pn - largest_acked
    } else {
        1
    };

    // Use twice the range as the encoding window
    let (encoded, len) = if num_unacked < 128 {
        ((full_pn & 0xFF) as u32, 1)
    } else if num_unacked < 32768 {
        ((full_pn & 0xFFFF) as u32, 2)
    } else if num_unacked < 8_388_608 {
        ((full_pn & 0xFF_FFFF) as u32, 3)
    } else {
        ((full_pn & 0xFFFF_FFFF) as u32, 4)
    };

    (encoded, len)
}

/// Decode a truncated packet number back to the full packet number.
///
/// Uses the expected packet number (largest received + 1) and the truncated
/// value to reconstruct the full packet number (RFC 9000 Appendix A).
pub fn decode_packet_number(largest_pn: u64, truncated_pn: u32, pn_len: usize) -> u64 {
    let expected_pn = largest_pn.wrapping_add(1);
    let pn_nbits = (pn_len * 8) as u64;
    let pn_win = 1u64 << pn_nbits;
    let pn_hwin = pn_win / 2;
    let pn_mask = pn_win - 1;

    // Replace lower bits of expected_pn with truncated value
    let candidate = (expected_pn & !pn_mask) | (truncated_pn as u64);

    if candidate.wrapping_add(pn_hwin) <= expected_pn
        && candidate < (1u64 << 62).wrapping_sub(pn_win)
    {
        candidate.wrapping_add(pn_win)
    } else if candidate > expected_pn.wrapping_add(pn_hwin) && candidate >= pn_win {
        candidate.wrapping_sub(pn_win)
    } else {
        candidate
    }
}

// ---------------------------------------------------------------------------
// Packet Types & Headers
// ---------------------------------------------------------------------------

/// QUIC long header packet types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongPacketType {
    Initial = 0x00,
    ZeroRtt = 0x01,
    Handshake = 0x02,
    Retry = 0x03,
}

impl LongPacketType {
    pub fn from_bits(bits: u8) -> QuicResult<Self> {
        match bits {
            0x00 => Ok(Self::Initial),
            0x01 => Ok(Self::ZeroRtt),
            0x02 => Ok(Self::Handshake),
            0x03 => Ok(Self::Retry),
            _ => Err(QuicError::InvalidPacket),
        }
    }
}

/// QUIC packet number space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PacketNumberSpace {
    Initial = 0,
    Handshake = 1,
    ApplicationData = 2,
}

/// A parsed QUIC long header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongHeader {
    /// First byte (form bit, fixed bit, type, reserved, pn_len)
    pub first_byte: u8,
    pub version: u32,
    pub dst_cid: ConnectionId,
    pub src_cid: ConnectionId,
    pub packet_type: LongPacketType,
    /// Token (Initial packets only)
    pub token: Vec<u8>,
    /// Packet number (decoded)
    pub packet_number: u64,
    /// Payload length (from Length field, includes packet number bytes)
    pub payload_length: u64,
}

impl LongHeader {
    /// Encode a long header into `buf`, returning bytes written.
    pub fn encode(&self, buf: &mut [u8]) -> QuicResult<usize> {
        let mut off = 0;

        // First byte: 1 (form) | 1 (fixed) | type (2 bits) | reserved (2) | pn_len (2)
        let (_, pn_len) = encode_packet_number(self.packet_number, 0);
        let first_byte =
            0xC0 | ((self.packet_type as u8) << 4) | ((pn_len as u8).wrapping_sub(1) & 0x03);

        if buf.len() < 7 {
            return Err(QuicError::BufferTooSmall);
        }
        buf[off] = first_byte;
        off += 1;

        // Version (4 bytes)
        buf[off..off + 4].copy_from_slice(&self.version.to_be_bytes());
        off += 4;

        // DCID length + DCID
        buf[off] = self.dst_cid.len;
        off += 1;
        let dcid_len = self.dst_cid.len as usize;
        if off + dcid_len > buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        buf[off..off + dcid_len].copy_from_slice(self.dst_cid.as_slice());
        off += dcid_len;

        // SCID length + SCID
        if off >= buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        buf[off] = self.src_cid.len;
        off += 1;
        let scid_len = self.src_cid.len as usize;
        if off + scid_len > buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        buf[off..off + scid_len].copy_from_slice(self.src_cid.as_slice());
        off += scid_len;

        // Token (Initial only)
        if self.packet_type == LongPacketType::Initial {
            let tok_len = self.token.len() as u64;
            let n = encode_varint(tok_len, &mut buf[off..])?;
            off += n;
            if off + self.token.len() > buf.len() {
                return Err(QuicError::BufferTooSmall);
            }
            buf[off..off + self.token.len()].copy_from_slice(&self.token);
            off += self.token.len();
        }

        // Length (varint): payload_length includes pn_len
        let total_payload = self.payload_length + pn_len as u64;
        let n = encode_varint(total_payload, &mut buf[off..])?;
        off += n;

        // Packet number
        let (pn_val, _) = encode_packet_number(self.packet_number, 0);
        if off + pn_len > buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        match pn_len {
            1 => buf[off] = pn_val as u8,
            2 => buf[off..off + 2].copy_from_slice(&(pn_val as u16).to_be_bytes()),
            3 => {
                buf[off] = (pn_val >> 16) as u8;
                buf[off + 1..off + 3].copy_from_slice(&(pn_val as u16).to_be_bytes());
            }
            4 => buf[off..off + 4].copy_from_slice(&pn_val.to_be_bytes()),
            _ => return Err(QuicError::InvalidPacket),
        }
        off += pn_len;

        Ok(off)
    }

    /// Decode a long header from `buf`, returning (header, bytes_consumed).
    pub fn decode(buf: &[u8]) -> QuicResult<(Self, usize)> {
        if buf.len() < 7 {
            return Err(QuicError::BufferTooSmall);
        }
        let mut off = 0;

        let first_byte = buf[off];
        off += 1;

        // Verify long header form bit
        if first_byte & 0x80 == 0 {
            return Err(QuicError::InvalidPacket);
        }

        let pkt_type_bits = (first_byte >> 4) & 0x03;
        let packet_type = LongPacketType::from_bits(pkt_type_bits)?;
        let pn_len = ((first_byte & 0x03) + 1) as usize;

        // Version
        let mut ver_bytes = [0u8; 4];
        ver_bytes.copy_from_slice(&buf[off..off + 4]);
        let version = u32::from_be_bytes(ver_bytes);
        off += 4;

        // DCID
        let dcid_len = buf[off] as usize;
        off += 1;
        if dcid_len > MAX_CID_LEN || off + dcid_len > buf.len() {
            return Err(QuicError::InvalidPacket);
        }
        let dst_cid = ConnectionId::new(&buf[off..off + dcid_len])?;
        off += dcid_len;

        // SCID
        if off >= buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        let scid_len = buf[off] as usize;
        off += 1;
        if scid_len > MAX_CID_LEN || off + scid_len > buf.len() {
            return Err(QuicError::InvalidPacket);
        }
        let src_cid = ConnectionId::new(&buf[off..off + scid_len])?;
        off += scid_len;

        // Token (Initial only)
        let mut token = Vec::new();
        if packet_type == LongPacketType::Initial {
            let (tok_len, n) = decode_varint(&buf[off..])?;
            off += n;
            if off + tok_len as usize > buf.len() {
                return Err(QuicError::BufferTooSmall);
            }
            token = buf[off..off + tok_len as usize].to_vec();
            off += tok_len as usize;
        }

        // Length
        let (payload_length, n) = decode_varint(&buf[off..])?;
        off += n;

        // Packet number
        if off + pn_len > buf.len() {
            return Err(QuicError::BufferTooSmall);
        }
        let packet_number = match pn_len {
            1 => buf[off] as u64,
            2 => {
                let mut b = [0u8; 2];
                b.copy_from_slice(&buf[off..off + 2]);
                u16::from_be_bytes(b) as u64
            }
            3 => ((buf[off] as u64) << 16) | ((buf[off + 1] as u64) << 8) | (buf[off + 2] as u64),
            4 => {
                let mut b = [0u8; 4];
                b.copy_from_slice(&buf[off..off + 4]);
                u32::from_be_bytes(b) as u64
            }
            _ => return Err(QuicError::InvalidPacket),
        };
        off += pn_len;

        let hdr = LongHeader {
            first_byte,
            version,
            dst_cid,
            src_cid,
            packet_type,
            token,
            packet_number,
            payload_length,
        };
        Ok((hdr, off))
    }
}

/// A parsed QUIC short header (1-RTT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortHeader {
    /// First byte: 0 (form) | 1 (fixed) | spin | reserved (2) | key_phase |
    /// pn_len (2)
    pub first_byte: u8,
    pub dst_cid: ConnectionId,
    pub packet_number: u64,
    pub spin_bit: bool,
    pub key_phase: bool,
}

impl ShortHeader {
    /// Create a new short header.
    pub fn new(
        dst_cid: ConnectionId,
        packet_number: u64,
        spin_bit: bool,
        key_phase: bool,
        largest_acked: u64,
    ) -> Self {
        let (_, pn_len) = encode_packet_number(packet_number, largest_acked);
        let mut first_byte = 0x40; // form=0, fixed=1
        if spin_bit {
            first_byte |= 0x20;
        }
        if key_phase {
            first_byte |= 0x04;
        }
        first_byte |= (pn_len as u8).wrapping_sub(1) & 0x03;

        Self {
            first_byte,
            dst_cid,
            packet_number,
            spin_bit,
            key_phase,
        }
    }

    /// Encode a short header into `buf`, returning bytes written.
    pub fn encode(&self, buf: &mut [u8], largest_acked: u64) -> QuicResult<usize> {
        let mut off = 0;
        let (pn_val, pn_len) = encode_packet_number(self.packet_number, largest_acked);

        let needed = 1 + self.dst_cid.len as usize + pn_len;
        if buf.len() < needed {
            return Err(QuicError::BufferTooSmall);
        }

        buf[off] = self.first_byte;
        off += 1;

        // DCID (length is known from connection context, not encoded)
        let dcid_len = self.dst_cid.len as usize;
        buf[off..off + dcid_len].copy_from_slice(self.dst_cid.as_slice());
        off += dcid_len;

        // Packet number
        match pn_len {
            1 => buf[off] = pn_val as u8,
            2 => buf[off..off + 2].copy_from_slice(&(pn_val as u16).to_be_bytes()),
            3 => {
                buf[off] = (pn_val >> 16) as u8;
                buf[off + 1..off + 3].copy_from_slice(&(pn_val as u16).to_be_bytes());
            }
            4 => buf[off..off + 4].copy_from_slice(&pn_val.to_be_bytes()),
            _ => return Err(QuicError::InvalidPacket),
        }
        off += pn_len;

        Ok(off)
    }
}

/// Apply header protection XOR mask to the first byte and packet number bytes.
///
/// `mask` is a 5-byte sample derived from the Header Protection (HP) key.
/// Byte 0 masks the first byte; bytes 1-4 mask packet number bytes.
pub fn apply_header_protection(buf: &mut [u8], pn_offset: usize, mask: &[u8; 5]) {
    if buf.is_empty() {
        return;
    }
    // Determine if long or short header
    if buf[0] & 0x80 != 0 {
        // Long header: mask lower 4 bits of first byte
        buf[0] ^= mask[0] & 0x0F;
    } else {
        // Short header: mask lower 5 bits of first byte
        buf[0] ^= mask[0] & 0x1F;
    }
    let pn_len = ((buf[0] & 0x03) + 1) as usize;
    for i in 0..pn_len {
        if pn_offset + i < buf.len() {
            buf[pn_offset + i] ^= mask[1 + i];
        }
    }
}

// ---------------------------------------------------------------------------
// QUIC Frames
// ---------------------------------------------------------------------------

/// QUIC frame types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicFrame {
    /// PADDING frame (0x00)
    Padding,

    /// PING frame (0x01)
    Ping,

    /// ACK frame (0x02 or 0x03)
    Ack {
        largest_acked: u64,
        ack_delay: u64,
        first_ack_range: u64,
        ack_ranges: Vec<AckRange>,
        /// ECN counts present if frame type is 0x03
        ecn_counts: Option<EcnCounts>,
    },

    /// CRYPTO frame (0x06)
    Crypto { offset: u64, data: Vec<u8> },

    /// NEW_CONNECTION_ID frame (0x18)
    NewConnectionId {
        sequence: u64,
        retire_prior_to: u64,
        connection_id: ConnectionId,
        stateless_reset_token: [u8; 16],
    },

    /// STREAM frame (0x08-0x0F)
    Stream {
        stream_id: u64,
        offset: u64,
        data: Vec<u8>,
        fin: bool,
    },

    /// MAX_DATA frame (0x10)
    MaxData { maximum_data: u64 },

    /// MAX_STREAM_DATA frame (0x11)
    MaxStreamData {
        stream_id: u64,
        maximum_stream_data: u64,
    },

    /// DATA_BLOCKED frame (0x14)
    DataBlocked { maximum_data: u64 },

    /// STREAM_DATA_BLOCKED frame (0x15)
    StreamDataBlocked {
        stream_id: u64,
        maximum_stream_data: u64,
    },

    /// CONNECTION_CLOSE frame (0x1C or 0x1D)
    ConnectionClose {
        error_code: u64,
        frame_type: Option<u64>,
        reason: Vec<u8>,
        is_application: bool,
    },

    /// PATH_CHALLENGE frame (0x1A)
    PathChallenge { data: [u8; 8] },

    /// PATH_RESPONSE frame (0x1B)
    PathResponse { data: [u8; 8] },
}

/// An ACK range (gap + ack_range_length).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckRange {
    pub gap: u64,
    pub ack_range_length: u64,
}

/// ECN counts for ACK frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcnCounts {
    pub ect0: u64,
    pub ect1: u64,
    pub ecn_ce: u64,
}

impl QuicFrame {
    /// Encode frame into `buf`, returning bytes written.
    pub fn encode(&self, buf: &mut [u8]) -> QuicResult<usize> {
        match self {
            Self::Padding => {
                if buf.is_empty() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[0] = 0x00;
                Ok(1)
            }

            Self::Ping => {
                if buf.is_empty() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[0] = 0x01;
                Ok(1)
            }

            Self::Ack {
                largest_acked,
                ack_delay,
                first_ack_range,
                ack_ranges,
                ecn_counts,
            } => {
                let mut off = 0;
                // Frame type
                let ft = if ecn_counts.is_some() { 0x03u8 } else { 0x02u8 };
                off += encode_varint(ft as u64, &mut buf[off..])?;
                off += encode_varint(*largest_acked, &mut buf[off..])?;
                off += encode_varint(*ack_delay, &mut buf[off..])?;
                off += encode_varint(ack_ranges.len() as u64, &mut buf[off..])?;
                off += encode_varint(*first_ack_range, &mut buf[off..])?;
                for range in ack_ranges {
                    off += encode_varint(range.gap, &mut buf[off..])?;
                    off += encode_varint(range.ack_range_length, &mut buf[off..])?;
                }
                if let Some(ecn) = ecn_counts {
                    off += encode_varint(ecn.ect0, &mut buf[off..])?;
                    off += encode_varint(ecn.ect1, &mut buf[off..])?;
                    off += encode_varint(ecn.ecn_ce, &mut buf[off..])?;
                }
                Ok(off)
            }

            Self::Crypto { offset, data } => {
                let mut off = 0;
                off += encode_varint(0x06, &mut buf[off..])?;
                off += encode_varint(*offset, &mut buf[off..])?;
                off += encode_varint(data.len() as u64, &mut buf[off..])?;
                if off + data.len() > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + data.len()].copy_from_slice(data);
                off += data.len();
                Ok(off)
            }

            Self::NewConnectionId {
                sequence,
                retire_prior_to,
                connection_id,
                stateless_reset_token,
            } => {
                let mut off = 0;
                off += encode_varint(0x18, &mut buf[off..])?;
                off += encode_varint(*sequence, &mut buf[off..])?;
                off += encode_varint(*retire_prior_to, &mut buf[off..])?;
                if off >= buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off] = connection_id.len;
                off += 1;
                let cid_len = connection_id.len as usize;
                if off + cid_len + 16 > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + cid_len].copy_from_slice(connection_id.as_slice());
                off += cid_len;
                buf[off..off + 16].copy_from_slice(stateless_reset_token);
                off += 16;
                Ok(off)
            }

            Self::Stream {
                stream_id,
                offset,
                data,
                fin,
            } => {
                let mut off = 0;
                // Frame type: 0x08 | OFF(0x04) | LEN(0x02) | FIN(0x01)
                let mut ft: u8 = 0x08;
                if *offset > 0 {
                    ft |= 0x04; // OFF bit
                }
                ft |= 0x02; // LEN bit (always include length)
                if *fin {
                    ft |= 0x01;
                }
                off += encode_varint(ft as u64, &mut buf[off..])?;
                off += encode_varint(*stream_id, &mut buf[off..])?;
                if *offset > 0 {
                    off += encode_varint(*offset, &mut buf[off..])?;
                }
                off += encode_varint(data.len() as u64, &mut buf[off..])?;
                if off + data.len() > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + data.len()].copy_from_slice(data);
                off += data.len();
                Ok(off)
            }

            Self::MaxData { maximum_data } => {
                let mut off = 0;
                off += encode_varint(0x10, &mut buf[off..])?;
                off += encode_varint(*maximum_data, &mut buf[off..])?;
                Ok(off)
            }

            Self::MaxStreamData {
                stream_id,
                maximum_stream_data,
            } => {
                let mut off = 0;
                off += encode_varint(0x11, &mut buf[off..])?;
                off += encode_varint(*stream_id, &mut buf[off..])?;
                off += encode_varint(*maximum_stream_data, &mut buf[off..])?;
                Ok(off)
            }

            Self::DataBlocked { maximum_data } => {
                let mut off = 0;
                off += encode_varint(0x14, &mut buf[off..])?;
                off += encode_varint(*maximum_data, &mut buf[off..])?;
                Ok(off)
            }

            Self::StreamDataBlocked {
                stream_id,
                maximum_stream_data,
            } => {
                let mut off = 0;
                off += encode_varint(0x15, &mut buf[off..])?;
                off += encode_varint(*stream_id, &mut buf[off..])?;
                off += encode_varint(*maximum_stream_data, &mut buf[off..])?;
                Ok(off)
            }

            Self::ConnectionClose {
                error_code,
                frame_type,
                reason,
                is_application,
            } => {
                let mut off = 0;
                let ft = if *is_application { 0x1Du64 } else { 0x1Cu64 };
                off += encode_varint(ft, &mut buf[off..])?;
                off += encode_varint(*error_code, &mut buf[off..])?;
                if !is_application {
                    off += encode_varint(frame_type.unwrap_or(0), &mut buf[off..])?;
                }
                off += encode_varint(reason.len() as u64, &mut buf[off..])?;
                if off + reason.len() > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + reason.len()].copy_from_slice(reason);
                off += reason.len();
                Ok(off)
            }

            Self::PathChallenge { data } => {
                let mut off = 0;
                off += encode_varint(0x1A, &mut buf[off..])?;
                if off + 8 > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + 8].copy_from_slice(data);
                off += 8;
                Ok(off)
            }

            Self::PathResponse { data } => {
                let mut off = 0;
                off += encode_varint(0x1B, &mut buf[off..])?;
                if off + 8 > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                buf[off..off + 8].copy_from_slice(data);
                off += 8;
                Ok(off)
            }
        }
    }

    /// Decode a single frame from `buf`, returning (frame, bytes_consumed).
    pub fn decode(buf: &[u8]) -> QuicResult<(Self, usize)> {
        if buf.is_empty() {
            return Err(QuicError::BufferTooSmall);
        }

        let (frame_type, mut off) = decode_varint(buf)?;

        match frame_type {
            0x00 => Ok((Self::Padding, off)),

            0x01 => Ok((Self::Ping, off)),

            0x02 | 0x03 => {
                let has_ecn = frame_type == 0x03;
                let (largest_acked, n) = decode_varint(&buf[off..])?;
                off += n;
                let (ack_delay, n) = decode_varint(&buf[off..])?;
                off += n;
                let (ack_range_count, n) = decode_varint(&buf[off..])?;
                off += n;
                let (first_ack_range, n) = decode_varint(&buf[off..])?;
                off += n;

                let mut ack_ranges = Vec::new();
                for _ in 0..ack_range_count {
                    let (gap, n) = decode_varint(&buf[off..])?;
                    off += n;
                    let (ack_range_length, n) = decode_varint(&buf[off..])?;
                    off += n;
                    ack_ranges.push(AckRange {
                        gap,
                        ack_range_length,
                    });
                }

                let ecn_counts = if has_ecn {
                    let (ect0, n) = decode_varint(&buf[off..])?;
                    off += n;
                    let (ect1, n) = decode_varint(&buf[off..])?;
                    off += n;
                    let (ecn_ce, n) = decode_varint(&buf[off..])?;
                    off += n;
                    Some(EcnCounts { ect0, ect1, ecn_ce })
                } else {
                    None
                };

                Ok((
                    Self::Ack {
                        largest_acked,
                        ack_delay,
                        first_ack_range,
                        ack_ranges,
                        ecn_counts,
                    },
                    off,
                ))
            }

            0x06 => {
                let (offset, n) = decode_varint(&buf[off..])?;
                off += n;
                let (length, n) = decode_varint(&buf[off..])?;
                off += n;
                let length = length as usize;
                if off + length > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                let data = buf[off..off + length].to_vec();
                off += length;
                Ok((Self::Crypto { offset, data }, off))
            }

            // STREAM frames: 0x08..=0x0F
            ft @ 0x08..=0x0F => {
                let has_offset = ft & 0x04 != 0;
                let has_length = ft & 0x02 != 0;
                let fin = ft & 0x01 != 0;

                let (stream_id, n) = decode_varint(&buf[off..])?;
                off += n;

                let offset = if has_offset {
                    let (o, n) = decode_varint(&buf[off..])?;
                    off += n;
                    o
                } else {
                    0
                };

                let data = if has_length {
                    let (length, n) = decode_varint(&buf[off..])?;
                    off += n;
                    let length = length as usize;
                    if off + length > buf.len() {
                        return Err(QuicError::BufferTooSmall);
                    }
                    let d = buf[off..off + length].to_vec();
                    off += length;
                    d
                } else {
                    // Remaining bytes in the packet
                    let d = buf[off..].to_vec();
                    off = buf.len();
                    d
                };

                Ok((
                    Self::Stream {
                        stream_id,
                        offset,
                        data,
                        fin,
                    },
                    off,
                ))
            }

            0x10 => {
                let (maximum_data, n) = decode_varint(&buf[off..])?;
                off += n;
                Ok((Self::MaxData { maximum_data }, off))
            }

            0x11 => {
                let (stream_id, n) = decode_varint(&buf[off..])?;
                off += n;
                let (maximum_stream_data, n) = decode_varint(&buf[off..])?;
                off += n;
                Ok((
                    Self::MaxStreamData {
                        stream_id,
                        maximum_stream_data,
                    },
                    off,
                ))
            }

            0x14 => {
                let (maximum_data, n) = decode_varint(&buf[off..])?;
                off += n;
                Ok((Self::DataBlocked { maximum_data }, off))
            }

            0x15 => {
                let (stream_id, n) = decode_varint(&buf[off..])?;
                off += n;
                let (maximum_stream_data, n) = decode_varint(&buf[off..])?;
                off += n;
                Ok((
                    Self::StreamDataBlocked {
                        stream_id,
                        maximum_stream_data,
                    },
                    off,
                ))
            }

            0x18 => {
                let (sequence, n) = decode_varint(&buf[off..])?;
                off += n;
                let (retire_prior_to, n) = decode_varint(&buf[off..])?;
                off += n;
                if off >= buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                let cid_len = buf[off] as usize;
                off += 1;
                if cid_len > MAX_CID_LEN || off + cid_len + 16 > buf.len() {
                    return Err(QuicError::InvalidFrame);
                }
                let connection_id = ConnectionId::new(&buf[off..off + cid_len])?;
                off += cid_len;
                let mut stateless_reset_token = [0u8; 16];
                stateless_reset_token.copy_from_slice(&buf[off..off + 16]);
                off += 16;
                Ok((
                    Self::NewConnectionId {
                        sequence,
                        retire_prior_to,
                        connection_id,
                        stateless_reset_token,
                    },
                    off,
                ))
            }

            0x1A => {
                if off + 8 > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                let mut data = [0u8; 8];
                data.copy_from_slice(&buf[off..off + 8]);
                off += 8;
                Ok((Self::PathChallenge { data }, off))
            }

            0x1B => {
                if off + 8 > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                let mut data = [0u8; 8];
                data.copy_from_slice(&buf[off..off + 8]);
                off += 8;
                Ok((Self::PathResponse { data }, off))
            }

            0x1C | 0x1D => {
                let is_application = frame_type == 0x1D;
                let (error_code, n) = decode_varint(&buf[off..])?;
                off += n;
                let frame_type_val = if !is_application {
                    let (ft, n) = decode_varint(&buf[off..])?;
                    off += n;
                    Some(ft)
                } else {
                    None
                };
                let (reason_len, n) = decode_varint(&buf[off..])?;
                off += n;
                let reason_len = reason_len as usize;
                if off + reason_len > buf.len() {
                    return Err(QuicError::BufferTooSmall);
                }
                let reason = buf[off..off + reason_len].to_vec();
                off += reason_len;
                Ok((
                    Self::ConnectionClose {
                        error_code,
                        frame_type: frame_type_val,
                        reason,
                        is_application,
                    },
                    off,
                ))
            }

            _ => Err(QuicError::InvalidFrame),
        }
    }

    /// Returns true if this frame is ack-eliciting.
    pub fn is_ack_eliciting(&self) -> bool {
        !matches!(self, Self::Ack { .. } | Self::Padding)
    }
}

// ---------------------------------------------------------------------------
// Connection Management
// ---------------------------------------------------------------------------

/// QUIC connection state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Handshake,
    Connected,
    Closing,
    Draining,
    Closed,
}

/// Packet number space state.
#[derive(Debug, Clone)]
pub struct PnSpace {
    /// Next packet number to send
    pub next_pn: u64,
    /// Largest acknowledged packet number
    pub largest_acked: u64,
    /// Largest packet number received from peer
    pub largest_received: u64,
    /// Sent packets awaiting acknowledgement: pn -> (sent_time_us,
    /// ack_eliciting, size)
    pub sent_packets: BTreeMap<u64, SentPacketInfo>,
    /// Whether we owe the peer an ACK
    pub ack_pending: bool,
}

/// Metadata about a sent packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentPacketInfo {
    pub sent_time_us: u64,
    pub ack_eliciting: bool,
    pub size: usize,
}

impl Default for PnSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl PnSpace {
    pub fn new() -> Self {
        Self {
            next_pn: 0,
            largest_acked: 0,
            largest_received: 0,
            sent_packets: BTreeMap::new(),
            ack_pending: false,
        }
    }

    /// Allocate the next packet number.
    pub fn alloc_pn(&mut self) -> u64 {
        let pn = self.next_pn;
        self.next_pn += 1;
        pn
    }

    /// Record a sent packet.
    pub fn on_packet_sent(&mut self, pn: u64, sent_time_us: u64, ack_eliciting: bool, size: usize) {
        self.sent_packets.insert(
            pn,
            SentPacketInfo {
                sent_time_us,
                ack_eliciting,
                size,
            },
        );
    }

    /// Record a received packet number from peer.
    pub fn on_packet_received(&mut self, pn: u64, ack_eliciting: bool) {
        if pn > self.largest_received {
            self.largest_received = pn;
        }
        if ack_eliciting {
            self.ack_pending = true;
        }
    }
}

/// QUIC connection.
#[derive(Debug)]
pub struct QuicConnection {
    pub state: ConnectionState,
    pub src_cid: ConnectionId,
    pub dst_cid: ConnectionId,
    /// Active connection IDs (for rotation)
    pub active_cids: Vec<ConnectionId>,
    /// Packet number spaces
    pub pn_spaces: [PnSpace; 3],
    /// Idle timeout in milliseconds
    pub idle_timeout_ms: u64,
    /// Last activity timestamp in microseconds
    pub last_activity_us: u64,
    /// RTT estimation
    pub rtt: RttEstimator,
    /// Connection-level flow control: maximum data we can send
    pub max_data_send: u64,
    /// Connection-level flow control: maximum data peer can send
    pub max_data_recv: u64,
    /// Total data sent
    pub data_sent: u64,
    /// Total data received
    pub data_received: u64,
    /// Stream manager
    pub streams: StreamManager,
    /// Path validation state
    pub path_challenge_data: Option<[u8; 8]>,
}

impl QuicConnection {
    /// Create a new QUIC connection (client side).
    pub fn new_client(src_cid: ConnectionId, dst_cid: ConnectionId) -> Self {
        Self {
            state: ConnectionState::Idle,
            src_cid: src_cid.clone(),
            dst_cid,
            active_cids: alloc::vec![src_cid],
            pn_spaces: [PnSpace::new(), PnSpace::new(), PnSpace::new()],
            idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
            last_activity_us: 0,
            rtt: RttEstimator::new(),
            max_data_send: DEFAULT_INITIAL_MAX_DATA,
            max_data_recv: DEFAULT_INITIAL_MAX_DATA,
            data_sent: 0,
            data_received: 0,
            streams: StreamManager::new(true),
            path_challenge_data: None,
        }
    }

    /// Create a new QUIC connection (server side).
    pub fn new_server(src_cid: ConnectionId, dst_cid: ConnectionId) -> Self {
        Self {
            state: ConnectionState::Idle,
            src_cid: src_cid.clone(),
            dst_cid,
            active_cids: alloc::vec![src_cid],
            pn_spaces: [PnSpace::new(), PnSpace::new(), PnSpace::new()],
            idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
            last_activity_us: 0,
            rtt: RttEstimator::new(),
            max_data_send: DEFAULT_INITIAL_MAX_DATA,
            max_data_recv: DEFAULT_INITIAL_MAX_DATA,
            data_sent: 0,
            data_received: 0,
            streams: StreamManager::new(false),
            path_challenge_data: None,
        }
    }

    /// Transition to a new connection state.
    pub fn transition(&mut self, new_state: ConnectionState) -> QuicResult<()> {
        let valid = matches!(
            (self.state, new_state),
            (ConnectionState::Idle, ConnectionState::Handshake)
                | (ConnectionState::Handshake, ConnectionState::Connected)
                | (ConnectionState::Handshake, ConnectionState::Closing)
                | (ConnectionState::Connected, ConnectionState::Closing)
                | (ConnectionState::Connected, ConnectionState::Draining)
                | (ConnectionState::Closing, ConnectionState::Draining)
                | (ConnectionState::Closing, ConnectionState::Closed)
                | (ConnectionState::Draining, ConnectionState::Closed)
        );
        if !valid {
            return Err(QuicError::ProtocolViolation);
        }
        self.state = new_state;
        Ok(())
    }

    /// Get the packet number space for a given space index.
    pub fn pn_space(&self, space: PacketNumberSpace) -> &PnSpace {
        &self.pn_spaces[space as usize]
    }

    /// Get the packet number space mutably.
    pub fn pn_space_mut(&mut self, space: PacketNumberSpace) -> &mut PnSpace {
        &mut self.pn_spaces[space as usize]
    }

    /// Check if the connection has timed out.
    pub fn is_idle_timeout(&self, now_us: u64) -> bool {
        if self.last_activity_us == 0 {
            return false;
        }
        let elapsed_ms = (now_us.saturating_sub(self.last_activity_us)) / 1000;
        elapsed_ms >= self.idle_timeout_ms
    }

    /// Update last activity timestamp.
    pub fn touch(&mut self, now_us: u64) {
        self.last_activity_us = now_us;
    }

    /// Rotate connection ID: add a new CID and optionally retire old ones.
    pub fn add_connection_id(&mut self, cid: ConnectionId) {
        self.active_cids.push(cid);
    }

    /// Retire connection IDs with sequence numbers below `retire_prior_to`.
    pub fn retire_connection_ids(&mut self, retire_prior_to: usize) {
        if retire_prior_to < self.active_cids.len() {
            self.active_cids.drain(0..retire_prior_to);
        }
    }

    /// Initiate path validation by sending PATH_CHALLENGE.
    pub fn initiate_path_challenge(&mut self, challenge_data: [u8; 8]) -> QuicFrame {
        self.path_challenge_data = Some(challenge_data);
        QuicFrame::PathChallenge {
            data: challenge_data,
        }
    }

    /// Validate path response.
    pub fn validate_path_response(&mut self, response_data: &[u8; 8]) -> bool {
        if let Some(expected) = self.path_challenge_data {
            if *response_data == expected {
                self.path_challenge_data = None;
                return true;
            }
        }
        false
    }

    /// Check connection-level send flow control.
    pub fn can_send(&self, bytes: u64) -> bool {
        self.data_sent + bytes <= self.max_data_send
    }

    /// Update connection-level max data (received MAX_DATA from peer).
    pub fn update_max_data_send(&mut self, max_data: u64) {
        if max_data > self.max_data_send {
            self.max_data_send = max_data;
        }
    }
}

// ---------------------------------------------------------------------------
// RTT Estimation
// ---------------------------------------------------------------------------

/// RTT estimator using Jacobson's algorithm (RFC 6298), integer-only.
///
/// SRTT and RTTVAR are stored shifted for fixed-point precision.
#[derive(Debug, Clone)]
pub struct RttEstimator {
    /// Smoothed RTT in microseconds (shifted left by SRTT_SHIFT)
    srtt_shifted: u64,
    /// RTT variance in microseconds (shifted left by RTTVAR_SHIFT)
    rttvar_shifted: u64,
    /// Minimum RTT observed (microseconds)
    pub min_rtt: u64,
    /// Latest RTT sample (microseconds)
    pub latest_rtt: u64,
    /// Whether first sample has been received
    initialized: bool,
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl RttEstimator {
    pub fn new() -> Self {
        Self {
            srtt_shifted: INITIAL_RTT_US << SRTT_SHIFT,
            rttvar_shifted: (INITIAL_RTT_US / 2) << RTTVAR_SHIFT,
            min_rtt: u64::MAX,
            latest_rtt: INITIAL_RTT_US,
            initialized: false,
        }
    }

    /// Update RTT estimate with a new sample.
    pub fn update(&mut self, rtt_sample_us: u64) {
        self.latest_rtt = rtt_sample_us;
        if rtt_sample_us < self.min_rtt {
            self.min_rtt = rtt_sample_us;
        }

        if !self.initialized {
            self.srtt_shifted = rtt_sample_us << SRTT_SHIFT;
            self.rttvar_shifted = (rtt_sample_us / 2) << RTTVAR_SHIFT;
            self.initialized = true;
            return;
        }

        // RTTVAR = (1 - beta) * RTTVAR + beta * |SRTT - R|
        // beta = 1/4 => shift by RTTVAR_SHIFT
        let srtt = self.smoothed_rtt();
        let abs_diff = rtt_sample_us.abs_diff(srtt);
        // rttvar_shifted = rttvar_shifted - (rttvar_shifted >> RTTVAR_SHIFT) + abs_diff
        self.rttvar_shifted =
            self.rttvar_shifted - (self.rttvar_shifted >> RTTVAR_SHIFT) + abs_diff;

        // SRTT = (1 - alpha) * SRTT + alpha * R
        // alpha = 1/8 => shift by SRTT_SHIFT
        self.srtt_shifted = self.srtt_shifted - (self.srtt_shifted >> SRTT_SHIFT) + rtt_sample_us;
    }

    /// Get smoothed RTT in microseconds.
    pub fn smoothed_rtt(&self) -> u64 {
        self.srtt_shifted >> SRTT_SHIFT
    }

    /// Get RTT variance in microseconds.
    pub fn rttvar(&self) -> u64 {
        self.rttvar_shifted >> RTTVAR_SHIFT
    }

    /// Calculate PTO (Probe Timeout) in microseconds.
    ///
    /// PTO = 2 * smoothed_RTT + max(4 * rttvar, 1ms)
    pub fn pto(&self) -> u64 {
        let srtt = self.smoothed_rtt();
        let rttvar = self.rttvar();
        let var_component = if rttvar * 4 > PTO_MIN_US {
            rttvar * 4
        } else {
            PTO_MIN_US
        };
        srtt * 2 + var_component
    }

    /// Calculate the loss detection time threshold in microseconds.
    ///
    /// Time threshold = max(9/8 * max(smoothed_rtt, latest_rtt), 1ms)
    pub fn loss_time_threshold(&self) -> u64 {
        let base = if self.smoothed_rtt() > self.latest_rtt {
            self.smoothed_rtt()
        } else {
            self.latest_rtt
        };
        let threshold = (base * TIME_THRESHOLD_NUM) / TIME_THRESHOLD_DEN;
        if threshold < PTO_MIN_US {
            PTO_MIN_US
        } else {
            threshold
        }
    }
}

// ---------------------------------------------------------------------------
// Stream Multiplexing
// ---------------------------------------------------------------------------

/// Stream type classification from stream ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// Client-initiated bidirectional (ID & 0x03 == 0x00)
    ClientBidi,
    /// Server-initiated bidirectional (ID & 0x03 == 0x01)
    ServerBidi,
    /// Client-initiated unidirectional (ID & 0x03 == 0x02)
    ClientUni,
    /// Server-initiated unidirectional (ID & 0x03 == 0x03)
    ServerUni,
}

impl StreamType {
    /// Classify a stream ID.
    pub fn from_id(stream_id: u64) -> Self {
        match stream_id & 0x03 {
            0x00 => Self::ClientBidi,
            0x01 => Self::ServerBidi,
            0x02 => Self::ClientUni,
            0x03 => Self::ServerUni,
            _ => unreachable!(),
        }
    }

    /// Whether this stream type is bidirectional.
    pub fn is_bidirectional(self) -> bool {
        matches!(self, Self::ClientBidi | Self::ServerBidi)
    }

    /// Whether this stream type is initiated by the client.
    pub fn is_client_initiated(self) -> bool {
        matches!(self, Self::ClientBidi | Self::ClientUni)
    }
}

/// Stream state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
}

/// A single QUIC stream.
#[derive(Debug, Clone)]
pub struct QuicStream {
    pub id: u64,
    pub state: StreamState,
    pub stream_type: StreamType,
    /// Send buffer
    pub send_buf: Vec<u8>,
    /// Send offset (next byte to send)
    pub send_offset: u64,
    /// Receive buffer
    pub recv_buf: Vec<u8>,
    /// Receive offset (next expected byte)
    pub recv_offset: u64,
    /// Max data we can send on this stream
    pub max_send_data: u64,
    /// Max data peer can send on this stream
    pub max_recv_data: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Total bytes received
    pub bytes_received: u64,
    /// Priority weight (higher = more important)
    pub priority: u8,
    /// Whether FIN has been sent
    pub fin_sent: bool,
    /// Whether FIN has been received
    pub fin_received: bool,
}

impl QuicStream {
    pub fn new(id: u64, max_send_data: u64, max_recv_data: u64) -> Self {
        Self {
            id,
            state: StreamState::Idle,
            stream_type: StreamType::from_id(id),
            send_buf: Vec::new(),
            send_offset: 0,
            recv_buf: Vec::new(),
            recv_offset: 0,
            max_send_data,
            max_recv_data,
            bytes_sent: 0,
            bytes_received: 0,
            priority: 128, // default middle priority
            fin_sent: false,
            fin_received: false,
        }
    }

    /// Transition stream state.
    pub fn transition(&mut self, new_state: StreamState) -> QuicResult<()> {
        let valid = matches!(
            (self.state, new_state),
            (StreamState::Idle, StreamState::Open)
                | (StreamState::Open, StreamState::HalfClosedLocal)
                | (StreamState::Open, StreamState::HalfClosedRemote)
                | (StreamState::Open, StreamState::Closed)
                | (StreamState::HalfClosedLocal, StreamState::Closed)
                | (StreamState::HalfClosedRemote, StreamState::Closed)
        );
        if !valid {
            return Err(QuicError::StreamStateError);
        }
        self.state = new_state;
        Ok(())
    }

    /// Write data to the send buffer.
    pub fn write(&mut self, data: &[u8]) -> QuicResult<usize> {
        if self.state == StreamState::HalfClosedLocal || self.state == StreamState::Closed {
            return Err(QuicError::StreamStateError);
        }
        let available = self.max_send_data.saturating_sub(self.bytes_sent) as usize;
        let to_write = if data.len() < available {
            data.len()
        } else {
            available
        };
        if to_write == 0 {
            return Err(QuicError::FlowControlError);
        }
        self.send_buf.extend_from_slice(&data[..to_write]);
        Ok(to_write)
    }

    /// Read data from the receive buffer.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = if buf.len() < self.recv_buf.len() {
            buf.len()
        } else {
            self.recv_buf.len()
        };
        buf[..to_read].copy_from_slice(&self.recv_buf[..to_read]);
        self.recv_buf.drain(..to_read);
        to_read
    }

    /// Receive data from a STREAM frame.
    pub fn receive_data(&mut self, offset: u64, data: &[u8], fin: bool) -> QuicResult<()> {
        if self.state == StreamState::HalfClosedRemote || self.state == StreamState::Closed {
            return Err(QuicError::StreamStateError);
        }

        let new_bytes = offset.saturating_add(data.len() as u64);
        if new_bytes > self.max_recv_data {
            return Err(QuicError::FlowControlError);
        }

        // Simple in-order receive (for now, ignore out-of-order/gaps)
        if offset == self.recv_offset {
            self.recv_buf.extend_from_slice(data);
            self.recv_offset += data.len() as u64;
            self.bytes_received += data.len() as u64;
        }

        if fin {
            self.fin_received = true;
            if self.state == StreamState::Open {
                self.state = StreamState::HalfClosedRemote;
            } else if self.state == StreamState::HalfClosedLocal {
                self.state = StreamState::Closed;
            }
        }

        Ok(())
    }

    /// Check if send flow control allows sending `bytes` more.
    pub fn can_send(&self, bytes: u64) -> bool {
        self.bytes_sent + bytes <= self.max_send_data
    }

    /// Update max send data (received MAX_STREAM_DATA from peer).
    pub fn update_max_send_data(&mut self, max_data: u64) {
        if max_data > self.max_send_data {
            self.max_send_data = max_data;
        }
    }
}

/// Manages all streams for a connection.
#[derive(Debug)]
pub struct StreamManager {
    pub streams: BTreeMap<u64, QuicStream>,
    /// Whether this is the client side
    pub is_client: bool,
    /// Next client-initiated bidirectional stream ID
    pub next_client_bidi: u64,
    /// Next server-initiated bidirectional stream ID
    pub next_server_bidi: u64,
    /// Next client-initiated unidirectional stream ID
    pub next_client_uni: u64,
    /// Next server-initiated unidirectional stream ID
    pub next_server_uni: u64,
    /// Default max stream data for new streams
    pub default_max_stream_data: u64,
}

impl StreamManager {
    pub fn new(is_client: bool) -> Self {
        Self {
            streams: BTreeMap::new(),
            is_client,
            next_client_bidi: 0, // 0x00, 0x04, 0x08, ...
            next_server_bidi: 1, // 0x01, 0x05, 0x09, ...
            next_client_uni: 2,  // 0x02, 0x06, 0x0A, ...
            next_server_uni: 3,  // 0x03, 0x07, 0x0B, ...
            default_max_stream_data: DEFAULT_INITIAL_MAX_STREAM_DATA,
        }
    }

    /// Open a new bidirectional stream, returning its ID.
    pub fn open_bidi_stream(&mut self) -> u64 {
        let id = if self.is_client {
            let id = self.next_client_bidi;
            self.next_client_bidi += 4;
            id
        } else {
            let id = self.next_server_bidi;
            self.next_server_bidi += 4;
            id
        };
        let mut stream = QuicStream::new(
            id,
            self.default_max_stream_data,
            self.default_max_stream_data,
        );
        stream.state = StreamState::Open;
        self.streams.insert(id, stream);
        id
    }

    /// Open a new unidirectional stream, returning its ID.
    pub fn open_uni_stream(&mut self) -> u64 {
        let id = if self.is_client {
            let id = self.next_client_uni;
            self.next_client_uni += 4;
            id
        } else {
            let id = self.next_server_uni;
            self.next_server_uni += 4;
            id
        };
        let mut stream = QuicStream::new(
            id,
            self.default_max_stream_data,
            self.default_max_stream_data,
        );
        stream.state = StreamState::Open;
        self.streams.insert(id, stream);
        id
    }

    /// Get a stream by ID, creating it if it was initiated by the peer.
    pub fn get_or_create(&mut self, stream_id: u64) -> &mut QuicStream {
        if !self.streams.contains_key(&stream_id) {
            let mut stream = QuicStream::new(
                stream_id,
                self.default_max_stream_data,
                self.default_max_stream_data,
            );
            stream.state = StreamState::Open;
            self.streams.insert(stream_id, stream);
        }
        self.streams.get_mut(&stream_id).unwrap()
    }

    /// Get a stream by ID.
    pub fn get(&self, stream_id: u64) -> Option<&QuicStream> {
        self.streams.get(&stream_id)
    }

    /// Get a mutable stream by ID.
    pub fn get_mut(&mut self, stream_id: u64) -> Option<&mut QuicStream> {
        self.streams.get_mut(&stream_id)
    }

    /// Close a stream.
    pub fn close_stream(&mut self, stream_id: u64) -> QuicResult<()> {
        let stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or(QuicError::StreamNotFound)?;
        match stream.state {
            StreamState::Open => stream.state = StreamState::Closed,
            StreamState::HalfClosedLocal | StreamState::HalfClosedRemote => {
                stream.state = StreamState::Closed
            }
            _ => return Err(QuicError::StreamStateError),
        }
        Ok(())
    }

    /// Number of active (non-closed) streams.
    pub fn active_count(&self) -> usize {
        self.streams
            .values()
            .filter(|s| s.state != StreamState::Closed && s.state != StreamState::Idle)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Loss Detection & Recovery
// ---------------------------------------------------------------------------

/// Loss detection state for a connection.
#[derive(Debug, Clone)]
pub struct LossDetector {
    /// Loss detection timer expiry (microseconds, 0 = not set)
    pub loss_detection_timer: u64,
    /// PTO count (doubles on each consecutive PTO)
    pub pto_count: u32,
    /// Time of last ack-eliciting packet sent (per space)
    pub time_of_last_ack_eliciting: [u64; 3],
}

impl Default for LossDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LossDetector {
    pub fn new() -> Self {
        Self {
            loss_detection_timer: 0,
            pto_count: 0,
            time_of_last_ack_eliciting: [0; 3],
        }
    }

    /// Detect lost packets in the given packet number space.
    ///
    /// Returns a list of lost packet numbers.
    pub fn detect_lost_packets(pn_space: &PnSpace, rtt: &RttEstimator, now_us: u64) -> Vec<u64> {
        let mut lost = Vec::new();
        let largest_acked = pn_space.largest_acked;
        let loss_delay = rtt.loss_time_threshold();

        for (&pn, info) in &pn_space.sent_packets {
            if pn > largest_acked {
                continue;
            }

            // Packet threshold: lost if more than PACKET_THRESHOLD packets
            // have been acknowledged after it
            let pkt_threshold_lost = largest_acked.saturating_sub(pn) >= PACKET_THRESHOLD;

            // Time threshold: lost if enough time has passed
            let time_threshold_lost = now_us.saturating_sub(info.sent_time_us) >= loss_delay;

            if pkt_threshold_lost || time_threshold_lost {
                lost.push(pn);
            }
        }

        lost
    }

    /// Calculate PTO with exponential backoff.
    pub fn compute_pto(&self, rtt: &RttEstimator) -> u64 {
        let base_pto = rtt.pto();
        // Exponential backoff: PTO * 2^pto_count
        base_pto.checked_shl(self.pto_count).unwrap_or(u64::MAX)
    }

    /// Record that an ack-eliciting packet was sent.
    pub fn on_ack_eliciting_sent(&mut self, space: PacketNumberSpace, now_us: u64) {
        self.time_of_last_ack_eliciting[space as usize] = now_us;
    }

    /// Process an ACK and update loss detection state.
    ///
    /// Returns (newly_acked_packets, rtt_sample_us).
    pub fn on_ack_received(
        pn_space: &mut PnSpace,
        largest_acked: u64,
        ack_delay_us: u64,
        now_us: u64,
    ) -> (Vec<u64>, Option<u64>) {
        let mut newly_acked = Vec::new();
        let mut rtt_sample = None;

        if largest_acked > pn_space.largest_acked {
            pn_space.largest_acked = largest_acked;
        }

        // Find all acked packets up to largest_acked
        let acked_pns: Vec<u64> = pn_space
            .sent_packets
            .keys()
            .copied()
            .filter(|&pn| pn <= largest_acked)
            .collect();

        for pn in acked_pns {
            if let Some(info) = pn_space.sent_packets.remove(&pn) {
                newly_acked.push(pn);
                // RTT sample from the largest newly acknowledged packet
                if pn == largest_acked {
                    let raw_rtt = now_us.saturating_sub(info.sent_time_us);
                    // Adjust for ACK delay (but don't let adjusted RTT go below min_rtt)
                    rtt_sample = Some(raw_rtt.saturating_sub(ack_delay_us));
                }
            }
        }

        (newly_acked, rtt_sample)
    }

    /// Reset PTO count (called when ack received).
    pub fn reset_pto(&mut self) {
        self.pto_count = 0;
    }

    /// Increment PTO count (called on PTO timeout).
    pub fn on_pto_timeout(&mut self) {
        self.pto_count += 1;
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

    // --- Variable-length integer tests ---

    #[test]
    fn test_varint_encode_decode_1byte() {
        let mut buf = [0u8; 8];
        let n = encode_varint(37, &mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 37);
        let (val, consumed) = decode_varint(&buf).unwrap();
        assert_eq!(val, 37);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_varint_encode_decode_2byte() {
        let mut buf = [0u8; 8];
        let n = encode_varint(15293, &mut buf).unwrap();
        assert_eq!(n, 2);
        let (val, consumed) = decode_varint(&buf).unwrap();
        assert_eq!(val, 15293);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn test_varint_encode_decode_4byte() {
        let mut buf = [0u8; 8];
        let n = encode_varint(494878333, &mut buf).unwrap();
        assert_eq!(n, 4);
        let (val, consumed) = decode_varint(&buf).unwrap();
        assert_eq!(val, 494878333);
        assert_eq!(consumed, 4);
    }

    #[test]
    fn test_varint_encode_decode_8byte() {
        let mut buf = [0u8; 8];
        let n = encode_varint(151_288_809_941_952_652, &mut buf).unwrap();
        assert_eq!(n, 8);
        let (val, consumed) = decode_varint(&buf).unwrap();
        assert_eq!(val, 151_288_809_941_952_652);
        assert_eq!(consumed, 8);
    }

    #[test]
    fn test_varint_boundary_values() {
        let mut buf = [0u8; 8];

        // Max 1-byte value
        let n = encode_varint(63, &mut buf).unwrap();
        assert_eq!(n, 1);
        let (val, _) = decode_varint(&buf).unwrap();
        assert_eq!(val, 63);

        // Min 2-byte value
        let n = encode_varint(64, &mut buf).unwrap();
        assert_eq!(n, 2);
        let (val, _) = decode_varint(&buf).unwrap();
        assert_eq!(val, 64);

        // Max 2-byte value
        let n = encode_varint(16383, &mut buf).unwrap();
        assert_eq!(n, 2);
        let (val, _) = decode_varint(&buf).unwrap();
        assert_eq!(val, 16383);

        // Min 4-byte value
        let n = encode_varint(16384, &mut buf).unwrap();
        assert_eq!(n, 4);
        let (val, _) = decode_varint(&buf).unwrap();
        assert_eq!(val, 16384);
    }

    #[test]
    fn test_varint_zero() {
        let mut buf = [0u8; 8];
        let n = encode_varint(0, &mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0);
        let (val, consumed) = decode_varint(&buf).unwrap();
        assert_eq!(val, 0);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_varint_buffer_too_small() {
        let mut buf = [0u8; 1];
        assert_eq!(
            encode_varint(16384, &mut buf),
            Err(QuicError::BufferTooSmall)
        );
    }

    // --- Long header tests ---

    #[test]
    fn test_long_header_initial_roundtrip() {
        let src_cid = ConnectionId::new(&[0x01, 0x02, 0x03, 0x04]).unwrap();
        let dst_cid = ConnectionId::new(&[0x05, 0x06, 0x07, 0x08]).unwrap();
        let hdr = LongHeader {
            first_byte: 0xC0,
            version: QUIC_V1,
            dst_cid: dst_cid.clone(),
            src_cid: src_cid.clone(),
            packet_type: LongPacketType::Initial,
            token: vec![0xAA, 0xBB],
            packet_number: 42,
            payload_length: 100,
        };

        let mut buf = [0u8; 256];
        let written = hdr.encode(&mut buf).unwrap();
        assert!(written > 0);

        let (decoded, consumed) = LongHeader::decode(&buf[..written]).unwrap();
        assert_eq!(decoded.version, QUIC_V1);
        assert_eq!(decoded.packet_type, LongPacketType::Initial);
        assert_eq!(decoded.dst_cid, dst_cid);
        assert_eq!(decoded.src_cid, src_cid);
        assert_eq!(decoded.token, vec![0xAA, 0xBB]);
        assert_eq!(decoded.packet_number, 42);
        assert_eq!(consumed, written);
    }

    #[test]
    fn test_long_header_handshake() {
        let src_cid = ConnectionId::new(&[0x10]).unwrap();
        let dst_cid = ConnectionId::new(&[0x20]).unwrap();
        let hdr = LongHeader {
            first_byte: 0xC0,
            version: QUIC_V1,
            dst_cid: dst_cid.clone(),
            src_cid: src_cid.clone(),
            packet_type: LongPacketType::Handshake,
            token: Vec::new(),
            packet_number: 0,
            payload_length: 50,
        };

        let mut buf = [0u8; 128];
        let written = hdr.encode(&mut buf).unwrap();
        let (decoded, _) = LongHeader::decode(&buf[..written]).unwrap();
        assert_eq!(decoded.packet_type, LongPacketType::Handshake);
        assert_eq!(decoded.packet_number, 0);
    }

    // --- Short header tests ---

    #[test]
    fn test_short_header_construction() {
        let dst_cid = ConnectionId::new(&[0x01, 0x02, 0x03, 0x04]).unwrap();
        let hdr = ShortHeader::new(dst_cid.clone(), 100, true, false, 90);

        assert!(hdr.spin_bit);
        assert!(!hdr.key_phase);
        assert_eq!(hdr.first_byte & 0x80, 0); // short header form bit = 0
        assert_eq!(hdr.first_byte & 0x40, 0x40); // fixed bit = 1
        assert!(hdr.first_byte & 0x20 != 0); // spin bit set
    }

    #[test]
    fn test_short_header_encode() {
        let dst_cid = ConnectionId::new(&[0xAB, 0xCD]).unwrap();
        let hdr = ShortHeader::new(dst_cid, 5, false, true, 0);
        let mut buf = [0u8; 32];
        let written = hdr.encode(&mut buf, 0).unwrap();
        assert!(written >= 4); // 1 (first) + 2 (cid) + 1 (pn)
    }

    // --- ACK frame tests ---

    #[test]
    fn test_ack_frame_roundtrip() {
        let frame = QuicFrame::Ack {
            largest_acked: 100,
            ack_delay: 50,
            first_ack_range: 10,
            ack_ranges: vec![AckRange {
                gap: 5,
                ack_range_length: 3,
            }],
            ecn_counts: None,
        };

        let mut buf = [0u8; 128];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, written);

        match decoded {
            QuicFrame::Ack {
                largest_acked,
                ack_delay,
                first_ack_range,
                ack_ranges,
                ecn_counts,
            } => {
                assert_eq!(largest_acked, 100);
                assert_eq!(ack_delay, 50);
                assert_eq!(first_ack_range, 10);
                assert_eq!(ack_ranges.len(), 1);
                assert_eq!(ack_ranges[0].gap, 5);
                assert_eq!(ack_ranges[0].ack_range_length, 3);
                assert!(ecn_counts.is_none());
            }
            _ => panic!("expected ACK frame"),
        }
    }

    // --- STREAM frame tests ---

    #[test]
    fn test_stream_frame_roundtrip() {
        let frame = QuicFrame::Stream {
            stream_id: 4,
            offset: 100,
            data: vec![0x01, 0x02, 0x03],
            fin: false,
        };

        let mut buf = [0u8; 128];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, written);

        match decoded {
            QuicFrame::Stream {
                stream_id,
                offset,
                data,
                fin,
            } => {
                assert_eq!(stream_id, 4);
                assert_eq!(offset, 100);
                assert_eq!(data, vec![0x01, 0x02, 0x03]);
                assert!(!fin);
            }
            _ => panic!("expected STREAM frame"),
        }
    }

    #[test]
    fn test_stream_frame_with_fin() {
        let frame = QuicFrame::Stream {
            stream_id: 8,
            offset: 0,
            data: vec![0xFF],
            fin: true,
        };

        let mut buf = [0u8; 64];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, _) = QuicFrame::decode(&buf[..written]).unwrap();

        match decoded {
            QuicFrame::Stream { fin, .. } => assert!(fin),
            _ => panic!("expected STREAM frame"),
        }
    }

    // --- CRYPTO frame tests ---

    #[test]
    fn test_crypto_frame_roundtrip() {
        let frame = QuicFrame::Crypto {
            offset: 0,
            data: vec![0x16, 0x03, 0x03, 0x00, 0x01], // TLS-like data
        };

        let mut buf = [0u8; 64];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, written);

        match decoded {
            QuicFrame::Crypto { offset, data } => {
                assert_eq!(offset, 0);
                assert_eq!(data, vec![0x16, 0x03, 0x03, 0x00, 0x01]);
            }
            _ => panic!("expected CRYPTO frame"),
        }
    }

    // --- CONNECTION_CLOSE frame tests ---

    #[test]
    fn test_connection_close_transport() {
        let frame = QuicFrame::ConnectionClose {
            error_code: QuicError::FlowControlError.as_u64(),
            frame_type: Some(0x08),
            reason: vec![0x62, 0x61, 0x64], // "bad"
            is_application: false,
        };

        let mut buf = [0u8; 64];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, consumed) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(consumed, written);

        match decoded {
            QuicFrame::ConnectionClose {
                error_code,
                frame_type,
                reason,
                is_application,
            } => {
                assert_eq!(error_code, 0x03);
                assert_eq!(frame_type, Some(0x08));
                assert_eq!(reason, vec![0x62, 0x61, 0x64]);
                assert!(!is_application);
            }
            _ => panic!("expected CONNECTION_CLOSE frame"),
        }
    }

    #[test]
    fn test_connection_close_application() {
        let frame = QuicFrame::ConnectionClose {
            error_code: 42,
            frame_type: None,
            reason: Vec::new(),
            is_application: true,
        };

        let mut buf = [0u8; 32];
        let written = frame.encode(&mut buf).unwrap();
        let (decoded, _) = QuicFrame::decode(&buf[..written]).unwrap();

        match decoded {
            QuicFrame::ConnectionClose {
                is_application,
                error_code,
                ..
            } => {
                assert!(is_application);
                assert_eq!(error_code, 42);
            }
            _ => panic!("expected CONNECTION_CLOSE frame"),
        }
    }

    // --- Stream ID classification tests ---

    #[test]
    fn test_stream_id_classification() {
        // Client-initiated bidirectional: 0, 4, 8, ...
        assert_eq!(StreamType::from_id(0), StreamType::ClientBidi);
        assert_eq!(StreamType::from_id(4), StreamType::ClientBidi);
        assert!(StreamType::ClientBidi.is_bidirectional());
        assert!(StreamType::ClientBidi.is_client_initiated());

        // Server-initiated bidirectional: 1, 5, 9, ...
        assert_eq!(StreamType::from_id(1), StreamType::ServerBidi);
        assert_eq!(StreamType::from_id(5), StreamType::ServerBidi);
        assert!(StreamType::ServerBidi.is_bidirectional());
        assert!(!StreamType::ServerBidi.is_client_initiated());

        // Client-initiated unidirectional: 2, 6, 10, ...
        assert_eq!(StreamType::from_id(2), StreamType::ClientUni);
        assert!(!StreamType::ClientUni.is_bidirectional());
        assert!(StreamType::ClientUni.is_client_initiated());

        // Server-initiated unidirectional: 3, 7, 11, ...
        assert_eq!(StreamType::from_id(3), StreamType::ServerUni);
        assert!(!StreamType::ServerUni.is_bidirectional());
        assert!(!StreamType::ServerUni.is_client_initiated());
    }

    // --- Stream state transition tests ---

    #[test]
    fn test_stream_state_transitions() {
        let mut stream = QuicStream::new(0, 65536, 65536);
        assert_eq!(stream.state, StreamState::Idle);

        stream.transition(StreamState::Open).unwrap();
        assert_eq!(stream.state, StreamState::Open);

        stream.transition(StreamState::HalfClosedLocal).unwrap();
        assert_eq!(stream.state, StreamState::HalfClosedLocal);

        stream.transition(StreamState::Closed).unwrap();
        assert_eq!(stream.state, StreamState::Closed);
    }

    #[test]
    fn test_stream_invalid_transition() {
        let mut stream = QuicStream::new(0, 65536, 65536);
        // Cannot go directly from Idle to Closed
        assert_eq!(
            stream.transition(StreamState::Closed),
            Err(QuicError::StreamStateError)
        );
    }

    // --- Connection state transition tests ---

    #[test]
    fn test_connection_state_transitions() {
        let src = ConnectionId::new(&[1]).unwrap();
        let dst = ConnectionId::new(&[2]).unwrap();
        let mut conn = QuicConnection::new_client(src, dst);
        assert_eq!(conn.state, ConnectionState::Idle);

        conn.transition(ConnectionState::Handshake).unwrap();
        assert_eq!(conn.state, ConnectionState::Handshake);

        conn.transition(ConnectionState::Connected).unwrap();
        assert_eq!(conn.state, ConnectionState::Connected);

        conn.transition(ConnectionState::Closing).unwrap();
        assert_eq!(conn.state, ConnectionState::Closing);

        conn.transition(ConnectionState::Closed).unwrap();
        assert_eq!(conn.state, ConnectionState::Closed);
    }

    #[test]
    fn test_connection_invalid_transition() {
        let src = ConnectionId::new(&[1]).unwrap();
        let dst = ConnectionId::new(&[2]).unwrap();
        let mut conn = QuicConnection::new_client(src, dst);
        // Cannot go from Idle directly to Connected
        assert_eq!(
            conn.transition(ConnectionState::Connected),
            Err(QuicError::ProtocolViolation)
        );
    }

    // --- Packet number encoding/decoding tests ---

    #[test]
    fn test_packet_number_encode_1byte() {
        let (encoded, len) = encode_packet_number(10, 5);
        assert_eq!(len, 1);
        assert_eq!(encoded, 10);
    }

    #[test]
    fn test_packet_number_encode_2byte() {
        let (encoded, len) = encode_packet_number(300, 0);
        assert_eq!(len, 2);
        assert_eq!(encoded, 300);
    }

    #[test]
    fn test_packet_number_decode_roundtrip() {
        let full_pn = 12345u64;
        let largest_acked = 12340u64;
        let (truncated, pn_len) = encode_packet_number(full_pn, largest_acked);
        let decoded = decode_packet_number(largest_acked, truncated, pn_len);
        assert_eq!(decoded, full_pn);
    }

    // --- Flow control tests ---

    #[test]
    fn test_stream_flow_control_window() {
        let mut stream = QuicStream::new(0, 1024, 1024);
        stream.state = StreamState::Open;

        assert!(stream.can_send(512));
        assert!(stream.can_send(1024));
        assert!(!stream.can_send(1025));

        // Write some data
        let written = stream.write(&[0u8; 512]).unwrap();
        assert_eq!(written, 512);
        stream.bytes_sent += written as u64;

        assert!(stream.can_send(512));
        assert!(!stream.can_send(513));
    }

    #[test]
    fn test_connection_flow_control() {
        let src = ConnectionId::new(&[1]).unwrap();
        let dst = ConnectionId::new(&[2]).unwrap();
        let mut conn = QuicConnection::new_client(src, dst);

        assert!(conn.can_send(1024));
        conn.data_sent = DEFAULT_INITIAL_MAX_DATA - 100;
        assert!(conn.can_send(100));
        assert!(!conn.can_send(101));

        // Update max data
        conn.update_max_data_send(DEFAULT_INITIAL_MAX_DATA + 1000);
        assert!(conn.can_send(1100));
    }

    // --- Loss detection tests ---

    #[test]
    fn test_loss_detection_packet_threshold() {
        let mut pn_space = PnSpace::new();
        let rtt = RttEstimator::new();

        // Send packets 0..5
        for pn in 0..5 {
            pn_space.on_packet_sent(pn, pn * 1000, true, 100);
        }

        // ACK packet 4 (largest)
        pn_space.largest_acked = 4;

        let now_us = 100_000;
        let lost = LossDetector::detect_lost_packets(&pn_space, &rtt, now_us);

        // Packets 0 and 1 are lost (4 - 0 >= 3, 4 - 1 >= 3)
        assert!(lost.contains(&0));
        assert!(lost.contains(&1));
        // Packet 2 is borderline (4 - 2 = 2 < 3) unless time threshold triggers
    }

    #[test]
    fn test_pto_calculation() {
        let rtt = RttEstimator::new();
        let pto = rtt.pto();
        // PTO = 2 * SRTT + max(4 * RTTVAR, 1ms)
        // SRTT = INITIAL_RTT_US = 333000
        // RTTVAR = INITIAL_RTT_US / 2 = 166500
        // PTO = 2 * 333000 + max(4 * 166500, 1000) = 666000 + 666000 = 1332000
        assert_eq!(pto, 1_332_000);
    }

    #[test]
    fn test_pto_exponential_backoff() {
        let rtt = RttEstimator::new();
        let mut detector = LossDetector::new();

        let base_pto = detector.compute_pto(&rtt);
        detector.on_pto_timeout();
        let pto_1 = detector.compute_pto(&rtt);
        assert_eq!(pto_1, base_pto * 2);

        detector.on_pto_timeout();
        let pto_2 = detector.compute_pto(&rtt);
        assert_eq!(pto_2, base_pto * 4);
    }

    // --- Connection ID tests ---

    #[test]
    fn test_connection_id_generate() {
        let cid = ConnectionId::generate(0xDEAD_BEEF, 8).unwrap();
        assert_eq!(cid.len, 8);
        assert_eq!(cid.as_slice().len(), 8);
    }

    #[test]
    fn test_connection_id_empty() {
        let cid = ConnectionId::EMPTY;
        assert_eq!(cid.len, 0);
        assert_eq!(cid.as_slice().len(), 0);
    }

    #[test]
    fn test_connection_id_max_length() {
        let data = [0u8; 20];
        let cid = ConnectionId::new(&data).unwrap();
        assert_eq!(cid.len, 20);

        let too_long = [0u8; 21];
        assert_eq!(
            ConnectionId::new(&too_long),
            Err(QuicError::ConnectionIdLimitError)
        );
    }

    // --- Idle timeout test ---

    #[test]
    fn test_idle_timeout() {
        let src = ConnectionId::new(&[1]).unwrap();
        let dst = ConnectionId::new(&[2]).unwrap();
        let mut conn = QuicConnection::new_client(src, dst);
        conn.idle_timeout_ms = 5000; // 5 seconds

        conn.touch(1_000_000); // 1 second
        assert!(!conn.is_idle_timeout(3_000_000)); // 3 seconds total, 2s elapsed

        // 7 seconds total = 6 seconds elapsed > 5 second timeout
        assert!(conn.is_idle_timeout(7_000_000));
    }

    // --- Path validation tests ---

    #[test]
    fn test_path_challenge_response() {
        let src = ConnectionId::new(&[1]).unwrap();
        let dst = ConnectionId::new(&[2]).unwrap();
        let mut conn = QuicConnection::new_client(src, dst);

        let challenge = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let frame = conn.initiate_path_challenge(challenge);

        match frame {
            QuicFrame::PathChallenge { data } => assert_eq!(data, challenge),
            _ => panic!("expected PATH_CHALLENGE"),
        }

        // Wrong response
        let wrong = [0xFF; 8];
        assert!(!conn.validate_path_response(&wrong));

        // Correct response
        assert!(conn.validate_path_response(&challenge));
        assert!(conn.path_challenge_data.is_none());
    }

    // --- RTT estimation tests ---

    #[test]
    fn test_rtt_first_sample() {
        let mut rtt = RttEstimator::new();
        rtt.update(100_000); // 100ms

        assert_eq!(rtt.smoothed_rtt(), 100_000);
        assert_eq!(rtt.min_rtt, 100_000);
        assert_eq!(rtt.latest_rtt, 100_000);
    }

    #[test]
    fn test_rtt_convergence() {
        let mut rtt = RttEstimator::new();
        // Feed constant RTT samples
        for _ in 0..20 {
            rtt.update(50_000); // 50ms
        }
        // SRTT should converge towards 50ms
        let srtt = rtt.smoothed_rtt();
        assert!(srtt > 45_000 && srtt < 55_000);
    }

    // --- Stream manager tests ---

    #[test]
    fn test_stream_manager_open_bidi() {
        let mut mgr = StreamManager::new(true); // client
        let id1 = mgr.open_bidi_stream();
        assert_eq!(id1, 0); // client bidi: 0, 4, 8, ...
        let id2 = mgr.open_bidi_stream();
        assert_eq!(id2, 4);

        assert_eq!(mgr.active_count(), 2);
    }

    #[test]
    fn test_stream_manager_open_uni() {
        let mut mgr = StreamManager::new(false); // server
        let id1 = mgr.open_uni_stream();
        assert_eq!(id1, 3); // server uni: 3, 7, 11, ...
        let id2 = mgr.open_uni_stream();
        assert_eq!(id2, 7);
    }

    // --- Frame type tests ---

    #[test]
    fn test_padding_frame() {
        let frame = QuicFrame::Padding;
        let mut buf = [0u8; 4];
        let written = frame.encode(&mut buf).unwrap();
        assert_eq!(written, 1);
        let (decoded, _) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(decoded, QuicFrame::Padding);
    }

    #[test]
    fn test_ping_frame() {
        let frame = QuicFrame::Ping;
        let mut buf = [0u8; 4];
        let written = frame.encode(&mut buf).unwrap();
        assert_eq!(written, 1);
        let (decoded, _) = QuicFrame::decode(&buf[..written]).unwrap();
        assert_eq!(decoded, QuicFrame::Ping);
    }

    #[test]
    fn test_ack_eliciting() {
        assert!(!QuicFrame::Padding.is_ack_eliciting());
        assert!(QuicFrame::Ping.is_ack_eliciting());
        assert!(!QuicFrame::Ack {
            largest_acked: 0,
            ack_delay: 0,
            first_ack_range: 0,
            ack_ranges: Vec::new(),
            ecn_counts: None,
        }
        .is_ack_eliciting());
        assert!(QuicFrame::Stream {
            stream_id: 0,
            offset: 0,
            data: Vec::new(),
            fin: false,
        }
        .is_ack_eliciting());
    }

    #[test]
    fn test_header_protection_apply() {
        let mut buf = [0xC0, 0x00, 0x00, 0x00, 0x01, 0x00, 0x42];
        let mask = [0x0F, 0xAA, 0xBB, 0xCC, 0xDD];
        let pn_offset = 6;

        let orig_first = buf[0];
        let orig_pn = buf[6];
        apply_header_protection(&mut buf, pn_offset, &mask);

        // Long header: mask lower 4 bits of first byte
        assert_eq!(buf[0], orig_first ^ (mask[0] & 0x0F));
        // PN byte masked with mask[1] (pn_len derived from first_byte after XOR)
        // After first byte XOR: 0xC0 ^ 0x0F = 0xCF, pn_len = (0xCF & 0x03) + 1 = 4
        // So all 4 PN bytes get masked, but we only have 1 byte at offset 6
        assert_eq!(buf[6], orig_pn ^ mask[1]);
    }

    // --- Varint len tests ---

    #[test]
    fn test_varint_len() {
        assert_eq!(varint_len(0), 1);
        assert_eq!(varint_len(63), 1);
        assert_eq!(varint_len(64), 2);
        assert_eq!(varint_len(16383), 2);
        assert_eq!(varint_len(16384), 4);
        assert_eq!(varint_len(1_073_741_823), 4);
        assert_eq!(varint_len(1_073_741_824), 8);
    }
}
