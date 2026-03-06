//! WireGuard VPN tunnel implementation
//!
//! Implements the WireGuard protocol (Noise_IKpsk2 handshake pattern) for
//! secure VPN tunneling. Provides:
//! - BLAKE2s hash function (RFC 7693)
//! - Noise IK handshake with pre-shared key
//! - ChaCha20-Poly1305 AEAD transport encryption
//! - Anti-replay sliding window
//! - Peer management with key rotation
//! - Virtual network interface (wg0)
//! - Timer-based session management

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use super::{IpAddress, Ipv4Address};
use crate::crypto::{
    cipher_suite::{CipherSuite, HmacAlgorithm, KdfAlgorithm},
    hash::{blake2s_hash, blake2s_keyed_hash},
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Default WireGuard UDP port
pub const DEFAULT_PORT: u16 = 51820;

/// WireGuard message types
const MSG_HANDSHAKE_INIT: u8 = 1;
const MSG_HANDSHAKE_RESP: u8 = 2;
const MSG_COOKIE_REPLY: u8 = 3;
const MSG_TRANSPORT_DATA: u8 = 4;

/// Handshake initiation message size (bytes)
const HANDSHAKE_INIT_SIZE: usize = 148;

/// Handshake response message size (bytes)
const HANDSHAKE_RESP_SIZE: usize = 92;

/// Key size in bytes (256-bit)
const KEY_SIZE: usize = 32;

/// Nonce size for ChaCha20-Poly1305 (96-bit)
const CHACHA_NONCE_SIZE: usize = 12;

/// Poly1305 authentication tag size (128-bit)
const TAG_SIZE: usize = 16;

/// Anti-replay window size in bits
const REPLAY_WINDOW_BITS: usize = 2048;

/// Anti-replay window size in u64 words
const REPLAY_WINDOW_WORDS: usize = REPLAY_WINDOW_BITS / 64;

/// Rekey after this many messages (2^60)
const REKEY_AFTER_MESSAGES: u64 = 1u64 << 60;

/// Rekey after this many seconds
const REKEY_AFTER_SECONDS: u64 = 120;

/// Session expires after this many seconds without data
const SESSION_EXPIRY_SECONDS: u64 = 180;

/// Default persistent keepalive interval (seconds)
const DEFAULT_KEEPALIVE_INTERVAL: u64 = 25;

/// Maximum handshake retry attempts
const MAX_HANDSHAKE_RETRIES: u32 = 5;

/// Initial handshake retry delay (milliseconds)
const INITIAL_RETRY_DELAY_MS: u64 = 1000;

/// WireGuard construction string (used in protocol derivation)
const CONSTRUCTION: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";

/// WireGuard identifier string
const IDENTIFIER: &[u8] = b"WireGuard v1 zx2c4 Jason@zx2c4.com";

/// Transport message header: type(4) + receiver(4) + counter(8)
const TRANSPORT_HEADER_SIZE: usize = 16;

// ── BLAKE2s (delegates to crate::crypto::hash) ─────────────────────────────

/// Compute BLAKE2s hash of data with given output length
///
/// Delegates to `crate::crypto::hash::blake2s_hash`.
pub fn blake2s(data: &[u8], outlen: usize) -> [u8; 32] {
    blake2s_hash(data, outlen)
}

/// Compute keyed BLAKE2s hash
///
/// Delegates to `crate::crypto::hash::blake2s_keyed_hash`.
pub fn blake2s_keyed(key: &[u8], data: &[u8], outlen: usize) -> [u8; 32] {
    blake2s_keyed_hash(key, data, outlen)
}

/// HMAC-BLAKE2s: delegates to `HmacAlgorithm::HmacBlake2s`
pub fn hmac_blake2s(key: &[u8], data: &[u8]) -> [u8; 32] {
    HmacAlgorithm::HmacBlake2s.compute(key, data)
}

/// HKDF-BLAKE2s key derivation (extract + expand, two outputs)
///
/// Delegates to `KdfAlgorithm::HkdfBlake2s`.
fn hkdf(chaining_key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32]) {
    KdfAlgorithm::HkdfBlake2s.extract_expand2(chaining_key, input)
}

/// HKDF with three outputs
///
/// Delegates to `KdfAlgorithm::HkdfBlake2s`.
fn hkdf3(chaining_key: &[u8; 32], input: &[u8]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    KdfAlgorithm::HkdfBlake2s.extract_expand3(chaining_key, input)
}

// ── X25519 Key Exchange (delegates to crate::crypto::asymmetric) ────────────

/// X25519 key pair (Curve25519 Diffie-Hellman)
///
/// Uses the real X25519 scalar multiplication from `crate::crypto::asymmetric`
/// for public key derivation and Diffie-Hellman key exchange.
#[derive(Clone)]
pub struct X25519KeyPair {
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
}

impl X25519KeyPair {
    /// Generate a new key pair from a seed (deterministic for testing)
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let mut private_key = *seed;
        // Clamp per RFC 7748
        private_key[0] &= 248;
        private_key[31] &= 127;
        private_key[31] |= 64;

        // Derive public key via real X25519 scalar multiplication with basepoint
        let public_key = crate::crypto::asymmetric::x25519_scalar_mult(
            &private_key,
            &crate::crypto::asymmetric::X25519_BASEPOINT,
        );
        Self {
            private_key,
            public_key,
        }
    }

    /// Perform Diffie-Hellman key exchange
    pub fn dh(&self, their_public: &[u8; 32]) -> [u8; 32] {
        crate::crypto::asymmetric::x25519_scalar_mult(&self.private_key, their_public)
    }
}

// ── ChaCha20-Poly1305 AEAD (delegates to CipherSuite) ──────────────────────

/// AEAD encrypt with ChaCha20-Poly1305
///
/// Uses `CipherSuite::ChaCha20Poly1305` from the shared crypto module.
/// Returns ciphertext || 16-byte tag.
fn aead_encrypt(key: &[u8; 32], nonce: u64, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce_bytes = [0u8; CHACHA_NONCE_SIZE];
    nonce_bytes[4..12].copy_from_slice(&nonce.to_le_bytes());

    CipherSuite::ChaCha20Poly1305
        .encrypt_aead(key, &nonce_bytes, aad, plaintext)
        .unwrap_or_default()
}

