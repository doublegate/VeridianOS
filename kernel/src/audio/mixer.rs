//! Fixed-point 16.16 audio mixer
//!
//! Provides software audio mixing using integer-only arithmetic suitable
//! for kernel context where FPU state is not available. All volume and
//! sample calculations use 16.16 fixed-point representation.
//!
//! ## Fixed-Point Format
//!
//! A `FixedPoint` value (i32) stores a number with 16 integer bits and
//! 16 fractional bits. For example:
//! - `0x0001_0000` = 1.0
//! - `0x0000_8000` = 0.5
//! - `0x0000_0000` = 0.0
//! - `0xFFFF_0000` = -1.0

#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::error::KernelError;

// ============================================================================
// Fixed-Point Arithmetic (16.16 format)
// ============================================================================

/// Fixed-point 16.16 type: 16 integer bits + 16 fractional bits
pub type FixedPoint = i32;

/// Number of fractional bits in the fixed-point representation
const FP_SHIFT: i32 = 16;

/// Fixed-point representation of 1.0
const FP_ONE: FixedPoint = 1 << FP_SHIFT;

/// Maximum representable fixed-point value (before saturation)
const FP_MAX: FixedPoint = i32::MAX;

/// Minimum representable fixed-point value (before saturation)
const FP_MIN: FixedPoint = i32::MIN;

/// Convert a signed 16-bit sample to fixed-point 16.16
///
/// Maps the full i16 range (-32768..32767) into fixed-point. Since an i16
/// fits in the integer portion of a 16.16 number, this is a simple shift.
#[inline]
pub fn fp_from_i16(sample: i16) -> FixedPoint {
    (sample as i32) << FP_SHIFT
}

/// Convert a fixed-point 16.16 value back to signed 16-bit with saturation
///
/// Extracts the integer portion (upper 16 bits) and clamps to i16 range.
#[inline]
pub fn fp_to_i16(fp: FixedPoint) -> i16 {
    let shifted = fp >> FP_SHIFT;
    if shifted > i16::MAX as i32 {
        i16::MAX
    } else if shifted < i16::MIN as i32 {
        i16::MIN
    } else {
        shifted as i16
    }
}

/// Multiply two fixed-point values with saturation
///
/// Uses i64 intermediate to avoid overflow, then clamps to i32 range.
#[inline]
pub fn fp_mul(a: FixedPoint, b: FixedPoint) -> FixedPoint {
    let result = (a as i64 * b as i64) >> FP_SHIFT;
    if result > FP_MAX as i64 {
        FP_MAX
    } else if result < FP_MIN as i64 {
        FP_MIN
    } else {
        result as i32
    }
}

/// Convert a volume value (0..65535) to fixed-point (0.0..1.0)
///
/// 0 maps to 0x0000_0000 (0.0), 65535 maps to 0x0000_FFFF (~1.0).
#[inline]
pub fn fp_from_volume(volume: u16) -> FixedPoint {
    volume as i32
}

/// Add two fixed-point values with saturation
#[inline]
fn fp_add_saturate(a: FixedPoint, b: FixedPoint) -> FixedPoint {
    let result = (a as i64) + (b as i64);
    if result > FP_MAX as i64 {
        FP_MAX
    } else if result < FP_MIN as i64 {
        FP_MIN
    } else {
        result as i32
    }
}

// ============================================================================
// Mixer Channel
// ============================================================================

/// A single mixer channel representing one audio source
pub struct MixerChannel {
    /// Unique channel identifier
    pub id: u16,
    /// Channel volume (0..65535), stored atomically for lock-free reads
    volume: AtomicU32,
    /// Whether this channel is muted
    muted: AtomicBool,
    /// Ring buffer read index for this channel's audio data
    buffer_index: u32,
    /// Human-readable channel name
    pub name: String,
    /// Per-channel sample buffer for mixing
    samples: Vec<i16>,
}

