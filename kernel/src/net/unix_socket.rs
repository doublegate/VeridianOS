//! Unix Domain Sockets (AF_UNIX)
//!
//! Provides local inter-process communication via the BSD socket API with
//! path-based addressing. Used by Wayland compositors for client-server
//! communication and by system daemons for local service connections.
//!
//! Supported features:
//! - Stream (SOCK_STREAM) and datagram (SOCK_DGRAM) modes
//! - Binding to a socket file (`/run/wayland-0`, `/tmp/.X11-unix/X0`): bind
//!   makes the file and a connect finds the socket through it, by inode, as
//!   Linux's `unix_find_bsd` (the caller makes and resolves it)
//! - `socketpair()` for anonymous connected socket pairs
//! - SCM_RIGHTS for file descriptor passing (Wayland buffer handles)
//! - Backlog queue for pending connections

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    fs::VfsNode,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum path length for a Unix socket address.
pub const UNIX_PATH_MAX: usize = 108;

/// Maximum backlog for pending connections.
pub const UNIX_BACKLOG_MAX: usize = 128;

/// Maximum message size for datagram mode.
pub const UNIX_DGRAM_MAX: usize = 65536;

/// Maximum number of file descriptors in a single SCM_RIGHTS message.
pub const SCM_RIGHTS_MAX: usize = 16;

/// Maximum number of concurrent Unix sockets.
pub const UNIX_SOCKET_MAX: usize = 1024;

// ---------------------------------------------------------------------------
// Socket Types
// ---------------------------------------------------------------------------

/// Unix socket type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixSocketType {
    /// Reliable, ordered, connection-oriented byte stream (SOCK_STREAM).
    Stream,
    /// Unreliable, unordered, connectionless datagrams (SOCK_DGRAM).
    Datagram,
}

/// Unix socket state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixSocketState {
    /// Newly created, not yet bound or connected.
    Unbound,
    /// Bound to a path, not yet listening or connected.
    Bound,
    /// Listening for incoming connections (stream only).
    Listening,
    /// Connected to a peer.
    Connected,
    /// Connection closed.
    Closed,
}

/// Ancillary data for SCM_RIGHTS file descriptor passing.
///
/// Carries the open files themselves, not the sender's fd numbers: an fd
/// number means nothing in the receiver's table. The receiver installs each
/// file in its own table and gets fresh fd numbers.
#[derive(Clone)]
pub struct ScmRights {
    /// Open files being passed.
    pub files: Vec<alloc::sync::Arc<crate::fs::file::File>>,
}

impl core::fmt::Debug for ScmRights {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScmRights")
            .field("files", &self.files.len())
            .finish()
    }
}

/// Receive-buffer charge per message, on top of its data.
const MSG_OVERHEAD: usize = 64;
/// Receive-buffer charge per file carried by a message.
const FILE_CHARGE: usize = 256;

/// A message in the Unix socket buffer.
#[derive(Debug, Clone)]
pub struct UnixMessage {
    /// Message payload.
    pub data: Vec<u8>,
    /// Optional SCM_RIGHTS ancillary data.
    pub rights: Option<ScmRights>,
    /// Sender socket ID (for datagram mode).
    pub sender: u64,
}

impl UnixMessage {
    /// What this message counts against the receiver's buffer limit.
    pub fn charge(&self) -> usize {
        let files = self.rights.as_ref().map_or(0, |r| r.files.len());
        self.data.len() + MSG_OVERHEAD + files * FILE_CHARGE
    }
}

// ---------------------------------------------------------------------------
// Unix Socket Structure
// ---------------------------------------------------------------------------

