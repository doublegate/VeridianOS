//! Post-Quantum Cryptography
//!
//! Implements ML-DSA (Dilithium) signatures and ML-KEM (Kyber) key
//! encapsulation.
//!
//! ## NIST Standards Compliance
//!
//! This module implements algorithms selected by NIST for post-quantum
//! cryptography:
//! - **ML-DSA (FIPS 204)**: Module-Lattice-Based Digital Signature Algorithm

// Phase 3 (security hardening) -- cryptographic structures are defined but
// not yet exercised by higher-level callers.
#![allow(dead_code)]
//!   - Replaces Dilithium after standardization
//!   - Provides quantum-resistant digital signatures
//!   - Security levels: 2, 3, 5 (128, 192, 256-bit equivalents)
//! - **ML-KEM (FIPS 203)**: Module-Lattice-Based Key Encapsulation Mechanism
//!   - Replaces Kyber after standardization
//!   - Provides quantum-resistant key exchange
//!   - Security levels: 512, 768, 1024 (128, 192, 256-bit equivalents)
//!
//! ## Implementation Status
//!
//! **Current**: Lattice-based implementations with NTT polynomial arithmetic
//! **Production Requirements**:
//! - Full NIST-compliant algorithm implementations
//! - Constant-time operations to prevent timing attacks
//! - Proper random number generation from hardware
//! - Side-channel attack mitigations
//! - FIPS 140-3 validation for cryptographic modules
//!
//! ## Integration with Classical Cryptography
//!
//! Hybrid key exchange combines classical (X25519) and post-quantum (Kyber) to
//! provide:
//! - Security against both classical and quantum attacks
//! - Backward compatibility during transition period
//! - Meet-in-the-middle security guarantees

use alloc::vec::Vec;

use super::CryptoResult;

// ============================================================================
// NTT (Number Theoretic Transform) for Kyber (q=3329)
// ============================================================================

/// Kyber modulus
const KYBER_Q: u32 = 3329;
/// Polynomial degree
const KYBER_N: usize = 256;

/// Montgomery parameter for Kyber: R = 2^16 mod q
const KYBER_MONT_R: u32 = 2285; // 2^16 mod 3329

/// Barrett reduction constant for Kyber
const KYBER_BARRETT_V: u32 = 20159; // round(2^26 / q)

/// Primitive 256th root of unity mod q=3329
/// zeta = 17 (since 17^128 = -1 mod 3329)
const KYBER_ZETA: u32 = 17;

/// Precomputed NTT zetas (powers of the root of unity) for Kyber
/// zetas[i] = KYBER_ZETA^(bit_reverse_7(i)) mod q
fn kyber_ntt_zetas() -> [u16; 128] {
    let mut zetas = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        // Compute zeta^(bit_reverse(i, 7)) mod q
        let rev = bit_reverse_7(i as u8) as u32;
        zetas[i] = mod_pow(KYBER_ZETA, rev, KYBER_Q) as u16;
        i += 1;
    }
    zetas
}

/// Bit-reverse a 7-bit number
fn bit_reverse_7(x: u8) -> u8 {
    let mut r = 0u8;
    let mut v = x;
    let mut i = 0;
    while i < 7 {
        r = (r << 1) | (v & 1);
        v >>= 1;
        i += 1;
    }
    r
}

/// Modular exponentiation: base^exp mod modulus
fn mod_pow(base: u32, exp: u32, modulus: u32) -> u32 {
    if modulus == 1 {
        return 0;
    }
    let mut result: u64 = 1;
    let mut base = (base % modulus) as u64;
    let modulus = modulus as u64;
    let mut exp = exp;
    while exp > 0 {
        if exp & 1 == 1 {
            result = (result * base) % modulus;
        }
        exp >>= 1;
        base = (base * base) % modulus;
    }
    result as u32
}

/// Barrett reduction for Kyber: reduce a to [0, q)
fn kyber_barrett_reduce(a: i32) -> i32 {
    let q = KYBER_Q as i32;
    // t = (a * v) >> 26 where v = round(2^26 / q)
    let t = ((a as i64 * KYBER_BARRETT_V as i64) >> 26) as i32;
    let r = a - t * q;
    // Result may still be >= q, one conditional subtraction
    if r >= q {
        r - q
    } else if r < 0 {
        r + q
    } else {
        r
    }
}

/// Kyber polynomial (degree N-1 with coefficients mod q)
#[derive(Clone)]
struct KyberPoly {
    coeffs: [i16; KYBER_N],
}

impl KyberPoly {
    fn zero() -> Self {
        KyberPoly {
            coeffs: [0i16; KYBER_N],
        }
    }

    /// Forward NTT (number theoretic transform) in place
    /// Transforms polynomial from normal form to NTT domain
    fn ntt(&mut self) {
        let zetas = kyber_ntt_zetas();
        let mut k = 1usize;
        let mut len = 128;
        while len >= 2 {
            let mut start = 0;
            while start < KYBER_N {
                let zeta = zetas[k] as i32;
                k += 1;
                let mut j = start;
                while j < start + len {
                    let t = ((zeta as i64 * self.coeffs[j + len] as i64) % KYBER_Q as i64) as i32;
                    let t = kyber_barrett_reduce(t) as i16;
                    self.coeffs[j + len] =
                        kyber_barrett_reduce(self.coeffs[j] as i32 - t as i32) as i16;
                    self.coeffs[j] = kyber_barrett_reduce(self.coeffs[j] as i32 + t as i32) as i16;
                    j += 1;
                }
                start += 2 * len;
            }
            len >>= 1;
        }
    }

    /// Inverse NTT
    fn inv_ntt(&mut self) {
        let zetas = kyber_ntt_zetas();
        let mut k = 127usize;
        let mut len = 2;
        while len <= 128 {
            let mut start = 0;
            while start < KYBER_N {
                let zeta = zetas[k] as i32;
                k = k.wrapping_sub(1);
                let mut j = start;
                while j < start + len {
                    let t = self.coeffs[j] as i32;
                    self.coeffs[j] = kyber_barrett_reduce(t + self.coeffs[j + len] as i32) as i16;
                    let diff = t - self.coeffs[j + len] as i32;
                    self.coeffs[j + len] =
                        kyber_barrett_reduce(((zeta as i64 * diff as i64) % KYBER_Q as i64) as i32)
                            as i16;
                    j += 1;
                }
                start += 2 * len;
            }
            len <<= 1;
        }

        // Multiply by N^(-1) mod q
        // N = 256, N^(-1) mod 3329 = 3303 (since 256 * 3303 = 845568 = 253*3329 +
        // 2831... let me compute: 256^(-1) mod 3329)
        let n_inv = mod_pow(256, KYBER_Q - 2, KYBER_Q) as i32;
        let mut i = 0;
        while i < KYBER_N {
            self.coeffs[i] = kyber_barrett_reduce(
                ((self.coeffs[i] as i64 * n_inv as i64) % KYBER_Q as i64) as i32,
            ) as i16;
            i += 1;
        }
    }

