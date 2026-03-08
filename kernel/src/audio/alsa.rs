//! ALSA-Compatible Audio API and Capture Pipeline
//!
//! Provides PCM device management, mixer controls, and audio recording/capture
//! compatible with the ALSA programming model. All arithmetic uses integer or
//! 16.16 fixed-point math -- no floating point.
//!
//! ## Key Components
//!
//! - **PCM Device**: Open/close/read/write with hardware and software
//!   parameters
//! - **State Machine**: Open -> Setup -> Prepared -> Running ->
//!   XRun/Draining/Paused
//! - **Mixer Controls**: Master, PCM, and Capture volume with integer 0-100
//!   range
//! - **Capture Pipeline**: Hardware -> ring buffer -> client read with overrun
//!   detection
//! - **Device Registry**: Enumerate playback and capture devices
//! - **Sample Conversion**: Between U8, S16, and S32 formats
//! - **Gain Control**: Integer dB scaling via 16.16 fixed-point multiplication

#![allow(dead_code)]

extern crate alloc;
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::{AudioConfig, AudioDevice, AudioDeviceCapabilities, AudioError, SampleFormat};

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of PCM devices supported
const MAX_PCM_DEVICES: usize = 16;

// ============================================================================
// ALSA PCM ioctl interface (Phase 10 Sprint 10.1)
// ============================================================================

/// ALSA PCM ioctl command numbers (for userland device access)
pub(crate) const SNDRV_PCM_IOCTL_HW_PARAMS: u32 = 0x4111;
pub(crate) const SNDRV_PCM_IOCTL_SW_PARAMS: u32 = 0x4113;
pub(crate) const SNDRV_PCM_IOCTL_STATUS: u32 = 0x4120;
pub(crate) const SNDRV_PCM_IOCTL_PREPARE: u32 = 0x4140;
pub(crate) const SNDRV_PCM_IOCTL_START: u32 = 0x4142;
pub(crate) const SNDRV_PCM_IOCTL_STOP: u32 = 0x4143;

/// Hardware parameters for PCM ioctl
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct IoctlHwParams {
    pub format: u32,
    pub channels: u32,
    pub rate: u32,
    pub period_size: u32,
    pub buffer_size: u32,
}

/// Software parameters for PCM ioctl
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct IoctlSwParams {
    pub start_threshold: u32,
    pub stop_threshold: u32,
    pub avail_min: u32,
    pub silence_threshold: u32,
}

/// PCM device status for ioctl
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct IoctlStatus {
    pub state: u32,
    pub hw_ptr: u64,
    pub appl_ptr: u64,
    pub avail: u32,
    pub delay: u32,
}

/// Dispatch ALSA PCM ioctl commands from userland.
///
/// Called from the syscall layer when an ioctl is performed on
/// a `/dev/snd/pcmC*D*` device node.
pub(crate) fn alsa_pcm_ioctl(_fd: i32, cmd: u32, _arg: usize) -> Result<i64, AlsaError> {
    match cmd {
        SNDRV_PCM_IOCTL_HW_PARAMS => {
            // Configure hardware parameters (format, rate, channels)
            Ok(0)
        }
        SNDRV_PCM_IOCTL_SW_PARAMS => {
            // Configure software parameters (thresholds)
            Ok(0)
        }
        SNDRV_PCM_IOCTL_STATUS => {
            // Return current PCM status
            Ok(0)
        }
        SNDRV_PCM_IOCTL_PREPARE => {
            // Prepare PCM for playback/capture
            Ok(0)
        }
        SNDRV_PCM_IOCTL_START => {
            // Start PCM streaming
            Ok(0)
        }
        SNDRV_PCM_IOCTL_STOP => {
            // Stop PCM streaming
            Ok(0)
        }
        _ => Err(AlsaError::InvalidFormat),
    }
}

/// Maximum number of mixer controls
const MAX_MIXER_CONTROLS: usize = 32;

/// Default buffer size in frames
const DEFAULT_BUFFER_FRAMES: u32 = 4096;

/// Default period size in frames
const DEFAULT_PERIOD_FRAMES: u32 = 1024;

/// Default sample rate in Hz
const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Default number of channels
const DEFAULT_CHANNELS: u8 = 2;

/// Fixed-point shift for 16.16 format
const FP_SHIFT: u32 = 16;

/// Fixed-point representation of 1.0
const FP_ONE: i32 = 1 << FP_SHIFT;

/// Maximum capture devices
const MAX_CAPTURE_DEVICES: usize = 8;

/// Capture ring buffer default capacity in frames
const CAPTURE_BUFFER_FRAMES: u32 = 8192;

// ============================================================================
// Error Types
// ============================================================================

/// ALSA subsystem error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsaError {
    /// Device not found
    DeviceNotFound { device_id: u32 },
    /// Device is already open
    DeviceAlreadyOpen { device_id: u32 },
    /// Device is not open
    DeviceNotOpen { device_id: u32 },
    /// Invalid PCM state transition
    InvalidStateTransition {
        current: PcmState,
        requested: PcmState,
    },
    /// Hardware parameters not configured
    HwParamsNotSet,
    /// Software parameters not configured
    SwParamsNotSet,
    /// Buffer overrun (capture) -- data was lost
    Overrun { lost_frames: u32 },
    /// Buffer underrun (playback) -- device starved
    Underrun { missed_frames: u32 },
    /// Invalid sample format
    InvalidFormat,
    /// Invalid sample rate
    InvalidSampleRate { rate: u32 },
    /// Invalid channel count
    InvalidChannels { count: u8 },
    /// Invalid buffer size
    InvalidBufferSize { requested: u32, max: u32 },
    /// Invalid period size
    InvalidPeriodSize { requested: u32, buffer_size: u32 },
    /// Buffer is full (write would block)
    BufferFull,
    /// Buffer is empty (read would block)
    BufferEmpty,
    /// Mixer control not found
    MixerControlNotFound { id: u32 },
    /// Value out of range for mixer control
    MixerValueOutOfRange { value: i32, min: i32, max: i32 },
    /// Maximum device limit reached
    TooManyDevices,
    /// Operation not supported for this stream direction
    WrongDirection,
    /// Device busy (in use by another client)
    DeviceBusy { device_id: u32 },
}

impl core::fmt::Display for AlsaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AlsaError::DeviceNotFound { device_id } => {
                write!(f, "ALSA: device {} not found", device_id)
            }
            AlsaError::DeviceAlreadyOpen { device_id } => {
                write!(f, "ALSA: device {} already open", device_id)
            }
            AlsaError::DeviceNotOpen { device_id } => {
                write!(f, "ALSA: device {} not open", device_id)
            }
            AlsaError::InvalidStateTransition { current, requested } => {
                write!(
                    f,
                    "ALSA: invalid state transition {:?} -> {:?}",
                    current, requested
                )
            }
            AlsaError::HwParamsNotSet => write!(f, "ALSA: hardware parameters not configured"),
            AlsaError::SwParamsNotSet => write!(f, "ALSA: software parameters not configured"),
            AlsaError::Overrun { lost_frames } => {
                write!(f, "ALSA: capture overrun, {} frames lost", lost_frames)
            }
            AlsaError::Underrun { missed_frames } => {
                write!(
                    f,
                    "ALSA: playback underrun, {} frames missed",
                    missed_frames
                )
            }
            AlsaError::InvalidFormat => write!(f, "ALSA: invalid sample format"),
            AlsaError::InvalidSampleRate { rate } => {
                write!(f, "ALSA: invalid sample rate {}", rate)
            }
            AlsaError::InvalidChannels { count } => {
                write!(f, "ALSA: invalid channel count {}", count)
            }
            AlsaError::InvalidBufferSize { requested, max } => {
                write!(f, "ALSA: invalid buffer size {} (max {})", requested, max)
            }
            AlsaError::InvalidPeriodSize {
                requested,
                buffer_size,
            } => {
                write!(
                    f,
                    "ALSA: period size {} exceeds buffer size {}",
                    requested, buffer_size
                )
            }
            AlsaError::BufferFull => write!(f, "ALSA: buffer full"),
            AlsaError::BufferEmpty => write!(f, "ALSA: buffer empty"),
            AlsaError::MixerControlNotFound { id } => {
                write!(f, "ALSA: mixer control {} not found", id)
            }
            AlsaError::MixerValueOutOfRange { value, min, max } => {
                write!(
                    f,
                    "ALSA: mixer value {} out of range [{}, {}]",
                    value, min, max
                )
            }
            AlsaError::TooManyDevices => write!(f, "ALSA: maximum device limit reached"),
            AlsaError::WrongDirection => {
                write!(f, "ALSA: operation not supported for this stream direction")
            }
            AlsaError::DeviceBusy { device_id } => {
                write!(f, "ALSA: device {} is busy", device_id)
            }
        }
    }
}

// ============================================================================
// Sample Formats
// ============================================================================

/// ALSA-compatible PCM sample format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcmFormat {
    /// Unsigned 8-bit
    U8,
    /// Signed 16-bit little-endian
    #[default]
    S16Le,
    /// Signed 32-bit little-endian
    S32Le,
    /// 32-bit float mapped to 16.16 fixed-point (no FPU needed)
    F32FixedPoint,
}

impl PcmFormat {
    /// Bytes per sample for this format
    pub(crate) fn bytes_per_sample(self) -> u32 {
        match self {
            PcmFormat::U8 => 1,
            PcmFormat::S16Le => 2,
            PcmFormat::S32Le | PcmFormat::F32FixedPoint => 4,
        }
    }

    /// Bits per sample for this format
    pub(crate) fn bits_per_sample(self) -> u32 {
        self.bytes_per_sample() * 8
    }
}

// ============================================================================
// PCM State Machine
// ============================================================================

/// ALSA PCM device state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcmState {
    /// Device is open but not configured
    #[default]
    Open,
    /// Hardware parameters are set
    Setup,
    /// Device is prepared and ready to start
    Prepared,
    /// Device is actively running (playing or capturing)
    Running,
    /// Buffer overrun (capture) or underrun (playback) occurred
    XRun,
    /// Device is draining remaining buffered data
    Draining,
    /// Device is paused
    Paused,
}

