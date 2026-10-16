//! Page-table entry formats.
//!
//! The walker (`page_table.rs`) works in the architecture-neutral
//! [`PageFlags`] and asks an entry format to encode and decode: the x86_64
//! layout (also used, for now, by AArch64, which runs with the MMU off,
//! N-28), and the RISC-V Sv39/Sv48 layout (HR-10). Both are plain functions
//! of the raw `u64` so that the host tests cover every format whatever the
//! build target.
//!
//! The formats differ in ways the walker has to respect:
//! - A pointer to the next-level table: x86_64 sets PRESENT, WRITABLE and, on
//!   the path to a user page, USER (the CPU checks every level). RISC-V sets
//!   only V; U, A and D are reserved in a non-leaf entry.
//! - A large-page leaf: x86_64 marks it with PS (`HUGE`); on RISC-V any entry
//!   with R, W or X set is a leaf, at whatever level.

use super::{FrameNumber, PageFlags};

/// x86_64 (4-level, 4 KiB pages): the flag bits are [`PageFlags`]' own.
pub mod x86 {
    use super::{FrameNumber, PageFlags};

    const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

    /// A leaf (4 KiB, or a large page when `flags` has `HUGE`).
    pub const fn leaf(frame: FrameNumber, flags: PageFlags) -> u64 {
        (frame.as_u64() << 12) | flags.bits()
    }

    /// A pointer to the next-level table, USER on the path to a user page.
    pub const fn table(frame: FrameNumber, user: bool) -> u64 {
        let mut e = (frame.as_u64() << 12) | PageFlags::PRESENT.bits() | PageFlags::WRITABLE.bits();
        if user {
            e |= PageFlags::USER.bits();
        }
        e
    }

    pub const fn is_present(entry: u64) -> bool {
        entry & PageFlags::PRESENT.bits() != 0
    }

    /// A large-page leaf (PS), for an entry above the last level.
    pub const fn is_huge(entry: u64) -> bool {
        is_present(entry) && entry & PageFlags::HUGE.bits() != 0
    }

    pub const fn frame(entry: u64) -> Option<FrameNumber> {
        if is_present(entry) {
            Some(FrameNumber::new((entry & ADDR_MASK) >> 12))
        } else {
            None
        }
    }

    /// The low flag bits and NX (bit 63). NX used to be dropped, so a fork
    /// re-mapped the child's data and stack pages executable.
    pub const fn flags(entry: u64) -> PageFlags {
        PageFlags::from_bits(entry & (0xFFF | PageFlags::NO_EXECUTE.bits()))
    }

    /// Add user access to a table entry on the path to a user page.
    pub const fn grant_user_table(entry: u64) -> u64 {
        if is_present(entry) {
            entry | PageFlags::USER.bits()
        } else {
            entry
        }
    }
}

/// RISC-V Sv39/Sv48/Sv57 (privileged spec, "Sv39"): V R W X U G A D in
/// bits 0-7, two software bits (RSW) in 8-9, the PPN from bit 10.
pub mod riscv {
    use super::{FrameNumber, PageFlags};

    pub const V: u64 = 1 << 0;
    pub const R: u64 = 1 << 1;
    pub const W: u64 = 1 << 2;
    pub const X: u64 = 1 << 3;
    pub const U: u64 = 1 << 4;
    pub const G: u64 = 1 << 5;
    pub const A: u64 = 1 << 6;
    pub const D: u64 = 1 << 7;
    /// RSW bit 8: copy-on-write (`PageFlags::COW`).
    pub const SW_COW: u64 = 1 << 8;
    /// RSW bit 9: `PageFlags::NO_CACHE`, kept so that the flag survives a
    /// round trip. The memory type itself needs Svpbmt (HR-11).
    pub const SW_NO_CACHE: u64 = 1 << 9;
    const PPN_SHIFT: u32 = 10;
    /// 44-bit PPN (bits 10-53).
    const PPN_MASK: u64 = (1 << 44) - 1;

    /// A leaf at any level (the level makes it 4 KiB, 2 MiB or 1 GiB, so
    /// `HUGE` is ignored). Readable whenever present (W without R is a
    /// reserved encoding); A and D are set up front, since updating them in
    /// hardware is optional (Svadu) and a clear bit would fault on access.
    pub const fn leaf(frame: FrameNumber, flags: PageFlags) -> u64 {
        if !flags.has(PageFlags::PRESENT) {
            return 0;
        }
        let mut e = (frame.as_u64() & PPN_MASK) << PPN_SHIFT | V | R | A | D;
        if flags.has(PageFlags::WRITABLE) {
            e |= W;
        }
        if !flags.has(PageFlags::NO_EXECUTE) {
            e |= X;
        }
        if flags.has(PageFlags::USER) {
            e |= U;
        }
        if flags.has(PageFlags::GLOBAL) {
            e |= G;
        }
        if flags.has(PageFlags::COW) {
            e |= SW_COW;
        }
        if flags.has(PageFlags::NO_CACHE) {
            e |= SW_NO_CACHE;
        }
        e
    }

    /// A pointer to the next-level table: V only (U, A, D are reserved in a
    /// non-leaf entry), whether or not the path leads to a user page.
    pub const fn table(frame: FrameNumber, user: bool) -> u64 {
        let _ = user;
        (frame.as_u64() & PPN_MASK) << PPN_SHIFT | V
    }

