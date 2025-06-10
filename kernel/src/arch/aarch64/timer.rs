//! AArch64 timer implementation

use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);

/// Get current timer ticks
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Increment timer ticks (called from timer interrupt)
#[allow(dead_code)]
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);

    // Trigger scheduler tick
    crate::sched::timer_tick();
}

/// Setup timer for periodic interrupts
pub fn setup_timer(interval_ms: u32) {
    unsafe {
        // Read counter frequency
        let cntfrq: u64;
        core::arch::asm!("mrs {}, CNTFRQ_EL0", out(reg) cntfrq);

        // Calculate timer value for desired interval
        let tval = (cntfrq * interval_ms as u64) / 1000;

        // Set timer value
        core::arch::asm!("msr CNTP_TVAL_EL0, {}", in(reg) tval);

        // Enable timer interrupt
        core::arch::asm!("msr CNTP_CTL_EL0, {}", in(reg) 1u64);
    }

    println!(
        "[TIMER] Configured generic timer for {}ms intervals",
        interval_ms
    );
}