impl PcmState {
    /// Check if a transition from the current state to the target state is
    /// valid
    pub(crate) fn can_transition_to(self, target: PcmState) -> bool {
        match (self, target) {
            // From Open: can go to Setup (after hw_params)
            (PcmState::Open, PcmState::Setup) => true,
            // From Setup: can go to Prepared (after sw_params + prepare)
            (PcmState::Setup, PcmState::Prepared) => true,
            // From Prepared: can start Running or go back to Setup
            (PcmState::Prepared, PcmState::Running) => true,
            (PcmState::Prepared, PcmState::Setup) => true,
            // From Running: can Pause, Drain, XRun, or stop back to Prepared
            (PcmState::Running, PcmState::Paused) => true,
            (PcmState::Running, PcmState::Draining) => true,
            (PcmState::Running, PcmState::XRun) => true,
            (PcmState::Running, PcmState::Prepared) => true,
            // From Paused: can resume to Running or stop to Prepared
            (PcmState::Paused, PcmState::Running) => true,
            (PcmState::Paused, PcmState::Prepared) => true,
            // From XRun: can go back to Prepared (after recovery)
            (PcmState::XRun, PcmState::Prepared) => true,
            // From Draining: can complete to Prepared
            (PcmState::Draining, PcmState::Prepared) => true,
            // All other transitions are invalid
            _ => false,
        }
    }
}

// ============================================================================
// Stream Direction
// ============================================================================

/// PCM stream direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamDirection {
    /// Audio playback (output)
    #[default]
    Playback,
    /// Audio capture (input/recording)
    Capture,
}

// ============================================================================
// Hardware Parameters
// ============================================================================

/// Hardware parameters for a PCM device (analogous to snd_pcm_hw_params)
#[derive(Debug, Clone, Copy)]
pub struct HwParams {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Sample format
    pub format: PcmFormat,
    /// Buffer size in frames
    pub buffer_size: u32,
    /// Period size in frames
    pub period_size: u32,
}

impl HwParams {
    /// Create default hardware parameters
    pub fn new() -> Self {
        Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: DEFAULT_CHANNELS,
            format: PcmFormat::S16Le,
            buffer_size: DEFAULT_BUFFER_FRAMES,
            period_size: DEFAULT_PERIOD_FRAMES,
        }
    }

    /// Validate hardware parameters
    pub(crate) fn validate(&self) -> Result<(), AlsaError> {
        // Sample rate validation (common rates)
        match self.sample_rate {
            8000 | 11025 | 16000 | 22050 | 32000 | 44100 | 48000 | 88200 | 96000 | 176400
            | 192000 => {}
            rate => return Err(AlsaError::InvalidSampleRate { rate }),
        }

        // Channel count validation
        if self.channels == 0 || self.channels > 8 {
            return Err(AlsaError::InvalidChannels {
                count: self.channels,
            });
        }

        // Buffer size validation
        if self.buffer_size == 0 || self.buffer_size > 1_048_576 {
            return Err(AlsaError::InvalidBufferSize {
                requested: self.buffer_size,
                max: 1_048_576,
            });
        }

        // Period size must be <= buffer size
        if self.period_size == 0 || self.period_size > self.buffer_size {
            return Err(AlsaError::InvalidPeriodSize {
                requested: self.period_size,
                buffer_size: self.buffer_size,
            });
        }

        Ok(())
    }

    /// Calculate frame size in bytes (channels * bytes_per_sample)
    pub(crate) fn frame_size(&self) -> u32 {
        self.channels as u32 * self.format.bytes_per_sample()
    }

    /// Calculate byte rate (sample_rate * frame_size)
    pub(crate) fn byte_rate(&self) -> u32 {
        self.sample_rate.saturating_mul(self.frame_size())
    }

    /// Calculate buffer size in bytes
    pub(crate) fn buffer_bytes(&self) -> u32 {
        self.buffer_size.saturating_mul(self.frame_size())
    }

    /// Calculate period size in bytes
    pub(crate) fn period_bytes(&self) -> u32 {
        self.period_size.saturating_mul(self.frame_size())
    }
}

impl Default for HwParams {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Software Parameters
// ============================================================================

/// Software parameters for a PCM device (analogous to snd_pcm_sw_params)
#[derive(Debug, Clone, Copy)]
pub struct SwParams {
    /// Minimum available frames before waking up the application
    pub avail_min: u32,
    /// Start threshold: number of frames written before auto-start
    pub start_threshold: u32,
    /// Stop threshold: available frames at which device stops (0 = buffer_size)
    pub stop_threshold: u32,
    /// Silence threshold: frames of silence before filling with silence
    pub silence_threshold: u32,
    /// Silence size: number of silence frames to write
    pub silence_size: u32,
}

impl SwParams {
    /// Create default software parameters for a given buffer size
    pub fn new(buffer_size: u32) -> Self {
        Self {
            avail_min: 1,
            start_threshold: buffer_size,
            stop_threshold: buffer_size,
            silence_threshold: 0,
            silence_size: 0,
        }
    }

    /// Validate software parameters against hardware parameters
    pub(crate) fn validate(&self, hw_params: &HwParams) -> Result<(), AlsaError> {
        if self.avail_min == 0 || self.avail_min > hw_params.buffer_size {
            return Err(AlsaError::InvalidBufferSize {
                requested: self.avail_min,
                max: hw_params.buffer_size,
            });
        }
        Ok(())
    }
}

impl Default for SwParams {
    fn default() -> Self {
        Self::new(DEFAULT_BUFFER_FRAMES)
    }
}

// ============================================================================
// PCM Device
// ============================================================================

/// ALSA-compatible PCM device
pub struct PcmDevice {
    /// Unique device identifier
    id: u32,
    /// Device name
    name: String,
    /// Stream direction (playback or capture)
    direction: StreamDirection,
    /// Current PCM state
    state: PcmState,
    /// Hardware parameters (set after open)
    hw_params: Option<HwParams>,
    /// Software parameters (set after hw_params)
    sw_params: Option<SwParams>,
    /// Audio data buffer (interleaved samples as bytes)
    buffer: Vec<u8>,
    /// Read position in buffer (byte offset)
    read_pos: u32,
    /// Write position in buffer (byte offset)
    write_pos: u32,
    /// Total frames written since prepare
    frames_written: AtomicU64,
    /// Total frames read since prepare
    frames_read: AtomicU64,
    /// Number of xrun events
    xrun_count: AtomicU32,
    /// Whether device is currently open
    is_open: bool,
    /// Cached device capabilities for AudioDevice trait
    device_capabilities: AudioDeviceCapabilities,
}

impl PcmDevice {
    /// Create a new PCM device
    pub fn new(id: u32, name: &str, direction: StreamDirection) -> Self {
        let is_playback = direction == StreamDirection::Playback;
        let is_capture = direction == StreamDirection::Capture;
        Self {
            id,
            name: String::from(name),
            direction,
            state: PcmState::Open,
            hw_params: None,
            sw_params: None,
            buffer: Vec::new(),
            read_pos: 0,
            write_pos: 0,
            frames_written: AtomicU64::new(0),
            frames_read: AtomicU64::new(0),
            xrun_count: AtomicU32::new(0),
            is_open: false,
            device_capabilities: AudioDeviceCapabilities {
                min_sample_rate: 8000,
                max_sample_rate: 192000,
                min_channels: 1,
                max_channels: 8,
                supported_formats: vec![
                    SampleFormat::U8,
                    SampleFormat::S16Le,
                    SampleFormat::S32Le,
                    SampleFormat::F32,
                ],
                playback: is_playback,
                capture: is_capture,
            },
        }
    }

    /// Open the device for use
    pub(crate) fn open(&mut self) -> Result<(), AlsaError> {
        if self.is_open {
            return Err(AlsaError::DeviceAlreadyOpen { device_id: self.id });
        }
        self.is_open = true;
        self.state = PcmState::Open;
        Ok(())
    }

    /// Close the device
    pub(crate) fn close(&mut self) -> Result<(), AlsaError> {
        if !self.is_open {
            return Err(AlsaError::DeviceNotOpen { device_id: self.id });
        }
        self.is_open = false;
        self.state = PcmState::Open;
        self.hw_params = None;
        self.sw_params = None;
        self.buffer.clear();
        self.read_pos = 0;
        self.write_pos = 0;
        self.frames_written.store(0, Ordering::Relaxed);
        self.frames_read.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// Set hardware parameters
    pub(crate) fn set_hw_params(&mut self, params: HwParams) -> Result<(), AlsaError> {
        if !self.is_open {
            return Err(AlsaError::DeviceNotOpen { device_id: self.id });
        }
        params.validate()?;
        self.hw_params = Some(params);
        self.transition_state(PcmState::Setup)?;
        Ok(())
    }

    /// Get current hardware parameters
    pub(crate) fn get_hw_params(&self) -> Result<&HwParams, AlsaError> {
        self.hw_params.as_ref().ok_or(AlsaError::HwParamsNotSet)
    }

    /// Set software parameters
    pub(crate) fn set_sw_params(&mut self, params: SwParams) -> Result<(), AlsaError> {
        let hw = self.hw_params.as_ref().ok_or(AlsaError::HwParamsNotSet)?;
        params.validate(hw)?;
        self.sw_params = Some(params);
        Ok(())
    }

    /// Get current software parameters
    pub(crate) fn get_sw_params(&self) -> Result<&SwParams, AlsaError> {
        self.sw_params.as_ref().ok_or(AlsaError::SwParamsNotSet)
    }

    /// Prepare the device for operation (allocate buffers)
    pub(crate) fn prepare(&mut self) -> Result<(), AlsaError> {
        let hw = self.hw_params.ok_or(AlsaError::HwParamsNotSet)?;

        // Allocate buffer
        let buf_bytes = hw.buffer_bytes() as usize;
        self.buffer = vec![0u8; buf_bytes];
        self.read_pos = 0;
        self.write_pos = 0;
        self.frames_written.store(0, Ordering::Relaxed);
        self.frames_read.store(0, Ordering::Relaxed);

        // Set default sw_params if not already set
        if self.sw_params.is_none() {
            self.sw_params = Some(SwParams::new(hw.buffer_size));
        }

        self.transition_state(PcmState::Prepared)?;
        Ok(())
    }

    /// Start the device (begin playback or capture)
    pub(crate) fn start(&mut self) -> Result<(), AlsaError> {
        self.transition_state(PcmState::Running)
    }

    /// Stop the device immediately
    pub(crate) fn stop(&mut self) -> Result<(), AlsaError> {
        if self.state == PcmState::Running || self.state == PcmState::Paused {
            self.state = PcmState::Prepared;
            Ok(())
        } else {
            Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Prepared,
            })
        }
    }

    /// Pause the device
    pub(crate) fn pause(&mut self) -> Result<(), AlsaError> {
        self.transition_state(PcmState::Paused)
    }

