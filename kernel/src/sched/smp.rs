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

use spin::Mutex;

use super::{queue::ReadyQueue, scheduler::Scheduler, task::Task};
use crate::error::KernelError;

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
    /// Per-CPU scheduler
    pub scheduler: Mutex<Scheduler>,
    /// Per-CPU ready queue
    pub ready_queue: Mutex<ReadyQueue>,
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
    pub const fn new(id: u8) -> Self {
        Self {
            id,
            online: AtomicBool::new(false),
            idle: AtomicBool::new(true),
            current_task: AtomicU64::new(0),
            load: AtomicU8::new(0),
            nr_running: AtomicU32::new(0),
            scheduler: Mutex::new(Scheduler::new()),
            ready_queue: Mutex::new(ReadyQueue::new()),
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
static CPU_TOPOLOGY: Mutex<CpuTopology> = Mutex::new(CpuTopology {
    total_cpus: 1,
    online_cpus: AtomicU8::new(1),
    sockets: 1,
    cores_per_socket: 1,
    threads_per_core: 1,
    #[cfg(feature = "alloc")]
    numa_nodes: Vec::new(),
});

/// Initialize SMP support
pub fn init() {
    kprintln!("[SMP] Initializing SMP support (BSP only)...");

    // All architectures currently use simplified BSP-only initialization.
    // Complex topology detection and AP wakeup deferred to Phase 3+.

    kprintln!("[SMP] SMP initialized (BSP only)");
}

/// Wake up all Application Processors
fn wake_up_aps() {
    let topology = CPU_TOPOLOGY.lock();
    let num_cpus = topology.total_cpus;

    if num_cpus <= 1 {
        println!("[SMP] Single CPU system, no APs to wake");
        return;
    }

    println!("[SMP] Waking up {} Application Processors", num_cpus - 1);

    // Wake up each AP
    for cpu_id in 1..num_cpus {
        if let Err(_e) = cpu_up(cpu_id) {
            println!("[SMP] Failed to wake CPU {}: {}", cpu_id, _e);
        } else {
            println!("[SMP] Successfully woke CPU {}", cpu_id);
        }
    }
}

/// Initialize specific CPU
pub fn init_cpu(cpu_id: u8) {
    // SAFETY: Each CPU slot is written exactly once during initialization.
    // cpu_id is bounds-checked by callers (cpu_up checks cpu_id < MAX_CPUS).
    // No concurrent write to the same index occurs.
    unsafe {
        let cpu_info = CpuInfo::new(cpu_id);

        // Initialize per-CPU scheduler with CPU ID
        {
            let mut scheduler = cpu_info.scheduler.lock();
            scheduler.cpu_id = cpu_id;
        }

        let cpu_data = PerCpuData {
            cpu_info,
            privilege_level: 0,
            irq_depth: 0,
            preempt_count: 0,
            kernel_stack: 0,
            tls: 0,
        };

        PER_CPU_DATA[cpu_id as usize] = Some(cpu_data);

        if let Some(ref mut data) = PER_CPU_DATA[cpu_id as usize] {
            data.cpu_info.bring_online();

            // Initialize idle task for this CPU if not BSP
            #[cfg(feature = "alloc")]
            if cpu_id != 0 {
                // Create per-CPU idle task
                create_cpu_idle_task(cpu_id);
            }
        }
    }
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
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: CPUID leaf 0x1 is an unprivileged read-only instruction.
        // The initial APIC ID is in bits 31:24 of EBX. This is safe to call
        // at any time on x86_64.
        unsafe {
            use core::arch::x86_64::__cpuid;
            let cpuid = __cpuid(0x1);
            ((cpuid.ebx >> 24) & 0xFF) as u8
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: MPIDR_EL1 is a read-only system register accessible from
        // EL1 (kernel mode). Bits [7:0] (Aff0) contain the CPU thread ID
        // within the core. Reading has no side effects.
        unsafe {
            let mpidr: u64;
            core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) mpidr);
            (mpidr & 0xFF) as u8
        }
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // SAFETY: mhartid is a read-only CSR that returns the hardware
        // thread (hart) ID. It is always readable from M-mode. This may
        // trap in S-mode if not delegated, but during bootstrap we run in
        // M-mode or the SBI provides this value.
        unsafe {
            let hartid: usize;
            core::arch::asm!("csrr {}, mhartid", out(reg) hartid);
            hartid as u8
        }
    }
}

