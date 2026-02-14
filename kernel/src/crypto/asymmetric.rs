//! Asymmetric Cryptography
//!
//! Implements Ed25519 signatures (RFC 8032) and X25519 key exchange (RFC 7748).
//!
//! ## Implementation Details
//!
//! Field arithmetic operates in GF(2^255-19) using a 5-limb representation
//! with 51 bits per limb, giving us 255 bits total. This allows intermediate
//! products to fit in u128 without overflow.
//!
//! Ed25519 uses the twisted Edwards curve -x^2 + y^2 = 1 + d*x^2*y^2
//! where d = -121665/121666 mod p.
//!
//! X25519 uses the Montgomery form of Curve25519 with the Montgomery ladder
//! for constant-time scalar multiplication.

use alloc::vec::Vec;

use super::{CryptoError, CryptoResult};

// ============================================================================
// Field Arithmetic in GF(2^255-19)
// ============================================================================

/// Field element in GF(2^255-19), represented as 5 limbs of 51 bits each.
#[derive(Clone, Copy, Debug)]
struct Fe([u64; 5]);

impl Fe {
    const ZERO: Fe = Fe([0; 5]);
    const ONE: Fe = Fe([1, 0, 0, 0, 0]);

    /// Create field element from bytes (little-endian, 32 bytes)
    fn from_bytes(s: &[u8; 32]) -> Fe {
        let mut h = [0u64; 5];
        // Load 256 bits from 32 bytes, then mask to 255 bits
        let load64 = |bytes: &[u8]| -> u64 {
            let mut buf = [0u8; 8];
            let len = core::cmp::min(bytes.len(), 8);
            buf[..len].copy_from_slice(&bytes[..len]);
            u64::from_le_bytes(buf)
        };

        h[0] = load64(&s[0..]) & 0x7ffffffffffff; // 51 bits
        h[1] = (load64(&s[6..]) >> 3) & 0x7ffffffffffff;
        h[2] = (load64(&s[12..]) >> 6) & 0x7ffffffffffff;
        h[3] = (load64(&s[19..]) >> 1) & 0x7ffffffffffff;
        h[4] = (load64(&s[24..]) >> 12) & 0x7ffffffffffff;

        Fe(h)
    }

    /// Convert field element to bytes (little-endian, 32 bytes)
    fn to_bytes(self) -> [u8; 32] {
        let mut h = self;
        h.reduce();

        // Full reduction: ensure 0 <= h < p
        // After reduce(), h is nearly reduced. We do a final conditional subtraction.
        let mut q = (h.0[0].wrapping_add(19)) >> 51;
        q = (h.0[1].wrapping_add(q)) >> 51;
        q = (h.0[2].wrapping_add(q)) >> 51;
        q = (h.0[3].wrapping_add(q)) >> 51;
        q = (h.0[4].wrapping_add(q)) >> 51;

        h.0[0] = h.0[0].wrapping_add(19u64.wrapping_mul(q));
        let mut carry: u64;
        carry = h.0[0] >> 51;
        h.0[0] &= 0x7ffffffffffff;
        h.0[1] = h.0[1].wrapping_add(carry);
        carry = h.0[1] >> 51;
        h.0[1] &= 0x7ffffffffffff;
        h.0[2] = h.0[2].wrapping_add(carry);
        carry = h.0[2] >> 51;
        h.0[2] &= 0x7ffffffffffff;
        h.0[3] = h.0[3].wrapping_add(carry);
        carry = h.0[3] >> 51;
        h.0[3] &= 0x7ffffffffffff;
        h.0[4] = h.0[4].wrapping_add(carry);
        h.0[4] &= 0x7ffffffffffff;

        let mut s = [0u8; 32];
        // Pack 5 limbs of 51 bits into 32 bytes (little-endian)
        s[0] = h.0[0] as u8;
        s[1] = (h.0[0] >> 8) as u8;
        s[2] = (h.0[0] >> 16) as u8;
        s[3] = (h.0[0] >> 24) as u8;
        s[4] = (h.0[0] >> 32) as u8;
        s[5] = (h.0[0] >> 40) as u8;
        s[6] = ((h.0[0] >> 48) | (h.0[1] << 3)) as u8;
        s[7] = (h.0[1] >> 5) as u8;
        s[8] = (h.0[1] >> 13) as u8;
        s[9] = (h.0[1] >> 21) as u8;
        s[10] = (h.0[1] >> 29) as u8;
        s[11] = (h.0[1] >> 37) as u8;
        s[12] = ((h.0[1] >> 45) | (h.0[2] << 6)) as u8;
        s[13] = (h.0[2] >> 2) as u8;
        s[14] = (h.0[2] >> 10) as u8;
        s[15] = (h.0[2] >> 18) as u8;
        s[16] = (h.0[2] >> 26) as u8;
        s[17] = (h.0[2] >> 34) as u8;
        s[18] = (h.0[2] >> 42) as u8;
        s[19] = ((h.0[2] >> 50) | (h.0[3] << 1)) as u8;
        s[20] = (h.0[3] >> 7) as u8;
        s[21] = (h.0[3] >> 15) as u8;
        s[22] = (h.0[3] >> 23) as u8;
        s[23] = (h.0[3] >> 31) as u8;
        s[24] = ((h.0[3] >> 39) | (h.0[4] << 12)) as u8;
        s[25] = (h.0[4] >> 4) as u8;
        s[26] = (h.0[4] >> 12) as u8;
        s[27] = (h.0[4] >> 20) as u8;
        s[28] = (h.0[4] >> 28) as u8;
        s[29] = (h.0[4] >> 36) as u8;
        s[30] = (h.0[4] >> 44) as u8;
        s[31] = 0; // top bit always 0 for reduced element

        s
    }

    /// Carry propagation / partial reduction
    fn reduce(&mut self) {
        let mut carry: u64;
        carry = self.0[0] >> 51;
        self.0[0] &= 0x7ffffffffffff;
        self.0[1] = self.0[1].wrapping_add(carry);
        carry = self.0[1] >> 51;
        self.0[1] &= 0x7ffffffffffff;
        self.0[2] = self.0[2].wrapping_add(carry);
        carry = self.0[2] >> 51;
        self.0[2] &= 0x7ffffffffffff;
        self.0[3] = self.0[3].wrapping_add(carry);
        carry = self.0[3] >> 51;
        self.0[3] &= 0x7ffffffffffff;
        self.0[4] = self.0[4].wrapping_add(carry);
        carry = self.0[4] >> 51;
        self.0[4] &= 0x7ffffffffffff;
        // 2^255 = 19 mod p, so carry from top limb wraps with factor 19
        self.0[0] = self.0[0].wrapping_add(carry.wrapping_mul(19));
    }

