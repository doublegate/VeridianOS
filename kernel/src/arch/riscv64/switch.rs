//! Task switching on kernel stacks (ADR 0006), RISC-V.
//!
//! As on x86_64 (`arch/x86_64/switch.rs`): a switch saves the callee-saved
//! registers (`ra`, `s0`-`s11`; the psABI's) on the outgoing task's stack,
//! stores its stack pointer, loads the incoming one's and restores its
//! registers; the `ret` continues wherever that task last switched out. A
//! new kernel thread's stack is seeded with a switch frame whose `ra` is a
//! start trampoline:
//!
//! ```text
//!   kernel thread: [ra s0 s1 .. s11] padding    ra = kthread_trampoline,
//!                                               s0 = entry, s1 = arg
//!   user thread:   [ra s0 .. s11] padding [UserFrame]   ra = user_trampoline
//! ```

use core::arch::naked_asm;

/// Bytes of a switch frame: 13 registers, rounded up to keep `sp` 16-byte
/// aligned (psABI).
const FRAME: usize = 112;

/// A user thread's registers as the trap entry saves them on its kernel
/// stack, and as the return to user mode restores them
/// (`veridian_riscv_return_to_user`): `x[i]` is register x<i> (`x[0]` is
/// unused), then `sepc` and `sstatus`. 272 bytes, a multiple of 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct UserFrame {
    pub x: [u64; 32],
    pub sepc: u64,
    pub sstatus: u64,
}

const _: () = assert!(core::mem::size_of::<UserFrame>() == 272);

impl UserFrame {
    pub const RA: usize = 1;
    pub const SP: usize = 2;
    pub const TP: usize = 4;
    pub const A0: usize = 10;
    pub const A7: usize = 17;
}

/// sstatus bits for a return to user mode.
const SSTATUS_SPP: u64 = 1 << 8;
const SSTATUS_SPIE: u64 = 1 << 5;
const SSTATUS_SIE: u64 = 1 << 1;
const SSTATUS_SUM: u64 = 1 << 18;
const SSTATUS_FS: u64 = 3 << 13;

/// sstatus.FS = Clean: the FP registers hold the task's saved state.
const SSTATUS_FS_CLEAN: u64 = 2 << 13;

/// The `sstatus` to return to user mode with: U-mode (SPP clear),
/// interrupts on there (SPIE), kernel interrupts off until the `sret`, no
/// kernel access to user pages (SUM clear), and FP Clean when the harts
/// have F/D (user FP state is switched with the task, `fpu`), else Off (an
/// FP instruction traps). Every other bit is kept.
pub fn user_sstatus(sstatus: u64) -> u64 {
    let fs = if super::fpu::present() {
        SSTATUS_FS_CLEAN
    } else {
        0
    };
    (sstatus & !(SSTATUS_SPP | SSTATUS_SIE | SSTATUS_SUM | SSTATUS_FS)) | SSTATUS_SPIE | fs
}

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
        "addi sp, sp, -112",
        "sd ra, 0(sp)",
        "sd s0, 8(sp)",
        "sd s1, 16(sp)",
        "sd s2, 24(sp)",
        "sd s3, 32(sp)",
        "sd s4, 40(sp)",
        "sd s5, 48(sp)",
        "sd s6, 56(sp)",
        "sd s7, 64(sp)",
        "sd s8, 72(sp)",
        "sd s9, 80(sp)",
        "sd s10, 88(sp)",
        "sd s11, 96(sp)",
        "sd sp, 0(a0)",
        "mv sp, a1",
        "ld ra, 0(sp)",
        "ld s0, 8(sp)",
        "ld s1, 16(sp)",
        "ld s2, 24(sp)",
        "ld s3, 32(sp)",
        "ld s4, 40(sp)",
        "ld s5, 48(sp)",
        "ld s6, 56(sp)",
        "ld s7, 64(sp)",
        "ld s8, 72(sp)",
        "ld s9, 80(sp)",
        "ld s10, 88(sp)",
        "ld s11, 96(sp)",
        "addi sp, sp, 112",
        "ret",
    );
}

/// First code of a new kernel thread: finish the switch, enable interrupts
/// (`schedule` switched with them off), then run `entry(arg)` (`s0`, `s1`)
/// and exit if it returns.
#[unsafe(naked)]
unsafe extern "C" fn kthread_trampoline() {
    naked_asm!(
        // sp is 16-byte aligned here (seeded so), as a call requires.
        "call {finish}",
        "csrsi sstatus, 2",
        "mv a0, s1",
        "jalr s0",
        "call {exit}",
        "unimp",
        finish = sym crate::sched::dispatch::finish_switch,
        exit = sym crate::sched::dispatch::exit_kernel_thread,
    );
}

