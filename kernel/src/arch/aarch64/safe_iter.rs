//! Safe iteration utilities for AArch64 to work around compiler loop bugs
//!
//! The AArch64 LLVM backend has a severe bug that causes all loop constructs
//! to hang on bare metal. This module provides workarounds.

/// Write a string to UART without using loops
///
/// # Safety
///
/// The caller must ensure:
/// - `uart_base` points to a valid UART data register
/// - The UART hardware is properly initialized
pub unsafe fn write_str_loopfree(uart_base: usize, s: &str) {
    let uart = uart_base as *mut u8;
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Manual unrolling for common cases
    if len >= 1 {
        *uart = bytes[0];
    }
    if len >= 2 {
        *uart = bytes[1];
    }
    if len >= 3 {
        *uart = bytes[2];
    }
    if len >= 4 {
        *uart = bytes[3];
    }
    if len >= 5 {
        *uart = bytes[4];
    }
    if len >= 6 {
        *uart = bytes[5];
    }
    if len >= 7 {
        *uart = bytes[6];
    }
    if len >= 8 {
        *uart = bytes[7];
    }

    // For longer strings, use recursive approach
    if len > 8 {
        write_str_recursive(uart, &bytes[8..]);
    }
}

/// Recursive string writer for longer strings
unsafe fn write_str_recursive(uart: *mut u8, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }

    *uart = bytes[0];
    if bytes.len() > 1 {
        write_str_recursive(uart, &bytes[1..]);
    }
}

/// Write a number without loops
///
/// # Safety
///
/// The caller must ensure:
/// - `uart_base` points to a valid UART data register
/// - The UART hardware is properly initialized
pub unsafe fn write_num_loopfree(uart_base: usize, num: u64) {
    let uart = uart_base as *mut u8;

    if num == 0 {
        *uart = b'0';
        return;
    }

    // Extract digits recursively
    write_num_recursive(uart, num);
}

unsafe fn write_num_recursive(uart: *mut u8, num: u64) {
    if num == 0 {
        return;
    }

    let digit = (num % 10) as u8;
    write_num_recursive(uart, num / 10);
    *uart = b'0' + digit;
}

/// Write hex number without loops
///
/// # Safety
///
/// The caller must ensure:
/// - `uart_base` points to a valid UART data register
/// - The UART hardware is properly initialized
pub unsafe fn write_hex_loopfree(uart_base: usize, num: u64) {
    let uart = uart_base as *mut u8;

    *uart = b'0';
    *uart = b'x';

    // Write each nibble
    write_hex_nibble(uart, (num >> 60) & 0xF);
    write_hex_nibble(uart, (num >> 56) & 0xF);
    write_hex_nibble(uart, (num >> 52) & 0xF);
    write_hex_nibble(uart, (num >> 48) & 0xF);
    write_hex_nibble(uart, (num >> 44) & 0xF);
    write_hex_nibble(uart, (num >> 40) & 0xF);
    write_hex_nibble(uart, (num >> 36) & 0xF);
    write_hex_nibble(uart, (num >> 32) & 0xF);
    write_hex_nibble(uart, (num >> 28) & 0xF);
    write_hex_nibble(uart, (num >> 24) & 0xF);
    write_hex_nibble(uart, (num >> 20) & 0xF);
    write_hex_nibble(uart, (num >> 16) & 0xF);
    write_hex_nibble(uart, (num >> 12) & 0xF);
    write_hex_nibble(uart, (num >> 8) & 0xF);
    write_hex_nibble(uart, (num >> 4) & 0xF);
    write_hex_nibble(uart, num & 0xF);
}

unsafe fn write_hex_nibble(uart: *mut u8, nibble: u64) {
    let c = if nibble < 10 {
        b'0' + nibble as u8
    } else {
        b'a' + (nibble - 10) as u8
    };
    *uart = c;
}

/// Initialize an array without loops
pub fn init_array_loopfree<T: Copy, const N: usize>(arr: &mut [T; N], value: T) {
    // Manual unrolling for small arrays
    if N >= 1 {
        arr[0] = value;
    }
    if N >= 2 {
        arr[1] = value;
    }
    if N >= 3 {
        arr[2] = value;
    }
    if N >= 4 {
        arr[3] = value;
    }
    if N >= 5 {
        arr[4] = value;
    }
    if N >= 6 {
        arr[5] = value;
    }
    if N >= 7 {
        arr[6] = value;
    }
    if N >= 8 {
        arr[7] = value;
    }

    // For larger arrays, use recursive approach
    if N > 8 {
        init_slice_recursive(&mut arr[8..], value);
    }
}