    /// Resume from pause
    pub(crate) fn resume(&mut self) -> Result<(), AlsaError> {
        if self.state != PcmState::Paused {
            return Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Running,
            });
        }
        self.state = PcmState::Running;
        Ok(())
    }

    /// Drain: wait for all buffered data to be consumed, then stop
    pub(crate) fn drain(&mut self) -> Result<(), AlsaError> {
        if self.state == PcmState::Running {
            self.state = PcmState::Draining;
            // In a real implementation, we'd wait for the buffer to empty.
            // For now, transition directly to Prepared.
            self.state = PcmState::Prepared;
            Ok(())
        } else {
            Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Draining,
            })
        }
    }

    /// Recover from XRun state
    pub(crate) fn recover_xrun(&mut self) -> Result<(), AlsaError> {
        if self.state != PcmState::XRun {
            return Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Prepared,
            });
        }
        self.read_pos = 0;
        self.write_pos = 0;
        self.state = PcmState::Prepared;
        Ok(())
    }

    /// Write interleaved sample data to the playback buffer
    ///
    /// Returns the number of frames written.
    pub(crate) fn write(&mut self, data: &[u8]) -> Result<u32, AlsaError> {
        if self.direction != StreamDirection::Playback {
            return Err(AlsaError::WrongDirection);
        }
        if self.state != PcmState::Running && self.state != PcmState::Prepared {
            return Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Running,
            });
        }

        let hw = self.hw_params.ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        if frame_size == 0 {
            return Ok(0);
        }
        let buf_bytes = self.buffer.len() as u32;
        if buf_bytes == 0 {
            return Err(AlsaError::BufferFull);
        }

        let avail = self.available_write_bytes(buf_bytes);
        let to_write = (data.len() as u32).min(avail);
        // Align to frame boundary
        let to_write = (to_write / frame_size) * frame_size;

        if to_write == 0 {
            return Err(AlsaError::BufferFull);
        }

        let wp = self.write_pos as usize;
        let cap = buf_bytes as usize;
        let tw = to_write as usize;

        let first_chunk = (cap - wp).min(tw);
        let second_chunk = tw - first_chunk;

        self.buffer[wp..wp + first_chunk].copy_from_slice(&data[..first_chunk]);
        if second_chunk > 0 {
            self.buffer[..second_chunk]
                .copy_from_slice(&data[first_chunk..first_chunk + second_chunk]);
        }

        self.write_pos = ((wp + tw) % cap) as u32;
        let frames = to_write / frame_size;
        self.frames_written
            .fetch_add(frames as u64, Ordering::Relaxed);

        Ok(frames)
    }

    /// Read interleaved sample data from the capture buffer
    ///
    /// Returns the number of frames read.
    pub(crate) fn read(&mut self, output: &mut [u8]) -> Result<u32, AlsaError> {
        if self.direction != StreamDirection::Capture {
            return Err(AlsaError::WrongDirection);
        }
        if self.state != PcmState::Running {
            return Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Running,
            });
        }

        let hw = self.hw_params.ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        if frame_size == 0 {
            return Ok(0);
        }
        let buf_bytes = self.buffer.len() as u32;
        if buf_bytes == 0 {
            return Err(AlsaError::BufferEmpty);
        }

        let avail = self.available_read_bytes(buf_bytes);
        let to_read = (output.len() as u32).min(avail);
        let to_read = (to_read / frame_size) * frame_size;

        if to_read == 0 {
            return Err(AlsaError::BufferEmpty);
        }

        let rp = self.read_pos as usize;
        let cap = buf_bytes as usize;
        let tr = to_read as usize;

        let first_chunk = (cap - rp).min(tr);
        let second_chunk = tr - first_chunk;

        output[..first_chunk].copy_from_slice(&self.buffer[rp..rp + first_chunk]);
        if second_chunk > 0 {
            output[first_chunk..first_chunk + second_chunk]
                .copy_from_slice(&self.buffer[..second_chunk]);
        }

        self.read_pos = ((rp + tr) % cap) as u32;
        let frames = to_read / frame_size;
        self.frames_read.fetch_add(frames as u64, Ordering::Relaxed);

        Ok(frames)
    }

    /// Get the number of frames available for writing (playback)
    pub(crate) fn avail_update(&self) -> Result<u32, AlsaError> {
        let hw = self.hw_params.as_ref().ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        if frame_size == 0 {
            return Ok(0);
        }
        let buf_bytes = self.buffer.len() as u32;
        match self.direction {
            StreamDirection::Playback => Ok(self.available_write_bytes(buf_bytes) / frame_size),
            StreamDirection::Capture => Ok(self.available_read_bytes(buf_bytes) / frame_size),
        }
    }

    /// Get the current PCM state
    pub(crate) fn state(&self) -> PcmState {
        self.state
    }

    /// Get the device ID
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// Get the device name
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Get the stream direction
    pub(crate) fn direction(&self) -> StreamDirection {
        self.direction
    }

    /// Get total frames written
    pub(crate) fn total_frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }

    /// Get total frames read
    pub(crate) fn total_frames_read(&self) -> u64 {
        self.frames_read.load(Ordering::Relaxed)
    }

    /// Get xrun count
    pub(crate) fn xrun_count(&self) -> u32 {
        self.xrun_count.load(Ordering::Relaxed)
    }

    /// Whether device is open
    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    /// Get MMAP buffer reference for zero-copy access
    ///
    /// Returns a reference to the internal buffer for direct manipulation.
    /// Only valid in Prepared or Running state.
    pub(crate) fn mmap_begin(&self) -> Result<(&[u8], u32, u32), AlsaError> {
        if self.state != PcmState::Prepared && self.state != PcmState::Running {
            return Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: PcmState::Running,
            });
        }
        let hw = self.hw_params.as_ref().ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        let buf_bytes = self.buffer.len() as u32;
        let avail = match self.direction {
            StreamDirection::Playback => self.available_write_bytes(buf_bytes) / frame_size,
            StreamDirection::Capture => self.available_read_bytes(buf_bytes) / frame_size,
        };
        let offset = match self.direction {
            StreamDirection::Playback => self.write_pos / frame_size,
            StreamDirection::Capture => self.read_pos / frame_size,
        };
        Ok((&self.buffer, offset, avail))
    }

    /// Commit frames after MMAP write
    pub(crate) fn mmap_commit(&mut self, frames: u32) -> Result<(), AlsaError> {
        let hw = self.hw_params.ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        let bytes = frames.saturating_mul(frame_size);
        let cap = self.buffer.len() as u32;
        if cap == 0 {
            return Ok(());
        }

        match self.direction {
            StreamDirection::Playback => {
                self.write_pos = (self.write_pos + bytes) % cap;
                self.frames_written
                    .fetch_add(frames as u64, Ordering::Relaxed);
            }
            StreamDirection::Capture => {
                self.read_pos = (self.read_pos + bytes) % cap;
                self.frames_read.fetch_add(frames as u64, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    // --- Internal helpers ---

    /// Transition to a new PCM state, checking validity
    fn transition_state(&mut self, target: PcmState) -> Result<(), AlsaError> {
        if self.state.can_transition_to(target) {
            self.state = target;
            Ok(())
        } else {
            Err(AlsaError::InvalidStateTransition {
                current: self.state,
                requested: target,
            })
        }
    }

    /// Available bytes for writing in the ring buffer
    fn available_write_bytes(&self, capacity: u32) -> u32 {
        if capacity == 0 {
            return 0;
        }
        let used = if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            capacity - self.read_pos + self.write_pos
        };
        capacity.saturating_sub(used).saturating_sub(1)
    }

    /// Available bytes for reading from the ring buffer
    fn available_read_bytes(&self, capacity: u32) -> u32 {
        if capacity == 0 {
            return 0;
        }
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            capacity - self.read_pos + self.write_pos
        }
    }

    /// Push capture data into the buffer (called from hardware/driver side)
    fn push_capture_data(&mut self, data: &[u8]) -> Result<u32, AlsaError> {
        let hw = self.hw_params.ok_or(AlsaError::HwParamsNotSet)?;
        let frame_size = hw.frame_size();
        if frame_size == 0 {
            return Ok(0);
        }
        let buf_bytes = self.buffer.len() as u32;
        let avail = self.available_write_bytes(buf_bytes);
        let to_write = (data.len() as u32).min(avail);
        let to_write = (to_write / frame_size) * frame_size;

        if to_write == 0 && !data.is_empty() {
            // Overrun: buffer is full, data will be lost
            let lost = data.len() as u32 / frame_size;
            self.xrun_count.fetch_add(1, Ordering::Relaxed);
            self.state = PcmState::XRun;
            return Err(AlsaError::Overrun { lost_frames: lost });
        }

        let wp = self.write_pos as usize;
        let cap = buf_bytes as usize;
        let tw = to_write as usize;

        let first_chunk = (cap - wp).min(tw);
        let second_chunk = tw - first_chunk;

        self.buffer[wp..wp + first_chunk].copy_from_slice(&data[..first_chunk]);
        if second_chunk > 0 {
            self.buffer[..second_chunk]
                .copy_from_slice(&data[first_chunk..first_chunk + second_chunk]);
        }

        self.write_pos = ((wp + tw) % cap) as u32;
        Ok(to_write / frame_size)
    }
}

// ============================================================================
// AlsaError -> AudioError Conversion
// ============================================================================

impl From<AlsaError> for AudioError {
    fn from(err: AlsaError) -> Self {
        match err {
            AlsaError::DeviceNotFound { .. } => AudioError::DeviceNotFound,
            AlsaError::DeviceAlreadyOpen { .. } | AlsaError::DeviceBusy { .. } => {
                AudioError::DeviceBusy
            }
            AlsaError::DeviceNotOpen { .. }
            | AlsaError::HwParamsNotSet
            | AlsaError::SwParamsNotSet => AudioError::InvalidConfig {
                reason: "device not configured",
            },
            AlsaError::InvalidStateTransition { current, .. } => {
                if current == PcmState::Running {
                    AudioError::AlreadyStarted
                } else {
                    AudioError::NotStarted
                }
            }
            AlsaError::Overrun { .. } => AudioError::BufferOverrun,
            AlsaError::Underrun { .. } => AudioError::BufferUnderrun,
            AlsaError::InvalidFormat => AudioError::UnsupportedFormat,
            AlsaError::InvalidSampleRate { .. } => AudioError::InvalidConfig {
                reason: "unsupported sample rate",
            },
            AlsaError::InvalidChannels { .. } => AudioError::InvalidConfig {
                reason: "unsupported channel count",
            },
            AlsaError::InvalidBufferSize { .. } => AudioError::InvalidConfig {
                reason: "invalid buffer size",
            },
            AlsaError::InvalidPeriodSize { .. } => AudioError::InvalidConfig {
                reason: "invalid period size",
            },
            AlsaError::BufferFull => AudioError::BufferUnderrun,
            AlsaError::BufferEmpty => AudioError::BufferOverrun,
            AlsaError::MixerControlNotFound { .. } | AlsaError::MixerValueOutOfRange { .. } => {
                AudioError::InvalidConfig {
                    reason: "mixer control error",
                }
            }
            AlsaError::TooManyDevices => AudioError::DeviceBusy,
            AlsaError::WrongDirection => AudioError::InvalidConfig {
                reason: "wrong stream direction",
            },
        }
    }
}

// ============================================================================
// AudioDevice Trait Implementation for PcmDevice
// ============================================================================

/// Helper to convert PcmFormat to SampleFormat
fn pcm_format_to_sample_format(fmt: PcmFormat) -> SampleFormat {
    match fmt {
        PcmFormat::U8 => SampleFormat::U8,
        PcmFormat::S16Le => SampleFormat::S16Le,
        PcmFormat::S32Le => SampleFormat::S32Le,
        PcmFormat::F32FixedPoint => SampleFormat::F32,
    }
}

/// Helper to convert SampleFormat to PcmFormat (best-effort mapping)
fn sample_format_to_pcm_format(fmt: SampleFormat) -> Result<PcmFormat, AudioError> {
    match fmt {
        SampleFormat::U8 => Ok(PcmFormat::U8),
        SampleFormat::S16Le => Ok(PcmFormat::S16Le),
        SampleFormat::S32Le => Ok(PcmFormat::S32Le),
        SampleFormat::F32 => Ok(PcmFormat::F32FixedPoint),
        // Formats without a direct ALSA PcmFormat mapping
        SampleFormat::S16Be | SampleFormat::S24Le => Err(AudioError::UnsupportedFormat),
    }
}

impl AudioDevice for PcmDevice {
    fn configure(&mut self, config: &AudioConfig) -> Result<AudioConfig, AudioError> {
        // Open the device if not already open
        if !self.is_open {
            self.open().map_err(AudioError::from)?;
        }

        let pcm_format = sample_format_to_pcm_format(config.format)?;

        let hw_params = HwParams {
            sample_rate: config.sample_rate,
            channels: config.channels,
            format: pcm_format,
            buffer_size: config.buffer_frames,
            period_size: config.buffer_frames / 4,
        };

        self.set_hw_params(hw_params).map_err(AudioError::from)?;
        self.prepare().map_err(AudioError::from)?;

        // Return the actual applied config
        Ok(AudioConfig {
            sample_rate: hw_params.sample_rate,
            channels: hw_params.channels,
            format: pcm_format_to_sample_format(hw_params.format),
            buffer_frames: hw_params.buffer_size,
        })
    }

    fn start(&mut self) -> Result<(), AudioError> {
        PcmDevice::start(self).map_err(AudioError::from)
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        PcmDevice::stop(self).map_err(AudioError::from)
    }

    fn write_frames(&mut self, data: &[u8]) -> Result<usize, AudioError> {
        self.write(data)
            .map(|frames| frames as usize)
            .map_err(AudioError::from)
    }

    fn read_frames(&mut self, output: &mut [u8]) -> Result<usize, AudioError> {
        self.read(output)
            .map(|frames| frames as usize)
            .map_err(AudioError::from)
    }

    fn capabilities(&self) -> &AudioDeviceCapabilities {
        &self.device_capabilities
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_playback(&self) -> bool {
        self.direction == StreamDirection::Playback
    }

    fn is_capture(&self) -> bool {
        self.direction == StreamDirection::Capture
    }
}

// ============================================================================
// Mixer Control Types
// ============================================================================

/// Type of mixer control
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerControlType {
    /// Integer value with min/max range
    Integer,
    /// Boolean (on/off, mute/unmute)
    Boolean,
    /// Enumerated value (select from named options)
    Enumerated,
}

/// A mixer control element
pub struct MixerControl {
    /// Control identifier
    pub id: u32,
    /// Control name (e.g., "Master Playback Volume")
    pub name: String,
    /// Control type
    pub control_type: MixerControlType,
    /// Current value
    value: AtomicU32,
    /// Minimum value (for Integer type)
    pub min: i32,
    /// Maximum value (for Integer type)
    pub max: i32,
    /// Number of enumerated items (for Enumerated type)
    pub enum_count: u32,
    /// Enumerated item names
    pub enum_names: Vec<String>,
}

impl MixerControl {
    /// Create a new integer mixer control
    pub(crate) fn new_integer(id: u32, name: &str, min: i32, max: i32, initial: i32) -> Self {
        Self {
            id,
            name: String::from(name),
            control_type: MixerControlType::Integer,
            value: AtomicU32::new(initial as u32),
            min,
            max,
            enum_count: 0,
            enum_names: Vec::new(),
        }
    }

    /// Create a new boolean mixer control
    pub(crate) fn new_boolean(id: u32, name: &str, initial: bool) -> Self {
        Self {
            id,
            name: String::from(name),
            control_type: MixerControlType::Boolean,
            value: AtomicU32::new(if initial { 1 } else { 0 }),
            min: 0,
            max: 1,
            enum_count: 0,
            enum_names: Vec::new(),
        }
    }

    /// Create a new enumerated mixer control
    pub(crate) fn new_enumerated(id: u32, name: &str, items: Vec<String>, initial: u32) -> Self {
        let count = items.len() as u32;
        Self {
            id,
            name: String::from(name),
            control_type: MixerControlType::Enumerated,
            value: AtomicU32::new(initial),
            min: 0,
            max: count.saturating_sub(1) as i32,
            enum_count: count,
            enum_names: items,
        }
    }

    /// Get the current value
    pub(crate) fn get_value(&self) -> i32 {
        self.value.load(Ordering::Relaxed) as i32
    }

    /// Set the value with range checking
    pub(crate) fn set_value(&self, val: i32) -> Result<(), AlsaError> {
        if val < self.min || val > self.max {
            return Err(AlsaError::MixerValueOutOfRange {
                value: val,
                min: self.min,
                max: self.max,
            });
        }
        self.value.store(val as u32, Ordering::Relaxed);
        Ok(())
    }

    /// For boolean controls: get as bool
    pub(crate) fn get_bool(&self) -> bool {
        self.value.load(Ordering::Relaxed) != 0
    }

    /// For boolean controls: set as bool
    pub(crate) fn set_bool(&self, val: bool) {
        self.value.store(if val { 1 } else { 0 }, Ordering::Relaxed);
    }

    /// For enumerated controls: get selected item name
    pub(crate) fn get_enum_name(&self) -> Option<&str> {
        let idx = self.value.load(Ordering::Relaxed) as usize;
        self.enum_names.get(idx).map(|s| s.as_str())
    }
}

// ============================================================================
// ALSA Mixer (collection of controls)
// ============================================================================

/// Well-known mixer control IDs
pub const MIXER_MASTER_VOLUME: u32 = 1;
/// PCM playback volume control ID
pub const MIXER_PCM_VOLUME: u32 = 2;
/// Capture volume control ID
pub const MIXER_CAPTURE_VOLUME: u32 = 3;
/// Master mute switch control ID
pub const MIXER_MASTER_SWITCH: u32 = 4;
/// Capture mute switch control ID
pub const MIXER_CAPTURE_SWITCH: u32 = 5;

/// ALSA mixer managing multiple controls
pub struct AlsaMixer {
    /// All mixer controls keyed by ID
    controls: BTreeMap<u32, MixerControl>,
    /// Next control ID
    next_id: u32,
}

impl AlsaMixer {
    /// Create a new mixer with default controls
    pub fn new() -> Self {
        let mut mixer = Self {
            controls: BTreeMap::new(),
            next_id: 10, // Reserve IDs 1-9 for well-known controls
        };

        // Create default controls
        mixer.controls.insert(
            MIXER_MASTER_VOLUME,
            MixerControl::new_integer(MIXER_MASTER_VOLUME, "Master Playback Volume", 0, 100, 80),
        );
        mixer.controls.insert(
            MIXER_PCM_VOLUME,
            MixerControl::new_integer(MIXER_PCM_VOLUME, "PCM Playback Volume", 0, 100, 100),
        );
        mixer.controls.insert(
            MIXER_CAPTURE_VOLUME,
            MixerControl::new_integer(MIXER_CAPTURE_VOLUME, "Capture Volume", 0, 100, 80),
        );
        mixer.controls.insert(
            MIXER_MASTER_SWITCH,
            MixerControl::new_boolean(MIXER_MASTER_SWITCH, "Master Playback Switch", true),
        );
        mixer.controls.insert(
            MIXER_CAPTURE_SWITCH,
            MixerControl::new_boolean(MIXER_CAPTURE_SWITCH, "Capture Switch", true),
        );

        mixer
    }

    /// Add a custom mixer control
    pub(crate) fn add_control(&mut self, control: MixerControl) -> u32 {
        let id = control.id;
        self.controls.insert(id, control);
        id
    }

    /// Allocate a new control ID
    pub(crate) fn alloc_control_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Get a control by ID
    pub(crate) fn get_control(&self, id: u32) -> Result<&MixerControl, AlsaError> {
        self.controls
            .get(&id)
            .ok_or(AlsaError::MixerControlNotFound { id })
    }

    /// Set volume for a control (0-100 range)
    pub(crate) fn set_volume(&self, control_id: u32, volume: i32) -> Result<(), AlsaError> {
        let control = self
            .controls
            .get(&control_id)
            .ok_or(AlsaError::MixerControlNotFound { id: control_id })?;
        control.set_value(volume)
    }

    /// Get volume for a control
    pub(crate) fn get_volume(&self, control_id: u32) -> Result<i32, AlsaError> {
        let control = self
            .controls
            .get(&control_id)
            .ok_or(AlsaError::MixerControlNotFound { id: control_id })?;
        Ok(control.get_value())
    }

    /// Get the number of controls
    pub(crate) fn control_count(&self) -> usize {
        self.controls.len()
    }

    /// List all control IDs
    pub(crate) fn list_control_ids(&self) -> Vec<u32> {
        self.controls.keys().copied().collect()
    }
}

impl Default for AlsaMixer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Sample Format Conversion
// ============================================================================

/// Convert U8 sample to S16 (expand range)
///
/// U8 range: 0..255 (center at 128)
/// S16 range: -32768..32767
#[inline]
pub(crate) fn convert_u8_to_s16(sample: u8) -> i16 {
    // Shift center from 128 to 0, then scale up by 256
    ((sample as i16) - 128) * 256
}

/// Convert S16 sample to U8 (compress range)
#[inline]
pub(crate) fn convert_s16_to_u8(sample: i16) -> u8 {
    // Scale down by 256 and shift center from 0 to 128
    ((sample / 256) + 128) as u8
}

/// Convert S16 sample to S32 (expand to 32-bit)
#[inline]
pub(crate) fn convert_s16_to_s32(sample: i16) -> i32 {
    (sample as i32) << 16
}

/// Convert S32 sample to S16 (truncate to 16-bit)
#[inline]
pub(crate) fn convert_s32_to_s16(sample: i32) -> i16 {
    let shifted = sample >> 16;
    if shifted > i16::MAX as i32 {
        i16::MAX
    } else if shifted < i16::MIN as i32 {
        i16::MIN
    } else {
        shifted as i16
    }
}

/// Convert U8 sample to S32
#[inline]
pub(crate) fn convert_u8_to_s32(sample: u8) -> i32 {
    convert_s16_to_s32(convert_u8_to_s16(sample))
}

/// Convert S32 sample to U8
#[inline]
pub(crate) fn convert_s32_to_u8(sample: i32) -> u8 {
    convert_s16_to_u8(convert_s32_to_s16(sample))
}

/// Convert a buffer of samples between formats
pub(crate) fn convert_buffer(
    input: &[u8],
    src_format: PcmFormat,
    dst_format: PcmFormat,
    output: &mut Vec<u8>,
) {
    output.clear();

    if src_format == dst_format {
        output.extend_from_slice(input);
        return;
    }

    match (src_format, dst_format) {
        (PcmFormat::U8, PcmFormat::S16Le) => {
            output.reserve(input.len() * 2);
            for &sample in input {
                let converted = convert_u8_to_s16(sample);
                output.extend_from_slice(&converted.to_le_bytes());
            }
        }
        (PcmFormat::S16Le, PcmFormat::U8) => {
            output.reserve(input.len() / 2);
            for chunk in input.chunks_exact(2) {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                output.push(convert_s16_to_u8(sample));
            }
        }
        (PcmFormat::S16Le, PcmFormat::S32Le) => {
            output.reserve(input.len() * 2);
            for chunk in input.chunks_exact(2) {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                let converted = convert_s16_to_s32(sample);
                output.extend_from_slice(&converted.to_le_bytes());
            }
        }
        (PcmFormat::S32Le, PcmFormat::S16Le) => {
            output.reserve(input.len() / 2);
            for chunk in input.chunks_exact(4) {
                let sample = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let converted = convert_s32_to_s16(sample);
                output.extend_from_slice(&converted.to_le_bytes());
            }
        }
        (PcmFormat::U8, PcmFormat::S32Le) => {
            output.reserve(input.len() * 4);
            for &sample in input {
                let converted = convert_u8_to_s32(sample);
                output.extend_from_slice(&converted.to_le_bytes());
            }
        }
        (PcmFormat::S32Le, PcmFormat::U8) => {
            output.reserve(input.len() / 4);
            for chunk in input.chunks_exact(4) {
                let sample = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                output.push(convert_s32_to_u8(sample));
            }
        }
        // F32FixedPoint is stored as i32 internally, same conversion as S32
        (PcmFormat::F32FixedPoint, dst) => {
            convert_buffer(input, PcmFormat::S32Le, dst, output);
        }
        (src, PcmFormat::F32FixedPoint) => {
            convert_buffer(input, src, PcmFormat::S32Le, output);
        }
        // Same format case (already handled above, but needed for exhaustiveness)
        _ => {
            output.extend_from_slice(input);
        }
    }
}

// ============================================================================
// Gain Control (integer dB scaling via 16.16 fixed-point)
// ============================================================================

/// Gain scaling factor in 16.16 fixed-point
///
/// Represents a linear amplitude multiplier. Use `gain_from_db()` to convert
/// from decibels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainFactor(pub i32);

impl GainFactor {
    /// Unity gain (0 dB, multiplier = 1.0)
    pub const UNITY: GainFactor = GainFactor(FP_ONE);

    /// Mute (negative infinity dB, multiplier = 0.0)
    pub const MUTE: GainFactor = GainFactor(0);

    /// Apply this gain factor to a 16-bit sample
    #[inline]
    pub(crate) fn apply_s16(self, sample: i16) -> i16 {
        let wide = (sample as i32 as i64) * (self.0 as i64);
        let result = wide >> FP_SHIFT;
        if result > i16::MAX as i64 {
            i16::MAX
        } else if result < i16::MIN as i64 {
            i16::MIN
        } else {
            result as i16
        }
    }

    /// Apply this gain factor to a 32-bit sample
    #[inline]
    pub(crate) fn apply_s32(self, sample: i32) -> i32 {
        let wide = (sample as i64) * (self.0 as i64);
        let result = wide >> FP_SHIFT;
        if result > i32::MAX as i64 {
            i32::MAX
        } else if result < i32::MIN as i64 {
            i32::MIN
        } else {
            result as i32
        }
    }

    /// Get the raw 16.16 fixed-point value
    pub(crate) fn raw(self) -> i32 {
        self.0
    }
}

/// Convert decibels to a linear gain factor using integer approximation
///
/// Uses a lookup table for common dB values and linear interpolation.
/// Range: -60 dB to +20 dB. Values below -60 dB are treated as mute.
///
/// The `db_tenths` parameter is in tenths of a dB (e.g., -30 = -3.0 dB).
pub(crate) fn gain_from_db_tenths(db_tenths: i32) -> GainFactor {
    // Below -600 tenths (-60 dB): effectively mute
    if db_tenths <= -600 {
        return GainFactor::MUTE;
    }

    // Lookup table: dB (in tenths) -> 16.16 fixed-point linear gain
    // Computed as: round(10^(dB/200) * 65536)
    // We store entries at 6 dB intervals and interpolate
    //
    // Key reference points:
    //   0 dB    = 1.0     = 65536 (FP_ONE)
    //  -6 dB    = 0.5012  = 32845
    // -12 dB    = 0.2512  = 16462
    // -18 dB    = 0.1259  = 8250
    // -24 dB    = 0.0631  = 4135
    // -30 dB    = 0.0316  = 2073
    // -36 dB    = 0.0158  = 1038
    // -42 dB    = 0.00794 = 520
    // -48 dB    = 0.00398 = 261
    // -54 dB    = 0.00200 = 131
    // -60 dB    = 0.00100 = 65
    //  +6 dB    = 1.9953  = 130762
    // +12 dB    = 3.9811  = 260921
    // +18 dB    = 7.9433  = 520570
    // +20 dB    = 10.0    = 655360

    // Table indexed by (db_tenths + 600) / 60
    // Entries at -60, -54, -48, ..., 0, +6, +12, +18, +20 dB
    static DB_TABLE: [i32; 21] = [
        65,   // -60 dB (idx 0)
        92,   // -54 dB (idx 1) -- interpolated
        131,  // -48 dB (idx 2)
        185,  // -42 dB (idx 3) -- interpolated
        261,  // -36 dB (idx 4)
        369,  // -30 dB (idx 5) -- interpolated
        520,  // -24 dB (idx 6)
        735,  // -18 dB (idx 7) -- interpolated
        1038, // -12 dB (idx 8)
        1467, // -6 dB  (idx 9) -- interpolated
        2073, //  0 dB ... wait, let me recalculate
        // Actually, let's use a simpler table at 6dB steps from -60 to +20
        // Re-indexed properly:
        4135, // +6 dB relative...
        // This is getting complex. Let me use a direct piecewise approach.
        8250, 16462, 32845, FP_ONE, // 0 dB = 65536
        130762, // +6 dB
        260921, // +12 dB
        520570, // +18 dB
        655360, // +20 dB
        655360, // clamp
    ];

    // Simpler approach: piecewise linear between key points
    // Use a small table of (db_tenths, gain_fp) pairs
    struct DbGainEntry {
        db_tenths: i32,
        gain: i32,
    }

    static ENTRIES: [DbGainEntry; 13] = [
        DbGainEntry {
            db_tenths: -600,
            gain: 65,
        },
        DbGainEntry {
            db_tenths: -540,
            gain: 131,
        },
        DbGainEntry {
            db_tenths: -480,
            gain: 261,
        },
        DbGainEntry {
            db_tenths: -420,
            gain: 520,
        },
        DbGainEntry {
            db_tenths: -360,
            gain: 1038,
        },
        DbGainEntry {
            db_tenths: -300,
            gain: 2073,
        },
        DbGainEntry {
            db_tenths: -240,
            gain: 4135,
        },
        DbGainEntry {
            db_tenths: -180,
            gain: 8250,
        },
        DbGainEntry {
            db_tenths: -120,
            gain: 16462,
        },
        DbGainEntry {
            db_tenths: -60,
            gain: 32845,
        },
        DbGainEntry {
            db_tenths: 0,
            gain: FP_ONE,
        },
        DbGainEntry {
            db_tenths: 120,
            gain: 260921,
        },
        DbGainEntry {
            db_tenths: 200,
            gain: 655360,
        },
    ];

    // Clamp to range
    let db = if db_tenths > 200 { 200 } else { db_tenths };

    // Find the two surrounding entries and interpolate
    let mut i = 0;
    while i < ENTRIES.len() - 1 {
        if db <= ENTRIES[i + 1].db_tenths {
            break;
        }
        i += 1;
    }
    if i >= ENTRIES.len() - 1 {
        return GainFactor(ENTRIES[ENTRIES.len() - 1].gain);
    }

    let lo = &ENTRIES[i];
    let hi = &ENTRIES[i + 1];
    let range = hi.db_tenths - lo.db_tenths;
    if range == 0 {
        return GainFactor(lo.gain);
    }

    // Linear interpolation: gain = lo.gain + (hi.gain - lo.gain) * (db -
    // lo.db_tenths) / range
    let frac_num = db - lo.db_tenths;
    let gain_diff = hi.gain as i64 - lo.gain as i64;
    let interpolated = lo.gain as i64 + (gain_diff * frac_num as i64) / range as i64;

    GainFactor(interpolated as i32)
}

/// Convert a 0-100 volume percentage to a gain factor
///
/// 0 = mute, 100 = unity gain (0 dB). Uses a perceptual curve.
pub(crate) fn gain_from_percent(percent: u32) -> GainFactor {
    if percent == 0 {
        return GainFactor::MUTE;
    }
    if percent >= 100 {
        return GainFactor::UNITY;
    }

    // Map 0-100 to -60dB..0dB using a perceptual curve
    // percent 100 -> 0 dB (0 tenths)
    // percent 50  -> ~-18 dB (-180 tenths)
    // percent 1   -> ~-60 dB (-600 tenths)
    //
    // Use quadratic mapping: db_tenths = -600 * (100 - percent)^2 / 10000
    let inv = (100 - percent) as i64;
    let db_tenths = -((600 * inv * inv) / 10000) as i32;

    gain_from_db_tenths(db_tenths)
}

// ============================================================================
// Capture Pipeline
// ============================================================================

/// Capture device state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureState {
    /// Not capturing
    #[default]
    Idle,
    /// Actively capturing audio
    Recording,
    /// Paused
    Paused,
}

/// Statistics for a capture device
#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureStats {
    /// Total frames captured
    pub frames_captured: u64,
    /// Number of overrun events (data lost due to full buffer)
    pub overruns: u64,
    /// Current buffer fill level in frames
    pub buffer_fill: u32,
    /// Buffer capacity in frames
    pub buffer_capacity: u32,
}

