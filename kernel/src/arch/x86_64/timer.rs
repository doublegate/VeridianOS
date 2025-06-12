//! x86_64 timer implementation

use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);

/// Get current timer ticks
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Increment timer ticks (called from timer interrupt)
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);

    // Trigger scheduler tick
    crate::sched::timer_tick();
}

/// Setup timer for periodic interrupts
pub fn setup_timer(interval_ms: u32) {
    // For now, we'll use the PIT (Programmable Interval Timer)
    // In a real implementation, we'd use the APIC timer

    const PIT_FREQUENCY: u32 = 1193182; // Hz
    let divisor = PIT_FREQUENCY / (1000 / interval_ms);

    unsafe {
        use x86_64::instructions::port::Port;

        // Command port
        let mut cmd_port: Port<u8> = Port::new(0x43);
        // Channel 0 data port
        let mut data_port: Port<u8> = Port::new(0x40);

        // Configure PIT channel 0 for periodic interrupts
        cmd_port.write(0x36); // Channel 0, lobyte/hibyte, rate generator

        // Set frequency divisor
        data_port.write((divisor & 0xFF) as u8);
        data_port.write((divisor >> 8) as u8);
    }

    // Enable timer interrupt (IRQ0 = interrupt 32)
    // This would be done in the IDT setup
    println!("[TIMER] Configured PIT for {}ms intervals", interval_ms);
}
