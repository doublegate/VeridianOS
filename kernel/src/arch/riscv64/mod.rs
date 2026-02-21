//! RISC-V 64-bit architecture support.
//!
//! Provides initialization, interrupt control, serial I/O, and I/O port
//! stubs for the RISC-V 64-bit platform. Uses the SBI (Supervisor Binary
//! Interface) for machine-mode services.

pub mod boot;
pub mod bootstrap;
pub mod entry;
pub mod serial;
pub mod usermode;

// Re-export context, PLIC, and timer from parent riscv module
pub use super::riscv::{context, plic, timer};

/// Called from bootstrap on RISC-V via `crate::arch::init()`.
pub fn init() {
    // Initialize SBI (Supervisor Binary Interface)
    super::riscv::sbi::init();

    // Initialize PLIC (all sources disabled, threshold at 0).
    // This is safe before stvec is configured because no sources are enabled.
    if let Err(e) = super::riscv::plic::init() {
        println!("[RISCV64] WARNING: PLIC initialization failed: {}", e);
    }

    // IMPORTANT: Do NOT enable interrupts during early boot!
    // There is no trap handler (stvec) set up yet, so any interrupt
    // would cause the CPU to jump to address 0 and crash/reboot.
    //
    // The trap handler must be set up first before enabling interrupts.
    // For now, keep interrupts disabled - WFI will still return on
    // external events even with interrupts disabled.
    // SAFETY: CSR writes to disable interrupts during early boot. csrci clears
    // the SIE bit in sstatus (supervisor interrupt enable). csrw sie, zero clears
    // all interrupt enable bits. Required because no trap handler (stvec) is
    // registered yet - any interrupt would jump to address 0 and crash.
    unsafe {
        // Ensure interrupts are DISABLED in sstatus
        core::arch::asm!("csrci sstatus, 2", options(nomem, nostack));

        // Clear all interrupt enable bits in sie
        // This prevents any interrupts from being delivered
        core::arch::asm!("csrw sie, zero", options(nomem, nostack));
    }

    println!("[RISCV64] Architecture initialization complete (interrupts disabled)");
}

/// Halt the CPU. Used by panic/shutdown paths via `crate::arch::halt()`.
pub fn halt() -> ! {
    loop {
        // SAFETY: wfi (Wait For Interrupt) halts the CPU until an interrupt occurs.
        // Safe in a halt loop; reduces power consumption. Returns on external events
        // even with interrupts disabled.
        unsafe { core::arch::asm!("wfi") };
    }
}

/// Enable supervisor interrupts. Requires stvec to be configured first.
#[allow(dead_code)] // Interrupt API -- used when trap handler is configured
pub fn enable_interrupts() {
    // SAFETY: csrsi sets the SIE bit in sstatus, enabling supervisor interrupts.
    // The caller must ensure a trap handler (stvec) is properly configured.
    unsafe {
        core::arch::asm!("csrsi sstatus, 2");
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
                // SAFETY: csrsi sets the SIE bit in sstatus, re-enabling supervisor
                // interrupts that were disabled by disable_interrupts().
                unsafe {
                    core::arch::asm!("csrsi sstatus, 2");
                }
            }
        }
    }

    let mut sstatus: usize;
    // SAFETY: csrr reads the sstatus CSR to save the current interrupt state.
    // csrci clears the SIE bit, disabling supervisor interrupts. Both are
    // privileged operations always available in supervisor mode.
    unsafe {
        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
        core::arch::asm!("csrci sstatus, 2");
    }
    InterruptGuard {
        was_enabled: (sstatus & 0x2) != 0,
    }
}

/// Idle the CPU until an interrupt. Called from the scheduler idle loop
/// via `crate::arch::idle()`.
pub fn idle() {
    // SAFETY: wfi (Wait For Interrupt) halts the CPU until an interrupt.
    // Non-destructive.
    unsafe { core::arch::asm!("wfi") };
}

/// Speculation barrier to mitigate Spectre-style attacks.
/// Uses FENCE.I on RISC-V which synchronizes instruction and data streams.
#[inline(always)]
pub fn speculation_barrier() {
    // SAFETY: fence.i ensures instruction cache coherence and acts as a
    // speculation barrier by serializing instruction fetch. No side effects
    // beyond pipeline synchronization.
    unsafe {
        core::arch::asm!("fence.i", options(nostack, nomem));
    }
}

pub fn serial_init() -> crate::serial::Uart16550Compat {
    // QEMU virt machine places 16550 UART at 0x10000000
    let mut uart = crate::serial::Uart16550Compat::new(0x1000_0000);
    uart.init();
    uart
}

/// Kernel heap start address (16MB into QEMU virt RAM at 0x80000000)
pub const HEAP_START: usize = 0x81000000;

/// Flush TLB for a specific virtual address. Called via
/// `crate::arch::tlb_flush_address()`.
pub fn tlb_flush_address(addr: u64) {
    // SAFETY: `sfence.vma` with a specific address and zero ASID invalidates
    // all TLB entries for that address. Supervisor-mode instruction, safe in
    // S-mode.
    unsafe {
        core::arch::asm!("sfence.vma {}, zero", in(reg) addr);
    }
}

/// Flush entire TLB. Called via `crate::arch::tlb_flush_all()`.
pub fn tlb_flush_all() {
    // SAFETY: `sfence.vma` with no arguments flushes all TLB entries.
    // Supervisor-mode fence, safe in S-mode.
    unsafe {
        core::arch::asm!("sfence.vma");
    }
}

/// I/O port stubs for RISC-V -- RISC-V does not have I/O ports.
/// These exist solely so that architecture-generic driver code compiles
/// on all platforms without conditional compilation at every call site.
/// Called via `crate::arch::outb()`, etc.
///
/// # Safety
///
/// These are no-op stubs for API compatibility. Safe to call on RISC-V.
pub unsafe fn outb(_port: u16, _value: u8) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read byte from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
pub unsafe fn inb(_port: u16) -> u8 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

/// Write word to I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
pub unsafe fn outw(_port: u16, _value: u16) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read word from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
pub unsafe fn inw(_port: u16) -> u16 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

/// Write long to I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
pub unsafe fn outl(_port: u16, _value: u32) {
    // No-op: RISC-V doesn't have I/O ports
}

/// Read long from I/O port (stub for RISC-V).
///
/// # Safety
///
/// This is a no-op stub for API compatibility. Safe to call on RISC-V.
pub unsafe fn inl(_port: u16) -> u32 {
    // No-op: RISC-V doesn't have I/O ports
    0
}
