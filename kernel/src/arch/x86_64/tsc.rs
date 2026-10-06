//! Time Stamp Counter: the x86_64 clock source.
//!
//! The kernel's monotonic clock is the TSC, read with RDTSC (unprivileged,
//! ~20 cycles, no I/O exit under KVM). Its frequency is taken, in order of
//! preference, from:
//!
//! 1. CPUID leaf 0x15 (TSC / core-crystal ratio and crystal frequency), with
//!    leaf 0x16's base frequency standing in for a missing crystal value --
//!    exact on Intel parts that report it;
//! 2. the hypervisor timing leaf 0x40000010 (EAX = TSC kHz), which KVM, EC2
//!    Nitro and VMware expose and FreeBSD/XNU guests use;
//! 3. calibration against PIT channel 2 (median of several 25 ms windows),
//!    which works everywhere, including QEMU's default CPU models.
//!
//! CPUID 0x80000007 EDX[8] (invariant TSC) says the rate is constant across
//! P-/C-states; without it the clock is still monotonic but may drift with
//! frequency changes, which is reported at boot.
//!
//! The timer interrupt uses the LAPIC in TSC-deadline mode when CPUID.1
//! ECX[24] says it is available (see `apic::start_timer`), so the tick is
//! programmed in the same units as the clock.

use core::{
    arch::x86_64::{__cpuid, __cpuid_count, _rdtsc},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Calibrated TSC frequency in Hz (0 until `init`).
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// TSC value at `init`: the zero of the monotonic clock.
static TSC_AT_BOOT: AtomicU64 = AtomicU64::new(0);
/// CPUID.1:ECX[24] -- LAPIC TSC-deadline mode.
static DEADLINE_MODE: AtomicBool = AtomicBool::new(false);

/// Where the frequency came from (for the boot log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TscSource {
    Cpuid15,
    Hypervisor,
    Pit,
}

/// Read the TSC.
#[inline]
pub fn read() -> u64 {
    // SAFETY: RDTSC is unprivileged and has no side effects.
    unsafe { _rdtsc() }
}

/// TSC frequency in Hz, or 0 before calibration.
pub fn hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}

/// Whether the LAPIC supports TSC-deadline mode.
pub fn deadline_mode_supported() -> bool {
    DEADLINE_MODE.load(Ordering::Relaxed)
}

/// Nanoseconds since `init` (0 before calibration).
pub fn ns_since_boot() -> u64 {
    let hz = hz();
    if hz == 0 {
        return 0;
    }
    let ticks = read().wrapping_sub(TSC_AT_BOOT.load(Ordering::Relaxed));
    ticks_to_ns(ticks, hz)
}

/// Convert TSC ticks to nanoseconds at `hz` without overflow.
pub fn ticks_to_ns(ticks: u64, hz: u64) -> u64 {
    ((ticks as u128 * 1_000_000_000) / hz as u128) as u64
}

/// Convert nanoseconds to TSC ticks at `hz`.
pub fn ns_to_ticks(ns: u64, hz: u64) -> u64 {
    ((ns as u128 * hz as u128) / 1_000_000_000) as u64
}

/// TSC frequency from CPUID leaf 0x15 (and 0x16), if reported.
fn from_cpuid_15() -> Option<u64> {
    // SAFETY: CPUID is available on every x86_64 CPU.
    let max_leaf = unsafe { __cpuid(0) }.eax;
    if max_leaf < 0x15 {
        return None;
    }
    // SAFETY: leaf 0x15 exists (max_leaf checked).
    let l15 = unsafe { __cpuid(0x15) };
    let (den, num, crystal) = (l15.eax as u64, l15.ebx as u64, l15.ecx as u64);
    if den == 0 || num == 0 {
        return None;
    }
    if crystal != 0 {
        return Some(crystal * num / den);
    }
    // Crystal frequency not enumerated: leaf 0x16 EAX is the base (TSC)
    // frequency in MHz on these parts.
    if max_leaf >= 0x16 {
        // SAFETY: leaf 0x16 exists (max_leaf checked).
        let base_mhz = unsafe { __cpuid(0x16) }.eax as u64;
        if base_mhz != 0 {
            return Some(base_mhz * 1_000_000);
        }
    }
    None
}

