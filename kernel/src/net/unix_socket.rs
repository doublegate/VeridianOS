//! Unix Domain Sockets (AF_UNIX)
//!
//! Provides local inter-process communication via the BSD socket API with
//! path-based addressing. Used by Wayland compositors for client-server
//! communication and by system daemons for local service connections.
//!
//! Supported features:
//! - Stream (SOCK_STREAM) and datagram (SOCK_DGRAM) modes
//! - Path-based binding (`/run/wayland-0`, `/tmp/.X11-unix/X0`)
//! - `socketpair()` for anonymous connected socket pairs
//! - SCM_RIGHTS for file descriptor passing (Wayland buffer handles)
//! - Backlog queue for pending connections

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::{String, ToString},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

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
#[derive(Debug, Clone)]
pub struct ScmRights {
    /// File descriptor numbers to pass.
    pub fds: Vec<u32>,
}

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
    /// Bound path (None if unbound or anonymous).
    pub path: Option<String>,
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
}

impl UnixSocket {
    fn new(id: u64, socket_type: UnixSocketType, owner_pid: u64) -> Self {
        Self {
            id,
            socket_type,
            state: UnixSocketState::Unbound,
            path: None,
            peer_id: None,
            recv_buffer: VecDeque::new(),
            recv_buffer_max: 65536,
            recv_buffer_used: 0,
            pending_connections: VecDeque::new(),
            backlog: 0,
            shutdown_read: false,
            shutdown_write: false,
            owner_pid,
        }
    }
}

// ---------------------------------------------------------------------------
// Global Registry
// ---------------------------------------------------------------------------

/// Next unique socket ID.
static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

/// Global registry of all Unix sockets, keyed by socket ID.
static UNIX_SOCKETS: Mutex<BTreeMap<u64, UnixSocket>> = Mutex::new(BTreeMap::new());

/// Path-to-socket-ID mapping for bound sockets.
static PATH_REGISTRY: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

// ---------------------------------------------------------------------------
// Socket API
// ---------------------------------------------------------------------------

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
    let socket = UnixSocket::new(id, socket_type, owner_pid);

    UNIX_SOCKETS.lock().insert(id, socket);
    Ok(id)
}

/// Bind a Unix socket to a filesystem path.
pub fn socket_bind(socket_id: u64, path: &str) -> KernelResult<()> {
    if path.is_empty() || path.len() > UNIX_PATH_MAX {
        return Err(KernelError::InvalidArgument {
            name: "path",
            value: "empty or exceeds UNIX_PATH_MAX",
        });
    }

    let mut paths = PATH_REGISTRY.lock();
    if paths.contains_key(path) {
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

    if socket.state != UnixSocketState::Unbound {
        return Err(KernelError::InvalidState {
            expected: "unbound",
            actual: "already bound or connected",
        });
    }

    socket.path = Some(path.to_string());
    socket.state = UnixSocketState::Bound;
    paths.insert(path.to_string(), socket_id);

    Ok(())
}

/// Start listening for incoming connections (stream sockets only).
pub fn socket_listen(socket_id: u64, backlog: usize) -> KernelResult<()> {
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
    Ok(())
}

/// Connect a stream socket to a listening socket at the given path.
///
/// Returns Ok(()) on success. The connection is immediately established
/// (no three-way handshake for local sockets).
pub fn socket_connect(socket_id: u64, path: &str) -> KernelResult<()> {
    // Find the target socket by path.
    let target_id = {
        let paths = PATH_REGISTRY.lock();
        *paths.get(path).ok_or(KernelError::NotFound {
            resource: "unix_socket_path",
            id: 0,
        })?
    };

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
    let target = sockets.get_mut(&target_id).unwrap();
    target.pending_connections.push_back(socket_id);

    // Mark the connecting socket as connected (peer will be set on accept).
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;
    socket.peer_id = Some(target_id);
    socket.state = UnixSocketState::Connected;

    Ok(())
}

/// Accept a pending connection on a listening socket.
///
/// Creates a new connected socket and returns its ID along with the
/// connecting socket's ID.
pub fn socket_accept(listen_socket_id: u64) -> KernelResult<(u64, u64)> {
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

    // Create a new server-side socket for this connection.
    let new_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let mut new_socket = UnixSocket::new(new_id, UnixSocketType::Stream, owner_pid);
    new_socket.state = UnixSocketState::Connected;
    new_socket.peer_id = Some(connecting_id);

    // Update the connecting socket's peer to point to the new server socket.
    if let Some(connecting) = sockets.get_mut(&connecting_id) {
        connecting.peer_id = Some(new_id);
    }

    sockets.insert(new_id, new_socket);

    Ok((new_id, connecting_id))
}

/// Send data on a connected socket.
pub fn socket_send(socket_id: u64, data: &[u8], rights: Option<ScmRights>) -> KernelResult<usize> {
    let sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    if socket.shutdown_write {
        return Err(KernelError::InvalidState {
            expected: "write enabled",
            actual: "shutdown for writing",
        });
    }

    let peer_id = socket.peer_id.ok_or(KernelError::InvalidState {
        expected: "connected",
        actual: "not connected",
    })?;
    drop(sockets);

    // Deliver to peer's receive buffer.
    let mut sockets = UNIX_SOCKETS.lock();
    let peer = sockets.get_mut(&peer_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: peer_id,
    })?;

    if peer.shutdown_read {
        return Err(KernelError::InvalidState {
            expected: "read enabled",
            actual: "peer shutdown for reading",
        });
    }

    if peer.recv_buffer_used + data.len() > peer.recv_buffer_max {
        return Err(KernelError::ResourceExhausted {
            resource: "recv_buffer",
        });
    }

    let msg = UnixMessage {
        data: data.to_vec(),
        rights,
        sender: socket_id,
    };
    let len = data.len();
    peer.recv_buffer_used += len;
    peer.recv_buffer.push_back(msg);

    Ok(len)
}

