//! Bluetooth Profiles
//!
//! Implements Bluetooth profile support including SDP (Service Discovery
//! Protocol) database, A2DP (Advanced Audio Distribution Profile) with SBC
//! codec configuration, HID (Human Interface Device) report handling, and
//! SPP (Serial Port Profile) over RFCOMM.
//!
//! Reference: Bluetooth Core Specification v5.4, Bluetooth Profile
//! Specifications

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// SDP Attribute Types
// ---------------------------------------------------------------------------

/// SDP Data Element types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdpAttribute {
    /// Nil / null value
    Nil,
    /// Unsigned 8-bit integer
    Uint8(u8),
    /// Unsigned 16-bit integer
    Uint16(u16),
    /// Unsigned 32-bit integer
    Uint32(u32),
    /// Signed 8-bit integer
    Int8(i8),
    /// Signed 16-bit integer
    Int16(i16),
    /// Signed 32-bit integer
    Int32(i32),
    /// 16-bit UUID
    Uuid16(u16),
    /// 128-bit UUID (stored as [u8; 16])
    Uuid128([u8; 16]),
    /// Text string
    #[cfg(feature = "alloc")]
    Text(String),
    /// Boolean value
    Bool(bool),
    /// Sequence of attributes
    #[cfg(feature = "alloc")]
    Sequence(Vec<SdpAttribute>),
}

impl SdpAttribute {
    /// Check if this attribute is nil
    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// Try to extract a u16 value
    pub fn as_u16(&self) -> Option<u16> {
        match self {
            Self::Uint16(v) => Some(*v),
            Self::Uint8(v) => Some(*v as u16),
            _ => None,
        }
    }

