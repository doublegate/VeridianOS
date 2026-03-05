//! Earliest Deadline First (EDF) Deadline Scheduler
//!
//! Implements SCHED_DEADLINE scheduling policy alongside the existing CFS
//! scheduler. Deadline tasks always preempt CFS tasks. Among deadline tasks,
//! the one with the earliest absolute deadline is selected. Admission control
//! ensures total CPU utilization does not exceed 100% (using fixed-point
//! arithmetic scaled by 1000).
//!
//! Key concepts:
//! - **Runtime**: Maximum execution time per period (nanoseconds)
//! - **Deadline**: Relative deadline from start of period (nanoseconds)
//! - **Period**: Activation period (nanoseconds)
//! - **Admission control**: Sum of (runtime/period) for all tasks must not
//!   exceed 1.0
//! - **Replenishment**: Runtime is reset at period boundaries

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

use crate::{
    error::{KernelError, SchedError},
    process::ProcessId,
};

/// SCHED_DEADLINE policy constant (Linux-compatible value)
pub const SCHED_DEADLINE: u32 = 6;

/// Fixed-point scale factor for utilization calculations.
/// Utilization is represented as parts per 1000 (permille).
/// A task with runtime=period has utilization = 1000.
/// Total utilization must not exceed 1000 (= 100%).
const UTIL_SCALE: u64 = 1000;

/// Maximum number of deadline tasks (for bounded resource usage)
const MAX_DEADLINE_TASKS: usize = 64;

/// A deadline scheduling entity describing a task's timing parameters.
#[derive(Debug, Clone, Copy)]
pub struct DeadlineEntity {
    /// Process ID
    pub pid: ProcessId,
    /// Worst-case execution time per period (nanoseconds)
    pub runtime_ns: u64,
    /// Relative deadline from period start (nanoseconds)
    pub deadline_ns: u64,
    /// Activation period (nanoseconds)
    pub period_ns: u64,
    /// Remaining runtime in current period (nanoseconds).
    /// Decremented by tick(); reset by replenish().
    pub runtime_remaining: u64,
    /// Absolute deadline for current period (nanoseconds since boot).
    /// Used to determine scheduling priority (earliest wins).
    pub deadline_abs: u64,
    /// Absolute start of current period (nanoseconds since boot).
    /// Used to detect period boundaries for replenishment.
    pub period_start: u64,
    /// Whether this entity is currently throttled (runtime exhausted).
    pub throttled: bool,
}

impl DeadlineEntity {
    /// Create a new deadline entity.
    ///
    /// # Arguments
    /// * `pid` - Process ID
    /// * `runtime_ns` - Worst-case execution time per period
    /// * `deadline_ns` - Relative deadline (must be <= period)
    /// * `period_ns` - Activation period
    /// * `now_ns` - Current time in nanoseconds since boot
    pub fn new(
        pid: ProcessId,
        runtime_ns: u64,
        deadline_ns: u64,
        period_ns: u64,
        now_ns: u64,
    ) -> Self {
        Self {
            pid,
            runtime_ns,
            deadline_ns,
            period_ns,
            runtime_remaining: runtime_ns,
            deadline_abs: now_ns.saturating_add(deadline_ns),
            period_start: now_ns,
            throttled: false,
        }
    }

    /// Compute the utilization of this entity in permille (parts per 1000).
    /// Returns `runtime_ns * 1000 / period_ns`.
    fn utilization_permille(&self) -> u64 {
        if self.period_ns == 0 {
            return UTIL_SCALE; // Treat zero-period as full utilization
        }
        self.runtime_ns.saturating_mul(UTIL_SCALE) / self.period_ns
    }

    /// Check if this entity's period has expired and needs replenishment.
    fn needs_replenish(&self, now_ns: u64) -> bool {
        now_ns >= self.period_start.saturating_add(self.period_ns)
    }

    /// Replenish runtime for a new period.
    fn replenish(&mut self, now_ns: u64) {
        // Advance to the current period (handle missed periods)
        if self.period_ns > 0 {
            while self.period_start.saturating_add(self.period_ns) <= now_ns {
                self.period_start = self.period_start.saturating_add(self.period_ns);
            }
        }
        self.deadline_abs = self.period_start.saturating_add(self.deadline_ns);
        self.runtime_remaining = self.runtime_ns;
        self.throttled = false;
    }
}

