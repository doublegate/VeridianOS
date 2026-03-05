//! USB Audio Class (UAC) and HDMI Audio drivers
//!
//! Provides USB Audio Class 1.0/2.0 device support and HDMI audio output:
//!
//! ## USB Audio Class (UAC)
//! - UAC 1.0 and 2.0 descriptor parsing (Audio Control, Audio Streaming)
//! - Terminal types: Input Terminal, Output Terminal, Feature Unit, Mixer Unit
//! - Audio format descriptors: PCM, sample rates (8000-192000), bit depths
//!   (16/24/32)
//! - Isochronous endpoint management (adaptive, synchronous, asynchronous)
//! - Sample rate control (SET_CUR/GET_CUR)
//! - Volume/mute control via Feature Unit (dB scaling, 8.8 fixed-point)
//!
//! ## HDMI Audio
//! - HDMI audio infoframe (CEA-861) construction
//! - Channel allocation (2ch stereo, 5.1, 7.1)
//! - Audio Clock Regeneration (N/CTS values)
//! - ELD (EDID-Like Data) parsing for sink capabilities
//! - Integration with GPU driver (HDA codec over HDMI)
//!
//! All arithmetic uses integer/fixed-point math only (no FPU).

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use crate::error::KernelError;

// ============================================================================
// USB Audio Class Constants
// ============================================================================

/// USB Audio device class code
pub const USB_CLASS_AUDIO: u8 = 0x01;

/// Audio subclass codes
pub const USB_SUBCLASS_AUDIO_CONTROL: u8 = 0x01;
pub const USB_SUBCLASS_AUDIO_STREAMING: u8 = 0x02;
pub const USB_SUBCLASS_MIDI_STREAMING: u8 = 0x03;

/// Audio protocol codes
pub const UAC_PROTOCOL_NONE: u8 = 0x00;
pub const UAC_PROTOCOL_IP_VERSION_02_00: u8 = 0x20;

/// Class-specific descriptor types
pub const CS_INTERFACE: u8 = 0x24;
pub const CS_ENDPOINT: u8 = 0x25;

// --- Audio Control Interface descriptor subtypes ---

/// AC interface header
pub const UAC_AC_HEADER: u8 = 0x01;
/// Input terminal descriptor
pub const UAC_INPUT_TERMINAL: u8 = 0x02;
/// Output terminal descriptor
pub const UAC_OUTPUT_TERMINAL: u8 = 0x03;
/// Mixer unit descriptor
pub const UAC_MIXER_UNIT: u8 = 0x04;
/// Selector unit descriptor
pub const UAC_SELECTOR_UNIT: u8 = 0x05;
/// Feature unit descriptor
pub const UAC_FEATURE_UNIT: u8 = 0x06;
/// Processing unit descriptor (UAC 1.0)
pub const UAC_PROCESSING_UNIT: u8 = 0x07;
/// Extension unit descriptor (UAC 1.0)
pub const UAC_EXTENSION_UNIT: u8 = 0x08;
/// Clock source (UAC 2.0)
pub const UAC2_CLOCK_SOURCE: u8 = 0x0A;
/// Clock selector (UAC 2.0)
pub const UAC2_CLOCK_SELECTOR: u8 = 0x0B;
/// Clock multiplier (UAC 2.0)
pub const UAC2_CLOCK_MULTIPLIER: u8 = 0x0C;

// --- Audio Streaming Interface descriptor subtypes ---

/// AS interface general descriptor
pub const UAC_AS_GENERAL: u8 = 0x01;
/// AS format type descriptor
pub const UAC_AS_FORMAT_TYPE: u8 = 0x02;

// --- Terminal types (USB Audio Terminal Types) ---

/// USB streaming terminal (host connection)
pub const UAC_TERMINAL_USB_STREAMING: u16 = 0x0101;
/// Generic speaker output
pub const UAC_TERMINAL_SPEAKER: u16 = 0x0301;
/// Headphones output
pub const UAC_TERMINAL_HEADPHONES: u16 = 0x0302;
/// Desktop speaker
pub const UAC_TERMINAL_DESKTOP_SPEAKER: u16 = 0x0304;
/// Generic microphone input
pub const UAC_TERMINAL_MICROPHONE: u16 = 0x0201;
/// Desktop microphone
pub const UAC_TERMINAL_DESKTOP_MIC: u16 = 0x0202;
/// Headset microphone
pub const UAC_TERMINAL_HEADSET_MIC: u16 = 0x0204;
/// HDMI output terminal
pub const UAC_TERMINAL_HDMI: u16 = 0x0605;
/// S/PDIF digital output
pub const UAC_TERMINAL_SPDIF: u16 = 0x0605;

// --- Audio data format codes ---

/// PCM format (uncompressed)
pub const UAC_FORMAT_PCM: u16 = 0x0001;
/// PCM8 format (8-bit unsigned)
pub const UAC_FORMAT_PCM8: u16 = 0x0002;
/// IEEE 754 float format
pub const UAC_FORMAT_IEEE_FLOAT: u16 = 0x0003;
/// A-Law format
pub const UAC_FORMAT_ALAW: u16 = 0x0004;
/// Mu-Law format
pub const UAC_FORMAT_MULAW: u16 = 0x0005;

// --- USB Audio control request codes ---

pub const UAC_SET_CUR: u8 = 0x01;
pub const UAC_GET_CUR: u8 = 0x81;
pub const UAC_SET_MIN: u8 = 0x02;
pub const UAC_GET_MIN: u8 = 0x82;
pub const UAC_SET_MAX: u8 = 0x03;
pub const UAC_GET_MAX: u8 = 0x83;
pub const UAC_SET_RES: u8 = 0x04;
pub const UAC_GET_RES: u8 = 0x84;

// --- Feature unit control selectors ---

pub const UAC_FU_MUTE_CONTROL: u8 = 0x01;
pub const UAC_FU_VOLUME_CONTROL: u8 = 0x02;
pub const UAC_FU_BASS_CONTROL: u8 = 0x03;
pub const UAC_FU_TREBLE_CONTROL: u8 = 0x04;
pub const UAC_FU_AUTOMATIC_GAIN: u8 = 0x07;

// --- Isochronous endpoint sync types (bmAttributes bits 2-3) ---

/// No synchronization
pub const UAC_EP_SYNC_NONE: u8 = 0x00;
/// Asynchronous: device sets its own clock
pub const UAC_EP_SYNC_ASYNC: u8 = 0x04;
/// Adaptive: device adapts to host clock
pub const UAC_EP_SYNC_ADAPTIVE: u8 = 0x08;
/// Synchronous: device uses SOF synchronization
pub const UAC_EP_SYNC_SYNC: u8 = 0x0C;

/// Maximum number of supported channels per stream
const MAX_CHANNELS: usize = 8;

/// Maximum number of supported sample rates per format
const MAX_SAMPLE_RATES: usize = 16;

/// Maximum number of audio units per device
const MAX_UNITS: usize = 32;

// ============================================================================
// USB Audio Descriptor Types
// ============================================================================

/// UAC version supported by a device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UacVersion {
    /// USB Audio Class 1.0
    Uac10,
    /// USB Audio Class 2.0
    Uac20,
}

/// Isochronous endpoint synchronization type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoSyncType {
    /// No synchronization
    None,
    /// Asynchronous: device provides its own clock
    Asynchronous,
    /// Adaptive: device adapts to host clock
    Adaptive,
    /// Synchronous: locked to SOF
    Synchronous,
}

impl IsoSyncType {
    /// Parse sync type from endpoint bmAttributes bits 2-3
    pub fn from_attributes(attr: u8) -> Self {
        match attr & 0x0C {
            UAC_EP_SYNC_ASYNC => IsoSyncType::Asynchronous,
            UAC_EP_SYNC_ADAPTIVE => IsoSyncType::Adaptive,
            UAC_EP_SYNC_SYNC => IsoSyncType::Synchronous,
            _ => IsoSyncType::None,
        }
    }

    /// Convert to endpoint bmAttributes bits
    pub fn to_attributes(self) -> u8 {
        match self {
            IsoSyncType::None => UAC_EP_SYNC_NONE,
            IsoSyncType::Asynchronous => UAC_EP_SYNC_ASYNC,
            IsoSyncType::Adaptive => UAC_EP_SYNC_ADAPTIVE,
            IsoSyncType::Synchronous => UAC_EP_SYNC_SYNC,
        }
    }
}

/// Audio terminal type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalDirection {
    /// Input terminal (captures audio)
    Input,
    /// Output terminal (renders audio)
    Output,
}

/// Input terminal descriptor
#[derive(Debug, Clone)]
pub struct InputTerminal {
    /// Terminal ID (unique within the audio function)
    pub terminal_id: u8,
    /// Terminal type code (e.g., UAC_TERMINAL_MICROPHONE)
    pub terminal_type: u16,
    /// Associated output terminal ID (0 if none)
    pub assoc_terminal: u8,
    /// Number of logical output channels
    pub nr_channels: u8,
    /// Spatial location of channels (bitmask)
    pub channel_config: u16,
}

/// Output terminal descriptor
#[derive(Debug, Clone)]
pub struct OutputTerminal {
    /// Terminal ID
    pub terminal_id: u8,
    /// Terminal type code (e.g., UAC_TERMINAL_SPEAKER)
    pub terminal_type: u16,
    /// Associated input terminal ID (0 if none)
    pub assoc_terminal: u8,
    /// Source unit/terminal ID that feeds this output
    pub source_id: u8,
}

/// Feature unit descriptor -- provides volume/mute/tone controls
#[derive(Debug, Clone)]
pub struct FeatureUnit {
    /// Unit ID
    pub unit_id: u8,
    /// Source unit/terminal ID
    pub source_id: u8,
    /// Per-channel control bitmask (index 0 = master, 1..=N = channels)
    /// Bit 0: Mute, Bit 1: Volume, Bit 2: Bass, etc.
    pub controls: Vec<u32>,
}

impl FeatureUnit {
    /// Check if mute control is available for a channel (0 = master)
    pub fn has_mute(&self, channel: usize) -> bool {
        self.controls
            .get(channel)
            .is_some_and(|c| c & (1 << 0) != 0)
    }

    /// Check if volume control is available for a channel (0 = master)
    pub fn has_volume(&self, channel: usize) -> bool {
        self.controls
            .get(channel)
            .is_some_and(|c| c & (1 << 1) != 0)
    }
}

/// Mixer unit descriptor
#[derive(Debug, Clone)]
pub struct MixerUnit {
    /// Unit ID
    pub unit_id: u8,
    /// Source IDs feeding this mixer
    pub source_ids: Vec<u8>,
    /// Number of output channels
    pub nr_channels: u8,
    /// Channel configuration bitmask
    pub channel_config: u16,
}

/// Clock source descriptor (UAC 2.0)
#[derive(Debug, Clone, Copy)]
pub struct ClockSource {
    /// Clock source ID
    pub clock_id: u8,
    /// Attributes: bit 0 = external, bit 1 = synced to SOF
    pub attributes: u8,
    /// Associated terminal ID
    pub assoc_terminal: u8,
}

