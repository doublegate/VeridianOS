//! AArch64 timer: the EL1 virtual timer of the Generic Timer.
//!
//! Clock source: the virtual counter CNTVCT_EL0 at CNTFRQ_EL0 (see
//! `arch::timer`). The tick uses the matching EL1 virtual timer (CNTV_*),
//! which is what an EL1 kernel should use: it counts the same virtual time
//! as the clock, and under a hypervisor it is the timer the guest owns
//! (Linux's arm_arch_timer makes the same choice when not running at EL2).
//! Its interrupt is PPI 11, GIC INTID 27.
//!
//! Deadlines are absolute (CNTV_CVAL_EL0), advanced by whole periods so the
//! tick does not drift.

use core::sync::atomic::{AtomicU64, Ordering};

/// Timer interrupts taken.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Tick period in counter ticks (0 = not started).
static PERIOD: AtomicU64 = AtomicU64::new(0);
/// Tick period in milliseconds.
static PERIOD_MS: AtomicU64 = AtomicU64::new(0);
/// Deadline currently armed.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// CNTV_CTL_EL0: ENABLE, with IMASK clear.
const CTL_ENABLE: u64 = 1;

/// Get current timer ticks (interrupts taken).
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

fn counter() -> u64 {
    crate::arch::timer::read_hw_timestamp()
}

fn set_compare(deadline: u64) {
    // SAFETY: CNTV_CVAL_EL0 is the EL1 virtual timer compare value,
    // accessible at EL1; writing it re-arms (and deasserts) the timer.
    unsafe {
        core::arch::asm!("msr cntv_cval_el0, {0}", "isb", in(reg) deadline, options(nostack));
    }
}

/// Start a periodic tick at `hz`: arm the virtual timer and enable its PPI
/// at the GIC. The vector table must be installed and the GIC initialized;
/// the caller unmasks IRQs afterwards.
pub fn start(hz: u32) -> crate::error::KernelResult<()> {
    let freq = crate::arch::timer::hw_ticks_per_second();
    let period = (freq / hz.max(1) as u64).max(1);
    PERIOD.store(period, Ordering::Relaxed);
    PERIOD_MS.store((1000 / hz.max(1) as u64).max(1), Ordering::Relaxed);
    let first = counter() + period;
    NEXT.store(first, Ordering::Relaxed);
    set_compare(first);
    // SAFETY: enables the EL1 virtual timer with its interrupt unmasked.
    unsafe {
        core::arch::asm!("msr cntv_ctl_el0, {0}", "isb", in(reg) CTL_ENABLE, options(nostack));
    }
    super::gic::set_irq_priority(super::exceptions::VIRTUAL_TIMER_INTID, 0x80)?;
    super::gic::enable_irq(super::exceptions::VIRTUAL_TIMER_INTID)?;
    let _ = core::fmt::Write::write_fmt(
        &mut super::direct_uart::writer(),
        format_args!(
            "[TIMER] EL1 virtual timer: counter {} Hz, tick {} Hz ({} counts), INTID {}\n",
            freq,
            hz,
            period,
            super::exceptions::VIRTUAL_TIMER_INTID
        ),
    );
    Ok(())
}

/// Handle the virtual timer interrupt (called from the IRQ vector).
///
/// Re-arms first (the timer interrupt is level-triggered and stays asserted
/// until the compare value moves past the counter), skipping missed ticks
/// rather than firing a burst. Only the clock and the timer wheel advance:
/// preempting the interrupted kernel code from here is not supported yet.
pub fn handle_interrupt() {
    // Only this handler writes TICKS, so a plain load and store suffice. An
    // atomic read-modify-write would use exclusive load/store, which is not
    // architecturally reliable while the MMU is off (N-28).
    TICKS.store(
        TICKS.load(Ordering::Relaxed).wrapping_add(1),
        Ordering::Relaxed,
    );
    let period = PERIOD.load(Ordering::Relaxed);
    if period == 0 {
        // Not started: silence the timer.
        // SAFETY: disables the EL1 virtual timer.
        unsafe { core::arch::asm!("msr cntv_ctl_el0, xzr", "isb", options(nostack)) };
        return;
    }
    let now = counter();
    let mut next = NEXT.load(Ordering::Relaxed).wrapping_add(period);
    if next <= now {
        next = now + period;
    }
    NEXT.store(next, Ordering::Relaxed);
    set_compare(next);
    crate::timer::timer_tick(PERIOD_MS.load(Ordering::Relaxed));
}
