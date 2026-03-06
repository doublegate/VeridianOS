//! IEEE 802.11 MAC Layer Implementation
//!
//! Provides frame parsing/construction, BSS scanning, station state machine,
//! and information element handling for WiFi connectivity.

use alloc::vec::Vec;

use crate::net::MacAddress;

// ============================================================================
// Frame Types and Subtypes
// ============================================================================

/// 802.11 frame type (2-bit field in Frame Control)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Management frames (beacons, probes, auth, assoc)
    Management = 0,
    /// Control frames (ACK, RTS, CTS)
    Control = 1,
    /// Data frames (payload transport)
    Data = 2,
}

impl FrameType {
    /// Parse frame type from 2-bit value
    pub fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0x03 {
            0 => Some(Self::Management),
            1 => Some(Self::Control),
            2 => Some(Self::Data),
            _ => None,
        }
    }
}

/// Management frame subtypes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ManagementSubtype {
    AssocReq = 0,
    AssocResp = 1,
    ProbeReq = 4,
    ProbeResp = 5,
    Beacon = 8,
    Disassoc = 10,
    Auth = 11,
    Deauth = 12,
    Action = 13,
}

impl ManagementSubtype {
    /// Parse management subtype from 4-bit value
    pub fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0x0F {
            0 => Some(Self::AssocReq),
            1 => Some(Self::AssocResp),
            4 => Some(Self::ProbeReq),
            5 => Some(Self::ProbeResp),
            8 => Some(Self::Beacon),
            10 => Some(Self::Disassoc),
            11 => Some(Self::Auth),
            12 => Some(Self::Deauth),
            13 => Some(Self::Action),
            _ => None,
        }
    }
}

// ============================================================================
// Frame Control Field
// ============================================================================

/// IEEE 802.11 Frame Control field (16 bits)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameControl {
    /// Protocol version (2 bits, always 0)
    pub protocol_version: u8,
    /// Frame type
    pub frame_type: FrameType,
    /// Frame subtype (4 bits)
    pub subtype: u8,
    /// To Distribution System
    pub to_ds: bool,
    /// From Distribution System
    pub from_ds: bool,
    /// More Fragments flag
    pub more_fragments: bool,
    /// Retry flag
    pub retry: bool,
    /// Power Management flag
    pub power_mgmt: bool,
    /// More Data flag
    pub more_data: bool,
    /// Protected Frame flag (WEP/WPA encryption)
    pub protected_frame: bool,
    /// Order flag (+HTC/Order)
    pub order: bool,
}

impl Default for FrameControl {
    fn default() -> Self {
        Self {
            protocol_version: 0,
            frame_type: FrameType::Management,
            subtype: 0,
            to_ds: false,
            from_ds: false,
            more_fragments: false,
            retry: false,
            power_mgmt: false,
            more_data: false,
            protected_frame: false,
            order: false,
        }
    }
}

impl FrameControl {
    /// Parse Frame Control from 2 bytes (little-endian on air)
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 2 {
            return None;
        }
        let b0 = bytes[0];
        let b1 = bytes[1];

        let protocol_version = b0 & 0x03;
        let type_bits = (b0 >> 2) & 0x03;
        let subtype = (b0 >> 4) & 0x0F;

        let frame_type = FrameType::from_bits(type_bits)?;

        Some(Self {
            protocol_version,
            frame_type,
            subtype,
            to_ds: (b1 & 0x01) != 0,
            from_ds: (b1 & 0x02) != 0,
            more_fragments: (b1 & 0x04) != 0,
            retry: (b1 & 0x08) != 0,
            power_mgmt: (b1 & 0x10) != 0,
            more_data: (b1 & 0x20) != 0,
            protected_frame: (b1 & 0x40) != 0,
            order: (b1 & 0x80) != 0,
        })
    }

    /// Serialize Frame Control to 2 bytes
    pub fn to_bytes(&self) -> [u8; 2] {
        let b0 = (self.protocol_version & 0x03)
            | ((self.frame_type as u8 & 0x03) << 2)
            | ((self.subtype & 0x0F) << 4);
        let b1 = (self.to_ds as u8)
            | ((self.from_ds as u8) << 1)
            | ((self.more_fragments as u8) << 2)
            | ((self.retry as u8) << 3)
            | ((self.power_mgmt as u8) << 4)
            | ((self.more_data as u8) << 5)
            | ((self.protected_frame as u8) << 6)
            | ((self.order as u8) << 7);
        [b0, b1]
    }
}

