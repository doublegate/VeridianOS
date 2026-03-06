//! Bluetooth RFCOMM (Serial Port Emulation Protocol)
//!
//! Implements the RFCOMM multiplexer protocol over L2CAP (PSM 0x0003),
//! providing RS-232 serial port emulation. Supports credit-based flow
//! control, FCS computation, modem status commands, and DLCI-based
//! multiplexing for multiple simultaneous serial connections.
//!
//! Reference: Bluetooth Core Specification v5.4, RFCOMM with TS 07.10

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
#[allow(unused_imports)]
use alloc::vec::Vec;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// RFCOMM Frame Types
// ---------------------------------------------------------------------------

/// SABM (Set Asynchronous Balanced Mode) - connection establishment
pub const FRAME_SABM: u8 = 0x2F;

/// UA (Unnumbered Acknowledgement) - acknowledgement
pub const FRAME_UA: u8 = 0x63;

/// DM (Disconnected Mode) - rejection / channel not available
pub const FRAME_DM: u8 = 0x0F;

/// DISC (Disconnect) - disconnect request
pub const FRAME_DISC: u8 = 0x43;

/// UIH (Unnumbered Information with Header check) - data transfer
pub const FRAME_UIH: u8 = 0xEF;

// ---------------------------------------------------------------------------
// RFCOMM Address Field
// ---------------------------------------------------------------------------

/// Maximum DLCI value (5 bits, 0-30 usable; 31 reserved)
pub const MAX_DLCI: u8 = 30;

/// DLCI 0 is the multiplexer control channel
pub const DLCI_CONTROL: u8 = 0;

/// L2CAP PSM for RFCOMM
pub const RFCOMM_PSM: u16 = 0x0003;

/// Default RFCOMM MTU (maximum frame size, N1)
pub const RFCOMM_DEFAULT_MTU: u16 = 127;

/// Maximum RFCOMM MTU
pub const RFCOMM_MAX_MTU: u16 = 32767;

/// Default initial credits for credit-based flow control
pub const DEFAULT_INITIAL_CREDITS: u8 = 7;

/// Encode the RFCOMM address byte from DLCI, C/R bit, and EA bit
///
/// Format: D5 D4 D3 D2 D1 C/R EA
/// - DLCI in bits [7:2]
/// - C/R (Command/Response) in bit 1
/// - EA (Extended Address) in bit 0 (always 1 for single-byte)
pub const fn encode_address(dlci: u8, cr: bool, ea: bool) -> u8 {
    ((dlci & 0x3F) << 2) | ((cr as u8) << 1) | (ea as u8)
}

/// Decode the DLCI from an RFCOMM address byte
pub const fn decode_dlci(address: u8) -> u8 {
    (address >> 2) & 0x3F
}

/// Decode the C/R bit from an RFCOMM address byte
pub const fn decode_cr(address: u8) -> bool {
    (address & 0x02) != 0
}

// ---------------------------------------------------------------------------
// FCS (Frame Check Sequence) - CRC-8
// ---------------------------------------------------------------------------

/// CRC-8 lookup table (polynomial 0xE0, reversed)
const FCS_TABLE: [u8; 256] = generate_fcs_table();

/// Generate the FCS lookup table at compile time
const fn generate_fcs_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut crc = i as u8;
        let mut bit = 0;
        while bit < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xE0;
            } else {
                crc >>= 1;
            }
            bit += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
}

/// Calculate FCS over a byte slice
pub fn calculate_fcs(data: &[u8]) -> u8 {
    let mut fcs = 0xFFu8;
    for &byte in data {
        fcs = FCS_TABLE[(fcs ^ byte) as usize];
    }
    // Complement
    0xFF - fcs
}

/// Verify FCS: compute over data + fcs byte, result should be 0xCF
pub fn verify_fcs(data: &[u8], fcs: u8) -> bool {
    let mut crc = 0xFFu8;
    for &byte in data {
        crc = FCS_TABLE[(crc ^ byte) as usize];
    }
    crc = FCS_TABLE[(crc ^ fcs) as usize];
    crc == 0xCF
}

// ---------------------------------------------------------------------------
// RFCOMM Frame
// ---------------------------------------------------------------------------

/// Maximum RFCOMM frame data size for fixed-size buffers
pub const RFCOMM_MAX_FRAME_DATA: usize = 512;

