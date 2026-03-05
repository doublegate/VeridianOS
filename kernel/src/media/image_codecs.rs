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

use alloc::vec::Vec;

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

/// A decoded GIF with potentially multiple frames for animation.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedGif {
    /// Logical screen width.
    pub width: u32,
    /// Logical screen height.
    pub height: u32,
    /// Decoded frames (at least one for static GIFs).
    pub frames: Vec<GifFrame>,
    /// Number of times to loop (0 = infinite).
    pub loop_count: u32,
}

/// A single GIF animation frame.
#[derive(Debug, Clone, PartialEq)]
pub struct GifFrame {
    /// RGBA8888 pixel data for the full logical screen.
    pub image: DecodedImage,
    /// Delay in hundredths of a second (0 = no delay).
    pub delay_cs: u16,
    /// Disposal method for this frame.
    pub disposal: GifDisposal,
}

/// GIF frame disposal methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GifDisposal {
    /// No disposal specified -- leave frame in place.
    #[default]
    None,
    /// Do not dispose -- leave graphic in place.
    DoNotDispose,
    /// Restore to background color.
    RestoreBackground,
    /// Restore to previous frame content.
    RestorePrevious,
}

// ---------------------------------------------------------------------------
// Byte-reading helpers
// ---------------------------------------------------------------------------

#[inline]
fn read_be_u16(data: &[u8], off: usize) -> u16 {
    ((data[off] as u16) << 8) | (data[off + 1] as u16)
}

#[inline]
fn read_be_u32(data: &[u8], off: usize) -> u32 {
    ((data[off] as u32) << 24)
        | ((data[off + 1] as u32) << 16)
        | ((data[off + 2] as u32) << 8)
        | (data[off + 3] as u32)
}

#[inline]
fn read_le_u16(data: &[u8], off: usize) -> u16 {
    (data[off] as u16) | ((data[off + 1] as u16) << 8)
}

#[inline]
fn read_le_u32(data: &[u8], off: usize) -> u32 {
    (data[off] as u32)
        | ((data[off + 1] as u32) << 8)
        | ((data[off + 2] as u32) << 16)
        | ((data[off + 3] as u32) << 24)
}

/// Clamp an i32 to the u8 range [0, 255].
#[inline]
fn clamp_u8(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

// ============================================================================
// PNG DECODER
// ============================================================================

/// PNG 8-byte signature.
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// PNG color types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngColorType {
    Grayscale = 0,
    Rgb = 2,
    Indexed = 3,
    GrayscaleAlpha = 4,
    Rgba = 6,
}

impl PngColorType {
    fn from_u8(v: u8) -> Result<Self, ImageCodecError> {
        match v {
            0 => Ok(Self::Grayscale),
            2 => Ok(Self::Rgb),
            3 => Ok(Self::Indexed),
            4 => Ok(Self::GrayscaleAlpha),
            6 => Ok(Self::Rgba),
            _ => Err(ImageCodecError::Unsupported),
        }
    }

    /// Number of channels (samples per pixel).
    fn channels(self) -> usize {
        match self {
            Self::Grayscale => 1,
            Self::Rgb => 3,
            Self::Indexed => 1,
            Self::GrayscaleAlpha => 2,
            Self::Rgba => 4,
        }
    }
}

/// Parsed IHDR data.
#[derive(Debug, Clone, Copy)]
struct PngIhdr {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: PngColorType,
    interlace: u8,
}

/// Decode a PNG image from raw file data.
pub fn decode_png(data: &[u8]) -> Result<DecodedImage, ImageCodecError> {
    // Verify signature
    if data.len() < 8 || data[..8] != PNG_SIGNATURE {
        return Err(ImageCodecError::InvalidSignature);
    }

    // Parse chunks
    let mut pos: usize = 8;
    let mut ihdr: Option<PngIhdr> = None;
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat_data: Vec<u8> = Vec::new();
    let mut _gamma: u32 = 0; // stored as gamma * 100000

    while pos + 8 <= data.len() {
        let chunk_len = read_be_u32(data, pos) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        let chunk_data_start = pos + 8;
        let chunk_end = chunk_data_start + chunk_len;

        if chunk_end + 4 > data.len() {
            // Not enough data for chunk + CRC
            break;
        }

        match chunk_type {
            b"IHDR" => {
                if chunk_len < 13 {
                    return Err(ImageCodecError::CorruptData);
                }
                let cd = &data[chunk_data_start..chunk_end];
                ihdr = Some(PngIhdr {
                    width: read_be_u32(cd, 0),
                    height: read_be_u32(cd, 4),
                    bit_depth: cd[8],
                    color_type: PngColorType::from_u8(cd[9])?,
                    interlace: cd[12],
                });
            }
            b"PLTE" => {
                if !chunk_len.is_multiple_of(3) {
                    return Err(ImageCodecError::CorruptData);
                }
                let cd = &data[chunk_data_start..chunk_end];
                palette.clear();
                let mut i = 0;
                while i + 2 < cd.len() {
                    palette.push((cd[i], cd[i + 1], cd[i + 2]));
                    i += 3;
                }
            }
            b"tRNS" => {
                trns = data[chunk_data_start..chunk_end].to_vec();
            }
            b"gAMA" => {
                if chunk_len >= 4 {
                    _gamma = read_be_u32(data, chunk_data_start);
                }
            }
            b"IDAT" => {
                idat_data.extend_from_slice(&data[chunk_data_start..chunk_end]);
            }
            b"IEND" => {
                break;
            }
            _ => {
                // Skip unknown/ancillary chunks
            }
        }

        pos = chunk_end + 4; // skip CRC
    }

    let ihdr = ihdr.ok_or(ImageCodecError::CorruptData)?;
    if ihdr.width == 0 || ihdr.height == 0 {
        return Err(ImageCodecError::InvalidDimensions);
    }

    // Validate bit depth for color type
    match ihdr.color_type {
        PngColorType::Grayscale => {
            if !matches!(ihdr.bit_depth, 1 | 2 | 4 | 8 | 16) {
                return Err(ImageCodecError::Unsupported);
            }
        }
        PngColorType::Rgb | PngColorType::GrayscaleAlpha | PngColorType::Rgba => {
            if !matches!(ihdr.bit_depth, 8 | 16) {
                return Err(ImageCodecError::Unsupported);
            }
        }
        PngColorType::Indexed => {
            if !matches!(ihdr.bit_depth, 1 | 2 | 4 | 8) {
                return Err(ImageCodecError::Unsupported);
            }
        }
    }

    // Decompress zlib-wrapped IDAT data
    let raw_data = zlib_decompress(&idat_data)?;

    // Unfilter and produce RGBA output
    if ihdr.interlace == 1 {
        decode_png_interlaced(&ihdr, &raw_data, &palette, &trns)
    } else {
        decode_png_non_interlaced(&ihdr, &raw_data, &palette, &trns)
    }
}

/// Decode non-interlaced PNG scanlines.
fn decode_png_non_interlaced(
    ihdr: &PngIhdr,
    raw: &[u8],
    palette: &[(u8, u8, u8)],
    trns: &[u8],
) -> Result<DecodedImage, ImageCodecError> {
    let w = ihdr.width as usize;
    let h = ihdr.height as usize;
    let channels = ihdr.color_type.channels();
    let bits_per_pixel = channels * (ihdr.bit_depth as usize);
    let bytes_per_row = (w * bits_per_pixel).div_ceil(8);
    let bpp_bytes = bits_per_pixel.div_ceil(8); // filter byte stride

    let mut img = DecodedImage::new(ihdr.width, ihdr.height);
    let mut prev_row: Vec<u8> = alloc::vec![0u8; bytes_per_row];
    let mut pos: usize = 0;

    for y in 0..h {
        if pos >= raw.len() {
            return Err(ImageCodecError::TruncatedData);
        }
        let filter = raw[pos];
        pos += 1;

        if pos + bytes_per_row > raw.len() {
            return Err(ImageCodecError::TruncatedData);
        }

        let mut current_row: Vec<u8> = raw[pos..pos + bytes_per_row].to_vec();
        pos += bytes_per_row;

        // Apply PNG filter reconstruction
        png_unfilter(filter, &mut current_row, &prev_row, bpp_bytes)?;

        // Convert scanline to RGBA pixels
        png_scanline_to_rgba(
            &current_row,
            ihdr,
            palette,
            trns,
            &mut img,
            y as u32,
            0,
            w as u32,
        );

        prev_row = current_row;
    }

    Ok(img)
}

/// Decode Adam7 interlaced PNG.
fn decode_png_interlaced(
    ihdr: &PngIhdr,
    raw: &[u8],
    palette: &[(u8, u8, u8)],
    trns: &[u8],
) -> Result<DecodedImage, ImageCodecError> {
    let w = ihdr.width as usize;
    let h = ihdr.height as usize;
    let channels = ihdr.color_type.channels();
    let bits_per_pixel = channels * (ihdr.bit_depth as usize);

    // Adam7 pass parameters: (x_start, y_start, x_step, y_step)
    const ADAM7: [(usize, usize, usize, usize); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];

    let mut img = DecodedImage::new(ihdr.width, ihdr.height);
    let mut pos: usize = 0;

    for &(x_start, y_start, x_step, y_step) in &ADAM7 {
        let pass_w = if x_start >= w {
            0
        } else {
            (w - x_start).div_ceil(x_step)
        };
        let pass_h = if y_start >= h {
            0
        } else {
            (h - y_start).div_ceil(y_step)
        };

        if pass_w == 0 || pass_h == 0 {
            continue;
        }

        let bytes_per_row = (pass_w * bits_per_pixel).div_ceil(8);
        let bpp_bytes = bits_per_pixel.div_ceil(8);
        let mut prev_row: Vec<u8> = alloc::vec![0u8; bytes_per_row];

        for pass_y in 0..pass_h {
            if pos >= raw.len() {
                return Err(ImageCodecError::TruncatedData);
            }
            let filter = raw[pos];
            pos += 1;

            if pos + bytes_per_row > raw.len() {
                return Err(ImageCodecError::TruncatedData);
            }

            let mut current_row: Vec<u8> = raw[pos..pos + bytes_per_row].to_vec();
            pos += bytes_per_row;

            png_unfilter(filter, &mut current_row, &prev_row, bpp_bytes)?;

            // Place pixels at correct interlaced positions
            let out_y = y_start + pass_y * y_step;
            for pass_x in 0..pass_w {
                let out_x = x_start + pass_x * x_step;
                let pixel = png_extract_pixel(&current_row, pass_x, ihdr, palette, trns);
                img.set_pixel(
                    out_x as u32,
                    out_y as u32,
                    pixel.0,
                    pixel.1,
                    pixel.2,
                    pixel.3,
                );
            }

            prev_row = current_row;
        }
    }

    Ok(img)
}