/// AEAD decrypt with ChaCha20-Poly1305
///
/// Uses `CipherSuite::ChaCha20Poly1305` from the shared crypto module.
/// Returns plaintext or error if authentication fails.
fn aead_decrypt(
    key: &[u8; 32],
    nonce: u64,
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Vec<u8>, WireGuardError> {
    if ciphertext_and_tag.len() < TAG_SIZE {
        return Err(WireGuardError::DecryptionFailed);
    }

    let mut nonce_bytes = [0u8; CHACHA_NONCE_SIZE];
    nonce_bytes[4..12].copy_from_slice(&nonce.to_le_bytes());

    CipherSuite::ChaCha20Poly1305
        .decrypt_aead(key, &nonce_bytes, aad, ciphertext_and_tag)
        .map_err(|_| WireGuardError::DecryptionFailed)
}

// ── Error Types ─────────────────────────────────────────────────────────────

/// WireGuard protocol errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireGuardError {
    /// Handshake failed or timed out
    HandshakeFailed,
    /// AEAD decryption or authentication failure
    DecryptionFailed,
    /// Message rejected by anti-replay window
    ReplayDetected,
    /// Session has expired
    SessionExpired,
    /// Peer not found
    PeerNotFound,
    /// Invalid message format
    InvalidMessage,
    /// Nonce counter overflow
    NonceOverflow,
    /// Key rotation required
    RekeyRequired,
    /// Interface not configured
    NotConfigured,
    /// Maximum peers reached
    MaxPeersReached,
}

// ── Anti-Replay Window ──────────────────────────────────────────────────────

/// Sliding-window anti-replay mechanism (2048-bit bitmap)
#[derive(Clone)]
pub struct AntiReplayWindow {
    /// Highest accepted counter value
    last_counter: u64,
    /// Bitmap of recently seen counters (relative to last_counter)
    bitmap: [u64; REPLAY_WINDOW_WORDS],
}

impl Default for AntiReplayWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl AntiReplayWindow {
    /// Create new anti-replay window
    pub fn new() -> Self {
        Self {
            last_counter: 0,
            bitmap: [0u64; REPLAY_WINDOW_WORDS],
        }
    }

    /// Check if a counter value is acceptable (not a replay)
    pub fn check(&self, counter: u64) -> bool {
        if counter == 0 && self.last_counter == 0 && self.bitmap[0] == 0 {
            // First packet ever
            return true;
        }
        if counter > self.last_counter {
            return true;
        }
        let diff = self.last_counter - counter;
        if diff >= REPLAY_WINDOW_BITS as u64 {
            return false; // Too old
        }
        let word_idx = (diff / 64) as usize;
        let bit_idx = (diff % 64) as u32;
        if word_idx >= REPLAY_WINDOW_WORDS {
            return false;
        }
        (self.bitmap[word_idx] & (1u64 << bit_idx)) == 0
    }

    /// Update the window after accepting a packet
    pub fn update(&mut self, counter: u64) {
        if counter > self.last_counter {
            let shift = counter - self.last_counter;
            if shift >= REPLAY_WINDOW_BITS as u64 {
                // Reset entire window
                self.bitmap = [0u64; REPLAY_WINDOW_WORDS];
            } else {
                self.shift_window(shift as usize);
            }
            self.last_counter = counter;
            // Mark current counter as seen (bit 0)
            self.bitmap[0] |= 1;
        } else {
            let diff = self.last_counter - counter;
            let word_idx = (diff / 64) as usize;
            let bit_idx = (diff % 64) as u32;
            if word_idx < REPLAY_WINDOW_WORDS {
                self.bitmap[word_idx] |= 1u64 << bit_idx;
            }
        }
    }

    /// Shift the bitmap window by the given number of positions
    fn shift_window(&mut self, shift: usize) {
        let word_shift = shift / 64;
        let bit_shift = (shift % 64) as u32;

        if word_shift >= REPLAY_WINDOW_WORDS {
            self.bitmap = [0u64; REPLAY_WINDOW_WORDS];
            return;
        }

        // Shift by whole words
        if word_shift > 0 {
            let mut i = REPLAY_WINDOW_WORDS;
            while i > word_shift {
                i -= 1;
                self.bitmap[i] = self.bitmap[i - word_shift];
            }
            let mut j = 0;
            while j < word_shift {
                self.bitmap[j] = 0;
                j += 1;
            }
        }

        // Shift by remaining bits
        if bit_shift > 0 {
            let mut i = REPLAY_WINDOW_WORDS;
            while i > 1 {
                i -= 1;
                self.bitmap[i] =
                    (self.bitmap[i] << bit_shift) | (self.bitmap[i - 1] >> (64 - bit_shift));
            }
            self.bitmap[0] <<= bit_shift;
        }
    }
}

// ── Handshake State ─────────────────────────────────────────────────────────

/// Handshake state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    /// No handshake in progress
    None,
    /// Initiation sent, waiting for response
    InitSent,
    /// Initiation received (responder)
    InitReceived,
    /// Handshake complete, session established
    Established,
}

/// Session keys derived from handshake
pub struct SessionKeys {
    /// Key for sending
    pub sending_key: [u8; 32],
    /// Key for receiving
    pub receiving_key: [u8; 32],
    /// Sending nonce counter
    pub sending_nonce: AtomicU64,
    /// Time when keys were derived (uptime seconds)
    pub created_at: u64,
    /// Number of messages sent with these keys
    pub messages_sent: AtomicU64,
}

impl SessionKeys {
    /// Create new session keys
    pub fn new(sending_key: [u8; 32], receiving_key: [u8; 32], now: u64) -> Self {
        Self {
            sending_key,
            receiving_key,
            sending_nonce: AtomicU64::new(0),
            created_at: now,
            messages_sent: AtomicU64::new(0),
        }
    }

