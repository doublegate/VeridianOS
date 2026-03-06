//! GIF decoder (87a/89a) with LZW decompression.
//!
//! Supports global/local color tables, animation frames, disposal methods,
//! transparency, and interlaced rendering.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::{read_le_u16, DecodedImage, ImageCodecError};

// ============================================================================
// GIF DECODER (87a / 89a)
// ============================================================================

/// GIF header magic bytes.
pub(crate) const GIF87A: &[u8; 6] = b"GIF87a";
pub(crate) const GIF89A: &[u8; 6] = b"GIF89a";

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
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

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
        // byte 0: bits 0-7: bit0=0,bit1=0,bit2=1,bit3=0,bit4=0,bit5=0,bit6=1,bit7=0 =
        // 0x44 byte 1: bit8=1 => 0x01
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
}
