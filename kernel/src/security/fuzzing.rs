//! In-kernel fuzzing infrastructure
//!
//! Provides mutation-based fuzzing for security-critical kernel subsystems.
//! This is a no_std implementation since cargo-fuzz requires std.

use core::sync::atomic::{AtomicU64, Ordering};

/// Fuzzing target trait - implement for each subsystem to fuzz
pub trait FuzzTarget {
    /// Name of the fuzz target
    fn name(&self) -> &'static str;

    /// Run one fuzzing iteration with the given input data
    fn fuzz(&self, data: &[u8]);

    /// Reset state between iterations if needed
    fn reset(&self) {}
}

/// Fuzz runner configuration
pub struct FuzzConfig {
    /// Maximum input size in bytes
    pub max_input_size: usize,
    /// Number of iterations to run
    pub max_iterations: u64,
    /// Seed for the PRNG
    pub seed: u64,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            max_input_size: 4096,
            max_iterations: 10_000,
            seed: 0xDEAD_BEEF_CAFE_BABE,
        }
    }
}

/// Simple PRNG for mutation (xorshift64)
struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_range(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        (self.next() as usize) % max
    }
}

/// Mutation strategies for input generation
enum Mutation {
    /// Flip random bits
    BitFlip,
    /// Replace byte with random value
    ByteReplace,
    /// Insert random bytes
    ByteInsert,
    /// Delete random bytes
    ByteDelete,
    /// Replace with interesting values (0, 0xFF, boundaries)
    InterestingValues,
}

/// Mutate input data in-place
fn mutate(data: &mut [u8], len: &mut usize, max_size: usize, rng: &mut FuzzRng) {
    if *len == 0 {
        // Generate initial random data
        *len = core::cmp::min(rng.next_range(64) + 1, max_size);
        let mut i = 0;
        while i < *len {
            data[i] = rng.next() as u8;
            i += 1;
        }
        return;
    }

    let mutation = match rng.next_range(5) {
        0 => Mutation::BitFlip,
        1 => Mutation::ByteReplace,
        2 => Mutation::ByteInsert,
        3 => Mutation::ByteDelete,
        _ => Mutation::InterestingValues,
    };

    match mutation {
        Mutation::BitFlip => {
            if *len > 0 {
                let pos = rng.next_range(*len);
                let bit = rng.next_range(8);
                data[pos] ^= 1u8 << bit;
            }
        }
        Mutation::ByteReplace => {
            if *len > 0 {
                let pos = rng.next_range(*len);
                data[pos] = rng.next() as u8;
            }
        }
        Mutation::ByteInsert => {
            if *len < max_size {
                let pos = rng.next_range(*len + 1);
                // Shift bytes right
                let mut i = *len;
                while i > pos {
                    data[i] = data[i - 1];
                    i -= 1;
                }
                data[pos] = rng.next() as u8;
                *len += 1;
            }
        }
        Mutation::ByteDelete => {
            if *len > 1 {
                let pos = rng.next_range(*len);
                let mut i = pos;
                while i < *len - 1 {
                    data[i] = data[i + 1];
                    i += 1;
                }
                *len -= 1;
            }
        }
        Mutation::InterestingValues => {
            if *len > 0 {
                let pos = rng.next_range(*len);
                let interesting: [u8; 8] = [0x00, 0xFF, 0x7F, 0x80, 0x01, 0xFE, 0x41, 0x00];
                data[pos] = interesting[rng.next_range(interesting.len())];
            }
        }
    }
}

/// Fuzzing statistics
pub struct FuzzStats {
    pub iterations: AtomicU64,
    pub crashes: AtomicU64,
    pub unique_crashes: AtomicU64,
}

impl FuzzStats {
    const fn new() -> Self {
        Self {
            iterations: AtomicU64::new(0),
            crashes: AtomicU64::new(0),
            unique_crashes: AtomicU64::new(0),
        }
    }
}

