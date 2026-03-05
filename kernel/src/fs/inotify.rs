//! inotify -- File Event Monitoring
//!
//! Provides inotify-style file event monitoring for the VFS layer.
//! Watches can be placed on files and directories to receive notifications
//! when filesystem events occur (create, delete, modify, move, etc.).
//!
//! Key features:
//! - Watch descriptor management (`inotify_init`, `inotify_add_watch`,
//!   `inotify_rm_watch`)
//! - Bounded per-instance event queues with configurable max (default 4096)
//! - Event coalescing: identical consecutive events are deduplicated
//! - Recursive watch support via `IN_RECURSIVE` flag (custom extension)
//! - Thread-safe via atomic operations and spin::RwLock

#![allow(dead_code)]

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    vec::Vec,
};
use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Event type constants (Linux-compatible values)
// ---------------------------------------------------------------------------

/// File was accessed (read).
pub const IN_ACCESS: u32 = 0x0000_0001;

/// File was modified (write).
pub const IN_MODIFY: u32 = 0x0000_0002;

/// File attributes changed (chmod, chown, etc.).
pub const IN_ATTRIB: u32 = 0x0000_0004;

/// File opened for writing was closed.
pub const IN_CLOSE_WRITE: u32 = 0x0000_0008;

/// File not opened for writing was closed.
pub const IN_CLOSE_NOWRITE: u32 = 0x0000_0010;

/// File was opened.
pub const IN_OPEN: u32 = 0x0000_0020;

/// File/directory moved out of watched directory.
pub const IN_MOVED_FROM: u32 = 0x0000_0040;

/// File/directory moved into watched directory.
pub const IN_MOVED_TO: u32 = 0x0000_0080;

/// File/directory created in watched directory.
pub const IN_CREATE: u32 = 0x0000_0100;

/// File/directory deleted from watched directory.
pub const IN_DELETE: u32 = 0x0000_0200;

/// Watched file/directory was itself deleted.
pub const IN_DELETE_SELF: u32 = 0x0000_0400;

/// Watched file/directory was itself moved.
pub const IN_MOVE_SELF: u32 = 0x0000_0800;

/// Combination of all standard event types.
pub const IN_ALL_EVENTS: u32 = IN_ACCESS
    | IN_MODIFY
    | IN_ATTRIB
    | IN_CLOSE_WRITE
    | IN_CLOSE_NOWRITE
    | IN_OPEN
    | IN_MOVED_FROM
    | IN_MOVED_TO
    | IN_CREATE
    | IN_DELETE
    | IN_DELETE_SELF
    | IN_MOVE_SELF;

/// Watch subdirectories recursively (VeridianOS extension, not in Linux
/// inotify).
pub const IN_RECURSIVE: u32 = 0x0100_0000;

/// Event occurred against a directory (set in returned events, not in watch
/// mask).
pub const IN_ISDIR: u32 = 0x4000_0000;

// ---------------------------------------------------------------------------
// Default configuration
// ---------------------------------------------------------------------------

/// Default maximum number of events per inotify instance.
const DEFAULT_MAX_EVENTS: usize = 4096;

/// Default maximum number of watches per inotify instance.
const DEFAULT_MAX_WATCHES: usize = 8192;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Global counter for generating unique instance IDs.
static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);

/// Global counter for generating unique watch descriptors within an instance.
static NEXT_WD: AtomicI32 = AtomicI32::new(1);

/// Global cookie counter for pairing MOVED_FROM/MOVED_TO events.
static NEXT_COOKIE: AtomicU64 = AtomicU64::new(1);

/// Global registry of all inotify instances, keyed by instance ID.
static INOTIFY_INSTANCES: RwLock<BTreeMap<u64, InotifyInstance>> = RwLock::new(BTreeMap::new());

