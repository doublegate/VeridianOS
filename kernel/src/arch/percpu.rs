//! Architecture per-CPU block.
//!
//! One `ArchCpu` per logical CPU, in a static array that exists from the
//! first instruction (no heap). Each CPU finds its own block through a
//! register: `GS_BASE` on x86_64 (while in ring 0; ring-3 entries and exits
//! swap it with `KernelGsBase`), `TPIDR_EL1` on AArch64, `tp` on RISC-V. Entry
//! assembly addresses fields by fixed offset, so the layout is ABI (asserted
//! below).
//!
//! Logical CPU ids are dense: 0 is the CPU that booted the kernel, whatever
//! its hardware id, and secondaries are numbered by the boot CPU in firmware
//! table order. Hardware ids (APIC ID, MPIDR affinity, hartid) are kept in
//! `hw_id` and are never used as indices.
//!
//! This is the arch layer only; scheduler per-CPU state is
//! `sched::smp::PerCpuData` (ADR 0004, SMP bring-up).

use core::cell::UnsafeCell;

pub use crate::sched::smp::MAX_CPUS;

/// Per-CPU state used by entry code and interrupt handlers.
#[repr(C, align(64))]
pub struct ArchCpu {
    /// x86_64: kernel stack loaded by `syscall_entry` (`gs:[0x00]`).
    pub kernel_rsp: u64,
    /// x86_64: user stack saved by `syscall_entry` (`gs:[0x08]`).
    pub user_rsp: u64,
    /// x86_64: the `SyscallFrame` of the syscall this CPU is executing, or 0
    /// (`gs:[0x10]`). Per CPU, so one CPU's frame is never read as
    /// another's (N-35).
    pub syscall_frame: u64,
    /// Address of this block.
    pub self_ptr: u64,
    /// Logical CPU id, 0 = boot CPU.
    pub cpu_id: u32,
    /// Hardware id: APIC ID (x86_64), MPIDR affinity (AArch64), hartid
    /// (RISC-V).
    pub hw_id: u32,
    /// Deadline of this CPU's next tick, in the tick source's units. The
    /// compare registers (IA32_TSC_DEADLINE, CNTV_CVAL_EL0, stimecmp) are
    /// per CPU, so the bookkeeping is too.
    pub timer_next: u64,
    /// Timer interrupts this CPU has taken.
    pub local_ticks: u64,
    /// Inter-processor interrupts this CPU has taken.
    pub ipis: u64,
    /// x86_64: address of this CPU's TSS.RSP0 slot (`gs:[0x40]`), the stack
    /// the CPU switches to for an interrupt or exception from ring 3. Kept
    /// equal to `kernel_rsp` by [`set_entry_stack`].
    pub rsp0_slot: u64,
}

const _: () = {
    assert!(core::mem::offset_of!(ArchCpu, kernel_rsp) == 0x00);
    assert!(core::mem::offset_of!(ArchCpu, user_rsp) == 0x08);
    assert!(core::mem::offset_of!(ArchCpu, syscall_frame) == 0x10);
    assert!(core::mem::offset_of!(ArchCpu, self_ptr) == 0x18);
    assert!(core::mem::offset_of!(ArchCpu, cpu_id) == 0x20);
    assert!(core::mem::offset_of!(ArchCpu, hw_id) == 0x24);
    assert!(core::mem::offset_of!(ArchCpu, timer_next) == 0x28);
    assert!(core::mem::offset_of!(ArchCpu, local_ticks) == 0x30);
    assert!(core::mem::offset_of!(ArchCpu, ipis) == 0x38);
    assert!(core::mem::offset_of!(ArchCpu, rsp0_slot) == 0x40);
    assert!(core::mem::size_of::<ArchCpu>() == 128);
};

/// Interior-mutable slot of [`ARCH_CPUS`].
#[repr(transparent)]
pub struct ArchCpuCell(UnsafeCell<ArchCpu>);