    /// Check if keys need rotation
    pub fn needs_rekey(&self, now: u64) -> bool {
        let messages = self.messages_sent.load(Ordering::Relaxed);
        let age = now.saturating_sub(self.created_at);
        messages >= REKEY_AFTER_MESSAGES || age >= REKEY_AFTER_SECONDS
    }

    /// Get and increment the sending nonce
    pub fn next_nonce(&self) -> Result<u64, WireGuardError> {
        let nonce = self.sending_nonce.fetch_add(1, Ordering::Relaxed);
        if nonce >= REKEY_AFTER_MESSAGES {
            return Err(WireGuardError::NonceOverflow);
        }
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
        Ok(nonce)
    }
}

/// Handshake context for Noise_IKpsk2
#[derive(Clone)]
pub struct HandshakeContext {
    /// Chaining key
    pub chaining_key: [u8; 32],
    /// Hash state
    pub hash: [u8; 32],
    /// Local ephemeral key pair
    pub ephemeral: Option<X25519KeyPair>,
    /// Remote ephemeral public key
    pub remote_ephemeral: Option<[u8; 32]>,
    /// Our sender index
    pub sender_index: u32,
    /// Their sender index
    pub receiver_index: u32,
}

impl Default for HandshakeContext {
    fn default() -> Self {
        Self::new()
    }
}

impl HandshakeContext {
    /// Initialize a new handshake context
    pub fn new() -> Self {
        // Initial chaining key = BLAKE2s(CONSTRUCTION)
        let chaining_key = blake2s(CONSTRUCTION, 32);
        // Initial hash = BLAKE2s(chaining_key || IDENTIFIER)
        let mut hash_input = Vec::with_capacity(32 + IDENTIFIER.len());
        hash_input.extend_from_slice(&chaining_key);
        hash_input.extend_from_slice(IDENTIFIER);
        let hash = blake2s(&hash_input, 32);
        Self {
            chaining_key,
            hash,
            ephemeral: None,
            remote_ephemeral: None,
            sender_index: 0,
            receiver_index: 0,
        }
    }

    /// Mix hash: h = BLAKE2s(h || data)
    pub fn mix_hash(&mut self, data: &[u8]) {
        let mut input = Vec::with_capacity(32 + data.len());
        input.extend_from_slice(&self.hash);
        input.extend_from_slice(data);
        self.hash = blake2s(&input, 32);
    }

    /// Build handshake initiation message (148 bytes)
    pub fn create_initiation(
        &mut self,
        static_key: &X25519KeyPair,
        remote_static_pub: &[u8; 32],
        preshared_key: &[u8; 32],
        timestamp: &[u8; 12],
        sender_index: u32,
    ) -> [u8; HANDSHAKE_INIT_SIZE] {
        let mut msg = [0u8; HANDSHAKE_INIT_SIZE];
        self.sender_index = sender_index;

        // Mix responder's static public key into hash
        self.mix_hash(remote_static_pub);

        // Generate ephemeral key (deterministic from static key + timestamp for test)
        let seed = hmac_blake2s(&static_key.private_key, timestamp);
        let ephemeral = X25519KeyPair::from_seed(&seed);

        // msg[0..4] = type + reserved
        msg[0] = MSG_HANDSHAKE_INIT;
        // msg[4..8] = sender index
        msg[4..8].copy_from_slice(&sender_index.to_le_bytes());
        // msg[8..40] = unencrypted ephemeral
        msg[8..40].copy_from_slice(&ephemeral.public_key);
        self.mix_hash(&ephemeral.public_key);

        // DH: ephemeral <-> remote static
        let dh_result = ephemeral.dh(remote_static_pub);
        let (ck, key) = hkdf(&self.chaining_key, &dh_result);
        self.chaining_key = ck;

        // msg[40..88] = AEAD(key, 0, static_pub, h)
        let encrypted_static = aead_encrypt(&key, 0, &self.hash, &static_key.public_key);
        let copy_len = core::cmp::min(encrypted_static.len(), 48);
        msg[40..40 + copy_len].copy_from_slice(&encrypted_static[..copy_len]);
        self.mix_hash(&msg[40..40 + copy_len]);

        // DH: static <-> remote static
        let dh_result2 = static_key.dh(remote_static_pub);
        let (ck2, key2) = hkdf(&self.chaining_key, &dh_result2);
        self.chaining_key = ck2;

        // msg[88..116] = AEAD(key2, 0, timestamp, h)
        let encrypted_ts = aead_encrypt(&key2, 0, &self.hash, timestamp);
        let ts_len = core::cmp::min(encrypted_ts.len(), 28);
        msg[88..88 + ts_len].copy_from_slice(&encrypted_ts[..ts_len]);
        self.mix_hash(&msg[88..88 + ts_len]);

        // PSK mixing
        let (ck3, psk_key) = hkdf(&self.chaining_key, preshared_key);
        self.chaining_key = ck3;
        let _ = psk_key; // Used for MAC in full implementation

        // msg[116..132] = MAC1 (BLAKE2s of msg[0..116] keyed with remote static hash)
        let mac_key = blake2s(remote_static_pub, 32);
        let mac1 = blake2s_keyed(&mac_key, &msg[..116], 16);
        msg[116..132].copy_from_slice(&mac1[..16]);

        // msg[132..148] = MAC2 (cookie, zero if no cookie)
        // Left as zero (no cookie)

        self.ephemeral = Some(ephemeral);
        msg
    }

