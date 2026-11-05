//! adjtimex's NTP state (Linux kernel/time/ntp.c and timekeeping.c,
//! without CONFIG_NTP_PPS).
//!
//! The state is stored and read back as Linux keeps it: the status bits,
//! time constant, error estimates, tick, the PLL/FLL frequency and phase
//! offset that ADJ_OFFSET computes, the adjtime() slew and the TAI offset.
//! ADJ_SETOFFSET steps CLOCK_REALTIME and ADJ_TAI sets CLOCK_TAI's offset
//! from it. The tick and the frequency set how long a second of
//! CLOCK_MONOTONIC and CLOCK_REALTIME is ([`Ntp::second_length`], Linux's
//! ntp_update_frequency); once a second ([`Ntp::second_overflow`], run by
//! `timer::steer`) a chunk of the phase offset and up to 500 us of the
//! adjtime() slew are added to the next second, maxerror grows, and a
//! leap second armed with STA_INS or STA_DEL is inserted or deleted at
//! midnight UTC. The phase and the slew are delivered a second at a time,
//! as Linux did before 7.0's per-tick skew drain; what they add up to is
//! the same.

pub const ADJ_OFFSET: u32 = 0x0001;
pub const ADJ_FREQUENCY: u32 = 0x0002;
pub const ADJ_MAXERROR: u32 = 0x0004;
pub const ADJ_ESTERROR: u32 = 0x0008;
pub const ADJ_STATUS: u32 = 0x0010;
pub const ADJ_TIMECONST: u32 = 0x0020;
pub const ADJ_TAI: u32 = 0x0080;
pub const ADJ_SETOFFSET: u32 = 0x0100;
pub const ADJ_MICRO: u32 = 0x1000;
pub const ADJ_NANO: u32 = 0x2000;
pub const ADJ_TICK: u32 = 0x4000;
/// adjtime(): with ADJ_OFFSET_SINGLESHOT (0x8001) only.
pub const ADJ_ADJTIME: u32 = 0x8000;
pub const ADJ_OFFSET_SINGLESHOT: u32 = 0x0001;
/// With ADJ_ADJTIME: read the slew left without setting one (0xa001).
pub const ADJ_OFFSET_READONLY: u32 = 0x2000;

pub const STA_PLL: i32 = 0x0001;
pub const STA_FLL: i32 = 0x0008;
pub const STA_UNSYNC: i32 = 0x0040;
pub const STA_FREQHOLD: i32 = 0x0080;
pub const STA_CLOCKERR: i32 = 0x1000;
pub const STA_NANO: i32 = 0x2000;
pub const STA_MODE: i32 = 0x4000;
/// The bits ADJ_STATUS cannot change (PPS signal state, CLOCKERR, NANO,
/// MODE, CLK).
pub const STA_RONLY: i32 =
    0x0100 | 0x0200 | 0x0400 | 0x0800 | STA_CLOCKERR | STA_NANO | STA_MODE | 0x8000;

pub const STA_INS: i32 = 0x0010;
pub const STA_DEL: i32 = 0x0020;

pub const TIME_OK: i32 = 0;
pub const TIME_INS: i32 = 1;
pub const TIME_DEL: i32 = 2;
pub const TIME_OOP: i32 = 3;
pub const TIME_WAIT: i32 = 4;
pub const TIME_ERROR: i32 = 5;

const NSEC_PER_USEC: i64 = 1000;
const USEC_PER_SEC: i64 = 1_000_000;
const NSEC_PER_SEC: i64 = 1_000_000_000;
/// USER_HZ: ADJ_TICK must stay within 10% of 1000000 / USER_HZ.
const USER_HZ: i64 = 100;
const NTP_SCALE_SHIFT: u32 = 32;
const SHIFT_USEC: u32 = 16;
const SHIFT_PLL: i64 = 2;
const SHIFT_FLL: u32 = 2;
const MAXTC: i64 = 10;
const MAXPHASE: i64 = 500_000_000;
const MAXFREQ: i64 = 500_000;
const MAXFREQ_SCALED: i64 = MAXFREQ << NTP_SCALE_SHIFT;
const MINSEC: i64 = 256;
const MAXSEC: i64 = 2048;
const PPM_SCALE: i64 = NSEC_PER_USEC << (NTP_SCALE_SHIFT - SHIFT_USEC);
const PPM_SCALE_INV_SHIFT: u32 = 19;
const PPM_SCALE_INV: i64 = (1i64 << (PPM_SCALE_INV_SHIFT + NTP_SCALE_SHIFT)) / PPM_SCALE + 1;
/// The largest error estimate (16 s, in microseconds).
const NTP_PHASE_LIMIT: i64 = (MAXPHASE / NSEC_PER_USEC) << 5;
const MAX_TAI_OFFSET: i64 = 100_000;
/// The most of the adjtime() slew delivered in a second, in microseconds.
const MAX_TICKADJ: i64 = 500;
const SECS_PER_DAY: i64 = 86_400;
/// NTP_INTERVAL_FREQ (HZ): only the rounding of the phase offset read
/// back depends on it; 1000 as a CONFIG_HZ_1000 kernel.
const NTP_INTERVAL_FREQ: i64 = 1000;
/// USER_TICK_USEC: 1000000 / USER_HZ.
const USER_TICK_USEC: i64 = USEC_PER_SEC / USER_HZ;