/// An RFCOMM frame
#[derive(Debug, Clone)]
pub struct RfcommFrame {
    /// Address byte (DLCI + C/R + EA)
    pub address: u8,
    /// Control byte (frame type)
    pub control: u8,
    /// Length of the information field
    pub length: u16,
    /// Information field (data payload)
    #[cfg(feature = "alloc")]
    pub data: Vec<u8>,
    #[cfg(not(feature = "alloc"))]
    pub data: [u8; RFCOMM_MAX_FRAME_DATA],
    #[cfg(not(feature = "alloc"))]
    pub data_len: usize,
    /// Frame Check Sequence
    pub fcs: u8,
    /// Credit byte (present in UIH frames when P/F bit set in credit-based
    /// flow)
    pub credits: Option<u8>,
}

impl RfcommFrame {
    /// Create a new RFCOMM frame
    #[cfg(feature = "alloc")]
    pub fn new(dlci: u8, control: u8, data: &[u8]) -> Self {
        let address = encode_address(dlci, true, true);
        // Compute FCS over address + control (for UIH frames)
        // For SABM/UA/DM/DISC, FCS covers address + control + length
        let fcs = if control == FRAME_UIH {
            calculate_fcs(&[address, control])
        } else {
            // For non-UIH, FCS covers address + control
            // (length included for SABM/UA/DM/DISC but spec says addr+ctrl for those too)
            calculate_fcs(&[address, control])
        };
        Self {
            address,
            control,
            length: data.len() as u16,
            data: Vec::from(data),
            fcs,
            credits: None,
        }
    }

    /// Create a SABM frame for connection establishment
    #[cfg(feature = "alloc")]
    pub fn sabm(dlci: u8) -> Self {
        Self::new(dlci, FRAME_SABM, &[])
    }

    /// Create a UA (acknowledgement) frame
    #[cfg(feature = "alloc")]
    pub fn ua(dlci: u8) -> Self {
        Self::new(dlci, FRAME_UA, &[])
    }

    /// Create a DM (disconnected mode) frame
    #[cfg(feature = "alloc")]
    pub fn dm(dlci: u8) -> Self {
        Self::new(dlci, FRAME_DM, &[])
    }

    /// Create a DISC (disconnect) frame
    #[cfg(feature = "alloc")]
    pub fn disc(dlci: u8) -> Self {
        Self::new(dlci, FRAME_DISC, &[])
    }

    /// Create a UIH (data) frame with optional credits
    #[cfg(feature = "alloc")]
    pub fn uih(dlci: u8, data: &[u8], credits: Option<u8>) -> Self {
        let mut frame = Self::new(dlci, FRAME_UIH, data);
        frame.credits = credits;
        frame
    }

    /// Get the DLCI from this frame's address byte
    pub fn dlci(&self) -> u8 {
        decode_dlci(self.address)
    }

    /// Check if this is a UIH (data) frame
    pub fn is_uih(&self) -> bool {
        self.control == FRAME_UIH
    }

    /// Verify the FCS of this frame
    pub fn verify_fcs(&self) -> bool {
        verify_fcs(&[self.address, self.control], self.fcs)
    }

    /// Serialize frame to buffer, returns bytes written
    #[cfg(feature = "alloc")]
    pub fn serialize(&self, buf: &mut [u8]) -> Result<usize, KernelError> {
        // Address(1) + Control(1) + Length(1 or 2) + Data + Credits?(1) + FCS(1)
        let length_bytes = if self.length > 127 { 2 } else { 1 };
        let credit_bytes = if self.credits.is_some() { 1 } else { 0 };
        let total = 1 + 1 + length_bytes + self.data.len() + credit_bytes + 1;

        if buf.len() < total {
            return Err(KernelError::InvalidArgument {
                name: "buffer",
                value: "too small for RFCOMM frame",
            });
        }

        let mut pos = 0;
        buf[pos] = self.address;
        pos += 1;
        buf[pos] = self.control;
        pos += 1;

        // Length field: EA bit in bit 0 (1 = last byte, 0 = continued)
        if self.length <= 127 {
            buf[pos] = ((self.length as u8) << 1) | 0x01; // EA=1
            pos += 1;
        } else {
            buf[pos] = (self.length as u8) << 1; // EA=0
            pos += 1;
            buf[pos] = (self.length >> 7) as u8;
            pos += 1;
        }

        // Credits (for UIH with credit-based flow)
        if let Some(credits) = self.credits {
            buf[pos] = credits;
            pos += 1;
        }

        // Data
        buf[pos..pos + self.data.len()].copy_from_slice(&self.data);
        pos += self.data.len();

        // FCS
        buf[pos] = self.fcs;
        pos += 1;

        Ok(pos)
    }
}

