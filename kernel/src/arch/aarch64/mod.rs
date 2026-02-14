//! AArch64 (ARM 64-bit) architecture support.
//!
//! Provides initialization, interrupt control (DAIF), serial I/O (PL011
//! UART at 0x0900_0000), and I/O port stubs for the AArch64 platform.

// Include the boot module
pub mod boot;
pub mod bootstrap;
pub mod context;
pub mod direct_uart;
pub mod entry;
pub mod serial;
pub mod timer;

/// Called from bootstrap on AArch64; appears unused on other architectures.
#[allow(dead_code)]
pub fn init() {
    // SAFETY: uart_write_str performs a raw MMIO write to the PL011 UART at
    // 0x09000000. This is safe during kernel init as the UART is memory-mapped
    // by QEMU's virt machine and the write is non-destructive.
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[ARCH] Performing AArch64-specific initialization\n");
    }
    // This will be expanded later
}

/// Halt the CPU. Used by panic/shutdown paths.
#[allow(dead_code)]
pub fn halt() -> ! {
    loop {
        // SAFETY: wfe (Wait For Event) halts the CPU until an event or interrupt
        // occurs. Safe to use in a halt loop as it reduces power consumption.
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

/// Idle the CPU until an event. Called from the scheduler idle loop.
#[allow(dead_code)]
pub fn idle() {
    // SAFETY: wfe (Wait For Event) halts the CPU until an event or interrupt.
    // Used for idle loops to reduce power consumption. Non-destructive.
    unsafe {
        core::arch::asm!("wfe");
    }
}

/// Disable interrupts with RAII guard that restores the previous state on drop.
#[allow(dead_code)]
pub fn disable_interrupts() -> impl Drop {
    struct InterruptGuard {
        was_enabled: bool,
    }

    impl Drop for InterruptGuard {
        fn drop(&mut self) {
            if self.was_enabled {
                // SAFETY: daifclr #2 clears the IRQ mask bit in DAIF, re-enabling
                // interrupts that were disabled by daifset #2 in disable_interrupts().
                unsafe {
                    core::arch::asm!("msr daifclr, #2");
                }
            }
        }
    }

    let mut daif: u64;
    // SAFETY: mrs reads the DAIF register to save the current interrupt state.
    // daifset #2 sets the IRQ mask bit, disabling interrupts. Both are privileged
    // operations always available in kernel (EL1) mode.
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
        core::arch::asm!("msr daifset, #2");
    }
    InterruptGuard {
        was_enabled: (daif & 0x80) == 0,
    }
}

/// Serial initialization for compatibility with the arch-generic interface.
#[allow(dead_code)]
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(0x0900_0000)
}

/// I/O port stubs for AArch64 -- ARM does not have I/O ports.
/// These exist solely so that architecture-generic driver code compiles
/// on all platforms without conditional compilation at every call site.
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
