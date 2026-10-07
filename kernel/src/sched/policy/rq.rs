//! One CPU's run queue: the deadline, real-time and fair classes in strict
//! priority order (ADR 0007), the running task, and its load average.
//!
//! The running task stays queued (and is charged by `update_curr`); a task
//! that blocks, exits or migrates is taken off with `dequeue`, which hands
//! back its [`Entity`] (class state, lag) for the next `enqueue`.

use alloc::collections::BTreeMap;

use super::{
    dl::{DlEntity, DlRq},
    fair::{self, FairEntity, FairRq},
    rt::{RtEntity, RtPolicy, RtRq, RR_QUANTUM_NS},
    TaskKey,
};

/// A task's scheduling policy (Linux `sched_setattr` semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// `SCHED_OTHER` with a nice value.
    Normal { nice: i8 },
    /// `SCHED_BATCH`: fair, never preempts on wakeup.
    Batch { nice: i8 },
    /// `SCHED_IDLE`: fair with the minimum weight.
    Idle,
    /// `SCHED_FIFO`, priority 1..=99.
    Fifo { prio: u8 },
    /// `SCHED_RR`, priority 1..=99.
    RoundRobin { prio: u8 },
    /// `SCHED_DEADLINE` (validated parameters).
    Deadline(DlEntity),
}

impl Default for Policy {
    fn default() -> Self {
        Policy::Normal { nice: 0 }
    }
}

/// Scheduling class, highest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Deadline = 0,
    RealTime = 1,
    Fair = 2,
}

/// A task's scheduling state while off any queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    pub policy: Policy,
    pub fair: FairEntity,
    /// Real-time priority boost from priority inheritance (SCHED-INC-01).
    pub boost: Option<u8>,
}

impl Entity {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            fair: FairEntity::new(weight_of(policy)),
            boost: None,
        }
    }

    /// The class this entity runs in, counting a priority-inheritance boost.
    pub fn class(&self) -> Class {
        match self.policy {
            Policy::Deadline(_) => Class::Deadline,
            Policy::Fifo { .. } | Policy::RoundRobin { .. } => Class::RealTime,
            _ if self.boost.is_some() => Class::RealTime,
            _ => Class::Fair,
        }
    }

    /// Effective real-time state (own priority or the boost, whichever is
    /// higher).
    fn rt(&self) -> RtEntity {
        let (own, policy) = match self.policy {
            Policy::Fifo { prio } => (prio, RtPolicy::Fifo),
            Policy::RoundRobin { prio } => (prio, RtPolicy::RoundRobin),
            _ => (0, RtPolicy::Fifo),
        };
        RtEntity::new(own.max(self.boost.unwrap_or(0)), policy)
    }
}

fn weight_of(policy: Policy) -> u64 {
    match policy {
        Policy::Normal { nice } | Policy::Batch { nice } => fair::weight_for_nice(nice as i32),
        Policy::Idle => fair::IDLE_WEIGHT,
        _ => fair::NICE_0_WEIGHT,
    }
}

#[derive(Debug, Clone, Copy)]
struct Meta {
    entity: Entity,
    /// Real-time state while queued in the RT class.
    rt: RtEntity,
}

/// Load-average decay per millisecond: y with y^32 = 1/2 (PELT half-life),
/// 1002/1024 ~= 0.97857, and 0.97857^32 ~= 0.4999.
const DECAY_NUM: u64 = 1002;
const DECAY_DEN: u64 = 1024;

/// One CPU's run queue.
#[derive(Debug, Default)]
pub struct RunQueue {
    dl: DlRq,
    rt: RtRq,
    fair: FairRq,
    meta: BTreeMap<TaskKey, Meta>,
    curr: Option<TaskKey>,
    exec_start: u64,
    /// Decaying average of runnable weight (same units as weights).
    load_avg: u64,
    load_stamp: u64,
}

impl RunQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn nr_running(&self) -> usize {
        self.meta.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    pub fn current(&self) -> Option<TaskKey> {
        self.curr
    }

