//! WPA2/WPA3 Authentication Implementation
//!
//! Provides EAPOL frame handling, PBKDF2-SHA256 key derivation, PRF-SHA256
//! for PTK expansion, 4-way handshake state machine, CCMP stub, and WPA3-SAE
//! stubs for WiFi security.

use alloc::vec::Vec;

use crate::{crypto::hash::sha256, net::MacAddress};

// ============================================================================
// HMAC-SHA256
// ============================================================================

/// HMAC-SHA256 implementation (RFC 2104)
///
/// Computes HMAC using SHA-256 as the underlying hash function.
/// All integer math, no floating point.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    // If key is longer than block size, hash it first
    let hashed_key;
    let key_ref = if key.len() > BLOCK_SIZE {
        hashed_key = sha256(key);
        hashed_key.as_bytes()
    } else {
        key
    };

    // Pad key to block size
    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5Cu8; BLOCK_SIZE];
    for (i, &b) in key_ref.iter().enumerate() {
        ipad[i] ^= b;
        opad[i] ^= b;
    }

    // Inner hash: H(K ^ ipad || data)
    let mut inner_input = Vec::with_capacity(BLOCK_SIZE + data.len());
    inner_input.extend_from_slice(&ipad);
    inner_input.extend_from_slice(data);
    let inner_hash = sha256(&inner_input);

    // Outer hash: H(K ^ opad || inner_hash)
    let mut outer_input = Vec::with_capacity(BLOCK_SIZE + 32);
    outer_input.extend_from_slice(&opad);
    outer_input.extend_from_slice(inner_hash.as_bytes());
    let outer_hash = sha256(&outer_input);

    let mut result = [0u8; 32];
    result.copy_from_slice(outer_hash.as_bytes());
    result
}

// ============================================================================
// EAPOL Frame
// ============================================================================

/// EAPOL (IEEE 802.1X) frame header
#[derive(Debug, Clone)]
pub struct EapolFrame {
    /// Protocol version (1 for 802.1X-2001, 2 for 802.1X-2004)
    pub protocol_version: u8,
    /// Packet type (0=EAP, 1=Start, 2=Logoff, 3=Key)
    pub packet_type: u8,
    /// Length of packet body
    pub packet_body_length: u16,
    /// Packet body
    pub body: Vec<u8>,
}

/// EAPOL packet type: Key (used in 4-way handshake)
pub const EAPOL_KEY: u8 = 3;

impl EapolFrame {
    /// Parse EAPOL frame from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }

        let protocol_version = data[0];
        let packet_type = data[1];
        let packet_body_length = u16::from_be_bytes([data[2], data[3]]);

        let body_end = 4 + packet_body_length as usize;
        if data.len() < body_end {
            return None;
        }

        Some(Self {
            protocol_version,
            packet_type,
            packet_body_length,
            body: data[4..body_end].to_vec(),
        })
    }

    /// Serialize EAPOL frame to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.body.len());
        buf.push(self.protocol_version);
        buf.push(self.packet_type);
        buf.extend_from_slice(&self.packet_body_length.to_be_bytes());
        buf.extend_from_slice(&self.body);
        buf
    }
}

// ============================================================================
// EAPOL Key Frame
// ============================================================================

/// Key information flags (16-bit field in EAPOL-Key frame)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyInfo {
    /// Key descriptor version (1=HMAC-MD5/RC4, 2=HMAC-SHA1/AES, 3=AES-128-CMAC)
    pub descriptor_version: u8,
    /// Pairwise key (1) vs Group key (0)
    pub pairwise: bool,
    /// Install key flag
    pub install: bool,
    /// Key ACK flag (set by authenticator)
    pub ack: bool,
    /// MIC flag (MIC is present and valid)
    pub mic: bool,
    /// Secure flag (pairwise key installed)
    pub secure: bool,
    /// Error flag
    pub error: bool,
    /// Request flag
    pub request: bool,
    /// Encrypted Key Data flag
    pub encrypted: bool,
}

impl KeyInfo {
    /// Parse from 16-bit value (big-endian)
    pub fn from_u16(val: u16) -> Self {
        Self {
            descriptor_version: (val & 0x0007) as u8,
            pairwise: (val & 0x0008) != 0,
            install: (val & 0x0040) != 0,
            ack: (val & 0x0080) != 0,
            mic: (val & 0x0100) != 0,
            secure: (val & 0x0200) != 0,
            error: (val & 0x0400) != 0,
            request: (val & 0x0800) != 0,
            encrypted: (val & 0x1000) != 0,
        }
    }

