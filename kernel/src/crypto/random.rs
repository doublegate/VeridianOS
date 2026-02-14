//! Secure Random Number Generation
//!
//! Provides cryptographically secure random number generation using a
//! ChaCha20-based CSPRNG seeded from hardware entropy sources.
//!
//! ## Design
//!
//! The CSPRNG uses ChaCha20 in counter mode as the core PRG:
//! - 256-bit key derived from entropy pool
//! - 96-bit nonce (fixed per reseed)
//! - 32-bit counter incremented for each 64-byte block
//!
//! Entropy sources:
//! - x86_64: RDRAND instruction (hardware RNG), TSC jitter
//! - AArch64: Timer counter (CNTVCT_EL0) jitter
//! - RISC-V: Timer counter (rdcycle) jitter
//!
//! Reseeding occurs every RESEED_INTERVAL calls to mix fresh entropy.

use alloc::{vec, vec::Vec};

use spin::Mutex;

use super::CryptoResult;
use crate::sync::once_lock::OnceLock;

/// Number of generate calls between automatic reseeds
const RESEED_INTERVAL: u64 = 4096;

/// Secure random number generator backed by ChaCha20
pub struct SecureRandom {
    state: Mutex<RandomState>,
}

struct RandomState {
    /// ChaCha20 key (256 bits)
    key: [u8; 32],
    /// ChaCha20 nonce (96 bits)
    nonce: [u8; 12],
    /// Block counter for ChaCha20
    counter: u32,
    /// Buffered keystream bytes (from the current ChaCha20 block)
    buffer: [u8; 64],
    /// Index into the buffer for the next byte to use
    buffer_pos: usize,
    /// Number of generate calls since last reseed
    reseed_counter: u64,
    /// Entropy pool for accumulating hardware entropy
    entropy_pool: [u8; 32],
    /// Entropy pool write index
    pool_idx: usize,
}

impl SecureRandom {
    /// Create new secure random number generator
    pub fn new() -> CryptoResult<Self> {
        // Initialize with hardware RNG or timer
        let seed = Self::get_entropy()?;

        // Use SHA-256 to derive initial key and nonce from entropy
        let key_material = super::hash::sha256(&seed);
        let nonce_input = {
            let mut input = [0u8; 33];
            input[..32].copy_from_slice(&seed);
            input[32] = 0x01; // domain separation
            input
        };
        let nonce_material = super::hash::sha256(&nonce_input);

        let mut key = [0u8; 32];
        key.copy_from_slice(key_material.as_bytes());

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&nonce_material.as_bytes()[..12]);

        // Generate first buffer block
        let buffer = Self::chacha20_block_static(&key, &nonce, 0);