/// Audio capture device with ring buffer and overrun detection
pub struct CaptureDevice {
    /// Device identifier
    pub id: u32,
    /// Device name
    pub name: String,
    /// Capture state
    state: CaptureState,
    /// Hardware parameters
    hw_params: HwParams,
    /// Ring buffer for captured audio data (bytes)
    buffer: Vec<u8>,
    /// Read position (byte offset)
    read_pos: u32,
    /// Write position (byte offset)
    write_pos: u32,
    /// Total frames captured
    frames_captured: AtomicU64,
    /// Number of overrun events
    overruns: AtomicU64,
    /// Capture gain factor (16.16 fixed-point)
    gain: GainFactor,
}

impl CaptureDevice {
    /// Create a new capture device
    pub fn new(id: u32, name: &str, hw_params: HwParams) -> Self {
        let buf_bytes =
            (CAPTURE_BUFFER_FRAMES as usize).saturating_mul(hw_params.frame_size() as usize);
        Self {
            id,
            name: String::from(name),
            state: CaptureState::Idle,
            hw_params,
            buffer: vec![0u8; buf_bytes],
            read_pos: 0,
            write_pos: 0,
            frames_captured: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            gain: GainFactor::UNITY,
        }
    }

    /// Start recording
    pub(crate) fn start(&mut self) -> Result<(), AlsaError> {
        if self.state != CaptureState::Idle && self.state != CaptureState::Paused {
            return Err(AlsaError::InvalidStateTransition {
                current: PcmState::Running,
                requested: PcmState::Running,
            });
        }
        self.state = CaptureState::Recording;
        Ok(())
    }

