//! Image format decoders (TGA, QOI)
//!
//! Provides decoders for Truevision TGA and Quite OK Image (QOI) formats.
//! Both decoders produce `VideoFrame` output using the parent module's
//! `PixelFormat::ARGB8888` for maximum fidelity.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::{PixelFormat, VideoFrame};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Format detection
// ---------------------------------------------------------------------------

/// Supported image formats the decoder can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    TGA,
    QOI,
    PPM,
    BMP,
    Unknown,
}

/// Detect the image format from the file header / magic bytes.
pub fn detect_format(data: &[u8]) -> ImageFormat {
    if data.len() < 4 {
        return ImageFormat::Unknown;
    }

    // QOI: starts with "qoif" (0x716F6966)
    if data[0] == b'q' && data[1] == b'o' && data[2] == b'i' && data[3] == b'f' {
        return ImageFormat::QOI;
    }

    // BMP: starts with "BM" (0x42 0x4D)
    if data[0] == 0x42 && data[1] == 0x4D {
        return ImageFormat::BMP;
    }

    // PPM: starts with "P3" or "P6"
    if data[0] == b'P' && (data[1] == b'3' || data[1] == b'6') {
        return ImageFormat::PPM;
    }

    // TGA has no reliable magic bytes. We apply heuristic checks on the header
    // fields when the data is long enough to contain a TGA header (18 bytes).
    if data.len() >= 18 {
        let color_map_type = data[1];
        let image_type = data[2];

        // color_map_type must be 0 or 1
        let valid_cmt = color_map_type <= 1;
        // image_type must be one of the known TGA types
        let valid_type = matches!(image_type, 0 | 1 | 2 | 3 | 9 | 10 | 11);
        // pixel depth must be a standard value
        let pixel_depth = data[16];
        let valid_depth = matches!(pixel_depth, 8 | 15 | 16 | 24 | 32);

        if valid_cmt && valid_type && valid_depth && image_type != 0 {
            return ImageFormat::TGA;
        }
    }

    ImageFormat::Unknown
}

// ---------------------------------------------------------------------------
// TGA decoder
// ---------------------------------------------------------------------------

/// TGA file header (18 bytes).
#[derive(Debug, Clone, Copy)]
pub struct TgaHeader {
    pub id_length: u8,
    pub color_map_type: u8,
    pub image_type: u8,
    pub color_map_spec: [u8; 5],
    pub x_origin: u16,
    pub y_origin: u16,
    pub width: u16,
    pub height: u16,
    pub pixel_depth: u8,
    pub image_descriptor: u8,
}

impl TgaHeader {
    fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 18 {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "TGA data too short for header",
            });
        }

        Ok(Self {
            id_length: data[0],
            color_map_type: data[1],
            image_type: data[2],
            color_map_spec: [data[3], data[4], data[5], data[6], data[7]],
            x_origin: read_le_u16(data, 8),
            y_origin: read_le_u16(data, 10),
            width: read_le_u16(data, 12),
            height: read_le_u16(data, 14),
            pixel_depth: data[16],
            image_descriptor: data[17],
        })
    }
}