/// Reverse mapping: inode -> list of (instance_id, wd) watching that inode.
/// Used by `notify_event()` to efficiently find watches for a given inode.
static INODE_WATCHES: RwLock<BTreeMap<u64, Vec<(u64, i32)>>> = RwLock::new(BTreeMap::new());

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// An inotify event delivered to userspace.
#[derive(Debug, Clone)]
pub struct InotifyEvent {
    /// Watch descriptor that triggered this event.
    pub wd: i32,
    /// Bitmask of event types (IN_CREATE, IN_MODIFY, etc.).
    pub mask: u32,
    /// Cookie for pairing MOVED_FROM / MOVED_TO events (0 if not a move).
    pub cookie: u32,
    /// Optional filename associated with the event (for directory watches).
    pub name: Option<String>,
}

impl InotifyEvent {
    /// Check if two events are identical for coalescing purposes.
    /// Two events are coalesceable if they have the same wd, mask, cookie,
    /// and name.
    fn is_coalesceable_with(&self, other: &InotifyEvent) -> bool {
        self.wd == other.wd
            && self.mask == other.mask
            && self.cookie == other.cookie
            && self.name == other.name
    }
}

/// A watch descriptor tracking a single watched path/inode.
#[derive(Debug, Clone)]
pub struct WatchDescriptor {
    /// Unique watch descriptor ID (returned to userspace).
    pub wd: i32,
    /// Inode number of the watched file/directory.
    pub inode: u64,
    /// Bitmask of event types to watch for.
    pub mask: u32,
    /// Whether to recursively watch subdirectories.
    pub recursive: bool,
    /// Original path being watched (for debugging/display).
    pub path: String,
}

/// An inotify instance, representing one open inotify file descriptor.
pub struct InotifyInstance {
    /// Unique instance ID.
    id: u64,
    /// Active watches, keyed by watch descriptor.
    watches: BTreeMap<i32, WatchDescriptor>,
    /// Pending events queue (bounded).
    events: VecDeque<InotifyEvent>,
    /// Maximum number of events in the queue before oldest are dropped.
    max_events: usize,
    /// Maximum number of watches allowed.
    max_watches: usize,
}

impl InotifyInstance {
    /// Create a new inotify instance with default limits.
    fn new(id: u64) -> Self {
        Self {
            id,
            watches: BTreeMap::new(),
            events: VecDeque::new(),
            max_events: DEFAULT_MAX_EVENTS,
            max_watches: DEFAULT_MAX_WATCHES,
        }
    }

    /// Create a new inotify instance with custom limits.
    fn with_limits(id: u64, max_events: usize, max_watches: usize) -> Self {
        Self {
            id,
            watches: BTreeMap::new(),
            events: VecDeque::new(),
            max_events,
            max_watches,
        }
    }

    /// Push an event into this instance's queue.
    /// If the queue is full, the oldest event is discarded.
    /// Performs event coalescing: if the new event is identical to the
    /// most recently queued event, it is silently dropped.
    fn push_event(&mut self, event: InotifyEvent) {
        // Coalesce: skip if identical to the last queued event
        if let Some(last) = self.events.back() {
            if last.is_coalesceable_with(&event) {
                return;
            }
        }

        // Enforce queue limit by dropping oldest
        while self.events.len() >= self.max_events {
            self.events.pop_front();
        }

        self.events.push_back(event);
    }

    /// Read and drain up to `max_count` events from the queue.
    /// Returns an empty Vec if no events are pending.
    fn read_events(&mut self, max_count: usize) -> Vec<InotifyEvent> {
        let count = max_count.min(self.events.len());
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(event) = self.events.pop_front() {
                result.push(event);
            }
        }
        result
    }

    /// Return the number of pending events.
    fn pending_count(&self) -> usize {
        self.events.len()
    }

    /// Check whether any events are pending.
    fn has_events(&self) -> bool {
        !self.events.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize a new inotify instance.
///
/// Returns the instance ID (analogous to the fd returned by inotify_init(2)).
pub fn inotify_init() -> Result<u64, KernelError> {
    let id = NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed);
    let instance = InotifyInstance::new(id);

    let mut instances = INOTIFY_INSTANCES.write();
    instances.insert(id, instance);

    Ok(id)
}