impl MixerChannel {
    /// Create a new mixer channel
    fn new(id: u16, name: String) -> Self {
        Self {
            id,
            volume: AtomicU32::new(65535), // Full volume by default
            muted: AtomicBool::new(false),
            buffer_index: 0,
            name,
            samples: Vec::new(),
        }
    }

    /// Get the current volume (0..65535)
    pub fn get_volume(&self) -> u16 {
        self.volume.load(Ordering::Relaxed) as u16
    }

    /// Set the volume (0..65535)
    pub fn set_volume(&self, vol: u16) {
        self.volume.store(vol as u32, Ordering::Relaxed);
    }

    /// Check if the channel is muted
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    /// Set mute state
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Write samples into this channel's buffer for the next mix cycle
    pub fn write_samples(&mut self, data: &[i16]) {
        self.samples.clear();
        self.samples.extend_from_slice(data);
        self.buffer_index = 0;
    }

    /// Read and consume up to `count` samples from this channel
    fn read_samples(&mut self, count: usize) -> &[i16] {
        let start = self.buffer_index as usize;
        let available = self.samples.len().saturating_sub(start);
        let to_read = count.min(available);
        let end = start + to_read;
        self.buffer_index = end as u32;
        &self.samples[start..end]
    }

    /// Number of samples remaining in this channel's buffer
    fn available_samples(&self) -> usize {
        self.samples.len().saturating_sub(self.buffer_index as usize)
    }
}

// ============================================================================
// Audio Mixer
// ============================================================================

/// Multi-channel audio mixer with fixed-point arithmetic
pub struct AudioMixer {
    /// Active mixer channels
    channels: Vec<MixerChannel>,
    /// Master volume (0..65535), atomically accessible
    master_volume: AtomicU32,
    /// Pre-allocated output buffer for mixed audio
    output_buffer: Vec<i16>,
    /// Output sample rate in Hz
    sample_rate: u32,
    /// Number of output channels (1=mono, 2=stereo)
    output_channels: u8,
    /// Next channel ID to assign
    next_channel_id: u16,
}

impl AudioMixer {
    /// Create a new audio mixer
    ///
    /// # Arguments
    /// * `sample_rate` - Output sample rate in Hz (e.g., 48000)
    /// * `channels` - Number of output channels (1=mono, 2=stereo)
    pub fn new(sample_rate: u32, channels: u8) -> Self {
        Self {
            channels: Vec::new(),
            master_volume: AtomicU32::new(65535),
            output_buffer: Vec::new(),
            sample_rate,
            output_channels: channels,
            next_channel_id: 1,
        }
    }

    /// Add a new mixer channel and return its ID
    pub fn add_channel(&mut self, name: &str) -> u16 {
        let id = self.next_channel_id;
        self.next_channel_id = self.next_channel_id.wrapping_add(1);
        let channel = MixerChannel::new(id, String::from(name));
        self.channels.push(channel);
        println!("[AUDIO] Mixer: added channel {} (id={})", name, id);
        id
    }

    /// Remove a mixer channel by ID
    pub fn remove_channel(&mut self, id: u16) {
        if let Some(pos) = self.channels.iter().position(|c| c.id == id) {
            let name = self.channels[pos].name.clone();
            self.channels.remove(pos);
            println!("[AUDIO] Mixer: removed channel {} (id={})", name, id);
        }
    }

    /// Set volume for a specific channel (0..65535)
    pub fn set_volume(&self, channel_id: u16, volume: u16) {
        if let Some(ch) = self.channels.iter().find(|c| c.id == channel_id) {
            ch.set_volume(volume);
        }
    }

    /// Set the master volume (0..65535)
    pub fn set_master_volume(&self, volume: u16) {
        self.master_volume.store(volume as u32, Ordering::Relaxed);
    }

    /// Get the master volume (0..65535)
    pub fn get_master_volume(&self) -> u16 {
        self.master_volume.load(Ordering::Relaxed) as u16
    }

