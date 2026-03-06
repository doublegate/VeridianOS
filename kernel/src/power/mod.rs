//! Power management subsystem for VeridianOS
//!
//! Provides CPU power state management:
//! - C-states (idle states): C0 (Active), C1 (Halt), C2 (StopClock), C3 (Sleep)
//! - P-states (performance states): frequency/voltage scaling
//! - OnDemand governor: automatic frequency scaling based on CPU utilization
//!
//! Architecture support:
//! - x86_64: HLT (C1), MWAIT (C2/C3), IA32_PERF_CTL MSR for P-states
//! - AArch64: WFI for idle states
//! - RISC-V: WFI for idle states

#![allow(dead_code)]

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use spin::RwLock;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// C-state definitions
// ---------------------------------------------------------------------------

/// CPU idle power states (ACPI C-states)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum CState {
    /// C0: CPU is actively executing instructions
    C0 = 0,
    /// C1: Halt — core clock stopped, fastest wake-up
    C1 = 1,
    /// C2: Stop-Clock — deeper sleep, longer exit latency
    C2 = 2,
    /// C3: Sleep — caches may be flushed, longest exit latency
    C3 = 3,
}

impl CState {
    /// Number of defined C-states
    pub(crate) const COUNT: usize = 4;

    /// Convert from a raw u8 value
    pub(crate) fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(CState::C0),
            1 => Some(CState::C1),
            2 => Some(CState::C2),
            3 => Some(CState::C3),
            _ => None,
        }
    }
}

/// Information about a specific C-state
#[derive(Debug, Clone, Copy)]
pub(crate) struct CStateInfo {
    /// The C-state this info describes
    pub(crate) state: CState,
    /// Exit latency in nanoseconds (time to return to C0)
    pub(crate) exit_latency_ns: u64,
    /// Minimum residency in nanoseconds (must stay this long for net benefit)
    pub(crate) target_residency_ns: u64,
    /// Whether this state is supported on the current hardware
    pub(crate) supported: bool,
}

/// Default C-state table with typical latencies
const DEFAULT_CSTATE_TABLE: [CStateInfo; CState::COUNT] = [
    CStateInfo {
        state: CState::C0,
        exit_latency_ns: 0,
        target_residency_ns: 0,
        supported: true,
    },
    CStateInfo {
        state: CState::C1,
        exit_latency_ns: 1_000,      // 1 us
        target_residency_ns: 10_000, // 10 us
        supported: true,
    },
    CStateInfo {
        state: CState::C2,
        exit_latency_ns: 100_000,     // 100 us
        target_residency_ns: 500_000, // 500 us
        supported: true,
    },
    CStateInfo {
        state: CState::C3,
        exit_latency_ns: 1_000_000,     // 1 ms
        target_residency_ns: 5_000_000, // 5 ms
        supported: true,
    },
];

// ---------------------------------------------------------------------------
// P-state definitions
// ---------------------------------------------------------------------------

/// CPU performance state (frequency/voltage operating point)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PState {
    /// CPU frequency in MHz
    pub(crate) frequency_mhz: u32,
    /// Core voltage in millivolts
    pub(crate) voltage_mv: u32,
    /// Estimated power draw in milliwatts
    pub(crate) power_mw: u32,
}

/// Maximum number of P-state entries
pub(crate) const MAX_PSTATES: usize = 16;

/// Default P-state table (generic x86_64-like frequency steps)
const DEFAULT_PSTATE_TABLE: [PState; 4] = [
    PState {
        frequency_mhz: 800,
        voltage_mv: 700,
        power_mw: 5_000,
    },
    PState {
        frequency_mhz: 1600,
        voltage_mv: 900,
        power_mw: 15_000,
    },
    PState {
        frequency_mhz: 2400,
        voltage_mv: 1050,
        power_mw: 35_000,
    },
    PState {
        frequency_mhz: 3200,
        voltage_mv: 1200,
        power_mw: 65_000,
    },
];

// ---------------------------------------------------------------------------
// Governor
// ---------------------------------------------------------------------------

/// Frequency scaling governor policy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Governor {
    /// Scale frequency based on CPU utilization
    OnDemand,
    /// Always run at maximum frequency
    Performance,
    /// Always run at minimum frequency
    PowerSave,
}

/// Utilization threshold above which governor selects max P-state (percent)
const GOVERNOR_HIGH_THRESHOLD: u32 = 80;
/// Utilization threshold below which governor selects min P-state (percent)
const GOVERNOR_LOW_THRESHOLD: u32 = 20;

