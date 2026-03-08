//! CPU Frequency Scaling via MSR for x86_64.
//!
//! Controls CPU P-states (performance states) through the IA32_PERF_CTL
//! and IA32_PERF_STATUS MSRs. Supports three governors:
//! - Performance: fixed at maximum frequency
//! - Powersave: fixed at minimum frequency
//! - Ondemand: dynamic scaling based on CPU utilization (integer math)
//!
//! P-state ratios are read from MSR_PLATFORM_INFO (0xCE) which provides
//! the minimum and maximum non-turbo frequency ratios. The bus clock
//! frequency (typically 100 MHz) is used to convert ratios to kHz.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// MSR addresses
// ---------------------------------------------------------------------------

/// IA32_PERF_STATUS: current P-state (read-only).
/// Bits 15:0 contain the current performance state value.
const MSR_IA32_PERF_STATUS: u32 = 0x198;

/// IA32_PERF_CTL: target P-state (read-write).
/// Bits 15:0 set the target performance state.
const MSR_IA32_PERF_CTL: u32 = 0x199;

/// MSR_PLATFORM_INFO: platform information including P-state ratios.
/// Bits 15:8  = maximum non-turbo ratio
/// Bits 47:40 = minimum ratio (maximum efficiency)
const MSR_PLATFORM_INFO: u32 = 0xCE;

/// IA32_MISC_ENABLE: miscellaneous feature control.
/// Bit 16 = Enhanced SpeedStep Technology Enable.
const MSR_IA32_MISC_ENABLE: u32 = 0x1A0;

/// Enhanced SpeedStep enable bit in IA32_MISC_ENABLE.
const EIST_ENABLE_BIT: u64 = 1 << 16;

/// CPUID leaf for checking SpeedStep/P-state support.
const CPUID_LEAF_POWER: u32 = 0x06;

/// CPUID EAX bit 1: Enhanced SpeedStep available.
const CPUID_EIST_BIT: u32 = 1 << 1;

// ---------------------------------------------------------------------------
// Governor definitions
// ---------------------------------------------------------------------------

/// CPU frequency scaling governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CpuGovernor {
    /// Fixed at maximum frequency.
    Performance = 0,
    /// Fixed at minimum frequency.
    Powersave = 1,
    /// Dynamic scaling based on CPU utilization.
    Ondemand = 2,
}

impl CpuGovernor {
    /// Convert from raw u8.
    fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Performance,
            1 => Self::Powersave,
            2 => Self::Ondemand,
            _ => Self::Performance,
        }
    }

    /// Governor name as a string.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Powersave => "powersave",
            Self::Ondemand => "ondemand",
        }
    }

    /// Parse governor name from string.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "performance" => Some(Self::Performance),
            "powersave" => Some(Self::Powersave),
            "ondemand" => Some(Self::Ondemand),
            _ => None,
        }
    }
}

impl core::fmt::Display for CpuGovernor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ---------------------------------------------------------------------------
// P-state information
// ---------------------------------------------------------------------------

/// CPU frequency scaling information.
#[derive(Debug, Clone, Copy)]
struct CpuFreqInfo {
    /// Minimum P-state ratio (maximum efficiency).
    min_ratio: u8,
    /// Maximum P-state ratio (non-turbo).
    max_ratio: u8,
    /// Bus clock frequency in kHz (typically 100000 = 100 MHz).
    bus_clock_khz: u64,
    /// Whether Enhanced SpeedStep is supported.
    eist_supported: bool,
    /// Whether P-state control is available.
    pstate_available: bool,
}

impl CpuFreqInfo {
    const fn new() -> Self {
        Self {
            min_ratio: 0,
            max_ratio: 0,
            bus_clock_khz: 100_000, // 100 MHz default bus clock
            eist_supported: false,
            pstate_available: false,
        }
    }

    /// Convert a P-state ratio to frequency in kHz.
    fn ratio_to_khz(&self, ratio: u8) -> u64 {
        (ratio as u64).saturating_mul(self.bus_clock_khz)
    }

    /// Minimum frequency in kHz.
    fn min_freq_khz(&self) -> u64 {
        self.ratio_to_khz(self.min_ratio)
    }

