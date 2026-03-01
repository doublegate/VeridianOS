//! CMOS Real-Time Clock (RTC) reader for x86_64.
//!
//! Reads date/time from the MC146818-compatible CMOS RTC via I/O ports
//! 0x70 (index) and 0x71 (data). Converts BCD-encoded values to binary
//! and provides a Unix-epoch-relative timestamp for the panel clock.

use core::sync::atomic::{AtomicU64, Ordering};

/// Seconds since Unix epoch at kernel boot time.
static BOOT_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Millisecond-granularity uptime counter base (TSC ticks at boot).
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

/// RTC time snapshot.
#[derive(Debug, Clone, Copy)]
pub struct RtcTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Read the CMOS RTC and initialize the boot epoch.
pub fn init() {
    let time = read_rtc();
    let epoch = rtc_to_epoch(&time);
    BOOT_EPOCH.store(epoch, Ordering::Relaxed);
    BOOT_TSC.store(crate::arch::timer::read_hw_timestamp(), Ordering::Relaxed);
    crate::println!(
        "[RTC] Boot time: {:04}-{:02}-{:02} {:02}:{:02}:{:02} (epoch={})",
        time.year,
        time.month,
        time.day,
        time.hour,
        time.minute,
        time.second,
        epoch,
    );
}

/// Get current wall-clock seconds since Unix epoch.
///
/// Adds elapsed uptime (from TSC) to the boot-time RTC snapshot.
pub fn current_epoch_secs() -> u64 {
    let boot = BOOT_EPOCH.load(Ordering::Relaxed);
    let boot_tsc = BOOT_TSC.load(Ordering::Relaxed);
    let now_tsc = crate::arch::timer::read_hw_timestamp();
    let elapsed_ticks = now_tsc.saturating_sub(boot_tsc);
    // Approximate TSC as ~1 GHz (typical for QEMU with KVM)
    let elapsed_secs = elapsed_ticks / 1_000_000_000;
    boot + elapsed_secs
}

/// Read the RTC registers, handling BCD conversion and update-in-progress.
pub fn read_rtc() -> RtcTime {
    // Wait for any in-progress update to complete
    wait_for_update();

    let mut second = read_cmos(0x00);
    let mut minute = read_cmos(0x02);
    let mut hour = read_cmos(0x04);
    let mut day = read_cmos(0x07);
    let mut month = read_cmos(0x08);
    let mut year = read_cmos(0x09);
    let century = read_cmos(0x32); // Century register (ACPI FADT)
    let reg_b = read_cmos(0x0B);

    // BCD to binary conversion (if register B bit 2 is clear)
    if reg_b & 0x04 == 0 {
        second = bcd_to_bin(second);
        minute = bcd_to_bin(minute);
        hour = bcd_to_bin(hour & 0x7F) | (hour & 0x80); // preserve AM/PM bit
        day = bcd_to_bin(day);
        month = bcd_to_bin(month);
        year = bcd_to_bin(year);
    }

    // 12-hour to 24-hour conversion (if register B bit 1 is clear)
    if reg_b & 0x02 == 0 && hour & 0x80 != 0 {
        hour = ((hour & 0x7F) + 12) % 24;
    }

    let century_val = if century > 0 {
        if reg_b & 0x04 == 0 {
            bcd_to_bin(century)
        } else {
            century
        }
    } else {
        20 // Default to 21st century
    };

    let full_year = (century_val as u16) * 100 + (year as u16);

    RtcTime {
        year: full_year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// Convert RTC time to seconds since Unix epoch (1970-01-01 00:00:00 UTC).
fn rtc_to_epoch(t: &RtcTime) -> u64 {
    // Days from 1970-01-01 to the start of the given year
    let mut days: u64 = 0;
    for y in 1970..t.year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    // Days in months of the current year
    let month_days: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..t.month {
        if m == 2 && is_leap(t.year) {
            days += 29;
        } else if (m as usize) <= 12 {
            days += month_days[(m - 1) as usize] as u64;
        }
    }
    days += (t.day as u64).saturating_sub(1);

    days * 86400 + (t.hour as u64) * 3600 + (t.minute as u64) * 60 + (t.second as u64)
}

fn is_leap(y: u16) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Wait until the RTC update-in-progress flag (register 0x0A bit 7) clears.
fn wait_for_update() {
    // Spin for at most ~2ms worth of iterations
    for _ in 0..10_000 {
        if read_cmos(0x0A) & 0x80 == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Read a CMOS register.
fn read_cmos(reg: u8) -> u8 {
    // SAFETY: Ports 0x70/0x71 are the standard CMOS RTC index/data ports.
    // Writing the register index to 0x70 and reading from 0x71 is the
    // defined access protocol. NMI disable bit (0x80) is preserved as 0.
    unsafe {
        crate::arch::x86_64::outb(0x70, reg);
        crate::arch::x86_64::inb(0x71)
    }
}

/// Convert BCD-encoded byte to binary.
fn bcd_to_bin(bcd: u8) -> u8 {
    (bcd >> 4) * 10 + (bcd & 0x0F)
}