/// `struct timex` (`struct __kernel_timex`, 208 bytes on 64-bit), with
/// its padding named so that a copy keeps the caller's bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timex {
    pub modes: u32,
    pub pad0: i32,
    pub offset: i64,
    pub freq: i64,
    pub maxerror: i64,
    pub esterror: i64,
    pub status: i32,
    pub pad1: i32,
    pub constant: i64,
    pub precision: i64,
    pub tolerance: i64,
    pub time_sec: i64,
    pub time_usec: i64,
    pub tick: i64,
    pub ppsfreq: i64,
    pub jitter: i64,
    pub shift: i32,
    pub pad2: i32,
    pub stabil: i64,
    pub jitcnt: i64,
    pub calcnt: i64,
    pub errcnt: i64,
    pub stbcnt: i64,
    pub tai: i32,
    pub pad3: [i32; 11],
}

const _: () = assert!(core::mem::size_of::<Timex>() == 208);

/// Why a timex is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// EINVAL.
    Invalid,
    /// EPERM: changing anything needs CAP_SYS_TIME.
    NotPermitted,
}

/// Linux's timekeeping_validate_timex, in its order: whether `tx` may be
/// applied by a caller that has CAP_SYS_TIME (`capable`) or not.
pub fn validate(tx: &Timex, capable: bool) -> Result<(), Refused> {
    if tx.modes & ADJ_ADJTIME != 0 {
        // The single-shot form must not carry any other mode bits.
        if tx.modes & ADJ_OFFSET_SINGLESHOT == 0 {
            return Err(Refused::Invalid);
        }
        if tx.modes & ADJ_OFFSET_READONLY == 0 && !capable {
            return Err(Refused::NotPermitted);
        }
    } else {
        if tx.modes != 0 && !capable {
            return Err(Refused::NotPermitted);
        }
        // A tick more than 10% off means something is very wrong.
        if tx.modes & ADJ_TICK != 0
            && (tx.tick < 900_000 / USER_HZ || tx.tick > 1_100_000 / USER_HZ)
        {
            return Err(Refused::Invalid);
        }
    }
    if tx.modes & ADJ_SETOFFSET != 0 {
        if !capable {
            return Err(Refused::NotPermitted);
        }
        let limit = if tx.modes & ADJ_NANO != 0 {
            NSEC_PER_SEC
        } else {
            USEC_PER_SEC
        };
        if !(0..limit).contains(&tx.time_usec) {
            return Err(Refused::Invalid);
        }
    }
    // The frequency is scaled by PPM_SCALE, which must not overflow.
    if tx.modes & ADJ_FREQUENCY != 0
        && (tx.freq < i64::MIN / PPM_SCALE || tx.freq > i64::MAX / PPM_SCALE)
    {
        return Err(Refused::Invalid);
    }
    Ok(())
}

/// The ADJ_SETOFFSET step in nanoseconds (`time` is a timeval, or a
/// timespec with ADJ_NANO); `tx` has passed [`validate`].
pub fn setoffset_ns(tx: &Timex) -> i64 {
    let frac = if tx.modes & ADJ_NANO != 0 {
        tx.time_usec
    } else {
        tx.time_usec * NSEC_PER_USEC
    };
    tx.time_sec
        .saturating_mul(NSEC_PER_SEC)
        .saturating_add(frac)
}

/// Linux's shift_right: an arithmetic shift that rounds toward zero.
fn shift_right(x: i64, s: u32) -> i64 {
    if x < 0 {
        -((x.wrapping_neg() as u64 >> s) as i64)
    } else {
        x >> s
    }
}

/// The NTP state (Linux's `struct ntp_data`, one timekeeper).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ntp {
    tick_usec: i64,
    time_state: i32,
    time_status: i32,
    time_constant: i64,
    time_maxerror: i64,
    time_esterror: i64,
    /// Frequency offset, scaled ns/s (<< NTP_SCALE_SHIFT).
    time_freq: i64,
    /// Phase offset, scaled ns per NTP interval.
    time_offset: i64,
    /// When the PLL was last updated (NTP seconds).
    time_reftime: i64,
    /// adjtime() slew left, in microseconds.
    time_adjust: i64,
    /// TAI minus UTC, in seconds.
    tai: i32,
    /// The wall second (UTC) at which the armed leap second happens.
    next_leap_sec: i64,
    /// What the phase offset and the slew add to the current second,
    /// scaled ns.
    phase: i64,
    /// `timer::steer`'s bookkeeping: the next wall second to process, and
    /// the wall clock's set count it last saw.
    pub(super) next_second: i64,
    pub(super) seen_sets: u64,
}

impl Default for Ntp {
    fn default() -> Self {
        Self::new()
    }
}

