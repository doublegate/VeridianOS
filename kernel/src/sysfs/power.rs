//! Sysfs power virtual files for VeridianOS.
//!
//! Exposes power management controls as virtual files under `/sys/`:
//! - `/sys/power/state` -- read supported sleep states, write to trigger
//! - `/sys/class/backlight/veridian/brightness` -- read/write 0-100
//! - `/sys/class/backlight/veridian/max_brightness` -- read returns 100
//! - `/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor` -- read/write
//! - `/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq` -- read current
//! - `/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors` -- read
//! - `/sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq` -- read
//! - `/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq` -- read
//!
//! These paths are compatible with Linux sysfs conventions so that
//! PowerDevil and other desktop tools can interact with the kernel.

#![allow(dead_code)]

extern crate alloc;

use alloc::{format, string::String};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::{
    error::{KernelError, KernelResult},
    sysfs::SysfsNode,
};

// ---------------------------------------------------------------------------
// Virtual backlight state
// ---------------------------------------------------------------------------

/// Current backlight brightness (0-100).
static BACKLIGHT_BRIGHTNESS: AtomicU32 = AtomicU32::new(100);

/// Maximum backlight brightness.
const MAX_BRIGHTNESS: u32 = 100;

// ---------------------------------------------------------------------------
// /sys/power/state
// ---------------------------------------------------------------------------

/// Read handler for /sys/power/state.
///
/// Returns the list of supported sleep states in Linux sysfs format:
/// "standby mem disk" (space-separated).
fn power_state_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if crate::arch::x86_64::acpi_pm::is_initialized() {
            return String::from(crate::arch::x86_64::acpi_pm::supported_states_string());
        }
    }
    String::from("standby mem disk")
}

/// Write handler for /sys/power/state.
///
/// Accepts "mem" (S3 suspend), "disk" (S4 hibernate), or "standby" (S1).
fn power_state_write(value: &str) -> KernelResult<()> {
    let trimmed = value.trim();

    match trimmed {
        "mem" => {
            println!("[SYSFS] Triggering S3 suspend via /sys/power/state");
            #[cfg(all(target_arch = "x86_64", target_os = "none"))]
            {
                crate::arch::x86_64::acpi_pm::acpi_suspend_s3()
            }
            #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
            {
                Err(KernelError::OperationNotSupported {
                    operation: "S3 suspend (not x86_64)",
                })
            }
        }
        "disk" => {
            println!("[SYSFS] Triggering S4 hibernate via /sys/power/state");
            #[cfg(all(target_arch = "x86_64", target_os = "none"))]
            {
                crate::arch::x86_64::acpi_pm::acpi_hibernate_s4()
            }
            #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
            {
                Err(KernelError::OperationNotSupported {
                    operation: "S4 hibernate (not x86_64)",
                })
            }
        }
        "standby" => {
            println!("[SYSFS] Standby requested via /sys/power/state");
            // S1 standby is a lightweight CPU halt.
            Ok(())
        }
        _ => Err(KernelError::InvalidArgument {
            name: "power state",
            value: "expected 'mem', 'disk', or 'standby'",
        }),
    }
}

// ---------------------------------------------------------------------------
// /sys/class/backlight/veridian/brightness
// ---------------------------------------------------------------------------

/// Read handler for backlight brightness.
fn backlight_brightness_read() -> String {
    format!("{}", BACKLIGHT_BRIGHTNESS.load(Ordering::Acquire))
}

/// Write handler for backlight brightness.
///
/// Accepts a value 0-100. Values outside this range are clamped.
fn backlight_brightness_write(value: &str) -> KernelResult<()> {
    let trimmed = value.trim();
    let brightness: u32 = trimmed.parse().map_err(|_| KernelError::InvalidArgument {
        name: "brightness",
        value: "not a valid integer",
    })?;

    let clamped = brightness.min(MAX_BRIGHTNESS);
    BACKLIGHT_BRIGHTNESS.store(clamped, Ordering::Release);
    println!("[SYSFS] Backlight brightness set to {}", clamped);
    Ok(())
}

/// Read handler for max_brightness.
fn backlight_max_brightness_read() -> String {
    format!("{}", MAX_BRIGHTNESS)
}

// ---------------------------------------------------------------------------
// /sys/devices/system/cpu/cpu0/cpufreq/*
// ---------------------------------------------------------------------------

/// Read handler for scaling_governor.
fn cpufreq_governor_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if crate::arch::x86_64::cpufreq::is_initialized() {
            return String::from(crate::arch::x86_64::cpufreq::cpufreq_get_governor().name());
        }
    }
    String::from("performance")
}

/// Write handler for scaling_governor.
fn cpufreq_governor_write(value: &str) -> KernelResult<()> {
    let trimmed = value.trim();

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        use crate::arch::x86_64::cpufreq::CpuGovernor;
        let governor = CpuGovernor::from_name(trimmed).ok_or(KernelError::InvalidArgument {
            name: "governor",
            value: "expected 'performance', 'powersave', or 'ondemand'",
        })?;
        crate::arch::x86_64::cpufreq::cpufreq_set_governor(governor)
    }

    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        let _ = trimmed;
        Err(KernelError::OperationNotSupported {
            operation: "cpufreq governor (not x86_64)",
        })
    }
}

/// Read handler for scaling_cur_freq.
fn cpufreq_cur_freq_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if crate::arch::x86_64::cpufreq::is_initialized() {
            return format!("{}", crate::arch::x86_64::cpufreq::cpufreq_get_frequency());
        }
    }
    String::from("0")
}

/// Read handler for scaling_available_governors.
fn cpufreq_available_governors_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        String::from(crate::arch::x86_64::cpufreq::cpufreq_available_governors())
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        String::from("performance powersave ondemand")
    }
}