// ---------------------------------------------------------------------------
// Modem Status Command (MSC)
// ---------------------------------------------------------------------------

/// Modem signal bits for MSC (Modem Status Command)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModemSignals {
    /// Data Terminal Ready
    pub dtr: bool,
    /// Request To Send
    pub rts: bool,
    /// Ring Indicator
    pub ri: bool,
    /// Data Carrier Detect
    pub dcd: bool,
}

impl ModemSignals {
    /// Encode modem signals into a V.24 signal byte
    ///
    /// Bit mapping: FC=0, RTC(DTR)=bit2, RTR(RTS)=bit3, IC(RI)=bit6,
    /// DV(DCD)=bit7
    pub fn to_byte(&self) -> u8 {
        let mut val = 0x01u8; // EA bit always set
        if self.dtr {
            val |= 0x04;
        }
        if self.rts {
            val |= 0x08;
        }
        if self.ri {
            val |= 0x40;
        }
        if self.dcd {
            val |= 0x80;
        }
        val
    }

    /// Decode modem signals from a V.24 signal byte
    pub fn from_byte(val: u8) -> Self {
        Self {
            dtr: val & 0x04 != 0,
            rts: val & 0x08 != 0,
            ri: val & 0x40 != 0,
            dcd: val & 0x80 != 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Multiplexer Control Commands
// ---------------------------------------------------------------------------

/// RFCOMM multiplexer control command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MuxCommandType {
    /// Parameter Negotiation (PN) - MTU, priority, flow control
    ParameterNegotiation = 0x20,
    /// Modem Status Command (MSC) - DTR/RTS/RI/DCD signals
    ModemStatusCommand = 0x38,
    /// Remote Port Negotiation (RPN) - baud rate, data bits, etc.
    RemotePortNegotiation = 0x24,
    /// Test Command
    Test = 0x08,
    /// Flow Control On (aggregate)
    FlowControlOn = 0x28,
    /// Flow Control Off (aggregate)
    FlowControlOff = 0x18,
    /// Non-Supported Command Response
    NonSupported = 0x04,
}

impl MuxCommandType {
    /// Parse from raw byte (upper 6 bits, bit 1 = C/R, bit 0 = EA)
    pub fn from_u8(val: u8) -> Option<Self> {
        match val & 0xFC {
            0x20 => Some(Self::ParameterNegotiation),
            0x38 => Some(Self::ModemStatusCommand),
            0x24 => Some(Self::RemotePortNegotiation),
            0x08 => Some(Self::Test),
            0x28 => Some(Self::FlowControlOn),
            0x18 => Some(Self::FlowControlOff),
            0x04 => Some(Self::NonSupported),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Credit-Based Flow Control
// ---------------------------------------------------------------------------

/// Credit-based flow control state for an RFCOMM channel
#[derive(Debug, Clone, Copy)]
pub struct CreditFlowControl {
    /// Credits granted by remote peer (how many frames we can send)
    pub tx_credits: u8,
    /// Credits granted to remote peer (how many frames they can send)
    pub rx_credits: u8,
    /// Initial credits to grant at connection setup
    pub initial_credits: u8,
}

impl Default for CreditFlowControl {
    fn default() -> Self {
        Self {
            tx_credits: DEFAULT_INITIAL_CREDITS,
            rx_credits: DEFAULT_INITIAL_CREDITS,
            initial_credits: DEFAULT_INITIAL_CREDITS,
        }
    }
}

impl CreditFlowControl {
    /// Create with specific initial credits
    pub fn with_initial(initial: u8) -> Self {
        Self {
            tx_credits: initial,
            rx_credits: initial,
            initial_credits: initial,
        }
    }

    /// Grant additional credits to the remote peer
    pub fn grant_credits(&mut self, count: u8) {
        self.rx_credits = self.rx_credits.saturating_add(count);
    }

    /// Consume one TX credit (returns false if no credits available)
    pub fn consume_credit(&mut self) -> bool {
        if self.tx_credits > 0 {
            self.tx_credits -= 1;
            true
        } else {
            false
        }
    }

    /// Add TX credits (received from remote peer)
    pub fn add_tx_credits(&mut self, count: u8) {
        self.tx_credits = self.tx_credits.saturating_add(count);
    }

    /// Check if we can send (have TX credits)
    pub fn can_send(&self) -> bool {
        self.tx_credits > 0
    }

    /// Check if remote peer's credit is low and needs replenishment
    pub fn needs_replenish(&self) -> bool {
        self.rx_credits < 2
    }
}

// ---------------------------------------------------------------------------
// RFCOMM Channel State
// ---------------------------------------------------------------------------

/// RFCOMM channel state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel is closed
    Closed,
    /// SABM sent, waiting for UA
    Opening,
    /// Channel is open and ready for data
    Open,
    /// DISC sent, waiting for UA/DM
    Closing,
}

// ---------------------------------------------------------------------------
// RFCOMM Channel
// ---------------------------------------------------------------------------

/// An RFCOMM data channel (one per DLCI)
#[derive(Debug, Clone)]
pub struct RfcommChannel {
    /// Data Link Connection Identifier (1-30)
    pub dlci: u8,
    /// Current channel state
    pub state: ChannelState,
    /// Credit-based flow control
    pub flow_control: CreditFlowControl,
    /// Negotiated MTU (N1 parameter)
    pub mtu: u16,
    /// Underlying L2CAP channel ID
    pub l2cap_cid: u16,
    /// Modem signals
    pub modem_signals: ModemSignals,
    /// Channel priority (0-63)
    pub priority: u8,
}

impl RfcommChannel {
    /// Create a new RFCOMM channel
    pub fn new(dlci: u8, l2cap_cid: u16) -> Self {
        Self {
            dlci,
            state: ChannelState::Closed,
            flow_control: CreditFlowControl::default(),
            mtu: RFCOMM_DEFAULT_MTU,
            l2cap_cid,
            modem_signals: ModemSignals::default(),
            priority: 0,
        }
    }

    /// Check if channel is open
    pub fn is_open(&self) -> bool {
        self.state == ChannelState::Open
    }
}

// ---------------------------------------------------------------------------
// RFCOMM Multiplexer
// ---------------------------------------------------------------------------

/// RFCOMM multiplexer: manages all channels over a single L2CAP connection
#[cfg(feature = "alloc")]
pub struct RfcommMultiplexer {
    /// Active channels indexed by DLCI
    channels: BTreeMap<u8, RfcommChannel>,
    /// L2CAP channel ID for the underlying L2CAP connection
    l2cap_cid: u16,
    /// Whether the multiplexer session is established (DLCI 0 SABM/UA done)
    session_open: bool,
    /// Whether we are the initiator of the multiplexer session
    is_initiator: bool,
}

#[cfg(feature = "alloc")]
impl Default for RfcommMultiplexer {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(feature = "alloc")]
impl RfcommMultiplexer {
    /// Create a new RFCOMM multiplexer
    pub fn new(l2cap_cid: u16) -> Self {
        Self {
            channels: BTreeMap::new(),
            l2cap_cid,
            session_open: false,
            is_initiator: false,
        }
    }

    /// Get the underlying L2CAP CID
    pub fn l2cap_cid(&self) -> u16 {
        self.l2cap_cid
    }

    /// Check if the multiplexer session is established
    pub fn is_session_open(&self) -> bool {
        self.session_open
    }

    /// Get number of active channels
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Get a channel by DLCI
    pub fn get_channel(&self, dlci: u8) -> Option<&RfcommChannel> {
        self.channels.get(&dlci)
    }

    /// Start multiplexer session (send SABM on DLCI 0)
    ///
    /// Returns the SABM frame to send over L2CAP
    pub fn start_session(&mut self) -> Result<RfcommFrame, KernelError> {
        if self.session_open {
            return Err(KernelError::InvalidState {
                expected: "session closed",
                actual: "session open",
            });
        }
        self.is_initiator = true;
        Ok(RfcommFrame::sabm(DLCI_CONTROL))
    }

    /// Open a data channel on the given DLCI
    ///
    /// Returns the SABM frame to send over L2CAP
    pub fn open_channel(&mut self, dlci: u8) -> Result<RfcommFrame, KernelError> {
        if !self.session_open {
            return Err(KernelError::InvalidState {
                expected: "session open",
                actual: "session closed",
            });
        }
        if dlci == DLCI_CONTROL || dlci > MAX_DLCI {
            return Err(KernelError::InvalidArgument {
                name: "dlci",
                value: "must be 1-30",
            });
        }
        if self.channels.contains_key(&dlci) {
            return Err(KernelError::InvalidArgument {
                name: "dlci",
                value: "channel already exists",
            });
        }

        let mut channel = RfcommChannel::new(dlci, self.l2cap_cid);
        channel.state = ChannelState::Opening;
        self.channels.insert(dlci, channel);

        Ok(RfcommFrame::sabm(dlci))
    }

    /// Close a data channel on the given DLCI
    ///
    /// Returns the DISC frame to send over L2CAP
    pub fn close_channel(&mut self, dlci: u8) -> Result<RfcommFrame, KernelError> {
        let channel = self
            .channels
            .get_mut(&dlci)
            .ok_or(KernelError::InvalidArgument {
                name: "dlci",
                value: "channel not found",
            })?;

        if channel.state != ChannelState::Open {
            return Err(KernelError::InvalidState {
                expected: "Open",
                actual: "not Open",
            });
        }

        channel.state = ChannelState::Closing;
        Ok(RfcommFrame::disc(dlci))
    }

    /// Send data on an open RFCOMM channel
    ///
    /// Returns the UIH frame to send, or error if no credits available
    pub fn send(&mut self, dlci: u8, data: &[u8]) -> Result<RfcommFrame, KernelError> {
        let channel = self
            .channels
            .get_mut(&dlci)
            .ok_or(KernelError::InvalidArgument {
                name: "dlci",
                value: "channel not found",
            })?;

        if channel.state != ChannelState::Open {
            return Err(KernelError::InvalidState {
                expected: "Open",
                actual: "not Open",
            });
        }

        if data.len() > channel.mtu as usize {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "exceeds channel MTU",
            });
        }

        if !channel.flow_control.consume_credit() {
            return Err(KernelError::ResourceExhausted {
                resource: "RFCOMM TX credits",
            });
        }

        // Grant credits back if remote is running low
        let grant = if channel.flow_control.needs_replenish() {
            let credits = channel.flow_control.initial_credits;
            channel.flow_control.grant_credits(credits);
            Some(credits)
        } else {
            None
        };

        Ok(RfcommFrame::uih(dlci, data, grant))
    }

    /// Process a received RFCOMM frame
    ///
    /// Returns an optional response frame to send back
    pub fn receive(&mut self, frame: &RfcommFrame) -> Result<Option<RfcommFrame>, KernelError> {
        let dlci = frame.dlci();

        match frame.control {
            FRAME_SABM => self.handle_sabm(dlci),
            FRAME_UA => self.handle_ua(dlci),
            FRAME_DM => self.handle_dm(dlci),
            FRAME_DISC => self.handle_disc(dlci),
            FRAME_UIH => self.handle_uih(dlci, frame),
            _ => {
                // Unknown frame type: respond with DM
                Ok(Some(RfcommFrame::dm(dlci)))
            }
        }
    }

    /// Handle incoming SABM (connection request)
    fn handle_sabm(&mut self, dlci: u8) -> Result<Option<RfcommFrame>, KernelError> {
        if dlci == DLCI_CONTROL {
            // Multiplexer session establishment
            self.session_open = true;
            Ok(Some(RfcommFrame::ua(DLCI_CONTROL)))
        } else if dlci > MAX_DLCI {
            Ok(Some(RfcommFrame::dm(dlci)))
        } else {
            // Incoming channel request
            let mut channel = RfcommChannel::new(dlci, self.l2cap_cid);
            channel.state = ChannelState::Open;
            self.channels.insert(dlci, channel);
            Ok(Some(RfcommFrame::ua(dlci)))
        }
    }

    /// Handle incoming UA (acknowledgement)
    fn handle_ua(&mut self, dlci: u8) -> Result<Option<RfcommFrame>, KernelError> {
        if dlci == DLCI_CONTROL {
            self.session_open = true;
            Ok(None)
        } else if let Some(channel) = self.channels.get_mut(&dlci) {
            match channel.state {
                ChannelState::Opening => {
                    channel.state = ChannelState::Open;
                }
                ChannelState::Closing => {
                    channel.state = ChannelState::Closed;
                }
                _ => {}
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }

    /// Handle incoming DM (rejected)
    fn handle_dm(&mut self, dlci: u8) -> Result<Option<RfcommFrame>, KernelError> {
        if dlci == DLCI_CONTROL {
            self.session_open = false;
        } else if let Some(channel) = self.channels.get_mut(&dlci) {
            channel.state = ChannelState::Closed;
        }
        Ok(None)
    }

    /// Handle incoming DISC (disconnect)
    fn handle_disc(&mut self, dlci: u8) -> Result<Option<RfcommFrame>, KernelError> {
        if dlci == DLCI_CONTROL {
            // Close entire multiplexer session
            self.session_open = false;
            for channel in self.channels.values_mut() {
                channel.state = ChannelState::Closed;
            }
            Ok(Some(RfcommFrame::ua(DLCI_CONTROL)))
        } else if let Some(channel) = self.channels.get_mut(&dlci) {
            channel.state = ChannelState::Closed;
            Ok(Some(RfcommFrame::ua(dlci)))
        } else {
            Ok(Some(RfcommFrame::dm(dlci)))
        }
    }

    /// Handle incoming UIH (data)
    fn handle_uih(
        &mut self,
        dlci: u8,
        frame: &RfcommFrame,
    ) -> Result<Option<RfcommFrame>, KernelError> {
        if dlci == DLCI_CONTROL {
            // Multiplexer control command
            return self.process_control(frame);
        }

        let channel = self
            .channels
            .get_mut(&dlci)
            .ok_or(KernelError::InvalidArgument {
                name: "dlci",
                value: "channel not found for UIH data",
            })?;

        if channel.state != ChannelState::Open {
            return Err(KernelError::InvalidState {
                expected: "Open",
                actual: "not Open",
            });
        }

        // Process credits from the frame
        if let Some(credits) = frame.credits {
            channel.flow_control.add_tx_credits(credits);
        }

        // Data is available in frame.data for upper-layer consumption
        // (The caller should read frame.data after this returns Ok)

        Ok(None)
    }

    /// Process a multiplexer control command (received on DLCI 0)
    fn process_control(&mut self, frame: &RfcommFrame) -> Result<Option<RfcommFrame>, KernelError> {
        #[cfg(feature = "alloc")]
        {
            if frame.data.is_empty() {
                return Ok(None);
            }

            let cmd_type_byte = frame.data[0];
            let cmd_type = MuxCommandType::from_u8(cmd_type_byte);

            match cmd_type {
                Some(MuxCommandType::ModemStatusCommand) => {
                    // MSC: DLCI(1) + signals(1)
                    if frame.data.len() >= 4 {
                        let msc_dlci = (frame.data[2] >> 2) & 0x3F;
                        let signals = ModemSignals::from_byte(frame.data[3]);
                        if let Some(channel) = self.channels.get_mut(&msc_dlci) {
                            channel.modem_signals = signals;
                        }
                    }
                    // Respond with MSC acknowledgement (same data, C/R flipped)
                    let mut resp_data = frame.data.clone();
                    if !resp_data.is_empty() {
                        resp_data[0] ^= 0x02; // Flip C/R bit
                    }
                    Ok(Some(RfcommFrame::uih(DLCI_CONTROL, &resp_data, None)))
                }
                Some(MuxCommandType::ParameterNegotiation) => {
                    // PN: negotiate MTU and flow control
                    if frame.data.len() >= 10 {
                        let pn_dlci = frame.data[2] & 0x3F;
                        let pn_mtu = u16::from_le_bytes([frame.data[6], frame.data[7]]);

                        if let Some(channel) = self.channels.get_mut(&pn_dlci) {
                            // Accept peer's MTU if within our limits
                            channel.mtu = pn_mtu.min(RFCOMM_MAX_MTU);
                        }
                    }
                    // Echo back PN response
                    Ok(Some(RfcommFrame::uih(DLCI_CONTROL, &frame.data, None)))
                }
                Some(MuxCommandType::Test) => {
                    // Test: echo back
                    Ok(Some(RfcommFrame::uih(DLCI_CONTROL, &frame.data, None)))
                }
                _ => {
                    // Non-supported command response
                    let resp = [
                        MuxCommandType::NonSupported as u8 | 0x03, // type + EA + C/R
                        0x01,                                      // length EA
                        cmd_type_byte,
                    ];
                    Ok(Some(RfcommFrame::uih(DLCI_CONTROL, &resp, None)))
                }
            }
        }
        #[cfg(not(feature = "alloc"))]
        {
            let _ = frame;
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_encode_decode_address() {
        let addr = encode_address(5, true, true);
        assert_eq!(decode_dlci(addr), 5);
        assert!(decode_cr(addr));
    }

    #[test]
    fn test_encode_address_dlci_zero() {
        let addr = encode_address(0, false, true);
        assert_eq!(decode_dlci(addr), 0);
        assert!(!decode_cr(addr));
    }

    #[test]
    fn test_fcs_calculation() {
        let data = [0x03, 0x3F]; // typical address + control
        let fcs = calculate_fcs(&data);
        assert!(verify_fcs(&data, fcs));
    }

    #[test]
    fn test_fcs_verify_bad() {
        let data = [0x03, 0x3F];
        let fcs = calculate_fcs(&data);
        assert!(!verify_fcs(&data, fcs.wrapping_add(1)));
    }

    #[test]
    fn test_modem_signals_roundtrip() {
        let signals = ModemSignals {
            dtr: true,
            rts: true,
            ri: false,
            dcd: true,
        };
        let byte = signals.to_byte();
        let decoded = ModemSignals::from_byte(byte);
        assert_eq!(decoded, signals);
    }

    #[test]
    fn test_credit_flow_control() {
        let mut fc = CreditFlowControl::with_initial(5);
        assert!(fc.can_send());
        assert_eq!(fc.tx_credits, 5);

        for _ in 0..5 {
            assert!(fc.consume_credit());
        }
        assert!(!fc.can_send());
        assert!(!fc.consume_credit());

        fc.add_tx_credits(3);
        assert!(fc.can_send());
        assert_eq!(fc.tx_credits, 3);
    }

    #[test]
    fn test_credit_grant_and_replenish() {
        let mut fc = CreditFlowControl::with_initial(3);
        fc.rx_credits = 1;
        assert!(fc.needs_replenish());
        fc.grant_credits(3);
        assert_eq!(fc.rx_credits, 4);
        assert!(!fc.needs_replenish());
    }

    #[test]
    fn test_mux_command_type_parse() {
        assert_eq!(
            MuxCommandType::from_u8(0x23),
            Some(MuxCommandType::ParameterNegotiation)
        );
        assert_eq!(
            MuxCommandType::from_u8(0x3B),
            Some(MuxCommandType::ModemStatusCommand)
        );
        assert_eq!(MuxCommandType::from_u8(0xFF), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_multiplexer_session_lifecycle() {
        let mut mux = RfcommMultiplexer::new(0x0040);
        assert!(!mux.is_session_open());

        // Start session
        let sabm = mux.start_session().unwrap();
        assert_eq!(sabm.dlci(), DLCI_CONTROL);
        assert_eq!(sabm.control, FRAME_SABM);

        // Simulate UA response
        let ua = RfcommFrame::ua(DLCI_CONTROL);
        mux.receive(&ua).unwrap();
        assert!(mux.is_session_open());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_multiplexer_channel_open_close() {
        let mut mux = RfcommMultiplexer::new(0x0040);
        mux.session_open = true;

        // Open channel
        let sabm = mux.open_channel(5).unwrap();
        assert_eq!(sabm.dlci(), 5);
        assert_eq!(mux.channel_count(), 1);

        // Simulate UA
        let ua = RfcommFrame::ua(5);
        mux.receive(&ua).unwrap();
        assert_eq!(mux.get_channel(5).unwrap().state, ChannelState::Open);

        // Close channel
        let disc = mux.close_channel(5).unwrap();
        assert_eq!(disc.control, FRAME_DISC);

        // Simulate UA for close
        let ua_close = RfcommFrame::ua(5);
        mux.receive(&ua_close).unwrap();
        assert_eq!(mux.get_channel(5).unwrap().state, ChannelState::Closed);
    }
}