/// Send inter-processor interrupt
pub fn send_ipi(target_cpu: u8, vector: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        // Send IPI via the Local APIC Interrupt Command Register.
        if let Err(e) = crate::arch::x86_64::apic::send_ipi(target_cpu, vector) {
            println!(
                "[SMP] IPI to CPU {} vector {:#x} failed: {}",
                target_cpu, vector, e
            );
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: GICD_SGIR is a memory-mapped I/O register at a fixed address
        // on the QEMU virt machine GIC (Generic Interrupt Controller). Writing
        // to it triggers a Software Generated Interrupt (SGI) on the target
        // CPU(s). The address is always mapped and the write is a volatile
        // MMIO operation that does not alias any Rust memory.
        unsafe {
            // GIC distributor base (QEMU virt machine)
            const GICD_BASE: usize = 0x0800_0000;
            const GICD_SGIR: usize = GICD_BASE + 0xF00;

            // SGI target list (bit per CPU)
            let target_list = 1u32 << target_cpu;
            // SGI ID (0-15 are software generated)
            let sgi_id = (vector & 0xF) as u32;

            // Write to GICD_SGIR to trigger SGI
            let sgir_value = (target_list << 16) | sgi_id;
            core::ptr::write_volatile(GICD_SGIR as *mut u32, sgir_value);
        }
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Use SBI IPI extension

        // Create hart mask for target CPU
        let hart_mask = 1u64 << target_cpu;
        let hart_mask_base = 0;

        // SBI call to send IPI
        // Function ID 0x735049 ('sPI' in ASCII) for sbi_send_ipi
        // SAFETY: This performs an SBI ecall to send an IPI to the target
        // hart. The calling convention uses a0 (hart_mask), a1 (base),
        // a7 (extension ID), a6 (function ID). The ecall is a supervisor-
        // level trap to the SBI firmware which handles IPI delivery. The
        // clobbered registers (a0, a1) are marked as lateout.
        unsafe {
            core::arch::asm!(
                "ecall",
                in("a0") hart_mask,
                in("a1") hart_mask_base,
                in("a7") 0x735049,
                in("a6") 0,
                lateout("a0") _,
                lateout("a1") _,
            )
        };

        // Note: vector parameter is not used in RISC-V as IPIs are fixed
        let _ = vector;
    }

    #[allow(unused_variables)]
    let _ = (target_cpu, vector); // Suppress warnings on some architectures
}

/// CPU hotplug: bring CPU online
pub fn cpu_up(cpu_id: u8) -> Result<(), KernelError> {
    if cpu_id >= MAX_CPUS as u8 {
        return Err(KernelError::InvalidArgument {
            name: "cpu_id",
            value: "exceeds MAX_CPUS",
        });
    }

    if let Some(cpu_data) = per_cpu(cpu_id) {
        if cpu_data.cpu_info.is_online() {
            return Err(KernelError::AlreadyExists {
                resource: "online CPU",
                id: cpu_id as u64,
            });
        }
    } else {
        init_cpu(cpu_id);
    }

    // Send INIT-SIPI-SIPI sequence to wake up AP (Application Processor).
    // Per Intel SDM: INIT -> 10ms delay -> SIPI -> 200us delay -> SIPI (if needed).
    #[cfg(target_arch = "x86_64")]
    {
        // Send INIT IPI via the APIC ICR with INIT delivery mode.
        if let Err(e) = crate::arch::x86_64::apic::send_init_ipi(cpu_id) {
            println!("[SMP] INIT IPI to CPU {} failed: {}", cpu_id, e);
            return Err(KernelError::HardwareError {
                device: "APIC",
                code: cpu_id as u32,
            });
        }

        // 10ms delay for INIT to be processed (spin-wait).
        for _ in 0..10_000_000 {
            core::hint::spin_loop();
        }

        // Send first Startup IPI (SIPI) with trampoline page vector.
        // Startup page 0x08 = physical address 0x8000 where AP trampoline
        // code would reside (not yet implemented -- requires 16-bit real mode code).
        let sipi_page = 0x08u8;
        let _ = crate::arch::x86_64::apic::send_startup_ipi(cpu_id, sipi_page);

        // 200us delay.
        for _ in 0..200_000 {
            core::hint::spin_loop();
        }

        // Send second SIPI if AP has not come online yet (per Intel SDM
        // recommendation).
        if let Some(cpu_data) = per_cpu(cpu_id) {
            if !cpu_data.cpu_info.is_online() {
                let _ = crate::arch::x86_64::apic::send_startup_ipi(cpu_id, sipi_page);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // On AArch64, use PSCI (Power State Coordination Interface)
        // For now, just send a wake-up SGI
        send_ipi(cpu_id, 0);
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // On RISC-V, use SBI HSM (Hart State Management) extension
        // For now, just send IPI
        send_ipi(cpu_id, 0);
    }

    // Wait for CPU to come online
    let mut retries = 100;
    while retries > 0 {
        if let Some(cpu_data) = per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                println!("[SMP] CPU {} is now online", cpu_id);
                return Ok(());
            }
        }
        // Simulated delay
        retries -= 1;
    }

    Err(KernelError::Timeout {
        operation: "CPU online",
        duration_ms: 100,
    })
}

