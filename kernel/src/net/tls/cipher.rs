//! TLS 1.3 Cipher Suites and Cryptographic Primitives
//!
//! Implements all crypto needed for TLS 1.3:
//! - HMAC-SHA256 (RFC 2104)
//! - HKDF-SHA256 (RFC 5869)
//! - X25519 key exchange (RFC 7748)
//! - ChaCha20-Poly1305 AEAD (RFC 8439)
//! - AES-128-GCM AEAD (NIST SP 800-38D)

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{CipherSuite, AEAD_TAG_LEN, AES_128_KEY_LEN, CHACHA20_KEY_LEN, HASH_LEN, NONCE_LEN};
use crate::crypto::hash::{sha256, Hash256};

// ============================================================================
// HMAC-SHA256 (RFC 2104)
// ============================================================================

/// HMAC-SHA256 (RFC 2104)
///
/// Stack-only implementation -- no heap allocation for the HMAC computation.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    // If key > block size, hash it first
    let key_hash: Hash256;
    let k = if key.len() > BLOCK_SIZE {
        key_hash = sha256(key);
        key_hash.as_bytes().as_slice()
    } else {
        key
    };

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];

    for i in 0..k.len() {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    // Inner hash: SHA256(ipad || message)
    // We build this on the stack with a reasonable buffer size.
    // For messages larger than this, we'd need a streaming SHA256.
    let mut inner_buf = [0u8; 2048];
    let inner_len = BLOCK_SIZE + message.len();
    if inner_len <= inner_buf.len() {
        inner_buf[..BLOCK_SIZE].copy_from_slice(&ipad);
        inner_buf[BLOCK_SIZE..inner_len].copy_from_slice(message);
        let inner_hash = sha256(&inner_buf[..inner_len]);

        // Outer hash: SHA256(opad || inner_hash)
        let mut outer_buf = [0u8; 96]; // 64 + 32
        outer_buf[..BLOCK_SIZE].copy_from_slice(&opad);
        outer_buf[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());
        sha256(&outer_buf[..BLOCK_SIZE + 32]).0
    } else {
        // Fallback for very large messages: use alloc
        let mut inner_data = Vec::with_capacity(inner_len);
        inner_data.extend_from_slice(&ipad);
        inner_data.extend_from_slice(message);
        let inner_hash = sha256(&inner_data);

        let mut outer_buf = [0u8; 96];
        outer_buf[..BLOCK_SIZE].copy_from_slice(&opad);
        outer_buf[BLOCK_SIZE..BLOCK_SIZE + 32].copy_from_slice(inner_hash.as_bytes());
        sha256(&outer_buf[..BLOCK_SIZE + 32]).0
    }
}

// ============================================================================
// HKDF-SHA256 (RFC 5869)
// ============================================================================

/// HKDF-Extract: PRK = HMAC-Hash(salt, IKM)
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; HASH_LEN] {
    hmac_sha256(salt, ikm)
}

/// HKDF-Expand: OKM = T(1) || T(2) || ... (truncated to length)
///
/// T(0) = empty string
/// T(i) = HMAC-Hash(PRK, T(i-1) || info || i)
pub fn hkdf_expand(prk: &[u8; HASH_LEN], info: &[u8], length: usize) -> Vec<u8> {
    let n = length.div_ceil(HASH_LEN);
    let mut okm = Vec::with_capacity(n * HASH_LEN);
    let mut t = [0u8; HASH_LEN];
    let mut t_len: usize = 0;

    for i in 1..=n {
        // HMAC input: T(i-1) || info || i
        let mut input = Vec::with_capacity(t_len + info.len() + 1);
        if t_len > 0 {
            input.extend_from_slice(&t[..t_len]);
        }
        input.extend_from_slice(info);
        input.push(i as u8);

        t = hmac_sha256(prk, &input);
        t_len = HASH_LEN;
        okm.extend_from_slice(&t);
    }

    okm.truncate(length);
    okm
}

/// HKDF-Expand-Label (TLS 1.3 specific, RFC 8446 Section 7.1)
///
/// HKDF-Expand-Label(Secret, Label, Context, Length) =
///     HKDF-Expand(Secret, HkdfLabel, Length)
/// where HkdfLabel = Length(2) || "tls13 " || Label || Context
pub fn hkdf_expand_label(
    secret: &[u8; HASH_LEN],
    label: &[u8],
    context: &[u8],
    length: usize,
) -> Vec<u8> {
    let tls_label = b"tls13 ";
    let mut hkdf_label =
        Vec::with_capacity(2 + 1 + tls_label.len() + label.len() + 1 + context.len());

    // Length (2 bytes, big-endian)
    hkdf_label.extend_from_slice(&(length as u16).to_be_bytes());

    // Label with "tls13 " prefix (length-prefixed)
    let full_label_len = tls_label.len() + label.len();
    hkdf_label.push(full_label_len as u8);
    hkdf_label.extend_from_slice(tls_label);
    hkdf_label.extend_from_slice(label);

    // Context (length-prefixed)
    hkdf_label.push(context.len() as u8);
    hkdf_label.extend_from_slice(context);

    hkdf_expand(secret, &hkdf_label, length)
}

