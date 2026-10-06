//! High-resolution timer management for VeridianOS.
//!
//! This module provides a software timer wheel that sits above the
//! architecture-specific hardware timer layer ([`crate::arch::timer`]).
//! It supports both one-shot and periodic timers with millisecond
//! granularity, using a hierarchical timer wheel with 256 slots for
//! efficient O(1) insertion and expiration.
//!
//! # Usage
//!
//! ```ignore
//! // Initialize the timer subsystem (called once during boot)
//! timer::init()?;
//!
//! // Create a one-shot timer that fires after 100ms
//! let id = timer::create_timer(TimerMode::OneShot, 100, my_callback)?;
//!
//! // Cancel a timer
//! timer::cancel_timer(id)?;
//!
//! // Called from the timer interrupt handler
//! timer::timer_tick(elapsed_ms);
//!
//! // Query monotonic uptime
//! let uptime = timer::get_uptime_ms();
//! ```

// Timer subsystem

#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    sync::once_lock::GlobalState,
};

/// Number of slots in the timer wheel.
///
/// 256 provides a good balance between memory usage and timer resolution.
/// Timers are hashed into slots based on their expiration tick modulo this
/// value.
const TIMER_WHEEL_SLOTS: usize = 256;

/// Maximum number of timers that can be active simultaneously.
///
/// This is a fixed upper bound to avoid unbounded heap allocation in the
/// kernel. Each timer entry is small (~48 bytes), so 1024 entries use
/// roughly 48 KiB.
const MAX_TIMERS: usize = 1024;

/// Callbacks handed out per pass by `TimerWheel::take_due`; a fixed buffer
/// so firing timers needs no heap allocation.
const FIRE_BATCH: usize = 64;

/// Monotonically increasing counter for assigning unique timer IDs.
static NEXT_TIMER_ID: AtomicU64 = AtomicU64::new(1);

/// Global timer wheel instance, protected by a spin mutex.
static TIMER_WHEEL: GlobalState<Mutex<TimerWheel>> = GlobalState::new();

/// Monotonic uptime counter in milliseconds, advanced by [`timer_tick`].
static UPTIME_MS: AtomicU64 = AtomicU64::new(0);

/// Milliseconds not yet applied to the timer wheel because its lock was
/// held when a tick arrived.
static PENDING_WHEEL_MS: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Unique identifier for a registered timer.
///
/// Wraps a `u64` value that is guaranteed unique for the lifetime of the
/// kernel (barring counter wrap at 2^64, which is practically impossible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TimerId(pub(crate) u64);

