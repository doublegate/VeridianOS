//! Secure Boot Verification
//!
//! Verifies the integrity of the boot chain using cryptographic measurements,
//! signature verification, and TPM PCR extensions.
//!
//! ## Boot Measurement Flow
//!
//! 1. Compute SHA-256 hash of the kernel image in memory
//! 2. Verify kernel signature (if a signature is provided)
//! 3. Record measurement in the boot measurement log
//! 4. Extend TPM PCR 0 with the kernel measurement
//! 5. Return verification status
//!
//! ## PCR Allocation
//!
//! - PCR 0: Kernel image measurement
//! - PCR 1: Kernel configuration / command line
//! - PCR 2: Boot stage measurements (bootloader, early init)

use spin::Mutex;

use crate::{crypto::hash::sha256, error::KernelError};

/// Maximum number of measurements in the boot log
const MAX_BOOT_MEASUREMENTS: usize = 32;

/// Boot verification status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStatus {
    /// Secure boot verified -- kernel hash and signature checked
    Verified,
    /// Secure boot not supported on this platform
    NotSupported,
    /// Secure boot verification failed (hash or signature mismatch)
    Failed,
    /// Secure boot disabled by configuration
    Disabled,
    /// Verified hash only (no signature available)
    HashOnly,
}

/// Secure boot configuration
pub struct SecureBootConfig {
    /// Enable secure boot verification
    pub enabled: bool,
    /// Enforce verification (fail hard on mismatch)
    pub enforce: bool,
    /// Expected kernel hash (for verification against a known-good image)
    pub kernel_hash: Option<[u8; 32]>,
    /// Boot signature for kernel verification
    pub signature: Option<BootSignature>,
    /// Signing public key (Ed25519 verifying key, 32 bytes)
    pub signer_public_key: Option<[u8; 32]>,
}

impl SecureBootConfig {
    /// Create default configuration (secure boot disabled)
    pub const fn default() -> Self {
        Self {
            enabled: false,
            enforce: false,
            kernel_hash: None,
            signature: None,
            signer_public_key: None,
        }
    }
}

/// Boot signature for kernel image verification
#[derive(Clone)]
pub struct BootSignature {
    /// Ed25519 signature bytes (64 bytes)
    pub signature_bytes: [u8; 64],
    /// Signer identity (e.g., "VeridianOS Release Signing Key")
    pub signer: &'static str,
    /// Signing algorithm identifier
    pub algorithm: SignatureAlgorithm,
}

/// Supported signature algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlgorithm {
    /// Ed25519 (RFC 8032)
    Ed25519,
}

/// A single boot measurement entry in the measurement log
#[derive(Clone)]
pub struct BootMeasurement {
    /// Human-readable description of what was measured
    pub stage: &'static str,
    /// SHA-256 hash of the measured data
    pub hash: [u8; 32],
    /// Monotonic timestamp (kernel tick counter or similar)
    pub timestamp: u64,
    /// PCR index this measurement was extended into (if any)
    pub pcr_index: Option<u8>,
}

/// Boot measurement log recording all measurements taken during boot.
///
/// This log is analogous to the TCG event log -- it records what was measured
/// and extended into PCRs, allowing later attestation to reconstruct the
/// expected PCR values.
pub struct BootMeasurementLog {
    entries: [Option<BootMeasurement>; MAX_BOOT_MEASUREMENTS],
    count: usize,
}

impl BootMeasurementLog {
    const fn new() -> Self {
        // Use const initialization compatible with no_std
        const NONE: Option<BootMeasurement> = None;
        Self {
            entries: [NONE; MAX_BOOT_MEASUREMENTS],
            count: 0,
        }
    }

    /// Record a new measurement in the log.
    ///
    /// Returns the index of the new entry, or None if the log is full.
    fn record(
        &mut self,
        stage: &'static str,
        hash: [u8; 32],
        timestamp: u64,
        pcr_index: Option<u8>,
    ) -> Option<usize> {
        if self.count >= MAX_BOOT_MEASUREMENTS {
            return None;
        }
        let idx = self.count;
        self.entries[idx] = Some(BootMeasurement {
            stage,
            hash,
            timestamp,
            pcr_index,
        });
        self.count += 1;
        Some(idx)
    }

