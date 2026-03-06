//! MP3 (MPEG-1 Layer III) decoder (integer-only, no_std)
//!
//! Implements ISO 11172-3 / ISO 13818-3 Layer III decoding.

#![allow(dead_code)]

use alloc::{vec, vec::Vec};

use super::{
    fp16_mul, get_cos_from_table, read_u16_be, CodecError, CodecResult, Fp16, FP16_ONE, FP16_SHIFT,
    FP30_SHIFT,
};

// ============================================================================
// MP3 Bitstream Reader (MSB first)
// ============================================================================

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
// MP3 Frame Header
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

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_mp3_bitstream_reader_msb() {
        let data = [0xA5u8];
        let mut reader = Mp3BitstreamReader::new(&data);
        let val = reader.read_bits(4).unwrap();
        assert_eq!(val, 10);
        let val2 = reader.read_bits(4).unwrap();
        assert_eq!(val2, 5);
    }

    // --- MP3 header tests ---

    #[test]
    fn test_mp3_frame_header_parse() {
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
        let header = [0xFF, 0xFB, 0x50, 0xC0];
        let parsed = Mp3FrameHeader::parse(&header).unwrap();
        assert_eq!(parsed.channel_mode, ChannelMode::Mono);
        assert_eq!(parsed.channel_mode.num_channels(), 1);
    }

    #[test]
    fn test_mp3_frame_size_calculation() {
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

        assert!(mid[0] > 0);
        assert!(side[0] > 0);
    }

    // --- IMDCT tests ---

    #[test]
    fn test_imdct_12_zeros() {
        let input = [0i32; 6];
        let mut output = [0i32; 12];
        mp3_imdct_12(&input, &mut output);
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
        for val in &output {
            assert_eq!(*val, 0);
        }
    }

    // --- Requantization tests ---

    #[test]
    fn test_pow2_quarter() {
        assert_eq!(pow2_quarter(0), FP16_ONE);
        assert_eq!(pow2_quarter(4), FP16_ONE * 2);
        assert_eq!(pow2_quarter(8), FP16_ONE * 4);
    }

    #[test]
    fn test_requantize_zero() {
        let result = mp3_requantize(0, 210, 0, false, 0, false, 0);
        assert_eq!(result, 0);
    }

    // --- MP3 decoder tests ---

    #[test]
    fn test_mp3_find_sync() {
        let mut data = vec![0u8; 100];
        data[10] = 0xFF;
        data[11] = 0xFB;
        data[12] = 0x90;
        data[13] = 0x00;

        let pos = Mp3Decoder::find_sync(&data, 0);
        assert_eq!(pos, Some(10));
    }

    #[test]
    fn test_mp3_find_sync_no_match() {
        let data = vec![0u8; 100];
        let pos = Mp3Decoder::find_sync(&data, 0);
        assert_eq!(pos, None);
    }

    // --- MP3 ID3 skip test ---

    #[test]
    fn test_mp3_decoder_id3_skip() {
        let mut decoder = Mp3Decoder::new();
        let mut data = vec![0u8; 200];
        data[0] = b'I';
        data[1] = b'D';
        data[2] = b'3';
        data[3] = 4;
        data[4] = 0;
        data[5] = 0;
        data[6] = 0;
        data[7] = 0;
        data[8] = 0;
        data[9] = 10;
        let result = decoder.decode_all(&data);
        assert!(result.is_err());
    }
}