// ============================================================================
// IEEE 802.11 Header
// ============================================================================

/// IEEE 802.11 MAC header
#[derive(Debug, Clone)]
pub struct Ieee80211Header {
    /// Frame control field
    pub frame_control: FrameControl,
    /// Duration/ID field (microseconds or association ID)
    pub duration_id: u16,
    /// Address 1: Receiver/Destination
    pub addr1: MacAddress,
    /// Address 2: Transmitter/Source
    pub addr2: MacAddress,
    /// Address 3: BSSID or other
    pub addr3: MacAddress,
    /// Sequence control (fragment + sequence number)
    pub sequence_control: u16,
    /// Address 4 (only in WDS/mesh frames: to_ds=1 && from_ds=1)
    pub addr4: Option<MacAddress>,
}

impl Ieee80211Header {
    /// Minimum header size (without addr4): 2+2+6+6+6+2 = 24 bytes
    pub const MIN_SIZE: usize = 24;
    /// Header size with addr4: 24 + 6 = 30 bytes
    pub const WITH_ADDR4_SIZE: usize = 30;

    /// Parse header from raw bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::MIN_SIZE {
            return None;
        }

        let frame_control = FrameControl::from_bytes(&data[0..2])?;
        let duration_id = u16::from_le_bytes([data[2], data[3]]);

        let mut addr1_bytes = [0u8; 6];
        addr1_bytes.copy_from_slice(&data[4..10]);
        let mut addr2_bytes = [0u8; 6];
        addr2_bytes.copy_from_slice(&data[10..16]);
        let mut addr3_bytes = [0u8; 6];
        addr3_bytes.copy_from_slice(&data[16..22]);

        let sequence_control = u16::from_le_bytes([data[22], data[23]]);

        // addr4 present only when both to_ds and from_ds are set
        let addr4 = if frame_control.to_ds && frame_control.from_ds {
            if data.len() < Self::WITH_ADDR4_SIZE {
                return None;
            }
            let mut addr4_bytes = [0u8; 6];
            addr4_bytes.copy_from_slice(&data[24..30]);
            Some(MacAddress::new(addr4_bytes))
        } else {
            None
        };

        Some(Self {
            frame_control,
            duration_id,
            addr1: MacAddress::new(addr1_bytes),
            addr2: MacAddress::new(addr2_bytes),
            addr3: MacAddress::new(addr3_bytes),
            sequence_control,
            addr4,
        })
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::WITH_ADDR4_SIZE);
        let fc = self.frame_control.to_bytes();
        buf.extend_from_slice(&fc);
        buf.extend_from_slice(&self.duration_id.to_le_bytes());
        buf.extend_from_slice(&self.addr1.0);
        buf.extend_from_slice(&self.addr2.0);
        buf.extend_from_slice(&self.addr3.0);
        buf.extend_from_slice(&self.sequence_control.to_le_bytes());
        if let Some(ref a4) = self.addr4 {
            buf.extend_from_slice(&a4.0);
        }
        buf
    }

    /// Get header length in bytes
    pub fn header_len(&self) -> usize {
        if self.addr4.is_some() {
            Self::WITH_ADDR4_SIZE
        } else {
            Self::MIN_SIZE
        }
    }
}

// ============================================================================
// Information Elements
// ============================================================================

/// Information Element IDs used in management frames
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InformationElementId {
    /// SSID (network name, 0-32 bytes)
    Ssid = 0,
    /// Supported rates
    SupportedRates = 1,
    /// DS Parameter Set (channel number)
    DsParameterSet = 3,
    /// RSN (Robust Security Network) - WPA2/WPA3
    Rsn = 48,
}

/// Parsed Information Element
#[derive(Debug, Clone)]
pub struct InformationElement {
    /// Element ID
    pub id: u8,
    /// Element data
    pub data: Vec<u8>,
}

