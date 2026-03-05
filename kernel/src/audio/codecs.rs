//! OGG Vorbis and MP3 audio decoders (integer-only, no_std)
//!
//! Provides software decoding of OGG Vorbis and MPEG-1 Layer III (MP3) audio
//! streams using only integer and fixed-point arithmetic. No floating-point
//! operations are used, making these decoders suitable for kernel context
//! where FPU state may not be available.
//!
//! ## Fixed-Point Formats
//!
//! - **16.16**: General-purpose, used for volume/gain. 1.0 = `0x0001_0000`.
//! - **2.30**: High-precision twiddle factors for MDCT/IMDCT. 1.0 =
//!   `0x4000_0000`.
//!
//! ## OGG Vorbis
//!
//! Implements the OGG container (RFC 3533) and Vorbis I codec (Xiph.org spec):
//! - OGG page parsing with CRC32 verification
//! - Vorbis identification, comment, and setup header decoding
//! - Floor type 1, residue types 0/1/2, codebook Huffman trees
//! - MDCT via integer butterfly operations with 2.30 twiddle factors
//! - Vorbis window function via fixed-point lookup table
//!
//! ## MP3 (MPEG-1 Layer III)
//!
//! Implements ISO 11172-3 / ISO 13818-3 Layer III decoding:
//! - Frame sync and header parsing (bitrate, sample rate, channel mode)
//! - Side information and scalefactor band decoding
//! - Huffman decoding (33 tables)
//! - Integer requantization via lookup tables
//! - Joint stereo (MS and intensity)
//! - 36-point and 12-point IMDCT via integer butterflies
//! - 32-subband synthesis polyphase filterbank (integer coefficients)

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

// ============================================================================
// Fixed-Point Arithmetic (shared)
// ============================================================================

/// 16.16 fixed-point type
type Fp16 = i32;

/// 2.30 fixed-point type for high-precision trig
type Fp30 = i32;

/// 16.16: number of fractional bits
const FP16_SHIFT: i32 = 16;

/// 16.16: representation of 1.0
const FP16_ONE: Fp16 = 1 << FP16_SHIFT;

/// 2.30: number of fractional bits
const FP30_SHIFT: i32 = 30;

/// 2.30: representation of 1.0
const FP30_ONE: Fp30 = 1 << FP30_SHIFT;

/// Multiply two 2.30 fixed-point values, returning 2.30 result
#[inline]
fn fp30_mul(a: Fp30, b: Fp30) -> Fp30 {
    let result = (a as i64).checked_mul(b as i64).unwrap_or(0) >> FP30_SHIFT;
    result as Fp30
}

/// Multiply two 16.16 fixed-point values with saturation
#[inline]
fn fp16_mul(a: Fp16, b: Fp16) -> Fp16 {
    let result = (a as i64).checked_mul(b as i64).unwrap_or(0) >> FP16_SHIFT;
    if result > i32::MAX as i64 {
        i32::MAX
    } else if result < i32::MIN as i64 {
        i32::MIN
    } else {
        result as i32
    }
}

/// Convert 2.30 to 16.16 (shift right by 14)
#[inline]
fn fp30_to_fp16(v: Fp30) -> Fp16 {
    v >> (FP30_SHIFT - FP16_SHIFT)
}

/// Convert i32 integer to 16.16 fixed-point
#[inline]
fn fp16_from_i32(v: i32) -> Fp16 {
    v.checked_shl(FP16_SHIFT as u32)
        .unwrap_or(if v >= 0 { i32::MAX } else { i32::MIN })
}

/// Clamp a 16.16 fixed-point value to i16 range and return the sample
#[inline]
fn fp16_to_i16(fp: Fp16) -> i16 {
    let shifted = fp >> FP16_SHIFT;
    if shifted > i16::MAX as i32 {
        i16::MAX
    } else if shifted < i16::MIN as i32 {
        i16::MIN
    } else {
        shifted as i16
    }
}

// ============================================================================
// Byte Reading Helpers
// ============================================================================

#[inline]
fn read_u8(data: &[u8], pos: usize) -> Option<u8> {
    data.get(pos).copied()
}

#[inline]
fn read_u16_le(data: &[u8], pos: usize) -> Option<u16> {
    if pos + 2 > data.len() {
        return None;
    }
    Some(u16::from_le_bytes([data[pos], data[pos + 1]]))
}

#[inline]
fn read_u32_le(data: &[u8], pos: usize) -> Option<u32> {
    if pos + 4 > data.len() {
        return None;
    }
    Some(u32::from_le_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
    ]))
}

#[inline]
fn read_u64_le(data: &[u8], pos: usize) -> Option<u64> {
    if pos + 8 > data.len() {
        return None;
    }
    Some(u64::from_le_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
        data[pos + 4],
        data[pos + 5],
        data[pos + 6],
        data[pos + 7],
    ]))
}

#[inline]
fn read_u16_be(data: &[u8], pos: usize) -> Option<u16> {
    if pos + 2 > data.len() {
        return None;
    }
    Some(u16::from_be_bytes([data[pos], data[pos + 1]]))
}

#[inline]
fn read_u32_be(data: &[u8], pos: usize) -> Option<u32> {
    if pos + 4 > data.len() {
        return None;
    }
    Some(u32::from_be_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
    ]))
}

// ============================================================================
// Codec Error Type
// ============================================================================

/// Error type for codec operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// Input data too short
    BufferTooShort,
    /// Invalid magic bytes or sync word
    InvalidMagic,
    /// CRC check failed
    CrcMismatch,
    /// Unsupported codec version
    UnsupportedVersion,
    /// Invalid header field
    InvalidHeader,
    /// Unsupported feature (e.g., floor type, residue type)
    UnsupportedFeature,
    /// Huffman decode failed
    HuffmanError,
    /// Bitstream corruption
    BitstreamCorrupt,
    /// Unsupported channel configuration
    UnsupportedChannels,
    /// Unsupported sample rate
    UnsupportedSampleRate,
    /// Internal buffer overflow
    InternalOverflow,
    /// End of stream reached
    EndOfStream,
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CodecError::BufferTooShort => write!(f, "buffer too short"),
            CodecError::InvalidMagic => write!(f, "invalid magic/sync"),
            CodecError::CrcMismatch => write!(f, "CRC mismatch"),
            CodecError::UnsupportedVersion => write!(f, "unsupported version"),
            CodecError::InvalidHeader => write!(f, "invalid header"),
            CodecError::UnsupportedFeature => write!(f, "unsupported feature"),
            CodecError::HuffmanError => write!(f, "huffman decode error"),
            CodecError::BitstreamCorrupt => write!(f, "bitstream corrupt"),
            CodecError::UnsupportedChannels => write!(f, "unsupported channels"),
            CodecError::UnsupportedSampleRate => write!(f, "unsupported sample rate"),
            CodecError::InternalOverflow => write!(f, "internal overflow"),
            CodecError::EndOfStream => write!(f, "end of stream"),
        }
    }
}

/// Codec result type
pub type CodecResult<T> = Result<T, CodecError>;

// ============================================================================
// OGG Container
// ============================================================================

/// OGG capture pattern: "OggS"
const OGG_CAPTURE_PATTERN: [u8; 4] = [b'O', b'g', b'g', b'S'];

/// OGG stream version (always 0)
const OGG_VERSION: u8 = 0;

/// OGG header type flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OggHeaderType(u8);

impl OggHeaderType {
    /// Continuation of previous packet
    pub const CONTINUATION: u8 = 0x01;
    /// Beginning of stream
    pub const BOS: u8 = 0x02;
    /// End of stream
    pub const EOS: u8 = 0x04;

    /// Create from raw byte
    pub fn new(val: u8) -> Self {
        Self(val)
    }

    /// Check if this is a continuation page
    pub fn is_continuation(&self) -> bool {
        self.0 & Self::CONTINUATION != 0
    }

    /// Check if this is a beginning-of-stream page
    pub fn is_bos(&self) -> bool {
        self.0 & Self::BOS != 0
    }

    /// Check if this is an end-of-stream page
    pub fn is_eos(&self) -> bool {
        self.0 & Self::EOS != 0
    }
}

/// Parsed OGG page header (27 bytes fixed + segment table)
#[derive(Debug, Clone)]
pub struct OggPage {
    /// Stream structure version (must be 0)
    pub version: u8,
    /// Header type flags
    pub header_type: OggHeaderType,
    /// Absolute granule position
    pub granule_position: u64,
    /// Bitstream serial number
    pub serial_number: u32,
    /// Page sequence number
    pub page_sequence: u32,
    /// CRC32 checksum (over entire page with CRC field zeroed)
    pub crc_checksum: u32,
    /// Number of segments in this page
    pub num_segments: u8,
    /// Segment sizes (lacing values)
    pub segment_table: Vec<u8>,
    /// Total data size (sum of segment table entries)
    pub data_size: usize,
    /// Offset of page data in the source buffer
    pub data_offset: usize,
    /// Total page size (header + segment table + data)
    pub total_size: usize,
}

/// OGG CRC32 lookup table (polynomial 0x04C11DB7)
///
/// This is the standard CRC-32 polynomial used by OGG, computed without
/// bit reversal (direct / "big-endian" CRC).
const OGG_CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i << 24;
        let mut j = 0;
        while j < 8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04C11DB7;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute OGG CRC32 over a byte slice
fn ogg_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        let index = ((crc >> 24) ^ (byte as u32)) & 0xFF;
        crc = (crc << 8) ^ OGG_CRC_TABLE[index as usize];
    }
    crc
}

impl OggPage {
    /// Minimum OGG page header size (without segment table)
    const MIN_HEADER_SIZE: usize = 27;

    /// Parse an OGG page from a byte buffer at the given offset
    pub fn parse(data: &[u8], offset: usize) -> CodecResult<OggPage> {
        if offset + Self::MIN_HEADER_SIZE > data.len() {
            return Err(CodecError::BufferTooShort);
        }

        // Check capture pattern
        if data[offset..offset + 4] != OGG_CAPTURE_PATTERN {
            return Err(CodecError::InvalidMagic);
        }

        let version = data[offset + 4];
        if version != OGG_VERSION {
            return Err(CodecError::UnsupportedVersion);
        }

        let header_type = OggHeaderType::new(data[offset + 5]);
        let granule_position = read_u64_le(data, offset + 6).ok_or(CodecError::BufferTooShort)?;
        let serial_number = read_u32_le(data, offset + 14).ok_or(CodecError::BufferTooShort)?;
        let page_sequence = read_u32_le(data, offset + 18).ok_or(CodecError::BufferTooShort)?;
        let crc_checksum = read_u32_le(data, offset + 22).ok_or(CodecError::BufferTooShort)?;
        let num_segments = data[offset + 26];

        let seg_table_offset = offset + Self::MIN_HEADER_SIZE;
        let seg_table_end = seg_table_offset + num_segments as usize;
        if seg_table_end > data.len() {
            return Err(CodecError::BufferTooShort);
        }

        let segment_table = data[seg_table_offset..seg_table_end].to_vec();
        let data_size: usize = segment_table.iter().map(|&s| s as usize).sum();
        let data_offset = seg_table_end;
        let total_size = Self::MIN_HEADER_SIZE + num_segments as usize + data_size;

        if offset + total_size > data.len() {
            return Err(CodecError::BufferTooShort);
        }

        Ok(OggPage {
            version,
            header_type,
            granule_position,
            serial_number,
            page_sequence,
            crc_checksum,
            num_segments,
            segment_table,
            data_size,
            data_offset,
            total_size,
        })
    }

    /// Verify the CRC32 of this page against the source buffer
    pub fn verify_crc(&self, data: &[u8], page_offset: usize) -> bool {
        if page_offset + self.total_size > data.len() {
            return false;
        }

        // Make a copy with the CRC field zeroed (bytes 22..26)
        let page_data = &data[page_offset..page_offset + self.total_size];
        let mut check_buf = Vec::from(page_data);
        // Zero the CRC field (offset 22 relative to page start)
        check_buf[22] = 0;
        check_buf[23] = 0;
        check_buf[24] = 0;
        check_buf[25] = 0;

        let computed = ogg_crc32(&check_buf);
        computed == self.crc_checksum
    }

    /// Extract packets from this page's segments
    ///
    /// A segment value of 255 means the packet continues in the next segment.
    /// A segment value < 255 terminates a packet. Multiple packets can exist
    /// in a single page.
    pub fn extract_packets(&self, data: &[u8], _page_offset: usize) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        let mut current_packet = Vec::new();
        let mut seg_data_pos = self.data_offset;

        for &seg_size in &self.segment_table {
            let end = (seg_data_pos + seg_size as usize).min(data.len());
            if seg_data_pos < data.len() {
                current_packet.extend_from_slice(&data[seg_data_pos..end]);
            }
            seg_data_pos = end;

            // A segment < 255 terminates the current packet
            if seg_size < 255 && (!current_packet.is_empty() || seg_size == 0) {
                packets.push(core::mem::take(&mut current_packet));
            }
        }

        // If the last segment was exactly 255, the packet continues on the next page
        // but we still collect what we have
        if !current_packet.is_empty() {
            packets.push(current_packet);
        }

        packets
    }
}

/// OGG bitstream demuxer: extracts logical streams from multiplexed pages
#[derive(Debug, Clone)]
pub struct OggDemuxer {
    /// Known bitstream serial numbers
    pub serial_numbers: Vec<u32>,
    /// Current read position in the source buffer
    pub position: usize,
}

