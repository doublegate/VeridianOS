//! CLOCK_MONOTONIC: the clock source steered by the NTP state (Linux's
//! timekeeping_advance with ntp_tick_length).
//!
//! The raw clock (CLOCK_MONOTONIC_RAW, [`raw_ns`]) is the architecture's
//! clock source. The steered clock advances `mult / 2^32` ns for each raw
//! ns, from an anchor where both were read; `mult` comes from the length
//! of a second that `timer::ntp` computes from the tick, the frequency,
//! and what the phase offset and the adjtime() slew add to this second.
//! CLOCK_REALTIME is the steered clock plus the wall offset
//! (`timer::realtime`), so both are steered, and every timed wait (which
//! counts on [`super::monotonic_ns`]) runs on it too.
//!
//! The rate is published in two slots, each under its own sequence count:
//! a writer (holding the NTP lock) fills the slot not in use and then
//! switches to it, so a reader, the timer interrupt included, never waits
//! for a writer it interrupted. A new rate is anchored where the old one
//! reads at the writer's raw time, so the clock is continuous.
//!
//! Once a wall second (UTC) starts, `ktimerd` runs [`service`]: Linux's
//! second_overflow for each second that started (the leap second state
//! machine, maxerror, the next second's phase), and steps the wall clock
//! for a leap second. It only wakes for this while the NTP state has
//! something to do (`Ntp::is_active`); an idle clock costs nothing.

use core::sync::atomic::{fence, AtomicU64, AtomicUsize, Ordering};

use super::ntp::Ntp;

const NS_PER_SEC: u64 = 1_000_000_000;
/// `mult` for a nominal second: one ns per raw ns.
pub(super) const NOMINAL_MULT: u64 = 1 << 32;
/// The most seconds one [`service`] runs; the clock is then resynchronized
/// (only after the thread could not run for that long).
const MAX_CATCH_UP: u32 = 64;

/// One rate of the steered clock: it read `anchor_mono` when the raw clock
/// read `anchor_raw`, and advances `mult / 2^32` ns per raw ns since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rate {
    pub anchor_raw: u64,
    pub anchor_mono: u64,
    pub mult: u64,
}

impl Rate {
    /// The steered clock equals the raw clock.
    pub const IDENTITY: Self = Self {
        anchor_raw: 0,
        anchor_mono: 0,
        mult: NOMINAL_MULT,
    };

    /// The steered time when the raw clock reads `raw` (a raw time before
    /// the anchor, read on a CPU slightly behind, reads the anchor).
    pub fn mono_at(&self, raw: u64) -> u64 {
        let d = raw.saturating_sub(self.anchor_raw);
        let d = if self.mult == NOMINAL_MULT {
            d
        } else {
            ((d as u128 * self.mult as u128) >> 32).min(u64::MAX as u128) as u64
        };
        self.anchor_mono.saturating_add(d)
    }

    /// The rate from raw time `raw` on, for seconds `second_length` long
    /// (scaled ns), continuous with this one there.
    pub fn rebase(&self, raw: u64, second_length: u64) -> Self {
        Self {
            anchor_raw: raw,
            anchor_mono: self.mono_at(raw),
            mult: mult_of(second_length),
        }
    }
}

/// `mult` for seconds `second_length` (ns << 32) long, to the nearest
/// 2^-32 (a quarter of a part per billion).
pub(super) fn mult_of(second_length: u64) -> u64 {
    (second_length.saturating_add(NS_PER_SEC / 2) / NS_PER_SEC).max(1)
}

/// A slot of the published rate.
struct Slot {
    seq: AtomicU64,
    anchor_raw: AtomicU64,
    anchor_mono: AtomicU64,
    mult: AtomicU64,
}

impl Slot {
    const fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            anchor_raw: AtomicU64::new(0),
            anchor_mono: AtomicU64::new(0),
            mult: AtomicU64::new(NOMINAL_MULT),
        }
    }
}

/// The published rate: two slots and which one is in use.
pub(super) struct Published {
    slots: [Slot; 2],
    active: AtomicUsize,
}

impl Published {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::new(), Slot::new()],
            active: AtomicUsize::new(0),
        }
    }

    /// The rate in use.
    pub fn load(&self) -> Rate {
        loop {
            let slot = &self.slots[self.active.load(Ordering::Acquire) & 1];
            let seq = slot.seq.load(Ordering::Acquire);
            if seq & 1 != 0 {
                // Being refilled: the writer has switched twice since.
                core::hint::spin_loop();
                continue;
            }
            let rate = Rate {
                anchor_raw: slot.anchor_raw.load(Ordering::Relaxed),
                anchor_mono: slot.anchor_mono.load(Ordering::Relaxed),
                mult: slot.mult.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            if slot.seq.load(Ordering::Relaxed) == seq {
                return rate;
            }
        }
    }

    /// Put `rate` in use. Writers must be serialized (the NTP lock).
    pub fn store(&self, rate: Rate) {
        let next = (self.active.load(Ordering::Relaxed) & 1) ^ 1;
        let slot = &self.slots[next];
        slot.seq.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        slot.anchor_raw.store(rate.anchor_raw, Ordering::Relaxed);
        slot.anchor_mono.store(rate.anchor_mono, Ordering::Relaxed);
        slot.mult.store(rate.mult, Ordering::Relaxed);
        slot.seq.fetch_add(1, Ordering::Release);
        self.active.store(next, Ordering::Release);
    }

    /// Steer from raw time `raw` on at seconds `second_length` long, if
    /// that changes the rate. Writers must be serialized (the NTP lock).
    pub fn steer(&self, raw: u64, second_length: u64) {
        let rate = self.load();
        if rate.mult != mult_of(second_length) {
            self.store(rate.rebase(raw, second_length));
        }
    }
}

