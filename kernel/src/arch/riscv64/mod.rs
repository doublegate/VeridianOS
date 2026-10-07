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

// Supervisor trap vector (Direct mode, 4-byte aligned).
//
// Kernel traps run on the interrupted kernel stack. sscratch is 0 while the
// kernel runs; a trap from U-mode (where sscratch would hold the kernel
// stack) is detected by the swap and treated as fatal until user mode is
// supported on RISC-V. The caller-saved registers plus sepc/sstatus are
// saved; the Rust handler preserves the callee-saved ones. The kernel uses
// no floating point (project rule), so FP state is not saved.
//
// Supervisor timer interrupts are handled; any other trap is a kernel bug
// and stops the system with scause/sepc/stval (before a vector existed,
// such a trap silently restarted boot -- N-13).
core::arch::global_asm!(
    ".section .text",
    ".balign 4",
    ".global veridian_riscv_trap",
    "veridian_riscv_trap:",
    "    csrrw sp, sscratch, sp",
    "    bnez sp, 2f",
    "    csrrw sp, sscratch, sp",
    "    addi sp, sp, -144",
    "    sd ra, 0(sp)",
    "    sd t0, 8(sp)",
    "    sd t1, 16(sp)",
    "    sd t2, 24(sp)",
    "    sd a0, 32(sp)",
    "    sd a1, 40(sp)",
    "    sd a2, 48(sp)",
    "    sd a3, 56(sp)",
    "    sd a4, 64(sp)",
    "    sd a5, 72(sp)",
    "    sd a6, 80(sp)",
    "    sd a7, 88(sp)",
    "    sd t3, 96(sp)",
    "    sd t4, 104(sp)",
    "    sd t5, 112(sp)",
    "    sd t6, 120(sp)",
    "    csrr t0, sepc",
    "    sd t0, 128(sp)",
    "    csrr t0, sstatus",
    "    sd t0, 136(sp)",
    "    csrr a0, scause",
    "    csrr a1, sepc",
    "    csrr a2, stval",
    "    call {handler}",
    "    ld t0, 136(sp)",
    "    csrw sstatus, t0",
    "    ld t0, 128(sp)",
    "    csrw sepc, t0",
    "    ld ra, 0(sp)",
    "    ld t0, 8(sp)",
    "    ld t1, 16(sp)",
    "    ld t2, 24(sp)",
    "    ld a0, 32(sp)",
    "    ld a1, 40(sp)",
    "    ld a2, 48(sp)",
    "    ld a3, 56(sp)",
    "    ld a4, 64(sp)",
    "    ld a5, 72(sp)",
    "    ld a6, 80(sp)",
    "    ld a7, 88(sp)",
    "    ld t3, 96(sp)",
    "    ld t4, 104(sp)",
    "    ld t5, 112(sp)",
    "    ld t6, 120(sp)",
    "    addi sp, sp, 144",
    "    sret",
    "2:",
    "    csrr a0, scause",
    "    csrr a1, sepc",
    "    csrr a2, stval",
    "    call {fatal}",
    "3:  j 3b",
    handler = sym riscv_trap,
    fatal = sym riscv_fatal_trap,
);

extern "C" {
    fn veridian_riscv_trap();
    static veridian_dtb_pa: u64;
}

/// Whether the hart supports Sv48, from the device tree's `mmu-type`
/// (`riscv,sv48` or `riscv,sv57`, which implies Sv48). Assumed true when the
/// device tree does not say (QEMU virt supports up to Sv57).
static SV48: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Whether the kernel's 4-level page tables (Sv48) can be used.
pub fn sv48_supported() -> bool {
    SV48.load(core::sync::atomic::Ordering::Relaxed)
}

