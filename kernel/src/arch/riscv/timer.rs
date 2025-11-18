//! RISC-V timer implementation

use core::sync::atomic::{AtomicU64, Ordering};
use super::sbi;

static TICKS: AtomicU64 = AtomicU64::new(0);
static TIMER_INTERVAL: AtomicU64 = AtomicU64::new(0);

/// Get current timer ticks
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Increment timer ticks (called from timer interrupt)
#[allow(dead_code)]
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);

    // Schedule next timer interrupt
    let interval = TIMER_INTERVAL.load(Ordering::Relaxed);
    if interval > 0 {
        unsafe {
            let time: u64;
            core::arch::asm!("rdtime {}", out(reg) time);
            let _ = sbi::set_timer(time + interval);
        }
    }

    // Trigger scheduler tick
    crate::sched::timer_tick();
}

/// Read current time value
pub fn read_time() -> u64 {
    let time: u64;
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) time);
    }
    time
}

/// Setup timer for periodic interrupts
pub fn setup_timer(interval_ms: u32) {
    // Timer frequency is platform-dependent, but typically 10 MHz for QEMU
    // For QEMU virt machine, the timebase frequency is 10 MHz
    const TIMER_FREQ: u64 = 10_000_000; // 10 MHz
    let interval_cycles = (TIMER_FREQ * interval_ms as u64) / 1000;

    // Store interval for use in tick handler
    TIMER_INTERVAL.store(interval_cycles, Ordering::Relaxed);

    // Read current time and set first timer interrupt
    let current_time = read_time();
    let next_time = current_time + interval_cycles;

    // Use SBI to set timer
    let result = sbi::set_timer(next_time);
    if !result.is_ok() {
        println!("[TIMER] WARNING: SBI set_timer failed with error {}", result.error);
    }

    // Enable supervisor timer interrupts in SIE
    unsafe {
        // Set STIE bit (bit 5) in sie register
        core::arch::asm!("csrs sie, {}", in(reg) 1u64 << 5);
    }

    println!(
        "[TIMER] Configured RISC-V timer for {}ms intervals ({} cycles)",
        interval_ms, interval_cycles
    );
    println!("[TIMER] Current time: {}, Next interrupt: {}", current_time, next_time);
}