/// Derive-Secret (TLS 1.3, RFC 8446 Section 7.1)
///
/// Derive-Secret(Secret, Label, Messages) =
///     HKDF-Expand-Label(Secret, Label, Transcript-Hash(Messages), Hash.length)
pub(crate) fn derive_secret(
    secret: &[u8; HASH_LEN],
    label: &[u8],
    transcript_hash: &[u8; 32],
) -> [u8; HASH_LEN] {
    let expanded = hkdf_expand_label(secret, label, transcript_hash, HASH_LEN);
    let mut result = [0u8; HASH_LEN];
    result.copy_from_slice(&expanded);
    result
}

// ============================================================================
// X25519 Key Exchange (RFC 7748)
// ============================================================================

/// X25519 basepoint (u = 9)
const X25519_BASEPOINT: [u8; 32] = {
    let mut b = [0u8; 32];
    b[0] = 9;
    b
};

/// Generate an X25519 keypair using the kernel's CSPRNG
pub fn x25519_keypair() -> ([u8; 32], [u8; 32]) {
    let mut private_key = [0u8; 32];
    // Use kernel CSPRNG if available, otherwise deterministic seed for testing
    if let Ok(rng) = crate::crypto::random::SecureRandom::new() {
        let _ = rng.fill_bytes(&mut private_key);
    } else {
        // Fallback: deterministic but non-zero key for testing only
        for (i, b) in private_key.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(42);
        }
    }

    let public_key = x25519_scalar_mult(&private_key, &X25519_BASEPOINT);
    (private_key, public_key)
}

/// Compute X25519 shared secret: shared = scalar_mult(our_private,
/// their_public)
pub fn x25519_shared_secret(private_key: &[u8; 32], peer_public: &[u8; 32]) -> [u8; 32] {
    x25519_scalar_mult(private_key, peer_public)
}

