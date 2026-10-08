//! Load averages (N-223): Linux's calc_load.
//!
//! Every five seconds the number of runnable tasks is folded into three
//! exponentially decaying averages, over 1, 5 and 15 minutes, in 11-bit
//! fixed point. /proc/loadavg and sysinfo report them.

use core::sync::atomic::{AtomicU64, Ordering};

/// Fixed-point fraction bits (Linux FSHIFT).
pub const FSHIFT: u32 = 11;
/// 1.0 in fixed point.
pub const FIXED_1: u64 = 1 << FSHIFT;
/// Sampling period: five seconds.
pub const LOAD_FREQ_NS: u64 = 5_000_000_000;
/// exp(-5 s / 1 min), exp(-5 s / 5 min), exp(-5 s / 15 min) in fixed point.
const EXP: [u64; 3] = [1884, 2014, 2037];

/// One step of an average toward `active` (tasks, in fixed point), rounding
/// up while it rises, as Linux does so a steady load is reached.
pub fn calc_load(load: u64, exp: u64, active: u64) -> u64 {
    let mut new = load * exp + active * (FIXED_1 - exp);
    if active >= load {
        new += FIXED_1 - 1;
    }
    new / FIXED_1
}

/// The averages, and when they were last sampled.
static AVENRUN: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LAST_SAMPLE_NS: AtomicU64 = AtomicU64::new(0);

/// Called on every tick with the number of runnable tasks: folds it in
/// once per period (a missed period counts the current load for each).
pub fn tick(now_ns: u64, nr_active: u64) {
    let last = LAST_SAMPLE_NS.load(Ordering::Relaxed);
    if now_ns < last.saturating_add(LOAD_FREQ_NS) {
        return;
    }
    if LAST_SAMPLE_NS
        .compare_exchange(last, now_ns, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let periods = if last == 0 {
        1
    } else {
        ((now_ns - last) / LOAD_FREQ_NS).clamp(1, 64)
    };
    let active = nr_active.saturating_mul(FIXED_1);
    for (avg, &exp) in AVENRUN.iter().zip(EXP.iter()) {
        let mut load = avg.load(Ordering::Relaxed);
        for _ in 0..periods {
            load = calc_load(load, exp, active);
        }
        avg.store(load, Ordering::Relaxed);
    }
}

/// The 1, 5 and 15 minute averages, in fixed point (FSHIFT bits).
pub fn averages() -> [u64; 3] {
    [
        AVENRUN[0].load(Ordering::Relaxed),
        AVENRUN[1].load(Ordering::Relaxed),
        AVENRUN[2].load(Ordering::Relaxed),
    ]
}

/// A fixed-point average as /proc/loadavg prints it: integer part and two
/// decimals, rounded (Linux's LOAD_INT and LOAD_FRAC).
pub fn format_parts(load: u64) -> (u64, u64) {
    let rounded = load + FIXED_1 / 200;
    (
        rounded >> FSHIFT,
        ((rounded & (FIXED_1 - 1)) * 100) >> FSHIFT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady load of one task approaches 1.00 within the window; no
    /// load decays back towards zero.
    #[test]
    fn averages_converge_and_decay() {
        let mut load = 0;
        for _ in 0..12 * 15 {
            load = calc_load(load, EXP[0], FIXED_1);
        }
        assert_eq!(format_parts(load), (1, 0));
        // One minute (12 periods) of no load: down to about e^-1.
        for _ in 0..12 {
            load = calc_load(load, EXP[0], 0);
        }
        let (int, frac) = format_parts(load);
        assert_eq!(int, 0);
        assert!((35..=39).contains(&frac), "{frac}");
        assert_eq!(format_parts(0), (0, 0));
        assert_eq!(format_parts(FIXED_1 * 3 / 2), (1, 50));
    }
}
