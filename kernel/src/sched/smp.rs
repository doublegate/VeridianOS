//! Symmetric multiprocessing (SMP) support

#![allow(
    clippy::fn_to_numeric_cast,
    clippy::needless_return,
    function_casts_as_integer
)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

/// CPU information
pub struct CpuInfo {
    /// CPU ID
    pub id: u8,
    /// CPU online status
    pub online: AtomicBool,
    /// CPU idle status
    pub idle: AtomicBool,
    /// Current task on this CPU
    pub current_task: AtomicU64,
    /// Load average (0-100)
    pub load: AtomicU8,
    /// Number of tasks in run queue
    pub nr_running: AtomicU32,
    /// CPU vendor string
    #[cfg(feature = "alloc")]
    pub vendor: String,
    /// CPU model string
    #[cfg(feature = "alloc")]
    pub model: String,
    /// CPU features
    pub features: CpuFeatures,
    /// Per-CPU page frame cache index (matches PER_CPU_PAGE_CACHES slot)
    pub page_cache_id: u8,
}

/// CPU features
#[derive(Debug, Default)]
pub struct CpuFeatures {
    /// Supports FPU
    pub fpu: bool,
    /// Supports SIMD
    pub simd: bool,
    /// Supports virtualization
    pub virtualization: bool,
    /// Supports hardware security features
    pub security: bool,
    /// Maximum physical address bits
    pub phys_addr_bits: u8,
    /// Maximum virtual address bits
    pub virt_addr_bits: u8,
}

impl CpuInfo {
    /// Create new CPU info
    pub fn new(id: u8) -> Self {
        Self {
            id,
            online: AtomicBool::new(false),
            idle: AtomicBool::new(true),
            current_task: AtomicU64::new(0),
            load: AtomicU8::new(0),
            nr_running: AtomicU32::new(0),
            #[cfg(feature = "alloc")]
            vendor: String::new(),
            #[cfg(feature = "alloc")]
            model: String::new(),
            features: CpuFeatures {
                fpu: false,
                simd: false,
                virtualization: false,
                security: false,
                phys_addr_bits: 0,
                virt_addr_bits: 0,
            },
            page_cache_id: id,
        }
    }

    /// Mark CPU as online
    pub fn bring_online(&self) {
        self.online.store(true, Ordering::Release);
        self.idle.store(true, Ordering::Release);
    }

    /// Mark CPU as offline
    pub fn bring_offline(&self) {
        self.online.store(false, Ordering::Release);
    }

    /// Check if CPU is online
    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Acquire)
    }

    /// Check if CPU is idle
    pub fn is_idle(&self) -> bool {
        self.idle.load(Ordering::Acquire)
    }

    /// Update load average
    pub fn update_load(&self) {
        let nr_running = self.nr_running.load(Ordering::Relaxed);
        let load = (nr_running * 100 / MAX_LOAD_FACTOR).min(100) as u8;
        self.load.store(load, Ordering::Relaxed);
    }
}

/// CPU topology information
#[derive(Debug)]
pub struct CpuTopology {
    /// Total number of CPUs
    pub total_cpus: u8,
    /// Number of online CPUs
    pub online_cpus: AtomicU8,
    /// Number of CPU sockets
    pub sockets: u8,
    /// Number of cores per socket
    pub cores_per_socket: u8,
    /// Number of threads per core
    pub threads_per_core: u8,
    /// NUMA nodes
    #[cfg(feature = "alloc")]
    pub numa_nodes: Vec<NumaNode>,
}

/// NUMA node information
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct NumaNode {
    /// Node ID
    pub id: u8,
    /// CPUs in this node
    pub cpus: Vec<u8>,
    /// Memory ranges
    pub memory_ranges: Vec<(usize, usize)>,
    /// Distance to other nodes
    pub distances: Vec<u8>,
}

impl CpuTopology {
    /// Create new topology
    pub fn new() -> Self {
        Self {
            total_cpus: 1,
            online_cpus: AtomicU8::new(1),
            sockets: 1,
            cores_per_socket: 1,
            threads_per_core: 1,
            #[cfg(feature = "alloc")]
            numa_nodes: Vec::new(),
        }
    }

    /// Detect CPU topology
    pub fn detect(&mut self) {
        #[cfg(target_arch = "x86_64")]
        self.detect_x86_64();

        #[cfg(target_arch = "aarch64")]
        self.detect_aarch64();

        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        self.detect_riscv();
    }

