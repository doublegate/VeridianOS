//! OpenVPN Protocol Implementation
//!
//! Implements the OpenVPN control and data channel protocols, including:
//! - Packet opcodes and header parsing (P_CONTROL, P_DATA, P_ACK)
//! - TLS-Auth pre-shared HMAC authentication
//! - Client state machine (Initial -> TLS -> Auth -> Connected)
//! - Packet ID anti-replay protection (sliding window)
//! - Configuration file parsing (key=value format)

#![allow(dead_code)]

use alloc::vec::Vec;

use super::tunnel::TunnelType;
use crate::net::Ipv4Address;

// ── Constants ────────────────────────────────────────────────────────────────

/// Default OpenVPN UDP port
pub const DEFAULT_PORT: u16 = 1194;

/// HMAC-SHA1 key size (bytes)
const HMAC_KEY_SIZE: usize = 20;

/// TLS-Auth pre-shared key size (bytes)
const TLS_AUTH_KEY_SIZE: usize = 64;

/// Packet ID anti-replay sliding window size (entries)
const REPLAY_WINDOW_SIZE: usize = 64;

/// Session ID size in bytes
const SESSION_ID_SIZE: usize = 8;

/// Maximum control channel message size
const MAX_CONTROL_SIZE: usize = 1500;

/// Maximum data channel payload
const MAX_DATA_SIZE: usize = 65536;

/// Maximum number of pending ACKs
const MAX_PENDING_ACKS: usize = 64;

/// Renegotiation interval in seconds
const RENEG_SECONDS: u64 = 3600;

/// Maximum config line length
const MAX_CONFIG_LINE: usize = 256;

// ── OpenVPN Opcodes ──────────────────────────────────────────────────────────

/// OpenVPN packet opcodes (5 bits, high nibble of first byte)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpenvpnOpcode {
    /// Control channel: hard reset from client (v2)
    ControlHardResetClientV2 = 7,
    /// Control channel: hard reset from server (v2)
    ControlHardResetServerV2 = 8,
    /// Control channel: reliable data
    ControlV1 = 4,
    /// Control channel: acknowledgement
    AckV1 = 5,
    /// Data channel: v1 (no peer-id)
    DataV1 = 6,
    /// Data channel: v2 (with peer-id)
    DataV2 = 9,
}

impl OpenvpnOpcode {
    /// Parse an opcode from the high 5 bits of a byte
    pub fn from_byte(b: u8) -> Option<Self> {
        match b >> 3 {
            7 => Some(Self::ControlHardResetClientV2),
            8 => Some(Self::ControlHardResetServerV2),
            4 => Some(Self::ControlV1),
            5 => Some(Self::AckV1),
            6 => Some(Self::DataV1),
            9 => Some(Self::DataV2),
            _ => None,
        }
    }

    /// Encode opcode into the high 5 bits with key_id in low 3 bits
    pub fn encode(&self, key_id: u8) -> u8 {
        ((*self as u8) << 3) | (key_id & 0x07)
    }

    /// Whether this opcode belongs to the control channel
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::ControlHardResetClientV2
                | Self::ControlHardResetServerV2
                | Self::ControlV1
                | Self::AckV1
        )
    }
}

// ── OpenVPN Header ───────────────────────────────────────────────────────────

/// Parsed OpenVPN packet header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenvpnHeader {
    /// Opcode (5 bits)
    pub opcode: OpenvpnOpcode,
    /// Key ID (3 bits) -- identifies the TLS session / key
    pub key_id: u8,
    /// Session ID (8 bytes)
    pub session_id: u64,
    /// HMAC hash for tls-auth (20 bytes, SHA-1)
    pub hmac_hash: [u8; HMAC_KEY_SIZE],
    /// Packet ID for replay protection
    pub packet_id: u32,
    /// Timestamp (seconds since epoch)
    pub timestamp: u32,
}

impl OpenvpnHeader {
    /// Header size in bytes: 1 (opcode+key_id) + 8 (session) + 20 (hmac) + 4
    /// (pid) + 4 (ts)
    pub const SIZE: usize = 1 + SESSION_ID_SIZE + HMAC_KEY_SIZE + 4 + 4;

    /// Parse a header from raw bytes
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < Self::SIZE {
            return None;
        }

