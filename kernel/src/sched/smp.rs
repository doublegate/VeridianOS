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
        // Read MPIDR_EL1 for affinity information
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
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[SMP] Initializing SMP support (simplified for AArch64)...\n");
            // Skip complex SMP initialization for AArch64
            // Just initialize BSP with minimal setup
            uart_write_str("[SMP] SMP initialized (BSP only for AArch64)\n");
        }
        return;
    }

    #[cfg(target_arch = "riscv64")]
    {
        println!("[SMP] Initializing SMP support (simplified for RISC-V)...");
        // Skip complex SMP initialization for RISC-V
        // Just initialize BSP with minimal setup
        println!("[SMP] SMP initialized (BSP only for RISC-V)");
        return;
    }

    // Simplified x86_64 SMP init for bootloader 0.11 compatibility
    // Skip complex topology detection and AP wakeup for now
    #[cfg(target_arch = "x86_64")]
    {
        println!("[SMP] Initializing SMP support (simplified for x86_64)...");
        // Skip CPU topology detection which uses CPUID
        // Skip init_cpu(0) which accesses large PER_CPU_DATA array
        // Skip wake_up_aps() which tries to wake secondary CPUs
        println!("[SMP] SMP initialized (BSP only for x86_64)");
    }
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
    unsafe {
        PER_CPU_DATA[cpu_id as usize]
            .as_ref()
            .expect("Per-CPU data not initialized")
    }
}

/// Get per-CPU data for specific CPU
pub fn per_cpu(cpu_id: u8) -> Option<&'static PerCpuData> {
    unsafe { PER_CPU_DATA[cpu_id as usize].as_ref() }
}

/// Get current CPU ID
pub fn current_cpu_id() -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        // Read from APIC ID or use CPUID
        unsafe {
            use core::arch::x86_64::__cpuid;
            let cpuid = __cpuid(0x1);
            ((cpuid.ebx >> 24) & 0xFF) as u8
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Read from MPIDR_EL1
        unsafe {
            let mpidr: u64;
            core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) mpidr);
            (mpidr & 0xFF) as u8
        }
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    {
        // Read hart ID
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
        // Use APIC to send IPI
        // For now, use a simplified implementation
        // Note: APIC module would be implemented in arch-specific code
        println!(
            "[SMP] IPI to CPU {} vector {:#x} (x86_64 APIC)",
            target_cpu, vector
        );
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Use GIC to send SGI (Software Generated Interrupt)
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
pub fn cpu_up(cpu_id: u8) -> Result<(), &'static str> {
    if cpu_id >= MAX_CPUS as u8 {
        return Err("Invalid CPU ID");
    }

    if let Some(cpu_data) = per_cpu(cpu_id) {
        if cpu_data.cpu_info.is_online() {
            return Err("CPU already online");
        }
    } else {
        init_cpu(cpu_id);
    }

    // Send INIT/SIPI to wake up CPU
    #[cfg(target_arch = "x86_64")]
    {
        // Send INIT IPI
        send_ipi(cpu_id, 0x00); // INIT vector

        // Wait 10ms (simulated)
        // In real implementation, would use timer

        // Send SIPI with startup vector
        let sipi_vector = 0x08; // Startup at 0x8000
        send_ipi(cpu_id, sipi_vector);

        // Wait 200us and send second SIPI if needed (simulated)

        if let Some(cpu_data) = per_cpu(cpu_id) {
            if !cpu_data.cpu_info.is_online() {
                send_ipi(cpu_id, sipi_vector);
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

    Err("CPU failed to come online")
}

/// CPU hotplug: bring CPU offline
pub fn cpu_down(cpu_id: u8) -> Result<(), &'static str> {
    if cpu_id == 0 {
        return Err("Cannot offline BSP");
    }

    if let Some(cpu_data) = per_cpu(cpu_id) {
        if !cpu_data.cpu_info.is_online() {
            return Err("CPU already offline");
        }

        // Migrate all tasks from this CPU
        let nr_tasks = cpu_data.cpu_info.nr_running.load(Ordering::Relaxed);
        if nr_tasks > 0 {
            println!("[SMP] Migrating {} tasks from CPU {}", nr_tasks, cpu_id);

            // Find target CPU with lowest load
            let target_cpu = find_least_loaded_cpu();
            if target_cpu == cpu_id {
                return Err("No other CPU available for migration");
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
        Err("CPU not initialized")
    }
}

/// Load balancing: migrate task between CPUs
pub fn migrate_task(
    task_ptr: core::ptr::NonNull<Task>,
    from_cpu: u8,
    to_cpu: u8,
) -> Result<(), &'static str> {
    unsafe {
        let task = task_ptr.as_ref();

        // Check if migration is allowed
        if !task.cpu_affinity.contains(to_cpu) {
            return Err("Task affinity prevents migration");
        }

        // Don't migrate running tasks
        if task.state == super::ProcessState::Running {
            return Err("Cannot migrate running task");
        }

        // Don't migrate idle tasks
        if task.sched_class == super::task::SchedClass::Idle {
            return Err("Cannot migrate idle task");
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
        return Err("Task not found in source CPU queue");
    }

    // Add to destination CPU queue
    if let Some(to_cpu_data) = per_cpu(to_cpu) {
        to_cpu_data.cpu_info.ready_queue.lock().enqueue(task_ptr);
        to_cpu_data
            .cpu_info
            .nr_running
            .fetch_add(1, Ordering::Relaxed);
        to_cpu_data.cpu_info.update_load();

        // Update task's current CPU hint
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
        Err("Destination CPU not initialized")
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
    let idle_ptr = NonNull::new(Box::leak(idle_task) as *mut _).unwrap();

    // Initialize per-CPU scheduler with idle task
    if let Some(cpu_data) = per_cpu(cpu_id) {
        let mut scheduler = cpu_data.cpu_info.scheduler.lock();
        scheduler.init(idle_ptr);
    }
}
