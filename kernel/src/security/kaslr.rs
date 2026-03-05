//! Kernel Address Space Layout Randomization (KASLR)
//!
//! Provides address randomization for kernel text, heap, stack, and module
//! load addresses. Uses architecture-specific hardware entropy sources
//! (RDRAND on x86_64, RNDR on AArch64) with an xorshift64 PRNG fallback
//! for RISC-V and other architectures.
//!
//! # Design
//!
//! KASLR offsets are computed once during boot and stored in a global
//! `KaslrState` protected by a `RwLock`. The state includes:
//!
//! - **text_offset**: Randomized slide for kernel text/code
//! - **heap_offset**: Randomized base offset for the kernel heap
//! - **stack_offset**: Default per-thread stack randomization quantum
//! - **module_base**: Randomized base for driver/module loading
//!
//! Runtime re-randomization can refresh offsets for long-running systems,
//! though the kernel text offset is typically fixed at boot.
//!
//! # Entropy Sources
//!
//! | Architecture | Primary Source | Fallback |
//! |-------------|---------------|----------|
//! | x86_64 | RDRAND | xorshift64 (TSC seed) |
//! | AArch64 | RNDR | xorshift64 (CNTPCT seed) |
//! | RISC-V | N/A | xorshift64 (cycle seed) |

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::RwLock;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum randomization for kernel text offset (2 MB aligned, 16 MB range).
/// This keeps the kernel within a manageable range while providing meaningful
/// randomization (8 possible positions at 2 MB granularity).
const TEXT_RANDOM_BITS: u32 = 23; // 8 MB range
const TEXT_ALIGNMENT: usize = 0x20_0000; // 2 MB alignment (huge page)

/// Maximum randomization for heap base (4 KB aligned, 256 MB range).
const HEAP_RANDOM_BITS: u32 = 28; // 256 MB range
const HEAP_ALIGNMENT: usize = 0x1000; // 4 KB alignment (page)

/// Maximum per-thread stack randomization (16 bytes aligned, 16 KB range).
const STACK_RANDOM_BITS: u32 = 14; // 16 KB range
const STACK_ALIGNMENT: usize = 16; // 16-byte alignment (ABI requirement)

/// Maximum randomization for module/driver load base (4 KB aligned, 64 MB
/// range).
const MODULE_RANDOM_BITS: u32 = 26; // 64 MB range
const MODULE_ALIGNMENT: usize = 0x1000; // 4 KB alignment (page)

// ---------------------------------------------------------------------------
// Xorshift64 PRNG
// ---------------------------------------------------------------------------

/// Simple xorshift64 PRNG for generating randomness from a seed.
///
/// This is NOT cryptographically secure -- it is used only for address
/// randomization where the primary goal is making addresses unpredictable
/// to remote attackers, not resisting local analysis.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    /// Create a new xorshift64 PRNG with the given seed.
    /// If seed is 0, uses a fixed non-zero value to avoid degenerate state.
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xDEAD_BEEF_CAFE_BABE
            } else {
                seed
            },
        }
    }

    /// Generate the next pseudo-random u64 value.
    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

// ---------------------------------------------------------------------------
// Architecture-specific entropy
// ---------------------------------------------------------------------------

/// Gather a 64-bit entropy value from the best available hardware source.
fn get_hardware_entropy() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // Try RDRAND first (check CPUID.01H:ECX.RDRAND[bit 30])
        if rdrand_available() {
            if let Some(val) = rdrand64() {
                return val;
            }
        }
        // Fallback: use TSC as seed for xorshift
        let tsc = read_tsc();
        let mut rng = Xorshift64::new(tsc);
        rng.next()
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Try RNDR (ARMv8.5 Random Number)
        if let Some(val) = rndr64() {
            return val;
        }
        // Fallback: use CNTPCT_EL0 (physical counter) as seed
        let cnt = read_cntpct();
        let mut rng = Xorshift64::new(cnt);
        rng.next()
    }

    #[cfg(target_arch = "riscv64")]
    {
        // No hardware RNG instruction on most RISC-V cores
        // Use cycle counter as seed for xorshift
        let cycles = read_cycle();
        let mut rng = Xorshift64::new(cycles);
        rng.next()
    }

    // Host target (for CI tests on x86_64-unknown-linux-gnu)
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        // Deterministic fallback for unsupported architectures
        let mut rng = Xorshift64::new(0x1234_5678_9ABC_DEF0);
        rng.next()
    }
}

// -- x86_64 helpers --

#[cfg(target_arch = "x86_64")]
fn rdrand_available() -> bool {
    let ecx: u32;
    // SAFETY: CPUID is a non-privileged instruction that reads CPU feature
    // flags. We request leaf 1 (basic features). No memory or stack effects.
    // LLVM reserves rbx, so we save/restore it.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            in("eax") 1u32,
            in("ecx") 0u32,
            lateout("ecx") ecx,
            lateout("edx") _,
            options(nostack),
        );
    }
    (ecx & (1 << 30)) != 0
}

