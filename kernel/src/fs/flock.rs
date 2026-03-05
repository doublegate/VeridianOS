//! POSIX File Locking (flock/fcntl)
//!
//! Implements both whole-file locking (flock semantics) and byte-range
//! locking (fcntl/POSIX semantics). Per-inode lock tables track ownership
//! by PID, with deadlock detection for blocking lock requests.
//!
//! - `flock()`: whole-file advisory locks (LOCK_SH, LOCK_EX, LOCK_UN)
//! - `fcntl_setlk()`: byte-range locks (F_SETLK semantics, non-blocking)
//! - `fcntl_setlkw()`: byte-range locks (F_SETLKW semantics, blocking with
//!   deadlock detection)
//! - `fcntl_getlk()`: query conflicting locks (F_GETLK)
//! - `cleanup_process_locks()`: release all locks held by a process on exit

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// POSIX flock() constants
// ---------------------------------------------------------------------------

/// Shared (read) lock
pub const LOCK_SH: u32 = 1;
/// Exclusive (write) lock
pub const LOCK_EX: u32 = 2;
/// Non-blocking flag (OR with LOCK_SH or LOCK_EX)
pub const LOCK_NB: u32 = 4;
/// Unlock
pub const LOCK_UN: u32 = 8;

// ---------------------------------------------------------------------------
// POSIX fcntl() lock type constants
// ---------------------------------------------------------------------------

/// Read (shared) lock
pub const F_RDLCK: u16 = 0;
/// Write (exclusive) lock
pub const F_WRLCK: u16 = 1;
/// Unlock
pub const F_UNLCK: u16 = 2;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A POSIX byte-range lock (used by fcntl F_SETLK / F_GETLK).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileLock {
    /// Lock type: F_RDLCK, F_WRLCK, or F_UNLCK.
    pub lock_type: u16,
    /// Starting byte offset of the locked region.
    pub start: u64,
    /// Length of the locked region (0 means "to end of file").
    pub len: u64,
    /// PID of the lock owner.
    pub pid: u64,
}

impl FileLock {
    /// Compute the exclusive end offset of this lock region.
    /// A length of 0 means "to end of file", represented as u64::MAX.
    fn end(&self) -> u64 {
        if self.len == 0 {
            u64::MAX
        } else {
            self.start.saturating_add(self.len)
        }
    }

    /// Check whether two lock regions overlap.
    fn overlaps(&self, other: &FileLock) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

/// A whole-file advisory lock entry (flock semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlockEntry {
    /// Lock type: LOCK_SH or LOCK_EX.
    pub lock_type: u32,
    /// PID of the lock owner.
    pub pid: u64,
}

/// Per-system lock table tracking all file locks by inode.
struct LockTable {
    /// Whole-file locks (flock): inode -> list of FlockEntry
    flock_locks: BTreeMap<u64, Vec<FlockEntry>>,
    /// Byte-range locks (fcntl): inode -> list of FileLock
    range_locks: BTreeMap<u64, Vec<FileLock>>,
}

impl LockTable {
    const fn new() -> Self {
        Self {
            flock_locks: BTreeMap::new(),
            range_locks: BTreeMap::new(),
        }
    }
}

/// Global lock table, protected by a reader-writer lock.
static LOCK_TABLE: RwLock<LockTable> = RwLock::new(LockTable::new());

// ---------------------------------------------------------------------------
// flock() implementation
// ---------------------------------------------------------------------------

/// Apply a whole-file advisory lock (flock semantics).
///
/// `operation` is a combination of LOCK_SH/LOCK_EX/LOCK_UN and optionally
/// LOCK_NB. Returns `WouldBlock` when LOCK_NB is set and the lock cannot
/// be acquired immediately.
pub fn flock(inode: u64, pid: u64, operation: u32) -> Result<(), KernelError> {
    let op = operation & !LOCK_NB;
    let non_blocking = (operation & LOCK_NB) != 0;

    match op {
        LOCK_UN => flock_unlock(inode, pid),
        LOCK_SH => flock_lock(inode, pid, LOCK_SH, non_blocking),
        LOCK_EX => flock_lock(inode, pid, LOCK_EX, non_blocking),
        _ => Err(KernelError::InvalidArgument {
            name: "operation",
            value: "invalid flock operation",
        }),
    }
}