/// TSC frequency from the hypervisor timing leaf 0x40000010, if present.
fn from_hypervisor_leaf() -> Option<u64> {
    // SAFETY: CPUID leaf 1 always exists.
    let hypervisor = unsafe { __cpuid(1) }.ecx & (1 << 31) != 0;
    if !hypervisor {
        return None;
    }
    // SAFETY: with the hypervisor bit set, leaf 0x40000000 reports the
    // highest hypervisor leaf.
    let max = unsafe { __cpuid(0x4000_0000) }.eax;
    if max < 0x4000_0010 {
        return None;
    }
    // SAFETY: leaf 0x40000010 exists (max checked).
    let khz = unsafe { __cpuid_count(0x4000_0010, 0) }.eax as u64;
    (khz != 0).then_some(khz * 1000)
}

/// Measure TSC ticks over `pit_ticks` of PIT channel 2 (1.193182 MHz).
fn pit_window(pit_ticks: u16) -> u64 {
    // SAFETY: standard PIT/port-0x61 programming, as in apic::calibrate_timer:
    // 0x61 gates channel 2 (speaker off), 0x43 is the command port, 0x42
    // channel 2 data. Boot-time, interrupts off, single CPU.
    unsafe {
        use x86_64::instructions::port::Port;
        let mut port_61 = Port::<u8>::new(0x61);
        let mut cmd = Port::<u8>::new(0x43);
        let mut ch2 = Port::<u8>::new(0x42);

        let gate = port_61.read();
        port_61.write((gate & 0xFD) | 0x01);
        cmd.write(0xB0); // channel 2, lobyte/hibyte, mode 0
        ch2.write((pit_ticks & 0xFF) as u8);
        ch2.write((pit_ticks >> 8) as u8);

        let g = port_61.read();
        port_61.write(g & 0xFE);
        port_61.write(g | 0x01); // rising gate starts the count
        let start = read();
        while port_61.read() & 0x20 == 0 {
            core::hint::spin_loop();
        }
        read().wrapping_sub(start)
    }
}

/// TSC frequency by PIT calibration: median of five 25 ms windows.
fn from_pit() -> u64 {
    const PIT_HZ: u64 = 1_193_182;
    const WINDOW_MS: u64 = 25;
    let pit_ticks = (PIT_HZ * WINDOW_MS / 1000) as u16;
    let mut samples = [0u64; 5];
    for s in samples.iter_mut() {
        *s = pit_window(pit_ticks);
    }
    samples.sort_unstable();
    samples[2] * PIT_HZ / pit_ticks as u64
}

/// Calibrate the TSC and start the monotonic clock. Call once, early, with
/// interrupts disabled (the PIT fallback busy-waits).
pub fn init() -> (u64, TscSource) {
    // SAFETY: CPUID leaf 1 always exists.
    let tsc_deadline = unsafe { __cpuid(1) }.ecx & (1 << 24) != 0;
    DEADLINE_MODE.store(tsc_deadline, Ordering::Relaxed);

    let (hz, source) = if let Some(hz) = from_cpuid_15() {
        (hz, TscSource::Cpuid15)
    } else if let Some(hz) = from_hypervisor_leaf() {
        (hz, TscSource::Hypervisor)
    } else {
        (from_pit(), TscSource::Pit)
    };
    TSC_AT_BOOT.store(read(), Ordering::Relaxed);
    TSC_HZ.store(hz, Ordering::Release);
    (hz, source)
}

/// CPUID 0x80000007 EDX[8]: the TSC runs at a constant rate.
pub fn invariant() -> bool {
    // SAFETY: CPUID is always available; the extended leaf is checked.
    unsafe { __cpuid(0x8000_0000).eax >= 0x8000_0007 && __cpuid(0x8000_0007).edx & (1 << 8) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip_without_overflow() {
        let hz = 3_600_000_000;
        // One hour of ticks would overflow u64 if multiplied naively.
        let hour = 3600 * hz;
        assert_eq!(ticks_to_ns(hour, hz), 3_600_000_000_000);
        assert_eq!(ns_to_ticks(1_000_000, hz), 3_600_000);
        // Both conversions floor, so a round trip may lose under 1 ns.
        let back = ticks_to_ns(ns_to_ticks(123_456_789, hz), hz);
        assert!((123_456_788..=123_456_789).contains(&back));
    }
}
