//! ktimerd: the kernel thread that acts on expiries of timers that send
//! signals (ITIMER_REAL).
//!
//! The timer interrupt runs on CPU 0 with interrupts off and must not take
//! the locks a signal send takes (the task table, the signal queues), so
//! timers are not fired from it: this thread sleeps until the earliest
//! expiry, then acts on every one that is due. Each timer has at most one
//! entry, named by its [`Key`]: arming replaces it and cancelling removes
//! it. An owner checks an expiry against its own state when it comes, so
//! one taken off just before the timer was set again does nothing.
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
}

impl Key {
    fn owner(self) -> u64 {
        match self {
            Key::Real(pid) => pid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The expiry (`monotonic_ns`).
    at: u64,
    key: Key,
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
fn put(entries: &mut Vec<Entry>, key: Key, at: u64) {
    match entries.iter_mut().find(|e| e.key == key) {
        Some(e) => e.at = at,
        None => entries.push(Entry { at, key }),
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
    put(&mut ENTRIES.lock(), key, at);
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
        put(&mut e, Key::Real(1), 50);
        put(&mut e, Key::Real(2), 30);
        put(&mut e, Key::Real(3), 90);
        put(&mut e, Key::Real(1), 20);
        assert_eq!(e.len(), 3);
        assert_eq!(earliest(&e), Some(20));
        assert_eq!(take_due(&mut e, 40), [Key::Real(1), Key::Real(2)]);
        assert_eq!(earliest(&e), Some(90));
        assert!(take_due(&mut e, 89).is_empty());
        assert_eq!(take_due(&mut e, 90), [Key::Real(3)]);
        assert_eq!(earliest(&e), None);
    }
}