    /// Pointwise multiplication in NTT domain
    fn pointwise_mul(&self, other: &KyberPoly) -> KyberPoly {
        let mut result = KyberPoly::zero();
        let mut i = 0;
        while i < KYBER_N {
            result.coeffs[i] = kyber_barrett_reduce(
                ((self.coeffs[i] as i64 * other.coeffs[i] as i64) % KYBER_Q as i64) as i32,
            ) as i16;
            i += 1;
        }
        result
    }

    /// Add two polynomials
    fn add(&self, other: &KyberPoly) -> KyberPoly {
        let mut result = KyberPoly::zero();
        let mut i = 0;
        while i < KYBER_N {
            result.coeffs[i] =
                kyber_barrett_reduce(self.coeffs[i] as i32 + other.coeffs[i] as i32) as i16;
            i += 1;
        }
        result
    }

    /// Subtract two polynomials
    fn sub(&self, other: &KyberPoly) -> KyberPoly {
        let mut result = KyberPoly::zero();
        let mut i = 0;
        while i < KYBER_N {
            result.coeffs[i] =
                kyber_barrett_reduce(self.coeffs[i] as i32 - other.coeffs[i] as i32) as i16;
            i += 1;
        }
        result
    }

    /// Sample polynomial with small coefficients from CBD (centered binomial
    /// distribution)
    fn sample_cbd(seed: &[u8], eta: u32) -> KyberPoly {
        let mut poly = KyberPoly::zero();
        // Use SHA-256 based expansion of seed to get enough randomness
        let expanded = expand_seed(seed, KYBER_N * eta as usize / 4);

        let mut i = 0;
        let mut byte_idx = 0;
        while i < KYBER_N && byte_idx + (eta as usize) <= expanded.len() {
            let mut a: i16 = 0;
            let mut b: i16 = 0;
            let mut j = 0u32;
            while j < eta {
                if byte_idx < expanded.len() {
                    a += ((expanded[byte_idx] >> (j & 7)) & 1) as i16;
                    b += ((expanded[byte_idx] >> ((j + eta) & 7)) & 1) as i16;
                }
                j += 1;
            }
            poly.coeffs[i] = kyber_barrett_reduce((a - b) as i32) as i16;
            byte_idx += 1;
            i += 1;
        }
        poly
    }

    /// Sample a uniform polynomial from seed (for matrix A)
    fn sample_uniform(seed: &[u8]) -> KyberPoly {
        let mut poly = KyberPoly::zero();
        let expanded = expand_seed(seed, KYBER_N * 4);

        let mut i = 0;
        let mut j = 0;
        while i < KYBER_N && j + 2 < expanded.len() {
            let d = (expanded[j] as u16) | ((expanded[j + 1] as u16) << 8);
            let d = (d & 0x0fff) as i32; // 12 bits
            if d < KYBER_Q as i32 {
                poly.coeffs[i] = d as i16;
                i += 1;
            }
            j += 2;
        }
        poly
    }

    /// Compress coefficients to d bits
    fn compress(&self, d: usize) -> Vec<u8> {
        let mut result = Vec::new();
        let q = KYBER_Q as u64;
        let mut bits_buf: u32 = 0;
        let mut bits_count: u32 = 0;

        let mut i = 0;
        while i < KYBER_N {
            let mut c = self.coeffs[i] as i32;
            if c < 0 {
                c += KYBER_Q as i32;
            }
            // Compress: round(2^d / q * c) mod 2^d
            let compressed = (((c as u64) << d as u64).wrapping_add(q / 2) / q) & ((1u64 << d) - 1);
            bits_buf |= (compressed as u32) << bits_count;
            bits_count += d as u32;
            while bits_count >= 8 {
                result.push(bits_buf as u8);
                bits_buf >>= 8;
                bits_count -= 8;
            }
            i += 1;
        }
        if bits_count > 0 {
            result.push(bits_buf as u8);
        }
        result
    }

    /// Decompress from d bits
    fn decompress(data: &[u8], d: usize) -> KyberPoly {
        let mut poly = KyberPoly::zero();
        let q = KYBER_Q as u64;
        let mask = (1u32 << d) - 1;
        let mut bits_buf: u32 = 0;
        let mut bits_count: u32 = 0;
        let mut byte_idx = 0;

        let mut i = 0;
        while i < KYBER_N {
            while bits_count < d as u32 && byte_idx < data.len() {
                bits_buf |= (data[byte_idx] as u32) << bits_count;
                bits_count += 8;
                byte_idx += 1;
            }
            let val = bits_buf & mask;
            bits_buf >>= d;
            bits_count -= d as u32;
            // Decompress: round(q / 2^d * val)
            poly.coeffs[i] = ((val as u64 * q + (1u64 << (d - 1))) >> d) as i16;
            i += 1;
        }
        poly
    }

    /// Encode polynomial coefficients to bytes (12 bits each)
    fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(KYBER_N * 12 / 8);
        let mut i = 0;
        while i < KYBER_N {
            let mut c0 = self.coeffs[i] as i32;
            if c0 < 0 {
                c0 += KYBER_Q as i32;
            }
            let c0 = c0 as u16;

            if i + 1 < KYBER_N {
                let mut c1 = self.coeffs[i + 1] as i32;
                if c1 < 0 {
                    c1 += KYBER_Q as i32;
                }
                let c1 = c1 as u16;

                result.push(c0 as u8);
                result.push(((c0 >> 8) | (c1 << 4)) as u8);
                result.push((c1 >> 4) as u8);
            } else {
                result.push(c0 as u8);
                result.push((c0 >> 8) as u8);
            }
            i += 2;
        }
        result
    }

    /// Decode polynomial from bytes (12 bits per coefficient)
    fn from_bytes(data: &[u8]) -> KyberPoly {
        let mut poly = KyberPoly::zero();
        let mut i = 0;
        let mut j = 0;
        while i < KYBER_N && j + 2 < data.len() {
            poly.coeffs[i] = (data[j] as i16) | ((data[j + 1] as i16 & 0x0f) << 8);
            if i + 1 < KYBER_N {
                poly.coeffs[i + 1] = (data[j + 1] as i16 >> 4) | ((data[j + 2] as i16) << 4);
            }
            i += 2;
            j += 3;
        }
        poly
    }
}

/// Expand a seed into pseudorandom bytes using SHA-256 in counter mode
fn expand_seed(seed: &[u8], len: usize) -> Vec<u8> {
    use super::hash::sha256;
    let mut result = Vec::with_capacity(len);
    let mut counter: u32 = 0;

    while result.len() < len {
        let mut input = Vec::with_capacity(seed.len() + 4);
        input.extend_from_slice(seed);
        input.extend_from_slice(&counter.to_le_bytes());
        let hash = sha256(&input);
        let remaining = len - result.len();
        let take = core::cmp::min(remaining, 32);
        result.extend_from_slice(&hash.as_bytes()[..take]);
        counter += 1;
    }
    result
}