    /// Try to extract a UUID16 value
    pub fn as_uuid16(&self) -> Option<u16> {
        match self {
            Self::Uuid16(v) => Some(*v),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Well-known SDP Attribute IDs
// ---------------------------------------------------------------------------

/// ServiceRecordHandle attribute ID
pub const ATTR_SERVICE_RECORD_HANDLE: u16 = 0x0000;

/// ServiceClassIDList attribute ID
pub const ATTR_SERVICE_CLASS_ID_LIST: u16 = 0x0001;

/// ProtocolDescriptorList attribute ID
pub const ATTR_PROTOCOL_DESCRIPTOR_LIST: u16 = 0x0004;

/// BrowseGroupList attribute ID
pub const ATTR_BROWSE_GROUP_LIST: u16 = 0x0005;

/// BluetoothProfileDescriptorList attribute ID
pub const ATTR_PROFILE_DESCRIPTOR_LIST: u16 = 0x0009;

/// ServiceName attribute ID
pub const ATTR_SERVICE_NAME: u16 = 0x0100;

/// ServiceDescription attribute ID
pub const ATTR_SERVICE_DESCRIPTION: u16 = 0x0101;

// ---------------------------------------------------------------------------
// Well-known UUIDs (duplicated from HCI for self-containedness)
// ---------------------------------------------------------------------------

/// L2CAP protocol UUID
pub const UUID_L2CAP: u16 = 0x0100;

/// RFCOMM protocol UUID
pub const UUID_RFCOMM: u16 = 0x0003;

/// SDP UUID
pub const UUID_SDP: u16 = 0x0001;

/// OBEX UUID
pub const UUID_OBEX: u16 = 0x0008;

/// Serial Port Profile UUID
pub const UUID_SERIAL_PORT: u16 = 0x1101;

/// OBEX Object Push UUID
pub const UUID_OBEX_PUSH: u16 = 0x1105;

/// A2DP Source UUID
pub const UUID_A2DP_SOURCE: u16 = 0x110A;

/// A2DP Sink UUID
pub const UUID_A2DP_SINK: u16 = 0x110B;

/// AVRCP Target UUID
pub const UUID_AVRCP_TARGET: u16 = 0x110C;

/// AVRCP Controller UUID
pub const UUID_AVRCP_CONTROLLER: u16 = 0x110E;

/// HFP AG UUID
pub const UUID_HFP_AG: u16 = 0x111F;

/// HFP HF UUID
pub const UUID_HFP_HF: u16 = 0x111E;

/// HID UUID
pub const UUID_HID: u16 = 0x1124;

/// PnP Information UUID
pub const UUID_PNP_INFO: u16 = 0x1200;

/// Public Browse Group UUID
pub const UUID_PUBLIC_BROWSE_GROUP: u16 = 0x1002;

// ---------------------------------------------------------------------------
// SDP Service Record
// ---------------------------------------------------------------------------

/// An SDP service record containing service attributes
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SdpRecord {
    /// Service record handle (unique within the database)
    pub handle: u32,
    /// Primary service class UUID (16-bit short form)
    pub service_class_uuid: u16,
    /// Protocol descriptor list (list of protocol UUIDs + parameters)
    pub protocol_list: Vec<u16>,
    /// Profile descriptor list (profile UUID + version pairs)
    pub profile_list: Vec<(u16, u16)>,
    /// Human-readable service name
    pub service_name: String,
    /// Additional attributes indexed by attribute ID
    pub attributes: BTreeMap<u16, SdpAttribute>,
}

#[cfg(feature = "alloc")]
impl SdpRecord {
    /// Create a new SDP record with the given handle and service class
    pub fn new(handle: u32, service_class_uuid: u16, name: &str) -> Self {
        Self {
            handle,
            service_class_uuid,
            protocol_list: Vec::new(),
            profile_list: Vec::new(),
            service_name: String::from(name),
            attributes: BTreeMap::new(),
        }
    }

    /// Add a protocol to the protocol descriptor list
    pub fn add_protocol(&mut self, uuid: u16) {
        self.protocol_list.push(uuid);
    }

    /// Add a profile descriptor (UUID + version)
    pub fn add_profile(&mut self, uuid: u16, version: u16) {
        self.profile_list.push((uuid, version));
    }

    /// Set an attribute on this record
    pub fn set_attribute(&mut self, id: u16, value: SdpAttribute) {
        self.attributes.insert(id, value);
    }

    /// Get an attribute by ID
    pub fn get_attribute(&self, id: u16) -> Option<&SdpAttribute> {
        self.attributes.get(&id)
    }

    /// Check if this record matches a given service class UUID
    pub fn matches_uuid(&self, uuid: u16) -> bool {
        self.service_class_uuid == uuid
    }
}

// ---------------------------------------------------------------------------
// SDP Database
// ---------------------------------------------------------------------------

/// SDP service database
#[cfg(feature = "alloc")]
pub struct SdpDatabase {
    /// Registered service records
    records: Vec<SdpRecord>,
    /// Next available service record handle
    next_handle: u32,
}

#[cfg(feature = "alloc")]
impl Default for SdpDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl SdpDatabase {
    /// Create a new empty SDP database
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            next_handle: 0x00010001, // First user handle
        }
    }

    /// Get number of registered services
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Register a new service record
    ///
    /// Returns the assigned service record handle
    pub fn register_service(&mut self, service_class_uuid: u16, name: &str) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        let record = SdpRecord::new(handle, service_class_uuid, name);
        self.records.push(record);
        handle
    }

    /// Register a fully constructed SDP record
    pub fn register_record(&mut self, mut record: SdpRecord) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        record.handle = handle;
        self.records.push(record);
        handle
    }

    /// Remove a service record by handle
    pub fn remove_service(&mut self, handle: u32) -> bool {
        let initial_len = self.records.len();
        self.records.retain(|r| r.handle != handle);
        self.records.len() < initial_len
    }

    /// Find a service record by service class UUID
    pub fn find_by_uuid(&self, uuid: u16) -> Option<&SdpRecord> {
        self.records.iter().find(|r| r.matches_uuid(uuid))
    }

    /// Search for all records matching a service class UUID
    pub fn search(&self, uuid: u16) -> Vec<&SdpRecord> {
        self.records
            .iter()
            .filter(|r| r.matches_uuid(uuid))
            .collect()
    }

    /// Get a record by handle
    pub fn get_by_handle(&self, handle: u32) -> Option<&SdpRecord> {
        self.records.iter().find(|r| r.handle == handle)
    }

    /// Get a mutable record by handle
    pub fn get_by_handle_mut(&mut self, handle: u32) -> Option<&mut SdpRecord> {
        self.records.iter_mut().find(|r| r.handle == handle)
    }
}

