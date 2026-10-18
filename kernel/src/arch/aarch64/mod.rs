//! AArch64 (ARM 64-bit) architecture support.
//!
//! Provides initialization, interrupt control (DAIF), serial I/O (the PL011
//! UART, `platform::PL011`), and I/O port stubs for the AArch64 platform.

// Include the boot module
pub mod boot;
pub mod bootstrap;
pub mod bti;
pub mod context;
pub mod direct_uart;
pub mod entry;
pub mod exceptions;
pub mod gic;
#[cfg(feature = "alloc")]
pub mod kmap;
pub mod platform;
pub mod psci;
pub mod serial;
#[cfg(feature = "smp")]
pub mod smp;
pub mod switch;
pub mod timer;
pub mod usercopy;
pub mod usermode;

/// Called from bootstrap on AArch64 via `crate::arch::init()`.
pub fn init() {
    // SAFETY: uart_write_str performs a raw MMIO write to the PL011 UART
    // (`platform::PL011`). This is safe during kernel init as the UART is
    // memory-mapped by QEMU's virt machine and the write is non-destructive.
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[ARCH] Performing AArch64-specific initialization\n");
    }

    // BTI for user programs, if this CPU has it (before any EL0 code).
    bti::init_cpu(true);

    // Initialize the GIC (Generic Interrupt Controller)
    if let Err(_e) = gic::init() {
        // SAFETY: uart_write_str performs a raw MMIO write to the PL011 UART.
        // Safe during kernel init on QEMU virt machine.
        unsafe {
            crate::arch::aarch64::direct_uart::uart_write_str(
                "[ARCH] WARNING: GIC initialization failed\n",
            );
        }
        return;
    }

    // 1000 Hz tick on the EL1 virtual timer, then unmask IRQs.
    match timer::start(1000) {
        // SAFETY: the vector table and GIC are set up, so IRQs can be taken.
        Ok(()) => unsafe { core::arch::asm!("msr daifclr, #2", "isb", options(nostack)) },
        Err(_) => {
            // SAFETY: raw PL011 write during single-threaded init.
            unsafe {
                crate::arch::aarch64::direct_uart::uart_write_str(
                    "[ARCH] WARNING: timer start failed; IRQs stay masked\n",
                );
            }
        }
    }
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

/// CTR_EL0, the cache type register.
fn ctr_el0() -> u64 {
    let ctr: u64;
    // SAFETY: reading CTR_EL0 has no side effects.
    unsafe { core::arch::asm!("mrs {0}, ctr_el0", out(reg) ctr, options(nomem, nostack)) };
    ctr
}

/// CTR_EL0.IDC: data written need not be cleaned to the point of
/// unification for instruction fetch to see it.
const CTR_IDC: u64 = 1 << 28;
/// CTR_EL0.DIC: the instruction caches need no invalidation for it either.
const CTR_DIC: u64 = 1 << 29;

/// Clean the data cache lines of `addr..addr + len` (kernel addresses,
/// such as the physical map's alias of a user page) to the point of
/// unification, so instruction fetch can see what was written there
/// (HA-25). Nothing to do on a core with CTR_EL0.IDC; [`sync_icache_all`]
/// must follow either way.
pub fn clean_dcache_to_pou(addr: usize, len: usize) {
    let ctr = ctr_el0();
    if ctr & CTR_IDC == 0 {
        clean_dcache_lines_to_pou(addr, len, 4 << ((ctr >> 16) & 0xF));
    }
}

/// [`clean_dcache_to_pou`] whatever CTR_EL0.IDC says, with lines of
/// `line` bytes (CTR_EL0.DminLine).
pub fn clean_dcache_lines_to_pou(addr: usize, len: usize, line: usize) {
    for a in crate::mm::page_ops::cache_line_starts(addr, len, line) {
        // SAFETY: `dc cvau` cleans the line holding `a`, a mapped kernel
        // address (the caller's range); it changes no data.
        unsafe { core::arch::asm!("dc cvau, {0}", in(reg) a, options(nostack)) };
    }
    // SAFETY: a barrier only: the cleans complete before what follows.
    unsafe { core::arch::asm!("dsb ish", options(nostack)) };
}

