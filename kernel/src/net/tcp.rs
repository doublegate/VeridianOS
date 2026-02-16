//! TCP protocol implementation

use alloc::{collections::BTreeMap, vec::Vec};

use spin::Mutex;

use super::SocketAddr;
use crate::error::KernelError;

/// TCP header flags
#[derive(Debug, Clone, Copy)]
pub struct TcpFlags(u8);

impl TcpFlags {
    pub const FIN: u8 = 0x01;
    pub const SYN: u8 = 0x02;
    pub const RST: u8 = 0x04;
    pub const PSH: u8 = 0x08;
    pub const ACK: u8 = 0x10;
    pub const URG: u8 = 0x20;

    pub fn new(flags: u8) -> Self {
        Self(flags)
    }

    pub fn has(&self, flag: u8) -> bool {
        (self.0 & flag) != 0
    }

    pub fn set(&mut self, flag: u8) {
        self.0 |= flag;
    }
}

/// TCP connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// TCP connection
#[derive(Debug, Clone)]
pub struct TcpConnection {
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub state: TcpState,
    pub seq_num: u32,
    pub ack_num: u32,
    pub window_size: u16,
}

impl TcpConnection {
    pub fn new(local: SocketAddr, remote: SocketAddr) -> Self {
        Self {
            local,
            remote,
            state: TcpState::Closed,
            seq_num: 0,
            ack_num: 0,
            window_size: 65535,
        }
    }

    /// Initiate connection (active open)
    pub fn connect(&mut self) -> Result<(), KernelError> {
        if self.state != TcpState::Closed {
            return Err(KernelError::InvalidState {
                expected: "Closed",
                actual: "Other",
            });
        }

        // Send SYN
        self.state = TcpState::SynSent;
        // TODO(future): Construct and send SYN packet via IP layer

        Ok(())
    }

    /// Listen for connections (passive open)
    pub fn listen(&mut self) -> Result<(), KernelError> {
        if self.state != TcpState::Closed {
            return Err(KernelError::InvalidState {
                expected: "Closed",
                actual: "Other",
            });
        }

        self.state = TcpState::Listen;
        Ok(())
    }

    /// Send data
    pub fn send(&mut self, data: &[u8]) -> Result<usize, KernelError> {
        if self.state != TcpState::Established {
            return Err(KernelError::InvalidState {
                expected: "Established",
                actual: "Other",
            });
        }

        // TODO(future): Segment data and send via TCP with retransmission

        Ok(data.len())
    }

    /// Receive data
    pub fn recv(&mut self, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        if self.state != TcpState::Established {
            return Err(KernelError::InvalidState {
                expected: "Established",
                actual: "Other",
            });
        }

        // TODO(future): Receive reassembled data from TCP receive buffer

        Ok(0)
    }

    /// Close connection
    pub fn close(&mut self) -> Result<(), KernelError> {
        match self.state {
            TcpState::Established => {
                // Send FIN
                self.state = TcpState::FinWait1;
                Ok(())
            }
            TcpState::CloseWait => {
                // Send FIN
                self.state = TcpState::LastAck;
                Ok(())
            }
            _ => Err(KernelError::InvalidState {
                expected: "Established or CloseWait",
                actual: "Other",
            }),
        }
    }
}

/// Initialize TCP
pub fn init() -> Result<(), KernelError> {
    println!("[TCP] Initializing TCP protocol...");
    println!("[TCP] TCP initialized");
    Ok(())
}

// ============================================================================
// Socket Layer Interface
// ============================================================================

/// TCP connection state for socket layer
struct TcpSocketState {
    connection: TcpConnection,
    send_buffer: Vec<u8>,
    recv_buffer: Vec<u8>,
    send_seq: u32,
    recv_seq: u32,
}

/// Global TCP connection table
static TCP_CONNECTIONS: Mutex<BTreeMap<usize, TcpSocketState>> = Mutex::new(BTreeMap::new());

/// Transmit data from socket layer
pub fn transmit_data(socket_id: usize, data: &[u8], remote: SocketAddr) {
    let mut connections = TCP_CONNECTIONS.lock();

    // Get or create connection state
    let state = connections.entry(socket_id).or_insert_with(|| {
        let local = SocketAddr::v4(super::Ipv4Address::UNSPECIFIED, 0);
        TcpSocketState {
            connection: TcpConnection::new(local, remote),
            send_buffer: Vec::new(),
            recv_buffer: Vec::new(),
            send_seq: generate_initial_seq(),
            recv_seq: 0,
        }
    });

    // Update connection state
    state.connection.remote = remote;
    if state.connection.state == TcpState::Closed {
        state.connection.state = TcpState::Established;
    }

    // Buffer data for transmission
    state.send_buffer.extend_from_slice(data);

    // In a real implementation, this would:
    // 1. Segment data into MSS-sized chunks
    // 2. Create TCP headers with proper seq/ack numbers
    // 3. Pass to IP layer for transmission
    // 4. Start retransmission timer

    // For now, simulate immediate transmission
    let bytes_sent = data.len();
    state.send_seq = state.send_seq.wrapping_add(bytes_sent as u32);
    state.send_buffer.clear();

    #[cfg(feature = "net_debug")]
    println!(
        "[TCP] Transmitted {} bytes to {:?} (socket {})",
        bytes_sent, remote, socket_id
    );
}

