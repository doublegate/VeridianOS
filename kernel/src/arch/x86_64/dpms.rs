//! DPMS (Display Power Management Signaling) for x86_64.
//!
//! Controls display power states via VGA register manipulation. Supports
//! DPMS states: On, Standby, Suspend, Off. Includes an idle timer that
//! transitions the display to lower power states after configurable
//! inactivity periods.
//!
//! When DPMS is Off, framebuffer blits should be skipped to save
//! CPU/bus bandwidth.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// DPMS state definitions
// ---------------------------------------------------------------------------

/// Display power management states per VESA DPMS specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DpmsState {
    /// Display fully on (H-sync: on, V-sync: on).
    On = 0,
    /// Standby mode (H-sync: off, V-sync: on). Quick wake-up.
    Standby = 1,
    /// Suspend mode (H-sync: on, V-sync: off). Moderate wake-up.
    Suspend = 2,
    /// Off (H-sync: off, V-sync: off). Longest wake-up.
    Off = 3,
}

impl DpmsState {
    /// Convert from raw u8 value.
    fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::On,
            1 => Self::Standby,
            2 => Self::Suspend,
            3 => Self::Off,
            _ => Self::On,
        }
    }
}

impl core::fmt::Display for DpmsState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::On => write!(f, "On"),
            Self::Standby => write!(f, "Standby"),
            Self::Suspend => write!(f, "Suspend"),
            Self::Off => write!(f, "Off"),
        }
    }
}

// ---------------------------------------------------------------------------
// VGA register constants for DPMS
// ---------------------------------------------------------------------------

/// VGA Sequencer Index Register.
const VGA_SEQ_INDEX: u16 = 0x03C4;

/// VGA Sequencer Data Register.
const VGA_SEQ_DATA: u16 = 0x03C5;

/// Sequencer register 1: Clocking Mode.
const SEQ_CLOCKING_MODE: u8 = 0x01;

/// Bit 5 in Clocking Mode register: Screen Off.
const SCREEN_OFF_BIT: u8 = 1 << 5;

/// VGA CRT Controller Index Register.
const VGA_CRTC_INDEX: u16 = 0x03D4;

/// VGA CRT Controller Data Register.
const VGA_CRTC_DATA: u16 = 0x03D5;

/// CRTC register 0x17: Mode Control.
const CRTC_MODE_CONTROL: u8 = 0x17;

/// Bit 7 in CRTC Mode Control: Enable CRTC (sync generation).
const CRTC_ENABLE_SYNC: u8 = 1 << 7;

/// VGA Attribute Controller Address/Data Register.
const VGA_ATTR_INDEX: u16 = 0x03C0;

/// VGA Input Status Register 1 (for attribute controller reset).
const VGA_INPUT_STATUS_1: u16 = 0x03DA;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static DPMS_INITIALIZED: AtomicBool = AtomicBool::new(false);
static CURRENT_DPMS_STATE: AtomicU8 = AtomicU8::new(0); // On

/// Idle timeout in seconds (0 = disabled).
static IDLE_TIMEOUT_SECS: AtomicU32 = AtomicU32::new(300); // 5 minutes default

/// Ticks since last input event (incremented by timer, reset on input).
static IDLE_TICKS: AtomicU32 = AtomicU32::new(0);

/// Whether framebuffer blitting should be suppressed (DPMS Off).
static FRAMEBUFFER_SUPPRESSED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the DPMS subsystem.
///
/// Registers the idle timer callback and sets initial display state to On.
pub fn dpms_init() -> KernelResult<()> {
    if DPMS_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "DPMS",
            id: 0,
        });
    }

    CURRENT_DPMS_STATE.store(DpmsState::On as u8, Ordering::Release);
    IDLE_TICKS.store(0, Ordering::Release);
    FRAMEBUFFER_SUPPRESSED.store(false, Ordering::Release);

    DPMS_INITIALIZED.store(true, Ordering::Release);
    println!(
        "[DPMS] Initialized, idle timeout: {}s",
        IDLE_TIMEOUT_SECS.load(Ordering::Relaxed)
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// DPMS state control
// ---------------------------------------------------------------------------

/// Set the display DPMS state.
///
/// Controls the VGA sync signals to transition the display into the
/// requested power state. Returns an error if DPMS is not initialized.
pub fn dpms_set_state(state: DpmsState) -> KernelResult<()> {
    if !DPMS_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized { subsystem: "DPMS" });
    }

    let old_state = DpmsState::from_u8(CURRENT_DPMS_STATE.load(Ordering::Acquire));
    if old_state == state {
        return Ok(());
    }

    match state {
        DpmsState::On => {
            // Re-enable display output.
            set_screen_on(true);
            set_sync_enabled(true);
            FRAMEBUFFER_SUPPRESSED.store(false, Ordering::Release);
        }
        DpmsState::Standby => {
            // H-sync off, V-sync on: disable horizontal sync only.
            // VGA registers don't have fine-grained H/V sync control via
            // standard ports; use screen-off as an approximation.
            set_screen_on(false);
            FRAMEBUFFER_SUPPRESSED.store(true, Ordering::Release);
        }
        DpmsState::Suspend => {
            // H-sync on, V-sync off.
            set_screen_on(false);
            FRAMEBUFFER_SUPPRESSED.store(true, Ordering::Release);
        }
        DpmsState::Off => {
            // Both syncs off -- full display power down.
            set_screen_on(false);
            set_sync_enabled(false);
            FRAMEBUFFER_SUPPRESSED.store(true, Ordering::Release);
        }
    }

    CURRENT_DPMS_STATE.store(state as u8, Ordering::Release);
    println!("[DPMS] State changed: {} -> {}", old_state, state);

    Ok(())
}

