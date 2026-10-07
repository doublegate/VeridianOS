//! SMP placement and load balancing decisions (ADR 0007).
//!
//! Pure functions over per-CPU snapshots; the dispatcher gathers the
//! snapshot, calls these, and performs any migration under the run queue
//! locks. Affinity is a bit mask of allowed CPUs.

/// A task that ran on a CPU within this window is assumed to still have a
/// warm cache there (Linux `sysctl_sched_migration_cost`, 0.5 ms; a larger
/// window suits the coarse 1 ms tick here).
pub const CACHE_HOT_NS: u64 = 5_000_000;
/// Balance only when the busiest CPU carries 25% more load than this one
/// (Linux `imbalance_pct = 125`).
pub const IMBALANCE_PCT: u64 = 125;
/// Periodic balancing interval.
pub const BALANCE_INTERVAL_NS: u64 = 4_000_000;

/// Per-CPU state the decisions read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuSnapshot {
    pub online: bool,
    /// Nothing runnable (running the idle task).
    pub idle: bool,
    pub nr_running: usize,
    /// Decaying load average.
    pub load: u64,
}

fn allowed(mask: u64, cpu: usize) -> bool {
    cpu < 64 && mask & (1 << cpu) != 0
}

/// Where a waking (or new) task should run.
///
/// The previous CPU if it is idle; else an idle allowed CPU; else the
/// previous CPU if the task is cache-hot there; else the least loaded
/// allowed CPU. `None` only if no allowed CPU is online.
pub fn select_cpu(
    prev: usize,
    affinity: u64,
    cpus: &[CpuSnapshot],
    since_ran_ns: u64,
) -> Option<usize> {
    let ok = |c: usize| allowed(affinity, c) && cpus.get(c).is_some_and(|s| s.online);
    if ok(prev) && cpus[prev].idle {
        return Some(prev);
    }
    if let Some(idle) = (0..cpus.len())
        .filter(|&c| ok(c) && cpus[c].idle)
        .min_by_key(|&c| cpus[c].load)
    {
        return Some(idle);
    }
    if ok(prev) && since_ran_ns < CACHE_HOT_NS {
        return Some(prev);
    }
    (0..cpus.len())
        .filter(|&c| ok(c))
        .min_by_key(|&c| (cpus[c].load, cpus[c].nr_running, c != prev))
}

/// The CPU `this` should pull from, if the imbalance justifies it. A CPU
/// that is idle (`newly_idle`) pulls from any CPU with a waiting task.
pub fn find_busiest(this: usize, cpus: &[CpuSnapshot], newly_idle: bool) -> Option<usize> {
    let me = cpus.get(this)?;
    let busiest = (0..cpus.len())
        .filter(|&c| c != this && cpus[c].online && cpus[c].nr_running >= 2)
        .max_by_key(|&c| (cpus[c].load, cpus[c].nr_running))?;
    let b = cpus[busiest];
    if newly_idle || me.nr_running == 0 {
        return Some(busiest);
    }
    let imbalanced = b.load * 100 > me.load * IMBALANCE_PCT && b.nr_running > me.nr_running + 1;
    imbalanced.then_some(busiest)
}

/// How many queued tasks to move from `busiest` to `this`: half the
/// difference in runnable tasks, at least one.
pub fn tasks_to_move(busiest: &CpuSnapshot, this: &CpuSnapshot) -> usize {
    (busiest.nr_running.saturating_sub(this.nr_running) / 2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(idle: bool, nr: usize, load: u64) -> CpuSnapshot {
        CpuSnapshot {
            online: true,
            idle,
            nr_running: nr,
            load,
        }
    }

    #[test]
    fn wake_prefers_idle_prev_then_any_idle() {
        let cpus = [cpu(false, 3, 3000), cpu(true, 0, 0), cpu(true, 0, 50)];
        assert_eq!(select_cpu(1, !0, &cpus, 0), Some(1));
        assert_eq!(select_cpu(0, !0, &cpus, 0), Some(1), "least loaded idle");
        assert_eq!(select_cpu(0, 0b101, &cpus, 0), Some(2), "affinity");
    }

    #[test]
    fn wake_stays_cache_hot_when_nothing_idle() {
        let cpus = [cpu(false, 2, 2048), cpu(false, 1, 1024)];
        assert_eq!(select_cpu(0, !0, &cpus, 1_000_000), Some(0));
        assert_eq!(select_cpu(0, !0, &cpus, 50_000_000), Some(1));
    }

    #[test]
    fn wake_skips_offline_and_disallowed() {
        let mut cpus = [cpu(true, 0, 0), cpu(false, 1, 1024)];
        cpus[0].online = false;
        assert_eq!(select_cpu(0, !0, &cpus, 0), Some(1));
        assert_eq!(select_cpu(0, 0b1, &cpus, 0), None);
    }

    #[test]
    fn balance_needs_real_imbalance() {
        let cpus = [cpu(false, 1, 1024), cpu(false, 2, 1100)];
        assert_eq!(find_busiest(0, &cpus, false), None, "within 25%");
        let cpus = [cpu(false, 1, 1024), cpu(false, 4, 4096)];
        assert_eq!(find_busiest(0, &cpus, false), Some(1));
        assert_eq!(tasks_to_move(&cpus[1], &cpus[0]), 1);
        let cpus = [cpu(true, 0, 0), cpu(false, 2, 1100)];
        assert_eq!(find_busiest(0, &cpus, true), Some(1), "idle pulls");
        let cpus = [cpu(true, 0, 0), cpu(false, 1, 1024)];
        assert_eq!(find_busiest(0, &cpus, true), None, "nothing waiting");
    }
}