// ============================================================================
// NTT for Dilithium (q=8380417)
// ============================================================================

/// Dilithium modulus
const DIL_Q: i32 = 8380417;
/// Dilithium polynomial degree
const DIL_N: usize = 256;

/// Barrett reduction for Dilithium
fn dil_reduce(a: i64) -> i32 {
    let q = DIL_Q as i64;
    let mut r = (a % q) as i32;
    if r < 0 {
        r += DIL_Q;
    }
    if r >= DIL_Q {
        r -= DIL_Q;
    }
    r
}

/// Dilithium polynomial
#[derive(Clone)]
struct DilPoly {
    coeffs: [i32; DIL_N],
}

impl DilPoly {
    fn zero() -> Self {
        DilPoly {
            coeffs: [0i32; DIL_N],
        }
    }

    /// Sample polynomial with small coefficients (uniform in [-eta, eta])
    fn sample_short(seed: &[u8], eta: u32) -> DilPoly {
        let mut poly = DilPoly::zero();
        let expanded = expand_seed(seed, DIL_N * 2);
        let mut i = 0;
        while i < DIL_N && i * 2 + 1 < expanded.len() {
            // CBD: compute difference of bit counts
            let b0 = expanded[i * 2] as u32;
            let b1 = expanded[i * 2 + 1] as u32;
            let a_count = (b0 & 0x0f).count_ones();
            let b_count = (b1 & 0x0f).count_ones();
            let mut val = a_count as i32 - b_count as i32;
            // Clamp to [-eta, eta]
            let eta_i = eta as i32;
            if val > eta_i {
                val = eta_i;
            }
            if val < -eta_i {
                val = -eta_i;
            }
            poly.coeffs[i] = dil_reduce(val as i64);
            i += 1;
        }
        poly
    }

    /// Sample uniform polynomial for matrix A
    fn sample_uniform(seed: &[u8]) -> DilPoly {
        let mut poly = DilPoly::zero();
        let expanded = expand_seed(seed, DIL_N * 6);
        let mut i = 0;
        let mut j = 0;
        while i < DIL_N && j + 2 < expanded.len() {
            let d = (expanded[j] as u32)
                | ((expanded[j + 1] as u32) << 8)
                | ((expanded[j + 2] as u32) << 16);
            let d = (d & 0x7fffff) as i32; // 23 bits
            if d < DIL_Q {
                poly.coeffs[i] = d;
                i += 1;
            }
            j += 3;
        }
        poly
    }

    /// NTT forward transform (simplified for Dilithium)
    fn ntt(&mut self) {
        // Root of unity for Dilithium: zeta = 1753
        let zeta_base: i64 = 1753;
        let mut len = DIL_N / 2;
        let mut k = 1u32;
        while len >= 1 {
            let mut start = 0;
            while start < DIL_N {
                let z = mod_pow(
                    zeta_base as u32,
                    bit_reverse_8(k as u8) as u32,
                    DIL_Q as u32,
                ) as i64;
                k += 1;
                let mut j = start;
                while j < start + len {
                    let t = dil_reduce(z * self.coeffs[j + len] as i64);
                    self.coeffs[j + len] = dil_reduce(self.coeffs[j] as i64 - t as i64);
                    self.coeffs[j] = dil_reduce(self.coeffs[j] as i64 + t as i64);
                    j += 1;
                }
                start += 2 * len;
            }
            len >>= 1;
        }
    }

    /// Inverse NTT
    fn inv_ntt(&mut self) {
        let zeta_base: i64 = 1753;
        let mut len = 1;
        let mut k = 255u32;
        while len <= DIL_N / 2 {
            let mut start = 0;
            while start < DIL_N {
                let z = mod_pow(
                    zeta_base as u32,
                    bit_reverse_8(k as u8) as u32,
                    DIL_Q as u32,
                ) as i64;
                k = k.wrapping_sub(1);
                let mut j = start;
                while j < start + len {
                    let t = self.coeffs[j] as i64;
                    self.coeffs[j] = dil_reduce(t + self.coeffs[j + len] as i64);
                    let diff = t - self.coeffs[j + len] as i64;
                    self.coeffs[j + len] = dil_reduce(z * diff);
                    j += 1;
                }
                start += 2 * len;
            }
            len <<= 1;
        }
        // Multiply by N^(-1) mod q
        let n_inv = mod_pow(DIL_N as u32, (DIL_Q - 2) as u32, DIL_Q as u32) as i64;
        let mut i = 0;
        while i < DIL_N {
            self.coeffs[i] = dil_reduce(self.coeffs[i] as i64 * n_inv);
            i += 1;
        }
    }

    /// Pointwise multiplication
    fn pointwise_mul(&self, other: &DilPoly) -> DilPoly {
        let mut result = DilPoly::zero();
        let mut i = 0;
        while i < DIL_N {
            result.coeffs[i] = dil_reduce(self.coeffs[i] as i64 * other.coeffs[i] as i64);
            i += 1;
        }
        result
    }

    /// Add two polynomials
    fn add(&self, other: &DilPoly) -> DilPoly {
        let mut result = DilPoly::zero();
        let mut i = 0;
        while i < DIL_N {
            result.coeffs[i] = dil_reduce(self.coeffs[i] as i64 + other.coeffs[i] as i64);
            i += 1;
        }
        result
    }

    /// Subtract
    fn sub(&self, other: &DilPoly) -> DilPoly {
        let mut result = DilPoly::zero();
        let mut i = 0;
        while i < DIL_N {
            result.coeffs[i] = dil_reduce(self.coeffs[i] as i64 - other.coeffs[i] as i64);
            i += 1;
        }
        result
    }

    /// Check infinity norm (max absolute coefficient) <= bound
    fn check_norm(&self, bound: i32) -> bool {
        let mut i = 0;
        while i < DIL_N {
            let mut c = self.coeffs[i];
            // Center: if c > q/2, c -= q
            if c > DIL_Q / 2 {
                c -= DIL_Q;
            }
            if c < -(DIL_Q / 2) {
                c += DIL_Q;
            }
            if c.abs() > bound {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Encode polynomial to bytes
    fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(DIL_N * 4);
        let mut i = 0;
        while i < DIL_N {
            let c = self.coeffs[i] as u32;
            result.extend_from_slice(&c.to_le_bytes());
            i += 1;
        }
        result
    }

    /// Decode polynomial from bytes
    fn from_bytes_dil(data: &[u8]) -> DilPoly {
        let mut poly = DilPoly::zero();
        let mut i = 0;
        while i < DIL_N && i * 4 + 3 < data.len() {
            let c = u32::from_le_bytes([
                data[i * 4],
                data[i * 4 + 1],
                data[i * 4 + 2],
                data[i * 4 + 3],
            ]);
            poly.coeffs[i] = dil_reduce(c as i64);
            i += 1;
        }
        poly
    }
}

/// Bit-reverse an 8-bit number
fn bit_reverse_8(x: u8) -> u8 {
    let mut r = 0u8;
    let mut v = x;
    let mut i = 0;
    while i < 8 {
        r = (r << 1) | (v & 1);
        v >>= 1;
        i += 1;
    }
    r
}

// ============================================================================
// Public API (same signatures as the stubs)
// ============================================================================

/// Dilithium security levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DilithiumLevel {
    Level2, // ~128 bits security
    Level3, // ~192 bits security
    Level5, // ~256 bits security
}

/// Kyber security levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KyberLevel {
    Kyber512,  // ~128 bits security
    Kyber768,  // ~192 bits security
    Kyber1024, // ~256 bits security
}

