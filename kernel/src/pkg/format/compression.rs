//! Compression and decompression implementations
//!
//! Provides LZ4, Zstandard, and Brotli compression algorithms for
//! package content. Each algorithm is implemented as a self-contained
//! submodule with compress/decompress functions.

use alloc::{string::String, vec::Vec};

use super::Compression;

// ============================================================================
// LZ4 Implementation (Simple Block Format)
// ============================================================================

/// LZ4 magic number for frame format
const LZ4_MAGIC: u32 = 0x184D2204;

/// LZ4 block maximum size (64KB)
const LZ4_BLOCK_SIZE: usize = 65536;

/// Minimum match length for LZ4
const LZ4_MIN_MATCH: usize = 4;

/// LZ4 compression implementation
mod lz4 {
    use super::*;

    /// Compress data using LZ4 block format
    pub fn compress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::with_capacity(input.len() + 16);

        // Write LZ4 frame header
        output.extend_from_slice(&LZ4_MAGIC.to_le_bytes());
        output.push(0x64); // FLG: version 01, no block checksum, content size present
        output.push(0x40); // BD: 64KB max block size

        // Write original size (8 bytes)
        output.extend_from_slice(&(input.len() as u64).to_le_bytes());

        // Simple header checksum (xxHash32 of header >> 8)
        let header_checksum = (output[4..12].iter().fold(0u8, |a, &b| a.wrapping_add(b))) >> 1;
        output.push(header_checksum);

        // Compress in blocks
        let mut pos = 0;
        while pos < input.len() {
            let block_end = core::cmp::min(pos + LZ4_BLOCK_SIZE, input.len());
            let block = &input[pos..block_end];

            let compressed_block = compress_block(block);

            // If compression doesn't help, store uncompressed
            if compressed_block.len() >= block.len() {
                // Uncompressed block (highest bit set in size)
                let block_size = (block.len() as u32) | 0x80000000;
                output.extend_from_slice(&block_size.to_le_bytes());
                output.extend_from_slice(block);
            } else {
                // Compressed block
                output.extend_from_slice(&(compressed_block.len() as u32).to_le_bytes());
                output.extend_from_slice(&compressed_block);
            }

            pos = block_end;
        }

        // End mark (0x00000000)
        output.extend_from_slice(&0u32.to_le_bytes());