/// Record the MMU type from the device tree.
fn read_mmu_type(dtb_pa: u64) {
    // SAFETY: dtb_pa is what firmware passed in a1 (saved by boot.S).
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(dtb_pa) }) else {
        return;
    };
    if let Some(mmu) = fdt.property(&["cpus", "cpu"], "mmu-type") {
        let mmu = mmu.split(|&c| c == 0).next().unwrap_or(&[]);
        let sv48 = mmu == b"riscv,sv48" || mmu == b"riscv,sv57";
        SV48.store(sv48, core::sync::atomic::Ordering::Relaxed);
        println!(
            "[RISCV64] MMU: {} ({})",
            core::str::from_utf8(mmu).unwrap_or("?"),
            if sv48 {
                "Sv48 available"
            } else {
                "no Sv48: user mode unavailable"
            }
        );
    }
}

/// scause: interrupt bit, and the supervisor software (IPI) and timer
/// interrupt codes.
const SCAUSE_INTERRUPT: usize = 1 << 63;
const IRQ_S_SOFT: usize = 1;
const IRQ_S_TIMER: usize = 5;

/// Rust side of the trap vector. Returns only for handled interrupts.
extern "C" fn riscv_trap(scause: usize, sepc: usize, stval: usize) {
    if scause == SCAUSE_INTERRUPT | IRQ_S_TIMER {
        super::riscv::timer::handle_interrupt();
        return;
    }
    if scause == SCAUSE_INTERRUPT | IRQ_S_SOFT {
        // An IPI (SBI send_ipi sets sip.SSIP). Acknowledge it by clearing
        // SSIP, or it stays pending and traps again on return.
        // SAFETY: clears only the supervisor software-interrupt pending bit.
        unsafe { core::arch::asm!("csrc sip, {0}", in(reg) 2usize, options(nomem, nostack)) };
        crate::arch::percpu::note_ipi();
        return;
    }
    riscv_fatal_trap(scause, sepc, stval)
}

/// Report an unexpected supervisor-mode trap and stop.
extern "C" fn riscv_fatal_trap(scause: usize, sepc: usize, stval: usize) -> ! {
    panic!(
        "unexpected supervisor trap: scause={:#x} sepc={:#x} stval={:#x}",
        scause, sepc, stval
    );
}

/// Called from bootstrap on RISC-V via `crate::arch::init()`.
pub fn init() {
    // Install the trap vector before anything else can trap. sscratch = 0
    // marks "running in the kernel" for the vector's U-mode check.
    // SAFETY: writing stvec only changes where supervisor traps go; the
    // target is the 4-byte-aligned vector defined above (Direct mode).
    unsafe {
        core::arch::asm!(
            "csrw sscratch, zero",
            "csrw stvec, {0}",
            in(reg) veridian_riscv_trap as unsafe extern "C" fn() as usize,
            options(nomem, nostack)
        );
    }

    // Initialize SBI (Supervisor Binary Interface)
    super::riscv::sbi::init();

    // Initialize PLIC (all sources disabled, threshold at 0).
    // This is safe before stvec is configured because no sources are enabled.
    if let Err(e) = super::riscv::plic::init() {
        println!("[RISCV64] WARNING: PLIC initialization failed: {}", e);
    }

    // Start from a clean slate: only sources enabled below are delivered.
    // SAFETY: clears the supervisor interrupt enables; no side effects.
    unsafe {
        core::arch::asm!("csrci sstatus, 2", options(nomem, nostack));
        core::arch::asm!("csrw sie, zero", options(nomem, nostack));
    }

    // Clock source and tick: timebase and Sstc from the device tree, then a
    // 1000 Hz supervisor timer interrupt. External interrupts (PLIC) stay
    // disabled.
    // SAFETY: veridian_dtb_pa is written once by boot.S before Rust runs.
    let dtb_pa = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(veridian_dtb_pa)) };
    super::riscv::timer::init_from_device_tree(dtb_pa);
    read_mmu_type(dtb_pa);
    super::riscv::timer::start(1000);

    // SAFETY: the trap vector is installed (above), so supervisor
    // interrupts can be taken.
    unsafe { core::arch::asm!("csrsi sstatus, 2", options(nomem, nostack)) };

    println!("[RISCV64] Architecture initialization complete (timer interrupts on)");
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
