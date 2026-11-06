//! Interval timers: setitimer, getitimer and alarm (Linux's
//! kernel/time/itimer.c).
//!
//! - **ITIMER_REAL** runs on the monotonic clock. When it expires,
//!   `timer::ktimerd` sends the process SIGALRM (SI_KERNEL). An interval timer
//!   is armed again when SIGALRM is taken off the process's pending set, not
//!   when it fires, so a blocked signal or a slow handler never piles up
//!   expiries; the next expiry moves on by whole intervals past the time it is
//!   taken.
//! - **ITIMER_VIRTUAL** and **ITIMER_PROF** count the process's user time
//!   (SIGVTALRM) or its user and system time (SIGPROF), checked on the way back
//!   to user mode as RLIMIT_CPU is. A new value gets one tick more, so the
//!   timer never fires early; an interval timer moves on by one interval each
//!   time it fires.
//! - A fork child starts with none (`Process::new`); exec keeps them.

use core::sync::atomic::Ordering;

use super::pcb::Process;

pub const ITIMER_REAL: usize = 0;
pub const ITIMER_VIRTUAL: usize = 1;
pub const ITIMER_PROF: usize = 2;

pub const SIGALRM: usize = 14;
const SIGVTALRM: usize = 26;
const SIGPROF: usize = 27;

/// Linux's TICK_NSEC at HZ 1000, the timer tick of every architecture here.
pub const TICK_NS: u64 = 1_000_000;

/// The remaining time getitimer reports for an ITIMER_REAL that is due but
/// has not fired yet (Linux: one microsecond).
const DUE_NS: u64 = 1_000;

/// An interval timer's setting in nanoseconds (`struct itimerval`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Setting {
    /// Time to the next expiry (0: disarmed).
    pub value: u64,
    /// The period after it (0: one shot).
    pub interval: u64,
}

/// Linux's hrtimer_forward: `expires` moved on by whole `interval`s until
/// it is after `now`, with the number of intervals (0 if it already was).
pub fn forward(expires: u64, interval: u64, now: u64) -> (u64, u64) {
    if now < expires || interval == 0 {
        return (expires, 0);
    }
    let n = (now - expires) / interval + 1;
    (expires.saturating_add(n.saturating_mul(interval)), n)
}

/// ITIMER_REAL.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RealTimer {
    /// The expiry (`monotonic_ns`).
    expires: u64,
    interval: u64,
    /// Waiting to fire (Linux: its hrtimer is queued).
    armed: bool,
}

impl RealTimer {
    /// The setting getitimer reports at `now`: an armed timer that is due
    /// has a microsecond left, one that is not armed has none.
    pub fn get(&self, now: u64) -> Setting {
        let value = if self.armed {
            self.expires.saturating_sub(now).max(DUE_NS)
        } else {
            0
        };
        Setting {
            value,
            interval: self.interval,
        }
    }

    /// Set to `new` at `now`. Returns the old setting and the expiry to
    /// wait for (None: disarmed); a zero value disarms the timer and drops
    /// its interval.
    pub fn set(&mut self, new: Setting, now: u64) -> (Setting, Option<u64>) {
        let old = self.get(now);
        if new.value == 0 {
            self.armed = false;
            self.interval = 0;
            return (old, None);
        }
        self.expires = now.saturating_add(new.value);
        self.interval = new.interval;
        self.armed = true;
        (old, Some(self.expires))
    }

    /// An expiry came at `now`: whether to send SIGALRM. An expiry the
    /// timer no longer waits for (set again since) sends nothing.
    pub fn fire(&mut self, now: u64) -> bool {
        if self.armed && now >= self.expires {
            self.armed = false;
            true
        } else {
            false
        }
    }

    /// SIGALRM was taken at `now`: an interval timer that fired waits
    /// again, for its expiry moved on past `now`. Returns that expiry.
    pub fn rearm(&mut self, now: u64) -> Option<u64> {
        if self.armed || self.interval == 0 {
            return None;
        }
        self.expires = forward(self.expires, self.interval, now).0;
        self.armed = true;
        Some(self.expires)
    }
}

/// ITIMER_VIRTUAL or ITIMER_PROF.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimer {
    /// The expiry in the process's CPU time (0: disarmed).
    expires: u64,
    interval: u64,
}

impl CpuTimer {
    /// The setting getitimer reports when the process has used `now`: a
    /// timer past due reports a tick (it fires at the next check).
    pub fn get(&self, now: u64) -> Setting {
        let value = match self.expires {
            0 => 0,
            e if e < now => TICK_NS,
            e => e - now,
        };
        Setting {
            value,
            interval: self.interval,
        }
    }