// ---------------------------------------------------------------------------
// A2DP (Advanced Audio Distribution Profile)
// ---------------------------------------------------------------------------

/// A2DP codec types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum A2dpCodec {
    /// SBC (Sub-Band Coding) - mandatory codec
    Sbc = 0x00,
    /// MPEG-1,2 Audio
    Mpeg12 = 0x01,
    /// AAC (MPEG-2,4 AAC)
    Aac = 0x02,
    /// ATRAC
    Atrac = 0x04,
    /// Vendor-specific (e.g., aptX)
    VendorSpecific = 0xFF,
}

impl A2dpCodec {
    /// Parse codec type from byte
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x00 => Some(Self::Sbc),
            0x01 => Some(Self::Mpeg12),
            0x02 => Some(Self::Aac),
            0x04 => Some(Self::Atrac),
            0xFF => Some(Self::VendorSpecific),
            _ => None,
        }
    }
}

/// SBC channel modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SbcChannelMode {
    /// Mono
    Mono = 0x08,
    /// Dual Channel
    DualChannel = 0x04,
    /// Stereo
    Stereo = 0x02,
    /// Joint Stereo
    JointStereo = 0x01,
}

/// SBC allocation method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SbcAllocationMethod {
    /// SNR (Signal-to-Noise Ratio)
    Snr = 0x02,
    /// Loudness
    Loudness = 0x01,
}

/// SBC sample frequencies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SbcSampleFrequency {
    /// 16 kHz
    Freq16000 = 0x80,
    /// 32 kHz
    Freq32000 = 0x40,
    /// 44.1 kHz
    Freq44100 = 0x20,
    /// 48 kHz
    Freq48000 = 0x10,
}

/// SBC codec configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbcConfig {
    /// Number of subbands (4 or 8)
    pub subbands: u8,
    /// Block length (4, 8, 12, or 16)
    pub block_length: u8,
    /// Allocation method
    pub allocation_method: SbcAllocationMethod,
    /// Channel mode
    pub channel_mode: SbcChannelMode,
    /// Sample frequency
    pub sample_frequency: SbcSampleFrequency,
    /// Minimum bitpool value (2-250)
    pub bitpool_min: u8,
    /// Maximum bitpool value (2-250)
    pub bitpool_max: u8,
}

impl Default for SbcConfig {
    fn default() -> Self {
        Self {
            subbands: 8,
            block_length: 16,
            allocation_method: SbcAllocationMethod::Loudness,
            channel_mode: SbcChannelMode::JointStereo,
            sample_frequency: SbcSampleFrequency::Freq44100,
            bitpool_min: 2,
            bitpool_max: 53,
        }
    }
}

impl SbcConfig {
    /// Validate the SBC configuration
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.subbands != 4 && self.subbands != 8 {
            return Err(KernelError::InvalidArgument {
                name: "subbands",
                value: "must be 4 or 8",
            });
        }
        if !matches!(self.block_length, 4 | 8 | 12 | 16) {
            return Err(KernelError::InvalidArgument {
                name: "block_length",
                value: "must be 4, 8, 12, or 16",
            });
        }
        if self.bitpool_min < 2 || self.bitpool_max > 250 {
            return Err(KernelError::InvalidArgument {
                name: "bitpool",
                value: "must be in range 2-250",
            });
        }
        if self.bitpool_min > self.bitpool_max {
            return Err(KernelError::InvalidArgument {
                name: "bitpool_min",
                value: "must not exceed bitpool_max",
            });
        }
        Ok(())
    }

    /// Encode SBC configuration into the 4-byte AVDTP codec information element
    pub fn to_bytes(&self) -> [u8; 4] {
        let mut bytes = [0u8; 4];
        // Byte 0: sample frequency (4 bits) | channel mode (4 bits)
        bytes[0] = (self.sample_frequency as u8) | (self.channel_mode as u8);
        // Byte 1: block length (4 bits) | subbands (2 bits) | allocation (2 bits)
        let bl = match self.block_length {
            4 => 0x80u8,
            8 => 0x40,
            12 => 0x20,
            16 => 0x10,
            _ => 0x10,
        };
        let sb = if self.subbands == 4 { 0x08u8 } else { 0x04 };
        bytes[1] = bl | sb | (self.allocation_method as u8);
        // Byte 2: minimum bitpool
        bytes[2] = self.bitpool_min;
        // Byte 3: maximum bitpool
        bytes[3] = self.bitpool_max;
        bytes
    }
}