    /// Addition in GF(p)
    fn add(&self, other: &Fe) -> Fe {
        let mut r = Fe::ZERO;
        r.0[0] = self.0[0].wrapping_add(other.0[0]);
        r.0[1] = self.0[1].wrapping_add(other.0[1]);
        r.0[2] = self.0[2].wrapping_add(other.0[2]);
        r.0[3] = self.0[3].wrapping_add(other.0[3]);
        r.0[4] = self.0[4].wrapping_add(other.0[4]);
        r
    }

    /// Subtraction in GF(p)
    fn sub(&self, other: &Fe) -> Fe {
        // Add 2*p to avoid underflow (each limb gets 2 * (2^51 - 1) headroom,
        // except limb 0 which gets 2*(2^51 - 19))
        let mut r = Fe::ZERO;
        r.0[0] = self.0[0]
            .wrapping_add(0xfffffffffffda) // 2*(2^51 - 19)
            .wrapping_sub(other.0[0]);
        r.0[1] = self.0[1]
            .wrapping_add(0xffffffffffffe) // 2*(2^51 - 1)
            .wrapping_sub(other.0[1]);
        r.0[2] = self.0[2]
            .wrapping_add(0xffffffffffffe)
            .wrapping_sub(other.0[2]);
        r.0[3] = self.0[3]
            .wrapping_add(0xffffffffffffe)
            .wrapping_sub(other.0[3]);
        r.0[4] = self.0[4]
            .wrapping_add(0xffffffffffffe)
            .wrapping_sub(other.0[4]);
        r.reduce();
        r
    }

    /// Multiplication in GF(p) using u128 for intermediate products
    fn mul(&self, other: &Fe) -> Fe {
        let a = &self.0;
        let b = &other.0;

        // Schoolbook multiplication with reduction
        // Since 2^255 = 19 mod p, we precompute 19*b[i] for the wrap-around terms
        let b1_19 = b[1].wrapping_mul(19);
        let b2_19 = b[2].wrapping_mul(19);
        let b3_19 = b[3].wrapping_mul(19);
        let b4_19 = b[4].wrapping_mul(19);

        let t0 = (a[0] as u128) * (b[0] as u128)
            + (a[1] as u128) * (b4_19 as u128)
            + (a[2] as u128) * (b3_19 as u128)
            + (a[3] as u128) * (b2_19 as u128)
            + (a[4] as u128) * (b1_19 as u128);

        let t1 = (a[0] as u128) * (b[1] as u128)
            + (a[1] as u128) * (b[0] as u128)
            + (a[2] as u128) * (b4_19 as u128)
            + (a[3] as u128) * (b3_19 as u128)
            + (a[4] as u128) * (b2_19 as u128);

        let t2 = (a[0] as u128) * (b[2] as u128)
            + (a[1] as u128) * (b[1] as u128)
            + (a[2] as u128) * (b[0] as u128)
            + (a[3] as u128) * (b4_19 as u128)
            + (a[4] as u128) * (b3_19 as u128);

        let t3 = (a[0] as u128) * (b[3] as u128)
            + (a[1] as u128) * (b[2] as u128)
            + (a[2] as u128) * (b[1] as u128)
            + (a[3] as u128) * (b[0] as u128)
            + (a[4] as u128) * (b4_19 as u128);

        let t4 = (a[0] as u128) * (b[4] as u128)
            + (a[1] as u128) * (b[3] as u128)
            + (a[2] as u128) * (b[2] as u128)
            + (a[3] as u128) * (b[1] as u128)
            + (a[4] as u128) * (b[0] as u128);

        // Carry propagation
        let mut r = [0u64; 5];
        let mut c: u128;

        c = t0 >> 51;
        r[0] = (t0 as u64) & 0x7ffffffffffff;
        let t1 = t1 + c;

        c = t1 >> 51;
        r[1] = (t1 as u64) & 0x7ffffffffffff;
        let t2 = t2 + c;

        c = t2 >> 51;
        r[2] = (t2 as u64) & 0x7ffffffffffff;
        let t3 = t3 + c;

        c = t3 >> 51;
        r[3] = (t3 as u64) & 0x7ffffffffffff;
        let t4 = t4 + c;

        c = t4 >> 51;
        r[4] = (t4 as u64) & 0x7ffffffffffff;

        // Wrap carry with factor 19
        r[0] = r[0].wrapping_add((c as u64).wrapping_mul(19));
        // One more carry from limb 0
        let c2 = r[0] >> 51;
        r[0] &= 0x7ffffffffffff;
        r[1] = r[1].wrapping_add(c2);

        Fe(r)
    }

    /// Squaring in GF(p) - optimized version of mul(self, self)
    fn square(&self) -> Fe {
        self.mul(self)
    }

    /// Compute self^(2^n) by repeated squaring
    fn pow2k(&self, k: u32) -> Fe {
        let mut r = *self;
        let mut i = 0;
        while i < k {
            r = r.square();
            i += 1;
        }
        r
    }

    /// Modular inversion using Fermat's little theorem: a^(-1) = a^(p-2) mod p
    /// p-2 = 2^255 - 21
    fn invert(&self) -> Fe {
        // Compute a^(p-2) using an addition chain
        let z2 = self.square(); // z^2
        let z9 = z2.pow2k(2).mul(self); // z^9 (z2^4 * z)
        let z11 = z9.mul(&z2); // z^11
        let z_5_0 = z11.square().mul(&z9); // z^(2^5 - 1)
        let z_10_0 = z_5_0.pow2k(5).mul(&z_5_0); // z^(2^10 - 1)
        let z_20_0 = z_10_0.pow2k(10).mul(&z_10_0); // z^(2^20 - 1)
        let z_40_0 = z_20_0.pow2k(20).mul(&z_20_0); // z^(2^40 - 1)
        let z_50_0 = z_40_0.pow2k(10).mul(&z_10_0); // z^(2^50 - 1)
        let z_100_0 = z_50_0.pow2k(50).mul(&z_50_0); // z^(2^100 - 1)
        let z_200_0 = z_100_0.pow2k(100).mul(&z_100_0); // z^(2^200 - 1)
        let z_250_0 = z_200_0.pow2k(50).mul(&z_50_0); // z^(2^250 - 1)
        z_250_0.pow2k(5).mul(&z11) // z^(2^255 - 21)
    }

    /// Negate in GF(p)
    fn neg(&self) -> Fe {
        Fe::ZERO.sub(self)
    }

    /// Check if this field element is negative (odd when reduced)
    fn is_negative(&self) -> bool {
        let bytes = self.to_bytes();
        (bytes[0] & 1) != 0
    }

