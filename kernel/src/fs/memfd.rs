//! memfd_create's files (N-229, N-230): anonymous regular files whose pages
//! are physical frames the file owns, so a MAP_SHARED mapping maps the file
//! itself -- every mapping, read and write sees one copy, which is how a
//! Wayland client and its compositor share a buffer. A mapping adds an
//! owner to each frame it maps (`mm::frame_refs`); a page leaves memory when
//! its last owner, file or mapping, lets go.
//!
//! Seals (fcntl F_ADD_SEALS) are enforced here: shrink, grow, write, future
//! write and exec, and SEAL against further seals.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use spin::{Mutex, RwLock};

use super::{seals, DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    mm::{FrameNumber, FRAME_ALLOCATOR},
};

const PAGE: usize = 4096;

/// The pages and their length.
struct Pages {
    size: usize,
    frames: Vec<FrameNumber>,
}

pub struct MemfdNode {
    pages: Mutex<Pages>,
    metadata: RwLock<Metadata>,
    seals: AtomicU32,
}

/// A frame's bytes in the kernel's physical map.
fn frame_bytes(frame: FrameNumber) -> *mut u8 {
    crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8
}

/// Give up the file's ownership of `frame`, freeing it if no mapping holds
/// it any more.
fn release(frame: FrameNumber) {
    if crate::mm::frame_refs::release(frame) {
        crate::mm::note_free_failure(FRAME_ALLOCATOR.lock().free_frames(frame, 1), frame, "memfd");
    }
}

fn eperm() -> KernelError {
    KernelError::FsError(FsError::OperationNotPermitted)
}

impl MemfdNode {
    /// A new, empty file: owned by `uid`/`gid` with `permissions`. Without
    /// `sealable` it starts sealed against further seals (Linux without
    /// MFD_ALLOW_SEALING); `initial_seals` apply otherwise (F_SEAL_EXEC for
    /// MFD_NOEXEC_SEAL).
    pub fn new(
        permissions: Permissions,
        uid: u32,
        gid: u32,
        sealable: bool,
        initial_seals: u32,
    ) -> Arc<Self> {
        let now = crate::arch::timer::get_timestamp_secs();
        Arc::new(Self {
            pages: Mutex::new(Pages {
                size: 0,
                frames: Vec::new(),
            }),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::File,
                size: 0,
                permissions,
                uid,
                gid,
                created: now,
                modified: now,
                accessed: now,
                inode: super::ramfs::next_inode(),
            }),
            seals: AtomicU32::new(if sealable {
                initial_seals & seals::ALL
            } else {
                seals::SEAL
            }),
        })
    }

    /// Resize to `size` bytes: new pages are zeroed frames; pages past the
    /// end are released (a mapping that still holds one keeps it).
    fn resize(&self, pages: &mut Pages, size: usize) -> Result<(), KernelError> {
        if size > super::MAX_RAM_FILE_SIZE {
            return Err(KernelError::FsError(FsError::FileTooLarge));
        }
        let want = size.div_ceil(PAGE);
        while pages.frames.len() < want {
            let frame = FRAME_ALLOCATOR
                .lock()
                .allocate_frames(1, None)
                .map_err(|_| KernelError::FsError(FsError::NoSpace))?;
            // SAFETY: `frame` was just allocated, is RAM reached through the
            // physical map and is mapped nowhere yet.
            unsafe { core::ptr::write_bytes(frame_bytes(frame), 0, PAGE) };
            pages.frames.push(frame);
        }
        while pages.frames.len() > want {
            if let Some(frame) = pages.frames.pop() {
                release(frame);
            }
        }
        // A shrink within the last page zeroes its tail, so a later grow
        // reads zeros there.
        if size < pages.size && !size.is_multiple_of(PAGE) {
            if let Some(&frame) = pages.frames.last() {
                let from = size % PAGE;
                // SAFETY: the file's own last frame; `from < PAGE`.
                unsafe { core::ptr::write_bytes(frame_bytes(frame).add(from), 0, PAGE - from) };
            }
        }
        pages.size = size;
        let mut meta = self.metadata.write();
        meta.size = size;
        meta.modified = crate::arch::timer::get_timestamp_secs();
        Ok(())
    }

    fn sealed(&self, which: u32) -> bool {
        self.seals.load(Ordering::Acquire) & which != 0
    }
}

impl Drop for MemfdNode {
    fn drop(&mut self) {
        for frame in self.pages.get_mut().frames.drain(..) {
            release(frame);
        }
    }
}

