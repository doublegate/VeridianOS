//! Block-device admission by I/O priority (ioprio_set; ADR 0027, the
//! maintainer's choice of an arbiter over asynchronous queues, which come
//! with v0.29's asynchronous completion path).
//!
//! The NVMe driver runs one request at a time behind the device's lock, so
//! there is no queue to reorder. Instead, callers wait here for their turn
//! at the device (BlockFS on virtio-blk does not: it issues requests under
//! its own spin locks, where a wait would sleep holding them), and each turn
//! goes to the waiter with the best priority: RT before BE before IDLE, by
//! level within a class (the lower ioprio value), the earliest first between
//! equals. Two rules keep anything from waiting forever, as Linux's I/O
//! schedulers keep classes from starving each other:
//! - a waiter passed over for [`STARVE_NS`] goes next, whatever its class;
//! - IDLE gets a turn only when no other class waits (or once starved).
//!
//! Before the dispatcher runs (boot) nothing waits here: the device lock
//! alone orders requests.

use alloc::collections::BTreeMap;

use spin::Mutex;

use crate::sched::dispatch::WaitQueue;

/// How long a waiter may be passed over before it goes next (1 s).
pub const STARVE_NS: u64 = 1_000_000_000;

/// A waiter's place: its ioprio value (lower is better), then arrival.
type Key = (u16, u64);

#[derive(Default)]
struct State {
    /// Whether a turn is out (the device is in use through it).
    busy: bool,
    /// Waiters: key -> when they started waiting.
    waiting: BTreeMap<Key, u64>,
    /// The waiter whose turn it is, until it notices.
    granted: Option<Key>,
    next: u64,
}

pub struct Arbiter {
    state: Mutex<State>,
    queue: WaitQueue,
}

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

/// The turn of the waiter that should go next among `waiting` at `now`:
/// the longest-waiting one if it has waited [`STARVE_NS`], else the best
/// priority that is not IDLE, else the best IDLE one.
fn choose(waiting: &BTreeMap<Key, u64>, now: u64) -> Option<Key> {
    let starved = waiting
        .iter()
        .filter(|(_, &since)| now.saturating_sub(since) >= STARVE_NS)
        .min_by_key(|(&(_, seq), &since)| (since, seq))
        .map(|(&k, _)| k);
    if starved.is_some() {
        return starved;
    }
    let idle = crate::syscall::ioprio::IOPRIO_CLASS_IDLE;
    waiting
        .keys()
        .find(|&&(prio, _)| crate::syscall::ioprio::class_of(prio) != idle)
        .or_else(|| waiting.keys().next())
        .copied()
}

/// A turn at the device; dropping it hands the device to the next waiter.
pub struct Turn<'a> {
    arbiter: Option<&'a Arbiter>,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let Some(a) = self.arbiter else {
            return;
        };
        let mut s = a.state.lock();
        match choose(&s.waiting, now()) {
            Some(key) => {
                s.waiting.remove(&key);
                s.granted = Some(key);
                drop(s);
                a.queue.wake_all();
            }
            None => s.busy = false,
        }
    }
}

fn now() -> u64 {
    crate::arch::timer::monotonic_ns()
}

impl Arbiter {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                busy: false,
                waiting: BTreeMap::new(),
                granted: None,
                next: 0,
            }),
            queue: WaitQueue::new(),
        }
    }

    /// Wait for a turn at `ioprio` (the caller's effective I/O priority).
    /// The wait cannot be interrupted, as Linux's for block I/O.
    pub fn admit(&self, ioprio: u16) -> Turn<'_> {
        if !crate::sched::dispatch::started() {
            return Turn { arbiter: None };
        }
        let key = {
            let mut s = self.state.lock();
            if !s.busy && s.waiting.is_empty() {
                s.busy = true;
                return Turn {
                    arbiter: Some(self),
                };
            }
            // Class and level only: the hint bits (3..13) sit between them
            // and would rank BE 0 with a hint below BE 7 (PR #45 review).
            let key = (ioprio & 0xe007, s.next);
            s.next += 1;
            s.waiting.insert(key, now());
            key
        };
        self.queue.wait_until(|| {
            let mut s = self.state.lock();
            if s.granted == Some(key) {
                s.granted = None;
                true
            } else {
                false
            }
        });
        Turn {
            arbiter: Some(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall::ioprio::{value, IOPRIO_CLASS_BE, IOPRIO_CLASS_IDLE, IOPRIO_CLASS_RT};

    fn waiting(entries: &[(u16, u64, u64)]) -> BTreeMap<Key, u64> {
        entries
            .iter()
            .map(|&(p, seq, since)| ((p, seq), since))
            .collect()
    }

    #[test]
    fn the_best_class_and_level_goes_first() {
        let be4 = value(IOPRIO_CLASS_BE, 4);
        let be1 = value(IOPRIO_CLASS_BE, 1);
        let rt7 = value(IOPRIO_CLASS_RT, 7);
        let w = waiting(&[(be4, 0, 0), (be1, 1, 0), (rt7, 2, 0)]);
        assert_eq!(choose(&w, 10), Some((rt7, 2)));
        let w = waiting(&[(be4, 0, 0), (be1, 1, 0)]);
        assert_eq!(choose(&w, 10), Some((be1, 1)));
        let w = waiting(&[(be4, 3, 0), (be4, 1, 0)]);
        assert_eq!(choose(&w, 10), Some((be4, 1)), "the earlier of equals");
        assert_eq!(choose(&BTreeMap::new(), 10), None);
    }

    #[test]
    fn idle_goes_only_when_nothing_else_waits() {
        let idle = value(IOPRIO_CLASS_IDLE, 0);
        let be7 = value(IOPRIO_CLASS_BE, 7);
        let w = waiting(&[(idle, 0, 0), (be7, 1, 0)]);
        assert_eq!(choose(&w, 10), Some((be7, 1)));
        let w = waiting(&[(idle, 0, 0)]);
        assert_eq!(choose(&w, 10), Some((idle, 0)));
    }

    #[test]
    fn a_starved_waiter_goes_next_whatever_its_class() {
        let idle = value(IOPRIO_CLASS_IDLE, 0);
        let rt0 = value(IOPRIO_CLASS_RT, 0);
        let w = waiting(&[(idle, 0, 0), (rt0, 1, STARVE_NS / 2)]);
        assert_eq!(choose(&w, STARVE_NS - 1), Some((rt0, 1)), "not yet starved");
        assert_eq!(choose(&w, STARVE_NS), Some((idle, 0)));
    }
}
