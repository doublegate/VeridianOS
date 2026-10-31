//! POSIX per-process timers: timer_create, timer_settime, timer_gettime,
//! timer_getoverrun and timer_delete (Linux's kernel/time/posix-timers.c and
//! the timer parts of kernel/signal.c).
//!
//! - A timer runs on the wall clock (CLOCK_REALTIME armed with TIMER_ABSTIME; a
//!   relative one runs on the monotonic clock, as on Linux, so setting the
//!   clock does not move it), the monotonic clock, or the CPU clock of the
//!   process or one of its threads. Clock timers expire in `timer::ktimerd`;
//!   CPU-clock timers are checked on the way back to user mode, as the CPU
//!   interval timers are.
//! - Each timer has at most one signal queued (SI_TIMER, `si_tid` the timer
//!   id). An interval timer is armed again when that signal is taken, not when
//!   it fires; the intervals that passed in between are its overrun. A signal
//!   from a timer set again or deleted since it was queued is dropped when it
//!   is taken (`signal_seq`). A periodic timer whose signal is ignored is
//!   parked until the signal stops being ignored.
//! - A fork child has none (`Process::new`); exec and exit delete them. Each
//!   timer is charged to its creator's RLIMIT_SIGPENDING, as Linux charges its
//!   preallocated signal.
//!
//! Lock order: `Process::posix_timers`, then the signal queues.

use alloc::collections::BTreeMap;
use core::sync::atomic::Ordering;

use super::pcb::Process;

pub const SIGEV_SIGNAL: i32 = 0;
pub const SIGEV_NONE: i32 = 1;
pub const SIGEV_THREAD: i32 = 2;
pub const SIGEV_THREAD_ID: i32 = 4;

/// timer_settime flag: `it_value` is a time on the clock, not an interval.
pub const TIMER_ABSTIME: usize = 1;

/// The clock a timer runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerClock {
    /// Wall time (`timer::realtime::now_ns`).
    Wall,
    /// Time since boot (`timer::monotonic_ns`).
    Mono,
    /// The process's CPU time: user only (`user`) or user and system.
    ProcessCpu { user: bool },
    /// One of its threads' CPU time.
    ThreadCpu { tid: u64, user: bool },
}

impl TimerClock {
    fn is_cpu(self) -> bool {
        matches!(self, Self::ProcessCpu { .. } | Self::ThreadCpu { .. })
    }
}

/// How a timer tells the process it expired (`struct sigevent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    /// SIGEV_NONE: nothing; timer_gettime still counts down.
    None,
    /// SIGEV_SIGNAL (and SIGEV_THREAD, which the C library builds on it):
    /// `signo` with `value` to the process, or to thread `thread`
    /// (SIGEV_THREAD_ID).
    Signal {
        signo: usize,
        value: u64,
        thread: Option<u64>,
    },
}

/// A timer setting in nanoseconds (`struct itimerspec`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spec {
    /// The time to the next expiry, or the expiry itself with
    /// TIMER_ABSTIME (0: disarmed).
    pub value: i64,
    /// The period after it (0: one shot).
    pub interval: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Disarmed,
    /// Waiting to expire.
    Armed,
    /// An interval timer whose signal is queued (or parked): armed again
    /// when it is taken.
    RequeuePending,
}

/// What a set timer waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// Nothing (disarmed, or SIGEV_NONE).
    Cancel,
    /// Its expiry, on its clock.
    At(i64),
    /// It is due already: fire it now.
    FireNow,
}

/// What an expiry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fire {
    /// Queue its signal.
    Signal,
    /// Not due yet (its clock was set back): wait for this expiry.
    Wait(i64),
    /// Nothing (disarmed or set again since).
    Nothing,
}

/// What taking a timer's signal does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The timer was set again or deleted since: drop the signal.
    Drop,
    /// Deliver it; an interval timer reports `overrun` and waits for
    /// `rearm`.
    Deliver {
        overrun: Option<i32>,
        rearm: Option<i64>,
    },
}

