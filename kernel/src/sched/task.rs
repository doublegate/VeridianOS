//! Task management and task control block (TCB) implementation

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::string::String;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{ProcessId, ProcessState, ThreadId};

/// Task priority levels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    /// Real-time highest priority
    RealTimeHigh = 0,
    /// Real-time normal priority
    RealTimeNormal = 10,
    /// Real-time low priority
    RealTimeLow = 20,
    /// System high priority
    SystemHigh = 30,
    /// System normal priority
    SystemNormal = 40,
    /// User high priority
    UserHigh = 50,
    /// User normal priority
    #[default]
    UserNormal = 60,
    /// User low priority
    UserLow = 70,
    /// Idle priority
    Idle = 99,
}

/// Scheduling class
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedClass {
    /// Real-time scheduling (FIFO/RR)
    RealTime,
    /// Normal scheduling (CFS-like)
    Normal,
    /// Idle scheduling
    Idle,
}

/// Task scheduling policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedPolicy {
    /// First-In-First-Out (real-time)
    Fifo,
    /// Round-Robin (real-time)
    RoundRobin,
    /// Completely Fair Scheduler
    Cfs,
    /// Idle tasks
    Idle,
}

/// CPU affinity mask
#[derive(Debug, Clone)]
pub struct CpuSet {
    /// Bitmap of allowed CPUs (bit N = CPU N)
    mask: u64,
}

impl CpuSet {
    /// Create new CPU set with all CPUs allowed
    pub fn all() -> Self {
        Self { mask: !0u64 }
    }

    /// Create new CPU set with single CPU
    pub fn single(cpu: u8) -> Self {
        Self { mask: 1u64 << cpu }
    }

    /// Create from raw mask
    pub fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    /// Check if CPU is in set
    pub fn contains(&self, cpu: u8) -> bool {
        (self.mask & (1u64 << cpu)) != 0
    }

    /// Add CPU to set
    pub fn add(&mut self, cpu: u8) {
        self.mask |= 1u64 << cpu;
    }

    /// Remove CPU from set
    pub fn remove(&mut self, cpu: u8) {
        self.mask &= !(1u64 << cpu);
    }
}

impl Default for CpuSet {
    fn default() -> Self {
        Self::all()
    }
}

/// Task statistics
#[derive(Debug, Default)]
pub struct TaskStats {
    /// Total time spent running (in ticks)
    pub runtime: AtomicU64,
    /// Number of times scheduled
    pub run_count: AtomicU64,
    /// Number of voluntary context switches
    pub voluntary_switches: AtomicU64,
    /// Number of involuntary context switches
    pub involuntary_switches: AtomicU64,
    /// Last time scheduled (in ticks)
    pub last_run: AtomicU64,
}

/// Architecture-specific task context
#[derive(Debug)]
pub enum TaskContext {
    /// x86_64 task context
    #[cfg(target_arch = "x86_64")]
    X86_64(crate::arch::x86_64::context::X86_64Context),

    /// AArch64 task context
    #[cfg(target_arch = "aarch64")]
    AArch64(crate::arch::aarch64::context::AArch64Context),

    /// RISC-V task context
    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    RiscV(crate::arch::riscv::context::RiscVContext),
}

/// Task Control Block (TCB)
pub struct Task {
    /// Process ID
    pub pid: ProcessId,
    /// Thread ID
    pub tid: ThreadId,
    /// Parent process ID
    pub parent_pid: ProcessId,
    /// Task name
    #[cfg(feature = "alloc")]
    pub name: String,
    /// Task state
    pub state: ProcessState,
    /// Scheduling priority
    pub priority: Priority,
    /// Scheduling class
    pub sched_class: SchedClass,
    /// Scheduling policy
    pub sched_policy: SchedPolicy,
    /// CPU affinity
    pub cpu_affinity: CpuSet,
    /// Current CPU (if running)
    pub current_cpu: Option<u8>,
    /// Time slice remaining (in ticks)
    pub time_slice: u32,
    /// Virtual runtime (for CFS)
    pub vruntime: u64,
    /// Task statistics
    pub stats: TaskStats,
    /// Architecture-specific context
    pub context: TaskContext,
    /// Kernel stack pointer
    pub kernel_stack: usize,
    /// User stack pointer
    pub user_stack: usize,
    /// Page table base address
    pub page_table: usize,
    /// IPC endpoint blocked on (if any)
    pub blocked_on: Option<u64>,
    /// Wait queue link (for blocking)
    pub wait_link: Option<usize>,
    /// Ready queue link
    pub ready_link: Option<usize>,
}

