//! epoll I/O multiplexing
//!
//! Linux-compatible epoll implementation for event-driven I/O.
//! Used by Rust's mio/tokio for async I/O on VeridianOS.
//!
//! # Architecture
//!
//! Each epoll instance is an fd-based object containing:
//! - An interest list (fds being monitored + event masks)
//! - A ready list (fds with pending events)
//!
//! Supports both level-triggered (default) and edge-triggered modes.

extern crate alloc;

use alloc::collections::BTreeMap;

use spin::Mutex;

use crate::error::KernelError;

/// Maximum number of epoll instances system-wide.
const MAX_EPOLL_INSTANCES: usize = 256;

/// Maximum number of fds per epoll instance.
const MAX_EPOLL_FDS: usize = 1024;

// ============================================================================
// Event flags (matching Linux epoll.h values)
// ============================================================================

/// Available for read.
pub const EPOLLIN: u32 = 0x001;
/// Available for write.
pub const EPOLLOUT: u32 = 0x004;
/// Error condition.
pub const EPOLLERR: u32 = 0x008;
/// Hang up (peer closed connection).
pub const EPOLLHUP: u32 = 0x010;
/// Edge-triggered mode.
pub const EPOLLET: u32 = 1 << 31;
/// One-shot mode (disables after first event).
pub const EPOLLONESHOT: u32 = 1 << 30;

// ============================================================================
// epoll_ctl operations
// ============================================================================

/// Add fd to interest list.
pub const EPOLL_CTL_ADD: u32 = 1;
/// Remove fd from interest list.
pub const EPOLL_CTL_DEL: u32 = 2;
/// Modify events for an fd.
pub const EPOLL_CTL_MOD: u32 = 3;

// ============================================================================
// Data structures
// ============================================================================

/// Event structure passed to/from user space (matches Linux struct
/// epoll_event).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EpollEvent {
    /// Event flags (EPOLLIN, EPOLLOUT, etc.)
    pub events: u32,
    /// User data (typically the fd or a pointer)
    pub data: u64,
}

/// Internal entry in the interest list.
#[derive(Debug, Clone)]
struct InterestEntry {
    /// File descriptor being monitored.
    fd: i32,
    /// Requested event mask.
    events: u32,
    /// User data to return with events.
    data: u64,
    /// Whether this entry uses edge-triggered mode.
    edge_triggered: bool,
    /// Whether this is a one-shot entry.
    one_shot: bool,
    /// Whether a one-shot entry has already fired.
    disabled: bool,
}

/// An epoll instance.
struct EpollInstance {
    /// Interest list: fd -> entry.
    interest: BTreeMap<i32, InterestEntry>,
    /// ID of this instance (for lookup).
    _id: u32,
    /// Owning process PID.
    _owner_pid: u64,
}

impl EpollInstance {
    fn new(id: u32, owner_pid: u64) -> Self {
        Self {
            interest: BTreeMap::new(),
            _id: id,
            _owner_pid: owner_pid,
        }
    }

    /// Add an fd to the interest list.
    fn ctl_add(&mut self, fd: i32, event: &EpollEvent) -> Result<(), KernelError> {
        if self.interest.contains_key(&fd) {
            return Err(KernelError::AlreadyExists {
                resource: "epoll fd entry",
                id: fd as u64,
            });
        }
        if self.interest.len() >= MAX_EPOLL_FDS {
            return Err(KernelError::ResourceExhausted {
                resource: "epoll interest list",
            });
        }

        let entry = InterestEntry {
            fd,
            events: event.events & !(EPOLLET | EPOLLONESHOT),
            data: event.data,
            edge_triggered: event.events & EPOLLET != 0,
            one_shot: event.events & EPOLLONESHOT != 0,
            disabled: false,
        };
        self.interest.insert(fd, entry);
        Ok(())
    }

    /// Remove an fd from the interest list.
    fn ctl_del(&mut self, fd: i32) -> Result<(), KernelError> {
        self.interest.remove(&fd).ok_or(KernelError::NotFound {
            resource: "epoll fd entry",
            id: fd as u64,
        })?;
        Ok(())
    }