/// Linux's hrtimer_forward: `expires` moved on by whole `interval`s until
/// it is after `now`, with the number of intervals (0 if it already was).
fn forward(expires: i64, interval: i64, now: i64) -> (i64, i64) {
    if now < expires || interval <= 0 {
        return (expires, 0);
    }
    let n = (now as i128 - expires as i128) / interval as i128 + 1;
    let next = expires as i128 + n * interval as i128;
    (
        next.min(i64::MAX as i128) as i64,
        n.min(i64::MAX as i128) as i64,
    )
}

/// One timer (Linux's `struct k_itimer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosixTimer {
    /// The clock it was created on.
    clock: TimerClock,
    /// The clock its current setting runs on: a relative CLOCK_REALTIME
    /// setting runs on the monotonic clock.
    base: TimerClock,
    notify: Notify,
    /// Tells this timer's queued signals from those of an earlier timer
    /// with the same id.
    serial: u64,
    /// The next expiry on `base`.
    expires: i64,
    interval: i64,
    status: Status,
    /// Bumped by every setting and every delivery: a signal queued under
    /// another value is stale.
    signal_seq: u32,
    /// `signal_seq` when the signal was last queued.
    sigqueue_seq: u32,
    /// Intervals passed since the signal was queued, less one.
    overrun: i64,
    /// The overrun of the last delivered signal (timer_getoverrun).
    overrun_last: i64,
    /// A periodic timer whose signal was ignored: queued again when the
    /// signal stops being ignored.
    parked: bool,
    /// The real user charged for it (RLIMIT_SIGPENDING).
    uid: u32,
}

impl PosixTimer {
    pub fn new(clock: TimerClock, notify: Notify, serial: u64, uid: u32) -> Self {
        Self {
            clock,
            base: clock,
            notify,
            serial,
            expires: 0,
            interval: 0,
            status: Status::Disarmed,
            signal_seq: 0,
            sigqueue_seq: 0,
            overrun: -1,
            overrun_last: 0,
            parked: false,
            uid,
        }
    }

    fn sig_none(&self) -> bool {
        self.notify == Notify::None
    }

    /// timer_gettime when `base` reads `now` (Linux's common_timer_get):
    /// an interval timer that is not waiting (its signal is pending, or it
    /// is SIGEV_NONE) is moved on as if armed again now; a timer that is
    /// due but whose signal is not yet taken has a nanosecond left; an
    /// expired one-shot SIGEV_NONE timer has none.
    pub fn get(&mut self, now: i64) -> Spec {
        let mut spec = Spec {
            value: 0,
            interval: self.interval,
        };
        if self.interval == 0 && self.status == Status::Disarmed && !self.sig_none() {
            return spec;
        }
        if self.interval != 0 && self.status != Status::Armed {
            let (expires, n) = forward(self.expires, self.interval, now);
            self.expires = expires;
            self.overrun = self.overrun.saturating_add(n);
        }
        let remaining = self.expires.saturating_sub(now);
        if remaining > 0 {
            spec.value = remaining;
        } else if !self.sig_none() {
            spec.value = 1;
        }
        spec
    }

    /// timer_settime on `base`, which reads `now` (Linux's
    /// common_timer_set): any signal already queued becomes stale, the
    /// overrun starts again, a zero value disarms (and drops the interval).
    pub fn set(&mut self, new: Spec, absolute: bool, base: TimerClock, now: i64) -> Arm {
        self.signal_seq = self.signal_seq.wrapping_add(1);
        self.status = Status::Disarmed;
        self.parked = false;
        self.base = base;
        self.interval = if new.value != 0 { new.interval } else { 0 };
        self.overrun_last = 0;
        self.overrun = -1;
        if new.value == 0 {
            self.expires = 0;
            return Arm::Cancel;
        }
        self.expires = if absolute {
            new.value
        } else {
            now.saturating_add(new.value)
        };
        if self.sig_none() {
            return Arm::Cancel;
        }
        self.status = Status::Armed;
        if self.expires <= now {
            Arm::FireNow
        } else {
            Arm::At(self.expires)
        }
    }

