//! Hardware Performance Monitoring Unit (PMU) Driver
//!
//! Provides access to hardware performance counters for profiling and
//! optimization. Supports x86_64 (IA32_PERFEVTSELx / IA32_PMCx MSRs),
//! AArch64 (PMCR_EL0, PMCNTENSET_EL0), and RISC-V (mcycle, minstret).
//!
//! Performance events that can be counted:
//! - Instructions retired
//! - CPU cycles
//! - Cache misses (L1, L2, LLC)
//! - Branch mispredictions
//! - TLB misses

use core::sync::atomic::{AtomicBool, Ordering};

/// Whether the PMU has been initialized.
static PMU_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Number of available general-purpose performance counters.
static mut NUM_COUNTERS: u8 = 0;

// ---------------------------------------------------------------------------
// Performance Event Selectors
// ---------------------------------------------------------------------------

/// Performance events that can be monitored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmuEvent {
    /// CPU cycles (unhalted core cycles).
    Cycles,
    /// Instructions retired.
    InstructionsRetired,
    /// L1 data cache misses.
    L1DCacheMisses,
    /// L2 cache misses (unified).
    L2CacheMisses,
    /// Last-level cache misses.
    LlcMisses,
    /// Branch mispredictions.
    BranchMispredicts,
    /// Instruction TLB misses.
    ITlbMisses,
    /// Data TLB misses.
    DTlbMisses,
}

impl PmuEvent {
    /// Convert to x86_64 architectural performance event selector.
    ///
    /// Returns (event_select, unit_mask) for IA32_PERFEVTSELx programming.
    /// These are Intel Architectural Performance Events (CPUID leaf 0x0A).
    #[cfg(target_arch = "x86_64")]
    fn to_x86_evtsel(self) -> (u8, u8) {
        match self {
            Self::Cycles => (0x3C, 0x00),              // UnHalted Core Cycles
            Self::InstructionsRetired => (0xC0, 0x00), // Instructions Retired
            Self::L1DCacheMisses => (0xCB, 0x01),      // MEM_LOAD_RETIRED.L1_MISS
            Self::L2CacheMisses => (0xCB, 0x04),       // MEM_LOAD_RETIRED.L2_MISS
            Self::LlcMisses => (0x2E, 0x41),           // LONGEST_LAT_CACHE.MISS
            Self::BranchMispredicts => (0xC5, 0x00),   // BR_MISP_RETIRED.ALL_BRANCHES
            Self::ITlbMisses => (0x85, 0x01),          // ITLB_MISSES.MISS_CAUSES_A_WALK
            Self::DTlbMisses => (0x08, 0x01),          // DTLB_LOAD_MISSES.MISS_CAUSES_A_WALK
        }
    }
}

/// A PMU counter configuration.
#[derive(Debug, Clone)]
pub struct PmuCounter {
    /// Counter index (0..NUM_COUNTERS-1).
    pub index: u8,
    /// The event being counted.
    pub event: PmuEvent,
    /// Whether this counter is currently active.
    pub active: bool,
}