    /// Set to `new` when the process has used `now`; returns the old
    /// setting. The interval is kept even with a zero value, as Linux's
    /// set_cpu_itimer keeps it.
    pub fn set(&mut self, new: Setting, now: u64) -> Setting {
        let value = match self.expires {
            0 => 0,
            e if e <= now => TICK_NS,
            e => e - now,
        };
        let old = Setting {
            value,
            interval: self.interval,
        };
        self.expires = if new.value == 0 {
            0
        } else {
            now.saturating_add(new.value).saturating_add(TICK_NS)
        };
        self.interval = new.interval;
        old
    }

    /// Whether the timer fires now that the process has used `now`; an
    /// interval timer moves on by one interval, a one-shot one disarms.
    pub fn check(&mut self, now: u64) -> bool {
        if self.expires == 0 || now < self.expires {
            return false;
        }
        self.expires = if self.interval == 0 {
            0
        } else {
            self.expires.saturating_add(self.interval)
        };
        true
    }

    fn armed(&self) -> bool {
        self.expires != 0
    }
}

/// A process's three interval timers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Itimers {
    real: RealTimer,
    /// ITIMER_VIRTUAL, ITIMER_PROF.
    cpu: [CpuTimer; 2],
}

/// The CPU times the CPU timers count: user (VIRTUAL), and user and
/// system (PROF).
fn cpu_now(process: &Process) -> [u64; 2] {
    let (times, (user, _)) = process.cpu_split();
    [user, times.runtime_ns]
}

/// getitimer: `which`'s setting (None: no such timer, EINVAL).
pub fn get(process: &Process, which: usize) -> Option<Setting> {
    match which {
        ITIMER_REAL => Some(
            process
                .itimers
                .lock()
                .real
                .get(crate::timer::monotonic_ns()),
        ),
        ITIMER_VIRTUAL | ITIMER_PROF => {
            let now = cpu_now(process)[which - 1];
            Some(process.itimers.lock().cpu[which - 1].get(now))
        }
        _ => None,
    }
}

/// setitimer: set `which` to `new`, returning its old setting (None: no
/// such timer, EINVAL; or no timer thread could be started, EAGAIN).
pub fn set(process: &Process, which: usize, new: Setting) -> Result<Setting, SetError> {
    match which {
        ITIMER_REAL => {
            crate::timer::ktimerd::start().map_err(|_| SetError::NoThread)?;
            let key = crate::timer::ktimerd::Key::Real(process.pid.0);
            let mut timers = process.itimers.lock();
            let (old, expiry) = timers.real.set(new, crate::timer::monotonic_ns());
            // Armed under the lock, so two calls arm in the order they set.
            match expiry {
                Some(at) => crate::timer::ktimerd::arm(key, at),
                None => crate::timer::ktimerd::cancel(key),
            }
            Ok(old)
        }
        ITIMER_VIRTUAL | ITIMER_PROF => {
            let now = cpu_now(process)[which - 1];
            let mut timers = process.itimers.lock();
            let old = timers.cpu[which - 1].set(new, now);
            process
                .cpu_itimers_armed
                .store(timers.cpu.iter().any(CpuTimer::armed), Ordering::Release);
            Ok(old)
        }
        _ => Err(SetError::Invalid),
    }
}

/// Why setitimer failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetError {
    /// No such timer (EINVAL).
    Invalid,
    /// The timer thread could not be started (EAGAIN).
    NoThread,
}

/// ktimerd: process `pid`'s ITIMER_REAL expired at `now`.
pub fn real_expired(pid: u64, now: u64) {
    let Some(process) = super::find_process(super::ProcessId(pid)) else {
        return;
    };
    let fire = process.itimers.lock().real.fire(now);
    if fire {
        super::signals::notify(&process, SIGALRM);
    }
}

/// SIGALRM was taken off `process`'s pending set (Linux's
/// posixtimer_rearm_itimer, from dequeue_signal): an interval
/// ITIMER_REAL waits for its next expiry.
pub fn sigalrm_taken(process: &Process) {
    let mut timers = process.itimers.lock();
    if let Some(at) = timers.real.rearm(crate::timer::monotonic_ns()) {
        crate::timer::ktimerd::arm(crate::timer::ktimerd::Key::Real(process.pid.0), at);
    }
}

/// On the way back to user mode: send SIGVTALRM and SIGPROF for the CPU
/// timers the process's CPU time has reached.
pub fn check_cpu(process: &Process) {
    if !process.cpu_itimers_armed.load(Ordering::Acquire) {
        return;
    }
    let now = cpu_now(process);
    let fired = {
        let mut timers = process.itimers.lock();
        let fired = [timers.cpu[0].check(now[0]), timers.cpu[1].check(now[1])];
        process
            .cpu_itimers_armed
            .store(timers.cpu.iter().any(CpuTimer::armed), Ordering::Release);
        fired
    };
    for (sig, fired) in [(SIGVTALRM, fired[0]), (SIGPROF, fired[1])] {
        if fired {
            super::signals::notify(process, sig);
        }
    }
}

