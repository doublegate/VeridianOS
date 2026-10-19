//! Page-table entry formats.
//!
//! The walker (`page_table.rs`) works in the architecture-neutral
//! [`PageFlags`] and asks an entry format to encode and decode: the x86_64
//! layout, the RISC-V Sv39/Sv48 layout (HR-10) and the AArch64 VMSAv8-64
//! layout. All are plain functions of the raw `u64` so that the host tests
//! cover every format whatever the build target.
//!
//! The formats differ in ways the walker has to respect:
//! - A pointer to the next-level table: x86_64 sets PRESENT, WRITABLE and, on
//!   the path to a user page, USER (the CPU checks every level). RISC-V sets
//!   only V; U, A and D are reserved in a non-leaf entry. AArch64 sets valid
//!   and "table" and leaves the table attributes at their permissive zero.
//! - A large-page leaf: x86_64 marks it with PS (`HUGE`); on RISC-V any entry
//!   with R, W or X set is a leaf, at whatever level; on AArch64 it is a block
//!   descriptor (bit 1 clear) at level 1 or 2.
//! - Execute permission: AArch64 has one never-execute bit per privilege level;
//!   kernel pages are never executable at EL0 nor user pages at EL1.

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

/// AArch64 (VMSAv8-64 stage 1, 4 KiB granule; Arm ARM D8.3): bit 0 valid,
/// bit 1 "table" at levels 0-2 and "page" at level 3 (a block, a large
/// page at level 1 or 2, has it clear), the lower attributes from bit 2
/// (AttrIndx, AP, SH, AF, nG), the output address in bits 12-47 and the
/// upper attributes (PXN 53, UXN 54, software bits 55-58).
pub mod arm {
    use super::{FrameNumber, PageFlags};

    pub const VALID: u64 = 1 << 0;
    /// Table (levels 0-2) or page (level 3) descriptor; clear in a block.
    pub const TABLE_OR_PAGE: u64 = 1 << 1;
    /// AttrIndx (bits 2-4): MAIR_EL1 attribute 0, Normal write-back.
    pub const ATTR_NORMAL: u64 = 0 << 2;
    /// AttrIndx: MAIR_EL1 attribute 1, Device-nGnRE.
    pub const ATTR_DEVICE: u64 = 1 << 2;
    const ATTR_MASK: u64 = 7 << 2;
    /// AP[1]: EL0 may access.
    pub const AP_EL0: u64 = 1 << 6;
    /// AP[2]: read-only (at every EL that may access).
    pub const AP_RO: u64 = 1 << 7;
    /// SH (bits 8-9): inner shareable.
    pub const SH_INNER: u64 = 3 << 8;
    /// Access flag: set up front, since a clear AF faults on access and
    /// hardware AF management is optional (FEAT_HAFDBS).
    pub const AF: u64 = 1 << 10;
    /// Not global: the translation belongs to the current ASID.
    pub const NG: u64 = 1 << 11;
    /// Guarded page (FEAT_BTI): indirect branches must land on BTI.
    pub const GP: u64 = 1 << 50;
    /// Privileged (EL1) execute-never.
    pub const PXN: u64 = 1 << 53;
    /// Unprivileged (EL0) execute-never.
    pub const UXN: u64 = 1 << 54;
    /// Software bit 55: copy-on-write (`PageFlags::COW`).
    pub const SW_COW: u64 = 1 << 55;
    /// Output address, bits 12-47 (48-bit physical addresses).
    const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