/// CPU hotplug: bring CPU offline
pub fn cpu_down(cpu_id: u8) -> Result<(), KernelError> {
    if cpu_id == 0 {
        return Err(KernelError::PermissionDenied {
            operation: "offline BSP (CPU 0)",
        });
    }

    if let Some(cpu_data) = per_cpu(cpu_id) {
        if !cpu_data.cpu_info.is_online() {
            return Err(KernelError::InvalidState {
                expected: "online",
                actual: "offline",
            });
        }

        // Migrate all tasks from this CPU
        let nr_tasks = cpu_data.cpu_info.nr_running.load(Ordering::Relaxed);
        if nr_tasks > 0 {
            println!("[SMP] Migrating {} tasks from CPU {}", nr_tasks, cpu_id);

            // Find target CPU with lowest load
            let target_cpu = find_least_loaded_cpu();
            if target_cpu == cpu_id {
                return Err(KernelError::ResourceExhausted {
                    resource: "available CPUs for migration",
                });
            }

            // Migrate all tasks
            let mut _migrated = 0;
            loop {
                let task = {
                    let mut queue = cpu_data.cpu_info.ready_queue.lock();
                    queue.dequeue()
                };

                if let Some(task_ptr) = task {
                    if migrate_task(task_ptr, cpu_id, target_cpu).is_ok() {
                        _migrated += 1;
                    }
                } else {
                    break;
                }
            }

            println!("[SMP] Migrated {} tasks to CPU {}", _migrated, target_cpu);
        }

        // Send CPU offline notification
        send_ipi(cpu_id, 0xFF); // Special offline vector

        // Mark CPU as offline
        cpu_data.cpu_info.bring_offline();

        println!("[SMP] CPU {} is now offline", cpu_id);
        Ok(())
    } else {
        Err(KernelError::NotInitialized { subsystem: "CPU" })
    }
}

/// Load balancing: migrate task between CPUs
pub fn migrate_task(
    task_ptr: core::ptr::NonNull<Task>,
    from_cpu: u8,
    to_cpu: u8,
) -> Result<(), KernelError> {
    // SAFETY: task_ptr is a valid NonNull<Task> passed by the caller
    // (cpu_down or load balancer). We only read task fields (cpu_affinity,
    // state, sched_class) for migration eligibility checks. The task is not
    // currently running on any CPU (verified by the Running state check).
    unsafe {
        let task = task_ptr.as_ref();

        // Check if migration is allowed
        if !task.cpu_affinity.contains(to_cpu) {
            return Err(KernelError::InvalidArgument {
                name: "cpu_affinity",
                value: "task affinity prevents migration",
            });
        }

        // Don't migrate running tasks
        if task.state == super::ProcessState::Running {
            return Err(KernelError::InvalidState {
                expected: "not running",
                actual: "running",
            });
        }

        // Don't migrate idle tasks
        if task.sched_class == super::task::SchedClass::Idle {
            return Err(KernelError::InvalidArgument {
                name: "sched_class",
                value: "cannot migrate idle task",
            });
        }
    }

    // Remove from source CPU queue
    let removed = if let Some(from_cpu_data) = per_cpu(from_cpu) {
        let mut queue = from_cpu_data.cpu_info.ready_queue.lock();
        if queue.remove(task_ptr) {
            from_cpu_data
                .cpu_info
                .nr_running
                .fetch_sub(1, Ordering::Relaxed);
            from_cpu_data.cpu_info.update_load();
            true
        } else {
            false
        }
    } else {
        false
    };

    if !removed {
        return Err(KernelError::NotFound {
            resource: "task in source CPU queue",
            id: from_cpu as u64,
        });
    }

    // Add to destination CPU queue
    if let Some(to_cpu_data) = per_cpu(to_cpu) {
        to_cpu_data.cpu_info.ready_queue.lock().enqueue(task_ptr);
        to_cpu_data
            .cpu_info
            .nr_running
            .fetch_add(1, Ordering::Relaxed);
        to_cpu_data.cpu_info.update_load();

        // SAFETY: task_ptr is a valid NonNull<Task> that was just removed
        // from the source CPU queue and enqueued on the destination. We
        // update current_cpu to reflect the new CPU assignment. No other
        // code is modifying this task concurrently because it was removed
        // from the source queue under lock.
        unsafe {
            let task_mut = task_ptr.as_ptr();
            (*task_mut).current_cpu = Some(to_cpu);
        }

        // Send IPI if destination CPU is idle
        if to_cpu_data.cpu_info.is_idle() {
            send_ipi(to_cpu, 0); // Wake up CPU
        }

        // Record migration metric
        super::metrics::SCHEDULER_METRICS.record_migration();

        Ok(())
    } else {
        Err(KernelError::NotInitialized {
            subsystem: "destination CPU",
        })
    }
}

