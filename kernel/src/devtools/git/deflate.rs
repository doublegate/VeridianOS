//! Minimal Zlib/DEFLATE Decompression
//!
//! Provides inflate (decompression) for reading Git packfiles and
//! loose objects stored in zlib format.

use alloc::vec::Vec;

/// Decompression error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateError {
    InvalidHeader,
    InvalidBlock,
    BufferOverflow,
    IncompleteInput,
    InvalidDistance,
    InvalidLitLen,
}

/// Zlib header check (2-byte header: CMF + FLG)
pub fn check_zlib_header(data: &[u8]) -> Result<usize, InflateError> {
    if data.len() < 2 {
        return Err(InflateError::IncompleteInput);
    }

    let cmf = data[0];
    let flg = data[1];

    // CM must be 8 (deflate)
    if (cmf & 0x0F) != 8 {
        return Err(InflateError::InvalidHeader);
    }

    // CMF*256 + FLG must be divisible by 31
    let check = (cmf as u16) * 256 + (flg as u16);
    if !check.is_multiple_of(31) {
        return Err(InflateError::InvalidHeader);
    }

    // Check if FDICT is set (bit 5 of FLG)
    let dict_present = (flg & 0x20) != 0;
    let header_size = if dict_present { 6 } else { 2 };

    Ok(header_size)
}

/// Bit reader for DEFLATE streams
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_pos: 0,
        }
    }

    fn read_bits(&mut self, count: u8) -> Result<u32, InflateError> {
        let mut val: u32 = 0;
        for i in 0..count {
            if self.pos >= self.data.len() {
                return Err(InflateError::IncompleteInput);
            }
            let bit = (self.data[self.pos] >> self.bit_pos) & 1;
            val |= (bit as u32) << i;
            self.bit_pos += 1;
            if self.bit_pos >= 8 {
                self.bit_pos = 0;
                self.pos += 1;
            }
        }
        Ok(val)
    }

    fn align_byte(&mut self) {
        if self.bit_pos > 0 {
            self.bit_pos = 0;
            self.pos += 1;
        }
    }

    fn read_u16_le(&mut self) -> Result<u16, InflateError> {
        self.align_byte();
        if self.pos + 2 > self.data.len() {
            return Err(InflateError::IncompleteInput);
        }
        let val = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(val)
    }

    fn bytes_consumed(&self) -> usize {
        if self.bit_pos > 0 {
            self.pos + 1
        } else {
            self.pos
        }
    }
}

/// Length base values for codes 257-285
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// Extra bits for length codes 257-285
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Distance base values
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// Extra bits for distance codes
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Inflate (decompress) raw DEFLATE data
pub fn inflate_raw(data: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    let mut reader = BitReader::new(data);
    let mut output = Vec::with_capacity(core::cmp::min(max_output, 65536));

    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;

        match btype {
            // No compression
            0 => {
                let len = reader.read_u16_le()?;
                let _nlen = reader.read_u16_le()?;

                for _ in 0..len {
                    if output.len() >= max_output {
                        return Err(InflateError::BufferOverflow);
                    }
                    if reader.pos >= reader.data.len() {
                        return Err(InflateError::IncompleteInput);
                    }
                    output.push(reader.data[reader.pos]);
                    reader.pos += 1;
                }
            }
            // Fixed Huffman
            1 => {
                inflate_fixed_huffman(&mut reader, &mut output, max_output)?;
            }
            // Dynamic Huffman
            2 => {
                inflate_dynamic_huffman(&mut reader, &mut output, max_output)?;
            }
            _ => return Err(InflateError::InvalidBlock),
        }

        if bfinal != 0 {
            break;
        }
    }

    Ok(output)
}

/// Inflate with zlib header
pub fn inflate_zlib(data: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    let header_size = check_zlib_header(data)?;
    inflate_raw(&data[header_size..], max_output)
}

fn decode_fixed_litlen(reader: &mut BitReader) -> Result<u16, InflateError> {
    // Fixed Huffman code table:
    // 0-143: 8 bits (00110000 - 10111111)
    // 144-255: 9 bits (110010000 - 111111111)
    // 256-279: 7 bits (0000000 - 0010111)
    // 280-287: 8 bits (11000000 - 11000111)

    let mut code: u32 = 0;
    for bits in 0..9u8 {
        let bit = reader.read_bits(1)?;
        // Bits are read LSB first but Huffman codes are MSB first
        code = (code << 1) | bit;

        match bits + 1 {
            7 => {
                if code <= 0b0010111 {
                    return Ok((code + 256) as u16);
                }
            }
            8 => {
                if (0b00110000..=0b10111111).contains(&code) {
                    return Ok((code - 0b00110000) as u16);
                }
                if (0b11000000..=0b11000111).contains(&code) {
                    return Ok((code - 0b11000000 + 280) as u16);
                }
            }
            9 => {
                if (0b110010000..=0b111111111).contains(&code) {
                    return Ok((code - 0b110010000 + 144) as u16);
                }
            }
            _ => {}
        }
    }

    Err(InflateError::InvalidLitLen)
}