        Ok(output)
    }

    /// Compress a single LZ4 block
    fn compress_block(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut pos = 0;
        let mut literal_start = 0;

        // Simple hash table for match finding
        let mut hash_table = [0usize; 4096];

        while pos + LZ4_MIN_MATCH <= input.len() {
            let hash = hash4(&input[pos..]) & 0xFFF;
            let match_pos = hash_table[hash];
            hash_table[hash] = pos;

            // Check for match
            if match_pos < pos && pos - match_pos < 65535 {
                let match_len = find_match_length(&input[match_pos..], &input[pos..]);

                if match_len >= LZ4_MIN_MATCH {
                    let offset = pos - match_pos;
                    let _literal_len = pos - literal_start;

                    // Write token
                    write_sequence(&mut output, &input[literal_start..pos], offset, match_len);

                    pos += match_len;
                    literal_start = pos;
                    continue;
                }
            }

            pos += 1;
        }

        // Write remaining literals
        if literal_start < input.len() {
            let literal_len = input.len() - literal_start;
            let token = core::cmp::min(literal_len, 15) as u8;
            output.push(token << 4);

            if literal_len >= 15 {
                let mut remaining = literal_len - 15;
                while remaining >= 255 {
                    output.push(255);
                    remaining -= 255;
                }
                output.push(remaining as u8);
            }

            output.extend_from_slice(&input[literal_start..]);
        }

        output
    }

    /// Calculate hash of 4 bytes
    fn hash4(data: &[u8]) -> usize {
        if data.len() < 4 {
            return 0;
        }
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        (val.wrapping_mul(2654435761) >> 20) as usize
    }

    /// Find match length between two positions
    fn find_match_length(match_data: &[u8], current: &[u8]) -> usize {
        let max_len = core::cmp::min(match_data.len(), current.len());
        let mut len = 0;

        while len < max_len && match_data[len] == current[len] {
            len += 1;
        }

        len
    }

    /// Write an LZ4 sequence (literals + match)
    fn write_sequence(output: &mut Vec<u8>, literals: &[u8], offset: usize, match_len: usize) {
        let literal_len = literals.len();
        let ml = match_len - LZ4_MIN_MATCH;

        // Token byte
        let lit_token = core::cmp::min(literal_len, 15) as u8;
        let match_token = core::cmp::min(ml, 15) as u8;
        output.push((lit_token << 4) | match_token);

        // Extended literal length
        if literal_len >= 15 {
            let mut remaining = literal_len - 15;
            while remaining >= 255 {
                output.push(255);
                remaining -= 255;
            }
            output.push(remaining as u8);
        }

        // Literals
        output.extend_from_slice(literals);

        // Offset (little-endian 16-bit)
        output.push((offset & 0xFF) as u8);
        output.push(((offset >> 8) & 0xFF) as u8);

        // Extended match length
        if ml >= 15 {
            let mut remaining = ml - 15;
            while remaining >= 255 {
                output.push(255);
                remaining -= 255;
            }
            output.push(remaining as u8);
        }
    }

    /// Decompress LZ4 frame format data
    pub fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.len() < 15 {
            return Err(String::from("LZ4: Input too short"));
        }

        // Verify magic number
        let magic = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
        if magic != LZ4_MAGIC {
            return Err(String::from("LZ4: Invalid magic number"));
        }

        // Parse frame descriptor
        let _flg = input[4];
        let _bd = input[5];

        // Read original size
        let original_size = u64::from_le_bytes([
            input[6], input[7], input[8], input[9], input[10], input[11], input[12], input[13],
        ]) as usize;

        let mut output = Vec::with_capacity(original_size);
        let mut pos = 15; // After header

        // Decompress blocks
        while pos + 4 <= input.len() {
            let block_size_raw =
                u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]]);
            pos += 4;

            // End mark
            if block_size_raw == 0 {
                break;
            }

            let uncompressed = (block_size_raw & 0x80000000) != 0;
            let block_size = (block_size_raw & 0x7FFFFFFF) as usize;

            if pos + block_size > input.len() {
                return Err(String::from("LZ4: Truncated block"));
            }

            if uncompressed {
                output.extend_from_slice(&input[pos..pos + block_size]);
            } else {
                decompress_block(&input[pos..pos + block_size], &mut output)?;
            }

            pos += block_size;
        }

        Ok(output)
    }

    /// Decompress a single LZ4 block
    fn decompress_block(input: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
        let mut pos = 0;

        while pos < input.len() {
            let token = input[pos];
            pos += 1;

            // Literal length
            let mut literal_len = ((token >> 4) & 0x0F) as usize;
            if literal_len == 15 {
                while pos < input.len() {
                    let byte = input[pos];
                    pos += 1;
                    literal_len += byte as usize;
                    if byte != 255 {
                        break;
                    }
                }
            }

            // Copy literals
            if pos + literal_len > input.len() {
                return Err(String::from("LZ4: Truncated literals"));
            }
            output.extend_from_slice(&input[pos..pos + literal_len]);
            pos += literal_len;

            // Check for end of block
            if pos >= input.len() {
                break;
            }

            // Match offset
            if pos + 2 > input.len() {
                return Err(String::from("LZ4: Truncated offset"));
            }
            let offset = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
            pos += 2;

            if offset == 0 || offset > output.len() {
                return Err(String::from("LZ4: Invalid offset"));
            }

            // Match length
            let mut match_len = (token & 0x0F) as usize + LZ4_MIN_MATCH;
            if (token & 0x0F) == 15 {
                while pos < input.len() {
                    let byte = input[pos];
                    pos += 1;
                    match_len += byte as usize;
                    if byte != 255 {
                        break;
                    }
                }
            }

            // Copy match
            let match_start = output.len() - offset;
            for i in 0..match_len {
                let byte = output[match_start + (i % offset)];
                output.push(byte);
            }
        }

        Ok(())
    }
}

// ============================================================================
// Zstandard Implementation (Simplified)
// ============================================================================

/// Zstd magic number
const ZSTD_MAGIC: u32 = 0xFD2FB528;