    /// Get the number of measurements recorded
    pub fn len(&self) -> usize {
        self.count
    }

    /// Check if the log is empty
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get a measurement by index
    pub fn get(&self, index: usize) -> Option<&BootMeasurement> {
        if index < self.count {
            self.entries[index].as_ref()
        } else {
            None
        }
    }

    /// Get all recorded measurements as a slice-like iterator
    pub fn measurements(&self) -> &[Option<BootMeasurement>] {
        &self.entries[..self.count]
    }
}

/// Global state: configuration + measurement log
struct SecureBootState {
    config: SecureBootConfig,
    measurement_log: BootMeasurementLog,
    last_status: BootStatus,
}

impl SecureBootState {
    const fn new() -> Self {
        Self {
            config: SecureBootConfig::default(),
            measurement_log: BootMeasurementLog::new(),
            last_status: BootStatus::Disabled,
        }
    }
}

static STATE: Mutex<SecureBootState> = Mutex::new(SecureBootState::new());

// ============================================================================
// Kernel image hashing
// ============================================================================

/// Compute a SHA-256 hash of the kernel image in memory.
///
/// Uses the `__kernel_end` linker symbol (defined in all three architecture
/// linker scripts) to determine the extent of the kernel image.  The start
/// address is derived from the architecture-specific load address.
///
/// If linker symbols are not available, falls back to hashing the first 64 KiB
/// of the kernel text section.
pub fn compute_kernel_hash() -> Result<[u8; 32], KernelError> {
    // Try to use linker-provided kernel extent
    let (start, size) = get_kernel_extent();

    println!(
        "[SECBOOT] Hashing kernel image: start=0x{:X}, size={} bytes",
        start, size
    );

    // Read kernel memory and hash it
    // SAFETY: We are reading the kernel's own mapped text/data/bss sections.
    // These addresses come from the linker script and are always valid while
    // the kernel is running.
    let kernel_bytes = unsafe { core::slice::from_raw_parts(start as *const u8, size) };

    let hash = sha256(kernel_bytes);
    let result = *hash.as_bytes();

    println!(
        "[SECBOOT] Kernel SHA-256: {:02X}{:02X}{:02X}{:02X}...{:02X}{:02X}{:02X}{:02X}",
        result[0], result[1], result[2], result[3], result[28], result[29], result[30], result[31],
    );

    Ok(result)
}

/// Get the kernel image start address and size from linker symbols.
///
/// Each architecture has a `__kernel_end` symbol in its linker script.
/// The start address is the architecture-specific kernel load address.
fn get_kernel_extent() -> (usize, usize) {
    extern "C" {
        static __kernel_end: u8;
    }

    // Architecture-specific kernel start addresses (from linker scripts)
    #[cfg(target_arch = "x86_64")]
    let kernel_start: usize = 0xFFFFFFFF80100000;

    #[cfg(target_arch = "aarch64")]
    let kernel_start: usize = 0x40080000; // QEMU virt machine load address

    #[cfg(target_arch = "riscv64")]
    let kernel_start: usize = 0x80200000; // OpenSBI jump address

    let kernel_end = unsafe { &__kernel_end as *const u8 as usize };

    // Sanity check: kernel_end must be after kernel_start
    if kernel_end > kernel_start && (kernel_end - kernel_start) < 64 * 1024 * 1024 {
        (kernel_start, kernel_end - kernel_start)
    } else {
        // Fallback: hash first 64 KiB from start
        println!(
            "[SECBOOT] Warning: kernel extent invalid (start=0x{:X}, end=0x{:X}), using 64K \
             fallback",
            kernel_start, kernel_end
        );
        (kernel_start, 64 * 1024)
    }
}

// ============================================================================
// Signature verification
// ============================================================================

