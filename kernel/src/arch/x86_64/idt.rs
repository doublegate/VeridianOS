//! Interrupt Descriptor Table.
//!
//! The table and every handler live in [`super::trap`]: assembly entry
//! stubs feeding one Rust dispatcher. This module loads it and keeps the
//! lock-free serial helpers that fault paths use.

/// Load the IDT on this CPU (building it on the first call).
pub fn init() {
    super::trap::init();
}

/// Write a byte string to COM1 serial, bypassing all locks.
///
/// # Safety
/// Port 0x3F8 must be a valid COM1 data register.
pub(crate) unsafe fn raw_serial_str(s: &[u8]) {
    for &b in s {
        // SAFETY: forwarded from this function's contract: port 0x3F8 is the
        // COM1 data register, so the OUT only transmits a byte.
        unsafe {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
}

/// Write a u64 as hex to COM1 serial, bypassing all locks.
///
/// # Safety
/// Port 0x3F8 must be a valid COM1 data register.
pub(crate) unsafe fn raw_serial_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // Print 16 hex digits (skip leading zeros after first nonzero)
    let mut started = false;
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as usize;
        if nibble != 0 || started || i == 0 {
            started = true;
            let b = HEX[nibble];
            // SAFETY: forwarded from this function's contract: port 0x3F8 is
            // the COM1 data register, so the OUT only transmits a byte.
            unsafe {
                core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
            }
        }
    }
}
