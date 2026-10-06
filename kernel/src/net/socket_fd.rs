//! Sockets as file descriptors.
//!
//! A socket lives in its subsystem's table (Unix or INET) under a global
//! id. User space never sees that id: `socket()`, `accept()` and
//! `socketpair()` wrap it in a [`SocketNode`] and install it in the calling
//! process's file table, so a socket is reachable only through an fd its
//! process actually holds, shares the fd namespace with files, and follows
//! the normal `dup`/`fork`/`close` lifetime. The socket is closed when the
//! last open file referring to it is dropped.
//!
//! Previously the global id itself was returned as the "fd": any process
//! could send, receive, poll or close any other process's socket by
//! guessing a small integer, and socket ids collided with real fds (W-14).

use alloc::{sync::Arc, vec::Vec};

use crate::{
    error::{FsError, KernelError},
    fs::{DirEntry, Metadata, NodeType, Permissions, VfsNode},
};

/// Which table a socket lives in, and its id there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketHandle {
    /// `net::unix_socket` id.
    Unix(u64),
    /// `net::socket` id.
    Inet(usize),
}

/// VFS node for an open socket.
pub struct SocketNode {
    handle: SocketHandle,
}

impl SocketNode {
    pub fn new(handle: SocketHandle) -> Self {
        Self { handle }
    }

    pub fn handle(&self) -> SocketHandle {
        self.handle
    }

    /// Send `data`; for Unix sockets optionally with passed files.
    pub fn send(
        &self,
        data: &[u8],
        rights: Option<super::unix_socket::ScmRights>,
    ) -> Result<usize, KernelError> {
        match self.handle {
            SocketHandle::Unix(id) => super::unix_socket::socket_send(id, data, rights),
            SocketHandle::Inet(id) => super::socket::with_socket_mut(id, |s| s.send(data, 0))?,
        }
    }

    /// Receive into `buf`, with any files passed alongside the data.
    pub fn recv(
        &self,
        buf: &mut [u8],
    ) -> Result<(usize, Option<super::unix_socket::ScmRights>), KernelError> {
        match self.handle {
            SocketHandle::Unix(id) => super::unix_socket::socket_recv(id, buf),
            SocketHandle::Inet(id) => {
                super::socket::with_socket_mut(id, |s| s.recv(buf, 0))?.map(|n| (n, None))
            }
        }
    }
}

impl Drop for SocketNode {
    fn drop(&mut self) {
        let _ = match self.handle {
            SocketHandle::Unix(id) => super::unix_socket::socket_close(id),
            SocketHandle::Inet(id) => super::socket::close_socket(id),
        };
    }
}

impl VfsNode for SocketNode {
    fn node_type(&self) -> NodeType {
        NodeType::Socket
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        // Files passed with plain read() are discarded, as on Linux.
        self.recv(buffer).map(|(n, _)| n)
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.send(data, None)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            node_type: NodeType::Socket,
            size: 0,
            permissions: Permissions::from_mode(0o777),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
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
        Err(KernelError::FsError(FsError::NotSupported))
    }

    fn poll_readiness(&self) -> u16 {
        match self.handle {
            SocketHandle::Unix(id) => super::unix_socket::socket_poll_readiness(id),
            // INET sockets have no readiness query yet; report writable so
            // poll() does not hang on them.
            SocketHandle::Inet(_) => 0x0004,
        }
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}