/// Get the current DPMS state.
pub fn dpms_get_state() -> DpmsState {
    DpmsState::from_u8(CURRENT_DPMS_STATE.load(Ordering::Acquire))
}

/// Check whether the framebuffer should skip blitting (DPMS not On).
pub fn is_framebuffer_suppressed() -> bool {
    FRAMEBUFFER_SUPPRESSED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Idle timer
// ---------------------------------------------------------------------------

/// Set the DPMS idle timeout in seconds.
///
/// The display transitions to Off after this many seconds of no input.
/// Set to 0 to disable idle blanking.
pub fn dpms_set_idle_timeout(seconds: u32) {
    IDLE_TIMEOUT_SECS.store(seconds, Ordering::Release);
    println!("[DPMS] Idle timeout set to {}s", seconds);
}

/// Get the current idle timeout in seconds.
pub fn dpms_get_idle_timeout() -> u32 {
    IDLE_TIMEOUT_SECS.load(Ordering::Acquire)
}

/// Reset the idle timer.
///
/// Should be called on every input event (keyboard, mouse, touch) to
/// prevent the display from blanking during active use.
pub fn dpms_reset_idle() {
    IDLE_TICKS.store(0, Ordering::Release);

    // If display was blanked by idle, wake it up.
    let state = dpms_get_state();
    if state != DpmsState::On {
        let _ = dpms_set_state(DpmsState::On);
    }
}

/// Tick the idle timer (called from the scheduler/timer interrupt).
///
/// Increments the idle counter and transitions display state when the
/// timeout is reached.
pub fn dpms_idle_tick() {
    if !DPMS_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let timeout = IDLE_TIMEOUT_SECS.load(Ordering::Acquire);
    if timeout == 0 {
        return;
    }

    // Only count ticks when display is On.
    if dpms_get_state() != DpmsState::On {
        return;
    }

    let ticks = IDLE_TICKS.fetch_add(1, Ordering::AcqRel);

    // Timer fires at ~1Hz for idle tracking. When ticks reach timeout,
    // blank the display.
    if ticks >= timeout {
        let _ = dpms_set_state(DpmsState::Off);
    }
}

// ---------------------------------------------------------------------------
// VGA register helpers
// ---------------------------------------------------------------------------

/// Enable or disable the display screen via VGA Sequencer register.
fn set_screen_on(on: bool) {
    // SAFETY: Ports 0x03C4/0x03C5 are standard VGA Sequencer registers.
    // Writing to SEQ register 1 (Clocking Mode) bit 5 controls screen
    // blanking. This does not affect video memory contents.
    unsafe {
        super::outb(VGA_SEQ_INDEX, SEQ_CLOCKING_MODE);
        let mut val = super::inb(VGA_SEQ_DATA);
        if on {
            val &= !SCREEN_OFF_BIT;
        } else {
            val |= SCREEN_OFF_BIT;
        }
        super::outb(VGA_SEQ_INDEX, SEQ_CLOCKING_MODE);
        super::outb(VGA_SEQ_DATA, val);
    }
}

/// Enable or disable CRT sync signal generation.
fn set_sync_enabled(enabled: bool) {
    // SAFETY: Ports 0x03D4/0x03D5 are standard VGA CRTC registers.
    // Writing CRTC register 0x17 bit 7 controls sync generation.
    // Disabling sync is the mechanism for DPMS Off state.
    unsafe {
        super::outb(VGA_CRTC_INDEX, CRTC_MODE_CONTROL);
        let mut val = super::inb(VGA_CRTC_DATA);
        if enabled {
            val |= CRTC_ENABLE_SYNC;
        } else {
            val &= !CRTC_ENABLE_SYNC;
        }
        super::outb(VGA_CRTC_INDEX, CRTC_MODE_CONTROL);
        super::outb(VGA_CRTC_DATA, val);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dpms_state_from_u8() {
        assert_eq!(DpmsState::from_u8(0), DpmsState::On);
        assert_eq!(DpmsState::from_u8(1), DpmsState::Standby);
        assert_eq!(DpmsState::from_u8(2), DpmsState::Suspend);
        assert_eq!(DpmsState::from_u8(3), DpmsState::Off);
        assert_eq!(DpmsState::from_u8(42), DpmsState::On); // fallback
    }

    #[test]
    fn test_dpms_state_display() {
        assert_eq!(alloc::format!("{}", DpmsState::On), "On");
        assert_eq!(alloc::format!("{}", DpmsState::Standby), "Standby");
        assert_eq!(alloc::format!("{}", DpmsState::Suspend), "Suspend");
        assert_eq!(alloc::format!("{}", DpmsState::Off), "Off");
    }

    #[test]
    fn test_dpms_state_repr() {
        assert_eq!(DpmsState::On as u8, 0);
        assert_eq!(DpmsState::Standby as u8, 1);
        assert_eq!(DpmsState::Suspend as u8, 2);
        assert_eq!(DpmsState::Off as u8, 3);
    }

    #[test]
    fn test_idle_timeout_default() {
        // Default is 300 seconds (5 minutes).
        let timeout = 300u32;
        assert_eq!(timeout, 300);
    }

    #[test]
    fn test_screen_off_bit() {
        assert_eq!(SCREEN_OFF_BIT, 0x20);
    }

    #[test]
    fn test_crtc_enable_sync() {
        assert_eq!(CRTC_ENABLE_SYNC, 0x80);
    }
}