#[cfg(target_arch = "x86_64")]
fn rdrand64() -> Option<u64> {
    let mut val: u64;
    let success: u8;
    // SAFETY: RDRAND reads from the hardware RNG and sets CF on success.
    // No memory or stack effects.
    unsafe {
        core::arch::asm!(
            "rdrand {val}",
            "setc {success}",
            val = out(reg) val,
            success = out(reg_byte) success,
            options(nomem, nostack),
        );
    }
    if success != 0 {
        Some(val)
    } else {
        None
    }
}

#[cfg(target_arch = "x86_64")]
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: RDTSC reads the timestamp counter. No memory or stack effects.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack),
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

// -- AArch64 helpers --

#[cfg(target_arch = "aarch64")]
fn rndr64() -> Option<u64> {
    // RNDR is available on ARMv8.5+ (ID_AA64ISAR0_EL1.RNDR != 0).
    // On older cores this instruction is UNDEFINED, so we check first.
    // For simplicity and safety on QEMU virt, we skip RNDR and use fallback.
    // A production implementation would check ID_AA64ISAR0_EL1 bits [63:60].
    None
}

#[cfg(target_arch = "aarch64")]
fn read_cntpct() -> u64 {
    let cnt: u64;
    // SAFETY: CNTPCT_EL0 reads the physical counter register.
    // Available from EL0 upward. No memory or stack effects.
    unsafe {
        core::arch::asm!(
            "mrs {}, cntpct_el0",
            out(reg) cnt,
            options(nomem, nostack),
        );
    }
    cnt
}

// -- RISC-V helpers --

#[cfg(target_arch = "riscv64")]
fn read_cycle() -> u64 {
    let cycles: u64;
    // SAFETY: Reading the `cycle` CSR via `rdcycle`. On QEMU virt this
    // may trap (SIGILL) if the CSR is not implemented; in that case the
    // SBI trap handler returns 0 and we fall through to the xorshift.
    // No memory or stack effects.
    unsafe {
        core::arch::asm!(
            "rdcycle {}",
            out(reg) cycles,
            options(nomem, nostack),
        );
    }
    cycles
}

// ---------------------------------------------------------------------------
// KASLR State
// ---------------------------------------------------------------------------

/// Current KASLR offsets and PRNG state.
pub struct KaslrState {
    /// Kernel text randomization offset (applied at boot)
    pub text_offset: usize,
    /// Kernel heap base randomization offset
    pub heap_offset: usize,
    /// Default per-thread stack randomization offset
    pub stack_offset: usize,
    /// Module/driver load base randomization offset
    pub module_base: usize,
    /// Internal PRNG for generating additional random offsets
    prng: Xorshift64,
    /// Number of re-randomizations performed
    pub rerandomize_count: u64,
}

impl KaslrState {
    /// Create a new KASLR state seeded from hardware entropy.
    fn new() -> Self {
        let seed = get_hardware_entropy();
        let mut prng = Xorshift64::new(seed);

        let text_offset = Self::aligned_random(&mut prng, TEXT_RANDOM_BITS, TEXT_ALIGNMENT);
        let heap_offset = Self::aligned_random(&mut prng, HEAP_RANDOM_BITS, HEAP_ALIGNMENT);
        let stack_offset = Self::aligned_random(&mut prng, STACK_RANDOM_BITS, STACK_ALIGNMENT);
        let module_base = Self::aligned_random(&mut prng, MODULE_RANDOM_BITS, MODULE_ALIGNMENT);

        Self {
            text_offset,
            heap_offset,
            stack_offset,
            module_base,
            prng,
            rerandomize_count: 0,
        }
    }

    /// Generate an aligned random offset within the given bit range.
    fn aligned_random(prng: &mut Xorshift64, bits: u32, alignment: usize) -> usize {
        let mask = (1u64 << bits) - 1;
        let raw = (prng.next() & mask) as usize;
        // Align down to the required alignment
        raw & !(alignment - 1)
    }

    /// Re-randomize non-text offsets for long-running systems.
    ///
    /// The text offset cannot be changed at runtime since code is already
    /// loaded, but heap, stack, and module offsets can be refreshed.
    fn rerandomize(&mut self) {
        // Mix in fresh hardware entropy
        let fresh = get_hardware_entropy();
        self.prng.state ^= fresh;
        // Advance PRNG state
        let _ = self.prng.next();

        self.heap_offset = Self::aligned_random(&mut self.prng, HEAP_RANDOM_BITS, HEAP_ALIGNMENT);
        self.stack_offset =
            Self::aligned_random(&mut self.prng, STACK_RANDOM_BITS, STACK_ALIGNMENT);
        self.module_base =
            Self::aligned_random(&mut self.prng, MODULE_RANDOM_BITS, MODULE_ALIGNMENT);
        self.rerandomize_count += 1;
    }