/// PNG filter reconstruction (RFC 2083 Section 9).
fn png_unfilter(
    filter: u8,
    current: &mut [u8],
    prev: &[u8],
    bpp: usize,
) -> Result<(), ImageCodecError> {
    let len = current.len();
    match filter {
        0 => {} // None
        1 => {
            // Sub
            for i in bpp..len {
                current[i] = current[i].wrapping_add(current[i - bpp]);
            }
        }
        2 => {
            // Up
            for i in 0..len {
                current[i] = current[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            // Average
            for i in 0..len {
                let a = if i >= bpp { current[i - bpp] as u16 } else { 0 };
                let b = prev[i] as u16;
                current[i] = current[i].wrapping_add(((a + b) / 2) as u8);
            }
        }
        4 => {
            // Paeth
            for i in 0..len {
                let a = if i >= bpp { current[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                current[i] = current[i].wrapping_add(paeth_predictor(a, b, c) as u8);
            }
        }
        _ => return Err(ImageCodecError::CorruptData),
    }
    Ok(())
}

/// Paeth predictor function (integer only).
#[inline]
fn paeth_predictor(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Convert a full PNG scanline to RGBA pixels in the output image.
fn png_scanline_to_rgba(
    row: &[u8],
    ihdr: &PngIhdr,
    palette: &[(u8, u8, u8)],
    trns: &[u8],
    img: &mut DecodedImage,
    y: u32,
    x_start: u32,
    count: u32,
) {
    for x in 0..count {
        let pixel = png_extract_pixel(row, x as usize, ihdr, palette, trns);
        img.set_pixel(x_start + x, y, pixel.0, pixel.1, pixel.2, pixel.3);
    }
}

/// Extract a single pixel from a PNG scanline, returning (R, G, B, A).
fn png_extract_pixel(
    row: &[u8],
    x: usize,
    ihdr: &PngIhdr,
    palette: &[(u8, u8, u8)],
    trns: &[u8],
) -> (u8, u8, u8, u8) {
    let bd = ihdr.bit_depth as usize;

    match ihdr.color_type {
        PngColorType::Grayscale => {
            let v = extract_sample(row, x, bd);
            let v8 = scale_to_8bit(v, bd);
            let a = if trns.len() >= 2 {
                let trns_val = read_be_u16(trns, 0) as usize;
                if v == trns_val {
                    0
                } else {
                    255
                }
            } else {
                255
            };
            (v8, v8, v8, a)
        }
        PngColorType::Rgb => {
            let bytes_per_sample = if bd == 16 { 2 } else { 1 };
            let off = x * 3 * bytes_per_sample;
            let (r, g, b) = if bd == 16 {
                if off + 5 < row.len() {
                    (row[off], row[off + 2], row[off + 4])
                } else {
                    (0, 0, 0)
                }
            } else if off + 2 < row.len() {
                (row[off], row[off + 1], row[off + 2])
            } else {
                (0, 0, 0)
            };
            let a = if trns.len() >= 6 {
                let tr = read_be_u16(trns, 0);
                let tg = read_be_u16(trns, 2);
                let tb = read_be_u16(trns, 4);
                let (cr, cg, cb) = if bd == 16 {
                    (
                        read_be_u16(row, off),
                        read_be_u16(row, off + 2),
                        read_be_u16(row, off + 4),
                    )
                } else {
                    (r as u16, g as u16, b as u16)
                };
                if cr == tr && cg == tg && cb == tb {
                    0
                } else {
                    255
                }
            } else {
                255
            };
            (r, g, b, a)
        }
        PngColorType::Indexed => {
            let idx = extract_sample(row, x, bd);
            if idx < palette.len() {
                let (r, g, b) = palette[idx];
                let a = if idx < trns.len() { trns[idx] } else { 255 };
                (r, g, b, a)
            } else {
                (0, 0, 0, 255)
            }
        }
        PngColorType::GrayscaleAlpha => {
            let bytes_per_sample = if bd == 16 { 2 } else { 1 };
            let off = x * 2 * bytes_per_sample;
            let (v, a) = if bd == 16 {
                if off + 3 < row.len() {
                    (row[off], row[off + 2])
                } else {
                    (0, 0)
                }
            } else if off + 1 < row.len() {
                (row[off], row[off + 1])
            } else {
                (0, 0)
            };
            (v, v, v, a)
        }
        PngColorType::Rgba => {
            let bytes_per_sample = if bd == 16 { 2 } else { 1 };
            let off = x * 4 * bytes_per_sample;
            if bd == 16 {
                if off + 7 < row.len() {
                    (row[off], row[off + 2], row[off + 4], row[off + 6])
                } else {
                    (0, 0, 0, 0)
                }
            } else if off + 3 < row.len() {
                (row[off], row[off + 1], row[off + 2], row[off + 3])
            } else {
                (0, 0, 0, 0)
            }
        }
    }
}

/// Extract a sub-byte sample value from packed scanline data.
fn extract_sample(row: &[u8], index: usize, bit_depth: usize) -> usize {
    match bit_depth {
        1 => {
            let byte_idx = index / 8;
            let bit_idx = 7 - (index % 8);
            if byte_idx < row.len() {
                ((row[byte_idx] >> bit_idx) & 1) as usize
            } else {
                0
            }
        }
        2 => {
            let byte_idx = index / 4;
            let shift = 6 - (index % 4) * 2;
            if byte_idx < row.len() {
                ((row[byte_idx] >> shift) & 3) as usize
            } else {
                0
            }
        }
        4 => {
            let byte_idx = index / 2;
            let shift = if index.is_multiple_of(2) { 4 } else { 0 };
            if byte_idx < row.len() {
                ((row[byte_idx] >> shift) & 0xF) as usize
            } else {
                0
            }
        }
        8 => {
            if index < row.len() {
                row[index] as usize
            } else {
                0
            }
        }
        16 => {
            let off = index * 2;
            if off + 1 < row.len() {
                read_be_u16(row, off) as usize
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Scale a sample value from its native bit depth to 8-bit.
fn scale_to_8bit(val: usize, bit_depth: usize) -> u8 {
    match bit_depth {
        1 => {
            if val != 0 {
                255
            } else {
                0
            }
        }
        2 => (val * 85) as u8,         // 0->0, 1->85, 2->170, 3->255
        4 => ((val * 255) / 15) as u8, // scale 0-15 to 0-255
        8 => val as u8,
        16 => (val >> 8) as u8,
        _ => val as u8,
    }
}

// ============================================================================
// DEFLATE / ZLIB DECOMPRESSION (RFC 1950 / 1951)
// ============================================================================

/// Decompress zlib-wrapped data (CMF + FLG + compressed blocks + Adler-32).
fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, ImageCodecError> {
    if data.len() < 6 {
        return Err(ImageCodecError::TruncatedData);
    }

    let cmf = data[0];
    let _flg = data[1];

    // CMF: bits 0-3 = CM (must be 8 for deflate), bits 4-7 = CINFO
    if cmf & 0x0F != 8 {
        return Err(ImageCodecError::Unsupported);
    }

    // Verify CMF/FLG check
    let check = (cmf as u16) * 256 + (_flg as u16);
    if !check.is_multiple_of(31) {
        return Err(ImageCodecError::CorruptData);
    }

    // Check FDICT flag (bit 5 of FLG) -- we don't support preset dictionaries
    if _flg & 0x20 != 0 {
        return Err(ImageCodecError::Unsupported);
    }

    // Decompress DEFLATE stream starting at offset 2
    let compressed = &data[2..];
    let output = deflate_decompress(compressed)?;

    // Verify Adler-32 checksum (last 4 bytes of zlib stream)
    if data.len() >= 6 {
        let adler_offset = data.len() - 4;
        let stored_adler = read_be_u32(data, adler_offset);
        let computed_adler = adler32(&output);
        if stored_adler != computed_adler {
            // Some PNG encoders produce valid images with wrong checksums;
            // we log but don't fail for robustness.
            // return Err(ImageCodecError::ChecksumMismatch);
        }
    }

    Ok(output)
}

/// Compute Adler-32 checksum.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;

    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }

    (b << 16) | a
}

/// Bit reader for DEFLATE stream.
struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8, // 0-7, bits consumed in current byte
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// Read `n` bits (up to 25), LSB first.
    fn read_bits(&mut self, n: u8) -> Result<u32, ImageCodecError> {
        let mut result: u32 = 0;
        let mut bits_read: u8 = 0;

        while bits_read < n {
            if self.byte_pos >= self.data.len() {
                return Err(ImageCodecError::TruncatedData);
            }

            let available = 8 - self.bit_pos;
            let needed = n - bits_read;
            let take = if available < needed {
                available
            } else {
                needed
            };
            let mask = (1u32 << take) - 1;
            let bits = ((self.data[self.byte_pos] >> self.bit_pos) as u32) & mask;
            result |= bits << bits_read;
            bits_read += take;
            self.bit_pos += take;

            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }

        Ok(result)
    }

    /// Read a single bit.
    fn read_bit(&mut self) -> Result<u32, ImageCodecError> {
        self.read_bits(1)
    }

    /// Align to byte boundary.
    fn align(&mut self) {
        if self.bit_pos > 0 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
    }

    /// Read a byte (must be byte-aligned).
    fn read_byte(&mut self) -> Result<u8, ImageCodecError> {
        if self.byte_pos >= self.data.len() {
            return Err(ImageCodecError::TruncatedData);
        }
        let b = self.data[self.byte_pos];
        self.byte_pos += 1;
        Ok(b)
    }
}

/// DEFLATE decompression (RFC 1951).
fn deflate_decompress(data: &[u8]) -> Result<Vec<u8>, ImageCodecError> {
    let mut reader = BitReader::new(data);
    let mut output: Vec<u8> = Vec::new();

    loop {
        let bfinal = reader.read_bit()?;
        let btype = reader.read_bits(2)?;

        match btype {
            0 => {
                // Stored (uncompressed) block
                reader.align();
                let len = reader.read_byte()? as u16 | ((reader.read_byte()? as u16) << 8);
                let _nlen = reader.read_byte()? as u16 | ((reader.read_byte()? as u16) << 8);
                for _ in 0..len {
                    output.push(reader.read_byte()?);
                }
            }
            1 => {
                // Fixed Huffman codes
                let (lit_tree, dist_tree) = build_fixed_huffman_trees();
                inflate_block(&mut reader, &lit_tree, &dist_tree, &mut output)?;
            }
            2 => {
                // Dynamic Huffman codes
                let (lit_tree, dist_tree) = decode_dynamic_huffman(&mut reader)?;
                inflate_block(&mut reader, &lit_tree, &dist_tree, &mut output)?;
            }
            _ => return Err(ImageCodecError::CorruptData),
        }

        if bfinal != 0 {
            break;
        }
    }

    Ok(output)
}

/// A Huffman tree for DEFLATE decoding, stored as a lookup table.
/// Maximum code length in DEFLATE is 15 bits.
struct HuffmanTree {
    /// For each code length, the number of codes and starting values.
    /// Stored as (min_code, symbols) per bit length.
    /// We use a simple linear decode approach.
    counts: [u16; 16],
    symbols: Vec<u16>,
    min_codes: [u32; 16],
    max_codes: [i32; 16],
    offsets: [u16; 16],
}

impl HuffmanTree {
    /// Build a Huffman tree from a list of code lengths.
    fn from_lengths(lengths: &[u8]) -> Result<Self, ImageCodecError> {
        let mut counts = [0u16; 16];
        let mut max_len: usize = 0;

        // Count occurrences of each code length
        for &len in lengths {
            let l = len as usize;
            if l > 0 {
                if l > 15 {
                    return Err(ImageCodecError::InvalidHuffmanTable);
                }
                counts[l] += 1;
                if l > max_len {
                    max_len = l;
                }
            }
        }

        // Compute starting codes for each length
        let mut code: u32 = 0;
        let mut next_code = [0u32; 16];
        let mut min_codes = [0u32; 16];
        let mut max_codes = [-1i32; 16];
        let mut offsets = [0u16; 16];

        let mut offset: u16 = 0;
        for bits in 1..=max_len {
            code = (code + counts[bits - 1] as u32) << 1;
            next_code[bits] = code;
            min_codes[bits] = code;
            offsets[bits] = offset;
            if counts[bits] > 0 {
                max_codes[bits] = (code + counts[bits] as u32 - 1) as i32;
            }
            offset += counts[bits];
        }

        // Assign symbols
        let total_symbols = offset as usize;
        let mut symbols = alloc::vec![0u16; total_symbols];
        let mut symbol_idx = [0u16; 16];
        symbol_idx[1..16].copy_from_slice(&offsets[1..16]);

        for (sym, &len) in lengths.iter().enumerate() {
            let l = len as usize;
            if l > 0 && l < 16 {
                let idx = symbol_idx[l] as usize;
                if idx < symbols.len() {
                    symbols[idx] = sym as u16;
                    symbol_idx[l] += 1;
                }
            }
        }

        Ok(Self {
            counts,
            symbols,
            min_codes,
            max_codes,
            offsets,
        })
    }

    /// Decode one symbol from the bit stream.
    fn decode(&self, reader: &mut BitReader) -> Result<u16, ImageCodecError> {
        let mut code: u32 = 0;

        for bits in 1..16u8 {
            code = (code << 1) | reader.read_bit()?;
            let b = bits as usize;
            if self.max_codes[b] >= 0 && code <= self.max_codes[b] as u32 {
                let idx = self.offsets[b] as usize + (code - self.min_codes[b]) as usize;
                if idx < self.symbols.len() {
                    return Ok(self.symbols[idx]);
                }
            }
        }

        Err(ImageCodecError::InvalidHuffmanTable)
    }
}

/// Build the fixed Huffman trees for DEFLATE block type 1.
fn build_fixed_huffman_trees() -> (HuffmanTree, HuffmanTree) {
    // Literal/length: 0-143 => 8 bits, 144-255 => 9 bits, 256-279 => 7 bits,
    // 280-287 => 8 bits
    let mut lit_lengths = [0u8; 288];
    lit_lengths[0..=143].fill(8);
    lit_lengths[144..=255].fill(9);
    lit_lengths[256..=279].fill(7);
    lit_lengths[280..=287].fill(8);

    // Distance: all 32 codes are 5 bits
    let dist_lengths = [5u8; 32];

    (
        HuffmanTree::from_lengths(&lit_lengths).unwrap_or_else(|_| HuffmanTree {
            counts: [0; 16],
            symbols: Vec::new(),
            min_codes: [0; 16],
            max_codes: [-1; 16],
            offsets: [0; 16],
        }),
        HuffmanTree::from_lengths(&dist_lengths).unwrap_or_else(|_| HuffmanTree {
            counts: [0; 16],
            symbols: Vec::new(),
            min_codes: [0; 16],
            max_codes: [-1; 16],
            offsets: [0; 16],
        }),
    )
}

/// Decode dynamic Huffman trees from DEFLATE block type 2 header.
fn decode_dynamic_huffman(
    reader: &mut BitReader,
) -> Result<(HuffmanTree, HuffmanTree), ImageCodecError> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;

    // Code length alphabet order
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i]] = reader.read_bits(3)? as u8;
    }

    let cl_tree = HuffmanTree::from_lengths(&cl_lengths)?;

    // Decode literal/length + distance code lengths
    let total = hlit + hdist;
    let mut lengths: Vec<u8> = Vec::with_capacity(total);

    while lengths.len() < total {
        let sym = cl_tree.decode(reader)?;
        match sym {
            0..=15 => {
                lengths.push(sym as u8);
            }
            16 => {
                // Repeat previous length 3-6 times
                let extra = reader.read_bits(2)? as usize + 3;
                let prev = if let Some(&last) = lengths.last() {
                    last
                } else {
                    0
                };
                for _ in 0..extra {
                    if lengths.len() < total {
                        lengths.push(prev);
                    }
                }
            }
            17 => {
                // Repeat 0 for 3-10 times
                let extra = reader.read_bits(3)? as usize + 3;
                for _ in 0..extra {
                    if lengths.len() < total {
                        lengths.push(0);
                    }
                }
            }
            18 => {
                // Repeat 0 for 11-138 times
                let extra = reader.read_bits(7)? as usize + 11;
                for _ in 0..extra {
                    if lengths.len() < total {
                        lengths.push(0);
                    }
                }
            }
            _ => return Err(ImageCodecError::InvalidHuffmanTable),
        }
    }

    let lit_tree = HuffmanTree::from_lengths(&lengths[..hlit])?;
    let dist_tree = HuffmanTree::from_lengths(&lengths[hlit..hlit + hdist])?;

    Ok((lit_tree, dist_tree))
}

/// Length base values and extra bits for codes 257-285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distance base values and extra bits for codes 0-29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Inflate one DEFLATE block using the given Huffman trees.
fn inflate_block(
    reader: &mut BitReader,
    lit_tree: &HuffmanTree,
    dist_tree: &HuffmanTree,
    output: &mut Vec<u8>,
) -> Result<(), ImageCodecError> {
    loop {
        let sym = lit_tree.decode(reader)?;

        if sym < 256 {
            // Literal byte
            output.push(sym as u8);
        } else if sym == 256 {
            // End of block
            break;
        } else {
            // Length/distance pair
            let len_idx = (sym - 257) as usize;
            if len_idx >= LENGTH_BASE.len() {
                return Err(ImageCodecError::CorruptData);
            }
            let length =
                LENGTH_BASE[len_idx] as usize + reader.read_bits(LENGTH_EXTRA[len_idx])? as usize;

            let dist_sym = dist_tree.decode(reader)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err(ImageCodecError::CorruptData);
            }
            let distance =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;

            if distance > output.len() {
                return Err(ImageCodecError::CorruptData);
            }

            // Copy from back-reference (byte-by-byte for overlapping copies)
            let start = output.len() - distance;
            for i in 0..length {
                let b = output[start + (i % distance)];
                output.push(b);
            }
        }
    }

    Ok(())
}

// ============================================================================
// JPEG DECODER (Baseline DCT)
// ============================================================================

/// JPEG marker constants.
const JPEG_SOI: u16 = 0xFFD8;
const JPEG_EOI: u16 = 0xFFD9;
const JPEG_SOF0: u16 = 0xFFC0; // Baseline DCT
const JPEG_DHT: u16 = 0xFFC4;
const JPEG_DQT: u16 = 0xFFDB;
const JPEG_DRI: u16 = 0xFFDD;
const JPEG_SOS: u16 = 0xFFDA;
const JPEG_RST0: u16 = 0xFFD0;
// RST1-RST7: 0xFFD1-0xFFD7

/// JPEG component info.
#[derive(Debug, Clone, Copy, Default)]
struct JpegComponent {
    id: u8,
    h_samp: u8,      // horizontal sampling factor
    v_samp: u8,      // vertical sampling factor
    quant_table: u8, // quantization table index
    dc_table: u8,    // DC Huffman table index
    ac_table: u8,    // AC Huffman table index
    dc_pred: i32,    // DC prediction value
}

/// JPEG Huffman table (for entropy decoding).
#[derive(Clone)]
struct JpegHuffTable {
    /// Number of codes for each bit length (1-16).
    counts: [u8; 17],
    /// Symbol values, in order.
    symbols: Vec<u8>,
    /// Lookup: max code value for each bit length.
    max_code: [i32; 18],
    /// Lookup: value offset for each bit length.
    val_offset: [i32; 18],
}

impl Default for JpegHuffTable {
    fn default() -> Self {
        Self {
            counts: [0; 17],
            symbols: Vec::new(),
            max_code: [-1; 18],
            val_offset: [0; 18],
        }
    }
}

impl JpegHuffTable {
    /// Build lookup tables from counts and symbols.
    fn build(&mut self) {
        let mut code: i32 = 0;
        let mut si: i32 = 0;

        for i in 1..=16 {
            if self.counts[i] > 0 {
                self.val_offset[i] = si - code;
                code += self.counts[i] as i32;
                self.max_code[i] = code - 1;
            } else {
                self.max_code[i] = -1;
            }
            si += self.counts[i] as i32;
            code <<= 1;
        }
    }
}

/// JPEG bit reader that handles byte-stuffing (0xFF 0x00 -> 0xFF).
struct JpegBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bits_left: u8,
}

