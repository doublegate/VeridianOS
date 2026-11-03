//! nsfs: the file a namespace is, as Linux 7.3's `fs/nsfs.c` (ADR 0022).
//! `/proc/<pid>/ns/*` links open one, and NS_GET_USERNS returns one; setns
//! joins the namespace it names and its ioctls (`syscall::namespaces`)
//! describe it. Its inode number is the namespace's (so `stat -L
//! /proc/self/ns/uts` shows 4026531838 for the initial UTS namespace), it
//! reads as EINVAL, and statfs reports NSFS_MAGIC.

use alloc::{sync::Arc, vec::Vec};

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    process::ns::NsRef,
};

/// An open namespace.
pub struct NsFileNode {
    ns: NsRef,
}

impl NsFileNode {
    pub fn new(ns: NsRef) -> Self {
        Self { ns }
    }

    pub fn ns(&self) -> &NsRef {
        &self.ns
    }
}

impl VfsNode for NsFileNode {
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::NSFS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::File
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a namespace file cannot be read",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a namespace file cannot be written",
        })
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::File,
            permissions: Permissions::from_mode(0o444),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: self.ns.ident().inum as u64,
        })
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

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a namespace file cannot be truncated",
        })
    }
}
