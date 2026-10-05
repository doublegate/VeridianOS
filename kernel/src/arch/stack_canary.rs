//! Boot-stack overflow detection for AArch64 and RISC-V.
//!
//! Both architectures run boot on a linker-reserved stack placed directly
//! after .bss, with the MMU off -- so there is no guard page, and an
//! overflow silently overwrites the kernel statics just below the stack
//! (on riscv64 it zeroed the global allocator and other singletons, N-13).
//! A canary block at the stack's lowest addresses turns that into a
//! detectable failure: `install` fills it at entry and `intact` checks it.

/// Pattern written to the canary words.
const PATTERN: u64 = 0x5EED_CA9A_57AC_C0DE;
/// Canary size in 64-bit words (512 bytes at the bottom of the stack).
const WORDS: usize = 64;

extern "C" {
    static __stack_bottom: u8;
}

fn canary_base() -> *mut u64 {
    core::ptr::addr_of!(__stack_bottom) as *mut u64
}

/// Fill the canary. Call once at entry, while the stack pointer is far above
/// the bottom of the stack.
pub(crate) fn install() {
    let base = canary_base();
    let mut i = 0;
    // Plain loop: iterator code is unreliable this early on AArch64.
    while i < WORDS {
        // SAFETY: [__stack_bottom, __stack_bottom + 512) is reserved stack
        // memory (linker script), 8-aligned, and unused at entry.
        unsafe { core::ptr::write_volatile(base.add(i), PATTERN) };
        i += 1;
    }
}

/// Whether the canary is unchanged, i.e. the boot stack has never grown
/// into its last 512 bytes.
pub(crate) fn intact() -> bool {
    let base = canary_base();
    let mut i = 0;
    while i < WORDS {
        // SAFETY: as in `install`; reading reserved stack memory.
        if unsafe { core::ptr::read_volatile(base.add(i)) } != PATTERN {
            return false;
        }
        i += 1;
    }
    true
}