    /// The expiry came, `base` reading `now` (Linux's
    /// posix_timer_queue_signal): an interval timer's signal is pending
    /// until taken, a one-shot timer is done.
    pub fn fire(&mut self, now: i64) -> Fire {
        if self.status != Status::Armed {
            return Fire::Nothing;
        }
        if now < self.expires {
            return Fire::Wait(self.expires);
        }
        self.status = if self.interval != 0 {
            Status::RequeuePending
        } else {
            Status::Disarmed
        };
        self.sigqueue_seq = self.signal_seq;
        Fire::Signal
    }

    /// Its signal, queued as `serial`, was taken with `base` reading `now`
    /// (Linux's __posixtimer_deliver_signal): a stale one is dropped; an
    /// interval timer counts the intervals that passed since it fired and
    /// waits for the next expiry after `now`.
    pub fn deliver(&mut self, serial: u64, now: i64) -> Delivery {
        if serial != self.serial || self.signal_seq != self.sigqueue_seq {
            return Delivery::Drop;
        }
        if self.interval == 0 || self.status != Status::RequeuePending {
            return Delivery::Deliver {
                overrun: None,
                rearm: None,
            };
        }
        let (expires, n) = forward(self.expires, self.interval, now);
        self.expires = expires;
        self.overrun = self.overrun.saturating_add(n);
        self.overrun_last = self.overrun;
        self.overrun = -1;
        self.signal_seq = self.signal_seq.wrapping_add(1);
        self.status = Status::Armed;
        Delivery::Deliver {
            overrun: Some(self.overrun_to_int()),
            rearm: Some(expires),
        }
    }

    /// timer_getoverrun: the last delivered signal's overrun, at most
    /// INT_MAX.
    pub fn overrun_to_int(&self) -> i32 {
        self.overrun_last.min(i32::MAX as i64) as i32
    }

    /// Whether it waits on a CPU clock.
    fn armed_on_cpu(&self) -> bool {
        self.status == Status::Armed && self.base.is_cpu()
    }
}

/// A process's timers (Linux's `signal_struct::posix_timers`).
#[derive(Debug, Default)]
pub struct PosixTimers {
    timers: BTreeMap<i32, PosixTimer>,
    /// Where the search for a free id starts (Linux's
    /// next_posix_timer_id): ids are handed out in turn, 0..=INT_MAX, and
    /// wrap.
    next_id: u32,
    /// The last serial handed out.
    serial: u64,
}

impl PosixTimers {
    /// Add a timer made by `make` from its id and serial; its id, or None
    /// when every id is in use (EAGAIN).
    pub fn add(&mut self, make: impl FnOnce(i32, u64) -> PosixTimer) -> Option<i32> {
        // Of len + 1 ids in a row at least one is free.
        for _ in 0..=self.timers.len() {
            let id = (self.next_id & i32::MAX as u32) as i32;
            self.next_id = self.next_id.wrapping_add(1);
            if !self.timers.contains_key(&id) {
                self.serial += 1;
                self.timers.insert(id, make(id, self.serial));
                return Some(id);
            }
        }
        None
    }

    pub fn get_mut(&mut self, id: i32) -> Option<&mut PosixTimer> {
        self.timers.get_mut(&id)
    }

    pub fn remove(&mut self, id: i32) -> Option<PosixTimer> {
        self.timers.remove(&id)
    }

    fn armed_on_cpu(&self) -> bool {
        self.timers.values().any(PosixTimer::armed_on_cpu)
    }
}

/// Why a timer call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerError {
    /// No such timer, or a bad argument (EINVAL).
    Invalid,
    /// No timer id or RLIMIT_SIGPENDING left, or no timer thread (EAGAIN).
    Again,
}

