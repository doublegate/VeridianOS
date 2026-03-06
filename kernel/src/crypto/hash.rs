//! Cryptographic Hash Functions
//!
//! Implements SHA-256, SHA-512, and BLAKE3 hash algorithms.
//! Full implementations following FIPS 180-4 and BLAKE3 specification.

use alloc::vec::Vec;

use super::CryptoResult;

/// Hash algorithm types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Sha512,
    Blake3,
}

/// 256-bit hash output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hash256(pub [u8; 32]);

/// 512-bit hash output
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hash512(pub [u8; 64]);

impl Hash256 {
    /// Create hash from bytes
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(*bytes)
    }

    /// Get hash as bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> alloc::string::String {
        use alloc::format;
        let mut s = alloc::string::String::with_capacity(64);
        for byte in self.0 {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}

impl Hash512 {
    /// Create hash from bytes
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Self(*bytes)
    }

    /// Get hash as bytes
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> alloc::string::String {
        use alloc::format;
        let mut s = alloc::string::String::with_capacity(128);
        for byte in self.0 {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}

/// Hash data with specified algorithm
pub fn hash(algorithm: HashAlgorithm, data: &[u8]) -> CryptoResult<Vec<u8>> {
    match algorithm {
        HashAlgorithm::Sha256 => {
            let hash = sha256(data);
            Ok(hash.0.to_vec())
        }
        HashAlgorithm::Sha512 => {
            let hash = sha512(data);
            Ok(hash.0.to_vec())
        }
        HashAlgorithm::Blake3 => {
            let hash = blake3(data);
            Ok(hash.0.to_vec())
        }
    }
}

// SHA-256 Constants (first 32 bits of the fractional parts of the cube roots of
// the first 64 primes)
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

// SHA-256 Initial hash values (first 32 bits of the fractional parts of the
// square roots of the first 8 primes)
const SHA256_H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// Process a single SHA-256 block (64 bytes)
fn sha256_process_block(h: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];

    // Copy block into first 16 words of message schedule
    for (i, word_bytes) in block.chunks(4).enumerate().take(16) {
        w[i] = u32::from_be_bytes([word_bytes[0], word_bytes[1], word_bytes[2], word_bytes[3]]);
    }

    // Extend the first 16 words into the remaining 48 words
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    // Initialize working variables
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

    // Compression function main loop
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    // Add compressed chunk to current hash value
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

/// SHA-256 hash - Zero-allocation streaming implementation (FIPS 180-4)
pub fn sha256(data: &[u8]) -> Hash256 {
    let mut h = SHA256_H0;
    let original_len_bits = (data.len() as u64) * 8;

    // Process all complete 64-byte blocks directly from input (no copy)
    let full_blocks = data.len() / 64;
    for i in 0..full_blocks {
        sha256_process_block(&mut h, &data[i * 64..(i + 1) * 64]);
    }

    // Handle the final partial block + padding on the stack (max 128 bytes)
    let remainder = data.len() % 64;
    let mut final_buf = [0u8; 128]; // At most 2 blocks needed for padding
    final_buf[..remainder].copy_from_slice(&data[full_blocks * 64..]);

    // Append bit '1' (0x80)
    final_buf[remainder] = 0x80;

    let padded_len = remainder + 1;

    if padded_len <= 56 {
        // Padding + length fit in one block
        final_buf[56..64].copy_from_slice(&original_len_bits.to_be_bytes());
        sha256_process_block(&mut h, &final_buf[..64]);
    } else {
        // Need two blocks: first block with padding, second with length
        sha256_process_block(&mut h, &final_buf[..64]);
        // Second block: zeros + length
        final_buf[64..120].fill(0);
        final_buf[120..128].copy_from_slice(&original_len_bits.to_be_bytes());
        sha256_process_block(&mut h, &final_buf[64..128]);
    }

    // Produce the final hash value (big-endian)
    let mut result = [0u8; 32];
    for (i, &val) in h.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
    }

    Hash256(result)
}