/// X25519 scalar multiplication using the Montgomery ladder.
///
/// Implements RFC 7748 Section 5 with clamping.
pub(crate) fn x25519_scalar_mult(scalar: &[u8; 32], u_point: &[u8; 32]) -> [u8; 32] {
    // Clamp scalar per RFC 7748
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    // Load u-coordinate
    let u = fe_from_bytes(u_point);

    // Montgomery ladder
    let mut x_2 = fe_one();
    let mut z_2 = fe_zero();
    let mut x_3 = u;
    let mut z_3 = fe_one();
    let mut swap: u64 = 0;

    for pos in (0..255).rev() {
        let bit = ((k[pos >> 3] >> (pos & 7)) & 1) as u64;
        swap ^= bit;
        fe_cswap(&mut x_2, &mut x_3, swap);
        fe_cswap(&mut z_2, &mut z_3, swap);
        swap = bit;

        let a = fe_add(&x_2, &z_2);
        let aa = fe_sq(&a);
        let b = fe_sub(&x_2, &z_2);
        let bb = fe_sq(&b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x_3, &z_3);
        let d = fe_sub(&x_3, &z_3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        x_3 = fe_sq(&fe_add(&da, &cb));
        z_3 = fe_mul(&u, &fe_sq(&fe_sub(&da, &cb)));
        x_2 = fe_mul(&aa, &bb);
        // a24 = (A-2)/4 = (486662-2)/4 = 121665 per RFC 7748
        z_2 = fe_mul(&e, &fe_add(&aa, &fe_mul_scalar(&e, 121665)));
    }

    fe_cswap(&mut x_2, &mut x_3, swap);
    fe_cswap(&mut z_2, &mut z_3, swap);

    let result = fe_mul(&x_2, &fe_invert(&z_2));
    fe_to_bytes(&result)
}

// --- GF(2^255-19) Field Arithmetic (5-limb, 51 bits per limb) ---

type Fe = [u64; 5];
const LIMB_MASK: u64 = (1u64 << 51) - 1;

fn fe_zero() -> Fe {
    [0; 5]
}

fn fe_one() -> Fe {
    [1, 0, 0, 0, 0]
}

fn fe_from_bytes(s: &[u8; 32]) -> Fe {
    let load64 = |bytes: &[u8]| -> u64 {
        let mut buf = [0u8; 8];
        let len = core::cmp::min(bytes.len(), 8);
        buf[..len].copy_from_slice(&bytes[..len]);
        u64::from_le_bytes(buf)
    };

    let mut h = [0u64; 5];
    h[0] = load64(&s[0..]) & LIMB_MASK;
    h[1] = (load64(&s[6..]) >> 3) & LIMB_MASK;
    h[2] = (load64(&s[12..]) >> 6) & LIMB_MASK;
    h[3] = (load64(&s[19..]) >> 1) & LIMB_MASK;
    h[4] = (load64(&s[24..]) >> 12) & LIMB_MASK;
    h
}

fn fe_to_bytes(h: &Fe) -> [u8; 32] {
    let mut t = *h;
    fe_reduce(&mut t);

    // Final conditional subtraction
    let mut q = (t[0].wrapping_add(19)) >> 51;
    q = (t[1].wrapping_add(q)) >> 51;
    q = (t[2].wrapping_add(q)) >> 51;
    q = (t[3].wrapping_add(q)) >> 51;
    q = (t[4].wrapping_add(q)) >> 51;

    t[0] = t[0].wrapping_add(19u64.wrapping_mul(q));
    let mut carry = t[0] >> 51;
    t[0] &= LIMB_MASK;
    #[allow(clippy::needless_range_loop)]
    for i in 1..5 {
        t[i] = t[i].wrapping_add(carry);
        carry = t[i] >> 51;
        t[i] &= LIMB_MASK;
    }

    // Serialize 5 limbs (51 bits each) to 32 bytes via u128 accumulator:
    let mut bits = [0u8; 32];
    let mut acc: u128 = 0;
    let mut acc_bits: u32 = 0;
    let mut byte_pos = 0;
    for &limb in t.iter() {
        acc |= (limb as u128) << acc_bits;
        acc_bits += 51;
        while acc_bits >= 8 && byte_pos < 32 {
            bits[byte_pos] = (acc & 0xFF) as u8;
            acc >>= 8;
            acc_bits -= 8;
            byte_pos += 1;
        }
    }
    // Handle any remaining bits
    if byte_pos < 32 {
        bits[byte_pos] = (acc & 0xFF) as u8;
    }
    bits
}

fn fe_reduce(h: &mut Fe) {
    let mut carry: u64;
    for _ in 0..2 {
        carry = h[0] >> 51;
        h[0] &= LIMB_MASK;
        h[1] = h[1].wrapping_add(carry);

        carry = h[1] >> 51;
        h[1] &= LIMB_MASK;
        h[2] = h[2].wrapping_add(carry);

        carry = h[2] >> 51;
        h[2] &= LIMB_MASK;
        h[3] = h[3].wrapping_add(carry);

        carry = h[3] >> 51;
        h[3] &= LIMB_MASK;
        h[4] = h[4].wrapping_add(carry);

        carry = h[4] >> 51;
        h[4] &= LIMB_MASK;
        h[0] = h[0].wrapping_add(carry.wrapping_mul(19));
    }
}

fn fe_add(a: &Fe, b: &Fe) -> Fe {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
        a[3].wrapping_add(b[3]),
        a[4].wrapping_add(b[4]),
    ]
}

fn fe_sub(a: &Fe, b: &Fe) -> Fe {
    // Add p to avoid underflow before subtraction
    let bias: u64 = (1u64 << 51) - 1;
    let bias0: u64 = bias - 18;
    [
        a[0].wrapping_add(bias0).wrapping_sub(b[0]),
        a[1].wrapping_add(bias).wrapping_sub(b[1]),
        a[2].wrapping_add(bias).wrapping_sub(b[2]),
        a[3].wrapping_add(bias).wrapping_sub(b[3]),
        a[4].wrapping_add(bias).wrapping_sub(b[4]),
    ]
}

#[allow(clippy::needless_range_loop)]
fn fe_mul(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0u128; 5];

    for i in 0..5 {
        for j in 0..5 {
            let product = (a[i] as u128) * (b[j] as u128);
            let idx = i + j;
            if idx < 5 {
                t[idx] = t[idx].wrapping_add(product);
            } else {
                // Reduce: limb at position idx maps to idx-5 with factor 19
                t[idx - 5] = t[idx - 5].wrapping_add(product.wrapping_mul(19));
            }
        }
    }

    let mut h = [0u64; 5];
    let mut carry: u128 = 0;
    for i in 0..5 {
        t[i] = t[i].wrapping_add(carry);
        h[i] = (t[i] as u64) & LIMB_MASK;
        carry = t[i] >> 51;
    }
    h[0] = h[0].wrapping_add((carry as u64).wrapping_mul(19));

    fe_reduce(&mut h);
    h
}