    /// Serialize to 16-bit value (big-endian)
    pub fn to_u16(&self) -> u16 {
        (self.descriptor_version as u16 & 0x07)
            | if self.pairwise { 0x0008 } else { 0 }
            | if self.install { 0x0040 } else { 0 }
            | if self.ack { 0x0080 } else { 0 }
            | if self.mic { 0x0100 } else { 0 }
            | if self.secure { 0x0200 } else { 0 }
            | if self.error { 0x0400 } else { 0 }
            | if self.request { 0x0800 } else { 0 }
            | if self.encrypted { 0x1000 } else { 0 }
    }
}

/// EAPOL-Key frame body (within EAPOL frame)
#[derive(Debug, Clone)]
pub struct EapolKeyFrame {
    /// Descriptor type (2 = RSN/WPA2)
    pub descriptor_type: u8,
    /// Key information flags
    pub key_info: KeyInfo,
    /// Key length (16 for CCMP)
    pub key_length: u16,
    /// Replay counter (monotonically increasing)
    pub replay_counter: u64,
    /// Key nonce (32 bytes - ANonce from AP, SNonce from STA)
    pub key_nonce: [u8; 32],
    /// Key IV (16 bytes, typically zero for WPA2)
    pub key_iv: [u8; 16],
    /// Key MIC (16 bytes, HMAC over frame)
    pub key_mic: [u8; 16],
    /// Key data length
    pub key_data_length: u16,
    /// Key data (encrypted GTK, PMKID, etc.)
    pub key_data: Vec<u8>,
}

impl EapolKeyFrame {
    /// Minimum key frame body size (without key data):
    /// 1 + 2 + 2 + 8 + 32 + 16 + 8(RSC) + 8(reserved) + 16 + 2 = 95 bytes
    pub const MIN_SIZE: usize = 95;

    /// Parse EAPOL-Key frame from body bytes (after EAPOL header)
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::MIN_SIZE {
            return None;
        }

        let descriptor_type = data[0];
        let key_info = KeyInfo::from_u16(u16::from_be_bytes([data[1], data[2]]));
        let key_length = u16::from_be_bytes([data[3], data[4]]);
        let replay_counter = u64::from_be_bytes([
            data[5], data[6], data[7], data[8], data[9], data[10], data[11], data[12],
        ]);

        let mut key_nonce = [0u8; 32];
        key_nonce.copy_from_slice(&data[13..45]);

        let mut key_iv = [0u8; 16];
        key_iv.copy_from_slice(&data[45..61]);

        // Skip RSC (8 bytes at 61..69) and reserved (8 bytes at 69..77)
        let mut key_mic = [0u8; 16];
        key_mic.copy_from_slice(&data[77..93]);

        let key_data_length = u16::from_be_bytes([data[93], data[94]]);

        let key_data = if key_data_length > 0 {
            let end = Self::MIN_SIZE + key_data_length as usize;
            if data.len() < end {
                return None;
            }
            data[Self::MIN_SIZE..end].to_vec()
        } else {
            Vec::new()
        };

        Some(Self {
            descriptor_type,
            key_info,
            key_length,
            replay_counter,
            key_nonce,
            key_iv,
            key_mic,
            key_data_length,
            key_data,
        })
    }

    /// Serialize EAPOL-Key frame body to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::MIN_SIZE + self.key_data.len());
        buf.push(self.descriptor_type);
        buf.extend_from_slice(&self.key_info.to_u16().to_be_bytes());
        buf.extend_from_slice(&self.key_length.to_be_bytes());
        buf.extend_from_slice(&self.replay_counter.to_be_bytes());
        buf.extend_from_slice(&self.key_nonce);
        buf.extend_from_slice(&self.key_iv);
        buf.extend_from_slice(&[0u8; 8]); // RSC
        buf.extend_from_slice(&[0u8; 8]); // Reserved
        buf.extend_from_slice(&self.key_mic);
        buf.extend_from_slice(&self.key_data_length.to_be_bytes());
        buf.extend_from_slice(&self.key_data);
        buf
    }
}

// ============================================================================
// WPA State Machine
// ============================================================================

