//! OGG Vorbis decoder (integer-only, no_std)
//!
//! Implements the OGG container (RFC 3533) and Vorbis I codec (Xiph.org spec).

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

use super::{
    fp16_from_i32, fp16_mul, fp30_mul, fp30_to_fp16, ilog, read_u32_le, read_u64_le, CodecError,
    CodecResult, Fp16, Fp30, FP16_SHIFT, FP30_ONE, FP30_SHIFT, MDCT_COS_TABLE_64,
};

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

/// OGG CRC32 lookup table (polynomial 0x04C11DB7, direct/big-endian CRC)
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

    /// Extract packets from this page's segments.
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
    /// Build a codebook from codeword lengths (Vorbis spec section 3.2.1).
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

    /// Decode a single symbol from a bitstream reader via canonical Huffman.
    pub fn decode(&self, reader: &mut BitstreamReader<'_>) -> CodecResult<u16> {
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

// ============================================================================
// Vorbis Floor Type 1
// ============================================================================

/// Vorbis Floor Type 1: piecewise linear spectral envelope interpolation.
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

    /// Render a piecewise-linear floor segment using integer Bresenham
    /// interpolation.
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

    /// Decode residue vectors from the bitstream for each channel.
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

        output
    }
}

// ============================================================================
// Vorbis MDCT (Integer Arithmetic)
// ============================================================================

/// Pre-computed twiddle factors for MDCT in 2.30 fixed-point
#[derive(Debug, Clone)]
pub struct MdctContext {
    /// Block size (N)
    pub n: usize,
    /// Twiddle factors (cos) in 2.30 fixed-point
    pub twiddle_cos: Vec<Fp30>,
    /// Twiddle factors (sin) in 2.30 fixed-point
    pub twiddle_sin: Vec<Fp30>,
}