    /// Conditional swap: swap self and other if swap_flag == 1
    fn cswap(&mut self, other: &mut Fe, swap_flag: u64) {
        let mask = swap_flag.wrapping_neg(); // 0 or 0xFFFFFFFFFFFFFFFF
        let mut i = 0;
        while i < 5 {
            let t = mask & (self.0[i] ^ other.0[i]);
            self.0[i] ^= t;
            other.0[i] ^= t;
            i += 1;
        }
    }

    /// Compute square root: self^((p+3)/8) for p = 2^255 - 19
    /// Returns Some(root) if self is a quadratic residue, None otherwise
    #[allow(dead_code)]
    fn sqrt(&self) -> Option<Fe> {
        // p = 2^255 - 19, (p+3)/8 = 2^252 - 2
        // We use the formula: if v = a^((p-5)/8), then
        // candidates are a*v and a*v*sqrt(-1)

        // First compute a^((p-5)/8) = a^(2^252 - 3)
        let a = *self;
        let a2 = a.square();
        let a9 = a2.pow2k(2).mul(&a);
        let a11 = a9.mul(&a2);
        let z_5_0 = a11.square().mul(&a9);
        let z_10_0 = z_5_0.pow2k(5).mul(&z_5_0);
        let z_20_0 = z_10_0.pow2k(10).mul(&z_10_0);
        let z_40_0 = z_20_0.pow2k(20).mul(&z_20_0);
        let z_50_0 = z_40_0.pow2k(10).mul(&z_10_0);
        let z_100_0 = z_50_0.pow2k(50).mul(&z_50_0);
        let z_200_0 = z_100_0.pow2k(100).mul(&z_100_0);
        let z_250_0 = z_200_0.pow2k(50).mul(&z_50_0);
        let beta = z_250_0.pow2k(2).mul(&a); // a^(2^252 - 3)

        // Check beta^2 == a
        let beta_sq = beta.square();
        let check = beta_sq.sub(&a);
        if check.to_bytes() == [0u8; 32] {
            return Some(beta);
        }

        // Try beta * sqrt(-1)
        // sqrt(-1) = 2^((p-1)/4) mod p
        let sqrt_m1 = Fe::SQRT_MINUS_ONE;
        let beta2 = beta.mul(&sqrt_m1);
        let beta2_sq = beta2.square();
        let check2 = beta2_sq.sub(&a);
        if check2.to_bytes() == [0u8; 32] {
            return Some(beta2);
        }

        None
    }

    // sqrt(-1) mod p = 2^((p-1)/4) mod p
    // = 0x2b8324804fc1df0b2b4d00993dfbd7a72f431806ad2fe478c4ee1b274a0ea0b0
    const SQRT_MINUS_ONE: Fe = Fe([
        0x00061b274a0ea0b0,
        0x0000d5a5fc8f189d,
        0x0007ef5e9cbd0c60,
        0x00078595a6804c9e,
        0x0002b8324804fc1d,
    ]);
}

// ============================================================================
// Edwards Curve Point Operations
// ============================================================================

/// Point on the Ed25519 curve in extended coordinates (X, Y, Z, T)
/// where x = X/Z, y = Y/Z, x*y = T/Z
#[derive(Clone, Copy)]
struct EdPoint {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

impl EdPoint {
    /// The identity point (neutral element)
    const IDENTITY: EdPoint = EdPoint {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    };

    /// The Ed25519 base point B
    fn basepoint() -> EdPoint {
        // B_y = 4/5 mod p
        // B_x is recovered from the curve equation
        // Standard encoding of the base point:
        let b_bytes: [u8; 32] = [
            0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
            0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
            0x66, 0x66, 0x66, 0x66,
        ];
        EdPoint::decode(&b_bytes).expect("basepoint decode must succeed")
    }

    /// Ed25519 curve constant d = -121665/121666 mod p
    fn curve_d() -> Fe {
        // d = 0x52036cee2b6ffe738cc740797779e89800700a4d4141d8ab75eb4dca135978a3
        let d_bytes: [u8; 32] = [
            0xa3, 0x78, 0x59, 0x13, 0xca, 0x4d, 0xeb, 0x75, 0xab, 0xd8, 0x41, 0x41, 0x4d, 0x0a,
            0x70, 0x00, 0x98, 0xe8, 0x79, 0x77, 0x79, 0x40, 0xc7, 0x8c, 0x73, 0xfe, 0x6f, 0x2b,
            0xee, 0x6c, 0x03, 0x52,
        ];
        Fe::from_bytes(&d_bytes)
    }

    /// 2*d
    fn curve_2d() -> Fe {
        let d = Self::curve_d();
        d.add(&d)
    }

    /// Encode point to 32 bytes (compressed Edwards point)
    fn encode(&self) -> [u8; 32] {
        let zi = self.z.invert();
        let x = self.x.mul(&zi);
        let y = self.y.mul(&zi);

        let mut s = y.to_bytes();
        // Set high bit to sign of x
        s[31] |= if x.is_negative() { 0x80 } else { 0x00 };
        s
    }

    /// Decode point from 32 bytes
    fn decode(s: &[u8; 32]) -> Option<EdPoint> {
        // Extract sign bit of x
        let x_sign = (s[31] >> 7) & 1;

        // Decode y (clear top bit)
        let mut y_bytes = *s;
        y_bytes[31] &= 0x7f;
        let y = Fe::from_bytes(&y_bytes);

        // Recover x from curve equation: -x^2 + y^2 = 1 + d*x^2*y^2
        // => x^2 = (y^2 - 1) / (d*y^2 + 1)
        let y2 = y.square();
        let d = Self::curve_d();
        let u = y2.sub(&Fe::ONE); // y^2 - 1
        let v = d.mul(&y2).add(&Fe::ONE); // d*y^2 + 1

        // x = sqrt(u/v) = u * v^3 * (u * v^7)^((p-5)/8)
        let v2 = v.square();
        let v3 = v2.mul(&v);
        let v4 = v2.square();
        let v7 = v4.mul(&v3);
        let uv7 = u.mul(&v7);

        // Compute (uv7)^((p-5)/8)
        // p-5 = 2^255-24, (p-5)/8 = 2^252-3
        let uv7_252_3 = {
            let a = uv7;
            let a2 = a.square();
            let a9 = a2.pow2k(2).mul(&a);
            let a11 = a9.mul(&a2);
            let z5 = a11.square().mul(&a9);
            let z10 = z5.pow2k(5).mul(&z5);
            let z20 = z10.pow2k(10).mul(&z10);
            let z40 = z20.pow2k(20).mul(&z20);
            let z50 = z40.pow2k(10).mul(&z10);
            let z100 = z50.pow2k(50).mul(&z50);
            let z200 = z100.pow2k(100).mul(&z100);
            let z250 = z200.pow2k(50).mul(&z50);
            z250.pow2k(2).mul(&a)
        };

        let mut x = u.mul(&v3).mul(&uv7_252_3);

        // Check: v * x^2 == u ?
        let check = v.mul(&x.square());
        let u_neg = u.neg();

        if check.sub(&u).to_bytes() == [0u8; 32] {
            // x is correct
        } else if check.sub(&u_neg).to_bytes() == [0u8; 32] {
            // Multiply x by sqrt(-1) to fix sign
            x = x.mul(&Fe::SQRT_MINUS_ONE);
        } else {
            return None; // Not on curve
        }

        // Adjust sign
        if x.is_negative() as u8 != x_sign {
            x = x.neg();
        }

        let t = x.mul(&y);

        Some(EdPoint {
            x,
            y,
            z: Fe::ONE,
            t,
        })
    }