    /// Maximum frequency in kHz.
    fn max_freq_khz(&self) -> u64 {
        self.ratio_to_khz(self.max_ratio)
    }
}

// ---------------------------------------------------------------------------
// Ondemand governor state
// ---------------------------------------------------------------------------

/// Ondemand governor configuration and state.
#[derive(Debug)]
struct OndemandState {
    /// CPU load percentage threshold to scale up (default 80%).
    up_threshold: u8,
    /// CPU load percentage threshold to scale down (default 20%).
    down_threshold: u8,
    /// Sampling interval in milliseconds (default 100ms).
    sampling_interval_ms: u32,
    /// Total ticks in the last sampling period.
    last_total_ticks: u64,
    /// Busy ticks in the last sampling period.
    last_busy_ticks: u64,
}

impl OndemandState {
    const fn new() -> Self {
        Self {
            up_threshold: 80,
            down_threshold: 20,
            sampling_interval_ms: 100,
            last_total_ticks: 0,
            last_busy_ticks: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static CPUFREQ_INITIALIZED: AtomicBool = AtomicBool::new(false);
static CURRENT_GOVERNOR: AtomicU8 = AtomicU8::new(0); // Performance
static CURRENT_FREQ_KHZ: AtomicU64 = AtomicU64::new(0);
static CPUFREQ_INFO: Mutex<CpuFreqInfo> = Mutex::new(CpuFreqInfo::new());
static ONDEMAND_STATE: Mutex<OndemandState> = Mutex::new(OndemandState::new());

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the CPU frequency scaling subsystem.
///
/// Detects P-state support via CPUID, reads min/max ratios from
/// MSR_PLATFORM_INFO, and enables Enhanced SpeedStep if available.
pub fn cpufreq_init() -> KernelResult<()> {
    if CPUFREQ_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "cpufreq",
            id: 0,
        });
    }

    println!("[CPUFREQ] Initializing CPU frequency scaling...");

    let mut info = CpuFreqInfo::new();

    // Check CPUID for Enhanced SpeedStep support.
    info.eist_supported = check_eist_support();
    if !info.eist_supported {
        println!("[CPUFREQ] Enhanced SpeedStep not supported by CPU");
        // Still initialize with defaults for frequency reporting.
        info.pstate_available = false;
    } else {
        println!("[CPUFREQ] Enhanced SpeedStep supported");
        info.pstate_available = true;

        // Enable EIST if not already enabled.
        enable_eist();
    }

    // Read P-state ratios from MSR_PLATFORM_INFO.
    if info.pstate_available {
        let platform_info = super::msr::rdmsr(MSR_PLATFORM_INFO);

        // Bits 15:8 = max non-turbo ratio.
        info.max_ratio = ((platform_info >> 8) & 0xFF) as u8;
        // Bits 47:40 = min ratio.
        info.min_ratio = ((platform_info >> 40) & 0xFF) as u8;

        // Sanity check: if min >= max, use defaults.
        if info.min_ratio == 0 || info.max_ratio == 0 || info.min_ratio >= info.max_ratio {
            // Fallback: assume a reasonable range.
            info.min_ratio = 8; // 800 MHz
            info.max_ratio = 30; // 3000 MHz
            println!("[CPUFREQ] Using fallback P-state ratios");
        }

        println!(
            "[CPUFREQ] P-state range: ratio {}..{} ({} MHz .. {} MHz)",
            info.min_ratio,
            info.max_ratio,
            info.min_freq_khz() / 1000,
            info.max_freq_khz() / 1000,
        );
    }

    // Read current frequency.
    let current = read_current_frequency(&info);
    CURRENT_FREQ_KHZ.store(current, Ordering::Release);

    *CPUFREQ_INFO.lock() = info;
    CPUFREQ_INITIALIZED.store(true, Ordering::Release);

    println!(
        "[CPUFREQ] Initialized: current={} MHz, governor=performance",
        current / 1000
    );

    Ok(())
}

