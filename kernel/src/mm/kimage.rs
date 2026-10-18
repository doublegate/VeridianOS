//! W^X for the kernel image: the permissions of each of its pages.
//!
//! The image is code `[start, text_end)`, read-only data up to
//! `rodata_end`, then data, BSS and the boot stack, as each architecture's
//! linker script lays it out (`__text_end`, `__rodata_end`). Each part gets
//! only what it needs: code read and execute, read-only data read, the rest
//! read and write. `arch::{aarch64, riscv64}::kmap` map the image with them.

use super::PageFlags;

/// The flags of the kernel-image page at `va`.
pub fn page_flags(va: u64, text_end: u64, rodata_end: u64) -> PageFlags {
    let flags = PageFlags::PRESENT | PageFlags::GLOBAL;
    if va < text_end {
        flags
    } else if va < rodata_end {
        flags | PageFlags::NO_EXECUTE
    } else {
        flags | PageFlags::WRITABLE | PageFlags::NO_EXECUTE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No image page is both writable and executable, and each part keeps
    /// what it needs.
    #[test]
    fn image_pages_are_w_xor_x() {
        let (text_end, rodata_end) = (0x3000, 0x5000);
        let wx = |f: PageFlags| {
            (
                f.contains(PageFlags::WRITABLE),
                !f.contains(PageFlags::NO_EXECUTE),
            )
        };
        assert_eq!(wx(page_flags(0x0, text_end, rodata_end)), (false, true));
        assert_eq!(wx(page_flags(0x2FFF, text_end, rodata_end)), (false, true));
        assert_eq!(wx(page_flags(0x3000, text_end, rodata_end)), (false, false));
        assert_eq!(wx(page_flags(0x4FFF, text_end, rodata_end)), (false, false));
        assert_eq!(wx(page_flags(0x5000, text_end, rodata_end)), (true, false));
        for va in (0..0x8000).step_by(0x1000) {
            let f = page_flags(va, text_end, rodata_end);
            assert!(f.contains(PageFlags::PRESENT | PageFlags::GLOBAL));
            assert!(!f.contains(PageFlags::USER));
        }
    }
}