/// Read handler for scaling_min_freq.
fn cpufreq_min_freq_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if crate::arch::x86_64::cpufreq::is_initialized() {
            return format!(
                "{}",
                crate::arch::x86_64::cpufreq::cpufreq_get_min_frequency()
            );
        }
    }
    String::from("0")
}

/// Read handler for scaling_max_freq.
fn cpufreq_max_freq_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        if crate::arch::x86_64::cpufreq::is_initialized() {
            return format!(
                "{}",
                crate::arch::x86_64::cpufreq::cpufreq_get_max_frequency()
            );
        }
    }
    String::from("0")
}

// ---------------------------------------------------------------------------
// DPMS sysfs node
// ---------------------------------------------------------------------------

/// Read handler for DPMS idle timeout.
fn dpms_idle_timeout_read() -> String {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        format!("{}", crate::arch::x86_64::dpms::dpms_get_idle_timeout())
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        String::from("300")
    }
}

/// Write handler for DPMS idle timeout.
fn dpms_idle_timeout_write(value: &str) -> KernelResult<()> {
    let trimmed = value.trim();
    let seconds: u32 = trimmed.parse().map_err(|_| KernelError::InvalidArgument {
        name: "idle_timeout",
        value: "not a valid integer",
    })?;

    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        crate::arch::x86_64::dpms::dpms_set_idle_timeout(seconds);
    }

    let _ = seconds;
    println!("[SYSFS] DPMS idle timeout set to {}s", seconds);
    Ok(())
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Register all power-related sysfs nodes.
pub(crate) fn sysfs_power_init() -> KernelResult<()> {
    // /sys/power/state
    super::register_node(SysfsNode::read_write(
        "/sys/power/state",
        "System sleep states (read: list supported, write: trigger)",
        power_state_read,
        power_state_write,
    ))?;

    // /sys/class/backlight/veridian/brightness
    super::register_node(SysfsNode::read_write(
        "/sys/class/backlight/veridian/brightness",
        "Backlight brightness (0-100)",
        backlight_brightness_read,
        backlight_brightness_write,
    ))?;

    // /sys/class/backlight/veridian/max_brightness
    super::register_node(SysfsNode::read_only(
        "/sys/class/backlight/veridian/max_brightness",
        "Maximum backlight brightness",
        backlight_max_brightness_read,
    ))?;

    // /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
    super::register_node(SysfsNode::read_write(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
        "CPU frequency scaling governor",
        cpufreq_governor_read,
        cpufreq_governor_write,
    ))?;

    // /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq
    super::register_node(SysfsNode::read_only(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
        "Current CPU frequency in kHz",
        cpufreq_cur_freq_read,
    ))?;

    // /sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors
    super::register_node(SysfsNode::read_only(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
        "Available CPU frequency governors",
        cpufreq_available_governors_read,
    ))?;

    // /sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq
    super::register_node(SysfsNode::read_only(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq",
        "Minimum CPU frequency in kHz",
        cpufreq_min_freq_read,
    ))?;

    // /sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq
    super::register_node(SysfsNode::read_only(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq",
        "Maximum CPU frequency in kHz",
        cpufreq_max_freq_read,
    ))?;

    // DPMS idle timeout
    super::register_node(SysfsNode::read_write(
        "/sys/class/drm/card0/dpms_idle_timeout",
        "DPMS idle timeout in seconds",
        dpms_idle_timeout_read,
        dpms_idle_timeout_write,
    ))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_state_read() {
        let states = power_state_read();
        assert!(states.contains("mem") || states.contains("standby"));
    }

    #[test]
    fn test_power_state_write_invalid() {
        let result = power_state_write("invalid_state");
        assert!(result.is_err());
    }

    #[test]
    fn test_backlight_brightness_read() {
        BACKLIGHT_BRIGHTNESS.store(75, Ordering::Release);
        let val = backlight_brightness_read();
        assert_eq!(val, "75");
    }

    #[test]
    fn test_backlight_brightness_write_valid() {
        let result = backlight_brightness_write("50");
        assert!(result.is_ok());
        assert_eq!(BACKLIGHT_BRIGHTNESS.load(Ordering::Acquire), 50);
    }

    #[test]
    fn test_backlight_brightness_write_clamp() {
        let result = backlight_brightness_write("200");
        assert!(result.is_ok());
        assert_eq!(BACKLIGHT_BRIGHTNESS.load(Ordering::Acquire), 100);
    }

    #[test]
    fn test_backlight_brightness_write_invalid() {
        let result = backlight_brightness_write("abc");
        assert!(result.is_err());
    }

    #[test]
    fn test_backlight_max_brightness() {
        let val = backlight_max_brightness_read();
        assert_eq!(val, "100");
    }

    #[test]
    fn test_cpufreq_governor_read() {
        let val = cpufreq_governor_read();
        assert!(!val.is_empty());
    }

    #[test]
    fn test_cpufreq_available_governors() {
        let val = cpufreq_available_governors_read();
        assert!(val.contains("performance"));
    }

    #[test]
    fn test_power_state_write_standby() {
        let result = power_state_write("standby");
        assert!(result.is_ok());
    }

    #[test]
    fn test_dpms_idle_timeout_read() {
        let val = dpms_idle_timeout_read();
        assert!(!val.is_empty());
    }

    #[test]
    fn test_dpms_idle_timeout_write_valid() {
        let result = dpms_idle_timeout_write("120");
        assert!(result.is_ok());
    }

    #[test]
    fn test_dpms_idle_timeout_write_invalid() {
        let result = dpms_idle_timeout_write("not_a_number");
        assert!(result.is_err());
    }
}