/// Acquire a whole-file lock (shared or exclusive).
fn flock_lock(inode: u64, pid: u64, lock_type: u32, non_blocking: bool) -> Result<(), KernelError> {
    let mut table = LOCK_TABLE.write();
    let entries = table.flock_locks.entry(inode).or_default();

    // Check for conflicts with existing locks.
    for entry in entries.iter() {
        // Same process can upgrade/downgrade its own lock.
        if entry.pid == pid {
            continue;
        }
        // Shared locks conflict only with exclusive requests.
        // Exclusive locks conflict with any other lock.
        let conflict = lock_type == LOCK_EX || entry.lock_type == LOCK_EX;
        if conflict {
            if non_blocking {
                return Err(KernelError::WouldBlock);
            }
            // In a real kernel we would block here. For now, return WouldBlock
            // since we have no wait-queue infrastructure wired for flock.
            return Err(KernelError::WouldBlock);
        }
    }

    // Remove any existing lock from this PID (upgrade/downgrade).
    entries.retain(|e| e.pid != pid);

    // Insert the new lock.
    entries.push(FlockEntry { lock_type, pid });
    Ok(())
}

/// Release a whole-file lock held by the given PID.
fn flock_unlock(inode: u64, pid: u64) -> Result<(), KernelError> {
    let mut table = LOCK_TABLE.write();
    if let Some(entries) = table.flock_locks.get_mut(&inode) {
        entries.retain(|e| e.pid != pid);
        if entries.is_empty() {
            table.flock_locks.remove(&inode);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// fcntl() range lock implementation
// ---------------------------------------------------------------------------

/// Check whether a proposed range lock conflicts with any existing lock.
///
/// Returns `true` if there is a conflict (i.e., the new lock cannot be placed).
fn check_range_conflict(existing: &[FileLock], new_lock: &FileLock) -> bool {
    for lock in existing {
        // Same owner can always re-lock.
        if lock.pid == new_lock.pid {
            continue;
        }
        // Locks must overlap to conflict.
        if !lock.overlaps(new_lock) {
            continue;
        }
        // Two read locks never conflict.
        if lock.lock_type == F_RDLCK && new_lock.lock_type == F_RDLCK {
            continue;
        }
        // At least one write lock and overlapping => conflict.
        return true;
    }
    false
}

/// Find the first existing lock that conflicts with the proposed lock.
///
/// Returns `Some(conflicting_lock)` if a conflict exists, `None` otherwise.
fn find_conflicting_lock(existing: &[FileLock], query: &FileLock) -> Option<FileLock> {
    for lock in existing {
        if lock.pid == query.pid {
            continue;
        }
        if !lock.overlaps(query) {
            continue;
        }
        if lock.lock_type == F_RDLCK && query.lock_type == F_RDLCK {
            continue;
        }
        return Some(*lock);
    }
    None
}

/// Set a byte-range lock (F_SETLK semantics -- non-blocking).
///
/// If `lock.lock_type` is F_UNLCK, removes matching locks from the table.
/// Otherwise, checks for conflicts and inserts the lock if none exist.
pub fn fcntl_setlk(inode: u64, lock: &FileLock) -> Result<(), KernelError> {
    if lock.lock_type == F_UNLCK {
        return fcntl_unlock(inode, lock);
    }

    if lock.lock_type != F_RDLCK && lock.lock_type != F_WRLCK {
        return Err(KernelError::InvalidArgument {
            name: "lock_type",
            value: "must be F_RDLCK, F_WRLCK, or F_UNLCK",
        });
    }

    let mut table = LOCK_TABLE.write();
    let locks = table.range_locks.entry(inode).or_default();

    if check_range_conflict(locks, lock) {
        return Err(KernelError::WouldBlock);
    }

    // Merge / replace: remove any existing locks from the same PID that
    // overlap with the new lock, then insert the new one.
    let pid = lock.pid;
    let new_start = lock.start;
    let new_end = lock.end();
    locks.retain(|existing| {
        if existing.pid != pid {
            return true;
        }
        // Keep non-overlapping locks from the same owner.
        !(existing.start < new_end && new_start < existing.end())
    });

    locks.push(*lock);
    Ok(())
}

/// Set a byte-range lock with blocking (F_SETLKW semantics).
///
/// Performs simple cycle-based deadlock detection before blocking. If a
/// deadlock is detected, returns a deadlock error. Since we lack a full
/// wait-queue, this currently falls back to `WouldBlock` when the lock
/// cannot be acquired and no deadlock is detected.
pub fn fcntl_setlkw(inode: u64, lock: &FileLock) -> Result<(), KernelError> {
    if lock.lock_type == F_UNLCK {
        return fcntl_unlock(inode, lock);
    }

    if lock.lock_type != F_RDLCK && lock.lock_type != F_WRLCK {
        return Err(KernelError::InvalidArgument {
            name: "lock_type",
            value: "must be F_RDLCK, F_WRLCK, or F_UNLCK",
        });
    }

    let mut table = LOCK_TABLE.write();
    let locks = table.range_locks.entry(inode).or_default();

    if !check_range_conflict(locks, lock) {
        // No conflict -- place the lock immediately.
        let pid = lock.pid;
        let new_start = lock.start;
        let new_end = lock.end();
        locks.retain(|existing| {
            if existing.pid != pid {
                return true;
            }
            !(existing.start < new_end && new_start < existing.end())
        });
        locks.push(*lock);
        return Ok(());
    }

    // Conflict exists. Check for deadlocks by seeing if any of the holders
    // we would wait on are themselves waiting on us (simple cycle detection
    // across all inodes).
    if detect_deadlock(&table, lock) {
        return Err(KernelError::InvalidState {
            expected: "no deadlock",
            actual: "deadlock detected",
        });
    }

    // In a real kernel we would sleep on a wait-queue. For now, return
    // WouldBlock since no scheduler integration is wired for file locks.
    Err(KernelError::WouldBlock)
}

/// Simple deadlock detector.
///
/// Starting from `requester`, walks the "waits-for" graph: if process A
/// requests a lock that conflicts with a lock held by process B, then A
/// waits-for B. If following this chain leads back to A, a deadlock exists.
///
/// This is O(P * L) where P is the number of processes in the chain and L
/// is the total number of range locks, which is acceptable for the expected
/// lock counts in practice.
fn detect_deadlock(table: &LockTable, request: &FileLock) -> bool {
    let requester_pid = request.pid;
    let mut visited = Vec::new();
    let mut frontier = Vec::new();

    // Seed: find all PIDs that hold conflicting locks with our request.
    for locks in table.range_locks.values() {
        for lock in locks {
            if lock.pid == requester_pid {
                continue;
            }
            if !lock.overlaps(request) {
                continue;
            }
            if lock.lock_type == F_RDLCK && request.lock_type == F_RDLCK {
                continue;
            }
            if !frontier.contains(&lock.pid) {
                frontier.push(lock.pid);
            }
        }
    }

    // BFS: for each PID in the frontier, check if any of the locks they
    // are waiting for are held by the original requester.
    while let Some(blocker_pid) = frontier.pop() {
        if blocker_pid == requester_pid {
            return true; // Cycle detected.
        }
        if visited.contains(&blocker_pid) {
            continue;
        }
        visited.push(blocker_pid);

        // Find all locks held by processes that would conflict with
        // any lock request from `blocker_pid`. We approximate: if
        // `blocker_pid` holds a write lock, any other write or read
        // lock holder on the same range is potentially blocking it
        // (and vice-versa). We look at all lock holders that conflict
        // with any lock `blocker_pid` holds.
        for locks in table.range_locks.values() {
            let blocker_locks: Vec<&FileLock> =
                locks.iter().filter(|l| l.pid == blocker_pid).collect();
            for bl in &blocker_locks {
                for other in locks {
                    if other.pid == blocker_pid || visited.contains(&other.pid) {
                        continue;
                    }
                    if !other.overlaps(bl) {
                        continue;
                    }
                    if other.lock_type == F_RDLCK && bl.lock_type == F_RDLCK {
                        continue;
                    }
                    if !frontier.contains(&other.pid) {
                        frontier.push(other.pid);
                    }
                }
            }
        }
    }

    false
}

/// Unlock byte-range locks matching the given region for the owning PID.
fn fcntl_unlock(inode: u64, lock: &FileLock) -> Result<(), KernelError> {
    let mut table = LOCK_TABLE.write();
    if let Some(locks) = table.range_locks.get_mut(&inode) {
        let unlock_start = lock.start;
        let unlock_end = lock.end();

        locks.retain(|existing| {
            if existing.pid != lock.pid {
                return true;
            }
            // Remove locks that fall entirely within the unlock region.
            !(existing.start >= unlock_start && existing.end() <= unlock_end)
        });

        if locks.is_empty() {
            table.range_locks.remove(&inode);
        }
    }
    Ok(())
}

/// Query for a conflicting lock (F_GETLK semantics).
///
/// If a lock exists that would conflict with the described lock, returns
/// `Some(conflicting_lock)`. Otherwise returns `None`, indicating the
/// lock could be placed.
pub fn fcntl_getlk(inode: u64, lock: &FileLock) -> Result<Option<FileLock>, KernelError> {
    if lock.lock_type != F_RDLCK && lock.lock_type != F_WRLCK {
        return Err(KernelError::InvalidArgument {
            name: "lock_type",
            value: "must be F_RDLCK or F_WRLCK for F_GETLK",
        });
    }

    let table = LOCK_TABLE.read();
    let locks = match table.range_locks.get(&inode) {
        Some(l) => l,
        None => return Ok(None),
    };

    Ok(find_conflicting_lock(locks, lock))
}

// ---------------------------------------------------------------------------
// Process cleanup
// ---------------------------------------------------------------------------

/// Remove all locks (flock and range) held by the specified PID.
///
/// Called during process exit to prevent leaked locks.
pub fn cleanup_process_locks(pid: u64) {
    let mut table = LOCK_TABLE.write();

    // Clean up flock entries.
    let empty_flock_inodes: Vec<u64> = table
        .flock_locks
        .iter_mut()
        .filter_map(|(&inode, entries)| {
            entries.retain(|e| e.pid != pid);
            if entries.is_empty() {
                Some(inode)
            } else {
                None
            }
        })
        .collect();
    for inode in empty_flock_inodes {
        table.flock_locks.remove(&inode);
    }

    // Clean up range locks.
    let empty_range_inodes: Vec<u64> = table
        .range_locks
        .iter_mut()
        .filter_map(|(&inode, locks)| {
            locks.retain(|l| l.pid != pid);
            if locks.is_empty() {
                Some(inode)
            } else {
                None
            }
        })
        .collect();
    for inode in empty_range_inodes {
        table.range_locks.remove(&inode);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Atomic counter to generate unique inode IDs per test, avoiding
    /// cross-test interference when tests run in parallel on the shared
    /// global LOCK_TABLE.
    static NEXT_TEST_INODE: AtomicU64 = AtomicU64::new(0x1_0000_0000);

    /// Allocate `n` unique inode IDs for a single test.
    fn unique_inodes(n: usize) -> Vec<u64> {
        let base = NEXT_TEST_INODE.fetch_add(n as u64, Ordering::Relaxed);
        (0..n as u64).map(|i| base + i).collect()
    }

    // ---- flock tests ----

    #[test]
    fn test_flock_shared_compatible() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_SH).is_ok());
        assert!(flock(ids[0], 200, LOCK_SH).is_ok());
    }

    #[test]
    fn test_flock_exclusive_blocks_shared() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_EX).is_ok());
        assert_eq!(
            flock(ids[0], 200, LOCK_SH | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
    }

    #[test]
    fn test_flock_exclusive_blocks_exclusive() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_EX).is_ok());
        assert_eq!(
            flock(ids[0], 200, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
    }

    #[test]
    fn test_flock_shared_blocks_exclusive() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_SH).is_ok());
        assert_eq!(
            flock(ids[0], 200, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
    }

    #[test]
    fn test_flock_same_pid_upgrade() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_SH).is_ok());
        // Same PID can upgrade to exclusive.
        assert!(flock(ids[0], 100, LOCK_EX).is_ok());
    }

    #[test]
    fn test_flock_unlock() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_EX).is_ok());
        assert!(flock(ids[0], 100, LOCK_UN).is_ok());
        // Now another process can lock.
        assert!(flock(ids[0], 200, LOCK_EX).is_ok());
    }

    #[test]
    fn test_flock_different_inodes_independent() {
        let ids = unique_inodes(2);
        assert!(flock(ids[0], 100, LOCK_EX).is_ok());
        assert!(flock(ids[1], 200, LOCK_EX).is_ok());
    }

    #[test]
    fn test_flock_invalid_operation() {
        let ids = unique_inodes(1);
        let result = flock(ids[0], 100, 0);
        assert!(result.is_err());
    }

    // ---- fcntl range lock tests ----

    #[test]
    fn test_fcntl_read_locks_compatible() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_RDLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        let l2 = FileLock {
            lock_type: F_RDLCK,
            start: 50,
            len: 100,
            pid: 200,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        assert!(fcntl_setlk(ids[0], &l2).is_ok());
    }

    #[test]
    fn test_fcntl_write_conflicts_read() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_RDLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 50,
            len: 100,
            pid: 200,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        assert_eq!(fcntl_setlk(ids[0], &l2), Err(KernelError::WouldBlock));
    }

    #[test]
    fn test_fcntl_write_conflicts_write() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 50,
            len: 50,
            pid: 200,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        assert_eq!(fcntl_setlk(ids[0], &l2), Err(KernelError::WouldBlock));
    }

    #[test]
    fn test_fcntl_non_overlapping_no_conflict() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 100,
            len: 100,
            pid: 200,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        assert!(fcntl_setlk(ids[0], &l2).is_ok());
    }

    #[test]
    fn test_fcntl_same_pid_can_relock() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        let l2 = FileLock {
            lock_type: F_RDLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        // Same PID can downgrade.
        assert!(fcntl_setlk(ids[0], &l2).is_ok());
    }

    #[test]
    fn test_fcntl_unlock_region() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        let unlock = FileLock {
            lock_type: F_UNLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlk(ids[0], &unlock).is_ok());
        // Now another process can lock.
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 200,
        };
        assert!(fcntl_setlk(ids[0], &l2).is_ok());
    }

    #[test]
    fn test_fcntl_zero_len_means_eof() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 0,
            pid: 100,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        // Any range should conflict.
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 99999,
            len: 1,
            pid: 200,
        };
        assert_eq!(fcntl_setlk(ids[0], &l2), Err(KernelError::WouldBlock));
    }

    // ---- fcntl_getlk tests ----

    #[test]
    fn test_getlk_no_conflict() {
        let ids = unique_inodes(1);
        let query = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert_eq!(fcntl_getlk(ids[0], &query).unwrap(), None);
    }

    #[test]
    fn test_getlk_returns_conflicting_lock() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        let query = FileLock {
            lock_type: F_WRLCK,
            start: 50,
            len: 50,
            pid: 200,
        };
        let result = fcntl_getlk(ids[0], &query).unwrap();
        assert!(result.is_some());
        let conflict = result.unwrap();
        assert_eq!(conflict.pid, 100);
        assert_eq!(conflict.lock_type, F_WRLCK);
    }

    #[test]
    fn test_getlk_invalid_type() {
        let ids = unique_inodes(1);
        let query = FileLock {
            lock_type: F_UNLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_getlk(ids[0], &query).is_err());
    }

    // ---- setlkw + deadlock detection tests ----

    #[test]
    fn test_setlkw_no_conflict_succeeds() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlkw(ids[0], &l1).is_ok());
    }

    #[test]
    fn test_setlkw_conflict_returns_would_block() {
        let ids = unique_inodes(1);
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 100,
        };
        assert!(fcntl_setlkw(ids[0], &l1).is_ok());
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 200,
        };
        // No deadlock, just a simple conflict -> WouldBlock.
        assert_eq!(fcntl_setlkw(ids[0], &l2), Err(KernelError::WouldBlock));
    }

    #[test]
    fn test_setlkw_deadlock_detection() {
        let ids = unique_inodes(2);
        // Use unique PIDs to avoid interference with other tests.
        let pid_a: u64 = 9000;
        let pid_b: u64 = 9001;

        // PID A holds [0..100) on inode ids[0].
        let la = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: pid_a,
        };
        assert!(fcntl_setlk(ids[0], &la).is_ok());
        // PID B holds [0..100) on inode ids[1].
        let lb = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: pid_b,
        };
        assert!(fcntl_setlk(ids[1], &lb).is_ok());

        // PID B requests a write lock on inode ids[0] [0..100) (held by PID A).
        // PID A does not hold anything on inode ids[1], so there is no deadlock cycle.
        let req = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: pid_b,
        };
        // Should be WouldBlock, not deadlock.
        assert_eq!(fcntl_setlkw(ids[0], &req), Err(KernelError::WouldBlock));
    }

    // ---- cleanup tests ----

    #[test]
    fn test_cleanup_process_locks_flock() {
        let ids = unique_inodes(2);
        // Use unique PIDs.
        let pid_owner: u64 = 7000;
        let pid_other: u64 = 7001;
        assert!(flock(ids[0], pid_owner, LOCK_EX).is_ok());
        assert!(flock(ids[1], pid_owner, LOCK_SH).is_ok());
        cleanup_process_locks(pid_owner);
        // Both inodes should now be unlocked.
        assert!(flock(ids[0], pid_other, LOCK_EX).is_ok());
        assert!(flock(ids[1], pid_other, LOCK_EX).is_ok());
    }

    #[test]
    fn test_cleanup_process_locks_range() {
        let ids = unique_inodes(1);
        let pid_owner: u64 = 7100;
        let pid_other: u64 = 7101;
        let l1 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: pid_owner,
        };
        assert!(fcntl_setlk(ids[0], &l1).is_ok());
        cleanup_process_locks(pid_owner);
        let l2 = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: pid_other,
        };
        assert!(fcntl_setlk(ids[0], &l2).is_ok());
    }

    #[test]
    fn test_cleanup_does_not_affect_other_pids() {
        let ids = unique_inodes(2);
        let pid_a: u64 = 7200;
        let pid_b: u64 = 7201;
        let pid_c: u64 = 7202;
        assert!(flock(ids[0], pid_a, LOCK_EX).is_ok());
        assert!(flock(ids[1], pid_b, LOCK_EX).is_ok());
        cleanup_process_locks(pid_a);
        // PID B's lock on ids[1] should still be in place.
        assert_eq!(
            flock(ids[1], pid_c, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
    }

    // ---- FileLock overlap tests ----

    #[test]
    fn test_filelock_overlap_basic() {
        let a = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 100,
            pid: 1,
        };
        let b = FileLock {
            lock_type: F_WRLCK,
            start: 50,
            len: 100,
            pid: 2,
        };
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));
    }

    #[test]
    fn test_filelock_no_overlap() {
        let a = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 50,
            pid: 1,
        };
        let b = FileLock {
            lock_type: F_WRLCK,
            start: 50,
            len: 50,
            pid: 2,
        };
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn test_filelock_zero_len_overlaps_everything() {
        let whole = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 0,
            pid: 1,
        };
        let small = FileLock {
            lock_type: F_WRLCK,
            start: 999999,
            len: 1,
            pid: 2,
        };
        assert!(whole.overlaps(&small));
        assert!(small.overlaps(&whole));
    }

    #[test]
    fn test_filelock_end() {
        let lock = FileLock {
            lock_type: F_WRLCK,
            start: 10,
            len: 20,
            pid: 1,
        };
        assert_eq!(lock.end(), 30);
        let whole = FileLock {
            lock_type: F_WRLCK,
            start: 0,
            len: 0,
            pid: 1,
        };
        assert_eq!(whole.end(), u64::MAX);
    }
}