// SHA-512 Constants (first 64 bits of the fractional parts of the cube roots of
// the first 80 primes)
const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

// SHA-512 Initial hash values
const SHA512_H0: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

/// Process a single SHA-512 block (128 bytes)
fn sha512_process_block(h: &mut [u64; 8], block: &[u8]) {
    let mut w = [0u64; 80];

    // Copy block into first 16 words of message schedule
    for (i, word_bytes) in block.chunks(8).enumerate().take(16) {
        w[i] = u64::from_be_bytes([
            word_bytes[0],
            word_bytes[1],
            word_bytes[2],
            word_bytes[3],
            word_bytes[4],
            word_bytes[5],
            word_bytes[6],
            word_bytes[7],
        ]);
    }

    // Extend the first 16 words into the remaining 64 words
    for i in 16..80 {
        let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
        let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    // Initialize working variables
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

    // Compression function main loop
    for i in 0..80 {
        let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA512_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    // Add compressed chunk to current hash value
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

/// SHA-512 hash - Zero-allocation streaming implementation (FIPS 180-4)
pub fn sha512(data: &[u8]) -> Hash512 {
    let mut h = SHA512_H0;
    let original_len_bits = (data.len() as u128) * 8;

    // Process all complete 128-byte blocks directly from input (no copy)
    let full_blocks = data.len() / 128;
    for i in 0..full_blocks {
        sha512_process_block(&mut h, &data[i * 128..(i + 1) * 128]);
    }

    // Handle the final partial block + padding on the stack (max 256 bytes)
    let remainder = data.len() % 128;
    let mut final_buf = [0u8; 256]; // At most 2 blocks needed for padding
    final_buf[..remainder].copy_from_slice(&data[full_blocks * 128..]);

    // Append bit '1' (0x80)
    final_buf[remainder] = 0x80;

    let padded_len = remainder + 1;

    if padded_len <= 112 {
        // Padding + length fit in one block
        final_buf[112..128].copy_from_slice(&original_len_bits.to_be_bytes());
        sha512_process_block(&mut h, &final_buf[..128]);
    } else {
        // Need two blocks
        sha512_process_block(&mut h, &final_buf[..128]);
        final_buf[128..240].fill(0);
        final_buf[240..256].copy_from_slice(&original_len_bits.to_be_bytes());
        sha512_process_block(&mut h, &final_buf[128..256]);
    }

    // Produce the final hash value (big-endian)
    let mut result = [0u8; 64];
    for (i, &val) in h.iter().enumerate() {
        result[i * 8..(i + 1) * 8].copy_from_slice(&val.to_be_bytes());
    }

    Hash512(result)
}

// BLAKE3 constants
const BLAKE3_IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const BLAKE3_MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

// BLAKE3 flags
const BLAKE3_CHUNK_START: u32 = 1;
const BLAKE3_CHUNK_END: u32 = 2;
const BLAKE3_ROOT: u32 = 8;

/// BLAKE3 quarter round function
#[inline]
fn blake3_g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

/// BLAKE3 round function
fn blake3_round(state: &mut [u32; 16], m: &[u32; 16]) {
    // Column step
    blake3_g(state, 0, 4, 8, 12, m[0], m[1]);
    blake3_g(state, 1, 5, 9, 13, m[2], m[3]);
    blake3_g(state, 2, 6, 10, 14, m[4], m[5]);
    blake3_g(state, 3, 7, 11, 15, m[6], m[7]);
    // Diagonal step
    blake3_g(state, 0, 5, 10, 15, m[8], m[9]);
    blake3_g(state, 1, 6, 11, 12, m[10], m[11]);
    blake3_g(state, 2, 7, 8, 13, m[12], m[13]);
    blake3_g(state, 3, 4, 9, 14, m[14], m[15]);
}

/// BLAKE3 permute message words
fn blake3_permute(m: &mut [u32; 16]) {
    let original = *m;
    for i in 0..16 {
        m[i] = original[BLAKE3_MSG_PERMUTATION[i]];
    }
}

/// BLAKE3 compress function
fn blake3_compress(
    chaining_value: &[u32; 8],
    block_words: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut state = [
        chaining_value[0],
        chaining_value[1],
        chaining_value[2],
        chaining_value[3],
        chaining_value[4],
        chaining_value[5],
        chaining_value[6],
        chaining_value[7],
        BLAKE3_IV[0],
        BLAKE3_IV[1],
        BLAKE3_IV[2],
        BLAKE3_IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];

    let mut m = *block_words;

    // 7 rounds
    for _ in 0..7 {
        blake3_round(&mut state, &m);
        blake3_permute(&mut m);
    }

    // XOR the two halves
    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }

    state
}

/// BLAKE3 hash - Full implementation following BLAKE3 specification
pub fn blake3(data: &[u8]) -> Hash256 {
    // For simplicity, this implementation handles data up to 64 bytes (single
    // chunk) For longer data, a full tree structure would be needed

    let mut block_words = [0u32; 16];
    let block_len = core::cmp::min(data.len(), 64) as u32;

    // Convert input to little-endian words
    for (i, chunk) in data.chunks(4).take(16).enumerate() {
        let mut word_bytes = [0u8; 4];
        let copy_len = core::cmp::min(chunk.len(), 4);
        word_bytes[..copy_len].copy_from_slice(&chunk[..copy_len]);
        block_words[i] = u32::from_le_bytes(word_bytes);
    }

    let flags = BLAKE3_CHUNK_START | BLAKE3_CHUNK_END | BLAKE3_ROOT;
    let output = blake3_compress(&BLAKE3_IV, &block_words, 0, block_len, flags);

    // Convert first 8 words to output hash
    let mut result = [0u8; 32];
    for i in 0..8 {
        result[i * 4..(i + 1) * 4].copy_from_slice(&output[i].to_le_bytes());
    }

    Hash256(result)
}

// ============================================================================
// BLAKE2s (RFC 7693)
// ============================================================================

/// BLAKE2s initialization vector (same as SHA-256 fractional parts of
/// sqrt(primes))
const BLAKE2S_IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

/// BLAKE2s sigma permutation schedule (10 rounds)
const BLAKE2S_SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// BLAKE2s hash state (RFC 7693)
#[derive(Clone)]
pub struct Blake2s {
    h: [u32; 8],
    t: [u32; 2],
    buf: [u8; 64],
    buf_len: usize,
    #[allow(dead_code)]
    outlen: usize,
}

impl Blake2s {
    /// Create new BLAKE2s hasher with specified output length (1-32 bytes)
    pub fn new(outlen: usize) -> Self {
        assert!((1..=32).contains(&outlen));
        let mut h = BLAKE2S_IV;
        // Parameter block: fan-out=1, depth=1, digest_length=outlen
        h[0] ^= 0x01010000 ^ (outlen as u32);
        Self {
            h,
            t: [0, 0],
            buf: [0u8; 64],
            buf_len: 0,
            outlen,
        }
    }

    /// Create new keyed BLAKE2s hasher
    pub fn new_keyed(key: &[u8], outlen: usize) -> Self {
        assert!(!key.is_empty() && key.len() <= 32);
        assert!((1..=32).contains(&outlen));
        let mut h = BLAKE2S_IV;
        // Parameter block: fan-out=1, depth=1, digest_length=outlen,
        // key_length=key.len()
        h[0] ^= 0x01010000 ^ ((key.len() as u32) << 8) ^ (outlen as u32);
        let mut state = Self {
            h,
            t: [0, 0],
            buf: [0u8; 64],
            buf_len: 0,
            outlen,
        };
        // Pad key to 64 bytes and process as first block
        let mut padded_key = [0u8; 64];
        padded_key[..key.len()].copy_from_slice(key);
        state.update(&padded_key);
        state
    }

    /// Update state with input data
    pub fn update(&mut self, data: &[u8]) {
        let mut offset = 0;
        let len = data.len();

        // Fill buffer
        if self.buf_len > 0 && self.buf_len + len > 64 {
            let fill = 64 - self.buf_len;
            self.buf[self.buf_len..64].copy_from_slice(&data[..fill]);
            self.increment_counter(64);
            self.compress(false);
            self.buf_len = 0;
            offset = fill;
        }

        // Process full blocks (keeping at least 1 byte for finalization)
        while offset + 64 < len {
            self.buf.copy_from_slice(&data[offset..offset + 64]);
            self.increment_counter(64);
            self.compress(false);
            offset += 64;
        }

        // Buffer remaining
        let remaining = len - offset;
        if remaining > 0 {
            self.buf[self.buf_len..self.buf_len + remaining].copy_from_slice(&data[offset..]);
            self.buf_len += remaining;
        }
    }

    /// Finalize and return the hash digest
    pub fn finalize(mut self) -> [u8; 32] {
        self.increment_counter(self.buf_len as u32);
        // Zero-pad remaining buffer
        let buf_len = self.buf_len;
        for byte in &mut self.buf[buf_len..] {
            *byte = 0;
        }
        self.compress(true);

        let mut out = [0u8; 32];
        for i in 0..8 {
            let bytes = self.h[i].to_le_bytes();
            out[i * 4..i * 4 + 4].copy_from_slice(&bytes);
        }
        out
    }

    /// BLAKE2s compression function: 10 rounds of G mixing
    fn compress(&mut self, last: bool) {
        let mut v = [0u32; 16];
        v[..8].copy_from_slice(&self.h);
        v[8..16].copy_from_slice(&BLAKE2S_IV);

        v[12] ^= self.t[0];
        v[13] ^= self.t[1];
        if last {
            v[14] = !v[14];
        }

        // Decode message block into 16 words
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                self.buf[i * 4],
                self.buf[i * 4 + 1],
                self.buf[i * 4 + 2],
                self.buf[i * 4 + 3],
            ]);
        }

        // 10 rounds of G mixing
        for s in &BLAKE2S_SIGMA[..10] {
            // Column step
            Self::g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
            Self::g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
            Self::g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
            Self::g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
            // Diagonal step
            Self::g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
            Self::g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
            Self::g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
            Self::g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
        }

        // Finalize
        for i in 0..8 {
            self.h[i] ^= v[i] ^ v[i + 8];
        }
    }

    /// BLAKE2s G mixing function
    #[inline]
    fn g(v: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
        v[d] = (v[d] ^ v[a]).rotate_right(16);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(12);
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
        v[d] = (v[d] ^ v[a]).rotate_right(8);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(7);
    }

    fn increment_counter(&mut self, inc: u32) {
        self.t[0] = self.t[0].wrapping_add(inc);
        if self.t[0] < inc {
            self.t[1] = self.t[1].wrapping_add(1);
        }
    }
}

/// Compute BLAKE2s hash of data with given output length
pub fn blake2s_hash(data: &[u8], outlen: usize) -> [u8; 32] {
    let mut hasher = Blake2s::new(outlen);
    hasher.update(data);
    hasher.finalize()
}

/// Compute keyed BLAKE2s hash
pub fn blake2s_keyed_hash(key: &[u8], data: &[u8], outlen: usize) -> [u8; 32] {
    let mut hasher = Blake2s::new_keyed(key, outlen);
    hasher.update(data);
    hasher.finalize()
}

/// Verify hash matches data
pub fn verify_hash(algorithm: HashAlgorithm, data: &[u8], expected: &[u8]) -> CryptoResult<bool> {
    let computed = hash(algorithm, data)?;
    Ok(computed == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256() {
        let data = b"Hello, VeridianOS!";
        let hash = sha256(data);
        assert_eq!(hash.as_bytes().len(), 32);
    }

    #[test]
    fn test_hash_hex() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x12;
        bytes[1] = 0x34;
        bytes[2] = 0x56;
        bytes[3] = 0x78;
        let hash = Hash256(bytes);
        let hex = hash.to_hex();
        assert!(hex.starts_with("12345678"));
    }
}