/// Compute the target P-state index for OnDemand governor.
///
/// - utilization >= 80%: returns max index (highest frequency = index 0 is
///   lowest, so max = num_pstates - 1... but conventionally P0 is fastest). We
///   use index `num_pstates - 1` as max frequency (highest index = fastest).
/// - utilization <= 20%: returns 0 (lowest frequency)
/// - Between: linear interpolation (integer math, no FPU)
///
/// Returns the target P-state index in `0..num_pstates`.
fn ondemand_target(utilization_percent: u32, num_pstates: usize) -> usize {
    if num_pstates == 0 {
        return 0;
    }
    let max_idx = num_pstates - 1;
    if max_idx == 0 {
        return 0;
    }

    if utilization_percent >= GOVERNOR_HIGH_THRESHOLD {
        return max_idx;
    }
    if utilization_percent <= GOVERNOR_LOW_THRESHOLD {
        return 0;
    }

    // Linear interpolation between low and high thresholds
    // scaled = (util - low) * max_idx / (high - low)
    let range = GOVERNOR_HIGH_THRESHOLD - GOVERNOR_LOW_THRESHOLD; // 60
    let offset = utilization_percent - GOVERNOR_LOW_THRESHOLD;
    let scaled = (offset as usize * max_idx) / range as usize;

    // Clamp just in case
    if scaled > max_idx {
        max_idx
    } else {
        scaled
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Global power management state
struct PowerState {
    /// Supported C-states
    cstates: [CStateInfo; CState::COUNT],
    /// Supported P-states (valid entries: 0..num_pstates)
    pstates: [PState; MAX_PSTATES],
    /// Number of valid P-state entries
    num_pstates: usize,
    /// Active governor policy
    governor: Governor,
    /// Whether the subsystem has been initialized
    initialized: bool,
    /// Whether MWAIT is supported (x86_64)
    mwait_supported: bool,
}

impl PowerState {
    const fn new() -> Self {
        Self {
            cstates: DEFAULT_CSTATE_TABLE,
            pstates: [PState {
                frequency_mhz: 0,
                voltage_mv: 0,
                power_mw: 0,
            }; MAX_PSTATES],
            num_pstates: 0,
            governor: Governor::OnDemand,
            initialized: false,
            mwait_supported: false,
        }
    }
}

static POWER_STATE: RwLock<PowerState> = RwLock::new(PowerState::new());

/// Current C-state (atomic for lock-free read from interrupt context)
static CURRENT_CSTATE: AtomicU8 = AtomicU8::new(0); // C0

/// Current P-state index (atomic for lock-free read)
static CURRENT_PSTATE: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the power management subsystem.
///
/// Detects hardware capabilities (MWAIT, P-state support) and populates
/// the C-state and P-state tables. Falls back to hardcoded defaults when
/// ACPI tables are not available.
pub(crate) fn init() -> Result<(), KernelError> {
    let mut state = POWER_STATE.write();
    if state.initialized {
        return Err(KernelError::AlreadyExists {
            resource: "power",
            id: 0,
        });
    }

    // Detect MWAIT support on x86_64 bare metal
    state.mwait_supported = detect_mwait();

    // Update C-state support based on hardware
    // C0 and C1 (HLT/WFI) are always supported
    // C2/C3 require MWAIT on x86_64
    state.cstates = DEFAULT_CSTATE_TABLE;
    #[cfg(target_arch = "x86_64")]
    {
        state.cstates[2].supported = state.mwait_supported; // C2
        state.cstates[3].supported = state.mwait_supported; // C3
    }

    // Populate P-state table from defaults
    // (In a full implementation, this would parse ACPI _PSS objects)
    let defaults = &DEFAULT_PSTATE_TABLE;
    for (i, pstate) in defaults.iter().enumerate() {
        state.pstates[i] = *pstate;
    }
    state.num_pstates = defaults.len();

    state.initialized = true;
    CURRENT_CSTATE.store(CState::C0 as u8, Ordering::Release);
    CURRENT_PSTATE.store(0, Ordering::Release);

    Ok(())
}

// ---------------------------------------------------------------------------
// MWAIT detection
// ---------------------------------------------------------------------------

/// Check if MWAIT/MONITOR instructions are supported.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn detect_mwait() -> bool {
    // CPUID leaf 1, ECX bit 3 = MONITOR/MWAIT support
    let ecx: u32;
    // SAFETY: CPUID is a non-privileged instruction that returns CPU feature
    // flags. Leaf 1 is universally supported on x86_64. We save/restore rbx
    // because LLVM reserves it.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") 1u32 => _,
            inout("ecx") 0u32 => ecx,
            out("edx") _,
        );
    }
    ecx & (1 << 3) != 0
}