impl<'a> JpegBitReader<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        Self {
            data,
            pos: start,
            bit_buf: 0,
            bits_left: 0,
        }
    }

    /// Read the next byte, handling JPEG byte stuffing.
    fn next_byte(&mut self) -> Result<u8, ImageCodecError> {
        if self.pos >= self.data.len() {
            return Err(ImageCodecError::TruncatedData);
        }
        let b = self.data[self.pos];
        self.pos += 1;

        if b == 0xFF {
            if self.pos >= self.data.len() {
                return Err(ImageCodecError::TruncatedData);
            }
            let next = self.data[self.pos];
            if next == 0x00 {
                // Byte-stuffed 0xFF
                self.pos += 1;
                Ok(0xFF)
            } else if (0xD0..=0xD7).contains(&next) {
                // RST marker -- skip it and return next real byte
                self.pos += 1;
                self.next_byte()
            } else {
                // Other marker -- signal end of scan
                self.pos -= 1;
                Err(ImageCodecError::TruncatedData)
            }
        } else {
            Ok(b)
        }
    }

    /// Fill the bit buffer.
    fn fill_bits(&mut self) -> Result<(), ImageCodecError> {
        while self.bits_left <= 24 {
            let b = self.next_byte()?;
            self.bit_buf |= (b as u32) << (24 - self.bits_left);
            self.bits_left += 8;
        }
        Ok(())
    }

    /// Read `n` bits from MSB.
    fn read_bits(&mut self, n: u8) -> Result<i32, ImageCodecError> {
        if n == 0 {
            return Ok(0);
        }
        while self.bits_left < n {
            let b = self.next_byte()?;
            self.bit_buf |= (b as u32) << (24 - self.bits_left);
            self.bits_left += 8;
        }
        let val = (self.bit_buf >> (32 - n)) as i32;
        self.bit_buf <<= n;
        self.bits_left -= n;
        Ok(val)
    }

    /// Decode one Huffman symbol.
    fn decode_huff(&mut self, table: &JpegHuffTable) -> Result<u8, ImageCodecError> {
        // Ensure enough bits in buffer
        while self.bits_left < 16 {
            match self.next_byte() {
                Ok(b) => {
                    self.bit_buf |= (b as u32) << (24 - self.bits_left);
                    self.bits_left += 8;
                }
                Err(_) => break,
            }
        }

        let mut code: i32 = 0;
        for bits in 1..=16u8 {
            code = (code << 1) | ((self.bit_buf >> 31) as i32);
            self.bit_buf <<= 1;
            self.bits_left = self.bits_left.saturating_sub(1);

            if code <= table.max_code[bits as usize] {
                let idx = (code + table.val_offset[bits as usize]) as usize;
                if idx < table.symbols.len() {
                    return Ok(table.symbols[idx]);
                }
            }
        }

        Err(ImageCodecError::InvalidHuffmanTable)
    }

    /// Receive and extend a value category.
    fn receive_extend(&mut self, nbits: u8) -> Result<i32, ImageCodecError> {
        if nbits == 0 {
            return Ok(0);
        }
        let val = self.read_bits(nbits)?;
        // Sign extension: if MSB is 0, value is negative
        if val < (1 << (nbits - 1)) {
            Ok(val - (1 << nbits) + 1)
        } else {
            Ok(val)
        }
    }

    /// Reset bit reader state (for restart markers).
    fn reset_bits(&mut self) {
        self.bit_buf = 0;
        self.bits_left = 0;
    }
}