/// Check if Enhanced SpeedStep Technology is supported via CPUID.
fn check_eist_support() -> bool {
    // Use CPUID leaf 0x06 to check thermal and power management features.
    // SAFETY: CPUID is a non-privileged instruction that reads CPU
    // identification data. Leaf 0x06 returns power management features.
    unsafe {
        core::arch::asm!(
            "push rbx",   // CPUID clobbers EBX
            "cpuid",
            "pop rbx",
            inout("eax") CPUID_LEAF_POWER => _,
            out("ecx") _,
            out("edx") _,
            options(nomem, preserves_flags),
        );
    }

    // Also check CPUID leaf 1, ECX bit 7 for EIST.
    let ecx_leaf1: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") 1u32 => _,
            out("ecx") ecx_leaf1,
            out("edx") _,
            options(nomem, preserves_flags),
        );
    }

    // EIST is indicated by CPUID.01H:ECX bit 7.
    (ecx_leaf1 & (1 << 7)) != 0
}

/// Enable Enhanced SpeedStep via IA32_MISC_ENABLE MSR.
fn enable_eist() {
    let misc_enable = super::msr::rdmsr(MSR_IA32_MISC_ENABLE);
    if misc_enable & EIST_ENABLE_BIT == 0 {
        super::msr::wrmsr(MSR_IA32_MISC_ENABLE, misc_enable | EIST_ENABLE_BIT);
        println!("[CPUFREQ] EIST enabled via IA32_MISC_ENABLE");
    }
}

/// Read current CPU frequency from IA32_PERF_STATUS.
fn read_current_frequency(info: &CpuFreqInfo) -> u64 {
    if !info.pstate_available {
        return info.max_freq_khz();
    }

    let perf_status = super::msr::rdmsr(MSR_IA32_PERF_STATUS);
    let current_ratio = ((perf_status >> 8) & 0xFF) as u8;

    if current_ratio == 0 {
        return info.max_freq_khz();
    }

    info.ratio_to_khz(current_ratio)
}

// ---------------------------------------------------------------------------
// Governor control
// ---------------------------------------------------------------------------

/// Set the active CPU frequency governor.
pub fn cpufreq_set_governor(governor: CpuGovernor) -> KernelResult<()> {
    if !CPUFREQ_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "cpufreq",
        });
    }

    let old = CpuGovernor::from_u8(CURRENT_GOVERNOR.load(Ordering::Acquire));
    CURRENT_GOVERNOR.store(governor as u8, Ordering::Release);

    // Apply the governor's frequency policy.
    let info = CPUFREQ_INFO.lock();
    match governor {
        CpuGovernor::Performance => {
            set_pstate_ratio(&info, info.max_ratio);
        }
        CpuGovernor::Powersave => {
            set_pstate_ratio(&info, info.min_ratio);
        }
        CpuGovernor::Ondemand => {
            // Reset ondemand state counters.
            let mut od = ONDEMAND_STATE.lock();
            od.last_total_ticks = 0;
            od.last_busy_ticks = 0;
            // Start at maximum frequency; ondemand will scale down if idle.
            set_pstate_ratio(&info, info.max_ratio);
        }
    }

    println!("[CPUFREQ] Governor changed: {} -> {}", old, governor);
    Ok(())
}

/// Get the current governor.
pub fn cpufreq_get_governor() -> CpuGovernor {
    CpuGovernor::from_u8(CURRENT_GOVERNOR.load(Ordering::Acquire))
}

/// Get the current CPU frequency in kHz.
pub fn cpufreq_get_frequency() -> u64 {
    if !CPUFREQ_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }

    let info = CPUFREQ_INFO.lock();
    let freq = read_current_frequency(&info);
    CURRENT_FREQ_KHZ.store(freq, Ordering::Release);
    freq
}

/// Set the target CPU frequency in kHz.
///
/// Only effective in Performance or Powersave governors. Ondemand
/// governor manages frequency automatically.
pub fn cpufreq_set_frequency(khz: u64) -> KernelResult<()> {
    if !CPUFREQ_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "cpufreq",
        });
    }

    let info = CPUFREQ_INFO.lock();
    if !info.pstate_available {
        return Err(KernelError::OperationNotSupported {
            operation: "cpufreq_set_frequency (no P-state support)",
        });
    }

    // Convert kHz to ratio (integer division).
    if info.bus_clock_khz == 0 {
        return Err(KernelError::InvalidArgument {
            name: "bus_clock_khz",
            value: "zero",
        });
    }
    let ratio = (khz / info.bus_clock_khz) as u8;

    // Clamp to valid range.
    let clamped = ratio.clamp(info.min_ratio, info.max_ratio);
    set_pstate_ratio(&info, clamped);

    let actual_khz = info.ratio_to_khz(clamped);
    CURRENT_FREQ_KHZ.store(actual_khz, Ordering::Release);

    Ok(())
}