        let opcode = OpenvpnOpcode::from_byte(data[0])?;
        let key_id = data[0] & 0x07;
        let session_id = u64::from_be_bytes([
            data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
        ]);

        let mut hmac_hash = [0u8; HMAC_KEY_SIZE];
        hmac_hash.copy_from_slice(&data[9..29]);

        let packet_id = u32::from_be_bytes([data[29], data[30], data[31], data[32]]);
        let timestamp = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);

        Some(Self {
            opcode,
            key_id,
            session_id,
            hmac_hash,
            packet_id,
            timestamp,
        })
    }

    /// Serialise the header to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::SIZE);
        buf.push(self.opcode.encode(self.key_id));
        buf.extend_from_slice(&self.session_id.to_be_bytes());
        buf.extend_from_slice(&self.hmac_hash);
        buf.extend_from_slice(&self.packet_id.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());
        buf
    }
}

// ── OpenVPN State Machine ────────────────────────────────────────────────────

/// Client connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenvpnState {
    /// Not connected, no session
    #[default]
    Initial,
    /// TLS handshake in progress
    TlsHandshake,
    /// Authenticating (username/password or certificate)
    Authentication,
    /// Fully connected and passing data
    Connected,
    /// Reconnecting after a disconnect
    Reconnecting,
    /// Cleanly disconnected
    Disconnected,
}

// ── TLS-Auth ─────────────────────────────────────────────────────────────────

/// TLS-Auth pre-shared HMAC key for control channel authentication
#[derive(Clone)]
pub struct TlsAuth {
    /// Pre-shared key (64 bytes: 32 encrypt + 32 HMAC, or split by direction)
    key: [u8; TLS_AUTH_KEY_SIZE],
    /// Direction: 0 = client->server keys first, 1 = server->client keys first
    direction: u8,
}

impl TlsAuth {
    /// Create a new TLS-Auth context from key material and direction
    pub fn new(key: [u8; TLS_AUTH_KEY_SIZE], direction: u8) -> Self {
        Self {
            key,
            direction: direction & 1,
        }
    }

    /// Get the HMAC key portion based on direction
    fn hmac_key(&self) -> &[u8] {
        if self.direction == 0 {
            &self.key[0..32]
        } else {
            &self.key[32..64]
        }
    }

    /// Compute HMAC-SHA1 over data using the directional key
    ///
    /// Uses a simplified HMAC construction: H(key XOR opad || H(key XOR ipad ||
    /// message)) where H is a simple hash (not cryptographically strong --
    /// placeholder for real SHA-1).
    pub fn compute_hmac(&self, data: &[u8]) -> [u8; HMAC_KEY_SIZE] {
        let key = self.hmac_key();
        let mut result = [0u8; HMAC_KEY_SIZE];

        // Inner hash: H(key XOR ipad || message)
        let mut inner = [0u8; HMAC_KEY_SIZE];
        for (i, byte) in inner.iter_mut().enumerate() {
            let k = if i < key.len() { key[i] } else { 0 };
            *byte = k ^ 0x36;
        }
        // Mix in data bytes
        for (i, &b) in data.iter().enumerate() {
            inner[i % HMAC_KEY_SIZE] ^= b;
        }

        // Outer hash: H(key XOR opad || inner)
        for (i, byte) in result.iter_mut().enumerate() {
            let k = if i < key.len() { key[i] } else { 0 };
            *byte = (k ^ 0x5C) ^ inner[i];
        }

        result
    }

    /// Verify an HMAC tag against data
    pub fn verify_hmac(&self, data: &[u8], expected: &[u8; HMAC_KEY_SIZE]) -> bool {
        let computed = self.compute_hmac(data);
        // Constant-time comparison
        let mut diff = 0u8;
        for (a, b) in computed.iter().zip(expected.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

// ── Cipher / Auth Enums ──────────────────────────────────────────────────────

/// Data channel cipher algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CipherAlgorithm {
    /// AES-256-GCM (AEAD, recommended)
    #[default]
    Aes256Gcm,
    /// AES-128-CBC (legacy)
    Aes128Cbc,
    /// ChaCha20-Poly1305 (AEAD, modern alternative)
    ChaCha20Poly1305,
}

/// HMAC authentication algorithm (for non-AEAD ciphers)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthAlgorithm {
    /// SHA-256
    #[default]
    Sha256,
    /// SHA-512
    Sha512,
}

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// No compression
    #[default]
    None,
    /// LZO compression (legacy)
    Lzo,
    /// LZ4 compression
    Lz4,
}