/// PMU counter sample (snapshot of a single counter).
#[derive(Debug, Clone, Copy)]
pub struct PmuSample {
    /// The event type.
    pub event: PmuEvent,
    /// Counter value.
    pub count: u64,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the PMU subsystem.
///
/// Detects the number of available performance counters and their
/// capabilities via CPUID (x86_64) or system register reads (ARM/RISC-V).
pub fn init() {
    if PMU_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    #[cfg(target_arch = "x86_64")]
    {
        init_x86_64();
    }

    #[cfg(target_arch = "aarch64")]
    {
        init_aarch64();
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        init_riscv();
    }

    PMU_INITIALIZED.store(true, Ordering::Release);
}

/// Check if PMU is initialized.
pub fn is_initialized() -> bool {
    PMU_INITIALIZED.load(Ordering::Acquire)
}

/// Get the number of general-purpose performance counters.
pub fn num_counters() -> u8 {
    // SAFETY: NUM_COUNTERS is written once during init() before any
    // concurrent reads. After init, it is read-only.
    unsafe { NUM_COUNTERS }
}

// ---------------------------------------------------------------------------
// x86_64 PMU
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
fn init_x86_64() {
    // CPUID leaf 0x0A: Architectural Performance Monitoring.
    // SAFETY: CPUID is a read-only instruction with no side effects.
    let cpuid = unsafe { core::arch::x86_64::__cpuid(0x0A) };

    let version_id = cpuid.eax & 0xFF;
    let num_gp_counters = ((cpuid.eax >> 8) & 0xFF) as u8;
    let counter_width = ((cpuid.eax >> 16) & 0xFF) as u8;

    // SAFETY: Written once during init, read-only afterwards.
    unsafe {
        NUM_COUNTERS = num_gp_counters;
    }

    println!(
        "[PMU] x86_64: version={}, counters={}, width={} bits",
        version_id, num_gp_counters, counter_width
    );
}

/// Configure a performance counter on x86_64.
///
/// Programs IA32_PERFEVTSELx with the event selector, unit mask,
/// and enable bits. The counter starts counting immediately.
#[cfg(target_arch = "x86_64")]
pub fn configure_counter(counter: u8, event: PmuEvent) -> bool {
    let num = num_counters();
    if counter >= num {
        return false;
    }

    let (evt_sel, umask) = event.to_x86_evtsel();

    // IA32_PERFEVTSELx MSR: base 0x186 + counter index.
    // Bits: [7:0] = EventSelect, [15:8] = UMask, [16] = USR, [17] = OS,
    //       [22] = EN (enable).
    let evtsel_msr = 0x186 + counter as u32;
    let value: u64 = (evt_sel as u64)
        | ((umask as u64) << 8)
        | (1 << 16)  // Count in user mode
        | (1 << 17)  // Count in kernel mode
        | (1 << 22); // Enable counter

    // Clear the counter first.
    let pmc_msr = 0xC1 + counter as u32; // IA32_PMCx
    crate::arch::x86_64::msr::wrmsr(pmc_msr, 0);

    // Program the event selector.
    crate::arch::x86_64::msr::wrmsr(evtsel_msr, value);

    true
}

/// Read a performance counter value on x86_64.
#[cfg(target_arch = "x86_64")]
pub fn read_counter(counter: u8) -> u64 {
    if counter >= num_counters() {
        return 0;
    }
    let pmc_msr = 0xC1 + counter as u32;
    crate::arch::x86_64::msr::rdmsr(pmc_msr)
}

/// Stop (disable) a performance counter on x86_64.
#[cfg(target_arch = "x86_64")]
pub fn stop_counter(counter: u8) {
    if counter >= num_counters() {
        return;
    }
    let evtsel_msr = 0x186 + counter as u32;
    crate::arch::x86_64::msr::wrmsr(evtsel_msr, 0);
}

// ---------------------------------------------------------------------------
// AArch64 PMU
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
fn init_aarch64() {
    // Read PMCR_EL0 to get the number of event counters.
    let pmcr: u64;
    // SAFETY: PMCR_EL0 is a read-only system register accessible from EL1.
    unsafe {
        core::arch::asm!("mrs {}, PMCR_EL0", out(reg) pmcr);
    }
    let n = ((pmcr >> 11) & 0x1F) as u8;
    unsafe {
        NUM_COUNTERS = n;
    }
    println!("[PMU] AArch64: {} event counters", n);
}

/// Read the cycle counter on AArch64 (PMCCNTR_EL0).
#[cfg(target_arch = "aarch64")]
pub fn read_cycle_counter() -> u64 {
    let val: u64;
    // SAFETY: Reading PMCCNTR_EL0, a read-only performance counter register.
    unsafe {
        core::arch::asm!("mrs {}, PMCCNTR_EL0", out(reg) val);
    }
    val
}

// ---------------------------------------------------------------------------
// RISC-V PMU
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
fn init_riscv() {
    // RISC-V has fixed counters: mcycle, minstret, plus optional HPM counters.
    unsafe {
        NUM_COUNTERS = 2; // cycle + instret at minimum
    }
    println!("[PMU] RISC-V: cycle + instret counters");
}

/// Read the cycle counter on RISC-V (mcycle CSR).
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn read_cycle_counter() -> u64 {
    let val: u64;
    // SAFETY: Reading mcycle CSR is a read-only operation.
    unsafe {
        core::arch::asm!("csrr {}, mcycle", out(reg) val);
    }
    val
}

/// Read the instruction counter on RISC-V (minstret CSR).
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn read_instret_counter() -> u64 {
    let val: u64;
    // SAFETY: Reading minstret CSR is a read-only operation.
    unsafe {
        core::arch::asm!("csrr {}, minstret", out(reg) val);
    }
    val
}

// ---------------------------------------------------------------------------
// Sampling Profiler
// ---------------------------------------------------------------------------

/// Maximum number of instruction pointer samples per buffer.
pub const MAX_SAMPLES: usize = 4096;

/// A sample captured by the sampling profiler.
#[derive(Debug, Clone, Copy)]
pub struct ProfileSample {
    /// Instruction pointer at sample time.
    pub ip: u64,
    /// CPU ID where the sample was taken.
    pub cpu: u8,
    /// Process ID (0 for kernel).
    pub pid: u64,
}

/// Per-CPU sample buffer.
pub struct SampleBuffer {
    /// Sample storage.
    pub samples: [ProfileSample; MAX_SAMPLES],
    /// Number of samples collected.
    pub count: usize,
    /// Whether sampling is active.
    pub active: bool,
}

impl SampleBuffer {
    /// Create a new empty sample buffer.
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            samples: [ProfileSample {
                ip: 0,
                cpu: 0,
                pid: 0,
            }; MAX_SAMPLES],
            count: 0,
            active: false,
        }
    }

    /// Record a sample. Returns false if buffer is full.
    pub fn record(&mut self, ip: u64, cpu: u8, pid: u64) -> bool {
        if self.count >= MAX_SAMPLES {
            return false;
        }
        self.samples[self.count] = ProfileSample { ip, cpu, pid };
        self.count += 1;
        true
    }

    /// Clear the sample buffer.
    pub fn clear(&mut self) {
        self.count = 0;
    }
}