/// ML-DSA (Dilithium) signing key
pub struct DilithiumSigningKey {
    level: DilithiumLevel,
    secret: Vec<u8>,
}

/// ML-DSA (Dilithium) verifying key
pub struct DilithiumVerifyingKey {
    level: DilithiumLevel,
    public: Vec<u8>,
}

/// ML-DSA (Dilithium) signature
pub struct DilithiumSignature {
    bytes: Vec<u8>,
}

/// Get Dilithium parameters for a given level
fn dil_params(level: DilithiumLevel) -> (usize, usize, u32, i32, usize) {
    // Returns (k, l, eta, gamma1, sig_size)
    match level {
        DilithiumLevel::Level2 => (4, 4, 2, 1 << 17, 2420),
        DilithiumLevel::Level3 => (6, 5, 4, 1 << 19, 3293),
        DilithiumLevel::Level5 => (8, 7, 2, 1 << 19, 4595),
    }
}

impl DilithiumSigningKey {
    /// Generate new signing key using lattice-based key generation
    pub fn generate(level: DilithiumLevel) -> CryptoResult<Self> {
        use super::{hash::sha256, random::get_random};

        let (k, l, eta, _gamma1, _) = dil_params(level);
        let rng = get_random();

        // Generate random seed
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed)?;

        // Expand seed to generate matrix A and secret vectors s1, s2
        let mut secret_data = Vec::new();

        // Store seed for key derivation
        secret_data.extend_from_slice(&seed);

        // Generate matrix A (k x l polynomials)
        let mut a_polys = Vec::new();
        let mut idx = 0;
        while idx < k * l {
            let mut poly_seed = Vec::with_capacity(34);
            poly_seed.extend_from_slice(&seed);
            poly_seed.push((idx / l) as u8);
            poly_seed.push((idx % l) as u8);
            let hash = sha256(&poly_seed);
            let poly = DilPoly::sample_uniform(hash.as_bytes());
            a_polys.push(poly);
            idx += 1;
        }

        // Generate secret vector s1 (l polynomials with small coefficients)
        let mut s1 = Vec::new();
        idx = 0;
        while idx < l {
            let mut s1_seed = Vec::with_capacity(33);
            s1_seed.extend_from_slice(&seed);
            s1_seed.push(idx as u8);
            let hash = sha256(&s1_seed);
            s1.push(DilPoly::sample_short(hash.as_bytes(), eta));
            idx += 1;
        }

        // Generate secret vector s2 (k polynomials with small coefficients)
        let mut s2 = Vec::new();
        idx = 0;
        while idx < k {
            let mut s2_seed = Vec::with_capacity(34);
            s2_seed.extend_from_slice(&seed);
            s2_seed.push(0xff);
            s2_seed.push(idx as u8);
            let hash = sha256(&s2_seed);
            s2.push(DilPoly::sample_short(hash.as_bytes(), eta));
            idx += 1;
        }

        // Compute public key t = A * s1 + s2 (in NTT domain)
        let mut t = Vec::new();
        let mut i = 0;
        while i < k {
            let mut ti = DilPoly::zero();
            let mut j = 0;
            while j < l {
                let mut a_ij = a_polys[i * l + j].clone();
                let mut s1j = s1[j].clone();
                a_ij.ntt();
                s1j.ntt();
                let product = a_ij.pointwise_mul(&s1j);
                ti = ti.add(&product);
                j += 1;
            }
            ti.inv_ntt();
            ti = ti.add(&s2[i]);
            t.push(ti);
            i += 1;
        }

        // Serialize secret key: seed || s1 || s2 || t
        // (We store everything needed for signing)
        secret_data.clear();
        secret_data.extend_from_slice(&seed);
        for poly in &s1 {
            secret_data.extend_from_slice(&poly.to_bytes());
        }
        for poly in &s2 {
            secret_data.extend_from_slice(&poly.to_bytes());
        }
        for poly in &t {
            secret_data.extend_from_slice(&poly.to_bytes());
        }