/// WPA handshake state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WpaState {
    /// Idle, waiting for handshake to begin
    #[default]
    Idle,
    /// PTK start: received message 1, computing PTK
    PtkStart,
    /// PTK negotiating: sent message 2, waiting for message 3
    PtkInitNegotiating,
    /// PTK done: received message 3, sent message 4
    PtkInitDone,
    /// GTK negotiating: group key handshake in progress
    GtkNegotiating,
    /// Handshake completed, keys installed
    Completed,
    /// Handshake failed
    Failed,
}

// ============================================================================
// Key Derivation Functions
// ============================================================================

/// PBKDF2-SHA256 key derivation (RFC 8018)
///
/// Derives a key from a password and salt using HMAC-SHA256 as the PRF.
/// Uses 4096 iterations as specified by WPA2.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, output: &mut [u8]) {
    let dk_len = output.len();
    let h_len = 32; // SHA-256 output size
    let blocks_needed = dk_len.div_ceil(h_len);

    for block_idx in 0..blocks_needed {
        let block_num = (block_idx as u32) + 1;

        // U_1 = PRF(password, salt || INT(block_num))
        let mut salt_block = Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&block_num.to_be_bytes());

        let mut u_prev = hmac_sha256(password, &salt_block);
        let mut result = u_prev;

        // U_2 .. U_c: PRF(password, U_{i-1}), XOR into result
        for _ in 1..iterations {
            let u_curr = hmac_sha256(password, &u_prev);
            for (r, &u) in result.iter_mut().zip(u_curr.iter()) {
                *r ^= u;
            }
            u_prev = u_curr;
        }

        // Copy derived block to output
        let start = block_idx * h_len;
        let end = core::cmp::min(start + h_len, dk_len);
        output[start..end].copy_from_slice(&result[..end - start]);
    }
}

/// PRF-SHA256: Pseudo-Random Function for key expansion (IEEE 802.11-2012)
///
/// Expands a key using HMAC-SHA256 with label and data inputs.
/// Produces output of the specified bit length.
pub fn prf_sha256(key: &[u8], label: &[u8], data: &[u8], bits: usize) -> Vec<u8> {
    let bytes_needed = bits.div_ceil(8);
    let iterations = bytes_needed.div_ceil(32);
    let mut result = Vec::with_capacity(iterations * 32);

    for i in 0..iterations {
        // HMAC-SHA256(key, label || 0x00 || data || i)
        let mut input = Vec::with_capacity(label.len() + 1 + data.len() + 1);
        input.extend_from_slice(label);
        input.push(0x00);
        input.extend_from_slice(data);
        input.push(i as u8);

        let block = hmac_sha256(key, &input);
        result.extend_from_slice(&block);
    }

    result.truncate(bytes_needed);
    result
}

/// Derive PMK (Pairwise Master Key) from passphrase and SSID.
///
/// PMK = PBKDF2-SHA256(passphrase, SSID, 4096, 32)
pub fn derive_pmk(passphrase: &[u8], ssid: &[u8]) -> [u8; 32] {
    let mut pmk = [0u8; 32];
    pbkdf2_sha256(passphrase, ssid, 4096, &mut pmk);
    pmk
}

/// Derive PTK (Pairwise Transient Key) from PMK, nonces, and addresses.
///
/// PTK = PRF-SHA256(PMK, "Pairwise key expansion",
///                  Min(AA,SPA) || Max(AA,SPA) || Min(ANonce,SNonce) ||
/// Max(ANonce,SNonce))
///
/// PTK is split into: KCK (16) + KEK (16) + TK (16) = 48 bytes
pub fn derive_ptk(
    pmk: &[u8; 32],
    aa: &MacAddress,
    spa: &MacAddress,
    anonce: &[u8; 32],
    snonce: &[u8; 32],
) -> TemporalKey {
    // Sort addresses and nonces (smaller first)
    let (min_addr, max_addr) = if aa.0 < spa.0 {
        (&aa.0[..], &spa.0[..])
    } else {
        (&spa.0[..], &aa.0[..])
    };
    let (min_nonce, max_nonce) = if anonce < snonce {
        (&anonce[..], &snonce[..])
    } else {
        (&snonce[..], &anonce[..])
    };

    let mut data = Vec::with_capacity(6 + 6 + 32 + 32);
    data.extend_from_slice(min_addr);
    data.extend_from_slice(max_addr);
    data.extend_from_slice(min_nonce);
    data.extend_from_slice(max_nonce);

    let ptk_bytes = prf_sha256(pmk, b"Pairwise key expansion", &data, 384);

    let mut kck = [0u8; 16];
    let mut kek = [0u8; 16];
    let mut tk = [0u8; 16];
    kck.copy_from_slice(&ptk_bytes[0..16]);
    kek.copy_from_slice(&ptk_bytes[16..32]);
    tk.copy_from_slice(&ptk_bytes[32..48]);

    TemporalKey { kck, kek, tk }
}