    /// Point doubling in extended coordinates
    fn double(&self) -> EdPoint {
        // Using formulas from https://hyperelliptic.org/EFD/g1p/auto-twisted-extended.html#doubling-dbl-2008-hwcd
        let a = self.x.square();
        let b = self.y.square();
        let c = self.z.square().add(&self.z.square()); // 2*Z^2
        let d = a.neg(); // -X^2 (for a=-1 in twisted Edwards)
        let e = self.x.add(&self.y).square().sub(&a).sub(&b);
        let g = d.add(&b);
        let f = g.sub(&c);
        let h = d.sub(&b);

        EdPoint {
            x: e.mul(&f),
            y: g.mul(&h),
            z: f.mul(&g),
            t: e.mul(&h),
        }
    }

    /// Point addition in extended coordinates
    fn add(&self, other: &EdPoint) -> EdPoint {
        let d2 = Self::curve_2d();

        let a = self.x.mul(&other.x);
        let b = self.y.mul(&other.y);
        let c = self.t.mul(&d2).mul(&other.t);
        let d = self.z.mul(&other.z).add(&self.z.mul(&other.z)); // 2*Z1*Z2

        let e = self
            .x
            .add(&self.y)
            .mul(&other.x.add(&other.y))
            .sub(&a)
            .sub(&b);
        let f = d.sub(&c);
        let g = d.add(&c);
        let h = b.add(&a); // b - (-a) = b + a (since curve a = -1)

        EdPoint {
            x: e.mul(&f),
            y: g.mul(&h),
            z: f.mul(&g),
            t: e.mul(&h),
        }
    }

