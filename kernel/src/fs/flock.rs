//! POSIX File Locking (flock/fcntl)
//!
//! Implements both whole-file locking (flock semantics) and byte-range
//! record locking (fcntl), in one table keyed by inode.
//!
//! - `flock()`: whole-file advisory locks (LOCK_SH, LOCK_EX, LOCK_UN), owned by
//!   an open file as on Linux (the caller passes its identity): duplicates and
//!   fork children share a lock, two opens of one file do not, and the lock
//!   goes with the open file's last close. A request that conflicts returns
//!   `WouldBlock`; a blocking caller sleeps in [`FLOCK_WAITERS`], which every
//!   release wakes (N-207)
//! - [`set_lock`] / [`test_lock`]: fcntl record locks (N-199), POSIX and OFD,
//!   as Linux's posix_lock_inode: an owner's locks split and join, a conflict
//!   is reported to the caller (F_SETLKW waits in [`RANGE_WAITERS`]), and a
//!   POSIX waiter that would close a cycle of waiting owners is a deadlock. A
//!   POSIX lock's owner is a descriptor table, an OFD lock's an open file;
//!   [`release_owner`] drops either
//! - [`release_all_of`]: every POSIX record lock of a descriptor table, when
//!   its last holder closes it (an exiting process, the last of those sharing
//!   it with CLONE_FILES)

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

use spin::RwLock;

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

/// A whole-file advisory lock entry (flock semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlockEntry {
    /// Lock type: LOCK_SH or LOCK_EX.
    pub lock_type: u32,
    /// The owning open file's identity.
    pub pid: u64,
}

/// Tasks waiting in flock(2) without LOCK_NB; woken whenever a flock lock
/// is released.
#[cfg(feature = "alloc")]
pub static FLOCK_WAITERS: crate::sched::dispatch::WaitQueue =
    crate::sched::dispatch::WaitQueue::new();

/// A flock lock went: let blocked requests try again.
fn flock_released() {
    #[cfg(feature = "alloc")]
    FLOCK_WAITERS.wake_all();
}

/// Per-system lock table tracking all file locks by inode.
struct LockTable {
    /// Whole-file locks (flock): inode -> list of FlockEntry
    flock_locks: BTreeMap<u64, Vec<FlockEntry>>,
    /// Record locks (fcntl): inode -> locks, sorted by start
    range_locks: BTreeMap<u64, Vec<RangeLock>>,
    /// POSIX owners waiting in F_SETLKW -> the owner they wait for (Linux's
    /// blocked_hash, which deadlock detection follows)
    blocked: BTreeMap<u64, u64>,
}

impl LockTable {
    const fn new() -> Self {
        Self {
            flock_locks: BTreeMap::new(),
            range_locks: BTreeMap::new(),
            blocked: BTreeMap::new(),
        }
    }
}

/// Global lock table, protected by a reader-writer lock.
static LOCK_TABLE: RwLock<LockTable> = RwLock::new(LockTable::new());

// ---------------------------------------------------------------------------
// flock() implementation
// ---------------------------------------------------------------------------

/// Apply a whole-file advisory lock (flock semantics) for the open file
/// `owner`.
///
/// `operation` is LOCK_SH, LOCK_EX or LOCK_UN, optionally with LOCK_NB.
/// A request that conflicts with another open file's lock returns
/// `WouldBlock` whether or not LOCK_NB is set: a blocking caller waits in
/// [`FLOCK_WAITERS`] and asks again.
pub fn flock(inode: u64, owner: u64, operation: u32) -> Result<(), KernelError> {
    match operation & !LOCK_NB {
        LOCK_UN => flock_unlock(inode, owner),
        LOCK_SH => flock_lock(inode, owner, LOCK_SH),
        LOCK_EX => flock_lock(inode, owner, LOCK_EX),
        _ => Err(KernelError::InvalidArgument {
            name: "operation",
            value: "invalid flock operation",
        }),
    }
}

/// Acquire a whole-file lock (shared or exclusive). Converting a lock the
/// owner holds drops it first, as Linux and BSD do: a conversion that has to
/// wait (or fails with LOCK_NB) has given up the old lock meanwhile.
fn flock_lock(inode: u64, owner: u64, lock_type: u32) -> Result<(), KernelError> {
    let (released, conflict) = {
        let mut table = LOCK_TABLE.write();
        let entries = table.flock_locks.entry(inode).or_default();
        if entries
            .iter()
            .any(|e| e.pid == owner && e.lock_type == lock_type)
        {
            return Ok(());
        }
        let held = entries.len();
        entries.retain(|e| e.pid != owner);
        let released = entries.len() != held;
        // Shared locks conflict only with exclusive ones; an exclusive
        // request with any other lock.
        let conflict = entries
            .iter()
            .any(|e| lock_type == LOCK_EX || e.lock_type == LOCK_EX);
        if !conflict {
            entries.push(FlockEntry {
                lock_type,
                pid: owner,
            });
        } else if entries.is_empty() {
            table.flock_locks.remove(&inode);
        }
        (released, conflict)
    };
    if released {
        flock_released();
    }
    if conflict {
        Err(KernelError::WouldBlock)
    } else {
        Ok(())
    }
}