/// Stub for non-x86_64 or host-target builds.
#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
fn detect_mwait() -> bool {
    false
}

// ---------------------------------------------------------------------------
// C-state transitions
// ---------------------------------------------------------------------------

/// Enter an idle state. The CPU will halt until an interrupt occurs.
///
/// The actual C-state entered may be lower than `suggested` if the hardware
/// does not support the requested state.
pub(crate) fn enter_idle(suggested: CState) {
    if suggested == CState::C0 {
        // C0 = active, nothing to do
        return;
    }

    // Clamp to the deepest supported state
    let effective = clamp_cstate(suggested);
    CURRENT_CSTATE.store(effective as u8, Ordering::Release);

    // Architecture-specific idle entry
    arch_enter_idle(effective);

    // We have returned from idle (interrupt woke us)
    CURRENT_CSTATE.store(CState::C0 as u8, Ordering::Release);
}

/// Clamp a requested C-state down to the deepest supported one.
fn clamp_cstate(requested: CState) -> CState {
    let state = POWER_STATE.read();
    let mut best = CState::C1; // C1 is always supported
    let req_val = requested as u8;

    for info in &state.cstates {
        let sv = info.state as u8;
        if sv <= req_val && sv >= (best as u8) && info.supported && sv > 0 {
            best = info.state;
        }
    }
    best
}

// --- x86_64 bare-metal idle ---

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn arch_enter_idle(cstate: CState) {
    match cstate {
        CState::C0 => {} // Should not reach here
        CState::C1 => {
            // HLT: halt until next interrupt
            // SAFETY: HLT is a privileged instruction that halts the CPU
            // until an interrupt fires. Interrupts must be enabled.
            unsafe {
                core::arch::asm!("sti; hlt", options(nomem, nostack));
            }
        }
        CState::C2 => {
            // MWAIT with C2 hint (sub-state 0x10)
            x86_mwait(0x10);
        }
        CState::C3 => {
            // MWAIT with C3 hint (sub-state 0x20)
            x86_mwait(0x20);
        }
    }
}

/// Execute MONITOR + MWAIT with the given hint.
///
/// MONITOR sets up an address range to watch; MWAIT halts until a write
/// to that range (or an interrupt) occurs. We use a dummy stack variable
/// as the monitored address.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn x86_mwait(hint: u32) {
    // SAFETY: MONITOR/MWAIT are privileged instructions. We pass a valid
    // stack address for MONITOR. MWAIT halts until an interrupt or store
    // to the monitored region. Interrupts must be enabled.
    unsafe {
        let dummy: u64 = 0;
        let addr = &dummy as *const u64 as usize;
        core::arch::asm!(
            "monitor",
            in("eax") addr as u32,
            in("ecx") 0u32,
            in("edx") 0u32,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "sti",
            "mwait",
            in("eax") hint,
            in("ecx") 0u32, // no extensions
            options(nomem, nostack),
        );
    }
}

// --- AArch64 bare-metal idle ---

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn arch_enter_idle(_cstate: CState) {
    // AArch64: WFI (Wait For Interrupt) for all idle states.
    // Deeper C-states would require platform-specific PSCI calls.
    // SAFETY: WFI halts the core until an interrupt occurs.
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
    }
}

// --- RISC-V bare-metal idle ---

#[cfg(all(target_arch = "riscv64", target_os = "none"))]
fn arch_enter_idle(_cstate: CState) {
    // RISC-V: WFI (Wait For Interrupt) for all idle states.
    // Deeper states would require SBI HSM extension calls.
    // SAFETY: WFI halts the hart until an interrupt occurs.
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
    }
}

// --- Host target stub ---

#[cfg(not(target_os = "none"))]
fn arch_enter_idle(_cstate: CState) {
    // No-op on host target (used for unit testing / CI)
}

// ---------------------------------------------------------------------------
// P-state transitions
// ---------------------------------------------------------------------------

/// MSR address for IA32_PERF_CTL (P-state selection)
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
const IA32_PERF_CTL: u32 = 0x199;