    pub const fn is_present(entry: u64) -> bool {
        entry & V != 0
    }

    /// A leaf above the last level (a large page): any of R, W, X.
    pub const fn is_huge(entry: u64) -> bool {
        is_present(entry) && entry & (R | W | X) != 0
    }

    pub const fn frame(entry: u64) -> Option<FrameNumber> {
        if is_present(entry) {
            Some(FrameNumber::new((entry >> PPN_SHIFT) & PPN_MASK))
        } else {
            None
        }
    }

    /// The entry's permissions as [`PageFlags`] (never `HUGE`: that is the
    /// level, which the walker knows).
    pub const fn flags(entry: u64) -> PageFlags {
        if !is_present(entry) {
            return PageFlags::empty();
        }
        let mut f = PageFlags::PRESENT;
        let pairs = [
            (W, PageFlags::WRITABLE),
            (U, PageFlags::USER),
            (G, PageFlags::GLOBAL),
            (A, PageFlags::ACCESSED),
            (D, PageFlags::DIRTY),
            (SW_COW, PageFlags::COW),
            (SW_NO_CACHE, PageFlags::NO_CACHE),
        ];
        let mut i = 0;
        while i < pairs.len() {
            if entry & pairs[i].0 != 0 {
                f = f.union(pairs[i].1);
            }
            i += 1;
        }
        if entry & X == 0 {
            f = f.union(PageFlags::NO_EXECUTE);
        }
        f
    }

    /// Non-leaf entries carry no permissions on RISC-V: nothing to add.
    pub const fn grant_user_table(entry: u64) -> u64 {
        entry
    }
}

/// The format of the target architecture.
#[cfg(target_arch = "riscv64")]
pub use riscv as native;
#[cfg(not(target_arch = "riscv64"))]
pub use x86 as native;

#[cfg(test)]
mod tests {
    use super::*;

    const F: FrameNumber = FrameNumber::new(0x8_0123);
    const RW_USER: PageFlags = PageFlags::PRESENT
        .union(PageFlags::WRITABLE)
        .union(PageFlags::USER)
        .union(PageFlags::NO_EXECUTE);

    #[test]
    fn x86_entries_are_unchanged() {
        // The layout every x86_64 table already holds.
        let e = x86::leaf(F, RW_USER);
        assert_eq!(e, (0x8_0123 << 12) | RW_USER.bits());
        assert!(x86::is_present(e) && !x86::is_huge(e));
        assert_eq!(x86::frame(e), Some(F));
        assert_eq!(x86::flags(e), RW_USER);
        let huge = x86::leaf(F, PageFlags::PRESENT | PageFlags::HUGE);
        assert!(x86::is_huge(huge));
        let t = x86::table(F, true);
        assert_eq!(t, (0x8_0123 << 12) | 0b111, "P | W | U on the user path");
        assert_eq!(x86::table(F, false), (0x8_0123 << 12) | 0b11);
        assert_eq!(x86::grant_user_table(x86::table(F, false)), t);
        assert_eq!(x86::frame(0), None);
    }

    #[test]
    fn riscv_leaf_encoding() {
        use riscv::*;
        let e = leaf(F, RW_USER);
        assert_eq!(e >> 10, 0x8_0123, "PPN from bit 10");
        assert_eq!(e & 0x3FF, V | R | W | U | A | D, "no X: NO_EXECUTE");
        assert_eq!(frame(e), Some(F));
        assert!(is_present(e));
        let back = flags(e);
        assert!(back.contains(RW_USER));
        let exec = leaf(F, PageFlags::PRESENT);
        assert_eq!(
            exec & 0x3FF,
            V | R | X | A | D,
            "kernel, read-only, executable"
        );
        assert!(!flags(exec).contains(PageFlags::NO_EXECUTE));
        let cow = leaf(F, PageFlags::PRESENT | PageFlags::USER | PageFlags::COW);
        assert_eq!(cow & (W | SW_COW), SW_COW, "COW: read-only, software bit");
        assert!(flags(cow).contains(PageFlags::COW));
        assert!(flags(leaf(F, PageFlags::PRESENT | PageFlags::GLOBAL)).contains(PageFlags::GLOBAL));
        assert_eq!(
            leaf(F, PageFlags::WRITABLE),
            0,
            "not present: invalid entry"
        );
        assert_eq!(frame(0), None);
    }

    #[test]
    fn riscv_tables_carry_no_permissions() {
        use riscv::*;
        let t = table(F, true);
        assert_eq!(t, (0x8_0123 << 10) | V, "V only, even on the user path");
        assert!(is_present(t) && !is_huge(t));
        assert_eq!(frame(t), Some(F));
        assert_eq!(grant_user_table(t), t);
        assert!(!flags(t).contains(PageFlags::USER));
        // Any of R, W, X makes a leaf at any level: a large page.
        assert!(is_huge(leaf(F, PageFlags::PRESENT | PageFlags::HUGE)));
        assert!(is_huge(leaf(F, PageFlags::PRESENT)));
        assert_eq!(
            leaf(F, PageFlags::PRESENT | PageFlags::HUGE),
            leaf(F, PageFlags::PRESENT),
            "HUGE is the level, not a bit"
        );
    }
}