    pub fn contains(&self, key: TaskKey) -> bool {
        self.meta.contains_key(&key)
    }

    pub fn class_of(&self, key: TaskKey) -> Option<Class> {
        self.meta.get(&key).map(|m| m.entity.class())
    }

    /// Instantaneous runnable weight (real-time and deadline tasks count as
    /// nice-0 tasks for balancing).
    pub fn load(&self) -> u64 {
        self.fair.load() + (self.rt.len() + self.dl.len()) as u64 * fair::NICE_0_WEIGHT
    }

    /// Decaying load average, updated to `now`.
    pub fn load_avg(&mut self, now: u64) -> u64 {
        self.update_load(now);
        self.load_avg
    }

    fn update_load(&mut self, now: u64) {
        let ms = now.saturating_sub(self.load_stamp) / 1_000_000;
        if ms == 0 {
            return;
        }
        self.load_stamp += ms * 1_000_000;
        let target = self.load();
        // After ~10 half-lives the old value no longer matters.
        let steps = ms.min(320);
        let mut avg = self.load_avg;
        for _ in 0..steps {
            avg = (avg * DECAY_NUM + target * (DECAY_DEN - DECAY_NUM)) / DECAY_DEN;
        }
        if ms > 320 || avg.abs_diff(target) <= 1 {
            avg = target;
        }
        self.load_avg = avg;
    }

    /// Add a runnable task. `now` drives the deadline class's CBS rule.
    pub fn enqueue(&mut self, key: TaskKey, entity: Entity, now: u64) {
        self.update_load(now);
        let rt = entity.rt();
        match entity.class() {
            Class::Deadline => {
                if let Policy::Deadline(d) = entity.policy {
                    self.dl.enqueue(key, d, now);
                }
            }
            Class::RealTime => self.rt.enqueue(key, rt.prio, false),
            Class::Fair => self.fair.enqueue(key, entity.fair),
        }
        self.meta.insert(key, Meta { entity, rt });
    }

    /// Remove a task (it blocked, exited or is migrating) and return its state.
    pub fn dequeue(&mut self, key: TaskKey, now: u64) -> Option<Entity> {
        if self.curr == Some(key) {
            self.update_curr(now);
            self.curr = None;
        }
        self.update_load(now);
        let meta = self.meta.remove(&key)?;
        let mut entity = meta.entity;
        match entity.class() {
            Class::Deadline => {
                if let Some(d) = self.dl.dequeue(key) {
                    entity.policy = Policy::Deadline(d);
                }
            }
            Class::RealTime => {
                self.rt.dequeue(key, meta.rt.prio);
            }
            Class::Fair => {
                if let Some(f) = self.fair.dequeue(key) {
                    entity.fair = f;
                }
            }
        }
        Some(entity)
    }

    /// Charge the running task up to `now`. Returns true at a reschedule
    /// point (slice or quantum used, runtime exhausted, bandwidth throttled).
    pub fn update_curr(&mut self, now: u64) -> bool {
        let Some(key) = self.curr else {
            self.exec_start = now;
            return false;
        };
        let delta = now.saturating_sub(self.exec_start);
        self.exec_start = now;
        if delta == 0 {
            return false;
        }
        let Some(meta) = self.meta.get_mut(&key) else {
            return false;
        };
        match meta.entity.class() {
            Class::Deadline => self.dl.charge(key, delta),
            Class::RealTime => {
                let throttled = self.rt.account(now, delta);
                let mut expired = false;
                if meta.rt.policy == RtPolicy::RoundRobin {
                    meta.rt.rr_left = meta.rt.rr_left.saturating_sub(delta);
                    if meta.rt.rr_left == 0 {
                        meta.rt.rr_left = RR_QUANTUM_NS;
                        self.rt.rotate(meta.rt.prio);
                        expired = true;
                    }
                }
                throttled || expired
            }
            Class::Fair => self.fair.charge(key, delta),
        }
    }

