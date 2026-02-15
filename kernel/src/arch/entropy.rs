//! Architecture-independent hardware entropy abstractions.
//!
//! Centralizes hardware entropy/random number operations so that non-arch code
//! (particularly `crypto/random.rs`) does not need scattered
//! `#[cfg(target_arch)]` blocks with inline assembly.
//!
//! # Functions
//!
//! * [`read_timestamp`] -- read the hardware cycle/timer counter for entropy
//!   jitter.
//! * [`try_hardware_rng`] -- attempt to read from a hardware RNG (RDRAND on
//!   x86_64).
//! * [`collect_timer_entropy`] -- collect 32 bytes of timer-jitter entropy.

/// Read the hardware timestamp/cycle counter.
///
/// Returns a raw counter value suitable for entropy collection via jitter
/// timing.
///
/// * **x86_64**: `RDTSC` (Time Stamp Counter).
/// * **AArch64**: `CNTVCT_EL0` (Virtual Timer Count).
/// * **RISC-V**: `rdcycle` CSR.
#[inline]
pub fn read_timestamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: _rdtsc reads the Time Stamp Counter register. It is always
        // available on x86_64 and returns the current cycle count as u64.
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[cfg(target_arch = "aarch64")]
    {
        let val: u64;
        // SAFETY: Reading CNTVCT_EL0 is a read-only operation that accesses
        // the virtual timer count register. Always safe from any exception level.
        unsafe {
            core::arch::asm!("mrs {}, cntvct_el0", out(reg) val);
        }
        val
    }

    #[cfg(target_arch = "riscv64")]
    {
        let val: u64;
        // SAFETY: Reading the cycle CSR is a read-only operation that
        // accesses a performance counter. Always safe.
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
        0u64
    }
}

/// Check whether the CPU supports the RDRAND instruction.
///
/// Queries CPUID leaf 1 and tests ECX bit 30 (RDRAND feature flag).
/// Returns `false` if CPUID is unavailable or the bit is not set.
#[cfg(target_arch = "x86_64")]
fn cpu_has_rdrand() -> bool {
    // SAFETY: CPUID with EAX=1 is a read-only, side-effect-free instruction
    // that returns CPU feature information. It is always available on x86_64
    // (CPUID support is mandatory in long mode). RBX is saved and restored
    // because LLVM reserves it as a frame pointer and CPUID clobbers it.
    let ecx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("ecx") ecx,
            out("eax") _,
            out("edx") _,
            options(nomem, preserves_flags),
        );
    }
    (ecx & (1 << 30)) != 0
}

/// Attempt to read 32 bytes from a hardware random number generator.
///
/// Returns `true` if the hardware RNG was available and `dest` was filled,
/// `false` if hardware RNG is unavailable (in which case `dest` is unmodified).
///
/// * **x86_64**: Uses `RDRAND` instruction with retry logic. Falls back to
///   `false` if the CPU does not support RDRAND (checked via CPUID).
/// * **AArch64/RISC-V**: No dedicated hardware RNG; always returns `false`.
pub fn try_hardware_rng(dest: &mut [u8; 32]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // Check CPUID before executing RDRAND. Without this guard, executing
        // RDRAND on a CPU that lacks the feature triggers #UD (Invalid Opcode),
        // which cascades to a double fault during early boot.
        if !cpu_has_rdrand() {
            return false;
        }

        use core::arch::x86_64::_rdrand64_step;

        // SAFETY: _rdrand64_step is an x86_64 RDRAND intrinsic that writes a
        // hardware-generated random u64 into `value`. Returns 0 on failure.
        // We have verified RDRAND support via CPUID above, so the instruction
        // will not fault.
        unsafe {
            for chunk in dest.chunks_exact_mut(8) {
                let mut value: u64 = 0;
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
                    return false;
                }
                chunk.copy_from_slice(&value.to_le_bytes());
            }
        }
        true
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = dest;
        false
    }
}

/// Collect 32 bytes of timer-jitter entropy.
///
/// Uses [`read_timestamp`] to sample the hardware counter with variable-work
/// delays, then mixes the jitter into the output buffer using an LCG.
pub fn collect_timer_entropy(dest: &mut [u8; 32]) {
    let mut pool = [0u64; 4];
    let mut sample = 0;
    while sample < 4 {
        let t1 = read_timestamp();
        // Introduce variable delay via computation
        let mut work: u64 = t1;
        let mut j = 0u32;
        while j < 100 + (sample as u32 * 37) {
            work = work
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            j += 1;
        }
        let t2 = read_timestamp();
        // Mix timing jitter with computation result
        pool[sample] = t1 ^ t2 ^ work;
        sample += 1;
    }

    // Convert pool to bytes
    let mut i = 0;
    while i < 32 {
        let pool_word = pool[i / 8];
        let byte_idx = i % 8;
        dest[i] = (pool_word >> (byte_idx * 8)) as u8;
        i += 1;
    }

    // Additional mixing pass using LCG
    let mut state = read_timestamp();
    i = 0;
    while i < 32 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        dest[i] ^= (state >> 33) as u8;
        i += 1;
    }
}
