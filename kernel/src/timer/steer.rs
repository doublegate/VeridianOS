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
//! The rate is published under a sequence count (Linux's tk_core.seq),
//! and a reader reads the raw clock inside the same section: a rate change
//! that overlaps the read makes the reader retry, so it never extrapolates
//! the old rate past where the new one was anchored, and CLOCK_MONOTONIC
//! never goes back. The writer (holding the NTP lock) samples the raw clock
//! after opening its section, with interrupts off on its CPU, so no reader
//! can wait on a writer it interrupted. A new rate is anchored where the
//! old one reads at that sample, so the clock is continuous.
//!
//! Once a wall second (UTC) starts, `ktimerd` runs [`service`]: Linux's
//! second_overflow for each second that started (the leap second state
//! machine, maxerror, the next second's phase), and steps the wall clock
//! for a leap second. It only wakes for this while the NTP state has
//! something to do (`Ntp::is_active`); an idle clock costs nothing.

use core::sync::atomic::{fence, AtomicU64, Ordering};

use super::ntp::Ntp;

const NS_PER_SEC: u64 = 1_000_000_000;
/// `mult` for a nominal second: one ns per raw ns.
pub(super) const NOMINAL_MULT: u64 = 1 << 32;
/// The most late seconds one [`service`] runs; the rest before the current
/// second are dropped (only after the thread could not run for that long).
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

/// The published rate, under a sequence count (odd while it changes).
pub(super) struct Published {
    seq: AtomicU64,
    anchor_raw: AtomicU64,
    anchor_mono: AtomicU64,
    mult: AtomicU64,
}

impl Published {
    pub const fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            anchor_raw: AtomicU64::new(0),
            anchor_mono: AtomicU64::new(0),
            mult: AtomicU64::new(NOMINAL_MULT),
        }
    }

    /// The rate as stored. Only for the writer (serialized), which needs no
    /// sequence check.
    fn current(&self) -> Rate {
        Rate {
            anchor_raw: self.anchor_raw.load(Ordering::Relaxed),
            anchor_mono: self.anchor_mono.load(Ordering::Relaxed),
            mult: self.mult.load(Ordering::Relaxed),
        }
    }

    /// The rate in use and a raw time `raw` read under it: no rate was
    /// published between the two (Linux's ktime_get read loop).
    pub fn read(&self, raw: impl Fn() -> u64) -> (Rate, u64) {
        loop {
            let seq = self.seq.load(Ordering::Acquire);
            if seq & 1 != 0 {
                // A writer on another CPU (interrupts are off on its own).
                core::hint::spin_loop();
                continue;
            }
            let rate = self.current();
            let now = raw();
            fence(Ordering::SeqCst);
            if self.seq.load(Ordering::Relaxed) == seq {
                return (rate, now);
            }
        }
    }

    /// The steered time when the raw clock `raw` is read.
    pub fn now(&self, raw: impl Fn() -> u64) -> u64 {
        let (rate, now) = self.read(raw);
        rate.mono_at(now)
    }

    /// The rate in use.
    #[cfg(test)]
    pub fn load(&self) -> Rate {
        self.read(|| 0).0
    }

    /// Steer at seconds `second_length` long from the raw time `raw` reads
    /// now, if that changes the rate; the raw clock is read inside the
    /// write section, so a reader that saw the old rate read it earlier.
    /// Writers must be serialized (the NTP lock), and readers must not run
    /// on this CPU meanwhile (interrupts off: [`steer`]).
    pub fn steer(&self, raw: impl Fn() -> u64, second_length: u64) {
        let rate = self.current();
        if rate.mult == mult_of(second_length) {
            return;
        }
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::SeqCst);
        let next = rate.rebase(raw(), second_length);
        self.anchor_raw.store(next.anchor_raw, Ordering::Relaxed);
        self.anchor_mono.store(next.anchor_mono, Ordering::Relaxed);
        self.mult.store(next.mult, Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }
}

static PUBLISHED: Published = Published::new();