/// Verify the kernel image signature using Ed25519.
///
/// If no signature or public key is configured, returns `Ok(false)` (not an
/// error, just unsigned).  If the crypto module's Ed25519 verifier is
/// available, delegates to it; otherwise falls back to hash comparison against
/// a known-good hash.
fn verify_kernel_signature(
    kernel_hash: &[u8; 32],
    config: &SecureBootConfig,
) -> Result<bool, KernelError> {
    // Check if we have a signature and public key
    let signature = match &config.signature {
        Some(sig) => sig,
        None => {
            println!("[SECBOOT] No boot signature configured -- skipping signature check");
            return Ok(false);
        }
    };

    let public_key = match &config.signer_public_key {
        Some(pk) => pk,
        None => {
            println!("[SECBOOT] No signer public key configured -- skipping signature check");
            return Ok(false);
        }
    };

    println!(
        "[SECBOOT] Verifying {:?} signature from '{}'",
        signature.algorithm, signature.signer
    );

    // Try Ed25519 verification via the crypto module
    match signature.algorithm {
        SignatureAlgorithm::Ed25519 => {
            use crate::crypto::asymmetric::{Signature, VerifyingKey};

            let verifying_key = match VerifyingKey::from_bytes(public_key) {
                Ok(vk) => vk,
                Err(_e) => {
                    println!("[SECBOOT] Invalid verifying key: {:?}", _e);
                    return Ok(false);
                }
            };

            let sig = match Signature::from_bytes(&signature.signature_bytes) {
                Ok(s) => s,
                Err(_e) => {
                    println!("[SECBOOT] Invalid signature format: {:?}", _e);
                    return Ok(false);
                }
            };

            // Verify the signature over the kernel hash
            match verifying_key.verify(kernel_hash, &sig) {
                Ok(valid) => {
                    #[allow(clippy::if_same_then_else)]
                    if valid {
                        println!("[SECBOOT] Ed25519 signature VALID");
                    } else {
                        println!("[SECBOOT] Ed25519 signature INVALID");
                    }
                    Ok(valid)
                }
                Err(_e) => {
                    println!("[SECBOOT] Signature verification error: {:?}", _e);
                    Ok(false)
                }
            }
        }
    }
}

/// Fall back to comparing the computed hash against a known-good hash.
fn verify_hash_only(computed_hash: &[u8; 32], config: &SecureBootConfig) -> bool {
    match &config.kernel_hash {
        Some(expected) => {
            let matches = computed_hash == expected;
            if matches {
                println!("[SECBOOT] Kernel hash matches expected value");
            } else {
                println!("[SECBOOT] Kernel hash MISMATCH");
                println!(
                    "[SECBOOT]   Expected: {:02X}{:02X}{:02X}{:02X}...",
                    expected[0], expected[1], expected[2], expected[3]
                );
                println!(
                    "[SECBOOT]   Computed: {:02X}{:02X}{:02X}{:02X}...",
                    computed_hash[0], computed_hash[1], computed_hash[2], computed_hash[3]
                );
            }
            matches
        }
        None => {
            println!("[SECBOOT] No expected kernel hash configured");
            false
        }
    }
}

// ============================================================================
// TPM PCR extension
// ============================================================================

/// Extend TPM PCR with a measurement hash.
///
/// Silently succeeds if no TPM is available (the measurement is still recorded
/// in the boot log regardless).
fn extend_pcr(pcr_index: u8, measurement: &[u8; 32]) {
    match super::tpm::pcr_extend(pcr_index, measurement) {
        Ok(()) => {
            println!("[SECBOOT] Extended TPM PCR {} with measurement", pcr_index);
        }
        Err(_e) => {
            println!(
                "[SECBOOT] TPM PCR extend failed for PCR {}: {:?} (continuing)",
                pcr_index, _e
            );
        }
    }
}

// ============================================================================
// Measured boot
// ============================================================================

