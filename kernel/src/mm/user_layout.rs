//! The user address-space layout, defined once for each architecture.
//!
//! User space is `[USER_SPACE_START, user_space_end())`. The limit is what
//! each architecture's paging allows:
//!
//! - **x86_64** (4-level paging): canonical user addresses end at
//!   `0x0000_8000_0000_0000`, and Linux additionally keeps the top page
//!   unmapped (`TASK_SIZE_MAX = 0x7FFF_FFFF_F000`). On Intel CPUs a SYSCALL
//!   instruction in that page makes SYSRET return to a non-canonical address,
//!   which faults in ring 0 on the user's stack; AMD Ryzen parts speculate past
//!   the end of canonical space when executing there. This kernel returns to
//!   user mode with SYSRETQ, so the page is excluded.
//! - **AArch64** (48-bit VA, 4 KiB granule): TTBR0 covers `[0,
//!   0x0001_0000_0000_0000)`; x86_64's limit is used, a subset of it.
//! - **RISC-V**: the paging mode's user half less the top page, set at boot
//!   ([`set_riscv_paging_levels`]): `0x3F_FFFF_F000` under Sv39, x86_64's limit
//!   under Sv48. What the kernel places stays in Sv39's range on every hart, as
//!   Linux keeps hint-less mappings there, and a program on a Sv48 hart can
//!   still ask for more (ADR 0018).
//!
//! Where things go, in each architecture's own range and in the same order
//! (x86_64's): the mmap area from [`MMAP_BASE`] up to [`PIE_BASE`], a PIE at
//! [`PIE_BASE`] with brk after it, thread stacks below [`THREAD_STACKS_TOP`],
//! the main stack below [`STACK_TOP`].
//!
//! The first page is never mappable either (`USER_SPACE_START`), so a NULL
//! pointer plus a small offset always faults.

/// Lowest mappable user address.
pub const USER_SPACE_START: usize = 0x1000;

/// x86_64 and AArch64. Both layouts are compiled everywhere so host tests
/// check each one; `arch` picks the target's.
#[cfg_attr(target_arch = "riscv64", allow(dead_code))]
mod x86_64_aarch64 {
    /// First address past user space (exclusive end).
    pub const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_F000;
    /// The main stack's top (it grows down).
    pub const STACK_TOP: usize = 0x0000_7FFF_FFFF_0000;
    /// Thread stacks are placed downward from here.
    pub const THREAD_STACKS_TOP: usize = 0x0000_7FFE_0000_0000;
    /// The mmap area's start (the cursor grows up).
    pub const MMAP_BASE: usize = 0x0000_4000_0000_0000;
    /// Where an ET_DYN program goes: Linux's `ELF_ET_DYN_BASE` on x86_64,
    /// without randomisation. Also the mmap area's end.
    pub const PIE_BASE: usize = 0x0000_5555_5555_4000;
    /// The heap's start before a program sets it.
    pub const HEAP_BASE: usize = 0x0000_2000_0000_0000;
    /// Where the legacy loader (`userspace::enhanced_loader`) puts an
    /// interpreter; exec places it in the mmap area (ADR 0010).
    pub const LEGACY_INTERP_BASE: usize = 0x0000_7F00_0000_0000;
}

/// RISC-V, on Sv39 and Sv48 alike (ADR 0018).
#[cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]
mod riscv64 {
    /// First address past user space under Sv39: its user half less the
    /// top page.
    pub const SV39_USER_SPACE_END: usize = 0x0000_003F_FFFF_F000;
    /// The same under Sv48 (x86_64's limit).
    pub const SV48_USER_SPACE_END: usize = 0x0000_7FFF_FFFF_F000;
    /// The main stack's top (it grows down).
    pub const STACK_TOP: usize = 0x0000_003F_FFFF_0000;
    /// Thread stacks are placed downward from here.
    pub const THREAD_STACKS_TOP: usize = 0x0000_003E_0000_0000;
    /// The mmap area's start (the cursor grows up).
    pub const MMAP_BASE: usize = 0x0000_0010_0000_0000;
    /// Where an ET_DYN program goes: Linux's `ELF_ET_DYN_BASE` for Sv39
    /// (two thirds of the user half), page-aligned. Also the mmap area's end.
    pub const PIE_BASE: usize = 0x0000_002A_AAAA_A000;
    /// The heap's start before a program sets it.
    pub const HEAP_BASE: usize = 0x0000_0008_0000_0000;
    /// Where the legacy loader (`userspace::enhanced_loader`) puts an
    /// interpreter; exec places it in the mmap area (ADR 0010).
    pub const LEGACY_INTERP_BASE: usize = 0x0000_003D_0000_0000;
}