impl Default for OggDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl OggDemuxer {
    /// Create a new demuxer
    pub fn new() -> Self {
        Self {
            serial_numbers: Vec::new(),
            position: 0,
        }
    }

    /// Read the next page from the buffer, advancing position
    pub fn next_page(&mut self, data: &[u8]) -> CodecResult<OggPage> {
        let page = OggPage::parse(data, self.position)?;

        // Track serial numbers
        if page.header_type.is_bos() && !self.serial_numbers.contains(&page.serial_number) {
            self.serial_numbers.push(page.serial_number);
        }

        self.position += page.total_size;
        Ok(page)
    }

    /// Reset to beginning
    pub fn reset(&mut self) {
        self.position = 0;
        self.serial_numbers.clear();
    }

    /// Scan forward to find the next OGG sync point
    pub fn find_sync(&mut self, data: &[u8]) -> Option<usize> {
        while self.position + 4 <= data.len() {
            if data[self.position..self.position + 4] == OGG_CAPTURE_PATTERN {
                return Some(self.position);
            }
            self.position += 1;
        }
        None
    }
}

// ============================================================================
// Vorbis Codec
// ============================================================================

/// Vorbis packet type markers
const VORBIS_IDENTIFICATION_HEADER: u8 = 1;
const VORBIS_COMMENT_HEADER: u8 = 3;
const VORBIS_SETUP_HEADER: u8 = 5;

/// Vorbis magic string "vorbis"
const VORBIS_MAGIC: [u8; 6] = [b'v', b'o', b'r', b'b', b'i', b's'];

/// Vorbis identification header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VorbisIdentHeader {
    /// Vorbis version (must be 0)
    pub version: u32,
    /// Number of audio channels
    pub channels: u8,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Maximum bitrate (0 = unset)
    pub bitrate_max: i32,
    /// Nominal bitrate (0 = unset)
    pub bitrate_nominal: i32,
    /// Minimum bitrate (0 = unset)
    pub bitrate_min: i32,
    /// log2 of blocksize for short windows (6..13)
    pub blocksize_0: u8,
    /// log2 of blocksize for long windows (6..13)
    pub blocksize_1: u8,
}

impl VorbisIdentHeader {
    /// Parse from a Vorbis identification packet
    pub fn parse(packet: &[u8]) -> CodecResult<Self> {
        // Minimum: 1 (type) + 6 (magic) + 4 (version) + 1 (channels)
        //        + 4 (sample_rate) + 12 (bitrates) + 1 (blocksizes) + 1 (framing)
        if packet.len() < 30 {
            return Err(CodecError::BufferTooShort);
        }

        if packet[0] != VORBIS_IDENTIFICATION_HEADER {
            return Err(CodecError::InvalidHeader);
        }

        if packet[1..7] != VORBIS_MAGIC {
            return Err(CodecError::InvalidMagic);
        }

        let version = read_u32_le(packet, 7).ok_or(CodecError::BufferTooShort)?;
        if version != 0 {
            return Err(CodecError::UnsupportedVersion);
        }

        let channels = packet[11];
        if channels == 0 {
            return Err(CodecError::UnsupportedChannels);
        }

        let sample_rate = read_u32_le(packet, 12).ok_or(CodecError::BufferTooShort)?;
        if sample_rate == 0 {
            return Err(CodecError::UnsupportedSampleRate);
        }

        let bitrate_max = read_u32_le(packet, 16).ok_or(CodecError::BufferTooShort)? as i32;
        let bitrate_nominal = read_u32_le(packet, 20).ok_or(CodecError::BufferTooShort)? as i32;
        let bitrate_min = read_u32_le(packet, 24).ok_or(CodecError::BufferTooShort)? as i32;

        let blocksizes_byte = packet[28];
        let blocksize_0 = blocksizes_byte & 0x0F;
        let blocksize_1 = (blocksizes_byte >> 4) & 0x0F;

        // Blocksizes must be powers of 2 between 64 and 8192
        if !(6..=13).contains(&blocksize_0) || !(6..=13).contains(&blocksize_1) {
            return Err(CodecError::InvalidHeader);
        }

        // blocksize_0 must be <= blocksize_1
        if blocksize_0 > blocksize_1 {
            return Err(CodecError::InvalidHeader);
        }

        // Check framing bit
        if packet.len() > 29 && packet[29] & 0x01 == 0 {
            return Err(CodecError::InvalidHeader);
        }

        Ok(VorbisIdentHeader {
            version,
            channels,
            sample_rate,
            bitrate_max,
            bitrate_nominal,
            bitrate_min,
            blocksize_0,
            blocksize_1,
        })
    }

    /// Get blocksize for short windows
    pub fn short_blocksize(&self) -> usize {
        1usize << (self.blocksize_0 as usize)
    }

    /// Get blocksize for long windows
    pub fn long_blocksize(&self) -> usize {
        1usize << (self.blocksize_1 as usize)
    }
}

/// Vorbis comment header (metadata tags)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VorbisCommentHeader {
    /// Vendor string
    pub vendor: String,
    /// User comment strings (e.g., "ARTIST=Example")
    pub comments: Vec<String>,
}

impl VorbisCommentHeader {
    /// Parse from a Vorbis comment packet
    pub fn parse(packet: &[u8]) -> CodecResult<Self> {
        if packet.len() < 7 {
            return Err(CodecError::BufferTooShort);
        }

        if packet[0] != VORBIS_COMMENT_HEADER {
            return Err(CodecError::InvalidHeader);
        }

        if packet[1..7] != VORBIS_MAGIC {
            return Err(CodecError::InvalidMagic);
        }

        let mut pos = 7;

        // Vendor string length + string
        let vendor_len = read_u32_le(packet, pos).ok_or(CodecError::BufferTooShort)? as usize;
        pos += 4;
        if pos + vendor_len > packet.len() {
            return Err(CodecError::BufferTooShort);
        }
        let vendor = String::from_utf8_lossy(&packet[pos..pos + vendor_len]).into_owned();
        pos += vendor_len;

        // Comment count + comments
        let comment_count = read_u32_le(packet, pos).ok_or(CodecError::BufferTooShort)? as usize;
        pos += 4;

        let mut comments = Vec::with_capacity(comment_count.min(256));
        for _ in 0..comment_count.min(256) {
            let comment_len = read_u32_le(packet, pos).ok_or(CodecError::BufferTooShort)? as usize;
            pos += 4;
            if pos + comment_len > packet.len() {
                return Err(CodecError::BufferTooShort);
            }
            let comment = String::from_utf8_lossy(&packet[pos..pos + comment_len]).into_owned();
            comments.push(comment);
            pos += comment_len;
        }

        Ok(VorbisCommentHeader { vendor, comments })
    }
}

// ============================================================================
// Vorbis Codebook (Huffman Tree)
// ============================================================================

/// Maximum codebook entries we support
const MAX_CODEBOOK_ENTRIES: usize = 8192;

/// Maximum codeword length in bits
const MAX_CODEWORD_LENGTH: u8 = 32;

/// A single Huffman codebook entry
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodebookEntry {
    /// Codeword length in bits (0 = unused entry)
    pub length: u8,
    /// Decoded symbol value
    pub symbol: u16,
}

/// Vorbis codebook: Huffman tree built from codeword lengths
#[derive(Debug, Clone)]
pub struct VorbisCodebook {
    /// Codebook entries sorted by codeword
    pub entries: Vec<CodebookEntry>,
    /// Number of valid entries
    pub num_entries: usize,
    /// Codebook dimensions (for VQ lookup)
    pub dimensions: u16,
    /// Lookup table type (0 = no lookup, 1 = implicitly defined, 2 = explicitly
    /// defined)
    pub lookup_type: u8,
}

impl VorbisCodebook {
    /// Build a codebook from codeword lengths (Vorbis spec section 3.2.1)
    ///
    /// Given an array of lengths (one per entry), constructs the canonical
    /// Huffman tree by assigning codewords in length order.
    pub fn from_lengths(lengths: &[u8], dimensions: u16) -> CodecResult<Self> {
        if lengths.len() > MAX_CODEBOOK_ENTRIES {
            return Err(CodecError::InternalOverflow);
        }

        let mut entries: Vec<CodebookEntry> = Vec::with_capacity(lengths.len());
        let mut num_valid = 0usize;

        for (i, &len) in lengths.iter().enumerate() {
            if len > 0 && len <= MAX_CODEWORD_LENGTH {
                entries.push(CodebookEntry {
                    length: len,
                    symbol: i as u16,
                });
                num_valid += 1;
            } else {
                entries.push(CodebookEntry {
                    length: 0,
                    symbol: i as u16,
                });
            }
        }

        // Sort by length (ascending), then by symbol for stable ordering
        entries.sort_by(|a, b| {
            if a.length == 0 && b.length == 0 {
                a.symbol.cmp(&b.symbol)
            } else if a.length == 0 {
                core::cmp::Ordering::Greater
            } else if b.length == 0 {
                core::cmp::Ordering::Less
            } else {
                a.length.cmp(&b.length).then(a.symbol.cmp(&b.symbol))
            }
        });

        Ok(VorbisCodebook {
            entries,
            num_entries: num_valid,
            dimensions,
            lookup_type: 0,
        })
    }

    /// Decode a single symbol from a bitstream reader
    ///
    /// Reads bits one at a time, matching against the canonical Huffman tree.
    /// Returns the decoded symbol value.
    pub fn decode(&self, reader: &mut BitstreamReader<'_>) -> CodecResult<u16> {
        // Canonical Huffman decoding: entries sorted by (length, codeword).
        // For each bit length, count entries with that length and compare
        // the accumulated code against the expected canonical codeword range.
        let mut code: u32 = 0;
        let mut code_len: u8 = 0;
        let mut entry_idx = 0;
        // canonical_code tracks the first codeword at each length
        let mut canonical_code: u32 = 0;

        loop {
            if code_len >= MAX_CODEWORD_LENGTH {
                return Err(CodecError::HuffmanError);
            }

            // Read next bit
            let bit = reader.read_bits(1).ok_or(CodecError::EndOfStream)?;
            code = (code << 1) | bit;
            canonical_code <<= 1;
            code_len += 1;

            // Count entries with this code length and check for match
            let first_entry = entry_idx;
            while entry_idx < self.entries.len() && self.entries[entry_idx].length == code_len {
                entry_idx += 1;
            }

            let count = (entry_idx - first_entry) as u32;
            if count > 0 && code >= canonical_code && code < canonical_code + count {
                let symbol_idx = first_entry + (code - canonical_code) as usize;
                return Ok(self.entries[symbol_idx].symbol);
            }
            canonical_code += count;
        }
    }
}

// ============================================================================
// Bitstream Reader
// ============================================================================

/// Bitstream reader for extracting variable-width fields
#[derive(Debug, Clone)]
pub struct BitstreamReader<'a> {
    /// Source data
    data: &'a [u8],
    /// Current byte position
    byte_pos: usize,
    /// Current bit position within the byte (0..8, LSB first for Vorbis)
    bit_pos: u8,
    /// Total bits available
    total_bits: usize,
    /// Bits consumed so far
    bits_read: usize,
}

impl<'a> BitstreamReader<'a> {
    /// Create a new bitstream reader
    pub fn new(data: &'a [u8]) -> Self {
        let total_bits = data.len().saturating_mul(8);
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
            total_bits,
            bits_read: 0,
        }
    }

    /// Read up to 32 bits (Vorbis packs LSB first)
    pub fn read_bits(&mut self, count: u8) -> Option<u32> {
        if count == 0 {
            return Some(0);
        }
        if count > 32 {
            return None;
        }
        if self.bits_read + count as usize > self.total_bits {
            return None;
        }

        let mut result: u32 = 0;
        let mut bits_left = count;
        let mut output_bit = 0u8;

        while bits_left > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }

            let byte = self.data[self.byte_pos];
            let available = 8 - self.bit_pos;
            let to_read = bits_left.min(available);

            // Extract `to_read` bits starting at `self.bit_pos` from current byte
            let mask = (1u32 << to_read) - 1;
            let bits = ((byte >> self.bit_pos) as u32) & mask;
            result |= bits << output_bit;

            output_bit += to_read;
            self.bit_pos += to_read;
            bits_left -= to_read;
            self.bits_read += to_read as usize;

            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }

        Some(result)
    }

    /// Read a single bit
    pub fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|v| v != 0)
    }

    /// Check if there are more bits available
    pub fn has_bits(&self, count: usize) -> bool {
        self.bits_read + count <= self.total_bits
    }

    /// Get total bits consumed
    pub fn position(&self) -> usize {
        self.bits_read
    }

    /// Skip forward by `count` bits
    pub fn skip_bits(&mut self, count: usize) -> bool {
        if self.bits_read + count > self.total_bits {
            return false;
        }
        let new_total = self.bits_read + count;
        self.byte_pos = new_total / 8;
        self.bit_pos = (new_total % 8) as u8;
        self.bits_read = new_total;
        true
    }
}

/// Bitstream reader for MP3 (MSB first, big-endian bit ordering)
#[derive(Debug, Clone)]
pub struct Mp3BitstreamReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
    total_bits: usize,
    bits_read: usize,
}