/// A Unix domain socket.
pub struct UnixSocket {
    /// Unique socket ID.
    pub id: u64,
    /// Socket type (stream or datagram).
    pub socket_type: UnixSocketType,
    /// Current state.
    pub state: UnixSocketState,
    /// Bound path as given to bind, for getsockname (None if unbound or
    /// anonymous).
    pub path: Option<String>,
    /// The socket file it is bound to, by `fs::inode_key`.
    bound_key: Option<NameKey>,
    /// A bind has reserved the socket and is making its socket file
    /// ([`socket_reserve_bind`]).
    binding: bool,
    /// Peer socket ID (for connected stream sockets).
    pub peer_id: Option<u64>,
    /// Receive buffer (incoming messages).
    pub recv_buffer: VecDeque<UnixMessage>,
    /// Maximum receive buffer size in bytes.
    pub recv_buffer_max: usize,
    /// Current receive buffer size in bytes.
    pub recv_buffer_used: usize,
    /// Pending connection queue (for listening sockets).
    pub pending_connections: VecDeque<u64>,
    /// Backlog limit for pending connections.
    pub backlog: usize,
    /// Whether the socket has been shut down for reading.
    pub shutdown_read: bool,
    /// Whether the socket has been shut down for writing.
    pub shutdown_write: bool,
    /// Owning process ID.
    pub owner_pid: u64,
    /// The credentials the peer had when the connection was made
    /// (SO_PEERCRED): the other end's creator for a socketpair, the
    /// listener's at listen() for a connecting socket, the connecting
    /// process's at connect() for an accepted one. `None` unconnected.
    pub peer_cred: Option<PeerCred>,
    /// This socket's own credentials when it last listened or connected:
    /// what its peer is given.
    pub own_cred: PeerCred,
}

/// A process's credentials as SO_PEERCRED reports them (`struct ucred`):
/// its process ID and effective user and group IDs.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

impl PeerCred {
    /// What SO_PEERCRED reports with no peer: process 0 and the overflow
    /// user and group (Linux's overflowuid and overflowgid).
    pub const NONE: Self = Self {
        pid: 0,
        uid: 65534,
        gid: 65534,
    };

    /// The calling process's (root with process 0 in kernel context).
    pub fn current() -> Self {
        crate::process::current_process().map_or(
            Self {
                pid: 0,
                uid: 0,
                gid: 0,
            },
            |p| {
                let c = p.credentials();
                Self {
                    pid: p.pid.0 as i32,
                    uid: c.euid,
                    gid: c.egid,
                }
            },
        )
    }
}

impl UnixSocket {
    /// A socket of `socket_type` owned by `owner_pid` with credentials
    /// `own_cred` (taken before the socket lock: the process's locks are
    /// never taken under it).
    fn new(id: u64, socket_type: UnixSocketType, owner_pid: u64, own_cred: PeerCred) -> Self {
        Self {
            id,
            socket_type,
            state: UnixSocketState::Unbound,
            path: None,
            bound_key: None,
            binding: false,
            peer_id: None,
            recv_buffer: VecDeque::new(),
            recv_buffer_max: 65536,
            recv_buffer_used: 0,
            pending_connections: VecDeque::new(),
            backlog: 0,
            shutdown_read: false,
            shutdown_write: false,
            owner_pid,
            peer_cred: None,
            own_cred,
        }
    }
}

/// The credentials of the peer of socket `socket_id` (SO_PEERCRED), or
/// [`PeerCred::NONE`] if it has none.
pub fn peer_cred(socket_id: u64) -> Option<PeerCred> {
    UNIX_SOCKETS
        .lock()
        .get(&socket_id)
        .map(|s| s.peer_cred.unwrap_or(PeerCred::NONE))
}

// ---------------------------------------------------------------------------
// Global Registry
// ---------------------------------------------------------------------------

/// Next unique socket ID.
static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

/// Global registry of all Unix sockets, keyed by socket ID.
static UNIX_SOCKETS: Mutex<BTreeMap<u64, UnixSocket>> = Mutex::new(BTreeMap::new());

/// Which socket file a bound socket holds: the file's `fs::inode_key`.
pub type NameKey = (u64, u64);

/// A bound socket and its socket file, held so that a key made from the
/// node's address cannot be reused while the binding lasts.
struct BoundName {
    socket_id: u64,
    _file: Arc<dyn VfsNode>,
}

/// Socket file to bound socket. An entry lasts as long as the socket: as on
/// Linux, unlinking the file leaves the socket bound but unreachable, and a
/// closed socket leaves its file behind.
static PATH_REGISTRY: Mutex<BTreeMap<NameKey, BoundName>> = Mutex::new(BTreeMap::new());

/// The socket file `key` names is gone (`fs::special_inode_freed`): a
/// socket still bound to it stays bound but cannot be reached, and a new
/// file given the same key can be bound.
pub fn forget_file(key: NameKey) {
    let stale = PATH_REGISTRY.lock().remove(&key);
    // The node, after the lock: dropping it may reach its filesystem.
    drop(stale);
}