fn inflate_fixed_huffman(
    reader: &mut BitReader,
    output: &mut Vec<u8>,
    max_output: usize,
) -> Result<(), InflateError> {
    loop {
        let lit = decode_fixed_litlen(reader)?;

        if lit < 256 {
            if output.len() >= max_output {
                return Err(InflateError::BufferOverflow);
            }
            output.push(lit as u8);
        } else if lit == 256 {
            return Ok(());
        } else {
            let len_idx = (lit - 257) as usize;
            if len_idx >= LENGTH_BASE.len() {
                return Err(InflateError::InvalidLitLen);
            }
            let length =
                LENGTH_BASE[len_idx] as usize + reader.read_bits(LENGTH_EXTRA[len_idx])? as usize;

            // Read 5-bit distance code (fixed)
            let dist_code = reader.read_bits(5)? as usize;
            // Reverse bits for fixed distance codes
            let dist_code = reverse_bits(dist_code as u32, 5) as usize;
            if dist_code >= DIST_BASE.len() {
                return Err(InflateError::InvalidDistance);
            }
            let distance =
                DIST_BASE[dist_code] as usize + reader.read_bits(DIST_EXTRA[dist_code])? as usize;

            if distance > output.len() {
                return Err(InflateError::InvalidDistance);
            }

            for _ in 0..length {
                if output.len() >= max_output {
                    return Err(InflateError::BufferOverflow);
                }
                let idx = output.len() - distance;
                output.push(output[idx]);
            }
        }
    }
}

fn inflate_dynamic_huffman(
    reader: &mut BitReader,
    output: &mut Vec<u8>,
    max_output: usize,
) -> Result<(), InflateError> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;

    // Code length code order
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    let mut cl_lengths = [0u8; 19];
    for i in 0..hclen {
        cl_lengths[CL_ORDER[i]] = reader.read_bits(3)? as u8;
    }

    // Build code-length Huffman table
    let cl_table = build_huffman_table(&cl_lengths)?;

    // Decode literal/length + distance code lengths
    let total = hlit + hdist;
    let mut code_lengths = Vec::with_capacity(total);

    while code_lengths.len() < total {
        let sym = decode_huffman(reader, &cl_table)?;
        match sym {
            0..=15 => code_lengths.push(sym as u8),
            16 => {
                let repeat = reader.read_bits(2)? as usize + 3;
                let prev = *code_lengths.last().ok_or(InflateError::InvalidBlock)?;
                code_lengths.resize(code_lengths.len() + repeat, prev);
            }
            17 => {
                let repeat = reader.read_bits(3)? as usize + 3;
                code_lengths.resize(code_lengths.len() + repeat, 0);
            }
            18 => {
                let repeat = reader.read_bits(7)? as usize + 11;
                code_lengths.resize(code_lengths.len() + repeat, 0);
            }
            _ => return Err(InflateError::InvalidBlock),
        }
    }

    let litlen_lengths = &code_lengths[..hlit];
    let dist_lengths = &code_lengths[hlit..];

    let litlen_table = build_huffman_table(litlen_lengths)?;
    let dist_table = build_huffman_table(dist_lengths)?;

    // Decode data
    loop {
        let sym = decode_huffman(reader, &litlen_table)?;

        if sym < 256 {
            if output.len() >= max_output {
                return Err(InflateError::BufferOverflow);
            }
            output.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let len_idx = (sym - 257) as usize;
            if len_idx >= LENGTH_BASE.len() {
                return Err(InflateError::InvalidLitLen);
            }
            let length =
                LENGTH_BASE[len_idx] as usize + reader.read_bits(LENGTH_EXTRA[len_idx])? as usize;

            let dist_sym = decode_huffman(reader, &dist_table)? as usize;
            if dist_sym >= DIST_BASE.len() {
                return Err(InflateError::InvalidDistance);
            }
            let distance =
                DIST_BASE[dist_sym] as usize + reader.read_bits(DIST_EXTRA[dist_sym])? as usize;

            if distance > output.len() {
                return Err(InflateError::InvalidDistance);
            }

            for _ in 0..length {
                if output.len() >= max_output {
                    return Err(InflateError::BufferOverflow);
                }
                let idx = output.len() - distance;
                output.push(output[idx]);
            }
        }
    }
}

/// Simple Huffman table entry
#[derive(Debug, Clone, Copy, Default)]
struct HuffEntry {
    symbol: u16,
    length: u8,
}

/// Huffman decode table (max 15-bit codes)
struct HuffTable {
    entries: Vec<HuffEntry>,
    max_bits: u8,
}

