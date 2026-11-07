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
//! for a time on the wall clock keeps that wall time and is converted to
//! the monotonic clock each time the thread looks, so a clock set
//! ([`clock_was_set`]) moves it however it races with the arming (PR #35
//! review: converting at arming and shifting at the set could miss the set,
//! or apply it twice).
//!
//! The thread also runs the NTP state's seconds while it has something to
//! do (`timer::steer`: the slew, the phase offset, a leap second).
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
    /// The expiry (`monotonic_ns`), for a timer on the monotonic clock.
    at: u64,
    key: Key,
    /// For a timer on the wall clock: the expiry as the wall clock reads it
    /// (ns since 1970); `at` is then unused.
    wall: Option<i64>,
}

impl Entry {
    /// The monotonic expiry, a wall time converted by `to_mono`
    /// (`timer::realtime::monotonic_at`).
    fn due(&self, to_mono: &impl Fn(i64) -> u64) -> u64 {
        self.wall.map_or(self.at, to_mono)
    }
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
fn put(entries: &mut Vec<Entry>, key: Key, at: u64, wall: Option<i64>) {
    match entries.iter_mut().find(|e| e.key == key) {
        Some(e) => (e.at, e.wall) = (at, wall),
        None => entries.push(Entry { at, key, wall }),
    }
}

/// Take the entries due at `now`, earliest first.
fn take_due(entries: &mut Vec<Entry>, now: u64, to_mono: &impl Fn(i64) -> u64) -> Vec<Key> {
    let mut due: Vec<(u64, Key)> = entries
        .iter()
        .map(|e| (e.due(to_mono), e.key))
        .filter(|&(at, _)| at <= now)
        .collect();
    entries.retain(|e| e.due(to_mono) > now);
    due.sort_by_key(|&(at, _)| at);
    due.into_iter().map(|(_, key)| key).collect()
}

/// The earliest expiry.
fn earliest(entries: &[Entry], to_mono: &impl Fn(i64) -> u64) -> Option<u64> {
    entries.iter().map(|e| e.due(to_mono)).min()
}

fn changed() {
    CHANGED.store(true, Ordering::Release);
    #[cfg(feature = "alloc")]
    WAKE.wake_all();
}

/// Arm `key` to expire at `at` (`monotonic_ns`), replacing its entry.
pub fn arm(key: Key, at: u64) {
    put(&mut ENTRIES.lock(), key, at, None);
    changed();
}

/// Arm `key` to expire when the wall clock reads `wall_ns` (ns since
/// 1970).
pub fn arm_wall(key: Key, wall_ns: i64) {
    put(&mut ENTRIES.lock(), key, 0, Some(wall_ns));
    changed();
}

/// The wall clock was set: the thread looks again at when its wall-clock
/// entries are due.
pub fn clock_was_set(_delta_ns: i64) {
    changed();
}

/// Have the thread look again at when it must wake (the NTP state
/// changed).
pub fn kick() {
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
        let to_mono = |wall| crate::timer::realtime::monotonic_at(wall);
        let next = earliest(&ENTRIES.lock(), &to_mono);
        let ntp = super::steer::next_due();
        let next = [next, ntp].into_iter().flatten().min();
        let _ = crate::sched::dispatch::wait_event(&WAKE, next, || {
            CHANGED.swap(false, Ordering::AcqRel)
        });
        // Looked at again: adjtimex may have made the state active while
        // the thread slept (it kicks it), and a clock set since must be
        // seen at once (`steer::next_due`).
        if ntp.is_some() || super::steer::next_due().is_some() {
            super::steer::service();
        }
        let now = crate::timer::monotonic_ns();
        let due = take_due(&mut ENTRIES.lock(), now, &to_mono);
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

    /// Monotonic clock only (no wall entries in these).
    fn mono(_: i64) -> u64 {
        unreachable!()
    }

    /// One entry per timer, the due ones taken earliest first, the rest
    /// left with the earliest of them next.
    #[test]
    fn entries_are_one_per_timer_and_taken_when_due() {
        let mut e = Vec::new();
        put(&mut e, Key::Real(1), 50, None);
        put(&mut e, Key::Real(2), 30, None);
        put(&mut e, Key::Real(3), 90, None);
        put(&mut e, Key::Real(1), 20, None);
        assert_eq!(e.len(), 3);
        assert_eq!(earliest(&e, &mono), Some(20));
        assert_eq!(take_due(&mut e, 40, &mono), [Key::Real(1), Key::Real(2)]);
        assert_eq!(earliest(&e, &mono), Some(90));
        assert!(take_due(&mut e, 89, &mono).is_empty());
        assert_eq!(take_due(&mut e, 90, &mono), [Key::Real(3)]);
        assert_eq!(earliest(&e, &mono), None);
    }

    /// A wall-clock entry is due when the wall clock reads its time,
    /// whatever the clock was set to between arming and now: it is
    /// converted when looked at, never shifted, so a set racing with the
    /// arming can be neither missed nor applied twice (PR #35 review). A
    /// timer re-armed off the wall clock stops following it.
    #[test]
    fn wall_entries_follow_the_clock() {
        let offset = core::cell::Cell::new(1_000i64); // wall = mono + offset
        let to_mono = |wall: i64| (wall - offset.get()).max(0) as u64;
        let mut e = Vec::new();
        put(&mut e, Key::Posix(1, 0), 0, Some(1_100));
        put(&mut e, Key::Posix(1, 1), 100, None);
        assert_eq!(earliest(&e, &to_mono), Some(100));
        // The wall clock goes ahead 30: the wall entry is due 30 sooner.
        offset.set(1_030);
        assert_eq!(take_due(&mut e, 70, &to_mono), [Key::Posix(1, 0)]);
        put(&mut e, Key::Posix(1, 0), 0, Some(1_130));
        // Back 50: later.
        offset.set(980);
        assert_eq!(earliest(&e, &to_mono), Some(100));
        assert_eq!(take_due(&mut e, 100, &to_mono), [Key::Posix(1, 1)]);
        assert_eq!(earliest(&e, &to_mono), Some(150));
        put(&mut e, Key::Posix(1, 0), 120, None);
        offset.set(10_000);
        assert_eq!(earliest(&e, &to_mono), Some(120));
    }
}