// SAFETY: slot `i` is written by CPU `i` itself, or by the boot CPU before
// CPU `i` starts (and never concurrently with it). Entry assembly accesses
// it through the per-CPU register, outside Rust's aliasing model.
unsafe impl Sync for ArchCpuCell {}

impl ArchCpuCell {
    const fn new(cpu_id: u32) -> Self {
        Self(UnsafeCell::new(ArchCpu {
            kernel_rsp: 0,
            user_rsp: 0,
            syscall_frame: 0,
            self_ptr: 0,
            cpu_id,
            hw_id: 0,
            timer_next: 0,
            local_ticks: 0,
            ipis: 0,
            rsp0_slot: 0,
        }))
    }

    /// Raw pointer to the block.
    pub fn get(&self) -> *mut ArchCpu {
        self.0.get()
    }
}

/// Every CPU's block, indexed by logical id. In `.bss`/`.data` of the kernel
/// image, which on x86_64 is mapped into every process page table, so it is
/// reachable with any CR3 loaded.
pub static ARCH_CPUS: [ArchCpuCell; MAX_CPUS] = {
    let mut cpus = [const { ArchCpuCell::new(0) }; MAX_CPUS];
    let mut i = 0;
    while i < MAX_CPUS {
        cpus[i] = ArchCpuCell::new(i as u32);
        i += 1;
    }
    cpus
};

/// Pointer to logical CPU `cpu`'s block.
pub fn arch_cpu_ptr(cpu: usize) -> *mut ArchCpu {
    ARCH_CPUS[cpu].get()
}

/// Whether `addr` is the start of one of the [`ARCH_CPUS`] blocks.
pub fn is_arch_cpu_block(addr: u64) -> bool {
    let base = ARCH_CPUS.as_ptr() as u64;
    let size = core::mem::size_of::<ArchCpuCell>() as u64;
    addr >= base && addr < base + size * MAX_CPUS as u64 && (addr - base).is_multiple_of(size)
}

/// Fill in the calling CPU's block and point the per-CPU register at it.
/// Called once per CPU, early, before anything reads the per-CPU register.
///
/// # Safety
///
/// Must run on the CPU that is logical CPU `cpu`, before any other code on
/// this CPU uses its block, and not concurrently with another call for the
/// same `cpu`.
pub unsafe fn install(cpu: usize, hw_id: u32) {
    let p = arch_cpu_ptr(cpu);
    // SAFETY: per this function's contract, nothing else accesses slot
    // `cpu` now.
    unsafe {
        (*p).self_ptr = p as u64;
        (*p).cpu_id = cpu as u32;
        (*p).hw_id = hw_id;
    }
    // Ring 0 runs with the block in GS_BASE; KernelGsBase holds the user
    // value (0: user code cannot set it) until a ring-3 entry swaps them.
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        use x86_64::registers::model_specific::{GsBase, KernelGsBase};
        GsBase::write(x86_64::VirtAddr::new(p as u64));
        KernelGsBase::write(x86_64::VirtAddr::new(0));
    }
    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    // SAFETY: TPIDR_EL1 is the kernel's per-CPU register; the value is a
    // valid 'static block.
    unsafe {
        core::arch::asm!("msr tpidr_el1, {}", in(reg) p as u64, options(nomem, nostack));
    }
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    // SAFETY: the kernel does not use `tp` for TLS; it holds the per-CPU
    // block, a valid 'static address.
    unsafe {
        core::arch::asm!("mv tp, {}", in(reg) p as u64, options(nomem, nostack));
    }
}

