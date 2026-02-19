//! Compiler intrinsics implementation
//!
//! These are required by LLVM but not provided by compiler_builtins
//! for no_std environments.

use core::ffi::c_void;

/// Memory copy intrinsic
///
/// # Safety
/// Caller must ensure src and dest don't overlap and are valid for the given
/// length
#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest_u8 = dest as *mut u8;
    let src_u8 = src as *const u8;

    for i in 0..n {
        *dest_u8.add(i) = *src_u8.add(i);
    }

    dest
}

/// Memory set intrinsic
///
/// # Safety
/// Caller must ensure dest is valid for the given length
#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let dest_u8 = dest as *mut u8;
    let byte = c as u8;

    for i in 0..n {
        *dest_u8.add(i) = byte;
    }

    dest
}

/// Memory move intrinsic (handles overlapping memory)
///
/// # Safety
/// Caller must ensure src and dest are valid for the given length
#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let dest_u8 = dest as *mut u8;
    let src_u8 = src as *const u8;

    use core::cmp::Ordering;
    match (dest_u8 as usize).cmp(&(src_u8 as usize)) {
        Ordering::Less => {
            // Copy forward
            for i in 0..n {
                *dest_u8.add(i) = *src_u8.add(i);
            }
        }
        Ordering::Greater => {
            // Copy backward to handle overlap
            for i in (0..n).rev() {
                *dest_u8.add(i) = *src_u8.add(i);
            }
        }
        Ordering::Equal => {
            // If dest == src, no-op
        }
    }

    dest
}

/// Memory compare intrinsic
///
/// # Safety
/// Caller must ensure s1 and s2 are valid for the given length
#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> i32 {
    let s1_u8 = s1 as *const u8;
    let s2_u8 = s2 as *const u8;

    for i in 0..n {
        let a = *s1_u8.add(i);
        let b = *s2_u8.add(i);
        if a != b {
            return a as i32 - b as i32;
        }
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memcpy() {
        let src = [1u8, 2, 3, 4, 5];
        let mut dest = [0u8; 5];

        // SAFETY: Both `src` and `dest` are stack-allocated [u8; 5] arrays.
        // The pointers are valid for exactly 5 bytes and do not overlap.
        unsafe {
            memcpy(
                dest.as_mut_ptr() as *mut c_void,
                src.as_ptr() as *const c_void,
                5,
            );
        }

        assert_eq!(dest, src);
    }

    #[test]
    fn test_memset() {
        let mut buf = [0u8; 10];

        // SAFETY: `buf` is a stack-allocated [u8; 10] array. The pointer
        // is valid for exactly 10 bytes, matching the count argument.
        unsafe {
            memset(buf.as_mut_ptr() as *mut c_void, 0x42, 10);
        }

        assert_eq!(buf, [0x42u8; 10]);
    }

    #[test]
    fn test_memcmp() {
        let a = [1u8, 2, 3, 4, 5];
        let b = [1u8, 2, 3, 4, 5];
        let c = [1u8, 2, 3, 4, 6];

        // SAFETY: `a`, `b`, and `c` are stack-allocated [u8; 5] arrays.
        // All pointers are valid for exactly 5 bytes.
        unsafe {
            assert_eq!(
                memcmp(a.as_ptr() as *const c_void, b.as_ptr() as *const c_void, 5),
                0
            );

            assert!(memcmp(a.as_ptr() as *const c_void, c.as_ptr() as *const c_void, 5) < 0);
        }
    }

    #[test]
    fn test_memmove_forward() {
        let mut buf = [1u8, 2, 3, 4, 5, 0, 0, 0];

        // SAFETY: `buf` is a stack-allocated [u8; 8] array. Source starts
        // at index 0 (5 bytes) and destination at index 3 (5 bytes to
        // index 7). Both ranges are within the 8-byte buffer. Overlapping
        // regions are handled correctly by memmove's forward copy path.
        unsafe {
            memmove(
                buf.as_mut_ptr().add(3) as *mut c_void,
                buf.as_ptr() as *const c_void,
                5,
            );
        }

        assert_eq!(buf, [1, 2, 3, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_memmove_backward() {
        let mut buf = [0u8, 0, 0, 1, 2, 3, 4, 5];

        // SAFETY: `buf` is a stack-allocated [u8; 8] array. Source starts
        // at index 3 (5 bytes to index 7) and destination at index 0 (5
        // bytes). Both ranges are within the 8-byte buffer. Overlapping
        // regions are handled correctly by memmove's backward copy path.
        unsafe {
            memmove(
                buf.as_mut_ptr() as *mut c_void,
                buf.as_ptr().add(3) as *const c_void,
                5,
            );
        }

        assert_eq!(buf, [1, 2, 3, 4, 5, 3, 4, 5]);
    }
}
