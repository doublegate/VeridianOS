//! Address-space identifiers (HA-26).
//!
//! An ASID tags the TLB entries of one address space, so switching between
//! address spaces needs no TLB invalidation: entries of the others stay, and
//! do not match. Each user page-table root gets its own ASID for as long as
//! the root lives; the ASID is released (and its entries invalidated on
//! every CPU) when the root is freed, before the number can be handed out
//! again. ASID 0 is never allocated: it is the empty root's (which maps
//! nothing) and the fallback when every ASID is in use, for which the switch
//! invalidates ASID 0's entries on the CPU instead.
//!
//! This is the architecture-neutral allocator; `arch::aarch64::switch` puts
//! the ASID in TTBR0_EL1.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec, vec::Vec};

/// Hands out ASIDs `1..limit` to page-table roots.
#[cfg(feature = "alloc")]
pub struct AsidAllocator {
    by_root: BTreeMap<u64, u16>,
    /// One bit per ASID; bit 0 (ASID 0) is always set.
    used: Vec<u64>,
    limit: usize,
    /// Where the search for a free ASID starts (the one after the last).
    next: usize,
}

#[cfg(feature = "alloc")]
impl AsidAllocator {
    /// An allocator for ASIDs below `limit` (256 for 8-bit ASIDs, 65536 for
    /// 16-bit ones), clamped to 2..=65536.
    pub fn new(limit: usize) -> Self {
        let limit = limit.clamp(2, 1 << 16);
        let mut used = vec![0u64; limit.div_ceil(64)];
        used[0] = 1;
        Self {
            by_root: BTreeMap::new(),
            used,
            limit,
            next: 1,
        }
    }

    fn is_used(&self, asid: usize) -> bool {
        self.used[asid / 64] & (1 << (asid % 64)) != 0
    }

    /// The ASID of `root`, allocating one if it has none; 0 when every ASID
    /// is in use (or `root` is 0).
    pub fn get_or_allocate(&mut self, root: u64) -> u16 {
        if root == 0 {
            return 0;
        }
        if let Some(&asid) = self.by_root.get(&root) {
            return asid;
        }
        let found = (self.next..self.limit)
            .chain(1..self.next)
            .find(|&a| !self.is_used(a));
        let Some(asid) = found else {
            return 0;
        };
        self.used[asid / 64] |= 1 << (asid % 64);
        self.next = if asid + 1 < self.limit { asid + 1 } else { 1 };
        self.by_root.insert(root, asid as u16);
        asid as u16
    }

    /// Release the ASID of `root`, returning it (the caller invalidates its
    /// TLB entries before the number can be reused); `None` if it had none.
    pub fn release(&mut self, root: u64) -> Option<u16> {
        let asid = self.by_root.remove(&root)?;
        let a = asid as usize;
        self.used[a / 64] &= !(1 << (a % 64));
        Some(asid)
    }

    /// The number of roots holding an ASID.
    pub fn in_use(&self) -> usize {
        self.by_root.len()
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    #[test]
    fn roots_get_distinct_nonzero_asids_and_keep_them() {
        let mut a = AsidAllocator::new(256);
        let x = a.get_or_allocate(0x1000);
        let y = a.get_or_allocate(0x2000);
        assert!(x != 0 && y != 0 && x != y);
        assert_eq!(a.get_or_allocate(0x1000), x);
        assert_eq!(a.in_use(), 2);
    }

    #[test]
    fn root_zero_has_asid_zero() {
        let mut a = AsidAllocator::new(256);
        assert_eq!(a.get_or_allocate(0), 0);
        assert_eq!(a.in_use(), 0);
    }

    #[test]
    fn exhaustion_falls_back_to_zero_and_release_frees() {
        let mut a = AsidAllocator::new(4); // ASIDs 1, 2, 3
        let got: Vec<u16> = (1..=3).map(|r| a.get_or_allocate(r * 0x1000)).collect();
        assert_eq!(got, [1, 2, 3]);
        assert_eq!(a.get_or_allocate(0x9000), 0);
        assert_eq!(a.release(0x2000), Some(2));
        assert_eq!(a.release(0x2000), None);
        assert_eq!(a.get_or_allocate(0x9000), 2);
    }

    #[test]
    fn released_asids_are_not_reused_at_once() {
        // The search moves on from the last allocation, so a freshly released
        // ASID is the last to be handed out again.
        let mut a = AsidAllocator::new(256);
        let x = a.get_or_allocate(0x1000);
        a.release(0x1000);
        let y = a.get_or_allocate(0x2000);
        assert_ne!(x, y);
    }

    #[test]
    fn sixteen_bit_limit_spans_words() {
        let mut a = AsidAllocator::new(1 << 16);
        for r in 1..=200u64 {
            assert_eq!(a.get_or_allocate(r << 12), r as u16);
        }
        assert_eq!(a.in_use(), 200);
    }
}