/// Release the whole-file lock `owner` holds, if any.
fn flock_unlock(inode: u64, owner: u64) -> Result<(), KernelError> {
    let released = {
        let mut table = LOCK_TABLE.write();
        let mut released = false;
        if let Some(entries) = table.flock_locks.get_mut(&inode) {
            let held = entries.len();
            entries.retain(|e| e.pid != owner);
            released = entries.len() != held;
            if entries.is_empty() {
                table.flock_locks.remove(&inode);
            }
        }
        released
    };
    if released {
        flock_released();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// fcntl() record locks
// ---------------------------------------------------------------------------

/// Linux's OFFSET_MAX: a lock ending here runs to the end of the file,
/// however far the file grows (an `l_len` of 0).
pub const OFFSET_MAX: u64 = i64::MAX as u64;

/// How many waiting owners deadlock detection follows (Linux's
/// MAX_DEADLK_ITERATIONS): a longer chain is not reported.
const MAX_DEADLK_ITERATIONS: usize = 10;

/// A byte-range lock (fcntl F_SETLK and the OFD calls), as Linux's
/// `struct file_lock` has one: an inclusive range, a type and an owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeLock {
    /// F_RDLCK or F_WRLCK; F_UNLCK in a request to remove.
    pub kind: u16,
    /// The first byte.
    pub start: u64,
    /// The last byte, inclusive: [`OFFSET_MAX`] to the end of the file.
    pub end: u64,
    /// Who holds it: a descriptor table for a POSIX lock (Linux's
    /// `current->files`, so threads share their locks and a fork child does
    /// not inherit them), an open file for an OFD lock. One owner's locks
    /// never conflict with each other.
    pub owner: u64,
    /// What F_GETLK reports as `l_pid`: the holder's process, -1 for an OFD
    /// lock.
    pub pid: i32,
    /// An OFD lock: never part of deadlock detection.
    pub ofd: bool,
}

impl RangeLock {
    fn overlaps(&self, other: &RangeLock) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    /// Whether the two ranges overlap or meet, so that two locks of one
    /// type and owner make one.
    fn touches(&self, other: &RangeLock) -> bool {
        self.start <= other.end.saturating_add(1) && other.start <= self.end.saturating_add(1)
    }

    /// Linux's posix_locks_conflict: another owner, an overlap, and a write
    /// lock on either side (an F_UNLCK query conflicts with a write lock).
    fn conflicts(&self, other: &RangeLock) -> bool {
        self.owner != other.owner
            && self.overlaps(other)
            && (self.kind == F_WRLCK || other.kind == F_WRLCK)
    }
}

/// Why a record lock was not placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    /// Another owner's lock is in the way (EAGAIN; F_SETLKW waits).
    Conflict,
    /// Waiting would close a cycle of POSIX owners, each waiting for a lock
    /// of the next (EDEADLK).
    Deadlock,
}

/// Tasks waiting in F_SETLKW or F_OFD_SETLKW; woken whenever a record
/// lock goes or changes.
#[cfg(feature = "alloc")]
pub static RANGE_WAITERS: crate::sched::dispatch::WaitQueue =
    crate::sched::dispatch::WaitQueue::new();

/// A record lock went or changed: let waiting requests try again.
fn range_released() {
    #[cfg(feature = "alloc")]
    RANGE_WAITERS.wake_all();
}

/// F_GETLK: the first lock of another owner that `query` conflicts with.
pub fn test_lock(key: u64, query: &RangeLock) -> Option<RangeLock> {
    let table = LOCK_TABLE.read();
    table
        .range_locks
        .get(&key)?
        .iter()
        .find(|lock| lock.conflicts(query))
        .copied()
}

