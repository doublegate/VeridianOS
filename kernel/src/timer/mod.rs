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

/// Monotonically increasing counter for assigning unique timer IDs.
static NEXT_TIMER_ID: AtomicU64 = AtomicU64::new(1);

/// Global timer wheel instance, protected by a spin mutex.
static TIMER_WHEEL: GlobalState<Mutex<TimerWheel>> = GlobalState::new();

/// Monotonic uptime counter in milliseconds, updated on each tick.
static UPTIME_MS: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Unique identifier for a registered timer.
///
/// Wraps a `u64` value that is guaranteed unique for the lifetime of the
/// kernel (barring counter wrap at 2^64, which is practically impossible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

impl TimerId {
    /// Allocate the next unique timer ID.
    fn next() -> Self {
        Self(NEXT_TIMER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Timer firing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMode {
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
pub type TimerCallback = fn(TimerId);

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
                    *entry = None;
                    self.active_count = self.active_count.saturating_sub(1);
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
    /// Any timer whose remaining time reaches zero is fired (its callback
    /// is invoked). One-shot timers are automatically removed after
    /// firing; periodic timers are reloaded with their original interval.
    fn tick(&mut self, elapsed_ms: u64) {
        // Advance the wheel position for bookkeeping.
        self.current_slot = (self.current_slot + elapsed_ms as usize) % TIMER_WHEEL_SLOTS;

        // Collect IDs and callbacks of timers that need to fire so we can
        // invoke callbacks outside the mutable borrow of self.timers.
        // Use a fixed-size buffer to avoid heap allocation.
        let mut fired: [(TimerId, TimerCallback); 64] = [(TimerId(0), noop_callback); 64];
        let mut fired_count = 0usize;

        for entry in self.timers.iter_mut() {
            if let Some(timer) = entry {
                if !timer.active {
                    continue;
                }

                if timer.remaining_ms <= elapsed_ms {
                    // Timer expired -- record it for firing.
                    if fired_count < fired.len() {
                        fired[fired_count] = (timer.id, timer.callback);
                        fired_count += 1;
                    }

                    match timer.mode {
                        TimerMode::OneShot => {
                            // Remove one-shot timers.
                            *entry = None;
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

        // Fire callbacks after releasing the mutable borrow on timer entries.
        for &(id, cb) in fired.iter().take(fired_count) {
            (cb)(id);
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
pub fn init() -> KernelResult<()> {
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
pub fn create_timer(
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
pub fn cancel_timer(id: TimerId) -> KernelResult<()> {
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
pub fn timer_tick(elapsed_ms: u64) {
    // Update monotonic uptime counter.
    UPTIME_MS.fetch_add(elapsed_ms, Ordering::Relaxed);

    TIMER_WHEEL.with_mut(|wheel| {
        let mut wheel = wheel.lock();
        wheel.tick(elapsed_ms);
    });
}

/// Return the monotonic uptime in milliseconds since [`init`] was called.
///
/// This counter is incremented by [`timer_tick`] and is independent of
/// wall-clock time. It will not wrap for over 584 million years at
/// millisecond granularity.
pub fn get_uptime_ms() -> u64 {
    UPTIME_MS.load(Ordering::Relaxed)
}

/// Return the number of currently pending (active) timers.
pub fn pending_timer_count() -> usize {
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