    /// A leaf: a page (level 3), or a block when `flags` has `HUGE` (a
    /// large page at level 1 or 2). Kernel pages are never executable at
    /// EL0 and user pages never at EL1 (the PAN/SMEP-style rule); EL1
    /// cannot be granted write without read, so WRITABLE alone is RW.
    /// NO_CACHE selects Device memory, which is also never executable.
    pub const fn leaf(frame: FrameNumber, flags: PageFlags) -> u64 {
        if !flags.has(PageFlags::PRESENT) {
            return 0;
        }
        let mut e = ((frame.as_u64() << 12) & ADDR_MASK) | VALID | AF;
        if !flags.has(PageFlags::HUGE) {
            e |= TABLE_OR_PAGE;
        }
        if flags.has(PageFlags::NO_CACHE) {
            e |= ATTR_DEVICE | PXN | UXN;
        } else {
            e |= ATTR_NORMAL | SH_INNER;
        }
        if !flags.has(PageFlags::WRITABLE) {
            e |= AP_RO;
        }
        if flags.has(PageFlags::USER) {
            e |= AP_EL0 | PXN | NG;
            if flags.has(PageFlags::NO_EXECUTE) {
                e |= UXN;
            }
        } else {
            e |= UXN;
            if flags.has(PageFlags::NO_EXECUTE) {
                e |= PXN;
            }
            if !flags.has(PageFlags::GLOBAL) {
                e |= NG;
            }
        }
        if flags.has(PageFlags::COW) {
            e |= SW_COW;
        }
        if flags.has(PageFlags::GUARDED) {
            e |= GP;
        }
        e
    }

    /// A pointer to the next-level table. Access is decided by the leaf
    /// (the table attributes APTable/XNTable are left zero, which
    /// restricts nothing), so `user` changes nothing.
    pub const fn table(frame: FrameNumber, user: bool) -> u64 {
        let _ = user;
        ((frame.as_u64() << 12) & ADDR_MASK) | VALID | TABLE_OR_PAGE
    }

    pub const fn is_present(entry: u64) -> bool {
        entry & VALID != 0
    }

    /// A block (large-page leaf), for an entry above the last level.
    pub const fn is_huge(entry: u64) -> bool {
        is_present(entry) && entry & TABLE_OR_PAGE == 0
    }

    pub const fn frame(entry: u64) -> Option<FrameNumber> {
        if is_present(entry) {
            Some(FrameNumber::new((entry & ADDR_MASK) >> 12))
        } else {
            None
        }
    }

    /// The entry's permissions as [`PageFlags`] (HUGE for a block; for a
    /// level-3 page that bit is meaningless, as the walker knows).
    pub const fn flags(entry: u64) -> PageFlags {
        if !is_present(entry) {
            return PageFlags::empty();
        }
        let mut f = PageFlags::PRESENT;
        if entry & AP_RO == 0 {
            f = f.union(PageFlags::WRITABLE);
        }
        let user = entry & AP_EL0 != 0;
        if user {
            f = f.union(PageFlags::USER);
        }
        if entry & AF != 0 {
            f = f.union(PageFlags::ACCESSED);
        }
        if entry & NG == 0 {
            f = f.union(PageFlags::GLOBAL);
        }
        if entry & ATTR_MASK == ATTR_DEVICE {
            f = f.union(PageFlags::NO_CACHE);
        }
        if entry & SW_COW != 0 {
            f = f.union(PageFlags::COW);
        }
        if entry & GP != 0 {
            f = f.union(PageFlags::GUARDED);
        }
        if entry & TABLE_OR_PAGE == 0 {
            f = f.union(PageFlags::HUGE);
        }
        let xn = if user { UXN } else { PXN };
        if entry & xn != 0 {
            f = f.union(PageFlags::NO_EXECUTE);
        }
        f
    }

    /// Table descriptors carry no permissions here: nothing to add.
    pub const fn grant_user_table(entry: u64) -> u64 {
        entry
    }
}