    /// Build handshake response message (92 bytes)
    pub fn create_response(
        &mut self,
        static_key: &X25519KeyPair,
        remote_static_pub: &[u8; 32],
        preshared_key: &[u8; 32],
        sender_index: u32,
        receiver_index: u32,
    ) -> [u8; HANDSHAKE_RESP_SIZE] {
        let mut msg = [0u8; HANDSHAKE_RESP_SIZE];

        // Generate ephemeral
        let seed = hmac_blake2s(&static_key.private_key, &sender_index.to_le_bytes());
        let ephemeral = X25519KeyPair::from_seed(&seed);

        // msg[0..4] = type + reserved
        msg[0] = MSG_HANDSHAKE_RESP;
        // msg[4..8] = sender index
        msg[4..8].copy_from_slice(&sender_index.to_le_bytes());
        // msg[8..12] = receiver index
        msg[8..12].copy_from_slice(&receiver_index.to_le_bytes());
        // msg[12..44] = unencrypted ephemeral
        msg[12..44].copy_from_slice(&ephemeral.public_key);
        self.mix_hash(&ephemeral.public_key);

        // DH: responder ephemeral <-> initiator ephemeral
        if let Some(ref remote_eph) = self.remote_ephemeral {
            let dh1 = ephemeral.dh(remote_eph);
            let (ck, _) = hkdf(&self.chaining_key, &dh1);
            self.chaining_key = ck;
        }

        // DH: responder ephemeral <-> initiator static
        let dh2 = ephemeral.dh(remote_static_pub);
        let (ck2, _) = hkdf(&self.chaining_key, &dh2);
        self.chaining_key = ck2;

        // PSK mixing
        let (ck3, tau, key) = hkdf3(&self.chaining_key, preshared_key);
        self.chaining_key = ck3;
        self.mix_hash(&tau);

        // msg[44..60] = AEAD(key, 0, empty, h) -- encrypted nothing
        let encrypted_empty = aead_encrypt(&key, 0, &self.hash, &[]);
        let empty_len = core::cmp::min(encrypted_empty.len(), 16);
        msg[44..44 + empty_len].copy_from_slice(&encrypted_empty[..empty_len]);
        self.mix_hash(&msg[44..44 + empty_len]);

        // msg[60..76] = MAC1
        let mac_key = blake2s(remote_static_pub, 32);
        let mac1 = blake2s_keyed(&mac_key, &msg[..60], 16);
        msg[60..76].copy_from_slice(&mac1[..16]);

        // msg[76..92] = MAC2 (zero if no cookie)

        self.sender_index = sender_index;
        self.receiver_index = receiver_index;
        self.ephemeral = Some(ephemeral);
        msg
    }

    /// Derive transport keys from completed handshake
    pub fn derive_transport_keys(&self) -> (SessionKeys, SessionKeys) {
        let (t1, t2) = hkdf(&self.chaining_key, &[]);
        let now = 0u64; // Caller should provide real timestamp
        (SessionKeys::new(t1, t2, now), SessionKeys::new(t2, t1, now))
    }
}

// ── Peer Management ─────────────────────────────────────────────────────────

/// Allowed IP range for a peer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllowedIp {
    /// Network address
    pub address: Ipv4Address,
    /// Prefix length (CIDR notation)
    pub prefix_len: u8,
}

impl AllowedIp {
    /// Create new allowed IP range
    pub fn new(address: Ipv4Address, prefix_len: u8) -> Self {
        Self {
            address,
            prefix_len,
        }
    }

    /// Check if an IP address matches this allowed range
    pub fn matches(&self, ip: &Ipv4Address) -> bool {
        if self.prefix_len == 0 {
            return true; // 0.0.0.0/0 matches everything
        }
        if self.prefix_len >= 32 {
            return self.address == *ip;
        }
        let mask = u32::MAX << (32 - self.prefix_len);
        (self.address.to_u32() & mask) == (ip.to_u32() & mask)
    }
}

/// WireGuard peer
pub struct WireGuardPeer {
    /// Peer's static public key
    pub public_key: [u8; 32],
    /// Pre-shared key (optional, all zeros if none)
    pub preshared_key: [u8; 32],
    /// Peer's endpoint (IP:port)
    pub endpoint: Option<super::SocketAddr>,
    /// Allowed IP ranges
    pub allowed_ips: Vec<AllowedIp>,
    /// Handshake state
    pub handshake_state: HandshakeState,
    /// Current handshake context
    pub handshake: HandshakeContext,
    /// Current session keys
    pub session: Option<SessionKeys>,
    /// Anti-replay window
    pub replay_window: AntiReplayWindow,
    /// Last handshake timestamp (uptime seconds)
    pub last_handshake: u64,
    /// Last data received timestamp
    pub last_received: u64,
    /// Last data sent timestamp
    pub last_sent: u64,
    /// Persistent keepalive interval (0 = disabled)
    pub keepalive_interval: u64,
    /// Handshake retry count
    pub handshake_retries: u32,
    /// Next retry time (uptime milliseconds)
    pub next_retry_ms: u64,
    /// Bytes transmitted
    pub tx_bytes: u64,
    /// Bytes received
    pub rx_bytes: u64,
}

impl WireGuardPeer {
    /// Create a new peer
    pub fn new(public_key: [u8; 32]) -> Self {
        Self {
            public_key,
            preshared_key: [0u8; 32],
            endpoint: None,
            allowed_ips: Vec::new(),
            handshake_state: HandshakeState::None,
            handshake: HandshakeContext::new(),
            session: None,
            replay_window: AntiReplayWindow::new(),
            last_handshake: 0,
            last_received: 0,
            last_sent: 0,
            keepalive_interval: DEFAULT_KEEPALIVE_INTERVAL,
            handshake_retries: 0,
            next_retry_ms: 0,
            tx_bytes: 0,
            rx_bytes: 0,
        }
    }

    /// Set pre-shared key
    pub fn set_preshared_key(&mut self, psk: [u8; 32]) {
        self.preshared_key = psk;
    }

    /// Add an allowed IP range
    pub fn add_allowed_ip(&mut self, ip: AllowedIp) {
        self.allowed_ips.push(ip);
    }

    /// Check if a destination IP is allowed for this peer
    pub fn is_allowed(&self, ip: &Ipv4Address) -> bool {
        self.allowed_ips.iter().any(|aip| aip.matches(ip))
    }

    /// Check if session has expired
    pub fn is_session_expired(&self, now: u64) -> bool {
        if self.last_received == 0 && self.last_sent == 0 {
            return false; // No session yet
        }
        let last_activity = core::cmp::max(self.last_received, self.last_sent);
        now.saturating_sub(last_activity) >= SESSION_EXPIRY_SECONDS
    }

    /// Check if keepalive should be sent
    pub fn needs_keepalive(&self, now: u64) -> bool {
        if self.keepalive_interval == 0 {
            return false;
        }
        if self.handshake_state != HandshakeState::Established {
            return false;
        }
        now.saturating_sub(self.last_sent) >= self.keepalive_interval
    }

