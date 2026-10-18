//! EEVDF fair class (ADR 0007).
//!
//! Earliest Eligible Virtual Deadline First, as in Linux since 6.6:
//!
//! - Each task accrues `vruntime += delta_exec * NICE_0_WEIGHT / weight`.
//! - `avg_vruntime` is the weight-averaged vruntime of every task on the queue,
//!   the running one included. A task is *eligible* when its vruntime is at or
//!   behind it, i.e. it has received no more than its fair share.
//! - Among eligible tasks the one with the earliest virtual deadline runs,
//!   `deadline = vruntime + slice * NICE_0_WEIGHT / weight`, so a shorter slice
//!   means lower latency without changing the long-run share.
//! - Lag (`avg - vruntime`, the service a task is owed) survives sleep and
//!   migration, clamped so that sleeping cannot bank unbounded credit.
//!
//! The running task stays in the deadline index; only a change of deadline
//! (slice used up, reweight) moves it.

use alloc::collections::{BTreeMap, BTreeSet};

use super::TaskKey;

/// Weight of a nice-0 task.
pub const NICE_0_WEIGHT: u64 = 1024;
/// Weight of a `SCHED_IDLE` task (Linux `WEIGHT_IDLEPRIO`).
pub const IDLE_WEIGHT: u64 = 3;
/// Base time slice (Linux `sysctl_sched_base_slice`, 3 ms).
pub const BASE_SLICE_NS: u64 = 3_000_000;
/// Smallest settable slice (0.1 ms) and largest (100 ms), as Linux.
pub const MIN_SLICE_NS: u64 = 100_000;
pub const MAX_SLICE_NS: u64 = 100_000_000;

/// Linux `sched_prio_to_weight`: nice -20..=19. Each step is ~1.25x, so one
/// nice level is ~10% of CPU against a nice-0 competitor.
pub const NICE_TO_WEIGHT: [u64; 40] = [
    88761, 71755, 56483, 46273, 36291, // -20
    29154, 23254, 18705, 14949, 11916, // -15
    9548, 7620, 6100, 4904, 3906, // -10
    3121, 2501, 1991, 1586, 1277, // -5
    1024, 820, 655, 526, 423, // 0
    335, 272, 215, 172, 137, // 5
    110, 87, 70, 56, 45, // 10
    36, 29, 23, 18, 15, // 15
];

/// The weight for a nice value (clamped to -20..=19).
pub fn weight_for_nice(nice: i32) -> u64 {
    NICE_TO_WEIGHT[(nice.clamp(-20, 19) + 20) as usize]
}

/// Per-task fair state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairEntity {
    pub weight: u64,
    /// Requested slice in ns (wall time at nice 0).
    pub slice: u64,
    pub vruntime: u64,
    pub deadline: u64,
    /// Lag saved while off a queue (virtual ns; positive = owed service).
    pub vlag: i64,
    /// Not yet placed on any queue: gets half a slice of deadline.
    pub fresh: bool,
}

impl FairEntity {
    pub fn new(weight: u64) -> Self {
        Self {
            weight: weight.max(1),
            slice: BASE_SLICE_NS,
            vruntime: 0,
            deadline: 0,
            vlag: 0,
            fresh: true,
        }
    }

    /// The slice in virtual time.
    pub fn vslice(&self) -> u64 {
        scale(self.slice, NICE_0_WEIGHT, self.weight)
    }
}

/// `v * mul / div` without overflow (saturating).
fn scale(v: u64, mul: u64, div: u64) -> u64 {
    let r = (v as u128) * (mul as u128) / (div.max(1) as u128);
    r.min(u64::MAX as u128) as u64
}

/// One CPU's fair queue.
#[derive(Debug, Default)]
pub struct FairRq {
    tasks: BTreeMap<TaskKey, FairEntity>,
    by_deadline: BTreeSet<(u64, TaskKey)>,
    /// Sum of weights on the queue.
    sum_w: u64,
    /// Sum of `weight * vruntime` (128-bit; vruntimes are 64-bit).
    sum_wv: u128,
    /// Largest average reached: a floor for placing tasks on an empty queue.
    floor: u64,
}

