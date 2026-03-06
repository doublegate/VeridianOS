//! JPEG decoder (Baseline DCT, SOF0).
//!
//! Supports Huffman entropy coding, integer IDCT, YCbCr-to-RGB via
//! fixed-point arithmetic, chroma subsampling 4:4:4/4:2:2/4:2:0,
//! and restart intervals.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::{clamp_u8, read_be_u16, DecodedImage, ImageCodecError};

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
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

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
}