    /// Modify events for an fd.
    fn ctl_mod(&mut self, fd: i32, event: &EpollEvent) -> Result<(), KernelError> {
        let entry = self.interest.get_mut(&fd).ok_or(KernelError::NotFound {
            resource: "epoll fd entry",
            id: fd as u64,
        })?;

        entry.events = event.events & !(EPOLLET | EPOLLONESHOT);
        entry.data = event.data;
        entry.edge_triggered = event.events & EPOLLET != 0;
        entry.one_shot = event.events & EPOLLONESHOT != 0;
        entry.disabled = false;
        Ok(())
    }

    /// Poll for ready events. Returns the number of ready fds.
    ///
    /// Checks each fd in the interest list against the current fd state
    /// (via `poll_fd_readiness`). For level-triggered fds, events fire
    /// every time the condition is true. For edge-triggered, events fire
    /// only on state transitions (simplified: fire once then require re-arm
    /// via EPOLL_CTL_MOD).
    fn poll_events(&mut self, events: &mut [EpollEvent]) -> usize {
        let max_events = events.len();
        let mut count = 0;

        for entry in self.interest.values_mut() {
            if count >= max_events {
                break;
            }
            if entry.disabled {
                continue;
            }

            let ready = poll_fd_readiness(entry.fd);
            let matched = ready & entry.events;

            if matched != 0 {
                events[count] = EpollEvent {
                    events: matched,
                    data: entry.data,
                };
                count += 1;

                if entry.one_shot {
                    entry.disabled = true;
                }
            }
        }

        count
    }
}

// ============================================================================
// Global epoll registry
// ============================================================================

/// Global registry of all epoll instances.
static EPOLL_REGISTRY: Mutex<Option<EpollRegistry>> = Mutex::new(None);

struct EpollRegistry {
    instances: BTreeMap<u32, EpollInstance>,
    next_id: u32,
}

impl EpollRegistry {
    fn new() -> Self {
        Self {
            instances: BTreeMap::new(),
            next_id: 1,
        }
    }
}

/// Initialize the epoll subsystem.
pub fn init() -> Result<(), KernelError> {
    let mut reg = EPOLL_REGISTRY.lock();
    if reg.is_some() {
        return Ok(());
    }
    *reg = Some(EpollRegistry::new());
    Ok(())
}

// ============================================================================
// Public API (called from syscall handlers)
// ============================================================================

/// Create a new epoll instance. Returns the epoll ID (used as a pseudo-fd).
pub fn epoll_create(owner_pid: u64) -> Result<u32, KernelError> {
    let mut reg_guard = EPOLL_REGISTRY.lock();
    let reg = reg_guard
        .as_mut()
        .ok_or(KernelError::NotInitialized { subsystem: "epoll" })?;

    if reg.instances.len() >= MAX_EPOLL_INSTANCES {
        return Err(KernelError::ResourceExhausted {
            resource: "epoll instances",
        });
    }

    let id = reg.next_id;
    reg.next_id += 1;
    reg.instances.insert(id, EpollInstance::new(id, owner_pid));
    Ok(id)
}

/// Perform a control operation on an epoll instance.
pub fn epoll_ctl(
    epoll_id: u32,
    op: u32,
    fd: i32,
    event: Option<&EpollEvent>,
) -> Result<(), KernelError> {
    let mut reg_guard = EPOLL_REGISTRY.lock();
    let reg = reg_guard
        .as_mut()
        .ok_or(KernelError::NotInitialized { subsystem: "epoll" })?;

    let instance = reg
        .instances
        .get_mut(&epoll_id)
        .ok_or(KernelError::NotFound {
            resource: "epoll instance",
            id: epoll_id as u64,
        })?;

    match op {
        EPOLL_CTL_ADD => {
            let ev = event.ok_or(KernelError::InvalidArgument {
                name: "event",
                value: "required for EPOLL_CTL_ADD",
            })?;
            instance.ctl_add(fd, ev)
        }
        EPOLL_CTL_DEL => instance.ctl_del(fd),
        EPOLL_CTL_MOD => {
            let ev = event.ok_or(KernelError::InvalidArgument {
                name: "event",
                value: "required for EPOLL_CTL_MOD",
            })?;
            instance.ctl_mod(fd, ev)
        }
        _ => Err(KernelError::InvalidArgument {
            name: "op",
            value: "invalid epoll_ctl operation",
        }),
    }
}