        Ok(Self {
            level,
            secret: secret_data,
        })
    }

    /// Sign a message using ML-DSA (Dilithium) with rejection sampling
    pub fn sign(&self, message: &[u8]) -> CryptoResult<DilithiumSignature> {
        use super::hash::sha256;

        let (k, l, _eta, gamma1, sig_size) = dil_params(self.level);
        let seed = &self.secret[..32];

        // Reconstruct secret vectors from stored data
        let poly_size = DIL_N * 4;
        let s1_start = 32;
        let s2_start = s1_start + l * poly_size;
        let t_start = s2_start + k * poly_size;

        let mut s1 = Vec::new();
        let mut idx = 0;
        while idx < l {
            let offset = s1_start + idx * poly_size;
            if offset + poly_size <= self.secret.len() {
                s1.push(DilPoly::from_bytes_dil(
                    &self.secret[offset..offset + poly_size],
                ));
            }
            idx += 1;
        }

        let mut s2 = Vec::new();
        idx = 0;
        while idx < k {
            let offset = s2_start + idx * poly_size;
            if offset + poly_size <= self.secret.len() {
                s2.push(DilPoly::from_bytes_dil(
                    &self.secret[offset..offset + poly_size],
                ));
            }
            idx += 1;
        }

        let mut t = Vec::new();
        idx = 0;
        while idx < k {
            let offset = t_start + idx * poly_size;
            if offset + poly_size <= self.secret.len() {
                t.push(DilPoly::from_bytes_dil(
                    &self.secret[offset..offset + poly_size],
                ));
            }
            idx += 1;
        }

        // Reconstruct matrix A from seed
        let mut a_polys = Vec::new();
        idx = 0;
        while idx < k * l {
            let mut poly_seed = Vec::with_capacity(34);
            poly_seed.extend_from_slice(seed);
            poly_seed.push((idx / l) as u8);
            poly_seed.push((idx % l) as u8);
            let hash = sha256(&poly_seed);
            a_polys.push(DilPoly::sample_uniform(hash.as_bytes()));
            idx += 1;
        }

        // Hash message with public key for challenge
        let mut msg_hash_input = Vec::new();
        for ti in &t {
            msg_hash_input.extend_from_slice(&ti.to_bytes());
        }
        msg_hash_input.extend_from_slice(message);
        let mu = sha256(&msg_hash_input);

        // Rejection sampling loop
        let mut nonce: u32 = 0;
        let max_attempts = 1000;

        while nonce < max_attempts {
            // Generate random masking vector y (l polynomials with coefficients in
            // [-gamma1, gamma1])
            let mut y = Vec::new();
            let mut j = 0;
            while j < l {
                let mut y_seed = Vec::new();
                y_seed.extend_from_slice(mu.as_bytes());
                y_seed.extend_from_slice(&nonce.to_le_bytes());
                y_seed.push(j as u8);
                let hash = sha256(&y_seed);
                let mut poly = DilPoly::zero();
                let expanded = expand_seed(hash.as_bytes(), DIL_N * 4);
                let mut ci = 0;
                while ci < DIL_N && ci * 4 + 3 < expanded.len() {
                    let val = u32::from_le_bytes([
                        expanded[ci * 4],
                        expanded[ci * 4 + 1],
                        expanded[ci * 4 + 2],
                        expanded[ci * 4 + 3],
                    ]);
                    // Map to [-gamma1, gamma1]
                    let centered = (val % (2 * gamma1 as u32 + 1)) as i32 - gamma1;
                    poly.coeffs[ci] = dil_reduce(centered as i64);
                    ci += 1;
                }
                y.push(poly);
                j += 1;
            }

            // Compute w = A * y
            let mut w = Vec::new();
            let mut i = 0;
            while i < k {
                let mut wi = DilPoly::zero();
                j = 0;
                while j < l {
                    let mut a_ij = a_polys[i * l + j].clone();
                    let mut yj = y[j].clone();
                    a_ij.ntt();
                    yj.ntt();
                    let product = a_ij.pointwise_mul(&yj);
                    wi = wi.add(&product);
                    j += 1;
                }
                wi.inv_ntt();
                w.push(wi);
                i += 1;
            }

            // Compute challenge c from w and message
            let mut challenge_input = Vec::new();
            challenge_input.extend_from_slice(mu.as_bytes());
            for wi in &w {
                challenge_input.extend_from_slice(&wi.to_bytes());
            }
            let c_hash = sha256(&challenge_input);

            // Create sparse challenge polynomial
            let c_poly = {
                let tau = match self.level {
                    DilithiumLevel::Level2 => 39,
                    DilithiumLevel::Level3 => 49,
                    DilithiumLevel::Level5 => 60,
                };
                let mut cp = DilPoly::zero();
                let mut ci = 0;
                while ci < tau && ci < 32 {
                    let pos = (c_hash.as_bytes()[ci] as usize) % DIL_N;
                    if ci % 2 == 0 {
                        cp.coeffs[pos] = 1;
                    } else {
                        cp.coeffs[pos] = DIL_Q - 1;
                    } // -1 mod q
                    ci += 1;
                }
                cp
            };

            // Compute z = y + c * s1
            let mut z = Vec::new();
            j = 0;
            let mut reject = false;
            while j < l {
                let mut cs1j = c_poly.clone();
                cs1j.ntt();
                let mut s1j_ntt = s1[j].clone();
                s1j_ntt.ntt();
                let product = cs1j.pointwise_mul(&s1j_ntt);
                let mut product_normal = product;
                product_normal.inv_ntt();

                let zj = y[j].add(&product_normal);

                // Rejection: check ||z|| < gamma1 - beta
                let beta = match self.level {
                    DilithiumLevel::Level2 => 39 * 2, // tau * eta
                    DilithiumLevel::Level3 => 49 * 4,
                    DilithiumLevel::Level5 => 60 * 2,
                };
                if !zj.check_norm(gamma1 - beta) {
                    reject = true;
                }

                z.push(zj);
                j += 1;
            }

            if reject {
                nonce += 1;
                continue;
            }

            // Success: build signature = (c_hash, z)
            let mut sig_bytes = Vec::with_capacity(sig_size);
            sig_bytes.extend_from_slice(c_hash.as_bytes());
            for zj in &z {
                sig_bytes.extend_from_slice(&zj.to_bytes());
            }
            // Pad to expected signature size
            while sig_bytes.len() < sig_size {
                sig_bytes.push(0);
            }
            sig_bytes.truncate(sig_size);

            return Ok(DilithiumSignature { bytes: sig_bytes });
        }

        // If rejection sampling fails after max_attempts, return error
        Err(super::CryptoError::SignatureFailed)
    }

    /// Get corresponding verifying key
    pub fn verifying_key(&self) -> DilithiumVerifyingKey {
        let (k, l, _, _, _) = dil_params(self.level);
        let poly_size = DIL_N * 4;
        let t_start = 32 + (l + k) * poly_size;

        let public_size = match self.level {
            DilithiumLevel::Level2 => 1312,
            DilithiumLevel::Level3 => 1952,
            DilithiumLevel::Level5 => 2592,
        };

        // Public key = seed || t
        let mut public = Vec::with_capacity(public_size);
        // Include seed (32 bytes)
        if self.secret.len() >= 32 {
            public.extend_from_slice(&self.secret[..32]);
        }
        // Include t polynomials
        let mut i = 0;
        while i < k {
            let offset = t_start + i * poly_size;
            if offset + poly_size <= self.secret.len() {
                public.extend_from_slice(&self.secret[offset..offset + poly_size]);
            }
            i += 1;
        }
        // Pad/truncate to expected size
        while public.len() < public_size {
            public.push(0);
        }
        public.truncate(public_size);

        DilithiumVerifyingKey {
            level: self.level,
            public,
        }
    }
}

