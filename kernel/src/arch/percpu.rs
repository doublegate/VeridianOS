//! Architecture per-CPU block.
//!
//! One `ArchCpu` per logical CPU, in a static array that exists from the
//! first instruction (no heap). Each CPU finds its own block through a
//! register: `KernelGsBase`/`GS_BASE` on x86_64, `TPIDR_EL1` on AArch64, `tp`
//! on RISC-V. Entry assembly addresses fields by fixed offset, so the layout
//! is ABI (asserted below).
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
    assert!(core::mem::size_of::<ArchCpu>() == 64);
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
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    x86_64::registers::model_specific::KernelGsBase::write(x86_64::VirtAddr::new(p as u64));
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
/// x86_64 keeps the block in `KernelGsBase` outside a syscall and in
/// `GS_BASE` inside one (`swapgs`), so whichever of the two holds a block
/// address is used. Neither MSR read causes a VM exit under KVM, unlike
/// CPUID. User code cannot point GS at kernel memory (no `ARCH_SET_GS`, no
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
        let end = arch_cpu_ptr(MAX_CPUS - 1) as u64 + 64;
        assert!(!is_arch_cpu_block(end));
    }
}