// `native`: the format of the target architecture. On AArch64 the boot
// tables are built in boot.S (ADR 0015); the ones built here are process
// address spaces, which are loaded once AArch64 has EL0.
#[cfg(target_arch = "aarch64")]
pub use arm as native;
#[cfg(target_arch = "riscv64")]
pub use riscv as native;
#[cfg(not(any(target_arch = "riscv64", target_arch = "aarch64")))]
pub use x86 as native;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_page_encoding() {
        use arm::*;
        // User read-write data: an EL0-accessible page, executable at
        // neither EL, not global, normal inner-shareable memory.
        let e = leaf(F, RW_USER);
        assert_eq!(e & 0x0000_FFFF_FFFF_F000, 0x8_0123 << 12, "output address");
        assert_eq!(e & 3, VALID | TABLE_OR_PAGE, "a level-3 page");
        assert_eq!(e & (AP_EL0 | AP_RO), AP_EL0, "EL0 read-write");
        assert_eq!(e & (PXN | UXN), PXN | UXN, "no execution");
        assert_eq!(e & (AF | NG | SH_INNER), AF | NG | SH_INNER);
        assert_eq!(frame(e), Some(F));
        assert!(flags(e).contains(RW_USER));
        // User code: EL0 may execute, EL1 never (PXN on user pages).
        let code = leaf(F, PageFlags::PRESENT | PageFlags::USER);
        assert_eq!(code & (PXN | UXN | AP_RO), PXN | AP_RO);
        assert!(!flags(code).contains(PageFlags::NO_EXECUTE));
        // Kernel code: EL1 may execute, EL0 never; global.
        let k = leaf(F, PageFlags::PRESENT | PageFlags::GLOBAL);
        assert_eq!(k & (PXN | UXN | NG | AP_EL0), UXN);
        let back = flags(k);
        assert!(back.contains(PageFlags::GLOBAL) && !back.contains(PageFlags::NO_EXECUTE));
        // Kernel data: NO_EXECUTE is PXN.
        let kd = leaf(
            F,
            PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE,
        );
        assert_eq!(kd & (PXN | UXN), PXN | UXN);
        assert!(flags(kd).contains(PageFlags::NO_EXECUTE | PageFlags::WRITABLE));
        assert_eq!(leaf(F, PageFlags::WRITABLE), 0, "not present: invalid");
        assert_eq!(frame(0), None);
    }

    #[test]
    fn arm_device_cow_and_blocks() {
        use arm::*;
        let dev = leaf(
            F,
            PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_CACHE,
        );
        assert_eq!(dev & (7 << 2), ATTR_DEVICE, "MAIR attribute 1");
        assert_eq!(dev & (PXN | UXN), PXN | UXN, "device memory never executes");
        assert!(flags(dev).contains(PageFlags::NO_CACHE));
        let cow = leaf(F, PageFlags::PRESENT | PageFlags::USER | PageFlags::COW);
        assert_eq!(
            cow & (AP_RO | SW_COW),
            AP_RO | SW_COW,
            "COW: read-only, software bit"
        );
        assert!(flags(cow).contains(PageFlags::COW));
        // HUGE makes a block (bit 1 clear), which is_huge recognises; a
        // table pointer is not one.
        let block = leaf(F, PageFlags::PRESENT | PageFlags::HUGE);
        assert_eq!(block & 3, VALID);
        assert!(is_huge(block) && flags(block).contains(PageFlags::HUGE));
        let t = table(F, true);
        assert_eq!(t, (0x8_0123 << 12) | 3, "valid table, no attributes");
        assert!(is_present(t) && !is_huge(t));
        assert_eq!(table(F, false), t);
        assert_eq!(grant_user_table(t), t);
    }

    #[test]
    fn arm_guarded_pages() {
        use arm::*;
        let code = PageFlags::PRESENT | PageFlags::USER;
        let gp = leaf(F, code | PageFlags::GUARDED);
        assert_eq!(gp & GP, GP, "PROT_BTI sets GP (bit 50)");
        assert_eq!(gp & !GP, leaf(F, code), "and nothing else");
        assert!(flags(gp).contains(PageFlags::GUARDED));
        assert!(!flags(leaf(F, code)).contains(PageFlags::GUARDED));
    }

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