fn init_slice_recursive<T: Copy>(slice: &mut [T], value: T) {
    if slice.is_empty() {
        return;
    }

    slice[0] = value;
    if slice.len() > 1 {
        init_slice_recursive(&mut slice[1..], value);
    }
}

/// Copy memory without loops
///
/// # Safety
///
/// The caller must ensure:
/// - `dest` and `src` are valid pointers
/// - `dest` and `src` do not overlap
/// - `count` bytes are readable from `src` and writable to `dest`
pub unsafe fn memcpy_loopfree(dest: *mut u8, src: *const u8, count: usize) {
    // Use u64 copies for efficiency
    let dest_u64 = dest as *mut u64;
    let src_u64 = src as *const u64;
    let u64_count = count / 8;

    // Manual unrolling for u64 copies
    if u64_count >= 1 {
        *dest_u64.add(0) = *src_u64.add(0);
    }
    if u64_count >= 2 {
        *dest_u64.add(1) = *src_u64.add(1);
    }
    if u64_count >= 3 {
        *dest_u64.add(2) = *src_u64.add(2);
    }
    if u64_count >= 4 {
        *dest_u64.add(3) = *src_u64.add(3);
    }

    // Handle remaining bytes
    let remainder = count % 8;
    let dest_bytes = dest.add(count - remainder);
    let src_bytes = src.add(count - remainder);

    if remainder >= 1 {
        *dest_bytes.add(0) = *src_bytes.add(0);
    }
    if remainder >= 2 {
        *dest_bytes.add(1) = *src_bytes.add(1);
    }
    if remainder >= 3 {
        *dest_bytes.add(2) = *src_bytes.add(2);
    }
    if remainder >= 4 {
        *dest_bytes.add(3) = *src_bytes.add(3);
    }
    if remainder >= 5 {
        *dest_bytes.add(4) = *src_bytes.add(4);
    }
    if remainder >= 6 {
        *dest_bytes.add(5) = *src_bytes.add(5);
    }
    if remainder >= 7 {
        *dest_bytes.add(6) = *src_bytes.add(6);
    }
}

/// Zero memory without loops
///
/// # Safety
///
/// The caller must ensure:
/// - `dest` is a valid pointer
/// - `count` bytes are writable from `dest`
pub unsafe fn memset_loopfree(dest: *mut u8, value: u8, count: usize) {
    // Create u64 value by repeating the byte
    let value_u64 = (value as u64) * 0x0101010101010101u64;
    let dest_u64 = dest as *mut u64;
    let u64_count = count / 8;

    // Manual unrolling for u64 writes
    if u64_count >= 1 {
        *dest_u64.add(0) = value_u64;
    }
    if u64_count >= 2 {
        *dest_u64.add(1) = value_u64;
    }
    if u64_count >= 3 {
        *dest_u64.add(2) = value_u64;
    }
    if u64_count >= 4 {
        *dest_u64.add(3) = value_u64;
    }

    // Handle remaining bytes
    let remainder = count % 8;
    let dest_bytes = dest.add(count - remainder);

    if remainder >= 1 {
        *dest_bytes.add(0) = value;
    }
    if remainder >= 2 {
        *dest_bytes.add(1) = value;
    }
    if remainder >= 3 {
        *dest_bytes.add(2) = value;
    }
    if remainder >= 4 {
        *dest_bytes.add(3) = value;
    }
    if remainder >= 5 {
        *dest_bytes.add(4) = value;
    }
    if remainder >= 6 {
        *dest_bytes.add(5) = value;
    }
    if remainder >= 7 {
        *dest_bytes.add(6) = value;
    }
}

/// Macro for safe iteration on AArch64
#[macro_export]
macro_rules! aarch64_for {
    ($i:ident in 0..$n:expr => $body:expr) => {{
        let mut $i = 0;
        aarch64_for_impl!($i, $n, $body);
    }};
}

#[macro_export]
macro_rules! aarch64_for_impl {
    ($i:ident, $n:expr, $body:expr) => {{
        if $i < $n {
            $body;
            $i += 1;
            aarch64_for_impl!($i, $n, $body);
        }
    }};
}