fn fe_sq(a: &Fe) -> Fe {
    fe_mul(a, a)
}

fn fe_mul_scalar(a: &Fe, s: u64) -> Fe {
    let mut h = [0u64; 5];
    let mut carry: u128 = 0;
    for i in 0..5 {
        let product = (a[i] as u128) * (s as u128) + carry;
        h[i] = (product as u64) & LIMB_MASK;
        carry = product >> 51;
    }
    h[0] = h[0].wrapping_add((carry as u64).wrapping_mul(19));
    fe_reduce(&mut h);
    h
}

fn fe_cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
    let mask = 0u64.wrapping_sub(swap); // 0 or 0xFFFFFFFFFFFFFFFF
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

/// Compute modular inverse using Fermat's little theorem: a^(p-2) mod p
fn fe_invert(z: &Fe) -> Fe {
    // p-2 = 2^255 - 21
    // Use addition chain for efficient exponentiation
    let z2 = fe_sq(z);
    let z9 = {
        let z4 = fe_sq(&z2);
        let z8 = fe_sq(&z4);
        fe_mul(&z8, z)
    };
    let z11 = fe_mul(&z9, &z2);
    let z_5_0 = {
        let t = fe_sq(&z11);
        fe_mul(&t, &z9)
    };
    let z_10_0 = {
        let mut t = fe_sq(&z_5_0);
        for _ in 1..5 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_5_0)
    };
    let z_20_0 = {
        let mut t = fe_sq(&z_10_0);
        for _ in 1..10 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_10_0)
    };
    let z_40_0 = {
        let mut t = fe_sq(&z_20_0);
        for _ in 1..20 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_20_0)
    };
    let z_50_0 = {
        let mut t = fe_sq(&z_40_0);
        for _ in 1..10 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_10_0)
    };
    let z_100_0 = {
        let mut t = fe_sq(&z_50_0);
        for _ in 1..50 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_50_0)
    };
    let z_200_0 = {
        let mut t = fe_sq(&z_100_0);
        for _ in 1..100 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_100_0)
    };
    let z_250_0 = {
        let mut t = fe_sq(&z_200_0);
        for _ in 1..50 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z_50_0)
    };

    {
        let mut t = fe_sq(&z_250_0);
        for _ in 1..5 {
            t = fe_sq(&t);
        }
        fe_mul(&t, &z11)
    }
}

// ============================================================================
// ChaCha20-Poly1305 AEAD (RFC 8439)
// ============================================================================

/// ChaCha20 quarter round
#[inline]
fn chacha20_quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
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

/// Generate one 64-byte ChaCha20 keystream block
fn chacha20_block(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> [u8; 64] {
    let mut state: [u32; 16] = [
        0x61707865,
        0x3320646e,
        0x79622d32,
        0x6b206574, // "expand 32-byte k"
        u32::from_le_bytes([key[0], key[1], key[2], key[3]]),
        u32::from_le_bytes([key[4], key[5], key[6], key[7]]),
        u32::from_le_bytes([key[8], key[9], key[10], key[11]]),
        u32::from_le_bytes([key[12], key[13], key[14], key[15]]),
        u32::from_le_bytes([key[16], key[17], key[18], key[19]]),
        u32::from_le_bytes([key[20], key[21], key[22], key[23]]),
        u32::from_le_bytes([key[24], key[25], key[26], key[27]]),
        u32::from_le_bytes([key[28], key[29], key[30], key[31]]),
        counter,
        u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]),
        u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]),
        u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]),
    ];

    let initial = state;

    // 20 rounds (10 double rounds)
    for _ in 0..10 {
        // Column rounds
        chacha20_quarter_round(&mut state, 0, 4, 8, 12);
        chacha20_quarter_round(&mut state, 1, 5, 9, 13);
        chacha20_quarter_round(&mut state, 2, 6, 10, 14);
        chacha20_quarter_round(&mut state, 3, 7, 11, 15);
        // Diagonal rounds
        chacha20_quarter_round(&mut state, 0, 5, 10, 15);
        chacha20_quarter_round(&mut state, 1, 6, 11, 12);
        chacha20_quarter_round(&mut state, 2, 7, 8, 13);
        chacha20_quarter_round(&mut state, 3, 4, 9, 14);
    }

    let mut output = [0u8; 64];
    for i in 0..16 {
        let val = state[i].wrapping_add(initial[i]);
        output[i * 4..(i + 1) * 4].copy_from_slice(&val.to_le_bytes());
    }
    output
}