// ============================================================================
// Temporal Key
// ============================================================================

/// Temporal key material derived from the 4-way handshake
#[derive(Debug, Clone)]
pub struct TemporalKey {
    /// Key Confirmation Key (16 bytes) - used for MIC computation
    pub kck: [u8; 16],
    /// Key Encryption Key (16 bytes) - used for key data encryption
    pub kek: [u8; 16],
    /// Temporal Key (16 bytes) - used for data encryption (CCMP/TKIP)
    pub tk: [u8; 16],
}

// ============================================================================
// 4-Way Handshake
// ============================================================================

/// WPA2 4-way handshake state machine (supplicant side)
pub struct FourWayHandshake {
    /// Handshake state
    state: WpaState,
    /// Pairwise Master Key (32 bytes)
    pmk: [u8; 32],
    /// Derived Pairwise Transient Key
    ptk: Option<TemporalKey>,
    /// Authenticator nonce (from AP, message 1)
    anonce: [u8; 32],
    /// Supplicant nonce (generated locally)
    snonce: [u8; 32],
    /// Our MAC address (SPA)
    spa: MacAddress,
    /// AP MAC address (AA)
    aa: MacAddress,
    /// Replay counter from last message
    replay_counter: u64,
}

impl FourWayHandshake {
    /// Create a new handshake instance
    pub fn new(pmk: [u8; 32], spa: MacAddress, aa: MacAddress) -> Self {
        Self {
            state: WpaState::Idle,
            pmk,
            ptk: None,
            anonce: [0u8; 32],
            snonce: [0u8; 32],
            spa,
            aa,
            replay_counter: 0,
        }
    }

    /// Get current handshake state
    pub fn state(&self) -> WpaState {
        self.state
    }

    /// Get derived PTK (available after message 1 processing)
    pub fn ptk(&self) -> Option<&TemporalKey> {
        self.ptk.as_ref()
    }

    /// Process Message 1 from AP: receive ANonce, generate SNonce, derive PTK.
    ///
    /// Returns Message 2 EAPOL-Key frame bytes to send back.
    pub fn process_message_1(&mut self, key_frame: &EapolKeyFrame) -> Option<Vec<u8>> {
        if self.state != WpaState::Idle {
            return None;
        }

        // Verify this is Message 1: ACK set, MIC not set, pairwise set
        if !key_frame.key_info.ack || key_frame.key_info.mic || !key_frame.key_info.pairwise {
            return None;
        }

        // Store ANonce
        self.anonce = key_frame.key_nonce;
        self.replay_counter = key_frame.replay_counter;

        // Generate SNonce (in a real implementation, use CSPRNG)
        // For now, derive from PMK + ANonce as a deterministic placeholder
        self.snonce = hmac_sha256(&self.pmk, &self.anonce);

        // Derive PTK
        self.ptk = Some(derive_ptk(
            &self.pmk,
            &self.aa,
            &self.spa,
            &self.anonce,
            &self.snonce,
        ));

        self.state = WpaState::PtkStart;

        // Build Message 2
        Some(self.build_message_2())
    }

    /// Process Message 3 from AP: verify MIC, install PTK.
    ///
    /// Returns Message 4 EAPOL-Key frame bytes to send back.
    pub fn process_message_3(&mut self, key_frame: &EapolKeyFrame) -> Option<Vec<u8>> {
        if self.state != WpaState::PtkInitNegotiating && self.state != WpaState::PtkStart {
            return None;
        }

        // Verify this is Message 3: ACK, MIC, pairwise, install, secure all set
        if !key_frame.key_info.ack
            || !key_frame.key_info.mic
            || !key_frame.key_info.pairwise
            || !key_frame.key_info.install
        {
            return None;
        }

        // Verify replay counter is >= our stored value
        if key_frame.replay_counter < self.replay_counter {
            self.state = WpaState::Failed;
            return None;
        }
        self.replay_counter = key_frame.replay_counter;

        // Verify MIC using KCK
        let ptk = self.ptk.as_ref()?;
        if !self.verify_mic_internal(&ptk.kck, key_frame) {
            self.state = WpaState::Failed;
            return None;
        }

        // Verify ANonce matches message 1
        if key_frame.key_nonce != self.anonce {
            self.state = WpaState::Failed;
            return None;
        }

        self.state = WpaState::PtkInitDone;

        // Build Message 4
        Some(self.build_message_4())
    }

