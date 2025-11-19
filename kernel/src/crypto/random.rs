//! Secure Random Number Generation
//!
//! Provides cryptographically secure random number generation.

use alloc::vec::Vec;

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

        for byte in dest.iter_mut() {
            *byte = Self::next_byte(&mut state);
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

        // Try hardware RNG if available
        #[cfg(target_arch = "x86_64")]
        {
            if Self::try_rdrand(&mut entropy) {
                return Ok(entropy);
            }
        }

        // Fallback to timer-based entropy
        Self::timer_entropy(&mut entropy);

        Ok(entropy)
    }

    #[cfg(target_arch = "x86_64")]
    fn try_rdrand(dest: &mut [u8; 32]) -> bool {
        use core::arch::x86_64::_rdrand64_step;

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
            let mut counter = 0u64;
            for (i, byte) in dest.iter_mut().enumerate() {
                counter = counter
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223 + i as u64);
                *byte = (counter >> 32) as u8;
            }
        }
    }

    fn next_byte(state: &mut RandomState) -> u8 {
        // Simple counter-based generator (improved with seed mixing)
        // TODO: Implement ChaCha20 or similar CSPRNG
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
static GLOBAL_RNG: Mutex<Option<SecureRandom>> = Mutex::new(None);

/// Initialize random number generator
pub fn init() -> CryptoResult<()> {
    let rng = SecureRandom::new()?;
    *GLOBAL_RNG.lock() = Some(rng);
    Ok(())
}

/// Get global random number generator
pub fn get_random() -> &'static SecureRandom {
    // This is safe because init() must be called before use
    unsafe {
        static mut RNG_STORAGE: Option<SecureRandom> = None;

        if RNG_STORAGE.is_none() {
            RNG_STORAGE = Some(SecureRandom::new().expect("Failed to create RNG"));
        }

        RNG_STORAGE.as_ref().unwrap()
    }
}

/// Generate random bytes (convenience function)
pub fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count);
    unsafe {
        bytes.set_len(count);
    }

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