/// Get available CPU frequencies in kHz.
///
/// Returns frequencies for each P-state ratio from min to max.
#[cfg(feature = "alloc")]
pub fn cpufreq_get_available_frequencies() -> alloc::vec::Vec<u64> {
    let info = CPUFREQ_INFO.lock();
    let mut freqs = alloc::vec::Vec::new();

    if info.min_ratio == 0 || info.max_ratio == 0 {
        return freqs;
    }

    let mut ratio = info.min_ratio;
    while ratio <= info.max_ratio {
        freqs.push(info.ratio_to_khz(ratio));
        ratio = ratio.saturating_add(1);
    }

    freqs
}

/// Get the minimum frequency in kHz.
pub fn cpufreq_get_min_frequency() -> u64 {
    let info = CPUFREQ_INFO.lock();
    info.min_freq_khz()
}

/// Get the maximum frequency in kHz.
pub fn cpufreq_get_max_frequency() -> u64 {
    let info = CPUFREQ_INFO.lock();
    info.max_freq_khz()
}

/// Get the list of available governor names.
pub fn cpufreq_available_governors() -> &'static str {
    "performance powersave ondemand"
}

// ---------------------------------------------------------------------------
// P-state MSR access
// ---------------------------------------------------------------------------

/// Set the target P-state ratio via IA32_PERF_CTL MSR.
fn set_pstate_ratio(info: &CpuFreqInfo, ratio: u8) {
    if !info.pstate_available || ratio == 0 {
        return;
    }

    // IA32_PERF_CTL bits 15:8 = target ratio.
    let value = (ratio as u64) << 8;
    super::msr::wrmsr(MSR_IA32_PERF_CTL, value);
}

// ---------------------------------------------------------------------------
// Ondemand governor tick
// ---------------------------------------------------------------------------

