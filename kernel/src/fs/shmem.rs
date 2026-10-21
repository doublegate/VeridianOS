//! Page-backed file contents, Linux's shmem: the bytes of a memfd (N-230)
//! or a tmpfs file live in physical frames the file owns, so a MAP_SHARED
//! mapping maps the file itself -- every mapping, read and write sees one
//! copy. A mapping adds an owner to each frame it maps (`mm::frame_refs`);
//! a page leaves memory when its last owner, file or mapping, lets go.
//!
//! The owner (`MemfdNode`, `TmpNode`) keeps a [`ShmemPages`] under its own
//! lock and checks its own rules (seals, size limit) before calling in.

use alloc::vec::Vec;

use crate::{
    error::{FsError, KernelError},
    mm::FrameNumber,
};

const PAGE: usize = 4096;

/// Where pages come from. The kernel takes frames from the frame allocator
/// and reaches them through the physical map; the host tests have neither,
/// so there a "frame" is a page-aligned heap block whose address is its
/// frame number shifted left by 12.
#[cfg(not(test))]
mod source {
    use super::{FrameNumber, PAGE};
    use crate::mm::FRAME_ALLOCATOR;

    /// A zeroed page, or None when memory is exhausted.
    pub(super) fn allocate() -> Option<FrameNumber> {
        let frame = FRAME_ALLOCATOR.lock().allocate_frames(1, None).ok()?;
        // SAFETY: `frame` was just allocated, is RAM reached through the
        // physical map and is mapped nowhere yet.
        unsafe { core::ptr::write_bytes(bytes(frame), 0, PAGE) };
        Some(frame)
    }

    pub(super) fn free(frame: FrameNumber) {
        crate::mm::note_free_failure(FRAME_ALLOCATOR.lock().free_frames(frame, 1), frame, "shmem");
    }

    /// The page's bytes in the kernel's physical map.
    pub(super) fn bytes(frame: FrameNumber) -> *mut u8 {
        crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8
    }
}

#[cfg(test)]
mod source {
    use core::alloc::Layout;

    use super::{FrameNumber, PAGE};

    fn layout() -> Layout {
        // PAGE is a non-zero power of two.
        Layout::from_size_align(PAGE, PAGE).unwrap()
    }

    pub(super) fn allocate() -> Option<FrameNumber> {
        // SAFETY: the layout has a non-zero size.
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout()) };
        (!ptr.is_null()).then(|| FrameNumber::new(ptr as u64 >> 12))
    }

    pub(super) fn free(frame: FrameNumber) {
        // SAFETY: `frame` came from `allocate` with this layout and its
        // last owner is giving it up.
        unsafe { alloc::alloc::dealloc(bytes(frame), layout()) };
    }

    pub(super) fn bytes(frame: FrameNumber) -> *mut u8 {
        (frame.as_u64() << 12) as *mut u8
    }
}

/// Give up the file's ownership of `frame`, freeing it if no mapping holds
/// it any more.
fn release(frame: FrameNumber) {
    if crate::mm::frame_refs::release(frame) {
        source::free(frame);
    }
}

/// A file's pages and its length in bytes.
pub(crate) struct ShmemPages {
    size: usize,
    frames: Vec<FrameNumber>,
}

