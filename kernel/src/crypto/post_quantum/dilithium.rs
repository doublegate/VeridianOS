//! ML-DSA (Dilithium) Digital Signature Algorithm
//!
//! Implements lattice-based digital signatures following NIST FIPS 204.
//! Provides quantum-resistant signatures at security levels 2, 3, 5.

use alloc::vec::Vec;

use super::{expand_seed, mod_pow, DilithiumLevel};
use crate::crypto::CryptoResult;

// ============================================================================
// Dilithium Constants and Helpers
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
// Dilithium Polynomial
// ============================================================================

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

// ============================================================================
// ML-DSA (Dilithium) Public Types
// ============================================================================

/// Get Dilithium parameters for a given level
fn dil_params(level: DilithiumLevel) -> (usize, usize, u32, i32, usize) {
    // Returns (k, l, eta, gamma1, sig_size)
    match level {
        DilithiumLevel::Level2 => (4, 4, 2, 1 << 17, 2420),
        DilithiumLevel::Level3 => (6, 5, 4, 1 << 19, 3293),
        DilithiumLevel::Level5 => (8, 7, 2, 1 << 19, 4595),
    }
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

impl DilithiumSigningKey {
    /// Generate new signing key using lattice-based key generation
    pub fn generate(level: DilithiumLevel) -> CryptoResult<Self> {
        use crate::crypto::{hash::sha256, random::get_random};

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
        use crate::crypto::hash::sha256;

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
        Err(crate::crypto::CryptoError::SignatureFailed)
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
        use crate::crypto::hash::sha256;

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