impl Ntp {
    /// Linux's boot state: unsynchronized, time constant 2, errors at the
    /// phase limit.
    pub const fn new() -> Self {
        Self {
            tick_usec: USER_TICK_USEC,
            time_state: TIME_OK,
            time_status: STA_UNSYNC,
            time_constant: 2,
            time_maxerror: NTP_PHASE_LIMIT,
            time_esterror: NTP_PHASE_LIMIT,
            time_freq: 0,
            time_offset: 0,
            time_reftime: 0,
            time_adjust: 0,
            tai: 0,
            next_leap_sec: i64::MAX,
            phase: 0,
            next_second: 0,
            seen_sets: 0,
        }
    }

    /// How long a second of the steered clocks is, in ns scaled by
    /// 2^NTP_SCALE_SHIFT: the tick and the frequency (Linux's
    /// ntp_update_frequency), plus what the phase offset and the adjtime()
    /// slew add to this second.
    pub fn second_length(&self) -> u64 {
        let base = ((self.tick_usec * NSEC_PER_USEC * USER_HZ) as i128) << NTP_SCALE_SHIFT;
        (base + self.time_freq as i128 + self.phase as i128).clamp(1, u64::MAX as i128) as u64
    }

    /// Whether anything happens at the next second: a phase offset or a
    /// slew to deliver, a leap second armed or under way, or maxerror still
    /// growing. Otherwise every second is the same and none need running.
    pub fn is_active(&self) -> bool {
        self.time_offset != 0
            || self.time_adjust != 0
            || self.phase != 0
            || self.time_state != TIME_OK
            || self.time_status & (STA_INS | STA_DEL) != 0
            || self.time_maxerror < NTP_PHASE_LIMIT
    }

    /// Linux's second_overflow, run as the wall clock starts second `secs`
    /// (UTC): the leap second state machine, the maxerror bump, and what
    /// the phase offset and the slew add to this second. Returns the leap:
    /// -1 when the clock must go back a second (an inserted 23:59:60), 1
    /// when it must go ahead (a deleted 23:59:59), else 0; the TAI offset
    /// has already moved with it.
    pub fn second_overflow(&mut self, secs: i64) -> i32 {
        let mut leap = 0;
        match self.time_state {
            TIME_OK => {
                if self.time_status & STA_INS != 0 {
                    self.time_state = TIME_INS;
                    self.next_leap_sec = secs + SECS_PER_DAY - secs.rem_euclid(SECS_PER_DAY);
                } else if self.time_status & STA_DEL != 0 {
                    self.time_state = TIME_DEL;
                    self.next_leap_sec = secs + SECS_PER_DAY - (secs + 1).rem_euclid(SECS_PER_DAY);
                }
            }
            TIME_INS => {
                if self.time_status & STA_INS == 0 {
                    self.next_leap_sec = i64::MAX;
                    self.time_state = TIME_OK;
                } else if secs == self.next_leap_sec {
                    leap = -1;
                    self.time_state = TIME_OOP;
                }
            }
            TIME_DEL => {
                if self.time_status & STA_DEL == 0 {
                    self.next_leap_sec = i64::MAX;
                    self.time_state = TIME_OK;
                } else if secs == self.next_leap_sec {
                    leap = 1;
                    self.next_leap_sec = i64::MAX;
                    self.time_state = TIME_WAIT;
                }
            }
            TIME_OOP => {
                self.next_leap_sec = i64::MAX;
                self.time_state = TIME_WAIT;
            }
            _ => {
                if self.time_status & (STA_INS | STA_DEL) == 0 {
                    self.time_state = TIME_OK;
                }
            }
        }
        // Linux's timekeeping: TAI is continuous, UTC takes the leap.
        self.tai -= leap;

        self.time_maxerror += MAXFREQ / NSEC_PER_USEC;
        if self.time_maxerror > NTP_PHASE_LIMIT {
            self.time_maxerror = NTP_PHASE_LIMIT;
            self.time_status |= STA_UNSYNC;
        }

        // The phase offset: a chunk a second, the rest once it rounds to
        // nothing (time_offset is per NTP interval).
        let mut chunk = shift_right(self.time_offset, (SHIFT_PLL + self.time_constant) as u32);
        if chunk == 0 {
            chunk = self.time_offset;
        }
        self.time_offset -= chunk;
        let mut phase = chunk.saturating_mul(NTP_INTERVAL_FREQ);
        // The adjtime() slew, at most MAX_TICKADJ us a second.
        let adj = self.time_adjust.clamp(-MAX_TICKADJ, MAX_TICKADJ);
        self.time_adjust -= adj;
        phase = phase.saturating_add((adj * NSEC_PER_USEC) << NTP_SCALE_SHIFT);
        self.phase = phase;
        leap
    }

    /// TAI minus UTC, in seconds.
    pub fn tai(&self) -> i32 {
        self.tai
    }

