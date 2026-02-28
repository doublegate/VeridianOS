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

pub mod buffer;
pub mod client;
pub mod mixer;
pub mod pipeline;
pub mod wav;
mod virtio_sound;

use alloc::string::String;

use crate::error::KernelError;

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