/// alarm's return value for the ITIMER_REAL setting it replaced: whole
/// seconds, rounded up from half a second, and never 0 for a timer that
/// had time left (Linux's alarm_setitimer).
pub fn alarm_seconds(old: Setting) -> u64 {
    let (secs, nsec) = (old.value / 1_000_000_000, old.value % 1_000_000_000);
    if (secs == 0 && nsec != 0) || nsec >= 500_000_000 {
        secs + 1
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    /// hrtimer_forward: past `now` by whole intervals, counting them.
    #[test]
    fn forward_moves_past_now_by_whole_intervals() {
        assert_eq!(forward(100, 10, 50), (100, 0));
        assert_eq!(forward(100, 10, 100), (110, 1));
        assert_eq!(forward(100, 10, 105), (110, 1));
        assert_eq!(forward(100, 10, 110), (120, 2));
        assert_eq!(forward(100, 10, 139), (140, 4));
        assert_eq!(forward(100, 0, 500), (100, 0));
    }

    /// ITIMER_REAL: set, read back, fire once, re-armed only when its
    /// signal is taken, and a stale expiry fires nothing.
    #[test]
    fn real_timer_rearms_when_its_signal_is_taken() {
        let mut t = RealTimer::default();
        let new = Setting {
            value: 50 * MS,
            interval: 20 * MS,
        };
        assert_eq!(t.set(new, 1000 * MS), (Setting::default(), Some(1050 * MS)));
        assert_eq!(
            t.get(1010 * MS),
            Setting {
                value: 40 * MS,
                interval: 20 * MS
            }
        );
        assert!(!t.fire(1049 * MS));
        // Due but not yet fired: a microsecond left.
        assert_eq!(t.get(1060 * MS).value, DUE_NS);
        assert!(t.fire(1060 * MS));
        assert!(!t.fire(1061 * MS));
        assert_eq!(t.get(1061 * MS).value, 0);
        // Taken late: the next expiry is past the time it is taken.
        assert_eq!(t.rearm(1095 * MS), Some(1110 * MS));
        assert_eq!(t.rearm(1096 * MS), None);
        // A zero value disarms and drops the interval.
        let (old, at) = t.set(Setting::default(), 1100 * MS);
        assert_eq!(
            (old, at),
            (
                Setting {
                    value: 10 * MS,
                    interval: 20 * MS
                },
                None
            )
        );
        assert!(!t.fire(2000 * MS));
        assert_eq!(t.rearm(2000 * MS), None);
    }

    /// A one-shot ITIMER_REAL is not re-armed.
    #[test]
    fn one_shot_real_timer_stays_disarmed() {
        let mut t = RealTimer::default();
        t.set(
            Setting {
                value: MS,
                interval: 0,
            },
            0,
        );
        assert!(t.fire(MS));
        assert_eq!(t.rearm(5 * MS), None);
    }

    /// CPU timers: a tick more than asked, a tick reported once due, one
    /// interval on per expiry, and the interval kept with a zero value.
    #[test]
    fn cpu_timer_follows_set_cpu_itimer() {
        let mut t = CpuTimer::default();
        let new = Setting {
            value: 10 * MS,
            interval: 5 * MS,
        };
        assert_eq!(t.set(new, 100 * MS), Setting::default());
        assert_eq!(t.get(100 * MS).value, 10 * MS + TICK_NS);
        assert!(!t.check(110 * MS));
        assert!(t.check(111 * MS));
        assert_eq!(t.get(111 * MS).value, 5 * MS);
        assert!(t.check(200 * MS));
        assert_eq!(t.get(200 * MS).value, TICK_NS);
        let old = t.set(
            Setting {
                value: 0,
                interval: 7 * MS,
            },
            300 * MS,
        );
        assert_eq!(old.value, TICK_NS);
        assert_eq!(
            t.get(300 * MS),
            Setting {
                value: 0,
                interval: 7 * MS
            }
        );
        assert!(!t.check(u64::MAX));
    }

    /// alarm rounds the old time up from half a second and never reports
    /// 0 for a timer with time left.
    #[test]
    fn alarm_seconds_round_as_linux() {
        let s = |value| Setting { value, interval: 0 };
        assert_eq!(alarm_seconds(s(0)), 0);
        assert_eq!(alarm_seconds(s(1)), 1);
        assert_eq!(alarm_seconds(s(2_499_999_999)), 2);
        assert_eq!(alarm_seconds(s(2_500_000_000)), 3);
        assert_eq!(alarm_seconds(s(3_000_000_000)), 3);
    }
}