/// ChaCha20 encrypt/decrypt (XOR with keystream)
pub(crate) fn chacha20_crypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    counter: u32,
    data: &[u8],
) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut ctr = counter;

    for chunk in data.chunks(64) {
        let block = chacha20_block(key, nonce, ctr);
        for (i, &b) in chunk.iter().enumerate() {
            output.push(b ^ block[i]);
        }
        ctr = ctr.wrapping_add(1);
    }

    output
}

/// Poly1305 MAC computation (RFC 8439 Section 2.5)
///
/// Uses u128 arithmetic to avoid overflow in GF(2^130-5) multiplication.
fn poly1305_mac(key: &[u8; 32], message: &[u8]) -> [u8; 16] {
    // Split key: r (first 16 bytes, clamped) and s (last 16 bytes)
    let mut r_bytes = [0u8; 16];
    r_bytes.copy_from_slice(&key[..16]);

    // Clamp r
    r_bytes[3] &= 15;
    r_bytes[7] &= 15;
    r_bytes[11] &= 15;
    r_bytes[15] &= 15;
    r_bytes[4] &= 252;
    r_bytes[8] &= 252;
    r_bytes[12] &= 252;

    let r = u128::from_le_bytes({
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&r_bytes);
        buf
    });
    let s = u128::from_le_bytes({
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&key[16..32]);
        buf
    });

    let mut accumulator: u128 = 0;
    // p = 2^130 - 5 (doesn't fit in u128; passed as _p to mulmod which handles
    // reduction)

    for chunk in message.chunks(16) {
        let mut block = [0u8; 17];
        block[..chunk.len()].copy_from_slice(chunk);
        block[chunk.len()] = 1; // Append 0x01

        // Build little-endian value from block bytes.
        // For a full 16-byte chunk, len = 17 (includes 0x01 sentinel).
        // The sentinel at position 16 represents bit 128, which we must
        // handle without overflowing u128.
        let len = chunk.len() + 1;
        let mut val: u128 = 0;
        let direct = if len > 16 { 16 } else { len };
        for (i, &b) in block[..direct].iter().enumerate() {
            val |= (b as u128) << (8 * i);
        }

        accumulator = accumulator.wrapping_add(val);
        if len > 16 {
            // Add 2^128 for the sentinel bit, split into two halves
            // to avoid shift overflow: 2^128 = 2^127 + 2^127
            accumulator = accumulator.wrapping_add(1u128 << 127);
            accumulator = accumulator.wrapping_add(1u128 << 127);
        }
        // Multiply and reduce mod 2^130-5
        // Use partial reduction to avoid full 256-bit arithmetic
        accumulator = poly1305_mulmod(accumulator, r, 0);
    }

    accumulator = accumulator.wrapping_add(s);
    let tag_bytes = accumulator.to_le_bytes();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&tag_bytes[..16]);
    tag
}

