//! Audio subsystem for VeridianOS
//!
//! Provides audio mixing, playback, and device management:
//! - Fixed-point 16.16 audio mixer (no FPU required)
//! - Ring buffer transport for audio streams
//! - WAV file parsing (PCM formats)
//! - Client API for creating and managing audio streams
//! - Output pipeline with underrun detection
//! - VirtIO-Sound driver for paravirtualized audio

#![allow(dead_code)]

pub mod alsa;
pub mod buffer;
pub mod client;
pub mod codecs;
pub mod mixer;
pub mod pipeline;
pub mod usb_audio;
pub(crate) mod virtio_sound;
pub mod wav;

use alloc::{string::String, vec::Vec};

use crate::error::KernelError;

// ============================================================================
// Unified Audio Error Type
// ============================================================================

/// Common audio error type for all audio backends
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum AudioError {
    /// Requested device was not found
    DeviceNotFound,
    /// Device is in use by another client
    DeviceBusy,
    /// Configuration is invalid
    InvalidConfig { reason: &'static str },
    /// Capture buffer overrun -- data was lost
    BufferOverrun,
    /// Playback buffer underrun -- device starved
    BufferUnderrun,
    /// Device has not been started
    NotStarted,
    /// Device is already running
    AlreadyStarted,
    /// Requested format is not supported by this device
    UnsupportedFormat,
    /// Low-level I/O error communicating with hardware
    IoError,
}

impl core::fmt::Display for AudioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AudioError::DeviceNotFound => write!(f, "audio device not found"),
            AudioError::DeviceBusy => write!(f, "audio device busy"),
            AudioError::InvalidConfig { reason } => {
                write!(f, "invalid audio config: {}", reason)
            }
            AudioError::BufferOverrun => write!(f, "audio buffer overrun"),
            AudioError::BufferUnderrun => write!(f, "audio buffer underrun"),
            AudioError::NotStarted => write!(f, "audio device not started"),
            AudioError::AlreadyStarted => write!(f, "audio device already started"),
            AudioError::UnsupportedFormat => write!(f, "unsupported audio format"),
            AudioError::IoError => write!(f, "audio I/O error"),
        }
    }
}

// ============================================================================
// Device Capabilities
// ============================================================================

/// Describes the hardware capabilities of an audio device
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AudioDeviceCapabilities {
    /// Minimum supported sample rate in Hz
    pub min_sample_rate: u32,
    /// Maximum supported sample rate in Hz
    pub max_sample_rate: u32,
    /// Minimum supported channel count
    pub min_channels: u8,
    /// Maximum supported channel count
    pub max_channels: u8,
    /// List of supported sample formats
    pub supported_formats: Vec<SampleFormat>,
    /// Whether the device supports playback
    pub playback: bool,
    /// Whether the device supports capture
    pub capture: bool,
}

// ============================================================================
// Unified Audio Device Trait
// ============================================================================

/// Unified audio device trait for playback and capture backends
///
/// Provides a common interface across ALSA PCM devices, VirtIO Sound streams,
/// USB Audio Class devices, and any future audio backends. Implementations
/// adapt existing backend-specific APIs to this shared contract.
#[allow(dead_code)]
pub trait AudioDevice {
    /// Configure the device with desired parameters.
    ///
    /// The device may adjust parameters to the nearest supported values.
    /// Returns the actual configuration that was applied.
    fn configure(&mut self, config: &AudioConfig) -> Result<AudioConfig, AudioError>;

    /// Start playback or capture.
    fn start(&mut self) -> Result<(), AudioError>;

    /// Stop playback or capture.
    fn stop(&mut self) -> Result<(), AudioError>;

    /// Write PCM samples for playback.
    ///
    /// Returns the number of frames written. The data must be interleaved
    /// samples in the format specified by the current configuration.
    fn write_frames(&mut self, data: &[u8]) -> Result<usize, AudioError>;

    /// Read PCM samples from capture.
    ///
    /// Returns the number of frames read. The output buffer is filled with
    /// interleaved samples in the format specified by the current
    /// configuration.
    fn read_frames(&mut self, output: &mut [u8]) -> Result<usize, AudioError>;

    /// Query device capabilities.
    fn capabilities(&self) -> &AudioDeviceCapabilities;

    /// Human-readable device name.
    fn name(&self) -> &str;

    /// Returns true if this device supports playback.
    fn is_playback(&self) -> bool;