impl ShmemPages {
    pub(crate) const fn new() -> Self {
        Self {
            size: 0,
            frames: Vec::new(),
        }
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Whether any page is also mapped (has an owner besides the file).
    pub(crate) fn any_mapped(&self) -> bool {
        self.frames
            .iter()
            .any(|&f| crate::mm::frame_refs::is_shared(f))
    }

    /// Copy bytes from `offset` into `buffer`; returns how many (0 at or
    /// past the end).
    pub(crate) fn read(&self, offset: usize, buffer: &mut [u8]) -> usize {
        if offset >= self.size {
            return 0;
        }
        let n = buffer.len().min(self.size - offset);
        let mut done = 0;
        while done < n {
            let at = offset + done;
            let (page, within) = (at / PAGE, at % PAGE);
            let chunk = (PAGE - within).min(n - done);
            // SAFETY: page `page` is below the size, so the file holds its
            // frame; `within + chunk <= PAGE`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    source::bytes(self.frames[page]).add(within),
                    buffer[done..].as_mut_ptr(),
                    chunk,
                );
            }
            done += chunk;
        }
        n
    }

    /// Store `data` at `offset`, growing the file first if it ends past
    /// the end (errors as [`Self::resize`]).
    pub(crate) fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), KernelError> {
        let end = super::ram_write_end(offset, data.len())?;
        if end > self.size {
            self.resize(end)?;
        }
        let mut done = 0;
        while done < data.len() {
            let at = offset + done;
            let (page, within) = (at / PAGE, at % PAGE);
            let chunk = (PAGE - within).min(data.len() - done);
            // SAFETY: as in read; the file now covers `end`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data[done..].as_ptr(),
                    source::bytes(self.frames[page]).add(within),
                    chunk,
                );
            }
            done += chunk;
        }
        Ok(())
    }

    /// Resize to `size` bytes: new pages are zeroed; pages past the end are
    /// released.
    ///
    /// A shrink that would drop a page still mapped MAP_SHARED is EBUSY:
    /// the mapping kept the old frame while a later grow gave the file a
    /// new one, so the two stopped sharing (PR #24 review). Linux unmaps
    /// such pages instead (access then raises SIGBUS); mappings cannot be
    /// invalidated here yet (docs/KNOWN-LIMITATIONS.md). A grow that runs
    /// out of memory part-way gives back what it took, leaving the file as
    /// it was (ENOSPC). Past the RAM-file limit: EFBIG.
    pub(crate) fn resize(&mut self, size: usize) -> Result<(), KernelError> {
        if size > super::MAX_RAM_FILE_SIZE {
            return Err(KernelError::FsError(FsError::FileTooLarge));
        }
        let want = size.div_ceil(PAGE);
        if self
            .frames
            .get(want..)
            .is_some_and(|dropped| dropped.iter().any(|&f| crate::mm::frame_refs::is_shared(f)))
        {
            return Err(KernelError::FsError(FsError::Busy));
        }
        let had = self.frames.len();
        while self.frames.len() < want {
            let Some(frame) = source::allocate() else {
                while self.frames.len() > had {
                    if let Some(frame) = self.frames.pop() {
                        release(frame);
                    }
                }
                return Err(KernelError::FsError(FsError::NoSpace));
            };
            self.frames.push(frame);
        }
        while self.frames.len() > want {
            if let Some(frame) = self.frames.pop() {
                release(frame);
            }
        }
        // A shrink within the last page zeroes its tail, so a later grow
        // reads zeros there.
        if size < self.size && !size.is_multiple_of(PAGE) {
            if let Some(&frame) = self.frames.last() {
                let from = size % PAGE;
                // SAFETY: the file's own last frame; `from < PAGE`.
                unsafe { core::ptr::write_bytes(source::bytes(frame).add(from), 0, PAGE - from) };
            }
        }
        self.size = size;
        Ok(())
    }

    /// The frames of pages `first_page..first_page + count`, each given one
    /// more owner for the mapping that takes them. The range must lie
    /// within the file (EINVAL): Linux maps past the end and faults on
    /// access (SIGBUS).
    pub(crate) fn share(
        &self,
        first_page: usize,
        count: usize,
    ) -> Result<Vec<FrameNumber>, KernelError> {
        let end = match first_page.checked_add(count) {
            Some(end) if end <= self.frames.len() => end,
            _ => {
                return Err(KernelError::InvalidArgument {
                    name: "offset",
                    value: "mapping extends past the end of the file",
                });
            }
        };
        let frames = self.frames[first_page..end].to_vec();
        for &frame in &frames {
            crate::mm::frame_refs::share(frame);
        }
        Ok(frames)
    }
}

impl Drop for ShmemPages {
    fn drop(&mut self) {
        for frame in self.frames.drain(..) {
            release(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_at(p: &ShmemPages, offset: usize, len: usize) -> Vec<u8> {
        let mut buf = alloc::vec![0xAAu8; len];
        let n = p.read(offset, &mut buf);
        buf.truncate(n);
        buf
    }

    #[test]
    fn writes_across_pages_read_back_and_holes_are_zero() {
        let mut p = ShmemPages::new();
        p.write(PAGE - 2, b"abcd").unwrap();
        assert_eq!(p.size(), PAGE + 2);
        assert_eq!(bytes_at(&p, PAGE - 2, 8), b"abcd");
        assert_eq!(bytes_at(&p, 0, 4), [0; 4]);
        assert!(bytes_at(&p, PAGE + 2, 4).is_empty(), "nothing past the end");
    }

    #[test]
    fn shrink_zeroes_the_tail_a_later_grow_exposes() {
        let mut p = ShmemPages::new();
        p.write(0, b"secret").unwrap();
        p.resize(2).unwrap();
        p.resize(6).unwrap();
        assert_eq!(bytes_at(&p, 0, 6), b"se\0\0\0\0");
    }

    #[test]
    fn a_shared_page_is_one_copy_and_blocks_a_shrink() {
        let mut p = ShmemPages::new();
        p.resize(2 * PAGE).unwrap();
        let frames = p.share(1, 1).unwrap();
        assert!(p.any_mapped());
        // A write through the mapping's frame is what the file reads.
        // SAFETY: the shared frame is a live page of this file.
        unsafe { source::bytes(frames[0]).write(7) };
        assert_eq!(bytes_at(&p, PAGE, 1), [7]);
        assert!(matches!(
            p.resize(PAGE),
            Err(KernelError::FsError(FsError::Busy))
        ));
        // The mapping goes: the page is the file's alone again.
        assert!(!crate::mm::frame_refs::release(frames[0]));
        assert!(!p.any_mapped());
        p.resize(PAGE).unwrap();
    }

    #[test]
    fn a_mapping_must_lie_within_the_file() {
        let mut p = ShmemPages::new();
        assert!(p.share(0, 0).unwrap().is_empty());
        assert!(p.share(0, 1).is_err());
        p.resize(1).unwrap();
        assert_eq!(p.share(0, 1).map(|f| f.len()).ok(), Some(1));
        assert!(p.share(usize::MAX, 2).is_err(), "no overflow");
        // Give back the owner the successful share added.
        let f = p.frames[0];
        assert!(!crate::mm::frame_refs::release(f));
    }

    #[test]
    fn too_large_is_efbig() {
        let mut p = ShmemPages::new();
        assert!(matches!(
            p.resize(super::super::MAX_RAM_FILE_SIZE + 1),
            Err(KernelError::FsError(FsError::FileTooLarge))
        ));
        assert_eq!(p.size(), 0);
    }
}
