//! memfd_create's files (N-229, N-230): anonymous regular files whose pages
//! are physical frames the file owns (`fs::shmem`), so a MAP_SHARED mapping
//! maps the file itself -- every mapping, read and write sees one copy,
//! which is how a Wayland client and its compositor share a buffer.
//!
//! Seals (fcntl F_ADD_SEALS) are enforced here: shrink, grow, write, future
//! write and exec, and SEAL against further seals.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use spin::{Mutex, RwLock};

use super::{seals, shmem::ShmemPages, DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    mm::FrameNumber,
};

pub struct MemfdNode {
    pages: Mutex<ShmemPages>,
    metadata: RwLock<Metadata>,
    seals: AtomicU32,
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
            pages: Mutex::new(ShmemPages::new()),
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

    /// Record the new size and modification time after the pages changed.
    fn touch(&self, size: usize) {
        let mut meta = self.metadata.write();
        meta.size = size;
        meta.modified = crate::arch::timer::get_timestamp_secs();
    }

    fn sealed(&self, which: u32) -> bool {
        self.seals.load(Ordering::Acquire) & which != 0
    }
}

impl VfsNode for MemfdNode {
    /// A memfd lives on Linux's internal tmpfs mount.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, crate::error::KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::TMPFS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::File
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        Ok(self.pages.lock().read(offset, buffer))
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        let end = super::ram_write_end(offset, data.len())?;
        let mut pages = self.pages.lock();
        if self.sealed(seals::WRITE | seals::FUTURE_WRITE)
            || (self.sealed(seals::GROW) && end > pages.size())
        {
            return Err(eperm());
        }
        pages.write(offset, data)?;
        self.touch(pages.size());
        Ok(data.len())
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        let mut pages = self.pages.lock();
        if (self.sealed(seals::SHRINK) && size < pages.size())
            || (self.sealed(seals::GROW) && size > pages.size())
        {
            return Err(eperm());
        }
        pages.resize(size)?;
        self.touch(size);
        Ok(())
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
        if add & seals::WRITE != 0 && current & seals::WRITE == 0 && pages.any_mapped() {
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
        Some(pages.share(first_page, count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eperm<T: core::fmt::Debug>(r: Result<T, KernelError>) -> bool {
        matches!(r, Err(KernelError::FsError(FsError::OperationNotPermitted)))
    }

    // The pages themselves are tested in fs::shmem.

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