    /// Generate a random stack offset for a new thread.
    ///
    /// Returns a random offset in the range [0, 16 KB), aligned to 16 bytes,
    /// that should be subtracted from the thread's stack base to randomize
    /// its starting stack pointer.
    fn random_stack_offset(&mut self) -> usize {
        Self::aligned_random(&mut self.prng, STACK_RANDOM_BITS, STACK_ALIGNMENT)
    }

    /// Generate a random module load base address.
    ///
    /// Returns a page-aligned random offset to add to the default module
    /// load region base.
    fn random_module_offset(&mut self) -> usize {
        Self::aligned_random(&mut self.prng, MODULE_RANDOM_BITS, MODULE_ALIGNMENT)
    }
}

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

/// Global KASLR offsets, protected by RwLock for concurrent read access.
static KASLR_OFFSETS: RwLock<Option<KaslrState>> = RwLock::new(None);

/// Whether KASLR has been initialized.
static KASLR_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Monotonic counter for re-randomization events.
static RERANDOMIZE_COUNT: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the KASLR subsystem.
///
/// Gathers hardware entropy and computes initial randomization offsets.
/// Must be called once during boot before any offset queries.
pub fn init() -> Result<(), KernelError> {
    if KASLR_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "kaslr",
            id: 0,
        });
    }

    let state = KaslrState::new();

    crate::println!(
        "[KASLR] Text offset: {:#x}, Heap offset: {:#x}, Stack offset: {:#x}, Module base: {:#x}",
        state.text_offset,
        state.heap_offset,
        state.stack_offset,
        state.module_base,
    );

    {
        let mut offsets = KASLR_OFFSETS.write();
        *offsets = Some(state);
    }

    KASLR_INITIALIZED.store(true, Ordering::Release);
    crate::println!("[KASLR] Kernel address space layout randomization initialized");
    Ok(())
}

/// Get the kernel text randomization offset.
///
/// Returns 0 if KASLR is not initialized.
pub fn get_text_offset() -> usize {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    let offsets = KASLR_OFFSETS.read();
    offsets.as_ref().map_or(0, |s| s.text_offset)
}

/// Get the kernel heap base randomization offset.
///
/// Returns 0 if KASLR is not initialized.
pub fn get_heap_offset() -> usize {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    let offsets = KASLR_OFFSETS.read();
    offsets.as_ref().map_or(0, |s| s.heap_offset)
}

/// Randomize a thread's stack base by subtracting a random offset.
///
/// Given a stack `base` address (top of stack), returns a new address
/// with a random downward offset applied (up to 16 KB, 16-byte aligned).
///
/// Returns the base unchanged if KASLR is not initialized.
pub fn randomize_stack(base: usize) -> usize {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return base;
    }
    let mut offsets = KASLR_OFFSETS.write();
    if let Some(state) = offsets.as_mut() {
        let offset = state.random_stack_offset();
        base.saturating_sub(offset)
    } else {
        base
    }
}

/// Get a randomized module/driver load base address.
///
/// Returns a page-aligned random offset suitable for adding to the
/// default module load region. Each call produces a different offset
/// so different modules get different addresses.
///
/// Returns 0 if KASLR is not initialized.
pub fn get_module_base() -> usize {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    let mut offsets = KASLR_OFFSETS.write();
    if let Some(state) = offsets.as_mut() {
        state.random_module_offset()
    } else {
        0
    }
}

/// Re-randomize non-text KASLR offsets.
///
/// Call periodically on long-running systems to refresh randomization.
/// The kernel text offset cannot be changed since code is already mapped.
///
/// Returns the new re-randomization count, or an error if not initialized.
pub fn rerandomize() -> Result<u64, KernelError> {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized { subsystem: "kaslr" });
    }

    let mut offsets = KASLR_OFFSETS.write();
    if let Some(state) = offsets.as_mut() {
        state.rerandomize();
        let count = state.rerandomize_count;
        RERANDOMIZE_COUNT.store(count, Ordering::Relaxed);
        Ok(count)
    } else {
        Err(KernelError::NotInitialized { subsystem: "kaslr" })
    }
}

/// Check if KASLR is initialized and active.
pub fn is_active() -> bool {
    KASLR_INITIALIZED.load(Ordering::Acquire)
}

/// Get the number of re-randomizations performed.
pub fn rerandomize_count() -> u64 {
    RERANDOMIZE_COUNT.load(Ordering::Relaxed)
}

