//! Image format decoders: PNG, JPEG (baseline DCT), GIF (87a/89a)
//!
//! All decoders are `no_std`-compatible, use integer/fixed-point math only
//! (no floating point), and produce pixel buffers suitable for the desktop
//! compositing pipeline.
//!
//! # Supported Formats
//!
//! - **PNG**: Full critical chunk support (IHDR, PLTE, IDAT, IEND), ancillary
//!   chunks (tRNS, gAMA), DEFLATE decompression, all 5 filter types, Adam7
//!   interlacing, color types 0/2/3/4/6, bit depths 1/2/4/8/16
//! - **JPEG**: Baseline DCT (SOF0), Huffman entropy coding, integer IDCT,
//!   YCbCr-to-RGB via fixed-point, chroma subsampling 4:4:4/4:2:2/4:2:0,
//!   restart intervals
//! - **GIF**: 87a/89a, LZW decompression, global/local color tables, animation
//!   frames, disposal methods, transparency, interlaced rendering

#![allow(dead_code)]

pub mod gif;
pub mod jpeg;
pub mod png;

use alloc::vec::Vec;

// Re-export all public items from submodules
pub use gif::{decode_gif, DecodedGif, GifDisposal, GifFrame};
pub use jpeg::decode_jpeg;
pub use png::decode_png;

// ============================================================================
// Common types and helpers
// ============================================================================

/// Errors produced by the image decoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCodecError {
    /// Data is too short or truncated.
    TruncatedData,
    /// Magic bytes / signature mismatch.
    InvalidSignature,
    /// Unsupported feature or variant.
    Unsupported,
    /// Corrupted or invalid data encountered during decoding.
    CorruptData,
    /// Checksum (CRC, Adler-32, etc.) verification failed.
    ChecksumMismatch,
    /// Image dimensions are zero or exceed limits.
    InvalidDimensions,
    /// Decompression failed (DEFLATE, LZW).
    DecompressionError,
    /// Huffman table construction failed.
    InvalidHuffmanTable,
    /// Quantization table error.
    InvalidQuantTable,
}

/// A decoded image frame with RGBA8888 pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8888 pixel data, row-major, 4 bytes per pixel.
    pub pixels: Vec<u8>,
}

impl DecodedImage {
    /// Create a new image filled with transparent black.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .unwrap_or(0);
        Self {
            width,
            height,
            pixels: alloc::vec![0u8; size],
        }
    }

    /// Set a pixel at (x, y). Out-of-bounds writes are silently ignored.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        if x < self.width && y < self.height {
            let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
            if off + 3 < self.pixels.len() {
                self.pixels[off] = r;
                self.pixels[off + 1] = g;
                self.pixels[off + 2] = b;
                self.pixels[off + 3] = a;
            }
        }
    }

    /// Get a pixel at (x, y) as (R, G, B, A).
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        if x < self.width && y < self.height {
            let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
            if off + 3 < self.pixels.len() {
                return (
                    self.pixels[off],
                    self.pixels[off + 1],
                    self.pixels[off + 2],
                    self.pixels[off + 3],
                );
            }
        }
        (0, 0, 0, 0)
    }
}

// ---------------------------------------------------------------------------
// Byte-reading helpers
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn read_be_u16(data: &[u8], off: usize) -> u16 {
    ((data[off] as u16) << 8) | (data[off + 1] as u16)
}

#[inline]
pub(crate) fn read_be_u32(data: &[u8], off: usize) -> u32 {
    ((data[off] as u32) << 24)
        | ((data[off + 1] as u32) << 16)
        | ((data[off + 2] as u32) << 8)
        | (data[off + 3] as u32)
}

#[inline]
pub(crate) fn read_le_u16(data: &[u8], off: usize) -> u16 {
    (data[off] as u16) | ((data[off + 1] as u16) << 8)
}

#[inline]
pub(crate) fn read_le_u32(data: &[u8], off: usize) -> u32 {
    (data[off] as u32)
        | ((data[off + 1] as u32) << 8)
        | ((data[off + 2] as u32) << 16)
        | ((data[off + 3] as u32) << 24)
}