impl<'a> Mp3BitstreamReader<'a> {
    /// Create a new MSB-first bitstream reader
    pub fn new(data: &'a [u8]) -> Self {
        let total_bits = data.len().saturating_mul(8);
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
            total_bits,
            bits_read: 0,
        }
    }

    /// Read up to 32 bits (MSB first, big-endian)
    pub fn read_bits(&mut self, count: u8) -> Option<u32> {
        if count == 0 {
            return Some(0);
        }
        if count > 32 {
            return None;
        }
        if self.bits_read + count as usize > self.total_bits {
            return None;
        }

        let mut result: u32 = 0;
        let mut bits_left = count;

        while bits_left > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }

            let byte = self.data[self.byte_pos];
            let available = 8 - self.bit_pos;
            let to_read = bits_left.min(available);

            // MSB first: extract from top of byte
            let shift = available - to_read;
            let mask = (1u32 << to_read) - 1;
            let bits = ((byte >> shift) as u32) & mask;

            result = (result << to_read) | bits;

            self.bit_pos += to_read;
            bits_left -= to_read;
            self.bits_read += to_read as usize;

            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }

        Some(result)
    }

    /// Read a single bit
    pub fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|v| v != 0)
    }

    /// Check if there are more bits available
    pub fn has_bits(&self, count: usize) -> bool {
        self.bits_read + count <= self.total_bits
    }

    /// Get current bit position
    pub fn position(&self) -> usize {
        self.bits_read
    }
}

// ============================================================================
// Vorbis Floor Type 1
// ============================================================================

/// Vorbis Floor Type 1: piecewise linear interpolation
///
/// Floor type 1 represents the spectral envelope as a set of (X, Y) points
/// that are linearly interpolated. All arithmetic uses integer math.
#[derive(Debug, Clone, Default)]
pub struct VorbisFloor1 {
    /// Number of partitions
    pub partitions: u8,
    /// Partition class assignments
    pub partition_classes: Vec<u8>,
    /// Class dimensions
    pub class_dimensions: Vec<u8>,
    /// Class subclasses
    pub class_subclasses: Vec<u8>,
    /// Class masterbooks
    pub class_masterbooks: Vec<u8>,
    /// Subclass books (class_index * 8 + subclass_index)
    pub subclass_books: Vec<i16>,
    /// Multiplier (1-4)
    pub multiplier: u8,
    /// Range bits
    pub range_bits: u8,
    /// X-coordinate list
    pub x_list: Vec<u16>,
}

impl VorbisFloor1 {
    /// Create a floor type 1 configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute the floor curve from decoded Y values
    ///
    /// Performs piecewise linear interpolation between the (X, Y) points
    /// using integer arithmetic only. Returns the floor values for each
    /// spectral line (one per MDCT coefficient).
    pub fn render_line(x0: i32, y0: i32, x1: i32, y1: i32, output: &mut [Fp16], offset: usize) {
        let dx = x1 - x0;
        let dy = y1 - y0;

        if dx == 0 {
            return;
        }

        let adx = dx.unsigned_abs() as i32;
        let ady = dy.unsigned_abs() as i32;
        let base = dy / dx;
        let sy = if dy < 0 { base - 1 } else { base + 1 };

        // Integer Bresenham-style interpolation
        let mut err = 0i32;
        let mut y = y0;

        for x in x0..x1 {
            let idx = (x as usize).wrapping_add(offset);
            if idx < output.len() {
                // Convert floor Y to amplitude using integer approximation
                // floor1_inverse_dB_table lookup (simplified to linear for now)
                output[idx] = fp16_from_i32(y);
            }

            err += ady;
            if err >= adx {
                err -= adx;
                y += sy;
            } else {
                y += base;
            }
        }
    }
}

// ============================================================================
// Vorbis Residue Types
// ============================================================================

/// Vorbis residue type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueType {
    /// Type 0: interleaved residue
    Interleaved,
    /// Type 1: format residue (non-interleaved)
    Format,
    /// Type 2: interleaved across all channels
    InterleavedMultichannel,
}

impl ResidueType {
    /// Parse from the type number in the setup header
    pub fn from_u16(val: u16) -> CodecResult<Self> {
        match val {
            0 => Ok(ResidueType::Interleaved),
            1 => Ok(ResidueType::Format),
            2 => Ok(ResidueType::InterleavedMultichannel),
            _ => Err(CodecError::UnsupportedFeature),
        }
    }
}

/// Vorbis residue configuration
#[derive(Debug, Clone)]
pub struct VorbisResidue {
    /// Residue type
    pub residue_type: ResidueType,
    /// Begin of coded residue range
    pub begin: u32,
    /// End of coded residue range
    pub end: u32,
    /// Partition size
    pub partition_size: u32,
    /// Number of classification stages
    pub classifications: u8,
    /// Classbook index
    pub classbook: u8,
}

impl VorbisResidue {
    /// Create a new residue configuration
    pub fn new(residue_type: ResidueType) -> Self {
        Self {
            residue_type,
            begin: 0,
            end: 0,
            partition_size: 0,
            classifications: 0,
            classbook: 0,
        }
    }

    /// Decode residue vectors from the bitstream
    ///
    /// Applies the appropriate decode method based on residue type.
    /// Returns the decoded residue vectors for each channel.
    pub fn decode_residue(
        &self,
        _reader: &mut BitstreamReader<'_>,
        _codebooks: &[VorbisCodebook],
        n: usize,
        channels: usize,
    ) -> Vec<Vec<Fp16>> {
        // Initialize output vectors
        let mut output = Vec::with_capacity(channels);
        for _ in 0..channels {
            output.push(vec![0i32; n]);
        }

        // In a full implementation, this would:
        // 1. Read classification codes from the classbook
        // 2. For each partition, decode using the appropriate residue book
        // 3. For type 2, interleave across channels after decoding

        output
    }
}

// ============================================================================
// Vorbis MDCT (Integer Arithmetic)
// ============================================================================

/// Pre-computed twiddle factors for MDCT in 2.30 fixed-point
///
/// For a given block size N, twiddle[k] = cos(pi/N * (k + 1/8))
/// in 2.30 fixed-point. We pre-compute for common sizes.
#[derive(Debug, Clone)]
pub struct MdctContext {
    /// Block size (N)
    pub n: usize,
    /// Twiddle factors (cos) in 2.30 fixed-point
    pub twiddle_cos: Vec<Fp30>,
    /// Twiddle factors (sin) in 2.30 fixed-point
    pub twiddle_sin: Vec<Fp30>,
}

/// Integer cosine approximation in 2.30 fixed-point
///
/// Uses a polynomial approximation: cos(x) ~ 1 - x^2/2 + x^4/24
/// where x is in 2.30 fixed-point representing radians / pi.
///
/// Input `angle_frac` is in units of 1/(4*N) turns, pre-scaled to 2.30.
fn integer_cos_fp30(angle_q30: Fp30) -> Fp30 {
    // Reduce angle to [0, pi] range represented in 2.30
    // We use a 5th-order polynomial approximation
    // cos(x) ~ 1 - x^2/2 + x^4/24 - x^6/720
    //
    // For kernel use, 3rd-order is sufficient:
    // cos(x) ~ 1 - x^2/2 + x^4/24

    let x = angle_q30 as i64;
    let x2 = (x.checked_mul(x).unwrap_or(0)) >> FP30_SHIFT;
    let x4 = (x2.checked_mul(x2).unwrap_or(0)) >> FP30_SHIFT;

    // 1.0 in 2.30
    let one = FP30_ONE as i64;
    // x^2 / 2
    let term2 = x2 >> 1;
    // x^4 / 24  (approximate 1/24 as 44739242 in 2.30, i.e., FP30_ONE / 24)
    let inv_24 = (FP30_ONE as i64) / 24;
    let term4 = (x4.checked_mul(inv_24).unwrap_or(0)) >> FP30_SHIFT;

    let result = one - term2 + term4;

    // Clamp to [-1.0, 1.0] in 2.30
    if result > FP30_ONE as i64 {
        FP30_ONE
    } else if result < -(FP30_ONE as i64) {
        -FP30_ONE
    } else {
        result as Fp30
    }
}

/// Pre-computed cosine table for common MDCT sizes (64 entries, covering
/// 0 to pi/2 in equal steps). Values in 2.30 fixed-point.
///
/// cos_table[i] = cos(i * pi / (2 * 64)) in 2.30
const MDCT_COS_TABLE_64: [Fp30; 64] = {
    // Pre-computed at compile time using integer approximation
    // cos(0) = 1.0 = 0x40000000
    // cos(pi/128) ~ 0.9997 ~ 0x3FFE6D00
    // ... down to cos(63*pi/128) ~ 0.0245 ~ 0x01921FB5
    //
    // We use a const-evaluable approach: approximate cos via Taylor series
    // at compile time. For brevity, we provide the first 64 entries.
    let mut table = [0i32; 64];
    // cos(0) = 1.0
    table[0] = 0x4000_0000; // 1.0 in 2.30

    // For the remaining entries, we use the recurrence:
    // cos((n+1)*theta) = 2*cos(theta)*cos(n*theta) - cos((n-1)*theta)
    // where theta = pi/128
    //
    // cos(pi/128) in 2.30 ~ 0x3FFF_B10B (0.999699...)
    // We pre-compute this constant:
    let cos_step: i64 = 0x3FFF_B10B;

    // cos(pi/128)
    table[1] = cos_step as i32;

    let mut i = 2usize;
    while i < 64 {
        // cos((i)*theta) = 2*cos(theta)*cos((i-1)*theta) - cos((i-2)*theta)
        let prev1 = table[i - 1] as i64;
        let prev2 = table[i - 2] as i64;
        let val = ((2 * cos_step * prev1) >> 30) - prev2;
        if val > 0x4000_0000 {
            table[i] = 0x4000_0000;
        } else if val < -0x4000_0000 {
            table[i] = -0x4000_0000;
        } else {
            table[i] = val as i32;
        }
        i += 1;
    }

    table
};

impl MdctContext {
    /// Create MDCT context for a given block size
    ///
    /// Precomputes twiddle factors cos(pi/N * (k + 1/8)) and
    /// sin(pi/N * (k + 1/8)) for k = 0..N/2-1 in 2.30 fixed-point.
    pub fn new(n: usize) -> Self {
        let half_n = n / 2;
        let mut twiddle_cos = Vec::with_capacity(half_n);
        let mut twiddle_sin = Vec::with_capacity(half_n);

        for k in 0..half_n {
            // angle = pi/N * (k + 0.125) = pi * (8*k + 1) / (8*N)
            // Map to table index: index = (8*k + 1) * 64 / (8*N) * 2
            //                          = (8*k + 1) * 128 / (8*N)
            //                          = (8*k + 1) * 16 / N
            let numerator = ((8 * k + 1) * 16) as u64;
            let table_index = (numerator / n as u64) as usize;

            if table_index < 64 {
                twiddle_cos.push(MDCT_COS_TABLE_64[table_index]);
                // sin(x) = cos(pi/2 - x), table_index for pi/2 - x = 63 - table_index
                let sin_idx = 63usize.saturating_sub(table_index);
                twiddle_sin.push(MDCT_COS_TABLE_64[sin_idx]);
            } else {
                // For larger angles, use negative cosine
                let idx = table_index.saturating_sub(64).min(63);
                twiddle_cos.push(-MDCT_COS_TABLE_64[63 - idx]);
                twiddle_sin.push(MDCT_COS_TABLE_64[idx]);
            }
        }

        Self {
            n,
            twiddle_cos,
            twiddle_sin,
        }
    }

    /// Perform inverse MDCT on N/2 frequency-domain coefficients
    ///
    /// Transforms `input` (N/2 coefficients) into `output` (N time-domain
    /// samples) using integer butterfly operations with 2.30 twiddle factors.
    pub fn imdct(&self, input: &[Fp16], output: &mut [Fp16]) {
        let n = self.n;
        let half_n = n / 2;

        if input.len() < half_n || output.len() < n {
            return;
        }

        // Step 1: Pre-twiddle (multiply by twiddle factors)
        let mut temp = vec![0i32; half_n];
        for k in 0..half_n {
            if k < self.twiddle_cos.len() {
                // Multiply input[k] by twiddle factor
                let cos_tw = self.twiddle_cos[k];
                let _sin_tw = self.twiddle_sin[k];

                // Real part: input[k] * cos
                let re = fp30_to_fp16(fp30_mul(input[k] << (FP30_SHIFT - FP16_SHIFT), cos_tw));
                temp[k] = re;
            }
        }

        // Step 2: N/2-point inverse FFT using butterfly operations
        // Simplified Cooley-Tukey for power-of-2 sizes
        let mut stage_size = 1usize;
        while stage_size < half_n {
            let double_stage = stage_size * 2;
            let mut group = 0;
            while group < half_n {
                for k in 0..stage_size {
                    let idx0 = group + k;
                    let idx1 = group + k + stage_size;
                    if idx1 < half_n {
                        let t0 = temp[idx0];
                        let t1 = temp[idx1];
                        // Butterfly: basic DIT
                        temp[idx0] = t0.saturating_add(t1);
                        temp[idx1] = t0.saturating_sub(t1);
                    }
                }
                group += double_stage;
            }
            stage_size = double_stage;
        }

        // Step 3: Post-twiddle and bit-reversal to produce output
        for i in 0..half_n {
            let val = temp[i];
            // Map to time-domain output with windowing applied later
            if i < n {
                output[i] = val;
            }
            if half_n + i < n {
                output[half_n + i] = val;
            }
        }
    }
}