impl Task {
    /// Create new task
    #[cfg(feature = "alloc")]
    pub fn new(
        pid: ProcessId,
        tid: ThreadId,
        name: String,
        entry_point: usize,
        stack_base: usize,
        page_table: usize,
    ) -> Self {
        Self {
            pid,
            tid,
            parent_pid: 0,
            name,
            state: ProcessState::Ready,
            priority: Priority::default(),
            sched_class: SchedClass::Normal,
            sched_policy: SchedPolicy::Cfs,
            cpu_affinity: CpuSet::default(),
            current_cpu: None,
            time_slice: DEFAULT_TIME_SLICE,
            vruntime: 0,
            stats: TaskStats::default(),
            context: TaskContext::new(entry_point, stack_base),
            kernel_stack: stack_base,
            user_stack: 0,
            page_table,
            blocked_on: None,
            wait_link: None,
            ready_link: None,
        }
    }

    /// Check if task can run on given CPU
    pub fn can_run_on(&self, cpu: u8) -> bool {
        self.cpu_affinity.contains(cpu)
    }

    /// Update runtime statistics
    pub fn update_runtime(&self, ticks: u64) {
        self.stats.runtime.fetch_add(ticks, Ordering::Relaxed);
        self.stats
            .last_run
            .store(crate::arch::timer::get_ticks(), Ordering::Relaxed);
    }

    /// Mark as scheduled
    pub fn mark_scheduled(&self, _cpu: u8, voluntary: bool) {
        self.stats.run_count.fetch_add(1, Ordering::Relaxed);
        if voluntary {
            self.stats
                .voluntary_switches
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats
                .involuntary_switches
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Calculate dynamic priority
    pub fn effective_priority(&self) -> u8 {
        match self.sched_class {
            SchedClass::RealTime => self.priority as u8,
            SchedClass::Normal => {
                // Boost priority based on how long task has been waiting
                let wait_time =
                    crate::arch::timer::get_ticks() - self.stats.last_run.load(Ordering::Relaxed);
                let boost = (wait_time / PRIORITY_BOOST_INTERVAL).min(20) as u8;
                (self.priority as u8).saturating_sub(boost)
            }
            SchedClass::Idle => Priority::Idle as u8,
        }
    }
}

/// Default time slice in timer ticks
pub const DEFAULT_TIME_SLICE: u32 = 10;

/// Interval for priority boosting (in ticks)
pub const PRIORITY_BOOST_INTERVAL: u64 = 100;

impl TaskContext {
    /// Create new task context for entry point
    #[cfg(target_arch = "x86_64")]
    pub fn new(entry_point: usize, stack_base: usize) -> Self {
        TaskContext::X86_64(crate::arch::x86_64::context::X86_64Context::new(
            entry_point,
            stack_base,
        ))
    }

    #[cfg(target_arch = "aarch64")]
    pub fn new(entry_point: usize, stack_base: usize) -> Self {
        TaskContext::AArch64(crate::arch::aarch64::context::AArch64Context::new(
            entry_point,
            stack_base,
        ))
    }

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    pub fn new(entry_point: usize, stack_base: usize) -> Self {
        TaskContext::RiscV(crate::arch::riscv::context::RiscVContext::new(
            entry_point,
            stack_base,
        ))
    }
}

/// Task ID allocator
static NEXT_TID: AtomicU64 = AtomicU64::new(1);

/// Allocate new thread ID
pub fn alloc_tid() -> ThreadId {
    NEXT_TID.fetch_add(1, Ordering::Relaxed)
}
