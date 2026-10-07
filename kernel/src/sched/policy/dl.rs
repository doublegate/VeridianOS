//! Deadline class: `SCHED_DEADLINE`, EDF with the Constant Bandwidth Server
//! (ADR 0007; Linux Documentation/scheduler/sched-deadline.rst).
//!
//! A task reserves `runtime` every `period` and must get it by `deadline`
//! after each activation. Among runnable tasks the earliest absolute deadline
//! runs. CBS isolates tasks from each other's overruns:
//!
//! - **Wakeup:** if the current deadline has passed, or the remaining runtime
//!   would exceed the reserved bandwidth over the time left (`remaining /
//!   (deadline - now) > runtime / period`), the task starts a fresh
//!   reservation: `deadline = now + rel_deadline`, full runtime.
//! - **Depletion:** at zero runtime the task is throttled until its current
//!   deadline, then replenished: `deadline += period`, `remaining += runtime`.
//!
//! Admission control (in the syscall layer) keeps the total bandwidth within
//! 95% of the online CPUs.

use alloc::collections::{BTreeMap, BTreeSet};

use super::TaskKey;

/// Bandwidth fixed point: 1.0 = `1 << BW_SHIFT` (Linux uses 20 bits).
pub const BW_SHIFT: u32 = 20;
/// Deadline tasks may use 95% of each CPU (Linux default).
pub const BW_LIMIT_PER_CPU: u64 = (95 << BW_SHIFT) / 100;

/// Per-task deadline reservation and state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlEntity {
    pub runtime: u64,
    pub rel_deadline: u64,
    pub period: u64,
    /// Runtime left in the current reservation (may go negative by an overrun).
    pub remaining: i64,
    /// Absolute deadline of the current reservation.
    pub deadline: u64,
}

/// Why a set of deadline parameters was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlParamError {
    /// Not `0 < runtime <= deadline <= period`, or below the 1 us minimum.
    Invalid,
    /// Admission control: not enough bandwidth left.
    Busy,
}

impl DlEntity {
    /// Validate parameters (`period == 0` means `period = deadline`, as Linux).
    pub fn new(runtime: u64, deadline: u64, period: u64) -> Result<Self, DlParamError> {
        let period = if period == 0 { deadline } else { period };
        if runtime < 1024 || runtime > deadline || deadline > period || period >> 63 != 0 {
            return Err(DlParamError::Invalid);
        }
        Ok(Self {
            runtime,
            rel_deadline: deadline,
            period,
            remaining: 0,
            deadline: 0,
        })
    }

    /// Reserved bandwidth (`runtime / period` in `BW_SHIFT` fixed point).
    pub fn bandwidth(&self) -> u64 {
        bandwidth(self.runtime, self.period)
    }

    /// CBS wakeup rule.
    pub fn on_wakeup(&mut self, now: u64) {
        let expired = self.deadline <= now;
        // remaining/(deadline-now) > runtime/period, cross-multiplied.
        let overflows = !expired
            && (self.remaining.max(0) as u128) * (self.period as u128)
                > (self.runtime as u128) * ((self.deadline - now) as u128);
        if expired || overflows || self.remaining <= 0 {
            self.deadline = now.saturating_add(self.rel_deadline);
            self.remaining = self.runtime as i64;
        }
    }

    /// Replenish after depletion (may take several periods after an overrun).
    pub fn replenish(&mut self, now: u64) {
        while self.remaining <= 0 {
            self.deadline = self.deadline.saturating_add(self.period);
            self.remaining = self.remaining.saturating_add(self.runtime as i64);
        }
        if self.deadline <= now {
            // Fell behind entirely (long overrun): restart the reservation.
            self.deadline = now.saturating_add(self.rel_deadline);
            self.remaining = self.runtime as i64;
        }
    }
}

/// `runtime / period` in `BW_SHIFT` fixed point.
pub fn bandwidth(runtime: u64, period: u64) -> u64 {
    (((runtime as u128) << BW_SHIFT) / (period.max(1) as u128)) as u64
}

/// Admission test: can `new_bw` join `total_bw` on `cpus` CPUs?
pub fn admit(total_bw: u64, new_bw: u64, cpus: u32) -> bool {
    total_bw.saturating_add(new_bw) <= BW_LIMIT_PER_CPU.saturating_mul(cpus.max(1) as u64)
}

/// One CPU's deadline queue.
#[derive(Debug, Default)]
pub struct DlRq {
    tasks: BTreeMap<TaskKey, DlEntity>,
    ready: BTreeSet<(u64, TaskKey)>,
    /// Throttled tasks and the time they are replenished.
    throttled: BTreeMap<TaskKey, u64>,
}

