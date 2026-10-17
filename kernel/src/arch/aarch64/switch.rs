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
//!   user thread:   [x19 .. x30] [UserFrame]   x30 = user_trampoline
//! ```
//!
//! A user thread's kernel stack top is where its exceptions from EL0 land:
//! the return to EL0 leaves SP_EL1 there (ADR 0016).
//!
//! A task's address space is its TTBR0_EL1 root (the lower half); the kernel
//! half is TTBR1's, shared by all (ADR 0015). Kernel tasks carry the empty
//! root boot.S installs.

use core::arch::naked_asm;

/// Bytes of a switch frame: 12 registers, a multiple of 16 (AAPCS64 keeps
/// `sp` 16-byte aligned).
const FRAME: usize = 96;

/// A user thread's registers as the EL0 exception entry saves them on its
/// kernel stack, and as the return to EL0 restores them
/// (`veridian_aarch64_return_to_user`): x0-x30, then SP_EL0, ELR_EL1 and
/// SPSR_EL1. 272 bytes, a multiple of 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct UserFrame {
    pub x: [u64; 31],
    pub sp: u64,
    pub elr: u64,
    pub spsr: u64,
}

const _: () = assert!(core::mem::size_of::<UserFrame>() == 272);

impl UserFrame {
    /// The system call number (x8) and the first argument and result (x0).
    pub const X8: usize = 8;
    pub const X0: usize = 0;
}

/// The SPSR to return to EL0 with: EL0t (M = 0), AArch64, no interrupt
/// masked (D, A, I, F clear), single-step and illegal-return clear; only the
/// condition flags (NZCV) and BTYPE are kept from `spsr`. BTYPE (bits
/// 11:10) records an indirect branch whose target was not yet checked, so
/// an interrupt between the branch and its target does not skip the BTI
/// check (Linux keeps it too); at EL0 it can only make the program fault.
pub fn user_spsr(spsr: u64) -> u64 {
    spsr & USER_SPSR_BITS
}

/// The SPSR bits a user program controls: NZCV and BTYPE.
pub const USER_SPSR_BITS: u64 = 0xF000_0C00;
/// SPSR BTYPE as after a BLR (`PSR_BTYPE_C`), for entering a handler.
pub const BTYPE_MASK: u64 = 0xC00;
pub const BTYPE_C: u64 = 0b10 << 10;

/// First code of a new user thread: finish the switch, act on a fatal
/// signal sent before it first ran, then enter EL0 with the `UserFrame`
/// at `sp`.
#[unsafe(naked)]
unsafe extern "C" fn user_trampoline() {
    naked_asm!(
        "bl {finish}",
        "bl {check}",
        "b veridian_aarch64_return_to_user",
        finish = sym crate::sched::dispatch::finish_switch,
        check = sym crate::process::user_return_check,
    );
}

/// Seed a kernel stack so that switching to it enters EL0 with `frame`.
/// Returns the saved stack pointer.
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
        regs.add(11).write(user_trampoline as *const () as u64); // x30
    }
    sp
}

/// A copy of the user registers the exception in progress saved (the
/// per-CPU block's `syscall_frame`): where the current system call returns
/// to. `None` outside one.
pub fn live_user_frame() -> Option<UserFrame> {
    // SAFETY: this CPU's own block; `syscall_frame` is the current task's
    // frame on its kernel stack (set by the exception entry and switched
    // with the task), or 0.
    let frame =
        unsafe { (*crate::arch::percpu::this_arch_cpu()).syscall_frame } as *const UserFrame;
    if frame.is_null() {
        return None;
    }
    // SAFETY: as above, a live frame.
    Some(unsafe { *frame })
}