/// Clean the data cache lines of `addr..addr + len` (kernel addresses) to
/// the point of coherency, where a non-coherent device reads memory
/// (streaming DMA, `mm::dma`; HA-25).
pub fn dcache_clean_poc(addr: usize, len: usize) {
    let line = 4 << ((ctr_el0() >> 16) & 0xF);
    for a in crate::mm::page_ops::cache_line_starts(addr, len, line) {
        // SAFETY: `dc cvac` cleans the line holding `a`, a mapped kernel
        // address (the caller's range); it changes no data.
        unsafe { core::arch::asm!("dc cvac, {0}", in(reg) a, options(nostack)) };
    }
    // SAFETY: a barrier only: the cleans complete before a device is told.
    unsafe { core::arch::asm!("dsb sy", options(nostack)) };
}

/// Clean and invalidate the data cache lines of `addr..addr + len` to the
/// point of coherency, so the CPU next reads what a non-coherent device
/// wrote (`mm::dma`; HA-25).
pub fn dcache_clean_inval_poc(addr: usize, len: usize) {
    let line = 4 << ((ctr_el0() >> 16) & 0xF);
    for a in crate::mm::page_ops::cache_line_starts(addr, len, line) {
        // SAFETY: `dc civac` writes back and drops the line holding `a`, a
        // mapped kernel address; memory keeps the line's data.
        unsafe { core::arch::asm!("dc civac, {0}", in(reg) a, options(nostack)) };
    }
    // SAFETY: a barrier only.
    unsafe { core::arch::asm!("dsb sy", options(nostack)) };
}

/// Make code just written (and, without CTR_EL0.IDC, cleaned with
/// [`clean_dcache_to_pou`]) visible to instruction fetch on every CPU:
/// invalidate the instruction caches to the inner-shareable domain, unless
/// CTR_EL0.DIC says they need none. Another CPU synchronises its context
/// on its next exception return to EL0.
pub fn sync_icache_all() {
    if ctr_el0() & CTR_DIC != 0 {
        // SAFETY: barriers only.
        unsafe { core::arch::asm!("dsb ish", "isb", options(nostack)) };
    } else {
        // SAFETY: cache maintenance and barriers only.
        unsafe { core::arch::asm!("dsb ish", "ic ialluis", "dsb ish", "isb", options(nostack)) };
    }
}

/// Serial initialization for compatibility with the arch-generic interface.
#[allow(dead_code)] // Arch-generic interface -- AArch64 serial initialized differently
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(platform::PL011)
}

/// Flush TLB for a specific virtual address. Called via
/// `crate::arch::tlb_flush_address()`.
pub fn tlb_flush_address(addr: u64) {
    // SAFETY: `tlbi vaae1is` invalidates the EL1 entries for this page, any
    // ASID, on every CPU in the Inner Shareable domain (MEM-ARCH-03: the
    // non-shareable `vae1` reached only the local CPU). `dsb ishst` orders
    // the page-table update before it; `dsb ish` waits for completion
    // everywhere; `isb` resynchronises this CPU.
    unsafe {
        let page_addr = (addr >> 12) & 0x0000_0FFF_FFFF_FFFF;
        core::arch::asm!("dsb ishst", "tlbi vaae1is, {}", "dsb ish", "isb",
            in(reg) page_addr, options(nostack));
    }
}

/// Flush entire TLB. Called via `crate::arch::tlb_flush_all()`.
pub fn tlb_flush_all() {
    // SAFETY: `tlbi vmalle1is` invalidates all EL1 entries on every CPU in
    // the Inner Shareable domain (MEM-ARCH-03); the barriers order and
    // complete it as in `tlb_flush_address`.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            options(nostack)
        );
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