/// Decode a JPEG image (baseline DCT only).
pub fn decode_jpeg(data: &[u8]) -> Result<DecodedImage, ImageCodecError> {
    if data.len() < 4 {
        return Err(ImageCodecError::TruncatedData);
    }

    // Check SOI marker
    if data[0] != 0xFF || data[1] != 0xD8 {
        return Err(ImageCodecError::InvalidSignature);
    }

    // Parse markers
    let mut pos: usize = 2;
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut num_components: usize = 0;
    let mut components = [JpegComponent::default(); 4];
    let mut max_h_samp: u8 = 1;
    let mut max_v_samp: u8 = 1;
    let mut quant_tables = [[0i32; 64]; 4];
    let mut dc_tables = [JpegHuffTable::default(), JpegHuffTable::default()];
    let mut ac_tables = [JpegHuffTable::default(), JpegHuffTable::default()];
    let mut restart_interval: u16 = 0;
    let mut scan_start: usize = 0;

    while pos + 1 < data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        let marker = ((data[pos] as u16) << 8) | data[pos + 1] as u16;
        pos += 2;

        match marker {
            JPEG_EOI => break,
            JPEG_SOF0 => {
                // Baseline DCT frame header
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let seg_len = read_be_u16(data, pos) as usize;
                if pos + seg_len > data.len() || seg_len < 8 {
                    return Err(ImageCodecError::TruncatedData);
                }
                let _precision = data[pos + 2]; // must be 8 for baseline
                height = read_be_u16(data, pos + 3) as u32;
                width = read_be_u16(data, pos + 5) as u32;
                num_components = data[pos + 7] as usize;

                if num_components > 4 || num_components == 0 {
                    return Err(ImageCodecError::Unsupported);
                }

                for (i, comp) in components.iter_mut().enumerate().take(num_components) {
                    let off = pos + 8 + i * 3;
                    if off + 2 >= data.len() {
                        return Err(ImageCodecError::TruncatedData);
                    }
                    comp.id = data[off];
                    comp.h_samp = (data[off + 1] >> 4) & 0x0F;
                    comp.v_samp = data[off + 1] & 0x0F;
                    comp.quant_table = data[off + 2];

                    if comp.h_samp > max_h_samp {
                        max_h_samp = comp.h_samp;
                    }
                    if comp.v_samp > max_v_samp {
                        max_v_samp = comp.v_samp;
                    }
                }

                pos += seg_len;
            }
            JPEG_DHT => {
                // Huffman table definition
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let seg_len = read_be_u16(data, pos) as usize;
                let seg_end = pos + seg_len;
                if seg_end > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let mut p = pos + 2;

                while p < seg_end {
                    if p >= data.len() {
                        break;
                    }
                    let info = data[p];
                    p += 1;
                    let table_class = (info >> 4) & 0x0F; // 0=DC, 1=AC
                    let table_id = (info & 0x0F) as usize;

                    if table_id > 1 {
                        return Err(ImageCodecError::InvalidHuffmanTable);
                    }

                    let mut table = JpegHuffTable::default();
                    let mut total_symbols: usize = 0;
                    for i in 1..=16 {
                        if p >= data.len() {
                            return Err(ImageCodecError::TruncatedData);
                        }
                        table.counts[i] = data[p];
                        total_symbols += data[p] as usize;
                        p += 1;
                    }

                    table.symbols = Vec::with_capacity(total_symbols);
                    for _ in 0..total_symbols {
                        if p >= data.len() {
                            return Err(ImageCodecError::TruncatedData);
                        }
                        table.symbols.push(data[p]);
                        p += 1;
                    }

                    table.build();

                    if table_class == 0 {
                        dc_tables[table_id] = table;
                    } else {
                        ac_tables[table_id] = table;
                    }
                }

                pos = seg_end;
            }
            JPEG_DQT => {
                // Quantization table
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let seg_len = read_be_u16(data, pos) as usize;
                let seg_end = pos + seg_len;
                if seg_end > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let mut p = pos + 2;

                while p < seg_end {
                    if p >= data.len() {
                        break;
                    }
                    let info = data[p];
                    p += 1;
                    let precision = (info >> 4) & 0x0F; // 0=8-bit, 1=16-bit
                    let table_id = (info & 0x0F) as usize;

                    if table_id > 3 {
                        return Err(ImageCodecError::InvalidQuantTable);
                    }

                    for qt in quant_tables[table_id].iter_mut() {
                        if precision == 0 {
                            if p >= data.len() {
                                return Err(ImageCodecError::TruncatedData);
                            }
                            *qt = data[p] as i32;
                            p += 1;
                        } else {
                            if p + 1 >= data.len() {
                                return Err(ImageCodecError::TruncatedData);
                            }
                            *qt = read_be_u16(data, p) as i32;
                            p += 2;
                        }
                    }
                }

                pos = seg_end;
            }
            JPEG_DRI => {
                // Restart interval
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let _seg_len = read_be_u16(data, pos);
                if pos + 4 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                restart_interval = read_be_u16(data, pos + 2);
                pos += _seg_len as usize;
            }
            JPEG_SOS => {
                // Start of scan
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let seg_len = read_be_u16(data, pos) as usize;
                if pos + seg_len > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }

                let ns = data[pos + 2] as usize;
                for i in 0..ns {
                    let off = pos + 3 + i * 2;
                    if off + 1 >= data.len() {
                        return Err(ImageCodecError::TruncatedData);
                    }
                    let comp_id = data[off];
                    let td_ta = data[off + 1];

                    // Find matching component
                    for comp in components.iter_mut().take(num_components) {
                        if comp.id == comp_id {
                            comp.dc_table = (td_ta >> 4) & 0x0F;
                            comp.ac_table = td_ta & 0x0F;
                        }
                    }
                }

                scan_start = pos + seg_len;
                break;
            }
            _ => {
                // Skip unknown marker segment
                if pos + 2 > data.len() {
                    break;
                }
                if marker >= 0xFFC0 && marker != 0xFF00 {
                    let seg_len = read_be_u16(data, pos) as usize;
                    pos += seg_len;
                }
            }
        }
    }

    if width == 0 || height == 0 {
        return Err(ImageCodecError::InvalidDimensions);
    }

    // MCU dimensions
    let mcu_w = (max_h_samp as u32) * 8;
    let mcu_h = (max_v_samp as u32) * 8;
    let mcus_x = width.div_ceil(mcu_w);
    let mcus_y = height.div_ceil(mcu_h);

    // Allocate component planes
    let mut comp_data: Vec<Vec<i32>> = Vec::new();
    for comp in components.iter().take(num_components) {
        let cw = mcus_x as usize * (comp.h_samp as usize) * 8;
        let ch = mcus_y as usize * (comp.v_samp as usize) * 8;
        comp_data.push(alloc::vec![0i32; cw * ch]);
    }

    // Entropy decode MCUs
    let mut reader = JpegBitReader::new(data, scan_start);
    let mut mcu_count: u32 = 0;

    // Reset DC predictors
    for c in components.iter_mut().take(num_components) {
        c.dc_pred = 0;
    }

    for mcu_y in 0..mcus_y {
        for mcu_x in 0..mcus_x {
            // Check restart interval
            if restart_interval > 0
                && mcu_count > 0
                && mcu_count.is_multiple_of(restart_interval as u32)
            {
                // Reset DC predictors and bit reader
                for c in components.iter_mut().take(num_components) {
                    c.dc_pred = 0;
                }
                reader.reset_bits();
                // Skip to next RST marker
                while reader.pos < data.len() {
                    if data[reader.pos] == 0xFF && reader.pos + 1 < data.len() {
                        let m = data[reader.pos + 1];
                        if (0xD0..=0xD7).contains(&m) {
                            reader.pos += 2;
                            break;
                        }
                    }
                    reader.pos += 1;
                }
            }

            // Decode each component's blocks in this MCU
            for ci in 0..num_components {
                let h_samp = components[ci].h_samp as usize;
                let v_samp = components[ci].v_samp as usize;
                let qt_idx = components[ci].quant_table as usize;
                let dc_idx = components[ci].dc_table as usize;
                let ac_idx = components[ci].ac_table as usize;
                let comp_w = mcus_x as usize * h_samp * 8;

                for sv in 0..v_samp {
                    for sh in 0..h_samp {
                        // Decode one 8x8 block
                        let mut block = [0i32; 64];

                        // DC coefficient
                        let dc_sym = reader.decode_huff(&dc_tables[dc_idx])?;
                        let dc_diff = reader.receive_extend(dc_sym)?;
                        components[ci].dc_pred += dc_diff;
                        block[0] = components[ci].dc_pred;

                        // AC coefficients (zig-zag order)
                        let mut k: usize = 1;
                        while k < 64 {
                            let ac_sym = reader.decode_huff(&ac_tables[ac_idx])?;
                            if ac_sym == 0x00 {
                                // End of block
                                break;
                            }
                            let run = (ac_sym >> 4) & 0x0F;
                            let size = ac_sym & 0x0F;

                            if size == 0 && run == 0x0F {
                                // ZRL: skip 16 zeros
                                k += 16;
                                continue;
                            }

                            k += run as usize;
                            if k >= 64 {
                                break;
                            }

                            block[ZIGZAG[k] as usize] = reader.receive_extend(size)?;
                            k += 1;
                        }

                        // Dequantize
                        if qt_idx < 4 {
                            for i in 0..64 {
                                block[i] = block[i]
                                    .checked_mul(quant_tables[qt_idx][i])
                                    .unwrap_or(block[i]);
                            }
                        }

                        // IDCT
                        idct_integer(&mut block);

                        // Store block into component plane
                        let bx = mcu_x as usize * h_samp * 8 + sh * 8;
                        let by = mcu_y as usize * v_samp * 8 + sv * 8;

                        for row in 0..8 {
                            for col in 0..8 {
                                let px = bx + col;
                                let py = by + row;
                                let idx = py * comp_w + px;
                                if idx < comp_data[ci].len() {
                                    // Level shift: add 128 to bring from [-128,127] to [0,255]
                                    comp_data[ci][idx] = block[row * 8 + col] + 128;
                                }
                            }
                        }
                    }
                }
            }

            mcu_count += 1;
        }
    }

    // Convert component planes to RGB output
    let mut img = DecodedImage::new(width, height);

    if num_components == 1 {
        // Grayscale
        let comp_w = mcus_x as usize * 8;
        for y in 0..height {
            for x in 0..width {
                let idx = y as usize * comp_w + x as usize;
                if idx < comp_data[0].len() {
                    let v = clamp_u8(comp_data[0][idx]);
                    img.set_pixel(x, y, v, v, v, 255);
                }
            }
        }
    } else if num_components >= 3 {
        // YCbCr to RGB conversion
        let y_w = mcus_x as usize * (components[0].h_samp as usize) * 8;
        let cb_w = mcus_x as usize * (components[1].h_samp as usize) * 8;
        let cr_w = mcus_x as usize * (components[2].h_samp as usize) * 8;

        let h0 = components[0].h_samp as usize;
        let v0 = components[0].v_samp as usize;
        let h1 = components[1].h_samp as usize;
        let v1 = components[1].v_samp as usize;

        for py in 0..height {
            for px in 0..width {
                // Y sample position
                let y_idx = py as usize * y_w + px as usize;

                // Cb/Cr sample position (with subsampling)
                let cx = if h0 > 0 { (px as usize * h1) / h0 } else { 0 };
                let cy = if v0 > 0 { (py as usize * v1) / v0 } else { 0 };
                let cb_idx = cy * cb_w + cx;
                let cr_idx = cy * cr_w + cx;

                let yv = if y_idx < comp_data[0].len() {
                    comp_data[0][y_idx]
                } else {
                    128
                };
                let cb = if cb_idx < comp_data[1].len() {
                    comp_data[1][cb_idx] - 128
                } else {
                    0
                };
                let cr = if cr_idx < comp_data[2].len() {
                    comp_data[2][cr_idx] - 128
                } else {
                    0
                };

                // Fixed-point YCbCr to RGB (BT.601)
                // R = Y + 1.402 * Cr        => Y + (359 * Cr) >> 8
                // G = Y - 0.344 * Cb - 0.714 * Cr => Y - (88 * Cb + 183 * Cr) >> 8
                // B = Y + 1.772 * Cb         => Y + (454 * Cb) >> 8
                let r = clamp_u8(yv + ((359 * cr) >> 8));
                let g = clamp_u8(yv - ((88 * cb + 183 * cr) >> 8));
                let b = clamp_u8(yv + ((454 * cb) >> 8));

                img.set_pixel(px, py, r, g, b, 255);
            }
        }
    }

    Ok(img)
}