/// Parse all Information Elements from a byte slice
pub fn parse_information_elements(data: &[u8]) -> Vec<InformationElement> {
    let mut elements = Vec::new();
    let mut offset = 0;

    while offset + 2 <= data.len() {
        let id = data[offset];
        let len = data[offset + 1] as usize;
        offset += 2;

        if offset + len > data.len() {
            break;
        }

        elements.push(InformationElement {
            id,
            data: data[offset..offset + len].to_vec(),
        });

        offset += len;
    }

    elements
}

/// Extract SSID from Information Elements
pub fn extract_ssid(ies: &[InformationElement]) -> Option<Vec<u8>> {
    for ie in ies {
        if ie.id == InformationElementId::Ssid as u8 && ie.data.len() <= 32 {
            return Some(ie.data.clone());
        }
    }
    None
}

/// Extract channel from DS Parameter Set IE
pub fn extract_channel(ies: &[InformationElement]) -> Option<u8> {
    for ie in ies {
        if ie.id == InformationElementId::DsParameterSet as u8 && ie.data.len() == 1 {
            return Some(ie.data[0]);
        }
    }
    None
}

/// Check if RSN (WPA2) IE is present
pub fn has_rsn(ies: &[InformationElement]) -> bool {
    ies.iter()
        .any(|ie| ie.id == InformationElementId::Rsn as u8)
}

// ============================================================================
// Security Type
// ============================================================================

/// WiFi security type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityType {
    /// Open (no encryption)
    #[default]
    Open,
    /// WPA2 Personal (PSK)
    Wpa2Psk,
    /// WPA2 Enterprise (802.1X)
    Wpa2Enterprise,
    /// WPA3 Personal (SAE)
    Wpa3Sae,
}

// ============================================================================
// BSS Information
// ============================================================================

/// BSS (Basic Service Set) information from scan results
#[derive(Debug, Clone)]
pub struct BssInfo {
    /// BSSID (AP MAC address)
    pub bssid: MacAddress,
    /// SSID (network name, up to 32 bytes)
    pub ssid: Vec<u8>,
    /// Channel number (1-14 for 2.4GHz, 36-165 for 5GHz)
    pub channel: u8,
    /// Beacon interval in TUs (1 TU = 1024 microseconds)
    pub beacon_interval: u16,
    /// Capability information
    pub capability: u16,
    /// Signal strength in dBm (integer, typically -90 to -20)
    pub signal_strength: i8,
    /// Security type detected from IEs
    pub security_type: SecurityType,
}

// ============================================================================
// Station State Machine
// ============================================================================

/// Station (STA) connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StaState {
    /// Not connected to any BSS
    #[default]
    Disconnected,
    /// Actively scanning for BSSes
    Scanning,
    /// Performing 802.11 authentication
    Authenticating,
    /// Sending association request
    Associating,
    /// Associated but not yet authenticated (WPA)
    Associated,
    /// Fully connected (802.11 + WPA handshake complete)
    Connected,
}

/// Station configuration for connection
#[derive(Debug, Clone)]
pub struct StaConfig {
    /// Target SSID to connect to
    pub ssid: Vec<u8>,
    /// Pre-computed password hash (PMK) for WPA
    pub password_hash: [u8; 32],
    /// Preferred BSSID (if any)
    pub preferred_bssid: Option<MacAddress>,
}

/// WiFi station (client) state machine
pub struct WifiStation {
    /// Current state
    state: StaState,
    /// Our MAC address
    own_addr: MacAddress,
    /// Current BSS we are associated with
    current_bss: Option<BssInfo>,
    /// Scan results from last scan
    scan_results: Vec<BssInfo>,
    /// Connection configuration
    _config: Option<StaConfig>,
    /// Sequence number counter for outgoing frames
    sequence_number: u16,
}

impl WifiStation {
    /// Create a new WiFi station
    pub fn new(own_addr: MacAddress) -> Self {
        Self {
            state: StaState::Disconnected,
            own_addr,
            current_bss: None,
            scan_results: Vec::new(),
            _config: None,
            sequence_number: 0,
        }
    }

    /// Get current station state
    pub fn state(&self) -> StaState {
        self.state
    }

