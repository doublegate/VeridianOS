//! TCP protocol implementation

use super::{IpAddress, Port, SocketAddr};
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
        // TODO: Actually send SYN packet

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

        // TODO: Actually send data

        Ok(data.len())
    }

    /// Receive data
    pub fn recv(&mut self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if self.state != TcpState::Established {
            return Err(KernelError::InvalidState {
                expected: "Established",
                actual: "Other",
            });
        }

        // TODO: Actually receive data from buffer

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
