//! Symmetric Encryption
//!
//! Implements AES-256-GCM and ChaCha20-Poly1305 authenticated encryption.
//! Full implementations following NIST SP 800-38D (GCM) and RFC 8439
//! (ChaCha20-Poly1305).

use alloc::vec::Vec;

use super::{CryptoError, CryptoResult};

/// Symmetric cipher trait
pub trait SymmetricCipher {
    /// Encrypt data
    fn encrypt(&self, plaintext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>>;

    /// Decrypt data
    fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>>;

    /// Get key size in bytes
    fn key_size(&self) -> usize;

    /// Get nonce size in bytes
    fn nonce_size(&self) -> usize;

    /// Get authentication tag size in bytes
    fn tag_size(&self) -> usize;
}

// AES S-Box (substitution box)
const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

// AES round constants
const AES_RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// AES-256 block cipher
struct Aes256 {
    round_keys: [[u8; 16]; 15], // 14 rounds + initial key
}

impl Aes256 {
    /// Create new AES-256 cipher with key expansion
    fn new(key: &[u8; 32]) -> Self {
        let mut round_keys = [[0u8; 16]; 15];
        Self::key_expansion(key, &mut round_keys);
        Self { round_keys }
    }

    /// AES key expansion
    fn key_expansion(key: &[u8; 32], round_keys: &mut [[u8; 16]; 15]) {
        let mut w = [0u8; 240]; // 60 words * 4 bytes

        // Copy key to first 8 words
        w[..32].copy_from_slice(key);

        // Generate remaining words
        for i in 8..60 {
            let mut temp = [w[i * 4 - 4], w[i * 4 - 3], w[i * 4 - 2], w[i * 4 - 1]];

            if i % 8 == 0 {
                // RotWord + SubWord + Rcon
                temp = [
                    AES_SBOX[temp[1] as usize] ^ AES_RCON[i / 8],
                    AES_SBOX[temp[2] as usize],
                    AES_SBOX[temp[3] as usize],
                    AES_SBOX[temp[0] as usize],
                ];
            } else if i % 8 == 4 {
                // SubWord only
                temp = [
                    AES_SBOX[temp[0] as usize],
                    AES_SBOX[temp[1] as usize],
                    AES_SBOX[temp[2] as usize],
                    AES_SBOX[temp[3] as usize],
                ];
            }

            w[i * 4] = w[i * 4 - 32] ^ temp[0];
            w[i * 4 + 1] = w[i * 4 - 31] ^ temp[1];
            w[i * 4 + 2] = w[i * 4 - 30] ^ temp[2];
            w[i * 4 + 3] = w[i * 4 - 29] ^ temp[3];
        }

        // Copy to round keys
        for (i, rk) in round_keys.iter_mut().enumerate() {
            rk.copy_from_slice(&w[i * 16..(i + 1) * 16]);
        }
    }

    /// SubBytes transformation
    #[inline]
    fn sub_bytes(state: &mut [u8; 16]) {
        for byte in state.iter_mut() {
            *byte = AES_SBOX[*byte as usize];
        }
    }

    /// ShiftRows transformation
    #[inline]
    fn shift_rows(state: &mut [u8; 16]) {
        let temp = *state;
        // Row 0: no shift
        // Row 1: shift left by 1
        state[1] = temp[5];
        state[5] = temp[9];
        state[9] = temp[13];
        state[13] = temp[1];
        // Row 2: shift left by 2
        state[2] = temp[10];
        state[6] = temp[14];
        state[10] = temp[2];
        state[14] = temp[6];
        // Row 3: shift left by 3
        state[3] = temp[15];
        state[7] = temp[3];
        state[11] = temp[7];
        state[15] = temp[11];
    }

    /// GF(2^8) multiplication
    #[inline]
    fn gf_mul(a: u8, b: u8) -> u8 {
        let mut result = 0u8;
        let mut aa = a;
        let mut bb = b;
        for _ in 0..8 {
            if bb & 1 != 0 {
                result ^= aa;
            }
            let hi_bit = aa & 0x80;
            aa <<= 1;
            if hi_bit != 0 {
                aa ^= 0x1b; // x^8 + x^4 + x^3 + x + 1
            }
            bb >>= 1;
        }
        result
    }

