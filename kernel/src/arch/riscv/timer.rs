//! RISC-V timer: clock source and timer interrupt.
//!
//! Clock source: the `time` CSR (`rdtime`, Zicntr), whose rate is the
//! device tree's `/cpus/timebase-frequency` -- read from the DTB that SBI
//! firmware passes in `a1` at entry. 10 MHz (QEMU virt) is used only if no
//! DTB is available.
//!
//! Timer interrupt (supervisor timer interrupt, scause = interrupt | 5):
//! with the Sstc extension the kernel writes `stimecmp` (CSR 0x14D)
//! directly; without it, `sbi_set_timer` asks M-mode firmware to do it,
//! which costs a trap to M-mode per tick. Linux prefers Sstc the same way.
//! Sstc support is read from the first CPU node's `riscv,isa-extensions`
//! or `riscv,isa` property.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::sbi;

/// Timer interrupts taken.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Timebase (time CSR) frequency in Hz.
static TIMEBASE_HZ: AtomicU64 = AtomicU64::new(10_000_000);
/// Whether `stimecmp` is usable (Sstc).
static SSTC: AtomicBool = AtomicBool::new(false);
/// Tick period in timebase units (0 = not started).
static PERIOD: AtomicU64 = AtomicU64::new(0);
/// Tick period in milliseconds.
static PERIOD_MS: AtomicU64 = AtomicU64::new(0);
/// Deadline currently armed.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// stimecmp CSR number (Sstc).
const CSR_STIMECMP: usize = 0x14D;
/// sie.STIE.
const SIE_STIE: usize = 1 << 5;

/// Get current timer ticks (interrupts taken).
pub fn get_ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Timebase frequency in Hz.
pub fn timebase_hz() -> u64 {
    TIMEBASE_HZ.load(Ordering::Relaxed)
}

/// Whether timer deadlines are written to `stimecmp` (Sstc).
pub fn uses_sstc() -> bool {
    SSTC.load(Ordering::Relaxed)
}

/// Read current time value
pub fn read_time() -> u64 {
    let time: u64;
    // SAFETY: The rdtime instruction reads the RISC-V real-time counter CSR,
    // which is always accessible in supervisor mode. No side effects.
    unsafe {
        core::arch::asm!("rdtime {}", out(reg) time);
    }
    time
}

/// Read the timebase frequency and Sstc support from the device tree at
/// physical address `dtb_pa` (0 if firmware passed none).
pub fn init_from_device_tree(dtb_pa: u64) {
    // SAFETY: dtb_pa is what firmware passed in a1 (saved by boot.S).
    let Some(fdt) = (unsafe { crate::arch::fdt::Fdt::from_phys(dtb_pa) }) else {
        crate::println!("[TIMER] No usable device tree: assuming a 10 MHz timebase, no Sstc");
        return;
    };
    if let Some(hz) = fdt.property_u64(&["cpus"], "timebase-frequency") {
        if hz != 0 {
            TIMEBASE_HZ.store(hz, Ordering::Relaxed);
        }
    }
    let sstc = fdt
        .property(&["cpus", "cpu"], "riscv,isa-extensions")
        .is_some_and(|isa| crate::arch::fdt::isa_has_extension(isa, b"sstc"))
        || fdt
            .property(&["cpus", "cpu"], "riscv,isa")
            .is_some_and(|isa| crate::arch::fdt::isa_has_extension(isa, b"sstc"));
    SSTC.store(sstc, Ordering::Relaxed);
    crate::println!(
        "[TIMER] Timebase {} Hz from device tree, Sstc {}",
        timebase_hz(),
        if sstc {
            "present (stimecmp)"
        } else {
            "absent (SBI set_timer)"
        }
    );
}

/// Program the next timer interrupt at absolute time `deadline`.
fn arm(deadline: u64) {
    if uses_sstc() {
        // SAFETY: Sstc is present (device tree), so S-mode may write
        // stimecmp; writing it also clears a pending timer interrupt.
        unsafe {
            core::arch::asm!("csrw {csr}, {v}", csr = const CSR_STIMECMP, v = in(reg) deadline,
                options(nomem, nostack));
        }
    } else {
        let _ = sbi::set_timer(deadline);
    }
}

/// Start a periodic tick at `hz` and enable the supervisor timer interrupt.
/// The trap vector must already be installed.
pub fn start(hz: u32) {
    let period = (timebase_hz() / hz.max(1) as u64).max(1);
    PERIOD.store(period, Ordering::Relaxed);
    PERIOD_MS.store((1000 / hz.max(1) as u64).max(1), Ordering::Relaxed);
    let first = read_time() + period;
    NEXT.store(first, Ordering::Relaxed);
    arm(first);
    // SAFETY: sets sie.STIE only; delivery still needs sstatus.SIE, which
    // the caller sets once the trap vector is installed.
    unsafe {
        core::arch::asm!("csrs sie, {0}", in(reg) SIE_STIE, options(nomem, nostack));
    }
    crate::println!(
        "[TIMER] Tick {} Hz ({} timebase ticks) via {}",
        hz,
        period,
        if uses_sstc() { "stimecmp" } else { "SBI" }
    );
}

/// Handle a supervisor timer interrupt (called from the trap handler).
///
/// Re-arms by whole periods so the tick does not drift, skipping ticks that
/// were missed rather than firing a burst. Only the clock and the timer
/// wheel are advanced: preempting the interrupted kernel code from here is
/// not supported yet.
pub fn handle_interrupt() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    let period = PERIOD.load(Ordering::Relaxed);
    if period == 0 {
        // Spurious: no tick configured. Push the deadline out so the
        // interrupt stops pending.
        arm(u64::MAX);
        return;
    }
    let now = read_time();
    let mut next = NEXT.load(Ordering::Relaxed).wrapping_add(period);
    if next <= now {
        next = now + period;
    }
    NEXT.store(next, Ordering::Relaxed);
    arm(next);
    crate::timer::timer_tick(PERIOD_MS.load(Ordering::Relaxed));
}