    /// Scalar multiplication using double-and-add (left-to-right)
    fn scalar_mul(&self, scalar: &[u8; 32]) -> EdPoint {
        let mut result = EdPoint::IDENTITY;
        let mut found_one = false;

        // Process from MSB to LSB
        let mut i: i32 = 255;
        while i >= 0 {
            let byte_idx = (i / 8) as usize;
            let bit_idx = (i % 8) as u32;
            let bit = (scalar[byte_idx] >> bit_idx) & 1;

            if found_one {
                result = result.double();
            }

            if bit == 1 {
                if found_one {
                    result = result.add(self);
                } else {
                    result = *self;
                    found_one = true;
                }
            }

            i -= 1;
        }

        result
    }
}

// ============================================================================
// SHA-512 helper for Ed25519
// ============================================================================

/// Use the existing SHA-512 implementation from hash module
fn sha512(data: &[u8]) -> [u8; 64] {
    let hash = super::hash::sha512(data);
    *hash.as_bytes()
}

/// SHA-512 of concatenated slices
fn sha512_2(a: &[u8], b: &[u8]) -> [u8; 64] {
    let mut combined = Vec::with_capacity(a.len() + b.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    sha512(&combined)
}

/// SHA-512 of three concatenated slices
fn sha512_3(a: &[u8], b: &[u8], c: &[u8]) -> [u8; 64] {
    let mut combined = Vec::with_capacity(a.len() + b.len() + c.len());
    combined.extend_from_slice(a);
    combined.extend_from_slice(b);
    combined.extend_from_slice(c);
    sha512(&combined)
}

/// Reduce a 512-bit scalar modulo L (the Ed25519 group order)
/// L = 2^252 + 27742317777372353535851937790883648493
///
/// Uses the reference Ed25519 reduction approach: load the 64-byte input into
/// 24 limbs of 21 bits each, then reduce from the top by subtracting
/// multiples of L.
fn sc_reduce(input: &[u8; 64]) -> [u8; 32] {
    // Load input into 21-bit limbs (base 2^21)
    let mut s = [0i64; 24];
    s[0] = 2097151 & load_3i(&input[0..3]);
    s[1] = 2097151 & (load_4i(&input[2..6]) >> 5);
    s[2] = 2097151 & (load_3i(&input[5..8]) >> 2);
    s[3] = 2097151 & (load_4i(&input[7..11]) >> 7);
    s[4] = 2097151 & (load_4i(&input[10..14]) >> 4);
    s[5] = 2097151 & (load_3i(&input[13..16]) >> 1);
    s[6] = 2097151 & (load_4i(&input[15..19]) >> 6);
    s[7] = 2097151 & (load_3i(&input[18..21]) >> 3);
    s[8] = 2097151 & load_3i(&input[21..24]);
    s[9] = 2097151 & (load_4i(&input[23..27]) >> 5);
    s[10] = 2097151 & (load_3i(&input[26..29]) >> 2);
    s[11] = 2097151 & (load_4i(&input[28..32]) >> 7);
    s[12] = 2097151 & (load_4i(&input[31..35]) >> 4);
    s[13] = 2097151 & (load_3i(&input[34..37]) >> 1);
    s[14] = 2097151 & (load_4i(&input[36..40]) >> 6);
    s[15] = 2097151 & (load_3i(&input[39..42]) >> 3);
    s[16] = 2097151 & load_3i(&input[42..45]);
    s[17] = 2097151 & (load_4i(&input[44..48]) >> 5);
    s[18] = 2097151 & (load_3i(&input[47..50]) >> 2);
    s[19] = 2097151 & (load_4i(&input[49..53]) >> 7);
    s[20] = 2097151 & (load_4i(&input[52..56]) >> 4);
    s[21] = 2097151 & (load_3i(&input[55..58]) >> 1);
    s[22] = 2097151 & (load_4i(&input[57..61]) >> 6);
    s[23] = load_4i(&input[60..64]) >> 3;

    // Reduce using the reference implementation approach
    // s[i] -= s[j] * L[i-j] for j > 11
    sc_muladd_reduce(&mut s);

    // Pack result
    let mut result = [0u8; 32];
    result[0] = s[0] as u8;
    result[1] = (s[0] >> 8) as u8;
    result[2] = ((s[0] >> 16) | (s[1] << 5)) as u8;
    result[3] = (s[1] >> 3) as u8;
    result[4] = (s[1] >> 11) as u8;
    result[5] = ((s[1] >> 19) | (s[2] << 2)) as u8;
    result[6] = (s[2] >> 6) as u8;
    result[7] = ((s[2] >> 14) | (s[3] << 7)) as u8;
    result[8] = (s[3] >> 1) as u8;
    result[9] = (s[3] >> 9) as u8;
    result[10] = ((s[3] >> 17) | (s[4] << 4)) as u8;
    result[11] = (s[4] >> 4) as u8;
    result[12] = (s[4] >> 12) as u8;
    result[13] = ((s[4] >> 20) | (s[5] << 1)) as u8;
    result[14] = (s[5] >> 7) as u8;
    result[15] = ((s[5] >> 15) | (s[6] << 6)) as u8;
    result[16] = (s[6] >> 2) as u8;
    result[17] = (s[6] >> 10) as u8;
    result[18] = ((s[6] >> 18) | (s[7] << 3)) as u8;
    result[19] = (s[7] >> 5) as u8;
    result[20] = (s[7] >> 13) as u8;
    result[21] = s[8] as u8;
    result[22] = (s[8] >> 8) as u8;
    result[23] = ((s[8] >> 16) | (s[9] << 5)) as u8;
    result[24] = (s[9] >> 3) as u8;
    result[25] = (s[9] >> 11) as u8;
    result[26] = ((s[9] >> 19) | (s[10] << 2)) as u8;
    result[27] = (s[10] >> 6) as u8;
    result[28] = ((s[10] >> 14) | (s[11] << 7)) as u8;
    result[29] = (s[11] >> 1) as u8;
    result[30] = (s[11] >> 9) as u8;
    result[31] = (s[11] >> 17) as u8;

    result
}

/// Load 3 bytes as i64 (little-endian)
fn load_3i(s: &[u8]) -> i64 {
    (s[0] as i64) | ((s[1] as i64) << 8) | ((s[2] as i64) << 16)
}

/// Load 4 bytes as i64 (little-endian)
fn load_4i(s: &[u8]) -> i64 {
    (s[0] as i64) | ((s[1] as i64) << 8) | ((s[2] as i64) << 16) | ((s[3] as i64) << 24)
}

/// Scalar reduction helper: reduce s[0..23] mod L
/// L = 2^252 + 27742317777372353535851937790883648493
///
/// In base 2^21:
/// L = [0x1cf5d3ed, 0x009318d2, 0x1de73596, 0x1df3b45c,
///      0x0000014d, 0, 0, 0, 0, 0, 0, 0x00200000]
///
/// We use the standard Ed25519 reference reduction approach.
fn sc_muladd_reduce(s: &mut [i64; 24]) {
    // L in base 2^21 limbs
    // L[0..11] = specific values, L[12] = 2^0 (since 2^(21*12) = 2^252)
    // The L coefficients for the reduction:
    const L0: i64 = 666643;
    const L1: i64 = 470296;
    const L2: i64 = 654183;
    const L3: i64 = -997805;
    const L4: i64 = 136657;
    const L5: i64 = -683901;

    // Reduce from s[23] down to s[12]
    // s[23] contributes at index 23, which is 2^(21*23) = 2^(252+231)
    // We subtract s[23] * L shifted by 11 positions

    s[11] += s[23] * L0;
    s[12] += s[23] * L1;
    s[13] += s[23] * L2;
    s[14] += s[23] * L3;
    s[15] += s[23] * L4;
    s[16] += s[23] * L5;
    s[23] = 0;

    s[10] += s[22] * L0;
    s[11] += s[22] * L1;
    s[12] += s[22] * L2;
    s[13] += s[22] * L3;
    s[14] += s[22] * L4;
    s[15] += s[22] * L5;
    s[22] = 0;

    s[9] += s[21] * L0;
    s[10] += s[21] * L1;
    s[11] += s[21] * L2;
    s[12] += s[21] * L3;
    s[13] += s[21] * L4;
    s[14] += s[21] * L5;
    s[21] = 0;

    s[8] += s[20] * L0;
    s[9] += s[20] * L1;
    s[10] += s[20] * L2;
    s[11] += s[20] * L3;
    s[12] += s[20] * L4;
    s[13] += s[20] * L5;
    s[20] = 0;

    s[7] += s[19] * L0;
    s[8] += s[19] * L1;
    s[9] += s[19] * L2;
    s[10] += s[19] * L3;
    s[11] += s[19] * L4;
    s[12] += s[19] * L5;
    s[19] = 0;

    s[6] += s[18] * L0;
    s[7] += s[18] * L1;
    s[8] += s[18] * L2;
    s[9] += s[18] * L3;
    s[10] += s[18] * L4;
    s[11] += s[18] * L5;
    s[18] = 0;

    // Carry
    let mut carry: i64;
    let mut i = 6;
    while i < 18 {
        carry = (s[i] + (1 << 20)) >> 21;
        s[i + 1] += carry;
        s[i] -= carry << 21;
        i += 1;
    }

    // Second round of reduction for s[12..17]
    s[0] += s[12] * L0;
    s[1] += s[12] * L1;
    s[2] += s[12] * L2;
    s[3] += s[12] * L3;
    s[4] += s[12] * L4;
    s[5] += s[12] * L5;
    s[12] = 0;

    s[1] += s[13] * L0;
    s[2] += s[13] * L1;
    s[3] += s[13] * L2;
    s[4] += s[13] * L3;
    s[5] += s[13] * L4;
    s[6] += s[13] * L5;
    s[13] = 0;

    s[2] += s[14] * L0;
    s[3] += s[14] * L1;
    s[4] += s[14] * L2;
    s[5] += s[14] * L3;
    s[6] += s[14] * L4;
    s[7] += s[14] * L5;
    s[14] = 0;

    s[3] += s[15] * L0;
    s[4] += s[15] * L1;
    s[5] += s[15] * L2;
    s[6] += s[15] * L3;
    s[7] += s[15] * L4;
    s[8] += s[15] * L5;
    s[15] = 0;

    s[4] += s[16] * L0;
    s[5] += s[16] * L1;
    s[6] += s[16] * L2;
    s[7] += s[16] * L3;
    s[8] += s[16] * L4;
    s[9] += s[16] * L5;
    s[16] = 0;

    s[5] += s[17] * L0;
    s[6] += s[17] * L1;
    s[7] += s[17] * L2;
    s[8] += s[17] * L3;
    s[9] += s[17] * L4;
    s[10] += s[17] * L5;
    s[17] = 0;

    // Final carries
    i = 0;
    while i < 12 {
        carry = (s[i] + (1 << 20)) >> 21;
        s[i + 1] += carry;
        s[i] -= carry << 21;
        i += 1;
    }

    // One more reduction pass if needed
    s[0] += s[12] * L0;
    s[1] += s[12] * L1;
    s[2] += s[12] * L2;
    s[3] += s[12] * L3;
    s[4] += s[12] * L4;
    s[5] += s[12] * L5;
    s[12] = 0;

    i = 0;
    while i < 12 {
        carry = s[i] >> 21;
        s[i + 1] += carry;
        s[i] -= carry << 21;
        i += 1;
    }
}

// ============================================================================
// Public API (same signatures as the stubs)
// ============================================================================

/// Signing key (private key)
pub struct SigningKey {
    bytes: [u8; 32],
}

/// Verifying key (public key)
pub struct VerifyingKey {
    bytes: [u8; 32],
}

/// Cryptographic signature
pub struct Signature {
    bytes: [u8; 64],
}

/// Key pair (signing + verifying keys)
pub struct KeyPair {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
}

impl SigningKey {
    /// Create signing key from bytes
    pub fn from_bytes(bytes: &[u8]) -> CryptoResult<Self> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeySize);
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes);