    #[cfg(target_arch = "x86_64")]
    fn detect_x86_64(&mut self) {
        use core::arch::x86_64::__cpuid;

        // SAFETY: CPUID is an unprivileged instruction that queries CPU
        // feature information. Leaf 0x1 returns basic processor info and
        // leaf 0xB returns extended topology. Both are read-only operations
        // with no side effects. max_cpuid() verifies leaf 0xB is supported
        // before accessing it.
        unsafe {
            // Get basic CPU info
            let cpuid = __cpuid(0x1);
            let logical_cpus = ((cpuid.ebx >> 16) & 0xFF) as u8;

            // Get extended topology
            if max_cpuid() >= 0xB {
                // Intel topology enumeration
                let cpuid = __cpuid(0xB);
                self.threads_per_core = (cpuid.ebx & 0xFFFF) as u8;

                let cpuid = __cpuid(0xB);
                self.cores_per_socket = ((cpuid.ebx & 0xFFFF) / self.threads_per_core as u32) as u8;

                self.total_cpus = logical_cpus;
                self.sockets = self.total_cpus / (self.cores_per_socket * self.threads_per_core);
            } else {
                // Fallback
                self.total_cpus = logical_cpus;
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    fn detect_aarch64(&mut self) {
        // SAFETY: MPIDR_EL1 is a read-only system register accessible from
        // EL1 (kernel mode) that provides the CPU's affinity information
        // (thread, core, cluster, socket). Reading it has no side effects.
        unsafe {
            let mpidr: u64;
            core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) mpidr);

            // Extract affinity levels
            let _aff0 = (mpidr & 0xFF) as u8; // Thread
            let _aff1 = ((mpidr >> 8) & 0xFF) as u8; // Core
            let _aff2 = ((mpidr >> 16) & 0xFF) as u8; // Cluster
            let _aff3 = ((mpidr >> 32) & 0xFF) as u8; // Socket

            // This is simplified - real detection would probe all CPUs
            self.threads_per_core = 1; // SMT not common on ARM
            self.cores_per_socket = 4; // Common configuration
            self.sockets = 1;
            self.total_cpus = self.sockets * self.cores_per_socket * self.threads_per_core;
        }
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    fn detect_riscv(&mut self) {
        // RISC-V detection through device tree or SBI
        // For now, assume single core
        self.total_cpus = 1;
        self.threads_per_core = 1;
        self.cores_per_socket = 1;
        self.sockets = 1;
    }
}

/// Per-CPU data
#[repr(C)]
pub struct PerCpuData {
    /// CPU information
    pub cpu_info: CpuInfo,
    /// Current privilege level
    pub privilege_level: u8,
    /// Interrupt nesting level
    pub irq_depth: u32,
    /// Preemption count
    pub preempt_count: u32,
    /// Kernel stack pointer
    pub kernel_stack: usize,
    /// Thread-local storage
    pub tls: usize,
}

impl Default for CpuTopology {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum number of CPUs
/// Reduced from 256 to 16 for bootloader 0.11 compatibility (reduces static
/// data size)
pub const MAX_CPUS: usize = 16;

/// Maximum load factor for load calculation
const MAX_LOAD_FACTOR: u32 = 10;

/// Per-CPU data array
///
/// SAFETY JUSTIFICATION: This static mut is intentionally kept because:
/// 1. Each CPU slot is written exactly once during init_cpu() (single-writer
///    per index)
/// 2. After initialization, slots are only read (immutable access)
/// 3. Each CPU accesses its own slot via cpu_id (no cross-CPU aliasing)
/// 4. Using a Mutex here would cause deadlocks in scheduler hot paths
/// 5. This is a pre-heap, per-CPU data structure that cannot use OnceLock
#[allow(static_mut_refs)]
static mut PER_CPU_DATA: [Option<PerCpuData>; MAX_CPUS] = [const { None }; MAX_CPUS];

/// CPU topology
/// Set once any secondary CPU is running; until then the current CPU is
/// always the boot CPU (0).
static SECONDARY_CPUS_ONLINE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Record that a secondary CPU has started. AP bring-up must call this
/// before the AP runs any code that uses per-CPU state.
pub fn mark_secondary_cpu_online() {
    SECONDARY_CPUS_ONLINE.store(true, core::sync::atomic::Ordering::Release);
}

/// Initialize SMP support
pub fn init() {
    // With the `smp` feature, start the other CPUs (stage S1: they park in
    // an idle loop and run no tasks yet). Without it, the boot CPU runs
    // alone, as on a single-CPU machine.
    #[cfg(feature = "smp")]
    crate::arch::smp_boot::bring_up_secondaries();
    #[cfg(not(feature = "smp"))]
    kprintln!("[SMP] SMP initialized (BSP only)");
}

/// Get per-CPU data for current CPU
pub fn this_cpu() -> &'static PerCpuData {
    let cpu_id = current_cpu_id();
    // SAFETY: PER_CPU_DATA is initialized during init_cpu() for each CPU
    // before any code calls this_cpu(). Each CPU reads only its own slot.
    // After init, the slot is never modified again.
    unsafe {
        PER_CPU_DATA[cpu_id as usize]
            .as_ref()
            .expect("Per-CPU data not initialized")
    }
}

/// Get per-CPU data for specific CPU
pub fn per_cpu(cpu_id: u8) -> Option<&'static PerCpuData> {
    // SAFETY: PER_CPU_DATA slots are written once during init_cpu() and
    // only read thereafter. The returned reference is valid for 'static
    // because the array lives for the kernel's lifetime.
    unsafe { PER_CPU_DATA[cpu_id as usize].as_ref() }
}

/// Get current CPU ID
pub fn current_cpu_id() -> u8 {
    // Only the boot CPU runs until secondary CPUs are brought up. Asking
    // the hardware is not free -- on x86_64 it is CPUID, a VM exit under
    // KVM costing microseconds, on every per-CPU fast path.
    if !SECONDARY_CPUS_ONLINE.load(core::sync::atomic::Ordering::Acquire) {
        return 0;
    }
    // Each CPU's logical id is in its architecture per-CPU block, reached
    // through a register (x86 GS-base MSRs, TPIDR_EL1, tp); see
    // arch::percpu. Hardware ids (APIC ID, MPIDR, hartid) are not logical
    // ids and are never used as indices.
    crate::arch::percpu::this_cpu_id() as u8
}

#[cfg(target_arch = "x86_64")]
fn max_cpuid() -> u32 {
    // SAFETY: CPUID leaf 0 is always valid on x86_64 processors. It returns
    // the maximum supported standard CPUID leaf number in EAX. This is a
    // read-only instruction with no side effects.
    unsafe {
        use core::arch::x86_64::__cpuid;
        let cpuid = __cpuid(0);
        cpuid.eax
    }
}
