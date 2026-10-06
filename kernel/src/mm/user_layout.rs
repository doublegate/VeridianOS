//! The user address-space limit, defined once for all architectures.
//!
//! User space is `[0, USER_SPACE_END)`. One value serves every architecture
//! because it is the conservative intersection of what each one requires:
//!
//! - **x86_64** (4-level paging): canonical user addresses end at
//!   `0x0000_8000_0000_0000`, and Linux additionally keeps the top page
//!   unmapped (`TASK_SIZE_MAX = 0x7FFF_FFFF_F000`). On Intel CPUs a SYSCALL
//!   instruction in that page makes SYSRET return to a non-canonical address,
//!   which faults in ring 0 on the user's stack; AMD Ryzen parts speculate past
//!   the end of canonical space when executing there. This kernel returns to
//!   user mode with SYSRETQ, so the page is excluded.
//! - **AArch64** (48-bit VA, 4 KiB granule): TTBR0 covers `[0,
//!   0x0001_0000_0000_0000)`; the limit below is a subset of it.
//! - **RISC-V Sv48** (satp MODE 9): user addresses are `[0,
//!   0x0000_8000_0000_0000)`. Sv39 would end user space at `0x40_0000_0000`,
//!   below the stacks this kernel places, so the kernel's 4-level tables are
//!   used with Sv48 only.
//!
//! The first page is never mappable either (`USER_SPACE_START`), so a NULL
//! pointer plus a small offset always faults.

/// Lowest mappable user address.
pub const USER_SPACE_START: usize = 0x1000;

/// First address past user space (exclusive end).
pub const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_F000;

/// Whether `[start, start + len)` lies entirely within user space. An empty
/// range at a user address counts as inside. Overflow is rejected.
pub fn is_user_range(start: usize, len: usize) -> bool {
    start >= USER_SPACE_START
        && start
            .checked_add(len)
            .is_some_and(|end| end <= USER_SPACE_END)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_range_bounds() {
        assert!(is_user_range(0x1000, 0x1000));
        assert!(is_user_range(USER_SPACE_END - 0x1000, 0x1000));
        // The top canonical page is excluded (SYSRET / Ryzen).
        assert!(!is_user_range(USER_SPACE_END, 1));
        assert!(!is_user_range(0x7FFF_FFFF_F000, 0x1000));
        assert!(!is_user_range(USER_SPACE_END - 0x1000, 0x1001));
        // NULL page, kernel half, overflow.
        assert!(!is_user_range(0, 16));
        assert!(!is_user_range(0xFFFF_8000_0000_0000, 0x1000));
        assert!(!is_user_range(0x1000, usize::MAX));
    }
}