/// Scheduling attributes for sched_setattr-style interface.
#[derive(Debug, Clone, Copy)]
pub struct SchedAttr {
    /// Scheduling policy (should be SCHED_DEADLINE)
    pub policy: u32,
    /// Worst-case execution time per period (nanoseconds)
    pub runtime_ns: u64,
    /// Relative deadline (nanoseconds)
    pub deadline_ns: u64,
    /// Activation period (nanoseconds)
    pub period_ns: u64,
}

impl SchedAttr {
    /// Validate the scheduling attributes.
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.policy != SCHED_DEADLINE {
            return Err(KernelError::InvalidArgument {
                name: "policy",
                value: "must be SCHED_DEADLINE",
            });
        }
        if self.runtime_ns == 0 {
            return Err(KernelError::InvalidArgument {
                name: "runtime_ns",
                value: "must be > 0",
            });
        }
        if self.deadline_ns == 0 {
            return Err(KernelError::InvalidArgument {
                name: "deadline_ns",
                value: "must be > 0",
            });
        }
        if self.period_ns == 0 {
            return Err(KernelError::InvalidArgument {
                name: "period_ns",
                value: "must be > 0",
            });
        }
        if self.runtime_ns > self.deadline_ns {
            return Err(KernelError::InvalidArgument {
                name: "runtime_ns",
                value: "must be <= deadline_ns",
            });
        }
        if self.deadline_ns > self.period_ns {
            return Err(KernelError::InvalidArgument {
                name: "deadline_ns",
                value: "must be <= period_ns",
            });
        }
        Ok(())
    }
}

/// The Earliest Deadline First (EDF) deadline scheduler.
///
/// Manages deadline tasks separately from CFS. Deadline tasks always have
/// higher priority than CFS tasks. Among deadline tasks, the one with the
/// earliest absolute deadline that still has runtime remaining is selected.
#[cfg(feature = "alloc")]
pub struct DeadlineScheduler {
    /// All registered deadline entities, keyed by PID.
    tasks: BTreeMap<u64, DeadlineEntity>,
    /// Total utilization in permille (sum of runtime/period * 1000 for all
    /// tasks). Must not exceed 1000 (100%).
    total_utilization: u64,
    /// PID of the currently running deadline task (if any).
    current_pid: Option<u64>,
}

#[cfg(feature = "alloc")]
impl Default for DeadlineScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl DeadlineScheduler {
    /// Create a new empty deadline scheduler.
    pub const fn new() -> Self {
        Self {
            tasks: BTreeMap::new(),
            total_utilization: 0,
            current_pid: None,
        }
    }

    /// Return the number of registered deadline tasks.
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Return the total utilization in permille.
    pub fn total_utilization(&self) -> u64 {
        self.total_utilization
    }

    /// Check if a task with the given PID is registered.
    pub fn has_task(&self, pid: ProcessId) -> bool {
        self.tasks.contains_key(&pid.0)
    }

    /// Add a deadline task with admission control.
    ///
    /// Performs admission control: the new task is accepted only if total
    /// utilization (including this task) does not exceed 1000 permille (100%).
    ///
    /// # Arguments
    /// * `pid` - Process ID
    /// * `runtime_ns` - Worst-case execution time per period (nanoseconds)
    /// * `deadline_ns` - Relative deadline (nanoseconds, must be <= period)
    /// * `period_ns` - Activation period (nanoseconds)
    /// * `now_ns` - Current time in nanoseconds since boot
    ///
    /// # Errors
    /// Returns `KernelError` if:
    /// - Parameters are invalid (zero values, runtime > deadline > period)
    /// - Admission control fails (total utilization would exceed 100%)
    /// - Maximum number of deadline tasks reached
    /// - Task already registered
    pub fn add_task(
        &mut self,
        pid: ProcessId,
        runtime_ns: u64,
        deadline_ns: u64,
        period_ns: u64,
        now_ns: u64,
    ) -> Result<(), KernelError> {
        // Validate parameters
        let attr = SchedAttr {
            policy: SCHED_DEADLINE,
            runtime_ns,
            deadline_ns,
            period_ns,
        };
        attr.validate()?;

        // Check for duplicate
        if self.tasks.contains_key(&pid.0) {
            return Err(KernelError::SchedulerError(SchedError::AlreadyScheduled));
        }

        // Check capacity
        if self.tasks.len() >= MAX_DEADLINE_TASKS {
            return Err(KernelError::ResourceExhausted {
                resource: "deadline_tasks",
            });
        }

        // Admission control: check if adding this task would exceed 100% utilization
        let entity = DeadlineEntity::new(pid, runtime_ns, deadline_ns, period_ns, now_ns);
        let new_util = entity.utilization_permille();
        let proposed_total = self.total_utilization.saturating_add(new_util);

        if proposed_total > UTIL_SCALE {
            return Err(KernelError::ResourceExhausted {
                resource: "deadline_bandwidth",
            });
        }

        self.total_utilization = proposed_total;
        self.tasks.insert(pid.0, entity);
        Ok(())
    }