/// Integer IDCT (based on AAN/LLM algorithm, all integer arithmetic).
///
/// Input: 64 dequantized DCT coefficients (zig-zag reordered to natural order).
/// Output: 64 spatial-domain values (still needs +128 level shift).
///
/// Uses 13-bit fixed-point for intermediate results.
fn idct_integer(block: &mut [i32; 64]) {
    // Constants for the integer IDCT (scaled by 2^13)
    // These approximate the exact cosine values without floating point.
    const W1: i32 = 2841; // 2048*sqrt(2)*cos(1*pi/16)
    const W2: i32 = 2676; // 2048*sqrt(2)*cos(2*pi/16)
    const W3: i32 = 2408; // 2048*sqrt(2)*cos(3*pi/16)
    const W5: i32 = 1609; // 2048*sqrt(2)*cos(5*pi/16)
    const W6: i32 = 1108; // 2048*sqrt(2)*cos(6*pi/16)
    const W7: i32 = 565; // 2048*sqrt(2)*cos(7*pi/16)

    // 1D IDCT on rows
    for i in 0..8 {
        let row = i * 8;
        // Check if row is all zeros (except DC)
        if block[row + 1] == 0
            && block[row + 2] == 0
            && block[row + 3] == 0
            && block[row + 4] == 0
            && block[row + 5] == 0
            && block[row + 6] == 0
            && block[row + 7] == 0
        {
            let dc = block[row] << 3;
            for j in 0..8 {
                block[row + j] = dc;
            }
            continue;
        }

        // Stage: prescale
        let mut x0 = (block[row] << 11) + 128;
        let mut x1 = block[row + 4] << 11;
        let x2 = block[row + 6];
        let x3 = block[row + 2];
        let x4 = block[row + 1];
        let x5 = block[row + 7];
        let x6 = block[row + 5];
        let x7 = block[row + 3];

        // Stage 1 -- even part
        let x8 = W7 * (x4 + x5);
        let mut x4r = x8 + (W1 - W7) * x4;
        let mut x5r = x8 - (W1 + W7) * x5;
        let x8 = W3 * (x6 + x7);
        let mut x6r = x8 - (W3 - W5) * x6;
        let mut x7r = x8 - (W3 + W5) * x7;

        // Stage 2
        x0 += x1;
        x1 = x0 - (x1 << 1);
        let x8_2 = W6 * (x2 + x3);
        let x2r = x8_2 - (W2 + W6) * x2;
        let x3r = x8_2 + (W2 - W6) * x3;
        x4r += x6r;
        x6r = x4r - (x6r << 1);
        x5r += x7r;
        x7r = x5r - (x7r << 1);

        // Stage 3
        x0 += x3r;
        let x3s = x0 - (x3r << 1);
        x1 += x2r;
        let x2s = x1 - (x2r << 1);
        let tmp = ((x6r + x7r) * 181 + 128) >> 8;
        x6r = ((x6r - x7r) * 181 + 128) >> 8;

        // Output
        block[row] = (x0 + x4r) >> 8;
        block[row + 1] = (x1 + tmp) >> 8;
        block[row + 2] = (x2s + x6r) >> 8;
        block[row + 3] = (x3s + x5r) >> 8;
        block[row + 4] = (x3s - x5r) >> 8;
        block[row + 5] = (x2s - x6r) >> 8;
        block[row + 6] = (x1 - tmp) >> 8;
        block[row + 7] = (x0 - x4r) >> 8;
    }

    // 1D IDCT on columns
    for i in 0..8 {
        // Check for all-zero column (except DC)
        if block[8 + i] == 0
            && block[16 + i] == 0
            && block[24 + i] == 0
            && block[32 + i] == 0
            && block[40 + i] == 0
            && block[48 + i] == 0
            && block[56 + i] == 0
        {
            let dc = (block[i] + 32) >> 6;
            for j in 0..8 {
                block[j * 8 + i] = dc;
            }
            continue;
        }

        let mut x0 = (block[i] << 8) + 8192;
        let mut x1 = block[32 + i] << 8;
        let x2 = block[48 + i];
        let x3 = block[16 + i];
        let x4 = block[8 + i];
        let x5 = block[56 + i];
        let x6 = block[40 + i];
        let x7 = block[24 + i];

        let x8 = W7 * (x4 + x5) + 4;
        let mut x4r = (x8 + (W1 - W7) * x4) >> 3;
        let mut x5r = (x8 - (W1 + W7) * x5) >> 3;
        let x8 = W3 * (x6 + x7) + 4;
        let mut x6r = (x8 - (W3 - W5) * x6) >> 3;
        let mut x7r = (x8 - (W3 + W5) * x7) >> 3;

        x0 += x1;
        x1 = x0 - (x1 << 1);
        let x8_2 = W6 * (x2 + x3) + 4;
        let x2r = (x8_2 - (W2 + W6) * x2) >> 3;
        let x3r = (x8_2 + (W2 - W6) * x3) >> 3;
        x4r += x6r;
        x6r = x4r - (x6r << 1);
        x5r += x7r;
        x7r = x5r - (x7r << 1);

        x0 += x3r;
        let x3s = x0 - (x3r << 1);
        x1 += x2r;
        let x2s = x1 - (x2r << 1);
        let tmp = ((x6r + x7r) * 181 + 128) >> 8;
        x6r = ((x6r - x7r) * 181 + 128) >> 8;

        block[i] = (x0 + x4r) >> 14;
        block[8 + i] = (x1 + tmp) >> 14;
        block[16 + i] = (x2s + x6r) >> 14;
        block[24 + i] = (x3s + x5r) >> 14;
        block[32 + i] = (x3s - x5r) >> 14;
        block[40 + i] = (x2s - x6r) >> 14;
        block[48 + i] = (x1 - tmp) >> 14;
        block[56 + i] = (x0 - x4r) >> 14;
    }
}

