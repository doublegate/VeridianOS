//! Secondary CPU bring-up (stage S1 of ADR 0004).
//!
//! The boot CPU enumerates the other CPUs from firmware (ACPI MADT on
//! x86_64, the device tree elsewhere), numbers them 1, 2, ... in table
//! order, and starts them one at a time through a single shared
//! [`ApBootArgs`] block, as Linux does. Each secondary installs its
//! per-CPU block, sets up its own exception vectors, local interrupt
//! controller and timer, reports ONLINE and parks in an idle loop. In S1 a
//! secondary takes no scheduler lock and never runs tasks.
//!
//! Failure degrades, it never fails the boot: a CPU that does not report
//! ONLINE within the timeout is left out, and no further CPUs are started
//! (it might still be reading the shared boot block).
//!
//! Built only with the `smp` feature; without it nothing here runs and a
//! multi-CPU machine boots exactly as a single-CPU one.

#[cfg(feature = "smp")]
use core::sync::atomic::fence;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(feature = "smp")]
use super::percpu;
#[cfg(any(feature = "smp", test))]
use super::percpu::MAX_CPUS;

/// The secondary has not run yet.
pub const AP_STATUS_NONE: u32 = 0;
/// The secondary reached its first Rust code.
pub const AP_STATUS_ALIVE: u32 = 1;
/// The secondary finished its own initialisation and is idling.
pub const AP_STATUS_ONLINE: u32 = 2;

/// What a starting secondary needs, filled by the boot CPU before each
/// start. Entry assembly reads it by offset (asserted below).
#[repr(C)]
pub struct ApBootArgs {
    /// Logical CPU id.
    pub cpu_id: AtomicU64,
    /// Top of the secondary's kernel stack (16-byte aligned).
    pub stack_top: AtomicU64,
    /// Address of the secondary's `ArchCpu` block.
    pub percpu: AtomicU64,
    /// x86_64: physical address of the kernel page table root.
    pub kernel_cr3: AtomicU64,
    /// x86_64: the boot CPU's CR4.
    pub cr4: AtomicU64,
    /// One of the `AP_STATUS_*` values, written by the secondary.
    pub status: AtomicU32,
    /// Last bring-up stage the secondary reached (for diagnosing a timeout).
    pub stage: AtomicU32,
}

const _: () = {
    assert!(core::mem::offset_of!(ApBootArgs, cpu_id) == 0x00);
    assert!(core::mem::offset_of!(ApBootArgs, stack_top) == 0x08);
    assert!(core::mem::offset_of!(ApBootArgs, percpu) == 0x10);
    assert!(core::mem::offset_of!(ApBootArgs, kernel_cr3) == 0x18);
    assert!(core::mem::offset_of!(ApBootArgs, cr4) == 0x20);
    assert!(core::mem::offset_of!(ApBootArgs, status) == 0x28);
    assert!(core::mem::offset_of!(ApBootArgs, stage) == 0x2c);
};

/// The one boot block, reused for each secondary in turn.
pub static AP_BOOT_ARGS: ApBootArgs = ApBootArgs {
    cpu_id: AtomicU64::new(0),
    stack_top: AtomicU64::new(0),
    percpu: AtomicU64::new(0),
    kernel_cr3: AtomicU64::new(0),
    cr4: AtomicU64::new(0),
    status: AtomicU32::new(AP_STATUS_NONE),
    stage: AtomicU32::new(0),
};

/// CPUs running, the boot CPU included.
static ONLINE_CPUS: AtomicU32 = AtomicU32::new(1);

/// How many CPUs are running.
pub fn online_cpus() -> u32 {
    ONLINE_CPUS.load(Ordering::Acquire)
}

/// Kernel stack of each secondary.
#[cfg(feature = "smp")]
const AP_STACK_SIZE: usize = 64 * 1024;

/// How long to wait for a secondary to report ONLINE. Generous because
/// AArch64 and RISC-V run under TCG in QEMU.
#[cfg(feature = "smp")]
const AP_TIMEOUT_NS: u64 = 1_000_000_000;

/// Record a bring-up stage from the secondary (for diagnostics).
pub fn ap_stage(stage: u32) {
    AP_BOOT_ARGS.stage.store(stage, Ordering::Release);
}