    /// Get current BSS info
    pub fn current_bss(&self) -> Option<&BssInfo> {
        self.current_bss.as_ref()
    }

    /// Get scan results
    pub fn scan_results(&self) -> &[BssInfo] {
        &self.scan_results
    }

    /// Start BSS scanning. Transitions to Scanning state.
    /// Returns a probe request frame to send on each channel.
    pub fn start_scan(&mut self) -> Vec<u8> {
        self.state = StaState::Scanning;
        self.scan_results.clear();
        self.build_probe_request()
    }

    /// Process a received beacon frame and extract BSS information.
    /// Returns the parsed BSS info if the frame is valid.
    pub fn process_beacon(&mut self, frame_data: &[u8]) -> Option<BssInfo> {
        // Beacon frame: header(24) + timestamp(8) + interval(2) + capability(2) + IEs
        let header = Ieee80211Header::from_bytes(frame_data)?;

        if header.frame_control.frame_type != FrameType::Management {
            return None;
        }
        if header.frame_control.subtype != ManagementSubtype::Beacon as u8 {
            return None;
        }

        let hdr_len = header.header_len();
        // Fixed fields: 8 (timestamp) + 2 (interval) + 2 (capability) = 12 bytes
        if frame_data.len() < hdr_len + 12 {
            return None;
        }

        let fixed_start = hdr_len;
        // Skip timestamp (8 bytes)
        let beacon_interval =
            u16::from_le_bytes([frame_data[fixed_start + 8], frame_data[fixed_start + 9]]);
        let capability =
            u16::from_le_bytes([frame_data[fixed_start + 10], frame_data[fixed_start + 11]]);

        let ie_start = fixed_start + 12;
        let ies = parse_information_elements(&frame_data[ie_start..]);

        let ssid = extract_ssid(&ies).unwrap_or_default();
        let channel = extract_channel(&ies).unwrap_or(0);
        let security_type = if has_rsn(&ies) {
            SecurityType::Wpa2Psk
        } else if capability & 0x0010 != 0 {
            // Privacy bit set but no RSN -> legacy WEP/WPA (treat as WPA2 for simplicity)
            SecurityType::Wpa2Psk
        } else {
            SecurityType::Open
        };

        let bss = BssInfo {
            bssid: header.addr2,
            ssid,
            channel,
            beacon_interval,
            capability,
            signal_strength: -50, // Placeholder; real value from PHY RSSI
            security_type,
        };

        // Add to scan results if not already present
        if !self.scan_results.iter().any(|b| b.bssid.0 == bss.bssid.0) {
            self.scan_results.push(bss.clone());
        }

        Some(bss)
    }

    /// Begin 802.11 Open System authentication with the target BSS.
    /// Returns authentication frame bytes to send.
    pub fn authenticate(&mut self, bss: &BssInfo) -> Option<Vec<u8>> {
        if self.state != StaState::Scanning && self.state != StaState::Disconnected {
            return None;
        }

        self.current_bss = Some(bss.clone());
        self.state = StaState::Authenticating;

        Some(self.build_auth_frame(&bss.bssid))
    }

    /// Process authentication response. On success, transition to Associating.
    /// Returns true if authentication succeeded.
    pub fn process_auth_response(&mut self, frame_data: &[u8]) -> bool {
        let header = match Ieee80211Header::from_bytes(frame_data) {
            Some(h) => h,
            None => return false,
        };

        if header.frame_control.frame_type != FrameType::Management {
            return false;
        }
        if header.frame_control.subtype != ManagementSubtype::Auth as u8 {
            return false;
        }

        let hdr_len = header.header_len();
        // Auth frame body: algo(2) + seq(2) + status(2) = 6 bytes minimum
        if frame_data.len() < hdr_len + 6 {
            return false;
        }

        let status = u16::from_le_bytes([frame_data[hdr_len + 4], frame_data[hdr_len + 5]]);

        if status == 0 {
            // Success
            self.state = StaState::Associating;
            true
        } else {
            self.state = StaState::Disconnected;
            false
        }
    }

    /// Send association request to the current BSS.
    /// Returns association request frame bytes.
    pub fn associate(&mut self) -> Option<Vec<u8>> {
        if self.state != StaState::Associating {
            return None;
        }

        let bssid = self.current_bss.as_ref()?.bssid;
        Some(self.build_assoc_request(&bssid))
    }