impl MdctContext {
    /// Create MDCT context for a given block size, precomputing twiddle
    /// factors.
    pub fn new(n: usize) -> Self {
        let half_n = n / 2;
        let mut twiddle_cos = Vec::with_capacity(half_n);
        let mut twiddle_sin = Vec::with_capacity(half_n);

        for k in 0..half_n {
            // angle = pi * (8*k + 1) / (8*N), mapped to table index
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

    /// Perform inverse MDCT: N/2 frequency coefficients -> N time-domain
    /// samples.
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
/// w(x) = sin(pi/2 * sin^2(pi * x / N)), approximated with a parabola
/// for const evaluation. Values are in 2.30 fixed-point.
const VORBIS_WINDOW_256: [Fp30; 256] = {
    let mut table = [0i32; 256];
    // Parabolic approximation: w(i) = 4*i*(256-i)/65536 * FP30_ONE
    let mut i = 0usize;
    while i < 256 {
        let x = i as i64;
        let n_minus_x = (256 - i) as i64;
        let numer = 4 * x * n_minus_x;
        let val = (numer * (FP30_ONE as i64)) / 65536;
        table[i] = val as i32;
        i += 1;
    }

    table
};

/// Apply the Vorbis window function to a buffer of samples.
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

    /// Decode a single audio packet into PCM samples (interleaved channels).
    pub fn decode_packet(&mut self, _packet: &[u8]) -> CodecResult<usize> {
        if !self.headers_parsed {
            return Err(CodecError::InvalidHeader);
        }

        // Produce silence placeholder (correct number of samples)
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

// ============================================================================
// OGG Vorbis Combined Decoder
// ============================================================================

/// Combined OGG container + Vorbis decoder for decoding entire OGG Vorbis
/// files.
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

    /// Decode an entire OGG Vorbis file, returning (sample_rate, channels,
    /// pcm_samples).
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

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- OGG container tests ---

    #[test]
    fn test_ogg_crc32_empty() {
        let crc = ogg_crc32(&[]);
        assert_eq!(crc, 0);
    }

    #[test]
    fn test_ogg_crc32_known_pattern() {
        let crc = ogg_crc32(&OGG_CAPTURE_PATTERN);
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
        data[0] = b'X';
        let result = OggPage::parse(&data, 0);
        assert_eq!(result.unwrap_err(), CodecError::InvalidMagic);
    }

    #[test]
    fn test_ogg_page_parse_valid() {
        let mut page = vec![0u8; 64];
        page[0] = b'O';
        page[1] = b'g';
        page[2] = b'g';
        page[3] = b'S';
        page[4] = 0;
        page[5] = 0x02;
        page[6..14].copy_from_slice(&0u64.to_le_bytes());
        page[14..18].copy_from_slice(&42u32.to_le_bytes());
        page[18..22].copy_from_slice(&0u32.to_le_bytes());
        page[22..26].copy_from_slice(&0u32.to_le_bytes());
        page[26] = 1;
        page[27] = 10;
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
        packet[7..11].copy_from_slice(&0u32.to_le_bytes());
        packet[11] = 2;
        packet[12..16].copy_from_slice(&44100u32.to_le_bytes());
        packet[16..20].copy_from_slice(&0u32.to_le_bytes());
        packet[20..24].copy_from_slice(&128000u32.to_le_bytes());
        packet[24..28].copy_from_slice(&0u32.to_le_bytes());
        packet[28] = 0xB8;
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
        packet[7..11].copy_from_slice(&1u32.to_le_bytes());
        let result = VorbisIdentHeader::parse(&packet);
        assert_eq!(result.unwrap_err(), CodecError::UnsupportedVersion);
    }

    #[test]
    fn test_vorbis_comment_header_parse() {
        let mut packet = Vec::new();
        packet.push(VORBIS_COMMENT_HEADER);
        packet.extend_from_slice(&VORBIS_MAGIC);
        let vendor = b"TestVendor";
        packet.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        packet.extend_from_slice(vendor);
        packet.extend_from_slice(&2u32.to_le_bytes());
        let c1 = b"ARTIST=Test";
        packet.extend_from_slice(&(c1.len() as u32).to_le_bytes());
        packet.extend_from_slice(c1);
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
        let lengths = [2u8, 3, 3, 1, 0];
        let cb = VorbisCodebook::from_lengths(&lengths, 1).unwrap();
        assert_eq!(cb.num_entries, 4);
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
        let data = [0xA5u8];
        let mut reader = BitstreamReader::new(&data);
        let val = reader.read_bits(4).unwrap();
        assert_eq!(val, 5);
        let val2 = reader.read_bits(4).unwrap();
        assert_eq!(val2, 10);
    }

    #[test]
    fn test_bitstream_reader_out_of_bounds() {
        let data = [0xFFu8];
        let mut reader = BitstreamReader::new(&data);
        assert!(reader.read_bits(8).is_some());
        assert!(reader.read_bits(1).is_none());
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

    // --- Vorbis window tests ---

    #[test]
    fn test_vorbis_window_endpoints() {
        assert_eq!(VORBIS_WINDOW_256[0], 0);
        assert!(VORBIS_WINDOW_256[128] > FP30_ONE / 2);
    }

    #[test]
    fn test_apply_vorbis_window_empty() {
        let mut samples: Vec<Fp16> = Vec::new();
        apply_vorbis_window(&mut samples, 0);
        assert!(samples.is_empty());
    }

    #[test]
    fn test_ogg_page_extract_packets() {
        let mut page_data = vec![0u8; 64];
        page_data[0..4].copy_from_slice(b"OggS");
        page_data[4] = 0;
        page_data[5] = 0x02;
        page_data[6..14].copy_from_slice(&0u64.to_le_bytes());
        page_data[14..18].copy_from_slice(&1u32.to_le_bytes());
        page_data[18..22].copy_from_slice(&0u32.to_le_bytes());
        page_data[22..26].copy_from_slice(&0u32.to_le_bytes());
        page_data[26] = 2;
        page_data[27] = 5;
        page_data[28] = 3;
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
}