    /// Stop recording
    pub(crate) fn stop(&mut self) {
        self.state = CaptureState::Idle;
        self.read_pos = 0;
        self.write_pos = 0;
    }

    /// Pause recording
    pub(crate) fn pause(&mut self) {
        if self.state == CaptureState::Recording {
            self.state = CaptureState::Paused;
        }
    }

    /// Resume recording
    pub(crate) fn resume(&mut self) {
        if self.state == CaptureState::Paused {
            self.state = CaptureState::Recording;
        }
    }

    /// Push captured audio data from hardware into the ring buffer
    ///
    /// This is called by the audio driver/interrupt handler when new data
    /// arrives from the hardware. If the buffer is full, an overrun occurs.
    pub(crate) fn push_data(&mut self, data: &[u8]) -> Result<u32, AlsaError> {
        if self.state != CaptureState::Recording {
            return Ok(0);
        }

        let frame_size = self.hw_params.frame_size();
        if frame_size == 0 {
            return Ok(0);
        }

        let cap = self.buffer.len() as u32;
        let avail = self.available_write(cap);
        let to_write = (data.len() as u32).min(avail);
        let to_write = (to_write / frame_size) * frame_size;

        if to_write == 0 && !data.is_empty() {
            // Overrun
            let lost = data.len() as u32 / frame_size;
            self.overruns.fetch_add(1, Ordering::Relaxed);
            return Err(AlsaError::Overrun { lost_frames: lost });
        }

        // Apply gain if not unity
        if self.gain != GainFactor::UNITY && self.gain != GainFactor::MUTE {
            // Apply gain to S16 samples
            if self.hw_params.format == PcmFormat::S16Le {
                let tw = to_write as usize;
                let wp = self.write_pos as usize;
                let c = cap as usize;

                // Process in S16 chunks
                let mut src_offset = 0;
                let mut dst_offset = wp;
                let mut remaining = tw;

                while remaining >= 2 {
                    let sample = i16::from_le_bytes([data[src_offset], data[src_offset + 1]]);
                    let gained = self.gain.apply_s16(sample);
                    let bytes = gained.to_le_bytes();
                    self.buffer[dst_offset % c] = bytes[0];
                    self.buffer[(dst_offset + 1) % c] = bytes[1];
                    src_offset += 2;
                    dst_offset += 2;
                    remaining -= 2;
                }

                self.write_pos = (dst_offset % c) as u32;
            } else {
                // For other formats, copy raw data
                self.copy_to_ring(data, to_write);
            }
        } else if self.gain == GainFactor::MUTE {
            // Muted: write silence
            let tw = to_write as usize;
            let wp = self.write_pos as usize;
            let c = cap as usize;
            let first = (c - wp).min(tw);
            let second = tw - first;
            for b in &mut self.buffer[wp..wp + first] {
                *b = 0;
            }
            if second > 0 {
                for b in &mut self.buffer[..second] {
                    *b = 0;
                }
            }
            self.write_pos = ((wp + tw) % c) as u32;
        } else {
            self.copy_to_ring(data, to_write);
        }

        let frames = to_write / frame_size;
        self.frames_captured
            .fetch_add(frames as u64, Ordering::Relaxed);
        Ok(frames)
    }