/// Transport protocol for the tunnel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportProto {
    /// UDP (default, preferred)
    #[default]
    Udp,
    /// TCP (fallback for restrictive networks)
    Tcp,
}

// ── OpenVPN Configuration ────────────────────────────────────────────────────

/// OpenVPN client/server configuration
#[derive(Debug, Clone)]
pub struct OpenvpnConfig {
    /// Remote server address
    pub remote_host: Ipv4Address,
    /// Remote server port
    pub remote_port: u16,
    /// Transport protocol (UDP or TCP)
    pub protocol: TransportProto,
    /// Data channel cipher
    pub cipher: CipherAlgorithm,
    /// Authentication algorithm (for non-AEAD ciphers)
    pub auth: AuthAlgorithm,
    /// Compression method
    pub compress: Compression,
    /// TLS-Auth pre-shared key (optional)
    pub tls_auth_key: Option<[u8; TLS_AUTH_KEY_SIZE]>,
    /// Device type (TUN or TAP)
    pub dev_type: TunnelType,
}

impl Default for OpenvpnConfig {
    fn default() -> Self {
        Self {
            remote_host: Ipv4Address::new(0, 0, 0, 0),
            remote_port: DEFAULT_PORT,
            protocol: TransportProto::Udp,
            cipher: CipherAlgorithm::Aes256Gcm,
            auth: AuthAlgorithm::Sha256,
            compress: Compression::None,
            tls_auth_key: None,
            dev_type: TunnelType::Tun,
        }
    }
}

// ── Anti-Replay ──────────────────────────────────────────────────────────────

/// Sliding-window anti-replay protection for packet IDs
pub struct PacketIdWindow {
    /// Bitmap of seen packet IDs relative to `base`
    window: [u8; REPLAY_WINDOW_SIZE / 8],
    /// Base packet ID (lowest ID in the window)
    base: u32,
    /// Highest packet ID seen so far
    highest: u32,
}

impl Default for PacketIdWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketIdWindow {
    /// Create a new anti-replay window
    pub fn new() -> Self {
        Self {
            window: [0u8; REPLAY_WINDOW_SIZE / 8],
            base: 0,
            highest: 0,
        }
    }

    /// Check whether a packet ID is acceptable (not replayed)
    pub fn check_replay(&self, packet_id: u32) -> bool {
        if packet_id == 0 {
            return false; // Packet ID 0 is never valid
        }

        // If packet_id is ahead of our window, it's always acceptable
        if packet_id > self.highest {
            return true;
        }

        // If packet_id is too old (below the window base), reject
        if packet_id < self.base {
            return false;
        }

        // Check if already seen within the window
        let offset = (packet_id - self.base) as usize;
        if offset >= REPLAY_WINDOW_SIZE {
            return false;
        }

        let byte_idx = offset / 8;
        let bit_idx = offset % 8;
        (self.window[byte_idx] & (1 << bit_idx)) == 0
    }

    /// Record a packet ID as seen and advance the window if needed
    pub fn update(&mut self, packet_id: u32) {
        if packet_id == 0 {
            return;
        }

        if packet_id > self.highest {
            // Advance the window
            let shift = (packet_id - self.highest) as usize;
            if shift >= REPLAY_WINDOW_SIZE {
                // Complete window reset
                self.window = [0u8; REPLAY_WINDOW_SIZE / 8];
            } else {
                // Shift window bits forward
                self.shift_window(shift);
            }
            self.highest = packet_id;
            self.base = packet_id.saturating_sub(REPLAY_WINDOW_SIZE as u32 - 1);
        }

        // Mark as seen
        if packet_id >= self.base {
            let offset = (packet_id - self.base) as usize;
            if offset < REPLAY_WINDOW_SIZE {
                let byte_idx = offset / 8;
                let bit_idx = offset % 8;
                self.window[byte_idx] |= 1 << bit_idx;
            }
        }
    }