impl FairRq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Total weight queued (the run queue's instantaneous load).
    pub fn load(&self) -> u64 {
        self.sum_w
    }

    pub fn get(&self, key: TaskKey) -> Option<&FairEntity> {
        self.tasks.get(&key)
    }

    pub fn contains(&self, key: TaskKey) -> bool {
        self.tasks.contains_key(&key)
    }

    /// The weight-averaged vruntime (the queue's virtual time).
    pub fn avg_vruntime(&self) -> u64 {
        if self.sum_w == 0 {
            self.floor
        } else {
            (self.sum_wv / self.sum_w as u128) as u64
        }
    }

    /// `vruntime <= avg`, exactly (no division rounding).
    fn eligible_v(&self, vruntime: u64) -> bool {
        self.sum_w == 0 || (vruntime as u128) * (self.sum_w as u128) <= self.sum_wv
    }

    pub fn is_eligible(&self, key: TaskKey) -> bool {
        self.tasks
            .get(&key)
            .is_some_and(|e| self.eligible_v(e.vruntime))
    }

    /// Put a task on the queue, placing it relative to the queue's virtual
    /// time with its saved lag.
    pub fn enqueue(&mut self, key: TaskKey, mut e: FairEntity) {
        debug_assert!(!self.tasks.contains_key(&key));
        let avg = self.avg_vruntime() as i128;
        // Linux place_entity: adding a task with lag L moves the average by
        // L*w/(W+w); pre-scaling the lag by (W+w)/W keeps it exactly L.
        let mut lag = e.vlag as i128;
        if self.sum_w > 0 {
            lag = lag * (self.sum_w as i128 + e.weight as i128) / self.sum_w as i128;
        }
        let v = (avg - lag).max(0);
        e.vruntime = v as u64;
        let vslice = e.vslice();
        e.deadline = e
            .vruntime
            .saturating_add(if e.fresh { vslice / 2 } else { vslice });
        e.fresh = false;
        e.vlag = 0;
        self.insert(key, e);
    }

    /// Take a task off the queue, saving its lag (clamped to two slices) for
    /// when it returns, here or on another CPU.
    pub fn dequeue(&mut self, key: TaskKey) -> Option<FairEntity> {
        let avg = self.avg_vruntime() as i128;
        let mut e = self.remove(key)?;
        let limit = (2 * e.vslice()).max(1_000_000) as i128;
        e.vlag = (avg - e.vruntime as i128).clamp(-limit, limit) as i64;
        if self.sum_w == 0 {
            self.floor = self.floor.max(avg as u64);
        }
        Some(e)
    }

    fn insert(&mut self, key: TaskKey, e: FairEntity) {
        self.sum_w += e.weight;
        self.sum_wv += e.weight as u128 * e.vruntime as u128;
        self.by_deadline.insert((e.deadline, key));
        self.tasks.insert(key, e);
    }

    fn remove(&mut self, key: TaskKey) -> Option<FairEntity> {
        let e = self.tasks.remove(&key)?;
        self.by_deadline.remove(&(e.deadline, key));
        self.sum_w -= e.weight;
        self.sum_wv -= e.weight as u128 * e.vruntime as u128;
        Some(e)
    }

    /// Charge `delta_ns` of CPU time to a queued (running) task. Returns
    /// true when it has used its slice (a reschedule point).
    pub fn charge(&mut self, key: TaskKey, delta_ns: u64) -> bool {
        let Some(mut e) = self.remove(key) else {
            return false;
        };
        e.vruntime = e
            .vruntime
            .saturating_add(scale(delta_ns, NICE_0_WEIGHT, e.weight));
        let expired = e.vruntime >= e.deadline;
        if expired {
            e.deadline = e.vruntime.saturating_add(e.vslice());
        }
        self.insert(key, e);
        expired
    }

    /// The task to run: the earliest deadline among eligible tasks.
    pub fn pick(&self) -> Option<TaskKey> {
        // At least one task is always eligible (not all can be above the
        // average), so this finds one; the fallback guards rounding only.
        self.by_deadline
            .iter()
            .find(|(_, k)| self.eligible_v(self.tasks[k].vruntime))
            .or_else(|| self.by_deadline.iter().next())
            .map(|&(_, k)| k)
    }

    /// Whether `woken` should preempt `curr` (both queued): it is eligible
    /// and its deadline is earlier.
    pub fn should_preempt(&self, curr: TaskKey, woken: TaskKey) -> bool {
        match (self.tasks.get(&curr), self.tasks.get(&woken)) {
            (Some(c), Some(w)) => self.eligible_v(w.vruntime) && w.deadline < c.deadline,
            (None, Some(_)) => true,
            _ => false,
        }
    }

    /// Change a queued task's weight keeping its lag (Linux reweight_entity).
    pub fn reweight(&mut self, key: TaskKey, weight: u64) {
        if let Some(mut e) = self.dequeue(key) {
            let old = e.weight;
            e.weight = weight.max(1);
            e.vlag = (e.vlag as i128 * old as i128 / e.weight as i128) as i64;
            self.enqueue(key, e);
        }
    }

    /// Change a queued task's slice (sched_setattr `sched_runtime`).
    pub fn set_slice(&mut self, key: TaskKey, slice_ns: u64) {
        if let Some(mut e) = self.remove(key) {
            e.slice = slice_ns.clamp(MIN_SLICE_NS, MAX_SLICE_NS);
            e.deadline = e.vruntime.saturating_add(e.vslice());
            self.insert(key, e);
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = TaskKey> + '_ {
        self.tasks.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    /// Run `rq` for `ticks` ticks of `tick_ns`, always running `pick()`.
    fn simulate(rq: &mut FairRq, ticks: usize, tick_ns: u64) -> BTreeMap<TaskKey, u64> {
        let mut ran = BTreeMap::new();
        let mut curr = rq.pick();
        for _ in 0..ticks {
            let k = curr.unwrap();
            *ran.entry(k).or_insert(0) += tick_ns;
            if rq.charge(k, tick_ns) || rq.pick() != Some(k) {
                curr = rq.pick();
            }
        }
        ran
    }

    #[test]
    fn weights_match_linux_table() {
        assert_eq!(weight_for_nice(0), 1024);
        assert_eq!(weight_for_nice(-20), 88761);
        assert_eq!(weight_for_nice(19), 15);
        assert_eq!(weight_for_nice(100), 15);
    }

    #[test]
    fn equal_weights_share_equally() {
        let mut rq = FairRq::new();
        for k in 1..=4 {
            rq.enqueue(k, FairEntity::new(1024));
        }
        let ran = simulate(&mut rq, 4000, 1_000_000);
        for k in 1..=4 {
            let share = ran[&k];
            assert!(
                (950_000_000..=1_050_000_000).contains(&share),
                "task {k} got {share}"
            );
        }
    }

    #[test]
    fn share_follows_weight() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(weight_for_nice(0))); // 1024
        rq.enqueue(2, FairEntity::new(weight_for_nice(5))); // 335
        let ran = simulate(&mut rq, 10_000, 1_000_000);
        // Expected 1024:335 = 75.35% : 24.65%.
        let a = ran[&1] as u128 * 10_000 / (ran[&1] + ran[&2]) as u128;
        assert!((7400..=7650).contains(&a), "nice-0 share {a} per 10k");
    }

    #[test]
    fn running_task_keeps_cpu_for_its_slice() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(1024));
        rq.enqueue(2, FairEntity::new(1024));
        let first = rq.pick().unwrap();
        // 1 ms ticks: the first pick runs until its (half, fresh) slice ends,
        // and the switch happens only at a slice boundary.
        let mut switches = Vec::new();
        let mut curr = first;
        for t in 0..30 {
            if rq.charge(curr, 1_000_000) {
                let next = rq.pick().unwrap();
                if next != curr {
                    switches.push(t);
                    curr = next;
                }
            }
        }
        assert!(switches.len() >= 8, "alternates by slice: {switches:?}");
        for w in switches.windows(2) {
            assert!(w[1] - w[0] >= 1, "no switch more often than a tick");
        }
    }

    #[test]
    fn short_slice_task_gets_lower_latency_same_share() {
        let mut rq = FairRq::new();
        let mut lat = FairEntity::new(1024);
        lat.slice = MIN_SLICE_NS * 10; // 1 ms
        rq.enqueue(1, lat);
        rq.enqueue(2, FairEntity::new(1024)); // 3 ms
        let ran = simulate(&mut rq, 6000, 500_000);
        let a = ran[&1] as u128 * 100 / (ran[&1] + ran[&2]) as u128;
        assert!((45..=55).contains(&a), "same long-run share, got {a}%");
    }

    #[test]
    fn sleeper_keeps_bounded_lag() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(1024));
        rq.enqueue(2, FairEntity::new(1024));
        // Task 2 sleeps for a long time while 1 runs.
        let e2 = rq.dequeue(2).unwrap();
        for _ in 0..1000 {
            rq.charge(1, 1_000_000);
        }
        rq.enqueue(2, e2);
        // On return it is not owed a second of CPU: within a few slices the
        // running task is scheduled again.
        let ran = simulate(&mut rq, 40, 1_000_000);
        assert!(ran.get(&1).copied().unwrap_or(0) >= 10_000_000, "{ran:?}");
    }

    #[test]
    fn lag_is_preserved_across_migration() {
        let mut a = FairRq::new();
        for k in 1..=3 {
            a.enqueue(k, FairEntity::new(1024));
        }
        a.charge(1, 2_000_000);
        let avg = a.avg_vruntime() as i128;
        let lag_before = avg - a.get(1).unwrap().vruntime as i128;
        let e = a.dequeue(1).unwrap();
        let mut b = FairRq::new();
        b.enqueue(7, FairEntity::new(1024));
        b.charge(7, 50_000_000);
        b.enqueue(1, e);
        let lag_after = b.avg_vruntime() as i128 - b.get(1).unwrap().vruntime as i128;
        assert!(
            (lag_after - lag_before).abs() <= 2,
            "{lag_before} vs {lag_after}"
        );
    }

    #[test]
    fn wakeup_preempts_only_with_earlier_eligible_deadline() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(1024));
        rq.charge(1, 2_900_000);
        let mut e = FairEntity::new(1024);
        e.fresh = false;
        rq.enqueue(2, e); // zero lag: at the average, full slice
        assert!(rq.is_eligible(2));
        let d1 = rq.get(1).unwrap().deadline;
        let d2 = rq.get(2).unwrap().deadline;
        assert_eq!(rq.should_preempt(1, 2), d2 < d1);
    }

    #[test]
    fn reweight_keeps_lag_sign() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(1024));
        rq.enqueue(2, FairEntity::new(1024));
        rq.charge(1, 5_000_000); // 1 is ahead (negative lag)
        rq.reweight(1, 2048);
        assert!(!rq.is_eligible(1));
        assert!(rq.is_eligible(2));
        assert_eq!(rq.load(), 3072);
    }

    #[test]
    fn empty_queue_floor_does_not_regress() {
        let mut rq = FairRq::new();
        rq.enqueue(1, FairEntity::new(1024));
        rq.charge(1, 10_000_000);
        let e = rq.dequeue(1).unwrap();
        let floor = rq.avg_vruntime();
        assert!(floor > 0);
        rq.enqueue(1, e);
        assert!(rq.get(1).unwrap().vruntime >= floor - 1);
    }
}