/// The time `clock` reads for `process`; None for a thread that is gone.
fn now_on(process: &Process, clock: TimerClock) -> Option<i64> {
    let clamp = |ns: u64| ns.min(i64::MAX as u64) as i64;
    match clock {
        TimerClock::Wall => Some(crate::timer::realtime::now_ns()),
        TimerClock::Mono => Some(clamp(crate::timer::monotonic_ns())),
        TimerClock::ProcessCpu { user } => {
            let (times, (user_ns, _)) = process.cpu_split();
            Some(clamp(if user { user_ns } else { times.runtime_ns }))
        }
        #[cfg(feature = "alloc")]
        TimerClock::ThreadCpu { tid, user } => {
            process.get_thread(super::ThreadId(tid))?;
            let (times, (user_ns, _)) =
                crate::sched::dispatch::thread_cpu_split((process.pid.0, tid)).unwrap_or_default();
            Some(clamp(if user { user_ns } else { times.runtime_ns }))
        }
        #[cfg(not(feature = "alloc"))]
        TimerClock::ThreadCpu { .. } => None,
    }
}

fn key(process: &Process, id: i32) -> crate::timer::ktimerd::Key {
    crate::timer::ktimerd::Key::Posix(process.pid.0, id)
}

/// Wait for `expires` on `clock` (`timer::ktimerd` for a clock; CPU-clock
/// timers are checked on the way back to user mode).
fn wait_for(process: &Process, id: i32, clock: TimerClock, expires: i64) {
    match clock {
        TimerClock::Wall => crate::timer::ktimerd::arm_wall(
            key(process, id),
            crate::timer::realtime::monotonic_at(expires),
        ),
        TimerClock::Mono => crate::timer::ktimerd::arm(key(process, id), expires.max(0) as u64),
        _ => {}
    }
}

/// Note whether any timer waits on a CPU clock.
fn update_cpu_armed(process: &Process, timers: &PosixTimers) {
    process
        .posix_cpu_armed
        .store(timers.armed_on_cpu(), Ordering::Release);
}

/// Queue `timer`'s signal (Linux's posixtimer_send_sigqueue); a periodic
/// timer whose signal is ignored is parked.
fn send(process: &Process, id: i32, timer: &mut PosixTimer) {
    let Notify::Signal {
        signo,
        value,
        thread,
    } = timer.notify
    else {
        return;
    };
    let info = super::signals::SigInfo::timer(signo, id, value, timer.serial);
    let target = match thread {
        Some(tid) => match process.get_thread(super::ThreadId(tid)) {
            Some(thread) => Some(thread),
            None => return,
        },
        None => None,
    };
    #[cfg(feature = "alloc")]
    if super::signals::send_timer(process, target.as_deref(), info)
        == super::signals::TimerSend::Ignored
        && timer.status == Status::RequeuePending
    {
        timer.parked = true;
    }
    #[cfg(not(feature = "alloc"))]
    let _ = (target, info);
}

/// `timer`'s expiry came: queue its signal, or wait again.
fn fire(process: &Process, id: i32, timer: &mut PosixTimer) {
    let Some(now) = now_on(process, timer.base) else {
        return;
    };
    match timer.fire(now) {
        Fire::Signal => send(process, id, timer),
        Fire::Wait(expires) => wait_for(process, id, timer.base, expires),
        Fire::Nothing => {}
    }
}

/// timer_create: a timer on `clock` that notifies as `notify` says (None:
/// SIGALRM to the process with the timer id as its value, as for a NULL
/// sigevent); its id.
pub fn create(
    process: &Process,
    clock: TimerClock,
    notify: Option<Notify>,
) -> Result<i32, TimerError> {
    crate::timer::ktimerd::start().map_err(|_| TimerError::Again)?;
    let charge = super::signals::Charge::of(process);
    if !super::signals::charge_user(charge) {
        return Err(TimerError::Again);
    }
    let id = process.posix_timers.lock().add(|id, serial| {
        let notify = notify.unwrap_or(Notify::Signal {
            signo: super::itimer::SIGALRM,
            value: id as u32 as u64,
            thread: None,
        });
        PosixTimer::new(clock, notify, serial, charge.uid)
    });
    id.ok_or_else(|| {
        super::signals::uncharge_user(charge.uid, 1);
        TimerError::Again
    })
}