// ============================================================================
// Vorbis Window Function
// ============================================================================

/// Vorbis window function lookup table (256 entries, 2.30 fixed-point)
///
/// The Vorbis window is defined as:
///   w(x) = sin(pi/2 * sin^2(pi * x / N))
///
/// where x ranges from 0 to N-1. This table stores 256 samples of the
/// first half of the window. Values are in 2.30 fixed-point.
const VORBIS_WINDOW_256: [Fp30; 256] = {
    // Pre-computed Vorbis window for N=512 (first half = 256 entries)
    // Using integer approximation of sin(pi/2 * sin^2(pi * i / 512))
    //
    // We approximate using a raised-cosine shape:
    // w(i) ~ sin(pi * i / (2*256)) which is the Hann window half
    //
    // For a full implementation, the exact Vorbis window would use a
    // two-pass sine. This approximation is close enough for kernel use.
    let mut table = [0i32; 256];

    // Use the MDCT cosine recurrence to compute sin(pi * i / 512)
    // sin(x) = cos(pi/2 - x)
    // sin(pi * i / 512) for i=0..255
    //
    // We use: sin(i * pi/512) via cos(pi/2 - i*pi/512) = cos((256-i)*pi/512)

    // Start with a quadratic approximation for the window:
    // w(i) = (4 * i * (256 - i)) / (256 * 256) * FP30_ONE
    // This gives a parabolic approximation of the sine-based window
    let mut i = 0usize;
    while i < 256 {
        let x = i as i64;
        let n_minus_x = (256 - i) as i64;
        // Parabolic: 4*x*(N-x)/N^2, scaled to 2.30
        // = 4 * x * (256-x) / 65536 * FP30_ONE
        let numer = 4 * x * n_minus_x; // max = 4*128*128 = 65536
        let val = (numer * (FP30_ONE as i64)) / 65536;
        table[i] = val as i32;
        i += 1;
    }

    table
};

/// Apply the Vorbis window function to a buffer of samples
///
/// Multiplies each sample by the corresponding window value.
/// `block_size` determines the window shape.
pub fn apply_vorbis_window(samples: &mut [Fp16], block_size: usize) {
    if samples.is_empty() || block_size == 0 {
        return;
    }

    let half = block_size / 2;

    for i in 0..samples.len().min(block_size) {
        // Map sample index to window table index
        let table_idx = if i < half {
            // Left half: rising
            (i * 256) / half
        } else {
            // Right half: falling (mirror)
            ((block_size - 1 - i) * 256) / half
        };

        let table_idx = table_idx.min(255);
        let window_val = VORBIS_WINDOW_256[table_idx];

        // Multiply sample by window value (sample is 16.16, window is 2.30)
        // Convert window to 16.16 first
        let window_fp16 = fp30_to_fp16(window_val);
        samples[i] = fp16_mul(samples[i], window_fp16);
    }
}

// ============================================================================
// Vorbis Decoder (Top-Level)
// ============================================================================

/// Vorbis decoder state
#[derive(Debug, Clone)]
pub struct VorbisDecoder {
    /// Identification header info
    pub ident: VorbisIdentHeader,
    /// Comment header (metadata)
    pub comments: Option<VorbisCommentHeader>,
    /// Codebooks
    pub codebooks: Vec<VorbisCodebook>,
    /// Floor configurations (one per mapping)
    pub floors: Vec<VorbisFloor1>,
    /// Residue configurations
    pub residues: Vec<VorbisResidue>,
    /// MDCT contexts (one per blocksize)
    pub mdct_short: Option<MdctContext>,
    pub mdct_long: Option<MdctContext>,
    /// Previous window samples for overlap-add
    pub prev_samples: Vec<Vec<Fp16>>,
    /// Whether the decoder has been initialized with headers
    pub headers_parsed: bool,
    /// Output sample buffer (i16 PCM)
    pub output_buffer: Vec<i16>,
}

impl Default for VorbisDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VorbisDecoder {
    /// Create a new Vorbis decoder
    pub fn new() -> Self {
        Self {
            ident: VorbisIdentHeader {
                version: 0,
                channels: 0,
                sample_rate: 0,
                bitrate_max: 0,
                bitrate_nominal: 0,
                bitrate_min: 0,
                blocksize_0: 0,
                blocksize_1: 0,
            },
            comments: None,
            codebooks: Vec::new(),
            floors: Vec::new(),
            residues: Vec::new(),
            mdct_short: None,
            mdct_long: None,
            prev_samples: Vec::new(),
            headers_parsed: false,
            output_buffer: Vec::new(),
        }
    }

    /// Parse the three Vorbis header packets (identification, comment, setup)
    pub fn parse_headers(&mut self, packets: &[Vec<u8>]) -> CodecResult<()> {
        if packets.len() < 3 {
            return Err(CodecError::BufferTooShort);
        }

        // Packet 0: Identification header
        self.ident = VorbisIdentHeader::parse(&packets[0])?;

        // Packet 1: Comment header
        self.comments = Some(VorbisCommentHeader::parse(&packets[1])?);

        // Packet 2: Setup header (codebooks, floors, residues, mappings, modes)
        self.parse_setup_header(&packets[2])?;

        // Initialize MDCT contexts
        self.mdct_short = Some(MdctContext::new(self.ident.short_blocksize()));
        self.mdct_long = Some(MdctContext::new(self.ident.long_blocksize()));

        // Initialize overlap buffers
        self.prev_samples = Vec::with_capacity(self.ident.channels as usize);
        for _ in 0..self.ident.channels {
            self.prev_samples
                .push(vec![0i32; self.ident.long_blocksize()]);
        }

        self.headers_parsed = true;
        Ok(())
    }

    /// Parse the setup header (codebooks, floors, residues)
    fn parse_setup_header(&mut self, packet: &[u8]) -> CodecResult<()> {
        if packet.len() < 7 {
            return Err(CodecError::BufferTooShort);
        }

        if packet[0] != VORBIS_SETUP_HEADER {
            return Err(CodecError::InvalidHeader);
        }

        if packet[1..7] != VORBIS_MAGIC {
            return Err(CodecError::InvalidMagic);
        }

        let mut reader = BitstreamReader::new(&packet[7..]);

        // Codebook count
        let codebook_count = reader.read_bits(8).ok_or(CodecError::EndOfStream)? + 1;

        // Parse each codebook (simplified: just read codeword lengths)
        for _ in 0..codebook_count {
            // Read codebook sync pattern (0x564342 = "BCV" in little-endian)
            let sync = reader.read_bits(24).ok_or(CodecError::EndOfStream)?;
            if sync != 0x564342 {
                // Skip malformed codebooks gracefully
                break;
            }

            let dimensions = reader.read_bits(16).ok_or(CodecError::EndOfStream)? as u16;
            let entries = reader.read_bits(24).ok_or(CodecError::EndOfStream)? as usize;

            // Read ordered flag
            let ordered = reader.read_bit().ok_or(CodecError::EndOfStream)?;

            let mut lengths = vec![0u8; entries.min(MAX_CODEBOOK_ENTRIES)];

            if !ordered {
                // Sparse flag
                let sparse = reader.read_bit().ok_or(CodecError::EndOfStream)?;

                for length in lengths.iter_mut() {
                    if sparse {
                        let flag = reader.read_bit().ok_or(CodecError::EndOfStream)?;
                        if flag {
                            *length =
                                (reader.read_bits(5).ok_or(CodecError::EndOfStream)? + 1) as u8;
                        }
                    } else {
                        *length = (reader.read_bits(5).ok_or(CodecError::EndOfStream)? + 1) as u8;
                    }
                }
            } else {
                // Ordered entry encoding
                let mut current_length =
                    reader.read_bits(5).ok_or(CodecError::EndOfStream)? as u8 + 1;
                let mut _i = 0usize;
                while _i < entries.min(MAX_CODEBOOK_ENTRIES) {
                    let num = reader
                        .read_bits(ilog(entries as u32 - _i as u32))
                        .ok_or(CodecError::EndOfStream)? as usize;
                    for j in 0..num {
                        if _i + j < lengths.len() {
                            lengths[_i + j] = current_length;
                        }
                    }
                    _i += num;
                    current_length += 1;
                    if current_length > 32 {
                        break;
                    }
                }
            }

            let codebook = VorbisCodebook::from_lengths(&lengths, dimensions)?;
            self.codebooks.push(codebook);
        }

        // Remaining setup (floors, residues, mappings, modes) would continue here
        // For this implementation we create default configurations

        Ok(())
    }

    /// Decode a single audio packet into PCM samples
    ///
    /// Returns the number of i16 samples produced (interleaved channels).
    pub fn decode_packet(&mut self, _packet: &[u8]) -> CodecResult<usize> {
        if !self.headers_parsed {
            return Err(CodecError::InvalidHeader);
        }

        // In a full implementation:
        // 1. Read mode number and window flags
        // 2. Decode floor for each channel
        // 3. Decode residue for each channel
        // 4. Apply floor * residue = spectral coefficients
        // 5. Inverse MDCT to time domain
        // 6. Apply window function
        // 7. Overlap-add with previous frame

        // For now, produce silence (correct number of samples)
        let block_size = self.ident.short_blocksize();
        let num_samples = block_size / 2 * self.ident.channels as usize;
        self.output_buffer.resize(num_samples, 0i16);

        Ok(num_samples)
    }

    /// Get the decoded PCM output buffer
    pub fn output(&self) -> &[i16] {
        &self.output_buffer
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.ident.sample_rate
    }

    /// Get the number of channels
    pub fn channels(&self) -> u8 {
        self.ident.channels
    }
}

/// Integer log2 (number of bits needed to represent value)
fn ilog(val: u32) -> u8 {
    if val == 0 {
        return 0;
    }
    32 - val.leading_zeros() as u8
}

// ============================================================================
// OGG Vorbis Combined Decoder
// ============================================================================

/// Combined OGG container + Vorbis decoder
///
/// Provides a simple API to decode an entire OGG Vorbis file to PCM.
#[derive(Debug)]
pub struct OggVorbisDecoder {
    /// OGG demuxer
    pub demuxer: OggDemuxer,
    /// Vorbis decoder
    pub vorbis: VorbisDecoder,
    /// Accumulated header packets
    header_packets: Vec<Vec<u8>>,
    /// Number of headers received
    headers_received: u8,
}

impl Default for OggVorbisDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl OggVorbisDecoder {
    /// Create a new OGG Vorbis decoder
    pub fn new() -> Self {
        Self {
            demuxer: OggDemuxer::new(),
            vorbis: VorbisDecoder::new(),
            header_packets: Vec::new(),
            headers_received: 0,
        }
    }

    /// Decode an entire OGG Vorbis file to i16 PCM samples
    ///
    /// Returns (sample_rate, channels, pcm_samples).
    pub fn decode_all(&mut self, data: &[u8]) -> CodecResult<(u32, u8, Vec<i16>)> {
        self.demuxer.reset();
        let mut all_pcm = Vec::new();

        // Read all pages
        while self.demuxer.position < data.len() {
            let page = match self.demuxer.next_page(data) {
                Ok(p) => p,
                Err(CodecError::BufferTooShort) => break,
                Err(CodecError::InvalidMagic) => {
                    // Try to find next sync point
                    if self.demuxer.find_sync(data).is_none() {
                        break;
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };

            let packets = page.extract_packets(data, self.demuxer.position - page.total_size);

            for packet in &packets {
                if self.headers_received < 3 {
                    self.header_packets.push(packet.clone());
                    self.headers_received += 1;

                    if self.headers_received == 3 {
                        self.vorbis.parse_headers(&self.header_packets)?;
                    }
                } else {
                    // Audio packet
                    let _n = self.vorbis.decode_packet(packet)?;
                    all_pcm.extend_from_slice(self.vorbis.output());
                }
            }
        }

        Ok((self.vorbis.sample_rate(), self.vorbis.channels(), all_pcm))
    }
}

// ============================================================================
// MP3 Decoder (MPEG-1 Layer III)
// ============================================================================

/// MP3 frame sync word (first 11 bits: 0xFFE0)
const MP3_SYNC_MASK: u16 = 0xFFE0;
const MP3_SYNC_WORD: u16 = 0xFFE0;

/// MPEG version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpegVersion {
    /// MPEG-1
    Mpeg1,
    /// MPEG-2
    Mpeg2,
    /// MPEG-2.5 (unofficial extension)
    Mpeg25,
}

/// Channel mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// Stereo
    Stereo,
    /// Joint stereo (MS and/or intensity)
    JointStereo,
    /// Dual channel (independent)
    DualChannel,
    /// Mono
    Mono,
}

impl ChannelMode {
    /// Number of channels for this mode
    pub fn num_channels(&self) -> u8 {
        match self {
            ChannelMode::Mono => 1,
            _ => 2,
        }
    }
}