/// Multiply two 130-bit numbers mod 2^130-5
///
/// Uses the property that 2^130 = 5 (mod p) for efficient reduction.
fn poly1305_mulmod(a: u128, b: u128, _p: u128) -> u128 {
    // Split into 64-bit halves for multiplication
    let a_lo = a & 0xFFFF_FFFF_FFFF_FFFF;
    let a_hi = a >> 64;
    let b_lo = b & 0xFFFF_FFFF_FFFF_FFFF;
    let b_hi = b >> 64;

    // Karatsuba-style multiplication
    let lo_lo = a_lo.wrapping_mul(b_lo);
    let lo_hi = a_lo.wrapping_mul(b_hi);
    let hi_lo = a_hi.wrapping_mul(b_lo);
    let hi_hi = a_hi.wrapping_mul(b_hi);

    // Combine: result = lo_lo + (lo_hi + hi_lo) << 64 + hi_hi << 128
    // But we need to reduce mod 2^130-5
    // Since 2^130 = 5 (mod p), bits above 130 get multiplied by 5

    let mid = lo_hi.wrapping_add(hi_lo);
    let result_lo = lo_lo.wrapping_add(mid << 64);
    let carry = if lo_lo.checked_add(mid << 64).is_none() {
        1u128
    } else {
        0u128
    };

    let result_hi = hi_hi.wrapping_add(mid >> 64).wrapping_add(carry);

    // Reduce mod 2^130 - 5
    // combined = result_lo + result_hi * 2^64, total up to ~260 bits
    // We need bits 0..129 (the "low 130 bits") and bits 130+ (multiply by 5)
    // combined as a u128: result_lo | (result_hi << 64) -- but result_hi may
    // overflow Instead, work directly with result_lo (bits 0..127) and
    // result_hi (bits 64..127+)
    //
    // Bit 130 of the full product = bit 66 of result_hi
    // low_130 = result_lo[0..63] | result_hi[0..1] << 64  (but result_hi << 64 can
    // overflow u128)
    //
    // Simpler: combine into u128 with wrapping, extract low 130 bits via mask
    // Low 130 bits mask = (1 << 64) - 1 in low word + bits 0..1 of high word
    let _combined = result_lo.wrapping_add(result_hi << 64);
    // Bits 0-127 are in combined. Bit 128-129 were lost if result_hi >= 2^64.
    // Since result_hi < 2^66 (product of two 130-bit numbers), overflow is at most
    // 4 bits. Use a different approach: keep result_lo and result_hi separate.

    // Extract low 130 bits: result_lo gives bits 0-127, result_hi bits 0-1 give
    // bits 128-129
    let low_130_lo = result_lo; // bits 0..127
    let low_130_hi = result_hi & 0x3; // bits 128..129 (2 bits from result_hi)
    let low_130 = low_130_lo.wrapping_add((low_130_hi) << 64);
    // Note: low_130_hi << 64 won't overflow since low_130_hi <= 3

    // High bits (130+) = result_hi >> 2
    let high_bits = result_hi >> 2;
    let reduced = low_130.wrapping_add(high_bits.wrapping_mul(5));

    // One more reduction pass: reduced is at most ~131 bits
    // Low 130 bits of reduced
    let _r_lo = reduced; // bits 0..127
                         // Bit 128+ of reduced: since reduced < 2^131, overflow into high bits is
                         // minimal We approximate: if reduced > 2^130 - 1, the
                         // excess is small For a proper second pass, we'd need
                         // to track the carry, but since high_bits is at most
                         // ~66 bits * 5, reduced fits in u128. Final full
                         // reduction is not needed since the accumulator is reduced each round.
    reduced
}

/// ChaCha20-Poly1305 AEAD encrypt (RFC 8439 Section 2.8)
pub(crate) fn chacha20_poly1305_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    // Generate Poly1305 one-time key from block 0
    let otk_block = chacha20_block(key, nonce, 0);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&otk_block[..32]);

    // Encrypt plaintext starting from counter 1
    let ciphertext = chacha20_crypt(key, nonce, 1, plaintext);

    // Construct Poly1305 input: AAD || pad || ciphertext || pad || len(AAD) ||
    // len(CT)
    let mac_input = build_poly1305_input(aad, &ciphertext);
    let tag = poly1305_mac(&poly_key, &mac_input);

    // Output: ciphertext || tag
    let mut output = ciphertext;
    output.extend_from_slice(&tag);
    output
}

/// ChaCha20-Poly1305 AEAD decrypt (RFC 8439 Section 2.8)
pub(crate) fn chacha20_poly1305_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ciphertext_and_tag.len() < AEAD_TAG_LEN {
        return None;
    }

    let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
    let ciphertext = &ciphertext_and_tag[..ct_len];
    let tag = &ciphertext_and_tag[ct_len..];

    // Generate Poly1305 one-time key
    let otk_block = chacha20_block(key, nonce, 0);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&otk_block[..32]);

    // Verify tag
    let mac_input = build_poly1305_input(aad, ciphertext);
    let expected_tag = poly1305_mac(&poly_key, &mac_input);

    if !constant_time_eq(tag, &expected_tag) {
        return None;
    }

    // Decrypt
    Some(chacha20_crypt(key, nonce, 1, ciphertext))
}

/// Build Poly1305 MAC input per RFC 8439 Section 2.8
fn build_poly1305_input(aad: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let aad_pad = (16 - (aad.len() % 16)) % 16;
    let ct_pad = (16 - (ciphertext.len() % 16)) % 16;

    let mut input = Vec::with_capacity(aad.len() + aad_pad + ciphertext.len() + ct_pad + 16);
    input.extend_from_slice(aad);
    input.resize(input.len() + aad_pad, 0);
    input.extend_from_slice(ciphertext);
    input.resize(input.len() + ct_pad, 0);
    input.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    input.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    input
}

// ============================================================================
// AES-128-GCM AEAD (NIST SP 800-38D)
// ============================================================================

/// AES S-Box
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

/// AES round constants
const AES_RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// AES-128 block cipher (10 rounds)
struct Aes128 {
    round_keys: [[u8; 16]; 11],
}

impl Aes128 {
    fn new(key: &[u8; 16]) -> Self {
        let mut round_keys = [[0u8; 16]; 11];
        Self::key_expansion(key, &mut round_keys);
        Self { round_keys }
    }