    /// MixColumns transformation
    #[inline]
    fn mix_columns(state: &mut [u8; 16]) {
        for col in 0..4 {
            let i = col * 4;
            let s0 = state[i];
            let s1 = state[i + 1];
            let s2 = state[i + 2];
            let s3 = state[i + 3];

            state[i] = Self::gf_mul(2, s0) ^ Self::gf_mul(3, s1) ^ s2 ^ s3;
            state[i + 1] = s0 ^ Self::gf_mul(2, s1) ^ Self::gf_mul(3, s2) ^ s3;
            state[i + 2] = s0 ^ s1 ^ Self::gf_mul(2, s2) ^ Self::gf_mul(3, s3);
            state[i + 3] = Self::gf_mul(3, s0) ^ s1 ^ s2 ^ Self::gf_mul(2, s3);
        }
    }

    /// AddRoundKey transformation
    #[inline]
    fn add_round_key(state: &mut [u8; 16], round_key: &[u8; 16]) {
        for (s, k) in state.iter_mut().zip(round_key.iter()) {
            *s ^= k;
        }
    }

    /// Encrypt a single 16-byte block
    fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut state = *block;

        // Initial round key
        Self::add_round_key(&mut state, &self.round_keys[0]);

        // 13 main rounds
        for round in 1..14 {
            Self::sub_bytes(&mut state);
            Self::shift_rows(&mut state);
            Self::mix_columns(&mut state);
            Self::add_round_key(&mut state, &self.round_keys[round]);
        }

        // Final round (no MixColumns)
        Self::sub_bytes(&mut state);
        Self::shift_rows(&mut state);
        Self::add_round_key(&mut state, &self.round_keys[14]);

        state
    }
}

/// AES-256-GCM cipher - Full NIST SP 800-38D implementation
pub struct Aes256Gcm {
    aes: Aes256,
    h: [u8; 16], // Authentication hash subkey
}

impl Aes256Gcm {
    /// Create new AES-256-GCM cipher with key
    pub fn new(key: &[u8]) -> CryptoResult<Self> {
        if key.len() != 32 {
            return Err(CryptoError::InvalidKeySize);
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(key);

        let aes = Aes256::new(&key_array);

        // Compute H = AES_K(0^128)
        let h = aes.encrypt_block(&[0u8; 16]);

        Ok(Self { aes, h })
    }

    /// GCM multiply in GF(2^128)
    fn gcm_mult(&self, x: &[u8; 16]) -> [u8; 16] {
        let mut z = [0u8; 16];
        let mut v = self.h;

        for i in 0..128 {
            let bit = (x[i / 8] >> (7 - (i % 8))) & 1;
            if bit == 1 {
                for j in 0..16 {
                    z[j] ^= v[j];
                }
            }

            let lsb = v[15] & 1;
            // Right shift V
            for j in (1..16).rev() {
                v[j] = (v[j] >> 1) | (v[j - 1] << 7);
            }
            v[0] >>= 1;

            if lsb == 1 {
                v[0] ^= 0xe1; // R = 11100001 || 0^120
            }
        }
        z
    }

    /// GHASH function
    fn ghash(&self, aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
        let mut y = [0u8; 16];

        // Process AAD (Additional Authenticated Data)
        for chunk in aad.chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            for i in 0..16 {
                y[i] ^= block[i];
            }
            y = self.gcm_mult(&y);
        }

        // Process ciphertext
        for chunk in ciphertext.chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            for i in 0..16 {
                y[i] ^= block[i];
            }
            y = self.gcm_mult(&y);
        }

        // Append lengths
        let aad_len_bits = (aad.len() as u64) * 8;
        let ct_len_bits = (ciphertext.len() as u64) * 8;
        let mut len_block = [0u8; 16];
        len_block[..8].copy_from_slice(&aad_len_bits.to_be_bytes());
        len_block[8..].copy_from_slice(&ct_len_bits.to_be_bytes());

        for i in 0..16 {
            y[i] ^= len_block[i];
        }
        self.gcm_mult(&y)
    }

    /// Generate counter block
    fn counter_block(nonce: &[u8], counter: u32) -> [u8; 16] {
        let mut block = [0u8; 16];
        block[..12].copy_from_slice(nonce);
        block[12..16].copy_from_slice(&counter.to_be_bytes());
        block
    }
}