    /// Returns true if this device supports capture.
    fn is_capture(&self) -> bool;
}

/// Audio sample format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit PCM
    U8,
    /// Signed 16-bit little-endian PCM
    S16Le,
    /// Signed 16-bit big-endian PCM
    S16Be,
    /// Signed 24-bit little-endian PCM (packed in 3 bytes)
    S24Le,
    /// Signed 32-bit little-endian PCM
    S32Le,
    /// 32-bit float stored as i32 fixed-point (16.16 format)
    F32,
}

impl SampleFormat {
    /// Returns the size in bytes of a single sample in this format
    pub fn bytes_per_sample(&self) -> usize {
        match self {
            SampleFormat::U8 => 1,
            SampleFormat::S16Le | SampleFormat::S16Be => 2,
            SampleFormat::S24Le => 3,
            SampleFormat::S32Le | SampleFormat::F32 => 4,
        }
    }
}

/// Audio stream configuration
#[derive(Debug, Clone, Copy)]
pub struct AudioConfig {
    /// Sample rate in Hz (e.g., 44100, 48000)
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo)
    pub channels: u8,
    /// Sample format
    pub format: SampleFormat,
    /// Number of frames per buffer period
    pub buffer_frames: u32,
}

impl AudioConfig {
    /// Create a default stereo 16-bit 48kHz configuration
    pub fn default_config() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            format: SampleFormat::S16Le,
            buffer_frames: 1024,
        }
    }

    /// Calculate the size of a single frame in bytes (channels * sample_size)
    pub fn frame_size(&self) -> u16 {
        (self.channels as u16) * (self.format.bytes_per_sample() as u16)
    }

    /// Calculate the byte rate (sample_rate * frame_size)
    pub fn byte_rate(&self) -> u32 {
        self.sample_rate * self.frame_size() as u32
    }
}

/// Audio routing entry connecting a source to a sink
#[derive(Debug, Clone, Copy)]
pub struct AudioRoute {
    /// Source stream/channel identifier
    pub source_id: u16,
    /// Sink stream/device identifier
    pub sink_id: u16,
    /// Volume level: 0..=65535 maps to 0.0..1.0 in fixed-point
    pub volume: u16,
}

/// Information about an audio device
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    /// Unique device identifier
    pub id: u16,
    /// Human-readable device name
    pub name: String,
    /// Whether this device supports audio output (playback)
    pub is_output: bool,
    /// Whether this device supports audio input (capture)
    pub is_input: bool,
    /// Current device configuration
    pub config: AudioConfig,
}

/// Initialize the audio subsystem
///
/// Sets up the mixer, output pipeline, client manager, and probes for
/// VirtIO-Sound hardware.
pub fn init() -> Result<(), KernelError> {
    println!("[AUDIO] Initializing audio subsystem...");

    // Initialize the mixer at 48kHz
    mixer::init(48000)?;

    // Initialize the output pipeline with default config
    pipeline::init(AudioConfig::default_config())?;

    // Initialize the client manager
    client::init();

    // Probe for VirtIO-Sound hardware (non-fatal if absent)
    if let Err(_e) = virtio_sound::init() {
        println!("[AUDIO] No VirtIO-Sound device found (non-fatal)");
    }

    println!("[AUDIO] Audio subsystem initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_format_size() {
        assert_eq!(SampleFormat::U8.bytes_per_sample(), 1);
        assert_eq!(SampleFormat::S16Le.bytes_per_sample(), 2);
        assert_eq!(SampleFormat::S16Be.bytes_per_sample(), 2);
        assert_eq!(SampleFormat::S24Le.bytes_per_sample(), 3);
        assert_eq!(SampleFormat::S32Le.bytes_per_sample(), 4);
        assert_eq!(SampleFormat::F32.bytes_per_sample(), 4);
    }

    #[test]
    fn test_audio_config_frame_size() {
        let config = AudioConfig {
            sample_rate: 44100,
            channels: 2,
            format: SampleFormat::S16Le,
            buffer_frames: 1024,
        };
        assert_eq!(config.frame_size(), 4); // 2 channels * 2 bytes
        assert_eq!(config.byte_rate(), 44100 * 4);
    }

    #[test]
    fn test_default_config() {
        let config = AudioConfig::default_config();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.format, SampleFormat::S16Le);
        assert_eq!(config.buffer_frames, 1024);
    }
}