/// A2DP stream endpoint
#[derive(Debug, Clone, Copy)]
pub struct A2dpEndpoint {
    /// Codec type
    pub codec: A2dpCodec,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u8,
    /// Minimum bitpool value
    pub bitpool_min: u8,
    /// Maximum bitpool value
    pub bitpool_max: u8,
}

impl Default for A2dpEndpoint {
    fn default() -> Self {
        Self {
            codec: A2dpCodec::Sbc,
            sample_rate: 44100,
            channels: 2,
            bitpool_min: 2,
            bitpool_max: 53,
        }
    }
}

/// A2DP stream state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A2dpState {
    /// Endpoint is idle (not configured)
    Idle,
    /// Endpoint is configured (codec negotiated)
    Configured,
    /// Stream is actively sending/receiving audio
    Streaming,
    /// Stream is temporarily suspended
    Suspended,
}

/// A2DP Sink: receives audio data from a remote source
pub struct A2dpSink {
    /// Endpoint configuration
    pub endpoint: A2dpEndpoint,
    /// SBC codec configuration (when SBC is selected)
    pub sbc_config: SbcConfig,
    /// Current stream state
    pub state: A2dpState,
    /// AVDTP stream handle (SEID)
    pub seid: u8,
    /// L2CAP channel ID for media transport
    pub transport_cid: u16,
}

impl Default for A2dpSink {
    fn default() -> Self {
        Self::new()
    }
}

impl A2dpSink {
    /// Create a new A2DP sink
    pub fn new() -> Self {
        Self {
            endpoint: A2dpEndpoint::default(),
            sbc_config: SbcConfig::default(),
            state: A2dpState::Idle,
            seid: 1,
            transport_cid: 0,
        }
    }

    /// Configure the sink with a specific codec and parameters
    pub fn configure(
        &mut self,
        codec: A2dpCodec,
        sample_rate: u32,
        channels: u8,
    ) -> Result<(), KernelError> {
        if self.state != A2dpState::Idle && self.state != A2dpState::Configured {
            return Err(KernelError::InvalidState {
                expected: "Idle or Configured",
                actual: "Streaming or Suspended",
            });
        }
        if channels == 0 || channels > 2 {
            return Err(KernelError::InvalidArgument {
                name: "channels",
                value: "must be 1 or 2",
            });
        }

        self.endpoint.codec = codec;
        self.endpoint.sample_rate = sample_rate;
        self.endpoint.channels = channels;

        // Update SBC config channel mode based on channel count
        if codec == A2dpCodec::Sbc {
            self.sbc_config.channel_mode = if channels == 1 {
                SbcChannelMode::Mono
            } else {
                SbcChannelMode::JointStereo
            };
            // Map sample rate to SBC frequency
            self.sbc_config.sample_frequency = match sample_rate {
                16000 => SbcSampleFrequency::Freq16000,
                32000 => SbcSampleFrequency::Freq32000,
                44100 => SbcSampleFrequency::Freq44100,
                48000 => SbcSampleFrequency::Freq48000,
                _ => SbcSampleFrequency::Freq44100,
            };
        }

        self.state = A2dpState::Configured;
        Ok(())
    }

    /// Start streaming audio
    pub fn start_stream(&mut self) -> Result<(), KernelError> {
        if self.state != A2dpState::Configured && self.state != A2dpState::Suspended {
            return Err(KernelError::InvalidState {
                expected: "Configured or Suspended",
                actual: "Idle or Streaming",
            });
        }
        self.state = A2dpState::Streaming;
        Ok(())
    }

    /// Suspend the audio stream
    pub fn suspend(&mut self) -> Result<(), KernelError> {
        if self.state != A2dpState::Streaming {
            return Err(KernelError::InvalidState {
                expected: "Streaming",
                actual: "not Streaming",
            });
        }
        self.state = A2dpState::Suspended;
        Ok(())
    }

    /// Close the stream and return to idle
    pub fn close(&mut self) {
        self.state = A2dpState::Idle;
        self.transport_cid = 0;
    }
}