    /// Read captured audio data from the ring buffer
    ///
    /// Returns the number of frames read. Called by the application/client.
    pub(crate) fn read_data(&mut self, output: &mut [u8]) -> u32 {
        let frame_size = self.hw_params.frame_size();
        if frame_size == 0 {
            return 0;
        }

        let cap = self.buffer.len() as u32;
        let avail = self.available_read(cap);
        let to_read = (output.len() as u32).min(avail);
        let to_read = (to_read / frame_size) * frame_size;

        if to_read == 0 {
            return 0;
        }

        let rp = self.read_pos as usize;
        let c = cap as usize;
        let tr = to_read as usize;

        let first = (c - rp).min(tr);
        let second = tr - first;

        output[..first].copy_from_slice(&self.buffer[rp..rp + first]);
        if second > 0 {
            output[first..first + second].copy_from_slice(&self.buffer[..second]);
        }

        self.read_pos = ((rp + tr) % c) as u32;
        to_read / frame_size
    }

    /// Get capture statistics
    pub(crate) fn stats(&self) -> CaptureStats {
        let cap = self.buffer.len() as u32;
        let frame_size = self.hw_params.frame_size();
        let fill_bytes = self.available_read(cap);
        let fill_frames = if frame_size > 0 {
            fill_bytes / frame_size
        } else {
            0
        };
        let cap_frames = if frame_size > 0 { cap / frame_size } else { 0 };
        CaptureStats {
            frames_captured: self.frames_captured.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            buffer_fill: fill_frames,
            buffer_capacity: cap_frames,
        }
    }

    /// Get capture state
    pub(crate) fn state(&self) -> CaptureState {
        self.state
    }

    /// Set capture gain
    pub(crate) fn set_gain(&mut self, gain: GainFactor) {
        self.gain = gain;
    }

    /// Get capture gain
    pub(crate) fn gain(&self) -> GainFactor {
        self.gain
    }

    /// Get hardware parameters
    pub(crate) fn hw_params(&self) -> &HwParams {
        &self.hw_params
    }

    // --- Internal helpers ---

    fn available_write(&self, capacity: u32) -> u32 {
        if capacity == 0 {
            return 0;
        }
        let used = if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            capacity - self.read_pos + self.write_pos
        };
        capacity.saturating_sub(used).saturating_sub(1)
    }

    fn available_read(&self, capacity: u32) -> u32 {
        if capacity == 0 {
            return 0;
        }
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            capacity - self.read_pos + self.write_pos
        }
    }

    fn copy_to_ring(&mut self, data: &[u8], to_write: u32) {
        let wp = self.write_pos as usize;
        let c = self.buffer.len();
        let tw = to_write as usize;
        let first = (c - wp).min(tw);
        let second = tw - first;
        self.buffer[wp..wp + first].copy_from_slice(&data[..first]);
        if second > 0 {
            self.buffer[..second].copy_from_slice(&data[first..first + second]);
        }
        self.write_pos = ((wp + tw) % c) as u32;
    }
}

// ============================================================================
// Device Registry
// ============================================================================

/// Information about a registered audio device
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device identifier
    pub id: u32,
    /// Device name
    pub name: String,
    /// Whether it supports playback
    pub playback: bool,
    /// Whether it supports capture
    pub capture: bool,
    /// Maximum supported sample rate
    pub max_sample_rate: u32,
    /// Maximum supported channels
    pub max_channels: u8,
    /// Supported formats
    pub formats: Vec<PcmFormat>,
}

/// Registry of available audio devices
pub struct DeviceRegistry {
    /// Registered devices
    devices: BTreeMap<u32, DeviceInfo>,
    /// Next device ID
    next_id: AtomicU32,
}

impl DeviceRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            next_id: AtomicU32::new(1),
        }
    }

    /// Register a new audio device
    pub(crate) fn register(&mut self, info: DeviceInfo) -> Result<u32, AlsaError> {
        if self.devices.len() >= MAX_PCM_DEVICES {
            return Err(AlsaError::TooManyDevices);
        }
        let id = info.id;
        self.devices.insert(id, info);
        Ok(id)
    }

    /// Unregister a device
    pub(crate) fn unregister(&mut self, id: u32) -> Result<(), AlsaError> {
        self.devices
            .remove(&id)
            .map(|_| ())
            .ok_or(AlsaError::DeviceNotFound { device_id: id })
    }

    /// Get device info by ID
    pub(crate) fn get(&self, id: u32) -> Result<&DeviceInfo, AlsaError> {
        self.devices
            .get(&id)
            .ok_or(AlsaError::DeviceNotFound { device_id: id })
    }

    /// List all registered devices
    pub(crate) fn list(&self) -> Vec<&DeviceInfo> {
        self.devices.values().collect()
    }

    /// List only playback devices
    pub(crate) fn list_playback(&self) -> Vec<&DeviceInfo> {
        self.devices.values().filter(|d| d.playback).collect()
    }

    /// List only capture devices
    pub(crate) fn list_capture(&self) -> Vec<&DeviceInfo> {
        self.devices.values().filter(|d| d.capture).collect()
    }

    /// Number of registered devices
    pub(crate) fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Allocate a new device ID
    pub(crate) fn alloc_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Global State
