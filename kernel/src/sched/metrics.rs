//! Scheduler performance metrics and measurement
//!
//! Provides comprehensive metrics for scheduler performance including
//! context switch latency, scheduling overhead, and load distribution.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Scheduler performance metrics
pub struct SchedulerMetrics {
    /// Total number of context switches
    pub context_switches: AtomicU64,
    /// Number of voluntary context switches (yield, sleep, block)
    pub voluntary_switches: AtomicU64,
    /// Number of involuntary context switches (preemption)
    pub involuntary_switches: AtomicU64,
    /// Total CPU cycles spent in scheduler
    pub scheduler_cycles: AtomicU64,
    /// Total CPU cycles spent in context switching
    pub switch_cycles: AtomicU64,
    /// Number of scheduling decisions made
    pub schedule_calls: AtomicU64,
    /// Number of times idle task was scheduled
    pub idle_scheduled: AtomicU64,
    /// Average context switch latency in cycles
    pub avg_switch_latency: AtomicU64,
    /// Minimum context switch latency
    pub min_switch_latency: AtomicU64,
    /// Maximum context switch latency
    pub max_switch_latency: AtomicU64,
    /// Load balancing operations
    pub load_balance_count: AtomicU64,
    /// Task migrations between CPUs
    pub task_migrations: AtomicU64,
    /// IPC blocks
    pub ipc_blocks: AtomicU64,
    /// IPC wakeups
    pub ipc_wakeups: AtomicU64,
}

impl SchedulerMetrics {
    /// Create new metrics instance
    pub const fn new() -> Self {
        Self {
            context_switches: AtomicU64::new(0),
            voluntary_switches: AtomicU64::new(0),
            involuntary_switches: AtomicU64::new(0),
            scheduler_cycles: AtomicU64::new(0),
            switch_cycles: AtomicU64::new(0),
            schedule_calls: AtomicU64::new(0),
            idle_scheduled: AtomicU64::new(0),
            avg_switch_latency: AtomicU64::new(0),
            min_switch_latency: AtomicU64::new(u64::MAX),
            max_switch_latency: AtomicU64::new(0),
            load_balance_count: AtomicU64::new(0),
            task_migrations: AtomicU64::new(0),
            ipc_blocks: AtomicU64::new(0),
            ipc_wakeups: AtomicU64::new(0),
        }
    }

    /// Record a context switch
    pub fn record_context_switch(&self, latency_cycles: u64, voluntary: bool) {
        self.context_switches.fetch_add(1, Ordering::Relaxed);

        if voluntary {
            self.voluntary_switches.fetch_add(1, Ordering::Relaxed);
        } else {
            self.involuntary_switches.fetch_add(1, Ordering::Relaxed);
        }

        self.switch_cycles
            .fetch_add(latency_cycles, Ordering::Relaxed);

        // Update min/max latency
        let mut min = self.min_switch_latency.load(Ordering::Relaxed);
        while latency_cycles < min {
            match self.min_switch_latency.compare_exchange_weak(
                min,
                latency_cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => min = current,
            }
        }

        let mut max = self.max_switch_latency.load(Ordering::Relaxed);
        while latency_cycles > max {
            match self.max_switch_latency.compare_exchange_weak(
                max,
                latency_cycles,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => max = current,
            }
        }

        // Update average
        let count = self.context_switches.load(Ordering::Relaxed);
        if count > 0 {
            let total_cycles = self.switch_cycles.load(Ordering::Relaxed);
            self.avg_switch_latency
                .store(total_cycles / count, Ordering::Relaxed);
        }
    }