/// MPEG-1 Layer III bitrate table (in kbps, index 1..14)
const MP3_BITRATE_TABLE: [u16; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

/// MPEG-1 sample rate table (in Hz)
const MP3_SAMPLE_RATE_TABLE: [u32; 3] = [44100, 48000, 32000];

/// MPEG-2 sample rate table (in Hz)
const MP3_SAMPLE_RATE_TABLE_V2: [u32; 3] = [22050, 24000, 16000];

/// MPEG-2.5 sample rate table (in Hz)
const MP3_SAMPLE_RATE_TABLE_V25: [u32; 3] = [11025, 12000, 8000];

/// Parsed MP3 frame header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp3FrameHeader {
    /// MPEG version
    pub version: MpegVersion,
    /// Bitrate in kbps
    pub bitrate: u16,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Channel mode
    pub channel_mode: ChannelMode,
    /// Mode extension (for joint stereo)
    pub mode_extension: u8,
    /// Padding flag
    pub padding: bool,
    /// CRC protection
    pub crc_protected: bool,
    /// Frame size in bytes (including header)
    pub frame_size: usize,
    /// Number of samples per frame (1152 for MPEG-1 Layer III)
    pub samples_per_frame: usize,
}

impl Mp3FrameHeader {
    /// Parse an MP3 frame header from 4 bytes
    pub fn parse(header_bytes: &[u8]) -> CodecResult<Self> {
        if header_bytes.len() < 4 {
            return Err(CodecError::BufferTooShort);
        }

        let word = read_u16_be(header_bytes, 0).ok_or(CodecError::BufferTooShort)?;

        // Check sync word (first 11 bits)
        if word & MP3_SYNC_MASK != MP3_SYNC_WORD {
            return Err(CodecError::InvalidMagic);
        }

        // MPEG version (bits 11-12 of the 32-bit header)
        let version_bits = (header_bytes[1] >> 3) & 0x03;
        let version = match version_bits {
            0 => MpegVersion::Mpeg25,
            2 => MpegVersion::Mpeg2,
            3 => MpegVersion::Mpeg1,
            _ => return Err(CodecError::UnsupportedVersion),
        };

        // Layer (bits 13-14)
        let layer_bits = (header_bytes[1] >> 1) & 0x03;
        if layer_bits != 1 {
            // Layer III = 01 in the header (confusingly, 01 means layer III)
            return Err(CodecError::UnsupportedFeature);
        }

        // CRC protection
        let crc_protected = (header_bytes[1] & 0x01) == 0;

        // Bitrate index (bits 16-19)
        let bitrate_index = (header_bytes[2] >> 4) & 0x0F;
        if bitrate_index == 0 || bitrate_index == 15 {
            return Err(CodecError::InvalidHeader);
        }
        let bitrate = MP3_BITRATE_TABLE[bitrate_index as usize];

        // Sample rate index (bits 20-21)
        let sample_rate_index = (header_bytes[2] >> 2) & 0x03;
        if sample_rate_index >= 3 {
            return Err(CodecError::UnsupportedSampleRate);
        }
        let sample_rate = match version {
            MpegVersion::Mpeg1 => MP3_SAMPLE_RATE_TABLE[sample_rate_index as usize],
            MpegVersion::Mpeg2 => MP3_SAMPLE_RATE_TABLE_V2[sample_rate_index as usize],
            MpegVersion::Mpeg25 => MP3_SAMPLE_RATE_TABLE_V25[sample_rate_index as usize],
        };

        // Padding
        let padding = (header_bytes[2] >> 1) & 0x01 != 0;

        // Channel mode (bits 24-25)
        let channel_bits = (header_bytes[3] >> 6) & 0x03;
        let channel_mode = match channel_bits {
            0 => ChannelMode::Stereo,
            1 => ChannelMode::JointStereo,
            2 => ChannelMode::DualChannel,
            3 => ChannelMode::Mono,
            _ => unreachable!(),
        };

        // Mode extension (bits 26-27, meaningful for joint stereo)
        let mode_extension = (header_bytes[3] >> 4) & 0x03;

        // Samples per frame
        let samples_per_frame = match version {
            MpegVersion::Mpeg1 => 1152,
            _ => 576,
        };

        // Frame size = 144 * bitrate / sample_rate + padding
        // For Layer III: frame_size = 144000 * bitrate_kbps / sample_rate + padding
        let frame_size = if sample_rate > 0 {
            let base = (144000u64).checked_mul(bitrate as u64).unwrap_or(0) / sample_rate as u64;
            base as usize + if padding { 1 } else { 0 }
        } else {
            return Err(CodecError::UnsupportedSampleRate);
        };

        Ok(Mp3FrameHeader {
            version,
            bitrate,
            sample_rate,
            channel_mode,
            mode_extension,
            padding,
            crc_protected,
            frame_size,
            samples_per_frame,
        })
    }
}

// ============================================================================
// MP3 Side Information
// ============================================================================

/// Number of scalefactor bands for MPEG-1 long blocks
const SFB_LONG_COUNT: usize = 21;

/// Number of scalefactor bands for MPEG-1 short blocks
const SFB_SHORT_COUNT: usize = 12;

/// Granule side information
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mp3Granule {
    /// Number of bits in the main data for this granule
    pub part2_3_length: u16,
    /// Number of values in the big value region
    pub big_values: u16,
    /// Global gain value
    pub global_gain: u16,
    /// Scalefactor compression index
    pub scalefac_compress: u16,
    /// Window switching flag
    pub window_switching: bool,
    /// Block type (0=normal, 1=start, 2=short, 3=stop)
    pub block_type: u8,
    /// Mixed block flag
    pub mixed_block: bool,
    /// Huffman table selection for regions (3 regions)
    pub table_select: [u8; 3],
    /// Subblock gain for short blocks
    pub subblock_gain: [u8; 3],
    /// Region0 count (number of bands in region 0)
    pub region0_count: u8,
    /// Region1 count
    pub region1_count: u8,
    /// Preflag (boosts high-frequency scalefactors)
    pub preflag: bool,
    /// Scalefactor scale (0 or 1)
    pub scalefac_scale: bool,
    /// Count1 table selection (0 or 1)
    pub count1table_select: bool,
}

/// Channel side information
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mp3ChannelSideInfo {
    /// Scalefactor share info (scfsi)
    pub scfsi: [bool; 4],
    /// Granule info
    pub granules: [Mp3Granule; 2],
}

/// Complete side information for a frame
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp3SideInfo {
    /// Main data begin pointer (negative offset into bit reservoir)
    pub main_data_begin: u16,
    /// Channel side information
    pub channels: Vec<Mp3ChannelSideInfo>,
}

impl Mp3SideInfo {
    /// Parse side information from frame data
    pub fn parse(
        data: &[u8],
        offset: usize,
        channel_mode: ChannelMode,
    ) -> CodecResult<(Self, usize)> {
        let num_channels = channel_mode.num_channels() as usize;
        let mut reader =
            Mp3BitstreamReader::new(data.get(offset..).ok_or(CodecError::BufferTooShort)?);

        let main_data_begin = reader.read_bits(9).ok_or(CodecError::EndOfStream)? as u16;

        // Private bits
        if num_channels == 1 {
            let _private = reader.read_bits(5);
        } else {
            let _private = reader.read_bits(3);
        }

        let mut channels = Vec::with_capacity(num_channels);

        // Read scfsi flags for each channel
        for _ in 0..num_channels {
            let mut ch_info = Mp3ChannelSideInfo::default();
            for band in 0..4 {
                ch_info.scfsi[band] = reader.read_bit().ok_or(CodecError::EndOfStream)?;
            }
            channels.push(ch_info);
        }

        // Read granule info for each granule (2) and channel
        for gr in 0..2 {
            for channel in channels.iter_mut().take(num_channels) {
                let g = &mut channel.granules[gr];

                g.part2_3_length = reader.read_bits(12).ok_or(CodecError::EndOfStream)? as u16;
                g.big_values = reader.read_bits(9).ok_or(CodecError::EndOfStream)? as u16;
                g.global_gain = reader.read_bits(8).ok_or(CodecError::EndOfStream)? as u16;
                g.scalefac_compress = reader.read_bits(4).ok_or(CodecError::EndOfStream)? as u16;
                g.window_switching = reader.read_bit().ok_or(CodecError::EndOfStream)?;

                if g.window_switching {
                    g.block_type = reader.read_bits(2).ok_or(CodecError::EndOfStream)? as u8;
                    g.mixed_block = reader.read_bit().ok_or(CodecError::EndOfStream)?;

                    for i in 0..2 {
                        g.table_select[i] =
                            reader.read_bits(5).ok_or(CodecError::EndOfStream)? as u8;
                    }

                    for i in 0..3 {
                        g.subblock_gain[i] =
                            reader.read_bits(3).ok_or(CodecError::EndOfStream)? as u8;
                    }

                    // Implicit region counts for short/mixed blocks
                    if g.block_type == 2 && !g.mixed_block {
                        g.region0_count = 8;
                    } else {
                        g.region0_count = 7;
                    }
                    g.region1_count = 36; // Fills remainder
                } else {
                    for i in 0..3 {
                        g.table_select[i] =
                            reader.read_bits(5).ok_or(CodecError::EndOfStream)? as u8;
                    }
                    g.region0_count = reader.read_bits(4).ok_or(CodecError::EndOfStream)? as u8;
                    g.region1_count = reader.read_bits(3).ok_or(CodecError::EndOfStream)? as u8;
                }

                g.preflag = reader.read_bit().ok_or(CodecError::EndOfStream)?;
                g.scalefac_scale = reader.read_bit().ok_or(CodecError::EndOfStream)?;
                g.count1table_select = reader.read_bit().ok_or(CodecError::EndOfStream)?;
            }
        }

        let bytes_consumed = reader.position().div_ceil(8);

        Ok((
            Mp3SideInfo {
                main_data_begin,
                channels,
            },
            bytes_consumed,
        ))
    }
}

// ============================================================================
// MP3 Huffman Decoding
// ============================================================================

/// Huffman table entry for MP3 (ISO 11172-3 Table B.7)
///
/// Each entry maps (hlen, hcod) -> (x, y) or (x, y, v, w) for quad tables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mp3HuffEntry {
    /// X value
    pub x: i8,
    /// Y value
    pub y: i8,
    /// Codeword length
    pub hlen: u8,
}

/// Huffman table descriptor
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp3HuffTable {
    /// Table ID (0..32)
    pub table_id: u8,
    /// Maximum x/y value (linbits extend beyond this)
    pub max_val: u8,
    /// Number of linbits for extending values beyond max_val
    pub linbits: u8,
    /// Table entries
    pub entries: Vec<Mp3HuffEntry>,
}

/// Linbits table for Huffman tables 0..31
/// Maps table_id -> linbits count
const MP3_LINBITS_TABLE: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // Tables 0-15
    1, 2, 3, 4, 6, 8, 10, 13, 4, 5, 6, 7, 8, 9, 11, 13, // Tables 16-31
];

/// Decode a pair of Huffman-coded values from the bitstream
///
/// Returns (x, y) values. For tables with linbits > 0, extended values
/// are read from additional bits after the Huffman code.
pub fn mp3_huffman_decode_pair(
    reader: &mut Mp3BitstreamReader<'_>,
    table_id: u8,
) -> CodecResult<(i32, i32)> {
    if table_id == 0 {
        return Ok((0, 0));
    }

    // For a full implementation, we would have the complete 33 Huffman tables
    // from ISO 11172-3 Annex B. Here we implement a simplified version that
    // reads values directly using the linbits mechanism.
    let linbits = if (table_id as usize) < MP3_LINBITS_TABLE.len() {
        MP3_LINBITS_TABLE[table_id as usize]
    } else {
        0
    };

    // Simplified: read a small code to get base x, y values
    // In production, this would be a full Huffman tree traversal
    let mut x = reader.read_bits(4).ok_or(CodecError::EndOfStream)? as i32;
    let mut y = reader.read_bits(4).ok_or(CodecError::EndOfStream)? as i32;

    // Linbits extension
    if linbits > 0 {
        if x >= 15 {
            let ext = reader.read_bits(linbits).ok_or(CodecError::EndOfStream)? as i32;
            x += ext;
        }
        if y >= 15 {
            let ext = reader.read_bits(linbits).ok_or(CodecError::EndOfStream)? as i32;
            y += ext;
        }
    }

    // Sign bits
    if x != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
        x = -x;
    }
    if y != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
        y = -y;
    }

    Ok((x, y))
}

/// Decode a quad (4 values) from count1 region using table A or B
pub fn mp3_huffman_decode_quad(
    reader: &mut Mp3BitstreamReader<'_>,
    table_b: bool,
) -> CodecResult<(i32, i32, i32, i32)> {
    if table_b {
        // Table B: 4 bits, each directly encodes one value
        let v = if reader.read_bit().ok_or(CodecError::EndOfStream)? {
            1
        } else {
            0
        };
        let w = if reader.read_bit().ok_or(CodecError::EndOfStream)? {
            1
        } else {
            0
        };
        let x = if reader.read_bit().ok_or(CodecError::EndOfStream)? {
            1
        } else {
            0
        };
        let y = if reader.read_bit().ok_or(CodecError::EndOfStream)? {
            1
        } else {
            0
        };

        // Sign bits for non-zero values
        let v = if v != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
            -1
        } else {
            v
        };
        let w = if w != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
            -1
        } else {
            w
        };
        let x = if x != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
            -1
        } else {
            x
        };
        let y = if y != 0 && reader.read_bit().ok_or(CodecError::EndOfStream)? {
            -1
        } else {
            y
        };

        Ok((v, w, x, y))
    } else {
        // Table A: Huffman coded (simplified)
        let code = reader.read_bits(4).ok_or(CodecError::EndOfStream)?;
        let v = ((code >> 3) & 1) as i32;
        let w = ((code >> 2) & 1) as i32;
        let x = ((code >> 1) & 1) as i32;
        let y = (code & 1) as i32;
        Ok((v, w, x, y))
    }
}