/// Record a boot stage measurement.
///
/// Computes SHA-256 of the given data, records it in the measurement log,
/// and optionally extends a TPM PCR.
pub fn measure_boot_stage(
    stage: &'static str,
    data: &[u8],
    pcr_index: Option<u8>,
) -> Result<[u8; 32], KernelError> {
    let hash = sha256(data);
    let hash_bytes = *hash.as_bytes();

    // Get a timestamp (use a simple counter since we may not have a timer yet)
    let timestamp = get_boot_timestamp();

    let mut state = STATE.lock();
    if let Some(_idx) = state
        .measurement_log
        .record(stage, hash_bytes, timestamp, pcr_index)
    {
        println!(
            "[SECBOOT] Measurement #{}: '{}' hash={:02X}{:02X}{:02X}{:02X}...",
            _idx, stage, hash_bytes[0], hash_bytes[1], hash_bytes[2], hash_bytes[3]
        );
    } else {
        println!(
            "[SECBOOT] Warning: measurement log full, could not record '{}'",
            stage
        );
    }
    drop(state);

    // Extend TPM PCR if requested
    if let Some(pcr) = pcr_index {
        extend_pcr(pcr, &hash_bytes);
    }

    Ok(hash_bytes)
}

/// Get a monotonic boot timestamp.
///
/// Uses architecture-specific cycle counter or falls back to a simple counter.
fn get_boot_timestamp() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ============================================================================
// Main verification entry point
// ============================================================================

/// Verify secure boot chain.
///
/// This is the main entry point called during kernel initialization.
/// It performs the following steps:
///
/// 1. Check if secure boot is enabled
/// 2. Hash the kernel image
/// 3. Record the measurement in the boot log
/// 4. Extend TPM PCR 0 with the kernel hash
/// 5. Verify the kernel signature (if configured)
/// 6. Fall back to hash comparison (if no signature)
/// 7. Return overall verification status
pub fn verify() -> Result<(), KernelError> {
    let state = STATE.lock();
    let enabled = state.config.enabled;
    let enforce = state.config.enforce;
    drop(state);

    if !enabled {
        println!("[SECBOOT] Secure boot disabled");
        // Skip kernel hashing when disabled - it's too slow on emulated platforms
        // (11MB+ in software SHA-256 takes minutes on QEMU).
        // Hash will be computed on-demand when secure boot is enabled.
        let mut state = STATE.lock();
        state.last_status = BootStatus::Disabled;
        drop(state);
        return Ok(());
    }

    println!("[SECBOOT] Verifying secure boot chain...");

    // Step 1: Hash the kernel image
    let kernel_hash = compute_kernel_hash()?;

    // Step 2: Record measurement and extend PCR 0
    {
        let mut state = STATE.lock();
        let ts = get_boot_timestamp();
        state
            .measurement_log
            .record("kernel_image", kernel_hash, ts, Some(0));
    }
    extend_pcr(0, &kernel_hash);

    // Step 3: Verify signature
    let state_guard = STATE.lock();
    let sig_valid = verify_kernel_signature(&kernel_hash, &state_guard.config)?;
    let hash_valid = verify_hash_only(&kernel_hash, &state_guard.config);
    let has_signature = state_guard.config.signature.is_some();
    drop(state_guard);

    // Step 4: Determine overall status
    let status = if has_signature && sig_valid {
        BootStatus::Verified
    } else if hash_valid {
        BootStatus::HashOnly
    } else if !has_signature && !state_guard_has_expected_hash() {
        // No signature and no expected hash configured -- can't verify
        println!("[SECBOOT] No verification material configured");
        BootStatus::NotSupported
    } else {
        BootStatus::Failed
    };

    // Step 5: Record status and enforce
    {
        let mut state = STATE.lock();
        state.last_status = status;
    }

    match status {
        BootStatus::Verified => {
            println!("[SECBOOT] Secure boot verification PASSED (signature valid)");
            Ok(())
        }
        BootStatus::HashOnly => {
            println!("[SECBOOT] Secure boot verification PASSED (hash match, no signature)");
            Ok(())
        }
        BootStatus::NotSupported => {
            println!("[SECBOOT] Secure boot: no verification material configured");
            if enforce {
                Err(KernelError::NotImplemented {
                    feature: "secure boot verification material",
                })
            } else {
                Ok(())
            }
        }
        BootStatus::Failed => {
            println!("[SECBOOT] Secure boot verification FAILED");
            if enforce {
                Err(KernelError::PermissionDenied {
                    operation: "secure boot enforcement",
                })
            } else {
                println!("[SECBOOT] Non-enforcing mode -- continuing despite failure");
                Ok(())
            }
        }
        BootStatus::Disabled => {
            // Should not reach here (we checked enabled above)
            Ok(())
        }
    }
}