impl DilithiumVerifyingKey {
    /// Verify a signature using ML-DSA (Dilithium)
    pub fn verify(&self, message: &[u8], signature: &DilithiumSignature) -> CryptoResult<bool> {
        use super::hash::sha256;

        let (k, l, _eta, gamma1, _) = dil_params(self.level);
        let expected_size = match self.level {
            DilithiumLevel::Level2 => 2420,
            DilithiumLevel::Level3 => 3293,
            DilithiumLevel::Level5 => 4595,
        };

        // Basic validation
        if signature.bytes.len() != expected_size {
            return Ok(false);
        }

        // Extract seed and t from public key
        if self.public.len() < 32 {
            return Ok(false);
        }
        let seed = &self.public[..32];

        let poly_size = DIL_N * 4;
        let mut t = Vec::new();
        let mut i = 0;
        while i < k {
            let offset = 32 + i * poly_size;
            if offset + poly_size <= self.public.len() {
                t.push(DilPoly::from_bytes_dil(
                    &self.public[offset..offset + poly_size],
                ));
            } else {
                t.push(DilPoly::zero());
            }
            i += 1;
        }

        // Extract challenge hash and z from signature
        if signature.bytes.len() < 32 {
            return Ok(false);
        }
        let c_hash_bytes = &signature.bytes[..32];

        let mut z = Vec::new();
        let mut idx = 0;
        while idx < l {
            let offset = 32 + idx * poly_size;
            if offset + poly_size <= signature.bytes.len() {
                z.push(DilPoly::from_bytes_dil(
                    &signature.bytes[offset..offset + poly_size],
                ));
            } else {
                z.push(DilPoly::zero());
            }
            idx += 1;
        }

        // Check z norm bound
        let beta = match self.level {
            DilithiumLevel::Level2 => 39 * 2,
            DilithiumLevel::Level3 => 49 * 4,
            DilithiumLevel::Level5 => 60 * 2,
        };
        for zj in &z {
            if !zj.check_norm(gamma1 - beta) {
                return Ok(false);
            }
        }

        // Reconstruct matrix A
        let mut a_polys = Vec::new();
        idx = 0;
        while idx < k * l {
            let mut poly_seed = Vec::with_capacity(34);
            poly_seed.extend_from_slice(seed);
            poly_seed.push((idx / l) as u8);
            poly_seed.push((idx % l) as u8);
            let hash = sha256(&poly_seed);
            a_polys.push(DilPoly::sample_uniform(hash.as_bytes()));
            idx += 1;
        }

        // Reconstruct challenge polynomial
        let c_poly = {
            let tau = match self.level {
                DilithiumLevel::Level2 => 39,
                DilithiumLevel::Level3 => 49,
                DilithiumLevel::Level5 => 60,
            };
            let mut cp = DilPoly::zero();
            let mut ci = 0;
            while ci < tau && ci < 32 {
                let pos = (c_hash_bytes[ci] as usize) % DIL_N;
                if ci % 2 == 0 {
                    cp.coeffs[pos] = 1;
                } else {
                    cp.coeffs[pos] = DIL_Q - 1;
                }
                ci += 1;
            }
            cp
        };

        // Verify: w' = A*z - c*t
        let mut w_prime = Vec::new();
        i = 0;
        while i < k {
            let mut wi = DilPoly::zero();
            let mut j = 0;
            while j < l {
                let mut a_ij = a_polys[i * l + j].clone();
                let mut zj = z[j].clone();
                a_ij.ntt();
                zj.ntt();
                let product = a_ij.pointwise_mul(&zj);
                wi = wi.add(&product);
                j += 1;
            }
            wi.inv_ntt();

            // Subtract c * t[i]
            let mut c_ntt = c_poly.clone();
            let mut ti_ntt = t[i].clone();
            c_ntt.ntt();
            ti_ntt.ntt();
            let ct = c_ntt.pointwise_mul(&ti_ntt);
            let mut ct_normal = ct;
            ct_normal.inv_ntt();

            wi = wi.sub(&ct_normal);
            w_prime.push(wi);
            i += 1;
        }

        // Recompute challenge from w' and message
        let mut msg_hash_input = Vec::new();
        for ti in &t {
            msg_hash_input.extend_from_slice(&ti.to_bytes());
        }
        msg_hash_input.extend_from_slice(message);
        let mu = sha256(&msg_hash_input);

        let mut challenge_input = Vec::new();
        challenge_input.extend_from_slice(mu.as_bytes());
        for wi in &w_prime {
            challenge_input.extend_from_slice(&wi.to_bytes());
        }
        let c_hash_verify = sha256(&challenge_input);

        // Compare challenge hashes
        let mut diff = 0u8;
        i = 0;
        while i < 32 {
            diff |= c_hash_bytes[i] ^ c_hash_verify.as_bytes()[i];
            i += 1;
        }

        Ok(diff == 0)
    }
}

// ============================================================================
// ML-KEM (Kyber) Implementation
// ============================================================================

/// ML-KEM (Kyber) secret key
pub struct KyberSecretKey {
    level: KyberLevel,
    secret: Vec<u8>,
}

/// ML-KEM (Kyber) public key
pub struct KyberPublicKey {
    level: KyberLevel,
    public: Vec<u8>,
}

/// ML-KEM (Kyber) ciphertext
pub struct KyberCiphertext {
    bytes: Vec<u8>,
}

/// ML-KEM (Kyber) shared secret
pub struct KyberSharedSecret {
    bytes: [u8; 32],
}

/// Get Kyber parameters
fn kyber_params(level: KyberLevel) -> (usize, u32, u32, usize, usize) {
    // Returns (k, eta1, eta2, du, dv)
    match level {
        KyberLevel::Kyber512 => (2, 3, 2, 10, 4),
        KyberLevel::Kyber768 => (3, 2, 2, 10, 4),
        KyberLevel::Kyber1024 => (4, 2, 2, 11, 5),
    }
}

impl KyberSecretKey {
    /// Generate new key pair using lattice-based key generation
    pub fn generate(level: KyberLevel) -> CryptoResult<Self> {
        use super::{hash::sha256, random::get_random};

        let (k, eta1, _eta2, _du, _dv) = kyber_params(level);
        let rng = get_random();

        // Generate random seed
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed)?;

        // Expand seed for deterministic generation
        let rho_sigma = {
            let mut input = Vec::from(seed.as_slice());
            input.push(k as u8);
            let hash = super::hash::sha512(&input);
            *hash.as_bytes()
        };
        let rho = &rho_sigma[..32]; // For matrix A
        let sigma = &rho_sigma[32..64]; // For secret/noise

        // Generate matrix A (k x k polynomials in NTT domain)
        let mut a_hat = Vec::new();
        let mut i = 0;
        while i < k * k {
            let mut a_seed = Vec::new();
            a_seed.extend_from_slice(rho);
            a_seed.push((i % k) as u8);
            a_seed.push((i / k) as u8);
            let hash = sha256(&a_seed);
            let mut poly = KyberPoly::sample_uniform(hash.as_bytes());
            poly.ntt();
            a_hat.push(poly);
            i += 1;
        }

        // Generate secret vector s (k polynomials with small coefficients)
        let mut s = Vec::new();
        i = 0;
        while i < k {
            let mut s_seed = Vec::new();
            s_seed.extend_from_slice(sigma);
            s_seed.push(i as u8);
            let hash = sha256(&s_seed);
            let poly = KyberPoly::sample_cbd(hash.as_bytes(), eta1);
            s.push(poly);
            i += 1;
        }

        // Generate error vector e (k polynomials)
        let mut e = Vec::new();
        i = 0;
        while i < k {
            let mut e_seed = Vec::new();
            e_seed.extend_from_slice(sigma);
            e_seed.push((k + i) as u8);
            let hash = sha256(&e_seed);
            let poly = KyberPoly::sample_cbd(hash.as_bytes(), eta1);
            e.push(poly);
            i += 1;
        }

        // Compute NTT of s
        let mut s_hat = Vec::new();
        for si in &s {
            let mut si_ntt = si.clone();
            si_ntt.ntt();
            s_hat.push(si_ntt);
        }

        // Compute NTT of e
        let mut e_hat = Vec::new();
        for ei in &e {
            let mut ei_ntt = ei.clone();
            ei_ntt.ntt();
            e_hat.push(ei_ntt);
        }