/// Zstandard compression implementation (simplified)
mod zstd {
    use super::*;

    /// Compress data using simplified Zstd-like format
    pub fn compress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::with_capacity(input.len() + 16);

        // Write Zstd frame header
        output.extend_from_slice(&ZSTD_MAGIC.to_le_bytes());

        // Frame header descriptor (simplified)
        // Bits 7-6 = content_size_flag: 0=1byte, 1=2bytes, 2=4bytes, 3=8bytes
        // Use flag=0 (bits 7-6 = 00) for 1-byte content size: FHD = 0x20
        output.push(0x20); // Single segment, content size present (1 byte, flag=0)
        output.push(input.len() as u8); // Content size (for small inputs)

        // For larger inputs, use 4-byte content size (flag=2, bits 7-6 = 10: FHD =
        // 0xA0)
        if input.len() > 255 {
            output[4] = 0xA0; // 4-byte content size, flag=2
            output.pop();
            output.extend_from_slice(&(input.len() as u32).to_le_bytes());
        }

        // Compress using FSE-like encoding with run-length
        let compressed = compress_block_rle(input);

        // Block header (3 bytes): size + type
        // Bit 0 = Last_Block, bits [2:1] = Block_Type, bits [31:3] = Block_Size
        //   Block_Type: 0=Raw, 1=RLE, 2=Compressed
        //   For Compressed (type 2 = 0b10): bits [2:1] = 10 → value = (1<<2) = 0x04
        let (block_header, actual_data_len) = if compressed.len() >= input.len() {
            // Raw block (type 0, bits [2:1] = 00)
            (((input.len() as u32) << 3) | 0x00, input.len())
        } else {
            // Compressed block (type 2, bits [2:1] = 10 → bit 2 set = 0x04)
            ((((compressed.len()) as u32) << 3) | 0x04, compressed.len())
        };

        output.push((block_header & 0xFF) as u8);
        output.push(((block_header >> 8) & 0xFF) as u8);
        output.push(((block_header >> 16) & 0xFF) as u8);

        if compressed.len() >= input.len() {
            output.extend_from_slice(input);
        } else {
            output.extend_from_slice(&compressed);
        }

        // Set the Last_Block flag (bit 0) on the block header we just wrote.
        // The block header starts at output.len() - (actual_data_len + 3).
        let flag_idx = output.len() - (actual_data_len + 3);
        output[flag_idx] |= 0x01;