    /// process_adj_status: PLL off resets the state; the read-only bits
    /// are kept.
    fn adj_status(&mut self, status: i32, ntp_secs: i64) {
        if self.time_status & STA_PLL != 0 && status & STA_PLL == 0 {
            self.time_state = TIME_OK;
            self.time_status = STA_UNSYNC;
            self.next_leap_sec = i64::MAX;
        }
        if self.time_status & STA_PLL == 0 && status & STA_PLL != 0 {
            self.time_reftime = ntp_secs;
        }
        self.time_status &= STA_RONLY;
        self.time_status |= status & !STA_RONLY;
    }

    /// ntp_update_offset: a new phase offset (ns with STA_NANO, else us)
    /// moves the frequency by the PLL, or the FLL for long intervals.
    fn update_offset(&mut self, offset: i64, ntp_secs: i64) {
        if self.time_status & STA_PLL == 0 {
            return;
        }
        let mut offset = offset;
        if self.time_status & STA_NANO == 0 {
            offset = offset.clamp(-USEC_PER_SEC, USEC_PER_SEC) * NSEC_PER_USEC;
        }
        let offset = offset.clamp(-MAXPHASE, MAXPHASE);
        let mut secs = ntp_secs.wrapping_sub(self.time_reftime);
        if self.time_status & STA_FREQHOLD != 0 {
            secs = 0;
        }
        self.time_reftime = ntp_secs;

        // ntp_update_offset_fll.
        self.time_status &= !STA_MODE;
        let mut freq_adj = 0;
        if secs >= MINSEC && (self.time_status & STA_FLL != 0 || secs > MAXSEC) {
            self.time_status |= STA_MODE;
            freq_adj = (offset << (NTP_SCALE_SHIFT - SHIFT_FLL)) / secs;
        }

        // A long interval would make the PLL gain unstable.
        let max_secs = 1i64 << (SHIFT_PLL + 1 + self.time_constant);
        if secs > max_secs {
            secs = max_secs;
        }
        let shift = NTP_SCALE_SHIFT as i64 - 2 * (SHIFT_PLL + 2 + self.time_constant);
        freq_adj = freq_adj.wrapping_add(offset.wrapping_mul(secs) << shift);
        freq_adj = freq_adj.wrapping_add(self.time_freq).min(MAXFREQ_SCALED);
        self.time_freq = freq_adj.max(-MAXFREQ_SCALED);
        self.time_offset = (offset << NTP_SCALE_SHIFT) / NTP_INTERVAL_FREQ;
    }

    /// process_adjtimex_modes, in Linux's order.
    fn apply_modes(&mut self, tx: &Timex, ntp_secs: i64) {
        if tx.modes & ADJ_STATUS != 0 {
            self.adj_status(tx.status, ntp_secs);
        }
        if tx.modes & ADJ_NANO != 0 {
            self.time_status |= STA_NANO;
        }
        if tx.modes & ADJ_MICRO != 0 {
            self.time_status &= !STA_NANO;
        }
        if tx.modes & ADJ_FREQUENCY != 0 {
            self.time_freq = (tx.freq * PPM_SCALE).clamp(-MAXFREQ_SCALED, MAXFREQ_SCALED);
        }
        if tx.modes & ADJ_MAXERROR != 0 {
            self.time_maxerror = tx.maxerror.clamp(0, NTP_PHASE_LIMIT);
        }
        if tx.modes & ADJ_ESTERROR != 0 {
            self.time_esterror = tx.esterror.clamp(0, NTP_PHASE_LIMIT);
        }
        if tx.modes & ADJ_TIMECONST != 0 {
            self.time_constant = tx.constant.clamp(0, MAXTC);
            if self.time_status & STA_NANO == 0 {
                self.time_constant += 4;
            }
            self.time_constant = self.time_constant.clamp(0, MAXTC);
        }
        if tx.modes & ADJ_TAI != 0 && (0..=MAX_TAI_OFFSET).contains(&tx.constant) {
            self.tai = tx.constant as i32;
        }
        if tx.modes & ADJ_OFFSET != 0 {
            self.update_offset(tx.offset, ntp_secs);
        }
        if tx.modes & ADJ_TICK != 0 {
            self.tick_usec = tx.tick;
        }
    }