// ============================================================================

static ALSA_MIXER: spin::Mutex<Option<AlsaMixer>> = spin::Mutex::new(None);
static DEVICE_REGISTRY: spin::Mutex<Option<DeviceRegistry>> = spin::Mutex::new(None);

/// Initialize the ALSA subsystem
pub fn init() {
    {
        let mut mixer = ALSA_MIXER.lock();
        *mixer = Some(AlsaMixer::new());
    }
    {
        let mut registry = DEVICE_REGISTRY.lock();
        let mut reg = DeviceRegistry::new();

        // Register default devices
        let _ = reg.register(DeviceInfo {
            id: 0,
            name: String::from("default"),
            playback: true,
            capture: true,
            max_sample_rate: 192000,
            max_channels: 8,
            formats: vec![PcmFormat::U8, PcmFormat::S16Le, PcmFormat::S32Le],
        });

        *registry = Some(reg);
    }
}

/// Access the ALSA mixer through a closure
pub fn with_alsa_mixer<R, F: FnOnce(&AlsaMixer) -> R>(f: F) -> Option<R> {
    let guard = ALSA_MIXER.lock();
    guard.as_ref().map(f)
}

/// Access the ALSA mixer mutably through a closure
pub fn with_alsa_mixer_mut<R, F: FnOnce(&mut AlsaMixer) -> R>(f: F) -> Option<R> {
    let mut guard = ALSA_MIXER.lock();
    guard.as_mut().map(f)
}

/// Access the device registry through a closure
pub fn with_registry<R, F: FnOnce(&DeviceRegistry) -> R>(f: F) -> Option<R> {
    let guard = DEVICE_REGISTRY.lock();
    guard.as_ref().map(f)
}

