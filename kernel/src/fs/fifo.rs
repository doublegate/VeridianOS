//! Named pipes (FIFOs): the pipe a FIFO inode names, shared by every open
//! of that inode, as Linux's `fifo_open` (fs/pipe.c).
//!
//! - **Open:** read-only waits for a writer and write-only for a reader (the
//!   caller does the waiting, [`FifoEnd::needs_partner`]); with O_NONBLOCK
//!   read-only succeeds at once and write-only with no reader is ENXIO;
//!   read-write never waits.
//! - **I/O:** a read with no writer is end of file, a write with no reader
//!   EPIPE; writes of at most `PIPE_BUF` bytes are atomic.
//! - **Lifetime:** the pipe lives while any end is open; when the last one
//!   closes, unread data is gone, as on Linux.
//!
//! A FIFO is found by its inode: `file_identity` where the filesystem has
//! one (BlockFS), otherwise the node's address, which stays unique because
//! every open end holds the inode.

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use spin::Mutex;

use super::{pipe::PIPE_BUF, DirEntry, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// A FIFO holds as much as an anonymous pipe (Linux: 16 pages).
const FIFO_CAPACITY: usize = 64 * 1024;

/// Wake whoever waits on a FIFO: data, space, an end opened or closed.
fn fifo_event() {
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::io_event();
}

/// Which inode a FIFO belongs to.
type Key = (u64, u64);

fn key_of(inode: &Arc<dyn VfsNode>) -> Key {
    super::inode_key(inode)
}

/// The open FIFOs, by inode. Entries are weak: a FIFO goes away with its
/// last open end.
static FIFOS: Mutex<BTreeMap<Key, Weak<Fifo>>> = Mutex::new(BTreeMap::new());

#[derive(Default)]
struct State {
    buffer: VecDeque<u8>,
    readers: usize,
    writers: usize,
    /// How many times a reader (writer) has opened the FIFO: an opener
    /// waiting for the other end waits for this to change, so a partner
    /// that comes and goes at once still lets it through (Linux's
    /// `r_counter`, `w_counter`).
    read_opens: u64,
    write_opens: u64,
}

/// The pipe of one FIFO inode.
struct Fifo {
    key: Key,
    state: Mutex<State>,
}

impl Drop for Fifo {
    fn drop(&mut self) {
        let mut fifos = FIFOS.lock();
        // A new open may already have put a live FIFO under this key.
        if fifos.get(&self.key).is_some_and(|w| w.strong_count() == 0) {
            fifos.remove(&self.key);
        }
    }
}

/// Forget the FIFO of a freed inode (`fs::special_inode_freed`): its open
/// ends keep their pipe, and the next open of a new file with that key gets
/// a new one.
pub(super) fn forget(key: Key) {
    let stale = FIFOS.lock().remove(&key);
    drop(stale);
}

/// The FIFO of `inode`, made on its first open.
fn attach(inode: &Arc<dyn VfsNode>) -> Arc<Fifo> {
    let key = key_of(inode);
    let mut fifos = FIFOS.lock();
    if let Some(fifo) = fifos.get(&key).and_then(Weak::upgrade) {
        return fifo;
    }
    let fifo = Arc::new(Fifo {
        key,
        state: Mutex::new(State::default()),
    });
    fifos.insert(key, Arc::downgrade(&fifo));
    fifo
}

/// One open of a FIFO: what an open file of a FIFO inode reads and writes.
pub struct FifoEnd {
    fifo: Arc<Fifo>,
    /// The FIFO inode, for stat and the like.
    inode: Arc<dyn VfsNode>,
    read: bool,
    write: bool,
    /// The partner open count this open waits to see change (`None`: it
    /// does not wait).
    waits_for: Option<u64>,
    /// No POLLHUP while the writer open count is still this: a reader
    /// opened without blocking before any writer sees no hangup until one
    /// has come (Linux's `f_pipe`).
    hup_after: u64,
}

impl FifoEnd {
    /// Open FIFO `inode` for reading and/or writing. Write-only with
    /// `nonblock` and no reader is ENXIO. A blocking open of one end with
    /// nobody at the other returns an end whose [`Self::needs_partner`]
    /// is true until somebody opens the other end.
    pub fn open(
        inode: Arc<dyn VfsNode>,
        read: bool,
        write: bool,
        nonblock: bool,
    ) -> Result<Self, KernelError> {
        if !read && !write {
            return Err(KernelError::InvalidArgument {
                name: "flags",
                value: "FIFO opened for neither reading nor writing",
            });
        }
        let fifo = attach(&inode);
        let (waits_for, hup_after) = {
            let mut st = fifo.state.lock();
            if write && !read && nonblock && st.readers == 0 {
                return Err(KernelError::FsError(FsError::NoDeviceOrAddress));
            }
            if read {
                st.readers += 1;
                st.read_opens += 1;
            }
            if write {
                st.writers += 1;
                st.write_opens += 1;
            }
            match (read, write) {
                (true, false) if st.writers == 0 && nonblock => (None, st.write_opens),
                (true, false) if st.writers == 0 => (Some(st.write_opens), 0),
                (false, true) if st.readers == 0 => (Some(st.read_opens), 0),
                _ => (None, 0),
            }
        };
        fifo_event();
        Ok(Self {
            fifo,
            inode,
            read,
            write,
            waits_for,
            hup_after,
        })
    }

    /// Whether this open still waits for the other end to be opened.
    pub fn needs_partner(&self) -> bool {
        let Some(seen) = self.waits_for else {
            return false;
        };
        let st = self.fifo.state.lock();
        let opens = if self.read {
            st.write_opens
        } else {
            st.read_opens
        };
        opens == seen
    }
}

impl Drop for FifoEnd {
    fn drop(&mut self) {
        {
            let mut st = self.fifo.state.lock();
            if self.read {
                st.readers -= 1;
            }
            if self.write {
                st.writers -= 1;
            }
        }
        fifo_event();
    }
}

impl VfsNode for FifoEnd {
    /// The filesystem holding the FIFO's inode.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        self.inode.statfs()
    }

    fn node_type(&self) -> NodeType {
        NodeType::Pipe
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let n = {
            let mut st = self.fifo.state.lock();
            if st.buffer.is_empty() {
                // No writer: end of file, as Linux's pipe_read.
                return if st.writers == 0 {
                    Ok(0)
                } else {
                    Err(KernelError::WouldBlock)
                };
            }
            let n = buffer.len().min(st.buffer.len());
            for (dst, src) in buffer.iter_mut().zip(st.buffer.drain(..n)) {
                *dst = src;
            }
            n
        };
        fifo_event();
        Ok(n)
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        let n = {
            let mut st = self.fifo.state.lock();
            if st.readers == 0 {
                return Err(KernelError::BrokenPipe);
            }
            let room = FIFO_CAPACITY - st.buffer.len();
            if data.len() <= PIPE_BUF && room < data.len() {
                return Err(KernelError::WouldBlock);
            }
            if room == 0 {
                return Err(KernelError::WouldBlock);
            }
            let n = data.len().min(room);
            st.buffer.extend(&data[..n]);
            n
        };
        fifo_event();
        Ok(n)
    }

    /// Linux's `pipe_poll`.
    fn poll_readiness(&self) -> u16 {
        const POLLIN: u16 = 0x001;
        const POLLOUT: u16 = 0x004;
        const POLLERR: u16 = 0x008;
        const POLLHUP: u16 = 0x010;
        let st = self.fifo.state.lock();
        let mut events = 0;
        if self.read {
            if !st.buffer.is_empty() {
                events |= POLLIN;
            }
            if st.writers == 0 && st.write_opens != self.hup_after {
                events |= POLLHUP;
            }
        }
        if self.write {
            if st.buffer.len() < FIFO_CAPACITY {
                events |= POLLOUT;
            }
            if st.readers == 0 {
                events |= POLLERR;
            }
        }
        events
    }

    fn wakes_io_waiters(&self) -> bool {
        true
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        self.inode.metadata()
    }

    fn file_identity(&self) -> Option<(u64, u64)> {
        self.inode.file_identity()
    }

    fn chmod(&self, permissions: Permissions) -> Result<(), KernelError> {
        self.inode.chmod(permissions)
    }

    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        self.inode.chown(uid, gid)
    }

    fn set_times(&self, atime: Option<u64>, mtime: Option<u64>) -> Result<(), KernelError> {
        self.inode.set_times(atime, mtime)
    }

    fn readlink(&self) -> Result<String, KernelError> {
        Err(KernelError::FsError(FsError::NotASymlink))
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(&self, _name: &str, _perms: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn mkdir(&self, _name: &str, _perms: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    /// ftruncate of a FIFO is EINVAL, as Linux's (not a regular file).
    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotAFile))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{ramfs::RamFs, Filesystem};

    fn fifo_inode() -> Arc<dyn VfsNode> {
        RamFs::new()
            .root()
            .mknod("f", NodeType::Pipe, Permissions::from_mode(0o644), 0)
            .unwrap()
    }

    /// BlockFS reuses a freed inode number: a new FIFO given the number of
    /// an unlinked one that is still open gets its own pipe, and the old
    /// one's open ends keep theirs.
    #[test]
    fn new_fifo_on_a_freed_blockfs_number_gets_a_new_pipe() {
        let fs = crate::fs::blockfs::BlockFs::format(1024, 64).unwrap();
        let root = fs.root();
        let perms = Permissions::from_mode(0o644);
        let old = root.mknod("f", NodeType::Pipe, perms, 0).unwrap();
        let old_end = FifoEnd::open(old.clone(), true, true, false).unwrap();
        assert_eq!(old_end.write(0, b"old"), Ok(3));
        root.unlink("f").unwrap();
        let new = root.mknod("g", NodeType::Pipe, perms, 0).unwrap();
        assert_eq!(
            super::super::inode_key(&new),
            super::super::inode_key(&old),
            "the test needs the inode number reused"
        );
        let new_end = FifoEnd::open(new, true, true, false).unwrap();
        let mut b = [0u8; 8];
        assert_eq!(
            new_end.read(0, &mut b),
            Err(KernelError::WouldBlock),
            "empty, with a writer"
        );
        assert_eq!(old_end.read(0, &mut b), Ok(3));
        assert_eq!(&b[..3], b"old");
    }

    #[test]
    fn opens_wait_for_the_other_end() {
        let inode = fifo_inode();
        let r = FifoEnd::open(inode.clone(), true, false, false).unwrap();
        assert!(r.needs_partner(), "a reader waits for a writer");
        let w = FifoEnd::open(inode.clone(), false, true, false).unwrap();
        assert!(!w.needs_partner(), "a reader is there");
        assert!(!r.needs_partner(), "the writer arrived");
        // A writer that comes and goes still releases a waiting reader.
        drop((r, w));
        let r = FifoEnd::open(inode.clone(), true, false, false).unwrap();
        drop(FifoEnd::open(inode.clone(), false, true, false).unwrap());
        assert!(!r.needs_partner());
        // Read-write never waits.
        let rw = FifoEnd::open(inode, true, true, false).unwrap();
        assert!(!rw.needs_partner());
    }

    #[test]
    fn nonblocking_opens_follow_posix() {
        let inode = fifo_inode();
        assert_eq!(
            FifoEnd::open(inode.clone(), false, true, true).err(),
            Some(KernelError::FsError(FsError::NoDeviceOrAddress)),
            "write-only without a reader is ENXIO"
        );
        let r = FifoEnd::open(inode.clone(), true, false, true).unwrap();
        assert!(!r.needs_partner());
        let mut b = [0u8; 4];
        assert_eq!(r.read(0, &mut b), Ok(0), "no writer: end of file");
        assert_eq!(r.poll_readiness() & 0x10, 0, "no hangup before a writer");
        let w = FifoEnd::open(inode, false, true, true).unwrap();
        drop(w);
        assert_ne!(r.poll_readiness() & 0x10, 0, "the writer left: hangup");
    }

    #[test]
    fn data_flows_between_opens_and_dies_with_the_last() {
        let inode = fifo_inode();
        let w = FifoEnd::open(inode.clone(), true, true, false).unwrap();
        let r = FifoEnd::open(inode.clone(), true, false, false).unwrap();
        assert_eq!(w.write(0, b"hello"), Ok(5));
        let mut b = [0u8; 8];
        assert_eq!(r.read(0, &mut b), Ok(5));
        assert_eq!(&b[..5], b"hello");
        assert_eq!(r.read(0, &mut b), Err(KernelError::WouldBlock));
        w.write(0, b"lost").unwrap();
        drop((r, w));
        // Every end closed: the data went with the pipe.
        let r = FifoEnd::open(inode, true, true, false).unwrap();
        assert_eq!(r.read(0, &mut b), Err(KernelError::WouldBlock));
    }

    #[test]
    fn writes_need_a_reader_and_small_ones_are_atomic() {
        let inode = fifo_inode();
        let r = FifoEnd::open(inode.clone(), true, false, true).unwrap();
        let w = FifoEnd::open(inode.clone(), false, true, true).unwrap();
        let big = alloc::vec![7u8; FIFO_CAPACITY - 10];
        assert_eq!(w.write(0, &big), Ok(FIFO_CAPACITY - 10));
        assert_eq!(w.write(0, &[1u8; 11]), Err(KernelError::WouldBlock));
        assert_eq!(w.write(0, &[1u8; 10]), Ok(10));
        assert_eq!(w.poll_readiness() & 0x4, 0, "full");
        drop(r);
        assert_eq!(w.write(0, b"x"), Err(KernelError::BrokenPipe));
        assert_ne!(w.poll_readiness() & 0x8, 0, "no reader: POLLERR");
        assert_eq!(w.metadata().unwrap().node_type, NodeType::Pipe);
    }
}