        Ok(output)
    }

    /// Simple RLE + dictionary compression
    fn compress_block_rle(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut pos = 0;

        while pos < input.len() {
            let byte = input[pos];
            let mut run_len = 1;

            // Count run length
            while pos + run_len < input.len() && input[pos + run_len] == byte && run_len < 127 {
                run_len += 1;
            }

            if run_len >= 4 {
                // Encode as run
                output.push(0x80 | (run_len as u8));
                output.push(byte);
                pos += run_len;
            } else {
                // Count literals
                let literal_start = pos;
                let mut literal_len = 0;

                while pos + literal_len < input.len() && literal_len < 127 {
                    let b = input[pos + literal_len];
                    let mut next_run = 1;
                    while pos + literal_len + next_run < input.len()
                        && input[pos + literal_len + next_run] == b
                        && next_run < 127
                    {
                        next_run += 1;
                    }

                    if next_run >= 4 {
                        break;
                    }
                    literal_len += 1;
                }

                if literal_len > 0 {
                    output.push(literal_len as u8);
                    output.extend_from_slice(&input[literal_start..literal_start + literal_len]);
                    pos += literal_len;
                }
            }
        }

        output
    }

    /// Decompress Zstd frame format data
    pub fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.len() < 8 {
            return Err(String::from("Zstd: Input too short"));
        }

        // Verify magic number
        let magic = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
        if magic != ZSTD_MAGIC {
            return Err(String::from("Zstd: Invalid magic number"));
        }

        // Parse frame header
        let fhd = input[4];
        let content_size_flag = (fhd >> 6) & 0x03;

        let (content_size, mut pos) = match content_size_flag {
            0 => (input[5] as usize, 6),
            1 => (u16::from_le_bytes([input[5], input[6]]) as usize + 256, 7),
            2 => (
                u32::from_le_bytes([input[5], input[6], input[7], input[8]]) as usize,
                9,
            ),
            _ => return Err(String::from("Zstd: Unsupported content size")),
        };

        let mut output = Vec::with_capacity(content_size);

        // Decompress blocks
        while pos + 3 <= input.len() {
            let block_header = u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], 0]);
            pos += 3;

            let last_block = (block_header & 0x01) != 0;
            let block_type = (block_header >> 1) & 0x03;
            let block_size = (block_header >> 3) as usize;

            if pos + block_size > input.len() {
                return Err(String::from("Zstd: Truncated block"));
            }

            match block_type {
                0 => {
                    // Raw block
                    output.extend_from_slice(&input[pos..pos + block_size]);
                }
                1 => {
                    // RLE block
                    let byte = input[pos];
                    for _ in 0..block_size {
                        output.push(byte);
                    }
                }
                2 => {
                    // Compressed block
                    decompress_block_rle(&input[pos..pos + block_size], &mut output)?;
                }
                _ => return Err(String::from("Zstd: Reserved block type")),
            }

            pos += block_size;

            if last_block {
                break;
            }
        }

        Ok(output)
    }

    /// Decompress RLE block
    fn decompress_block_rle(input: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
        let mut pos = 0;

        while pos < input.len() {
            let control = input[pos];
            pos += 1;

            if (control & 0x80) != 0 {
                // Run-length encoded
                let run_len = (control & 0x7F) as usize;
                if pos >= input.len() {
                    return Err(String::from("Zstd: Truncated RLE"));
                }
                let byte = input[pos];
                pos += 1;
                for _ in 0..run_len {
                    output.push(byte);
                }
            } else {
                // Literal sequence
                let literal_len = control as usize;
                if pos + literal_len > input.len() {
                    return Err(String::from("Zstd: Truncated literals"));
                }
                output.extend_from_slice(&input[pos..pos + literal_len]);
                pos += literal_len;
            }
        }

        Ok(())
    }
}

// ============================================================================
// Brotli Implementation (Simplified)
// ============================================================================

/// Brotli window size bits (default)
const BROTLI_WINDOW_BITS: u8 = 22;

/// Brotli compression implementation (simplified)
mod brotli {
    use alloc::vec;

    use super::*;

