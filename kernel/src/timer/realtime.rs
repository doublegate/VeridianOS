//! The wall clock: CLOCK_REALTIME (N-219).
//!
//! Wall time is the monotonic clock plus an offset: set from the real-time
//! clock at boot (UTC seconds since 1970), and by clock_settime and
//! settimeofday. Moving it never moves the monotonic clock, as on Linux.
//! Until an RTC has been read the offset is zero and the wall clock counts
//! from boot. The RTC is the CMOS clock on x86_64 and a device-tree PL031
//! or Goldfish RTC on AArch64 and RISC-V (`arch::mmio_rtc`).

use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// Wall time minus monotonic time, in nanoseconds.
static OFFSET_NS: AtomicI64 = AtomicI64::new(0);

/// Whether the wall clock has been set (from an RTC, or by clock_settime
/// or settimeofday): before that it counts from boot, not from 1970.
static KNOWN: AtomicBool = AtomicBool::new(false);

/// Whether the wall clock reads the real date. Decisions that depend on
/// the date (account expiry) must not trust it otherwise.
pub fn is_known() -> bool {
    KNOWN.load(Ordering::Acquire)
}

const NS_PER_SEC: i64 = 1_000_000_000;

/// The wall time (nanoseconds since 1970) when the monotonic clock reads
/// `mono_ns`.
pub fn wall_ns_at(mono_ns: u64) -> i64 {
    (mono_ns.min(i64::MAX as u64) as i64).saturating_add(OFFSET_NS.load(Ordering::Acquire))
}

/// The wall time now, in nanoseconds since 1970.
pub fn now_ns() -> i64 {
    wall_ns_at(super::monotonic_ns())
}

/// Wall time minus monotonic time now, in nanoseconds: what a wall time is
/// converted with ([`monotonic_at`] is `wall_ns - offset_ns()`, from 0).
pub fn offset_ns() -> i64 {
    OFFSET_NS.load(Ordering::Acquire)
}

/// The monotonic time at which the wall clock will read `wall_ns`, for
/// absolute sleeps and timers on CLOCK_REALTIME (0 if it has passed or is
/// before boot).
pub fn monotonic_at(wall_ns: i64) -> u64 {
    wall_ns
        .saturating_sub(OFFSET_NS.load(Ordering::Acquire))
        .max(0) as u64
}

/// Make the wall clock read `wall_ns` now. Returns how far it moved (new
/// minus old offset), for timers on the wall clock to follow.
pub fn set_now_ns(wall_ns: i64) -> i64 {
    let mono = super::monotonic_ns().min(i64::MAX as u64) as i64;
    let new = wall_ns.saturating_sub(mono);
    let old = OFFSET_NS.swap(new, Ordering::AcqRel);
    KNOWN.store(true, Ordering::Release);
    SET_COUNT.fetch_add(1, Ordering::AcqRel);
    new.saturating_sub(old)
}

/// The largest wall time settimeofday accepts, in seconds: Linux's
/// TIME_SETTOD_SEC_MAX (KTIME_SEC_MAX less 30 years of uptime).
const SETTOD_SEC_MAX: i64 = i64::MAX / NS_PER_SEC - 30 * 365 * 86_400;

/// Step the wall clock by `delta_ns` (adjtimex ADJ_SETOFFSET), as Linux's
/// __timekeeping_inject_offset: refused (`None`) if the wall clock would
/// read earlier than boot or beyond what settimeofday accepts. Returns how
/// far it moved.
pub fn inject_offset(delta_ns: i64) -> Option<i64> {
    let mono = super::monotonic_ns().min(i64::MAX as u64) as i64;
    OFFSET_NS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            let new = old.checked_add(delta_ns)?;
            let wall = mono.checked_add(new)?;
            (new >= 0 && wall >= 0 && wall / NS_PER_SEC < SETTOD_SEC_MAX).then_some(new)
        })
        .ok()?;
    KNOWN.store(true, Ordering::Release);
    SET_COUNT.fetch_add(1, Ordering::AcqRel);
    Some(delta_ns)
}