    /// ntp_adjtimex: apply `tx` (it has passed [`validate`]) and fill it
    /// with the state. `now` is the wall time read before any
    /// ADJ_SETOFFSET step, as Linux reports it; `ntp_secs` is the wall
    /// time in seconds after it. Returns the clock state (TIME_ERROR while
    /// unsynchronized).
    pub fn adjtimex(&mut self, tx: &mut Timex, now_ns: i64, ntp_secs: i64) -> i32 {
        if tx.modes & ADJ_ADJTIME != 0 {
            let save_adjust = self.time_adjust;
            if tx.modes & ADJ_OFFSET_READONLY == 0 {
                self.time_adjust = tx.offset;
            }
            tx.offset = save_adjust;
        } else {
            if tx.modes != 0 {
                self.apply_modes(tx, ntp_secs);
            }
            tx.offset = shift_right(
                self.time_offset.wrapping_mul(NTP_INTERVAL_FREQ),
                NTP_SCALE_SHIFT,
            );
            if self.time_status & STA_NANO == 0 {
                tx.offset /= NSEC_PER_USEC;
            }
        }
        let result = if self.time_status & (STA_UNSYNC | STA_CLOCKERR) != 0 {
            TIME_ERROR
        } else {
            self.time_state
        };
        tx.freq = shift_right(
            (self.time_freq >> PPM_SCALE_INV_SHIFT).wrapping_mul(PPM_SCALE_INV),
            NTP_SCALE_SHIFT,
        );
        tx.maxerror = self.time_maxerror;
        tx.esterror = self.time_esterror;
        tx.status = self.time_status;
        tx.constant = self.time_constant;
        tx.precision = 1;
        tx.tolerance = MAXFREQ_SCALED / PPM_SCALE;
        tx.tick = self.tick_usec;
        tx.tai = self.tai;
        // No PPS: its fields read zero.
        tx.ppsfreq = 0;
        tx.jitter = 0;
        tx.shift = 0;
        tx.stabil = 0;
        tx.jitcnt = 0;
        tx.calcnt = 0;
        tx.errcnt = 0;
        tx.stbcnt = 0;
        tx.time_sec = now_ns.div_euclid(NSEC_PER_SEC);
        tx.time_usec = now_ns.rem_euclid(NSEC_PER_SEC);
        if self.time_status & STA_NANO == 0 {
            tx.time_usec /= NSEC_PER_USEC;
        }
        // A leap second due but not yet processed reads as one.
        let secs = now_ns.div_euclid(NSEC_PER_SEC);
        let mut result = result;
        if secs >= self.next_leap_sec {
            if self.time_state == TIME_INS && self.time_status & STA_INS != 0 {
                result = TIME_OOP;
                tx.tai += 1;
                tx.time_sec -= 1;
            }
            if self.time_state == TIME_DEL && self.time_status & STA_DEL != 0 {
                result = TIME_WAIT;
                tx.tai -= 1;
                tx.time_sec += 1;
            }
            if self.time_state == TIME_OOP && secs == self.next_leap_sec {
                result = TIME_WAIT;
            }
        }
        result
    }
}

/// The system's NTP state.
static NTP: spin::Mutex<Ntp> = spin::Mutex::new(Ntp::new());

/// Run `f` on the system's NTP state.
pub fn with<R>(f: impl FnOnce(&mut Ntp) -> R) -> R {
    f(&mut NTP.lock())
}