    /// The task that should run now, without changing anything.
    pub fn peek(&self) -> Option<TaskKey> {
        self.dl
            .pick()
            .or_else(|| self.rt.pick())
            .or_else(|| self.fair.pick())
        // A throttled real-time class still has tasks: they wait, and
        // with nothing else runnable the CPU idles (Linux does the same).
    }

    /// Choose and install the task to run (`None`: run the idle task).
    pub fn pick_next(&mut self, now: u64) -> Option<TaskKey> {
        self.update_curr(now);
        self.dl.replenish_due(now);
        self.rt.roll_period(now);
        let next = self.peek();
        if next != self.curr {
            self.exec_start = now;
        }
        self.curr = next;
        next
    }

    /// Periodic tick. Returns true when the running task should be switched
    /// out (the dispatcher then calls `pick_next`).
    pub fn tick(&mut self, now: u64) -> bool {
        let boundary = self.update_curr(now);
        let replenished = self.dl.replenish_due(now);
        self.rt.roll_period(now);
        self.update_load(now);
        let want = self.peek();
        if want == self.curr {
            return false;
        }
        // A higher class appeared (replenished deadline task, unthrottled
        // real-time class) or the running task reached a boundary.
        let higher = match (
            want.and_then(|k| self.class_of(k)),
            self.curr.and_then(|k| self.class_of(k)),
        ) {
            (Some(w), Some(c)) => w < c,
            (Some(_), None) => true,
            _ => false,
        };
        boundary || replenished || higher || self.curr.is_none()
    }

    /// Whether a task just woken onto this queue should preempt the running
    /// one.
    pub fn should_preempt(&mut self, woken: TaskKey, now: u64) -> bool {
        let Some(curr) = self.curr else {
            return true;
        };
        if curr == woken {
            return false;
        }
        self.update_curr(now);
        let (Some(c), Some(w)) = (self.meta.get(&curr), self.meta.get(&woken)) else {
            return false;
        };
        let (cc, wc) = (c.entity.class(), w.entity.class());
        if wc != cc {
            return wc < cc;
        }
        match wc {
            Class::Deadline => self.dl.should_preempt(curr, woken),
            Class::RealTime => w.rt.prio > c.rt.prio,
            Class::Fair => {
                !matches!(w.entity.policy, Policy::Batch { .. } | Policy::Idle)
                    && self.fair.should_preempt(curr, woken)
            }
        }
    }

    /// The running task gives up the CPU (sched_yield).
    pub fn yield_current(&mut self, now: u64) {
        let Some(key) = self.curr else { return };
        self.update_curr(now);
        let Some(meta) = self.meta.get(&key).copied() else {
            return;
        };
        match meta.entity.class() {
            Class::RealTime => self.rt.rotate(meta.rt.prio),
            Class::Fair => {
                // Forfeit the rest of the slice (Linux yield_task_fair).
                if let Some(e) = self.fair.get(key).copied() {
                    let left = e.deadline.saturating_sub(e.vruntime);
                    let w = e.weight;
                    self.fair.charge(
                        key,
                        (left as u128 * w as u128 / fair::NICE_0_WEIGHT as u128) as u64 + 1,
                    );
                }
            }
            Class::Deadline => {
                // Give up the rest of this reservation (Linux yield_task_dl).
                let left = self.dl.get(key).map_or(0, |d| d.remaining.max(0) as u64);
                self.dl.charge(key, left.max(1));
            }
        }
    }

    /// Change a queued task's policy, keeping its fair lag.
    pub fn set_policy(&mut self, key: TaskKey, policy: Policy, now: u64) -> bool {
        let Some(mut e) = self.dequeue(key, now) else {
            return false;
        };
        let was_curr = self.curr.is_none();
        e.fair.weight = weight_of(policy);
        e.policy = policy;
        self.enqueue(key, e, now);
        let _ = was_curr;
        true
    }

    /// Set or clear a priority-inheritance boost on a queued task.
    pub fn set_boost(&mut self, key: TaskKey, boost: Option<u8>, now: u64) -> bool {
        let curr = self.curr == Some(key);
        let Some(mut e) = self.dequeue(key, now) else {
            return false;
        };
        e.boost = boost;
        self.enqueue(key, e, now);
        if curr {
            self.curr = Some(key);
            self.exec_start = now;
        }
        true
    }