    /// Calculate next handshake retry delay with exponential backoff (ms)
    pub fn retry_delay_ms(&self) -> u64 {
        if self.handshake_retries >= MAX_HANDSHAKE_RETRIES {
            return 0; // Give up
        }
        // Exponential backoff: 1s, 2s, 4s, 8s, 16s
        let mut delay = INITIAL_RETRY_DELAY_MS;
        let mut i = 0u32;
        while i < self.handshake_retries {
            delay = delay.saturating_mul(2);
            i += 1;
        }
        delay
    }
}

// ── Transport ───────────────────────────────────────────────────────────────

/// Encrypt a transport data packet
pub fn encrypt_transport(
    session: &SessionKeys,
    receiver_index: u32,
    payload: &[u8],
) -> Result<Vec<u8>, WireGuardError> {
    let nonce = session.next_nonce()?;

    // Pad payload to 16-byte boundary
    let padded_len = (payload.len() + 15) & !15;
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(payload);
    padded.resize(padded_len, 0);

    // Encrypt
    let encrypted = aead_encrypt(&session.sending_key, nonce, &[], &padded);

    // Build transport message: type(4) + receiver(4) + counter(8) + encrypted
    let mut msg = Vec::with_capacity(TRANSPORT_HEADER_SIZE + encrypted.len());
    msg.extend_from_slice(&[MSG_TRANSPORT_DATA, 0, 0, 0]);
    msg.extend_from_slice(&receiver_index.to_le_bytes());
    msg.extend_from_slice(&nonce.to_le_bytes());
    msg.extend_from_slice(&encrypted);

    Ok(msg)
}

/// Decrypt a transport data packet
pub fn decrypt_transport(
    session: &SessionKeys,
    replay_window: &mut AntiReplayWindow,
    packet: &[u8],
) -> Result<Vec<u8>, WireGuardError> {
    if packet.len() < TRANSPORT_HEADER_SIZE + TAG_SIZE {
        return Err(WireGuardError::InvalidMessage);
    }
    if packet[0] != MSG_TRANSPORT_DATA {
        return Err(WireGuardError::InvalidMessage);
    }

    let counter = u64::from_le_bytes([
        packet[8], packet[9], packet[10], packet[11], packet[12], packet[13], packet[14],
        packet[15],
    ]);

    // Anti-replay check
    if !replay_window.check(counter) {
        return Err(WireGuardError::ReplayDetected);
    }

    // Decrypt
    let plaintext = aead_decrypt(
        &session.receiving_key,
        counter,
        &[],
        &packet[TRANSPORT_HEADER_SIZE..],
    )?;

    // Update replay window after successful decryption
    replay_window.update(counter);

    Ok(plaintext)
}

// ── Virtual Interface ───────────────────────────────────────────────────────

/// WireGuard virtual network interface (wg0)
pub struct WireGuardInterface {
    /// Interface name
    pub name: [u8; 16],
    /// Local static key pair
    pub static_key: X25519KeyPair,
    /// Listening UDP port
    pub listen_port: u16,
    /// Tunnel IP address
    pub tunnel_address: Option<IpAddress>,
    /// Tunnel subnet prefix length
    pub tunnel_prefix: u8,
    /// Peer table: hash of public key -> peer
    pub peers: BTreeMap<u64, WireGuardPeer>,
    /// Interface MTU
    pub mtu: u16,
    /// Whether the interface is up
    pub is_up: bool,
    /// Next sender index to assign
    next_sender_index: u32,
    /// Packet counter for statistics
    pub packets_in: u64,
    pub packets_out: u64,
}

impl WireGuardInterface {
    /// Create a new WireGuard interface
    pub fn new(name: &[u8], static_key: X25519KeyPair, listen_port: u16) -> Self {
        let mut name_buf = [0u8; 16];
        let copy_len = core::cmp::min(name.len(), 15);
        name_buf[..copy_len].copy_from_slice(&name[..copy_len]);
        Self {
            name: name_buf,
            static_key,
            listen_port,
            tunnel_address: None,
            tunnel_prefix: 24,
            peers: BTreeMap::new(),
            mtu: 1420, // Standard WireGuard MTU (1500 - 80)
            is_up: false,
            next_sender_index: 1,
            packets_in: 0,
            packets_out: 0,
        }
    }

    /// Set tunnel IP address
    pub fn set_address(&mut self, addr: IpAddress, prefix: u8) {
        self.tunnel_address = Some(addr);
        self.tunnel_prefix = prefix;
    }

    /// Calculate effective MTU based on outer transport.
    ///
    /// - IPv4 outer: outer_mtu - 20 (IP) - 8 (UDP) - 32 (WG overhead) =
    ///   outer_mtu - 60
    /// - IPv6 outer: outer_mtu - 40 (IP) - 8 (UDP) - 32 (WG overhead) =
    ///   outer_mtu - 80
    pub fn calculate_mtu(outer_mtu: u16, is_ipv6: bool) -> u16 {
        let overhead = if is_ipv6 { 80u16 } else { 60u16 };
        outer_mtu.saturating_sub(overhead)
    }

    /// Compute a hash key for a peer's public key
    fn peer_key_hash(public_key: &[u8; 32]) -> u64 {
        let hash = blake2s(public_key, 32);
        u64::from_le_bytes([
            hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
        ])
    }

    /// Add a peer
    pub fn add_peer(&mut self, peer: WireGuardPeer) -> Result<(), WireGuardError> {
        let key = Self::peer_key_hash(&peer.public_key);
        self.peers.insert(key, peer);
        Ok(())
    }

    /// Remove a peer by public key
    pub fn remove_peer(&mut self, public_key: &[u8; 32]) -> Result<(), WireGuardError> {
        let key = Self::peer_key_hash(public_key);
        self.peers
            .remove(&key)
            .map(|_| ())
            .ok_or(WireGuardError::PeerNotFound)
    }

    /// Look up a peer by public key
    pub fn get_peer(&self, public_key: &[u8; 32]) -> Option<&WireGuardPeer> {
        let key = Self::peer_key_hash(public_key);
        self.peers.get(&key)
    }

