//! ktimerd: the kernel thread that acts on expiries of timers that send
//! signals (ITIMER_REAL, POSIX timers).
//!
//! The timer interrupt runs on CPU 0 with interrupts off and must not take
//! the locks a signal send takes (the task table, the signal queues), so
//! timers are not fired from it: this thread sleeps until the earliest
//! expiry, then acts on every one that is due. Each timer has at most one
//! entry, named by its [`Key`]: arming replaces it and cancelling removes
//! it. An owner checks an expiry against its own state when it comes, so
//! one taken off just before the timer was set again does nothing. An entry
//! for a time on the wall clock moves when the clock is set
//! ([`clock_was_set`]).
//!
//! Lock order: an owner's timer lock, then [`ENTRIES`]; the thread never
//! holds [`ENTRIES`] while it calls an owner.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use spin::Mutex;

/// A timer with an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Process `pid`'s ITIMER_REAL.
    Real(u64),
    /// Process `pid`'s POSIX timer `id`.
    Posix(u64, i32),
}

impl Key {
    fn owner(self) -> u64 {
        match self {
            Key::Real(pid) | Key::Posix(pid, _) => pid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The expiry (`monotonic_ns`).
    at: u64,
    key: Key,
    /// For a time on the wall clock: `at` follows the clock when it is set.
    wall: bool,
}

/// The armed timers.
static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// Set when [`ENTRIES`] changes, so the thread recomputes its deadline.
static CHANGED: AtomicBool = AtomicBool::new(false);
/// The thread sleeps here.
#[cfg(feature = "alloc")]
static WAKE: crate::sched::dispatch::WaitQueue = crate::sched::dispatch::WaitQueue::new();
/// 0: not started, 1: starting, 2: running.
static STATE: AtomicU8 = AtomicU8::new(0);

/// Put `key`'s entry, replacing any it has.
fn put(entries: &mut Vec<Entry>, key: Key, at: u64, wall: bool) {
    match entries.iter_mut().find(|e| e.key == key) {
        Some(e) => (e.at, e.wall) = (at, wall),
        None => entries.push(Entry { at, key, wall }),
    }
}

/// The wall clock moved by `delta_ns`: wall-clock entries keep their wall
/// time, so their monotonic expiry moves the other way.
fn shift_wall(entries: &mut [Entry], delta_ns: i64) {
    for e in entries.iter_mut().filter(|e| e.wall) {
        e.at = if delta_ns >= 0 {
            e.at.saturating_sub(delta_ns as u64)
        } else {
            e.at.saturating_add(delta_ns.unsigned_abs())
        };
    }
}

/// Take the entries due at `now`, earliest first.
fn take_due(entries: &mut Vec<Entry>, now: u64) -> Vec<Key> {
    let mut due: Vec<Entry> = entries.iter().copied().filter(|e| e.at <= now).collect();
    entries.retain(|e| e.at > now);
    due.sort_by_key(|e| e.at);
    due.into_iter().map(|e| e.key).collect()
}

/// The earliest expiry.
fn earliest(entries: &[Entry]) -> Option<u64> {
    entries.iter().map(|e| e.at).min()
}

fn changed() {
    CHANGED.store(true, Ordering::Release);
    #[cfg(feature = "alloc")]
    WAKE.wake_all();
}

/// Arm `key` to expire at `at` (`monotonic_ns`), replacing its entry.
pub fn arm(key: Key, at: u64) {
    put(&mut ENTRIES.lock(), key, at, false);
    changed();
}

/// Arm `key` for a time on the wall clock, which reads it at `at`
/// (`monotonic_ns`, `timer::realtime::monotonic_at`).
pub fn arm_wall(key: Key, at: u64) {
    put(&mut ENTRIES.lock(), key, at, true);
    changed();
}

/// The wall clock was set, moving by `delta_ns`.
pub fn clock_was_set(delta_ns: i64) {
    shift_wall(&mut ENTRIES.lock(), delta_ns);
    changed();
}

/// Disarm `key`.
pub fn cancel(key: Key) {
    ENTRIES.lock().retain(|e| e.key != key);
}

/// Disarm every timer of process `pid` (its exit).
pub fn cancel_owner(pid: u64) {
    ENTRIES.lock().retain(|e| e.key.owner() != pid);
}

/// Start the thread, once; later calls return at once.
pub fn start() -> Result<(), crate::error::KernelError> {
    loop {
        match STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(2) => return Ok(()),
            // Another caller is starting it.
            Err(_) => core::hint::spin_loop(),
        }
    }
    #[cfg(feature = "alloc")]
    let started = crate::sched::dispatch::spawn_kernel("ktimerd", ktimerd, 0).map(|_| ());
    #[cfg(not(feature = "alloc"))]
    let started = Err(crate::error::KernelError::NotImplemented { feature: "ktimerd" });
    STATE.store(if started.is_ok() { 2 } else { 0 }, Ordering::Release);
    started
}

#[cfg(feature = "alloc")]
extern "C" fn ktimerd(_: usize) {
    loop {
        let next = earliest(&ENTRIES.lock());
        let _ = crate::sched::dispatch::wait_event(&WAKE, next, || {
            CHANGED.swap(false, Ordering::AcqRel)
        });
        let now = crate::timer::monotonic_ns();
        let due = take_due(&mut ENTRIES.lock(), now);
        for key in due {
            match key {
                Key::Real(pid) => crate::process::itimer::real_expired(pid, now),
                Key::Posix(pid, id) => crate::process::posix_timer::expired(pid, id),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry per timer, the due ones taken earliest first, the rest
    /// left with the earliest of them next.
    #[test]
    fn entries_are_one_per_timer_and_taken_when_due() {
        let mut e = Vec::new();
        put(&mut e, Key::Real(1), 50, false);
        put(&mut e, Key::Real(2), 30, false);
        put(&mut e, Key::Real(3), 90, false);
        put(&mut e, Key::Real(1), 20, false);
        assert_eq!(e.len(), 3);
        assert_eq!(earliest(&e), Some(20));
        assert_eq!(take_due(&mut e, 40), [Key::Real(1), Key::Real(2)]);
        assert_eq!(earliest(&e), Some(90));
        assert!(take_due(&mut e, 89).is_empty());
        assert_eq!(take_due(&mut e, 90), [Key::Real(3)]);
        assert_eq!(earliest(&e), None);
    }

    /// Setting the wall clock moves only wall-clock entries, keeping their
    /// wall time; a timer re-armed off the wall clock stops moving.
    #[test]
    fn wall_entries_follow_the_clock() {
        let mut e = Vec::new();
        put(&mut e, Key::Posix(1, 0), 100, true);
        put(&mut e, Key::Posix(1, 1), 100, false);
        shift_wall(&mut e, 30);
        assert_eq!(take_due(&mut e, 70), [Key::Posix(1, 0)]);
        put(&mut e, Key::Posix(1, 0), 100, true);
        shift_wall(&mut e, -50);
        assert_eq!(earliest(&e), Some(100));
        put(&mut e, Key::Posix(1, 0), 120, false);
        shift_wall(&mut e, 500);
        assert_eq!(take_due(&mut e, 110), [Key::Posix(1, 1)]);
        assert_eq!(earliest(&e), Some(120));
    }
}