impl DlRq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn get(&self, key: TaskKey) -> Option<&DlEntity> {
        self.tasks.get(&key)
    }

    pub fn is_throttled(&self, key: TaskKey) -> bool {
        self.throttled.contains_key(&key)
    }

    /// Queue a waking task (CBS wakeup rule applied).
    pub fn enqueue(&mut self, key: TaskKey, mut e: DlEntity, now: u64) {
        e.on_wakeup(now);
        self.ready.insert((e.deadline, key));
        self.tasks.insert(key, e);
    }

    pub fn dequeue(&mut self, key: TaskKey) -> Option<DlEntity> {
        let e = self.tasks.remove(&key)?;
        self.ready.remove(&(e.deadline, key));
        self.throttled.remove(&key);
        Some(e)
    }

    /// Charge execution time. Returns true when the task was throttled.
    pub fn charge(&mut self, key: TaskKey, delta_ns: u64) -> bool {
        let Some(e) = self.tasks.get_mut(&key) else {
            return false;
        };
        e.remaining = e
            .remaining
            .saturating_sub(delta_ns.min(i64::MAX as u64) as i64);
        if e.remaining <= 0 && !self.throttled.contains_key(&key) {
            self.ready.remove(&(e.deadline, key));
            self.throttled.insert(key, e.deadline);
            return true;
        }
        false
    }

    /// Replenish every throttled task whose time has come. Returns true if
    /// any task became runnable.
    pub fn replenish_due(&mut self, now: u64) -> bool {
        let due: alloc::vec::Vec<TaskKey> = self
            .throttled
            .iter()
            .filter(|&(_, &at)| at <= now)
            .map(|(&k, _)| k)
            .collect();
        for &k in &due {
            self.throttled.remove(&k);
            if let Some(e) = self.tasks.get_mut(&k) {
                e.replenish(now);
                self.ready.insert((e.deadline, k));
            }
        }
        !due.is_empty()
    }

    /// The earliest deadline among runnable (non-throttled) tasks.
    pub fn pick(&self) -> Option<TaskKey> {
        self.ready.iter().next().map(|&(_, k)| k)
    }

    pub fn should_preempt(&self, curr: TaskKey, woken: TaskKey) -> bool {
        match (self.tasks.get(&curr), self.tasks.get(&woken)) {
            (Some(c), Some(w)) => !self.is_throttled(woken) && w.deadline < c.deadline,
            (None, Some(_)) => !self.is_throttled(woken),
            _ => false,
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = TaskKey> + '_ {
        self.tasks.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn parameters_are_validated() {
        assert!(DlEntity::new(10 * MS, 30 * MS, 100 * MS).is_ok());
        assert_eq!(
            DlEntity::new(40 * MS, 30 * MS, 100 * MS),
            Err(DlParamError::Invalid)
        );
        assert_eq!(
            DlEntity::new(10 * MS, 300 * MS, 100 * MS),
            Err(DlParamError::Invalid)
        );
        assert_eq!(
            DlEntity::new(0, 30 * MS, 100 * MS),
            Err(DlParamError::Invalid)
        );
        let implicit = DlEntity::new(10 * MS, 30 * MS, 0).unwrap();
        assert_eq!(implicit.period, 30 * MS);
    }

    #[test]
    fn admission_caps_at_95_percent_per_cpu() {
        let half = bandwidth(50 * MS, 100 * MS);
        assert!(admit(0, half, 1));
        assert!(admit(half, bandwidth(45 * MS, 100 * MS), 1));
        assert!(!admit(half, bandwidth(46 * MS, 100 * MS), 1));
        // Two CPUs allow 190%: 150% + 40% fits, 150% + 50% does not.
        assert!(admit(half * 3, bandwidth(40 * MS, 100 * MS), 2));
        assert!(!admit(half * 3, half, 2));
    }

    #[test]
    fn earliest_deadline_first() {
        let mut rq = DlRq::new();
        rq.enqueue(1, DlEntity::new(5 * MS, 50 * MS, 100 * MS).unwrap(), 0);
        rq.enqueue(2, DlEntity::new(5 * MS, 20 * MS, 100 * MS).unwrap(), 0);
        assert_eq!(rq.pick(), Some(2));
        assert!(rq.should_preempt(1, 2));
        assert!(!rq.should_preempt(2, 1));
    }

    #[test]
    fn depletion_throttles_until_deadline_then_replenishes() {
        let mut rq = DlRq::new();
        rq.enqueue(1, DlEntity::new(10 * MS, 30 * MS, 100 * MS).unwrap(), 0);
        assert!(!rq.charge(1, 6 * MS));
        assert!(rq.charge(1, 4 * MS), "runtime used up");
        assert_eq!(rq.pick(), None);
        assert!(!rq.replenish_due(29 * MS));
        assert!(rq.replenish_due(30 * MS));
        let e = rq.get(1).unwrap();
        assert_eq!(e.deadline, 130 * MS);
        assert_eq!(e.remaining, (10 * MS) as i64);
        assert_eq!(rq.pick(), Some(1));
    }

    #[test]
    fn cbs_wakeup_resets_when_bandwidth_would_be_exceeded() {
        let mut e = DlEntity::new(10 * MS, 100 * MS, 100 * MS).unwrap();
        e.on_wakeup(0);
        assert_eq!(e.deadline, 100 * MS);
        // Used 2 ms, slept until 95 ms: 8 ms left in 5 ms > 10%: reset.
        e.remaining = (8 * MS) as i64;
        e.on_wakeup(95 * MS);
        assert_eq!(e.deadline, 195 * MS);
        assert_eq!(e.remaining, (10 * MS) as i64);
        // Used 9 ms, wakes at 10 ms: 1 ms left in 90 ms is within 10%: keep.
        e.remaining = MS as i64;
        let d = e.deadline;
        e.on_wakeup(105 * MS);
        assert_eq!(e.deadline, d);
    }

    #[test]
    fn long_overrun_restarts_reservation() {
        let mut e = DlEntity::new(10 * MS, 30 * MS, 100 * MS).unwrap();
        e.on_wakeup(0);
        e.remaining = -(500 * MS as i64);
        e.replenish(10_000 * MS);
        assert!(e.deadline > 10_000 * MS);
        assert!(e.remaining > 0);
    }
}