/// Find least loaded CPU
pub fn find_least_loaded_cpu() -> u8 {
    let mut min_load = 100;
    let mut best_cpu = 0;

    for cpu_id in 0..MAX_CPUS as u8 {
        if let Some(cpu_data) = per_cpu(cpu_id) {
            if cpu_data.cpu_info.is_online() {
                let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);
                if load < min_load {
                    min_load = load;
                    best_cpu = cpu_id;
                }
            }
        }
    }

    best_cpu
}

/// Find least loaded CPU that matches affinity mask
pub fn find_least_loaded_cpu_with_affinity(affinity_mask: u64) -> u8 {
    let mut best_cpu = 0;
    let mut min_load = 100;
    let mut found_any = false;

    for cpu_id in 0..64.min(MAX_CPUS as u8) {
        // Check up to 64 CPUs (mask size)
        if (affinity_mask & (1u64 << cpu_id)) != 0 {
            if let Some(cpu_data) = per_cpu(cpu_id) {
                if cpu_data.cpu_info.is_online() {
                    let load = cpu_data.cpu_info.load.load(Ordering::Relaxed);
                    if load < min_load || !found_any {
                        min_load = load;
                        best_cpu = cpu_id;
                        found_any = true;
                    }
                }
            }
        }
    }

    // If no CPU matches affinity, fall back to least loaded
    if !found_any {
        find_least_loaded_cpu()
    } else {
        best_cpu
    }
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

/// Create idle task for specific CPU
#[cfg(feature = "alloc")]
fn create_cpu_idle_task(cpu_id: u8) {
    use alloc::{boxed::Box, format};
    use core::ptr::NonNull;

    use super::{
        idle_task_entry,
        task::{Priority, SchedClass, SchedPolicy, Task},
    };
    use crate::process::{ProcessId, ThreadId};

    // Allocate stack for idle task (8KB)
    const IDLE_STACK_SIZE: usize = 8192;
    let idle_stack = Box::leak(Box::new([0u8; IDLE_STACK_SIZE]));
    let idle_stack_top = idle_stack.as_ptr() as usize + IDLE_STACK_SIZE;

    // Get kernel page table
    let kernel_page_table = crate::mm::get_kernel_page_table();

    // Create idle task
    let mut idle_task = Box::new(Task::new(
        ProcessId(0),            // PID 0 for idle
        ThreadId(cpu_id as u64), // TID = CPU ID for idle tasks
        format!("idle-cpu{}", cpu_id),
        idle_task_entry as usize,
        idle_stack_top,
        kernel_page_table,
    ));

    // Set as idle priority
    idle_task.priority = Priority::Idle;
    idle_task.sched_class = SchedClass::Idle;
    idle_task.sched_policy = SchedPolicy::Idle;

    // Set CPU affinity to only this CPU
    idle_task.cpu_affinity = super::task::CpuSet::single(cpu_id);

    // Get raw pointer to idle task
    // Box::leak always returns a non-null pointer
    let idle_ptr =
        NonNull::new(Box::leak(idle_task) as *mut _).expect("Box::leak returned null (impossible)");

    // Initialize per-CPU scheduler with idle task
    if let Some(cpu_data) = per_cpu(cpu_id) {
        let mut scheduler = cpu_data.cpu_info.scheduler.lock();
        scheduler.init(idle_ptr);
    }
}