/// Initialize a new inotify instance with custom limits.
///
/// Returns the instance ID.
pub fn inotify_init_with_limits(max_events: usize, max_watches: usize) -> Result<u64, KernelError> {
    if max_events == 0 {
        return Err(KernelError::InvalidArgument {
            name: "max_events",
            value: "must be > 0",
        });
    }
    if max_watches == 0 {
        return Err(KernelError::InvalidArgument {
            name: "max_watches",
            value: "must be > 0",
        });
    }

    let id = NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed);
    let instance = InotifyInstance::with_limits(id, max_events, max_watches);

    let mut instances = INOTIFY_INSTANCES.write();
    instances.insert(id, instance);

    Ok(id)
}

/// Add a watch to an inotify instance.
///
/// Watches the inode at the given path for the specified event types.
/// If the inode is already watched by this instance, the watch mask is updated.
///
/// # Arguments
/// * `instance_id` - The inotify instance (from `inotify_init`)
/// * `inode` - Inode number to watch
/// * `path` - Path being watched (stored for debugging)
/// * `mask` - Bitmask of event types to watch (IN_CREATE, IN_MODIFY, etc.)
///
/// Returns the watch descriptor (wd).
pub fn inotify_add_watch(
    instance_id: u64,
    inode: u64,
    path: &str,
    mask: u32,
) -> Result<i32, KernelError> {
    let event_mask = mask & IN_ALL_EVENTS;
    if event_mask == 0 {
        return Err(KernelError::InvalidArgument {
            name: "mask",
            value: "no valid event types specified",
        });
    }

    let recursive = mask & IN_RECURSIVE != 0;

    let mut instances = INOTIFY_INSTANCES.write();
    let instance = instances
        .get_mut(&instance_id)
        .ok_or(KernelError::NotFound {
            resource: "inotify instance",
            id: instance_id,
        })?;

    // Check if this inode is already watched by this instance
    for watch in instance.watches.values_mut() {
        if watch.inode == inode {
            // Update existing watch mask
            watch.mask = event_mask;
            watch.recursive = recursive;
            return Ok(watch.wd);
        }
    }

    // Check watch limit
    if instance.watches.len() >= instance.max_watches {
        return Err(KernelError::ResourceExhausted {
            resource: "inotify watches",
        });
    }

    // Create new watch
    let wd = NEXT_WD.fetch_add(1, Ordering::Relaxed);
    let watch = WatchDescriptor {
        wd,
        inode,
        mask: event_mask,
        recursive,
        path: String::from(path),
    };

    instance.watches.insert(wd, watch);

    // Update reverse mapping
    let mut inode_watches = INODE_WATCHES.write();
    inode_watches
        .entry(inode)
        .or_default()
        .push((instance_id, wd));

    Ok(wd)
}

/// Remove a watch from an inotify instance.
///
/// # Arguments
/// * `instance_id` - The inotify instance
/// * `wd` - The watch descriptor to remove (from `inotify_add_watch`)
pub fn inotify_rm_watch(instance_id: u64, wd: i32) -> Result<(), KernelError> {
    let mut instances = INOTIFY_INSTANCES.write();
    let instance = instances
        .get_mut(&instance_id)
        .ok_or(KernelError::NotFound {
            resource: "inotify instance",
            id: instance_id,
        })?;

    let watch = instance.watches.remove(&wd).ok_or(KernelError::NotFound {
        resource: "inotify watch",
        id: wd as u64,
    })?;

    // Remove from reverse mapping
    let mut inode_watches = INODE_WATCHES.write();
    if let Some(watchers) = inode_watches.get_mut(&watch.inode) {
        watchers.retain(|&(iid, w)| !(iid == instance_id && w == wd));
        if watchers.is_empty() {
            inode_watches.remove(&watch.inode);
        }
    }

    Ok(())
}

/// Destroy an inotify instance, removing all its watches.
///
/// # Arguments
/// * `instance_id` - The inotify instance to destroy
pub fn inotify_close(instance_id: u64) -> Result<(), KernelError> {
    let mut instances = INOTIFY_INSTANCES.write();
    let instance = instances
        .remove(&instance_id)
        .ok_or(KernelError::NotFound {
            resource: "inotify instance",
            id: instance_id,
        })?;

    // Clean up all reverse mappings for this instance's watches
    let mut inode_watches = INODE_WATCHES.write();
    for watch in instance.watches.values() {
        if let Some(watchers) = inode_watches.get_mut(&watch.inode) {
            watchers.retain(|&(iid, _)| iid != instance_id);
            if watchers.is_empty() {
                inode_watches.remove(&watch.inode);
            }
        }
    }

    Ok(())
}

