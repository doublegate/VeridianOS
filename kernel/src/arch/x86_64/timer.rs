//! x86_64 timer implementation

use core::sync::atomic::{AtomicU64, Ordering};

static TICKS: AtomicU64 = AtomicU64::new(0);

/// Get current timer ticks
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Increment timer ticks (called from APIC timer interrupt handler at vector
/// 48).
///
/// Every CPU ticks its own run queue (time slices, CPU accounting); only
/// the timekeeping CPU also advances the tick count, the timer wheel and the
/// console input (see `is_timekeeper`), and wakes expired sleepers
/// (`dispatch::tick`).
pub fn tick(user: bool) {
    if crate::arch::percpu::is_timekeeper() {
        TICKS.fetch_add(1, Ordering::Relaxed);
        crate::timer::clock_tick();
        #[cfg(feature = "alloc")]
        crate::drivers::console_input::tick();
    }
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::tick(user);
    #[cfg(not(feature = "alloc"))]
    let _ = user;
    // In TSC-deadline mode the timer is one-shot: arm the next tick, after
    // the work and against the time it ended, so that a tick outlasting the
    // period still leaves the interrupted code a whole one (arming first
    // left the deadline already past: an interrupt livelock). Interrupts
    // stay off until the return, so nothing fires early.
    super::apic::rearm_deadline_timer();
}

/// Setup timer for periodic interrupts
pub fn setup_timer(interval_ms: u32) {
    // For now, we'll use the PIT (Programmable Interval Timer)
    // In a real implementation, we'd use the APIC timer

    const PIT_FREQUENCY: u32 = 1193182; // Hz
    let divisor = PIT_FREQUENCY / (1000 / interval_ms);

    // SAFETY: I/O port writes to the 8254 PIT (Programmable Interval Timer).
    // Port 0x43 is the command register, port 0x40 is channel 0 data. The
    // command 0x36 configures channel 0 for rate generator mode with
    // lobyte/hibyte access. The divisor is split into low and high bytes.
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