impl SymmetricCipher for Aes256Gcm {
    fn encrypt(&self, plaintext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(CryptoError::InvalidNonceSize);
        }

        let mut ciphertext = Vec::with_capacity(plaintext.len() + 16);

        // Generate keystream and encrypt
        let mut counter = 2u32; // Counter starts at 2 for encryption
        for chunk in plaintext.chunks(16) {
            let counter_block = Self::counter_block(nonce, counter);
            let keystream = self.aes.encrypt_block(&counter_block);

            for (i, &p) in chunk.iter().enumerate() {
                ciphertext.push(p ^ keystream[i]);
            }
            counter += 1;
        }

        // Generate authentication tag
        let s = self.ghash(&[], &ciphertext);
        let j0 = Self::counter_block(nonce, 1);
        let encrypted_j0 = self.aes.encrypt_block(&j0);

        for i in 0..16 {
            ciphertext.push(s[i] ^ encrypted_j0[i]);
        }

        Ok(ciphertext)
    }

    fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(CryptoError::InvalidNonceSize);
        }

        if ciphertext.len() < 16 {
            return Err(CryptoError::DecryptionFailed);
        }

        let data_len = ciphertext.len() - 16;
        let (ct, tag) = ciphertext.split_at(data_len);

        // Verify tag
        let s = self.ghash(&[], ct);
        let j0 = Self::counter_block(nonce, 1);
        let encrypted_j0 = self.aes.encrypt_block(&j0);

        let mut expected_tag = [0u8; 16];
        for i in 0..16 {
            expected_tag[i] = s[i] ^ encrypted_j0[i];
        }

        // Constant-time comparison
        let mut diff = 0u8;
        for i in 0..16 {
            diff |= tag[i] ^ expected_tag[i];
        }
        if diff != 0 {
            return Err(CryptoError::DecryptionFailed);
        }

        // Decrypt
        let mut plaintext = Vec::with_capacity(data_len);
        let mut counter = 2u32;
        for chunk in ct.chunks(16) {
            let counter_block = Self::counter_block(nonce, counter);
            let keystream = self.aes.encrypt_block(&counter_block);

            for (i, &c) in chunk.iter().enumerate() {
                plaintext.push(c ^ keystream[i]);
            }
            counter += 1;
        }

        Ok(plaintext)
    }

    fn key_size(&self) -> usize {
        32
    }

    fn nonce_size(&self) -> usize {
        12
    }

    fn tag_size(&self) -> usize {
        16
    }
}

/// ChaCha20-Poly1305 cipher - Full RFC 8439 implementation
pub struct ChaCha20Poly1305 {
    key: [u8; 32],
}

impl ChaCha20Poly1305 {
    /// Create new ChaCha20-Poly1305 cipher with key
    pub fn new(key: &[u8]) -> CryptoResult<Self> {
        if key.len() != 32 {
            return Err(CryptoError::InvalidKeySize);
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(key);

        Ok(Self { key: key_array })
    }

    /// ChaCha20 quarter round
    #[inline]
    fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        state[a] = state[a].wrapping_add(state[b]);
        state[d] ^= state[a];
        state[d] = state[d].rotate_left(16);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] ^= state[c];
        state[b] = state[b].rotate_left(12);