/// Decode a TGA image.
///
/// Supports:
/// - Type 2: uncompressed true-color (24-bit and 32-bit)
/// - Type 10: RLE-compressed true-color (24-bit and 32-bit)
/// - Bottom-left (default) and top-left origin (bit 5 of image_descriptor)
pub fn decode_tga(data: &[u8]) -> Result<VideoFrame, KernelError> {
    let header = TgaHeader::parse(data)?;

    // Validate image type
    if header.image_type != 2 && header.image_type != 10 {
        return Err(KernelError::InvalidArgument {
            name: "image_type",
            value: "only uncompressed (2) and RLE (10) true-color supported",
        });
    }

    // Validate pixel depth
    let bpp = header.pixel_depth;
    if bpp != 24 && bpp != 32 {
        return Err(KernelError::InvalidArgument {
            name: "pixel_depth",
            value: "only 24-bit and 32-bit TGA supported",
        });
    }

    let width = header.width as u32;
    let height = header.height as u32;
    if width == 0 || height == 0 {
        return Err(KernelError::InvalidArgument {
            name: "dimensions",
            value: "zero width or height",
        });
    }

    let bytes_per_pixel = (bpp / 8) as usize;
    let pixel_count = (width as usize) * (height as usize);

    // Offset past header + image ID field
    let pixel_data_start = 18 + header.id_length as usize;
    if pixel_data_start > data.len() {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "TGA data truncated before pixel data",
        });
    }

    // Decode pixels into a flat RGBA buffer (row-major, top-to-bottom).
    // We'll handle origin flipping at the end.
    let mut pixels: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(pixel_count);

    if header.image_type == 2 {
        // Uncompressed
        let needed = pixel_data_start + pixel_count * bytes_per_pixel;
        if data.len() < needed {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "TGA uncompressed data truncated",
            });
        }

        let mut pos = pixel_data_start;
        for _ in 0..pixel_count {
            let (r, g, b, a) = read_tga_pixel(data, pos, bytes_per_pixel);
            pixels.push((r, g, b, a));
            pos += bytes_per_pixel;
        }
    } else {
        // RLE (type 10)
        let mut pos = pixel_data_start;
        while pixels.len() < pixel_count && pos < data.len() {
            let packet = data[pos];
            pos += 1;
            let count = (packet & 0x7F) as usize + 1;

            if packet & 0x80 != 0 {
                // RLE packet: one pixel repeated `count` times
                if pos + bytes_per_pixel > data.len() {
                    break;
                }
                let (r, g, b, a) = read_tga_pixel(data, pos, bytes_per_pixel);
                pos += bytes_per_pixel;
                for _ in 0..count {
                    if pixels.len() >= pixel_count {
                        break;
                    }
                    pixels.push((r, g, b, a));
                }
            } else {
                // Raw packet: `count` literal pixels
                for _ in 0..count {
                    if pixels.len() >= pixel_count || pos + bytes_per_pixel > data.len() {
                        break;
                    }
                    let (r, g, b, a) = read_tga_pixel(data, pos, bytes_per_pixel);
                    pixels.push((r, g, b, a));
                    pos += bytes_per_pixel;
                }
            }
        }
    }

    if pixels.len() < pixel_count {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "TGA pixel data incomplete",
        });
    }

    // Origin: bit 5 of image_descriptor => 1 = top-left, 0 = bottom-left
    let top_left_origin = (header.image_descriptor & 0x20) != 0;

    let mut frame = VideoFrame::new(width, height, PixelFormat::ARGB8888);
    for row in 0..height {
        let src_row = if top_left_origin {
            row
        } else {
            height - 1 - row
        };
        for col in 0..width {
            let idx = (src_row as usize) * (width as usize) + (col as usize);
            let (r, g, b, a) = pixels[idx];
            frame.set_pixel(col, row, r, g, b, a);
        }
    }

    Ok(frame)
}

/// Read a single TGA pixel in BGR(A) order.
fn read_tga_pixel(data: &[u8], offset: usize, bpp: usize) -> (u8, u8, u8, u8) {
    // TGA stores pixels as B, G, R [, A]
    let b = data[offset];
    let g = data[offset + 1];
    let r = data[offset + 2];
    let a = if bpp >= 4 { data[offset + 3] } else { 0xFF };
    (r, g, b, a)
}

// ---------------------------------------------------------------------------
// QOI decoder (Quite OK Image format)
// ---------------------------------------------------------------------------

