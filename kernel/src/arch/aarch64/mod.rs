// AArch64 architecture support

// Include the boot module
pub mod boot;
pub mod bootstrap;
pub mod context;
pub mod direct_uart;
pub mod entry;
pub mod serial;
pub mod timer;

#[allow(dead_code)]
pub fn init() {
    // Architecture-specific initialization
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[ARCH] Performing AArch64-specific initialization\n");
    }
    // This will be expanded later
}

#[allow(dead_code)]
pub fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

#[allow(dead_code)]
pub fn idle() {
    unsafe {
        core::arch::asm!("wfe");
    }
}

#[allow(dead_code)]
pub fn disable_interrupts() -> impl Drop {
    struct InterruptGuard {
        was_enabled: bool,
    }

    impl Drop for InterruptGuard {
        fn drop(&mut self) {
            if self.was_enabled {
                unsafe {
                    core::arch::asm!("msr daifclr, #2");
                }
            }
        }
    }

    let mut daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
        core::arch::asm!("msr daifset, #2");
    }
    InterruptGuard {
        was_enabled: (daif & 0x80) == 0,
    }
}

// Simple serial initialization for compatibility
#[allow(dead_code)]
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(0x0900_0000)
}

/// I/O port functions (stubs for AArch64 - ARM doesn't have I/O ports)
///
/// # Safety
///
/// These are no-op stubs for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outb(_port: u16, _value: u8) {
    // No-op: ARM doesn't have I/O ports
}

/// Read byte from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inb(_port: u16) -> u8 {
    // No-op: ARM doesn't have I/O ports
    0
}

/// Write word to I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outw(_port: u16, _value: u16) {
    // No-op: ARM doesn't have I/O ports
}

/// Read word from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inw(_port: u16) -> u16 {
    // No-op: ARM doesn't have I/O ports
    0
}

/// Write long to I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outl(_port: u16, _value: u32) {
    // No-op: ARM doesn't have I/O ports
}

/// Read long from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inl(_port: u16) -> u32 {
    // No-op: ARM doesn't have I/O ports
    0
}