    /// Mix all active channels into the provided output buffer
    ///
    /// For each sample position, reads from all channels, applies per-channel
    /// volume and mute, sums with saturation, then applies master volume.
    /// The result is written directly to `output`.
    pub fn mix_to_output(&mut self, output: &mut [i16]) {
        let master_vol = fp_from_volume(self.get_master_volume());
        let num_samples = output.len();

        // Clear output buffer
        for sample in output.iter_mut() {
            *sample = 0;
        }

        // If no channels or master muted, output silence
        if self.channels.is_empty() || master_vol == 0 {
            return;
        }

        // Mix each channel into the output
        for channel in self.channels.iter_mut() {
            if channel.is_muted() {
                continue;
            }

            let ch_vol = fp_from_volume(channel.get_volume());
            if ch_vol == 0 {
                continue;
            }

            let samples = channel.read_samples(num_samples);
            for (i, &sample) in samples.iter().enumerate() {
                if i >= num_samples {
                    break;
                }

                // Convert sample to fixed-point
                let fp_sample = fp_from_i16(sample);

                // Apply channel volume
                let scaled = fp_mul(fp_sample, ch_vol);

                // Apply master volume
                let mastered = fp_mul(scaled, master_vol);

                // Accumulate with saturation
                let existing = fp_from_i16(output[i]);
                let mixed = fp_add_saturate(existing, mastered);
                output[i] = fp_to_i16(mixed);
            }
        }
    }

    /// Return the number of active channels
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Get a mutable reference to a channel by ID
    pub fn get_channel_mut(&mut self, id: u16) -> Option<&mut MixerChannel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    /// Write samples to a specific channel
    pub fn write_channel_samples(&mut self, channel_id: u16, samples: &[i16]) {
        if let Some(ch) = self.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.write_samples(samples);
        }
    }
}

// ============================================================================
// Global Mixer State
// ============================================================================

static MIXER: spin::Mutex<Option<AudioMixer>> = spin::Mutex::new(None);

/// Initialize the global audio mixer
pub fn init(sample_rate: u32) -> Result<(), KernelError> {
    let mut mixer = MIXER.lock();
    if mixer.is_some() {
        return Err(KernelError::InvalidState {
            expected: "uninitialized",
            actual: "already initialized",
        });
    }

    *mixer = Some(AudioMixer::new(sample_rate, 2));
    println!("[AUDIO] Mixer initialized at {} Hz, stereo", sample_rate);
    Ok(())
}