    /// Queued tasks that may be migrated: not running, fair or real-time
    /// (deadline tasks stay where admission placed them).
    pub fn migratable(&self) -> impl Iterator<Item = TaskKey> + '_ {
        let curr = self.curr;
        self.fair
            .keys()
            .chain(self.rt.keys())
            .filter(move |&k| Some(k) != curr)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    const MS: u64 = 1_000_000;

    fn normal() -> Entity {
        Entity::new(Policy::default())
    }

    #[test]
    fn classes_in_priority_order() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, Entity::new(Policy::Fifo { prio: 10 }), 0);
        let d = DlEntity::new(5 * MS, 50 * MS, 100 * MS).unwrap();
        rq.enqueue(3, Entity::new(Policy::Deadline(d)), 0);
        assert_eq!(rq.pick_next(0), Some(3));
        rq.dequeue(3, MS);
        assert_eq!(rq.pick_next(MS), Some(2));
        rq.dequeue(2, 2 * MS);
        assert_eq!(rq.pick_next(2 * MS), Some(1));
        rq.dequeue(1, 3 * MS);
        assert_eq!(rq.pick_next(3 * MS), None);
    }

    #[test]
    fn fair_tasks_alternate_by_slice_under_ticks() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, normal(), 0);
        let mut now = 0;
        let mut curr = rq.pick_next(now);
        let mut runs: BTreeMap<TaskKey, u64> = BTreeMap::new();
        for _ in 0..1000 {
            now += MS;
            *runs.entry(curr.unwrap()).or_insert(0) += 1;
            if rq.tick(now) {
                curr = rq.pick_next(now);
            }
        }
        assert!((450..=550).contains(&runs[&1]), "{runs:?}");
    }

    #[test]
    fn round_robin_rotates_after_quantum_fifo_does_not() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, Entity::new(Policy::RoundRobin { prio: 5 }), 0);
        rq.enqueue(2, Entity::new(Policy::RoundRobin { prio: 5 }), 0);
        let mut now = 0;
        assert_eq!(rq.pick_next(now), Some(1));
        let mut switched_at = None;
        for t in 1..=150 {
            now = t * MS;
            if rq.tick(now) {
                rq.pick_next(now);
                switched_at.get_or_insert(t);
            }
        }
        assert_eq!(switched_at, Some(100));
        assert_eq!(rq.current(), Some(2));

        let mut f = RunQueue::new();
        f.enqueue(1, Entity::new(Policy::Fifo { prio: 5 }), 0);
        f.enqueue(2, Entity::new(Policy::Fifo { prio: 5 }), 0);
        f.pick_next(0);
        for t in 1..=300 {
            assert!(!f.tick(t * MS), "FIFO runs until it blocks");
        }
    }

    #[test]
    fn realtime_wakeup_preempts_fair_and_lower_rt() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.pick_next(0);
        rq.enqueue(2, Entity::new(Policy::Fifo { prio: 10 }), MS);
        assert!(rq.should_preempt(2, MS));
        rq.pick_next(MS);
        rq.enqueue(3, Entity::new(Policy::Fifo { prio: 5 }), 2 * MS);
        assert!(!rq.should_preempt(3, 2 * MS));
        rq.enqueue(4, Entity::new(Policy::Fifo { prio: 50 }), 2 * MS);
        assert!(rq.should_preempt(4, 2 * MS));
    }

    #[test]
    fn batch_and_idle_never_preempt_on_wakeup() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.pick_next(0);
        for _ in 0..5 {
            rq.update_curr(10 * MS);
        }
        rq.enqueue(2, Entity::new(Policy::Batch { nice: 0 }), 10 * MS);
        assert!(!rq.should_preempt(2, 10 * MS));
        rq.enqueue(3, Entity::new(Policy::Idle), 10 * MS);
        assert!(!rq.should_preempt(3, 10 * MS));
    }

    #[test]
    fn sched_idle_gets_tiny_share() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, Entity::new(Policy::Idle), 0);
        let mut now = 0;
        let mut curr = rq.pick_next(now);
        let mut idle_ms = 0;
        for _ in 0..10_000 {
            now += MS;
            if curr == Some(2) {
                idle_ms += 1;
            }
            if rq.tick(now) {
                curr = rq.pick_next(now);
            }
        }
        // 3 / (1024 + 3) = 0.3%; allow slice granularity.
        assert!(idle_ms <= 150, "SCHED_IDLE ran {idle_ms} ms of 10000");
    }

    #[test]
    fn deadline_task_is_throttled_and_fair_runs_meanwhile() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        let d = DlEntity::new(10 * MS, 100 * MS, 100 * MS).unwrap();
        rq.enqueue(2, Entity::new(Policy::Deadline(d)), 0);
        let mut now = 0;
        let mut curr = rq.pick_next(now);
        let mut dl_ms = 0;
        for _ in 0..1000 {
            now += MS;
            if curr == Some(2) {
                dl_ms += 1;
            }
            if rq.tick(now) {
                curr = rq.pick_next(now);
            }
        }
        assert!(
            (95..=110).contains(&dl_ms),
            "10% reservation, ran {dl_ms} ms"
        );
    }

    #[test]
    fn realtime_bandwidth_leaves_room_for_fair() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, Entity::new(Policy::Fifo { prio: 99 }), 0);
        let mut now = 0;
        let mut curr = rq.pick_next(now);
        let mut fair_ms = 0;
        for _ in 0..2000 {
            now += MS;
            if curr == Some(1) {
                fair_ms += 1;
            }
            if rq.tick(now) {
                curr = rq.pick_next(now);
            }
        }
        assert!(
            (80..=120).contains(&fair_ms),
            "fair got {fair_ms} ms of 2000"
        );
    }

    #[test]
    fn priority_inheritance_boost_lifts_fair_task() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, Entity::new(Policy::Fifo { prio: 20 }), 0);
        rq.set_boost(1, Some(30), 0);
        assert_eq!(rq.pick_next(0), Some(1));
        rq.set_boost(1, None, MS);
        assert_eq!(rq.pick_next(MS), Some(2));
    }

    #[test]
    fn dequeue_returns_state_for_migration() {
        let mut a = RunQueue::new();
        a.enqueue(1, Entity::new(Policy::Normal { nice: 5 }), 0);
        a.enqueue(2, normal(), 0);
        a.pick_next(0);
        let e = a.dequeue(1, MS).unwrap();
        assert_eq!(e.fair.weight, fair::weight_for_nice(5));
        let mut b = RunQueue::new();
        b.enqueue(1, e, MS);
        assert_eq!(b.pick_next(MS), Some(1));
        assert_eq!(a.nr_running(), 1);
    }

    #[test]
    fn migratable_excludes_running_and_deadline() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        rq.enqueue(2, normal(), 0);
        let d = DlEntity::new(5 * MS, 50 * MS, 100 * MS).unwrap();
        rq.enqueue(3, Entity::new(Policy::Deadline(d)), 0);
        rq.enqueue(4, Entity::new(Policy::Fifo { prio: 1 }), 0);
        let running = rq.pick_next(0);
        assert_eq!(running, Some(3));
        let m: Vec<_> = rq.migratable().collect();
        assert_eq!(m, [1, 2, 4]);
    }

    #[test]
    fn load_average_converges_with_half_life() {
        let mut rq = RunQueue::new();
        rq.enqueue(1, normal(), 0);
        let after_32 = rq.load_avg(32 * MS);
        assert!(
            (480..=540).contains(&after_32),
            "half of 1024 after 32 ms: {after_32}"
        );
        let later = rq.load_avg(400 * MS);
        assert_eq!(later, 1024);
        rq.dequeue(1, 400 * MS);
        let decayed = rq.load_avg(432 * MS);
        assert!((480..=540).contains(&decayed), "{decayed}");
    }
}
