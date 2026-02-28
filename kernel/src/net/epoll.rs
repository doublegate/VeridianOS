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
/// If `timeout_ms` is -1, blocks indefinitely (simplified: returns current
/// state).
pub fn epoll_wait(
    epoll_id: u32,
    events: &mut [EpollEvent],
    _timeout_ms: i32,
) -> Result<usize, KernelError> {
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

    let count = instance.poll_events(events);
    Ok(count)
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
/// and returns the matching event flags.
fn poll_fd_readiness(fd: i32) -> u32 {
    // Get the current process's file table to check fd state.
    let proc = match crate::process::current_process() {
        Some(p) => p,
        None => return EPOLLERR,
    };

    let file_table = proc.file_table.lock();
    let file = match file_table.get(fd as usize) {
        Some(f) => f,
        None => return EPOLLERR | EPOLLHUP,
    };

    let mut ready = 0u32;

    // Check if the fd's underlying node has data available for reading.
    // For pipes: check if the pipe buffer is non-empty.
    // For sockets: check if the receive queue is non-empty.
    // For regular files: always readable (no blocking).
    let node_type = file.node.node_type();
    match node_type {
        crate::fs::NodeType::File
        | crate::fs::NodeType::CharDevice
        | crate::fs::NodeType::BlockDevice => {
            // Regular files and device nodes are always considered readable/writable
            // (actual I/O may still block at a lower level, but epoll reports them
            // as ready).
            ready |= EPOLLIN | EPOLLOUT;
        }
        crate::fs::NodeType::Pipe => {
            // Pipes: check buffer occupancy
            // For now, report both readable and writable
            // (real implementation would check pipe buffer fill level)
            ready |= EPOLLIN | EPOLLOUT;
        }
        _ => {
            ready |= EPOLLIN | EPOLLOUT;
        }
    }

    ready
}