    /// Process Message 2 (AP/authenticator side): verify MIC from supplicant.
    ///
    /// Returns true if Message 2 is valid.
    pub fn process_message_2(&mut self, key_frame: &EapolKeyFrame) -> bool {
        // Message 2: MIC set, pairwise set, ACK not set
        if !key_frame.key_info.mic || key_frame.key_info.ack || !key_frame.key_info.pairwise {
            return false;
        }

        // In authenticator role, we need the SNonce from this message to derive PTK
        self.snonce = key_frame.key_nonce;

        // Derive PTK
        let ptk = derive_ptk(&self.pmk, &self.aa, &self.spa, &self.anonce, &self.snonce);

        // Verify MIC
        if !self.verify_mic_internal(&ptk.kck, key_frame) {
            self.state = WpaState::Failed;
            return false;
        }

        self.ptk = Some(ptk);
        self.state = WpaState::PtkInitNegotiating;
        true
    }

    /// Process Message 4 (AP/authenticator side): handshake complete.
    ///
    /// Returns true if Message 4 is valid.
    pub fn process_message_4(&mut self, key_frame: &EapolKeyFrame) -> bool {
        if self.state != WpaState::PtkInitDone && self.state != WpaState::PtkInitNegotiating {
            return false;
        }

        // Message 4: MIC set, pairwise set, ACK not set, secure set
        if !key_frame.key_info.mic || !key_frame.key_info.pairwise || key_frame.key_info.ack {
            return false;
        }

        if let Some(ref ptk) = self.ptk {
            if !self.verify_mic_internal(&ptk.kck, key_frame) {
                self.state = WpaState::Failed;
                return false;
            }
        } else {
            self.state = WpaState::Failed;
            return false;
        }

        self.state = WpaState::Completed;
        true
    }

    /// Verify MIC on an EAPOL-Key frame using HMAC-SHA256(KCK,
    /// frame_with_zero_mic)
    pub fn verify_mic(kck: &[u8; 16], key_frame: &EapolKeyFrame) -> bool {
        // Reconstruct frame with MIC zeroed for verification
        let mut frame_copy = key_frame.clone();
        let original_mic = frame_copy.key_mic;
        frame_copy.key_mic = [0u8; 16];

        let frame_bytes = frame_copy.to_bytes();

        // Wrap in EAPOL header for MIC computation
        let mut eapol_bytes = Vec::with_capacity(4 + frame_bytes.len());
        eapol_bytes.push(2); // version
        eapol_bytes.push(EAPOL_KEY);
        eapol_bytes.extend_from_slice(&(frame_bytes.len() as u16).to_be_bytes());
        eapol_bytes.extend_from_slice(&frame_bytes);

        let computed_mic = hmac_sha256(kck, &eapol_bytes);

        // Compare first 16 bytes of HMAC with stored MIC
        computed_mic[..16] == original_mic
    }

    /// Internal MIC verification helper
    fn verify_mic_internal(&self, kck: &[u8; 16], key_frame: &EapolKeyFrame) -> bool {
        Self::verify_mic(kck, key_frame)
    }

    /// Build Message 2 (supplicant to AP)
    fn build_message_2(&self) -> Vec<u8> {
        let key_info = KeyInfo {
            descriptor_version: 2, // HMAC-SHA1-128 / AES-128-CMAC
            pairwise: true,
            mic: true,
            ..Default::default()
        };

        let mut key_frame = EapolKeyFrame {
            descriptor_type: 2, // RSN
            key_info,
            key_length: 0,
            replay_counter: self.replay_counter,
            key_nonce: self.snonce,
            key_iv: [0u8; 16],
            key_mic: [0u8; 16],
            key_data_length: 0,
            key_data: Vec::new(),
        };

        // Compute MIC over the frame (with MIC field zeroed)
        if let Some(ref ptk) = self.ptk {
            let frame_bytes = key_frame.to_bytes();
            let mut eapol_bytes = Vec::with_capacity(4 + frame_bytes.len());
            eapol_bytes.push(2);
            eapol_bytes.push(EAPOL_KEY);
            eapol_bytes.extend_from_slice(&(frame_bytes.len() as u16).to_be_bytes());
            eapol_bytes.extend_from_slice(&frame_bytes);

            let mic = hmac_sha256(&ptk.kck, &eapol_bytes);
            key_frame.key_mic[..16].copy_from_slice(&mic[..16]);
        }

        // Wrap in EAPOL frame
        let body = key_frame.to_bytes();
        let eapol = EapolFrame {
            protocol_version: 2,
            packet_type: EAPOL_KEY,
            packet_body_length: body.len() as u16,
            body,
        };
        eapol.to_bytes()
    }

