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

pub mod mp3;
pub mod vorbis;

// ============================================================================
// Fixed-Point Arithmetic (shared)
// ============================================================================

/// 16.16 fixed-point type
pub(crate) type Fp16 = i32;

/// 2.30 fixed-point type for high-precision trig
pub(crate) type Fp30 = i32;

/// 16.16: number of fractional bits
pub(crate) const FP16_SHIFT: i32 = 16;

/// 16.16: representation of 1.0
pub(crate) const FP16_ONE: Fp16 = 1 << FP16_SHIFT;

/// 2.30: number of fractional bits
pub(crate) const FP30_SHIFT: i32 = 30;

/// 2.30: representation of 1.0
pub(crate) const FP30_ONE: Fp30 = 1 << FP30_SHIFT;

/// Multiply two 2.30 fixed-point values, returning 2.30 result
#[inline]
pub(crate) fn fp30_mul(a: Fp30, b: Fp30) -> Fp30 {
    let result = (a as i64).checked_mul(b as i64).unwrap_or(0) >> FP30_SHIFT;
    result as Fp30
}

/// Multiply two 16.16 fixed-point values with saturation
#[inline]
pub(crate) fn fp16_mul(a: Fp16, b: Fp16) -> Fp16 {
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
pub(crate) fn fp30_to_fp16(v: Fp30) -> Fp16 {
    v >> (FP30_SHIFT - FP16_SHIFT)
}

/// Convert i32 integer to 16.16 fixed-point
#[inline]
pub(crate) fn fp16_from_i32(v: i32) -> Fp16 {
    v.checked_shl(FP16_SHIFT as u32)
        .unwrap_or(if v >= 0 { i32::MAX } else { i32::MIN })
}

/// Clamp a 16.16 fixed-point value to i16 range and return the sample
#[inline]
pub(crate) fn fp16_to_i16(fp: Fp16) -> i16 {
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
pub(crate) fn read_u8(data: &[u8], pos: usize) -> Option<u8> {
    data.get(pos).copied()
}

#[inline]
pub(crate) fn read_u16_le(data: &[u8], pos: usize) -> Option<u16> {
    if pos + 2 > data.len() {
        return None;
    }
    Some(u16::from_le_bytes([data[pos], data[pos + 1]]))
}

#[inline]
pub(crate) fn read_u32_le(data: &[u8], pos: usize) -> Option<u32> {
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
pub(crate) fn read_u64_le(data: &[u8], pos: usize) -> Option<u64> {
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
pub(crate) fn read_u16_be(data: &[u8], pos: usize) -> Option<u16> {
    if pos + 2 > data.len() {
        return None;
    }
    Some(u16::from_be_bytes([data[pos], data[pos + 1]]))
}

#[inline]
pub(crate) fn read_u32_be(data: &[u8], pos: usize) -> Option<u32> {
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
// Shared Lookup Tables and Helpers
// ============================================================================

/// Pre-computed cosine table for common MDCT sizes (64 entries, covering
/// 0 to pi/2 in equal steps). Values in 2.30 fixed-point.
///
/// cos_table[i] = cos(i * pi / (2 * 64)) in 2.30
pub(crate) const MDCT_COS_TABLE_64: [Fp30; 64] = {
    let mut table = [0i32; 64];
    table[0] = 0x4000_0000; // 1.0 in 2.30

    let cos_step: i64 = 0x3FFF_B10B;

    table[1] = cos_step as i32;

    let mut i = 2usize;
    while i < 64 {
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

/// Get cosine value from lookup table with quadrant folding
///
/// table_idx is in units of pi/128 (0..512 covers 0..4*pi)
pub(crate) fn get_cos_from_table(table_idx: usize) -> Fp30 {
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

/// Integer cosine approximation in 2.30 fixed-point
///
/// Uses a polynomial approximation: cos(x) ~ 1 - x^2/2 + x^4/24
/// where x is in 2.30 fixed-point representing radians / pi.
///
/// Input `angle_frac` is in units of 1/(4*N) turns, pre-scaled to 2.30.
pub(crate) fn integer_cos_fp30(angle_q30: Fp30) -> Fp30 {
    let x = angle_q30 as i64;
    let x2 = (x.checked_mul(x).unwrap_or(0)) >> FP30_SHIFT;
    let x4 = (x2.checked_mul(x2).unwrap_or(0)) >> FP30_SHIFT;

    let one = FP30_ONE as i64;
    let term2 = x2 >> 1;
    let inv_24 = (FP30_ONE as i64) / 24;
    let term4 = (x4.checked_mul(inv_24).unwrap_or(0)) >> FP30_SHIFT;

    let result = one - term2 + term4;

    if result > FP30_ONE as i64 {
        FP30_ONE
    } else if result < -(FP30_ONE as i64) {
        -FP30_ONE
    } else {
        result as Fp30
    }
}

/// Integer log2 (number of bits needed to represent value)
pub(crate) fn ilog(val: u32) -> u8 {
    if val == 0 {
        return 0;
    }
    32 - val.leading_zeros() as u8
}

// Re-export all public types from submodules
pub use mp3::*;
pub use vorbis::*;

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- Fixed-point arithmetic tests ---

    #[test]
    fn test_fp30_mul_identity() {
        let result = fp30_mul(FP30_ONE, FP30_ONE);
        assert_eq!(result, FP30_ONE);
    }

    #[test]
    fn test_fp30_mul_half() {
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

    // --- Codec error display ---

    #[test]
    fn test_codec_error_display() {
        let err = CodecError::BufferTooShort;
        let msg = alloc::format!("{}", err);
        assert_eq!(msg, "buffer too short");
    }

    // --- Cosine table tests ---

    #[test]
    fn test_get_cos_from_table_symmetry() {
        let cos_0 = get_cos_from_table(0);
        assert!(cos_0 > 0);

        let cos_90 = get_cos_from_table(64);
        assert!(cos_90 <= 0);

        let cos_180 = get_cos_from_table(128);
        assert!(cos_180 < 0);
    }
}