/// TAI minus UTC, in seconds (CLOCK_TAI reads CLOCK_REALTIME plus this).
pub fn tai_offset_s() -> i32 {
    NTP.lock().tai
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(modes: u32) -> Timex {
        Timex {
            modes,
            ..Timex::default()
        }
    }

    #[test]
    fn validation_follows_linux_order() {
        // Reading is open to everyone; anything else needs CAP_SYS_TIME.
        assert_eq!(validate(&modes(0), false), Ok(()));
        assert_eq!(
            validate(&modes(ADJ_MAXERROR), false),
            Err(Refused::NotPermitted)
        );
        assert_eq!(validate(&modes(ADJ_MAXERROR), true), Ok(()));
        // adjtime: single-shot only; reading it back is unprivileged.
        assert_eq!(validate(&modes(ADJ_ADJTIME), true), Err(Refused::Invalid));
        assert_eq!(validate(&modes(0xa001), false), Ok(()));
        assert_eq!(validate(&modes(0x8001), false), Err(Refused::NotPermitted));
        assert_eq!(validate(&modes(0x8001), true), Ok(()));
        // A tick more than 10% from 10000 us.
        let mut tx = modes(ADJ_TICK);
        tx.tick = 8999;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        tx.tick = 11001;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        tx.tick = 11000;
        assert_eq!(validate(&tx, true), Ok(()));
        // The permission check comes before the tick check.
        tx.tick = 1;
        assert_eq!(validate(&tx, false), Err(Refused::NotPermitted));
        // ADJ_SETOFFSET: the fraction must be within a second.
        let mut tx = modes(ADJ_SETOFFSET);
        tx.time_usec = -1;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        tx.time_usec = 1_000_000;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        tx.modes |= ADJ_NANO;
        assert_eq!(validate(&tx, true), Ok(()));
        assert_eq!(setoffset_ns(&tx), 1_000_000);
        tx.time_usec = NSEC_PER_SEC;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        // A frequency that would overflow when scaled.
        let mut tx = modes(ADJ_FREQUENCY);
        tx.freq = i64::MAX / PPM_SCALE + 1;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
        tx.freq = i64::MIN / PPM_SCALE - 1;
        assert_eq!(validate(&tx, true), Err(Refused::Invalid));
    }

    #[test]
    fn read_reports_linux_defaults() {
        let mut ntp = Ntp::new();
        let mut tx = modes(0);
        tx.pad3[0] = 77;
        let state = ntp.adjtimex(
            &mut tx,
            1_700_000_000 * NSEC_PER_SEC + 1_234_567,
            1_700_000_000,
        );
        assert_eq!(state, TIME_ERROR);
        assert_eq!(tx.status, STA_UNSYNC);
        assert_eq!(tx.constant, 2);
        assert_eq!((tx.maxerror, tx.esterror), (16_000_000, 16_000_000));
        assert_eq!(
            (tx.tick, tx.precision, tx.tolerance),
            (10_000, 1, 32_768_000)
        );
        assert_eq!((tx.offset, tx.freq, tx.tai), (0, 0, 0));
        assert_eq!((tx.time_sec, tx.time_usec), (1_700_000_000, 1234));
        // Padding is the caller's.
        assert_eq!(tx.pad3[0], 77);
    }

    #[test]
    fn modes_are_stored_and_clamped() {
        let mut ntp = Ntp::new();
        let mut tx = modes(ADJ_STATUS | ADJ_MAXERROR | ADJ_ESTERROR | ADJ_TIMECONST | ADJ_TICK);
        tx.status = STA_NANO; // read-only: ignored; UNSYNC cleared
        tx.maxerror = -5;
        tx.esterror = 99_000_000;
        tx.constant = 3;
        tx.tick = 10_500;
        assert_eq!(ntp.adjtimex(&mut tx, 0, 0), TIME_OK);
        assert_eq!(tx.status, 0);
        assert_eq!((tx.maxerror, tx.esterror), (0, NTP_PHASE_LIMIT));
        // Without STA_NANO the time constant is offset by 4.
        assert_eq!(tx.constant, 7);
        assert_eq!(tx.tick, 10_500);

        // Frequency round-trips (scaled ppm), clamped to 500 ppm.
        let mut tx = modes(ADJ_FREQUENCY);
        tx.freq = 100 << 16;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.freq, 100 << 16);
        tx = modes(ADJ_FREQUENCY);
        tx.freq = 900 << 16;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.freq, 500 << 16);

        // TAI: only 0..=100000 is taken.
        let mut tx = modes(ADJ_TAI);
        tx.constant = 37;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!((tx.tai, ntp.tai()), (37, 37));
        tx = modes(ADJ_TAI);
        tx.constant = -1;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.tai, 37);

        // ADJ_NANO reports nanoseconds; ADJ_MICRO goes back.
        let mut tx = modes(ADJ_NANO);
        ntp.adjtimex(&mut tx, 5 * NSEC_PER_SEC + 42, 5);
        assert_eq!((tx.status & STA_NANO, tx.time_usec), (STA_NANO, 42));
        tx = modes(ADJ_MICRO);
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.status & STA_NANO, 0);
    }

    #[test]
    fn offset_needs_the_pll_and_reads_back() {
        let mut ntp = Ntp::new();
        let mut tx = modes(ADJ_OFFSET);
        tx.offset = 500;
        ntp.adjtimex(&mut tx, 0, 100);
        assert_eq!(tx.offset, 0, "ignored without STA_PLL");

        let mut tx = modes(ADJ_STATUS);
        tx.status = STA_PLL;
        ntp.adjtimex(&mut tx, 0, 100);
        let mut tx = modes(ADJ_OFFSET);
        tx.offset = 500; // microseconds
        ntp.adjtimex(&mut tx, 0, 104);
        assert_eq!(tx.offset, 500);
        // Four seconds since the reference time: the PLL moved the
        // frequency, (500000 ns * 4) << (32 - 2 * (2 + 2 + 2)), as Linux.
        let expect = (500_000i64 * 4) << 20;
        let ppm = shift_right((expect >> PPM_SCALE_INV_SHIFT) * PPM_SCALE_INV, 32);
        assert_eq!(tx.freq, ppm);
        assert_eq!(tx.status & STA_MODE, 0);

        // Turning the PLL off resets the state; as in Linux the status is
        // then the caller's, so UNSYNC stays only if it is asked for.
        let mut tx = modes(ADJ_STATUS);
        tx.status = 0;
        assert_eq!(ntp.adjtimex(&mut tx, 0, 105), TIME_OK);
        assert_eq!(tx.status, 0);
        let mut tx = modes(ADJ_STATUS);
        tx.status = STA_UNSYNC;
        assert_eq!(ntp.adjtimex(&mut tx, 0, 106), TIME_ERROR);
    }

    #[test]
    fn adjtime_swaps_the_slew() {
        let mut ntp = Ntp::new();
        let mut tx = modes(0x8001);
        tx.offset = 250_000;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.offset, 0);
        let mut tx = modes(0xa001);
        tx.offset = 9;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.offset, 250_000, "read-only keeps it");
        let mut tx = modes(0x8001);
        tx.offset = 0;
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(tx.offset, 250_000);
    }

    /// The nominal second: tick 10000 us at USER_HZ 100, scaled.
    const NOMINAL: i128 = (NSEC_PER_SEC as i128) << NTP_SCALE_SHIFT;

    /// ADJ_FREQUENCY and ADJ_TICK change the length of a second at once,
    /// as Linux's ntp_update_frequency.
    #[test]
    fn frequency_and_tick_set_the_second_length() {
        let mut ntp = Ntp::new();
        assert_eq!(ntp.second_length() as i128, NOMINAL);
        let mut tx = modes(ADJ_FREQUENCY);
        tx.freq = 100 << 16; // +100 ppm
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(
            ntp.second_length() as i128,
            NOMINAL + (100_000i128 << NTP_SCALE_SHIFT)
        );
        let mut tx = modes(ADJ_TICK);
        tx.tick = 9_990; // 0.1% short, plus the 100 ppm
        ntp.adjtimex(&mut tx, 0, 0);
        assert_eq!(
            ntp.second_length() as i128,
            (999_000_000i128 + 100_000) << NTP_SCALE_SHIFT
        );
    }

    /// adjtime()'s slew is delivered at most 500 us a second, and what is
    /// left reads back.
    #[test]
    fn adjtime_slew_drains_at_the_tickadj_rate() {
        let mut ntp = Ntp::new();
        let mut tx = modes(0x8001);
        tx.offset = 1_200; // us
        ntp.adjtimex(&mut tx, 0, 0);
        assert!(ntp.is_active());
        assert_eq!(ntp.second_overflow(1), 0);
        assert_eq!(
            ntp.second_length() as i128,
            NOMINAL + (500_000i128 << NTP_SCALE_SHIFT)
        );
        let mut tx = modes(0xa001);
        ntp.adjtimex(&mut tx, 0, 1);
        assert_eq!(tx.offset, 700);
        ntp.second_overflow(2);
        ntp.second_overflow(3);
        assert_eq!(
            ntp.second_length() as i128,
            NOMINAL + (200_000i128 << NTP_SCALE_SHIFT)
        );
        // Drained: the next second is nominal again.
        ntp.second_overflow(4);
        assert_eq!(ntp.second_length() as i128, NOMINAL);
        let mut tx = modes(0xa001);
        ntp.adjtimex(&mut tx, 0, 4);
        assert_eq!(tx.offset, 0);
        // A negative slew slows the clock.
        let mut tx = modes(0x8001);
        tx.offset = -300;
        ntp.adjtimex(&mut tx, 0, 4);
        ntp.second_overflow(5);
        assert_eq!(
            ntp.second_length() as i128,
            NOMINAL - (300_000i128 << NTP_SCALE_SHIFT)
        );
    }

    /// The PLL's phase offset is delivered a chunk a second (offset >>
    /// (SHIFT_PLL + time constant)) and decays to zero; the offset read back
    /// shrinks with it.
    #[test]
    fn phase_offset_decays_each_second() {
        let mut ntp = Ntp::new();
        let mut tx = modes(ADJ_STATUS | ADJ_NANO | ADJ_TIMECONST);
        tx.status = STA_PLL;
        tx.constant = 0;
        ntp.adjtimex(&mut tx, 0, 100);
        let mut tx = modes(ADJ_OFFSET);
        tx.offset = 400_000; // ns
        ntp.adjtimex(&mut tx, 0, 100);
        let freq = ntp.time_freq as i128;
        ntp.second_overflow(101);
        // A quarter (shift 2) of 400 us this second, give or take the
        // scaled rounding.
        let delivered = (ntp.second_length() as i128 - NOMINAL - freq) >> NTP_SCALE_SHIFT;
        assert!((99_999..=100_000).contains(&delivered), "{delivered}");
        let mut tx = modes(0);
        ntp.adjtimex(&mut tx, 0, 101);
        assert!((299_999..=300_000).contains(&tx.offset), "{}", tx.offset);
        // It converges to zero, and the clock goes back to its frequency.
        for s in 102..400 {
            ntp.second_overflow(s);
        }
        let mut tx = modes(0);
        ntp.adjtimex(&mut tx, 0, 400);
        assert_eq!(tx.offset, 0);
        assert_eq!(ntp.second_length() as i128, NOMINAL + freq);
    }

    /// maxerror grows 500 us a second; at the limit the clock is
    /// unsynchronized.
    #[test]
    fn maxerror_grows_until_unsynchronized() {
        let mut ntp = Ntp::new();
        let mut tx = modes(ADJ_STATUS | ADJ_MAXERROR);
        tx.status = 0;
        tx.maxerror = NTP_PHASE_LIMIT - 700;
        assert_eq!(ntp.adjtimex(&mut tx, 0, 0), TIME_OK);
        assert!(ntp.is_active());
        ntp.second_overflow(1);
        let mut tx = modes(0);
        assert_eq!(ntp.adjtimex(&mut tx, 0, 1), TIME_OK);
        assert_eq!(tx.maxerror, NTP_PHASE_LIMIT - 200);
        ntp.second_overflow(2);
        let mut tx = modes(0);
        assert_eq!(ntp.adjtimex(&mut tx, 0, 2), TIME_ERROR);
        assert_eq!((tx.maxerror, tx.status), (NTP_PHASE_LIMIT, STA_UNSYNC));
    }

    const DAY: i64 = 86_400;

    /// STA_INS: the second after 23:59:59 is 23:59:59 again (the leap
    /// second, read through adjtimex as 23:59:60 in TIME_OOP), TAI moves
    /// one second further from UTC, and the state ends in TIME_WAIT until
    /// the flag is cleared.
    #[test]
    fn leap_second_insert() {
        let mut ntp = Ntp::new();
        let midnight = 20_000 * DAY;
        // Synchronized (a small maxerror, as ntpd sets): else the state
        // reads TIME_ERROR, as on Linux.
        let mut tx = modes(ADJ_STATUS | ADJ_MAXERROR);
        tx.status = STA_INS;
        tx.maxerror = 0;
        ntp.adjtimex(&mut tx, 0, midnight - 2);
        assert!(ntp.is_active());
        assert_eq!(ntp.second_overflow(midnight - 1), 0);
        assert_eq!(ntp.time_state, TIME_INS);
        assert_eq!(ntp.next_leap_sec, midnight);
        // Midnight reached but not yet processed: adjtimex reports the leap.
        let mut tx = modes(0);
        let state = ntp.adjtimex(&mut tx, midnight * NSEC_PER_SEC + 5_000, midnight);
        assert_eq!((state, tx.time_sec, tx.tai), (TIME_OOP, midnight - 1, 1));
        // The leap: the clock goes back one second.
        assert_eq!(ntp.second_overflow(midnight), -1);
        assert_eq!((ntp.time_state, ntp.tai()), (TIME_OOP, 1));
        let mut tx = modes(0);
        assert_eq!(
            ntp.adjtimex(&mut tx, (midnight - 1) * NSEC_PER_SEC, midnight - 1),
            TIME_OOP
        );
        // The repeated second ends: midnight comes once more, without a
        // second leap.
        assert_eq!(ntp.second_overflow(midnight), 0);
        assert_eq!(ntp.time_state, TIME_WAIT);
        assert_eq!(ntp.second_overflow(midnight + 1), 0);
        assert_eq!(ntp.time_state, TIME_WAIT);
        let mut tx = modes(ADJ_STATUS);
        tx.status = 0;
        ntp.adjtimex(&mut tx, 0, midnight + 1);
        ntp.second_overflow(midnight + 2);
        assert_eq!(ntp.time_state, TIME_OK);
        assert_eq!(ntp.tai(), 1);
    }

    /// STA_DEL: 23:59:59 is skipped (the clock goes ahead one second at
    /// its start).
    #[test]
    fn leap_second_delete() {
        let mut ntp = Ntp::new();
        let midnight = 20_000 * DAY;
        let mut tx = modes(ADJ_STATUS | ADJ_TAI);
        tx.status = STA_DEL;
        tx.constant = 37;
        ntp.adjtimex(&mut tx, 0, midnight - 3);
        assert_eq!(ntp.second_overflow(midnight - 2), 0);
        assert_eq!(
            (ntp.time_state, ntp.next_leap_sec),
            (TIME_DEL, midnight - 1)
        );
        assert_eq!(ntp.second_overflow(midnight - 1), 1);
        assert_eq!((ntp.time_state, ntp.tai()), (TIME_WAIT, 36));
    }

    /// Clearing STA_INS before midnight cancels the leap; turning the PLL
    /// off resets the leap state.
    #[test]
    fn leap_second_cancelled() {
        let mut ntp = Ntp::new();
        let midnight = 20_000 * DAY;
        let mut tx = modes(ADJ_STATUS);
        tx.status = STA_INS;
        ntp.adjtimex(&mut tx, 0, midnight - 5);
        ntp.second_overflow(midnight - 4);
        assert_eq!(ntp.time_state, TIME_INS);
        let mut tx = modes(ADJ_STATUS);
        tx.status = 0;
        ntp.adjtimex(&mut tx, 0, midnight - 4);
        assert_eq!(ntp.second_overflow(midnight - 3), 0);
        assert_eq!((ntp.time_state, ntp.next_leap_sec), (TIME_OK, i64::MAX));
        assert_eq!(ntp.second_overflow(midnight), 0);

        let mut tx = modes(ADJ_STATUS);
        tx.status = STA_PLL | STA_INS;
        ntp.adjtimex(&mut tx, 0, midnight + 10);
        ntp.second_overflow(midnight + 11);
        assert_eq!(ntp.time_state, TIME_INS);
        let mut tx = modes(ADJ_STATUS);
        tx.status = 0;
        ntp.adjtimex(&mut tx, 0, midnight + 11);
        assert_eq!((ntp.time_state, ntp.next_leap_sec), (TIME_OK, i64::MAX));
    }

    /// With nothing to deliver and nothing to watch, the clock needs no
    /// per-second work.
    #[test]
    fn idle_state_needs_no_seconds() {
        assert!(!Ntp::new().is_active());
    }

    #[test]
    fn negative_time_rounds_down() {
        let mut ntp = Ntp::new();
        let mut tx = modes(0);
        ntp.adjtimex(&mut tx, -1, 0);
        assert_eq!((tx.time_sec, tx.time_usec), (-1, 999_999));
        assert_eq!(shift_right(-5, 1), -2);
    }
}