    /// Add a task using SchedAttr parameters.
    pub fn add_task_attr(
        &mut self,
        pid: ProcessId,
        attr: &SchedAttr,
        now_ns: u64,
    ) -> Result<(), KernelError> {
        attr.validate()?;
        self.add_task(
            pid,
            attr.runtime_ns,
            attr.deadline_ns,
            attr.period_ns,
            now_ns,
        )
    }

    /// Remove a deadline task.
    ///
    /// Returns the removed entity, or an error if the task was not found.
    pub fn remove_task(&mut self, pid: ProcessId) -> Result<DeadlineEntity, KernelError> {
        let entity = self
            .tasks
            .remove(&pid.0)
            .ok_or(KernelError::SchedulerError(SchedError::TaskNotFound {
                id: pid.0,
            }))?;

        // Reclaim utilization
        let util = entity.utilization_permille();
        self.total_utilization = self.total_utilization.saturating_sub(util);

        // Clear current if this was the running task
        if self.current_pid == Some(pid.0) {
            self.current_pid = None;
        }

        Ok(entity)
    }

    /// Pick the next deadline task to run.
    ///
    /// Returns the PID of the task with the earliest absolute deadline that
    /// still has runtime remaining (not throttled). Returns `None` if no
    /// eligible deadline task exists.
    pub fn pick_next(&mut self) -> Option<ProcessId> {
        let mut best_pid: Option<u64> = None;
        let mut best_deadline = u64::MAX;

        for (&pid, entity) in self.tasks.iter() {
            // Skip throttled tasks (runtime exhausted)
            if entity.throttled {
                continue;
            }
            // Skip tasks with no remaining runtime
            if entity.runtime_remaining == 0 {
                continue;
            }
            // Pick the task with the earliest absolute deadline
            if entity.deadline_abs < best_deadline {
                best_deadline = entity.deadline_abs;
                best_pid = Some(pid);
            }
        }

        if let Some(pid) = best_pid {
            self.current_pid = Some(pid);
            Some(ProcessId(pid))
        } else {
            self.current_pid = None;
            None
        }
    }

    /// Account elapsed time against the currently running deadline task.
    ///
    /// Decrements `runtime_remaining` for the currently running deadline task.
    /// If runtime is exhausted, the task is throttled until the next period.
    ///
    /// # Arguments
    /// * `elapsed_ns` - Nanoseconds elapsed since last tick
    ///
    /// # Returns
    /// `true` if the current task was throttled (needs reschedule), `false`
    /// otherwise.
    pub fn tick(&mut self, elapsed_ns: u64) -> bool {
        let pid = match self.current_pid {
            Some(pid) => pid,
            None => return false,
        };

        let entity = match self.tasks.get_mut(&pid) {
            Some(e) => e,
            None => {
                self.current_pid = None;
                return false;
            }
        };

        if entity.runtime_remaining <= elapsed_ns {
            entity.runtime_remaining = 0;
            entity.throttled = true;
            true // Needs reschedule
        } else {
            entity.runtime_remaining = entity.runtime_remaining.saturating_sub(elapsed_ns);
            false
        }
    }

    /// Replenish runtime for tasks whose periods have expired.
    ///
    /// Iterates all deadline tasks and resets runtime for any whose period
    /// has elapsed. Should be called periodically (e.g., on timer tick).
    ///
    /// # Arguments
    /// * `now_ns` - Current time in nanoseconds since boot
    ///
    /// # Returns
    /// Number of tasks that were replenished.
    pub fn replenish(&mut self, now_ns: u64) -> usize {
        let mut count = 0;

        for entity in self.tasks.values_mut() {
            if entity.needs_replenish(now_ns) {
                entity.replenish(now_ns);
                count += 1;
            }
        }

        count
    }