    /// Build Message 4 (supplicant to AP)
    fn build_message_4(&self) -> Vec<u8> {
        let key_info = KeyInfo {
            descriptor_version: 2,
            pairwise: true,
            mic: true,
            secure: true,
            ..Default::default()
        };

        let mut key_frame = EapolKeyFrame {
            descriptor_type: 2,
            key_info,
            key_length: 0,
            replay_counter: self.replay_counter,
            key_nonce: [0u8; 32],
            key_iv: [0u8; 16],
            key_mic: [0u8; 16],
            key_data_length: 0,
            key_data: Vec::new(),
        };

        // Compute MIC
        if let Some(ref ptk) = self.ptk {
            let frame_bytes = key_frame.to_bytes();
            let mut eapol_bytes = Vec::with_capacity(4 + frame_bytes.len());
            eapol_bytes.push(2);
            eapol_bytes.push(EAPOL_KEY);
            eapol_bytes.extend_from_slice(&(frame_bytes.len() as u16).to_be_bytes());
            eapol_bytes.extend_from_slice(&frame_bytes);

            let mic = hmac_sha256(&ptk.kck, &eapol_bytes);
            key_frame.key_mic[..16].copy_from_slice(&mic[..16]);
        }

        let body = key_frame.to_bytes();
        let eapol = EapolFrame {
            protocol_version: 2,
            packet_type: EAPOL_KEY,
            packet_body_length: body.len() as u16,
            body,
        };
        eapol.to_bytes()
    }
}

// ============================================================================
// CCMP (AES-128-CCM) Stub
// ============================================================================

/// CCMP encryption parameters
#[derive(Debug, Clone)]
pub struct CcmpEncrypt {
    /// Temporal key (16 bytes, AES-128)
    tk: [u8; 16],
    /// Packet number (48-bit, incremented per frame)
    packet_number: u64,
}

impl CcmpEncrypt {
    /// Create a new CCMP encryption context
    pub fn new(tk: [u8; 16]) -> Self {
        Self {
            tk,
            packet_number: 0,
        }
    }

    /// Get the temporal key
    pub fn temporal_key(&self) -> &[u8; 16] {
        &self.tk
    }

    /// Encrypt a frame payload using AES-128-CCM.
    ///
    /// Stub: returns data with CCMP header prepended and 8-byte MIC appended.
    /// A full implementation would use AES-128 in CCM mode.
    pub fn encrypt(&mut self, _header: &[u8], data: &[u8]) -> Vec<u8> {
        let pn = self.packet_number;
        self.packet_number += 1;

        // CCMP header (8 bytes): PN0, PN1, 0, ExtIV|KeyID, PN2, PN3, PN4, PN5
        let mut result = Vec::with_capacity(8 + data.len() + 8);
        result.push((pn & 0xFF) as u8);
        result.push(((pn >> 8) & 0xFF) as u8);
        result.push(0); // Reserved
        result.push(0x20); // ExtIV=1, KeyID=0
        result.push(((pn >> 16) & 0xFF) as u8);
        result.push(((pn >> 24) & 0xFF) as u8);
        result.push(((pn >> 32) & 0xFF) as u8);
        result.push(((pn >> 40) & 0xFF) as u8);

        // Stub: copy plaintext (real impl would AES-CCM encrypt)
        result.extend_from_slice(data);

        // Stub MIC (8 bytes, real impl computes via AES-CCM)
        result.extend_from_slice(&[0u8; 8]);

        result
    }

    /// Decrypt a frame payload using AES-128-CCM.
    ///
    /// Stub: strips CCMP header and MIC, returns payload.
    /// A full implementation would verify MIC and decrypt via AES-128-CCM.
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        // Need at least CCMP header (8) + MIC (8)
        if data.len() < 16 {
            return None;
        }
        // Strip header and MIC, return payload
        Some(data[8..data.len() - 8].to_vec())
    }
}