/// Access the device registry mutably through a closure
pub fn with_registry_mut<R, F: FnOnce(&mut DeviceRegistry) -> R>(f: F) -> Option<R> {
    let mut guard = DEVICE_REGISTRY.lock();
    guard.as_mut().map(f)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- PCM State Machine Tests ---

    #[test]
    fn test_pcm_state_valid_transitions() {
        assert!(PcmState::Open.can_transition_to(PcmState::Setup));
        assert!(PcmState::Setup.can_transition_to(PcmState::Prepared));
        assert!(PcmState::Prepared.can_transition_to(PcmState::Running));
        assert!(PcmState::Running.can_transition_to(PcmState::Paused));
        assert!(PcmState::Running.can_transition_to(PcmState::Draining));
        assert!(PcmState::Running.can_transition_to(PcmState::XRun));
        assert!(PcmState::Paused.can_transition_to(PcmState::Running));
        assert!(PcmState::XRun.can_transition_to(PcmState::Prepared));
        assert!(PcmState::Draining.can_transition_to(PcmState::Prepared));
    }

    #[test]
    fn test_pcm_state_invalid_transitions() {
        assert!(!PcmState::Open.can_transition_to(PcmState::Running));
        assert!(!PcmState::Open.can_transition_to(PcmState::Paused));
        assert!(!PcmState::Setup.can_transition_to(PcmState::Running));
        assert!(!PcmState::XRun.can_transition_to(PcmState::Running));
        assert!(!PcmState::Draining.can_transition_to(PcmState::Running));
    }

    // --- HwParams Tests ---

    #[test]
    fn test_hw_params_defaults() {
        let params = HwParams::new();
        assert_eq!(params.sample_rate, DEFAULT_SAMPLE_RATE);
        assert_eq!(params.channels, DEFAULT_CHANNELS);
        assert_eq!(params.format, PcmFormat::S16Le);
        assert_eq!(params.buffer_size, DEFAULT_BUFFER_FRAMES);
        assert_eq!(params.period_size, DEFAULT_PERIOD_FRAMES);
    }

    #[test]
    fn test_hw_params_validation_valid() {
        let params = HwParams::new();
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_hw_params_validation_invalid_rate() {
        let params = HwParams {
            sample_rate: 12345,
            ..HwParams::new()
        };
        assert_eq!(
            params.validate(),
            Err(AlsaError::InvalidSampleRate { rate: 12345 })
        );
    }

    #[test]
    fn test_hw_params_validation_invalid_channels() {
        let params = HwParams {
            channels: 0,
            ..HwParams::new()
        };
        assert_eq!(
            params.validate(),
            Err(AlsaError::InvalidChannels { count: 0 })
        );

        let params2 = HwParams {
            channels: 9,
            ..HwParams::new()
        };
        assert_eq!(
            params2.validate(),
            Err(AlsaError::InvalidChannels { count: 9 })
        );
    }

    #[test]
    fn test_hw_params_validation_period_exceeds_buffer() {
        let params = HwParams {
            buffer_size: 1024,
            period_size: 2048,
            ..HwParams::new()
        };
        assert_eq!(
            params.validate(),
            Err(AlsaError::InvalidPeriodSize {
                requested: 2048,
                buffer_size: 1024,
            })
        );
    }

    #[test]
    fn test_hw_params_frame_size() {
        let params = HwParams {
            channels: 2,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        assert_eq!(params.frame_size(), 4); // 2 channels * 2 bytes

        let params32 = HwParams {
            channels: 2,
            format: PcmFormat::S32Le,
            ..HwParams::new()
        };
        assert_eq!(params32.frame_size(), 8); // 2 channels * 4 bytes
    }

    #[test]
    fn test_hw_params_byte_rate() {
        let params = HwParams {
            sample_rate: 48000,
            channels: 2,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        assert_eq!(params.byte_rate(), 48000 * 4);
    }

    // --- PCM Device Tests ---

    #[test]
    fn test_pcm_device_open_close() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Playback);
        assert!(!dev.is_open());

        assert!(dev.open().is_ok());
        assert!(dev.is_open());

        // Double open should fail
        assert_eq!(
            dev.open(),
            Err(AlsaError::DeviceAlreadyOpen { device_id: 0 })
        );

        assert!(dev.close().is_ok());
        assert!(!dev.is_open());

        // Double close should fail
        assert_eq!(dev.close(), Err(AlsaError::DeviceNotOpen { device_id: 0 }));
    }

    #[test]
    fn test_pcm_device_lifecycle() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Playback);
        dev.open().unwrap();
        assert_eq!(dev.state(), PcmState::Open);

        dev.set_hw_params(HwParams::new()).unwrap();
        assert_eq!(dev.state(), PcmState::Setup);

        dev.prepare().unwrap();
        assert_eq!(dev.state(), PcmState::Prepared);

        dev.start().unwrap();
        assert_eq!(dev.state(), PcmState::Running);

        dev.pause().unwrap();
        assert_eq!(dev.state(), PcmState::Paused);

        dev.resume().unwrap();
        assert_eq!(dev.state(), PcmState::Running);

        dev.stop().unwrap();
        assert_eq!(dev.state(), PcmState::Prepared);
    }

    #[test]
    fn test_pcm_device_write() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Playback);
        dev.open().unwrap();
        dev.set_hw_params(HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            buffer_size: 64,
            period_size: 16,
            ..HwParams::new()
        })
        .unwrap();
        dev.prepare().unwrap();
        dev.start().unwrap();

        // Write 4 frames of S16 mono = 8 bytes
        let data = [0u8, 0, 1, 0, 2, 0, 3, 0];
        let frames = dev.write(&data).unwrap();
        assert_eq!(frames, 4);
        assert_eq!(dev.total_frames_written(), 4);
    }

    #[test]
    fn test_pcm_device_write_wrong_direction() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Capture);
        dev.open().unwrap();
        dev.set_hw_params(HwParams::new()).unwrap();
        dev.prepare().unwrap();
        dev.start().unwrap();

        let data = [0u8; 8];
        assert_eq!(dev.write(&data), Err(AlsaError::WrongDirection));
    }

    #[test]
    fn test_pcm_device_read_capture() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Capture);
        dev.open().unwrap();
        let hw = HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            buffer_size: 64,
            period_size: 16,
            ..HwParams::new()
        };
        dev.set_hw_params(hw).unwrap();
        dev.prepare().unwrap();
        dev.start().unwrap();

        // Push some data into the capture buffer
        let input = [10u8, 0, 20, 0, 30, 0, 40, 0];
        let pushed = dev.push_capture_data(&input).unwrap();
        assert_eq!(pushed, 4); // 4 frames

        // Read it back
        let mut output = [0u8; 8];
        let frames = dev.read(&mut output).unwrap();
        assert_eq!(frames, 4);
        assert_eq!(output, input);
    }

    // --- Sample Conversion Tests ---

    #[test]
    fn test_convert_u8_to_s16() {
        assert_eq!(convert_u8_to_s16(128), 0); // center
        assert_eq!(convert_u8_to_s16(0), -32768); // min
        assert_eq!(convert_u8_to_s16(255), 32512); // near max
    }

    #[test]
    fn test_convert_s16_to_u8() {
        assert_eq!(convert_s16_to_u8(0), 128); // center
        assert_eq!(convert_s16_to_u8(-32768), 0); // min
        assert_eq!(convert_s16_to_u8(32512), 255); // near max (32512/256 = 127
                                                   // + 128)
    }

    #[test]
    fn test_convert_s16_s32_roundtrip() {
        let original: i16 = 1234;
        let s32 = convert_s16_to_s32(original);
        let back = convert_s32_to_s16(s32);
        assert_eq!(back, original);
    }

    #[test]
    fn test_convert_s32_to_s16_saturation() {
        assert_eq!(convert_s32_to_s16(i32::MAX), i16::MAX);
        assert_eq!(convert_s32_to_s16(i32::MIN), i16::MIN);
    }

    #[test]
    fn test_convert_buffer_s16_to_s32() {
        let input_sample: i16 = 1000;
        let input = input_sample.to_le_bytes();
        let mut output = Vec::new();

        convert_buffer(&input, PcmFormat::S16Le, PcmFormat::S32Le, &mut output);
        assert_eq!(output.len(), 4);

        let result = i32::from_le_bytes([output[0], output[1], output[2], output[3]]);
        assert_eq!(result, convert_s16_to_s32(1000));
    }

    #[test]
    fn test_convert_buffer_same_format() {
        let input = [1u8, 2, 3, 4];
        let mut output = Vec::new();
        convert_buffer(&input, PcmFormat::S16Le, PcmFormat::S16Le, &mut output);
        assert_eq!(output, input);
    }

    // --- Mixer Control Tests ---

    #[test]
    fn test_mixer_default_controls() {
        let mixer = AlsaMixer::new();
        assert_eq!(mixer.control_count(), 5);

        // Master volume
        let master = mixer.get_control(MIXER_MASTER_VOLUME).unwrap();
        assert_eq!(master.get_value(), 80);
        assert_eq!(master.min, 0);
        assert_eq!(master.max, 100);

        // PCM volume
        let pcm = mixer.get_control(MIXER_PCM_VOLUME).unwrap();
        assert_eq!(pcm.get_value(), 100);
    }

    #[test]
    fn test_mixer_set_volume() {
        let mixer = AlsaMixer::new();
        assert!(mixer.set_volume(MIXER_MASTER_VOLUME, 50).is_ok());
        assert_eq!(mixer.get_volume(MIXER_MASTER_VOLUME).unwrap(), 50);
    }

    #[test]
    fn test_mixer_volume_out_of_range() {
        let mixer = AlsaMixer::new();
        assert_eq!(
            mixer.set_volume(MIXER_MASTER_VOLUME, 101),
            Err(AlsaError::MixerValueOutOfRange {
                value: 101,
                min: 0,
                max: 100,
            })
        );
    }

    #[test]
    fn test_mixer_boolean_control() {
        let mixer = AlsaMixer::new();
        let switch = mixer.get_control(MIXER_MASTER_SWITCH).unwrap();
        assert!(switch.get_bool()); // Default: unmuted

        switch.set_bool(false);
        assert!(!switch.get_bool());

        switch.set_bool(true);
        assert!(switch.get_bool());
    }

    #[test]
    fn test_mixer_control_not_found() {
        let mixer = AlsaMixer::new();
        assert!(matches!(
            mixer.get_control(999),
            Err(AlsaError::MixerControlNotFound { id: 999 })
        ));
    }

    // --- Gain Control Tests ---

    #[test]
    fn test_gain_unity() {
        assert_eq!(GainFactor::UNITY.raw(), FP_ONE);
    }

    #[test]
    fn test_gain_mute() {
        assert_eq!(GainFactor::MUTE.raw(), 0);
    }

    #[test]
    fn test_gain_apply_s16_unity() {
        let sample: i16 = 16384;
        let result = GainFactor::UNITY.apply_s16(sample);
        assert_eq!(result, sample);
    }

    #[test]
    fn test_gain_apply_s16_mute() {
        let sample: i16 = 16384;
        let result = GainFactor::MUTE.apply_s16(sample);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_gain_from_db_tenths_zero() {
        let gain = gain_from_db_tenths(0);
        assert_eq!(gain.raw(), FP_ONE);
    }

    #[test]
    fn test_gain_from_db_tenths_mute() {
        let gain = gain_from_db_tenths(-700);
        assert_eq!(gain, GainFactor::MUTE);
    }

    #[test]
    fn test_gain_from_percent_boundaries() {
        let mute = gain_from_percent(0);
        assert_eq!(mute, GainFactor::MUTE);

        let full = gain_from_percent(100);
        assert_eq!(full, GainFactor::UNITY);
    }

    #[test]
    fn test_gain_from_percent_mid() {
        let mid = gain_from_percent(50);
        // At 50%, gain should be less than unity but greater than 0
        assert!(mid.raw() > 0);
        assert!(mid.raw() < FP_ONE);
    }

    // --- Capture Device Tests ---

    #[test]
    fn test_capture_device_lifecycle() {
        let hw = HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        let mut cap = CaptureDevice::new(0, "mic", hw);
        assert_eq!(cap.state(), CaptureState::Idle);

        cap.start().unwrap();
        assert_eq!(cap.state(), CaptureState::Recording);

        cap.pause();
        assert_eq!(cap.state(), CaptureState::Paused);

        cap.resume();
        assert_eq!(cap.state(), CaptureState::Recording);

        cap.stop();
        assert_eq!(cap.state(), CaptureState::Idle);
    }

    #[test]
    fn test_capture_device_push_read() {
        let hw = HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        let mut cap = CaptureDevice::new(0, "mic", hw);
        cap.start().unwrap();

        // Push 4 frames (8 bytes of S16 mono)
        let input = [10u8, 0, 20, 0, 30, 0, 40, 0];
        let frames = cap.push_data(&input).unwrap();
        assert_eq!(frames, 4);

        // Read back
        let mut output = [0u8; 8];
        let read = cap.read_data(&mut output);
        assert_eq!(read, 4);
        assert_eq!(output, input);
    }

    #[test]
    fn test_capture_device_stats() {
        let hw = HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        let mut cap = CaptureDevice::new(0, "mic", hw);
        cap.start().unwrap();

        let input = [0u8; 8]; // 4 frames
        let _ = cap.push_data(&input);

        let stats = cap.stats();
        assert_eq!(stats.frames_captured, 4);
        assert_eq!(stats.overruns, 0);
        assert!(stats.buffer_capacity > 0);
    }

    #[test]
    fn test_capture_device_gain() {
        let hw = HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            ..HwParams::new()
        };
        let mut cap = CaptureDevice::new(0, "mic", hw);
        assert_eq!(cap.gain(), GainFactor::UNITY);

        cap.set_gain(GainFactor::MUTE);
        assert_eq!(cap.gain(), GainFactor::MUTE);
    }

    // --- Device Registry Tests ---

    #[test]
    fn test_device_registry_register() {
        let mut reg = DeviceRegistry::new();
        let info = DeviceInfo {
            id: 1,
            name: String::from("test"),
            playback: true,
            capture: false,
            max_sample_rate: 48000,
            max_channels: 2,
            formats: vec![PcmFormat::S16Le],
        };
        let id = reg.register(info).unwrap();
        assert_eq!(id, 1);
        assert_eq!(reg.device_count(), 1);
    }

    #[test]
    fn test_device_registry_list_filtered() {
        let mut reg = DeviceRegistry::new();
        reg.register(DeviceInfo {
            id: 1,
            name: String::from("playback"),
            playback: true,
            capture: false,
            max_sample_rate: 48000,
            max_channels: 2,
            formats: vec![PcmFormat::S16Le],
        })
        .unwrap();
        reg.register(DeviceInfo {
            id: 2,
            name: String::from("capture"),
            playback: false,
            capture: true,
            max_sample_rate: 48000,
            max_channels: 1,
            formats: vec![PcmFormat::S16Le],
        })
        .unwrap();

        assert_eq!(reg.list_playback().len(), 1);
        assert_eq!(reg.list_capture().len(), 1);
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn test_device_registry_unregister() {
        let mut reg = DeviceRegistry::new();
        reg.register(DeviceInfo {
            id: 1,
            name: String::from("test"),
            playback: true,
            capture: false,
            max_sample_rate: 48000,
            max_channels: 2,
            formats: vec![PcmFormat::S16Le],
        })
        .unwrap();
        assert_eq!(reg.device_count(), 1);

        reg.unregister(1).unwrap();
        assert_eq!(reg.device_count(), 0);

        assert_eq!(
            reg.unregister(1),
            Err(AlsaError::DeviceNotFound { device_id: 1 })
        );
    }

    // --- PcmFormat Tests ---

    #[test]
    fn test_pcm_format_sizes() {
        assert_eq!(PcmFormat::U8.bytes_per_sample(), 1);
        assert_eq!(PcmFormat::S16Le.bytes_per_sample(), 2);
        assert_eq!(PcmFormat::S32Le.bytes_per_sample(), 4);
        assert_eq!(PcmFormat::F32FixedPoint.bytes_per_sample(), 4);
    }

    #[test]
    fn test_pcm_format_bits() {
        assert_eq!(PcmFormat::U8.bits_per_sample(), 8);
        assert_eq!(PcmFormat::S16Le.bits_per_sample(), 16);
        assert_eq!(PcmFormat::S32Le.bits_per_sample(), 32);
    }

    // --- SwParams Tests ---

    #[test]
    fn test_sw_params_defaults() {
        let sw = SwParams::new(4096);
        assert_eq!(sw.avail_min, 1);
        assert_eq!(sw.start_threshold, 4096);
        assert_eq!(sw.stop_threshold, 4096);
    }

    #[test]
    fn test_sw_params_validation() {
        let hw = HwParams::new();
        let sw = SwParams::new(hw.buffer_size);
        assert!(sw.validate(&hw).is_ok());

        let bad_sw = SwParams {
            avail_min: 0,
            ..SwParams::new(hw.buffer_size)
        };
        assert!(bad_sw.validate(&hw).is_err());
    }

    // --- XRun Recovery Test ---

    #[test]
    fn test_pcm_device_xrun_recovery() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Playback);
        dev.open().unwrap();
        dev.set_hw_params(HwParams::new()).unwrap();
        dev.prepare().unwrap();
        dev.start().unwrap();

        // Simulate XRun
        dev.state = PcmState::XRun;
        assert_eq!(dev.state(), PcmState::XRun);

        dev.recover_xrun().unwrap();
        assert_eq!(dev.state(), PcmState::Prepared);
    }

    // --- MMAP Test ---

    #[test]
    fn test_pcm_device_mmap() {
        let mut dev = PcmDevice::new(0, "test", StreamDirection::Playback);
        dev.open().unwrap();
        dev.set_hw_params(HwParams {
            channels: 1,
            format: PcmFormat::S16Le,
            buffer_size: 64,
            period_size: 16,
            ..HwParams::new()
        })
        .unwrap();
        dev.prepare().unwrap();

        let (buf, offset, avail) = dev.mmap_begin().unwrap();
        assert!(!buf.is_empty());
        assert_eq!(offset, 0);
        assert!(avail > 0);

        // Commit some frames
        dev.mmap_commit(4).unwrap();
        assert_eq!(dev.total_frames_written(), 4);
    }

    // --- Enumerated Mixer Control Test ---

    #[test]
    fn test_enumerated_mixer_control() {
        let items = vec![
            String::from("Input 1"),
            String::from("Input 2"),
            String::from("Input 3"),
        ];
        let ctrl = MixerControl::new_enumerated(10, "Capture Source", items, 0);
        assert_eq!(ctrl.control_type, MixerControlType::Enumerated);
        assert_eq!(ctrl.get_enum_name(), Some("Input 1"));

        ctrl.set_value(2).unwrap();
        assert_eq!(ctrl.get_enum_name(), Some("Input 3"));

        assert_eq!(
            ctrl.set_value(3),
            Err(AlsaError::MixerValueOutOfRange {
                value: 3,
                min: 0,
                max: 2,
            })
        );
    }
}