    /// Compute the time (in nanoseconds) until the next deadline event.
    ///
    /// This is the minimum of:
    /// - The runtime remaining for the current task (throttle point)
    /// - The time until the next period boundary (replenishment point)
    /// - The time until the earliest absolute deadline
    ///
    /// Returns `None` if there are no deadline tasks. The returned value can
    /// be used to program the APIC timer for precise deadline scheduling.
    pub fn next_deadline_event(&self, now_ns: u64) -> Option<u64> {
        if self.tasks.is_empty() {
            return None;
        }

        let mut min_event = u64::MAX;

        for entity in self.tasks.values() {
            // Time until period boundary (replenishment)
            let period_end = entity.period_start.saturating_add(entity.period_ns);
            if period_end > now_ns {
                let until_replenish = period_end.saturating_sub(now_ns);
                if until_replenish < min_event {
                    min_event = until_replenish;
                }
            }

            // Time until absolute deadline
            if entity.deadline_abs > now_ns {
                let until_deadline = entity.deadline_abs.saturating_sub(now_ns);
                if until_deadline < min_event {
                    min_event = until_deadline;
                }
            }
        }

        // Also consider runtime remaining of current task
        if let Some(pid) = self.current_pid {
            if let Some(entity) = self.tasks.get(&pid) {
                if !entity.throttled && entity.runtime_remaining < min_event {
                    min_event = entity.runtime_remaining;
                }
            }
        }

        if min_event == u64::MAX {
            None
        } else {
            Some(min_event)
        }
    }

    /// Check whether a deadline task should preempt the current CFS task.
    ///
    /// Returns `true` if any non-throttled deadline task with remaining runtime
    /// exists, meaning it should preempt CFS.
    pub fn should_preempt_cfs(&self) -> bool {
        self.tasks
            .values()
            .any(|e| !e.throttled && e.runtime_remaining > 0)
    }

    /// Get a reference to a deadline entity by PID.
    pub fn get_task(&self, pid: ProcessId) -> Option<&DeadlineEntity> {
        self.tasks.get(&pid.0)
    }

    /// Set the currently running deadline task PID.
    pub fn set_current(&mut self, pid: Option<ProcessId>) {
        self.current_pid = pid.map(|p| p.0);
    }

    /// Get the currently running deadline task PID.
    pub fn current(&self) -> Option<ProcessId> {
        self.current_pid.map(ProcessId)
    }

    /// Recalculate total utilization from scratch.
    /// Useful after bulk modifications.
    fn recalculate_utilization(&mut self) {
        self.total_utilization = self
            .tasks
            .values()
            .map(|e| e.utilization_permille())
            .fold(0u64, |acc, u| acc.saturating_add(u));
    }
}