/// The socket bound to `file`, if any.
fn bound_to(file: &Arc<dyn VfsNode>) -> KernelResult<u64> {
    let key = crate::fs::inode_key(file);
    PATH_REGISTRY
        .lock()
        .get(&key)
        .map(|b| b.socket_id)
        .ok_or(KernelError::NotFound {
            resource: "unix_socket_path",
            id: 0,
        })
}

// ---------------------------------------------------------------------------
// Socket API
// ---------------------------------------------------------------------------

/// Reports a readiness change (data, room, a connection, a close) to
/// blocked readers, writers and pollers when dropped. Declared first in a
/// function, it drops last, after the function's own locks.
struct ReadinessChange;

impl Drop for ReadinessChange {
    fn drop(&mut self) {
        #[cfg(feature = "alloc")]
        crate::sched::dispatch::io_event();
    }
}

/// Create a new Unix domain socket.
///
/// Returns the socket ID.
pub fn socket_create(socket_type: UnixSocketType, owner_pid: u64) -> KernelResult<u64> {
    let sockets = UNIX_SOCKETS.lock();
    if sockets.len() >= UNIX_SOCKET_MAX {
        return Err(KernelError::ResourceExhausted {
            resource: "unix_sockets",
        });
    }
    drop(sockets);

    let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let socket = UnixSocket::new(id, socket_type, owner_pid, PeerCred::current());

    UNIX_SOCKETS.lock().insert(id, socket);
    Ok(id)
}

/// Reserve an unbound socket for a bind, before its socket file is made:
/// a socket that is bound, or that another bind has reserved, is
/// InvalidState (EINVAL). With the reservation held, [`socket_bind`] can
/// fail only for a file another socket holds, so a bind never has to take
/// the file it made back out by path. [`socket_bind`] ends the reservation;
/// [`socket_release_bind`] ends it if the file could not be made.
pub fn socket_reserve_bind(socket_id: u64) -> KernelResult<()> {
    let mut sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;
    if socket.state != UnixSocketState::Unbound || socket.path.is_some() || socket.binding {
        return Err(KernelError::InvalidState {
            expected: "unbound",
            actual: "bound or binding",
        });
    }
    socket.binding = true;
    Ok(())
}

/// End a reservation from [`socket_reserve_bind`] without binding.
pub fn socket_release_bind(socket_id: u64) {
    if let Some(socket) = UNIX_SOCKETS.lock().get_mut(&socket_id) {
        socket.binding = false;
    }
}

/// Bind a Unix socket to the socket file `file`, made for it at `path`.
pub fn socket_bind(socket_id: u64, file: Arc<dyn VfsNode>, path: &str) -> KernelResult<()> {
    if path.is_empty() || path.len() > UNIX_PATH_MAX {
        return Err(KernelError::InvalidArgument {
            name: "path",
            value: "empty or exceeds UNIX_PATH_MAX",
        });
    }

    let key = crate::fs::inode_key(&file);
    let mut paths = PATH_REGISTRY.lock();
    if paths.contains_key(&key) {
        return Err(KernelError::AlreadyExists {
            resource: "unix_socket_path",
            id: 0,
        });
    }

    let mut sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    socket.binding = false;
    if socket.state != UnixSocketState::Unbound {
        return Err(KernelError::InvalidState {
            expected: "unbound",
            actual: "already bound or connected",
        });
    }

    socket.path = Some(path.to_string());
    socket.bound_key = Some(key);
    socket.state = UnixSocketState::Bound;
    paths.insert(
        key,
        BoundName {
            socket_id,
            _file: file,
        },
    );

    Ok(())
}

/// Start listening for incoming connections (stream sockets only).
pub fn socket_listen(socket_id: u64, backlog: usize) -> KernelResult<()> {
    let _changed = ReadinessChange;
    let cred = PeerCred::current();
    let mut sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    if socket.socket_type != UnixSocketType::Stream {
        return Err(KernelError::InvalidArgument {
            name: "socket_type",
            value: "listen requires SOCK_STREAM",
        });
    }

    if socket.state != UnixSocketState::Bound {
        return Err(KernelError::InvalidState {
            expected: "bound",
            actual: "not bound",
        });
    }

    socket.backlog = backlog.min(UNIX_BACKLOG_MAX);
    socket.state = UnixSocketState::Listening;
    // What connecting sockets will see as their peer's (Linux's
    // init_peercred at listen).
    socket.own_cred = cred;
    Ok(())
}