// ---------------------------------------------------------------------------
// HID (Human Interface Device Profile)
// ---------------------------------------------------------------------------

/// HID report types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HidReportType {
    /// Input report (device -> host, e.g., key presses, mouse movements)
    Input = 0x01,
    /// Output report (host -> device, e.g., keyboard LEDs)
    Output = 0x02,
    /// Feature report (bidirectional, device configuration)
    Feature = 0x03,
}

impl HidReportType {
    /// Parse report type from byte
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::Input),
            0x02 => Some(Self::Output),
            0x03 => Some(Self::Feature),
            _ => None,
        }
    }
}

/// Maximum HID report data size
pub const HID_MAX_REPORT_SIZE: usize = 64;

/// A HID report
#[derive(Debug, Clone)]
pub struct HidReport {
    /// Report type
    pub report_type: HidReportType,
    /// Report ID (0 if not used)
    pub report_id: u8,
    /// Report data
    pub data: [u8; HID_MAX_REPORT_SIZE],
    /// Valid data length
    pub data_len: usize,
}

impl HidReport {
    /// Create a new empty HID report
    pub fn new(report_type: HidReportType, report_id: u8) -> Self {
        Self {
            report_type,
            report_id,
            data: [0u8; HID_MAX_REPORT_SIZE],
            data_len: 0,
        }
    }

    /// Create a report with data
    pub fn with_data(report_type: HidReportType, report_id: u8, data: &[u8]) -> Self {
        let copy_len = data.len().min(HID_MAX_REPORT_SIZE);
        let mut report = Self::new(report_type, report_id);
        report.data[..copy_len].copy_from_slice(&data[..copy_len]);
        report.data_len = copy_len;
        report
    }

    /// Get the report data as a slice
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.data_len]
    }
}

/// HID Report Descriptor item (simplified)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidDescriptor {
    /// Usage Page (e.g., 0x01 = Generic Desktop, 0x07 = Keyboard)
    pub usage_page: u16,
    /// Usage (e.g., 0x06 = Keyboard, 0x02 = Mouse)
    pub usage: u16,
    /// Size of each report field in bits
    pub report_size: u8,
    /// Number of fields in the report
    pub report_count: u8,
}

impl HidDescriptor {
    /// Create a keyboard HID descriptor
    pub fn keyboard() -> Self {
        Self {
            usage_page: 0x01, // Generic Desktop
            usage: 0x06,      // Keyboard
            report_size: 8,
            report_count: 6, // 6-key rollover
        }
    }

    /// Create a mouse HID descriptor
    pub fn mouse() -> Self {
        Self {
            usage_page: 0x01, // Generic Desktop
            usage: 0x02,      // Mouse
            report_size: 8,
            report_count: 3, // buttons + X + Y
        }
    }

    /// Create a gamepad HID descriptor
    pub fn gamepad() -> Self {
        Self {
            usage_page: 0x01, // Generic Desktop
            usage: 0x05,      // Game Pad
            report_size: 8,
            report_count: 8, // buttons + axes
        }
    }

    /// Total report size in bytes
    pub fn report_byte_size(&self) -> usize {
        let total_bits = (self.report_size as usize) * (self.report_count as usize);
        total_bits.div_ceil(8)
    }
}

/// HID device state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidState {
    /// Device not connected
    Disconnected,
    /// Device connected, not yet configured
    Connected,
    /// Device ready for reports
    Ready,
    /// Device is suspended
    Suspended,
}

/// A Bluetooth HID device
pub struct HidDevice {
    /// Device descriptor
    pub descriptor: HidDescriptor,
    /// Current device state
    pub state: HidState,
    /// L2CAP control channel CID
    pub control_cid: u16,
    /// L2CAP interrupt channel CID
    pub interrupt_cid: u16,
    /// Last received input report
    pub last_input_report: HidReport,
}

impl Default for HidDevice {
    fn default() -> Self {
        Self::new(HidDescriptor::keyboard())
    }
}

impl HidDevice {
    /// Create a new HID device with the given descriptor
    pub fn new(descriptor: HidDescriptor) -> Self {
        Self {
            descriptor,
            state: HidState::Disconnected,
            control_cid: 0,
            interrupt_cid: 0,
            last_input_report: HidReport::new(HidReportType::Input, 0),
        }
    }