impl TimerId {
    /// Allocate the next unique timer ID.
    fn next() -> Self {
        Self(NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Timer firing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerMode {
    /// Fire once after the interval elapses, then auto-deactivate.
    OneShot,
    /// Fire repeatedly at the given interval until explicitly cancelled.
    Periodic,
}

/// Type alias for timer callback functions.
///
/// Callbacks are plain function pointers (not closures) so they can be
/// stored in static data without requiring `alloc`. The [`TimerId`] of the
/// firing timer is passed so the callback can identify which timer expired.
pub(crate) type TimerCallback = fn(TimerId);

/// A single software timer entry.
#[derive(Debug, Clone, Copy)]
struct Timer {
    /// Unique identifier for this timer.
    id: TimerId,
    /// One-shot or periodic.
    mode: TimerMode,
    /// Interval in milliseconds (used for periodic reload).
    interval_ms: u64,
    /// Milliseconds remaining until this timer fires.
    remaining_ms: u64,
    /// Function to call when the timer expires.
    callback: TimerCallback,
    /// Whether this timer is currently active.
    active: bool,
    /// Expired, and its callback not yet handed out by `take_due`.
    due: bool,
}

// ---------------------------------------------------------------------------
// TimerWheel
// ---------------------------------------------------------------------------

/// Hierarchical timer wheel with 256 slots.
///
/// Each slot holds a fixed-size array of timer entries. On each tick the
/// wheel advances and fires any expired timers in the current slot, then
/// decrements remaining timers in other slots.
///
/// This design avoids heap allocation by using a flat array of timer
/// entries and a free-list encoded via the `active` flag.
struct TimerWheel {
    /// All timer entries (flat pool).
    timers: [Option<Timer>; MAX_TIMERS],
    /// Current wheel position (0..TIMER_WHEEL_SLOTS).
    current_slot: usize,
    /// Number of currently active timers.
    active_count: usize,
}

impl TimerWheel {
    /// Create a new, empty timer wheel.
    fn new() -> Self {
        // Initialize all slots to None using array init pattern
        const NONE_TIMER: Option<Timer> = None;
        Self {
            timers: [NONE_TIMER; MAX_TIMERS],
            current_slot: 0,
            active_count: 0,
        }
    }

    /// Register a new timer in the wheel.
    ///
    /// Returns the [`TimerId`] assigned to the new timer, or an error if
    /// the maximum number of timers has been reached.
    fn add_timer(
        &mut self,
        mode: TimerMode,
        interval_ms: u64,
        callback: TimerCallback,
    ) -> KernelResult<TimerId> {
        if interval_ms == 0 {
            return Err(KernelError::InvalidArgument {
                name: "interval_ms",
                value: "must be > 0",
            });
        }

        // Find a free slot in the timer pool.
        let slot =
            self.timers
                .iter()
                .position(|t| t.is_none())
                .ok_or(KernelError::ResourceExhausted {
                    resource: "timer slots",
                })?;

        let id = TimerId::next();

        self.timers[slot] = Some(Timer {
            id,
            mode,
            interval_ms,
            remaining_ms: interval_ms,
            callback,
            active: true,
            due: false,
        });

        self.active_count += 1;
        Ok(id)
    }

    /// Cancel an active timer by its ID.
    ///
    /// Returns `Ok(())` if the timer was found and removed, or an error
    /// if no timer with the given ID exists.
    fn cancel_timer(&mut self, id: TimerId) -> KernelResult<()> {
        for entry in self.timers.iter_mut() {
            if let Some(timer) = entry {
                if timer.id == id {
                    // An expired one-shot awaiting its callback was already
                    // taken off the active count by advance().
                    if timer.active {
                        self.active_count = self.active_count.saturating_sub(1);
                    }
                    *entry = None;
                    return Ok(());
                }
            }
        }

        Err(KernelError::NotFound {
            resource: "timer",
            id: id.0,
        })
    }

    /// Advance all timers by `elapsed_ms` milliseconds.
    ///
    /// Every timer whose remaining time reaches zero is marked due. One-shot
    /// timers stop counting as active; periodic timers are reloaded with
    /// their interval. Nothing is fired here: [`Self::take_due`] hands the
    /// due callbacks out in bounded batches so the caller can run them after
    /// dropping the wheel lock.
    fn advance(&mut self, elapsed_ms: u64) {
        // Advance the wheel position for bookkeeping.
        self.current_slot = (self.current_slot + elapsed_ms as usize) % TIMER_WHEEL_SLOTS;

        for timer in self.timers.iter_mut().flatten() {
            if !timer.active {
                continue;
            }

            if timer.remaining_ms <= elapsed_ms {
                // A due timer fires once even if it expires again before
                // its callback is collected.
                timer.due = true;
                match timer.mode {
                    TimerMode::OneShot => {
                        // The entry is freed when take_due() collects it.
                        timer.active = false;
                        self.active_count = self.active_count.saturating_sub(1);
                    }
                    TimerMode::Periodic => {
                        // Reload periodic timers, accounting for overshoot.
                        let overshoot = elapsed_ms.saturating_sub(timer.remaining_ms);
                        timer.remaining_ms = timer
                            .interval_ms
                            .saturating_sub(overshoot % timer.interval_ms);
                    }
                }
            } else {
                timer.remaining_ms -= elapsed_ms;
            }
        }
    }

    /// Move up to `out.len()` due timers into `out`, clearing their due flag
    /// and freeing collected one-shot entries. Returns how many were taken;
    /// call again until it returns 0.
    ///
    /// The old tick() recorded at most 64 expirations per tick and silently
    /// dropped the callbacks of the rest while still removing or reloading
    /// them (review of the v0.26.0 stack, PR #11). Due timers past the batch
    /// now simply stay due for the next call.
    fn take_due(&mut self, out: &mut [(TimerId, TimerCallback)]) -> usize {
        let mut n = 0;
        for entry in self.timers.iter_mut() {
            if n == out.len() {
                break;
            }
            let Some(timer) = entry else { continue };
            if !timer.due {
                continue;
            }
            out[n] = (timer.id, timer.callback);
            n += 1;
            if timer.active {
                timer.due = false;
            } else {
                *entry = None;
            }
        }
        n
    }

    /// Advance by `elapsed_ms` and fire every due callback (host tests; the
    /// kernel path is [`timer_tick`], which fires outside the lock).
    #[cfg(test)]
    fn tick(&mut self, elapsed_ms: u64) {
        self.advance(elapsed_ms);
        let mut batch = [(TimerId(0), noop_callback as TimerCallback); FIRE_BATCH];
        loop {
            let n = self.take_due(&mut batch);
            if n == 0 {
                break;
            }
            for &(id, cb) in &batch[..n] {
                cb(id);
            }
        }
    }

    /// Return the number of currently active (pending) timers.
    fn pending_count(&self) -> usize {
        self.active_count
    }
}

/// No-op callback used as a placeholder in the fired-timers buffer.
fn noop_callback(_id: TimerId) {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the timer subsystem.
///
/// Must be called once during kernel boot, after the global allocator is
/// available (for the `GlobalState` mutex). Repeated calls return an
/// error.
pub(crate) fn init() -> KernelResult<()> {
    TIMER_WHEEL
        .init(Mutex::new(TimerWheel::new()))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "timer wheel",
            id: 0,
        })
}

/// Create and register a new timer.
///
/// # Arguments
/// * `mode` -- [`TimerMode::OneShot`] or [`TimerMode::Periodic`].
/// * `interval_ms` -- Time in milliseconds until (each) expiration. Must be
///   greater than zero.
/// * `callback` -- Function to invoke when the timer fires.
///
/// # Returns
/// The [`TimerId`] of the newly created timer.
///
/// The callback runs in timer-interrupt context: it must not block, spin
/// on a lock, or print.
pub(crate) fn create_timer(
    mode: TimerMode,
    interval_ms: u64,
    callback: TimerCallback,
) -> KernelResult<TimerId> {
    TIMER_WHEEL
        .with_mut(|wheel| {
            let mut wheel = wheel.lock();
            wheel.add_timer(mode, interval_ms, callback)
        })
        .unwrap_or(Err(KernelError::NotInitialized { subsystem: "timer" }))
}

/// Cancel an active timer.
///
/// Returns `Ok(())` if the timer was found and removed, or a
/// [`KernelError::NotFound`] if no such timer exists.
pub(crate) fn cancel_timer(id: TimerId) -> KernelResult<()> {
    TIMER_WHEEL
        .with_mut(|wheel| {
            let mut wheel = wheel.lock();
            wheel.cancel_timer(id)
        })
        .unwrap_or(Err(KernelError::NotInitialized { subsystem: "timer" }))
}

/// Advance all timers by `elapsed_ms` milliseconds and fire expired ones.
///
/// This function should be called from the timer interrupt handler (or a
/// periodic scheduler tick) with the number of milliseconds that have
/// elapsed since the last call.
pub(crate) fn timer_tick(elapsed_ms: u64) {
    // Update monotonic uptime counter.
    UPTIME_MS.fetch_add(elapsed_ms, Ordering::Relaxed);

    // Called from the timer interrupt: never spin on either lock around the
    // wheel (the interrupted code may hold one -- e.g. while initializing
    // or adding a timer). Carry the time over to the next tick instead.
    PENDING_WHEEL_MS.fetch_add(elapsed_ms, Ordering::AcqRel);
    TIMER_WHEEL.try_with_mut(|wheel| {
        if let Some(mut wheel) = wheel.try_lock() {
            wheel.advance(PENDING_WHEEL_MS.swap(0, Ordering::AcqRel));
        }
    });

    // Fire due callbacks in batches, each after both locks are dropped, so a
    // callback that creates or cancels a timer does not spin on the lock its
    // own caller holds. Anything left due (lock contended) fires next tick.
    let mut batch = [(TimerId(0), noop_callback as TimerCallback); FIRE_BATCH];
    loop {
        let n = TIMER_WHEEL
            .try_with_mut(|wheel| wheel.try_lock().map(|mut w| w.take_due(&mut batch)))
            .flatten()
            .unwrap_or(0);
        if n == 0 {
            break;
        }
        for &(id, cb) in &batch[..n] {
            cb(id);
        }
    }
}

/// Return the monotonic uptime in milliseconds.
///
/// Every timed wait in the kernel (nanosleep, poll/epoll timeouts, timerfd,
/// futex timeouts) and CLOCK_MONOTONIC read this. Nothing used to advance
/// it, so it stayed 0 and those waits never ended.
///
/// It is read from each architecture's clock source
/// ([`crate::arch::timer::monotonic_ns`]), so it advances whether or not
/// timer interrupts are running and does not drift with missed ticks. Host
/// unit tests use the tick-driven counter instead.
pub(crate) fn get_uptime_ms() -> u64 {
    monotonic_ns() / 1_000_000
}

/// Monotonic time in nanoseconds (CLOCK_MONOTONIC), at the clock source's
/// resolution.
pub(crate) fn monotonic_ns() -> u64 {
    #[cfg(target_os = "none")]
    {
        crate::arch::timer::monotonic_ns()
    }
    #[cfg(not(target_os = "none"))]
    {
        UPTIME_MS.load(Ordering::Relaxed) * 1_000_000
    }
}

/// Return the number of currently pending (active) timers.
pub(crate) fn pending_timer_count() -> usize {
    TIMER_WHEEL
        .with(|wheel| {
            let wheel = wheel.lock();
            wheel.pending_count()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Dummy callback that does nothing (used in tests).
    fn test_callback(_id: TimerId) {}

    #[test]
    fn test_timer_wheel_add_and_cancel() {
        let mut wheel = TimerWheel::new();

        let id = wheel
            .add_timer(TimerMode::OneShot, 100, test_callback)
            .unwrap();
        assert_eq!(wheel.pending_count(), 1);

        wheel.cancel_timer(id).unwrap();
        assert_eq!(wheel.pending_count(), 0);
    }

    #[test]
    fn test_timer_wheel_cancel_nonexistent() {
        let mut wheel = TimerWheel::new();
        let result = wheel.cancel_timer(TimerId(999));
        assert!(result.is_err());
    }

    #[test]
    fn test_timer_wheel_one_shot_fires_and_removes() {
        let mut wheel = TimerWheel::new();
        let _id = wheel
            .add_timer(TimerMode::OneShot, 50, test_callback)
            .unwrap();
        assert_eq!(wheel.pending_count(), 1);

        // Tick past the expiry.
        wheel.tick(60);
        assert_eq!(wheel.pending_count(), 0);
    }

    #[test]
    fn test_timer_wheel_periodic_reloads() {
        let mut wheel = TimerWheel::new();
        let _id = wheel
            .add_timer(TimerMode::Periodic, 100, test_callback)
            .unwrap();
        assert_eq!(wheel.pending_count(), 1);

        // Tick past the first expiry.
        wheel.tick(110);
        // Periodic timer should still be active.
        assert_eq!(wheel.pending_count(), 1);
    }

    static ONE_SHOT_FIRES: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);
    static PERIODIC_FIRES: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    fn count_one_shot(_id: TimerId) {
        ONE_SHOT_FIRES.fetch_add(1, Ordering::Relaxed);
    }

    fn count_periodic(_id: TimerId) {
        PERIODIC_FIRES.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn test_timer_wheel_fires_every_one_shot_past_a_batch() {
        // More than one 64-entry batch expiring in the same tick (review of
        // the v0.26.0 stack, PR #11): every callback must run exactly once.
        let mut wheel = TimerWheel::new();
        for _ in 0..100 {
            wheel
                .add_timer(TimerMode::OneShot, 10, count_one_shot)
                .unwrap();
        }
        wheel.tick(10);
        assert_eq!(ONE_SHOT_FIRES.load(Ordering::Relaxed), 100);
        assert_eq!(wheel.pending_count(), 0);
        assert!(wheel.timers.iter().all(|t| t.is_none()));
        wheel.tick(10);
        assert_eq!(ONE_SHOT_FIRES.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_timer_wheel_fires_every_periodic_past_a_batch() {
        let mut wheel = TimerWheel::new();
        for _ in 0..100 {
            wheel
                .add_timer(TimerMode::Periodic, 10, count_periodic)
                .unwrap();
        }
        wheel.tick(10);
        assert_eq!(PERIODIC_FIRES.load(Ordering::Relaxed), 100);
        assert_eq!(wheel.pending_count(), 100);
        wheel.tick(10);
        assert_eq!(PERIODIC_FIRES.load(Ordering::Relaxed), 200);
    }

    #[test]
    fn test_timer_wheel_zero_interval_rejected() {
        let mut wheel = TimerWheel::new();
        let result = wheel.add_timer(TimerMode::OneShot, 0, test_callback);
        assert!(result.is_err());
    }

    #[test]
    fn test_timer_id_uniqueness() {
        let id1 = TimerId::next();
        let id2 = TimerId::next();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_uptime_counter() {
        // Reset the counter for this test.
        UPTIME_MS.store(0, Ordering::Relaxed);
        assert_eq!(get_uptime_ms(), 0);
        UPTIME_MS.fetch_add(42, Ordering::Relaxed);
        assert_eq!(get_uptime_ms(), 42);
    }
}