/// timer_settime: set timer `id` to `new` (`absolute`: TIMER_ABSTIME),
/// returning its old setting.
pub fn settime(process: &Process, id: i32, absolute: bool, new: Spec) -> Result<Spec, TimerError> {
    let mut timers = process.posix_timers.lock();
    let timer = timers.get_mut(id).ok_or(TimerError::Invalid)?;
    let old = now_on(process, timer.base)
        .map(|now| timer.get(now))
        .unwrap_or_default();
    let base = match timer.clock {
        TimerClock::Wall if !absolute => TimerClock::Mono,
        clock => clock,
    };
    let now = now_on(process, base).ok_or(TimerError::Invalid)?;
    crate::timer::ktimerd::cancel(key(process, id));
    match timer.set(new, absolute, base, now) {
        Arm::Cancel => {}
        Arm::At(expires) => wait_for(process, id, base, expires),
        Arm::FireNow => fire(process, id, timer),
    }
    update_cpu_armed(process, &timers);
    Ok(old)
}

/// timer_gettime: timer `id`'s setting.
pub fn gettime(process: &Process, id: i32) -> Result<Spec, TimerError> {
    let mut timers = process.posix_timers.lock();
    let timer = timers.get_mut(id).ok_or(TimerError::Invalid)?;
    Ok(now_on(process, timer.base)
        .map(|now| timer.get(now))
        .unwrap_or_default())
}

/// timer_getoverrun: timer `id`'s last overrun.
pub fn getoverrun(process: &Process, id: i32) -> Result<i32, TimerError> {
    let mut timers = process.posix_timers.lock();
    let timer = timers.get_mut(id).ok_or(TimerError::Invalid)?;
    Ok(timer.overrun_to_int())
}

/// timer_delete: a signal it has queued is dropped when taken.
pub fn delete(process: &Process, id: i32) -> Result<(), TimerError> {
    let mut timers = process.posix_timers.lock();
    let timer = timers.remove(id).ok_or(TimerError::Invalid)?;
    crate::timer::ktimerd::cancel(key(process, id));
    super::signals::uncharge_user(timer.uid, 1);
    update_cpu_armed(process, &timers);
    Ok(())
}

/// Delete every timer (exec, exit: Linux's exit_itimers).
pub fn delete_all(process: &Process) {
    let mut timers = process.posix_timers.lock();
    for (id, timer) in core::mem::take(&mut timers.timers) {
        crate::timer::ktimerd::cancel(key(process, id));
        super::signals::uncharge_user(timer.uid, 1);
    }
    update_cpu_armed(process, &timers);
}

/// ktimerd: process `pid`'s timer `id` expired.
pub fn expired(pid: u64, id: i32) {
    let Some(process) = super::find_process(super::ProcessId(pid)) else {
        return;
    };
    let mut timers = process.posix_timers.lock();
    if let Some(timer) = timers.get_mut(id) {
        fire(&process, id, timer);
    }
}

/// A timer's signal was taken with `info` (SI_TIMER), off the queues: true
/// to deliver it, with its overrun filled in; false to drop it (stale).
pub fn deliver(process: &Process, info: &mut super::signals::SigInfo) -> bool {
    let (id, serial) = (info.timer_id(), info.fields[2]);
    // The serial is the kernel's, not part of the user's siginfo.
    info.fields[2] = 0;
    let mut timers = process.posix_timers.lock();
    let Some(timer) = timers.get_mut(id) else {
        return false;
    };
    let Some(now) = now_on(process, timer.base) else {
        return false;
    };
    let delivered = match timer.deliver(serial, now) {
        Delivery::Drop => false,
        Delivery::Deliver { overrun, rearm } => {
            if let Some(overrun) = overrun {
                info.set_overrun(overrun);
            }
            if let Some(expires) = rearm {
                wait_for(process, id, timer.base, expires);
            }
            true
        }
    };
    update_cpu_armed(process, &timers);
    delivered
}

/// On the way back to user mode: fire the CPU-clock timers whose clocks
/// have reached their expiries.
pub fn check_cpu(process: &Process) {
    if !process.posix_cpu_armed.load(Ordering::Acquire) {
        return;
    }
    let mut timers = process.posix_timers.lock();
    for (&id, timer) in timers.timers.iter_mut() {
        if timer.armed_on_cpu() {
            fire(process, id, timer);
        }
    }
    update_cpu_armed(process, &timers);
}