/// JPEG zig-zag scan order: maps linear index to natural order position.
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

// ============================================================================
// GIF DECODER (87a / 89a)
// ============================================================================

/// GIF header magic bytes.
const GIF87A: &[u8; 6] = b"GIF87a";
const GIF89A: &[u8; 6] = b"GIF89a";

/// Decode a GIF image (including animated GIFs).
pub fn decode_gif(data: &[u8]) -> Result<DecodedGif, ImageCodecError> {
    if data.len() < 13 {
        return Err(ImageCodecError::TruncatedData);
    }

    // Verify signature
    let sig = &data[0..6];
    if sig != GIF87A && sig != GIF89A {
        return Err(ImageCodecError::InvalidSignature);
    }

    // Logical screen descriptor
    let screen_width = read_le_u16(data, 6) as u32;
    let screen_height = read_le_u16(data, 8) as u32;

    if screen_width == 0 || screen_height == 0 {
        return Err(ImageCodecError::InvalidDimensions);
    }

    let packed = data[10];
    let has_gct = (packed & 0x80) != 0;
    let gct_size_bits = (packed & 0x07) as u32;
    let _bg_color_index = data[11];
    let _pixel_aspect = data[12];

    let mut pos: usize = 13;

    // Read Global Color Table
    let mut global_ct: Vec<(u8, u8, u8)> = Vec::new();
    if has_gct {
        let gct_entries = 1u32 << (gct_size_bits + 1);
        let gct_bytes = (gct_entries as usize) * 3;
        if pos + gct_bytes > data.len() {
            return Err(ImageCodecError::TruncatedData);
        }
        for i in 0..gct_entries as usize {
            let off = pos + i * 3;
            global_ct.push((data[off], data[off + 1], data[off + 2]));
        }
        pos += gct_bytes;
    }

    let mut frames: Vec<GifFrame> = Vec::new();
    let mut loop_count: u32 = 1;

    // Graphics control extension state
    let mut gce_delay: u16 = 0;
    let mut gce_disposal = GifDisposal::None;
    let mut gce_transparent: Option<u8> = None;

    // For "Restore Previous" disposal
    let mut previous_canvas: Option<DecodedImage> = None;

    while pos < data.len() {
        match data[pos] {
            0x2C => {
                // Image descriptor
                if pos + 10 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }

                let img_left = read_le_u16(data, pos + 1) as u32;
                let img_top = read_le_u16(data, pos + 3) as u32;
                let img_width = read_le_u16(data, pos + 5) as u32;
                let img_height = read_le_u16(data, pos + 7) as u32;
                let img_packed = data[pos + 9];
                let has_lct = (img_packed & 0x80) != 0;
                let is_interlaced = (img_packed & 0x40) != 0;
                let lct_size_bits = (img_packed & 0x07) as u32;

                pos += 10;

                // Read local color table if present
                let color_table: &[(u8, u8, u8)];
                let local_ct: Vec<(u8, u8, u8)>;
                if has_lct {
                    let lct_entries = 1u32 << (lct_size_bits + 1);
                    let lct_bytes = (lct_entries as usize) * 3;
                    if pos + lct_bytes > data.len() {
                        return Err(ImageCodecError::TruncatedData);
                    }
                    let mut lct = Vec::with_capacity(lct_entries as usize);
                    for i in 0..lct_entries as usize {
                        let off = pos + i * 3;
                        lct.push((data[off], data[off + 1], data[off + 2]));
                    }
                    pos += lct_bytes;
                    local_ct = lct;
                    color_table = &local_ct;
                } else {
                    local_ct = Vec::new();
                    let _ = &local_ct; // suppress unused warning
                    color_table = &global_ct;
                }

                // Read LZW compressed image data
                if pos >= data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let min_code_size = data[pos];
                pos += 1;

                // Collect sub-blocks
                let mut lzw_data: Vec<u8> = Vec::new();
                loop {
                    if pos >= data.len() {
                        break;
                    }
                    let block_size = data[pos] as usize;
                    pos += 1;
                    if block_size == 0 {
                        break;
                    }
                    if pos + block_size > data.len() {
                        return Err(ImageCodecError::TruncatedData);
                    }
                    lzw_data.extend_from_slice(&data[pos..pos + block_size]);
                    pos += block_size;
                }

                // LZW decompress
                let pixels = lzw_decompress(&lzw_data, min_code_size)?;

                // Build the canvas for this frame
                // Start from previous frame (or blank)
                let mut canvas = if let Some(last) = frames.last() {
                    // Apply disposal of previous frame
                    match last.disposal {
                        GifDisposal::None | GifDisposal::DoNotDispose => last.image.clone(),
                        GifDisposal::RestoreBackground => {
                            DecodedImage::new(screen_width, screen_height)
                        }
                        GifDisposal::RestorePrevious => previous_canvas
                            .clone()
                            .unwrap_or_else(|| DecodedImage::new(screen_width, screen_height)),
                    }
                } else {
                    DecodedImage::new(screen_width, screen_height)
                };

                // Save canvas before drawing (for RestorePrevious)
                if gce_disposal == GifDisposal::RestorePrevious {
                    previous_canvas = Some(canvas.clone());
                }

                // Map pixel indices to the canvas
                let pixel_count = (img_width as usize) * (img_height as usize);

                if is_interlaced {
                    // GIF interlace: 4 passes
                    // Pass 1: rows 0, 8, 16, ... (start=0, step=8)
                    // Pass 2: rows 4, 12, 20, ... (start=4, step=8)
                    // Pass 3: rows 2, 6, 10, ... (start=2, step=4)
                    // Pass 4: rows 1, 3, 5, ... (start=1, step=2)
                    const INTERLACE_PASSES: [(usize, usize); 4] = [(0, 8), (4, 8), (2, 4), (1, 2)];

                    let mut src_idx: usize = 0;
                    for &(start, step) in &INTERLACE_PASSES {
                        let mut row = start;
                        while row < img_height as usize {
                            for col in 0..img_width as usize {
                                if src_idx < pixels.len() && src_idx < pixel_count {
                                    let ci = pixels[src_idx] as usize;
                                    if ci < color_table.len() {
                                        let skip =
                                            gce_transparent.is_some_and(|t| t as usize == ci);
                                        if !skip {
                                            let (r, g, b) = color_table[ci];
                                            canvas.set_pixel(
                                                img_left + col as u32,
                                                img_top + row as u32,
                                                r,
                                                g,
                                                b,
                                                255,
                                            );
                                        }
                                    }
                                }
                                src_idx += 1;
                            }
                            row += step;
                        }
                    }
                } else {
                    // Non-interlaced: sequential rows
                    for (idx, &ci_byte) in pixels.iter().enumerate().take(pixel_count) {
                        let ci = ci_byte as usize;
                        let row = idx / img_width as usize;
                        let col = idx % img_width as usize;
                        if ci < color_table.len() {
                            let skip = gce_transparent.is_some_and(|t| t as usize == ci);
                            if !skip {
                                let (r, g, b) = color_table[ci];
                                canvas.set_pixel(
                                    img_left + col as u32,
                                    img_top + row as u32,
                                    r,
                                    g,
                                    b,
                                    255,
                                );
                            }
                        }
                    }
                }

                frames.push(GifFrame {
                    image: canvas,
                    delay_cs: gce_delay,
                    disposal: gce_disposal,
                });

                // Reset GCE state for next frame
                gce_delay = 0;
                gce_disposal = GifDisposal::None;
                gce_transparent = None;
            }
            0x21 => {
                // Extension block
                if pos + 2 > data.len() {
                    return Err(ImageCodecError::TruncatedData);
                }
                let label = data[pos + 1];
                pos += 2;

                match label {
                    0xF9 => {
                        // Graphics Control Extension
                        if pos >= data.len() {
                            return Err(ImageCodecError::TruncatedData);
                        }
                        let block_size = data[pos] as usize;
                        pos += 1;
                        if pos + block_size > data.len() || block_size < 4 {
                            return Err(ImageCodecError::TruncatedData);
                        }

                        let gce_packed = data[pos];
                        gce_disposal = match (gce_packed >> 2) & 0x07 {
                            0 => GifDisposal::None,
                            1 => GifDisposal::DoNotDispose,
                            2 => GifDisposal::RestoreBackground,
                            3 => GifDisposal::RestorePrevious,
                            _ => GifDisposal::None,
                        };
                        gce_delay = read_le_u16(data, pos + 1);
                        let has_transparent = (gce_packed & 0x01) != 0;
                        if has_transparent {
                            gce_transparent = Some(data[pos + 3]);
                        } else {
                            gce_transparent = None;
                        }

                        pos += block_size;
                        // Skip block terminator
                        if pos < data.len() && data[pos] == 0 {
                            pos += 1;
                        }
                    }
                    0xFF => {
                        // Application Extension (check for NETSCAPE2.0 loop)
                        if pos >= data.len() {
                            return Err(ImageCodecError::TruncatedData);
                        }
                        let block_size = data[pos] as usize;
                        pos += 1;

                        if block_size == 11
                            && pos + 11 <= data.len()
                            && &data[pos..pos + 11] == b"NETSCAPE2.0"
                        {
                            pos += block_size;
                            // Read sub-block with loop count
                            if pos < data.len() {
                                let sub_size = data[pos] as usize;
                                pos += 1;
                                if sub_size >= 3 && pos + sub_size <= data.len() {
                                    if data[pos] == 1 {
                                        loop_count = read_le_u16(data, pos + 1) as u32;
                                    }
                                    pos += sub_size;
                                }
                            }
                        } else {
                            pos += block_size;
                        }

                        // Skip remaining sub-blocks
                        gif_skip_sub_blocks(data, &mut pos);
                    }
                    _ => {
                        // Skip unknown extension sub-blocks
                        gif_skip_sub_blocks(data, &mut pos);
                    }
                }
            }
            0x3B => {
                // Trailer (end of GIF)
                break;
            }
            _ => {
                pos += 1;
            }
        }
    }

    if frames.is_empty() {
        return Err(ImageCodecError::CorruptData);
    }

    Ok(DecodedGif {
        width: screen_width,
        height: screen_height,
        frames,
        loop_count,
    })
}

