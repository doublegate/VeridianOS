//! Direct UART implementation for AArch64
//!
//! This module provides UART functionality that bypasses LLVM's loop
//! compilation issues by using inline assembly for the critical loop
//! operations.

use core::fmt;

/// UART base address for QEMU virt machine
#[allow(dead_code)]
const UART0_BASE: usize = 0x0900_0000;

/// Write bytes to UART using pure assembly - avoiding all Rust constructs
///
/// This implementation uses pure inline assembly for the entire operation.
unsafe fn uart_write_bytes_asm(ptr: *const u8, len: usize) {
    // Use inline assembly to perform the entire operation
    core::arch::asm!(
        "mov {uart}, #0x09000000",      // Load UART base address
        "mov {i}, #0",                  // Initialize counter
        "1:",                           // Loop start
        "cmp {i}, {len}",               // Compare counter with length
        "b.ge 2f",                      // Branch if counter >= length
        "ldrb {byte:w}, [{ptr}, {i}]",  // Load byte from string[i]
        "strb {byte:w}, [{uart}]",      // Store byte to UART
        "add {i}, {i}, #1",             // Increment counter
        "b 1b",                         // Branch back to loop
        "2:",                           // End
        ptr = in(reg) ptr,
        len = in(reg) len,
        uart = out(reg) _,
        i = out(reg) _,
        byte = out(reg) _,
        options(nostack, preserves_flags)
    );
}

/// Print a string directly to UART
pub fn direct_print_str(s: &str) {
    unsafe {
        uart_write_bytes_asm(s.as_ptr(), s.len());
    }
}

/// Print a single character directly to UART
pub fn direct_print_char(c: char) {
    let mut buffer = [0u8; 4];
    let str_slice = c.encode_utf8(&mut buffer);
    direct_print_str(str_slice);
}

/// Print a newline character
pub fn direct_print_newline() {
    direct_print_char('\n');
}

/// Print a number in decimal format
pub fn direct_print_num(n: u64) {
    if n == 0 {
        direct_print_char('0');
        return;
    }

    // Convert number to string manually to avoid heap allocation
    let mut buffer = [0u8; 20]; // Enough for u64::MAX
    let mut pos = buffer.len();
    let mut num = n;

    while num > 0 {
        pos -= 1;
        buffer[pos] = b'0' + (num % 10) as u8;
        num /= 10;
    }

    unsafe {
        uart_write_bytes_asm(buffer.as_ptr().add(pos), buffer.len() - pos);
    }
}

/// A writer that implements fmt::Write for use with format macros
pub struct DirectUartWriter;

impl fmt::Write for DirectUartWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        direct_print_str(s);
        Ok(())
    }
}

/// Create a new UART writer
pub fn writer() -> DirectUartWriter {
    DirectUartWriter
}

/// Initialize UART (no-op for QEMU, but kept for compatibility)
pub fn init() {
    // QEMU's UART doesn't need initialization, but we can add
    // basic setup here if needed for real hardware
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_print() {
        direct_print_str("Test message\n");
    }

    #[test]
    fn test_number_print() {
        direct_print_num(12345);
        direct_print_newline();
    }
}