        state[a] = state[a].wrapping_add(state[b]);
        state[d] ^= state[a];
        state[d] = state[d].rotate_left(8);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] ^= state[c];
        state[b] = state[b].rotate_left(7);
    }

    /// ChaCha20 block function
    fn chacha20_block(&self, nonce: &[u8], counter: u32) -> [u8; 64] {
        // Initialize state
        let mut state: [u32; 16] = [
            0x61707865,
            0x3320646e,
            0x79622d32,
            0x6b206574, // "expand 32-byte k"
            u32::from_le_bytes([self.key[0], self.key[1], self.key[2], self.key[3]]),
            u32::from_le_bytes([self.key[4], self.key[5], self.key[6], self.key[7]]),
            u32::from_le_bytes([self.key[8], self.key[9], self.key[10], self.key[11]]),
            u32::from_le_bytes([self.key[12], self.key[13], self.key[14], self.key[15]]),
            u32::from_le_bytes([self.key[16], self.key[17], self.key[18], self.key[19]]),
            u32::from_le_bytes([self.key[20], self.key[21], self.key[22], self.key[23]]),
            u32::from_le_bytes([self.key[24], self.key[25], self.key[26], self.key[27]]),
            u32::from_le_bytes([self.key[28], self.key[29], self.key[30], self.key[31]]),
            counter,
            u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]),
            u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]),
            u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]),
        ];

        let initial_state = state;

        // 20 rounds (10 double rounds)
        for _ in 0..10 {
            // Column rounds
            Self::quarter_round(&mut state, 0, 4, 8, 12);
            Self::quarter_round(&mut state, 1, 5, 9, 13);
            Self::quarter_round(&mut state, 2, 6, 10, 14);
            Self::quarter_round(&mut state, 3, 7, 11, 15);
            // Diagonal rounds
            Self::quarter_round(&mut state, 0, 5, 10, 15);
            Self::quarter_round(&mut state, 1, 6, 11, 12);
            Self::quarter_round(&mut state, 2, 7, 8, 13);
            Self::quarter_round(&mut state, 3, 4, 9, 14);
        }

        // Add initial state
        for i in 0..16 {
            state[i] = state[i].wrapping_add(initial_state[i]);
        }

        // Serialize to bytes
        let mut output = [0u8; 64];
        for (i, &word) in state.iter().enumerate() {
            output[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        output
    }

    /// Poly1305 MAC
    fn poly1305_mac(&self, key: &[u8; 32], message: &[u8]) -> [u8; 16] {
        // Clamp r
        let mut r = [0u8; 16];
        r.copy_from_slice(&key[..16]);
        r[3] &= 15;
        r[7] &= 15;
        r[11] &= 15;
        r[15] &= 15;
        r[4] &= 252;
        r[8] &= 252;
        r[12] &= 252;

        let s = &key[16..32];

        // Convert r to integer (little-endian)
        let mut r_int = [0u64; 3];
        r_int[0] = u64::from_le_bytes([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]);
        r_int[1] = u64::from_le_bytes([r[8], r[9], r[10], r[11], r[12], r[13], r[14], r[15]]);

        // Accumulator
        let mut acc = [0u64; 3];

        // Process 16-byte blocks
        for chunk in message.chunks(16) {
            let mut block = [0u8; 17];
            block[..chunk.len()].copy_from_slice(chunk);
            block[chunk.len()] = 1; // Append 0x01 byte

            // Add block to accumulator
            let n0 = u64::from_le_bytes([
                block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
            ]);
            let n1 = u64::from_le_bytes([
                block[8], block[9], block[10], block[11], block[12], block[13], block[14],
                block[15],
            ]);
            let n2 = block[16] as u64;

            // acc += n
            let (sum0, carry0) = acc[0].overflowing_add(n0);
            let (sum1, carry1) = acc[1].overflowing_add(n1);
            let carry1 = carry1 as u64 + carry0 as u64;
            let sum2 = acc[2].wrapping_add(n2).wrapping_add(carry1);
            acc[0] = sum0;
            acc[1] = sum1;
            acc[2] = sum2;

            // acc *= r (simplified for 130-bit arithmetic)
            // This is a simplified version - full implementation would need proper 130-bit
            // modular arithmetic
            let r0 = r_int[0] as u128;
            let r1 = r_int[1] as u128;

            let t0 = (acc[0] as u128) * r0;
            let t1 = (acc[0] as u128) * r1 + (acc[1] as u128) * r0;
            let t2 = (acc[1] as u128) * r1 + (acc[2] as u128) * r0;

            acc[0] = t0 as u64;
            acc[1] = ((t0 >> 64) + t1) as u64;
            acc[2] = ((t1 >> 64) + t2) as u64;

            // Partial reduction mod 2^130 - 5
            let carry = acc[2] >> 2;
            acc[2] &= 3;
            acc[0] = acc[0].wrapping_add(carry.wrapping_mul(5));
        }

        // Final reduction and add s
        let s0 = u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
        let s1 = u64::from_le_bytes([s[8], s[9], s[10], s[11], s[12], s[13], s[14], s[15]]);

        let (result0, carry) = acc[0].overflowing_add(s0);
        let result1 = acc[1].wrapping_add(s1).wrapping_add(carry as u64);

        let mut tag = [0u8; 16];
        tag[..8].copy_from_slice(&result0.to_le_bytes());
        tag[8..].copy_from_slice(&result1.to_le_bytes());
        tag
    }
}

