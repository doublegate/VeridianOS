//! Client API for audio streams
//!
//! Provides the user-facing interface for creating, controlling, and writing
//! to audio streams. Each stream is backed by a `SharedAudioBuffer` and a
//! corresponding mixer channel.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::{
    audio::{buffer::SharedAudioBuffer, AudioConfig},
    error::KernelError,
};

/// Default ring buffer capacity in frames
const DEFAULT_BUFFER_FRAMES: u32 = 4096;

// ============================================================================
// Audio Stream Types
// ============================================================================

/// Unique identifier for an audio stream
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioStreamId(pub u32);

impl AudioStreamId {
    /// Get the raw numeric ID
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

/// State of an audio stream
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    /// Stream is created but not playing
    Stopped,
    /// Stream is actively playing audio
    Playing,
    /// Stream is temporarily paused
    Paused,
}

/// An active audio stream with buffer and mixer channel association
pub struct AudioStream {
    /// Unique stream identifier
    pub id: AudioStreamId,
    /// Audio configuration for this stream
    pub config: AudioConfig,
    /// Ring buffer for sample data
    pub buffer: SharedAudioBuffer,
    /// Associated mixer channel ID
    pub mixer_channel_id: u16,
    /// Current stream state
    pub state: StreamState,
    /// Human-readable stream name
    pub name: String,
}

// ============================================================================
// Audio Client Manager
// ============================================================================

/// Manages all active audio streams
pub struct AudioClient {
    /// Map of stream ID -> AudioStream
    streams: BTreeMap<u32, AudioStream>,
    /// Next stream ID counter
    next_id: AtomicU32,
}

impl AudioClient {
    /// Create a new audio client manager
    fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
            next_id: AtomicU32::new(1),
        }
    }

    /// Create a new audio stream
    ///
    /// Allocates a ring buffer and registers a mixer channel for the stream.
    /// The stream starts in the `Stopped` state.
    pub fn create_stream(
        &mut self,
        name: &str,
        config: AudioConfig,
    ) -> Result<AudioStreamId, KernelError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let stream_id = AudioStreamId(id);

        // Create ring buffer for this stream
        let buffer_frames = if config.buffer_frames > 0 {
            config.buffer_frames
        } else {
            DEFAULT_BUFFER_FRAMES
        };
        let buffer = SharedAudioBuffer::new(buffer_frames, config);

        // Register a mixer channel for this stream
        let mixer_channel_id = crate::audio::mixer::with_mixer(|mixer| mixer.add_channel(name))?;

        let stream = AudioStream {
            id: stream_id,
            config,
            buffer,
            mixer_channel_id,
            state: StreamState::Stopped,
            name: String::from(name),
        };

        self.streams.insert(id, stream);
        println!("[AUDIO] Created stream '{}' (id={})", name, id);

        Ok(stream_id)
    }

    /// Destroy an audio stream, freeing its buffer and mixer channel
    pub fn destroy_stream(&mut self, id: AudioStreamId) -> Result<(), KernelError> {
        let stream = self.streams.remove(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        // Remove the mixer channel
        let _ = crate::audio::mixer::with_mixer(|mixer| {
            mixer.remove_channel(stream.mixer_channel_id);
        });

        println!("[AUDIO] Destroyed stream '{}' (id={})", stream.name, id.0);
        Ok(())
    }

    /// Write i16 samples to a stream's buffer
    ///
    /// Returns the number of samples actually written (may be less if the
    /// buffer is full).
    pub fn write_samples(
        &mut self,
        id: AudioStreamId,
        samples: &[i16],
    ) -> Result<usize, KernelError> {
        let stream = self.streams.get_mut(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        let written = stream.buffer.write_samples(samples);

        // Also feed samples to the mixer channel so they appear in mixed output
        let channel_id = stream.mixer_channel_id;
        let _ = crate::audio::mixer::with_mixer(|mixer| {
            mixer.write_channel_samples(channel_id, samples);
        });

        Ok(written)
    }

    /// Start playing a stream
    pub fn play(&mut self, id: AudioStreamId) -> Result<(), KernelError> {
        let stream = self.streams.get_mut(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        stream.state = StreamState::Playing;
        println!("[AUDIO] Stream '{}' (id={}) -> Playing", stream.name, id.0);
        Ok(())
    }

    /// Pause a playing stream
    pub fn pause(&mut self, id: AudioStreamId) -> Result<(), KernelError> {
        let stream = self.streams.get_mut(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        if stream.state != StreamState::Playing {
            return Err(KernelError::InvalidState {
                expected: "Playing",
                actual: "not Playing",
            });
        }

        stream.state = StreamState::Paused;
        println!("[AUDIO] Stream '{}' (id={}) -> Paused", stream.name, id.0);
        Ok(())
    }

    /// Stop a stream (resets to beginning)
    pub fn stop(&mut self, id: AudioStreamId) -> Result<(), KernelError> {
        let stream = self.streams.get_mut(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        stream.state = StreamState::Stopped;
        println!("[AUDIO] Stream '{}' (id={}) -> Stopped", stream.name, id.0);
        Ok(())
    }

    /// Set the volume for a stream (0..65535 maps to 0.0..1.0)
    pub fn set_volume(&self, id: AudioStreamId, volume: u16) -> Result<(), KernelError> {
        let stream = self.streams.get(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;

        let channel_id = stream.mixer_channel_id;
        crate::audio::mixer::with_mixer(|mixer| {
            mixer.set_volume(channel_id, volume);
        })?;

        Ok(())
    }

    /// Get the state of a stream
    pub fn get_state(&self, id: AudioStreamId) -> Result<StreamState, KernelError> {
        let stream = self.streams.get(&id.0).ok_or(KernelError::NotFound {
            resource: "audio stream",
            id: id.0 as u64,
        })?;
        Ok(stream.state)
    }

    /// Get the number of active streams
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }
}

// ============================================================================
// Global Client State
// ============================================================================

static CLIENT: spin::Mutex<Option<AudioClient>> = spin::Mutex::new(None);

/// Initialize the global audio client manager
pub fn init() {
    let mut client = CLIENT.lock();
    *client = Some(AudioClient::new());
    println!("[AUDIO] Client manager initialized");
}

/// Access the global audio client through a closure
pub fn with_client<R, F: FnOnce(&mut AudioClient) -> R>(f: F) -> Result<R, KernelError> {
    let mut guard = CLIENT.lock();
    match guard.as_mut() {
        Some(client) => Ok(f(client)),
        None => Err(KernelError::NotInitialized {
            subsystem: "audio client",
        }),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SampleFormat;

    fn test_config() -> AudioConfig {
        AudioConfig {
            sample_rate: 48000,
            channels: 2,
            format: SampleFormat::S16Le,
            buffer_frames: 1024,
        }
    }

    #[test]
    fn test_audio_stream_id() {
        let id = AudioStreamId(42);
        assert_eq!(id.as_u32(), 42);
    }

    #[test]
    fn test_stream_state() {
        assert_eq!(StreamState::Stopped, StreamState::Stopped);
        assert_ne!(StreamState::Playing, StreamState::Paused);
    }
}