/// Read pending events from an inotify instance.
///
/// Returns up to `max_count` events, removing them from the queue.
/// Returns an empty Vec if no events are pending.
pub fn inotify_read(instance_id: u64, max_count: usize) -> Result<Vec<InotifyEvent>, KernelError> {
    let mut instances = INOTIFY_INSTANCES.write();
    let instance = instances
        .get_mut(&instance_id)
        .ok_or(KernelError::NotFound {
            resource: "inotify instance",
            id: instance_id,
        })?;

    Ok(instance.read_events(max_count))
}

/// Check how many events are pending for an inotify instance.
pub fn inotify_pending(instance_id: u64) -> Result<usize, KernelError> {
    let instances = INOTIFY_INSTANCES.read();
    let instance = instances.get(&instance_id).ok_or(KernelError::NotFound {
        resource: "inotify instance",
        id: instance_id,
    })?;

    Ok(instance.pending_count())
}

/// Generate a new cookie for pairing MOVED_FROM / MOVED_TO events.
pub fn generate_move_cookie() -> u32 {
    // Truncate to u32; wrapping is fine for cookies
    NEXT_COOKIE.fetch_add(1, Ordering::Relaxed) as u32
}

// ---------------------------------------------------------------------------
// VFS integration: event notification
// ---------------------------------------------------------------------------

/// Notify all watchers of a filesystem event on the given inode.
///
/// This is the primary integration point: VFS operations (create, delete,
/// write, rename, etc.) call this function to push events to any inotify
/// instances watching the affected inode.
///
/// # Arguments
/// * `inode` - The inode where the event occurred
/// * `mask` - Event type bitmask (IN_CREATE, IN_MODIFY, etc.)
/// * `cookie` - Cookie for pairing move events (0 for non-move events)
/// * `name` - Optional filename (for events in a watched directory)
pub fn notify_event(inode: u64, mask: u32, cookie: u32, name: Option<&str>) {
    // Clone watchers inside a scope so the read lock is released before
    // acquiring the write lock on INOTIFY_INSTANCES.
    let watchers = {
        let inode_watches = INODE_WATCHES.read();
        match inode_watches.get(&inode) {
            Some(w) => w.clone(),
            None => return, // No watches on this inode
        }
    };

    let mut instances = INOTIFY_INSTANCES.write();

    for (instance_id, wd) in &watchers {
        if let Some(instance) = instances.get_mut(instance_id) {
            // Check if this watch cares about this event type
            if let Some(watch) = instance.watches.get(wd) {
                if watch.mask & mask != 0 {
                    let event = InotifyEvent {
                        wd: *wd,
                        mask,
                        cookie,
                        name: name.map(String::from),
                    };
                    instance.push_event(event);
                }
            }
        }
    }
}

/// Convenience wrapper: notify a directory watch about an event on a child.
///
/// Sets the IN_ISDIR flag if `is_dir` is true.
pub fn notify_child_event(
    parent_inode: u64,
    mask: u32,
    cookie: u32,
    child_name: &str,
    is_dir: bool,
) {
    let effective_mask = if is_dir { mask | IN_ISDIR } else { mask };
    notify_event(parent_inode, effective_mask, cookie, Some(child_name));
}

// ---------------------------------------------------------------------------
// Statistics / introspection
// ---------------------------------------------------------------------------

/// Statistics about the inotify subsystem.
#[derive(Debug, Clone, Copy)]
pub struct InotifyStats {
    /// Total number of active inotify instances.
    pub instance_count: usize,
    /// Total number of active watches across all instances.
    pub total_watches: usize,
    /// Total number of pending events across all instances.
    pub total_pending_events: usize,
}