/// Connect a stream socket to the listening socket bound to the socket
/// file `file` (NotFound if none is).
///
/// Returns Ok(()) on success. The connection is immediately established
/// (no three-way handshake for local sockets).
pub fn socket_connect(socket_id: u64, file: &Arc<dyn VfsNode>) -> KernelResult<()> {
    let _changed = ReadinessChange;
    let target_id = bound_to(file)?;

    let cred = PeerCred::current();
    let mut sockets = UNIX_SOCKETS.lock();

    // Verify target is listening.
    let target = sockets.get(&target_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: target_id,
    })?;

    if target.state != UnixSocketState::Listening {
        return Err(KernelError::InvalidState {
            expected: "listening",
            actual: "not listening",
        });
    }

    if target.pending_connections.len() >= target.backlog {
        return Err(KernelError::ResourceExhausted {
            resource: "connection backlog",
        });
    }

    // Enqueue the connection request.
    let target = sockets.get_mut(&target_id).ok_or(KernelError::NotFound {
        resource: "unix_socket_target",
        id: target_id,
    })?;
    target.pending_connections.push_back(socket_id);
    let listener_cred = target.own_cred;

    // Mark the connecting socket as connected (peer will be set on accept).
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;
    socket.peer_id = Some(target_id);
    socket.state = UnixSocketState::Connected;
    // The client sees the listener's credentials; the accepted socket will
    // see the client's as they are now.
    socket.peer_cred = Some(listener_cred);
    socket.own_cred = cred;

    Ok(())
}

/// Accept a pending connection on a listening socket.
///
/// Creates a new connected socket and returns its ID along with the
/// connecting socket's ID.
pub fn socket_accept(listen_socket_id: u64) -> KernelResult<(u64, u64)> {
    let _changed = ReadinessChange;
    let mut sockets = UNIX_SOCKETS.lock();

    let listen = sockets
        .get_mut(&listen_socket_id)
        .ok_or(KernelError::NotFound {
            resource: "unix_socket",
            id: listen_socket_id,
        })?;

    if listen.state != UnixSocketState::Listening {
        return Err(KernelError::InvalidState {
            expected: "listening",
            actual: "not listening",
        });
    }

    let connecting_id = listen
        .pending_connections
        .pop_front()
        .ok_or(KernelError::WouldBlock)?;

    let owner_pid = listen.owner_pid;
    let listener_cred = listen.own_cred;

    // Create a new server-side socket for this connection.
    let new_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let mut new_socket = UnixSocket::new(new_id, UnixSocketType::Stream, owner_pid, listener_cred);
    new_socket.state = UnixSocketState::Connected;
    new_socket.peer_id = Some(connecting_id);

    // Update the connecting socket's peer to point to the new server socket.
    if let Some(connecting) = sockets.get_mut(&connecting_id) {
        connecting.peer_id = Some(new_id);
        new_socket.peer_cred = Some(connecting.own_cred);
    }

    sockets.insert(new_id, new_socket);

    Ok((new_id, connecting_id))
}

/// Whether an accept on `listen_socket_id` would not block: a connection
/// is pending, or the socket is gone or no longer listening (the accept
/// then fails).
pub fn accept_ready(listen_socket_id: u64) -> bool {
    UNIX_SOCKETS
        .lock()
        .get(&listen_socket_id)
        .is_none_or(|s| s.state != UnixSocketState::Listening || !s.pending_connections.is_empty())
}