pub use arch::*;
#[cfg(target_arch = "riscv64")]
use riscv64 as arch;
#[cfg(not(target_arch = "riscv64"))]
use x86_64_aarch64 as arch;

/// First address past user space (exclusive end).
#[cfg(not(target_arch = "riscv64"))]
#[inline]
pub fn user_space_end() -> usize {
    USER_SPACE_END
}

/// The RISC-V limit for the paging mode boot turned on; Sv39's (the
/// smallest) until then.
#[cfg(target_arch = "riscv64")]
static USER_END: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(SV39_USER_SPACE_END);

/// First address past user space (exclusive end): the paging mode's.
#[cfg(target_arch = "riscv64")]
#[inline]
pub fn user_space_end() -> usize {
    USER_END.load(core::sync::atomic::Ordering::Relaxed)
}

/// Set the RISC-V user limit for `levels`-level paging (3: Sv39, 4: Sv48),
/// once, before the first user program.
#[cfg(target_arch = "riscv64")]
pub fn set_riscv_paging_levels(levels: u32) {
    let end = if levels >= 4 {
        SV48_USER_SPACE_END
    } else {
        SV39_USER_SPACE_END
    };
    USER_END.store(end, core::sync::atomic::Ordering::Relaxed);
}

/// The mmap area's end: the cursor never passes the PIE base (ADR 0018).
pub const MMAP_END: usize = PIE_BASE;

/// A user address no mapping uses during boot, for tests that need a
/// fault: 4 GiB below the main stack's top, in the 8 GiB gap between it
/// and the thread stacks.
pub const UNMAPPED_TEST_ADDR: usize = (STACK_TOP - (1 << 32)) & !0xFFF;

/// Whether `[start, start + len)` lies entirely within user space. An empty
/// range at a user address counts as inside. Overflow is rejected.
pub fn is_user_range(start: usize, len: usize) -> bool {
    start >= USER_SPACE_START
        && start
            .checked_add(len)
            .is_some_and(|end| end <= user_space_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_range_bounds() {
        let end = user_space_end();
        assert!(is_user_range(0x1000, 0x1000));
        assert!(is_user_range(end - 0x1000, 0x1000));
        // The top canonical page is excluded (SYSRET / Ryzen).
        assert!(!is_user_range(end, 1));
        assert!(!is_user_range(0x7FFF_FFFF_F000, 0x1000));
        assert!(!is_user_range(end - 0x1000, 0x1001));
        // NULL page, kernel half, overflow.
        assert!(!is_user_range(0, 16));
        assert!(!is_user_range(0xFFFF_8000_0000_0000, 0x1000));
        assert!(!is_user_range(0x1000, usize::MAX));
    }

    /// Each layout is ordered within its user space: the mmap area, the
    /// PIE, the thread stacks, the test address in the gap below the main
    /// stack, the main stack, the end. All page-aligned.
    #[test]
    fn layouts_are_ordered_and_fit() {
        macro_rules! check {
            ($m:ident, $end:ident, $limit:expr) => {{
                use super::$m::*;
                let unmapped = (STACK_TOP - (1 << 32)) & !0xFFF;
                let order = [
                    USER_SPACE_START,
                    HEAP_BASE,
                    MMAP_BASE,
                    PIE_BASE,
                    LEGACY_INTERP_BASE,
                    THREAD_STACKS_TOP,
                    unmapped,
                    STACK_TOP - 8 * 1024 * 1024,
                    STACK_TOP,
                    $end,
                ];
                assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:#x?}");
                assert!(order.iter().all(|a| a % 4096 == 0));
                assert!($end < $limit, "inside the paging mode's user half");
                // 8 GiB of thread stacks are reachable below their top.
                assert!(THREAD_STACKS_TOP - (8 << 30) > PIE_BASE);
            }};
        }
        check!(x86_64_aarch64, USER_SPACE_END, 0x0000_8000_0000_0000);
        // What the kernel places fits Sv39's user half, the smallest mode.
        check!(riscv64, SV39_USER_SPACE_END, 0x40_0000_0000);
        assert_eq!(super::riscv64::SV39_USER_SPACE_END, 0x40_0000_0000 - 0x1000);
        assert_eq!(
            super::riscv64::SV48_USER_SPACE_END,
            super::x86_64_aarch64::USER_SPACE_END
        );
    }
}