    /// Look up a peer mutably by public key
    pub fn get_peer_mut(&mut self, public_key: &[u8; 32]) -> Option<&mut WireGuardPeer> {
        let key = Self::peer_key_hash(public_key);
        self.peers.get_mut(&key)
    }

    /// Find a peer that handles a given destination IP
    pub fn find_peer_for_ip(&self, dst: &Ipv4Address) -> Option<&WireGuardPeer> {
        self.peers.values().find(|peer| peer.is_allowed(dst))
    }

    /// Bring the interface up
    pub fn up(&mut self) -> Result<(), WireGuardError> {
        if self.tunnel_address.is_none() {
            return Err(WireGuardError::NotConfigured);
        }
        self.is_up = true;
        Ok(())
    }

    /// Bring the interface down
    pub fn down(&mut self) {
        self.is_up = false;
    }

    /// Allocate a new sender index
    pub fn alloc_sender_index(&mut self) -> u32 {
        let idx = self.next_sender_index;
        self.next_sender_index = self.next_sender_index.wrapping_add(1);
        if self.next_sender_index == 0 {
            self.next_sender_index = 1;
        }
        idx
    }

    /// Get peer count
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

// ── Timer Management ────────────────────────────────────────────────────────

/// Timer events for WireGuard session management
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerEvent {
    /// Time to initiate a rekey
    RekeyInitiate,
    /// Handshake retry needed
    HandshakeRetry,
    /// Session has expired, clear keys
    SessionExpiry,
    /// Dead peer detected
    DeadPeer,
    /// Send keepalive
    Keepalive,
}

/// Timer state for a peer
pub struct PeerTimers {
    /// Handshake initiated timestamp (ms)
    pub handshake_initiated_ms: u64,
    /// Last keepalive sent timestamp (s)
    pub last_keepalive_sent: u64,
    /// Whether a rekey is pending
    pub rekey_pending: bool,
}

impl Default for PeerTimers {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerTimers {
    pub fn new() -> Self {
        Self {
            handshake_initiated_ms: 0,
            last_keepalive_sent: 0,
            rekey_pending: false,
        }
    }
}

/// Check timer events for a peer
pub fn check_peer_timers(
    peer: &WireGuardPeer,
    timers: &PeerTimers,
    now_secs: u64,
    now_ms: u64,
) -> Option<TimerEvent> {
    // Check session expiry first (highest priority)
    if peer.is_session_expired(now_secs) {
        return Some(TimerEvent::SessionExpiry);
    }

    // Check rekey needed
    if let Some(ref session) = peer.session {
        if session.needs_rekey(now_secs) && !timers.rekey_pending {
            return Some(TimerEvent::RekeyInitiate);
        }
    }

    // Check handshake retry
    if peer.handshake_state == HandshakeState::InitSent {
        if peer.handshake_retries >= MAX_HANDSHAKE_RETRIES {
            return Some(TimerEvent::DeadPeer);
        }
        if now_ms >= peer.next_retry_ms && peer.next_retry_ms > 0 {
            return Some(TimerEvent::HandshakeRetry);
        }
    }

    // Check keepalive
    if peer.needs_keepalive(now_secs) {
        return Some(TimerEvent::Keepalive);
    }

    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // ── BLAKE2s Tests ───────────────────────────────────────────────────

    #[test]
    fn test_blake2s_empty_input() {
        // RFC 7693 Appendix A: BLAKE2s-256("")
        let hash = blake2s(b"", 32);
        // Known BLAKE2s-256 of empty string
        assert_eq!(hash[0], 0x69);
        assert_eq!(hash[1], 0x21);
        assert_eq!(hash[2], 0x7a);
        assert_eq!(hash[3], 0x30);
    }

    #[test]
    fn test_blake2s_abc() {
        // BLAKE2s-256("abc") known test vector
        let hash = blake2s(b"abc", 32);
        assert_eq!(hash[0], 0x50);
        assert_eq!(hash[1], 0x8C);
        // Non-zero output
        assert!(hash.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_blake2s_deterministic() {
        let h1 = blake2s(b"test data", 32);
        let h2 = blake2s(b"test data", 32);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_blake2s_different_inputs() {
        let h1 = blake2s(b"hello", 32);
        let h2 = blake2s(b"world", 32);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_blake2s_keyed_mode() {
        let key = [0x42u8; 32];
        let h1 = blake2s_keyed(&key, b"data", 32);
        let h2 = blake2s(b"data", 32);
        // Keyed hash should differ from unkeyed
        assert_ne!(h1, h2);
        // Keyed hash is deterministic
        let h3 = blake2s_keyed(&key, b"data", 32);
        assert_eq!(h1, h3);
    }

    #[test]
    fn test_blake2s_keyed_different_keys() {
        let key1 = [0x01u8; 32];
        let key2 = [0x02u8; 32];
        let h1 = blake2s_keyed(&key1, b"data", 32);
        let h2 = blake2s_keyed(&key2, b"data", 32);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hmac_blake2s() {
        let key = [0xABu8; 32];
        let mac1 = hmac_blake2s(&key, b"message");
        let mac2 = hmac_blake2s(&key, b"message");
        assert_eq!(mac1, mac2);

        let mac3 = hmac_blake2s(&key, b"different message");
        assert_ne!(mac1, mac3);
    }

    // ── Anti-Replay Window Tests ────────────────────────────────────────

    #[test]
    fn test_replay_window_accept_new() {
        let mut window = AntiReplayWindow::new();
        assert!(window.check(0));
        window.update(0);
        assert!(window.check(1));
        window.update(1);
        assert!(window.check(2));
        window.update(2);
        assert!(window.check(100));
    }

    #[test]
    fn test_replay_window_reject_duplicate() {
        let mut window = AntiReplayWindow::new();
        window.update(5);
        assert!(!window.check(5)); // Already seen
    }

    #[test]
    fn test_replay_window_reject_old() {
        let mut window = AntiReplayWindow::new();
        window.update(3000);
        // Counter 0 is now outside the 2048-bit window
        assert!(!window.check(0));
    }

    #[test]
    fn test_replay_window_accept_within_window() {
        let mut window = AntiReplayWindow::new();
        window.update(100);
        // Counter 99 is within window and not yet seen
        assert!(window.check(99));
        window.update(99);
        // But now it's seen
        assert!(!window.check(99));
    }

    #[test]
    fn test_replay_window_large_jump() {
        let mut window = AntiReplayWindow::new();
        window.update(0);
        window.update(10000);
        // All old counters are outside the window
        assert!(!window.check(0));
        assert!(!window.check(100));
        // New counter is accepted
        assert!(window.check(10001));
    }

    // ── Peer Management Tests ───────────────────────────────────────────

    #[test]
    fn test_peer_add_remove() {
        let seed = [1u8; 32];
        let key = X25519KeyPair::from_seed(&seed);
        let mut iface = WireGuardInterface::new(b"wg0", key, DEFAULT_PORT);

        let peer_pub = [0x42u8; 32];
        let peer = WireGuardPeer::new(peer_pub);
        assert!(iface.add_peer(peer).is_ok());
        assert_eq!(iface.peer_count(), 1);

        assert!(iface.get_peer(&peer_pub).is_some());
        assert!(iface.remove_peer(&peer_pub).is_ok());
        assert_eq!(iface.peer_count(), 0);
        assert!(iface.get_peer(&peer_pub).is_none());
    }

    #[test]
    fn test_peer_remove_not_found() {
        let seed = [1u8; 32];
        let key = X25519KeyPair::from_seed(&seed);
        let mut iface = WireGuardInterface::new(b"wg0", key, DEFAULT_PORT);

        let fake_pub = [0xFFu8; 32];
        assert_eq!(
            iface.remove_peer(&fake_pub),
            Err(WireGuardError::PeerNotFound)
        );
    }

    #[test]
    fn test_peer_lookup() {
        let seed = [1u8; 32];
        let key = X25519KeyPair::from_seed(&seed);
        let mut iface = WireGuardInterface::new(b"wg0", key, DEFAULT_PORT);

        let pub1 = [0x01u8; 32];
        let pub2 = [0x02u8; 32];
        iface.add_peer(WireGuardPeer::new(pub1)).unwrap();
        iface.add_peer(WireGuardPeer::new(pub2)).unwrap();

        assert!(iface.get_peer(&pub1).is_some());
        assert!(iface.get_peer(&pub2).is_some());
        assert_eq!(iface.peer_count(), 2);
    }

    // ── Key Rotation and Timer Tests ────────────────────────────────────

    #[test]
    fn test_key_rotation_by_time() {
        let keys = SessionKeys::new([1u8; 32], [2u8; 32], 0);
        assert!(!keys.needs_rekey(0));
        assert!(!keys.needs_rekey(119));
        assert!(keys.needs_rekey(120)); // REKEY_AFTER_SECONDS
        assert!(keys.needs_rekey(200));
    }

    #[test]
    fn test_key_rotation_by_messages() {
        let keys = SessionKeys::new([1u8; 32], [2u8; 32], 0);
        // Simulate many messages by setting counter directly
        keys.messages_sent
            .store(REKEY_AFTER_MESSAGES, Ordering::Relaxed);
        assert!(keys.needs_rekey(0));
    }

    // ── MTU Calculation Tests ───────────────────────────────────────────

    #[test]
    fn test_mtu_calculation_ipv4() {
        // Standard 1500 MTU - 60 (IPv4 overhead) = 1440
        assert_eq!(WireGuardInterface::calculate_mtu(1500, false), 1440);
    }

    #[test]
    fn test_mtu_calculation_ipv6() {
        // Standard 1500 MTU - 80 (IPv6 overhead) = 1420
        assert_eq!(WireGuardInterface::calculate_mtu(1500, true), 1420);
    }

    #[test]
    fn test_mtu_calculation_small() {
        // Saturating subtraction prevents underflow
        assert_eq!(WireGuardInterface::calculate_mtu(50, false), 0);
        assert_eq!(WireGuardInterface::calculate_mtu(60, false), 0);
        assert_eq!(WireGuardInterface::calculate_mtu(61, false), 1);
    }

    // ── Nonce Counter Tests ─────────────────────────────────────────────

    #[test]
    fn test_nonce_counter_increment() {
        let keys = SessionKeys::new([1u8; 32], [2u8; 32], 0);
        assert_eq!(keys.next_nonce().unwrap(), 0);
        assert_eq!(keys.next_nonce().unwrap(), 1);
        assert_eq!(keys.next_nonce().unwrap(), 2);
    }

    #[test]
    fn test_nonce_counter_overflow() {
        let keys = SessionKeys::new([1u8; 32], [2u8; 32], 0);
        keys.sending_nonce
            .store(REKEY_AFTER_MESSAGES, Ordering::Relaxed);
        assert_eq!(keys.next_nonce(), Err(WireGuardError::NonceOverflow));
    }

    // ── Allowed IP Matching Tests ───────────────────────────────────────

    #[test]
    fn test_allowed_ip_exact_match() {
        let aip = AllowedIp::new(Ipv4Address::new(10, 0, 0, 1), 32);
        assert!(aip.matches(&Ipv4Address::new(10, 0, 0, 1)));
        assert!(!aip.matches(&Ipv4Address::new(10, 0, 0, 2)));
    }

    #[test]
    fn test_allowed_ip_subnet_match() {
        let aip = AllowedIp::new(Ipv4Address::new(10, 0, 0, 0), 24);
        assert!(aip.matches(&Ipv4Address::new(10, 0, 0, 1)));
        assert!(aip.matches(&Ipv4Address::new(10, 0, 0, 254)));
        assert!(!aip.matches(&Ipv4Address::new(10, 0, 1, 1)));
    }

    #[test]
    fn test_allowed_ip_wildcard() {
        let aip = AllowedIp::new(Ipv4Address::new(0, 0, 0, 0), 0);
        assert!(aip.matches(&Ipv4Address::new(192, 168, 1, 1)));
        assert!(aip.matches(&Ipv4Address::new(10, 0, 0, 1)));
    }

    // ── Session State Transitions ───────────────────────────────────────

    #[test]
    fn test_session_state_transitions() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        assert_eq!(peer.handshake_state, HandshakeState::None);

        peer.handshake_state = HandshakeState::InitSent;
        assert_eq!(peer.handshake_state, HandshakeState::InitSent);

        peer.handshake_state = HandshakeState::Established;
        assert_eq!(peer.handshake_state, HandshakeState::Established);
    }

    #[test]
    fn test_session_expiry() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.last_received = 100;
        peer.last_sent = 100;

        assert!(!peer.is_session_expired(200)); // 100s elapsed < 180s threshold
        assert!(peer.is_session_expired(281)); // 181s elapsed >= 180s threshold
    }

    #[test]
    fn test_handshake_retry_backoff() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.handshake_retries = 0;
        assert_eq!(peer.retry_delay_ms(), 1000);

        peer.handshake_retries = 1;
        assert_eq!(peer.retry_delay_ms(), 2000);

        peer.handshake_retries = 2;
        assert_eq!(peer.retry_delay_ms(), 4000);

        peer.handshake_retries = 3;
        assert_eq!(peer.retry_delay_ms(), 8000);

        peer.handshake_retries = 4;
        assert_eq!(peer.retry_delay_ms(), 16000);

        // Max retries exceeded -> give up
        peer.handshake_retries = MAX_HANDSHAKE_RETRIES;
        assert_eq!(peer.retry_delay_ms(), 0);
    }

    // ── Handshake Message Construction ──────────────────────────────────

    #[test]
    fn test_handshake_initiation_size() {
        let static_key = X25519KeyPair::from_seed(&[0x11u8; 32]);
        let remote_pub = [0x22u8; 32];
        let psk = [0x33u8; 32];
        let timestamp = [0u8; 12];

        let mut ctx = HandshakeContext::new();
        let msg = ctx.create_initiation(&static_key, &remote_pub, &psk, &timestamp, 1);

        assert_eq!(msg.len(), HANDSHAKE_INIT_SIZE);
        assert_eq!(msg[0], MSG_HANDSHAKE_INIT);
        // Sender index at offset 4
        assert_eq!(u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]), 1);
    }

    #[test]
    fn test_handshake_response_size() {
        let static_key = X25519KeyPair::from_seed(&[0x44u8; 32]);
        let remote_pub = [0x55u8; 32];
        let psk = [0x66u8; 32];

        let mut ctx = HandshakeContext::new();
        let msg = ctx.create_response(&static_key, &remote_pub, &psk, 2, 1);

        assert_eq!(msg.len(), HANDSHAKE_RESP_SIZE);
        assert_eq!(msg[0], MSG_HANDSHAKE_RESP);
        // Sender index at offset 4
        assert_eq!(u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]), 2);
        // Receiver index at offset 8
        assert_eq!(u32::from_le_bytes([msg[8], msg[9], msg[10], msg[11]]), 1);
    }