impl VfsNode for MemfdNode {
    fn node_type(&self) -> NodeType {
        NodeType::File
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let pages = self.pages.lock();
        if offset >= pages.size {
            return Ok(0);
        }
        let n = buffer.len().min(pages.size - offset);
        let mut done = 0;
        while done < n {
            let at = offset + done;
            let (page, within) = (at / PAGE, at % PAGE);
            let chunk = (PAGE - within).min(n - done);
            // SAFETY: page `page` is below the size, so the file holds its
            // frame; `within + chunk <= PAGE`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    frame_bytes(pages.frames[page]).add(within),
                    buffer[done..].as_mut_ptr(),
                    chunk,
                );
            }
            done += chunk;
        }
        Ok(n)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        let end = super::ram_write_end(offset, data.len())?;
        let mut pages = self.pages.lock();
        if self.sealed(seals::WRITE | seals::FUTURE_WRITE)
            || (self.sealed(seals::GROW) && end > pages.size)
        {
            return Err(eperm());
        }
        if end > pages.size {
            self.resize(&mut pages, end)?;
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
                    frame_bytes(pages.frames[page]).add(within),
                    chunk,
                );
            }
            done += chunk;
        }
        self.metadata.write().modified = crate::arch::timer::get_timestamp_secs();
        Ok(data.len())
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        let mut pages = self.pages.lock();
        if (self.sealed(seals::SHRINK) && size < pages.size)
            || (self.sealed(seals::GROW) && size > pages.size)
        {
            return Err(eperm());
        }
        self.resize(&mut pages, size)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn rename(
        &self,
        _old_name: &str,
        _new_parent: &Arc<dyn VfsNode>,
        _new_name: &str,
    ) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn chmod(&self, permissions: Permissions) -> Result<(), KernelError> {
        let mut meta = self.metadata.write();
        if self.sealed(seals::EXEC)
            && (meta.permissions.to_mode() ^ permissions.to_mode()) & 0o111 != 0
        {
            return Err(eperm());
        }
        meta.permissions = permissions;
        meta.modified = crate::arch::timer::get_timestamp_secs();
        Ok(())
    }

    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        let mut meta = self.metadata.write();
        if let Some(uid) = uid {
            meta.uid = uid;
        }
        if let Some(gid) = gid {
            meta.gid = gid;
        }
        Ok(())
    }

    fn seals(&self) -> Option<u32> {
        Some(self.seals.load(Ordering::Acquire))
    }

    fn add_seals(&self, add: u32) -> Result<(), KernelError> {
        if add & !seals::ALL != 0 {
            return Err(KernelError::InvalidArgument {
                name: "seals",
                value: "unknown seal",
            });
        }
        // Under the pages lock, so no write, truncate or new mapping slips
        // in between the check and the change.
        let pages = self.pages.lock();
        let current = self.seals.load(Ordering::Acquire);
        if current & seals::SEAL != 0 {
            return Err(eperm());
        }
        // F_SEAL_WRITE while the file is mapped shared: EBUSY on Linux for a
        // writable mapping. A frame with an owner besides the file is
        // mapped; whether writably is not recorded, so any mapping counts.
        if add & seals::WRITE != 0
            && current & seals::WRITE == 0
            && pages
                .frames
                .iter()
                .any(|&f| crate::mm::frame_refs::is_shared(f))
        {
            return Err(KernelError::FsError(FsError::Busy));
        }
        self.seals.store(current | add, Ordering::Release);
        Ok(())
    }

    fn share_pages(
        &self,
        first_page: usize,
        count: usize,
        writable: bool,
    ) -> Option<Result<Vec<FrameNumber>, KernelError>> {
        let pages = self.pages.lock();
        if writable && self.sealed(seals::WRITE | seals::FUTURE_WRITE) {
            return Some(Err(eperm()));
        }
        let end = match first_page.checked_add(count) {
            Some(end) if end <= pages.frames.len() => end,
            // Linux maps past the end and faults on access (SIGBUS); here
            // the mapping must lie within the file.
            _ => {
                return Some(Err(KernelError::InvalidArgument {
                    name: "offset",
                    value: "mapping extends past the end of the memfd",
                }));
            }
        };
        let frames = pages.frames[first_page..end].to_vec();
        for &frame in &frames {
            crate::mm::frame_refs::share(frame);
        }
        Some(Ok(frames))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eperm<T: core::fmt::Debug>(r: Result<T, KernelError>) -> bool {
        matches!(r, Err(KernelError::FsError(FsError::OperationNotPermitted)))
    }

    // The host tests have no frame allocator: they cover what is decided
    // before a page is allocated; the runtime test covers the pages.

    #[test]
    fn without_allow_sealing_no_seal_can_be_added() {
        let m = MemfdNode::new(Permissions::from_mode(0o777), 0, 0, false, 0);
        assert_eq!(m.seals(), Some(seals::SEAL));
        assert!(eperm(m.add_seals(seals::WRITE)));
    }

    #[test]
    fn seals_refuse_what_they_seal() {
        let m = MemfdNode::new(Permissions::from_mode(0o777), 1000, 100, true, 0);
        let meta = m.metadata().unwrap();
        assert_eq!((meta.uid, meta.gid, meta.size), (1000, 100, 0));
        m.add_seals(seals::GROW).unwrap();
        assert!(eperm(m.truncate(4096)));
        assert!(eperm(m.write(0, b"x")), "a write past the end grows");
        m.add_seals(seals::WRITE).unwrap();
        assert!(eperm(m.write(0, b"")));
        assert!(eperm(m.share_pages(0, 0, true).unwrap()));
        assert!(matches!(
            m.add_seals(0x40),
            Err(KernelError::InvalidArgument { .. })
        ));
        m.add_seals(seals::SEAL).unwrap();
        assert!(eperm(m.add_seals(seals::SHRINK)));
        assert_eq!(m.seals(), Some(seals::GROW | seals::WRITE | seals::SEAL));
    }

    #[test]
    fn exec_seal_fixes_the_execute_bits() {
        let m = MemfdNode::new(Permissions::from_mode(0o666), 0, 0, true, seals::EXEC);
        assert!(eperm(m.chmod(Permissions::from_mode(0o766))));
        m.chmod(Permissions::from_mode(0o600)).unwrap();
        assert_eq!(m.metadata().unwrap().permissions.to_mode() & 0o777, 0o600);
    }

    #[test]
    fn a_shared_mapping_must_lie_within_the_file() {
        let m = MemfdNode::new(Permissions::from_mode(0o777), 0, 0, true, 0);
        assert!(matches!(
            m.share_pages(0, 1, false),
            Some(Err(KernelError::InvalidArgument { .. }))
        ));
        assert!(matches!(m.share_pages(0, 0, false), Some(Ok(v)) if v.is_empty()));
    }
}