/// F_SETLK: place, change or (F_UNLCK) remove the owner's lock on
/// `req`'s range, splitting and joining the owner's other locks there as
/// Linux's posix_lock_inode does. A conflict is reported, never waited
/// for. With `will_wait` a POSIX request is about to wait: it is
/// EDEADLK if the owner it waits for is itself waiting, through at most
/// [`MAX_DEADLK_ITERATIONS`] others, for this one, and is otherwise
/// recorded as waiting until it succeeds or [`stop_waiting`].
pub fn set_lock(key: u64, req: &RangeLock, will_wait: bool) -> Result<(), RangeError> {
    let changed = {
        let mut table = LOCK_TABLE.write();
        if req.kind != F_UNLCK {
            let blocker = table
                .range_locks
                .get(&key)
                .and_then(|locks| locks.iter().find(|lock| lock.conflicts(req)))
                .map(|lock| lock.owner);
            if let Some(blocker) = blocker {
                if will_wait && !req.ofd {
                    if would_deadlock(&table.blocked, req.owner, blocker) {
                        table.blocked.remove(&req.owner);
                        return Err(RangeError::Deadlock);
                    }
                    table.blocked.insert(req.owner, blocker);
                }
                return Err(RangeError::Conflict);
            }
        }
        table.blocked.remove(&req.owner);
        let locks = table.range_locks.entry(key).or_default();
        let changed = apply(locks, *req);
        if locks.is_empty() {
            table.range_locks.remove(&key);
        }
        changed
    };
    if changed {
        range_released();
    }
    Ok(())
}

/// The owner no longer waits (its F_SETLKW was interrupted).
pub fn stop_waiting(owner: u64) {
    LOCK_TABLE.write().blocked.remove(&owner);
}

/// Linux's posix_locks_deadlock: follow, from `blocker`, the owner each
/// waiting owner waits for; reaching `caller` closes a cycle.
fn would_deadlock(blocked: &BTreeMap<u64, u64>, caller: u64, blocker: u64) -> bool {
    let mut owner = blocker;
    for _ in 0..MAX_DEADLK_ITERATIONS {
        match blocked.get(&owner) {
            Some(&next) if next == caller => return true,
            Some(&next) => owner = next,
            None => return false,
        }
    }
    false
}

/// Apply `req` to its owner's locks in `locks`: a lock of the same type
/// that overlaps or meets the range joins it; one of another type (or
/// any, for F_UNLCK) loses the overlapping bytes, keeping what lies on
/// either side. Returns whether a lock went or changed type (what a
/// waiter might wait for).
fn apply(locks: &mut Vec<RangeLock>, mut req: RangeLock) -> bool {
    let mine = |lock: &RangeLock| lock.owner == req.owner;
    if req.kind != F_UNLCK {
        for lock in locks.iter() {
            if mine(lock) && lock.kind == req.kind && lock.touches(&req) {
                req.start = req.start.min(lock.start);
                req.end = req.end.max(lock.end);
            }
        }
    }
    let mut changed = false;
    let mut kept = Vec::with_capacity(locks.len() + 2);
    for lock in locks.drain(..) {
        if lock.owner != req.owner || !lock.overlaps(&req) {
            // A same-type neighbour that only met the range was joined.
            if !(lock.owner == req.owner && lock.kind == req.kind && lock.touches(&req)) {
                kept.push(lock);
            }
            continue;
        }
        if lock.kind == req.kind {
            continue; // joined
        }
        changed = true;
        if lock.start < req.start {
            kept.push(RangeLock {
                end: req.start - 1,
                ..lock
            });
        }
        if lock.end > req.end {
            kept.push(RangeLock {
                start: req.end + 1,
                ..lock
            });
        }
    }
    if req.kind != F_UNLCK {
        kept.push(req);
    }
    kept.sort_by_key(|lock| (lock.start, lock.owner));
    *locks = kept;
    changed
}

/// Remove every lock `owner` holds on `key`: a POSIX owner's locks when it
/// closes any descriptor of the file, an OFD lock's at the open file's
/// last close.
pub fn release_owner(key: u64, owner: u64) {
    // Every close asks; most files have no record locks at all.
    let holds = LOCK_TABLE
        .read()
        .range_locks
        .get(&key)
        .is_some_and(|locks| locks.iter().any(|lock| lock.owner == owner));
    if !holds {
        return;
    }
    let changed = {
        let mut table = LOCK_TABLE.write();
        let mut changed = false;
        if let Some(locks) = table.range_locks.get_mut(&key) {
            let held = locks.len();
            locks.retain(|lock| lock.owner != owner);
            changed = locks.len() != held;
            if locks.is_empty() {
                table.range_locks.remove(&key);
            }
        }
        changed
    };
    if changed {
        range_released();
    }
}

// ---------------------------------------------------------------------------
// Process cleanup
// ---------------------------------------------------------------------------