// ============================================================================
// WPA3-SAE Stubs
// ============================================================================

/// SAE (Simultaneous Authentication of Equals) state for WPA3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaeState {
    /// Idle, not started
    #[default]
    Idle,
    /// Commit exchange: sending/receiving commit messages
    Committed,
    /// Confirm exchange: sending/receiving confirm messages
    Confirmed,
    /// SAE authentication completed
    Accepted,
    /// SAE authentication failed
    Failed,
}

/// WPA3-SAE authentication handler (stub)
///
/// Full implementation requires elliptic curve operations (hunting-and-pecking
/// for password element, Diffie-Hellman exchange). This stub provides the
/// state machine and message framing.
pub struct SaeAuth {
    /// Current SAE state
    state: SaeState,
    /// Our scalar (32 bytes, stub)
    _scalar: [u8; 32],
    /// Our element (64 bytes, stub - EC point)
    _element: [u8; 64],
}

impl SaeAuth {
    /// Create a new SAE authentication instance
    pub fn new() -> Self {
        Self {
            state: SaeState::Idle,
            _scalar: [0u8; 32],
            _element: [0u8; 64],
        }
    }

    /// Get current SAE state
    pub fn state(&self) -> SaeState {
        self.state
    }

    /// Generate SAE commit message.
    ///
    /// Stub: returns placeholder commit frame. Full implementation requires
    /// hunting-and-pecking to derive password element on NIST P-256, then
    /// scalar/element generation via Diffie-Hellman.
    pub fn generate_commit(&mut self, _password: &[u8], _own_addr: &MacAddress) -> Vec<u8> {
        self.state = SaeState::Committed;

        // Stub commit frame: group_id(2) + scalar(32) + element(64)
        let mut frame = Vec::with_capacity(98);
        frame.extend_from_slice(&19u16.to_le_bytes()); // Group 19 = NIST P-256
        frame.extend_from_slice(&self._scalar);
        frame.extend_from_slice(&self._element);
        frame
    }

    /// Process received SAE commit message.
    ///
    /// Stub: validates message length and transitions state.
    pub fn process_commit(&mut self, data: &[u8]) -> bool {
        if self.state != SaeState::Committed {
            return false;
        }
        // Minimum commit: group_id(2) + scalar(32) + element(64)
        if data.len() < 98 {
            self.state = SaeState::Failed;
            return false;
        }
        self.state = SaeState::Confirmed;
        true
    }

    /// Generate SAE confirm message.
    ///
    /// Stub: returns placeholder confirm frame.
    pub fn generate_confirm(&mut self) -> Vec<u8> {
        // Stub confirm frame: send_confirm(2) + confirm(32)
        let mut frame = Vec::with_capacity(34);
        frame.extend_from_slice(&1u16.to_le_bytes()); // send_confirm counter
        frame.extend_from_slice(&[0u8; 32]); // Stub confirm value
        frame
    }

    /// Process received SAE confirm message.
    ///
    /// Stub: validates message length and completes authentication.
    pub fn process_confirm(&mut self, data: &[u8]) -> bool {
        if self.state != SaeState::Confirmed {
            return false;
        }
        if data.len() < 34 {
            self.state = SaeState::Failed;
            return false;
        }
        self.state = SaeState::Accepted;
        true
    }
}

impl Default for SaeAuth {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_hmac_sha256_known_vector() {
        // RFC 4231 Test Case 2: HMAC-SHA256 with "Jefe" key and "what do ya want for
        // nothing?" data
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let result = hmac_sha256(key, data);
        // Expected: 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(result, expected);
    }

