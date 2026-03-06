//! CMOS Real-Time Clock (RTC) reader for x86_64.
//!
//! Reads date/time from the MC146818-compatible CMOS RTC via I/O ports
//! 0x70 (index) and 0x71 (data). Converts BCD-encoded values to binary
//! and provides a Unix-epoch-relative timestamp for the panel clock.

use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

/// Seconds since Unix epoch at kernel boot time.
static BOOT_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Millisecond-granularity uptime counter base (TSC ticks at boot).
static BOOT_TSC: AtomicU64 = AtomicU64::new(0);

/// Timezone offset from UTC in seconds (e.g., -18000 for EST/UTC-5).
static TIMEZONE_OFFSET: AtomicI64 = AtomicI64::new(0);

/// NTP time correction in milliseconds (positive = clock behind, negative =
/// clock ahead).
static NTP_CORRECTION_MS: AtomicI64 = AtomicI64::new(0);

/// Whether an alarm interrupt is currently enabled.
static ALARM_ENABLED: AtomicBool = AtomicBool::new(false);

/// Alarm callback function pointer (set by `set_alarm`).
static ALARM_CALLBACK: AtomicU64 = AtomicU64::new(0);

/// Ioctl command: set alarm time.
pub const RTC_ALM_SET: u32 = 0x01;
/// Ioctl command: enable alarm interrupt.
pub const RTC_AIE_ON: u32 = 0x02;
/// Ioctl command: disable alarm interrupt.
pub const RTC_AIE_OFF: u32 = 0x03;

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
/// Adds elapsed uptime (from TSC) to the boot-time RTC snapshot,
/// then applies timezone offset and NTP correction.
pub fn current_epoch_secs() -> u64 {
    let boot = BOOT_EPOCH.load(Ordering::Relaxed);
    let boot_tsc = BOOT_TSC.load(Ordering::Relaxed);
    let now_tsc = crate::arch::timer::read_hw_timestamp();
    let elapsed_ticks = now_tsc.saturating_sub(boot_tsc);
    // Approximate TSC as ~1 GHz (typical for QEMU with KVM)
    let elapsed_secs = elapsed_ticks / 1_000_000_000;
    let base = boot + elapsed_secs;

    // Apply NTP correction (milliseconds -> seconds, truncated)
    let ntp_ms = NTP_CORRECTION_MS.load(Ordering::Relaxed);
    let ntp_secs = ntp_ms / 1000;
    let corrected = (base as i64).saturating_add(ntp_secs);

    // Apply timezone offset
    let tz_offset = TIMEZONE_OFFSET.load(Ordering::Relaxed);
    let adjusted = corrected.saturating_add(tz_offset);

    // Clamp to non-negative (Unix epoch cannot go before 1970)
    if adjusted < 0 {
        0
    } else {
        adjusted as u64
    }
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
#[cfg(target_os = "none")]
fn read_cmos(reg: u8) -> u8 {
    // SAFETY: Ports 0x70/0x71 are the standard CMOS RTC index/data ports.
    // Writing the register index to 0x70 and reading from 0x71 is the
    // defined access protocol. NMI disable bit (0x80) is preserved as 0.
    unsafe {
        crate::arch::x86_64::outb(0x70, reg);
        crate::arch::x86_64::inb(0x71)
    }
}

/// Host stub: CMOS registers are not available on user-space targets.
#[cfg(not(target_os = "none"))]
fn read_cmos(_reg: u8) -> u8 {
    0
}

/// Convert BCD-encoded byte to binary.
fn bcd_to_bin(bcd: u8) -> u8 {
    (bcd >> 4) * 10 + (bcd & 0x0F)
}

/// Convert binary value to BCD encoding.
#[allow(dead_code)]
fn bin_to_bcd(val: u8) -> u8 {
    ((val / 10) << 4) | (val % 10)
}

// ---------------------------------------------------------------------------
// Alarm registers
// ---------------------------------------------------------------------------

/// Set the CMOS alarm to fire at the given hour:minute:second.
///
/// This programs CMOS alarm registers 0x01 (seconds), 0x03 (minutes),
/// 0x05 (hours) and enables the alarm interrupt bit in Register B.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn set_alarm(hour: u8, minute: u8, second: u8) {
    let reg_b = read_cmos(0x0B);
    let is_bcd = reg_b & 0x04 == 0;

    let (s, m, h) = if is_bcd {
        (bin_to_bcd(second), bin_to_bcd(minute), bin_to_bcd(hour))
    } else {
        (second, minute, hour)
    };

    write_cmos(0x01, s); // Alarm seconds
    write_cmos(0x03, m); // Alarm minutes
    write_cmos(0x05, h); // Alarm hours

    // Enable alarm interrupt (Register B, bit 5)
    let new_b = reg_b | 0x20;
    write_cmos(0x0B, new_b);
    ALARM_ENABLED.store(true, Ordering::Relaxed);

    crate::println!(
        "[RTC] Alarm set for {:02}:{:02}:{:02}",
        hour,
        minute,
        second,
    );
}