    // ── Transport Encrypt/Decrypt ───────────────────────────────────────

    #[test]
    fn test_transport_encrypt_decrypt() {
        let send_key = [0xAAu8; 32];
        let recv_key = [0xAAu8; 32]; // Same for round-trip test
        let send_session = SessionKeys::new(send_key, [0u8; 32], 0);
        let recv_session = SessionKeys::new([0u8; 32], recv_key, 0);

        let payload = b"hello wireguard";
        let encrypted = encrypt_transport(&send_session, 42, payload).unwrap();

        // Header: type(4) + receiver(4) + counter(8)
        assert_eq!(encrypted[0], MSG_TRANSPORT_DATA);
        assert_eq!(
            u32::from_le_bytes([encrypted[4], encrypted[5], encrypted[6], encrypted[7]]),
            42
        );

        let mut window = AntiReplayWindow::new();
        let decrypted = decrypt_transport(&recv_session, &mut window, &encrypted).unwrap();
        // Decrypted payload is padded to 16-byte boundary
        assert!(decrypted.len() >= payload.len());
        assert_eq!(&decrypted[..payload.len()], payload);
    }

    // ── Timer Event Tests ───────────────────────────────────────────────

    #[test]
    fn test_keepalive_timing() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.handshake_state = HandshakeState::Established;
        peer.keepalive_interval = 25;
        peer.last_sent = 100;