/// First code of a new user thread: finish the switch, act on a fatal
/// signal sent before it first ran, then enter user mode with the
/// `UserFrame` at `sp`.
#[unsafe(naked)]
unsafe extern "C" fn user_trampoline() {
    naked_asm!(
        "call {finish}",
        "call {check}",
        "j veridian_riscv_return_to_user",
        finish = sym crate::sched::dispatch::finish_switch,
        check = sym crate::process::user_return_check,
    );
}

/// Seed a kernel stack so that switching to it enters user mode with
/// `frame`. Returns the saved stack pointer.
///
/// # Safety
/// The top `size_of::<UserFrame>() + FRAME + 16` bytes below `stack_top`
/// must be writable and unused.
pub unsafe fn seed_user_thread(stack_top: usize, frame: &UserFrame) -> usize {
    let frame_at = (stack_top & !0xF) - core::mem::size_of::<UserFrame>();
    let sp = frame_at - FRAME;
    let regs = sp as *mut u64;
    // SAFETY: within the caller's writable stack top; both areas are
    // 16-byte aligned.
    unsafe {
        (frame_at as *mut UserFrame).write(*frame);
        for i in 0..FRAME / 8 {
            regs.add(i).write(0);
        }
        regs.write(user_trampoline as *const () as u64); // ra
    }
    sp
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
    // SAFETY: within the caller's writable stack top; 8-byte aligned.
    unsafe {
        for i in 0..FRAME / 8 {
            frame.add(i).write(0);
        }
        frame.write(kthread_trampoline as *const () as u64); // ra
        frame.add(1).write(entry as *const () as u64); // s0
        frame.add(2).write(arg as u64); // s1
    }
    sp
}

/// The physical root of the active address space (from `satp`).
pub fn current_root() -> u64 {
    crate::mm::get_kernel_page_table() as u64
}

/// Load the address space whose root table is at physical `root`, if it is
/// not the active one (Sv48).
///
/// # Safety
/// `root` must be a valid root table that maps the kernel half.
pub unsafe fn switch_address_space(root: u64) {
    if root == 0 || root == current_root() {
        return;
    }
    let satp = (9u64 << 60) | (root >> 12);
    // SAFETY: per the contract the new tables map the kernel, so execution
    // continues; the fence drops translations of the old space.
    unsafe {
        core::arch::asm!(
            "csrw satp, {0}",
            "sfence.vma",
            in(reg) satp,
            options(nostack)
        );
    }
}

/// Disable supervisor interrupts; returns whether they were enabled.
pub fn irq_save() -> bool {
    let old: usize;
    // SAFETY: clears sstatus.SIE and returns the previous sstatus.
    unsafe { core::arch::asm!("csrrci {0}, sstatus, 2", out(reg) old, options(nomem, nostack)) };
    old & 2 != 0
}

/// Enable supervisor interrupts.
pub fn irq_enable() {
    // SAFETY: sets sstatus.SIE; the trap vector is installed.
    unsafe { core::arch::asm!("csrsi sstatus, 2", options(nomem, nostack)) };
}

/// Wait for an interrupt with interrupts disabled, then enable them so it
/// is taken. `wfi` resumes on a pending, enabled-in-`sie` interrupt even
/// while `sstatus.SIE` is clear, so a wake between the caller's check and
/// the `wfi` is not lost (the RISC-V form of `sti; hlt`).
pub fn wait_for_interrupt() {
    // SAFETY: wfi only waits; sstatus.SIE is set afterwards.
    unsafe { core::arch::asm!("wfi", "csrsi sstatus, 2", options(nomem, nostack)) };
}

/// Interrupt logical CPU `cpu` (an SBI IPI to its hart): it ends a `wfi` or
/// reaches its next preemption point.
pub fn kick(cpu: usize) {
    // SAFETY: the per-CPU block of a started CPU; `hw_id` is its hart id,
    // written before the CPU dispatches.
    let hart = unsafe { (*crate::arch::percpu::arch_cpu_ptr(cpu)).hw_id };
    let _ = crate::arch::riscv::sbi::send_ipi(hart as usize);
}