    /// Shift the window bitmap forward by `count` bits
    fn shift_window(&mut self, count: usize) {
        if count >= REPLAY_WINDOW_SIZE {
            self.window = [0u8; REPLAY_WINDOW_SIZE / 8];
            return;
        }

        let byte_shift = count / 8;
        let bit_shift = count % 8;

        if byte_shift > 0 {
            for i in (byte_shift..self.window.len()).rev() {
                self.window[i] = self.window[i - byte_shift];
            }
            for byte in self.window.iter_mut().take(byte_shift) {
                *byte = 0;
            }
        }

        if bit_shift > 0 {
            for i in (1..self.window.len()).rev() {
                self.window[i] =
                    (self.window[i] << bit_shift) | (self.window[i - 1] >> (8 - bit_shift));
            }
            self.window[0] <<= bit_shift;
        }
    }
}

// ── OpenVPN Client ───────────────────────────────────────────────────────────

/// OpenVPN protocol error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenvpnError {
    /// Not connected
    NotConnected,
    /// Already connected
    AlreadyConnected,
    /// Invalid packet format
    InvalidPacket,
    /// HMAC verification failed
    HmacFailed,
    /// Replay attack detected
    ReplayDetected,
    /// Packet too large
    PacketTooLarge,
    /// Session not established
    NoSession,
    /// Invalid opcode
    InvalidOpcode,
    /// Invalid state for the requested operation
    InvalidState,
    /// Renegotiation required
    RenegotiationRequired,
}

/// OpenVPN client connection
pub struct OpenvpnClient {
    /// Configuration
    config: OpenvpnConfig,
    /// Current state
    state: OpenvpnState,
    /// Local session ID
    session_id: u64,
    /// Remote session ID (received from server)
    remote_session_id: u64,
    /// Monotonic packet ID counter for outgoing packets
    packet_id_counter: u32,
    /// Anti-replay window for incoming packets
    replay_window: PacketIdWindow,
    /// Current key_id (rotates on renegotiation)
    key_id: u8,
    /// TLS-Auth context (if configured)
    tls_auth: Option<TlsAuth>,
    /// Pending ACK packet IDs to send
    pending_acks: Vec<u32>,
    /// Seconds since session start (for renegotiation timer)
    session_start: u64,
    /// Total bytes sent on this session
    bytes_sent: u64,
    /// Total bytes received on this session
    bytes_received: u64,
}