/// Remove every POSIX record lock the descriptor table `owner` holds
/// (`FileTable::lock_owner`), when its last holder closes it (closing its
/// descriptors has released them already; this leaves none behind). Not
/// by pid: a table shared with CLONE_FILES outlives the process that took
/// a lock through it, and keeps the lock, as Linux's put_files_struct.
/// flock and OFD locks belong to open files and go with their last close.
pub fn release_all_of(owner: u64) {
    let changed = {
        let mut table = LOCK_TABLE.write();
        let mut changed = false;
        table.range_locks.retain(|_, locks| {
            let held = locks.len();
            locks.retain(|lock| lock.ofd || lock.owner != owner);
            changed |= locks.len() != held;
            !locks.is_empty()
        });
        changed
    };
    if changed {
        range_released();
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

    /// A conversion that cannot be granted has dropped the old lock (Linux
    /// and BSD), so the other holder can then convert.
    #[test]
    fn test_flock_failed_conversion_drops_the_old_lock() {
        let ids = unique_inodes(1);
        assert!(flock(ids[0], 100, LOCK_SH).is_ok());
        assert!(flock(ids[0], 200, LOCK_SH).is_ok());
        assert_eq!(
            flock(ids[0], 100, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
        assert!(flock(ids[0], 200, LOCK_EX | LOCK_NB).is_ok());
        // Re-requesting a held lock is a no-op.
        assert!(flock(ids[0], 200, LOCK_EX).is_ok());
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

    // ---- fcntl record locks ----

    /// A request of `owner` (its pid the same number) on [start, end].
    fn rl(kind: u16, start: u64, end: u64, owner: u64) -> RangeLock {
        RangeLock {
            kind,
            start,
            end,
            owner,
            pid: owner as i32,
            ofd: false,
        }
    }

    fn held(key: u64) -> Vec<(u16, u64, u64, u64)> {
        LOCK_TABLE
            .read()
            .range_locks
            .get(&key)
            .map(|locks| {
                locks
                    .iter()
                    .map(|l| (l.kind, l.start, l.end, l.owner))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn record_locks_conflict_as_read_and_write_locks() {
        let k = unique_inodes(1)[0];
        assert_eq!(set_lock(k, &rl(F_RDLCK, 0, 99, 1), false), Ok(()));
        assert_eq!(set_lock(k, &rl(F_RDLCK, 50, 149, 2), false), Ok(()));
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 90, 90, 3), false),
            Err(RangeError::Conflict)
        );
        assert_eq!(set_lock(k, &rl(F_WRLCK, 150, OFFSET_MAX, 3), false), Ok(()));
        // F_GETLK finds the first lock in the way, with its holder.
        let hit = test_lock(k, &rl(F_WRLCK, 0, 10, 4)).unwrap();
        assert_eq!((hit.start, hit.end, hit.pid), (0, 99, 1));
        assert_eq!(test_lock(k, &rl(F_RDLCK, 0, 140, 4)), None);
        // An F_UNLCK query conflicts with a write lock, as on Linux.
        assert_eq!(test_lock(k, &rl(F_UNLCK, 200, 200, 4)).unwrap().owner, 3);
        // An owner's own locks never conflict with it.
        assert_eq!(
            test_lock(k, &rl(F_WRLCK, 0, 99, 1)).map(|l| l.owner),
            Some(2)
        );
    }

    #[test]
    fn unlocking_the_middle_splits_a_lock() {
        let k = unique_inodes(1)[0];
        set_lock(k, &rl(F_WRLCK, 0, 99, 1), false).unwrap();
        set_lock(k, &rl(F_UNLCK, 40, 59, 1), false).unwrap();
        assert_eq!(held(k), [(F_WRLCK, 0, 39, 1), (F_WRLCK, 60, 99, 1)]);
        assert_eq!(set_lock(k, &rl(F_WRLCK, 45, 50, 2), false), Ok(()));
        // Unlocking more than is held removes what is there.
        set_lock(k, &rl(F_UNLCK, 0, OFFSET_MAX, 1), false).unwrap();
        assert_eq!(held(k), [(F_WRLCK, 45, 50, 2)]);
    }

    #[test]
    fn changing_the_type_of_a_part_splits_and_joins() {
        let k = unique_inodes(1)[0];
        set_lock(k, &rl(F_WRLCK, 0, 99, 1), false).unwrap();
        set_lock(k, &rl(F_RDLCK, 40, 59, 1), false).unwrap();
        assert_eq!(
            held(k),
            [
                (F_WRLCK, 0, 39, 1),
                (F_RDLCK, 40, 59, 1),
                (F_WRLCK, 60, 99, 1)
            ]
        );
        assert_eq!(set_lock(k, &rl(F_RDLCK, 45, 50, 2), false), Ok(()));
        assert_eq!(
            set_lock(k, &rl(F_RDLCK, 30, 35, 2), false),
            Err(RangeError::Conflict)
        );
        // Writing the read part again joins the three into one.
        set_lock(k, &rl(F_UNLCK, 45, 50, 2), false).unwrap();
        set_lock(k, &rl(F_WRLCK, 40, 59, 1), false).unwrap();
        assert_eq!(held(k), [(F_WRLCK, 0, 99, 1)]);
    }

    #[test]
    fn locks_of_one_type_that_meet_become_one() {
        let k = unique_inodes(1)[0];
        set_lock(k, &rl(F_RDLCK, 0, 9, 1), false).unwrap();
        set_lock(k, &rl(F_RDLCK, 20, 29, 1), false).unwrap();
        set_lock(k, &rl(F_RDLCK, 10, 19, 1), false).unwrap();
        assert_eq!(held(k), [(F_RDLCK, 0, 29, 1)]);
        // Another owner's adjacent lock stays separate.
        set_lock(k, &rl(F_RDLCK, 30, 39, 2), false).unwrap();
        assert_eq!(held(k), [(F_RDLCK, 0, 29, 1), (F_RDLCK, 30, 39, 2)]);
    }

    #[test]
    fn waiting_in_a_cycle_is_a_deadlock() {
        let k = unique_inodes(1)[0];
        set_lock(k, &rl(F_WRLCK, 1, 1, 11), false).unwrap();
        set_lock(k, &rl(F_WRLCK, 2, 2, 12), false).unwrap();
        set_lock(k, &rl(F_WRLCK, 3, 3, 13), false).unwrap();
        // 11 waits for 12, 12 for 13: 13 asking for 11's byte closes it.
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 2, 2, 11), true),
            Err(RangeError::Conflict)
        );
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 3, 3, 12), true),
            Err(RangeError::Conflict)
        );
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 1, 1, 13), true),
            Err(RangeError::Deadlock)
        );
        // Without waiting there is nothing to detect.
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 1, 1, 13), false),
            Err(RangeError::Conflict)
        );
        // An OFD request is never a deadlock; once 12 stops waiting, no
        // cycle is left.
        let ofd = RangeLock {
            ofd: true,
            ..rl(F_WRLCK, 1, 1, 13)
        };
        assert_eq!(set_lock(k, &ofd, true), Err(RangeError::Conflict));
        stop_waiting(12);
        assert_eq!(
            set_lock(k, &rl(F_WRLCK, 1, 1, 13), true),
            Err(RangeError::Conflict)
        );
        for owner in [11, 12, 13] {
            stop_waiting(owner);
            release_owner(k, owner);
        }
        assert!(held(k).is_empty());
    }

    #[test]
    fn exit_cleanup_takes_posix_locks_only() {
        let k = unique_inodes(1)[0];
        set_lock(k, &rl(F_WRLCK, 0, 9, 21), false).unwrap();
        let ofd = RangeLock {
            pid: -1,
            ofd: true,
            ..rl(F_WRLCK, 10, 19, 0xAB00)
        };
        set_lock(k, &ofd, false).unwrap();
        release_all_of(21);
        assert_eq!(held(k), [(F_WRLCK, 10, 19, 0xAB00)]);
        release_owner(k, 0xAB00);
        assert!(held(k).is_empty());
    }

    // ---- cleanup tests ----

    /// flock locks belong to open files, not processes: a process exit
    /// (which may leave the file open in a fork child) does not drop them.
    #[test]
    fn test_release_all_of_keeps_flock_locks() {
        let ids = unique_inodes(1);
        let file_owner: u64 = 7000;
        let other: u64 = 7001;
        assert!(flock(ids[0], file_owner, LOCK_EX).is_ok());
        release_all_of(file_owner);
        assert_eq!(
            flock(ids[0], other, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
        assert!(flock(ids[0], file_owner, LOCK_UN).is_ok());
        assert!(flock(ids[0], other, LOCK_EX | LOCK_NB).is_ok());
    }

    #[test]
    fn test_cleanup_does_not_affect_other_pids() {
        let ids = unique_inodes(2);
        let pid_a: u64 = 7200;
        let pid_b: u64 = 7201;
        let pid_c: u64 = 7202;
        assert!(flock(ids[0], pid_a, LOCK_EX).is_ok());
        assert!(flock(ids[1], pid_b, LOCK_EX).is_ok());
        release_all_of(pid_a);
        // PID B's lock on ids[1] should still be in place.
        assert_eq!(
            flock(ids[1], pid_c, LOCK_EX | LOCK_NB),
            Err(KernelError::WouldBlock)
        );
    }
}
