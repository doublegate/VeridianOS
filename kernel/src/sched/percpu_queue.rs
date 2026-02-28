//! Per-CPU Ready Queues with Work-Stealing
//!
//! Eliminates the global scheduler lock by giving each CPU its own run queue.
//! When a CPU's queue is empty, it steals work from the busiest neighbor.
//!
//! ## Design
//!
//! - Each CPU has a `PerCpuQueue` protected by a per-CPU spin lock
//! - `AtomicU32` run_length allows lock-free queue length queries
//! - Work stealing takes half the victim's queue (from the back)
//! - Steal threshold prevents thrashing on lightly loaded systems

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU32, Ordering};

use spin::Mutex;

use crate::process::ProcessId;

/// Maximum CPUs supported (matches smp::MAX_CPUS).
const MAX_CPUS: usize = 16;

/// Minimum queue depth before stealing is attempted.
const STEAL_THRESHOLD: u32 = 2;

/// Per-CPU run queue.
pub struct PerCpuQueue {
    /// Local run queue of process IDs.
    queue: Mutex<VecDeque<ProcessId>>,
    /// Atomic run length for lock-free queue length queries.
    run_length: AtomicU32,
}

impl Default for PerCpuQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PerCpuQueue {
    /// Create a new empty per-CPU queue.
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            run_length: AtomicU32::new(0),
        }
    }

    /// Push a process onto the local queue.
    pub fn push(&self, pid: ProcessId) {
        let mut q = self.queue.lock();
        q.push_back(pid);
        self.run_length.fetch_add(1, Ordering::Release);
    }

    /// Pop a process from the local queue (front = oldest).
    pub fn pop(&self) -> Option<ProcessId> {
        let mut q = self.queue.lock();
        if let Some(pid) = q.pop_front() {
            self.run_length.fetch_sub(1, Ordering::Release);
            Some(pid)
        } else {
            None
        }
    }

    /// Steal half the tasks from this queue (from the back = newest).
    ///
    /// Returns stolen tasks or an empty vec if queue is below threshold.
    pub fn steal(&self) -> alloc::vec::Vec<ProcessId> {
        let mut q = self.queue.lock();
        let len = q.len();
        if len < STEAL_THRESHOLD as usize {
            return alloc::vec::Vec::new();
        }

        let steal_count = len / 2;
        let mut stolen = alloc::vec::Vec::with_capacity(steal_count);

        for _ in 0..steal_count {
            if let Some(pid) = q.pop_back() {
                stolen.push(pid);
                self.run_length.fetch_sub(1, Ordering::Release);
            }
        }

        stolen
    }

    /// Get the current queue length (lock-free).
    pub fn queue_length(&self) -> u32 {
        self.run_length.load(Ordering::Acquire)
    }
}

/// Per-CPU scheduler managing all CPU queues.
pub struct PerCpuScheduler {
    /// One queue per CPU.
    queues: [PerCpuQueue; MAX_CPUS],
    /// Number of CPUs actually in use.
    cpu_count: AtomicU32,
}

impl Default for PerCpuScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl PerCpuScheduler {
    /// Create a new per-CPU scheduler.
    #[allow(clippy::declare_interior_mutable_const)]
    pub const fn new() -> Self {
        const EMPTY_QUEUE: PerCpuQueue = PerCpuQueue::new();
        Self {
            queues: [EMPTY_QUEUE; MAX_CPUS],
            cpu_count: AtomicU32::new(1),
        }
    }

    /// Set the number of active CPUs.
    pub fn set_cpu_count(&self, count: u32) {
        self.cpu_count
            .store(count.min(MAX_CPUS as u32), Ordering::Release);
    }

    /// Push a process onto a specific CPU's queue.
    pub fn push(&self, cpu_id: usize, pid: ProcessId) {
        if cpu_id < MAX_CPUS {
            self.queues[cpu_id].push(pid);
        }
    }

    /// Pop the next process from a specific CPU's queue.
    pub fn pop(&self, cpu_id: usize) -> Option<ProcessId> {
        if cpu_id < MAX_CPUS {
            self.queues[cpu_id].pop()
        } else {
            None
        }
    }

    /// Try to steal work from another CPU.
    ///
    /// Finds the busiest CPU and steals half its tasks.
    pub fn steal_for(&self, cpu_id: usize) -> Option<ProcessId> {
        let count = self.cpu_count.load(Ordering::Acquire) as usize;
        let mut busiest = 0usize;
        let mut max_len = 0u32;

        for i in 0..count {
            if i == cpu_id {
                continue;
            }
            let len = self.queues[i].queue_length();
            if len > max_len {
                max_len = len;
                busiest = i;
            }
        }

        if max_len < STEAL_THRESHOLD {
            return None;
        }

        let stolen = self.queues[busiest].steal();
        if stolen.is_empty() {
            return None;
        }

        // Push all but the first stolen task onto our queue
        let mut first = None;
        for pid in stolen {
            if first.is_none() {
                first = Some(pid);
            } else {
                self.queues[cpu_id].push(pid);
            }
        }

        first
    }