/// Receive data from TCP connection
pub fn receive_data(socket_id: usize, buffer: &mut Vec<u8>) -> usize {
    let mut connections = TCP_CONNECTIONS.lock();

    if let Some(state) = connections.get_mut(&socket_id) {
        if state.connection.state != TcpState::Established {
            return 0;
        }

        // Copy data from receive buffer
        let bytes_available = state.recv_buffer.len();
        if bytes_available > 0 {
            buffer.extend_from_slice(&state.recv_buffer);
            state.recv_buffer.clear();
            state.recv_seq = state.recv_seq.wrapping_add(bytes_available as u32);

            #[cfg(feature = "net_debug")]
            println!(
                "[TCP] Received {} bytes from socket {}",
                bytes_available, socket_id
            );

            return bytes_available;
        }
    }

    0
}

/// Close a TCP connection
pub fn close_connection(socket_id: usize) {
    let mut connections = TCP_CONNECTIONS.lock();

    if let Some(state) = connections.get_mut(&socket_id) {
        // Initiate TCP close sequence
        match state.connection.state {
            TcpState::Established => {
                // Send FIN, transition to FIN_WAIT_1
                state.connection.state = TcpState::FinWait1;

                // In real implementation: send FIN packet and wait for ACK
                // For simulation, immediately transition through close sequence
                state.connection.state = TcpState::Closed;
            }
            TcpState::CloseWait => {
                // Send FIN, transition to LAST_ACK
                state.connection.state = TcpState::LastAck;
                state.connection.state = TcpState::Closed;
            }
            _ => {
                // Force close
                state.connection.state = TcpState::Closed;
            }
        }

        // Clear buffers
        state.send_buffer.clear();
        state.recv_buffer.clear();
    }

    // Remove from connection table
    connections.remove(&socket_id);

    #[cfg(feature = "net_debug")]
    println!("[TCP] Closed connection for socket {}", socket_id);
}

/// Process incoming TCP packet (called by IP layer)
pub fn process_packet(
    src_addr: super::IpAddress,
    dst_addr: super::IpAddress,
    data: &[u8],
) -> Result<(), KernelError> {
    if data.len() < 20 {
        return Err(KernelError::InvalidArgument {
            name: "tcp_packet",
            value: "too_short",
        });
    }

    // Parse TCP header
    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq_num = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let _ack_num = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let data_offset = ((data[12] >> 4) * 4) as usize;
    let flags = TcpFlags::new(data[13]);
    let _window = u16::from_be_bytes([data[14], data[15]]);

    // Find matching connection
    let mut connections = TCP_CONNECTIONS.lock();
    let remote = SocketAddr::new(src_addr, src_port);
    let _local = SocketAddr::new(dst_addr, dst_port);

    // Find socket by remote address match
    for (_socket_id, state) in connections.iter_mut() {
        if state.connection.remote == remote
            || (state.connection.state == TcpState::Listen
                && state.connection.local.port() == dst_port)
        {
            // Process based on flags and state
            if flags.has(TcpFlags::SYN) && !flags.has(TcpFlags::ACK) {
                // SYN received - new connection
                if state.connection.state == TcpState::Listen {
                    // Queue for accept
                    let local_addr = state.connection.local;
                    if let Err(_e) =
                        super::socket::queue_pending_connection(local_addr, remote, seq_num)
                    {
                        #[cfg(feature = "net_debug")]
                        println!("[TCP] Failed to queue connection: {:?}", _e);
                    }
                    return Ok(());
                }
            } else if flags.has(TcpFlags::ACK) {
                // ACK received
                state.recv_seq = seq_num;
            }

            // Extract payload
            if data.len() > data_offset {
                let payload = &data[data_offset..];
                if !payload.is_empty() {
                    state.recv_buffer.extend_from_slice(payload);
                    state.recv_seq = seq_num.wrapping_add(payload.len() as u32);
                }
            }

            if flags.has(TcpFlags::FIN) {
                // FIN received
                match state.connection.state {
                    TcpState::Established => {
                        state.connection.state = TcpState::CloseWait;
                    }
                    TcpState::FinWait1 | TcpState::FinWait2 => {
                        state.connection.state = TcpState::TimeWait;
                    }
                    _ => {}
                }
            }

            #[cfg(feature = "net_debug")]
            println!(
                "[TCP] Processed packet for socket {}: seq={} ack={} flags={:02x}",
                _socket_id, seq_num, _ack_num, flags.0
            );

            return Ok(());
        }
    }

    // No matching connection - send RST if not RST
    if !flags.has(TcpFlags::RST) {
        #[cfg(feature = "net_debug")]
        println!("[TCP] No matching connection, would send RST");
    }

    Ok(())
}

/// Generate initial sequence number
fn generate_initial_seq() -> u32 {
    // In real implementation, use secure random + timestamp
    // For now, use a simple counter
    static COUNTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1000000);
    COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

/// Get connection statistics
pub fn get_stats() -> TcpStats {
    let connections = TCP_CONNECTIONS.lock();
    TcpStats {
        active_connections: connections.len(),
        total_bytes_sent: 0, // Would track in real implementation
        total_bytes_recv: 0, // Would track in real implementation
        retransmissions: 0,  // Would track in real implementation
    }
}

/// TCP statistics
#[derive(Debug, Clone, Copy, Default)]
pub struct TcpStats {
    pub active_connections: usize,
    pub total_bytes_sent: u64,
    pub total_bytes_recv: u64,
    pub retransmissions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Ipv4Address;

    #[test_case]
    fn test_tcp_flags() {
        let mut flags = TcpFlags::new(0);
        flags.set(TcpFlags::SYN);
        assert!(flags.has(TcpFlags::SYN));
        assert!(!flags.has(TcpFlags::ACK));
    }

    #[test_case]
    fn test_tcp_connection() {
        let local = SocketAddr::v4(Ipv4Address::LOCALHOST, 8080);
        let remote = SocketAddr::v4(Ipv4Address::new(192, 168, 1, 1), 80);
        let conn = TcpConnection::new(local, remote);

        assert_eq!(conn.state, TcpState::Closed);
    }
}