impl ClockSource {
    /// Check if clock is external
    pub fn is_external(&self) -> bool {
        self.attributes & 0x01 != 0
    }

    /// Check if clock is synced to SOF
    pub fn is_sof_synced(&self) -> bool {
        self.attributes & 0x02 != 0
    }
}

/// Audio unit -- variant type for all unit/terminal descriptors
#[derive(Debug, Clone)]
pub enum AudioUnit {
    /// Input terminal
    InputTerminal(InputTerminal),
    /// Output terminal
    OutputTerminal(OutputTerminal),
    /// Feature unit (volume/mute)
    FeatureUnit(FeatureUnit),
    /// Mixer unit
    MixerUnit(MixerUnit),
    /// Clock source (UAC 2.0)
    ClockSource(ClockSource),
}

impl AudioUnit {
    /// Get the unit/terminal ID
    pub fn id(&self) -> u8 {
        match self {
            AudioUnit::InputTerminal(t) => t.terminal_id,
            AudioUnit::OutputTerminal(t) => t.terminal_id,
            AudioUnit::FeatureUnit(u) => u.unit_id,
            AudioUnit::MixerUnit(u) => u.unit_id,
            AudioUnit::ClockSource(c) => c.clock_id,
        }
    }
}

// ============================================================================
// Audio Format Descriptor
// ============================================================================

/// Supported sample rate entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRateRange {
    /// Minimum sample rate in Hz
    pub min: u32,
    /// Maximum sample rate in Hz
    pub max: u32,
}

impl SampleRateRange {
    /// Create a discrete (single-value) sample rate
    pub fn discrete(rate: u32) -> Self {
        Self {
            min: rate,
            max: rate,
        }
    }

    /// Create a continuous range
    pub fn range(min: u32, max: u32) -> Self {
        Self { min, max }
    }

    /// Check if a given rate falls within this range
    pub fn contains(&self, rate: u32) -> bool {
        rate >= self.min && rate <= self.max
    }

    /// Check if this is a discrete (single-value) rate
    pub fn is_discrete(&self) -> bool {
        self.min == self.max
    }
}

/// Audio format type descriptor
#[derive(Debug, Clone)]
pub struct AudioFormatDescriptor {
    /// Format tag (e.g., UAC_FORMAT_PCM)
    pub format_tag: u16,
    /// Number of physical channels
    pub nr_channels: u8,
    /// Number of bytes per audio subframe (sample container)
    pub subframe_size: u8,
    /// Number of significant bits per sample
    pub bit_resolution: u8,
    /// Supported sample rates
    pub sample_rates: Vec<SampleRateRange>,
}

impl AudioFormatDescriptor {
    /// Check if a given sample rate is supported
    pub fn supports_rate(&self, rate: u32) -> bool {
        // If no rates listed, we assume any rate is ok (continuous)
        if self.sample_rates.is_empty() {
            return true;
        }
        self.sample_rates.iter().any(|r| r.contains(rate))
    }

    /// Check if this format is PCM
    pub fn is_pcm(&self) -> bool {
        self.format_tag == UAC_FORMAT_PCM || self.format_tag == UAC_FORMAT_PCM8
    }

    /// Get the frame size in bytes (channels * subframe_size)
    pub fn frame_size(&self) -> u16 {
        self.nr_channels as u16 * self.subframe_size as u16
    }
}

// ============================================================================
// Audio Streaming Interface
// ============================================================================

/// An audio streaming interface alternate setting
#[derive(Debug, Clone)]
pub struct AudioStreamingInterface {
    /// Interface number
    pub interface_num: u8,
    /// Alternate setting number (0 = zero-bandwidth)
    pub alternate_setting: u8,
    /// Terminal ID this stream is linked to
    pub terminal_link: u8,
    /// Format descriptor
    pub format: AudioFormatDescriptor,
    /// Isochronous endpoint address (0x80 | ep_num for IN, ep_num for OUT)
    pub endpoint_address: u8,
    /// Maximum packet size for the isochronous endpoint
    pub max_packet_size: u16,
    /// Synchronization type
    pub sync_type: IsoSyncType,
    /// Sync endpoint address (for async mode feedback)
    pub sync_endpoint: u8,
}

impl AudioStreamingInterface {
    /// Check if this is an input (capture) stream
    pub fn is_input(&self) -> bool {
        self.endpoint_address & 0x80 != 0
    }

    /// Check if this is an output (playback) stream
    pub fn is_output(&self) -> bool {
        self.endpoint_address & 0x80 == 0
    }

    /// Check if this is the zero-bandwidth alternate setting
    pub fn is_zero_bandwidth(&self) -> bool {
        self.alternate_setting == 0
    }

    /// Calculate bytes per frame at a given sample rate
    /// Returns bytes per USB frame (1ms at full speed, 125us at high speed)
    pub fn bytes_per_usb_frame(&self, sample_rate: u32) -> u32 {
        // For full-speed: 1 frame = 1ms, so bytes = sample_rate/1000 * frame_size
        let frame_size = self.format.frame_size() as u32;
        // Use integer division: (sample_rate * frame_size) / 1000
        sample_rate.saturating_mul(frame_size) / 1000
    }
}

// ============================================================================
// Volume Control (8.8 Fixed-Point dB)
// ============================================================================

/// Volume in 8.8 fixed-point dB format (USB Audio spec)
///
/// The USB Audio Class represents volume as a signed 16-bit value in
/// 1/256 dB steps. The integer part is the upper 8 bits (signed),
/// the fractional part is the lower 8 bits.
///
/// Examples:
/// - 0x0000 = 0.0 dB (unity gain)
/// - 0x0100 = +1.0 dB
/// - 0xFF00 = -1.0 dB (two's complement)
/// - 0x8000 = -128.0 dB (silence)
/// - 0x7FFF = +127.99609375 dB (max)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VolumeDb(pub i16);

/// Silence volume (minimum, -128.0 dB)
pub const VOLUME_SILENCE: VolumeDb = VolumeDb(i16::MIN);

/// Unity gain (0.0 dB)
pub const VOLUME_UNITY: VolumeDb = VolumeDb(0);

/// Maximum volume (+127.99 dB)
pub const VOLUME_MAX: VolumeDb = VolumeDb(i16::MAX);

impl VolumeDb {
    /// Create a volume from integer dB value
    pub fn from_db(db: i8) -> Self {
        VolumeDb((db as i16) << 8)
    }

    /// Get the integer part of the dB value
    pub fn integer_db(&self) -> i8 {
        (self.0 >> 8) as i8
    }

    /// Get the fractional part (0-255, representing 0/256 to 255/256 dB)
    pub fn fraction(&self) -> u8 {
        self.0 as u8
    }

    /// Get the raw 8.8 fixed-point value
    pub fn raw(&self) -> i16 {
        self.0
    }

    /// Convert to a linear gain multiplier (0..65535 range, 16-bit unsigned)
    ///
    /// Uses a piecewise integer approximation of 10^(dB/20):
    /// - Maps -128 dB -> 0 (silence)
    /// - Maps 0 dB -> 65535 (unity)
    /// - Maps positive dB values to saturated 65535
    ///
    /// This is an approximation suitable for kernel audio mixing.
    pub fn to_linear_u16(&self) -> u16 {
        let db_int = self.integer_db();

        if db_int >= 0 {
            // Positive dB: clamp to max
            return 65535;
        }

        if db_int <= -96 {
            // Below -96 dB is effectively silence
            return 0;
        }

        // Piecewise linear approximation for -96..0 dB range
        // Map -96 dB -> 0, 0 dB -> 65535
        // linear_gain = 65535 * 10^(dB/20)
        // Approximation: use a lookup table for every 6 dB step
        // Every -6 dB halves the amplitude
        let abs_db = (-db_int) as u32;
        let steps_6db = abs_db / 6;
        let remainder = abs_db % 6;

        // Start from 65535 and halve for each 6 dB step
        let mut gain: u32 = 65535;
        let mut i = 0u32;
        while i < steps_6db && i < 16 {
            gain /= 2;
            i += 1;
        }

        // Interpolate the remainder (linear approx within 6 dB)
        // For each dB below a 6 dB boundary, scale by ~(6-remainder)/6
        if remainder > 0 {
            gain = gain.saturating_mul(6u32.saturating_sub(remainder)) / 6;
        }

        if gain > 65535 {
            65535
        } else {
            gain as u16
        }
    }

    /// Create from a linear 0..65535 volume value
    ///
    /// Inverse of `to_linear_u16()`, approximate.
    pub fn from_linear_u16(linear: u16) -> Self {
        if linear == 0 {
            return VOLUME_SILENCE;
        }
        if linear == u16::MAX {
            return VOLUME_UNITY;
        }

        // Count how many times we can double to reach 65535
        // Each doubling is ~+6 dB
        let mut val = linear as u32;
        let mut db: i32 = 0;
        while val < 65535 && db > -96 {
            val = val.saturating_mul(2);
            db -= 6;
        }

        VolumeDb::from_db(db as i8)
    }
}

// ============================================================================
// USB Audio Device State
// ============================================================================

/// A parsed USB Audio device with all its audio topology
#[derive(Debug, Clone)]
pub struct UsbAudioDevice {
    /// USB device address
    pub device_address: u8,
    /// UAC version detected
    pub version: UacVersion,
    /// Audio Control interface number
    pub control_interface: u8,
    /// All audio units/terminals in the topology
    pub units: Vec<AudioUnit>,
    /// All audio streaming interface alternate settings
    pub streaming_interfaces: Vec<AudioStreamingInterface>,
    /// Currently active streaming interface (interface_num, alternate_setting)
    pub active_stream: Option<(u8, u8)>,
    /// Current sample rate in Hz
    pub current_sample_rate: u32,
    /// Device name (from USB string descriptor)
    pub name: String,
}

impl UsbAudioDevice {
    /// Create a new USB audio device
    pub fn new(device_address: u8, version: UacVersion) -> Self {
        Self {
            device_address,
            version,
            control_interface: 0,
            units: Vec::new(),
            streaming_interfaces: Vec::new(),
            active_stream: None,
            current_sample_rate: 0,
            name: String::new(),
        }
    }

    /// Find an input terminal by ID
    pub fn find_input_terminal(&self, id: u8) -> Option<&InputTerminal> {
        self.units.iter().find_map(|u| match u {
            AudioUnit::InputTerminal(t) if t.terminal_id == id => Some(t),
            _ => None,
        })
    }

    /// Find an output terminal by ID
    pub fn find_output_terminal(&self, id: u8) -> Option<&OutputTerminal> {
        self.units.iter().find_map(|u| match u {
            AudioUnit::OutputTerminal(t) if t.terminal_id == id => Some(t),
            _ => None,
        })
    }