/// Pointer to the calling CPU's block.
///
/// x86_64 keeps the block in `GS_BASE` whenever ring 0 runs. In the few
/// instructions between a `swapgs` and the `sysretq`/`iretq` that follows
/// (or before the `swapgs` of an entry) it is in `KernelGsBase`; an NMI or
/// machine check can arrive there, so whichever MSR holds a block address
/// is used. Neither MSR read causes a VM exit under KVM, unlike CPUID. User
/// code cannot point GS at kernel memory (no `ARCH_SET_GS`, no
/// CR4.FSGSBASE), so the range check cannot be fooled.
#[inline]
pub fn this_arch_cpu_ptr() -> *mut ArchCpu {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        use x86_64::registers::model_specific::{GsBase, KernelGsBase};
        let gs = GsBase::read().as_u64();
        if is_arch_cpu_block(gs) {
            gs as *mut ArchCpu
        } else {
            KernelGsBase::read().as_u64() as *mut ArchCpu
        }
    }
    #[cfg(all(target_arch = "aarch64", target_os = "none"))]
    {
        let p: u64;
        // SAFETY: reading TPIDR_EL1 at EL1 has no side effects.
        unsafe { core::arch::asm!("mrs {}, tpidr_el1", out(reg) p, options(nomem, nostack)) };
        p as *mut ArchCpu
    }
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    {
        // INVARIANT (N-14): `tp` is the user thread pointer in U-mode. Any
        // trap from U-mode must restore the kernel `tp` (from sscratch)
        // before reaching code that calls this. There is no U-mode entry on
        // riscv64 yet.
        let p: u64;
        // SAFETY: reading a general-purpose register has no side effects.
        unsafe { core::arch::asm!("mv {}, tp", out(reg) p, options(nomem, nostack)) };
        p as *mut ArchCpu
    }
    #[cfg(not(target_os = "none"))]
    {
        arch_cpu_ptr(0)
    }
}

/// The calling CPU's block, or the boot CPU's before `install` has run
/// (only the boot CPU runs then).
#[inline]
pub fn this_arch_cpu() -> *mut ArchCpu {
    let p = this_arch_cpu_ptr();
    if is_arch_cpu_block(p as u64) {
        p
    } else {
        arch_cpu_ptr(0)
    }
}

/// Count a timer interrupt on this CPU and advance its deadline by
/// `period` from the one previously armed (from `now` if none was),
/// skipping whole periods already in the past so missed ticks do not fire
/// as a burst. Returns the new deadline to program. Call only from this
/// CPU's timer interrupt.
pub fn advance_timer(now: u64, period: u64) -> u64 {
    let cpu = this_arch_cpu();
    // SAFETY: this CPU's own block, touched only by this CPU's timer
    // interrupt or by its timer start with interrupts off.
    unsafe {
        let base = if (*cpu).timer_next == 0 {
            now
        } else {
            (*cpu).timer_next
        };
        let mut next = base.wrapping_add(period);
        if next <= now {
            next = now + period;
        }
        (*cpu).timer_next = next;
        (*cpu).local_ticks = (*cpu).local_ticks.wrapping_add(1);
        next
    }
}

/// x86_64: make `top` the kernel stack this CPU enters on from ring 3, both
/// through `syscall` (`kernel_rsp`) and through an interrupt or exception
/// (TSS.RSP0). `top` must be 16-byte aligned.
#[cfg(target_arch = "x86_64")]
pub fn set_entry_stack(top: u64) {
    let cpu = this_arch_cpu();
    // SAFETY: this CPU's own block; `rsp0_slot` is 0 or the address of this
    // CPU's TSS.RSP0 field, which lives for the kernel's lifetime and is
    // read by the CPU only on a ring-3 entry, which cannot happen while
    // ring 0 runs here.
    unsafe {
        (*cpu).kernel_rsp = top;
        let slot = (*cpu).rsp0_slot;
        if slot != 0 {
            // The TSS is packed: RSP0 sits at offset 4, so the field is
            // only 4-byte aligned.
            core::ptr::write_unaligned(slot as *mut u64, top);
        }
    }
}

/// x86_64: the kernel stack this CPU enters on from ring 3.
#[cfg(target_arch = "x86_64")]
pub fn entry_stack() -> u64 {
    // SAFETY: this CPU's own block.
    unsafe { (*this_arch_cpu()).kernel_rsp }
}