/// Helper to check if expected hash is configured (avoids holding lock across
/// verify calls)
fn state_guard_has_expected_hash() -> bool {
    let state = STATE.lock();
    state.config.kernel_hash.is_some()
}

// ============================================================================
// Configuration API
// ============================================================================

/// Enable secure boot with optional enforcement.
pub fn enable(enforce: bool) {
    let mut state = STATE.lock();
    state.config.enabled = true;
    state.config.enforce = enforce;
    println!("[SECBOOT] Secure boot enabled (enforce={})", enforce);
}

/// Disable secure boot.
pub fn disable() {
    let mut state = STATE.lock();
    state.config.enabled = false;
    state.config.enforce = false;
    state.last_status = BootStatus::Disabled;
    println!("[SECBOOT] Secure boot disabled");
}

/// Set the expected kernel hash for verification.
pub fn set_expected_hash(hash: [u8; 32]) {
    let mut state = STATE.lock();
    state.config.kernel_hash = Some(hash);
}

/// Set the boot signature and signer public key.
pub fn set_signature(signature: BootSignature, public_key: [u8; 32]) {
    let mut state = STATE.lock();
    state.config.signature = Some(signature);
    state.config.signer_public_key = Some(public_key);
}

/// Get the current boot verification status.
pub fn get_status() -> BootStatus {
    let state = STATE.lock();
    state.last_status
}

/// Get the number of recorded boot measurements.
pub fn measurement_count() -> usize {
    let state = STATE.lock();
    state.measurement_log.len()
}

/// Get a recorded boot measurement by index.
///
/// Returns a copy of the measurement entry (stage name, hash, timestamp, PCR
/// index).
pub fn get_measurement(index: usize) -> Option<(&'static str, [u8; 32], u64, Option<u8>)> {
    let state = STATE.lock();
    state
        .measurement_log
        .get(index)
        .map(|m| (m.stage, m.hash, m.timestamp, m.pcr_index))
}

/// Print all boot measurements to the kernel console.
pub fn print_measurement_log() {
    let state = STATE.lock();
    println!(
        "[SECBOOT] Boot Measurement Log ({} entries):",
        state.measurement_log.len()
    );
    for i in 0..state.measurement_log.len() {
        if let Some(_m) = state.measurement_log.get(i) {
            println!(
                "[SECBOOT]   #{}: stage='{}' hash={:02X}{:02X}{:02X}{:02X}... pcr={:?} ts={}",
                i,
                _m.stage,
                _m.hash[0],
                _m.hash[1],
                _m.hash[2],
                _m.hash[3],
                _m.pcr_index,
                _m.timestamp
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_kernel_hash() {
        let hash = compute_kernel_hash();
        assert!(hash.is_ok());
        let h = hash.unwrap();
        // Hash should be non-zero (kernel image exists)
        assert_ne!(h, [0u8; 32]);
    }

    #[test_case]
    fn test_verify_disabled() {
        // Should succeed when disabled (default)
        assert!(verify().is_ok());
    }

    #[test_case]
    fn test_boot_measurement_log() {
        let mut log = BootMeasurementLog::new();
        assert!(log.is_empty());

        let hash = [0x42u8; 32];
        let idx = log.record("test_stage", hash, 100, Some(0));
        assert_eq!(idx, Some(0));
        assert_eq!(log.len(), 1);
        assert!(!log.is_empty());

        let m = log.get(0).unwrap();
        assert_eq!(m.stage, "test_stage");
        assert_eq!(m.hash, hash);
        assert_eq!(m.timestamp, 100);
        assert_eq!(m.pcr_index, Some(0));
    }

    #[test_case]
    fn test_measure_boot_stage() {
        let data = b"test boot stage data";
        let result = measure_boot_stage("test_boot", data, None);
        assert!(result.is_ok());
        let hash = result.unwrap();
        // Hash should match SHA-256 of the input
        let expected = sha256(data);
        assert_eq!(&hash, expected.as_bytes());
    }
}