static PUBLISHED: Published = Published::new();

/// The raw clock (CLOCK_MONOTONIC_RAW), in nanoseconds since boot. Host
/// unit tests use the tick-driven counter.
pub(crate) fn raw_ns() -> u64 {
    #[cfg(target_os = "none")]
    {
        crate::arch::timer::monotonic_ns()
    }
    #[cfg(not(target_os = "none"))]
    {
        super::UPTIME_MS.load(Ordering::Relaxed) * 1_000_000
    }
}

/// The steered clock (CLOCK_MONOTONIC), in nanoseconds since boot.
pub(crate) fn now_ns() -> u64 {
    PUBLISHED.load().mono_at(raw_ns())
}

/// Run the seconds that have started when the wall clock reads second
/// `wall_secs`, whose set count is `sets`. A set of the wall clock since
/// the last run starts counting again from the next second, running none.
/// Returns how many ran, and how many seconds the wall clock must move for
/// a leap second.
pub(super) fn run_due(ntp: &mut Ntp, wall_secs: i64, sets: u64) -> (u32, i64) {
    if ntp.seen_sets != sets {
        ntp.seen_sets = sets;
        ntp.next_second = wall_secs.saturating_add(1);
        return (0, 0);
    }
    let mut wall_secs = wall_secs;
    let (mut ran, mut moved) = (0, 0);
    while wall_secs >= ntp.next_second && ran < MAX_CATCH_UP {
        let secs = ntp.next_second;
        let leap = ntp.second_overflow(secs) as i64;
        ran += 1;
        moved += leap;
        wall_secs += leap;
        // An inserted second starts this second again; a deleted one
        // skips the next.
        ntp.next_second = secs + 1 + leap;
    }
    if wall_secs >= ntp.next_second {
        ntp.next_second = wall_secs.saturating_add(1);
    }
    (ran, moved)
}

/// From `ktimerd`, when [`next_due`] comes: run the seconds that started,
/// steer the clock for the next one, and step the wall clock for a leap
/// second (the timers on it follow).
pub(crate) fn service() {
    let moved_ns = super::ntp::with(|ntp| {
        let raw = raw_ns();
        let wall = super::realtime::wall_ns_at(PUBLISHED.load().mono_at(raw));
        let (ran, leap) = run_due(
            ntp,
            wall.div_euclid(NS_PER_SEC as i64),
            super::realtime::set_count(),
        );
        let moved_ns = leap.saturating_mul(NS_PER_SEC as i64);
        if leap != 0 {
            super::realtime::leap(moved_ns);
            ntp.seen_sets = super::realtime::set_count();
        }
        if ran > 0 {
            PUBLISHED.steer(raw, ntp.second_length());
        }
        moved_ns
    });
    if moved_ns != 0 {
        crate::syscall::time::clock_was_set(moved_ns);
    }
}

/// When [`service`] must next run (`monotonic_ns`): the start of the next
/// wall second, while the NTP state has something to do; `None` otherwise.
pub(crate) fn next_due() -> Option<u64> {
    super::ntp::with(|ntp| {
        ntp.is_active().then(|| {
            super::realtime::monotonic_at(ntp.next_second.saturating_mul(NS_PER_SEC as i64))
        })
    })
}

/// After adjtimex changed `ntp` (its lock held; `was_active` is whether it
/// had anything to do before): the new frequency or tick takes effect now,
/// and a state that has just become active counts its seconds from the
/// next one. Returns whether `ktimerd` must run the seconds.
pub(crate) fn adjusted(ntp: &mut Ntp, was_active: bool) -> bool {
    PUBLISHED.steer(raw_ns(), ntp.second_length());
    let active = ntp.is_active();
    if active && !was_active {
        let wall = super::realtime::now_ns();
        ntp.next_second = wall.div_euclid(NS_PER_SEC as i64).saturating_add(1);
        ntp.seen_sets = super::realtime::set_count();
    }
    active
}

#[cfg(test)]
mod tests {
    use super::{
        super::ntp::{Timex, ADJ_MAXERROR, ADJ_STATUS, STA_INS, TIME_OOP},
        *,
    };

    const NS: u64 = NS_PER_SEC;