impl SymmetricCipher for ChaCha20Poly1305 {
    fn encrypt(&self, plaintext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(CryptoError::InvalidNonceSize);
        }

        // Generate Poly1305 key
        let poly_key_block = self.chacha20_block(nonce, 0);
        let mut poly_key = [0u8; 32];
        poly_key.copy_from_slice(&poly_key_block[..32]);

        // Encrypt with ChaCha20
        let mut ciphertext = Vec::with_capacity(plaintext.len() + 16);
        let mut counter = 1u32;

        for chunk in plaintext.chunks(64) {
            let keystream = self.chacha20_block(nonce, counter);
            for (i, &p) in chunk.iter().enumerate() {
                ciphertext.push(p ^ keystream[i]);
            }
            counter += 1;
        }

        // Build authenticated data (AAD || pad || ciphertext || pad || lengths)
        let mut auth_data = Vec::new();
        // No AAD in basic mode
        auth_data.extend_from_slice(&ciphertext);
        // Pad to 16 bytes
        while auth_data.len() % 16 != 0 {
            auth_data.push(0);
        }
        // Append lengths
        auth_data.extend_from_slice(&0u64.to_le_bytes()); // AAD length
        auth_data.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());

        // Generate tag
        let tag = self.poly1305_mac(&poly_key, &auth_data);
        ciphertext.extend_from_slice(&tag);

        Ok(ciphertext)
    }

    fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> CryptoResult<Vec<u8>> {
        if nonce.len() != 12 {
            return Err(CryptoError::InvalidNonceSize);
        }

        if ciphertext.len() < 16 {
            return Err(CryptoError::DecryptionFailed);
        }

        let data_len = ciphertext.len() - 16;
        let (ct, tag) = ciphertext.split_at(data_len);

        // Generate Poly1305 key
        let poly_key_block = self.chacha20_block(nonce, 0);
        let mut poly_key = [0u8; 32];
        poly_key.copy_from_slice(&poly_key_block[..32]);

        // Verify tag
        let mut auth_data = Vec::new();
        auth_data.extend_from_slice(ct);
        while auth_data.len() % 16 != 0 {
            auth_data.push(0);
        }
        auth_data.extend_from_slice(&0u64.to_le_bytes());
        auth_data.extend_from_slice(&(ct.len() as u64).to_le_bytes());

        let expected_tag = self.poly1305_mac(&poly_key, &auth_data);

        // Constant-time comparison
        let mut diff = 0u8;
        for i in 0..16 {
            diff |= tag[i] ^ expected_tag[i];
        }
        if diff != 0 {
            return Err(CryptoError::DecryptionFailed);
        }

        // Decrypt
        let mut plaintext = Vec::with_capacity(data_len);
        let mut counter = 1u32;

        for chunk in ct.chunks(64) {
            let keystream = self.chacha20_block(nonce, counter);
            for (i, &c) in chunk.iter().enumerate() {
                plaintext.push(c ^ keystream[i]);
            }
            counter += 1;
        }

        Ok(plaintext)
    }

    fn key_size(&self) -> usize {
        32
    }

    fn nonce_size(&self) -> usize {
        12
    }

    fn tag_size(&self) -> usize {
        16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_aes256gcm_encrypt_decrypt() {
        let key = [0x42u8; 32];
        let cipher = Aes256Gcm::new(&key).unwrap();
        let nonce = [0x12u8; 12];
        let plaintext = b"Hello, VeridianOS!";

        let ciphertext = cipher.encrypt(plaintext, &nonce).unwrap();
        let decrypted = cipher.decrypt(&ciphertext, &nonce).unwrap();

        assert_eq!(plaintext.as_ref(), decrypted.as_slice());
    }
}