/// QOI operation tags.
const QOI_OP_RGB: u8 = 0xFE;
const QOI_OP_RGBA: u8 = 0xFF;
const QOI_OP_INDEX_MASK: u8 = 0x00; // 2-bit tag: 00xxxxxx
const QOI_OP_DIFF_MASK: u8 = 0x40; // 2-bit tag: 01xxxxxx
const QOI_OP_LUMA_MASK: u8 = 0x80; // 2-bit tag: 10xxxxxx
const QOI_OP_RUN_MASK: u8 = 0xC0; // 2-bit tag: 11xxxxxx

/// QOI hash function: (r * 3 + g * 5 + b * 7 + a * 11) % 64
#[inline]
fn qoi_hash(r: u8, g: u8, b: u8, a: u8) -> usize {
    ((r as usize) * 3 + (g as usize) * 5 + (b as usize) * 7 + (a as usize) * 11) % 64
}

/// Decode a QOI (Quite OK Image) file.
///
/// QOI specification: <https://qoiformat.org/qoi-specification.pdf>
///
/// Header: "qoif" (4B), width (u32 BE), height (u32 BE), channels (u8),
/// colorspace (u8) End marker: 7 zero bytes + 0x01
pub fn decode_qoi(data: &[u8]) -> Result<VideoFrame, KernelError> {
    // Minimum: 14 byte header + 8 byte end marker
    if data.len() < 22 {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "QOI data too short",
        });
    }

    // Check magic
    if data[0] != b'q' || data[1] != b'o' || data[2] != b'i' || data[3] != b'f' {
        return Err(KernelError::InvalidArgument {
            name: "magic",
            value: "not a QOI file",
        });
    }

    let width = read_be_u32(data, 4);
    let height = read_be_u32(data, 8);
    let channels = data[12];
    let _colorspace = data[13];

    if width == 0 || height == 0 {
        return Err(KernelError::InvalidArgument {
            name: "dimensions",
            value: "zero width or height",
        });
    }

    if channels != 3 && channels != 4 {
        return Err(KernelError::InvalidArgument {
            name: "channels",
            value: "must be 3 or 4",
        });
    }

    let pixel_count = (width as usize) * (height as usize);
    let mut frame = VideoFrame::new(width, height, PixelFormat::ARGB8888);

    // Previously seen pixel array (64 entries)
    let mut index: [(u8, u8, u8, u8); 64] = [(0, 0, 0, 0); 64];

    // Current pixel (starts as r=0, g=0, b=0, a=255 per spec)
    let mut pr: u8 = 0;
    let mut pg: u8 = 0;
    let mut pb: u8 = 0;
    let mut pa: u8 = 255;

    let mut pos: usize = 14; // past header
    let mut px_idx: usize = 0;

    while px_idx < pixel_count && pos < data.len() {
        let b1 = data[pos];

        if b1 == QOI_OP_RGB {
            // RGB literal
            if pos + 3 >= data.len() {
                break;
            }
            pr = data[pos + 1];
            pg = data[pos + 2];
            pb = data[pos + 3];
            pos += 4;
        } else if b1 == QOI_OP_RGBA {
            // RGBA literal
            if pos + 4 >= data.len() {
                break;
            }
            pr = data[pos + 1];
            pg = data[pos + 2];
            pb = data[pos + 3];
            pa = data[pos + 4];
            pos += 5;
        } else {
            let tag = b1 & 0xC0;
            match tag {
                0x00 => {
                    // QOI_OP_INDEX: 00xxxxxx
                    let idx = (b1 & 0x3F) as usize;
                    let (ir, ig, ib, ia) = index[idx];
                    pr = ir;
                    pg = ig;
                    pb = ib;
                    pa = ia;
                    pos += 1;
                }
                0x40 => {
                    // QOI_OP_DIFF: 01drr dgg dbb
                    // dr, dg, db are stored with bias of 2: actual = stored - 2
                    let dr = ((b1 >> 4) & 0x03) as i8 - 2;
                    let dg = ((b1 >> 2) & 0x03) as i8 - 2;
                    let db = (b1 & 0x03) as i8 - 2;
                    pr = pr.wrapping_add(dr as u8);
                    pg = pg.wrapping_add(dg as u8);
                    pb = pb.wrapping_add(db as u8);
                    pos += 1;
                }
                0x80 => {
                    // QOI_OP_LUMA: 10dddddd followed by one byte: drdg(4) dbdg(4)
                    if pos + 1 >= data.len() {
                        break;
                    }
                    let b2 = data[pos + 1];
                    let dg = (b1 & 0x3F) as i8 - 32;
                    let dr_dg = ((b2 >> 4) & 0x0F) as i8 - 8;
                    let db_dg = (b2 & 0x0F) as i8 - 8;
                    let dr = (dr_dg + dg) as u8;
                    let db = (db_dg + dg) as u8;
                    pr = pr.wrapping_add(dr);
                    pg = pg.wrapping_add(dg as u8);
                    pb = pb.wrapping_add(db);
                    pos += 2;
                }
                0xC0 => {
                    // QOI_OP_RUN: 11rrrrrr, run length = (rr & 0x3F) + 1 (1..62)
                    let run = (b1 & 0x3F) as usize + 1;
                    // Write `run` copies of the current pixel
                    for _ in 0..run {
                        if px_idx >= pixel_count {
                            break;
                        }
                        let x = (px_idx % width as usize) as u32;
                        let y = (px_idx / width as usize) as u32;
                        frame.set_pixel(x, y, pr, pg, pb, pa);
                        px_idx += 1;
                    }
                    // Update the index for the run pixel
                    index[qoi_hash(pr, pg, pb, pa)] = (pr, pg, pb, pa);
                    continue; // already wrote pixels
                }
                _ => {
                    pos += 1;
                    continue;
                }
            }
        }

        // Store current pixel in index
        index[qoi_hash(pr, pg, pb, pa)] = (pr, pg, pb, pa);

        // Write one pixel
        if px_idx < pixel_count {
            let x = (px_idx % width as usize) as u32;
            let y = (px_idx / width as usize) as u32;
            frame.set_pixel(x, y, pr, pg, pb, pa);
            px_idx += 1;
        }
    }

    Ok(frame)
}