/// Clamp an i32 to the u8 range [0, 255].
#[inline]
pub(crate) fn clamp_u8(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

// ============================================================================
// Format detection helper
// ============================================================================

/// Detected image codec format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCodecFormat {
    Png,
    Jpeg,
    Gif,
    Unknown,
}

/// Detect whether data is PNG, JPEG, or GIF.
pub fn detect_codec_format(data: &[u8]) -> ImageCodecFormat {
    if data.len() >= 8 && data[..8] == png::PNG_SIGNATURE {
        ImageCodecFormat::Png
    } else if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        ImageCodecFormat::Jpeg
    } else if data.len() >= 6 && (&data[..6] == gif::GIF87A || &data[..6] == gif::GIF89A) {
        ImageCodecFormat::Gif
    } else {
        ImageCodecFormat::Unknown
    }
}

/// Auto-detect format and decode.
pub fn decode_image(data: &[u8]) -> Result<DecodedImage, ImageCodecError> {
    match detect_codec_format(data) {
        ImageCodecFormat::Png => decode_png(data),
        ImageCodecFormat::Jpeg => decode_jpeg(data),
        ImageCodecFormat::Gif => {
            let gif = decode_gif(data)?;
            if let Some(frame) = gif.frames.into_iter().next() {
                Ok(frame.image)
            } else {
                Err(ImageCodecError::CorruptData)
            }
        }
        ImageCodecFormat::Unknown => Err(ImageCodecError::Unsupported),
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -----------------------------------------------------------------------
    // Common / format detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_png() {
        let data = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0];
        assert_eq!(detect_codec_format(&data), ImageCodecFormat::Png);
    }

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_codec_format(&data), ImageCodecFormat::Jpeg);
    }

    #[test]
    fn test_detect_gif87a() {
        let mut data = vec![0u8; 20];
        data[..6].copy_from_slice(b"GIF87a");
        assert_eq!(detect_codec_format(&data), ImageCodecFormat::Gif);
    }

    #[test]
    fn test_detect_gif89a() {
        let mut data = vec![0u8; 20];
        data[..6].copy_from_slice(b"GIF89a");
        assert_eq!(detect_codec_format(&data), ImageCodecFormat::Gif);
    }

    #[test]
    fn test_detect_unknown() {
        let data = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_codec_format(&data), ImageCodecFormat::Unknown);
    }

    #[test]
    fn test_decoded_image_set_get_pixel() {
        let mut img = DecodedImage::new(4, 4);
        img.set_pixel(1, 2, 0xAA, 0xBB, 0xCC, 0xDD);
        assert_eq!(img.get_pixel(1, 2), (0xAA, 0xBB, 0xCC, 0xDD));
    }

    #[test]
    fn test_decoded_image_out_of_bounds() {
        let mut img = DecodedImage::new(2, 2);
        img.set_pixel(10, 10, 255, 0, 0, 255); // should not panic
        assert_eq!(img.get_pixel(10, 10), (0, 0, 0, 0));
    }

    #[test]
    fn test_clamp_u8_values() {
        assert_eq!(clamp_u8(-10), 0);
        assert_eq!(clamp_u8(0), 0);
        assert_eq!(clamp_u8(128), 128);
        assert_eq!(clamp_u8(255), 255);
        assert_eq!(clamp_u8(300), 255);
    }

    #[test]
    fn test_image_codec_error_equality() {
        assert_eq!(
            ImageCodecError::TruncatedData,
            ImageCodecError::TruncatedData
        );
        assert_ne!(ImageCodecError::TruncatedData, ImageCodecError::CorruptData);
    }

    #[test]
    fn test_decoded_image_new_size() {
        let img = DecodedImage::new(10, 20);
        assert_eq!(img.width, 10);
        assert_eq!(img.height, 20);
        assert_eq!(img.pixels.len(), 10 * 20 * 4);
    }

    #[test]
    fn test_read_helpers() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_be_u16(&data, 0), 0x0102);
        assert_eq!(read_be_u32(&data, 0), 0x01020304);
        assert_eq!(read_le_u16(&data, 0), 0x0201);
        assert_eq!(read_le_u32(&data, 0), 0x04030201);
    }
}