    /// Process association response. On success, transition to Associated.
    /// Returns true if association succeeded.
    pub fn process_assoc_response(&mut self, frame_data: &[u8]) -> bool {
        let header = match Ieee80211Header::from_bytes(frame_data) {
            Some(h) => h,
            None => return false,
        };

        if header.frame_control.frame_type != FrameType::Management {
            return false;
        }
        if header.frame_control.subtype != ManagementSubtype::AssocResp as u8 {
            return false;
        }

        let hdr_len = header.header_len();
        // Assoc response body: capability(2) + status(2) + AID(2)
        if frame_data.len() < hdr_len + 6 {
            return false;
        }

        let status = u16::from_le_bytes([frame_data[hdr_len + 2], frame_data[hdr_len + 3]]);

        if status == 0 {
            self.state = StaState::Associated;
            true
        } else {
            self.state = StaState::Disconnected;
            false
        }
    }

    /// Deauthenticate from the current BSS.
    /// Returns deauthentication frame bytes.
    pub fn deauthenticate(&mut self, reason: u16) -> Option<Vec<u8>> {
        let bssid = self.current_bss.as_ref()?.bssid;
        let frame = self.build_deauth_frame(&bssid, reason);
        self.state = StaState::Disconnected;
        self.current_bss = None;
        Some(frame)
    }

    /// Mark station as fully connected (after WPA handshake completes)
    pub fn set_connected(&mut self) {
        if self.state == StaState::Associated {
            self.state = StaState::Connected;
        }
    }

    /// Parse a raw 802.11 frame into header + body
    pub fn parse_frame(data: &[u8]) -> Option<(Ieee80211Header, &[u8])> {
        let header = Ieee80211Header::from_bytes(data)?;
        let hdr_len = header.header_len();
        if data.len() >= hdr_len {
            Some((header, &data[hdr_len..]))
        } else {
            None
        }
    }

    /// Get the next sequence number and advance counter
    fn next_sequence_control(&mut self) -> u16 {
        let seq = self.sequence_number;
        self.sequence_number = self.sequence_number.wrapping_add(1) & 0x0FFF;
        seq << 4 // Fragment number = 0
    }

    /// Build a Probe Request frame (broadcast)
    fn build_probe_request(&mut self) -> Vec<u8> {
        let fc = FrameControl {
            frame_type: FrameType::Management,
            subtype: ManagementSubtype::ProbeReq as u8,
            ..Default::default()
        };
        let seq = self.next_sequence_control();
        let header = Ieee80211Header {
            frame_control: fc,
            duration_id: 0,
            addr1: MacAddress::BROADCAST,
            addr2: self.own_addr,
            addr3: MacAddress::BROADCAST,
            sequence_control: seq,
            addr4: None,
        };

        let mut frame = header.to_bytes();

        // SSID IE (wildcard = zero length)
        frame.push(InformationElementId::Ssid as u8);
        frame.push(0);

        // Supported Rates IE (basic set)
        frame.push(InformationElementId::SupportedRates as u8);
        frame.push(4);
        frame.extend_from_slice(&[0x82, 0x84, 0x8B, 0x96]); // 1, 2, 5.5, 11 Mbps

        frame
    }

    /// Build an Authentication frame (Open System, seq 1)
    fn build_auth_frame(&mut self, bssid: &MacAddress) -> Vec<u8> {
        let fc = FrameControl {
            frame_type: FrameType::Management,
            subtype: ManagementSubtype::Auth as u8,
            ..Default::default()
        };
        let seq = self.next_sequence_control();
        let header = Ieee80211Header {
            frame_control: fc,
            duration_id: 0,
            addr1: *bssid,
            addr2: self.own_addr,
            addr3: *bssid,
            sequence_control: seq,
            addr4: None,
        };

        let mut frame = header.to_bytes();

        // Auth algorithm: Open System (0)
        frame.extend_from_slice(&0u16.to_le_bytes());
        // Auth sequence number: 1
        frame.extend_from_slice(&1u16.to_le_bytes());
        // Status code: 0 (reserved, should be 0 in request)
        frame.extend_from_slice(&0u16.to_le_bytes());

        frame
    }