/// Set the CPU frequency to the given P-state index.
///
/// Index 0 is the lowest frequency; index `num_pstates - 1` is the highest.
/// Returns `Err` if the index is out of range or the subsystem is not
/// initialized.
pub(crate) fn set_frequency(pstate_index: usize) -> Result<(), KernelError> {
    let state = POWER_STATE.read();
    if !state.initialized {
        return Err(KernelError::NotInitialized { subsystem: "power" });
    }
    if pstate_index >= state.num_pstates {
        return Err(KernelError::InvalidArgument {
            name: "pstate_index",
            value: "out of range",
        });
    }

    let pstate = state.pstates[pstate_index];
    drop(state); // Release lock before hardware access

    arch_set_pstate(pstate_index, &pstate);
    CURRENT_PSTATE.store(pstate_index, Ordering::Release);

    Ok(())
}

// --- x86_64 bare-metal P-state write ---

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn arch_set_pstate(index: usize, _pstate: &PState) {
    // IA32_PERF_CTL bits [15:0] select the target P-state.
    // The actual encoding is platform-specific; we use the index directly
    // as a simplified mapping. A production implementation would use the
    // ratio field from ACPI _PSS.
    let value = index as u64 & 0xFFFF;
    crate::arch::x86_64::msr::wrmsr(IA32_PERF_CTL, value);
}

// --- Non-x86_64 / host-target stub ---

#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
fn arch_set_pstate(_index: usize, _pstate: &PState) {
    // P-state control is architecture-specific.
    // AArch64: SCMI or platform firmware calls would go here.
    // RISC-V: No standard P-state interface; vendor-specific.
    // Host: No-op for testing.
}

// ---------------------------------------------------------------------------
// Governor
// ---------------------------------------------------------------------------

/// Called periodically (e.g., on scheduler tick) to adjust P-state
/// based on CPU utilization.
///
/// `utilization_percent` should be 0..=100 representing the fraction
/// of the last tick period the CPU was not idle.
pub(crate) fn governor_tick(utilization_percent: u32) -> Result<(), KernelError> {
    let state = POWER_STATE.read();
    if !state.initialized {
        return Err(KernelError::NotInitialized { subsystem: "power" });
    }

    let governor = state.governor;
    let num = state.num_pstates;
    drop(state);

    if num == 0 {
        return Ok(());
    }

    let target = match governor {
        Governor::OnDemand => ondemand_target(utilization_percent, num),
        Governor::Performance => num - 1,
        Governor::PowerSave => 0,
    };

    let current = CURRENT_PSTATE.load(Ordering::Acquire);
    if target != current {
        set_frequency(target)?;
    }

    Ok(())
}

/// Set the active governor policy.
pub(crate) fn set_governor(gov: Governor) -> Result<(), KernelError> {
    let mut state = POWER_STATE.write();
    if !state.initialized {
        return Err(KernelError::NotInitialized { subsystem: "power" });
    }
    state.governor = gov;
    Ok(())
}

/// Get the active governor policy.
pub(crate) fn get_governor() -> Governor {
    POWER_STATE.read().governor
}

// ---------------------------------------------------------------------------
// Query functions
// ---------------------------------------------------------------------------

/// Get the current C-state (lock-free atomic read).
pub(crate) fn get_current_cstate() -> CState {
    let val = CURRENT_CSTATE.load(Ordering::Acquire);
    CState::from_u8(val).unwrap_or(CState::C0)
}

/// Get the current P-state index (lock-free atomic read).
pub(crate) fn get_current_pstate() -> usize {
    CURRENT_PSTATE.load(Ordering::Acquire)
}

/// Get the table of supported C-states.
///
/// Returns a fixed-size array; check `supported` field per entry.
pub(crate) fn get_supported_cstates() -> [CStateInfo; CState::COUNT] {
    POWER_STATE.read().cstates
}

/// Get the supported P-states.
///
/// Returns a slice-like view: the first `count` entries in the returned
/// tuple are valid.
pub(crate) fn get_supported_pstates() -> ([PState; MAX_PSTATES], usize) {
    let state = POWER_STATE.read();
    (state.pstates, state.num_pstates)
}

/// Get the number of supported P-states.
pub(crate) fn get_pstate_count() -> usize {
    POWER_STATE.read().num_pstates
}