    /// Send an output report to the device (e.g., keyboard LEDs)
    pub fn send_report(&self, report: &HidReport) -> Result<(), KernelError> {
        if self.state != HidState::Ready {
            return Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "not Ready",
            });
        }
        if report.report_type != HidReportType::Output
            && report.report_type != HidReportType::Feature
        {
            return Err(KernelError::InvalidArgument {
                name: "report_type",
                value: "send_report expects Output or Feature",
            });
        }
        // In a real implementation, this would send via L2CAP interrupt channel
        let _ = report;
        Ok(())
    }

    /// Receive an input report from the device
    pub fn receive_report(&mut self, data: &[u8]) -> Result<HidReport, KernelError> {
        if self.state != HidState::Ready {
            return Err(KernelError::InvalidState {
                expected: "Ready",
                actual: "not Ready",
            });
        }
        if data.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "empty report data",
            });
        }

        let report = HidReport::with_data(HidReportType::Input, 0, data);
        self.last_input_report = report.clone();
        Ok(report)
    }

    /// Parse a raw HID report into individual field values
    ///
    /// Returns a list of field values extracted based on
    /// report_size/report_count
    pub fn parse_report(&self, report: &HidReport) -> Result<ParsedHidReport, KernelError> {
        if report.data_len == 0 {
            return Err(KernelError::InvalidArgument {
                name: "report",
                value: "empty report data",
            });
        }

        let report_size = self.descriptor.report_size as usize;
        let report_count = self.descriptor.report_count as usize;
        let mut fields = [0u32; 16]; // Up to 16 fields
        let mut field_count = 0;

        for (i, field) in fields.iter_mut().enumerate().take(report_count.min(16)) {
            let bit_offset = i * report_size;
            let byte_offset = bit_offset / 8;
            let bit_shift = bit_offset % 8;

            if byte_offset >= report.data_len {
                break;
            }

            let mut value = report.data[byte_offset] as u32 >> bit_shift;
            // Handle fields spanning byte boundaries
            if bit_shift + report_size > 8 && byte_offset + 1 < report.data_len {
                value |= (report.data[byte_offset + 1] as u32) << (8 - bit_shift);
            }
            value &= (1u32 << report_size) - 1;
            *field = value;
            field_count += 1;
        }

        Ok(ParsedHidReport {
            fields,
            field_count,
        })
    }

    /// Connect the HID device
    pub fn connect(&mut self, control_cid: u16, interrupt_cid: u16) {
        self.control_cid = control_cid;
        self.interrupt_cid = interrupt_cid;
        self.state = HidState::Connected;
    }

    /// Set the device to ready state
    pub fn set_ready(&mut self) {
        if self.state == HidState::Connected {
            self.state = HidState::Ready;
        }
    }

    /// Disconnect the HID device
    pub fn disconnect(&mut self) {
        self.state = HidState::Disconnected;
        self.control_cid = 0;
        self.interrupt_cid = 0;
    }
}

/// Parsed HID report with extracted field values
#[derive(Debug, Clone)]
pub struct ParsedHidReport {
    /// Extracted field values
    pub fields: [u32; 16],
    /// Number of valid fields
    pub field_count: usize,
}