    /// Find a feature unit by ID
    pub fn find_feature_unit(&self, id: u8) -> Option<&FeatureUnit> {
        self.units.iter().find_map(|u| match u {
            AudioUnit::FeatureUnit(f) if f.unit_id == id => Some(f),
            _ => None,
        })
    }

    /// Get all feature units in the topology
    pub fn feature_units(&self) -> Vec<&FeatureUnit> {
        self.units
            .iter()
            .filter_map(|u| match u {
                AudioUnit::FeatureUnit(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// Get all output (playback) streaming interfaces
    pub fn playback_interfaces(&self) -> Vec<&AudioStreamingInterface> {
        self.streaming_interfaces
            .iter()
            .filter(|s| s.is_output() && !s.is_zero_bandwidth())
            .collect()
    }

    /// Get all input (capture) streaming interfaces
    pub fn capture_interfaces(&self) -> Vec<&AudioStreamingInterface> {
        self.streaming_interfaces
            .iter()
            .filter(|s| s.is_input() && !s.is_zero_bandwidth())
            .collect()
    }

    /// Find a streaming interface that supports the given sample rate and
    /// channels
    pub fn find_compatible_interface(
        &self,
        sample_rate: u32,
        channels: u8,
        output: bool,
    ) -> Option<&AudioStreamingInterface> {
        self.streaming_interfaces.iter().find(|s| {
            let direction_ok = if output { s.is_output() } else { s.is_input() };
            direction_ok
                && !s.is_zero_bandwidth()
                && s.format.nr_channels == channels
                && s.format.supports_rate(sample_rate)
        })
    }

    /// Get the number of units in the topology
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }
}

// ============================================================================
// UAC Descriptor Parser
// ============================================================================

/// Parse a UAC Audio Control interface header
///
/// Returns (UAC version, total_length) or error.
pub fn parse_ac_header(data: &[u8]) -> Result<(UacVersion, u16), KernelError> {
    // Minimum AC header: bLength(1) + bDescriptorType(1) + bDescriptorSubtype(1)
    //                    + bcdADC(2) + wTotalLength(2) + bInCollection(1) +
    //                      baInterfaceNr(1...)
    if data.len() < 8 {
        return Err(KernelError::InvalidArgument {
            name: "ac_header",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_AC_HEADER {
        return Err(KernelError::InvalidArgument {
            name: "ac_header",
            value: "wrong descriptor type",
        });
    }

    let bcd_adc = u16::from_le_bytes([data[3], data[4]]);
    let version = if bcd_adc >= 0x0200 {
        UacVersion::Uac20
    } else {
        UacVersion::Uac10
    };

    let total_length = u16::from_le_bytes([data[5], data[6]]);

    Ok((version, total_length))
}

/// Parse an Input Terminal descriptor from raw bytes
pub fn parse_input_terminal(data: &[u8]) -> Result<InputTerminal, KernelError> {
    // UAC 1.0 input terminal is at least 12 bytes
    if data.len() < 12 {
        return Err(KernelError::InvalidArgument {
            name: "input_terminal",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_INPUT_TERMINAL {
        return Err(KernelError::InvalidArgument {
            name: "input_terminal",
            value: "wrong subtype",
        });
    }

    Ok(InputTerminal {
        terminal_id: data[3],
        terminal_type: u16::from_le_bytes([data[4], data[5]]),
        assoc_terminal: data[6],
        nr_channels: data[7],
        channel_config: u16::from_le_bytes([data[8], data[9]]),
    })
}

/// Parse an Output Terminal descriptor from raw bytes
pub fn parse_output_terminal(data: &[u8]) -> Result<OutputTerminal, KernelError> {
    // UAC 1.0 output terminal is at least 9 bytes
    if data.len() < 9 {
        return Err(KernelError::InvalidArgument {
            name: "output_terminal",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_OUTPUT_TERMINAL {
        return Err(KernelError::InvalidArgument {
            name: "output_terminal",
            value: "wrong subtype",
        });
    }

    Ok(OutputTerminal {
        terminal_id: data[3],
        terminal_type: u16::from_le_bytes([data[4], data[5]]),
        assoc_terminal: data[6],
        source_id: data[7],
    })
}

/// Parse a Feature Unit descriptor from raw bytes (UAC 1.0)
pub fn parse_feature_unit(data: &[u8]) -> Result<FeatureUnit, KernelError> {
    // Minimum: bLength(1) + bDescriptorType(1) + bDescriptorSubtype(1)
    //        + bUnitID(1) + bSourceID(1) + bControlSize(1) + bmaControls(...)
    if data.len() < 7 {
        return Err(KernelError::InvalidArgument {
            name: "feature_unit",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_FEATURE_UNIT {
        return Err(KernelError::InvalidArgument {
            name: "feature_unit",
            value: "wrong subtype",
        });
    }

    let unit_id = data[3];
    let source_id = data[4];
    let control_size = data[5] as usize;

    if control_size == 0 {
        return Ok(FeatureUnit {
            unit_id,
            source_id,
            controls: Vec::new(),
        });
    }

    // Parse per-channel control bitmasks
    let controls_start = 6;
    let bma_len = data[0] as usize - controls_start - 1; // subtract iString at end
    let num_controls = if control_size > 0 {
        bma_len / control_size
    } else {
        0
    };

    let mut controls = Vec::new();
    for i in 0..num_controls {
        let offset = controls_start + i * control_size;
        let mut ctrl: u32 = 0;
        for b in 0..control_size.min(4) {
            if offset + b < data.len() {
                ctrl |= (data[offset + b] as u32) << (b * 8);
            }
        }
        controls.push(ctrl);
    }

    Ok(FeatureUnit {
        unit_id,
        source_id,
        controls,
    })
}

/// Parse a Mixer Unit descriptor from raw bytes
pub fn parse_mixer_unit(data: &[u8]) -> Result<MixerUnit, KernelError> {
    // Minimum: bLength(1) + bDescriptorType(1) + bDescriptorSubtype(1)
    //        + bUnitID(1) + bNrInPins(1) + baSourceID(...)
    if data.len() < 5 {
        return Err(KernelError::InvalidArgument {
            name: "mixer_unit",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_MIXER_UNIT {
        return Err(KernelError::InvalidArgument {
            name: "mixer_unit",
            value: "wrong subtype",
        });
    }

    let unit_id = data[3];
    let nr_in_pins = data[4] as usize;

    let mut source_ids = Vec::new();
    for i in 0..nr_in_pins {
        let idx = 5 + i;
        if idx < data.len() {
            source_ids.push(data[idx]);
        }
    }

    // After source IDs: bNrChannels, wChannelConfig
    let chan_offset = 5 + nr_in_pins;
    let nr_channels = if chan_offset < data.len() {
        data[chan_offset]
    } else {
        0
    };
    let channel_config = if chan_offset + 2 < data.len() {
        u16::from_le_bytes([data[chan_offset + 1], data[chan_offset + 2]])
    } else {
        0
    };

    Ok(MixerUnit {
        unit_id,
        source_ids,
        nr_channels,
        channel_config,
    })
}

/// Parse an Audio Streaming format type descriptor (UAC 1.0 Type I)
pub fn parse_format_type_i(data: &[u8]) -> Result<AudioFormatDescriptor, KernelError> {
    // Minimum: bLength(1) + bDescriptorType(1) + bDescriptorSubtype(1)
    //        + bFormatType(1) + bNrChannels(1) + bSubframeSize(1)
    //        + bBitResolution(1) + bSamFreqType(1)
    if data.len() < 8 {
        return Err(KernelError::InvalidArgument {
            name: "format_type",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC_AS_FORMAT_TYPE {
        return Err(KernelError::InvalidArgument {
            name: "format_type",
            value: "wrong subtype",
        });
    }

    let nr_channels = data[4];
    let subframe_size = data[5];
    let bit_resolution = data[6];
    let sam_freq_type = data[7];

    let mut sample_rates = Vec::new();

    if sam_freq_type == 0 {
        // Continuous range: tLowerSamFreq(3) + tUpperSamFreq(3)
        if data.len() >= 14 {
            let lower = read_u24_le(&data[8..11]);
            let upper = read_u24_le(&data[11..14]);
            sample_rates.push(SampleRateRange::range(lower, upper));
        }
    } else {
        // Discrete sample rates: N * tSamFreq(3)
        for i in 0..sam_freq_type as usize {
            let offset = 8 + i * 3;
            if offset + 3 <= data.len() {
                let rate = read_u24_le(&data[offset..offset + 3]);
                sample_rates.push(SampleRateRange::discrete(rate));
            }
        }
    }

    Ok(AudioFormatDescriptor {
        format_tag: UAC_FORMAT_PCM, // Type I is always PCM-like
        nr_channels,
        subframe_size,
        bit_resolution,
        sample_rates,
    })
}

/// Read a 24-bit little-endian unsigned integer
fn read_u24_le(data: &[u8]) -> u32 {
    data[0] as u32 | ((data[1] as u32) << 8) | ((data[2] as u32) << 16)
}

/// Parse a Clock Source descriptor (UAC 2.0)
pub fn parse_clock_source(data: &[u8]) -> Result<ClockSource, KernelError> {
    // bLength(1) + bDescriptorType(1) + bDescriptorSubtype(1)
    // + bClockID(1) + bmAttributes(1) + bmControls(1) + bAssocTerminal(1)
    if data.len() < 7 {
        return Err(KernelError::InvalidArgument {
            name: "clock_source",
            value: "too short",
        });
    }

    if data[1] != CS_INTERFACE || data[2] != UAC2_CLOCK_SOURCE {
        return Err(KernelError::InvalidArgument {
            name: "clock_source",
            value: "wrong subtype",
        });
    }

    Ok(ClockSource {
        clock_id: data[3],
        attributes: data[4],
        assoc_terminal: data[6],
    })
}

// ============================================================================
// Sample Rate Control
// ============================================================================

/// Build a SET_CUR sample rate control request for UAC 1.0
///
/// Returns the control request bytes for setting the sample rate on an
/// isochronous endpoint.
pub fn build_set_sample_rate_request(endpoint: u8, sample_rate: u32) -> SampleRateRequest {
    SampleRateRequest {
        request_type: 0x22, // Host-to-device, class, endpoint
        request: UAC_SET_CUR,
        value: 0x0100, // Sampling Frequency Control
        index: endpoint as u16,
        sample_rate,
    }
}

/// Build a GET_CUR sample rate control request for UAC 1.0
pub fn build_get_sample_rate_request(endpoint: u8) -> SampleRateRequest {
    SampleRateRequest {
        request_type: 0xA2, // Device-to-host, class, endpoint
        request: UAC_GET_CUR,
        value: 0x0100,
        index: endpoint as u16,
        sample_rate: 0,
    }
}

/// Sample rate control request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRateRequest {
    /// bmRequestType
    pub request_type: u8,
    /// bRequest (SET_CUR or GET_CUR)
    pub request: u8,
    /// wValue (control selector << 8 | channel number)
    pub value: u16,
    /// wIndex (endpoint address)
    pub index: u16,
    /// Sample rate in Hz (3 bytes in USB, stored as u32)
    pub sample_rate: u32,
}

impl SampleRateRequest {
    /// Encode the sample rate as 3-byte little-endian for USB transfer
    pub fn rate_bytes(&self) -> [u8; 3] {
        [
            (self.sample_rate & 0xFF) as u8,
            ((self.sample_rate >> 8) & 0xFF) as u8,
            ((self.sample_rate >> 16) & 0xFF) as u8,
        ]
    }

    /// Decode a 3-byte sample rate response
    pub fn decode_rate(data: &[u8]) -> u32 {
        if data.len() >= 3 {
            read_u24_le(data)
        } else {
            0
        }
    }
}

// ============================================================================
// Volume Control Requests
// ============================================================================

/// Build a SET_CUR volume control request
pub fn build_set_volume_request(
    unit_id: u8,
    channel: u8,
    interface: u16,
    volume: VolumeDb,
) -> VolumeControlRequest {
    VolumeControlRequest {
        request_type: 0x21, // Host-to-device, class, interface
        request: UAC_SET_CUR,
        value: (UAC_FU_VOLUME_CONTROL as u16) << 8 | channel as u16,
        index: (unit_id as u16) << 8 | interface,
        volume,
    }
}

/// Build a GET_CUR volume control request
pub fn build_get_volume_request(unit_id: u8, channel: u8, interface: u16) -> VolumeControlRequest {
    VolumeControlRequest {
        request_type: 0xA1, // Device-to-host, class, interface
        request: UAC_GET_CUR,
        value: (UAC_FU_VOLUME_CONTROL as u16) << 8 | channel as u16,
        index: (unit_id as u16) << 8 | interface,
        volume: VolumeDb(0),
    }
}

/// Build a SET_CUR mute control request
pub fn build_set_mute_request(
    unit_id: u8,
    channel: u8,
    interface: u16,
    muted: bool,
) -> MuteControlRequest {
    MuteControlRequest {
        request_type: 0x21,
        request: UAC_SET_CUR,
        value: (UAC_FU_MUTE_CONTROL as u16) << 8 | channel as u16,
        index: (unit_id as u16) << 8 | interface,
        muted,
    }
}

/// Volume control request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeControlRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub volume: VolumeDb,
}

impl VolumeControlRequest {
    /// Encode the volume value as 2-byte little-endian for USB transfer
    pub fn volume_bytes(&self) -> [u8; 2] {
        self.volume.raw().to_le_bytes()
    }

    /// Decode a 2-byte volume response
    pub fn decode_volume(data: &[u8]) -> VolumeDb {
        if data.len() >= 2 {
            VolumeDb(i16::from_le_bytes([data[0], data[1]]))
        } else {
            VOLUME_SILENCE
        }
    }
}

/// Mute control request
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuteControlRequest {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub muted: bool,
}

// ============================================================================
// Device Enumeration
// ============================================================================

/// Scan USB descriptor data for audio class interfaces and build device
/// topology
///
/// Takes the full configuration descriptor bytes and parses all audio-related
/// descriptors into a `UsbAudioDevice`.
pub fn enumerate_audio_device(
    device_address: u8,
    config_descriptor: &[u8],
) -> Result<UsbAudioDevice, KernelError> {
    if config_descriptor.len() < 9 {
        return Err(KernelError::InvalidArgument {
            name: "config_descriptor",
            value: "too short",
        });
    }

    let mut device = UsbAudioDevice::new(device_address, UacVersion::Uac10);
    let mut offset = 0;

    // Walk through all descriptors
    while offset + 2 <= config_descriptor.len() {
        let desc_len = config_descriptor[offset] as usize;
        if desc_len < 2 || offset + desc_len > config_descriptor.len() {
            break;
        }

        let desc_type = config_descriptor[offset + 1];
        let desc_data = &config_descriptor[offset..offset + desc_len];

        if desc_type == CS_INTERFACE && desc_len >= 3 {
            let subtype = desc_data[2];
            match subtype {
                UAC_AC_HEADER => {
                    if let Ok((version, _total_len)) = parse_ac_header(desc_data) {
                        device.version = version;
                    }
                }
                UAC_INPUT_TERMINAL => {
                    // UAC_INPUT_TERMINAL == UAC_AS_FORMAT_TYPE (0x02).
                    // Try input terminal first, then fall back to format type.
                    if let Ok(terminal) = parse_input_terminal(desc_data) {
                        device.units.push(AudioUnit::InputTerminal(terminal));
                    } else if let Ok(format) = parse_format_type_i(desc_data) {
                        if let Some(last_stream) = device.streaming_interfaces.last_mut() {
                            last_stream.format = format;
                        }
                    }
                }
                UAC_OUTPUT_TERMINAL => {
                    if let Ok(terminal) = parse_output_terminal(desc_data) {
                        device.units.push(AudioUnit::OutputTerminal(terminal));
                    }
                }
                UAC_FEATURE_UNIT => {
                    if let Ok(unit) = parse_feature_unit(desc_data) {
                        device.units.push(AudioUnit::FeatureUnit(unit));
                    }
                }
                UAC_MIXER_UNIT => {
                    if let Ok(unit) = parse_mixer_unit(desc_data) {
                        device.units.push(AudioUnit::MixerUnit(unit));
                    }
                }
                UAC2_CLOCK_SOURCE => {
                    if let Ok(clock) = parse_clock_source(desc_data) {
                        device.units.push(AudioUnit::ClockSource(clock));
                    }
                }
                _ => {} // Other subtypes ignored for now
            }
        }

        offset += desc_len;
    }

    Ok(device)
}

// ============================================================================
// Standard Sample Rate Table
// ============================================================================

/// Standard USB audio sample rates in Hz
pub const STANDARD_SAMPLE_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
];

/// Check if a sample rate is a standard USB audio rate
pub fn is_standard_sample_rate(rate: u32) -> bool {
    STANDARD_SAMPLE_RATES.contains(&rate)
}

// ============================================================================
// HDMI Audio Constants
// ============================================================================

/// HDMI audio infoframe type code (CEA-861)
pub const HDMI_AUDIO_INFOFRAME_TYPE: u8 = 0x84;

/// HDMI audio infoframe version
pub const HDMI_AUDIO_INFOFRAME_VERSION: u8 = 0x01;

/// HDMI audio infoframe length (fixed at 10 bytes)
pub const HDMI_AUDIO_INFOFRAME_LENGTH: u8 = 0x0A;

/// HDMI audio coding types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HdmiAudioCoding {
    /// Refer to stream header
    StreamHeader = 0,
    /// IEC 60958 PCM (L-PCM)
    Pcm = 1,
    /// AC-3
    Ac3 = 2,
    /// MPEG-1 (layers 1 & 2)
    Mpeg1 = 3,
    /// MP3 (MPEG-1 layer 3)
    Mp3 = 4,
    /// MPEG-2 multichannel
    Mpeg2 = 5,
    /// AAC-LC
    AacLc = 6,
    /// DTS
    Dts = 7,
    /// ATRAC
    Atrac = 8,
    /// One Bit Audio (DSD)
    OneBitAudio = 9,
    /// Enhanced AC-3 (Dolby Digital Plus)
    EnhancedAc3 = 10,
    /// DTS-HD
    DtsHd = 11,
    /// MAT (MLP / Dolby TrueHD)
    Mat = 12,
    /// DST
    Dst = 13,
    /// WMA Pro
    WmaPro = 14,
}

/// HDMI channel allocation codes (CEA-861-D Table 20)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdmiChannelAllocation {
    /// 2.0 Stereo: FL, FR
    Stereo = 0x00,
    /// 2.1: FL, FR, LFE
    Stereo21 = 0x01,
    /// 3.0: FL, FR, FC
    Surround30 = 0x02,
    /// 3.1: FL, FR, LFE, FC
    Surround31 = 0x03,
    /// 5.0: FL, FR, FC, RL, RR
    Surround50 = 0x0A,
    /// 5.1: FL, FR, LFE, FC, RL, RR
    Surround51 = 0x0B,
    /// 7.0: FL, FR, FC, RL, RR, FLC, FRC
    Surround70 = 0x12,
    /// 7.1: FL, FR, LFE, FC, RL, RR, FLC, FRC
    Surround71 = 0x13,
}

impl HdmiChannelAllocation {
    /// Get the number of audio channels for this allocation
    pub fn channel_count(&self) -> u8 {
        match self {
            HdmiChannelAllocation::Stereo => 2,
            HdmiChannelAllocation::Stereo21 => 3,
            HdmiChannelAllocation::Surround30 => 3,
            HdmiChannelAllocation::Surround31 => 4,
            HdmiChannelAllocation::Surround50 => 5,
            HdmiChannelAllocation::Surround51 => 6,
            HdmiChannelAllocation::Surround70 => 7,
            HdmiChannelAllocation::Surround71 => 8,
        }
    }

    /// Get the allocation code byte
    pub fn code(&self) -> u8 {
        *self as u8
    }
}

/// HDMI audio sample rate encoding for infoframe
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HdmiSampleRate {
    /// Refer to stream header
    StreamHeader = 0,
    /// 32 kHz
    Rate32000 = 1,
    /// 44.1 kHz
    Rate44100 = 2,
    /// 48 kHz
    Rate48000 = 3,
    /// 88.2 kHz
    Rate88200 = 4,
    /// 96 kHz
    Rate96000 = 5,
    /// 176.4 kHz
    Rate176400 = 6,
    /// 192 kHz
    Rate192000 = 7,
}

impl HdmiSampleRate {
    /// Convert from Hz to HDMI sample rate code
    pub fn from_hz(hz: u32) -> Self {
        match hz {
            32000 => HdmiSampleRate::Rate32000,
            44100 => HdmiSampleRate::Rate44100,
            48000 => HdmiSampleRate::Rate48000,
            88200 => HdmiSampleRate::Rate88200,
            96000 => HdmiSampleRate::Rate96000,
            176400 => HdmiSampleRate::Rate176400,
            192000 => HdmiSampleRate::Rate192000,
            _ => HdmiSampleRate::StreamHeader,
        }
    }

    /// Convert to Hz
    pub fn to_hz(&self) -> u32 {
        match self {
            HdmiSampleRate::StreamHeader => 0,
            HdmiSampleRate::Rate32000 => 32000,
            HdmiSampleRate::Rate44100 => 44100,
            HdmiSampleRate::Rate48000 => 48000,
            HdmiSampleRate::Rate88200 => 88200,
            HdmiSampleRate::Rate96000 => 96000,
            HdmiSampleRate::Rate176400 => 176400,
            HdmiSampleRate::Rate192000 => 192000,
        }
    }
}

/// HDMI audio sample size encoding for infoframe
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HdmiSampleSize {
    /// Refer to stream header
    StreamHeader = 0,
    /// 16 bits per sample
    Bits16 = 1,
    /// 20 bits per sample
    Bits20 = 2,
    /// 24 bits per sample
    Bits24 = 3,
}

impl HdmiSampleSize {
    /// Convert from bit depth
    pub fn from_bits(bits: u8) -> Self {
        match bits {
            16 => HdmiSampleSize::Bits16,
            20 => HdmiSampleSize::Bits20,
            24 => HdmiSampleSize::Bits24,
            _ => HdmiSampleSize::StreamHeader,
        }
    }
}

// ============================================================================
// HDMI Audio Infoframe
// ============================================================================

/// HDMI Audio InfoFrame packet (CEA-861)
///
/// Transmitted over HDMI to inform the sink (TV/receiver) about the audio
/// stream format. The infoframe is 10 bytes of payload after the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdmiAudioInfoframe {
    /// Audio coding type
    pub coding_type: HdmiAudioCoding,
    /// Channel count (actual count - 1, as stored in CEA)
    pub channel_count: u8,
    /// Sample rate
    pub sample_rate: HdmiSampleRate,
    /// Sample size (for PCM)
    pub sample_size: HdmiSampleSize,
    /// Channel allocation (speaker mapping)
    pub channel_allocation: HdmiChannelAllocation,
    /// Level shift value (0-15, in 1 dB steps)
    pub level_shift: u8,
    /// Downmix inhibit flag
    pub downmix_inhibit: bool,
}

impl HdmiAudioInfoframe {
    /// Create a stereo PCM infoframe with common settings
    pub fn stereo_pcm(sample_rate: u32, bit_depth: u8) -> Self {
        Self {
            coding_type: HdmiAudioCoding::Pcm,
            channel_count: 2,
            sample_rate: HdmiSampleRate::from_hz(sample_rate),
            sample_size: HdmiSampleSize::from_bits(bit_depth),
            channel_allocation: HdmiChannelAllocation::Stereo,
            level_shift: 0,
            downmix_inhibit: false,
        }
    }

    /// Create a 5.1 surround PCM infoframe
    pub fn surround51_pcm(sample_rate: u32, bit_depth: u8) -> Self {
        Self {
            coding_type: HdmiAudioCoding::Pcm,
            channel_count: 6,
            sample_rate: HdmiSampleRate::from_hz(sample_rate),
            sample_size: HdmiSampleSize::from_bits(bit_depth),
            channel_allocation: HdmiChannelAllocation::Surround51,
            level_shift: 0,
            downmix_inhibit: false,
        }
    }

    /// Create a 7.1 surround PCM infoframe
    pub fn surround71_pcm(sample_rate: u32, bit_depth: u8) -> Self {
        Self {
            coding_type: HdmiAudioCoding::Pcm,
            channel_count: 8,
            sample_rate: HdmiSampleRate::from_hz(sample_rate),
            sample_size: HdmiSampleSize::from_bits(bit_depth),
            channel_allocation: HdmiChannelAllocation::Surround71,
            level_shift: 0,
            downmix_inhibit: false,
        }
    }

    /// Serialize the infoframe to a byte array (header + payload)
    ///
    /// Returns a 13-byte array:
    /// - Byte 0: Type code (0x84)
    /// - Byte 1: Version (0x01)
    /// - Byte 2: Length (0x0A)
    /// - Byte 3: Checksum
    /// - Bytes 4-13: Payload (DB1-DB10)
    pub fn to_bytes(&self) -> [u8; 14] {
        let mut buf = [0u8; 14];

        // Header
        buf[0] = HDMI_AUDIO_INFOFRAME_TYPE;
        buf[1] = HDMI_AUDIO_INFOFRAME_VERSION;
        buf[2] = HDMI_AUDIO_INFOFRAME_LENGTH;

        // DB1: Coding Type (bits 7-4) | Channel Count - 1 (bits 2-0)
        let cc = if self.channel_count > 0 {
            self.channel_count - 1
        } else {
            0
        };
        buf[4] = ((self.coding_type as u8) << 4) | (cc & 0x07);

        // DB2: Sample Rate (bits 4-2) | Sample Size (bits 1-0)
        buf[5] = ((self.sample_rate as u8) << 2) | (self.sample_size as u8);

        // DB3: Reserved (0x00)
        buf[6] = 0x00;

        // DB4: Channel Allocation
        buf[7] = self.channel_allocation.code();

        // DB5: Level Shift (bits 6-3) | Downmix Inhibit (bit 7)
        buf[8] = (self.level_shift & 0x0F) << 3;
        if self.downmix_inhibit {
            buf[8] |= 0x80;
        }

        // DB6-DB10: Reserved
        // buf[9..14] already zeroed

        // Calculate checksum: sum of all bytes (header + payload) must be 0 mod 256
        let mut sum: u8 = 0;
        for (i, &byte) in buf.iter().enumerate().take(14) {
            if i != 3 {
                // Skip checksum byte itself
                sum = sum.wrapping_add(byte);
            }
        }
        buf[3] = 0u8.wrapping_sub(sum);

        buf
    }

    /// Verify an infoframe checksum
    pub fn verify_checksum(data: &[u8]) -> bool {
        if data.len() < 14 {
            return false;
        }
        let sum: u8 = data[..14].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        sum == 0
    }

    /// Parse an infoframe from raw bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 14 {
            return Err(KernelError::InvalidArgument {
                name: "hdmi_infoframe",
                value: "too short",
            });
        }

        if data[0] != HDMI_AUDIO_INFOFRAME_TYPE {
            return Err(KernelError::InvalidArgument {
                name: "hdmi_infoframe",
                value: "wrong type code",
            });
        }

        if !Self::verify_checksum(data) {
            return Err(KernelError::InvalidArgument {
                name: "hdmi_infoframe",
                value: "bad checksum",
            });
        }

        let coding_raw = (data[4] >> 4) & 0x0F;
        let coding_type = match coding_raw {
            0 => HdmiAudioCoding::StreamHeader,
            1 => HdmiAudioCoding::Pcm,
            2 => HdmiAudioCoding::Ac3,
            3 => HdmiAudioCoding::Mpeg1,
            4 => HdmiAudioCoding::Mp3,
            5 => HdmiAudioCoding::Mpeg2,
            6 => HdmiAudioCoding::AacLc,
            7 => HdmiAudioCoding::Dts,
            8 => HdmiAudioCoding::Atrac,
            9 => HdmiAudioCoding::OneBitAudio,
            10 => HdmiAudioCoding::EnhancedAc3,
            11 => HdmiAudioCoding::DtsHd,
            12 => HdmiAudioCoding::Mat,
            13 => HdmiAudioCoding::Dst,
            14 => HdmiAudioCoding::WmaPro,
            _ => HdmiAudioCoding::StreamHeader,
        };

        let channel_count = (data[4] & 0x07) + 1;

        let sr_raw = (data[5] >> 2) & 0x07;
        let sample_rate = match sr_raw {
            1 => HdmiSampleRate::Rate32000,
            2 => HdmiSampleRate::Rate44100,
            3 => HdmiSampleRate::Rate48000,
            4 => HdmiSampleRate::Rate88200,
            5 => HdmiSampleRate::Rate96000,
            6 => HdmiSampleRate::Rate176400,
            7 => HdmiSampleRate::Rate192000,
            _ => HdmiSampleRate::StreamHeader,
        };

        let ss_raw = data[5] & 0x03;
        let sample_size = match ss_raw {
            1 => HdmiSampleSize::Bits16,
            2 => HdmiSampleSize::Bits20,
            3 => HdmiSampleSize::Bits24,
            _ => HdmiSampleSize::StreamHeader,
        };

        let ca_code = data[7];
        let channel_allocation = match ca_code {
            0x00 => HdmiChannelAllocation::Stereo,
            0x01 => HdmiChannelAllocation::Stereo21,
            0x02 => HdmiChannelAllocation::Surround30,
            0x03 => HdmiChannelAllocation::Surround31,
            0x0A => HdmiChannelAllocation::Surround50,
            0x0B => HdmiChannelAllocation::Surround51,
            0x12 => HdmiChannelAllocation::Surround70,
            0x13 => HdmiChannelAllocation::Surround71,
            _ => HdmiChannelAllocation::Stereo, // Default fallback
        };

        let level_shift = (data[8] >> 3) & 0x0F;
        let downmix_inhibit = data[8] & 0x80 != 0;

        Ok(Self {
            coding_type,
            channel_count,
            sample_rate,
            sample_size,
            channel_allocation,
            level_shift,
            downmix_inhibit,
        })
    }
}

// ============================================================================
// HDMI Audio Clock Regeneration (ACR)
// ============================================================================

/// Audio Clock Regeneration (ACR) parameters
///
/// HDMI uses ACR packets to reconstruct the audio sampling clock at the sink.
/// The relationship is: Fs = N * TMDS_clock / (128 * CTS)
///
/// The N and CTS values for standard sample rates at standard TMDS clock
/// frequencies are defined in the HDMI specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioClockRegeneration {
    /// N value (audio clock numerator)
    pub n: u32,
    /// CTS value (Cycle Time Stamp, denominator)
    pub cts: u32,
}

impl AudioClockRegeneration {
    /// Get recommended N value for a given sample rate
    ///
    /// N values from HDMI 1.4b specification, Table 7-1/7-2/7-3.
    /// These are the recommended values for 148.5 MHz TMDS clock (1080p60).
    pub fn recommended_n(sample_rate: u32) -> u32 {
        match sample_rate {
            32000 => 4096,
            44100 => 6272,
            48000 => 6144,
            88200 => 12544,
            96000 => 12288,
            176400 => 25088,
            192000 => 24576,
            _ => {
                // For non-standard rates, use the formula: N = 128 * Fs / 1000
                // This gives a reasonable default when CTS is computed from TMDS clock
                sample_rate.saturating_mul(128) / 1000
            }
        }
    }

    /// Calculate CTS from N, sample rate, and TMDS clock
    ///
    /// CTS = N * TMDS_clock / (128 * Fs)
    ///
    /// Uses u64 intermediate to avoid overflow.
    pub fn calculate_cts(n: u32, sample_rate: u32, tmds_clock_khz: u32) -> u32 {
        if sample_rate == 0 {
            return 0;
        }

        // CTS = N * (TMDS_clock_kHz * 1000) / (128 * Fs)
        // Rearrange to avoid overflow: CTS = (N * TMDS_clock_kHz) / (128 * Fs / 1000)
        // But simpler with u64: CTS = (N as u64 * TMDS_clock_kHz as u64 * 1000) / (128
        // * Fs as u64)
        let numerator = (n as u64)
            .checked_mul(tmds_clock_khz as u64)
            .and_then(|v| v.checked_mul(1000));
        let denominator = 128u64.checked_mul(sample_rate as u64);

        match (numerator, denominator) {
            (Some(num), Some(den)) if den > 0 => (num / den) as u32,
            _ => 0,
        }
    }

    /// Create ACR parameters for a standard configuration
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `tmds_clock_khz` - TMDS pixel clock in kHz (e.g., 148500 for 1080p60)
    pub fn new(sample_rate: u32, tmds_clock_khz: u32) -> Self {
        let n = Self::recommended_n(sample_rate);
        let cts = Self::calculate_cts(n, sample_rate, tmds_clock_khz);
        Self { n, cts }
    }

    /// Standard ACR for 1080p60 (148.5 MHz TMDS clock)
    pub fn for_1080p60(sample_rate: u32) -> Self {
        Self::new(sample_rate, 148500)
    }

    /// Standard ACR for 4K60 (594 MHz TMDS clock)
    pub fn for_4k60(sample_rate: u32) -> Self {
        Self::new(sample_rate, 594000)
    }

    /// Standard ACR for 720p60 (74.25 MHz TMDS clock)
    pub fn for_720p60(sample_rate: u32) -> Self {
        Self::new(sample_rate, 74250)
    }
}

// ============================================================================
// ELD (EDID-Like Data) Parsing
// ============================================================================

/// ELD (EDID-Like Data) for HDMI audio sink capabilities
///
/// The ELD block is provided by the GPU driver after reading the EDID from
/// the connected display. It contains information about the audio capabilities
/// of the sink device (TV, receiver, soundbar).
#[derive(Debug, Clone)]
pub struct HdmiEld {
    /// ELD version (2 for CEA version compatible)
    pub eld_ver: u8,
    /// Baseline ELD length (in 32-bit words)
    pub baseline_len: u8,
    /// CEA EDID version
    pub cea_edid_ver: u8,
    /// Monitor name string
    pub monitor_name: String,
    /// Number of supported Short Audio Descriptors
    pub sad_count: u8,
    /// Short Audio Descriptors
    pub sads: Vec<ShortAudioDescriptor>,
    /// Connection type (0 = HDMI, 1 = DisplayPort)
    pub conn_type: u8,
    /// Supports audio return channel (ARC)
    pub supports_arc: bool,
    /// Supports AI (audio input)
    pub supports_ai: bool,
    /// Port ID for multi-output GPUs
    pub port_id: u64,
    /// Manufacturer vendor ID
    pub manufacturer_id: u16,
    /// Product code
    pub product_code: u16,
}

/// CEA Short Audio Descriptor (SAD)
///
/// 3-byte structure from CEA-861-D defining audio format support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortAudioDescriptor {
    /// Audio format code (matches HdmiAudioCoding values)
    pub format_code: u8,
    /// Maximum number of channels supported
    pub max_channels: u8,
    /// Supported sample rate bitmask:
    /// bit 0: 32 kHz, bit 1: 44.1 kHz, bit 2: 48 kHz,
    /// bit 3: 88.2 kHz, bit 4: 96 kHz, bit 5: 176.4 kHz, bit 6: 192 kHz
    pub sample_rate_mask: u8,
    /// For PCM: bit depth mask (bit 0: 16-bit, bit 1: 20-bit, bit 2: 24-bit)
    /// For compressed: max bitrate / 8000
    pub detail: u8,
}

impl ShortAudioDescriptor {
    /// Parse a SAD from 3 raw bytes
    pub fn from_bytes(data: &[u8; 3]) -> Self {
        Self {
            format_code: (data[0] >> 3) & 0x0F,
            max_channels: (data[0] & 0x07) + 1,
            sample_rate_mask: data[1] & 0x7F,
            detail: data[2],
        }
    }

    /// Check if this SAD supports a given sample rate
    pub fn supports_rate(&self, rate: u32) -> bool {
        let bit = match rate {
            32000 => 0,
            44100 => 1,
            48000 => 2,
            88200 => 3,
            96000 => 4,
            176400 => 5,
            192000 => 6,
            _ => return false,
        };
        self.sample_rate_mask & (1 << bit) != 0
    }

    /// Check if this SAD supports PCM and a given bit depth
    pub fn supports_pcm_depth(&self, bits: u8) -> bool {
        if self.format_code != 1 {
            // Only PCM has bit depth info
            return false;
        }
        match bits {
            16 => self.detail & 0x01 != 0,
            20 => self.detail & 0x02 != 0,
            24 => self.detail & 0x04 != 0,
            _ => false,
        }
    }

    /// Check if this is a PCM format descriptor
    pub fn is_pcm(&self) -> bool {
        self.format_code == 1
    }

    /// Serialize to 3-byte representation
    pub fn to_bytes(&self) -> [u8; 3] {
        let byte0 =
            ((self.format_code & 0x0F) << 3) | ((self.max_channels.saturating_sub(1)) & 0x07);
        [byte0, self.sample_rate_mask & 0x7F, self.detail]
    }
}

impl HdmiEld {
    /// Parse an ELD block from raw bytes
    ///
    /// The ELD format is defined in the HDA specification and contains
    /// baseline and vendor-specific sections.
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 16 {
            return Err(KernelError::InvalidArgument {
                name: "eld",
                value: "too short",
            });
        }

        let eld_ver = (data[0] >> 3) & 0x1F;
        let baseline_len = data[2];
        let cea_edid_ver = (data[4] >> 5) & 0x07;

        let sad_count = (data[4] & 0x0F).min(15); // Max 15 SADs
        let conn_type = (data[5] >> 2) & 0x03;
        let supports_arc = data[5] & 0x02 != 0;
        let supports_ai = data[5] & 0x01 != 0;

        // Monitor name length (byte 4, bits 7-5 are CEA ver, bits 4-0 are MNL...
        // Actually in ELD v2: byte 4 bits 4-0 = SAD count, byte 6 = MNL
        let mnl = data[6] as usize;

        let manufacturer_id = u16::from_le_bytes([data[8], data[9]]);
        let product_code = u16::from_le_bytes([data[10], data[11]]);

        let port_id = if data.len() >= 20 {
            u64::from_le_bytes([
                data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
            ])
        } else {
            0
        };

        // Monitor name starts at offset 20
        let name_start = 20;
        let name_end = (name_start + mnl).min(data.len());
        let monitor_name = if name_start < data.len() {
            let name_bytes = &data[name_start..name_end];
            String::from_utf8_lossy(name_bytes).into_owned()
        } else {
            String::new()
        };

        // SADs start after monitor name
        let sad_start = name_start + mnl;
        let mut sads = Vec::new();
        for i in 0..sad_count as usize {
            let offset = sad_start + i * 3;
            if offset + 3 <= data.len() {
                let sad_bytes: [u8; 3] = [data[offset], data[offset + 1], data[offset + 2]];
                sads.push(ShortAudioDescriptor::from_bytes(&sad_bytes));
            }
        }

        Ok(Self {
            eld_ver,
            baseline_len,
            cea_edid_ver,
            monitor_name,
            sad_count,
            sads,
            conn_type,
            supports_arc,
            supports_ai,
            port_id,
            manufacturer_id,
            product_code,
        })
    }

    /// Check if the sink supports a specific audio format
    pub fn supports_format(&self, format_code: u8, rate: u32, channels: u8) -> bool {
        self.sads.iter().any(|sad| {
            sad.format_code == format_code
                && sad.max_channels >= channels
                && sad.supports_rate(rate)
        })
    }

    /// Check if the sink supports stereo PCM at a given rate and depth
    pub fn supports_stereo_pcm(&self, rate: u32, bit_depth: u8) -> bool {
        self.sads.iter().any(|sad| {
            sad.is_pcm()
                && sad.max_channels >= 2
                && sad.supports_rate(rate)
                && sad.supports_pcm_depth(bit_depth)
        })
    }

    /// Get the maximum number of PCM channels supported
    pub fn max_pcm_channels(&self) -> u8 {
        self.sads
            .iter()
            .filter(|sad| sad.is_pcm())
            .map(|sad| sad.max_channels)
            .max()
            .unwrap_or(0)
    }

    /// Get all supported PCM sample rates
    pub fn supported_pcm_rates(&self) -> Vec<u32> {
        let mut rates = Vec::new();
        let rate_table: &[(u32, u8)] = &[
            (32000, 0),
            (44100, 1),
            (48000, 2),
            (88200, 3),
            (96000, 4),
            (176400, 5),
            (192000, 6),
        ];

        for sad in &self.sads {
            if sad.is_pcm() {
                for &(rate, bit) in rate_table {
                    if sad.sample_rate_mask & (1 << bit) != 0 && !rates.contains(&rate) {
                        rates.push(rate);
                    }
                }
            }
        }

        rates
    }
}

// ============================================================================
// HDMI Audio HDA Codec Integration
// ============================================================================

/// HDA codec widget types relevant to HDMI audio
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdaWidgetType {
    /// Audio output converter (DAC)
    AudioOutput,
    /// Pin complex (HDMI output pin)
    PinComplex,
}

/// HDMI audio output configuration for HDA codec
#[derive(Debug, Clone)]
pub struct HdmiAudioOutput {
    /// HDA codec address (0-15)
    pub codec_address: u8,
    /// Audio output converter widget NID
    pub converter_nid: u16,
    /// Pin complex widget NID
    pub pin_nid: u16,
    /// ELD data from connected sink (if available)
    pub eld: Option<HdmiEld>,
    /// Current infoframe being transmitted
    pub infoframe: Option<HdmiAudioInfoframe>,
    /// Current ACR parameters
    pub acr: Option<AudioClockRegeneration>,
    /// Whether audio is currently enabled on this output
    pub enabled: bool,
}

impl HdmiAudioOutput {
    /// Create a new HDMI audio output
    pub fn new(codec_address: u8, converter_nid: u16, pin_nid: u16) -> Self {
        Self {
            codec_address,
            converter_nid,
            pin_nid,
            eld: None,
            infoframe: None,
            acr: None,
            enabled: false,
        }
    }

    /// Configure the output for stereo PCM at the given rate
    pub fn configure_stereo_pcm(
        &mut self,
        sample_rate: u32,
        bit_depth: u8,
        tmds_clock_khz: u32,
    ) -> Result<(), KernelError> {
        // Check if sink supports this format
        if let Some(ref eld) = self.eld {
            if !eld.supports_stereo_pcm(sample_rate, bit_depth) {
                return Err(KernelError::OperationNotSupported {
                    operation: "sink does not support requested format",
                });
            }
        }

        // Build infoframe
        self.infoframe = Some(HdmiAudioInfoframe::stereo_pcm(sample_rate, bit_depth));

        // Calculate ACR
        self.acr = Some(AudioClockRegeneration::new(sample_rate, tmds_clock_khz));

        Ok(())
    }

    /// Enable audio output
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable audio output
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Check if a sink is connected (ELD available)
    pub fn has_sink(&self) -> bool {
        self.eld.is_some()
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

    // --- IsoSyncType tests ---

    #[test]
    fn test_iso_sync_type_from_attributes() {
        assert_eq!(IsoSyncType::from_attributes(0x00), IsoSyncType::None);
        assert_eq!(
            IsoSyncType::from_attributes(0x04),
            IsoSyncType::Asynchronous
        );
        assert_eq!(IsoSyncType::from_attributes(0x08), IsoSyncType::Adaptive);
        assert_eq!(IsoSyncType::from_attributes(0x0C), IsoSyncType::Synchronous);
        // Extra bits should be masked off
        assert_eq!(
            IsoSyncType::from_attributes(0xF4),
            IsoSyncType::Asynchronous
        );
    }

    #[test]
    fn test_iso_sync_type_roundtrip() {
        for sync in [
            IsoSyncType::None,
            IsoSyncType::Asynchronous,
            IsoSyncType::Adaptive,
            IsoSyncType::Synchronous,
        ] {
            assert_eq!(IsoSyncType::from_attributes(sync.to_attributes()), sync);
        }
    }

    // --- VolumeDb tests ---

    #[test]
    fn test_volume_db_from_db() {
        let vol = VolumeDb::from_db(0);
        assert_eq!(vol.raw(), 0);
        assert_eq!(vol.integer_db(), 0);
        assert_eq!(vol.fraction(), 0);

        let vol_pos = VolumeDb::from_db(10);
        assert_eq!(vol_pos.integer_db(), 10);

        let vol_neg = VolumeDb::from_db(-20);
        assert_eq!(vol_neg.integer_db(), -20);
    }

    #[test]
    fn test_volume_db_to_linear() {
        // 0 dB should map to max (65535)
        assert_eq!(VOLUME_UNITY.to_linear_u16(), 65535);

        // Positive dB should also saturate to max
        assert_eq!(VolumeDb::from_db(10).to_linear_u16(), 65535);

        // Very negative dB should be silence
        assert_eq!(VOLUME_SILENCE.to_linear_u16(), 0);
        assert_eq!(VolumeDb::from_db(-96).to_linear_u16(), 0);

        // -6 dB should be approximately half amplitude
        let half = VolumeDb::from_db(-6).to_linear_u16();
        assert!(
            half > 30000 && half < 35000,
            "Expected ~32767, got {}",
            half
        );
    }

    #[test]
    fn test_volume_db_from_linear() {
        assert_eq!(VolumeDb::from_linear_u16(0), VOLUME_SILENCE);
        assert_eq!(VolumeDb::from_linear_u16(65535), VOLUME_UNITY);
    }

    // --- SampleRateRange tests ---

    #[test]
    fn test_sample_rate_range_discrete() {
        let rate = SampleRateRange::discrete(48000);
        assert!(rate.is_discrete());
        assert!(rate.contains(48000));
        assert!(!rate.contains(44100));
    }

    #[test]
    fn test_sample_rate_range_continuous() {
        let range = SampleRateRange::range(8000, 96000);
        assert!(!range.is_discrete());
        assert!(range.contains(44100));
        assert!(range.contains(48000));
        assert!(range.contains(8000));
        assert!(range.contains(96000));
        assert!(!range.contains(192000));
    }

    // --- AudioFormatDescriptor tests ---

    #[test]
    fn test_audio_format_supports_rate() {
        let format = AudioFormatDescriptor {
            format_tag: UAC_FORMAT_PCM,
            nr_channels: 2,
            subframe_size: 2,
            bit_resolution: 16,
            sample_rates: vec![
                SampleRateRange::discrete(44100),
                SampleRateRange::discrete(48000),
            ],
        };
        assert!(format.supports_rate(44100));
        assert!(format.supports_rate(48000));
        assert!(!format.supports_rate(96000));
        assert!(format.is_pcm());
        assert_eq!(format.frame_size(), 4); // 2 channels * 2 bytes
    }

    #[test]
    fn test_audio_format_empty_rates() {
        let format = AudioFormatDescriptor {
            format_tag: UAC_FORMAT_PCM,
            nr_channels: 1,
            subframe_size: 2,
            bit_resolution: 16,
            sample_rates: Vec::new(),
        };
        // Empty rates list means any rate is acceptable
        assert!(format.supports_rate(44100));
        assert!(format.supports_rate(192000));
    }

    // --- Feature Unit tests ---

    #[test]
    fn test_feature_unit_controls() {
        let fu = FeatureUnit {
            unit_id: 5,
            source_id: 1,
            controls: vec![0x03, 0x01, 0x01], // master: mute+volume, ch1: mute, ch2: mute
        };
        assert!(fu.has_mute(0));
        assert!(fu.has_volume(0));
        assert!(fu.has_mute(1));
        assert!(!fu.has_volume(1));
        assert!(!fu.has_mute(10)); // Out of range
    }

    // --- Descriptor parser tests ---

    #[test]
    fn test_parse_ac_header() {
        // UAC 1.0 AC header
        let data = [
            9,             // bLength
            CS_INTERFACE,  // bDescriptorType
            UAC_AC_HEADER, // bDescriptorSubtype
            0x00,
            0x01, // bcdADC = 0x0100 (UAC 1.0)
            0x40,
            0x00, // wTotalLength = 64
            0x01, // bInCollection = 1
            0x01, // baInterfaceNr[0] = 1
        ];
        let (version, total_len) = parse_ac_header(&data).unwrap();
        assert_eq!(version, UacVersion::Uac10);
        assert_eq!(total_len, 64);
    }

    #[test]
    fn test_parse_ac_header_uac20() {
        let data = [
            9,
            CS_INTERFACE,
            UAC_AC_HEADER,
            0x00,
            0x02, // bcdADC = 0x0200 (UAC 2.0)
            0x80,
            0x00,
            0x01,
            0x01,
        ];
        let (version, _) = parse_ac_header(&data).unwrap();
        assert_eq!(version, UacVersion::Uac20);
    }

    #[test]
    fn test_parse_input_terminal() {
        let data = [
            12,                 // bLength
            CS_INTERFACE,       // bDescriptorType
            UAC_INPUT_TERMINAL, // bDescriptorSubtype
            0x01,               // bTerminalID
            0x01,
            0x02, // wTerminalType = 0x0201 (microphone)
            0x00, // bAssocTerminal
            0x02, // bNrChannels
            0x03,
            0x00, // wChannelConfig (front left + front right)
            0x00, // iChannelNames
            0x00, // iTerminal
        ];
        let terminal = parse_input_terminal(&data).unwrap();
        assert_eq!(terminal.terminal_id, 1);
        assert_eq!(terminal.terminal_type, 0x0201);
        assert_eq!(terminal.nr_channels, 2);
        assert_eq!(terminal.channel_config, 0x0003);
    }

    #[test]
    fn test_parse_output_terminal() {
        let data = [
            9,                   // bLength
            CS_INTERFACE,        // bDescriptorType
            UAC_OUTPUT_TERMINAL, // bDescriptorSubtype
            0x03,                // bTerminalID
            0x01,
            0x03, // wTerminalType = 0x0301 (speaker)
            0x00, // bAssocTerminal
            0x02, // bSourceID (from unit 2)
            0x00, // iTerminal
        ];
        let terminal = parse_output_terminal(&data).unwrap();
        assert_eq!(terminal.terminal_id, 3);
        assert_eq!(terminal.terminal_type, 0x0301);
        assert_eq!(terminal.source_id, 2);
    }

    #[test]
    fn test_parse_feature_unit() {
        let data = [
            10,               // bLength
            CS_INTERFACE,     // bDescriptorType
            UAC_FEATURE_UNIT, // bDescriptorSubtype
            0x02,             // bUnitID
            0x01,             // bSourceID
            0x01,             // bControlSize = 1
            0x03,             // bmaControls(0) master: mute + volume
            0x01,             // bmaControls(1) ch1: mute
            0x01,             // bmaControls(2) ch2: mute
            0x00,             // iFeature
        ];
        let fu = parse_feature_unit(&data).unwrap();
        assert_eq!(fu.unit_id, 2);
        assert_eq!(fu.source_id, 1);
        assert_eq!(fu.controls.len(), 3);
        assert!(fu.has_mute(0));
        assert!(fu.has_volume(0));
    }

    #[test]
    fn test_parse_format_type_i_discrete() {
        let data = [
            14,                 // bLength
            CS_INTERFACE,       // bDescriptorType
            UAC_AS_FORMAT_TYPE, // bDescriptorSubtype
            0x01,               // bFormatType = FORMAT_TYPE_I
            0x02,               // bNrChannels
            0x02,               // bSubframeSize
            16,                 // bBitResolution
            0x02,               // bSamFreqType = 2 (discrete)
            0x44,
            0xAC,
            0x00, // tSamFreq[0] = 44100
            0x80,
            0xBB,
            0x00, // tSamFreq[1] = 48000
        ];
        let fmt = parse_format_type_i(&data).unwrap();
        assert_eq!(fmt.nr_channels, 2);
        assert_eq!(fmt.subframe_size, 2);
        assert_eq!(fmt.bit_resolution, 16);
        assert_eq!(fmt.sample_rates.len(), 2);
        assert!(fmt.supports_rate(44100));
        assert!(fmt.supports_rate(48000));
        assert!(!fmt.supports_rate(96000));
    }

    #[test]
    fn test_parse_format_type_i_continuous() {
        let data = [
            14,
            CS_INTERFACE,
            UAC_AS_FORMAT_TYPE,
            0x01, // FORMAT_TYPE_I
            0x02, // 2 channels
            0x02, // 2 bytes/sample
            16,   // 16 bit
            0x00, // continuous
            0x40,
            0x1F,
            0x00, // lower = 8000
            0x00,
            0xEE,
            0x02, // upper = 192000
        ];
        let fmt = parse_format_type_i(&data).unwrap();
        assert_eq!(fmt.sample_rates.len(), 1);
        assert!(!fmt.sample_rates[0].is_discrete());
        assert!(fmt.supports_rate(44100));
        assert!(fmt.supports_rate(192000));
    }

    // --- Sample rate request tests ---

    #[test]
    fn test_sample_rate_request_bytes() {
        let req = build_set_sample_rate_request(0x01, 48000);
        let bytes = req.rate_bytes();
        assert_eq!(bytes, [0x80, 0xBB, 0x00]);
        assert_eq!(SampleRateRequest::decode_rate(&bytes), 48000);
    }

    #[test]
    fn test_sample_rate_44100() {
        let req = build_set_sample_rate_request(0x01, 44100);
        let bytes = req.rate_bytes();
        assert_eq!(SampleRateRequest::decode_rate(&bytes), 44100);
    }

    // --- Volume control request tests ---

    #[test]
    fn test_volume_control_request() {
        let req = build_set_volume_request(2, 0, 0, VolumeDb::from_db(-10));
        assert_eq!(req.request, UAC_SET_CUR);
        let bytes = req.volume_bytes();
        let decoded = VolumeControlRequest::decode_volume(&bytes);
        assert_eq!(decoded.integer_db(), -10);
    }

    // --- HDMI infoframe tests ---

    #[test]
    fn test_hdmi_infoframe_stereo_pcm() {
        let frame = HdmiAudioInfoframe::stereo_pcm(48000, 16);
        assert_eq!(frame.coding_type, HdmiAudioCoding::Pcm);
        assert_eq!(frame.channel_count, 2);
        assert_eq!(frame.sample_rate, HdmiSampleRate::Rate48000);
        assert_eq!(frame.sample_size, HdmiSampleSize::Bits16);
        assert_eq!(frame.channel_allocation, HdmiChannelAllocation::Stereo);
    }

    #[test]
    fn test_hdmi_infoframe_serialize_parse_roundtrip() {
        let original = HdmiAudioInfoframe::stereo_pcm(48000, 16);
        let bytes = original.to_bytes();

        // Verify checksum
        assert!(HdmiAudioInfoframe::verify_checksum(&bytes));

        // Verify type and version
        assert_eq!(bytes[0], HDMI_AUDIO_INFOFRAME_TYPE);
        assert_eq!(bytes[1], HDMI_AUDIO_INFOFRAME_VERSION);

        // Parse back
        let parsed = HdmiAudioInfoframe::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_hdmi_infoframe_51_roundtrip() {
        let original = HdmiAudioInfoframe::surround51_pcm(44100, 24);
        let bytes = original.to_bytes();
        assert!(HdmiAudioInfoframe::verify_checksum(&bytes));
        let parsed = HdmiAudioInfoframe::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_hdmi_infoframe_bad_checksum() {
        let frame = HdmiAudioInfoframe::stereo_pcm(48000, 16);
        let mut bytes = frame.to_bytes();
        bytes[3] = bytes[3].wrapping_add(1); // Corrupt checksum
        assert!(!HdmiAudioInfoframe::verify_checksum(&bytes));
        assert!(HdmiAudioInfoframe::from_bytes(&bytes).is_err());
    }

    // --- HDMI ACR tests ---

    #[test]
    fn test_acr_recommended_n() {
        assert_eq!(AudioClockRegeneration::recommended_n(48000), 6144);
        assert_eq!(AudioClockRegeneration::recommended_n(44100), 6272);
        assert_eq!(AudioClockRegeneration::recommended_n(32000), 4096);
        assert_eq!(AudioClockRegeneration::recommended_n(96000), 12288);
        assert_eq!(AudioClockRegeneration::recommended_n(192000), 24576);
    }

    #[test]
    fn test_acr_cts_calculation() {
        // For 48 kHz at 148.5 MHz TMDS:
        // CTS = N * TMDS / (128 * Fs) = 6144 * 148500000 / (128 * 48000)
        //     = 6144 * 148500 * 1000 / (128 * 48000) = 148500
        let cts = AudioClockRegeneration::calculate_cts(6144, 48000, 148500);
        assert_eq!(cts, 148500);
    }

    #[test]
    fn test_acr_1080p60() {
        let acr = AudioClockRegeneration::for_1080p60(48000);
        assert_eq!(acr.n, 6144);
        assert_eq!(acr.cts, 148500);
    }

    // --- HDMI channel allocation tests ---

    #[test]
    fn test_channel_allocation_count() {
        assert_eq!(HdmiChannelAllocation::Stereo.channel_count(), 2);
        assert_eq!(HdmiChannelAllocation::Surround51.channel_count(), 6);
        assert_eq!(HdmiChannelAllocation::Surround71.channel_count(), 8);
    }

    // --- Short Audio Descriptor tests ---

    #[test]
    fn test_sad_from_bytes() {
        // PCM, 2 channels, 48 kHz + 44.1 kHz, 16-bit + 24-bit
        let bytes: [u8; 3] = [
            (1 << 3) | 0x01, // format=PCM, max_channels=2
            0x06,            // 44.1 kHz (bit 1) + 48 kHz (bit 2)
            0x05,            // 16-bit (bit 0) + 24-bit (bit 2)
        ];
        let sad = ShortAudioDescriptor::from_bytes(&bytes);
        assert_eq!(sad.format_code, 1);
        assert_eq!(sad.max_channels, 2);
        assert!(sad.is_pcm());
        assert!(sad.supports_rate(44100));
        assert!(sad.supports_rate(48000));
        assert!(!sad.supports_rate(96000));
        assert!(sad.supports_pcm_depth(16));
        assert!(!sad.supports_pcm_depth(20));
        assert!(sad.supports_pcm_depth(24));
    }

    #[test]
    fn test_sad_roundtrip() {
        let original = ShortAudioDescriptor {
            format_code: 1,
            max_channels: 6,
            sample_rate_mask: 0x07, // 32k, 44.1k, 48k
            detail: 0x07,           // 16, 20, 24 bit
        };
        let bytes = original.to_bytes();
        let parsed = ShortAudioDescriptor::from_bytes(&bytes);
        assert_eq!(parsed.format_code, original.format_code);
        assert_eq!(parsed.max_channels, original.max_channels);
        assert_eq!(parsed.sample_rate_mask, original.sample_rate_mask);
        assert_eq!(parsed.detail, original.detail);
    }

    // --- HdmiSampleRate tests ---

    #[test]
    fn test_hdmi_sample_rate_from_hz() {
        assert_eq!(HdmiSampleRate::from_hz(48000), HdmiSampleRate::Rate48000);
        assert_eq!(HdmiSampleRate::from_hz(44100), HdmiSampleRate::Rate44100);
        assert_eq!(HdmiSampleRate::from_hz(192000), HdmiSampleRate::Rate192000);
        assert_eq!(HdmiSampleRate::from_hz(12345), HdmiSampleRate::StreamHeader);
    }

    #[test]
    fn test_hdmi_sample_rate_to_hz() {
        assert_eq!(HdmiSampleRate::Rate48000.to_hz(), 48000);
        assert_eq!(HdmiSampleRate::Rate44100.to_hz(), 44100);
        assert_eq!(HdmiSampleRate::StreamHeader.to_hz(), 0);
    }

    // --- Standard sample rates ---

    #[test]
    fn test_standard_sample_rates() {
        assert!(is_standard_sample_rate(44100));
        assert!(is_standard_sample_rate(48000));
        assert!(is_standard_sample_rate(96000));
        assert!(!is_standard_sample_rate(12345));
    }

    // --- UsbAudioDevice tests ---

    #[test]
    fn test_usb_audio_device_find_units() {
        let mut dev = UsbAudioDevice::new(1, UacVersion::Uac10);

        dev.units.push(AudioUnit::InputTerminal(InputTerminal {
            terminal_id: 1,
            terminal_type: UAC_TERMINAL_MICROPHONE,
            assoc_terminal: 0,
            nr_channels: 1,
            channel_config: 0x01,
        }));

        dev.units.push(AudioUnit::FeatureUnit(FeatureUnit {
            unit_id: 2,
            source_id: 1,
            controls: vec![0x03],
        }));

        dev.units.push(AudioUnit::OutputTerminal(OutputTerminal {
            terminal_id: 3,
            terminal_type: UAC_TERMINAL_SPEAKER,
            assoc_terminal: 0,
            source_id: 2,
        }));

        assert!(dev.find_input_terminal(1).is_some());
        assert!(dev.find_input_terminal(99).is_none());
        assert!(dev.find_output_terminal(3).is_some());
        assert!(dev.find_feature_unit(2).is_some());
        assert_eq!(dev.feature_units().len(), 1);
        assert_eq!(dev.unit_count(), 3);
    }

    // --- AudioUnit ID test ---

    #[test]
    fn test_audio_unit_id() {
        let unit = AudioUnit::FeatureUnit(FeatureUnit {
            unit_id: 42,
            source_id: 1,
            controls: Vec::new(),
        });
        assert_eq!(unit.id(), 42);

        let unit2 = AudioUnit::ClockSource(ClockSource {
            clock_id: 7,
            attributes: 0x01,
            assoc_terminal: 0,
        });
        assert_eq!(unit2.id(), 7);
    }

    // --- Clock source tests ---

    #[test]
    fn test_clock_source_attributes() {
        let clock = ClockSource {
            clock_id: 1,
            attributes: 0x03, // external + SOF synced
            assoc_terminal: 0,
        };
        assert!(clock.is_external());
        assert!(clock.is_sof_synced());

        let internal = ClockSource {
            clock_id: 2,
            attributes: 0x00,
            assoc_terminal: 0,
        };
        assert!(!internal.is_external());
        assert!(!internal.is_sof_synced());
    }

    // --- Streaming interface tests ---

    #[test]
    fn test_streaming_interface_direction() {
        let output = AudioStreamingInterface {
            interface_num: 1,
            alternate_setting: 1,
            terminal_link: 1,
            format: AudioFormatDescriptor {
                format_tag: UAC_FORMAT_PCM,
                nr_channels: 2,
                subframe_size: 2,
                bit_resolution: 16,
                sample_rates: Vec::new(),
            },
            endpoint_address: 0x01, // OUT endpoint
            max_packet_size: 192,
            sync_type: IsoSyncType::Adaptive,
            sync_endpoint: 0,
        };
        assert!(output.is_output());
        assert!(!output.is_input());

        let input = AudioStreamingInterface {
            endpoint_address: 0x82, // IN endpoint (bit 7 set)
            ..output.clone()
        };
        assert!(input.is_input());
        assert!(!input.is_output());
    }

    #[test]
    fn test_streaming_bytes_per_frame() {
        let stream = AudioStreamingInterface {
            interface_num: 1,
            alternate_setting: 1,
            terminal_link: 1,
            format: AudioFormatDescriptor {
                format_tag: UAC_FORMAT_PCM,
                nr_channels: 2,
                subframe_size: 2,
                bit_resolution: 16,
                sample_rates: Vec::new(),
            },
            endpoint_address: 0x01,
            max_packet_size: 192,
            sync_type: IsoSyncType::Adaptive,
            sync_endpoint: 0,
        };
        // 48000 Hz * 4 bytes/frame / 1000 = 192 bytes per USB frame
        assert_eq!(stream.bytes_per_usb_frame(48000), 192);
        // 44100 Hz * 4 / 1000 = 176 (truncated)
        assert_eq!(stream.bytes_per_usb_frame(44100), 176);
    }

    // --- HdmiAudioOutput tests ---

    #[test]
    fn test_hdmi_audio_output_lifecycle() {
        let mut output = HdmiAudioOutput::new(0, 0x02, 0x05);
        assert!(!output.has_sink());
        assert!(!output.enabled);

        output.enable();
        assert!(output.enabled);
        output.disable();
        assert!(!output.enabled);
    }

    // --- Mute control request tests ---

    #[test]
    fn test_mute_control_request() {
        let req = build_set_mute_request(2, 0, 0, true);
        assert_eq!(req.request, UAC_SET_CUR);
        assert!(req.muted);
        assert_eq!(req.value >> 8, UAC_FU_MUTE_CONTROL as u16);
    }

    // --- read_u24_le tests ---

    #[test]
    fn test_read_u24_le() {
        assert_eq!(read_u24_le(&[0x44, 0xAC, 0x00]), 44100);
        assert_eq!(read_u24_le(&[0x80, 0xBB, 0x00]), 48000);
        assert_eq!(read_u24_le(&[0x00, 0xEE, 0x02]), 192000);
    }
}
