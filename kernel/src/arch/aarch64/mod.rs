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

/// Called from bootstrap on AArch64 via `crate::arch::init()`.
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

/// Halt the CPU. Used by panic/shutdown paths via `crate::arch::halt()`.
pub fn halt() -> ! {
    loop {
        // SAFETY: wfe (Wait For Event) halts the CPU until an event or interrupt
        // occurs. Safe to use in a halt loop as it reduces power consumption.
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

/// Idle the CPU until an event. Called from the scheduler idle loop
/// via `crate::arch::idle()`.
pub fn idle() {
    // SAFETY: wfe (Wait For Event) halts the CPU until an event or interrupt.
    // Used for idle loops to reduce power consumption. Non-destructive.
    unsafe {
        core::arch::asm!("wfe");
    }
}

/// Speculation barrier to mitigate Spectre-style attacks.
/// Uses CSDB (Consumption of Speculative Data Barrier) on AArch64.
#[inline(always)]
pub fn speculation_barrier() {
    // SAFETY: csdb prevents speculative use of data loaded before the barrier.
    // No side effects beyond constraining speculative execution.
    unsafe {
        core::arch::asm!("csdb", options(nostack, nomem, preserves_flags));
    }
}

/// Disable interrupts with RAII guard that restores the previous state on drop.
/// Called via `crate::arch::disable_interrupts()`.
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
/// Not currently called on AArch64 (serial is initialized differently).
#[allow(dead_code)]
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(0x0900_0000)
}

/// Kernel heap start address (16MB into QEMU virt RAM at 0x40000000)
pub const HEAP_START: usize = 0x41000000;

/// Flush TLB for a specific virtual address. Called via
/// `crate::arch::tlb_flush_address()`.
pub fn tlb_flush_address(addr: u64) {
    // SAFETY: `tlbi vae1` invalidates the TLB entry at EL1. Address is shifted
    // right by 12 for page-number format. DSB SY + ISB ensure completion.
    unsafe {
        let page_addr = addr >> 12;
        core::arch::asm!("tlbi vae1, {}", in(reg) page_addr);
        core::arch::asm!("dsb sy");
        core::arch::asm!("isb");
    }
}

/// Flush entire TLB. Called via `crate::arch::tlb_flush_all()`.
pub fn tlb_flush_all() {
    // SAFETY: `tlbi vmalle1` invalidates all EL1 TLB entries. DSB SY + ISB
    // ensure completion. Architectural maintenance instructions, safe at EL1.
    unsafe {
        core::arch::asm!("tlbi vmalle1");
        core::arch::asm!("dsb sy");
        core::arch::asm!("isb");
    }
}

/// I/O port stubs for AArch64 -- ARM does not have I/O ports.
/// These exist solely so that architecture-generic driver code compiles
/// on all platforms without conditional compilation at every call site.
/// Called via `crate::arch::outb()`, etc.
///
/// # Safety
///
/// These are no-op stubs for API compatibility. Safe to call on AArch64.
pub unsafe fn outb(_port: u16, _value: u8) {
    // No-op: ARM doesn't have I/O ports
}

/// Read byte from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
pub unsafe fn inb(_port: u16) -> u8 {
    // No-op: ARM doesn't have I/O ports
    0
}

/// Write word to I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
pub unsafe fn outw(_port: u16, _value: u16) {
    // No-op: ARM doesn't have I/O ports
}

/// Read word from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
pub unsafe fn inw(_port: u16) -> u16 {
    // No-op: ARM doesn't have I/O ports
    0
}

/// Write long to I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
pub unsafe fn outl(_port: u16, _value: u32) {
    // No-op: ARM doesn't have I/O ports
}

/// Read long from I/O port (stub for AArch64).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on AArch64.
pub unsafe fn inl(_port: u16) -> u32 {
    // No-op: ARM doesn't have I/O ports
    0
}