/// Steer the system clock at seconds `second_length` long from now (the
/// NTP lock held), with interrupts off so the timer interrupt cannot read
/// the clock while the rate changes on this CPU.
fn steer(second_length: u64) {
    #[cfg(target_os = "none")]
    let _irq = crate::arch::disable_interrupts();
    PUBLISHED.steer(raw_ns, second_length);
}

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
    PUBLISHED.now(raw_ns)
}

/// Run the seconds that have started when the wall clock reads second
/// `wall_secs`, whose set count is `sets`. A set of the wall clock since
/// the last run starts counting again from the next second, running none.
/// A second that has already ended when it is run (ktimerd ran late) ran
/// at the rate published before it, so it is run as late: it delivers what
/// that rate did (`Ntp::second_overflow`). After [`MAX_CATCH_UP`] of them
/// the rest before the current second are dropped, and the current one
/// runs on time. Returns how many ran, and how many seconds the wall clock
/// must move for a leap second.
pub(super) fn run_due(ntp: &mut Ntp, wall_secs: i64, sets: u64) -> (u32, i64) {
    if ntp.seen_sets != sets {
        ntp.seen_sets = sets;
        ntp.next_second = wall_secs.saturating_add(1);
        ntp.skipped_to(ntp.next_second);
        return (0, 0);
    }
    let mut wall_secs = wall_secs;
    let (mut ran, mut moved) = (0, 0);
    while wall_secs >= ntp.next_second && ran < MAX_CATCH_UP {
        let secs = ntp.next_second;
        let leap = ntp.second_overflow(secs, secs < wall_secs) as i64;
        ran += 1;
        moved += leap;
        wall_secs += leap;
        // An inserted second starts this second again; a deleted one
        // skips the next.
        ntp.next_second = secs.saturating_add(1 + leap);
    }
    if wall_secs >= ntp.next_second {
        // Beyond the catch-up: the seconds before this one are dropped, and
        // this one runs on time, so its phase is its own rather than the
        // one from before the stall.
        ntp.skipped_to(wall_secs);
        let leap = ntp.second_overflow(wall_secs, false) as i64;
        ran += 1;
        moved += leap;
        ntp.next_second = wall_secs.saturating_add(1 + leap);
    }
    (ran, moved)
}

/// From `ktimerd`, when [`next_due`] comes: run the seconds that started,
/// steer the clock for the next one, and step the wall clock for a leap
/// second (the timers on it follow). The wall clock is only set under the
/// NTP lock (`timer::set_wall_clock`, ADJ_SETOFFSET), so the time and the
/// set count read here belong together.
pub(crate) fn service() {
    let moved_ns = super::ntp::with(|ntp| {
        let wall = super::realtime::now_ns();
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
            steer(ntp.second_length());
        }
        moved_ns
    });
    if moved_ns != 0 {
        crate::syscall::time::clock_was_set(moved_ns);
    }
}

/// When [`service`] must next run (`monotonic_ns`): the start of the next
/// wall second, while the NTP state has something to do; `None` otherwise.
/// After a set of the wall clock it has not seen, now: `next_second` still
/// counts from the old date (a step back would else put the next run off
/// by the length of the step).
pub(crate) fn next_due() -> Option<u64> {
    super::ntp::with(|ntp| {
        ntp.is_active().then(|| {
            if ntp.seen_sets != super::realtime::set_count() {
                0
            } else {
                super::realtime::monotonic_at(ntp.next_second.saturating_mul(NS_PER_SEC as i64))
            }
        })
    })
}

