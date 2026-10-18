//! Clearing and copying whole 4 KiB pages (Linux `clear_page`, `copy_page`).
//!
//! Every fresh user page is zeroed and every copy-on-write break copies one,
//! so these are on the mmap, brk, fault and fork paths. `write_bytes` and
//! `copy_nonoverlapping` call the `memset` and `memcpy` of
//! `compiler_builtins`, which `-Zbuild-std` builds at the kernel's own
//! opt-level: in a dev kernel that is about 30 cycles a byte, so a 16-page
//! mmap took ~570 us, nearly all of it zeroing (N-281). On x86_64 a string
//! store or move of 512 quadwords is used instead (~450 cycles a page under
//! KVM, against ~3,400 for `write_bytes` even in a release kernel).
//! Elsewhere the plain functions are used.

/// Bytes in a page.
pub const PAGE_BYTES: usize = 4096;

/// Zero the page at `page`.
///
/// # Safety
///
/// `page` must be valid for writes of [`PAGE_BYTES`] bytes and 8-byte
/// aligned, and nothing else may access the page meanwhile.
#[inline]
pub unsafe fn clear_page(page: *mut u8) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the caller guarantees `page` is valid for 4096 bytes of
    // writes; `rep stosq` stores RCX = 512 quadwords of RAX = 0 from RDI
    // upwards (the ABI guarantees the direction flag is clear) and touches
    // nothing else.
    unsafe {
        core::arch::asm!(
            "rep stosq",
            inout("rcx") PAGE_BYTES / 8 => _,
            inout("rdi") page => _,
            in("rax") 0u64,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: as above, the caller guarantees the page is writable.
    unsafe {
        core::ptr::write_bytes(page, 0, PAGE_BYTES);
    }
}

/// Copy the page at `src` to `dst`.
///
/// # Safety
///
/// `src` must be valid for reads and `dst` for writes of [`PAGE_BYTES`]
/// bytes, both 8-byte aligned and not overlapping, and nothing may write
/// either meanwhile.
#[inline]
pub unsafe fn copy_page(dst: *mut u8, src: *const u8) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the caller guarantees both pages are valid and distinct;
    // `rep movsq` moves RCX = 512 quadwords from RSI to RDI, upwards (the
    // direction flag is clear by the ABI).
    unsafe {
        core::arch::asm!(
            "rep movsq",
            inout("rcx") PAGE_BYTES / 8 => _,
            inout("rdi") dst => _,
            inout("rsi") src => _,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: as above, the caller guarantees both pages are valid and
    // do not overlap.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, PAGE_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page between two guard pages, to catch writes past either end.
    #[repr(C, align(4096))]
    struct Pages([[u8; PAGE_BYTES]; 3]);

    #[test]
    fn clear_page_zeroes_exactly_one_page() {
        let mut p = alloc::boxed::Box::new(Pages([[0xA5; PAGE_BYTES]; 3]));
        // SAFETY: the middle page is in bounds, aligned and borrowed here.
        unsafe { clear_page(p.0[1].as_mut_ptr()) };
        assert!(p.0[1].iter().all(|&b| b == 0));
        assert!(p.0[0].iter().chain(p.0[2].iter()).all(|&b| b == 0xA5));
    }

    #[test]
    fn copy_page_copies_exactly_one_page() {
        let mut p = alloc::boxed::Box::new(Pages([[0; PAGE_BYTES]; 3]));
        let src: alloc::vec::Vec<u8> = (0..PAGE_BYTES).map(|i| (i * 7 + 3) as u8).collect();
        let mut aligned = alloc::boxed::Box::new(Pages([[0; PAGE_BYTES]; 3]));
        aligned.0[0].copy_from_slice(&src);
        p.0[0].fill(0x11);
        p.0[2].fill(0x22);
        // SAFETY: both pages are in bounds, aligned and distinct.
        unsafe { copy_page(p.0[1].as_mut_ptr(), aligned.0[0].as_ptr()) };
        assert_eq!(&p.0[1][..], &src[..]);
        assert!(p.0[0].iter().all(|&b| b == 0x11));
        assert!(p.0[2].iter().all(|&b| b == 0x22));
    }
}