impl ParsedHidReport {
    /// Get field value at index
    pub fn field(&self, index: usize) -> Option<u32> {
        if index < self.field_count {
            Some(self.fields[index])
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// SPP (Serial Port Profile)
// ---------------------------------------------------------------------------

/// Serial Port Profile: wraps an RFCOMM channel for serial communication
pub struct SerialPortProfile {
    /// RFCOMM DLCI for this serial port
    pub dlci: u8,
    /// Whether the serial port is connected
    pub connected: bool,
    /// Baud rate (informational, not enforced over Bluetooth)
    pub baud_rate: u32,
    /// Data bits (5-8)
    pub data_bits: u8,
    /// Stop bits (1 or 2)
    pub stop_bits: u8,
    /// Parity: 0=none, 1=odd, 2=even
    pub parity: u8,
    /// Read buffer
    pub read_buf: [u8; 256],
    /// Number of valid bytes in read buffer
    pub read_len: usize,
}

impl Default for SerialPortProfile {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SerialPortProfile {
    /// Create a new SPP instance for the given RFCOMM DLCI
    pub fn new(dlci: u8) -> Self {
        Self {
            dlci,
            connected: false,
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
            read_buf: [0u8; 256],
            read_len: 0,
        }
    }

    /// Connect the serial port
    pub fn connect(&mut self) -> Result<(), KernelError> {
        if self.connected {
            return Err(KernelError::InvalidState {
                expected: "disconnected",
                actual: "connected",
            });
        }
        self.connected = true;
        Ok(())
    }

    /// Disconnect the serial port
    pub fn disconnect(&mut self) -> Result<(), KernelError> {
        if !self.connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "disconnected",
            });
        }
        self.connected = false;
        self.read_len = 0;
        Ok(())
    }

    /// Write data to the serial port (to be sent over RFCOMM)
    ///
    /// Returns the data slice that should be sent via RFCOMM
    pub fn write<'a>(&self, data: &'a [u8]) -> Result<&'a [u8], KernelError> {
        if !self.connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "disconnected",
            });
        }
        if data.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "empty write data",
            });
        }
        // Data passes through to RFCOMM layer
        Ok(data)
    }

    /// Buffer received data from RFCOMM
    pub fn receive(&mut self, data: &[u8]) -> Result<usize, KernelError> {
        if !self.connected {
            return Err(KernelError::InvalidState {
                expected: "connected",
                actual: "disconnected",
            });
        }
        let available = self.read_buf.len() - self.read_len;
        let copy_len = data.len().min(available);
        self.read_buf[self.read_len..self.read_len + copy_len].copy_from_slice(&data[..copy_len]);
        self.read_len += copy_len;
        Ok(copy_len)
    }

    /// Read buffered data
    ///
    /// Returns number of bytes read
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let copy_len = buf.len().min(self.read_len);
        buf[..copy_len].copy_from_slice(&self.read_buf[..copy_len]);
        // Shift remaining data forward
        if copy_len < self.read_len {
            let remaining = self.read_len - copy_len;
            // Use a temporary buffer to avoid overlapping copy
            let mut temp = [0u8; 256];
            temp[..remaining].copy_from_slice(&self.read_buf[copy_len..copy_len + remaining]);
            self.read_buf[..remaining].copy_from_slice(&temp[..remaining]);
        }
        self.read_len -= copy_len;
        copy_len
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
    fn test_sdp_attribute_types() {
        assert!(SdpAttribute::Nil.is_nil());
        assert!(!SdpAttribute::Uint8(0).is_nil());
        assert_eq!(SdpAttribute::Uint16(0x1101).as_u16(), Some(0x1101));
        assert_eq!(SdpAttribute::Uuid16(0x0003).as_uuid16(), Some(0x0003));
        assert_eq!(SdpAttribute::Uint32(42).as_u16(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_sdp_database_register_find() {
        let mut db = SdpDatabase::new();
        let h1 = db.register_service(UUID_SERIAL_PORT, "Serial Port");
        let h2 = db.register_service(UUID_A2DP_SINK, "A2DP Sink");
        assert_eq!(db.record_count(), 2);

        let rec = db.find_by_uuid(UUID_SERIAL_PORT).unwrap();
        assert_eq!(rec.handle, h1);
        assert_eq!(rec.service_name, "Serial Port");

        let rec2 = db.find_by_uuid(UUID_A2DP_SINK).unwrap();
        assert_eq!(rec2.handle, h2);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_sdp_database_remove() {
        let mut db = SdpDatabase::new();
        let h = db.register_service(UUID_HID, "HID");
        assert_eq!(db.record_count(), 1);
        assert!(db.remove_service(h));
        assert_eq!(db.record_count(), 0);
        assert!(!db.remove_service(h)); // Already removed
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_sdp_search() {
        let mut db = SdpDatabase::new();
        db.register_service(UUID_SERIAL_PORT, "SPP 1");
        db.register_service(UUID_SERIAL_PORT, "SPP 2");
        db.register_service(UUID_A2DP_SINK, "A2DP");
        let results = db.search(UUID_SERIAL_PORT);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_sbc_config_default_valid() {
        let config = SbcConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_sbc_config_invalid_subbands() {
        let mut config = SbcConfig::default();
        config.subbands = 3;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_sbc_config_to_bytes() {
        let config = SbcConfig::default();
        let bytes = config.to_bytes();
        // Sample freq 44100 (0x20) | Joint Stereo (0x01)
        assert_eq!(bytes[0], 0x21);
        // Block 16 (0x10) | Subbands 8 (0x04) | Loudness (0x01)
        assert_eq!(bytes[1], 0x15);
        assert_eq!(bytes[2], 2); // bitpool_min
        assert_eq!(bytes[3], 53); // bitpool_max
    }

    #[test]
    fn test_a2dp_sink_lifecycle() {
        let mut sink = A2dpSink::new();
        assert_eq!(sink.state, A2dpState::Idle);

        sink.configure(A2dpCodec::Sbc, 44100, 2).unwrap();
        assert_eq!(sink.state, A2dpState::Configured);

        sink.start_stream().unwrap();
        assert_eq!(sink.state, A2dpState::Streaming);

        sink.suspend().unwrap();
        assert_eq!(sink.state, A2dpState::Suspended);

        sink.start_stream().unwrap();
        assert_eq!(sink.state, A2dpState::Streaming);

        sink.close();
        assert_eq!(sink.state, A2dpState::Idle);
    }

    #[test]
    fn test_hid_report_creation() {
        let data = [0x01, 0x02, 0x03];
        let report = HidReport::with_data(HidReportType::Input, 1, &data);
        assert_eq!(report.report_type, HidReportType::Input);
        assert_eq!(report.report_id, 1);
        assert_eq!(report.as_bytes(), &data);
    }

    #[test]
    fn test_hid_descriptor_keyboard() {
        let desc = HidDescriptor::keyboard();
        assert_eq!(desc.usage_page, 0x01);
        assert_eq!(desc.usage, 0x06);
        assert_eq!(desc.report_byte_size(), 6);
    }

    #[test]
    fn test_hid_device_lifecycle() {
        let mut dev = HidDevice::new(HidDescriptor::keyboard());
        assert_eq!(dev.state, HidState::Disconnected);

        dev.connect(0x0011, 0x0013);
        assert_eq!(dev.state, HidState::Connected);

        dev.set_ready();
        assert_eq!(dev.state, HidState::Ready);

        let data = [0x00, 0x00, 0x04, 0x00, 0x00, 0x00]; // 'a' key
        let report = dev.receive_report(&data).unwrap();
        assert_eq!(report.data_len, 6);

        dev.disconnect();
        assert_eq!(dev.state, HidState::Disconnected);
    }

    #[test]
    fn test_hid_parse_report() {
        let dev = HidDevice::new(HidDescriptor {
            usage_page: 0x01,
            usage: 0x02,
            report_size: 8,
            report_count: 3,
        });
        let report = HidReport::with_data(HidReportType::Input, 0, &[0x01, 0x0A, 0xF0]);
        // Device must be ready to use parse_report (no state check in parse)
        let parsed = dev.parse_report(&report).unwrap();
        assert_eq!(parsed.field_count, 3);
        assert_eq!(parsed.field(0), Some(0x01));
        assert_eq!(parsed.field(1), Some(0x0A));
        assert_eq!(parsed.field(2), Some(0xF0));
        assert_eq!(parsed.field(3), None);
    }

    #[test]
    fn test_spp_read_write() {
        let mut spp = SerialPortProfile::new(5);
        spp.connect().unwrap();

        // Write passes through
        let data = [0x48, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
        let result = spp.write(&data).unwrap();
        assert_eq!(result, &data);

        // Receive some data
        let incoming = [0x41, 0x42, 0x43]; // "ABC"
        let received = spp.receive(&incoming).unwrap();
        assert_eq!(received, 3);

        // Read it back
        let mut buf = [0u8; 10];
        let read = spp.read(&mut buf);
        assert_eq!(read, 3);
        assert_eq!(&buf[..3], &incoming);

        spp.disconnect().unwrap();
    }

    #[test]
    fn test_spp_errors() {
        let mut spp = SerialPortProfile::new(1);
        // Cannot write when disconnected
        assert!(spp.write(&[0x01]).is_err());
        // Cannot disconnect when not connected
        assert!(spp.disconnect().is_err());
        // Cannot connect twice
        spp.connect().unwrap();
        assert!(spp.connect().is_err());
    }
}