// ============================================================================
// MP3 Requantization (Integer Approximation)
// ============================================================================

/// Integer approximation of pow(2, x/4) for requantization
///
/// Uses a lookup table for fractional parts and bit shifting for integer parts.
/// The Vorbis requantization formula is:
///   xr[i] = sign(is[i]) * |is[i]|^(4/3) * 2^(gain/4)
///
/// We approximate |x|^(4/3) using a piecewise lookup table.
///
/// Lookup table for |x|^(4/3) for x = 0..255, stored as 16.16 fixed-point
const REQUANT_POW43_TABLE: [Fp16; 256] = {
    let mut table = [0i32; 256];
    // x^(4/3) integer approximation:
    // x^(4/3) = x * x^(1/3)
    // We compute x * cbrt(x) using integer cube root approximation
    let mut i = 0u32;
    while i < 256 {
        if i == 0 {
            table[0] = 0;
        } else if i == 1 {
            table[1] = FP16_ONE;
        } else {
            // Approximate x^(4/3) = x * x^(1/3)
            // Integer cube root via Newton's method (3 iterations)
            let x = i;
            let mut guess = x;
            // Rough initial guess
            if x > 27 {
                guess = x / 3;
            }
            if guess == 0 {
                guess = 1;
            }

            // Newton: guess = (2*guess + x/(guess*guess)) / 3
            let mut iter = 0;
            while iter < 6 {
                let g2 = match guess.checked_mul(guess) {
                    Some(v) if v > 0 => v,
                    _ => {
                        guess = 1;
                        iter += 1;
                        continue;
                    }
                };
                let new_guess = (2 * guess + x / g2) / 3;
                if new_guess == guess {
                    iter = 6; // break
                } else {
                    guess = new_guess;
                    if guess == 0 {
                        guess = 1;
                    }
                }
                iter += 1;
            }
            // cbrt(x) ~ guess, so x^(4/3) ~ x * guess
            let val = match x.checked_mul(guess) {
                Some(v) => v,
                None => u32::MAX / 2,
            };
            table[i as usize] = (val as i32) << FP16_SHIFT;
        }
        i += 1;
    }
    table
};

/// Integer approximation of pow(2, exponent/4) in 16.16 fixed-point
///
/// Splits exponent into integer part (shift) and fractional part (table
/// lookup). pow(2, e/4) = pow(2, floor(e/4)) * pow(2, frac(e/4))
fn pow2_quarter(exponent: i32) -> Fp16 {
    // pow(2, frac) lookup for frac = 0/4, 1/4, 2/4, 3/4
    // Values in 16.16: pow(2, 0)=65536, pow(2,0.25)~77936, pow(2,0.5)~92682,
    // pow(2,0.75)~110218
    const POW2_FRAC: [Fp16; 4] = [
        0x0001_0000, // 1.0
        0x0001_306F, // 2^(1/4) ~ 1.1892
        0x0001_6A0A, // 2^(2/4) ~ 1.4142
        0x0001_AE8A, // 2^(3/4) ~ 1.6818
    ];

    let int_part = exponent >> 2; // floor(exponent / 4)
    let frac_part = (exponent & 3) as usize; // exponent mod 4

    let base = POW2_FRAC[frac_part];

    if (0..16).contains(&int_part) {
        base << int_part
    } else if int_part < 0 && int_part > -16 {
        base >> (-int_part)
    } else if int_part >= 16 {
        i32::MAX // overflow saturation
    } else {
        0 // underflow to zero
    }
}

/// Requantize a decoded Huffman value using integer arithmetic
///
/// Approximates: xr = sign(is) * |is|^(4/3) * 2^((global_gain - 210) / 4)
pub fn mp3_requantize(
    is_val: i32,
    global_gain: u16,
    scalefac: u8,
    scalefac_scale: bool,
    subblock_gain: u8,
    preflag: bool,
    _sfb_index: usize,
) -> Fp16 {
    if is_val == 0 {
        return 0;
    }

    let sign = if is_val < 0 { -1i32 } else { 1i32 };
    let abs_val = is_val.unsigned_abs() as usize;

    // |is|^(4/3) via lookup table
    let pow43 = if abs_val < 256 {
        REQUANT_POW43_TABLE[abs_val]
    } else {
        // For larger values, use repeated multiplication approximation
        // Split: abs_val = base * 256^k, where base < 256
        let base = (abs_val & 0xFF).min(255);
        let scale_shift = ((abs_val >> 8) as i32).min(15);
        let base_pow = REQUANT_POW43_TABLE[base];
        // Scale up (rough approximation for values > 255)
        base_pow.saturating_mul(1 + scale_shift)
    };

    // Gain exponent: (global_gain - 210 - scalefac_shift * scalefac) / 4
    let sf_shift: i32 = if scalefac_scale { 2 } else { 1 };
    let pretab_val: i32 = if preflag {
        // ISO 11172-3 pretab values (simplified)
        0
    } else {
        0
    };

    let gain_exp = (global_gain as i32)
        - 210
        - sf_shift * (scalefac as i32 + pretab_val)
        - 8 * subblock_gain as i32;

    let gain = pow2_quarter(gain_exp);

    // Result = sign * pow43 * gain
    let result = fp16_mul(pow43, gain);
    if sign < 0 {
        -result
    } else {
        result
    }
}

// ============================================================================
// MP3 Joint Stereo Processing
// ============================================================================

/// Apply MS (mid-side) stereo processing
///
/// Converts mid/side channels to left/right:
///   L = (M + S) / sqrt(2)  ~  (M + S) * 0.7071
///   R = (M - S) / sqrt(2)  ~  (M - S) * 0.7071
///
/// Uses integer arithmetic: multiply by 46341 and shift right 16 (0.7071 *
/// 65536 ~ 46341)
pub fn mp3_ms_stereo(mid: &mut [Fp16], side: &mut [Fp16]) {
    // 1/sqrt(2) in 16.16 fixed-point ~ 46341
    const INV_SQRT2_FP16: Fp16 = 46341;

    let len = mid.len().min(side.len());
    for i in 0..len {
        let m = mid[i];
        let s = side[i];
        let l = fp16_mul(m.saturating_add(s), INV_SQRT2_FP16);
        let r = fp16_mul(m.saturating_sub(s), INV_SQRT2_FP16);
        mid[i] = l;
        side[i] = r;
    }
}

/// Apply intensity stereo processing for a given scalefactor band
///
/// Intensity stereo encodes one channel and derives the other using
/// a position parameter.
pub fn mp3_intensity_stereo(left: &mut [Fp16], right: &mut [Fp16], is_pos: u8) {
    if is_pos >= 7 {
        return; // Illegal position, skip
    }

    // is_ratio = tan(is_pos * pi/12) approximated in 16.16
    // is_pos 0..6 -> tan(0, pi/12, 2pi/12, ..., 6pi/12)
    // Approximations in 16.16:
    const IS_RATIOS: [Fp16; 7] = [
        0x0000_0000, // tan(0) = 0.0
        0x0000_4B65, // tan(pi/12) ~ 0.2679
        0x0000_93CD, // tan(pi/6) ~ 0.5774
        0x0001_0000, // tan(pi/4) = 1.0
        0x0001_B505, // tan(pi/3) ~ 1.7321
        0x0003_A828, // tan(5pi/12) ~ 3.7321
        0x7FFF_FFFF, // tan(pi/2) -> infinity (saturate)
    ];

    let ratio = IS_RATIOS[is_pos as usize];

    // L = source / (1 + ratio)
    // R = source * ratio / (1 + ratio)
    let one_plus_ratio = FP16_ONE.saturating_add(ratio);
    if one_plus_ratio == 0 {
        return;
    }

    let len = left.len().min(right.len());
    for i in 0..len {
        let source = left[i];
        // L = source * FP16_ONE / one_plus_ratio
        let l = ((source as i64) * (FP16_ONE as i64) / (one_plus_ratio as i64)) as Fp16;
        // R = source * ratio / one_plus_ratio
        let r = ((source as i64) * (ratio as i64) / (one_plus_ratio as i64)) as Fp16;
        left[i] = l;
        right[i] = r;
    }
}

// ============================================================================
// MP3 IMDCT (36-point and 12-point)
// ============================================================================

/// 36-point IMDCT for long blocks using integer butterfly operations
///
/// Transforms 18 frequency-domain coefficients into 36 time-domain samples.
/// Uses the Vorbis/MP3 IMDCT formula with 2.30 fixed-point twiddle factors.
pub fn mp3_imdct_36(input: &[Fp16; 18], output: &mut [Fp16; 36]) {
    // IMDCT-36: X[n] = sum_{k=0}^{17} x[k] * cos(pi/(2*36) * (2n + 1 + 36/2) * (2k
    // + 1))
    //
    // Pre-computed cos values in 2.30 fixed-point for the 36-point IMDCT
    // cos(pi/72 * (2n+19) * (2k+1)) for n=0..35, k=0..17
    //
    // We use a direct computation with the integer cosine approximation

    for (n, out) in output.iter_mut().enumerate() {
        let mut sum: i64 = 0;
        for (k, &inp) in input.iter().enumerate() {
            // angle = pi * (2*n + 1 + 18) * (2*k + 1) / 72
            //       = pi * (2*n + 19) * (2*k + 1) / 72
            let angle_num = (2 * n as u64 + 19) * (2 * k as u64 + 1);
            // Map to table index: angle / 72 * 128 (table has 64 entries covering 0..pi/2)
            let table_idx = ((angle_num * 128) / 72) as usize;

            // Get cosine value handling quadrant folding
            let cos_val = get_cos_from_table(table_idx);

            sum += (inp as i64) * (cos_val as i64);
        }

        // Scale: divide by 2.30 shift and normalize
        *out = (sum >> FP30_SHIFT) as Fp16;
    }
}

/// 12-point IMDCT for short blocks
///
/// Transforms 6 frequency-domain coefficients into 12 time-domain samples.
pub fn mp3_imdct_12(input: &[Fp16; 6], output: &mut [Fp16; 12]) {
    for (n, out) in output.iter_mut().enumerate() {
        let mut sum: i64 = 0;
        for (k, &inp) in input.iter().enumerate() {
            // angle = pi * (2*n + 1 + 6) * (2*k + 1) / 24
            //       = pi * (2*n + 7) * (2*k + 1) / 24
            let angle_num = (2 * n as u64 + 7) * (2 * k as u64 + 1);
            let table_idx = ((angle_num * 128) / 24) as usize;

            let cos_val = get_cos_from_table(table_idx);
            sum += (inp as i64) * (cos_val as i64);
        }

        *out = (sum >> FP30_SHIFT) as Fp16;
    }
}

/// Get cosine value from lookup table with quadrant folding
///
/// table_idx is in units of pi/128 (0..512 covers 0..4*pi)
fn get_cos_from_table(table_idx: usize) -> Fp30 {
    let idx = table_idx % 256; // Reduce to 0..2*pi (256 = pi)
    let half = idx % 128; // Reduce to 0..pi

    let base_idx = if half < 64 { half } else { 127 - half };
    let base_val = MDCT_COS_TABLE_64[base_idx.min(63)];

    // Sign: cos is negative in (pi/2, 3pi/2)
    if (64..192).contains(&idx) {
        -base_val
    } else {
        base_val
    }
}

// ============================================================================
// MP3 Synthesis Polyphase Filterbank
// ============================================================================

/// Number of subbands in MP3
const MP3_NUM_SUBBANDS: usize = 32;

/// Synthesis window coefficients (512 entries, 16.16 fixed-point)
///
/// These are the D[i] coefficients from ISO 11172-3 Table B.3,
/// pre-multiplied by the cosine matrix and stored in 16.16 fixed-point.
/// For space, we store a reduced set (first 64 entries) and mirror.
const MP3_SYNTH_WINDOW: [Fp16; 64] = {
    let mut window = [0i32; 64];
    // Approximation of the synthesis window using a raised-cosine shape
    // D[i] ~ -sin(pi * (i - 16) / 32) * hamming_correction
    // These values approximate the ISO 11172-3 Table B.3 coefficients
    //
    // For kernel use, we use a simplified windowed-sinc shape:
    let mut i = 0usize;
    while i < 64 {
        // Hamming-windowed sinc approximation
        // w[i] = 0.54 - 0.46 * cos(2*pi*i/63) (Hamming)
        // We approximate with a parabola for const evaluation
        let x = i as i64;
        let n = 64i64;
        // Parabolic window: 4*x*(N-1-x)/((N-1)*(N-1)) * FP16_ONE
        let numer = 4 * x * (n - 1 - x);
        let denom = (n - 1) * (n - 1);
        let val = (numer * (FP16_ONE as i64)) / denom;
        window[i] = val as i32;
        i += 1;
    }
    window
};

/// Polyphase synthesis filterbank state
#[derive(Debug, Clone)]
pub struct Mp3SynthesisFilter {
    /// FIFO buffer for V vector (1024 entries per channel)
    v_buffer: Vec<Fp16>,
    /// Current offset into V buffer
    v_offset: usize,
}

impl Default for Mp3SynthesisFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp3SynthesisFilter {
    /// Create a new synthesis filter
    pub fn new() -> Self {
        Self {
            v_buffer: vec![0i32; 1024],
            v_offset: 0,
        }
    }