        Ok(Self {
            state: Mutex::new(RandomState {
                key,
                nonce,
                counter: 1, // First block used for initial buffer
                buffer,
                buffer_pos: 0,
                reseed_counter: 0,
                entropy_pool: [0u8; 32],
                pool_idx: 0,
            }),
        })
    }

    /// Generate random bytes
    pub fn fill_bytes(&self, dest: &mut [u8]) -> CryptoResult<()> {
        let mut state = self.state.lock();

        // Check if reseed is needed
        state.reseed_counter += 1;
        if state.reseed_counter >= RESEED_INTERVAL {
            Self::reseed_state(&mut state);
        }

        // Use index-based loop instead of iter_mut() to avoid AArch64 LLVM hang
        let mut i = 0;
        while i < dest.len() {
            if state.buffer_pos >= 64 {
                // Generate next block
                state.buffer = Self::chacha20_block_static(&state.key, &state.nonce, state.counter);
                state.counter = state.counter.wrapping_add(1);
                state.buffer_pos = 0;

                // Every 256 blocks, mix entropy into the state
                if state.counter.is_multiple_of(256) {
                    Self::mix_entropy_into_key(&mut state);
                }
            }
            dest[i] = state.buffer[state.buffer_pos];
            state.buffer_pos += 1;
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

    // ========================================================================
    // Private methods
    // ========================================================================

    /// ChaCha20 quarter round
    #[inline]
    fn qr(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        state[a] = state[a].wrapping_add(state[b]);
        state[d] ^= state[a];
        state[d] = state[d].rotate_left(16);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] ^= state[c];
        state[b] = state[b].rotate_left(12);

        state[a] = state[a].wrapping_add(state[b]);
        state[d] ^= state[a];
        state[d] = state[d].rotate_left(8);

        state[c] = state[c].wrapping_add(state[d]);
        state[b] ^= state[c];
        state[b] = state[b].rotate_left(7);
    }

    /// ChaCha20 block function (static method for use without &self)
    fn chacha20_block_static(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> [u8; 64] {
        let mut state: [u32; 16] = [
            0x61707865,
            0x3320646e,
            0x79622d32,
            0x6b206574, // "expand 32-byte k"
            u32::from_le_bytes([key[0], key[1], key[2], key[3]]),
            u32::from_le_bytes([key[4], key[5], key[6], key[7]]),
            u32::from_le_bytes([key[8], key[9], key[10], key[11]]),
            u32::from_le_bytes([key[12], key[13], key[14], key[15]]),
            u32::from_le_bytes([key[16], key[17], key[18], key[19]]),
            u32::from_le_bytes([key[20], key[21], key[22], key[23]]),
            u32::from_le_bytes([key[24], key[25], key[26], key[27]]),
            u32::from_le_bytes([key[28], key[29], key[30], key[31]]),
            counter,
            u32::from_le_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]),
            u32::from_le_bytes([nonce[4], nonce[5], nonce[6], nonce[7]]),
            u32::from_le_bytes([nonce[8], nonce[9], nonce[10], nonce[11]]),
        ];

        let initial_state = state;

        // 20 rounds (10 double rounds) per RFC 8439
        let mut round = 0;
        while round < 10 {
            // Column rounds
            Self::qr(&mut state, 0, 4, 8, 12);
            Self::qr(&mut state, 1, 5, 9, 13);
            Self::qr(&mut state, 2, 6, 10, 14);
            Self::qr(&mut state, 3, 7, 11, 15);
            // Diagonal rounds
            Self::qr(&mut state, 0, 5, 10, 15);
            Self::qr(&mut state, 1, 6, 11, 12);
            Self::qr(&mut state, 2, 7, 8, 13);
            Self::qr(&mut state, 3, 4, 9, 14);
            round += 1;
        }

        // Add initial state
        let mut i = 0;
        while i < 16 {
            state[i] = state[i].wrapping_add(initial_state[i]);
            i += 1;
        }

        // Serialize to bytes (little-endian)
        let mut output = [0u8; 64];
        i = 0;
        while i < 16 {
            let bytes = state[i].to_le_bytes();
            output[i * 4] = bytes[0];
            output[i * 4 + 1] = bytes[1];
            output[i * 4 + 2] = bytes[2];
            output[i * 4 + 3] = bytes[3];
            i += 1;
        }
        output
    }

    /// Collect entropy from hardware sources
    fn get_entropy() -> CryptoResult<[u8; 32]> {
        let mut entropy = [0u8; 32];

        #[cfg(target_arch = "x86_64")]
        {
            // Try RDRAND first for high-quality hardware entropy
            if Self::try_rdrand(&mut entropy) {
                return Ok(entropy);
            }
            // Fall back to TSC-based entropy
            Self::timer_entropy(&mut entropy);
        }

        #[cfg(target_arch = "aarch64")]
        {
            Self::aarch64_timer_entropy(&mut entropy);
        }

        #[cfg(target_arch = "riscv64")]
        {
            Self::riscv_timer_entropy(&mut entropy);
        }

        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )))]
        {
            Self::fallback_entropy(&mut entropy);
        }

        Ok(entropy)
    }

    /// Try to use RDRAND instruction for hardware entropy (x86_64)
    #[cfg(target_arch = "x86_64")]
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
                // Try up to 10 times per word (RDRAND can transiently fail)
                let mut attempts = 0;
                let mut success = false;
                while attempts < 10 {
                    if _rdrand64_step(&mut value) != 0 {
                        success = true;
                        break;
                    }
                    attempts += 1;
                }
                if !success {
                    return false; // RDRAND not available or consistently
                                  // failing
                }
                chunk.copy_from_slice(&value.to_le_bytes());
            }
        }

        true
    }

    /// TSC-based timer entropy for x86_64
    #[cfg(target_arch = "x86_64")]
    fn timer_entropy(dest: &mut [u8; 32]) {
        // SAFETY: _rdtsc reads the Time Stamp Counter register. It is always
        // available on x86_64 and returns the current cycle count as u64.
        unsafe {
            use core::arch::x86_64::_rdtsc;

            // Collect multiple TSC samples with varying delays for jitter
            let mut pool = [0u64; 4];
            let mut sample = 0;
            while sample < 4 {
                let t1 = _rdtsc();
                // Introduce variable delay via computation
                let mut work: u64 = t1;
                let mut j = 0u32;
                while j < 100 + (sample as u32 * 37) {
                    work = work
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    j += 1;
                }
                let t2 = _rdtsc();
                // Mix timing jitter with computation result
                pool[sample] = t1 ^ t2 ^ work;
                sample += 1;
            }

            // Hash the pool to produce uniform output
            let mut i = 0;
            while i < 32 {
                let pool_word = pool[i / 8];
                let byte_idx = i % 8;
                dest[i] = (pool_word >> (byte_idx * 8)) as u8;
                i += 1;
            }

            // Additional mixing pass using LCG
            let mut state = _rdtsc();
            i = 0;
            while i < 32 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                dest[i] ^= (state >> 33) as u8;
                i += 1;
            }
        }
    }

    /// Timer-based entropy for AArch64
    #[cfg(target_arch = "aarch64")]
    fn aarch64_timer_entropy(dest: &mut [u8; 32]) {
        // Read CNTVCT_EL0 (virtual timer counter) for entropy
        let read_timer = || -> u64 {
            let val: u64;
            // SAFETY: Reading CNTVCT_EL0 is a read-only operation that accesses
            // the virtual timer count register. This is always safe to read from
            // any exception level.
            unsafe {
                core::arch::asm!("mrs {}, cntvct_el0", out(reg) val);
            }
            val
        };

        let mut pool = [0u64; 4];
        let mut sample = 0;
        while sample < 4 {
            let t1 = read_timer();
            let mut work: u64 = t1;
            let mut j = 0u32;
            while j < 100 + (sample as u32 * 37) {
                work = work
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                j += 1;
            }
            let t2 = read_timer();
            pool[sample] = t1 ^ t2 ^ work;
            sample += 1;
        }

        let mut i = 0;
        while i < 32 {
            let pool_word = pool[i / 8];
            let byte_idx = i % 8;
            dest[i] = (pool_word >> (byte_idx * 8)) as u8;
            i += 1;
        }

        // Additional mixing
        let mut state = read_timer();
        i = 0;
        while i < 32 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            dest[i] ^= (state >> 33) as u8;
            i += 1;
        }
    }

    /// Timer-based entropy for RISC-V
    #[cfg(target_arch = "riscv64")]
    fn riscv_timer_entropy(dest: &mut [u8; 32]) {
        // Read rdcycle CSR for entropy
        let read_cycle = || -> u64 {
            let val: u64;
            // SAFETY: Reading the cycle CSR is a read-only operation that
            // accesses a performance counter. This is always safe.
            unsafe {
                core::arch::asm!("rdcycle {}", out(reg) val);
            }
            val
        };

        let mut pool = [0u64; 4];
        let mut sample = 0;
        while sample < 4 {
            let t1 = read_cycle();
            let mut work: u64 = t1;
            let mut j = 0u32;
            while j < 100 + (sample as u32 * 37) {
                work = work
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                j += 1;
            }
            let t2 = read_cycle();
            pool[sample] = t1 ^ t2 ^ work;
            sample += 1;
        }

        let mut i = 0;
        while i < 32 {
            let pool_word = pool[i / 8];
            let byte_idx = i % 8;
            dest[i] = (pool_word >> (byte_idx * 8)) as u8;
            i += 1;
        }

        // Additional mixing
        let mut state = read_cycle();
        i = 0;
        while i < 32 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            dest[i] ^= (state >> 33) as u8;
            i += 1;
        }
    }

    /// Fallback entropy source (for architectures without specific support)
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    fn fallback_entropy(dest: &mut [u8; 32]) {
        // Use a counter-based approach with stack address mixing
        let stack_var: u64 = 0;
        let stack_addr = &stack_var as *const u64 as u64;
        let mut counter: u64 = stack_addr;
        let mut i = 0;
        while i < 32 {
            counter = counter
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407 + i as u64);
            dest[i] = (counter >> 33) as u8;
            i += 1;
        }
    }

    /// Reseed the CSPRNG state with fresh entropy
    fn reseed_state(state: &mut RandomState) {
        state.reseed_counter = 0;

        // Collect fresh entropy
        let mut fresh_entropy = [0u8; 32];

        #[cfg(target_arch = "x86_64")]
        {
            if !Self::try_rdrand(&mut fresh_entropy) {
                Self::timer_entropy(&mut fresh_entropy);
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            Self::aarch64_timer_entropy(&mut fresh_entropy);
        }

        #[cfg(target_arch = "riscv64")]
        {
            Self::riscv_timer_entropy(&mut fresh_entropy);
        }

        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )))]
        {
            Self::fallback_entropy(&mut fresh_entropy);
        }

        // Mix fresh entropy with current key using SHA-256
        let mut mix_input = [0u8; 64];
        mix_input[..32].copy_from_slice(&state.key);
        mix_input[32..].copy_from_slice(&fresh_entropy);
        let new_key = super::hash::sha256(&mix_input);
        state.key.copy_from_slice(new_key.as_bytes());

        // Derive new nonce from counter and fresh entropy
        let mut nonce_input = [0u8; 44];
        nonce_input[..32].copy_from_slice(&fresh_entropy);
        nonce_input[32..36].copy_from_slice(&state.counter.to_le_bytes());
        nonce_input[36..44].copy_from_slice(&state.reseed_counter.to_le_bytes());
        let nonce_hash = super::hash::sha256(&nonce_input);
        state.nonce.copy_from_slice(&nonce_hash.as_bytes()[..12]);

        // Reset counter and buffer
        state.counter = 0;
        state.buffer = Self::chacha20_block_static(&state.key, &state.nonce, 0);
        state.counter = 1;
        state.buffer_pos = 0;
    }

    /// Mix accumulated entropy pool into the key
    fn mix_entropy_into_key(state: &mut RandomState) {
        // Collect a fresh entropy sample into the pool
        let sample = {
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: _rdtsc reads the Time Stamp Counter, always safe on x86_64
                unsafe { core::arch::x86_64::_rdtsc() }
            }
            #[cfg(target_arch = "aarch64")]
            {
                let val: u64;
                // SAFETY: Reading CNTVCT_EL0 is always safe
                unsafe {
                    core::arch::asm!("mrs {}, cntvct_el0", out(reg) val);
                }
                val
            }
            #[cfg(target_arch = "riscv64")]
            {
                let val: u64;
                // SAFETY: Reading cycle CSR is always safe
                unsafe {
                    core::arch::asm!("rdcycle {}", out(reg) val);
                }
                val
            }
            #[cfg(not(any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "riscv64"
            )))]
            {
                state.counter as u64
            }
        };

        // Fold sample into entropy pool
        let sample_bytes = sample.to_le_bytes();
        let mut j = 0;
        while j < 8 {
            state.entropy_pool[(state.pool_idx + j) % 32] ^= sample_bytes[j];
            j += 1;
        }
        state.pool_idx = (state.pool_idx + 8) % 32;

        // Periodically mix pool into key (every 1024 blocks)
        if state.counter.is_multiple_of(1024) {
            let mut mix = [0u8; 64];
            mix[..32].copy_from_slice(&state.key);
            mix[32..].copy_from_slice(&state.entropy_pool);
            let new_key = super::hash::sha256(&mix);
            state.key.copy_from_slice(new_key.as_bytes());
        }
    }
}

impl Default for SecureRandom {
    fn default() -> Self {
        Self::new().expect("Failed to initialize SecureRandom")
    }
}

/// Global secure random number generator
static RNG_STORAGE: OnceLock<SecureRandom> = OnceLock::new();

/// Initialize random number generator
pub fn init() -> CryptoResult<()> {
    let rng = SecureRandom::new()?;
    let _ = RNG_STORAGE.set(rng);
    Ok(())
}

/// Get global random number generator
pub fn get_random() -> &'static SecureRandom {
    RNG_STORAGE.get_or_init(|| SecureRandom::new().expect("Failed to create RNG"))
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