/// Get current inotify subsystem statistics.
pub fn get_stats() -> InotifyStats {
    let instances = INOTIFY_INSTANCES.read();
    let mut total_watches = 0;
    let mut total_pending = 0;

    for instance in instances.values() {
        total_watches += instance.watches.len();
        total_pending += instance.events.len();
    }

    InotifyStats {
        instance_count: instances.len(),
        total_watches,
        total_pending_events: total_pending,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inotify_init() {
        let id = inotify_init().unwrap();
        assert!(id > 0);

        // Clean up
        inotify_close(id).unwrap();
    }

    #[test]
    fn test_inotify_init_with_limits() {
        let id = inotify_init_with_limits(100, 50).unwrap();
        assert!(id > 0);
        inotify_close(id).unwrap();
    }

    #[test]
    fn test_inotify_init_zero_events_rejected() {
        let result = inotify_init_with_limits(0, 50);
        assert!(result.is_err());
    }

    #[test]
    fn test_inotify_init_zero_watches_rejected() {
        let result = inotify_init_with_limits(100, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_and_remove_watch() {
        let id = inotify_init().unwrap();

        let wd = inotify_add_watch(id, 42, "/tmp/test", IN_MODIFY | IN_CREATE).unwrap();
        assert!(wd > 0);

        inotify_rm_watch(id, wd).unwrap();
        inotify_close(id).unwrap();
    }

    #[test]
    fn test_add_watch_invalid_mask() {
        let id = inotify_init().unwrap();

        // Mask with no valid event bits
        let result = inotify_add_watch(id, 42, "/tmp/test", 0);
        assert!(result.is_err());

        // Only IN_RECURSIVE set, no event types
        let result = inotify_add_watch(id, 42, "/tmp/test", IN_RECURSIVE);
        assert!(result.is_err());

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_add_watch_updates_existing() {
        let id = inotify_init().unwrap();

        let wd1 = inotify_add_watch(id, 42, "/tmp/test", IN_MODIFY).unwrap();
        let wd2 = inotify_add_watch(id, 42, "/tmp/test", IN_CREATE).unwrap();

        // Same inode should reuse the same wd
        assert_eq!(wd1, wd2);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_rm_watch_invalid_instance() {
        let result = inotify_rm_watch(999_999, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_rm_watch_invalid_wd() {
        let id = inotify_init().unwrap();
        let result = inotify_rm_watch(id, 999);
        assert!(result.is_err());
        inotify_close(id).unwrap();
    }

    #[test]
    fn test_close_invalid_instance() {
        let result = inotify_close(999_999);
        assert!(result.is_err());
    }

    #[test]
    fn test_notify_event_delivery() {
        let id = inotify_init().unwrap();
        let inode = 100;
        let wd = inotify_add_watch(id, inode, "/tmp/watched", IN_MODIFY | IN_CREATE).unwrap();

        // Fire a MODIFY event
        notify_event(inode, IN_MODIFY, 0, None);

        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].wd, wd);
        assert_eq!(events[0].mask, IN_MODIFY);
        assert_eq!(events[0].cookie, 0);
        assert!(events[0].name.is_none());

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_notify_event_with_name() {
        let id = inotify_init().unwrap();
        let inode = 101;
        let wd = inotify_add_watch(id, inode, "/tmp/dir", IN_CREATE).unwrap();

        notify_child_event(inode, IN_CREATE, 0, "newfile.txt", false);

        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].wd, wd);
        assert_eq!(events[0].mask, IN_CREATE);
        assert_eq!(events[0].name.as_deref(), Some("newfile.txt"));

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_notify_isdir_flag() {
        let id = inotify_init().unwrap();
        let inode = 102;
        inotify_add_watch(id, inode, "/tmp/dir", IN_CREATE).unwrap();

        notify_child_event(inode, IN_CREATE, 0, "subdir", true);

        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mask, IN_CREATE | IN_ISDIR);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_event_filtering_by_mask() {
        let id = inotify_init().unwrap();
        let inode = 103;
        // Only watch for MODIFY, not CREATE
        inotify_add_watch(id, inode, "/tmp/filtered", IN_MODIFY).unwrap();

        // Fire a CREATE event -- should be filtered out
        notify_event(inode, IN_CREATE, 0, Some("ignored.txt"));

        // Fire a MODIFY event -- should be delivered
        notify_event(inode, IN_MODIFY, 0, None);

        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mask, IN_MODIFY);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_event_coalescing() {
        let id = inotify_init().unwrap();
        let inode = 104;
        inotify_add_watch(id, inode, "/tmp/coalesce", IN_MODIFY).unwrap();

        // Fire three identical MODIFY events
        notify_event(inode, IN_MODIFY, 0, None);
        notify_event(inode, IN_MODIFY, 0, None);
        notify_event(inode, IN_MODIFY, 0, None);

        // Should be coalesced to 1
        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_no_coalescing_different_events() {
        let id = inotify_init().unwrap();
        let inode = 105;
        inotify_add_watch(id, inode, "/tmp/nocoalesce", IN_ALL_EVENTS).unwrap();

        notify_event(inode, IN_MODIFY, 0, None);
        notify_event(inode, IN_CREATE, 0, Some("a.txt"));
        notify_event(inode, IN_MODIFY, 0, None);

        // All 3 should remain (MODIFY, CREATE, MODIFY are not consecutive-identical)
        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 3);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_event_queue_overflow() {
        let id = inotify_init_with_limits(5, 100).unwrap();
        let inode = 106;
        inotify_add_watch(id, inode, "/tmp/overflow", IN_MODIFY).unwrap();

        // Fire 10 events with different names to avoid coalescing
        for i in 0..10 {
            let name = if i % 2 == 0 { Some("a") } else { Some("b") };
            notify_event(inode, IN_MODIFY, 0, name);
        }

        // Queue limit is 5, so only the last 5 should remain
        let events = inotify_read(id, 20).unwrap();
        assert_eq!(events.len(), 5);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_read_events_empty() {
        let id = inotify_init().unwrap();

        let events = inotify_read(id, 10).unwrap();
        assert!(events.is_empty());

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_read_events_partial() {
        let id = inotify_init().unwrap();
        let inode = 107;
        inotify_add_watch(id, inode, "/tmp/partial", IN_ALL_EVENTS).unwrap();

        notify_event(inode, IN_CREATE, 0, Some("a.txt"));
        notify_event(inode, IN_MODIFY, 0, Some("a.txt"));
        notify_event(inode, IN_DELETE, 0, Some("a.txt"));

        // Read only 2 of 3
        let events = inotify_read(id, 2).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].mask, IN_CREATE);
        assert_eq!(events[1].mask, IN_MODIFY);

        // Remaining 1
        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mask, IN_DELETE);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_inotify_pending() {
        let id = inotify_init().unwrap();
        let inode = 108;
        inotify_add_watch(id, inode, "/tmp/pending", IN_MODIFY).unwrap();

        assert_eq!(inotify_pending(id).unwrap(), 0);

        notify_event(inode, IN_MODIFY, 0, Some("x"));
        assert_eq!(inotify_pending(id).unwrap(), 1);

        notify_event(inode, IN_MODIFY, 0, Some("y"));
        assert_eq!(inotify_pending(id).unwrap(), 2);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_notify_unwatched_inode() {
        // Should not panic or error; just a no-op
        notify_event(999_999, IN_MODIFY, 0, None);
    }

    #[test]
    fn test_move_cookie() {
        let id = inotify_init().unwrap();
        let src_inode = 200;
        let dst_inode = 201;
        inotify_add_watch(id, src_inode, "/tmp/src", IN_MOVED_FROM).unwrap();
        inotify_add_watch(id, dst_inode, "/tmp/dst", IN_MOVED_TO).unwrap();

        let cookie = generate_move_cookie();
        notify_event(src_inode, IN_MOVED_FROM, cookie, Some("file.txt"));
        notify_event(dst_inode, IN_MOVED_TO, cookie, Some("file.txt"));

        let events = inotify_read(id, 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].mask, IN_MOVED_FROM);
        assert_eq!(events[1].mask, IN_MOVED_TO);
        // Same cookie pairs the events
        assert_eq!(events[0].cookie, events[1].cookie);
        assert!(events[0].cookie > 0);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_recursive_watch_flag() {
        let id = inotify_init().unwrap();

        let wd = inotify_add_watch(id, 300, "/tmp/recursive", IN_CREATE | IN_RECURSIVE).unwrap();

        // Verify the watch was created with recursive flag
        {
            let instances = INOTIFY_INSTANCES.read();
            let instance = instances.get(&id).unwrap();
            let watch = instance.watches.get(&wd).unwrap();
            assert!(watch.recursive);
        }

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_multiple_instances_same_inode() {
        let id1 = inotify_init().unwrap();
        let id2 = inotify_init().unwrap();
        let inode = 400;

        inotify_add_watch(id1, inode, "/tmp/multi", IN_MODIFY).unwrap();
        inotify_add_watch(id2, inode, "/tmp/multi", IN_CREATE).unwrap();

        // MODIFY event should reach id1 but not id2
        notify_event(inode, IN_MODIFY, 0, None);
        assert_eq!(inotify_pending(id1).unwrap(), 1);
        assert_eq!(inotify_pending(id2).unwrap(), 0);

        // CREATE event should reach id2 but not id1
        notify_event(inode, IN_CREATE, 0, Some("new.txt"));
        assert_eq!(inotify_pending(id1).unwrap(), 1); // still 1, no new
        assert_eq!(inotify_pending(id2).unwrap(), 1);

        inotify_close(id1).unwrap();
        inotify_close(id2).unwrap();
    }

    #[test]
    fn test_close_cleans_up_inode_watches() {
        let id = inotify_init().unwrap();
        let inode = 500;
        inotify_add_watch(id, inode, "/tmp/cleanup", IN_MODIFY).unwrap();

        inotify_close(id).unwrap();

        // After close, events on that inode should not deliver anywhere
        // (should not panic)
        notify_event(inode, IN_MODIFY, 0, None);
    }

    #[test]
    fn test_rm_watch_cleans_up_inode_mapping() {
        let id = inotify_init().unwrap();
        let inode = 501;
        let wd = inotify_add_watch(id, inode, "/tmp/rmclean", IN_MODIFY).unwrap();

        inotify_rm_watch(id, wd).unwrap();

        // Event should not be delivered after watch removal
        notify_event(inode, IN_MODIFY, 0, None);
        assert_eq!(inotify_pending(id).unwrap(), 0);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_watch_limit_enforcement() {
        let id = inotify_init_with_limits(1000, 3).unwrap();

        inotify_add_watch(id, 601, "/a", IN_MODIFY).unwrap();
        inotify_add_watch(id, 602, "/b", IN_MODIFY).unwrap();
        inotify_add_watch(id, 603, "/c", IN_MODIFY).unwrap();

        // 4th watch should fail
        let result = inotify_add_watch(id, 604, "/d", IN_MODIFY);
        assert!(result.is_err());

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_get_stats() {
        let id = inotify_init().unwrap();
        let inode = 700;
        inotify_add_watch(id, inode, "/tmp/stats", IN_MODIFY).unwrap();
        notify_event(inode, IN_MODIFY, 0, None);

        let stats = get_stats();
        assert!(stats.instance_count >= 1);
        assert!(stats.total_watches >= 1);
        assert!(stats.total_pending_events >= 1);

        inotify_close(id).unwrap();
    }

    #[test]
    fn test_event_coalesceable() {
        let e1 = InotifyEvent {
            wd: 1,
            mask: IN_MODIFY,
            cookie: 0,
            name: None,
        };
        let e2 = InotifyEvent {
            wd: 1,
            mask: IN_MODIFY,
            cookie: 0,
            name: None,
        };
        let e3 = InotifyEvent {
            wd: 1,
            mask: IN_CREATE,
            cookie: 0,
            name: None,
        };
        let e4 = InotifyEvent {
            wd: 2,
            mask: IN_MODIFY,
            cookie: 0,
            name: None,
        };
        let e5 = InotifyEvent {
            wd: 1,
            mask: IN_MODIFY,
            cookie: 0,
            name: Some(String::from("file.txt")),
        };

        assert!(e1.is_coalesceable_with(&e2));
        assert!(!e1.is_coalesceable_with(&e3)); // different mask
        assert!(!e1.is_coalesceable_with(&e4)); // different wd
        assert!(!e1.is_coalesceable_with(&e5)); // different name
    }
}