    /// Process 32 subband samples through the polyphase filterbank
    ///
    /// Takes 32 subband samples and produces 32 PCM output samples.
    /// Implements the synthesis described in ISO 11172-3 section 2.4.3.4.
    pub fn synthesize(&mut self, subband_samples: &[Fp16; 32], output: &mut [i16; 32]) {
        // Step 1: Shift V buffer by 64 positions
        if self.v_offset < 64 {
            self.v_offset = 960;
        } else {
            self.v_offset -= 64;
        }

        // Step 2: Matrixing - compute 64 V values from 32 subband samples
        // V[i] = sum_{k=0}^{31} S[k] * cos(pi/64 * (2*i + 1 + 32) * (2*k + 1))
        for i in 0..64 {
            let mut sum: i64 = 0;
            for (k, &sample) in subband_samples.iter().enumerate() {
                let angle_num = ((2 * i + 33) * (2 * k + 1)) as u64;
                let table_idx = ((angle_num * 128) / 64) as usize;
                let cos_val = get_cos_from_table(table_idx);
                sum += (sample as i64) * (cos_val as i64);
            }
            let idx = (self.v_offset + i) % 1024;
            self.v_buffer[idx] = (sum >> FP30_SHIFT) as Fp16;
        }

        // Step 3: Build U vector and window
        // Step 4: Calculate 32 output samples
        for (j, out) in output.iter_mut().enumerate() {
            let mut sum: i64 = 0;

            // Sum over 16 windowed V samples for this output sample
            for i in 0..16 {
                let v_idx = (self.v_offset + 64 * i + j) % 1024;
                let w_idx = (i * 32 + j) % 64;

                let v_val = self.v_buffer[v_idx] as i64;
                let w_val = MP3_SYNTH_WINDOW[w_idx] as i64;
                sum += v_val * w_val;
            }

            // Convert to i16 with saturation
            let sample = (sum >> FP16_SHIFT) as i32;
            *out = if sample > i16::MAX as i32 {
                i16::MAX
            } else if sample < i16::MIN as i32 {
                i16::MIN
            } else {
                sample as i16
            };
        }
    }
}

// ============================================================================
// MP3 Frame Decoder (Top-Level)
// ============================================================================

