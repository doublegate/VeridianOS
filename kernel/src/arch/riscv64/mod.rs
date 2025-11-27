// RISC-V 64 architecture support

pub mod boot;
pub mod bootstrap;
pub mod entry;
pub mod serial;

// Re-export context and timer from parent riscv module
#[allow(unused_imports)]
pub use super::riscv::{context, timer};

#[allow(dead_code)]
pub fn init() {
    // Initialize SBI (Supervisor Binary Interface)
    super::riscv::sbi::init();

    // IMPORTANT: Do NOT enable interrupts during early boot!
    // There is no trap handler (stvec) set up yet, so any interrupt
    // would cause the CPU to jump to address 0 and crash/reboot.
    //
    // The trap handler must be set up first before enabling interrupts.
    // For now, keep interrupts disabled - WFI will still return on
    // external events even with interrupts disabled.
    unsafe {
        // Ensure interrupts are DISABLED in sstatus
        core::arch::asm!("csrci sstatus, 2", options(nomem, nostack));

        // Clear all interrupt enable bits in sie
        // This prevents any interrupts from being delivered
        core::arch::asm!("csrw sie, zero", options(nomem, nostack));
    }

    println!("[RISCV64] Architecture initialization complete (interrupts disabled)");
}

#[allow(dead_code)]
pub fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[allow(dead_code)]
pub fn enable_interrupts() {
    unsafe {
        core::arch::asm!("csrsi sstatus, 2");
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
                    core::arch::asm!("csrsi sstatus, 2");
                }
            }
        }
    }

    let mut sstatus: usize;
    unsafe {
        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
        core::arch::asm!("csrci sstatus, 2");
    }
    InterruptGuard {
        was_enabled: (sstatus & 0x2) != 0,
    }
}

#[allow(dead_code)]
pub fn idle() {
    unsafe { core::arch::asm!("wfi") };
}

pub fn serial_init() -> crate::serial::Uart16550Compat {
    // QEMU virt machine places 16550 UART at 0x10000000
    let mut uart = crate::serial::Uart16550Compat::new(0x1000_0000);
    uart.init();
    uart
}

/// I/O port functions (stubs for RISC-V - no I/O ports like x86)
///
/// # Safety
///
/// These are no-op stubs for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outb(_port: u16, _value: u8) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read byte from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inb(_port: u16) -> u8 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

/// Write word to I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outw(_port: u16, _value: u16) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read word from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inw(_port: u16) -> u16 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

/// Write long to I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn outl(_port: u16, _value: u32) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read long from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
#[allow(dead_code, clippy::missing_safety_doc)]
pub unsafe fn inl(_port: u16) -> u32 {
    // No-op: RISC-V doesn't have I/O ports
    0
}