        Ok(Self { bytes: key_bytes })
    }

    /// Get key bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Sign a message using Ed25519 (RFC 8032 Section 5.1.6)
    pub fn sign(&self, message: &[u8]) -> CryptoResult<Signature> {
        // Step 1: Hash the private key
        let h = sha512(&self.bytes);

        // Step 2: Derive the scalar a from first half of hash
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..32]);
        a[0] &= 248; // Clear low 3 bits
        a[31] &= 127; // Clear high bit
        a[31] |= 64; // Set bit 254

        // Step 3: Compute public key A = a * B
        let bp = EdPoint::basepoint();
        let big_a = bp.scalar_mul(&a);
        let a_bytes = big_a.encode();

        // Step 4: Compute nonce r = SHA-512(h[32..64] || message) mod L
        let nonce_hash = sha512_2(&h[32..64], message);
        let r = sc_reduce(&nonce_hash);

        // Step 5: Compute R = r * B
        let big_r = bp.scalar_mul(&r);
        let r_bytes = big_r.encode();

        // Step 6: Compute S = (r + SHA-512(R || A || message) * a) mod L
        let k_hash = sha512_3(&r_bytes, &a_bytes, message);
        let k = sc_reduce(&k_hash);

        // S = r + k * a mod L
        let s = sc_muladd(&k, &a, &r);

        // Build signature: R || S
        let mut sig_bytes = [0u8; 64];
        sig_bytes[..32].copy_from_slice(&r_bytes);
        sig_bytes[32..].copy_from_slice(&s);

        Ok(Signature { bytes: sig_bytes })
    }
}

impl VerifyingKey {
    /// Create verifying key from bytes
    pub fn from_bytes(bytes: &[u8]) -> CryptoResult<Self> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeySize);
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes);

        Ok(Self { bytes: key_bytes })
    }

    /// Get key bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Verify a signature using Ed25519 (RFC 8032 Section 5.1.7)
    pub fn verify(&self, message: &[u8], signature: &Signature) -> CryptoResult<bool> {
        // Parse R and S from signature
        let r_bytes: [u8; 32] = {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&signature.bytes[..32]);
            arr
        };
        let s_bytes: [u8; 32] = {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&signature.bytes[32..]);
            arr
        };

        // Check S < L (group order)
        // S must be less than L = 2^252 + ...
        // Simple check: top byte must have bit 252 clear (approx)
        if s_bytes[31] & 0xf0 != 0 {
            return Ok(false);
        }

        // Decode R
        let big_r = match EdPoint::decode(&r_bytes) {
            Some(p) => p,
            None => return Ok(false),
        };

        // Decode A (public key)
        let big_a = match EdPoint::decode(&self.bytes) {
            Some(p) => p,
            None => return Ok(false),
        };

        // Compute k = SHA-512(R || A || message) mod L
        let k_hash = sha512_3(&r_bytes, &self.bytes, message);
        let k = sc_reduce(&k_hash);

        // Check: S * B == R + k * A
        let bp = EdPoint::basepoint();
        let sb = bp.scalar_mul(&s_bytes);
        let ka = big_a.scalar_mul(&k);
        let rhs = big_r.add(&ka);

        // Compare encoded points
        let lhs_enc = sb.encode();
        let rhs_enc = rhs.encode();

        Ok(lhs_enc == rhs_enc)
    }
}

impl Signature {
    /// Create signature from bytes
    pub fn from_bytes(bytes: &[u8]) -> CryptoResult<Self> {
        if bytes.len() != 64 {
            return Err(CryptoError::InvalidKey);
        }

        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(bytes);

        Ok(Self { bytes: sig_bytes })
    }

    /// Get signature bytes
    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.bytes
    }

    /// Convert to vector
    pub fn to_vec(&self) -> Vec<u8> {
        self.bytes.to_vec()
    }
}

impl KeyPair {
    /// Generate new key pair
    pub fn generate() -> CryptoResult<Self> {
        use super::random::get_random;

        let rng = get_random();
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed)?;

        Self::from_seed(&seed)
    }

    /// Create key pair from seed using Ed25519 key derivation
    pub fn from_seed(seed: &[u8; 32]) -> CryptoResult<Self> {
        let signing_key = SigningKey { bytes: *seed };

        // Derive public key: hash seed, clamp, scalar multiply by basepoint
        let h = sha512(seed);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..32]);
        a[0] &= 248;
        a[31] &= 127;
        a[31] |= 64;

        let bp = EdPoint::basepoint();
        let public_point = bp.scalar_mul(&a);
        let pub_key_bytes = public_point.encode();

        let verifying_key = VerifyingKey {
            bytes: pub_key_bytes,
        };

        Ok(Self {
            signing_key,
            verifying_key,
        })
    }

    /// Sign a message with this key pair
    pub fn sign(&self, message: &[u8]) -> CryptoResult<Signature> {
        self.signing_key.sign(message)
    }

    /// Verify a signature with this key pair
    pub fn verify(&self, message: &[u8], signature: &Signature) -> CryptoResult<bool> {
        self.verifying_key.verify(message, signature)
    }
}