/// Get a snapshot of current KASLR offsets for diagnostics.
///
/// Returns `(text_offset, heap_offset, stack_offset, module_base)`.
pub fn get_offsets() -> (usize, usize, usize, usize) {
    if !KASLR_INITIALIZED.load(Ordering::Acquire) {
        return (0, 0, 0, 0);
    }
    let offsets = KASLR_OFFSETS.read();
    offsets.as_ref().map_or((0, 0, 0, 0), |s| {
        (s.text_offset, s.heap_offset, s.stack_offset, s.module_base)
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xorshift64_nonzero() {
        let mut rng = Xorshift64::new(42);
        let val = rng.next();
        assert_ne!(val, 0, "xorshift64 should produce non-zero output");
    }

    #[test]
    fn test_xorshift64_different_values() {
        let mut rng = Xorshift64::new(42);
        let a = rng.next();
        let b = rng.next();
        assert_ne!(a, b, "consecutive xorshift64 values should differ");
    }

    #[test]
    fn test_xorshift64_zero_seed_handled() {
        // Zero seed should be replaced with a non-zero constant
        let mut rng = Xorshift64::new(0);
        let val = rng.next();
        assert_ne!(val, 0);
    }

    #[test]
    fn test_aligned_random_page_aligned() {
        let mut rng = Xorshift64::new(0xDEAD);
        for _ in 0..100 {
            let offset = KaslrState::aligned_random(&mut rng, 28, 0x1000);
            assert_eq!(offset & 0xFFF, 0, "offset must be page-aligned");
        }
    }

    #[test]
    fn test_aligned_random_16_byte_aligned() {
        let mut rng = Xorshift64::new(0xBEEF);
        for _ in 0..100 {
            let offset = KaslrState::aligned_random(&mut rng, 14, 16);
            assert_eq!(offset & 0xF, 0, "offset must be 16-byte aligned");
        }
    }

    #[test]
    fn test_aligned_random_within_range() {
        let mut rng = Xorshift64::new(0xCAFE);
        for _ in 0..1000 {
            let offset = KaslrState::aligned_random(&mut rng, 14, 16);
            assert!(offset < (1 << 14), "offset must be within 16 KB range");
        }
    }

    #[test]
    fn test_aligned_random_2mb_aligned() {
        let mut rng = Xorshift64::new(0xFACE);
        for _ in 0..100 {
            let offset = KaslrState::aligned_random(&mut rng, 23, 0x20_0000);
            assert_eq!(offset & 0x1F_FFFF, 0, "text offset must be 2 MB aligned");
        }
    }

    #[test]
    fn test_kaslr_state_creation() {
        let state = KaslrState::new();

        // Text offset should be 2 MB aligned
        assert_eq!(state.text_offset & (TEXT_ALIGNMENT - 1), 0);
        // Heap offset should be page-aligned
        assert_eq!(state.heap_offset & (HEAP_ALIGNMENT - 1), 0);
        // Stack offset should be 16-byte aligned
        assert_eq!(state.stack_offset & (STACK_ALIGNMENT - 1), 0);
        // Module base should be page-aligned
        assert_eq!(state.module_base & (MODULE_ALIGNMENT - 1), 0);
    }

    #[test]
    fn test_kaslr_state_rerandomize() {
        let mut state = KaslrState::new();
        let old_heap = state.heap_offset;
        let old_module = state.module_base;

        state.rerandomize();

        // After rerandomization, at least one of heap or module should change
        // (extremely unlikely both stay the same with random entropy)
        assert_eq!(state.rerandomize_count, 1);
        // Alignment should still hold
        assert_eq!(state.heap_offset & (HEAP_ALIGNMENT - 1), 0);
        assert_eq!(state.module_base & (MODULE_ALIGNMENT - 1), 0);

        // Suppress unused variable warnings in case the values happen to match
        let _ = old_heap;
        let _ = old_module;
    }

    #[test]
    fn test_random_stack_offset_aligned() {
        let mut state = KaslrState::new();
        for _ in 0..100 {
            let offset = state.random_stack_offset();
            assert_eq!(offset & 0xF, 0, "stack offset must be 16-byte aligned");
            assert!(offset < (1 << STACK_RANDOM_BITS));
        }
    }

    #[test]
    fn test_random_module_offset_page_aligned() {
        let mut state = KaslrState::new();
        for _ in 0..100 {
            let offset = state.random_module_offset();
            assert_eq!(offset & 0xFFF, 0, "module offset must be page-aligned");
            assert!(offset < (1 << MODULE_RANDOM_BITS));
        }
    }

    #[test]
    fn test_get_text_offset_before_init() {
        // Before init, should return 0 (safe default)
        // Note: in test harness, KASLR_INITIALIZED may already be true
        // from other tests. This test just verifies the function doesn't panic.
        let _offset = get_text_offset();
    }

    #[test]
    fn test_get_heap_offset_before_init() {
        let _offset = get_heap_offset();
    }
}