// ---------------------------------------------------------------------------
// Unified decoder
// ---------------------------------------------------------------------------

/// Auto-detect the image format and decode it.
///
/// Supports TGA, QOI.  PPM and BMP are detected but not decoded here
/// (use the desktop image_viewer for those).
pub fn decode_image(data: &[u8]) -> Result<VideoFrame, KernelError> {
    let fmt = detect_format(data);
    match fmt {
        ImageFormat::TGA => decode_tga(data),
        ImageFormat::QOI => decode_qoi(data),
        ImageFormat::PPM | ImageFormat::BMP => Err(KernelError::InvalidArgument {
            name: "format",
            value: "PPM/BMP should be decoded via desktop::image_viewer",
        }),
        ImageFormat::Unknown => Err(KernelError::InvalidArgument {
            name: "format",
            value: "unknown or unsupported image format",
        }),
    }
}

// ---------------------------------------------------------------------------
// Byte-reading helpers
// ---------------------------------------------------------------------------

/// Read a big-endian u32.
fn read_be_u32(data: &[u8], off: usize) -> u32 {
    ((data[off] as u32) << 24)
        | ((data[off + 1] as u32) << 16)
        | ((data[off + 2] as u32) << 8)
        | (data[off + 3] as u32)
}

/// Read a little-endian u16.
fn read_le_u16(data: &[u8], off: usize) -> u16 {
    (data[off] as u16) | ((data[off + 1] as u16) << 8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use alloc::vec;

    #[test]
    fn test_detect_qoi() {
        let data = b"qoif\x00\x00\x00\x01\x00\x00\x00\x01\x04\x00extra";
        assert_eq!(detect_format(data), ImageFormat::QOI);
    }

    #[test]
    fn test_detect_bmp() {
        let mut data = vec![0u8; 54];
        data[0] = 0x42;
        data[1] = 0x4D;
        assert_eq!(detect_format(&data), ImageFormat::BMP);
    }

    #[test]
    fn test_detect_ppm() {
        let data = b"P6\n10 10\n255\n";
        assert_eq!(detect_format(data), ImageFormat::PPM);
    }

    #[test]
    fn test_detect_unknown() {
        let data = b"\x00\x00\x00\x00";
        assert_eq!(detect_format(data), ImageFormat::Unknown);
    }

    #[test]
    fn test_tga_header_parse() {
        // Minimal valid TGA header for a 2x2, 24-bit, uncompressed image
        let mut header_data = vec![0u8; 18];
        header_data[2] = 2; // image type: uncompressed true-color
                            // width = 2 (LE)
        header_data[12] = 2;
        header_data[13] = 0;
        // height = 2 (LE)
        header_data[14] = 2;
        header_data[15] = 0;
        // pixel depth = 24
        header_data[16] = 24;
        // image descriptor: bit 5 set = top-left origin
        header_data[17] = 0x20;

        let hdr = TgaHeader::parse(&header_data).expect("parse should succeed");
        assert_eq!(hdr.width, 2);
        assert_eq!(hdr.height, 2);
        assert_eq!(hdr.pixel_depth, 24);
        assert_eq!(hdr.image_type, 2);
        assert_ne!(hdr.image_descriptor & 0x20, 0);
    }

    #[test]
    fn test_decode_tga_uncompressed_24() {
        // Build a minimal 2x2, 24-bit, uncompressed, top-left origin TGA
        let mut data = vec![0u8; 18 + 2 * 2 * 3];
        data[2] = 2; // uncompressed true-color
        data[12] = 2;
        data[13] = 0; // width=2
        data[14] = 2;
        data[15] = 0; // height=2
        data[16] = 24; // 24 bpp
        data[17] = 0x20; // top-left origin

        // Pixel data (BGR)
        let pixels = &mut data[18..];
        // row 0: (0,0)=red, (1,0)=green
        pixels[0] = 0;
        pixels[1] = 0;
        pixels[2] = 255; // BGR -> R=255
        pixels[3] = 0;
        pixels[4] = 255;
        pixels[5] = 0; // BGR -> G=255
                       // row 1: (0,1)=blue, (1,1)=white
        pixels[6] = 255;
        pixels[7] = 0;
        pixels[8] = 0; // BGR -> B=255
        pixels[9] = 255;
        pixels[10] = 255;
        pixels[11] = 255; // white

        let frame = decode_tga(&data).expect("decode should succeed");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.get_pixel(0, 0), (255, 0, 0, 255)); // red
        assert_eq!(frame.get_pixel(1, 0), (0, 255, 0, 255)); // green
        assert_eq!(frame.get_pixel(0, 1), (0, 0, 255, 255)); // blue
        assert_eq!(frame.get_pixel(1, 1), (255, 255, 255, 255)); // white
    }

    #[test]
    fn test_qoi_magic_detection() {
        let good = b"qoif\x00\x00\x00\x01\x00\x00\x00\x01\x03\x00";
        assert_eq!(detect_format(good), ImageFormat::QOI);

        let bad = b"qoix\x00\x00\x00\x01\x00\x00\x00\x01\x03\x00";
        // "qoix" is not QOI magic
        assert_ne!(detect_format(bad), ImageFormat::QOI);
    }

    #[test]
    fn test_format_detection_priority() {
        // BMP should be detected before TGA heuristic
        let mut bmp = vec![0u8; 54];
        bmp[0] = 0x42;
        bmp[1] = 0x4D;
        // Even if TGA heuristic could match, BMP should win
        assert_eq!(detect_format(&bmp), ImageFormat::BMP);
    }
}