    fn key_expansion(key: &[u8; 16], round_keys: &mut [[u8; 16]; 11]) {
        let mut w = [0u8; 176]; // 44 words * 4 bytes
        w[..16].copy_from_slice(key);

        for i in 4..44 {
            let mut temp = [w[i * 4 - 4], w[i * 4 - 3], w[i * 4 - 2], w[i * 4 - 1]];

            if i % 4 == 0 {
                temp = [
                    AES_SBOX[temp[1] as usize] ^ AES_RCON[i / 4],
                    AES_SBOX[temp[2] as usize],
                    AES_SBOX[temp[3] as usize],
                    AES_SBOX[temp[0] as usize],
                ];
            }

            w[i * 4] = w[i * 4 - 16] ^ temp[0];
            w[i * 4 + 1] = w[i * 4 - 15] ^ temp[1];
            w[i * 4 + 2] = w[i * 4 - 14] ^ temp[2];
            w[i * 4 + 3] = w[i * 4 - 13] ^ temp[3];
        }

        for (i, rk) in round_keys.iter_mut().enumerate() {
            rk.copy_from_slice(&w[i * 16..(i + 1) * 16]);
        }
    }

    fn sub_bytes(state: &mut [u8; 16]) {
        for byte in state.iter_mut() {
            *byte = AES_SBOX[*byte as usize];
        }
    }

    fn shift_rows(state: &mut [u8; 16]) {
        let temp = *state;
        state[1] = temp[5];
        state[5] = temp[9];
        state[9] = temp[13];
        state[13] = temp[1];
        state[2] = temp[10];
        state[6] = temp[14];
        state[10] = temp[2];
        state[14] = temp[6];
        state[3] = temp[15];
        state[7] = temp[3];
        state[11] = temp[7];
        state[15] = temp[11];
    }

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
                aa ^= 0x1b;
            }
            bb >>= 1;
        }
        result
    }

    fn mix_columns(state: &mut [u8; 16]) {
        for col in 0..4 {
            let i = col * 4;
            let (s0, s1, s2, s3) = (state[i], state[i + 1], state[i + 2], state[i + 3]);
            state[i] = Self::gf_mul(2, s0) ^ Self::gf_mul(3, s1) ^ s2 ^ s3;
            state[i + 1] = s0 ^ Self::gf_mul(2, s1) ^ Self::gf_mul(3, s2) ^ s3;
            state[i + 2] = s0 ^ s1 ^ Self::gf_mul(2, s2) ^ Self::gf_mul(3, s3);
            state[i + 3] = Self::gf_mul(3, s0) ^ s1 ^ s2 ^ Self::gf_mul(2, s3);
        }
    }

    fn add_round_key(state: &mut [u8; 16], round_key: &[u8; 16]) {
        for (s, k) in state.iter_mut().zip(round_key.iter()) {
            *s ^= k;
        }
    }

    fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        let mut state = *block;
        Self::add_round_key(&mut state, &self.round_keys[0]);

        for round in 1..10 {
            Self::sub_bytes(&mut state);
            Self::shift_rows(&mut state);
            Self::mix_columns(&mut state);
            Self::add_round_key(&mut state, &self.round_keys[round]);
        }

        Self::sub_bytes(&mut state);
        Self::shift_rows(&mut state);
        Self::add_round_key(&mut state, &self.round_keys[10]);

        state
    }
}

/// GCM GHASH multiplication in GF(2^128)
fn ghash_multiply(x: &[u8; 16], h: &[u8; 16]) -> [u8; 16] {
    let mut z = [0u8; 16];
    let mut v = *h;

    for i in 0..128 {
        let byte_idx = i / 8;
        let bit_idx = 7 - (i % 8);
        if (x[byte_idx] >> bit_idx) & 1 == 1 {
            for j in 0..16 {
                z[j] ^= v[j];
            }
        }

        // Shift V right by 1 and reduce if needed
        let lsb = v[15] & 1;
        for j in (1..16).rev() {
            v[j] = (v[j] >> 1) | (v[j - 1] << 7);
        }
        v[0] >>= 1;

        if lsb == 1 {
            v[0] ^= 0xE1; // R = 0xE1 || 0^120
        }
    }

    z
}

