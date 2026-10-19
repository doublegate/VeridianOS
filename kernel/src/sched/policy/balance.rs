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

/// A deadline field with no deadline task behind it.
pub const NO_DL: u64 = u64::MAX;

/// Per-CPU state the decisions read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuSnapshot {
    pub online: bool,
    /// Nothing runnable (running the idle task).
    pub idle: bool,
    pub nr_running: usize,
    /// Decaying load average.
    pub load: u64,
    /// Earliest absolute deadline among the CPU's runnable deadline tasks,
    /// running or waiting ([`NO_DL`] with none).
    pub dl_earliest: u64,
    /// Earliest absolute deadline among its deadline tasks that wait (not
    /// running, not throttled) ([`NO_DL`] with none).
    pub dl_next: u64,
}

impl Default for CpuSnapshot {
    fn default() -> Self {
        Self {
            online: false,
            idle: false,
            nr_running: 0,
            load: 0,
            dl_earliest: NO_DL,
            dl_next: NO_DL,
        }
    }
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

/// Where a waking deadline task whose absolute deadline is `deadline`
/// should run (Linux `find_later_rq`): a CPU with no deadline task, by the
/// [`select_cpu`] rules among those; else the CPU whose earliest deadline
/// is the latest, if that is later than `deadline` (the task preempts
/// there); else the previous CPU. `None` only if no allowed CPU is online.
pub fn select_cpu_dl(
    prev: usize,
    affinity: u64,
    cpus: &[CpuSnapshot],
    since_ran_ns: u64,
    deadline: u64,
) -> Option<usize> {
    let ok = |c: usize| allowed(affinity, c) && cpus.get(c).is_some_and(|s| s.online);
    let mut free = [CpuSnapshot::default(); 64];
    let mut any_free = false;
    for (c, s) in cpus.iter().enumerate().take(64) {
        if ok(c) && s.dl_earliest == NO_DL {
            free[c] = *s;
            any_free = true;
        }
    }
    if any_free {
        return select_cpu(prev, affinity, &free[..cpus.len().min(64)], since_ran_ns);
    }
    let later = (0..cpus.len())
        .filter(|&c| ok(c) && cpus[c].dl_earliest > deadline)
        .max_by_key(|&c| (cpus[c].dl_earliest, c == prev));
    later.or_else(|| {
        if ok(prev) {
            Some(prev)
        } else {
            (0..cpus.len()).find(|&c| ok(c))
        }
    })
}

/// A CPU `this` should pull a waiting deadline task from (Linux
/// `pull_dl_task`): the one whose earliest waiting deadline is the earliest,
/// provided it is earlier than everything `this` runs, so the task would
/// preempt here rather than keep waiting there.
pub fn find_dl_source(this: usize, cpus: &[CpuSnapshot]) -> Option<usize> {
    let mine = cpus.get(this)?.dl_earliest;
    (0..cpus.len())
        .filter(|&c| c != this && cpus[c].online && cpus[c].dl_next < mine)
        .min_by_key(|&c| cpus[c].dl_next)
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
            ..CpuSnapshot::default()
        }
    }

    /// A busy CPU running a deadline task due at `earliest`, with another
    /// waiting due at `next`.
    fn dl_cpu(earliest: u64, next: u64) -> CpuSnapshot {
        CpuSnapshot {
            dl_earliest: earliest,
            dl_next: next,
            ..cpu(false, if next == NO_DL { 1 } else { 2 }, 2048)
        }
    }

    #[test]
    fn deadline_wake_prefers_cpu_without_deadline_work() {
        let cpus = [dl_cpu(100, NO_DL), cpu(false, 3, 3000), dl_cpu(50, NO_DL)];
        assert_eq!(
            select_cpu_dl(0, !0, &cpus, 0, 500),
            Some(1),
            "no deadline task"
        );
        let cpus = [dl_cpu(100, NO_DL), cpu(true, 0, 0), cpu(false, 1, 1024)];
        assert_eq!(select_cpu_dl(0, !0, &cpus, 0, 500), Some(1), "idle first");
        assert_eq!(
            select_cpu_dl(2, !0, &cpus, 0, 500),
            Some(1),
            "idle over busy prev"
        );
        assert_eq!(select_cpu_dl(0, 0b101, &cpus, 0, 500), Some(2), "affinity");
    }

    #[test]
    fn deadline_wake_goes_where_it_preempts() {
        // Every CPU runs a deadline task: the latest one, if later than ours.
        let cpus = [dl_cpu(100, NO_DL), dl_cpu(900, NO_DL), dl_cpu(400, NO_DL)];
        assert_eq!(select_cpu_dl(0, !0, &cpus, 0, 500), Some(1));
        assert_eq!(select_cpu_dl(0, 0b101, &cpus, 0, 300), Some(2));
        // Nowhere would it run first: stay on the previous CPU.
        assert_eq!(select_cpu_dl(2, 0b101, &cpus, 0, 50), Some(2));
        assert_eq!(select_cpu_dl(0, 0, &cpus, 0, 50), None);
    }

    #[test]
    fn deadline_pull_only_when_it_would_run_sooner() {
        // CPU 1 has a task waiting (due 200) behind one due at 100.
        let cpus = [cpu(true, 0, 0), dl_cpu(100, 200), dl_cpu(300, NO_DL)];
        assert_eq!(find_dl_source(0, &cpus), Some(1), "idle CPU pulls");
        assert_eq!(find_dl_source(2, &cpus), Some(1), "300 is later than 200");
        let cpus = [dl_cpu(150, NO_DL), dl_cpu(100, 200)];
        assert_eq!(find_dl_source(0, &cpus), None, "150 runs before 200");
        assert_eq!(find_dl_source(1, &cpus), None, "nothing waits elsewhere");
        // Two sources: the earliest waiting deadline wins.
        let cpus = [cpu(false, 1, 1024), dl_cpu(10, 400), dl_cpu(10, 250)];
        assert_eq!(find_dl_source(0, &cpus), Some(2));
        let mut off = cpus;
        off[2].online = false;
        assert_eq!(find_dl_source(0, &off), Some(1), "offline CPUs are skipped");
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