/// Disable the CMOS alarm interrupt.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn disable_alarm() {
    let reg_b = read_cmos(0x0B);
    write_cmos(0x0B, reg_b & !0x20); // Clear bit 5
    ALARM_ENABLED.store(false, Ordering::Relaxed);
}

/// Non-x86_64 stub for `set_alarm`.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn set_alarm(_hour: u8, _minute: u8, _second: u8) {}

/// Non-x86_64 stub for `disable_alarm`.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn disable_alarm() {
    ALARM_ENABLED.store(false, Ordering::Relaxed);
}

/// Register a callback function pointer to invoke when the alarm fires.
pub fn set_alarm_callback(callback: fn()) {
    ALARM_CALLBACK.store(callback as usize as u64, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// IRQ 8 handler
// ---------------------------------------------------------------------------

/// RTC interrupt handler (IRQ 8, vector 40 on PIC / remapped on APIC).
///
/// Acknowledges the interrupt by reading Register C, then dispatches the
/// alarm callback if an alarm interrupt occurred (bit 5 of Register C).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn rtc_interrupt_handler() {
    // Reading Register C clears the interrupt flags and allows future IRQs.
    let status = read_cmos(0x0C);

    // Bit 5: alarm interrupt flag
    if status & 0x20 != 0 && ALARM_ENABLED.load(Ordering::Relaxed) {
        let cb_ptr = ALARM_CALLBACK.load(Ordering::Relaxed);
        if cb_ptr != 0 {
            // SAFETY: The callback was set via `set_alarm_callback` which takes
            // a valid `fn()` pointer. We reconstruct it here.
            let callback: fn() = unsafe { core::mem::transmute(cb_ptr as usize) };
            callback();
        }
    }
}

/// Non-x86_64 stub for the RTC interrupt handler.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
pub fn rtc_interrupt_handler() {}

// ---------------------------------------------------------------------------
// Timezone configuration
// ---------------------------------------------------------------------------

/// Set the timezone offset from UTC in seconds.
///
/// Positive values are east of UTC, negative values west.
/// Example: EST (UTC-5) = -18000, CET (UTC+1) = 3600.
pub fn set_timezone_offset(seconds: i64) {
    TIMEZONE_OFFSET.store(seconds, Ordering::Relaxed);
}

/// Get the current timezone offset in seconds.
pub fn get_timezone_offset() -> i64 {
    TIMEZONE_OFFSET.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// NTP integration point
// ---------------------------------------------------------------------------

/// Apply a time correction from an NTP client.
///
/// `offset_ms` is the difference between NTP server time and local time
/// in milliseconds. Positive means local clock is behind (needs advancing),
/// negative means local clock is ahead (needs slowing).
///
/// The correction is applied atomically and takes effect on the next call
/// to `current_epoch_secs()`.
pub fn set_time_correction(offset_ms: i64) {
    NTP_CORRECTION_MS.store(offset_ms, Ordering::Relaxed);
}

/// Get the current NTP correction in milliseconds.
pub fn get_time_correction() -> i64 {
    NTP_CORRECTION_MS.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// /dev/rtc interface stubs
// ---------------------------------------------------------------------------

/// Alarm time for ioctl RTC_ALM_SET.
#[derive(Debug, Clone, Copy)]
pub struct RtcAlarm {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Read the current RTC time (/dev/rtc read interface).
///
/// Returns the current wall-clock time as an `RtcTime` struct.
pub fn rtc_read() -> RtcTime {
    read_rtc()
}

/// Perform an RTC ioctl operation.
///
/// Supported commands:
/// - `RTC_ALM_SET`: Set alarm time (arg interpreted as `&RtcAlarm`)
/// - `RTC_AIE_ON`: Enable alarm interrupt
/// - `RTC_AIE_OFF`: Disable alarm interrupt
///
/// Returns 0 on success, -1 on invalid command.
pub fn rtc_ioctl(cmd: u32, arg: u64) -> i32 {
    match cmd {
        RTC_ALM_SET => {
            // arg is a pointer to RtcAlarm in the caller's address space.
            // In kernel context we treat it as a direct struct pointer.
            let alarm = unsafe { &*(arg as *const RtcAlarm) };
            if alarm.hour > 23 || alarm.minute > 59 || alarm.second > 59 {
                return -1;
            }
            set_alarm(alarm.hour, alarm.minute, alarm.second);
            0
        }
        RTC_AIE_ON => {
            ALARM_ENABLED.store(true, Ordering::Relaxed);
            0
        }
        RTC_AIE_OFF => {
            disable_alarm();
            0
        }
        _ => -1, // Unknown command
    }
}

// ---------------------------------------------------------------------------
// CMOS write helper
// ---------------------------------------------------------------------------

/// Write a value to a CMOS register.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn write_cmos(reg: u8, val: u8) {
    // SAFETY: Ports 0x70/0x71 are the standard CMOS RTC index/data ports.
    unsafe {
        crate::arch::x86_64::outb(0x70, reg);
        crate::arch::x86_64::outb(0x71, val);
    }
}

/// Non-x86_64 stub (no-op).
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
#[allow(dead_code)]
fn write_cmos(_reg: u8, _val: u8) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bcd_conversions() {
        // BCD -> binary
        assert_eq!(bcd_to_bin(0x59), 59);
        assert_eq!(bcd_to_bin(0x23), 23);
        assert_eq!(bcd_to_bin(0x00), 0);

        // binary -> BCD
        assert_eq!(bin_to_bcd(59), 0x59);
        assert_eq!(bin_to_bcd(23), 0x23);
        assert_eq!(bin_to_bcd(0), 0x00);

        // Round-trip
        for val in 0..=99u8 {
            assert_eq!(bcd_to_bin(bin_to_bcd(val)), val);
        }
    }

    #[test]
    fn test_timezone_offset() {
        // Default is 0 (UTC)
        set_timezone_offset(0);
        assert_eq!(get_timezone_offset(), 0);

        // EST (UTC-5)
        set_timezone_offset(-18000);
        assert_eq!(get_timezone_offset(), -18000);

        // CET (UTC+1)
        set_timezone_offset(3600);
        assert_eq!(get_timezone_offset(), 3600);

        // Reset
        set_timezone_offset(0);
    }

    #[test]
    fn test_ntp_correction() {
        // Default is 0
        set_time_correction(0);
        assert_eq!(get_time_correction(), 0);

        // Clock behind by 500ms
        set_time_correction(500);
        assert_eq!(get_time_correction(), 500);

        // Clock ahead by 200ms
        set_time_correction(-200);
        assert_eq!(get_time_correction(), -200);

        // Reset
        set_time_correction(0);
    }

    #[test]
    fn test_rtc_to_epoch_known_dates() {
        // 1970-01-01 00:00:00 = epoch 0
        let t = RtcTime {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(rtc_to_epoch(&t), 0);

        // 2000-01-01 00:00:00 = 946684800
        let t2 = RtcTime {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(rtc_to_epoch(&t2), 946684800);

        // 2026-03-05 12:30:45
        let t3 = RtcTime {
            year: 2026,
            month: 3,
            day: 5,
            hour: 12,
            minute: 30,
            second: 45,
        };
        let epoch3 = rtc_to_epoch(&t3);
        // Sanity check: should be > 2025-01-01 epoch (1735689600)
        assert!(epoch3 > 1_735_689_600);
    }

    #[test]
    fn test_rtc_read_returns_valid_time() {
        let time = rtc_read();
        // rtc_read() delegates to read_rtc() -- on host, CMOS reads return 0
        // so we just check the struct is constructible and fields are in range
        assert!(time.month <= 12);
        assert!(time.day <= 31);
        assert!(time.hour <= 23);
        assert!(time.minute <= 59);
        assert!(time.second <= 59);
    }

    #[test]
    fn test_rtc_ioctl_invalid_command() {
        assert_eq!(rtc_ioctl(0xFF, 0), -1);
    }

    #[test]
    fn test_rtc_ioctl_alarm_set() {
        let alarm = RtcAlarm {
            hour: 14,
            minute: 30,
            second: 0,
        };
        let result = rtc_ioctl(RTC_ALM_SET, &alarm as *const RtcAlarm as u64);
        assert_eq!(result, 0);

        // Invalid alarm time
        let bad_alarm = RtcAlarm {
            hour: 25, // invalid
            minute: 0,
            second: 0,
        };
        let result2 = rtc_ioctl(RTC_ALM_SET, &bad_alarm as *const RtcAlarm as u64);
        assert_eq!(result2, -1);
    }

    #[test]
    fn test_rtc_ioctl_alarm_enable_disable() {
        assert_eq!(rtc_ioctl(RTC_AIE_ON, 0), 0);
        assert!(ALARM_ENABLED.load(Ordering::Relaxed));

        assert_eq!(rtc_ioctl(RTC_AIE_OFF, 0), 0);
        assert!(!ALARM_ENABLED.load(Ordering::Relaxed));
    }

    #[test]
    fn test_alarm_callback_registration() {
        fn dummy_callback() {}
        set_alarm_callback(dummy_callback);
        let stored = ALARM_CALLBACK.load(Ordering::Relaxed);
        assert_ne!(stored, 0);
        // Reset
        ALARM_CALLBACK.store(0, Ordering::Relaxed);
    }
}