/// The user TLS register (TPIDR_EL0) of the running thread.
pub fn read_tls() -> u64 {
    let v: u64;
    // SAFETY: reading TPIDR_EL0 has no side effects.
    unsafe { core::arch::asm!("mrs {0}, tpidr_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// Set the user TLS register (TPIDR_EL0) for the thread about to run.
pub fn write_tls(value: u64) {
    // SAFETY: TPIDR_EL0 is user state only; the kernel does not use it.
    unsafe { core::arch::asm!("msr tpidr_el0, {0}", in(reg) value, options(nomem, nostack)) };
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

/// The TTBR0_EL1 ASID field: bits 63:48 (55:48 with 8-bit ASIDs).
const TTBR_ASID_SHIFT: u32 = 48;

/// The whole TTBR0_EL1 value: the root of the active lower half and its
/// ASID. A task saves this, so it comes back with the same ASID.
pub fn current_ttbr0() -> u64 {
    let ttbr0: u64;
    // SAFETY: reading TTBR0_EL1 has no side effects at EL1.
    unsafe { core::arch::asm!("mrs {0}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack)) };
    ttbr0
}

/// The ASIDs of the user address spaces (HA-26), by root.
#[cfg(feature = "alloc")]
static ASIDS: spin::Mutex<Option<crate::mm::asid::AsidAllocator>> = spin::Mutex::new(None);

/// How many ASIDs the CPU tags with: 65536 when the boot code enabled
/// 16-bit ASIDs (TCR_EL1.AS, bit 36), else 256.
#[cfg(feature = "alloc")]
fn asid_limit() -> usize {
    let tcr: u64;
    // SAFETY: reading TCR_EL1 has no side effects at EL1.
    unsafe { core::arch::asm!("mrs {0}, tcr_el1", out(reg) tcr, options(nomem, nostack)) };
    if tcr & (1 << 36) != 0 {
        1 << 16
    } else {
        1 << 8
    }
}

/// Run `f` on the ASID allocator with IRQs masked: the switch (IRQs masked)
/// may take the lock, so no holder may be interrupted into a switch.
#[cfg(feature = "alloc")]
fn with_asids<R>(f: impl FnOnce(&mut crate::mm::asid::AsidAllocator) -> R) -> R {
    let irq = irq_save();
    let r = f(ASIDS
        .lock()
        .get_or_insert_with(|| crate::mm::asid::AsidAllocator::new(asid_limit())));
    if irq {
        irq_enable();
    }
    r
}

/// The TTBR0_EL1 value for user address space `root`: the root with its
/// ASID, allocated on first use. ASID 0 (no ASID) when all are in use: the
/// switch then invalidates ASID 0's entries instead.
#[cfg(feature = "alloc")]
pub fn ttbr0_for(root: u64) -> u64 {
    let root = root & TTBR_BADDR;
    let asid = with_asids(|a| a.get_or_allocate(root));
    root | (asid as u64) << TTBR_ASID_SHIFT
}

/// Release the ASID of user address space `root` once no CPU runs in it
/// (its tables are being freed): its TLB entries are invalidated on every
/// CPU before the ASID can be handed out again.
#[cfg(feature = "alloc")]
pub fn release_asid(root: u64) {
    with_asids(|a| {
        if let Some(asid) = a.release(root & TTBR_BADDR) {
            // SAFETY: `tlbi aside1is` invalidates the entries tagged `asid`
            // (bits 63:48 of the operand) on every CPU in the Inner
            // Shareable domain; `dsb ish` waits for that everywhere. Done
            // under the allocator lock, so before any reuse of the ASID.
            unsafe {
                core::arch::asm!(
                    "dsb ishst",
                    "tlbi aside1is, {0}",
                    "dsb ish",
                    "isb",
                    in(reg) (asid as u64) << TTBR_ASID_SHIFT,
                    options(nostack)
                );
            }
        }
    });
}

/// Load `ttbr0` (a root and its ASID, see [`ttbr0_for`]) into TTBR0, if it
/// is not the active value. With an ASID the TLB is left alone: the other
/// spaces' entries carry other ASIDs and do not match (HA-26). Without one
/// (ASID 0: the empty root, a bare root, or ASIDs exhausted) ASID 0's
/// entries are invalidated on this CPU, as another space may have left
/// some.
///
/// # Safety
/// The root in `ttbr0` must be a valid level-0 table (the kernel half is
/// TTBR1's, so it need not map the kernel), and a nonzero ASID must be the
/// one `ttbr0_for` gave that root.
pub unsafe fn switch_address_space(ttbr0: u64) {
    if ttbr0 == 0 || ttbr0 == current_ttbr0() {
        return;
    }
    // SAFETY: per the contract; the kernel keeps running from TTBR1.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "msr ttbr0_el1, {0}",
            "isb",
            in(reg) ttbr0,
            options(nostack)
        );
        if ttbr0 >> TTBR_ASID_SHIFT == 0 {
            core::arch::asm!("tlbi aside1, xzr", "dsb nsh", "isb", options(nostack));
        }
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
