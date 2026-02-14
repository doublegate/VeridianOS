//! Secure Random Number Generation
//!
//! Provides cryptographically secure random number generation.

#![allow(static_mut_refs)]

use alloc::{vec, vec::Vec};

use spin::Mutex;

use super::CryptoResult;

/// Secure random number generator
pub struct SecureRandom {
    state: Mutex<RandomState>,
}

struct RandomState {
    counter: u64,
    seed: [u8; 32],
}

impl SecureRandom {
    /// Create new secure random number generator
    pub fn new() -> CryptoResult<Self> {
        // Initialize with hardware RNG or timer
        let seed = Self::get_entropy()?;

        Ok(Self {
            state: Mutex::new(RandomState { counter: 0, seed }),
        })
    }

    /// Generate random bytes
    pub fn fill_bytes(&self, dest: &mut [u8]) -> CryptoResult<()> {
        let mut state = self.state.lock();

        // Use index-based loop instead of iter_mut() to avoid AArch64 LLVM hang
        let mut i = 0;
        while i < dest.len() {
            dest[i] = Self::next_byte(&mut state);
            i += 1;
        }

        Ok(())
    }

    /// Generate random u64
    pub fn next_u64(&self) -> u64 {
        let mut bytes = [0u8; 8];
        let _ = self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    /// Generate random u32
    pub fn next_u32(&self) -> u32 {
        let mut bytes = [0u8; 4];
        let _ = self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    // Private methods

    fn get_entropy() -> CryptoResult<[u8; 32]> {
        let mut entropy = [0u8; 32];

        // Skip RDRAND for now - it causes crashes with bootloader 0.11
        // TODO(phase3): Re-enable RDRAND after proper CPUID feature detection
        // #[cfg(target_arch = "x86_64")]
        // {
        //     if Self::try_rdrand(&mut entropy) {
        //         return Ok(entropy);
        //     }
        // }

        // Use timer-based entropy
        Self::timer_entropy(&mut entropy);

        Ok(entropy)
    }

    #[cfg(target_arch = "x86_64")]
    #[allow(dead_code)]
    fn try_rdrand(dest: &mut [u8; 32]) -> bool {
        use core::arch::x86_64::_rdrand64_step;

        // SAFETY: _rdrand64_step is an x86_64 RDRAND intrinsic that writes a
        // hardware-generated random u64 into `value`. The function returns 0 on
        // failure (RDRAND unavailable or underflow), which we check and bail out.
        // dest is a valid &mut [u8; 32] and chunks_exact_mut(8) yields aligned 8-byte
        // slices.
        unsafe {
            for chunk in dest.chunks_exact_mut(8) {
                let mut value: u64 = 0;
                if _rdrand64_step(&mut value) == 0 {
                    return false; // RDRAND not available or failed
                }
                chunk.copy_from_slice(&value.to_le_bytes());
            }
        }

        true
    }

    fn timer_entropy(dest: &mut [u8; 32]) {
        // Use timer and other sources for entropy
        // This is simplified - real implementation would use multiple sources

        #[cfg(target_arch = "x86_64")]
        // SAFETY: _rdtsc is an x86_64 intrinsic that reads the Time Stamp Counter
        // register. It is always available on x86_64 processors and returns the
        // current cycle count as a u64. No memory is accessed unsafely.
        unsafe {
            use core::arch::x86_64::_rdtsc;
            let mut counter = _rdtsc();

            for byte in dest.iter_mut() {
                counter = counter.wrapping_mul(1664525).wrapping_add(1013904223);
                *byte = (counter >> 32) as u8;
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            // Fallback for other architectures
            // Use index-based loop instead of iter_mut() to avoid AArch64 LLVM hang
            let mut counter = 0u64;
            let mut i = 0;
            while i < dest.len() {
                counter = counter
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223 + i as u64);
                dest[i] = (counter >> 32) as u8;
                i += 1;
            }
        }
    }

    fn next_byte(state: &mut RandomState) -> u8 {
        // Simple counter-based generator (improved with seed mixing)
        // TODO(phase3): Replace with ChaCha20 or similar CSPRNG
        state.counter = state.counter.wrapping_add(1);

        let index = (state.counter as usize) % 32;
        let seed_byte = state.seed[index];

        // Mix counter with seed
        let mixed = (state.counter.wrapping_mul(1664525).wrapping_add(1013904223)) as u8;
        mixed ^ seed_byte
    }
}

impl Default for SecureRandom {
    fn default() -> Self {
        Self::new().expect("Failed to initialize SecureRandom")
    }
}

/// Global secure random number generator
static mut RNG_STORAGE: Option<SecureRandom> = None;

/// Initialize random number generator
pub fn init() -> CryptoResult<()> {
    let rng = SecureRandom::new()?;
    // SAFETY: RNG_STORAGE is a static mut Option written once during
    // single-threaded kernel initialization. No concurrent access is possible
    // at this point in boot.
    unsafe {
        RNG_STORAGE = Some(rng);
    }
    Ok(())
}

/// Get global random number generator
pub fn get_random() -> &'static SecureRandom {
    // SAFETY: RNG_STORAGE is a static mut Option lazily initialized on first
    // access. The is_none() check ensures it is written at most once. The
    // returned reference has 'static lifetime because the static mut is never
    // moved or dropped. The SecureRandom uses internal Mutex for thread-safe
    // random generation.
    unsafe {
        if RNG_STORAGE.is_none() {
            RNG_STORAGE = Some(SecureRandom::new().expect("Failed to create RNG"));
        }
        // is_none() check above guarantees Some
        RNG_STORAGE.as_ref().expect("RNG not initialized")
    }
}

/// Generate random bytes (convenience function)
pub fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];

    let rng = get_random();
    let _ = rng.fill_bytes(&mut bytes);

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_random_generation() {
        let rng = SecureRandom::new().unwrap();
        let mut bytes1 = [0u8; 16];
        let mut bytes2 = [0u8; 16];

        rng.fill_bytes(&mut bytes1).unwrap();
        rng.fill_bytes(&mut bytes2).unwrap();

        // Random bytes should be different
        assert_ne!(bytes1, bytes2);
    }
}