    /// Build an Association Request frame
    fn build_assoc_request(&mut self, bssid: &MacAddress) -> Vec<u8> {
        let fc = FrameControl {
            frame_type: FrameType::Management,
            subtype: ManagementSubtype::AssocReq as u8,
            ..Default::default()
        };
        let seq = self.next_sequence_control();
        let header = Ieee80211Header {
            frame_control: fc,
            duration_id: 0,
            addr1: *bssid,
            addr2: self.own_addr,
            addr3: *bssid,
            sequence_control: seq,
            addr4: None,
        };

        let mut frame = header.to_bytes();

        // Capability info: ESS(bit 0) + short preamble(bit 5)
        let capability: u16 = 0x0021;
        frame.extend_from_slice(&capability.to_le_bytes());
        // Listen interval (in beacon intervals)
        frame.extend_from_slice(&10u16.to_le_bytes());

        // SSID IE
        if let Some(ref bss) = self.current_bss {
            frame.push(InformationElementId::Ssid as u8);
            frame.push(bss.ssid.len() as u8);
            frame.extend_from_slice(&bss.ssid);
        }

        // Supported Rates IE
        frame.push(InformationElementId::SupportedRates as u8);
        frame.push(4);
        frame.extend_from_slice(&[0x82, 0x84, 0x8B, 0x96]);

        frame
    }

    /// Build a Deauthentication frame
    fn build_deauth_frame(&mut self, bssid: &MacAddress, reason: u16) -> Vec<u8> {
        let fc = FrameControl {
            frame_type: FrameType::Management,
            subtype: ManagementSubtype::Deauth as u8,
            ..Default::default()
        };
        let seq = self.next_sequence_control();
        let header = Ieee80211Header {
            frame_control: fc,
            duration_id: 0,
            addr1: *bssid,
            addr2: self.own_addr,
            addr3: *bssid,
            sequence_control: seq,
            addr4: None,
        };

        let mut frame = header.to_bytes();
        frame.extend_from_slice(&reason.to_le_bytes());
        frame
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
    fn test_frame_type_from_bits() {
        assert_eq!(FrameType::from_bits(0), Some(FrameType::Management));
        assert_eq!(FrameType::from_bits(1), Some(FrameType::Control));
        assert_eq!(FrameType::from_bits(2), Some(FrameType::Data));
        assert_eq!(FrameType::from_bits(3), None);
    }

    #[test]
    fn test_management_subtype_from_bits() {
        assert_eq!(
            ManagementSubtype::from_bits(8),
            Some(ManagementSubtype::Beacon)
        );
        assert_eq!(
            ManagementSubtype::from_bits(11),
            Some(ManagementSubtype::Auth)
        );
        assert_eq!(
            ManagementSubtype::from_bits(0),
            Some(ManagementSubtype::AssocReq)
        );
        assert_eq!(ManagementSubtype::from_bits(15), None);
    }

    #[test]
    fn test_frame_control_roundtrip() {
        let fc = FrameControl {
            protocol_version: 0,
            frame_type: FrameType::Management,
            subtype: ManagementSubtype::Beacon as u8,
            to_ds: false,
            from_ds: false,
            retry: true,
            protected_frame: true,
            ..Default::default()
        };
        let bytes = fc.to_bytes();
        let parsed = FrameControl::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.frame_type, FrameType::Management);
        assert_eq!(parsed.subtype, ManagementSubtype::Beacon as u8);
        assert!(parsed.retry);
        assert!(parsed.protected_frame);
        assert!(!parsed.to_ds);
    }