        // Compute t_hat = A_hat * s_hat + e_hat
        let mut t_hat = Vec::new();
        i = 0;
        while i < k {
            let mut ti = KyberPoly::zero();
            let mut j = 0;
            while j < k {
                let product = a_hat[i * k + j].pointwise_mul(&s_hat[j]);
                ti = ti.add(&product);
                j += 1;
            }
            ti = ti.add(&e_hat[i]);
            t_hat.push(ti);
            i += 1;
        }

        // Serialize secret key: seed || s || pk
        let mut secret_data = Vec::new();
        secret_data.extend_from_slice(&seed);
        // Store s in NTT domain
        for si in &s_hat {
            secret_data.extend_from_slice(&si.to_bytes());
        }
        // Store hash of public key for decapsulation
        let mut pk_bytes = Vec::new();
        pk_bytes.extend_from_slice(rho);
        for ti in &t_hat {
            pk_bytes.extend_from_slice(&ti.to_bytes());
        }
        let pk_hash = sha256(&pk_bytes);
        secret_data.extend_from_slice(pk_hash.as_bytes());
        // Store rho for matrix reconstruction
        secret_data.extend_from_slice(rho);
        // Store t_hat for public key derivation
        for ti in &t_hat {
            secret_data.extend_from_slice(&ti.to_bytes());
        }

        Ok(Self {
            level,
            secret: secret_data,
        })
    }

    /// Get corresponding public key
    pub fn public_key(&self) -> KyberPublicKey {
        let (k, _, _, _, _) = kyber_params(self.level);

        let public_size = match self.level {
            KyberLevel::Kyber512 => 800,
            KyberLevel::Kyber768 => 1184,
            KyberLevel::Kyber1024 => 1568,
        };

        // Public key = rho || t_hat
        // rho is at offset: 32 (seed) + k * poly_bytes_full (s_hat) + 32 (pk_hash)
        let s_hat_bytes = k * KYBER_N * 12 / 8;
        let rho_offset = 32 + s_hat_bytes + 32;

        let mut public = Vec::with_capacity(public_size);

        // Extract rho and t_hat from secret key
        if rho_offset + 32 <= self.secret.len() {
            public.extend_from_slice(&self.secret[rho_offset..rho_offset + 32]);
        } else {
            public.extend_from_slice(&[0u8; 32]);
        }

        // t_hat follows rho
        let t_start = rho_offset + 32;
        let mut i = 0;
        while i < k {
            let offset = t_start + i * (KYBER_N * 12 / 8);
            if offset + KYBER_N * 12 / 8 <= self.secret.len() {
                public.extend_from_slice(&self.secret[offset..offset + KYBER_N * 12 / 8]);
            }
            i += 1;
        }

        // Pad to expected size
        while public.len() < public_size {
            public.push(0);
        }
        public.truncate(public_size);

        KyberPublicKey {
            level: self.level,
            public,
        }
    }

    /// Decapsulate to get shared secret
    pub fn decapsulate(&self, ciphertext: &KyberCiphertext) -> CryptoResult<KyberSharedSecret> {
        use super::hash::sha256;

        let (k, _, _, du, dv) = kyber_params(self.level);

        // Extract s_hat from secret key
        let poly_bytes = KYBER_N * 12 / 8;
        let mut s_hat = Vec::new();
        let mut i = 0;
        while i < k {
            let offset = 32 + i * poly_bytes;
            if offset + poly_bytes <= self.secret.len() {
                s_hat.push(KyberPoly::from_bytes(
                    &self.secret[offset..offset + poly_bytes],
                ));
            } else {
                s_hat.push(KyberPoly::zero());
            }
            i += 1;
        }

        // Parse ciphertext: u (k compressed polynomials) || v (1 compressed polynomial)
        let u_bytes_per_poly = KYBER_N * du / 8;
        let v_bytes = KYBER_N * dv / 8;
        let mut u = Vec::new();
        i = 0;
        while i < k {
            let offset = i * u_bytes_per_poly;
            if offset + u_bytes_per_poly <= ciphertext.bytes.len() {
                let mut ui =
                    KyberPoly::decompress(&ciphertext.bytes[offset..offset + u_bytes_per_poly], du);
                ui.ntt();
                u.push(ui);
            } else {
                u.push(KyberPoly::zero());
            }
            i += 1;
        }

        let v_offset = k * u_bytes_per_poly;
        let v = if v_offset + v_bytes <= ciphertext.bytes.len() {
            KyberPoly::decompress(&ciphertext.bytes[v_offset..v_offset + v_bytes], dv)
        } else {
            KyberPoly::zero()
        };

        // Compute m' = v - s^T * u
        let mut su = KyberPoly::zero();
        i = 0;
        while i < k {
            let product = s_hat[i].pointwise_mul(&u[i]);
            su = su.add(&product);
            i += 1;
        }
        su.inv_ntt();

        let m_prime = v.sub(&su);

        // Decode message from polynomial (each coefficient encodes 1 bit)
        let mut msg = [0u8; 32];
        let mut bi = 0;
        while bi < 256 && bi / 8 < 32 {
            let mut coeff = m_prime.coeffs[bi] as i32;
            if coeff < 0 {
                coeff += KYBER_Q as i32;
            }
            // Round to nearest: if coeff > q/2, bit = 1; else bit = 0
            let bit = if coeff > KYBER_Q as i32 / 2 { 1u8 } else { 0u8 };
            msg[bi / 8] |= bit << (bi % 8);
            bi += 1;
        }

        // Derive shared secret from message using hash
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&msg);
        hash_input.extend_from_slice(&ciphertext.bytes);
        let shared = sha256(&hash_input);

        Ok(KyberSharedSecret {
            bytes: *shared.as_bytes(),
        })
    }
}