/// `sig` became ignored and its pending instances were discarded: periodic
/// timers that queued it are parked (Linux's posixtimer_sig_ignore).
pub fn park(process: &Process, sig: usize) {
    let mut timers = process.posix_timers.lock();
    for timer in timers.timers.values_mut() {
        if matches!(timer.notify, Notify::Signal { signo, .. } if signo == sig)
            && timer.status == Status::RequeuePending
        {
            timer.parked = true;
        }
    }
}

/// `sig` is no longer ignored: parked timers queue it again (Linux's
/// posixtimer_sig_unignore); delivery decides what is stale.
pub fn unpark(process: &Process, sig: usize) {
    let mut timers = process.posix_timers.lock();
    for (&id, timer) in timers.timers.iter_mut() {
        if timer.parked && matches!(timer.notify, Notify::Signal { signo, .. } if signo == sig) {
            timer.parked = false;
            send(process, id, timer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;

    fn signal_timer() -> PosixTimer {
        let notify = Notify::Signal {
            signo: 34,
            value: 7,
            thread: None,
        };
        PosixTimer::new(TimerClock::Mono, notify, 1, 0)
    }

    fn spec(value: i64, interval: i64) -> Spec {
        Spec { value, interval }
    }

    /// hrtimer_forward, in signed time.
    #[test]
    fn forward_moves_past_now_by_whole_intervals() {
        assert_eq!(forward(100, 10, 50), (100, 0));
        assert_eq!(forward(100, 10, 100), (110, 1));
        assert_eq!(forward(100, 10, 139), (140, 4));
        assert_eq!(forward(-100, 30, -95), (-70, 1));
        assert_eq!(forward(100, 0, 500), (100, 0));
    }

    /// A periodic timer fires once, is armed again only when its signal is
    /// taken, and reports the intervals missed meanwhile as its overrun.
    #[test]
    fn periodic_timer_counts_overruns_at_delivery() {
        let mut t = signal_timer();
        assert_eq!(
            t.set(spec(10 * MS, 10 * MS), false, TimerClock::Mono, 0),
            Arm::At(10 * MS)
        );
        assert_eq!(t.fire(9 * MS), Fire::Wait(10 * MS));
        assert_eq!(t.fire(10 * MS), Fire::Signal);
        // Not waiting while its signal is pending.
        assert_eq!(t.fire(20 * MS), Fire::Nothing);
        // Taken at 45 ms: 10 + 4 intervals = 50 ms; 3 missed.
        assert_eq!(
            t.deliver(1, 45 * MS),
            Delivery::Deliver {
                overrun: Some(3),
                rearm: Some(50 * MS)
            }
        );
        assert_eq!(t.overrun_to_int(), 3);
        assert_eq!(t.fire(50 * MS), Fire::Signal);
        // Taken in time: no overrun.
        assert_eq!(
            t.deliver(1, 51 * MS),
            Delivery::Deliver {
                overrun: Some(0),
                rearm: Some(60 * MS)
            }
        );
        assert_eq!(t.overrun_to_int(), 0);
    }

    /// A signal queued before the timer was set again, or by an earlier
    /// timer with the same id, is dropped.
    #[test]
    fn stale_signals_are_dropped() {
        let mut t = signal_timer();
        t.set(spec(MS, MS), false, TimerClock::Mono, 0);
        assert_eq!(t.fire(MS), Fire::Signal);
        t.set(spec(5 * MS, 0), false, TimerClock::Mono, 2 * MS);
        assert_eq!(t.deliver(1, 3 * MS), Delivery::Drop);
        assert_eq!(t.fire(7 * MS), Fire::Signal);
        assert_eq!(t.deliver(2, 7 * MS), Delivery::Drop);
        assert_eq!(
            t.deliver(1, 7 * MS),
            Delivery::Deliver {
                overrun: None,
                rearm: None
            }
        );
        // A one-shot timer is done.
        assert_eq!(t.fire(100 * MS), Fire::Nothing);
    }

    /// timer_gettime: time left; a due timer whose signal is not taken
    /// has a nanosecond; a pending periodic one moves on; a one-shot that
    /// fired reports nothing.
    #[test]
    fn gettime_follows_common_timer_get() {
        let mut t = signal_timer();
        assert_eq!(t.get(0), Spec::default());
        t.set(spec(10 * MS, 4 * MS), false, TimerClock::Mono, 0);
        assert_eq!(t.get(3 * MS), spec(7 * MS, 4 * MS));
        assert_eq!(t.get(12 * MS), spec(1, 4 * MS));
        assert_eq!(t.fire(12 * MS), Fire::Signal);
        // Pending: as if armed again now (10 + 2 intervals = 18 ms).
        assert_eq!(t.get(15 * MS), spec(3 * MS, 4 * MS));
        let mut once = signal_timer();
        once.set(spec(MS, 0), false, TimerClock::Mono, 0);
        assert_eq!(once.fire(MS), Fire::Signal);
        assert_eq!(once.get(2 * MS), Spec::default());
    }

    /// TIMER_ABSTIME: a time already past fires at once; a zero value
    /// disarms and drops the interval.
    #[test]
    fn absolute_and_zero_settings() {
        let mut t = signal_timer();
        assert_eq!(
            t.set(spec(5 * MS, MS), true, TimerClock::Wall, 9 * MS),
            Arm::FireNow
        );
        assert_eq!(t.fire(9 * MS), Fire::Signal);
        assert_eq!(
            t.set(spec(20 * MS, 0), true, TimerClock::Wall, 9 * MS),
            Arm::At(20 * MS)
        );
        assert_eq!(
            t.set(spec(0, 3 * MS), false, TimerClock::Mono, 10 * MS),
            Arm::Cancel
        );
        assert_eq!(t.get(10 * MS), Spec::default());
        assert_eq!(t.fire(30 * MS), Fire::Nothing);
    }

    /// SIGEV_NONE: never fires, but timer_gettime counts down, a one-shot
    /// one reaching 0 and a periodic one moving on.
    #[test]
    fn sigev_none_is_emulated() {
        let mut t = PosixTimer::new(TimerClock::Mono, Notify::None, 1, 0);
        assert_eq!(
            t.set(spec(10 * MS, 0), false, TimerClock::Mono, 0),
            Arm::Cancel
        );
        assert_eq!(t.fire(20 * MS), Fire::Nothing);
        assert_eq!(t.get(4 * MS), spec(6 * MS, 0));
        assert_eq!(t.get(20 * MS), Spec::default());
        t.set(spec(10 * MS, 10 * MS), false, TimerClock::Mono, 0);
        assert_eq!(t.get(25 * MS), spec(5 * MS, 10 * MS));
    }

    /// Ids are handed out in turn, skip those in use, wrap at INT_MAX and
    /// run out only when every id is taken.
    #[test]
    fn ids_are_handed_out_in_turn() {
        let mut timers = PosixTimers::default();
        let add = |t: &mut PosixTimers| {
            t.add(|_, s| PosixTimer::new(TimerClock::Mono, Notify::None, s, 0))
        };
        assert_eq!(add(&mut timers), Some(0));
        assert_eq!(add(&mut timers), Some(1));
        assert!(timers.remove(0).is_some());
        // Not reused at once.
        assert_eq!(add(&mut timers), Some(2));
        timers.next_id = i32::MAX as u32;
        assert_eq!(add(&mut timers), Some(i32::MAX));
        // Wrapped: 0 is free, 1 and 2 are not.
        assert_eq!(add(&mut timers), Some(0));
        assert_eq!(add(&mut timers), Some(3));
        let serials: alloc::vec::Vec<u64> = timers.timers.values().map(|t| t.serial).collect();
        assert_eq!(serials.len(), 5);
        assert!(serials.windows(2).all(|w| w[0] != w[1]));
    }
}