/// Receive data from a connected or datagram socket.
///
/// Returns the number of bytes received and optional SCM_RIGHTS data.
pub fn socket_recv(socket_id: u64, buf: &mut [u8]) -> KernelResult<(usize, Option<ScmRights>)> {
    let mut sockets = UNIX_SOCKETS.lock();
    let socket = sockets.get_mut(&socket_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: socket_id,
    })?;

    if socket.shutdown_read {
        return Ok((0, None)); // EOF
    }

    let msg = socket
        .recv_buffer
        .pop_front()
        .ok_or(KernelError::WouldBlock)?;

    let copy_len = buf.len().min(msg.data.len());
    buf[..copy_len].copy_from_slice(&msg.data[..copy_len]);
    socket.recv_buffer_used = socket.recv_buffer_used.saturating_sub(msg.data.len());

    Ok((copy_len, msg.rights))
}

/// Create an anonymous connected socket pair (socketpair).
///
/// Returns (socket_a_id, socket_b_id) where both sockets are connected.
pub fn socketpair(socket_type: UnixSocketType, owner_pid: u64) -> KernelResult<(u64, u64)> {
    let id_a = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let id_b = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);

    let mut sock_a = UnixSocket::new(id_a, socket_type, owner_pid);
    let mut sock_b = UnixSocket::new(id_b, socket_type, owner_pid);

    sock_a.state = UnixSocketState::Connected;
    sock_a.peer_id = Some(id_b);
    sock_b.state = UnixSocketState::Connected;
    sock_b.peer_id = Some(id_a);

    let mut sockets = UNIX_SOCKETS.lock();
    sockets.insert(id_a, sock_a);
    sockets.insert(id_b, sock_b);

    Ok((id_a, id_b))
}

/// Close a Unix socket.
pub fn socket_close(socket_id: u64) -> KernelResult<()> {
    let mut sockets = UNIX_SOCKETS.lock();

    if let Some(socket) = sockets.remove(&socket_id) {
        // Remove path binding if any.
        if let Some(ref path) = socket.path {
            PATH_REGISTRY.lock().remove(path);
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

    Ok(())
}

/// Send a datagram to a named socket (connectionless).
pub fn socket_sendto(socket_id: u64, data: &[u8], dest_path: &str) -> KernelResult<usize> {
    if data.len() > UNIX_DGRAM_MAX {
        return Err(KernelError::InvalidArgument {
            name: "data",
            value: "exceeds UNIX_DGRAM_MAX",
        });
    }

    let dest_id = {
        let paths = PATH_REGISTRY.lock();
        *paths.get(dest_path).ok_or(KernelError::NotFound {
            resource: "unix_socket_path",
            id: 0,
        })?
    };

    let mut sockets = UNIX_SOCKETS.lock();
    let dest = sockets.get_mut(&dest_id).ok_or(KernelError::NotFound {
        resource: "unix_socket",
        id: dest_id,
    })?;

    if dest.recv_buffer_used + data.len() > dest.recv_buffer_max {
        return Err(KernelError::ResourceExhausted {
            resource: "recv_buffer",
        });
    }

    let msg = UnixMessage {
        data: data.to_vec(),
        rights: None,
        sender: socket_id,
    };
    let len = data.len();
    dest.recv_buffer_used += len;
    dest.recv_buffer.push_back(msg);

    Ok(len)
}

/// Get the number of active Unix sockets.
pub fn socket_count() -> usize {
    UNIX_SOCKETS.lock().len()
}
