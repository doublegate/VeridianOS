//! Constant-Time Cryptographic Primitives
//!
//! Side-channel resistant operations for cryptographic implementations.
//!
//! ## Security Considerations
//!
//! - All operations must execute in constant time to prevent timing attacks
//! - No data-dependent branches or memory accesses
//! - Compiler optimizations must not introduce timing variations
//! - Use volatile reads/writes where necessary to prevent optimization

use core::sync::atomic::{compiler_fence, Ordering};

/// Constant-time byte comparison
///
/// Returns 1 if equal, 0 otherwise. Runs in constant time regardless of input.
#[inline(never)]
pub fn ct_eq_bytes(a: &[u8], b: &[u8]) -> u8 {
    if a.len() != b.len() {
        return 0;
    }

    let mut result = 0u8;

    // Process all bytes regardless of intermediate results
    for i in 0..a.len() {
        result |= a[i] ^ b[i];
    }

    // Prevent compiler optimization
    compiler_fence(Ordering::SeqCst);

    // Convert to 0 or 1
    ((!result & (result.wrapping_sub(1))) >> 7) & 1
}

/// Constant-time conditional select
///
/// Returns `a` if `condition` is 1, `b` if `condition` is 0.
/// Condition must be 0 or 1.
#[inline(always)]
pub fn ct_select_u8(condition: u8, a: u8, b: u8) -> u8 {
    let mask = condition.wrapping_neg();
    (a & mask) | (b & !mask)
}

/// Constant-time conditional select for u32
#[inline(always)]
pub fn ct_select_u32(condition: u8, a: u32, b: u32) -> u32 {
    let mask = (condition as u32).wrapping_neg();
    (a & mask) | (b & !mask)
}

/// Constant-time conditional select for u64
#[inline(always)]
pub fn ct_select_u64(condition: u8, a: u64, b: u64) -> u64 {
    let mask = (condition as u64).wrapping_neg();
    (a & mask) | (b & !mask)
}

/// Constant-time conditional copy
///
/// Copies `src` to `dst` if `condition` is 1.
/// Always reads from `src` and writes to `dst` to maintain constant time.
#[inline(never)]
pub fn ct_copy(dst: &mut [u8], src: &[u8], condition: u8) {
    assert_eq!(dst.len(), src.len());

    for i in 0..dst.len() {
        dst[i] = ct_select_u8(condition, src[i], dst[i]);
    }

    compiler_fence(Ordering::SeqCst);
}

/// Constant-time array zeroing
///
/// Zeros an array in constant time, preventing compiler optimization.
#[inline(never)]
pub fn ct_zero(data: &mut [u8]) {
    for byte in data.iter_mut() {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }

    compiler_fence(Ordering::SeqCst);
}

/// Constant-time less-than comparison for u32
///
/// Returns 1 if a < b, 0 otherwise
#[inline(always)]
pub fn ct_lt_u32(a: u32, b: u32) -> u8 {
    let diff = a ^ ((a ^ b) | ((a.wrapping_sub(b)) ^ b));
    ((diff >> 31) & 1) as u8
}

/// Constant-time byte array comparison
///
/// Returns -1 if a < b, 0 if a == b, 1 if a > b
#[inline(never)]
pub fn ct_cmp_bytes(a: &[u8], b: &[u8]) -> i8 {
    assert_eq!(a.len(), b.len());

    let mut greater = 0u8;
    let mut less = 0u8;

    for i in 0..a.len() {
        let gt = ct_lt_u32(b[i] as u32, a[i] as u32);
        let lt = ct_lt_u32(a[i] as u32, b[i] as u32);

        greater |= gt & !less & !greater;
        less |= lt & !greater & !less;
    }

    compiler_fence(Ordering::SeqCst);

    if greater != 0 {
        1
    } else if less != 0 {
        -1
    } else {
        0
    }
}

/// Memory barrier to prevent reordering
#[inline(always)]
pub fn memory_barrier() {
    compiler_fence(Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_ct_eq_bytes() {
        let a = [1u8, 2, 3, 4];
        let b = [1u8, 2, 3, 4];
        let c = [1u8, 2, 3, 5];

        assert_eq!(ct_eq_bytes(&a, &b), 1);
        assert_eq!(ct_eq_bytes(&a, &c), 0);
    }

    #[test_case]
    fn test_ct_select() {
        assert_eq!(ct_select_u8(1, 0xAA, 0x55), 0xAA);
        assert_eq!(ct_select_u8(0, 0xAA, 0x55), 0x55);

        assert_eq!(ct_select_u32(1, 0x12345678, 0xABCDEF00), 0x12345678);
        assert_eq!(ct_select_u32(0, 0x12345678, 0xABCDEF00), 0xABCDEF00);
    }

    #[test_case]
    fn test_ct_copy() {
        let mut dst = [0u8; 4];
        let src = [1u8, 2, 3, 4];

        ct_copy(&mut dst, &src, 1);
        assert_eq!(dst, [1, 2, 3, 4]);

        ct_copy(&mut dst, &[5, 6, 7, 8], 0);
        assert_eq!(dst, [1, 2, 3, 4]); // Should not change
    }

    #[test_case]
    fn test_ct_cmp() {
        let a = [1u8, 2, 3];
        let b = [1u8, 2, 3];
        let c = [1u8, 2, 4];

        assert_eq!(ct_cmp_bytes(&a, &b), 0);
        assert_eq!(ct_cmp_bytes(&a, &c), -1);
        assert_eq!(ct_cmp_bytes(&c, &a), 1);
    }
}