/// Send data on a connected socket.
pub fn socket_send(socket_id: u64, data: &[u8], rights: Option<ScmRights>) -> KernelResult<usize> {
    let _changed = ReadinessChange;
    let sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    // A stream shut down for writing -- by shutdown(SHUT_WR), or because
    // its peer closed -- is EPIPE, as on Linux (the caller adds SIGPIPE).
    if socket.shutdown_write {
        if socket.socket_type == UnixSocketType::Stream {
            return Err(KernelError::BrokenPipe);
        }
        return Err(KernelError::InvalidState {
            expected: "write enabled",
            actual: "shutdown for writing",
        });
    }

    let peer_id = socket.peer_id.ok_or(KernelError::InvalidState {
        expected: "connected",
        actual: "not connected",
    })?;
    let is_stream = socket.socket_type == UnixSocketType::Stream;
    drop(sockets);

    // A stream has no record boundaries, so a send with no bytes and no
    // files transfers nothing. Queuing it would make the peer's recv return
    // 0, which reads as EOF, and would charge MSG_OVERHEAD per empty send
    // (review of the v0.26.0 stack, PR #10). Datagrams keep empty messages.
    if is_stream && data.is_empty() && rights.as_ref().is_none_or(|r| r.files.is_empty()) {
        return Ok(0);
    }

    // Deliver to peer's receive buffer.
    let mut sockets = UNIX_SOCKETS.lock();
    let Some(peer) = sockets.get_mut(&peer_id) else {
        return Err(if is_stream {
            KernelError::BrokenPipe
        } else {
            KernelError::NotFound {
                resource: "unix_socket",
                id: peer_id,
            }
        });
    };

    if peer.shutdown_read {
        if is_stream {
            return Err(KernelError::BrokenPipe);
        }
        return Err(KernelError::InvalidState {
            expected: "read enabled",
            actual: "peer shutdown for reading",
        });
    }

    let msg = UnixMessage {
        data: data.to_vec(),
        rights,
        sender: socket_id,
    };
    // Charge the message, not just its bytes: otherwise zero-length
    // messages carrying files could queue without limit.
    let charge = msg.charge();
    if peer.recv_buffer_used + charge > peer.recv_buffer_max {
        // `msg` (and any files in it) is dropped after the lock below.
        drop(sockets);
        return Err(KernelError::ResourceExhausted {
            resource: "recv_buffer",
        });
    }
    let len = data.len();
    peer.recv_buffer_used += charge;
    peer.recv_buffer.push_back(msg);

    Ok(len)
}

/// Receive data from a connected or datagram socket.
///
/// Returns the number of bytes received and optional SCM_RIGHTS data.
pub fn socket_recv(socket_id: u64, buf: &mut [u8]) -> KernelResult<(usize, Option<ScmRights>)> {
    let _changed = ReadinessChange;
    let mut sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    // Data the peer sent before it closed is still read; end-of-file
    // comes once it is drained (it used to be discarded).
    let Some(mut msg) = socket.recv_buffer.pop_front() else {
        return if socket.shutdown_read {
            Ok((0, None)) // EOF
        } else {
            Err(KernelError::WouldBlock)
        };
    };

    let copy_len = buf.len().min(msg.data.len());
    buf[..copy_len].copy_from_slice(&msg.data[..copy_len]);
    socket.recv_buffer_used = socket.recv_buffer_used.saturating_sub(msg.charge());
    // A stream is bytes, not messages: what did not fit stays queued for
    // the next read (it used to be dropped). A datagram is truncated.
    if socket.socket_type == UnixSocketType::Stream && copy_len < msg.data.len() {
        let rest = UnixMessage {
            data: msg.data.split_off(copy_len),
            rights: None,
            sender: msg.sender,
        };
        socket.recv_buffer_used += rest.charge();
        socket.recv_buffer.push_front(rest);
    }

    // The files are returned to the caller, so they are released (if
    // unused) after this lock is dropped.
    Ok((copy_len, msg.rights))
}

/// The type of socket `socket_id`, if it exists.
pub fn socket_type(socket_id: u64) -> Option<UnixSocketType> {
    UNIX_SOCKETS.lock().get(&socket_id).map(|s| s.socket_type)
}

/// Create an anonymous connected socket pair (socketpair).
///
/// Returns (socket_a_id, socket_b_id) where both sockets are connected.
pub fn socketpair(socket_type: UnixSocketType, owner_pid: u64) -> KernelResult<(u64, u64)> {
    let id_a = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let id_b = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);

    let cred = PeerCred::current();
    let mut sock_a = UnixSocket::new(id_a, socket_type, owner_pid, cred);
    let mut sock_b = UnixSocket::new(id_b, socket_type, owner_pid, cred);

    sock_a.state = UnixSocketState::Connected;
    sock_a.peer_id = Some(id_b);
    sock_b.state = UnixSocketState::Connected;
    sock_b.peer_id = Some(id_a);
    // Each end's peer is the creator.
    sock_a.peer_cred = Some(sock_b.own_cred);
    sock_b.peer_cred = Some(sock_a.own_cred);

    let mut sockets = UNIX_SOCKETS.lock();
    sockets.insert(id_a, sock_a);
    sockets.insert(id_b, sock_b);

    Ok((id_a, id_b))
}