fn build_huffman_table(lengths: &[u8]) -> Result<HuffTable, InflateError> {
    let max_bits = *lengths.iter().max().unwrap_or(&0);
    if max_bits == 0 {
        return Ok(HuffTable {
            entries: Vec::new(),
            max_bits: 0,
        });
    }

    let table_size = 1usize << max_bits;
    let mut entries = alloc::vec![HuffEntry::default(); table_size];

    // Count code lengths
    let mut bl_count = [0u16; 16];
    for &len in lengths {
        bl_count[len as usize] += 1;
    }
    bl_count[0] = 0;

    // Compute next_code
    let mut next_code = [0u16; 16];
    let mut code: u16 = 0;
    for bits in 1..=max_bits {
        code = (code + bl_count[bits as usize - 1]) << 1;
        next_code[bits as usize] = code;
    }

    // Assign codes
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        let code = next_code[len as usize];
        next_code[len as usize] += 1;

        // Fill table entries (expand to max_bits)
        let reversed = reverse_bits(code as u32, len) as usize;
        let fill_count = 1usize << (max_bits - len);
        for i in 0..fill_count {
            let idx = reversed | (i << len);
            if idx < table_size {
                entries[idx] = HuffEntry {
                    symbol: sym as u16,
                    length: len,
                };
            }
        }
    }

    Ok(HuffTable { entries, max_bits })
}

fn decode_huffman(reader: &mut BitReader, table: &HuffTable) -> Result<u16, InflateError> {
    if table.max_bits == 0 {
        return Err(InflateError::InvalidBlock);
    }

    let bits = reader.read_bits(table.max_bits)? as usize;
    let entry = &table.entries[bits];
    if entry.length == 0 {
        return Err(InflateError::InvalidBlock);
    }

    // Put back unused bits
    let unused = table.max_bits - entry.length;
    if unused > 0 {
        // Move bit position back
        let total_bits = reader.pos * 8 + reader.bit_pos as usize;
        let new_total = total_bits - unused as usize;
        reader.pos = new_total / 8;
        reader.bit_pos = (new_total % 8) as u8;
    }

    Ok(entry.symbol)
}

fn reverse_bits(val: u32, bits: u8) -> u32 {
    let mut result = 0u32;
    let mut v = val;
    for _ in 0..bits {
        result = (result << 1) | (v & 1);
        v >>= 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_zlib_header_valid() {
        // CMF=0x78 (deflate, window=32768), FLG=0x01
        let result = check_zlib_header(&[0x78, 0x01]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
    }

    #[test]
    fn test_check_zlib_header_9c() {
        let result = check_zlib_header(&[0x78, 0x9C]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_zlib_header_too_short() {
        let result = check_zlib_header(&[0x78]);
        assert_eq!(result, Err(InflateError::IncompleteInput));
    }

    #[test]
    fn test_check_zlib_header_invalid_method() {
        let result = check_zlib_header(&[0x77, 0x01]);
        assert_eq!(result, Err(InflateError::InvalidHeader));
    }

    #[test]
    fn test_reverse_bits() {
        assert_eq!(reverse_bits(0b110, 3), 0b011);
        assert_eq!(reverse_bits(0b1010, 4), 0b0101);
        assert_eq!(reverse_bits(0b1, 1), 0b1);
    }

    #[test]
    fn test_inflate_raw_stored_block() {
        // BFINAL=1, BTYPE=00 (stored), LEN=5, NLEN=0xFFFA, "hello"
        let data = [
            0x01, // bfinal=1, btype=00
            0x05, 0x00, // LEN=5
            0xFA, 0xFF, // NLEN=~5
            b'h', b'e', b'l', b'l', b'o',
        ];
        let result = inflate_raw(&data, 1024);
        assert!(result.is_ok());
        assert_eq!(&result.unwrap(), b"hello");
    }

    #[test]
    fn test_inflate_error_types() {
        assert_eq!(InflateError::InvalidHeader, InflateError::InvalidHeader);
        assert_ne!(InflateError::InvalidHeader, InflateError::InvalidBlock);
    }

    #[test]
    fn test_build_huffman_table_empty() {
        let lengths: [u8; 0] = [];
        let table = build_huffman_table(&lengths).unwrap();
        assert_eq!(table.max_bits, 0);
    }

    #[test]
    fn test_bit_reader_basic() {
        let data = [0b10110100u8];
        let mut reader = BitReader::new(&data);

        assert_eq!(reader.read_bits(1).unwrap(), 0); // bit 0
        assert_eq!(reader.read_bits(1).unwrap(), 0); // bit 1
        assert_eq!(reader.read_bits(1).unwrap(), 1); // bit 2
        assert_eq!(reader.read_bits(1).unwrap(), 0); // bit 3
        assert_eq!(reader.read_bits(1).unwrap(), 1); // bit 4
    }

    #[test]
    fn test_bit_reader_multi_bit() {
        let data = [0xFF];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(4).unwrap(), 0xF);
        assert_eq!(reader.read_bits(4).unwrap(), 0xF);
    }

    #[test]
    fn test_bit_reader_overflow() {
        let data = [0x00];
        let mut reader = BitReader::new(&data);
        let _ = reader.read_bits(8); // consume all
        let result = reader.read_bits(1);
        assert!(result.is_err());
    }

    #[test]
    fn test_inflate_buffer_overflow() {
        let data = [0x01, 0x05, 0x00, 0xFA, 0xFF, b'h', b'e', b'l', b'l', b'o'];
        let result = inflate_raw(&data, 3); // Too small buffer
        assert_eq!(result, Err(InflateError::BufferOverflow));
    }
}