/// x86_64: record where this CPU's TSS.RSP0 lives (GDT setup, once per CPU).
///
/// # Safety
///
/// `slot` must be the address of the RSP0 field of the TSS loaded on CPU
/// `cpu`, valid for the kernel's lifetime.
#[cfg(target_arch = "x86_64")]
pub unsafe fn set_rsp0_slot(cpu: usize, slot: u64) {
    // SAFETY: per the contract; the block is written only by its own CPU or
    // by the boot CPU before that CPU starts.
    unsafe { (*arch_cpu_ptr(cpu)).rsp0_slot = slot };
}

/// Count an inter-processor interrupt taken by this CPU (from its IPI
/// handler).
pub fn note_ipi() {
    let cpu = this_arch_cpu();
    // SAFETY: this CPU's own block, written only by this CPU's interrupt
    // handlers.
    unsafe { (*cpu).ipis = (*cpu).ipis.wrapping_add(1) };
}

/// Set this CPU's armed deadline (timer start).
pub fn set_timer_next(deadline: u64) {
    // SAFETY: as in `advance_timer`.
    unsafe { (*this_arch_cpu()).timer_next = deadline };
}

/// Whether the calling CPU keeps global time. Exactly one CPU advances the
/// uptime clock and the timer wheel; with every CPU ticking at the same
/// rate, each would otherwise add its own period and time would run N
/// times fast (Linux has one `tick_do_timer_cpu` for the same reason).
#[inline]
pub fn is_timekeeper() -> bool {
    this_cpu_id() == 0
}

/// Logical id of the calling CPU, read from its block.
#[inline]
pub fn this_cpu_id() -> u32 {
    let p = this_arch_cpu_ptr();
    if !is_arch_cpu_block(p as u64) {
        // Before `install` ran on this CPU (only the boot CPU runs then).
        return 0;
    }
    // SAFETY: `p` is one of the ARCH_CPUS blocks (checked); `cpu_id` is
    // written before the CPU runs and never changes.
    unsafe { (*p).cpu_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_recognised_and_numbered() {
        for i in 0..MAX_CPUS {
            let p = arch_cpu_ptr(i) as u64;
            assert!(is_arch_cpu_block(p));
            assert!(!is_arch_cpu_block(p + 8));
            // SAFETY: test-only read of a static block.
            assert_eq!(unsafe { (*arch_cpu_ptr(i)).cpu_id }, i as u32);
        }
        assert!(!is_arch_cpu_block(0));
        let end = arch_cpu_ptr(MAX_CPUS - 1) as u64 + core::mem::size_of::<ArchCpu>() as u64;
        assert!(!is_arch_cpu_block(end));
    }

    /// The TSS is `packed(4)`, so its RSP0 field is only 4-byte aligned;
    /// writing it as an aligned u64 panicked in the first nested child run.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn entry_stack_reaches_the_packed_tss_rsp0() {
        use alloc::boxed::Box;

        let tss = Box::into_raw(Box::new(x86_64::structures::tss::TaskStateSegment::new()));
        // SAFETY: test-only; `tss` is a live heap TSS, and only this test
        // uses CPU 0's RSP0 slot on the host.
        unsafe {
            let slot = core::ptr::addr_of_mut!((*tss).privilege_stack_table[0]) as u64;
            assert_eq!(slot - tss as u64, 4, "RSP0 follows a 4-byte reserved field");
            let saved = ((*arch_cpu_ptr(0)).rsp0_slot, (*arch_cpu_ptr(0)).kernel_rsp);
            set_rsp0_slot(0, slot);
            set_entry_stack(0xFFFF_E000_0001_0000);
            assert_eq!(entry_stack(), 0xFFFF_E000_0001_0000);
            let rsp0 = core::ptr::addr_of!((*tss).privilege_stack_table[0]).read_unaligned();
            assert_eq!(rsp0.as_u64(), 0xFFFF_E000_0001_0000);
            (*arch_cpu_ptr(0)).rsp0_slot = saved.0;
            (*arch_cpu_ptr(0)).kernel_rsp = saved.1;
            drop(Box::from_raw(tss));
        }
    }
}