/// Global deadline scheduler instance, protected by a spinlock.
#[cfg(feature = "alloc")]
pub static DEADLINE_SCHEDULER: spin::Mutex<DeadlineScheduler> =
    spin::Mutex::new(DeadlineScheduler::new());

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // Helper to create a scheduler and add a task
    #[cfg(feature = "alloc")]
    fn make_scheduler() -> DeadlineScheduler {
        DeadlineScheduler::new()
    }

    // --- DeadlineEntity tests ---

    #[test]
    fn test_entity_utilization_permille() {
        // 50% utilization: runtime=5ms, period=10ms
        let e = DeadlineEntity::new(ProcessId(1), 5_000_000, 10_000_000, 10_000_000, 0);
        assert_eq!(e.utilization_permille(), 500);

        // 100% utilization
        let e = DeadlineEntity::new(ProcessId(2), 10_000_000, 10_000_000, 10_000_000, 0);
        assert_eq!(e.utilization_permille(), 1000);

        // 10% utilization
        let e = DeadlineEntity::new(ProcessId(3), 1_000_000, 10_000_000, 10_000_000, 0);
        assert_eq!(e.utilization_permille(), 100);

        // Zero period => full utilization (safety)
        let e = DeadlineEntity::new(ProcessId(4), 1_000_000, 0, 0, 0);
        assert_eq!(e.utilization_permille(), 1000);
    }

    #[test]
    fn test_entity_needs_replenish() {
        let e = DeadlineEntity::new(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 100);
        // Before period end
        assert!(!e.needs_replenish(5_000_000));
        // At period end
        assert!(e.needs_replenish(10_000_100));
        // After period end
        assert!(e.needs_replenish(20_000_000));
    }

    #[test]
    fn test_entity_replenish() {
        let mut e = DeadlineEntity::new(ProcessId(1), 2_000_000, 5_000_000, 10_000_000, 0);
        e.runtime_remaining = 0;
        e.throttled = true;

        // Replenish at period boundary
        e.replenish(10_000_000);
        assert_eq!(e.runtime_remaining, 2_000_000);
        assert_eq!(e.period_start, 10_000_000);
        assert_eq!(e.deadline_abs, 15_000_000); // 10M + 5M
        assert!(!e.throttled);
    }

    #[test]
    fn test_entity_replenish_skipped_periods() {
        let mut e = DeadlineEntity::new(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        e.runtime_remaining = 0;
        e.throttled = true;

        // Skip 3 periods
        e.replenish(35_000_000);
        assert_eq!(e.period_start, 30_000_000);
        assert_eq!(e.deadline_abs, 35_000_000);
        assert_eq!(e.runtime_remaining, 1_000_000);
    }

    // --- SchedAttr validation tests ---

    #[test]
    fn test_sched_attr_validate_valid() {
        let attr = SchedAttr {
            policy: SCHED_DEADLINE,
            runtime_ns: 1_000_000,
            deadline_ns: 5_000_000,
            period_ns: 10_000_000,
        };
        assert!(attr.validate().is_ok());
    }

    #[test]
    fn test_sched_attr_validate_wrong_policy() {
        let attr = SchedAttr {
            policy: 0,
            runtime_ns: 1_000_000,
            deadline_ns: 5_000_000,
            period_ns: 10_000_000,
        };
        assert!(attr.validate().is_err());
    }

    #[test]
    fn test_sched_attr_validate_zero_runtime() {
        let attr = SchedAttr {
            policy: SCHED_DEADLINE,
            runtime_ns: 0,
            deadline_ns: 5_000_000,
            period_ns: 10_000_000,
        };
        assert!(attr.validate().is_err());
    }

    #[test]
    fn test_sched_attr_validate_runtime_exceeds_deadline() {
        let attr = SchedAttr {
            policy: SCHED_DEADLINE,
            runtime_ns: 6_000_000,
            deadline_ns: 5_000_000,
            period_ns: 10_000_000,
        };
        assert!(attr.validate().is_err());
    }

    #[test]
    fn test_sched_attr_validate_deadline_exceeds_period() {
        let attr = SchedAttr {
            policy: SCHED_DEADLINE,
            runtime_ns: 1_000_000,
            deadline_ns: 15_000_000,
            period_ns: 10_000_000,
        };
        assert!(attr.validate().is_err());
    }

    // --- DeadlineScheduler tests ---

    #[cfg(feature = "alloc")]
    #[test]
    fn test_add_task_basic() {
        let mut sched = make_scheduler();
        let result = sched.add_task(
            ProcessId(1),
            1_000_000,  // 1ms runtime
            5_000_000,  // 5ms deadline
            10_000_000, // 10ms period
            0,
        );
        assert!(result.is_ok());
        assert_eq!(sched.task_count(), 1);
        assert_eq!(sched.total_utilization(), 100); // 10%
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_add_task_duplicate() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        let result = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        assert!(result.is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_admission_control_accept() {
        let mut sched = make_scheduler();
        // 30% + 30% + 30% = 90% -> should accept
        assert!(sched
            .add_task(ProcessId(1), 3_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
        assert!(sched
            .add_task(ProcessId(2), 3_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
        assert!(sched
            .add_task(ProcessId(3), 3_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
        assert_eq!(sched.total_utilization(), 900);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_admission_control_reject() {
        let mut sched = make_scheduler();
        // 60% + 50% = 110% -> second should be rejected
        assert!(sched
            .add_task(ProcessId(1), 6_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
        let result = sched.add_task(ProcessId(2), 5_000_000, 10_000_000, 10_000_000, 0);
        assert!(result.is_err());
        assert_eq!(sched.task_count(), 1);
        assert_eq!(sched.total_utilization(), 600);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_admission_control_exact_100_percent() {
        let mut sched = make_scheduler();
        // Exactly 100% should be accepted
        assert!(sched
            .add_task(ProcessId(1), 10_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
        assert_eq!(sched.total_utilization(), 1000);
        // Adding any more should fail
        let result = sched.add_task(ProcessId(2), 1_000_000, 10_000_000, 10_000_000, 0);
        assert!(result.is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_remove_task() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 3_000_000, 10_000_000, 10_000_000, 0);
        let _ = sched.add_task(ProcessId(2), 2_000_000, 10_000_000, 10_000_000, 0);
        assert_eq!(sched.total_utilization(), 500); // 30% + 20%

        let result = sched.remove_task(ProcessId(1));
        assert!(result.is_ok());
        assert_eq!(sched.task_count(), 1);
        assert_eq!(sched.total_utilization(), 200); // only 20% remains
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_remove_task_not_found() {
        let mut sched = make_scheduler();
        let result = sched.remove_task(ProcessId(99));
        assert!(result.is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_remove_then_readmit() {
        let mut sched = make_scheduler();
        // Fill to 100%
        let _ = sched.add_task(ProcessId(1), 10_000_000, 10_000_000, 10_000_000, 0);
        assert_eq!(sched.total_utilization(), 1000);

        // Remove frees bandwidth
        let _ = sched.remove_task(ProcessId(1));
        assert_eq!(sched.total_utilization(), 0);

        // Can add a new task now
        assert!(sched
            .add_task(ProcessId(2), 5_000_000, 10_000_000, 10_000_000, 0)
            .is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_pick_next_earliest_deadline() {
        let mut sched = make_scheduler();
        // Task 1: deadline_abs = 0 + 20M = 20M
        let _ = sched.add_task(ProcessId(1), 1_000_000, 20_000_000, 30_000_000, 0);
        // Task 2: deadline_abs = 0 + 10M = 10M (earlier)
        let _ = sched.add_task(ProcessId(2), 1_000_000, 10_000_000, 30_000_000, 0);
        // Task 3: deadline_abs = 0 + 15M = 15M
        let _ = sched.add_task(ProcessId(3), 1_000_000, 15_000_000, 30_000_000, 0);

        let next = sched.pick_next();
        assert_eq!(next, Some(ProcessId(2))); // Earliest deadline wins
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_pick_next_skips_throttled() {
        let mut sched = make_scheduler();
        // Task 1: earlier deadline but will be throttled
        let _ = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        // Task 2: later deadline
        let _ = sched.add_task(ProcessId(2), 1_000_000, 8_000_000, 10_000_000, 0);

        // Throttle task 1
        if let Some(e) = sched.tasks.get_mut(&1) {
            e.throttled = true;
            e.runtime_remaining = 0;
        }

        let next = sched.pick_next();
        assert_eq!(next, Some(ProcessId(2)));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_pick_next_none_when_all_throttled() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);

        if let Some(e) = sched.tasks.get_mut(&1) {
            e.throttled = true;
            e.runtime_remaining = 0;
        }

        assert_eq!(sched.pick_next(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_pick_next_empty() {
        let mut sched = make_scheduler();
        assert_eq!(sched.pick_next(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_tick_decrements_runtime() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 5_000_000, 10_000_000, 10_000_000, 0);
        sched.current_pid = Some(1);

        let throttled = sched.tick(1_000_000); // 1ms tick
        assert!(!throttled);

        let entity = sched.get_task(ProcessId(1)).unwrap();
        assert_eq!(entity.runtime_remaining, 4_000_000);
        assert!(!entity.throttled);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_tick_runtime_exhaustion() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 2_000_000, 10_000_000, 10_000_000, 0);
        sched.current_pid = Some(1);

        // Tick away all runtime
        let throttled = sched.tick(2_000_000);
        assert!(throttled);

        let entity = sched.get_task(ProcessId(1)).unwrap();
        assert_eq!(entity.runtime_remaining, 0);
        assert!(entity.throttled);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_tick_over_exhaustion() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 1_000_000, 10_000_000, 10_000_000, 0);
        sched.current_pid = Some(1);

        // Tick more than runtime remaining
        let throttled = sched.tick(5_000_000);
        assert!(throttled);

        let entity = sched.get_task(ProcessId(1)).unwrap();
        assert_eq!(entity.runtime_remaining, 0);
        assert!(entity.throttled);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_tick_no_current() {
        let mut sched = make_scheduler();
        let throttled = sched.tick(1_000_000);
        assert!(!throttled);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_replenish_at_period_boundary() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 2_000_000, 5_000_000, 10_000_000, 0);
        sched.current_pid = Some(1);

        // Exhaust runtime
        sched.tick(2_000_000);
        assert!(sched.get_task(ProcessId(1)).unwrap().throttled);

        // Replenish at period boundary (10ms)
        let count = sched.replenish(10_000_000);
        assert_eq!(count, 1);

        let entity = sched.get_task(ProcessId(1)).unwrap();
        assert_eq!(entity.runtime_remaining, 2_000_000);
        assert!(!entity.throttled);
        assert_eq!(entity.period_start, 10_000_000);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_replenish_not_yet_due() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 2_000_000, 5_000_000, 10_000_000, 0);

        // Too early for replenishment
        let count = sched.replenish(5_000_000);
        assert_eq!(count, 0);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_next_deadline_event() {
        let mut sched = make_scheduler();
        // period=10ms, deadline=5ms
        let _ = sched.add_task(ProcessId(1), 2_000_000, 5_000_000, 10_000_000, 0);

        let event = sched.next_deadline_event(0);
        assert!(event.is_some());
        // Should be min of: period_end(10M), deadline_abs(5M)
        // = 5M (earliest deadline)
        assert_eq!(event.unwrap(), 5_000_000);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_next_deadline_event_with_current_runtime() {
        let mut sched = make_scheduler();
        // period=10ms, deadline=5ms, runtime=1ms
        let _ = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        sched.current_pid = Some(1);

        let event = sched.next_deadline_event(0);
        assert!(event.is_some());
        // min(period_end=10M, deadline_abs=5M, runtime_remaining=1M) = 1M
        assert_eq!(event.unwrap(), 1_000_000);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_next_deadline_event_empty() {
        let sched = make_scheduler();
        assert_eq!(sched.next_deadline_event(0), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_should_preempt_cfs() {
        let mut sched = make_scheduler();
        assert!(!sched.should_preempt_cfs());

        let _ = sched.add_task(ProcessId(1), 1_000_000, 5_000_000, 10_000_000, 0);
        assert!(sched.should_preempt_cfs());

        // Throttle the task
        if let Some(e) = sched.tasks.get_mut(&1) {
            e.throttled = true;
        }
        assert!(!sched.should_preempt_cfs());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_full_lifecycle() {
        let mut sched = make_scheduler();

        // Add two tasks: T1 (20%, deadline 5ms) and T2 (30%, deadline 8ms)
        let _ = sched.add_task(ProcessId(1), 2_000_000, 5_000_000, 10_000_000, 0);
        let _ = sched.add_task(ProcessId(2), 3_000_000, 8_000_000, 10_000_000, 0);
        assert_eq!(sched.total_utilization(), 500); // 50%

        // Pick next: T1 has earlier deadline (5ms vs 8ms)
        assert_eq!(sched.pick_next(), Some(ProcessId(1)));

        // T1 runs for 2ms -> exhausted
        assert!(sched.tick(2_000_000));

        // Pick next: T1 throttled, T2 runs
        assert_eq!(sched.pick_next(), Some(ProcessId(2)));

        // T2 runs for 3ms -> exhausted
        assert!(sched.tick(3_000_000));

        // All throttled
        assert_eq!(sched.pick_next(), None);
        assert!(!sched.should_preempt_cfs());

        // Replenish at period boundary (10ms)
        let count = sched.replenish(10_000_000);
        assert_eq!(count, 2);
        assert!(sched.should_preempt_cfs());

        // Both tasks available again; pick earliest deadline
        let next = sched.pick_next();
        assert!(next.is_some());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_recalculate_utilization() {
        let mut sched = make_scheduler();
        let _ = sched.add_task(ProcessId(1), 3_000_000, 10_000_000, 10_000_000, 0);
        let _ = sched.add_task(ProcessId(2), 2_000_000, 10_000_000, 10_000_000, 0);

        // Manually corrupt utilization
        sched.total_utilization = 999;
        sched.recalculate_utilization();
        assert_eq!(sched.total_utilization, 500); // 30% + 20%
    }
}