/// Access the global mixer through a closure
pub fn with_mixer<R, F: FnOnce(&mut AudioMixer) -> R>(f: F) -> Result<R, KernelError> {
    let mut guard = MIXER.lock();
    match guard.as_mut() {
        Some(mixer) => Ok(f(mixer)),
        None => Err(KernelError::NotInitialized {
            subsystem: "audio mixer",
        }),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fp_from_i16() {
        assert_eq!(fp_from_i16(0), 0);
        assert_eq!(fp_from_i16(1), FP_ONE);
        assert_eq!(fp_from_i16(-1), -FP_ONE);
        assert_eq!(fp_from_i16(i16::MAX), (i16::MAX as i32) << FP_SHIFT);
    }

    #[test]
    fn test_fp_to_i16_roundtrip() {
        for val in [0i16, 1, -1, 100, -100, i16::MAX, i16::MIN] {
            assert_eq!(fp_to_i16(fp_from_i16(val)), val);
        }
    }

    #[test]
    fn test_fp_to_i16_saturation() {
        // Value larger than i16::MAX should saturate
        assert_eq!(fp_to_i16(i32::MAX), i16::MAX);
        // Value smaller than i16::MIN should saturate
        assert_eq!(fp_to_i16(i32::MIN), i16::MIN);
    }

    #[test]
    fn test_fp_mul_basic() {
        // 1.0 * 1.0 = 1.0
        assert_eq!(fp_mul(FP_ONE, FP_ONE), FP_ONE);
        // 1.0 * 0.0 = 0.0
        assert_eq!(fp_mul(FP_ONE, 0), 0);
        // 2.0 * 0.5 = 1.0
        let two = 2 << FP_SHIFT;
        let half = FP_ONE / 2;
        assert_eq!(fp_mul(two, half), FP_ONE);
    }

    #[test]
    fn test_fp_mul_saturation() {
        // Large values should saturate instead of wrapping
        let large = i32::MAX;
        let result = fp_mul(large, large);
        assert_eq!(result, FP_MAX);
    }

    #[test]
    fn test_fp_from_volume() {
        assert_eq!(fp_from_volume(0), 0);
        assert_eq!(fp_from_volume(65535), 65535);
        assert_eq!(fp_from_volume(32768), 32768);
    }

    #[test]
    fn test_mixer_add_remove_channel() {
        let mut mixer = AudioMixer::new(48000, 2);
        assert_eq!(mixer.channel_count(), 0);

        let id1 = mixer.add_channel("test1");
        assert_eq!(mixer.channel_count(), 1);

        let id2 = mixer.add_channel("test2");
        assert_eq!(mixer.channel_count(), 2);

        mixer.remove_channel(id1);
        assert_eq!(mixer.channel_count(), 1);

        mixer.remove_channel(id2);
        assert_eq!(mixer.channel_count(), 0);
    }

    #[test]
    fn test_mixer_volume_scaling() {
        let mut mixer = AudioMixer::new(48000, 1);
        let ch_id = mixer.add_channel("test");

        // Write a known sample pattern
        let input = [16384i16; 4]; // ~0.5 amplitude
        mixer.write_channel_samples(ch_id, &input);

        // Full volume: output should be close to input
        let mut output = [0i16; 4];
        mixer.mix_to_output(&mut output);

        // With full channel and master volume, samples pass through
        // (volume is 65535/65536 ~= 1.0 so there may be tiny rounding)
        for &sample in &output {
            assert!(sample > 16000 && sample < 16500,
                "Expected ~16384, got {}", sample);
        }
    }

    #[test]
    fn test_mixer_two_channels() {
        let mut mixer = AudioMixer::new(48000, 1);
        let ch1 = mixer.add_channel("ch1");
        let ch2 = mixer.add_channel("ch2");

        // Two channels with the same data should sum
        let samples1 = [8000i16; 4];
        let samples2 = [8000i16; 4];
        mixer.write_channel_samples(ch1, &samples1);
        mixer.write_channel_samples(ch2, &samples2);

        let mut output = [0i16; 4];
        mixer.mix_to_output(&mut output);

        // Sum of two channels should be roughly double (with volume scaling)
        for &sample in &output {
            assert!(sample > 14000, "Expected combined ~16000, got {}", sample);
        }
    }

    #[test]
    fn test_mixer_muted_channel() {
        let mut mixer = AudioMixer::new(48000, 1);
        let ch_id = mixer.add_channel("muted");

        let input = [16384i16; 4];
        mixer.write_channel_samples(ch_id, &input);

        // Mute the channel
        if let Some(ch) = mixer.get_channel_mut(ch_id) {
            ch.set_muted(true);
        }

        let mut output = [0i16; 4];
        mixer.mix_to_output(&mut output);

        // Muted channel should produce silence
        for &sample in &output {
            assert_eq!(sample, 0);
        }
    }

    #[test]
    fn test_mixer_saturation() {
        let mut mixer = AudioMixer::new(48000, 1);
        let ch1 = mixer.add_channel("ch1");
        let ch2 = mixer.add_channel("ch2");

        // Two channels at near-max amplitude should saturate, not wrap
        let max_samples = [i16::MAX; 4];
        mixer.write_channel_samples(ch1, &max_samples);
        mixer.write_channel_samples(ch2, &max_samples);

        let mut output = [0i16; 4];
        mixer.mix_to_output(&mut output);

        // Should saturate at i16::MAX, not wrap to negative
        for &sample in &output {
            assert!(sample > 0, "Saturation failed: got {}", sample);
        }
    }
}