    #[test]
    fn identity_rate_is_the_raw_clock() {
        assert_eq!(Rate::IDENTITY.mono_at(123_456_789), 123_456_789);
        // Before the anchor: the anchor.
        let r = Rate {
            anchor_raw: 1_000,
            anchor_mono: 5_000,
            mult: NOMINAL_MULT,
        };
        assert_eq!(r.mono_at(10), 5_000);
        assert_eq!(r.mono_at(1_500), 5_500);
    }

    /// 500 ppm fast: a raw second is 1.0005 s of the steered clock, and a
    /// change of rate is continuous where it is made.
    #[test]
    fn steered_rate_is_continuous() {
        let fast = (NS << 32) + (500_000u64 << 32);
        let r = Rate::IDENTITY.rebase(10 * NS, fast);
        // Within a nanosecond a second: mult is kept to 2^-32.
        let near = |a: u64, b: u64| a.abs_diff(b) <= 1;
        assert_eq!(r.mono_at(10 * NS), 10 * NS);
        assert!(near(r.mono_at(11 * NS), 11 * NS + 500_000));
        let slow = (NS << 32) - (500_000u64 << 32);
        let s = r.rebase(11 * NS, slow);
        assert_eq!(s.mono_at(11 * NS), r.mono_at(11 * NS));
        assert!(near(s.mono_at(12 * NS), 12 * NS));
        assert_eq!(mult_of(NS << 32), NOMINAL_MULT);
    }

    /// The published rate reads back what was stored, through both slots,
    /// and is left alone when the rate does not change.
    #[test]
    fn published_rate_switches_slots() {
        let p = Published::new();
        assert_eq!(p.load(), Rate::IDENTITY);
        let fast = (NS << 32) + (100_000u64 << 32);
        p.steer(5 * NS, fast);
        let r = p.load();
        assert_eq!((r.anchor_raw, r.mult), (5 * NS, mult_of(fast)));
        p.steer(6 * NS, fast);
        assert_eq!(p.load(), r, "same rate: not re-anchored");
        p.steer(7 * NS, NS << 32);
        let n = p.load();
        assert_eq!((n.anchor_raw, n.mult), (7 * NS, NOMINAL_MULT));
        assert_eq!(n.anchor_mono, r.mono_at(7 * NS));
    }

    /// Seconds run once each as the wall clock reaches them; a set of the
    /// clock restarts the count; a long gap is not run second by second
    /// beyond the catch-up limit.
    #[test]
    fn due_seconds_run_once() {
        let mut ntp = Ntp::new();
        ntp.next_second = 101;
        assert_eq!(run_due(&mut ntp, 100, 0), (0, 0));
        assert_eq!(run_due(&mut ntp, 101, 0), (1, 0));
        assert_eq!(run_due(&mut ntp, 101, 0), (0, 0));
        assert_eq!(run_due(&mut ntp, 104, 0), (3, 0));
        // The clock was set: count from the next second, running none.
        assert_eq!(run_due(&mut ntp, 9_000, 1), (0, 0));
        assert_eq!(ntp.next_second, 9_001);
        assert_eq!(run_due(&mut ntp, 100_000, 1).0, MAX_CATCH_UP);
        assert_eq!(ntp.next_second, 100_001);
    }

    /// The leap second end to end: a wall clock read every quarter second
    /// across midnight with STA_INS set reads 23:59:59 for two seconds,
    /// then midnight, and TAI moves one second; adjtimex reads TIME_OOP in
    /// the repeated second.
    #[test]
    fn inserted_leap_second_repeats_23_59_59() {
        let midnight: i64 = 20_000 * 86_400;
        let mut ntp = Ntp::new();
        let mut offset_ns = (midnight - 2) * NS as i64; // wall = mono + offset
        let mut sets = 0;
        let mut tx = Timex {
            modes: ADJ_STATUS | ADJ_MAXERROR,
            status: STA_INS,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, offset_ns, midnight - 2);
        ntp.next_second = midnight - 1;
        ntp.seen_sets = sets;

        let mut seen = alloc::vec::Vec::new();
        let mut oop = false;
        for q in 0..20u64 {
            let mono = q * NS / 4;
            let wall = mono as i64 + offset_ns;
            let (_, leap) = run_due(&mut ntp, wall.div_euclid(NS as i64), sets);
            if leap != 0 {
                offset_ns += leap * NS as i64;
                sets += 1;
                ntp.seen_sets = sets;
            }
            let wall = mono as i64 + offset_ns;
            let mut tx = Timex::default();
            oop |= ntp.adjtimex(&mut tx, wall, wall.div_euclid(NS as i64)) == TIME_OOP;
            seen.push(wall.div_euclid(NS as i64) - midnight);
        }
        // Two seconds before midnight, then 23:59:59 twice over.
        assert_eq!(
            seen,
            [-2, -2, -2, -2, -1, -1, -1, -1, -1, -1, -1, -1, 0, 0, 0, 0, 1, 1, 1, 1]
        );
        assert!(oop);
        assert_eq!(ntp.tai(), 1);
    }
}