/// Move the wall clock by `delta_ns` for a leap second (`timer::steer`):
/// back a second for an inserted one, ahead for a deleted one. Counted as
/// a set, for TFD_TIMER_CANCEL_ON_SET timers, as Linux's
/// clock_was_set_delayed.
pub fn leap(delta_ns: i64) {
    let _ = OFFSET_NS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
        Some(old.saturating_add(delta_ns))
    });
    SET_COUNT.fetch_add(1, Ordering::AcqRel);
}

/// Count a change of the wall clocks that did not move CLOCK_REALTIME
/// (a new TAI offset), for TFD_TIMER_CANCEL_ON_SET timers.
pub fn mark_set() {
    SET_COUNT.fetch_add(1, Ordering::AcqRel);
}

/// How many times the wall clock has been set (TFD_TIMER_CANCEL_ON_SET
/// timers notice a change).
static SET_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// The wall clock's set count.
pub fn set_count() -> u64 {
    SET_COUNT.load(Ordering::Acquire)
}

/// The RTC read `epoch_secs` (UTC) when the monotonic clock read `mono_ns`.
pub fn set_from_rtc(epoch_secs: u64, mono_ns: u64) {
    let wall = (epoch_secs.min((i64::MAX / NS_PER_SEC) as u64) as i64) * NS_PER_SEC;
    OFFSET_NS.store(
        wall.saturating_sub(mono_ns.min(i64::MAX as u64) as i64),
        Ordering::Release,
    );
    KNOWN.store(true, Ordering::Release);
}

/// Serializes the tests that move the (global) wall clock.
#[cfg(test)]
pub(crate) static TEST_CLOCK_LOCK: spin::Mutex<()> = spin::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// The offset maps monotonic to wall time and back, and an RTC reading
    /// anchors it.
    #[test]
    fn wall_time_is_monotonic_plus_offset() {
        let _clock = TEST_CLOCK_LOCK.lock();
        let saved = OFFSET_NS.load(Ordering::Acquire);
        set_from_rtc(1_800_000_000, 5 * NS_PER_SEC as u64);
        assert_eq!(
            wall_ns_at(5 * NS_PER_SEC as u64),
            1_800_000_000 * NS_PER_SEC
        );
        assert_eq!(
            wall_ns_at(6 * NS_PER_SEC as u64),
            1_800_000_001 * NS_PER_SEC
        );
        assert_eq!(
            monotonic_at(1_800_000_001 * NS_PER_SEC),
            6 * NS_PER_SEC as u64
        );
        // Before boot: no negative monotonic time.
        assert_eq!(monotonic_at(0), 0);
        OFFSET_NS.store(saved, Ordering::Release);
    }

    /// ADJ_SETOFFSET steps the wall clock, but never to before boot or
    /// beyond what settimeofday could set.
    #[test]
    fn inject_offset_is_bounded() {
        let _clock = TEST_CLOCK_LOCK.lock();
        let saved = OFFSET_NS.load(Ordering::Acquire);
        OFFSET_NS.store(10 * NS_PER_SEC, Ordering::Release);
        let count = set_count();
        assert_eq!(inject_offset(NS_PER_SEC), Some(NS_PER_SEC));
        assert_eq!(OFFSET_NS.load(Ordering::Acquire), 11 * NS_PER_SEC);
        assert_eq!(set_count(), count + 1);
        assert_eq!(inject_offset(-11 * NS_PER_SEC), Some(-11 * NS_PER_SEC));
        assert_eq!(inject_offset(-1), None);
        assert_eq!(inject_offset(i64::MAX), None);
        assert_eq!(inject_offset(SETTOD_SEC_MAX * NS_PER_SEC), None);
        assert_eq!(OFFSET_NS.load(Ordering::Acquire), 0);
        OFFSET_NS.store(saved, Ordering::Release);
    }
}