        assert!(!peer.needs_keepalive(120)); // 20s < 25s
        assert!(peer.needs_keepalive(125)); // 25s >= 25s
        assert!(peer.needs_keepalive(200)); // 100s >= 25s
    }

    #[test]
    fn test_keepalive_disabled() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.handshake_state = HandshakeState::Established;
        peer.keepalive_interval = 0;
        peer.last_sent = 0;

        assert!(!peer.needs_keepalive(1000));
    }

    #[test]
    fn test_timer_event_session_expiry() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.last_received = 100;
        peer.last_sent = 100;
        let timers = PeerTimers::new();

        let event = check_peer_timers(&peer, &timers, 300, 300_000);
        assert_eq!(event, Some(TimerEvent::SessionExpiry));
    }

    #[test]
    fn test_timer_event_dead_peer() {
        let mut peer = WireGuardPeer::new([0x01u8; 32]);
        peer.handshake_state = HandshakeState::InitSent;
        peer.handshake_retries = MAX_HANDSHAKE_RETRIES;
        let timers = PeerTimers::new();

        let event = check_peer_timers(&peer, &timers, 0, 0);
        assert_eq!(event, Some(TimerEvent::DeadPeer));
    }

    #[test]
    fn test_interface_up_down() {
        let seed = [1u8; 32];
        let key = X25519KeyPair::from_seed(&seed);
        let mut iface = WireGuardInterface::new(b"wg0", key, DEFAULT_PORT);

        // Cannot bring up without address
        assert_eq!(iface.up(), Err(WireGuardError::NotConfigured));

        iface.set_address(IpAddress::V4(Ipv4Address::new(10, 0, 0, 1)), 24);
        assert!(iface.up().is_ok());
        assert!(iface.is_up);

        iface.down();
        assert!(!iface.is_up);
    }
}