    #[test]
    fn test_frame_control_data_frame() {
        let fc = FrameControl {
            frame_type: FrameType::Data,
            subtype: 0,
            to_ds: true,
            from_ds: false,
            ..Default::default()
        };
        let bytes = fc.to_bytes();
        let parsed = FrameControl::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.frame_type, FrameType::Data);
        assert!(parsed.to_ds);
        assert!(!parsed.from_ds);
    }

    #[test]
    fn test_frame_control_too_short() {
        assert!(FrameControl::from_bytes(&[0x80]).is_none());
        assert!(FrameControl::from_bytes(&[]).is_none());
    }

    #[test]
    fn test_header_roundtrip() {
        let header = Ieee80211Header {
            frame_control: FrameControl {
                frame_type: FrameType::Management,
                subtype: ManagementSubtype::Beacon as u8,
                ..Default::default()
            },
            duration_id: 0x1234,
            addr1: MacAddress::BROADCAST,
            addr2: MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]),
            addr3: MacAddress::new([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]),
            sequence_control: 0x0010,
            addr4: None,
        };

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), Ieee80211Header::MIN_SIZE);

        let parsed = Ieee80211Header::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.duration_id, 0x1234);
        assert_eq!(parsed.addr1.0, MacAddress::BROADCAST.0);
        assert_eq!(parsed.addr2.0, [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(parsed.sequence_control, 0x0010);
        assert!(parsed.addr4.is_none());
    }

    #[test]
    fn test_header_with_addr4() {
        let mut fc = FrameControl::default();
        fc.to_ds = true;
        fc.from_ds = true;

        let header = Ieee80211Header {
            frame_control: fc,
            duration_id: 0,
            addr1: MacAddress::BROADCAST,
            addr2: MacAddress::ZERO,
            addr3: MacAddress::ZERO,
            sequence_control: 0,
            addr4: Some(MacAddress::new([0x01, 0x02, 0x03, 0x04, 0x05, 0x06])),
        };

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), Ieee80211Header::WITH_ADDR4_SIZE);

        let parsed = Ieee80211Header::from_bytes(&bytes).unwrap();
        assert!(parsed.addr4.is_some());
        assert_eq!(
            parsed.addr4.unwrap().0,
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06]
        );
    }

    #[test]
    fn test_header_too_short() {
        let data = [0u8; 20];
        assert!(Ieee80211Header::from_bytes(&data).is_none());
    }

    #[test]
    fn test_parse_information_elements() {
        // SSID IE: id=0, len=4, data="Test"
        // Channel IE: id=3, len=1, data=6
        let data = [0, 4, b'T', b'e', b's', b't', 3, 1, 6];
        let ies = parse_information_elements(&data);
        assert_eq!(ies.len(), 2);
        assert_eq!(ies[0].id, 0);
        assert_eq!(ies[0].data, b"Test");
        assert_eq!(ies[1].id, 3);
        assert_eq!(ies[1].data, &[6]);
    }

    #[test]
    fn test_extract_ssid() {
        let ies = vec![
            InformationElement {
                id: 0,
                data: b"MyNetwork".to_vec(),
            },
            InformationElement {
                id: 3,
                data: vec![11],
            },
        ];
        let ssid = extract_ssid(&ies);
        assert_eq!(ssid, Some(b"MyNetwork".to_vec()));
    }

    #[test]
    fn test_extract_channel() {
        let ies = vec![InformationElement {
            id: 3,
            data: vec![6],
        }];
        assert_eq!(extract_channel(&ies), Some(6));
    }

    #[test]
    fn test_has_rsn() {
        let ies_with_rsn = vec![InformationElement {
            id: 48,
            data: vec![1, 0],
        }];
        assert!(has_rsn(&ies_with_rsn));

        let ies_without = vec![InformationElement {
            id: 0,
            data: vec![],
        }];
        assert!(!has_rsn(&ies_without));
    }

    #[test]
    fn test_wifi_station_initial_state() {
        let sta = WifiStation::new(MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
        assert_eq!(sta.state(), StaState::Disconnected);
        assert!(sta.current_bss().is_none());
        assert!(sta.scan_results().is_empty());
    }

    #[test]
    fn test_wifi_station_start_scan() {
        let mut sta = WifiStation::new(MacAddress::new([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
        let probe = sta.start_scan();
        assert_eq!(sta.state(), StaState::Scanning);
        // Probe request should have at least a header + SSID IE + Rates IE
        assert!(probe.len() >= Ieee80211Header::MIN_SIZE + 2 + 6);
    }

    #[test]
    fn test_security_type_default() {
        assert_eq!(SecurityType::default(), SecurityType::Open);
    }
}