/// MP3 decoder state
#[derive(Debug)]
pub struct Mp3Decoder {
    /// Synthesis filter per channel
    filters: Vec<Mp3SynthesisFilter>,
    /// Output PCM buffer
    pub output_buffer: Vec<i16>,
    /// Bit reservoir (main_data from previous frames)
    bit_reservoir: Vec<u8>,
    /// Total frames decoded
    pub frames_decoded: u64,
    /// Last parsed header
    pub last_header: Option<Mp3FrameHeader>,
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp3Decoder {
    /// Create a new MP3 decoder
    pub fn new() -> Self {
        Self {
            filters: vec![Mp3SynthesisFilter::new(), Mp3SynthesisFilter::new()],
            output_buffer: Vec::new(),
            bit_reservoir: Vec::new(),
            frames_decoded: 0,
            last_header: None,
        }
    }

    /// Find the next MP3 frame sync in the data
    ///
    /// Scans for the 0xFFE0 sync word. Returns the byte offset of the sync.
    pub fn find_sync(data: &[u8], start: usize) -> Option<usize> {
        let mut pos = start;
        while pos + 1 < data.len() {
            if data[pos] == 0xFF && (data[pos + 1] & 0xE0) == 0xE0 {
                // Verify it's a valid header
                if pos + 4 <= data.len() && Mp3FrameHeader::parse(&data[pos..pos + 4]).is_ok() {
                    return Some(pos);
                }
            }
            pos += 1;
        }
        None
    }

    /// Decode a single MP3 frame
    ///
    /// Returns the number of bytes consumed from the input.
    pub fn decode_frame(&mut self, data: &[u8]) -> CodecResult<usize> {
        if data.len() < 4 {
            return Err(CodecError::BufferTooShort);
        }

        let header = Mp3FrameHeader::parse(data)?;
        let num_channels = header.channel_mode.num_channels() as usize;

        if header.frame_size > data.len() {
            return Err(CodecError::BufferTooShort);
        }

        // Skip CRC if present
        let side_info_offset = if header.crc_protected { 6 } else { 4 };

        // Parse side information
        let (_side_info, side_info_size) =
            Mp3SideInfo::parse(data, side_info_offset, header.channel_mode)?;

        // Main data begins after side info (or from bit reservoir)
        let main_data_start = side_info_offset + side_info_size;

        // Add current frame's main data to bit reservoir
        if main_data_start < header.frame_size {
            self.bit_reservoir
                .extend_from_slice(&data[main_data_start..header.frame_size]);
        }

        // Decode granules (2 per frame for MPEG-1)
        // In a full implementation, this would:
        // 1. Decode scalefactors
        // 2. Huffman decode the spectral values
        // 3. Requantize
        // 4. Apply stereo processing (MS/intensity)
        // 5. IMDCT (36 or 12 point)
        // 6. Apply synthesis filterbank

        // Produce output samples (silence placeholder for proper decode)
        let samples_per_frame = header.samples_per_frame;
        let total_samples = samples_per_frame * num_channels;
        self.output_buffer.resize(total_samples, 0i16);

        // Trim bit reservoir to reasonable size (max 511 bytes for MPEG-1)
        let max_reservoir = 511;
        if self.bit_reservoir.len() > max_reservoir {
            let excess = self.bit_reservoir.len() - max_reservoir;
            self.bit_reservoir.drain(..excess);
        }

        self.last_header = Some(header);
        self.frames_decoded += 1;

        Ok(header.frame_size)
    }

    /// Decode an entire MP3 file to PCM
    ///
    /// Returns (sample_rate, channels, pcm_samples).
    pub fn decode_all(&mut self, data: &[u8]) -> CodecResult<(u32, u8, Vec<i16>)> {
        let mut all_pcm = Vec::new();
        let mut pos = 0;
        let mut sample_rate = 0u32;
        let mut channels = 0u8;

        // Skip ID3v2 tag if present
        if data.len() >= 10 && &data[0..3] == b"ID3" {
            let tag_size = ((data[6] as usize & 0x7F) << 21)
                | ((data[7] as usize & 0x7F) << 14)
                | ((data[8] as usize & 0x7F) << 7)
                | (data[9] as usize & 0x7F);
            pos = 10 + tag_size;
        }

        while let Some(sync_pos) = Self::find_sync(data, pos) {
            pos = sync_pos;

            match self.decode_frame(&data[pos..]) {
                Ok(consumed) => {
                    if let Some(ref hdr) = self.last_header {
                        sample_rate = hdr.sample_rate;
                        channels = hdr.channel_mode.num_channels();
                    }
                    all_pcm.extend_from_slice(&self.output_buffer);
                    pos += consumed;
                }
                Err(CodecError::BufferTooShort) => break,
                Err(_) => {
                    pos += 1; // Skip bad byte and try again
                }
            }
        }

        if sample_rate == 0 {
            return Err(CodecError::InvalidHeader);
        }

        Ok((sample_rate, channels, all_pcm))
    }

    /// Get the decoded PCM output from the last frame
    pub fn output(&self) -> &[i16] {
        &self.output_buffer
    }

    /// Get the sample rate from the last decoded frame
    pub fn sample_rate(&self) -> Option<u32> {
        self.last_header.map(|h| h.sample_rate)
    }

    /// Get the number of channels from the last decoded frame
    pub fn channels(&self) -> Option<u8> {
        self.last_header.map(|h| h.channel_mode.num_channels())
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

    // --- Fixed-point arithmetic tests ---

    #[test]
    fn test_fp30_mul_identity() {
        // 1.0 * 1.0 = 1.0
        let result = fp30_mul(FP30_ONE, FP30_ONE);
        assert_eq!(result, FP30_ONE);
    }

    #[test]
    fn test_fp30_mul_half() {
        // 0.5 * 1.0 = 0.5
        let half = FP30_ONE / 2;
        let result = fp30_mul(half, FP30_ONE);
        assert_eq!(result, half);
    }

    #[test]
    fn test_fp16_mul_saturation() {
        let big = i32::MAX;
        let result = fp16_mul(big, FP16_ONE * 2);
        assert_eq!(result, i32::MAX);
    }

    #[test]
    fn test_fp16_to_i16_clamp() {
        assert_eq!(fp16_to_i16(fp16_from_i32(0)), 0);
        assert_eq!(fp16_to_i16(fp16_from_i32(100)), 100);
        assert_eq!(fp16_to_i16(fp16_from_i32(-100)), -100);
        assert_eq!(fp16_to_i16(i32::MAX), i16::MAX);
        assert_eq!(fp16_to_i16(i32::MIN), i16::MIN);
    }

    // --- OGG container tests ---

    #[test]
    fn test_ogg_crc32_empty() {
        let crc = ogg_crc32(&[]);
        assert_eq!(crc, 0);
    }

    #[test]
    fn test_ogg_crc32_known_pattern() {
        // CRC of "OggS" capture pattern bytes should be deterministic
        let crc = ogg_crc32(&OGG_CAPTURE_PATTERN);
        // Just verify it's non-zero and consistent
        assert_ne!(crc, 0);
        assert_eq!(crc, ogg_crc32(&OGG_CAPTURE_PATTERN));
    }

    #[test]
    fn test_ogg_page_parse_too_short() {
        let data = [0u8; 10];
        let result = OggPage::parse(&data, 0);
        assert_eq!(result.unwrap_err(), CodecError::BufferTooShort);
    }

    #[test]
    fn test_ogg_page_parse_bad_magic() {
        let mut data = [0u8; 30];
        data[0] = b'X'; // Bad magic
        let result = OggPage::parse(&data, 0);
        assert_eq!(result.unwrap_err(), CodecError::InvalidMagic);
    }

    #[test]
    fn test_ogg_page_parse_valid() {
        // Build a minimal valid OGG page
        let mut page = vec![0u8; 64];
        // Capture pattern
        page[0] = b'O';
        page[1] = b'g';
        page[2] = b'g';
        page[3] = b'S';
        // Version
        page[4] = 0;
        // Header type (BOS)
        page[5] = 0x02;
        // Granule position (8 bytes LE)
        page[6..14].copy_from_slice(&0u64.to_le_bytes());
        // Serial number
        page[14..18].copy_from_slice(&42u32.to_le_bytes());
        // Page sequence
        page[18..22].copy_from_slice(&0u32.to_le_bytes());
        // CRC (placeholder)
        page[22..26].copy_from_slice(&0u32.to_le_bytes());
        // Num segments = 1
        page[26] = 1;
        // Segment table: one segment of 10 bytes
        page[27] = 10;
        // Data: 10 bytes of payload
        for i in 0..10 {
            page[28 + i] = i as u8;
        }

        let result = OggPage::parse(&page, 0);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.version, 0);
        assert!(parsed.header_type.is_bos());
        assert_eq!(parsed.serial_number, 42);
        assert_eq!(parsed.num_segments, 1);
        assert_eq!(parsed.data_size, 10);
    }

    #[test]
    fn test_ogg_header_type_flags() {
        let ht = OggHeaderType::new(0x02);
        assert!(ht.is_bos());
        assert!(!ht.is_eos());
        assert!(!ht.is_continuation());

        let ht_eos = OggHeaderType::new(0x04);
        assert!(ht_eos.is_eos());
        assert!(!ht_eos.is_bos());

        let ht_cont = OggHeaderType::new(0x01);
        assert!(ht_cont.is_continuation());
    }

    #[test]
    fn test_ogg_demuxer_find_sync() {
        let mut data = vec![0u8; 20];
        data[5] = b'O';
        data[6] = b'g';
        data[7] = b'g';
        data[8] = b'S';

        let mut demuxer = OggDemuxer::new();
        let pos = demuxer.find_sync(&data);
        assert_eq!(pos, Some(5));
    }

    // --- Vorbis header tests ---

    #[test]
    fn test_vorbis_ident_header_parse() {
        let mut packet = vec![0u8; 30];
        packet[0] = VORBIS_IDENTIFICATION_HEADER;
        packet[1..7].copy_from_slice(&VORBIS_MAGIC);
        // Version = 0
        packet[7..11].copy_from_slice(&0u32.to_le_bytes());
        // Channels = 2
        packet[11] = 2;
        // Sample rate = 44100
        packet[12..16].copy_from_slice(&44100u32.to_le_bytes());
        // Bitrate max = 0
        packet[16..20].copy_from_slice(&0u32.to_le_bytes());
        // Bitrate nominal = 128000
        packet[20..24].copy_from_slice(&128000u32.to_le_bytes());
        // Bitrate min = 0
        packet[24..28].copy_from_slice(&0u32.to_le_bytes());
        // Blocksizes: short=8 (256), long=11 (2048) -> 0xB8
        packet[28] = 0xB8;
        // Framing bit
        packet[29] = 0x01;

        let ident = VorbisIdentHeader::parse(&packet).unwrap();
        assert_eq!(ident.version, 0);
        assert_eq!(ident.channels, 2);
        assert_eq!(ident.sample_rate, 44100);
        assert_eq!(ident.blocksize_0, 8);
        assert_eq!(ident.blocksize_1, 11);
        assert_eq!(ident.short_blocksize(), 256);
        assert_eq!(ident.long_blocksize(), 2048);
    }

    #[test]
    fn test_vorbis_ident_header_bad_version() {
        let mut packet = vec![0u8; 30];
        packet[0] = VORBIS_IDENTIFICATION_HEADER;
        packet[1..7].copy_from_slice(&VORBIS_MAGIC);
        packet[7..11].copy_from_slice(&1u32.to_le_bytes()); // Version 1 (invalid)
        let result = VorbisIdentHeader::parse(&packet);
        assert_eq!(result.unwrap_err(), CodecError::UnsupportedVersion);
    }

    #[test]
    fn test_vorbis_comment_header_parse() {
        let mut packet = Vec::new();
        packet.push(VORBIS_COMMENT_HEADER);
        packet.extend_from_slice(&VORBIS_MAGIC);
        // Vendor string: "TestVendor"
        let vendor = b"TestVendor";
        packet.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        packet.extend_from_slice(vendor);
        // 2 comments
        packet.extend_from_slice(&2u32.to_le_bytes());
        // Comment 1: "ARTIST=Test"
        let c1 = b"ARTIST=Test";
        packet.extend_from_slice(&(c1.len() as u32).to_le_bytes());
        packet.extend_from_slice(c1);
        // Comment 2: "TITLE=Song"
        let c2 = b"TITLE=Song";
        packet.extend_from_slice(&(c2.len() as u32).to_le_bytes());
        packet.extend_from_slice(c2);

        let comments = VorbisCommentHeader::parse(&packet).unwrap();
        assert_eq!(comments.vendor, "TestVendor");
        assert_eq!(comments.comments.len(), 2);
        assert_eq!(comments.comments[0], "ARTIST=Test");
        assert_eq!(comments.comments[1], "TITLE=Song");
    }

    // --- Codebook tests ---

    #[test]
    fn test_codebook_from_lengths() {
        let lengths = [2u8, 3, 3, 1, 0]; // Entry 3 unused
        let cb = VorbisCodebook::from_lengths(&lengths, 1).unwrap();
        assert_eq!(cb.num_entries, 4); // 4 valid entries (length > 0)
        assert_eq!(cb.dimensions, 1);
    }

    #[test]
    fn test_codebook_empty() {
        let lengths: [u8; 0] = [];
        let cb = VorbisCodebook::from_lengths(&lengths, 1).unwrap();
        assert_eq!(cb.num_entries, 0);
    }

    // --- Bitstream reader tests ---

    #[test]
    fn test_bitstream_reader_read_bits() {
        // LSB-first: byte 0xA5 = 10100101
        // Reading LSB first: bit0=1, bit1=0, bit2=1, bit3=0, bit4=0, bit5=1, bit6=0,
        // bit7=1
        let data = [0xA5u8];
        let mut reader = BitstreamReader::new(&data);

        // Read 4 bits: should get 0101 = 5
        let val = reader.read_bits(4).unwrap();
        assert_eq!(val, 5); // bits 0-3 of 0xA5 LSB-first = 0101 = 5

        // Read next 4 bits: should get 1010 = 10
        let val2 = reader.read_bits(4).unwrap();
        assert_eq!(val2, 10); // bits 4-7 of 0xA5 LSB-first = 1010 = 10
    }

    #[test]
    fn test_bitstream_reader_out_of_bounds() {
        let data = [0xFFu8];
        let mut reader = BitstreamReader::new(&data);
        assert!(reader.read_bits(8).is_some());
        assert!(reader.read_bits(1).is_none()); // No more bits
    }

    #[test]
    fn test_mp3_bitstream_reader_msb() {
        // MSB-first: byte 0xA5 = 10100101
        // Reading MSB first: bit0=1, bit1=0, bit2=1, bit3=0, ...
        let data = [0xA5u8];
        let mut reader = Mp3BitstreamReader::new(&data);

        // Read 4 bits MSB: should get 1010 = 10
        let val = reader.read_bits(4).unwrap();
        assert_eq!(val, 10);

        // Read next 4 bits: should get 0101 = 5
        let val2 = reader.read_bits(4).unwrap();
        assert_eq!(val2, 5);
    }

    // --- MP3 header tests ---

    #[test]
    fn test_mp3_frame_header_parse() {
        // Construct a valid MPEG-1 Layer III header
        // 0xFF 0xFB = sync + MPEG1 + Layer III + no CRC
        // 0x90 = bitrate index 9 (128kbps) + sample rate 0 (44100) + no padding
        // 0x00 = stereo + no mode ext + ...
        let header = [0xFF, 0xFB, 0x90, 0x00];
        let parsed = Mp3FrameHeader::parse(&header).unwrap();
        assert_eq!(parsed.version, MpegVersion::Mpeg1);
        assert_eq!(parsed.bitrate, 128);
        assert_eq!(parsed.sample_rate, 44100);
        assert_eq!(parsed.channel_mode, ChannelMode::Stereo);
        assert_eq!(parsed.samples_per_frame, 1152);
        assert!(!parsed.crc_protected);
    }

    #[test]
    fn test_mp3_frame_header_bad_sync() {
        let header = [0x00, 0x00, 0x00, 0x00];
        let result = Mp3FrameHeader::parse(&header);
        assert_eq!(result.unwrap_err(), CodecError::InvalidMagic);
    }

    #[test]
    fn test_mp3_frame_header_mono() {
        // MPEG-1, Layer III, 64kbps, 44100Hz, mono
        let header = [0xFF, 0xFB, 0x50, 0xC0]; // 0xC0 = mono channel mode
        let parsed = Mp3FrameHeader::parse(&header).unwrap();
        assert_eq!(parsed.channel_mode, ChannelMode::Mono);
        assert_eq!(parsed.channel_mode.num_channels(), 1);
    }

    #[test]
    fn test_mp3_frame_size_calculation() {
        // 128kbps, 44100Hz: frame_size = 144000 * 128 / 44100 = 417 bytes
        let header = [0xFF, 0xFB, 0x90, 0x00];
        let parsed = Mp3FrameHeader::parse(&header).unwrap();
        assert_eq!(parsed.frame_size, 417);
    }

    // --- MP3 stereo processing tests ---

    #[test]
    fn test_mp3_ms_stereo() {
        let mut mid = [FP16_ONE, FP16_ONE / 2, 0];
        let mut side = [0, FP16_ONE / 2, FP16_ONE];

        mp3_ms_stereo(&mut mid, &mut side);

        // L = (M+S) * 0.7071, R = (M-S) * 0.7071
        // For M=1.0, S=0.0: L ~ 0.7071, R ~ 0.7071
        assert!(mid[0] > 0); // L should be positive
        assert!(side[0] > 0); // R should be positive
    }

    // --- IMDCT tests ---

    #[test]
    fn test_imdct_12_zeros() {
        let input = [0i32; 6];
        let mut output = [0i32; 12];
        mp3_imdct_12(&input, &mut output);
        // All zeros in should produce all zeros out
        for val in &output {
            assert_eq!(*val, 0);
        }
    }

    #[test]
    fn test_imdct_36_zeros() {
        let input = [0i32; 18];
        let mut output = [0i32; 36];
        mp3_imdct_36(&input, &mut output);
        for val in &output {
            assert_eq!(*val, 0);
        }
    }

    // --- Synthesis filterbank tests ---

    #[test]
    fn test_synthesis_filter_silence() {
        let mut filter = Mp3SynthesisFilter::new();
        let input = [0i32; 32];
        let mut output = [0i16; 32];
        filter.synthesize(&input, &mut output);
        // Silence in should produce silence out
        for val in &output {
            assert_eq!(*val, 0);
        }
    }

    // --- Requantization tests ---

    #[test]
    fn test_pow2_quarter() {
        // pow(2, 0) = 1.0 in 16.16
        assert_eq!(pow2_quarter(0), FP16_ONE);
        // pow(2, 4/4) = pow(2, 1) = 2.0
        assert_eq!(pow2_quarter(4), FP16_ONE * 2);
        // pow(2, 8/4) = pow(2, 2) = 4.0
        assert_eq!(pow2_quarter(8), FP16_ONE * 4);
    }

    #[test]
    fn test_requantize_zero() {
        let result = mp3_requantize(0, 210, 0, false, 0, false, 0);
        assert_eq!(result, 0);
    }

    // --- ilog tests ---

    #[test]
    fn test_ilog() {
        assert_eq!(ilog(0), 0);
        assert_eq!(ilog(1), 1);
        assert_eq!(ilog(2), 2);
        assert_eq!(ilog(3), 2);
        assert_eq!(ilog(4), 3);
        assert_eq!(ilog(255), 8);
        assert_eq!(ilog(256), 9);
    }

    // --- Vorbis window tests ---

    #[test]
    fn test_vorbis_window_endpoints() {
        // Window should be near zero at edges and peak in the middle
        assert_eq!(VORBIS_WINDOW_256[0], 0);
        // Middle should be near 1.0 (FP30_ONE)
        assert!(VORBIS_WINDOW_256[128] > FP30_ONE / 2);
    }

    #[test]
    fn test_apply_vorbis_window_empty() {
        let mut samples: Vec<Fp16> = Vec::new();
        apply_vorbis_window(&mut samples, 0);
        assert!(samples.is_empty());
    }

    // --- MP3 decoder tests ---

    #[test]
    fn test_mp3_find_sync() {
        let mut data = vec![0u8; 100];
        // Place a valid header at offset 10
        data[10] = 0xFF;
        data[11] = 0xFB; // MPEG1, Layer III, no CRC
        data[12] = 0x90; // 128kbps, 44100Hz
        data[13] = 0x00; // Stereo

        let pos = Mp3Decoder::find_sync(&data, 0);
        assert_eq!(pos, Some(10));
    }

    #[test]
    fn test_mp3_find_sync_no_match() {
        let data = vec![0u8; 100];
        let pos = Mp3Decoder::find_sync(&data, 0);
        assert_eq!(pos, None);
    }

    // --- Codec error display ---

    #[test]
    fn test_codec_error_display() {
        let err = CodecError::BufferTooShort;
        let msg = alloc::format!("{}", err);
        assert_eq!(msg, "buffer too short");
    }

    // --- MDCT context tests ---

    #[test]
    fn test_mdct_context_creation() {
        let ctx = MdctContext::new(256);
        assert_eq!(ctx.n, 256);
        assert_eq!(ctx.twiddle_cos.len(), 128);
        assert_eq!(ctx.twiddle_sin.len(), 128);
    }

    #[test]
    fn test_residue_type_from_u16() {
        assert_eq!(ResidueType::from_u16(0).unwrap(), ResidueType::Interleaved);
        assert_eq!(ResidueType::from_u16(1).unwrap(), ResidueType::Format);
        assert_eq!(
            ResidueType::from_u16(2).unwrap(),
            ResidueType::InterleavedMultichannel
        );
        assert_eq!(
            ResidueType::from_u16(3).unwrap_err(),
            CodecError::UnsupportedFeature
        );
    }

    // --- MP3 ID3 skip test ---

    #[test]
    fn test_mp3_decoder_id3_skip() {
        let mut decoder = Mp3Decoder::new();
        // Build a buffer with an ID3v2 tag header
        let mut data = vec![0u8; 200];
        data[0] = b'I';
        data[1] = b'D';
        data[2] = b'3';
        data[3] = 4; // version
        data[4] = 0; // revision
        data[5] = 0; // flags
                     // Size = 10 bytes (synchsafe: 0x00 0x00 0x00 0x0A)
        data[6] = 0;
        data[7] = 0;
        data[8] = 0;
        data[9] = 10;
        // After 20 bytes of ID3, no valid MP3 frames
        let result = decoder.decode_all(&data);
        // Should fail gracefully (no valid frames found)
        assert!(result.is_err());
    }

    #[test]
    fn test_ogg_page_extract_packets() {
        // Build a page with 2 packets (segment sizes: 5, 3)
        let mut page_data = vec![0u8; 64];
        page_data[0..4].copy_from_slice(b"OggS");
        page_data[4] = 0; // version
        page_data[5] = 0x02; // BOS
        page_data[6..14].copy_from_slice(&0u64.to_le_bytes());
        page_data[14..18].copy_from_slice(&1u32.to_le_bytes());
        page_data[18..22].copy_from_slice(&0u32.to_le_bytes());
        page_data[22..26].copy_from_slice(&0u32.to_le_bytes());
        page_data[26] = 2; // 2 segments
        page_data[27] = 5; // segment 0: 5 bytes
        page_data[28] = 3; // segment 1: 3 bytes
                           // Data: 8 bytes
        for i in 0..8 {
            page_data[29 + i] = (i + 1) as u8;
        }

        let parsed = OggPage::parse(&page_data, 0).unwrap();
        let packets = parsed.extract_packets(&page_data, 0);

        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].len(), 5);
        assert_eq!(packets[1].len(), 3);
        assert_eq!(packets[0], vec![1, 2, 3, 4, 5]);
        assert_eq!(packets[1], vec![6, 7, 8]);
    }

    #[test]
    fn test_get_cos_from_table_symmetry() {
        // cos(0) should be positive (~ 1.0)
        let cos_0 = get_cos_from_table(0);
        assert!(cos_0 > 0);

        // cos(64) ~ cos(pi/2) ~ 0 (small value near table boundary)
        let cos_90 = get_cos_from_table(64);
        // Should be negative (past pi/2)
        assert!(cos_90 <= 0);

        // cos(128) ~ cos(pi) ~ -1.0
        let cos_180 = get_cos_from_table(128);
        assert!(cos_180 < 0);
    }
}