/// Sample CPU load and adjust frequency (called periodically).
///
/// Uses integer math: `load_pct = (busy_ticks * 100) / total_ticks`.
/// Scales up when load > 80%, scales down when load < 20%.
///
/// `busy_ticks` and `total_ticks` are the cumulative scheduler tick
/// counts since the last sample.
pub fn cpufreq_ondemand_sample(busy_ticks: u64, total_ticks: u64) {
    if !CPUFREQ_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    if CpuGovernor::from_u8(CURRENT_GOVERNOR.load(Ordering::Acquire)) != CpuGovernor::Ondemand {
        return;
    }

    let info = CPUFREQ_INFO.lock();
    if !info.pstate_available {
        return;
    }

    let mut od = ONDEMAND_STATE.lock();

    // Calculate delta since last sample.
    let delta_total = total_ticks.saturating_sub(od.last_total_ticks);
    let delta_busy = busy_ticks.saturating_sub(od.last_busy_ticks);

    od.last_total_ticks = total_ticks;
    od.last_busy_ticks = busy_ticks;

    if delta_total == 0 {
        return;
    }

    // Integer percentage: load_pct = (busy * 100) / total.
    let load_pct = (delta_busy.saturating_mul(100)) / delta_total;

    // Read current ratio from IA32_PERF_STATUS.
    let perf_status = super::msr::rdmsr(MSR_IA32_PERF_STATUS);
    let current_ratio = ((perf_status >> 8) & 0xFF) as u8;

    let new_ratio = if load_pct >= od.up_threshold as u64 {
        // High load: jump to max frequency.
        info.max_ratio
    } else if load_pct < od.down_threshold as u64 {
        // Low load: step down one ratio.
        current_ratio.saturating_sub(1).max(info.min_ratio)
    } else {
        // Medium load: keep current.
        current_ratio
    };

    if new_ratio != current_ratio && new_ratio >= info.min_ratio && new_ratio <= info.max_ratio {
        set_pstate_ratio(&info, new_ratio);
        let new_freq = info.ratio_to_khz(new_ratio);
        CURRENT_FREQ_KHZ.store(new_freq, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Query API
// ---------------------------------------------------------------------------

/// Check if cpufreq is initialized.
pub fn is_initialized() -> bool {
    CPUFREQ_INITIALIZED.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_governor_from_u8() {
        assert_eq!(CpuGovernor::from_u8(0), CpuGovernor::Performance);
        assert_eq!(CpuGovernor::from_u8(1), CpuGovernor::Powersave);
        assert_eq!(CpuGovernor::from_u8(2), CpuGovernor::Ondemand);
        assert_eq!(CpuGovernor::from_u8(99), CpuGovernor::Performance);
    }

    #[test]
    fn test_governor_name() {
        assert_eq!(CpuGovernor::Performance.name(), "performance");
        assert_eq!(CpuGovernor::Powersave.name(), "powersave");
        assert_eq!(CpuGovernor::Ondemand.name(), "ondemand");
    }

    #[test]
    fn test_governor_from_name() {
        assert_eq!(
            CpuGovernor::from_name("performance"),
            Some(CpuGovernor::Performance)
        );
        assert_eq!(
            CpuGovernor::from_name("powersave"),
            Some(CpuGovernor::Powersave)
        );
        assert_eq!(
            CpuGovernor::from_name("ondemand"),
            Some(CpuGovernor::Ondemand)
        );
        assert_eq!(CpuGovernor::from_name("turbo"), None);
    }

    #[test]
    fn test_governor_display() {
        assert_eq!(
            alloc::format!("{}", CpuGovernor::Performance),
            "performance"
        );
        assert_eq!(alloc::format!("{}", CpuGovernor::Powersave), "powersave");
        assert_eq!(alloc::format!("{}", CpuGovernor::Ondemand), "ondemand");
    }

    #[test]
    fn test_cpufreq_info_defaults() {
        let info = CpuFreqInfo::new();
        assert_eq!(info.bus_clock_khz, 100_000);
        assert_eq!(info.min_ratio, 0);
        assert_eq!(info.max_ratio, 0);
        assert!(!info.eist_supported);
    }

    #[test]
    fn test_ratio_to_khz() {
        let mut info = CpuFreqInfo::new();
        info.min_ratio = 8;
        info.max_ratio = 30;
        // 8 * 100_000 = 800_000 kHz = 800 MHz
        assert_eq!(info.ratio_to_khz(8), 800_000);
        // 30 * 100_000 = 3_000_000 kHz = 3000 MHz
        assert_eq!(info.ratio_to_khz(30), 3_000_000);
    }

    #[test]
    fn test_min_max_freq() {
        let mut info = CpuFreqInfo::new();
        info.min_ratio = 10;
        info.max_ratio = 40;
        assert_eq!(info.min_freq_khz(), 1_000_000);
        assert_eq!(info.max_freq_khz(), 4_000_000);
    }

    #[test]
    fn test_ondemand_state_defaults() {
        let od = OndemandState::new();
        assert_eq!(od.up_threshold, 80);
        assert_eq!(od.down_threshold, 20);
        assert_eq!(od.sampling_interval_ms, 100);
    }

    #[test]
    fn test_ondemand_load_calculation() {
        // Simulate: 80 busy ticks out of 100 total = 80% load.
        let busy = 80u64;
        let total = 100u64;
        let load_pct = (busy * 100) / total;
        assert_eq!(load_pct, 80);

        // 10 busy out of 100 = 10% load.
        let load_low = (10u64 * 100) / 100;
        assert_eq!(load_low, 10);
    }

    #[test]
    fn test_available_governors_string() {
        let govs = cpufreq_available_governors();
        assert!(govs.contains("performance"));
        assert!(govs.contains("powersave"));
        assert!(govs.contains("ondemand"));
    }

    #[test]
    fn test_eist_enable_bit() {
        assert_eq!(EIST_ENABLE_BIT, 1 << 16);
    }
}