/// Check if the subsystem is initialized.
pub(crate) fn is_initialized() -> bool {
    POWER_STATE.read().initialized
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Governor logic tests --

    #[test]
    fn test_ondemand_high_utilization() {
        // >= 80% should return max index
        assert_eq!(ondemand_target(80, 4), 3);
        assert_eq!(ondemand_target(100, 4), 3);
        assert_eq!(ondemand_target(95, 8), 7);
    }

    #[test]
    fn test_ondemand_low_utilization() {
        // <= 20% should return 0
        assert_eq!(ondemand_target(0, 4), 0);
        assert_eq!(ondemand_target(10, 4), 0);
        assert_eq!(ondemand_target(20, 4), 0);
    }

    #[test]
    fn test_ondemand_mid_utilization() {
        // 50% is in the middle of the 20-80 range (offset 30 out of 60)
        // With 4 P-states (max_idx=3): 30*3/60 = 1
        assert_eq!(ondemand_target(50, 4), 1);

        // 60% -> offset 40 out of 60, 4 states: 40*3/60 = 2
        assert_eq!(ondemand_target(60, 4), 2);

        // 70% -> offset 50 out of 60, 4 states: 50*3/60 = 2
        assert_eq!(ondemand_target(70, 4), 2);
    }

    #[test]
    fn test_ondemand_edge_cases() {
        // 0 P-states
        assert_eq!(ondemand_target(50, 0), 0);
        // 1 P-state
        assert_eq!(ondemand_target(50, 1), 0);
        // 2 P-states, 50% -> offset 30/60 * 1 = 0
        assert_eq!(ondemand_target(50, 2), 0);
        // 2 P-states, 79% -> offset 59/60 * 1 = 0 (integer division)
        assert_eq!(ondemand_target(79, 2), 0);
        // 2 P-states, 80% -> max
        assert_eq!(ondemand_target(80, 2), 1);
    }

    #[test]
    fn test_ondemand_linear_scaling() {
        // With 16 P-states (max_idx=15):
        // 50% -> offset 30 out of 60: 30*15/60 = 7
        assert_eq!(ondemand_target(50, 16), 7);
        // 21% -> offset 1 out of 60: 1*15/60 = 0
        assert_eq!(ondemand_target(21, 16), 0);
        // 79% -> offset 59 out of 60: 59*15/60 = 14
        assert_eq!(ondemand_target(79, 16), 14);
    }

    // -- C-state tests --

    #[test]
    fn test_cstate_ordering() {
        assert!(CState::C0 < CState::C1);
        assert!(CState::C1 < CState::C2);
        assert!(CState::C2 < CState::C3);
    }

    #[test]
    fn test_cstate_from_u8() {
        assert_eq!(CState::from_u8(0), Some(CState::C0));
        assert_eq!(CState::from_u8(1), Some(CState::C1));
        assert_eq!(CState::from_u8(2), Some(CState::C2));
        assert_eq!(CState::from_u8(3), Some(CState::C3));
        assert_eq!(CState::from_u8(4), None);
        assert_eq!(CState::from_u8(255), None);
    }

    #[test]
    fn test_cstate_info_defaults() {
        let table = DEFAULT_CSTATE_TABLE;
        // C0 has zero latency
        assert_eq!(table[0].exit_latency_ns, 0);
        assert_eq!(table[0].target_residency_ns, 0);
        assert!(table[0].supported);

        // C1 has 1us exit latency
        assert_eq!(table[1].exit_latency_ns, 1_000);
        assert!(table[1].supported);

        // Deeper states have increasing latency
        assert!(table[2].exit_latency_ns > table[1].exit_latency_ns);
        assert!(table[3].exit_latency_ns > table[2].exit_latency_ns);
    }

    // -- P-state tests --

    #[test]
    fn test_pstate_defaults() {
        let table = DEFAULT_PSTATE_TABLE;
        // Frequency increases with index
        for i in 1..table.len() {
            assert!(table[i].frequency_mhz > table[i - 1].frequency_mhz);
        }
        // Power increases with frequency
        for i in 1..table.len() {
            assert!(table[i].power_mw > table[i - 1].power_mw);
        }
    }

    #[test]
    fn test_pstate_bounds() {
        // Ensure default P-states are within reasonable bounds
        for p in &DEFAULT_PSTATE_TABLE {
            assert!(p.frequency_mhz >= 100);
            assert!(p.frequency_mhz <= 10_000);
            assert!(p.voltage_mv >= 500);
            assert!(p.voltage_mv <= 2_000);
            assert!(p.power_mw > 0);
            assert!(p.power_mw <= 500_000);
        }
    }

    // -- Integration-style tests (use global state) --

    #[test]
    fn test_enter_idle_c0_noop() {
        // C0 should be a no-op (no halt)
        enter_idle(CState::C0);
        // If we get here, it did not halt
    }

    #[test]
    fn test_governor_variants() {
        // Verify governor enum values
        assert_ne!(Governor::OnDemand, Governor::Performance);
        assert_ne!(Governor::Performance, Governor::PowerSave);
        assert_ne!(Governor::OnDemand, Governor::PowerSave);
    }
}