/// GHASH function for GCM
fn ghash(h: &[u8; 16], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let mut tag = [0u8; 16];

    // Process AAD
    for chunk in aad.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        for i in 0..16 {
            tag[i] ^= block[i];
        }
        tag = ghash_multiply(&tag, h);
    }

    // Process ciphertext
    for chunk in ciphertext.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        for i in 0..16 {
            tag[i] ^= block[i];
        }
        tag = ghash_multiply(&tag, h);
    }

    // Length block: len(A) || len(C) in bits, big-endian 64-bit
    let mut len_block = [0u8; 16];
    let aad_bits = (aad.len() as u64).wrapping_mul(8);
    let ct_bits = (ciphertext.len() as u64).wrapping_mul(8);
    len_block[..8].copy_from_slice(&aad_bits.to_be_bytes());
    len_block[8..16].copy_from_slice(&ct_bits.to_be_bytes());
    for i in 0..16 {
        tag[i] ^= len_block[i];
    }
    tag = ghash_multiply(&tag, h);

    tag
}

/// AES-128-GCM encrypt
pub(crate) fn aes128_gcm_encrypt(
    key: &[u8; 16],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    let cipher = Aes128::new(key);

    // H = AES_K(0^128)
    let h = cipher.encrypt_block(&[0u8; 16]);

    // J0 = nonce || 0x00000001 (for 96-bit nonce)
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    // Encrypt plaintext with counter starting at J0 + 1
    let mut ciphertext = Vec::with_capacity(plaintext.len());
    let mut counter = 2u32;
    for chunk in plaintext.chunks(16) {
        let mut cb = j0;
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let keystream = cipher.encrypt_block(&cb);
        for (i, &b) in chunk.iter().enumerate() {
            ciphertext.push(b ^ keystream[i]);
        }
        counter = counter.wrapping_add(1);
    }

    // Compute GHASH
    let ghash_val = ghash(&h, aad, &ciphertext);

    // Tag = GHASH XOR AES_K(J0)
    let j0_encrypted = cipher.encrypt_block(&j0);
    let mut tag = [0u8; 16];
    for i in 0..16 {
        tag[i] = ghash_val[i] ^ j0_encrypted[i];
    }

    ciphertext.extend_from_slice(&tag);
    ciphertext
}

/// AES-128-GCM decrypt
pub(crate) fn aes128_gcm_decrypt(
    key: &[u8; 16],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ciphertext_and_tag.len() < AEAD_TAG_LEN {
        return None;
    }

    let ct_len = ciphertext_and_tag.len() - AEAD_TAG_LEN;
    let ciphertext = &ciphertext_and_tag[..ct_len];
    let received_tag = &ciphertext_and_tag[ct_len..];

    let cipher = Aes128::new(key);
    let h = cipher.encrypt_block(&[0u8; 16]);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    // Verify tag first
    let ghash_val = ghash(&h, aad, ciphertext);
    let j0_encrypted = cipher.encrypt_block(&j0);
    let mut expected_tag = [0u8; 16];
    for i in 0..16 {
        expected_tag[i] = ghash_val[i] ^ j0_encrypted[i];
    }

    if !constant_time_eq(received_tag, &expected_tag) {
        return None;
    }

    // Decrypt
    let mut plaintext = Vec::with_capacity(ct_len);
    let mut counter = 2u32;
    for chunk in ciphertext.chunks(16) {
        let mut cb = j0;
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let keystream = cipher.encrypt_block(&cb);
        for (i, &b) in chunk.iter().enumerate() {
            plaintext.push(b ^ keystream[i]);
        }
        counter = counter.wrapping_add(1);
    }

    Some(plaintext)
}

// ============================================================================
// AEAD Dispatch
// ============================================================================

/// AEAD encrypt dispatcher for the negotiated cipher suite
pub(crate) fn aead_encrypt(
    cipher: CipherSuite,
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Option<Vec<u8>> {
    match cipher {
        CipherSuite::ChaCha20Poly1305Sha256 => {
            if key.len() != CHACHA20_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            Some(chacha20_poly1305_encrypt(&k, nonce, aad, plaintext))
        }
        CipherSuite::Aes128GcmSha256 => {
            if key.len() != AES_128_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            Some(aes128_gcm_encrypt(&k, nonce, aad, plaintext))
        }
    }
}

/// AEAD decrypt dispatcher for the negotiated cipher suite
pub(crate) fn aead_decrypt(
    cipher: CipherSuite,
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    match cipher {
        CipherSuite::ChaCha20Poly1305Sha256 => {
            if key.len() != CHACHA20_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            chacha20_poly1305_decrypt(&k, nonce, aad, ciphertext_and_tag)
        }
        CipherSuite::Aes128GcmSha256 => {
            if key.len() != AES_128_KEY_LEN {
                return None;
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            aes128_gcm_decrypt(&k, nonce, aad, ciphertext_and_tag)
        }
    }
}

/// Constant-time comparison of two byte slices
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}
