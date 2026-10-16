//! RISC-V 64-bit architecture support.
//!
//! Provides initialization, interrupt control, serial I/O, and I/O port
//! stubs for the RISC-V 64-bit platform. Uses the SBI (Supervisor Binary
//! Interface) for machine-mode services.

pub mod boot;
pub mod bootstrap;
pub mod entry;
pub mod serial;
#[cfg(feature = "smp")]
pub mod smp;
pub mod switch;
pub mod usercopy;
pub mod usermode;

// Re-export context, PLIC, and timer from parent riscv module
pub use super::riscv::{context, plic, timer};

// Supervisor trap vector (Direct mode, 4-byte aligned).
//
// `sscratch` tells where a trap came from (Linux's convention): 0 while the
// kernel runs; in user mode it holds this CPU's per-CPU block, and `tp`
// belongs to the user (TLS). The vector swaps the two: a non-zero `tp` after
// the swap means user mode.
//
// - From the kernel: the trap runs on the interrupted kernel stack. The
//   caller-saved registers plus sepc/sstatus are saved; the Rust handler
//   preserves the callee-saved ones and may change the saved sepc (the
//   user-copy fixup). The kernel is soft-float (N-178): no FP state.
// - From user mode: the user `sp` goes to the per-CPU scratch slot, the task's
//   kernel stack comes from the per-CPU block (`kernel_rsp`, set at each
//   switch), and the whole register file is saved there as a
//   `switch::UserFrame`. `veridian_riscv_return_to_user` restores it; a new
//   user task starts there too.
//
// Per-CPU block (`arch::percpu::ArchCpu`): 0x00 kernel stack for user
// entries, 0x08 user sp scratch, 0x10 the current user frame.
core::arch::global_asm!(
    ".section .text",
    ".balign 4",
    ".global veridian_riscv_trap",
    "veridian_riscv_trap:",
    "    csrrw tp, sscratch, tp",
    "    bnez tp, 4f",
    "    csrrw tp, sscratch, tp",
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
    "    mv a3, sp",
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
    // From user mode: tp = per-CPU block, sscratch = user tp.
    "4:",
    "    sd sp, 8(tp)",
    "    ld sp, 0(tp)",
    "    addi sp, sp, -272",
    "    sd x1, 8(sp)",
    "    sd x3, 24(sp)",
    "    sd x5, 40(sp)",
    "    sd x6, 48(sp)",
    "    sd x7, 56(sp)",
    "    sd x8, 64(sp)",
    "    sd x9, 72(sp)",
    "    sd x10, 80(sp)",
    "    sd x11, 88(sp)",
    "    sd x12, 96(sp)",
    "    sd x13, 104(sp)",
    "    sd x14, 112(sp)",
    "    sd x15, 120(sp)",
    "    sd x16, 128(sp)",
    "    sd x17, 136(sp)",
    "    sd x18, 144(sp)",
    "    sd x19, 152(sp)",
    "    sd x20, 160(sp)",
    "    sd x21, 168(sp)",
    "    sd x22, 176(sp)",
    "    sd x23, 184(sp)",
    "    sd x24, 192(sp)",
    "    sd x25, 200(sp)",
    "    sd x26, 208(sp)",
    "    sd x27, 216(sp)",
    "    sd x28, 224(sp)",
    "    sd x29, 232(sp)",
    "    sd x30, 240(sp)",
    "    sd x31, 248(sp)",
    "    ld t0, 8(tp)",
    "    sd t0, 16(sp)",
    "    csrr t0, sscratch",
    "    sd t0, 32(sp)",
    "    csrw sscratch, zero",
    "    csrr t0, sepc",
    "    sd t0, 256(sp)",
    "    csrr t0, sstatus",
    "    sd t0, 264(sp)",
    "    sd sp, 16(tp)",
    "    mv a0, sp",
    "    call {user_handler}",
    // Return to user mode with the UserFrame at sp (also a new task's
    // first entry). Interrupts off until the sret; sscratch = this CPU's
    // block again.
    ".global veridian_riscv_return_to_user",
    "veridian_riscv_return_to_user:",
    "    csrci sstatus, 2",
    "    ld t0, 264(sp)",
    "    csrw sstatus, t0",
    "    ld t0, 256(sp)",
    "    csrw sepc, t0",
    "    csrw sscratch, tp",
    "    ld x1, 8(sp)",
    "    ld x3, 24(sp)",
    "    ld x5, 40(sp)",
    "    ld x6, 48(sp)",
    "    ld x7, 56(sp)",
    "    ld x8, 64(sp)",
    "    ld x9, 72(sp)",
    "    ld x10, 80(sp)",
    "    ld x11, 88(sp)",
    "    ld x12, 96(sp)",
    "    ld x13, 104(sp)",
    "    ld x14, 112(sp)",
    "    ld x15, 120(sp)",
    "    ld x16, 128(sp)",
    "    ld x17, 136(sp)",
    "    ld x18, 144(sp)",
    "    ld x19, 152(sp)",
    "    ld x20, 160(sp)",
    "    ld x21, 168(sp)",
    "    ld x22, 176(sp)",
    "    ld x23, 184(sp)",
    "    ld x24, 192(sp)",
    "    ld x25, 200(sp)",
    "    ld x26, 208(sp)",
    "    ld x27, 216(sp)",
    "    ld x28, 224(sp)",
    "    ld x29, 232(sp)",
    "    ld x30, 240(sp)",
    "    ld x31, 248(sp)",
    "    ld tp, 32(sp)",
    "    ld sp, 16(sp)",
    "    sret",
    handler = sym riscv_trap,
    user_handler = sym riscv_user_trap,
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

/// scause exception codes.
const EXC_ILLEGAL: usize = 2;
const EXC_BREAKPOINT: usize = 3;
const EXC_ECALL_U: usize = 8;
const EXC_INSN_PAGE: usize = 12;
const EXC_LOAD_PAGE: usize = 13;
const EXC_STORE_PAGE: usize = 15;

/// Rust side of the vector for traps taken in the kernel. Returns for
/// handled interrupts and for a user-copy fault (`frame[16]` is the saved
/// sepc, which it then points at the fixup).
extern "C" fn riscv_trap(scause: usize, sepc: usize, stval: usize, frame: *mut u64) {
    if scause == SCAUSE_INTERRUPT | IRQ_S_TIMER {
        super::riscv::timer::handle_interrupt(false);
        return;
    }
    if scause == SCAUSE_INTERRUPT | IRQ_S_SOFT {
        ack_ipi();
        return;
    }
    if matches!(scause, EXC_LOAD_PAGE | EXC_STORE_PAGE) {
        if let Some(fixup) = usercopy::fixup_for(sepc) {
            // A user copy touched a page that is not present: demand paging
            // first (the page may just not be backed yet), else EFAULT.
            let info = crate::mm::page_fault::from_riscv(scause as u64, stval as u64, sepc as u64);
            if !crate::mm::page_fault::resolve_user_copy_fault(&info) {
                // SAFETY: `frame` is this trap's save area (the vector
                // passes its sp); slot 16 is the saved sepc it restores.
                unsafe { frame.add(16).write(fixup as u64) };
            }
            return;
        }
    }
    riscv_fatal_trap(scause, sepc, stval)
}

/// Acknowledge an IPI (SBI send_ipi sets sip.SSIP): clear SSIP, or it stays
/// pending and traps again on return.
fn ack_ipi() {
    // SAFETY: clears only the supervisor software-interrupt pending bit.
    unsafe { core::arch::asm!("csrc sip, {0}", in(reg) 2usize, options(nomem, nostack)) };
    crate::arch::percpu::note_ipi();
}

/// Rust side of the vector for traps from user mode, on the task's kernel
/// stack with its registers in `f`: system calls, interrupts, faults.
/// Returns to `veridian_riscv_return_to_user`.
extern "C" fn riscv_user_trap(f: &mut switch::UserFrame) {
    let scause: usize;
    let stval: usize;
    // SAFETY: reading the trap CSRs has no side effects.
    unsafe {
        core::arch::asm!("csrr {0}, scause", out(reg) scause, options(nomem, nostack));
        core::arch::asm!("csrr {0}, stval", out(reg) stval, options(nomem, nostack));
    }
    match scause {
        s if s == SCAUSE_INTERRUPT | IRQ_S_TIMER => super::riscv::timer::handle_interrupt(true),
        s if s == SCAUSE_INTERRUPT | IRQ_S_SOFT => ack_ipi(),
        EXC_ECALL_U => {
            // Past the ecall; the call runs with interrupts on (as x86_64's
            // system calls do since N-139).
            f.sepc += 4;
            switch::irq_enable();
            let a = |i: usize| f.x[switch::UserFrame::A0 + i] as usize;
            let ret = crate::syscall::riscv64_syscall(
                f.x[switch::UserFrame::A7] as usize,
                a(0),
                a(1),
                a(2),
                a(3),
                a(4),
            );
            f.x[switch::UserFrame::A0] = ret as u64;
        }
        EXC_INSN_PAGE | EXC_LOAD_PAGE | EXC_STORE_PAGE => {
            switch::irq_enable();
            let info = crate::mm::page_fault::from_riscv(scause as u64, stval as u64, f.sepc);
            if crate::mm::page_fault::handle_page_fault(info).is_err() {
                kill_user(11); // SIGSEGV
            }
        }
        EXC_ILLEGAL => kill_user(4),    // SIGILL
        EXC_BREAKPOINT => kill_user(5), // SIGTRAP
        _ => kill_user(7),              // SIGBUS
    }
    exit_to_user(f);
}

/// Last step before every return to user mode: preemption, a fatal signal
/// acted on, and a frame that can only return to user mode. Signal handlers
/// are not delivered on RISC-V yet (sprint E).
fn exit_to_user(f: &mut switch::UserFrame) {
    switch::irq_enable();
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::preempt_user();
    crate::process::user_return_check();
    switch::irq_save();
    f.sstatus = switch::user_sstatus(f.sstatus);
    if !crate::syscall::userspace::is_user_address(f.sepc as usize) {
        kill_user(11);
    }
}

/// End the current process with `signal` (a fault it did not handle).
fn kill_user(signal: u32) -> ! {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        let _ = crate::syscall::process::exit_current(0, signal);
    }
    panic!(
        "RISC-V user fault (signal {}) outside a dispatched thread",
        signal
    );
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

    let satp: usize;
    // SAFETY: reads the address-translation CSR; no side effects.
    unsafe { core::arch::asm!("csrr {}, satp", out(reg) satp, options(nomem, nostack)) };
    println!(
        "[RISCV64] Paging: satp mode {} root {:#x}, kernel at {:#x}",
        satp >> 60,
        (satp & 0xFFF_FFFF_FFFF) << 12,
        init as *const () as usize
    );

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