/// After adjtimex changed `ntp` (its lock held; `was_active` is whether it
/// had anything to do before): the new frequency or tick takes effect now,
/// and a state that has just become active counts its seconds from the
/// next one. Returns whether `ktimerd` must run the seconds.
pub(crate) fn adjusted(ntp: &mut Ntp, was_active: bool) -> bool {
    steer(ntp.second_length());
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
        super::ntp::{
            Timex, ADJ_ADJTIME, ADJ_MAXERROR, ADJ_OFFSET_SINGLESHOT, ADJ_STATUS, STA_INS, TIME_OOP,
        },
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

    /// The published rate reads back what was stored, and is left alone
    /// when the rate does not change.
    #[test]
    fn published_rate_reads_back() {
        let p = Published::new();
        assert_eq!(p.load(), Rate::IDENTITY);
        let fast = (NS << 32) + (100_000u64 << 32);
        p.steer(|| 5 * NS, fast);
        let r = p.load();
        assert_eq!((r.anchor_raw, r.mult), (5 * NS, mult_of(fast)));
        p.steer(|| 6 * NS, fast);
        assert_eq!(p.load(), r, "same rate: not re-anchored");
        p.steer(|| 7 * NS, NS << 32);
        let n = p.load();
        assert_eq!((n.anchor_raw, n.mult), (7 * NS, NOMINAL_MULT));
        assert_eq!(n.anchor_mono, r.mono_at(7 * NS));
    }

    /// A reader that took the old (faster) rate and reads the raw clock
    /// after a slower rate was anchored retries under the new one: it does
    /// not extrapolate the old rate past the new anchor, which a later read
    /// would then come in under (CLOCK_MONOTONIC going back).
    #[test]
    fn a_rate_change_during_a_read_is_not_extrapolated() {
        let p = Published::new();
        let fast = (NS << 32) + (500_000u64 << 32);
        p.steer(|| 0, fast);
        let old = p.load();
        let changed = core::cell::Cell::new(false);
        // The raw clock read inside the section: the first time, a writer
        // anchors nominal seconds at 10 s first (as on another CPU).
        let t = p.now(|| {
            if !changed.replace(true) {
                p.steer(|| 10 * NS, NS << 32);
            }
            20 * NS
        });
        let new = p.load();
        assert_eq!(new.anchor_raw, 10 * NS);
        assert_eq!(t, new.mono_at(20 * NS));
        assert!(t < old.mono_at(20 * NS), "the old rate was not used");
        // And the next read, later, is not behind it.
        assert!(p.now(|| 20 * NS + 1) >= t);
    }

    /// A leap second armed for a midnight that is skipped over (ktimerd
    /// could not run past its catch-up, or the clock was set past it) is
    /// armed again for the coming midnight, not left armed for a second
    /// that will never be run (adjtimex would read TIME_OOP for ever).
    #[test]
    fn a_leap_second_skipped_over_is_armed_again() {
        const DAY: i64 = 86_400;
        let midnight: i64 = 20_000 * DAY;
        let armed = || {
            let mut ntp = Ntp::new();
            let mut tx = Timex {
                modes: ADJ_STATUS | ADJ_MAXERROR,
                status: STA_INS,
                ..Timex::default()
            };
            ntp.adjtimex(&mut tx, 0, midnight - 100);
            ntp.next_second = midnight - 99;
            assert_eq!(run_due(&mut ntp, midnight - 99, 0), (1, 0));
            ntp
        };
        let read = |ntp: &mut Ntp, secs: i64| {
            let mut tx = Timex::default();
            let state = ntp.adjtimex(&mut tx, secs * NS as i64, secs);
            (state, tx.time_sec)
        };

        // ktimerd stalled across midnight, beyond its catch-up.
        let mut ntp = armed();
        assert_eq!(run_due(&mut ntp, midnight + 1_000, 0).0, MAX_CATCH_UP + 1);
        assert_eq!(run_due(&mut ntp, midnight + 1_001, 0), (1, 0));
        let (state, secs) = read(&mut ntp, midnight + 1_001);
        assert_ne!(state, TIME_OOP);
        assert_eq!(secs, midnight + 1_001, "no leap applied to the reading");
        // The next midnight inserts it (ktimerd running again by then).
        ntp.next_second = midnight + DAY - 3;
        let leaps: i64 = (midnight + DAY - 3..=midnight + DAY)
            .map(|s| run_due(&mut ntp, s, 0).1)
            .sum();
        assert_eq!(leaps, -1);

        // The clock set past midnight (which clears the state, as
        // `timer::set_wall_clock`): the next midnight inserts it.
        let mut ntp = armed();
        ntp.clear();
        assert_eq!(run_due(&mut ntp, midnight + DAY - 5, 1), (0, 0));
        let leaps: i64 = (midnight + DAY - 4..=midnight + DAY)
            .map(|s| run_due(&mut ntp, s, 1).1)
            .sum();
        assert_eq!(leaps, -1);
    }

    /// Seconds run late (ktimerd did not run for three boundaries) ran at
    /// the rate before them, nominal here, so an adjtime() slew set before
    /// them has delivered nothing: none of it is lost. The second run on
    /// time starts delivering it.
    #[test]
    fn late_seconds_keep_an_undelivered_slew() {
        let mut ntp = Ntp::new();
        let mut tx = Timex {
            modes: super::super::ntp::ADJ_ADJTIME,
            offset: 1_000,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, 0, 100);
        ntp.next_second = 101;
        assert_eq!(run_due(&mut ntp, 103, 0), (3, 0));
        let mut tx = Timex {
            modes: super::super::ntp::ADJ_ADJTIME | super::super::ntp::ADJ_OFFSET_READONLY,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, 0, 103);
        // 500 us of it in the second now starting, 500 us left after it.
        assert_eq!(tx.offset, 500);
        assert!(ntp.second_length() > NS << 32);
        // The next second, late again: it ran at this second's rate, so it
        // delivered the other 500 us.
        assert_eq!(run_due(&mut ntp, 105, 0), (2, 0));
        let mut tx = Timex {
            modes: super::super::ntp::ADJ_ADJTIME | super::super::ntp::ADJ_OFFSET_READONLY,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, 0, 105);
        assert_eq!(tx.offset, 0);
        assert_eq!(ntp.second_length(), NS << 32);
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
        // Beyond the catch-up the seconds before this one are dropped, and
        // this one runs on time.
        assert_eq!(run_due(&mut ntp, 100_000, 1).0, MAX_CATCH_UP + 1);
        assert_eq!(ntp.next_second, 100_001);
        assert_eq!(run_due(&mut ntp, 100_000, 1), (0, 0));
    }

    /// The second ktimerd wakes in after a stall beyond the catch-up gets
    /// its own phase, not the one from before the stall: a 500 us slew is
    /// delivered by second 1, and the late seconds that ran at its rate
    /// delivered it again, so the current second gives some back.
    #[test]
    fn the_second_after_a_long_stall_is_steered_afresh() {
        let mut ntp = Ntp::new();
        let mut tx = Timex {
            modes: ADJ_ADJTIME | ADJ_OFFSET_SINGLESHOT,
            offset: 500,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, 0, 0);
        ntp.next_second = 1;
        assert_eq!(run_due(&mut ntp, 1, 0), (1, 0));
        let normal = Ntp::new().second_length();
        assert!(ntp.second_length() > normal, "second 1 delivers the slew");
        assert_eq!(run_due(&mut ntp, 1_000, 0).0, MAX_CATCH_UP + 1);
        assert!(ntp.second_length() < normal, "second 1000 gives back");
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

    /// An idle NTP state needs ktimerd for no second; one that becomes
    /// active starts counting at the next second, once. The NTP state here
    /// is a local one at the default second length, so the rate it
    /// publishes is the identity, as the system's (which no host test
    /// changes) is.
    #[test]
    fn adjusted_arms_the_next_second_when_activated() {
        assert_eq!(next_due(), None);
        let mut ntp = Ntp::new();
        assert!(!adjusted(&mut ntp, false));
        assert_eq!(ntp.next_second, 0);
        let mut tx = Timex {
            modes: ADJ_STATUS,
            status: STA_INS,
            ..Timex::default()
        };
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(ntp.second_length(), Ntp::new().second_length());
        assert!(adjusted(&mut ntp, false));
        // Already active: an adjustment keeps the second being counted.
        let counting = ntp.next_second.wrapping_add(5);
        ntp.next_second = counting;
        assert!(adjusted(&mut ntp, true));
        assert_eq!(ntp.next_second, counting);
    }
}