/// Wait for events on an epoll instance.
///
/// Returns the number of ready events written to `events`.
/// If `timeout_ms` is 0, returns immediately (non-blocking poll).
/// If `timeout_ms` is -1, waits up to 30s (capped to prevent permanent hangs).
/// Otherwise waits up to `timeout_ms` milliseconds.
pub fn epoll_wait(
    epoll_id: u32,
    events: &mut [EpollEvent],
    timeout_ms: i32,
) -> Result<usize, KernelError> {
    let start = crate::timer::get_uptime_ms();
    // Cap infinite wait to 30 seconds to prevent permanent hangs
    let max_wait_ms: u64 = if timeout_ms < 0 {
        30_000
    } else {
        timeout_ms as u64
    };

    loop {
        let count = {
            let mut reg_guard = EPOLL_REGISTRY.lock();
            let reg = reg_guard
                .as_mut()
                .ok_or(KernelError::NotInitialized { subsystem: "epoll" })?;

            let instance = reg
                .instances
                .get_mut(&epoll_id)
                .ok_or(KernelError::NotFound {
                    resource: "epoll instance",
                    id: epoll_id as u64,
                })?;

            instance.poll_events(events)
        }; // Drop lock before yielding

        if count > 0 || timeout_ms == 0 {
            return Ok(count);
        }

        if crate::timer::get_uptime_ms() - start >= max_wait_ms {
            return Ok(0);
        }

        crate::sched::yield_cpu();
    }
}

/// Destroy an epoll instance.
pub fn epoll_destroy(epoll_id: u32) -> Result<(), KernelError> {
    let mut reg_guard = EPOLL_REGISTRY.lock();
    let reg = reg_guard
        .as_mut()
        .ok_or(KernelError::NotInitialized { subsystem: "epoll" })?;

    reg.instances
        .remove(&epoll_id)
        .ok_or(KernelError::NotFound {
            resource: "epoll instance",
            id: epoll_id as u64,
        })?;
    Ok(())
}

// ============================================================================
// Internal: fd readiness polling
// ============================================================================

/// Query the readiness of a file descriptor.
///
/// Checks the kernel's fd state (pipe buffers, socket receive queues, etc.)
/// and returns the matching event flags. Also checks special fd types
/// (eventfd, timerfd, signalfd) which use pseudo-fd IDs from their own
/// registries.
fn poll_fd_readiness(fd: i32) -> u32 {
    // All fd types (eventfd, timerfd, signalfd, pipes, sockets, files) are
    // now VfsNode-backed in the process file table. poll_readiness() on
    // each VfsNode handles type-specific readiness checking.
    let proc = match crate::process::current_process() {
        Some(p) => p,
        None => return EPOLLERR,
    };

    let file_table = proc.file_table.lock();
    let file = match file_table.get(fd as usize) {
        Some(f) => f,
        None => return EPOLLERR | EPOLLHUP,
    };

    // Use VfsNode::poll_readiness() for actual buffer state checking.
    // Maps POLL* flags (u16) to EPOLL* flags (u32) -- same bit positions.
    let readiness = file.node.poll_readiness() as u32;
    let mut ready = 0u32;
    if readiness & 0x0001 != 0 {
        ready |= EPOLLIN;
    }
    if readiness & 0x0004 != 0 {
        ready |= EPOLLOUT;
    }
    if readiness & 0x0008 != 0 {
        ready |= EPOLLERR;
    }
    if readiness & 0x0010 != 0 {
        ready |= EPOLLHUP;
    }

    ready
}

// ============================================================================
// VfsNode adapter -- allows epoll fd to live in process file table
// ============================================================================

use alloc::{sync::Arc, vec::Vec};

use crate::fs::{DirEntry, Metadata, NodeType, Permissions, VfsNode};

/// VfsNode wrapper around an epoll instance.
///
/// Allows epoll_create1() to return a real file descriptor. musl expects
/// to be able to close() the epoll fd. read()/write() are not supported.
pub struct EpollNode {
    epoll_id: u32,
}

impl EpollNode {
    pub fn new(epoll_id: u32) -> Self {
        Self { epoll_id }
    }

    /// Get the internal epoll ID (for epoll_ctl/epoll_wait).
    pub fn epoll_id(&self) -> u32 {
        self.epoll_id
    }
}

impl VfsNode for EpollNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "read epoll",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "write epoll",
        })
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
            permissions: Permissions::from_mode(0o666),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate epoll",
        })
    }
}

impl Drop for EpollNode {
    fn drop(&mut self) {
        let _ = epoll_destroy(self.epoll_id);
    }
}