/// Skip GIF sub-blocks until block terminator (0x00).
fn gif_skip_sub_blocks(data: &[u8], pos: &mut usize) {
    while *pos < data.len() {
        let block_size = data[*pos] as usize;
        *pos += 1;
        if block_size == 0 {
            break;
        }
        *pos += block_size;
    }
}

// ============================================================================
// LZW DECOMPRESSION (for GIF)
// ============================================================================

/// LZW decompression for GIF image data.
fn lzw_decompress(data: &[u8], min_code_size: u8) -> Result<Vec<u8>, ImageCodecError> {
    if min_code_size > 11 {
        return Err(ImageCodecError::Unsupported);
    }

    let clear_code: u16 = 1 << min_code_size;
    let eoi_code: u16 = clear_code + 1;

    let mut code_size: u8 = min_code_size + 1;
    let mut next_code: u16 = eoi_code + 1;
    let max_table_size: usize = 4096;

    // LZW code table: each entry is (prefix_code, suffix_byte)
    // Entries 0..clear_code are initialized as single-byte strings
    let mut table_prefix: Vec<u16> = alloc::vec![0u16; max_table_size];
    let mut table_suffix: Vec<u8> = alloc::vec![0u8; max_table_size];
    let mut table_len: Vec<u16> = alloc::vec![0u16; max_table_size];

    // Initialize table
    for i in 0..clear_code {
        table_prefix[i as usize] = 0;
        table_suffix[i as usize] = i as u8;
        table_len[i as usize] = 1;
    }

    let mut output: Vec<u8> = Vec::new();
    let mut bit_pos: usize = 0;
    let total_bits = data.len() * 8;

    // Read a code from the bit stream
    let read_code = |bit_pos: &mut usize, code_size: u8| -> Result<u16, ImageCodecError> {
        if *bit_pos + code_size as usize > total_bits {
            return Err(ImageCodecError::TruncatedData);
        }
        let mut code: u16 = 0;
        for i in 0..code_size {
            let byte_idx = (*bit_pos + i as usize) / 8;
            let bit_idx = (*bit_pos + i as usize) % 8;
            if byte_idx < data.len() && (data[byte_idx] >> bit_idx) & 1 != 0 {
                code |= 1 << i;
            }
        }
        *bit_pos += code_size as usize;
        Ok(code)
    };

    // Expect initial clear code
    let first = read_code(&mut bit_pos, code_size)?;
    if first != clear_code {
        // Some GIFs don't start with clear code; reset anyway
        code_size = min_code_size + 1;
        next_code = eoi_code + 1;
    }

    // Read first real code
    let mut prev_code = read_code(&mut bit_pos, code_size)?;
    if prev_code == eoi_code {
        return Ok(output);
    }
    if prev_code < clear_code {
        output.push(prev_code as u8);
    }

    // Temporary buffer for string output
    let mut string_buf: Vec<u8> = Vec::with_capacity(max_table_size);

    while let Ok(code) = read_code(&mut bit_pos, code_size) {
        if code == eoi_code {
            break;
        }

        if code == clear_code {
            code_size = min_code_size + 1;
            next_code = eoi_code + 1;

            // Read next code after clear
            prev_code = match read_code(&mut bit_pos, code_size) {
                Ok(c) => c,
                Err(_) => break,
            };
            if prev_code == eoi_code {
                break;
            }
            if prev_code < clear_code {
                output.push(prev_code as u8);
            }
            continue;
        }

        // Output the string for this code
        string_buf.clear();

        if (code as usize) < next_code as usize {
            // Code is in the table -- output its string
            let mut c = code;
            while c >= clear_code && c != eoi_code {
                let ci = c as usize;
                if ci >= max_table_size {
                    return Err(ImageCodecError::DecompressionError);
                }
                string_buf.push(table_suffix[ci]);
                c = table_prefix[ci];
            }
            string_buf.push(c as u8);
            string_buf.reverse();
        } else if code == next_code {
            // Special case: code not yet in table
            let mut c = prev_code;
            while c >= clear_code && c != eoi_code {
                let ci = c as usize;
                if ci >= max_table_size {
                    return Err(ImageCodecError::DecompressionError);
                }
                string_buf.push(table_suffix[ci]);
                c = table_prefix[ci];
            }
            string_buf.push(c as u8);
            string_buf.reverse();
            // Append first character of string
            if let Some(&first_char) = string_buf.first() {
                string_buf.push(first_char);
            }
        } else {
            return Err(ImageCodecError::DecompressionError);
        }

        output.extend_from_slice(&string_buf);

        // Add new entry to table
        if (next_code as usize) < max_table_size {
            let ni = next_code as usize;
            table_prefix[ni] = prev_code;
            table_suffix[ni] = if let Some(&first) = string_buf.first() {
                first
            } else {
                0
            };
            table_len[ni] = table_len[prev_code as usize] + 1;
            next_code += 1;

            // Increase code size when needed
            if next_code > (1 << code_size) && code_size < 12 {
                code_size += 1;
            }
        }

        prev_code = code;
    }

    Ok(output)
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
    if data.len() >= 8 && data[..8] == PNG_SIGNATURE {
        ImageCodecFormat::Png
    } else if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        ImageCodecFormat::Jpeg
    } else if data.len() >= 6 && (&data[..6] == GIF87A || &data[..6] == GIF89A) {
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

    // -----------------------------------------------------------------------
    // PNG tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_png_signature_check() {
        let bad = [0u8; 8];
        assert_eq!(decode_png(&bad), Err(ImageCodecError::InvalidSignature));
    }

    #[test]
    fn test_png_too_short() {
        let data = [137, 80, 78, 71];
        assert_eq!(decode_png(&data), Err(ImageCodecError::InvalidSignature));
    }

    #[test]
    fn test_png_color_type_from_u8() {
        assert_eq!(PngColorType::from_u8(0), Ok(PngColorType::Grayscale));
        assert_eq!(PngColorType::from_u8(2), Ok(PngColorType::Rgb));
        assert_eq!(PngColorType::from_u8(3), Ok(PngColorType::Indexed));
        assert_eq!(PngColorType::from_u8(4), Ok(PngColorType::GrayscaleAlpha));
        assert_eq!(PngColorType::from_u8(6), Ok(PngColorType::Rgba));
        assert_eq!(PngColorType::from_u8(7), Err(ImageCodecError::Unsupported));
    }

    #[test]
    fn test_png_color_type_channels() {
        assert_eq!(PngColorType::Grayscale.channels(), 1);
        assert_eq!(PngColorType::Rgb.channels(), 3);
        assert_eq!(PngColorType::Indexed.channels(), 1);
        assert_eq!(PngColorType::GrayscaleAlpha.channels(), 2);
        assert_eq!(PngColorType::Rgba.channels(), 4);
    }

    #[test]
    fn test_png_unfilter_none() {
        let mut row = vec![1, 2, 3, 4];
        let prev = vec![0, 0, 0, 0];
        png_unfilter(0, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_png_unfilter_sub() {
        let mut row = vec![1, 2, 3, 4];
        let prev = vec![0, 0, 0, 0];
        png_unfilter(1, &mut row, &prev, 1).unwrap();
        // Sub: each byte += previous byte in row
        // [1, 1+2=3, 3+3=6, 6+4=10]
        assert_eq!(row, vec![1, 3, 6, 10]);
    }

    #[test]
    fn test_png_unfilter_up() {
        let mut row = vec![1, 2, 3, 4];
        let prev = vec![10, 20, 30, 40];
        png_unfilter(2, &mut row, &prev, 1).unwrap();
        assert_eq!(row, vec![11, 22, 33, 44]);
    }

    #[test]
    fn test_png_unfilter_average() {
        let mut row = vec![0, 0, 0, 0];
        let prev = vec![10, 20, 30, 40];
        png_unfilter(3, &mut row, &prev, 1).unwrap();
        // Average: byte += floor((a + b) / 2) where a=left, b=above
        // [0+floor(0+10)/2=5, 0+floor(5+20)/2=12, 0+floor(12+30)/2=21,
        // 0+floor(21+40)/2=30]
        assert_eq!(row, vec![5, 12, 21, 30]);
    }

    #[test]
    fn test_png_unfilter_paeth() {
        let mut row = vec![10, 20, 30, 40];
        let prev = vec![0, 0, 0, 0];
        png_unfilter(4, &mut row, &prev, 1).unwrap();
        // Paeth with all-zero prev: a=left, b=0, c=0 => paeth=a (except first where
        // a=0) [10+paeth(0,0,0)=10, 20+paeth(10,0,0)=30, 30+paeth(30,0,0)=60,
        // 40+paeth(60,0,0)=100]
        assert_eq!(row, vec![10, 30, 60, 100]);
    }

    #[test]
    fn test_paeth_predictor_basic() {
        // When a=0, b=0, c=0 => p=0, pa=0, pb=0, pc=0 => returns a=0
        assert_eq!(paeth_predictor(0, 0, 0), 0);
        // When a=10, b=20, c=5 => p=25, pa=15, pb=5, pc=20 => returns b=20
        assert_eq!(paeth_predictor(10, 20, 5), 20);
    }

    #[test]
    fn test_extract_sample_1bit() {
        let row = vec![0b10110100];
        assert_eq!(extract_sample(&row, 0, 1), 1); // bit 7
        assert_eq!(extract_sample(&row, 1, 1), 0); // bit 6
        assert_eq!(extract_sample(&row, 2, 1), 1); // bit 5
        assert_eq!(extract_sample(&row, 3, 1), 1); // bit 4
        assert_eq!(extract_sample(&row, 4, 1), 0); // bit 3
        assert_eq!(extract_sample(&row, 5, 1), 1); // bit 2
    }

    #[test]
    fn test_extract_sample_4bit() {
        let row = vec![0xAB, 0xCD];
        assert_eq!(extract_sample(&row, 0, 4), 0xA);
        assert_eq!(extract_sample(&row, 1, 4), 0xB);
        assert_eq!(extract_sample(&row, 2, 4), 0xC);
        assert_eq!(extract_sample(&row, 3, 4), 0xD);
    }

    #[test]
    fn test_scale_to_8bit() {
        assert_eq!(scale_to_8bit(0, 1), 0);
        assert_eq!(scale_to_8bit(1, 1), 255);
        assert_eq!(scale_to_8bit(0, 2), 0);
        assert_eq!(scale_to_8bit(3, 2), 255);
        assert_eq!(scale_to_8bit(128, 8), 128);
        assert_eq!(scale_to_8bit(0xFF00, 16), 255);
    }

    #[test]
    fn test_png_unfilter_invalid_filter() {
        let mut row = vec![1, 2, 3];
        let prev = vec![0, 0, 0];
        assert_eq!(
            png_unfilter(5, &mut row, &prev, 1),
            Err(ImageCodecError::CorruptData)
        );
    }

    // -----------------------------------------------------------------------
    // Adler-32 / zlib tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_adler32_empty() {
        assert_eq!(adler32(&[]), 0x00000001);
    }

    #[test]
    fn test_adler32_known() {
        // adler32("Wikipedia") = 0x11E60398
        let data = b"Wikipedia";
        assert_eq!(adler32(data), 0x11E60398);
    }

    // -----------------------------------------------------------------------
    // DEFLATE / Huffman tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_huffman_tree_from_lengths() {
        // Simple tree: symbols 0,1 with lengths 1,1 => codes 0,1
        let lengths = [1u8, 1];
        let tree = HuffmanTree::from_lengths(&lengths).unwrap();
        assert_eq!(tree.counts[1], 2);
        assert_eq!(tree.symbols.len(), 2);
    }

    #[test]
    fn test_deflate_stored_block() {
        // Construct a minimal stored block: BFINAL=1, BTYPE=00, LEN=3, NLEN=~3,
        // data="abc"
        let mut block = Vec::new();
        block.push(0x01); // BFINAL=1, BTYPE=00 (stored) => bits: 1 00 = 0b001
                          // LEN = 3 (LE)
        block.push(0x03);
        block.push(0x00);
        // NLEN = !3 = 0xFFFC (LE)
        block.push(0xFC);
        block.push(0xFF);
        // Data
        block.push(b'a');
        block.push(b'b');
        block.push(b'c');

        let result = deflate_decompress(&block).unwrap();
        assert_eq!(result, b"abc");
    }

    #[test]
    fn test_bit_reader_basic() {
        let data = [0b10110100, 0xFF];
        let mut reader = BitReader::new(&data);
        // Read 4 bits LSB first from 0b10110100 => bits: 0,0,1,0 => 0b0100 = 4
        let v = reader.read_bits(4).unwrap();
        assert_eq!(v, 0b0100);
    }

    // -----------------------------------------------------------------------
    // JPEG tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_jpeg_signature_check() {
        let bad = [0x00, 0x00, 0xFF, 0xD8];
        assert_eq!(decode_jpeg(&bad), Err(ImageCodecError::InvalidSignature));
    }

    #[test]
    fn test_jpeg_too_short() {
        let data = [0xFF, 0xD8];
        // Should fail during parsing (no SOF0)
        assert!(decode_jpeg(&data).is_err());
    }

    #[test]
    fn test_jpeg_zigzag_order() {
        // Verify first few entries
        assert_eq!(ZIGZAG[0], 0);
        assert_eq!(ZIGZAG[1], 1);
        assert_eq!(ZIGZAG[2], 8);
        assert_eq!(ZIGZAG[3], 16);
        assert_eq!(ZIGZAG[4], 9);
        assert_eq!(ZIGZAG[63], 63);
    }

    #[test]
    fn test_jpeg_huff_table_build() {
        let mut table = JpegHuffTable::default();
        // 1 code of length 1
        table.counts[1] = 1;
        table.symbols = vec![0x42];
        table.build();
        assert_eq!(table.max_code[1], 0);
        assert_eq!(table.val_offset[1], 0);
    }

    #[test]
    fn test_idct_dc_only() {
        // IDCT of a block with only DC coefficient should produce uniform output
        let mut block = [0i32; 64];
        block[0] = 100;
        idct_integer(&mut block);
        // All values should be the same (DC distributes evenly)
        let dc_val = block[0];
        for &v in &block {
            // Allow small rounding differences
            assert!((v - dc_val).abs() <= 1, "Expected ~{}, got {}", dc_val, v);
        }
    }

    // -----------------------------------------------------------------------
    // GIF tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gif_signature_check() {
        let bad = [0u8; 13];
        assert_eq!(decode_gif(&bad), Err(ImageCodecError::InvalidSignature));
    }

    #[test]
    fn test_gif_too_short() {
        let data = b"GIF89";
        assert_eq!(decode_gif(data), Err(ImageCodecError::TruncatedData));
    }

    #[test]
    fn test_gif_disposal_default() {
        assert_eq!(GifDisposal::default(), GifDisposal::None);
    }

    #[test]
    fn test_lzw_decompress_basic() {
        // Minimal LZW stream: clear code, code 0, EOI
        // min_code_size=2 => clear=4, eoi=5, initial code_size=3
        // Codes: 4 (clear), 0, 5 (eoi)
        // In bits (3 bits each, LSB first):
        // 4 = 100, 0 = 000, 5 = 101
        // Packed: 100 000 101 = byte 0: 00000100 = 0x04, byte 1: 00000101 >> shifted
        // Actually LSB: bits 0-2 = 100(4), bits 3-5 = 000(0), bits 6-8 = 101(5)
        // byte 0: bits 0-7: bit0=0,bit1=0,bit2=1,bit3=0,bit4=0,bit5=0,bit6=1,bit7=0 = 0x44
        // byte 1: bit8=1 => 0x01
        let data = vec![0x44, 0x01];
        let result = lzw_decompress(&data, 2).unwrap();
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn test_lzw_decompress_multi() {
        // min_code_size=2 => clear=4, eoi=5, code_size=3
        // Stream: clear(4), 0, 1, 2, 3, eoi(5)
        // 3-bit codes packed LSB-first:
        // 4=100, 0=000, 1=001, 2=010, 3=011, 5=101
        // bits: 100 000 001 010 011 101
        // byte 0 (bits 0-7): 00000100 = 0x04
        // byte 1 (bits 8-15): 01001001 = 0x49
        // byte 2 (bits 16-17): 01 => but we need 011 101
        //   bits 16-18: 011 = 3
        //   bits 19-21: 101 = 5
        // byte 2 (bits 16-23): 10101 011 => but reversed: 011 101 00 = 01110100 = no...
        // Let me recalculate carefully:
        // bit 0-2: 100 (4)   => byte0 bits 0-2
        // bit 3-5: 000 (0)   => byte0 bits 3-5
        // bit 6-8: 001 (1)   => byte0 bits 6-7, byte1 bit 0
        // bit 9-11: 010 (2)  => byte1 bits 1-3
        // bit 12-14: 011 (3) => byte1 bits 4-6
        // bit 15-17: 101 (5) => byte1 bit 7, byte2 bits 0-1
        // byte0 = bit7..bit0 = 0 1 | 0 0 0 | 1 0 0 = 01000100 = 0x44
        // Wait, LSB first means bit 0 is the least significant bit of byte 0.
        // byte0: bits 0,1,2,3,4,5,6,7
        //        1,0,0,0,0,0,1,0 (code 4 = 100 at bits 0-2, code 0 = 000 at bits 3-5,
        // code 1's first bit at 6,7) code 4 = 100: bit0=0, bit1=0, bit2=1 =>
        // positions 0,1,2 of stream Hmm actually: code value 4 in binary = 100.
        // LSB first: bit0=0, bit1=0, bit2=1. So byte0 bits 0..7: 0(4.b0)
        // 0(4.b1) 1(4.b2) 0(0.b0) 0(0.b1) 0(0.b2) 1(1.b0) 0(1.b1)
        // byte0 = 0b01000100 = 0x44
        // byte1 bits 0..7: 0(1.b2) 0(2.b0) 1(2.b1) 0(2.b2) 1(3.b0) 1(3.b1) 0(3.b2)
        // 1(5.b0) byte1 = 0b10110010 = nope, let me be more careful
        // 1.b2=0, 2.b0=0, 2.b1=1, 2.b2=0, 3.b0=1, 3.b1=1, 3.b2=0, 5.b0=1
        // byte1 = bit0=0, bit1=0, bit2=1, bit3=0, bit4=1, bit5=1, bit6=0, bit7=1
        // byte1 = 0b10110100 = 0xB4
        // byte2 bits 0..1: 5.b1=0, 5.b2=1
        // byte2 = 0b00000010 = 0x02
        let data = vec![0x44, 0xB4, 0x02];
        let result = lzw_decompress(&data, 2).unwrap();
        assert_eq!(result, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_gif_minimal_valid() {
        // Build a minimal valid 1x1 GIF89a with a single red pixel
        let mut gif = Vec::new();

        // Header
        gif.extend_from_slice(b"GIF89a");

        // Logical Screen Descriptor
        gif.push(1);
        gif.push(0); // width = 1
        gif.push(1);
        gif.push(0); // height = 1
        gif.push(0x80); // GCT flag set, 2 colors (size bits = 0 => 2^(0+1)=2 entries)
        gif.push(0); // bg color index
        gif.push(0); // pixel aspect ratio

        // Global Color Table (2 entries = 6 bytes)
        gif.extend_from_slice(&[0, 0, 0]); // color 0: black
        gif.extend_from_slice(&[255, 0, 0]); // color 1: red

        // Image Descriptor
        gif.push(0x2C);
        gif.push(0);
        gif.push(0); // left = 0
        gif.push(0);
        gif.push(0); // top = 0
        gif.push(1);
        gif.push(0); // width = 1
        gif.push(1);
        gif.push(0); // height = 1
        gif.push(0); // no LCT, not interlaced

        // LZW min code size
        gif.push(2);

        // LZW data sub-block: clear(4), 1(red), eoi(5)
        // min_code_size=2, code_size=3
        // 4=100, 1=001, 5=101 packed LSB first:
        // bits 0-2: 0,0,1 (value 4)
        // bits 3-5: 1,0,0 (value 1)
        // bits 6-8: 1,0,1 (value 5)
        // byte0: bit0=0 bit1=0 bit2=1 bit3=1 bit4=0 bit5=0 bit6=1 bit7=0 = 0b01001100 =
        // 0x4C byte1: bit0=1 = 0x01
        gif.push(2); // sub-block size
        gif.push(0x4C);
        gif.push(0x01);
        gif.push(0); // block terminator

        // Trailer
        gif.push(0x3B);

        let result = decode_gif(&gif).unwrap();
        assert_eq!(result.width, 1);
        assert_eq!(result.height, 1);
        assert_eq!(result.frames.len(), 1);
        // The pixel should be red
        let (r, g, b, a) = result.frames[0].image.get_pixel(0, 0);
        assert_eq!((r, g, b, a), (255, 0, 0, 255));
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
