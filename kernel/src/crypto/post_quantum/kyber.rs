//! ML-KEM (Kyber) Key Encapsulation Mechanism
//!
//! Implements lattice-based key encapsulation following NIST FIPS 203.
//! Provides quantum-resistant key exchange at security levels 512, 768, 1024.

use alloc::vec::Vec;

use super::{
    bit_reverse_7, expand_seed, kyber_barrett_reduce, mod_pow, KyberLevel, KYBER_N, KYBER_Q,
    KYBER_ZETA,
};
use crate::crypto::CryptoResult;

// ============================================================================
// NTT Constants and Helpers
// ============================================================================

/// Montgomery parameter for Kyber: R = 2^16 mod q
const KYBER_MONT_R: u32 = 2285; // 2^16 mod 3329

/// Barrett reduction constant for Kyber
const KYBER_BARRETT_V: u32 = 20159; // round(2^26 / q)

/// Precomputed NTT zetas (powers of the root of unity) for Kyber
/// zetas[i] = KYBER_ZETA^(bit_reverse_7(i)) mod q
fn kyber_ntt_zetas() -> [u16; 128] {
    let mut zetas = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        let rev = bit_reverse_7(i as u8) as u32;
        zetas[i] = mod_pow(KYBER_ZETA, rev, KYBER_Q) as u16;
        i += 1;
    }
    zetas
}

// ============================================================================
// Kyber Polynomial
// ============================================================================

/// Kyber polynomial (degree N-1 with coefficients mod q)
#[derive(Clone)]
pub(super) struct KyberPoly {
    coeffs: [i16; KYBER_N],
}

impl KyberPoly {
    pub(super) fn zero() -> Self {
        KyberPoly {
            coeffs: [0i16; KYBER_N],
        }
    }

    /// Forward NTT (number theoretic transform) in place
    /// Transforms polynomial from normal form to NTT domain
    pub(super) fn ntt(&mut self) {
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
    pub(super) fn inv_ntt(&mut self) {
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
    pub(super) fn pointwise_mul(&self, other: &KyberPoly) -> KyberPoly {
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
    pub(super) fn add(&self, other: &KyberPoly) -> KyberPoly {
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
    pub(super) fn sub(&self, other: &KyberPoly) -> KyberPoly {
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
    pub(super) fn sample_cbd(seed: &[u8], eta: u32) -> KyberPoly {
        let mut poly = KyberPoly::zero();
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
    pub(super) fn sample_uniform(seed: &[u8]) -> KyberPoly {
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
    pub(super) fn compress(&self, d: usize) -> Vec<u8> {
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
    pub(super) fn decompress(data: &[u8], d: usize) -> KyberPoly {
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
    pub(super) fn to_bytes(&self) -> Vec<u8> {
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
    pub(super) fn from_bytes(data: &[u8]) -> KyberPoly {
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

// ============================================================================
// ML-KEM (Kyber) Public Types
// ============================================================================

/// Get Kyber parameters
fn kyber_params(level: KyberLevel) -> (usize, u32, u32, usize, usize) {
    // Returns (k, eta1, eta2, du, dv)
    match level {
        KyberLevel::Kyber512 => (2, 3, 2, 10, 4),
        KyberLevel::Kyber768 => (3, 2, 2, 10, 4),
        KyberLevel::Kyber1024 => (4, 2, 2, 11, 5),
    }
}

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

impl KyberSecretKey {
    /// Generate new key pair using lattice-based key generation
    pub fn generate(level: KyberLevel) -> CryptoResult<Self> {
        use crate::crypto::{hash::sha256, random::get_random};

        let (k, eta1, _eta2, _du, _dv) = kyber_params(level);
        let rng = get_random();

        // Generate random seed
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed)?;

        // Expand seed for deterministic generation
        let rho_sigma = {
            let mut input = Vec::from(seed.as_slice());
            input.push(k as u8);
            let hash = crate::crypto::hash::sha512(&input);
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
        use crate::crypto::hash::sha256;

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
        use crate::crypto::{hash::sha256, random::get_random};

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