/// Close a Unix socket.
pub fn socket_close(socket_id: u64) -> KernelResult<()> {
    let _changed = ReadinessChange;
    let mut sockets = UNIX_SOCKETS.lock();

    let removed = sockets.remove(&socket_id);
    let mut unbound = None;
    if let Some(socket) = removed.as_ref() {
        // Remove the binding; the file stays, as on Linux.
        // Only its own: once its file was freed the key may name another
        // socket's file.
        if let Some(key) = socket.bound_key {
            let mut paths = PATH_REGISTRY.lock();
            if paths.get(&key).is_some_and(|b| b.socket_id == socket_id) {
                unbound = paths.remove(&key);
            }
        }

        // Notify peer of disconnection.
        if let Some(peer_id) = socket.peer_id {
            if let Some(peer) = sockets.get_mut(&peer_id) {
                peer.peer_id = None;
                peer.shutdown_read = true;
                peer.shutdown_write = true;
            }
        }
    }
    // Drop the socket -- and the files queued in its buffer -- only after
    // the lock is released: the last reference to a queued socket file
    // closes that socket, which takes this lock again.
    drop(sockets);
    drop(removed);
    // The socket file's node too: dropping it may reach its filesystem.
    drop(unbound);

    Ok(())
}

/// Send a datagram to the socket bound to the socket file `dest`
/// (connectionless).
pub fn socket_sendto(socket_id: u64, data: &[u8], dest: &Arc<dyn VfsNode>) -> KernelResult<usize> {
    let _changed = ReadinessChange;
    if data.len() > UNIX_DGRAM_MAX {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "exceeds UNIX_DGRAM_MAX",
        });
    }

    let dest_id = bound_to(dest)?;

    let mut sockets = UNIX_SOCKETS.lock();
    let dest = sockets.get_mut(&dest_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: dest_id,
    })?;

    let msg = UnixMessage {
        data: data.to_vec(),
        rights: None,
        sender: socket_id,
    };
    let charge = msg.charge();
    if dest.recv_buffer_used + charge > dest.recv_buffer_max {
        return Err(KernelError::ResourceExhausted {
            resource: "recv_buffer",
        });
    }
    let len = data.len();
    dest.recv_buffer_used += charge;
    dest.recv_buffer.push_back(msg);

    Ok(len)
}

/// Get the number of active Unix sockets.
pub fn socket_count() -> usize {
    UNIX_SOCKETS.lock().len()
}

/// Check if a socket ID exists in the unix socket registry.
pub fn socket_exists(socket_id: u64) -> bool {
    UNIX_SOCKETS.lock().contains_key(&socket_id)
}