static FUZZ_STATS: FuzzStats = FuzzStats::new();

/// Run the fuzzer on a target
pub fn run_fuzz_target(target: &dyn FuzzTarget, config: &FuzzConfig) -> &'static FuzzStats {
    let mut rng = FuzzRng::new(config.seed);
    let max_size = core::cmp::min(config.max_input_size, 8192);

    // Stack-allocated input buffer (limited to 8KB for kernel stack safety)
    let mut input_buf = [0u8; 8192];
    let mut input_len: usize = 0;

    let mut iteration = 0u64;
    while iteration < config.max_iterations {
        // Mutate input
        mutate(&mut input_buf, &mut input_len, max_size, &mut rng);

        // Run target (panics are caught by kernel panic handler)
        target.fuzz(&input_buf[..input_len]);

        FUZZ_STATS.iterations.fetch_add(1, Ordering::Relaxed);
        target.reset();

        iteration += 1;
    }

    crate::println!(
        "[FUZZ] {} completed: {} iterations, {} crashes",
        target.name(),
        FUZZ_STATS.iterations.load(Ordering::Relaxed),
        FUZZ_STATS.crashes.load(Ordering::Relaxed),
    );

    &FUZZ_STATS
}

// ============================================================================
// Built-in fuzz targets
// ============================================================================

/// ELF parser fuzz target
pub struct ElfParserTarget;

impl FuzzTarget for ElfParserTarget {
    fn name(&self) -> &'static str {
        "elf_parser"
    }

    fn fuzz(&self, data: &[u8]) {
        // Attempt to parse arbitrary bytes as ELF
        if data.len() >= 64 {
            let loader = crate::elf::ElfLoader::new();
            let _ = loader.parse(data);
        }
    }
}

/// Capability token fuzz target
pub struct CapabilityTokenTarget;

impl FuzzTarget for CapabilityTokenTarget {
    fn name(&self) -> &'static str {
        "capability_token"
    }

    fn fuzz(&self, data: &[u8]) {
        if data.len() >= 8 {
            let value = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            let token = crate::cap::CapabilityToken::from_u64(value);

            // Exercise token operations
            let _ = token.id();
            let _ = token.generation();
            let _ = token.cap_type();
            let _ = token.to_u64();

            // Check validity (should not panic)
            let _ = crate::cap::manager::cap_manager().is_valid(token);
        }
    }
}

/// IPC message fuzz target
pub struct IpcMessageTarget;

impl FuzzTarget for IpcMessageTarget {
    fn name(&self) -> &'static str {
        "ipc_message"
    }

    fn fuzz(&self, data: &[u8]) {
        if data.len() >= core::mem::size_of::<crate::ipc::SmallMessage>() {
            // SAFETY: SmallMessage is Copy and repr(C). We read from a byte
            // buffer that is at least as large as SmallMessage. The data may
            // contain garbage values but SmallMessage fields are all primitive
            // types (u64) that accept any bit pattern.
            let msg = unsafe {
                core::ptr::read_unaligned(data.as_ptr() as *const crate::ipc::SmallMessage)
            };

            // Exercise message operations (should not panic)
            let _ = msg.data;
        }
    }
}

/// Syscall number fuzz target
pub struct SyscallTarget;

impl FuzzTarget for SyscallTarget {
    fn name(&self) -> &'static str {
        "syscall_dispatch"
    }

    fn fuzz(&self, data: &[u8]) {
        if data.len() >= 2 {
            let syscall_num = u16::from_le_bytes([data[0], data[1]]) as usize;
            // Only test the dispatch path, not actual execution
            let _ = crate::syscall::Syscall::try_from(syscall_num);
        }
    }
}

/// Get fuzzing statistics
pub fn stats() -> &'static FuzzStats {
    &FUZZ_STATS
}

/// Record a crash (called from panic handler hook)
pub fn record_crash() {
    FUZZ_STATS.crashes.fetch_add(1, Ordering::Relaxed);
}