/// Scalar multiply-add: compute (a * b + c) mod L
/// Where L is the Ed25519 group order
fn sc_muladd(a: &[u8; 32], b: &[u8; 32], c: &[u8; 32]) -> [u8; 32] {
    // Load a, b, c into 21-bit limbs
    let a0 = 2097151 & load_3i(&a[0..3]);
    let a1 = 2097151 & (load_4i(&a[2..6]) >> 5);
    let a2 = 2097151 & (load_3i(&a[5..8]) >> 2);
    let a3 = 2097151 & (load_4i(&a[7..11]) >> 7);
    let a4 = 2097151 & (load_4i(&a[10..14]) >> 4);
    let a5 = 2097151 & (load_3i(&a[13..16]) >> 1);
    let a6 = 2097151 & (load_4i(&a[15..19]) >> 6);
    let a7 = 2097151 & (load_3i(&a[18..21]) >> 3);
    let a8 = 2097151 & load_3i(&a[21..24]);
    let a9 = 2097151 & (load_4i(&a[23..27]) >> 5);
    let a10 = 2097151 & (load_3i(&a[26..29]) >> 2);
    let a11 = load_4i(&a[28..32]) >> 7;

    let b0 = 2097151 & load_3i(&b[0..3]);
    let b1 = 2097151 & (load_4i(&b[2..6]) >> 5);
    let b2 = 2097151 & (load_3i(&b[5..8]) >> 2);
    let b3 = 2097151 & (load_4i(&b[7..11]) >> 7);
    let b4 = 2097151 & (load_4i(&b[10..14]) >> 4);
    let b5 = 2097151 & (load_3i(&b[13..16]) >> 1);
    let b6 = 2097151 & (load_4i(&b[15..19]) >> 6);
    let b7 = 2097151 & (load_3i(&b[18..21]) >> 3);
    let b8 = 2097151 & load_3i(&b[21..24]);
    let b9 = 2097151 & (load_4i(&b[23..27]) >> 5);
    let b10 = 2097151 & (load_3i(&b[26..29]) >> 2);
    let b11 = load_4i(&b[28..32]) >> 7;

    let c0 = 2097151 & load_3i(&c[0..3]);
    let c1 = 2097151 & (load_4i(&c[2..6]) >> 5);
    let c2 = 2097151 & (load_3i(&c[5..8]) >> 2);
    let c3 = 2097151 & (load_4i(&c[7..11]) >> 7);
    let c4 = 2097151 & (load_4i(&c[10..14]) >> 4);
    let c5 = 2097151 & (load_3i(&c[13..16]) >> 1);
    let c6 = 2097151 & (load_4i(&c[15..19]) >> 6);
    let c7 = 2097151 & (load_3i(&c[18..21]) >> 3);
    let c8 = 2097151 & load_3i(&c[21..24]);
    let c9 = 2097151 & (load_4i(&c[23..27]) >> 5);
    let c10 = 2097151 & (load_3i(&c[26..29]) >> 2);
    let c11 = load_4i(&c[28..32]) >> 7;

    // Compute s = a*b + c using schoolbook multiplication in 21-bit limbs
    let mut s = [0i64; 24];
    s[0] = c0 + a0 * b0;
    s[1] = c1 + a0 * b1 + a1 * b0;
    s[2] = c2 + a0 * b2 + a1 * b1 + a2 * b0;
    s[3] = c3 + a0 * b3 + a1 * b2 + a2 * b1 + a3 * b0;
    s[4] = c4 + a0 * b4 + a1 * b3 + a2 * b2 + a3 * b1 + a4 * b0;
    s[5] = c5 + a0 * b5 + a1 * b4 + a2 * b3 + a3 * b2 + a4 * b1 + a5 * b0;
    s[6] = c6 + a0 * b6 + a1 * b5 + a2 * b4 + a3 * b3 + a4 * b2 + a5 * b1 + a6 * b0;
    s[7] = c7 + a0 * b7 + a1 * b6 + a2 * b5 + a3 * b4 + a4 * b3 + a5 * b2 + a6 * b1 + a7 * b0;
    s[8] = c8
        + a0 * b8
        + a1 * b7
        + a2 * b6
        + a3 * b5
        + a4 * b4
        + a5 * b3
        + a6 * b2
        + a7 * b1
        + a8 * b0;
    s[9] = c9
        + a0 * b9
        + a1 * b8
        + a2 * b7
        + a3 * b6
        + a4 * b5
        + a5 * b4
        + a6 * b3
        + a7 * b2
        + a8 * b1
        + a9 * b0;
    s[10] = c10
        + a0 * b10
        + a1 * b9
        + a2 * b8
        + a3 * b7
        + a4 * b6
        + a5 * b5
        + a6 * b4
        + a7 * b3
        + a8 * b2
        + a9 * b1
        + a10 * b0;
    s[11] = c11
        + a0 * b11
        + a1 * b10
        + a2 * b9
        + a3 * b8
        + a4 * b7
        + a5 * b6
        + a6 * b5
        + a7 * b4
        + a8 * b3
        + a9 * b2
        + a10 * b1
        + a11 * b0;
    s[12] = a1 * b11
        + a2 * b10
        + a3 * b9
        + a4 * b8
        + a5 * b7
        + a6 * b6
        + a7 * b5
        + a8 * b4
        + a9 * b3
        + a10 * b2
        + a11 * b1;
    s[13] = a2 * b11
        + a3 * b10
        + a4 * b9
        + a5 * b8
        + a6 * b7
        + a7 * b6
        + a8 * b5
        + a9 * b4
        + a10 * b3
        + a11 * b2;
    s[14] =
        a3 * b11 + a4 * b10 + a5 * b9 + a6 * b8 + a7 * b7 + a8 * b6 + a9 * b5 + a10 * b4 + a11 * b3;
    s[15] = a4 * b11 + a5 * b10 + a6 * b9 + a7 * b8 + a8 * b7 + a9 * b6 + a10 * b5 + a11 * b4;
    s[16] = a5 * b11 + a6 * b10 + a7 * b9 + a8 * b8 + a9 * b7 + a10 * b6 + a11 * b5;
    s[17] = a6 * b11 + a7 * b10 + a8 * b9 + a9 * b8 + a10 * b7 + a11 * b6;
    s[18] = a7 * b11 + a8 * b10 + a9 * b9 + a10 * b8 + a11 * b7;
    s[19] = a8 * b11 + a9 * b10 + a10 * b9 + a11 * b8;
    s[20] = a9 * b11 + a10 * b10 + a11 * b9;
    s[21] = a10 * b11 + a11 * b10;
    s[22] = a11 * b11;
    s[23] = 0;

    // Reduce mod L
    sc_muladd_reduce(&mut s);

    // Pack result
    let mut result = [0u8; 32];
    result[0] = s[0] as u8;
    result[1] = (s[0] >> 8) as u8;
    result[2] = ((s[0] >> 16) | (s[1] << 5)) as u8;
    result[3] = (s[1] >> 3) as u8;
    result[4] = (s[1] >> 11) as u8;
    result[5] = ((s[1] >> 19) | (s[2] << 2)) as u8;
    result[6] = (s[2] >> 6) as u8;
    result[7] = ((s[2] >> 14) | (s[3] << 7)) as u8;
    result[8] = (s[3] >> 1) as u8;
    result[9] = (s[3] >> 9) as u8;
    result[10] = ((s[3] >> 17) | (s[4] << 4)) as u8;
    result[11] = (s[4] >> 4) as u8;
    result[12] = (s[4] >> 12) as u8;
    result[13] = ((s[4] >> 20) | (s[5] << 1)) as u8;
    result[14] = (s[5] >> 7) as u8;
    result[15] = ((s[5] >> 15) | (s[6] << 6)) as u8;
    result[16] = (s[6] >> 2) as u8;
    result[17] = (s[6] >> 10) as u8;
    result[18] = ((s[6] >> 18) | (s[7] << 3)) as u8;
    result[19] = (s[7] >> 5) as u8;
    result[20] = (s[7] >> 13) as u8;
    result[21] = s[8] as u8;
    result[22] = (s[8] >> 8) as u8;
    result[23] = ((s[8] >> 16) | (s[9] << 5)) as u8;
    result[24] = (s[9] >> 3) as u8;
    result[25] = (s[9] >> 11) as u8;
    result[26] = ((s[9] >> 19) | (s[10] << 2)) as u8;
    result[27] = (s[10] >> 6) as u8;
    result[28] = ((s[10] >> 14) | (s[11] << 7)) as u8;
    result[29] = (s[11] >> 1) as u8;
    result[30] = (s[11] >> 9) as u8;
    result[31] = (s[11] >> 17) as u8;

    result
}

