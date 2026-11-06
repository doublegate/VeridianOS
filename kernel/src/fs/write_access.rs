//! Text-busy accounting (ETXTBSY), as Linux's `i_writecount`.
//!
//! Each regular file has one count: positive while it is open for writing
//! (a writable open, or a truncate by name), negative while it is being or
//! has been executed. Taking write access to a file that is denied, or
//! denying one that is open for writing, fails with
//! [`FsError::TextBusy`]. The guards keep the node alive and give their
//! count back when dropped.
//!
//! Who holds what, as on Linux (`include/linux/fs.h`, `fs/exec.c`,
//! `kernel/fork.c`):
//!
//! - an open file opened for writing holds a [`WriteAccess`] until its last
//!   close, and `truncate` holds one for the truncate;
//! - exec denies writes to the program, a `#!` script and the interpreter while
//!   it runs, and the process keeps the program's [`WriteDenial`] (its
//!   `mm->exe_file`) until it exits or execs again; fork copies it.

use alloc::{collections::BTreeMap, sync::Arc};

use spin::Mutex;

use super::{NodeType, VfsNode};
use crate::error::{FsError, KernelError};

/// Writers (> 0) or denials (< 0) per file key; files at 0 are absent.
static COUNTS: Mutex<BTreeMap<u64, i32>> = Mutex::new(BTreeMap::new());

/// The key of `node`'s file: its (filesystem, inode) identity where the
/// filesystem has one, else the node's address, which stays the file's
/// while a guard holds the node. Two opens of one BlockFS file get
/// different node objects with one identity.
pub fn node_key(node: &Arc<dyn VfsNode>) -> u64 {
    match node.file_identity() {
        Some((fs, ino)) => fs ^ ino.wrapping_mul(0x9E37_79B9_7F4A_7C15),
        None => Arc::as_ptr(node) as *const () as u64,
    }
}

/// Whether `node` takes part: regular files only, as Linux leaves devices,
/// FIFOs and sockets out (`special_file`) and nothing else can be opened
/// for writing or executed.
fn counted(node: &Arc<dyn VfsNode>) -> bool {
    matches!(node.node_type(), NodeType::File)
}

/// Add `step` (+1 for a writer, -1 for a denial) to `key`'s count, unless
/// the count has the other sign.
fn adjust(key: u64, step: i32) -> Result<(), KernelError> {
    let mut counts = COUNTS.lock();
    let count = counts.entry(key).or_insert(0);
    if *count != 0 && (*count > 0) != (step > 0) {
        return Err(KernelError::FsError(FsError::TextBusy));
    }
    *count += step;
    Ok(())
}

/// Give back `step` taken from `key`.
fn release(key: u64, step: i32) {
    let mut counts = COUNTS.lock();
    if let Some(count) = counts.get_mut(&key) {
        *count -= step;
        if *count == 0 {
            counts.remove(&key);
        }
    }
}

/// Write access to a file (Linux's `get_write_access`), given back when
/// dropped.
pub struct WriteAccess {
    _node: Arc<dyn VfsNode>,
    key: Option<u64>,
}

impl WriteAccess {
    /// Write access to `node`, or `TextBusy` while it is being executed.
    /// Files other than regular ones are not counted and always succeed.
    pub fn get(node: &Arc<dyn VfsNode>) -> Result<Self, KernelError> {
        let key = counted(node).then(|| node_key(node));
        if let Some(key) = key {
            adjust(key, 1)?;
        }
        Ok(Self {
            _node: node.clone(),
            key,
        })
    }
}

impl Drop for WriteAccess {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            release(key, 1);
        }
    }
}

/// A denial of write access to a file being executed (Linux's
/// `exe_file_deny_write_access`), lifted when dropped. A clone is one more
/// denial (fork copying the program's).
pub struct WriteDenial {
    node: Arc<dyn VfsNode>,
    key: Option<u64>,
}

impl WriteDenial {
    /// Deny writes to `node`, or `TextBusy` while it is open for writing.
    pub fn deny(node: &Arc<dyn VfsNode>) -> Result<Self, KernelError> {
        let key = counted(node).then(|| node_key(node));
        if let Some(key) = key {
            adjust(key, -1)?;
        }
        Ok(Self {
            node: node.clone(),
            key,
        })
    }
}

impl Clone for WriteDenial {
    fn clone(&self) -> Self {
        if let Some(key) = self.key {
            // Already denied by `self`, so the count is negative and this
            // cannot fail.
            let _ = adjust(key, -1);
        }
        Self {
            node: self.node.clone(),
            key: self.key,
        }
    }
}

impl Drop for WriteDenial {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            release(key, -1);
        }
    }
}

impl core::fmt::Debug for WriteDenial {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WriteDenial")
            .field("key", &self.key)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::{ramfs::RamFs, Filesystem, Permissions};

    fn file(fs: &RamFs, name: &str) -> Arc<dyn VfsNode> {
        fs.root()
            .create(name, Permissions::from_mode(0o755))
            .unwrap()
    }

    fn busy(r: Result<impl Sized, KernelError>) -> bool {
        matches!(r, Err(KernelError::FsError(FsError::TextBusy)))
    }

    #[test]
    fn writers_and_executions_exclude_each_other() {
        let fs = RamFs::new();
        let f = file(&fs, "prog");

        // Open for writing: no exec.
        let w1 = WriteAccess::get(&f).unwrap();
        let w2 = WriteAccess::get(&f).unwrap();
        assert!(busy(WriteDenial::deny(&f)));
        drop(w1);
        assert!(busy(WriteDenial::deny(&f)));
        drop(w2);

        // Running, twice over (fork): no writer until both are gone.
        let d1 = WriteDenial::deny(&f).unwrap();
        let d2 = d1.clone();
        assert!(busy(WriteAccess::get(&f)));
        drop(d1);
        assert!(busy(WriteAccess::get(&f)));
        drop(d2);
        let w = WriteAccess::get(&f).unwrap();
        drop(w);
        assert!(!COUNTS.lock().contains_key(&node_key(&f)));
    }

    #[test]
    fn only_regular_files_are_counted() {
        let fs = RamFs::new();
        let f = file(&fs, "prog");
        let other = file(&fs, "other");
        let dir = fs.root().mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let _deny = WriteDenial::deny(&f).unwrap();
        // Another file is not affected, and a directory is not counted.
        let _w = WriteAccess::get(&other).unwrap();
        let _dd = WriteDenial::deny(&dir).unwrap();
        let _dw = WriteAccess::get(&dir).unwrap();
        assert!(!COUNTS.lock().contains_key(&node_key(&dir)));
    }
}
