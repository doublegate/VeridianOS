//! Real-time class: `SCHED_FIFO` and `SCHED_RR` (ADR 0007).
//!
//! 100 priority levels, one FIFO per level and a 128-bit bitmap of the
//! non-empty levels, so pick, enqueue and dequeue are O(1) (dequeue of an
//! arbitrary task is O(tasks at its level)). Level 99 is the highest, as in
//! Linux's `sched_priority`.
//!
//! Real-time bandwidth is limited per CPU (950 ms per 1 s by default, Linux's
//! `sched_rt_runtime_us`): once used up, the class is throttled until the
//! period ends, so a runaway real-time task cannot lock out everything else.

use alloc::collections::VecDeque;

use super::TaskKey;

/// Number of real-time priority levels (1..=99 usable, as Linux).
pub const RT_LEVELS: usize = 100;
/// `SCHED_RR` quantum (Linux `sched_rr_timeslice_ms`, 100 ms).
pub const RR_QUANTUM_NS: u64 = 100_000_000;
/// Real-time bandwidth: runtime per period.
pub const RT_RUNTIME_NS: u64 = 950_000_000;
pub const RT_PERIOD_NS: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtPolicy {
    Fifo,
    RoundRobin,
}

/// Per-task real-time state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtEntity {
    /// 1..=99, higher runs first.
    pub prio: u8,
    pub policy: RtPolicy,
    /// RR time left in the current quantum.
    pub rr_left: u64,
}

impl RtEntity {
    pub fn new(prio: u8, policy: RtPolicy) -> Self {
        Self {
            prio: prio.clamp(1, (RT_LEVELS - 1) as u8),
            policy,
            rr_left: RR_QUANTUM_NS,
        }
    }
}

/// One CPU's real-time queue.
#[derive(Debug)]
pub struct RtRq {
    levels: [VecDeque<TaskKey>; RT_LEVELS],
    bitmap: u128,
    count: usize,
    /// Real-time time used in the current bandwidth period.
    period_used: u64,
    period_start: u64,
    throttled: bool,
}

impl Default for RtRq {
    fn default() -> Self {
        Self::new()
    }
}

impl RtRq {
    pub fn new() -> Self {
        Self {
            levels: [const { VecDeque::new() }; RT_LEVELS],
            bitmap: 0,
            count: 0,
            period_used: 0,
            period_start: 0,
            throttled: false,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_throttled(&self) -> bool {
        self.throttled
    }

    /// Add at the tail of its level (or the head: a preempted FIFO task keeps
    /// its place, as POSIX requires).
    pub fn enqueue(&mut self, key: TaskKey, prio: u8, at_head: bool) {
        let l = prio as usize % RT_LEVELS;
        if at_head {
            self.levels[l].push_front(key);
        } else {
            self.levels[l].push_back(key);
        }
        self.bitmap |= 1u128 << l;
        self.count += 1;
    }

    pub fn dequeue(&mut self, key: TaskKey, prio: u8) -> bool {
        let l = prio as usize % RT_LEVELS;
        let q = &mut self.levels[l];
        let Some(i) = q.iter().position(|&k| k == key) else {
            return false;
        };
        q.remove(i);
        if q.is_empty() {
            self.bitmap &= !(1u128 << l);
        }
        self.count -= 1;
        true
    }

    /// Highest priority waiting level, if any.
    pub fn top_prio(&self) -> Option<u8> {
        (self.bitmap != 0).then(|| (127 - self.bitmap.leading_zeros()) as u8)
    }

    /// The task to run, unless the class is throttled.
    pub fn pick(&self) -> Option<TaskKey> {
        if self.throttled {
            return None;
        }
        let l = self.top_prio()? as usize;
        self.levels[l].front().copied()
    }

    /// Move the head of a level to its tail (RR quantum expiry, sched_yield).
    pub fn rotate(&mut self, prio: u8) {
        let q = &mut self.levels[prio as usize % RT_LEVELS];
        if let Some(k) = q.pop_front() {
            q.push_back(k);
        }
    }

    /// Account `delta_ns` of real-time execution at `now`. Returns true when
    /// the class just became throttled.
    pub fn account(&mut self, now: u64, delta_ns: u64) -> bool {
        self.roll_period(now);
        self.period_used = self.period_used.saturating_add(delta_ns);
        if !self.throttled && self.period_used >= RT_RUNTIME_NS {
            self.throttled = true;
            return true;
        }
        false
    }

    /// Start a new bandwidth period when the old one has ended (unthrottles).
    pub fn roll_period(&mut self, now: u64) {
        if now.saturating_sub(self.period_start) >= RT_PERIOD_NS {
            self.period_start = now - (now - self.period_start) % RT_PERIOD_NS;
            self.period_used = 0;
            self.throttled = false;
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = TaskKey> + '_ {
        self.levels.iter().flat_map(|q| q.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_priority_first_fifo_within_level() {
        let mut rq = RtRq::new();
        rq.enqueue(1, 10, false);
        rq.enqueue(2, 50, false);
        rq.enqueue(3, 50, false);
        assert_eq!(rq.top_prio(), Some(50));
        assert_eq!(rq.pick(), Some(2));
        rq.dequeue(2, 50);
        assert_eq!(rq.pick(), Some(3));
        rq.dequeue(3, 50);
        assert_eq!(rq.pick(), Some(1));
        rq.dequeue(1, 10);
        assert_eq!(rq.pick(), None);
        assert!(rq.is_empty());
    }

    #[test]
    fn rotate_round_robins_a_level() {
        let mut rq = RtRq::new();
        rq.enqueue(1, 20, false);
        rq.enqueue(2, 20, false);
        rq.rotate(20);
        assert_eq!(rq.pick(), Some(2));
        rq.enqueue(3, 20, true);
        assert_eq!(rq.pick(), Some(3), "preempted task keeps the head");
    }

    #[test]
    fn bandwidth_throttles_then_recovers() {
        let mut rq = RtRq::new();
        rq.enqueue(1, 99, false);
        let mut now = 0;
        let mut throttled_at = None;
        while now < RT_PERIOD_NS {
            if rq.account(now, 10_000_000) {
                throttled_at = Some(now);
                break;
            }
            now += 10_000_000;
        }
        assert_eq!(throttled_at, Some(940_000_000));
        assert_eq!(rq.pick(), None);
        rq.roll_period(RT_PERIOD_NS);
        assert_eq!(rq.pick(), Some(1));
    }

    #[test]
    fn priority_is_clamped() {
        assert_eq!(RtEntity::new(0, RtPolicy::Fifo).prio, 1);
        assert_eq!(RtEntity::new(200, RtPolicy::Fifo).prio, 99);
    }
}