// ============================================================================
// X25519 Key Exchange (RFC 7748)
// ============================================================================

/// X25519 scalar multiplication on the Montgomery form of Curve25519
/// Uses the Montgomery ladder for constant-time computation
fn x25519_scalar_mult(scalar: &[u8; 32], u_point: &[u8; 32]) -> [u8; 32] {
    // Clamp scalar per RFC 7748 Section 5
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    // Decode u-coordinate (clear top bit)
    let mut u_bytes = *u_point;
    u_bytes[31] &= 0x7f;
    let u = Fe::from_bytes(&u_bytes);

    // Montgomery ladder
    let x_1 = u;
    let mut x_2 = Fe::ONE;
    let mut z_2 = Fe::ZERO;
    let mut x_3 = u;
    let mut z_3 = Fe::ONE;
    let mut swap: u64 = 0;

    let a24 = Fe([121666, 0, 0, 0, 0]); // (A+2)/4 where A=486662

    let mut t: i32 = 254;
    while t >= 0 {
        let k_t = ((k[(t >> 3) as usize] >> (t & 7)) & 1) as u64;
        swap ^= k_t;
        x_2.cswap(&mut x_3, swap);
        z_2.cswap(&mut z_3, swap);
        swap = k_t;

        let a = x_2.add(&z_2);
        let aa = a.square();
        let b = x_2.sub(&z_2);
        let bb = b.square();
        let e = aa.sub(&bb);
        let c = x_3.add(&z_3);
        let d = x_3.sub(&z_3);
        let da = d.mul(&a);
        let cb = c.mul(&b);
        x_3 = da.add(&cb).square();
        z_3 = x_1.mul(&da.sub(&cb).square());
        x_2 = aa.mul(&bb);
        z_2 = e.mul(&aa.add(&a24.mul(&e)));

        t -= 1;
    }

    x_2.cswap(&mut x_3, swap);
    z_2.cswap(&mut z_3, swap);

    // Return x_2 / z_2
    let result = x_2.mul(&z_2.invert());
    result.to_bytes()
}

/// X25519 base point (u=9)
const X25519_BASEPOINT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

/// X25519 key exchange
pub mod key_exchange {
    use super::CryptoResult;

    /// Public key for key exchange
    pub struct PublicKey {
        bytes: [u8; 32],
    }

    /// Secret key for key exchange
    pub struct SecretKey {
        bytes: [u8; 32],
    }

    /// Shared secret
    pub struct SharedSecret {
        bytes: [u8; 32],
    }

    impl PublicKey {
        /// Create from bytes
        pub fn from_bytes(bytes: &[u8; 32]) -> Self {
            Self { bytes: *bytes }
        }

        /// Get bytes
        pub fn as_bytes(&self) -> &[u8; 32] {
            &self.bytes
        }
    }

    impl SecretKey {
        /// Generate new secret key
        pub fn generate() -> CryptoResult<Self> {
            use super::super::random::get_random;

            let rng = get_random();
            let mut bytes = [0u8; 32];
            rng.fill_bytes(&mut bytes)?;

            Ok(Self { bytes })
        }

        /// Get corresponding public key using X25519
        /// public_key = scalar_mult(secret, basepoint)
        pub fn public_key(&self) -> PublicKey {
            let pub_bytes = super::x25519_scalar_mult(&self.bytes, &super::X25519_BASEPOINT);
            PublicKey { bytes: pub_bytes }
        }

        /// Perform X25519 key exchange
        /// shared_secret = scalar_mult(my_secret, their_public)
        pub fn exchange(&self, their_public: &PublicKey) -> CryptoResult<SharedSecret> {
            let shared = super::x25519_scalar_mult(&self.bytes, &their_public.bytes);

            // Check for all-zero output (low-order point)
            let mut is_zero = true;
            let mut i = 0;
            while i < 32 {
                if shared[i] != 0 {
                    is_zero = false;
                }
                i += 1;
            }
            if is_zero {
                return Err(super::CryptoError::InvalidKey);
            }

            Ok(SharedSecret { bytes: shared })
        }
    }

    impl SharedSecret {
        /// Get shared secret bytes
        pub fn as_bytes(&self) -> &[u8; 32] {
            &self.bytes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_keypair_generation() {
        let keypair = KeyPair::generate().unwrap();
        let message = b"Hello, VeridianOS!";

        let signature = keypair.sign(message).unwrap();
        let verified = keypair.verify(message, &signature).unwrap();

        assert!(verified);
    }

    #[test_case]
    fn test_key_exchange() {
        use key_exchange::*;

        let alice_secret = SecretKey::generate().unwrap();
        let alice_public = alice_secret.public_key();

        let bob_secret = SecretKey::generate().unwrap();
        let bob_public = bob_secret.public_key();

        let alice_shared = alice_secret.exchange(&bob_public).unwrap();
        let bob_shared = bob_secret.exchange(&alice_public).unwrap();

        // Shared secrets should match
        assert_eq!(alice_shared.as_bytes(), bob_shared.as_bytes());
    }
}