    /// Find the least-loaded CPU.
    pub fn find_least_loaded(&self) -> usize {
        let count = self.cpu_count.load(Ordering::Acquire) as usize;
        let mut min_len = u32::MAX;
        let mut best = 0;

        for i in 0..count {
            let len = self.queues[i].queue_length();
            if len < min_len {
                min_len = len;
                best = i;
            }
        }

        best
    }

    /// Rebalance: move tasks from overloaded CPUs to underloaded ones.
    pub fn rebalance(&self) {
        let count = self.cpu_count.load(Ordering::Acquire) as usize;
        if count < 2 {
            return;
        }

        // Find min and max loaded CPUs
        let mut min_cpu = 0;
        let mut max_cpu = 0;
        let mut min_len = u32::MAX;
        let mut max_len = 0u32;

        for i in 0..count {
            let len = self.queues[i].queue_length();
            if len < min_len {
                min_len = len;
                min_cpu = i;
            }
            if len > max_len {
                max_len = len;
                max_cpu = i;
            }
        }

        // Only rebalance if imbalance exceeds threshold
        if max_len > min_len + STEAL_THRESHOLD {
            let stolen = self.queues[max_cpu].steal();
            for pid in stolen {
                self.queues[min_cpu].push(pid);
            }
        }
    }

    /// Get queue length for a specific CPU.
    pub fn queue_length(&self, cpu_id: usize) -> u32 {
        if cpu_id < MAX_CPUS {
            self.queues[cpu_id].queue_length()
        } else {
            0
        }
    }
}

/// Global per-CPU scheduler instance.
pub static PERCPU_SCHED: Mutex<Option<PerCpuScheduler>> = Mutex::new(None);

/// Push a process onto the appropriate CPU's queue.
pub fn percpu_push(cpu_id: usize, pid: ProcessId) {
    if let Some(ref sched) = *PERCPU_SCHED.lock() {
        sched.push(cpu_id, pid);
    }
}

/// Pop the next process from a CPU's queue, with work-stealing fallback.
pub fn percpu_pop(cpu_id: usize) -> Option<ProcessId> {
    if let Some(ref sched) = *PERCPU_SCHED.lock() {
        // Try local queue first
        if let Some(pid) = sched.pop(cpu_id) {
            return Some(pid);
        }
        // Try stealing from busiest neighbor
        sched.steal_for(cpu_id)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let q = PerCpuQueue::new();
        q.push(ProcessId(1));
        q.push(ProcessId(2));
        assert_eq!(q.queue_length(), 2);
        assert_eq!(q.pop(), Some(ProcessId(1)));
        assert_eq!(q.pop(), Some(ProcessId(2)));
        assert_eq!(q.pop(), None);
        assert_eq!(q.queue_length(), 0);
    }

    #[test]
    fn test_steal() {
        let q = PerCpuQueue::new();
        q.push(ProcessId(1));
        q.push(ProcessId(2));
        q.push(ProcessId(3));
        q.push(ProcessId(4));

        let stolen = q.steal();
        assert_eq!(stolen.len(), 2);
        assert_eq!(q.queue_length(), 2);
    }

    #[test]
    fn test_steal_empty() {
        let q = PerCpuQueue::new();
        let stolen = q.steal();
        assert!(stolen.is_empty());
    }

    #[test]
    fn test_steal_single() {
        let q = PerCpuQueue::new();
        q.push(ProcessId(1));
        let stolen = q.steal();
        assert!(stolen.is_empty()); // Below threshold
    }

    #[test]
    fn test_percpu_scheduler() {
        let sched = PerCpuScheduler::new();
        sched.set_cpu_count(4);

        sched.push(0, ProcessId(10));
        sched.push(0, ProcessId(11));
        sched.push(1, ProcessId(20));

        assert_eq!(sched.queue_length(0), 2);
        assert_eq!(sched.queue_length(1), 1);
        assert_eq!(sched.find_least_loaded(), 2); // CPU 2 has 0 tasks
    }

    #[test]
    fn test_rebalance() {
        let sched = PerCpuScheduler::new();
        sched.set_cpu_count(2);

        // Load CPU 0 heavily
        for i in 0..8 {
            sched.push(0, ProcessId(i));
        }

        assert_eq!(sched.queue_length(0), 8);
        assert_eq!(sched.queue_length(1), 0);

        sched.rebalance();

        // After rebalance, some work should have moved
        assert!(sched.queue_length(0) < 8);
        assert!(sched.queue_length(1) > 0);
    }

    #[test]
    fn test_invalid_cpu() {
        let sched = PerCpuScheduler::new();
        assert_eq!(sched.pop(999), None);
        assert_eq!(sched.queue_length(999), 0);
    }
}