/// Poll readiness of a unix socket for epoll/poll integration.
///
/// Returns POLL-style flags:
/// - 0x0001 (POLLIN): data available to read, or pending connection to accept
/// - 0x0004 (POLLOUT): socket is writable (connected and send buffer has space)
/// - 0x0008 (POLLERR): error condition
/// - 0x0010 (POLLHUP): peer disconnected
pub fn socket_poll_readiness(socket_id: u64) -> u16 {
    let sockets = UNIX_SOCKETS.lock();
    let socket = match sockets.get(&socket_id) {
        Some(s) => s,
        None => return 0x0008 | 0x0010, // POLLERR | POLLHUP for invalid socket
    };

    let mut events = 0u16;

    match socket.state {
        UnixSocketState::Listening => {
            // Listening socket: POLLIN if pending connections
            if !socket.pending_connections.is_empty() {
                events |= 0x0001; // POLLIN
            }
        }
        UnixSocketState::Connected => {
            // Connected socket: POLLIN if data in recv buffer
            if !socket.recv_buffer.is_empty() {
                events |= 0x0001; // POLLIN
            }
            // POLLOUT if we can send (peer exists and buffer has space)
            if !socket.shutdown_write {
                events |= 0x0004; // POLLOUT
            }
            // POLLHUP if peer shut down
            if socket.shutdown_read {
                events |= 0x0010; // POLLHUP
            }
        }
        UnixSocketState::Unbound | UnixSocketState::Bound => {
            // Not yet connected or listening -- writable
            events |= 0x0004; // POLLOUT
        }
        UnixSocketState::Closed => {
            events |= 0x0010; // POLLHUP
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A socket is found through the file it is bound to, not a name: a
    /// second bind to the file fails, another file reaches nothing, and the
    /// binding ends with the socket.
    #[test]
    fn binding_is_by_socket_file() {
        use crate::fs::{ramfs::RamFs, Filesystem, NodeType, Permissions};
        let fs = RamFs::new();
        let perms = Permissions::from_mode(0o777);
        let a = fs.root().mknod("a", NodeType::Socket, perms, 0).unwrap();
        let b = fs.root().mknod("b", NodeType::Socket, perms, 0).unwrap();
        let srv = socket_create(UnixSocketType::Stream, 1).unwrap();
        let other = socket_create(UnixSocketType::Stream, 1).unwrap();
        let cli = socket_create(UnixSocketType::Stream, 1).unwrap();
        // One bind at a time reserves a socket; a bound one cannot be.
        assert_eq!(socket_reserve_bind(srv), Ok(()));
        assert!(socket_reserve_bind(srv).is_err());
        socket_release_bind(srv);
        assert_eq!(socket_reserve_bind(srv), Ok(()));
        assert_eq!(socket_bind(srv, a.clone(), "/x/a"), Ok(()));
        assert!(socket_reserve_bind(srv).is_err());
        assert!(matches!(
            socket_bind(other, a.clone(), "/y/a"),
            Err(KernelError::AlreadyExists { .. })
        ));
        socket_listen(srv, 4).unwrap();
        assert!(matches!(
            socket_connect(cli, &b),
            Err(KernelError::NotFound { .. })
        ));
        assert_eq!(socket_connect(cli, &a), Ok(()));
        socket_close(srv).unwrap();
        let late = socket_create(UnixSocketType::Stream, 1).unwrap();
        assert!(matches!(
            socket_connect(late, &a),
            Err(KernelError::NotFound { .. })
        ));
        // The file is free again once its socket is gone.
        assert_eq!(socket_bind(other, a, "/x/a"), Ok(()));
        for s in [other, cli, late] {
            socket_close(s).unwrap();
        }
    }

    /// BlockFS finds a file by inode number and reuses a freed number:
    /// a new socket file given the number of an unlinked one that a socket
    /// is still bound to must be bindable, reach only its own socket, and
    /// survive the old socket's close.
    #[test]
    fn freed_blockfs_socket_file_number_is_not_the_old_binding() {
        use crate::fs::{blockfs::BlockFs, Filesystem, NodeType, Permissions};
        let fs = BlockFs::format(1024, 64).unwrap();
        let root = fs.root();
        let perms = Permissions::from_mode(0o777);
        let old_file = root.mknod("s", NodeType::Socket, perms, 0).unwrap();
        let old = socket_create(UnixSocketType::Stream, 1).unwrap();
        socket_bind(old, old_file.clone(), "/s").unwrap();
        socket_listen(old, 4).unwrap();
        root.unlink("s").unwrap();
        let new_file = root.mknod("t", NodeType::Socket, perms, 0).unwrap();
        assert_eq!(
            crate::fs::inode_key(&new_file),
            crate::fs::inode_key(&old_file),
            "the test needs the inode number reused"
        );
        let new = socket_create(UnixSocketType::Stream, 1).unwrap();
        assert_eq!(socket_bind(new, new_file.clone(), "/t"), Ok(()));
        let cli = socket_create(UnixSocketType::Stream, 1).unwrap();
        // Not listening yet: it must not reach the old, listening socket.
        assert!(socket_connect(cli, &new_file).is_err());
        socket_close(old).unwrap();
        socket_listen(new, 4).unwrap();
        assert_eq!(socket_connect(cli, &new_file), Ok(()));
        for s in [new, cli] {
            socket_close(s).unwrap();
        }
    }

    /// A zero-length stream send carries nothing, so it must not queue a
    /// message: the peer's recv would return 0, which reads as EOF (review
    /// of the v0.26.0 stack, PR #10).
    #[test]
    fn empty_stream_send_queues_nothing() {
        let (a, b) = socketpair(UnixSocketType::Stream, 1).unwrap();
        for _ in 0..1_000 {
            assert_eq!(socket_send(a, &[], None), Ok(0));
        }
        let mut buf = [0u8; 8];
        assert!(matches!(
            socket_recv(b, &mut buf),
            Err(KernelError::WouldBlock)
        ));
        assert_eq!(UNIX_SOCKETS.lock().get(&b).unwrap().recv_buffer_used, 0);
        socket_close(a).unwrap();
        socket_close(b).unwrap();
    }

    /// A zero-length datagram is a real message and still queues.
    #[test]
    fn empty_datagram_send_still_queues() {
        let (a, b) = socketpair(UnixSocketType::Datagram, 1).unwrap();
        assert_eq!(socket_send(a, &[], None), Ok(0));
        let mut buf = [0u8; 8];
        assert!(matches!(socket_recv(b, &mut buf), Ok((0, None))));
        socket_close(a).unwrap();
        socket_close(b).unwrap();
    }

    /// SO_PEERCRED: a socketpair's ends see their creator; an unconnected
    /// socket sees no one (process 0, the overflow IDs).
    #[test]
    fn peer_credentials_are_recorded() {
        let (a, b) = socketpair(UnixSocketType::Stream, 1).unwrap();
        assert_eq!(peer_cred(a), Some(PeerCred::current()));
        assert_eq!(peer_cred(b), Some(PeerCred::current()));
        let lone = socket_create(UnixSocketType::Stream, 1).unwrap();
        assert_eq!(peer_cred(lone), Some(PeerCred::NONE));
        assert_eq!(peer_cred(u64::MAX), None);
        for s in [a, b, lone] {
            socket_close(s).unwrap();
        }
    }

    /// Sending on a stream whose peer has closed is EPIPE (Linux's
    /// unix_stream_sendmsg); a datagram socket keeps its own error.
    #[test]
    fn stream_send_to_closed_peer_is_broken_pipe() {
        let (a, b) = socketpair(UnixSocketType::Stream, 1).unwrap();
        socket_close(b).unwrap();
        assert_eq!(socket_send(a, b"x", None), Err(KernelError::BrokenPipe));
        socket_close(a).unwrap();

        let (c, d) = socketpair(UnixSocketType::Datagram, 1).unwrap();
        socket_close(d).unwrap();
        assert!(!matches!(
            socket_send(c, b"x", None),
            Ok(_) | Err(KernelError::BrokenPipe)
        ));
        socket_close(c).unwrap();
    }

    /// Data sent before the peer closed is read before end-of-file.
    #[test]
    fn data_before_close_is_read_before_eof() {
        let (a, b) = socketpair(UnixSocketType::Stream, 1).unwrap();
        assert_eq!(socket_send(a, b"bye", None), Ok(3));
        socket_close(a).unwrap();
        let mut buf = [0u8; 8];
        assert!(matches!(socket_recv(b, &mut buf), Ok((3, None))));
        assert_eq!(&buf[..3], b"bye");
        assert!(matches!(socket_recv(b, &mut buf), Ok((0, None))));
        socket_close(b).unwrap();
    }

    /// A short stream read leaves the rest queued; a datagram is cut.
    #[test]
    fn short_reads_keep_stream_bytes_and_cut_datagrams() {
        let (a, b) = socketpair(UnixSocketType::Stream, 1).unwrap();
        socket_send(a, b"abcdef", None).unwrap();
        let mut two = [0u8; 2];
        assert!(matches!(socket_recv(b, &mut two), Ok((2, None))));
        let mut rest = [0u8; 8];
        assert!(matches!(socket_recv(b, &mut rest), Ok((4, None))));
        assert_eq!(&rest[..4], b"cdef");
        socket_close(a).unwrap();
        socket_close(b).unwrap();

        let (c, d) = socketpair(UnixSocketType::Datagram, 1).unwrap();
        socket_send(c, b"abcdef", None).unwrap();
        assert!(matches!(socket_recv(d, &mut two), Ok((2, None))));
        assert!(matches!(
            socket_recv(d, &mut rest),
            Err(KernelError::WouldBlock)
        ));
        socket_close(c).unwrap();
        socket_close(d).unwrap();
    }
}