    /// Record scheduler overhead
    pub fn record_scheduler_overhead(&self, cycles: u64) {
        self.scheduler_cycles.fetch_add(cycles, Ordering::Relaxed);
        self.schedule_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record idle task scheduled
    pub fn record_idle_scheduled(&self) {
        self.idle_scheduled.fetch_add(1, Ordering::Relaxed);
    }

    /// Record load balancing operation
    pub fn record_load_balance(&self) {
        self.load_balance_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record task migration
    pub fn record_migration(&self) {
        self.task_migrations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record IPC block
    pub fn record_ipc_block(&self) {
        self.ipc_blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// Record IPC wakeup
    pub fn record_ipc_wakeup(&self) {
        self.ipc_wakeups.fetch_add(1, Ordering::Relaxed);
    }

    /// Get summary of metrics
    pub fn get_summary(&self) -> MetricsSummary {
        let context_switches = self.context_switches.load(Ordering::Relaxed);
        let scheduler_cycles = self.scheduler_cycles.load(Ordering::Relaxed);
        let schedule_calls = self.schedule_calls.load(Ordering::Relaxed);

        MetricsSummary {
            context_switches,
            voluntary_switches: self.voluntary_switches.load(Ordering::Relaxed),
            involuntary_switches: self.involuntary_switches.load(Ordering::Relaxed),
            avg_switch_latency: self.avg_switch_latency.load(Ordering::Relaxed),
            min_switch_latency: {
                let min = self.min_switch_latency.load(Ordering::Relaxed);
                if min == u64::MAX {
                    0
                } else {
                    min
                }
            },
            max_switch_latency: self.max_switch_latency.load(Ordering::Relaxed),
            scheduler_overhead_pct: if context_switches > 0 {
                (scheduler_cycles * 100)
                    / (scheduler_cycles + self.switch_cycles.load(Ordering::Relaxed))
            } else {
                0
            },
            idle_percentage: if schedule_calls > 0 {
                (self.idle_scheduled.load(Ordering::Relaxed) * 100) / schedule_calls
            } else {
                0
            },
            load_balance_count: self.load_balance_count.load(Ordering::Relaxed),
            task_migrations: self.task_migrations.load(Ordering::Relaxed),
            ipc_blocks: self.ipc_blocks.load(Ordering::Relaxed),
            ipc_wakeups: self.ipc_wakeups.load(Ordering::Relaxed),
        }
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.context_switches.store(0, Ordering::Relaxed);
        self.voluntary_switches.store(0, Ordering::Relaxed);
        self.involuntary_switches.store(0, Ordering::Relaxed);
        self.scheduler_cycles.store(0, Ordering::Relaxed);
        self.switch_cycles.store(0, Ordering::Relaxed);
        self.schedule_calls.store(0, Ordering::Relaxed);
        self.idle_scheduled.store(0, Ordering::Relaxed);
        self.avg_switch_latency.store(0, Ordering::Relaxed);
        self.min_switch_latency.store(u64::MAX, Ordering::Relaxed);
        self.max_switch_latency.store(0, Ordering::Relaxed);
        self.load_balance_count.store(0, Ordering::Relaxed);
        self.task_migrations.store(0, Ordering::Relaxed);
        self.ipc_blocks.store(0, Ordering::Relaxed);
        self.ipc_wakeups.store(0, Ordering::Relaxed);
    }
}

/// Summary of scheduler metrics
pub struct MetricsSummary {
    pub context_switches: u64,
    pub voluntary_switches: u64,
    pub involuntary_switches: u64,
    pub avg_switch_latency: u64,
    pub min_switch_latency: u64,
    pub max_switch_latency: u64,
    pub scheduler_overhead_pct: u64,
    pub idle_percentage: u64,
    pub load_balance_count: u64,
    pub task_migrations: u64,
    pub ipc_blocks: u64,
    pub ipc_wakeups: u64,
}

/// Per-CPU scheduler metrics
pub struct PerCpuMetrics {
    /// CPU ID
    pub cpu_id: u8,
    /// Number of tasks scheduled on this CPU
    pub tasks_scheduled: AtomicU64,
    /// Total runtime on this CPU
    pub total_runtime: AtomicU64,
    /// Idle time on this CPU
    pub idle_time: AtomicU64,
    /// Number of IPIs received
    pub ipis_received: AtomicU32,
    /// Number of IPIs sent
    pub ipis_sent: AtomicU32,
}

impl PerCpuMetrics {
    /// Create new per-CPU metrics
    pub const fn new(cpu_id: u8) -> Self {
        Self {
            cpu_id,
            tasks_scheduled: AtomicU64::new(0),
            total_runtime: AtomicU64::new(0),
            idle_time: AtomicU64::new(0),
            ipis_received: AtomicU32::new(0),
            ipis_sent: AtomicU32::new(0),
        }
    }
}

impl Default for SchedulerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Global scheduler metrics
pub static SCHEDULER_METRICS: SchedulerMetrics = SchedulerMetrics::new();

/// Read CPU timestamp counter.
///
/// Delegates to the centralized [`crate::arch::entropy::read_timestamp`]
/// which provides architecture-specific implementations for x86_64 (RDTSC),
/// AArch64 (CNTVCT_EL0), and RISC-V (rdcycle).
#[inline]
pub fn read_tsc() -> u64 {
    crate::arch::entropy::read_timestamp()
}

/// Print scheduler metrics
pub fn print_metrics() {
    let summary = SCHEDULER_METRICS.get_summary();

    #[cfg(target_arch = "x86_64")]
    {
        println!("[SCHED] Scheduler Metrics:");
        println!(
            "  Context switches: {} (voluntary: {}, involuntary: {})",
            summary.context_switches, summary.voluntary_switches, summary.involuntary_switches
        );
        println!(
            "  Switch latency: avg={} cycles, min={}, max={}",
            summary.avg_switch_latency, summary.min_switch_latency, summary.max_switch_latency
        );
        println!("  Scheduler overhead: {}%", summary.scheduler_overhead_pct);
        println!("  Idle time: {}%", summary.idle_percentage);
        println!(
            "  Load balancing: {} ops, {} migrations",
            summary.load_balance_count, summary.task_migrations
        );
        println!(
            "  IPC: {} blocks, {} wakeups",
            summary.ipc_blocks, summary.ipc_wakeups
        );
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // Other architectures may not have println! available yet
        let _ = summary;
    }
}