impl KyberPublicKey {
    /// Encapsulate to generate shared secret and ciphertext
    pub fn encapsulate(&self) -> CryptoResult<(KyberCiphertext, KyberSharedSecret)> {
        use super::{hash::sha256, random::get_random};

        let (k, _eta1, eta2, du, dv) = kyber_params(self.level);
        let rng = get_random();

        // Generate random message
        let mut msg = [0u8; 32];
        rng.fill_bytes(&mut msg)?;

        // Derive coins from message and public key hash
        let pk_hash = sha256(&self.public);
        let mut coin_input = Vec::new();
        coin_input.extend_from_slice(&msg);
        coin_input.extend_from_slice(pk_hash.as_bytes());
        let coins = sha256(&coin_input);

        // Extract rho and t_hat from public key
        let rho = if self.public.len() >= 32 {
            &self.public[..32]
        } else {
            &[0u8; 32]
        };
        let poly_bytes = KYBER_N * 12 / 8;

        let mut t_hat = Vec::new();
        let mut i = 0;
        while i < k {
            let offset = 32 + i * poly_bytes;
            if offset + poly_bytes <= self.public.len() {
                t_hat.push(KyberPoly::from_bytes(
                    &self.public[offset..offset + poly_bytes],
                ));
            } else {
                t_hat.push(KyberPoly::zero());
            }
            i += 1;
        }

        // Reconstruct matrix A_hat
        let mut a_hat = Vec::new();
        i = 0;
        while i < k * k {
            let mut a_seed = Vec::new();
            a_seed.extend_from_slice(rho);
            a_seed.push((i % k) as u8);
            a_seed.push((i / k) as u8);
            let hash = sha256(&a_seed);
            let mut poly = KyberPoly::sample_uniform(hash.as_bytes());
            poly.ntt();
            a_hat.push(poly);
            i += 1;
        }

        // Generate random vectors r, e1, e2
        let mut r = Vec::new();
        i = 0;
        while i < k {
            let mut r_seed = Vec::new();
            r_seed.extend_from_slice(coins.as_bytes());
            r_seed.push(i as u8);
            let hash = sha256(&r_seed);
            let mut poly = KyberPoly::sample_cbd(hash.as_bytes(), eta2);
            poly.ntt();
            r.push(poly);
            i += 1;
        }

        let mut e1 = Vec::new();
        i = 0;
        while i < k {
            let mut e1_seed = Vec::new();
            e1_seed.extend_from_slice(coins.as_bytes());
            e1_seed.push((k + i) as u8);
            let hash = sha256(&e1_seed);
            e1.push(KyberPoly::sample_cbd(hash.as_bytes(), eta2));
            i += 1;
        }

        let mut e2_seed = Vec::new();
        e2_seed.extend_from_slice(coins.as_bytes());
        e2_seed.push((2 * k) as u8);
        let e2_hash = sha256(&e2_seed);
        let e2 = KyberPoly::sample_cbd(e2_hash.as_bytes(), eta2);

        // Compute u = A^T * r + e1
        let mut u = Vec::new();
        i = 0;
        while i < k {
            let mut ui = KyberPoly::zero();
            let mut j = 0;
            while j < k {
                // A^T: swap indices
                let product = a_hat[j * k + i].pointwise_mul(&r[j]);
                ui = ui.add(&product);
                j += 1;
            }
            ui.inv_ntt();
            ui = ui.add(&e1[i]);
            u.push(ui);
            i += 1;
        }

        // Compute v = t^T * r + e2 + encode(msg)
        let mut v = KyberPoly::zero();
        i = 0;
        while i < k {
            let product = t_hat[i].pointwise_mul(&r[i]);
            v = v.add(&product);
            i += 1;
        }
        v.inv_ntt();
        v = v.add(&e2);

        // Encode message into polynomial (each bit becomes q/2 or 0)
        let mut msg_poly = KyberPoly::zero();
        let mut bi = 0;
        while bi < 256 && bi / 8 < 32 {
            let bit = (msg[bi / 8] >> (bi % 8)) & 1;
            if bit == 1 {
                msg_poly.coeffs[bi] = KYBER_Q.div_ceil(2) as i16;
            }
            bi += 1;
        }
        v = v.add(&msg_poly);

        // Build ciphertext: compress(u) || compress(v)
        let mut ct_bytes = Vec::new();
        for ui in &u {
            ct_bytes.extend_from_slice(&ui.compress(du));
        }
        ct_bytes.extend_from_slice(&v.compress(dv));

        // Derive shared secret
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&msg);
        hash_input.extend_from_slice(&ct_bytes);
        let shared = sha256(&hash_input);

        Ok((
            KyberCiphertext { bytes: ct_bytes },
            KyberSharedSecret {
                bytes: *shared.as_bytes(),
            },
        ))
    }
}

impl KyberSharedSecret {
    /// Get shared secret bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

/// Hybrid key exchange (combining classical and post-quantum)
pub struct HybridKeyExchange {
    /// X25519 classical key
    classical_secret: super::asymmetric::key_exchange::SecretKey,
    /// Kyber post-quantum key
    pq_secret: KyberSecretKey,
}

impl HybridKeyExchange {
    /// Generate new hybrid key pair
    pub fn generate(kyber_level: KyberLevel) -> CryptoResult<Self> {
        let classical_secret = super::asymmetric::key_exchange::SecretKey::generate()?;
        let pq_secret = KyberSecretKey::generate(kyber_level)?;

        Ok(Self {
            classical_secret,
            pq_secret,
        })
    }

    /// Get public keys
    pub fn public_keys(&self) -> (super::asymmetric::key_exchange::PublicKey, KyberPublicKey) {
        (
            self.classical_secret.public_key(),
            self.pq_secret.public_key(),
        )
    }

    /// Perform hybrid key exchange
    pub fn exchange(
        &self,
        their_classical: &super::asymmetric::key_exchange::PublicKey,
        their_pq_ct: &KyberCiphertext,
    ) -> CryptoResult<[u8; 32]> {
        // Classical key exchange
        let classical_shared = self.classical_secret.exchange(their_classical)?;

        // Post-quantum key decapsulation
        let pq_shared = self.pq_secret.decapsulate(their_pq_ct)?;

        // Combine both secrets (XOR for simplicity, KDF in production)
        use super::hash::sha256;

        let mut combined = Vec::new();
        combined.extend_from_slice(classical_shared.as_bytes());
        combined.extend_from_slice(pq_shared.as_bytes());

        let final_secret = sha256(&combined);

        Ok(*final_secret.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_dilithium_signing() {
        let signing_key = DilithiumSigningKey::generate(DilithiumLevel::Level2).unwrap();
        let verifying_key = signing_key.verifying_key();

        let message = b"Hello, Post-Quantum World!";
        let signature = signing_key.sign(message).unwrap();

        assert!(verifying_key.verify(message, &signature).unwrap());
    }

    #[test_case]
    fn test_kyber_kem() {
        let secret_key = KyberSecretKey::generate(KyberLevel::Kyber768).unwrap();
        let public_key = secret_key.public_key();

        let (ciphertext, shared_secret1) = public_key.encapsulate().unwrap();
        let shared_secret2 = secret_key.decapsulate(&ciphertext).unwrap();

        // Secrets should match
        assert_eq!(shared_secret1.as_bytes(), shared_secret2.as_bytes());
    }

    #[test_case]
    fn test_hybrid_exchange() {
        let alice = HybridKeyExchange::generate(KyberLevel::Kyber768).unwrap();
        let bob = HybridKeyExchange::generate(KyberLevel::Kyber768).unwrap();

        let (alice_classical_pub, alice_pq_pub) = alice.public_keys();
        let (bob_classical_pub, bob_pq_pub) = bob.public_keys();

        let (bob_pq_ct, _) = alice_pq_pub.encapsulate().unwrap();

        let alice_shared = alice.exchange(&bob_classical_pub, &bob_pq_ct).unwrap();

        // Just verify it completes without error
        assert_eq!(alice_shared.len(), 32);
    }
}
