//! RISC-V timer implementation

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
    // RISC-V uses memory-mapped timer compare register
    // This is typically provided by the platform (SBI)

    // For now, we'll use a simple approximation
    // In real implementation, we'd use SBI timer calls
    const TIMER_FREQ: u64 = 10_000_000; // 10 MHz typical
    let interval_cycles = (TIMER_FREQ * interval_ms as u64) / 1000;

    unsafe {
        // Read current time
        let time: u64;
        core::arch::asm!("rdtime {}", out(reg) time);

        // Set next timer interrupt
        // This would typically be done via SBI ecall
        let _next_time = time + interval_cycles;

        // Enable timer interrupt
        core::arch::asm!("csrs mie, {}", in(reg) 1u64 << 5); // MIE.MTIE
    }

    println!(
        "[TIMER] Configured RISC-V timer for {}ms intervals",
        interval_ms
    );
}