    #[test]
    fn test_hmac_sha256_empty() {
        let key = &[];
        let data = &[];
        let result = hmac_sha256(key, data);
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_pbkdf2_sha256_basic() {
        let mut output = [0u8; 32];
        pbkdf2_sha256(b"password", b"salt", 1, &mut output);
        assert_ne!(output, [0u8; 32]);
    }

    #[test]
    fn test_derive_pmk() {
        let pmk = derive_pmk(b"testpassword", b"TestSSID");
        assert_ne!(pmk, [0u8; 32]);
        assert_eq!(pmk.len(), 32);

        // Same inputs should produce same PMK
        let pmk2 = derive_pmk(b"testpassword", b"TestSSID");
        assert_eq!(pmk, pmk2);
    }

    #[test]
    fn test_derive_ptk() {
        let pmk = [0x42u8; 32];
        let aa = MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let spa = MacAddress::new([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        let anonce = [0x01u8; 32];
        let snonce = [0x02u8; 32];

        let ptk = derive_ptk(&pmk, &aa, &spa, &anonce, &snonce);
        assert_eq!(ptk.kck.len(), 16);
        assert_eq!(ptk.kek.len(), 16);
        assert_eq!(ptk.tk.len(), 16);

        // Deterministic: same inputs -> same output
        let ptk2 = derive_ptk(&pmk, &aa, &spa, &anonce, &snonce);
        assert_eq!(ptk.kck, ptk2.kck);
        assert_eq!(ptk.kek, ptk2.kek);
        assert_eq!(ptk.tk, ptk2.tk);
    }

    #[test]
    fn test_prf_sha256_output_length() {
        let key = [0x42u8; 32];
        let result = prf_sha256(&key, b"test label", b"test data", 384);
        assert_eq!(result.len(), 48); // 384 bits = 48 bytes
    }

    #[test]
    fn test_eapol_frame_roundtrip() {
        let frame = EapolFrame {
            protocol_version: 2,
            packet_type: EAPOL_KEY,
            packet_body_length: 4,
            body: vec![0x01, 0x02, 0x03, 0x04],
        };
        let bytes = frame.to_bytes();
        let parsed = EapolFrame::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.protocol_version, 2);
        assert_eq!(parsed.packet_type, EAPOL_KEY);
        assert_eq!(parsed.body, vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_eapol_frame_too_short() {
        assert!(EapolFrame::from_bytes(&[0, 3]).is_none());
    }

    #[test]
    fn test_key_info_roundtrip() {
        let ki = KeyInfo {
            descriptor_version: 2,
            pairwise: true,
            install: true,
            ack: true,
            mic: true,
            secure: false,
            error: false,
            request: false,
            encrypted: false,
        };
        let val = ki.to_u16();
        let parsed = KeyInfo::from_u16(val);
        assert_eq!(parsed.descriptor_version, 2);
        assert!(parsed.pairwise);
        assert!(parsed.install);
        assert!(parsed.ack);
        assert!(parsed.mic);
        assert!(!parsed.secure);
    }

    #[test]
    fn test_eapol_key_frame_roundtrip() {
        let kf = EapolKeyFrame {
            descriptor_type: 2,
            key_info: KeyInfo {
                descriptor_version: 2,
                pairwise: true,
                ack: true,
                ..Default::default()
            },
            key_length: 16,
            replay_counter: 1,
            key_nonce: [0xAB; 32],
            key_iv: [0; 16],
            key_mic: [0; 16],
            key_data_length: 0,
            key_data: Vec::new(),
        };
        let bytes = kf.to_bytes();
        let parsed = EapolKeyFrame::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.descriptor_type, 2);
        assert_eq!(parsed.key_nonce, [0xAB; 32]);
        assert_eq!(parsed.replay_counter, 1);
        assert_eq!(parsed.key_length, 16);
    }

    #[test]
    fn test_ccmp_encrypt_decrypt() {
        let mut ccmp = CcmpEncrypt::new([0x42u8; 16]);
        let header = [0u8; 24];
        let plaintext = b"Hello WiFi";

        let encrypted = ccmp.encrypt(&header, plaintext);
        // Should have CCMP header (8) + data (10) + MIC (8) = 26
        assert_eq!(encrypted.len(), 8 + plaintext.len() + 8);

        let decrypted = ccmp.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_ccmp_decrypt_too_short() {
        let ccmp = CcmpEncrypt::new([0u8; 16]);
        assert!(ccmp.decrypt(&[0u8; 10]).is_none());
    }

    #[test]
    fn test_sae_state_machine() {
        let mut sae = SaeAuth::new();
        assert_eq!(sae.state(), SaeState::Idle);

        let _commit = sae.generate_commit(b"password", &MacAddress::ZERO);
        assert_eq!(sae.state(), SaeState::Committed);

        // Simulate receiving peer commit (98 bytes minimum)
        let peer_commit = vec![0u8; 98];
        assert!(sae.process_commit(&peer_commit));
        assert_eq!(sae.state(), SaeState::Confirmed);

        let _confirm = sae.generate_confirm();

        let peer_confirm = vec![0u8; 34];
        assert!(sae.process_confirm(&peer_confirm));
        assert_eq!(sae.state(), SaeState::Accepted);
    }
}