/// Logical ids for `count` secondaries: 1..=count, capped so the total
/// never exceeds `MAX_CPUS`.
#[cfg(any(feature = "smp", test))]
fn logical_ids(count: usize) -> core::ops::RangeInclusive<usize> {
    1..=count.min(MAX_CPUS - 1)
}

/// Start every secondary CPU the firmware describes. Runs on the boot CPU
/// once, after the heap and the boot CPU's own interrupt and timer setup.
#[cfg(feature = "smp")]
pub fn bring_up_secondaries() {
    use alloc::vec::Vec;

    let hw_ids: Vec<u32> = super::smp_arch::enumerate_secondaries();
    let total = hw_ids.len().min(MAX_CPUS - 1) + 1;
    crate::println!("[SMP] {} CPUs described by firmware", hw_ids.len() + 1);
    if hw_ids.is_empty() {
        return;
    }
    // From here on, current_cpu_id() asks the per-CPU register instead of
    // assuming the boot CPU. The boot CPU's register is already set.
    crate::sched::smp::mark_secondary_cpu_online();

    for (cpu, &hw) in logical_ids(hw_ids.len()).zip(hw_ids.iter()) {
        let stack = alloc::vec![0u8; AP_STACK_SIZE].leak();
        let stack_top = (stack.as_ptr() as u64 + AP_STACK_SIZE as u64) & !0xF;
        let block = percpu::arch_cpu_ptr(cpu);
        // SAFETY: CPU `cpu` has not started, so the boot CPU is the only
        // writer of its block.
        unsafe { (*block).hw_id = hw };

        let args = &AP_BOOT_ARGS;
        args.cpu_id.store(cpu as u64, Ordering::Relaxed);
        args.stack_top.store(stack_top, Ordering::Relaxed);
        args.percpu.store(block as u64, Ordering::Relaxed);
        args.status.store(AP_STATUS_NONE, Ordering::Relaxed);
        args.stage.store(0, Ordering::Relaxed);
        super::smp_arch::prepare_boot_args(args);
        // Everything above is visible before the secondary can run.
        fence(Ordering::SeqCst);

        if let Err(e) = super::smp_arch::start_ap(hw, args) {
            crate::println!("[SMP] CPU {} (hw id {:#x}): start failed: {}", cpu, hw, e);
            continue;
        }
        let start = super::timer::monotonic_ns();
        while args.status.load(Ordering::Acquire) != AP_STATUS_ONLINE {
            if super::timer::monotonic_ns().saturating_sub(start) > AP_TIMEOUT_NS {
                break;
            }
            core::hint::spin_loop();
        }
        if args.status.load(Ordering::Acquire) == AP_STATUS_ONLINE {
            ONLINE_CPUS.fetch_add(1, Ordering::AcqRel);
            crate::println!("[SMP] CPU {} online (hw id {:#x})", cpu, hw);
        } else {
            crate::println!(
                "[SMP] CPU {} (hw id {:#x}) did not come online (status {}, stage {}); not \
                 starting more CPUs",
                cpu,
                hw,
                args.status.load(Ordering::Acquire),
                args.stage.load(Ordering::Acquire)
            );
            break;
        }
    }
    crate::println!("[SMP] {}/{} CPUs online", online_cpus(), total);
}

/// Common secondary entry, called by the architecture's entry code once the
/// stack and the per-CPU register are set. Never returns.
#[cfg(feature = "smp")]
pub fn ap_main() -> ! {
    AP_BOOT_ARGS
        .status
        .store(AP_STATUS_ALIVE, Ordering::Release);
    super::smp_arch::ap_init();
    AP_BOOT_ARGS
        .status
        .store(AP_STATUS_ONLINE, Ordering::Release);
    super::smp_arch::idle()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_ids_are_dense_and_capped() {
        assert_eq!(logical_ids(0).count(), 0);
        assert_eq!(logical_ids(3).collect::<alloc::vec::Vec<_>>(), [1, 2, 3]);
        assert_eq!(logical_ids(100).count(), MAX_CPUS - 1);
        assert_eq!(*logical_ids(100).end(), MAX_CPUS - 1);
    }
}
