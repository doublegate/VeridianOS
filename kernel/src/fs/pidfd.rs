//! pidfd: a file that names a process (or, with PIDFD_THREAD, a thread),
//! as Linux 7.3's `fs/pidfs.c` and `kernel/pid.c`.
//!
//! A pidfd holds its process weakly and by number: the process is reaped
//! when it leaves the process table, which a later process with the same
//! number (pids are not reused while the counter runs) cannot be taken
//! for. It polls readable (POLLIN | POLLRDNORM) once its process has
//! exited -- the whole thread group, or the one thread -- and hung up
//! (POLLHUP too) once it is reaped. Reads and writes are EINVAL, as Linux's
//! pidfs file has neither.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::{
    error::{FsError, KernelError},
    process::{
        pcb::{Process, ProcessState},
        thread::ThreadState,
        ProcessId, ThreadId,
    },
};

/// `PIDFD_NONBLOCK`: O_NONBLOCK.
pub const PIDFD_NONBLOCK: u32 = 0o4000;
/// `PIDFD_THREAD`: O_EXCL; the pidfd names one thread.
pub const PIDFD_THREAD: u32 = 0o200;

const POLLIN: u16 = 0x0001;
const POLLHUP: u16 = 0x0010;
const POLLRDNORM: u16 = 0x0040;

/// Where a pidfd's process (or thread) is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Life {
    Running,
    /// Exited, not yet reaped (a zombie; for a thread, exited).
    Exited,
    /// Gone.
    Reaped,
}

/// The file a pidfd is.
pub struct PidFdNode {
    pid: ProcessId,
    /// PIDFD_THREAD: the thread this pidfd names.
    tid: Option<ThreadId>,
    process: Weak<Process>,
}

impl PidFdNode {
    /// A pidfd for `process`, or for its thread `tid`.
    pub fn new(process: &Arc<Process>, tid: Option<ThreadId>) -> Self {
        Self {
            pid: process.pid,
            tid,
            process: Arc::downgrade(process),
        }
    }

    pub fn pid(&self) -> ProcessId {
        self.pid
    }

    /// The thread a PIDFD_THREAD pidfd names.
    pub fn thread(&self) -> Option<ThreadId> {
        self.tid
    }

    /// The process, unless it has been reaped.
    pub fn process(&self) -> Option<Arc<Process>> {
        let process = self.process.upgrade()?;
        crate::process::table::get_process(self.pid).filter(|p| Arc::ptr_eq(p, &process))
    }

    pub fn life(&self) -> Life {
        let Some(process) = self.process() else {
            return Life::Reaped;
        };
        match self.tid {
            None => match process.get_state() {
                ProcessState::Zombie | ProcessState::Dead => Life::Exited,
                _ => Life::Running,
            },
            Some(tid) => match process.get_thread(tid).map(|t| t.get_state()) {
                None => Life::Reaped,
                Some(ThreadState::Zombie | ThreadState::Dead) => Life::Exited,
                Some(_) => Life::Running,
            },
        }
    }
}

/// What a pidfd in `life` polls as.
pub fn readiness(life: Life) -> u16 {
    match life {
        Life::Running => 0,
        Life::Exited => POLLIN | POLLRDNORM,
        Life::Reaped => POLLIN | POLLRDNORM | POLLHUP,
    }
}

impl VfsNode for PidFdNode {
    /// pidfs, as on Linux.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::PID_FS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a pidfd cannot be read",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a pidfd cannot be written",
        })
    }

    fn poll_readiness(&self) -> u16 {
        readiness(self.life())
    }

    fn wakes_io_waiters(&self) -> bool {
        true // a process becoming a zombie, and its reaping, call io_event
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o600),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: self.pid.0,
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
            value: "a pidfd cannot be truncated",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_follows_the_life_of_the_process() {
        assert_eq!(readiness(Life::Running), 0);
        assert_eq!(readiness(Life::Exited), POLLIN | POLLRDNORM);
        assert_eq!(readiness(Life::Reaped), POLLIN | POLLRDNORM | POLLHUP);
    }
}