    /// Compress data using simplified Brotli-like format
    pub fn compress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Ok(vec![0x06]); // Empty Brotli stream
        }

        let mut output = Vec::with_capacity(input.len() + 16);

        // Brotli stream header
        // WBITS (window size) in first byte
        output.push(BROTLI_WINDOW_BITS);

        // Compress using LZ77 + Huffman-like encoding
        let compressed = compress_meta_block(input);

        // Meta-block header
        let is_last = true;
        let _mnibbles = ((compressed.len() + 1) / 16) + 1;

        // Write meta-block header (simplified)
        let header_byte = if is_last { 0x01 } else { 0x00 };
        output.push(header_byte);

        // Write length
        output.extend_from_slice(&(input.len() as u32).to_le_bytes());

        // Check if compression helped
        if compressed.len() >= input.len() {
            // Uncompressed meta-block
            output.push(0x80); // Uncompressed flag
            output.extend_from_slice(input);
        } else {
            output.push(0x00); // Compressed flag
            output.extend_from_slice(&compressed);
        }

        Ok(output)
    }

    /// Compress a meta-block using LZ77 + simple encoding
    fn compress_meta_block(input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        let mut pos = 0;

        // Simple hash table
        let mut hash_table = [0usize; 8192];

        while pos < input.len() {
            // Look for match
            if pos + 4 <= input.len() {
                let hash = hash4(&input[pos..]) & 0x1FFF;
                let match_pos = hash_table[hash];
                hash_table[hash] = pos;

                if match_pos < pos && pos - match_pos < 65535 {
                    let match_len = find_match(&input[match_pos..], &input[pos..]);

                    if match_len >= 4 {
                        let distance = pos - match_pos;

                        // Write match marker + length + distance
                        if match_len < 16 && distance < 256 {
                            output.push(0xF0 | (match_len as u8 - 4));
                            output.push(distance as u8);
                        } else {
                            output.push(0xFF);
                            output.extend_from_slice(&(match_len as u16).to_le_bytes());
                            output.extend_from_slice(&(distance as u16).to_le_bytes());
                        }

                        pos += match_len;
                        continue;
                    }
                }
            }

            // Write literal
            let byte = input[pos];
            if byte >= 0xF0 {
                output.push(0xFE); // Escape byte
            }
            output.push(byte);
            pos += 1;
        }

        output
    }

    /// Calculate hash of 4 bytes
    fn hash4(data: &[u8]) -> usize {
        if data.len() < 4 {
            return 0;
        }
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        (val.wrapping_mul(0x1E35A7BD) >> 19) as usize
    }

    /// Find match length
    fn find_match(match_data: &[u8], current: &[u8]) -> usize {
        let max_len = core::cmp::min(core::cmp::min(match_data.len(), current.len()), 258);
        let mut len = 0;

        while len < max_len && match_data[len] == current[len] {
            len += 1;
        }

        len
    }

    /// Decompress Brotli format data
    pub fn decompress(input: &[u8]) -> Result<Vec<u8>, String> {
        if input.is_empty() {
            return Err(String::from("Brotli: Empty input"));
        }

        // Check for empty stream
        if input.len() == 1 && input[0] == 0x06 {
            return Ok(Vec::new());
        }

        if input.len() < 7 {
            return Err(String::from("Brotli: Input too short"));
        }

        // Parse header
        let _window_bits = input[0];
        let _header_byte = input[1];

        // Read original length
        let original_len = u32::from_le_bytes([input[2], input[3], input[4], input[5]]) as usize;

        let compressed_flag = input[6];
        let data_start = 7;

        let mut output = Vec::with_capacity(original_len);

        if compressed_flag == 0x80 {
            // Uncompressed
            output.extend_from_slice(&input[data_start..]);
        } else {
            // Decompress
            decompress_meta_block(&input[data_start..], &mut output)?;
        }

        Ok(output)
    }

    /// Decompress a meta-block
    fn decompress_meta_block(input: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
        let mut pos = 0;

        while pos < input.len() {
            let byte = input[pos];
            pos += 1;

            if byte == 0xFE {
                // Escaped literal
                if pos >= input.len() {
                    return Err(String::from("Brotli: Truncated escape"));
                }
                output.push(input[pos]);
                pos += 1;
            } else if byte == 0xFF {
                // Long match
                if pos + 4 > input.len() {
                    return Err(String::from("Brotli: Truncated long match"));
                }
                let match_len = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                let distance = u16::from_le_bytes([input[pos + 2], input[pos + 3]]) as usize;
                pos += 4;

                if distance > output.len() {
                    return Err(String::from("Brotli: Invalid distance"));
                }

                let match_start = output.len() - distance;
                for i in 0..match_len {
                    let b = output[match_start + (i % distance)];
                    output.push(b);
                }
            } else if byte >= 0xF0 {
                // Short match
                let match_len = (byte & 0x0F) as usize + 4;
                if pos >= input.len() {
                    return Err(String::from("Brotli: Truncated short match"));
                }
                let distance = input[pos] as usize;
                pos += 1;

                if distance > output.len() {
                    return Err(String::from("Brotli: Invalid distance"));
                }

                let match_start = output.len() - distance;
                for i in 0..match_len {
                    let b = output[match_start + (i % distance)];
                    output.push(b);
                }
            } else {
                // Literal
                output.push(byte);
            }
        }

        Ok(())
    }
}

// ============================================================================
// Public Compression API
// ============================================================================

/// Decompress data based on compression algorithm
pub fn decompress(data: &[u8], compression: Compression) -> Result<Vec<u8>, String> {
    match compression {
        Compression::None => Ok(data.to_vec()),
        Compression::Zstd => zstd::decompress(data),
        Compression::Lz4 => lz4::decompress(data),
        Compression::Brotli => brotli::decompress(data),
    }
}

/// Compress data using specified algorithm
pub fn compress(data: &[u8], compression: Compression) -> Result<Vec<u8>, String> {
    match compression {
        Compression::None => Ok(data.to_vec()),
        Compression::Zstd => zstd::compress(data),
        Compression::Lz4 => lz4::compress(data),
        Compression::Brotli => brotli::compress(data),
    }
}
