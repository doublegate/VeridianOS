//! Memory-mapped real-time clocks found through the device tree, which set
//! the wall clock (CLOCK_REALTIME) at boot on AArch64 and RISC-V (x86_64
//! reads the CMOS RTC, `x86_64::rtc`).
//!
//! - ARM PrimeCell PL031 (`arm,pl031`; QEMU `virt` on AArch64): RTCDR at offset
//!   0 holds the UTC seconds (PL031 TRM, DDI 0224).
//! - Goldfish RTC (`google,goldfish-rtc`; QEMU `virt` on RISC-V): TIME_LOW at
//!   0x00 and TIME_HIGH at 0x04 hold UTC nanoseconds; reading TIME_LOW latches
//!   TIME_HIGH (Linux `drivers/rtc/rtc-goldfish.c`).
//!
//! Without either the wall clock counts from boot, as before.

/// A supported clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pl031,
    Goldfish,
}

/// The `compatible` strings, in the order they are looked for.
const DEVICES: [(&[u8], Kind); 2] = [
    (b"arm,pl031", Kind::Pl031),
    (b"google,goldfish-rtc", Kind::Goldfish),
];

/// UTC seconds from a clock's registers: `low` is the first word read
/// (RTCDR, TIME_LOW), `high` the second (TIME_HIGH; unused by the PL031).
fn epoch_secs(kind: Kind, low: u32, high: u32) -> u64 {
    match kind {
        Kind::Pl031 => u64::from(low),
        Kind::Goldfish => ((u64::from(high) << 32) | u64::from(low)) / 1_000_000_000,
    }
}

/// Set the wall clock from the first RTC the device tree describes.
#[cfg(all(
    any(target_arch = "aarch64", target_arch = "riscv64"),
    target_os = "none"
))]
pub fn init() {
    #[cfg(target_arch = "aarch64")]
    let dtb_pa = super::aarch64::boot::dtb_pa();
    #[cfg(target_arch = "riscv64")]
    let dtb_pa = super::riscv64::dtb_pa();
    // SAFETY: firmware passed this device tree; `from_phys` validates the
    // header and size before building the view.
    let Some(fdt) = (unsafe { super::fdt::Fdt::from_phys(dtb_pa) }) else {
        return;
    };
    for (compat, kind) in DEVICES {
        let Some((base, size)) = fdt.compatible_reg(compat) else {
            continue;
        };
        if size < 8 {
            continue;
        }
        let va = crate::mm::phys_to_virt_addr(base) as usize;
        // SAFETY: the device tree places this clock's registers at `base`
        // (at least 8 bytes); the physical map covers it as device memory
        // (the first GiB on both architectures, boot.S), and the reads have
        // no side effect beyond the Goldfish latch, which is wanted.
        let (low, high) = unsafe {
            let low = core::ptr::read_volatile(va as *const u32);
            let high = match kind {
                Kind::Goldfish => core::ptr::read_volatile((va + 4) as *const u32),
                Kind::Pl031 => 0,
            };
            (low, high)
        };
        let secs = epoch_secs(kind, low, high);
        crate::timer::realtime::set_from_rtc(secs, crate::timer::monotonic_ns());
        crate::println!(
            "[RTC] {} at {:#x}: epoch {}",
            core::str::from_utf8(compat).unwrap_or("rtc"),
            base,
            secs
        );
        return;
    }
    crate::println!("[RTC] none in the device tree: the wall clock counts from boot");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtc_registers_give_utc_seconds() {
        // 2026-10-16 00:00:00 UTC.
        let t: u64 = 1_792_108_800;
        assert_eq!(epoch_secs(Kind::Pl031, t as u32, 0xdead_beef), t);
        let ns = t * 1_000_000_000 + 999_999_999;
        assert_eq!(
            epoch_secs(Kind::Goldfish, ns as u32, (ns >> 32) as u32),
            t,
            "nanoseconds truncate to the second"
        );
        assert_eq!(DEVICES[0].1, Kind::Pl031);
    }
}
