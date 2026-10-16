//! Task switching on kernel stacks (ADR 0006), AArch64.
//!
//! As on x86_64 and RISC-V (`arch/riscv64/switch.rs`): a switch saves the
//! callee-saved registers (x19-x28, the frame pointer x29 and the link
//! register x30; AAPCS64) on the outgoing task's stack, stores its stack
//! pointer, loads the incoming one's and restores its registers; the `ret`
//! continues wherever that task last switched out. The kernel is
//! soft-float (N-178), so no FP/SIMD register is live in kernel code. A new
//! kernel thread's stack is seeded with a switch frame whose x30 is a start
//! trampoline:
//!
//! ```text
//!   kernel thread: [x19 x20 .. x28 x29 x30]   x30 = kthread_trampoline,
//!                                             x19 = entry, x20 = arg
//! ```
//!
//! A task's address space is its TTBR0_EL1 root (the lower half); the kernel
//! half is TTBR1's, shared by all (ADR 0015). Kernel tasks carry the empty
//! root boot.S installs.

use core::arch::naked_asm;

/// Bytes of a switch frame: 12 registers, a multiple of 16 (AAPCS64 keeps
/// `sp` 16-byte aligned).
const FRAME: usize = 96;

/// Save the callee-saved registers and stack pointer of the running task
/// into `*prev_sp`, and resume the task whose saved stack pointer is
/// `next_sp`. Returns when the outgoing task is switched back in.
///
/// # Safety
/// Interrupts must be disabled. `prev_sp` must be writable; `next_sp` must be
/// a stack pointer saved by this function or produced by
/// [`seed_kernel_thread`], on a stack that stays mapped while the task runs.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_stacks(prev_sp: *mut usize, next_sp: usize) {
    naked_asm!(
        "sub sp, sp, #96",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "mov x9, sp",
        "str x9, [x0]",
        "mov sp, x1",
        "ldp x19, x20, [sp, #0]",
        "ldp x21, x22, [sp, #16]",
        "ldp x23, x24, [sp, #32]",
        "ldp x25, x26, [sp, #48]",
        "ldp x27, x28, [sp, #64]",
        "ldp x29, x30, [sp, #80]",
        "add sp, sp, #96",
        "ret",
    );
}

/// First code of a new kernel thread: finish the switch, enable interrupts
/// (`schedule` switched with them off), then run `entry(arg)` (x19, x20)
/// and exit if it returns.
#[unsafe(naked)]
unsafe extern "C" fn kthread_trampoline() {
    naked_asm!(
        // sp is 16-byte aligned here (seeded so), as a call requires.
        "bl {finish}",
        "msr daifclr, #2",
        "mov x0, x20",
        "blr x19",
        "bl {exit}",
        "udf #0",
        finish = sym crate::sched::dispatch::finish_switch,
        exit = sym crate::sched::dispatch::exit_kernel_thread,
    );
}

/// Seed a kernel stack (top `stack_top`) so that switching to it runs
/// `entry(arg)`. Returns the saved stack pointer.
///
/// # Safety
/// The top `FRAME + 16` bytes below `stack_top` must be writable and unused.
pub unsafe fn seed_kernel_thread(
    stack_top: usize,
    entry: extern "C" fn(usize),
    arg: usize,
) -> usize {
    let sp = (stack_top & !0xF) - FRAME;
    let frame = sp as *mut u64;
    // SAFETY: within the caller's writable stack top; 16-byte aligned.
    unsafe {
        for i in 0..FRAME / 8 {
            frame.add(i).write(0);
        }
        frame.write(entry as *const () as u64); // x19
        frame.add(1).write(arg as u64); // x20
        frame.add(11).write(kthread_trampoline as *const () as u64); // x30
    }
    sp
}

/// The table address in TTBR0_EL1: a 4 KiB-aligned level-0 table below
/// 2^48 (the ASID is in bits 63:48, CnP in bit 0).
const TTBR_BADDR: u64 = 0x0000_FFFF_FFFF_F000;

/// The physical root of the active lower-half address space (TTBR0_EL1).
pub fn current_root() -> u64 {
    let ttbr0: u64;
    // SAFETY: reading TTBR0_EL1 has no side effects at EL1.
    unsafe { core::arch::asm!("mrs {0}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack)) };
    ttbr0 & TTBR_BADDR
}

/// Load the address space whose level-0 table is at physical `root` into
/// TTBR0, if it is not the active one. There are no ASIDs yet (HA-26), so
/// the TLB is invalidated: the old space's translations must not survive.
///
/// # Safety
/// `root` must be a valid level-0 table (the kernel half is TTBR1's, so it
/// need not map the kernel).
pub unsafe fn switch_address_space(root: u64) {
    if root == 0 || root == current_root() {
        return;
    }
    // SAFETY: per the contract; the kernel keeps running from TTBR1.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "msr ttbr0_el1, {0}",
            "isb",
            "tlbi vmalle1",
            "dsb nsh",
            "isb",
            in(reg) root,
            options(nostack)
        );
    }
}

/// Disable IRQs; returns whether they were enabled.
pub fn irq_save() -> bool {
    let daif: u64;
    // SAFETY: reads DAIF, then masks IRQs on this CPU.
    unsafe {
        core::arch::asm!("mrs {0}, daif", "msr daifset, #2", out(reg) daif, options(nomem, nostack));
    }
    daif & 0x80 == 0
}

/// Enable IRQs.
pub fn irq_enable() {
    // SAFETY: unmasks IRQs; the vectors and the GIC CPU interface are set up.
    unsafe { core::arch::asm!("msr daifclr, #2", options(nomem, nostack)) };
}

/// Wait for an interrupt with IRQs masked, then unmask them so it is taken.
/// WFI resumes on a pending IRQ even while PSTATE.I masks it (as Linux's
/// `cpu_do_idle` relies on), so a wake between the caller's check and the
/// WFI is not lost.
pub fn wait_for_interrupt() {
    // SAFETY: WFI only waits; IRQs are unmasked afterwards.
    unsafe { core::arch::asm!("dsb sy", "wfi", "msr daifclr, #2", options(nomem, nostack)) };
}

/// Interrupt logical CPU `cpu` (an SGI to its GIC CPU interface): it ends a
/// WFI or reaches its next scheduling point. On QEMU `virt` the CPU
/// interface number is the MPIDR Aff0 (`hw_id`); a GICv2 board can differ
/// (HA-09).
pub fn kick(cpu: usize) {
    // SAFETY: the per-CPU block of a started CPU; `hw_id` is its MPIDR
    // affinity, written before the CPU dispatches.
    let hw = unsafe { (*crate::arch::percpu::arch_cpu_ptr(cpu)).hw_id };
    super::gic::send_sgi_raw(hw & 0xFF, super::exceptions::SCHED_WAKE_SGI);
}