impl OpenvpnClient {
    /// Create a new OpenVPN client with the given configuration
    pub fn new(config: OpenvpnConfig, session_id: u64) -> Self {
        let tls_auth = config.tls_auth_key.map(|key| TlsAuth::new(key, 0));

        Self {
            config,
            state: OpenvpnState::Initial,
            session_id,
            remote_session_id: 0,
            packet_id_counter: 1, // Packet ID 0 is invalid
            replay_window: PacketIdWindow::new(),
            key_id: 0,
            tls_auth,
            pending_acks: Vec::new(),
            session_start: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    /// Get current connection state
    pub fn state(&self) -> OpenvpnState {
        self.state
    }

    /// Get the session ID
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Initiate a connection (send hard reset)
    pub fn connect(&mut self, now: u64) -> Result<Vec<u8>, OpenvpnError> {
        if self.state != OpenvpnState::Initial && self.state != OpenvpnState::Disconnected {
            return Err(OpenvpnError::InvalidState);
        }

        self.state = OpenvpnState::TlsHandshake;
        self.session_start = now;

        // Build P_CONTROL_HARD_RESET_CLIENT_V2 packet
        let packet =
            self.build_control_packet(OpenvpnOpcode::ControlHardResetClientV2, &[], now as u32);
        Ok(packet)
    }

    /// Clean disconnect
    pub fn disconnect(&mut self) {
        self.state = OpenvpnState::Disconnected;
        self.pending_acks.clear();
    }

    /// Send a control channel message (reliable, with pending ACK)
    pub fn send_control(&mut self, payload: &[u8], now: u64) -> Result<Vec<u8>, OpenvpnError> {
        if self.state == OpenvpnState::Initial || self.state == OpenvpnState::Disconnected {
            return Err(OpenvpnError::NotConnected);
        }
        if payload.len() > MAX_CONTROL_SIZE {
            return Err(OpenvpnError::PacketTooLarge);
        }

        let packet = self.build_control_packet(OpenvpnOpcode::ControlV1, payload, now as u32);
        Ok(packet)
    }

    /// Encrypt and send a data channel packet
    pub fn send_data(&mut self, payload: &[u8]) -> Result<Vec<u8>, OpenvpnError> {
        if self.state != OpenvpnState::Connected {
            return Err(OpenvpnError::NotConnected);
        }
        if payload.len() > MAX_DATA_SIZE {
            return Err(OpenvpnError::PacketTooLarge);
        }

        let pid = self.next_packet_id();
        let mut packet = Vec::with_capacity(1 + 4 + payload.len());

        // Opcode + key_id byte
        packet.push(OpenvpnOpcode::DataV2.encode(self.key_id));
        // Packet ID (for replay protection on data channel too)
        packet.extend_from_slice(&pid.to_be_bytes());
        // Payload (in a real implementation, this would be encrypted)
        packet.extend_from_slice(payload);

        self.bytes_sent += payload.len() as u64;
        Ok(packet)
    }

    /// Process an incoming packet
    pub fn receive_packet(&mut self, data: &[u8]) -> Result<Vec<u8>, OpenvpnError> {
        if data.is_empty() {
            return Err(OpenvpnError::InvalidPacket);
        }

        let opcode = OpenvpnOpcode::from_byte(data[0]).ok_or(OpenvpnError::InvalidOpcode)?;

        if opcode.is_control() {
            self.process_control(data)
        } else {
            self.process_data(data)
        }
    }

    /// Handle control channel messages
    pub fn process_control(&mut self, data: &[u8]) -> Result<Vec<u8>, OpenvpnError> {
        let header = OpenvpnHeader::parse(data).ok_or(OpenvpnError::InvalidPacket)?;

        // Verify HMAC if tls-auth is configured
        if let Some(ref tls_auth) = self.tls_auth {
            // HMAC covers everything after the HMAC field
            let hmac_data = if data.len() > 29 { &data[29..] } else { &[] };
            if !tls_auth.verify_hmac(hmac_data, &header.hmac_hash) {
                return Err(OpenvpnError::HmacFailed);
            }
        }

        // Anti-replay check on control channel
        if !self.replay_window.check_replay(header.packet_id) {
            return Err(OpenvpnError::ReplayDetected);
        }
        self.replay_window.update(header.packet_id);

        // State machine transitions
        match header.opcode {
            OpenvpnOpcode::ControlHardResetServerV2 => {
                if self.state == OpenvpnState::TlsHandshake {
                    self.remote_session_id = header.session_id;
                    // Queue ACK
                    self.queue_ack(header.packet_id);
                    self.state = OpenvpnState::Authentication;
                }
            }
            OpenvpnOpcode::ControlV1 => {
                self.queue_ack(header.packet_id);
                // In Authentication state, a ControlV1 from the server signals
                // successful auth and transition to Connected
                if self.state == OpenvpnState::Authentication {
                    self.state = OpenvpnState::Connected;
                }
            }
            OpenvpnOpcode::AckV1 => {
                // Remove acknowledged packet IDs from pending list
                self.pending_acks.retain(|&pid| pid != header.packet_id);
            }
            _ => {}
        }

        // Return any payload beyond the header
        if data.len() > OpenvpnHeader::SIZE {
            self.bytes_received += (data.len() - OpenvpnHeader::SIZE) as u64;
            Ok(data[OpenvpnHeader::SIZE..].to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    /// Handle data channel packets
    fn process_data(&mut self, data: &[u8]) -> Result<Vec<u8>, OpenvpnError> {
        if self.state != OpenvpnState::Connected {
            return Err(OpenvpnError::NotConnected);
        }

        // Data packet: opcode(1) + packet_id(4) + payload
        if data.len() < 5 {
            return Err(OpenvpnError::InvalidPacket);
        }

        let packet_id = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);

        // Anti-replay
        if !self.replay_window.check_replay(packet_id) {
            return Err(OpenvpnError::ReplayDetected);
        }
        self.replay_window.update(packet_id);

        let payload = data[5..].to_vec();
        self.bytes_received += payload.len() as u64;
        Ok(payload)
    }

    /// Initiate TLS key renegotiation
    pub fn renegotiate(&mut self, now: u64) -> Result<Vec<u8>, OpenvpnError> {
        if self.state != OpenvpnState::Connected {
            return Err(OpenvpnError::InvalidState);
        }

        self.key_id = (self.key_id + 1) & 0x07;
        self.state = OpenvpnState::TlsHandshake;
        self.replay_window = PacketIdWindow::new();

        let packet =
            self.build_control_packet(OpenvpnOpcode::ControlHardResetClientV2, &[], now as u32);
        Ok(packet)
    }

    /// Check if renegotiation is needed based on elapsed time
    pub fn needs_renegotiation(&self, now: u64) -> bool {
        if self.state != OpenvpnState::Connected {
            return false;
        }
        now.saturating_sub(self.session_start) >= RENEG_SECONDS
    }

    /// Get the next packet ID and increment the counter
    fn next_packet_id(&mut self) -> u32 {
        let id = self.packet_id_counter;
        self.packet_id_counter = self.packet_id_counter.wrapping_add(1);
        if self.packet_id_counter == 0 {
            self.packet_id_counter = 1; // Skip 0
        }
        id
    }

    /// Queue an ACK for a received packet ID
    fn queue_ack(&mut self, packet_id: u32) {
        if self.pending_acks.len() < MAX_PENDING_ACKS {
            self.pending_acks.push(packet_id);
        }
    }

    /// Build a control channel packet with header
    fn build_control_packet(
        &mut self,
        opcode: OpenvpnOpcode,
        payload: &[u8],
        timestamp: u32,
    ) -> Vec<u8> {
        let pid = self.next_packet_id();

        let hmac_hash = if let Some(ref tls_auth) = self.tls_auth {
            // Compute HMAC over packet_id + timestamp + payload
            let mut hmac_data = Vec::new();
            hmac_data.extend_from_slice(&pid.to_be_bytes());
            hmac_data.extend_from_slice(&timestamp.to_be_bytes());
            hmac_data.extend_from_slice(payload);
            tls_auth.compute_hmac(&hmac_data)
        } else {
            [0u8; HMAC_KEY_SIZE]
        };

        let header = OpenvpnHeader {
            opcode,
            key_id: self.key_id,
            session_id: self.session_id,
            hmac_hash,
            packet_id: pid,
            timestamp,
        };

        let mut packet = header.to_bytes();
        packet.extend_from_slice(payload);
        packet
    }

    /// Get bytes sent/received counters
    pub fn traffic_stats(&self) -> (u64, u64) {
        (self.bytes_sent, self.bytes_received)
    }
}

// ── Config File Parser ───────────────────────────────────────────────────────

/// Parse an OpenVPN-style configuration from key=value lines.
///
/// Supported directives: proto, remote, port, dev, cipher, auth, compress,
/// tls-auth (key bytes are not parsed here -- set separately).
pub fn parse_config(input: &str) -> OpenvpnConfig {
    let mut config = OpenvpnConfig::default();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Split at first whitespace
        let (key, value) = match line.find(|c: char| c.is_ascii_whitespace()) {
            Some(pos) => (line[..pos].trim(), line[pos + 1..].trim()),
            None => (line, ""),
        };

        match key {
            "proto" => {
                config.protocol = match value {
                    "tcp" | "tcp-client" => TransportProto::Tcp,
                    _ => TransportProto::Udp,
                };
            }
            "remote" => {
                // "remote <host> [port]"
                let parts: Vec<&str> = value.split_whitespace().collect();
                if let Some(host) = parts.first() {
                    config.remote_host = parse_ipv4(host);
                }
                if let Some(port_str) = parts.get(1) {
                    if let Some(port) = parse_u16(port_str) {
                        config.remote_port = port;
                    }
                }
            }
            "port" => {
                if let Some(port) = parse_u16(value) {
                    config.remote_port = port;
                }
            }
            "dev" => {
                config.dev_type = if value.starts_with("tap") {
                    TunnelType::Tap
                } else {
                    TunnelType::Tun
                };
            }
            "cipher" => {
                config.cipher = match value {
                    "AES-128-CBC" => CipherAlgorithm::Aes128Cbc,
                    "CHACHA20-POLY1305" => CipherAlgorithm::ChaCha20Poly1305,
                    _ => CipherAlgorithm::Aes256Gcm,
                };
            }
            "auth" => {
                config.auth = match value {
                    "SHA512" => AuthAlgorithm::Sha512,
                    _ => AuthAlgorithm::Sha256,
                };
            }
            "compress" => {
                config.compress = match value {
                    "lzo" => Compression::Lzo,
                    "lz4" | "lz4-v2" => Compression::Lz4,
                    _ => Compression::None,
                };
            }
            _ => {} // Ignore unknown directives
        }
    }

    config
}

/// Parse a dotted-quad IPv4 address string (simple, no error handling)
fn parse_ipv4(s: &str) -> Ipv4Address {
    let mut octets = [0u8; 4];
    for (i, part) in s.split('.').take(4).enumerate() {
        if let Some(val) = parse_u8(part) {
            octets[i] = val;
        }
    }
    Ipv4Address(octets)
}

/// Parse a u16 from a decimal string
fn parse_u16(s: &str) -> Option<u16> {
    let mut result: u16 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u16)?;
    }
    Some(result)
}

/// Parse a u8 from a decimal string
fn parse_u8(s: &str) -> Option<u8> {
    let mut result: u16 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u16)?;
    }
    if result > 255 {
        None
    } else {
        Some(result as u8)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_encode_decode() {
        for opcode in [
            OpenvpnOpcode::ControlHardResetClientV2,
            OpenvpnOpcode::ControlHardResetServerV2,
            OpenvpnOpcode::ControlV1,
            OpenvpnOpcode::AckV1,
            OpenvpnOpcode::DataV1,
            OpenvpnOpcode::DataV2,
        ] {
            let key_id = 3u8;
            let byte = opcode.encode(key_id);
            let decoded = OpenvpnOpcode::from_byte(byte).unwrap();
            assert_eq!(decoded, opcode);
            assert_eq!(byte & 0x07, key_id);
        }
    }

    #[test]
    fn test_opcode_is_control() {
        assert!(OpenvpnOpcode::ControlV1.is_control());
        assert!(OpenvpnOpcode::AckV1.is_control());
        assert!(OpenvpnOpcode::ControlHardResetClientV2.is_control());
        assert!(!OpenvpnOpcode::DataV1.is_control());
        assert!(!OpenvpnOpcode::DataV2.is_control());
    }

    #[test]
    fn test_header_serialize_parse() {
        let header = OpenvpnHeader {
            opcode: OpenvpnOpcode::ControlV1,
            key_id: 2,
            session_id: 0xDEADBEEF_CAFEBABE,
            hmac_hash: [0xAA; HMAC_KEY_SIZE],
            packet_id: 42,
            timestamp: 1000,
        };

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), OpenvpnHeader::SIZE);

        let parsed = OpenvpnHeader::parse(&bytes).unwrap();
        assert_eq!(parsed.opcode, OpenvpnOpcode::ControlV1);
        assert_eq!(parsed.key_id, 2);
        assert_eq!(parsed.session_id, 0xDEADBEEF_CAFEBABE);
        assert_eq!(parsed.hmac_hash, [0xAA; HMAC_KEY_SIZE]);
        assert_eq!(parsed.packet_id, 42);
        assert_eq!(parsed.timestamp, 1000);
    }

    #[test]
    fn test_header_parse_too_short() {
        assert!(OpenvpnHeader::parse(&[0u8; 10]).is_none());
    }

    #[test]
    fn test_tls_auth_hmac() {
        let key = [0x42u8; TLS_AUTH_KEY_SIZE];
        let tls_auth = TlsAuth::new(key, 0);

        let mac1 = tls_auth.compute_hmac(b"hello");
        let mac2 = tls_auth.compute_hmac(b"hello");
        assert_eq!(mac1, mac2);

        let mac3 = tls_auth.compute_hmac(b"world");
        assert_ne!(mac1, mac3);
    }

    #[test]
    fn test_tls_auth_verify() {
        let key = [0x42u8; TLS_AUTH_KEY_SIZE];
        let tls_auth = TlsAuth::new(key, 0);

        let mac = tls_auth.compute_hmac(b"test data");
        assert!(tls_auth.verify_hmac(b"test data", &mac));
        assert!(!tls_auth.verify_hmac(b"wrong data", &mac));
    }

    #[test]
    fn test_tls_auth_direction() {
        // Verify that different directions select different key halves
        let mut key = [0u8; TLS_AUTH_KEY_SIZE];
        for (i, b) in key.iter_mut().enumerate() {
            *b = if i < 32 { 0x42 } else { 0xAB };
        }
        let auth0 = TlsAuth::new(key, 0);
        let auth1 = TlsAuth::new(key, 1);

        // Direction 0 uses first half, direction 1 uses second half
        assert_eq!(auth0.hmac_key()[0], 0x42);
        assert_eq!(auth1.hmac_key()[0], 0xAB);
    }

    #[test]
    fn test_replay_window_basic() {
        let mut window = PacketIdWindow::new();

        // Packet ID 0 is always invalid
        assert!(!window.check_replay(0));

        // New IDs are accepted
        assert!(window.check_replay(1));
        window.update(1);

        // Duplicate is rejected
        assert!(!window.check_replay(1));

        // Forward ID is accepted
        assert!(window.check_replay(2));
        window.update(2);
        assert!(window.check_replay(100));
    }

    #[test]
    fn test_replay_window_large_jump() {
        let mut window = PacketIdWindow::new();

        window.update(1);
        window.update(1000);

        // Old IDs are outside the window
        assert!(!window.check_replay(1));

        // IDs within the window are still available
        assert!(window.check_replay(999));
    }

    #[test]
    fn test_client_connect_disconnect() {
        let config = OpenvpnConfig::default();
        let mut client = OpenvpnClient::new(config, 0x1234);

        assert_eq!(client.state(), OpenvpnState::Initial);

        let pkt = client.connect(100).unwrap();
        assert!(!pkt.is_empty());
        assert_eq!(client.state(), OpenvpnState::TlsHandshake);

        // Cannot connect twice
        assert_eq!(client.connect(101), Err(OpenvpnError::InvalidState));

        client.disconnect();
        assert_eq!(client.state(), OpenvpnState::Disconnected);
    }

    #[test]
    fn test_client_send_data_requires_connected() {
        let config = OpenvpnConfig::default();
        let mut client = OpenvpnClient::new(config, 0x1234);

        assert_eq!(client.send_data(b"hello"), Err(OpenvpnError::NotConnected));
    }

    #[test]
    fn test_config_parse_basic() {
        let input = "\
proto udp
remote 10.0.0.1 1194
dev tun
cipher AES-256-GCM
auth SHA256
compress lz4
";
        let config = parse_config(input);
        assert_eq!(config.protocol, TransportProto::Udp);
        assert_eq!(config.remote_host, Ipv4Address::new(10, 0, 0, 1));
        assert_eq!(config.remote_port, 1194);
        assert_eq!(config.dev_type, TunnelType::Tun);
        assert_eq!(config.cipher, CipherAlgorithm::Aes256Gcm);
        assert_eq!(config.auth, AuthAlgorithm::Sha256);
        assert_eq!(config.compress, Compression::Lz4);
    }

    #[test]
    fn test_config_parse_comments_and_blanks() {
        let input = "\
# This is a comment
; Another comment

proto tcp
port 443
dev tap0
cipher AES-128-CBC
auth SHA512
";
        let config = parse_config(input);
        assert_eq!(config.protocol, TransportProto::Tcp);
        assert_eq!(config.remote_port, 443);
        assert_eq!(config.dev_type, TunnelType::Tap);
        assert_eq!(config.cipher, CipherAlgorithm::Aes128Cbc);
        assert_eq!(config.auth, AuthAlgorithm::Sha512);
    }

    #[test]
    fn test_parse_ipv4() {
        assert_eq!(parse_ipv4("192.168.1.1"), Ipv4Address::new(192, 168, 1, 1));
        assert_eq!(parse_ipv4("10.0.0.1"), Ipv4Address::new(10, 0, 0, 1));
        assert_eq!(parse_ipv4("0.0.0.0"), Ipv4Address::new(0, 0, 0, 0));
    }
}
