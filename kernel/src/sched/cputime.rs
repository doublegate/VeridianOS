//! CPU time accounting (N-218, N-223).
//!
//! The dispatcher charges every task the time it runs (Linux's
//! `sum_exec_runtime`), samples at each timer tick whether it interrupted
//! the task in user or kernel mode, and counts its context switches. CPU
//! clocks report the run time; getrusage and times split it into user and
//! system time in proportion to the tick samples, as Linux's
//! `cputime_adjust` does without fine-grained accounting.

use core::sync::atomic::{AtomicU64, Ordering};

/// A task's running totals (updated by the dispatcher).
#[derive(Debug, Default)]
pub struct CpuUsage {
    runtime_ns: AtomicU64,
    user_ticks: AtomicU64,
    system_ticks: AtomicU64,
    voluntary: AtomicU64,
    involuntary: AtomicU64,
}

impl CpuUsage {
    pub const fn new() -> Self {
        Self {
            runtime_ns: AtomicU64::new(0),
            user_ticks: AtomicU64::new(0),
            system_ticks: AtomicU64::new(0),
            voluntary: AtomicU64::new(0),
            involuntary: AtomicU64::new(0),
        }
    }

    /// Charge `ns` of running.
    pub fn charge(&self, ns: u64) {
        self.runtime_ns.fetch_add(ns, Ordering::Relaxed);
    }

    /// A timer tick found the task running in user (or kernel) mode.
    pub fn tick(&self, user: bool) {
        if user {
            self.user_ticks.fetch_add(1, Ordering::Relaxed);
        } else {
            self.system_ticks.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The task left the CPU: by blocking or exiting (voluntary) or by
    /// preemption.
    pub fn switched(&self, voluntary: bool) {
        if voluntary {
            self.voluntary.fetch_add(1, Ordering::Relaxed);
        } else {
            self.involuntary.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The totals so far.
    pub fn snapshot(&self) -> CpuTimes {
        CpuTimes {
            runtime_ns: self.runtime_ns.load(Ordering::Relaxed),
            user_ticks: self.user_ticks.load(Ordering::Relaxed),
            system_ticks: self.system_ticks.load(Ordering::Relaxed),
            voluntary_switches: self.voluntary.load(Ordering::Relaxed),
            involuntary_switches: self.involuntary.load(Ordering::Relaxed),
        }
    }

    /// The totals, leaving zero (handed to the process when a thread
    /// exits, so they are counted once).
    pub fn take(&self) -> CpuTimes {
        CpuTimes {
            runtime_ns: self.runtime_ns.swap(0, Ordering::Relaxed),
            user_ticks: self.user_ticks.swap(0, Ordering::Relaxed),
            system_ticks: self.system_ticks.swap(0, Ordering::Relaxed),
            voluntary_switches: self.voluntary.swap(0, Ordering::Relaxed),
            involuntary_switches: self.involuntary.swap(0, Ordering::Relaxed),
        }
    }
}

/// CPU time used, as a value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub runtime_ns: u64,
    pub user_ticks: u64,
    pub system_ticks: u64,
    pub voluntary_switches: u64,
    pub involuntary_switches: u64,
}

impl CpuTimes {
    /// Nothing used.
    pub const ZERO: Self = Self {
        runtime_ns: 0,
        user_ticks: 0,
        system_ticks: 0,
        voluntary_switches: 0,
        involuntary_switches: 0,
    };

    /// Both totals together.
    pub fn plus(self, other: Self) -> Self {
        Self {
            runtime_ns: self.runtime_ns.saturating_add(other.runtime_ns),
            user_ticks: self.user_ticks.saturating_add(other.user_ticks),
            system_ticks: self.system_ticks.saturating_add(other.system_ticks),
            voluntary_switches: self
                .voluntary_switches
                .saturating_add(other.voluntary_switches),
            involuntary_switches: self
                .involuntary_switches
                .saturating_add(other.involuntary_switches),
        }
    }

    /// The run time split into (user, system) nanoseconds in proportion to
    /// the tick samples; all user time when there are none.
    pub fn user_system_ns(&self) -> (u64, u64) {
        let ticks = self.user_ticks as u128 + self.system_ticks as u128;
        if ticks == 0 {
            return (self.runtime_ns, 0);
        }
        let system = (self.runtime_ns as u128 * self.system_ticks as u128 / ticks) as u64;
        (self.runtime_ns - system, system)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_time_splits_by_tick_samples() {
        let t = CpuTimes {
            runtime_ns: 1_000,
            user_ticks: 3,
            system_ticks: 1,
            ..CpuTimes::default()
        };
        assert_eq!(t.user_system_ns(), (750, 250));
        let none = CpuTimes {
            runtime_ns: 7,
            ..CpuTimes::default()
        };
        assert_eq!(none.user_system_ns(), (7, 0));
        // No overflow for a long-running process.
        let big = CpuTimes {
            runtime_ns: u64::MAX,
            user_ticks: u64::MAX,
            system_ticks: u64::MAX,
            ..CpuTimes::default()
        };
        let (u, s) = big.user_system_ns();
        assert_eq!(u + s, u64::MAX);
    }

    #[test]
    fn usage_counts_and_is_taken_once() {
        let u = CpuUsage::new();
        u.charge(10);
        u.charge(5);
        u.tick(true);
        u.tick(false);
        u.tick(false);
        u.switched(true);
        u.switched(false);
        let taken = u.take();
        assert_eq!(
            taken,
            CpuTimes {
                runtime_ns: 15,
                user_ticks: 1,
                system_ticks: 2,
                voluntary_switches: 1,
                involuntary_switches: 1,
            }
        );
        assert_eq!(u.snapshot(), CpuTimes::default());
        assert_eq!(taken.plus(taken).runtime_ns, 30);
    }
}
