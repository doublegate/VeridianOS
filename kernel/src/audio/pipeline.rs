//! Audio output pipeline
//!
//! Manages the flow of mixed audio data to the output device. The pipeline
//! periodically calls the mixer to produce output frames and tracks
//! statistics such as total frames processed and buffer underrun events.

#![allow(dead_code)]

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{audio::AudioConfig, error::KernelError};

// ============================================================================
// Pipeline State
// ============================================================================

/// Current state of the audio output pipeline
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Pipeline is idle (not producing output)
    Idle,
    /// Pipeline is actively processing and outputting audio
    Running,
    /// Pipeline is draining remaining buffered data before stopping
    Draining,
}

/// Pipeline performance statistics
#[derive(Debug, Clone, Copy)]
pub struct PipelineStats {
    /// Total number of audio frames processed since pipeline start
    pub frames_processed: u64,
    /// Number of buffer underrun events (mixer had no data)
    pub underruns: u64,
    /// Current pipeline state
    pub state: PipelineState,
}

// ============================================================================
// Audio Pipeline
// ============================================================================

/// Audio output pipeline
///
/// Coordinates the mixer with the output device, managing buffering,
/// underrun detection, and output timing.
pub struct AudioPipeline {
    /// Current pipeline state
    state: PipelineState,
    /// Pre-allocated output buffer for mixed audio (one period)
    output_buffer: Vec<i16>,
    /// Output audio configuration
    output_config: AudioConfig,
    /// Total frames processed
    frames_processed: AtomicU64,
    /// Total underrun events
    underruns: AtomicU64,
}

impl AudioPipeline {
    /// Create a new audio pipeline with the given output configuration
    pub fn new(config: AudioConfig) -> Self {
        let buffer_size = config.buffer_frames as usize * config.channels as usize;
        Self {
            state: PipelineState::Idle,
            output_buffer: alloc::vec![0i16; buffer_size],
            output_config: config,
            frames_processed: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
        }
    }

    /// Process one frame period: mix all active channels into the output buffer
    ///
    /// Calls the mixer to fill the output buffer with mixed audio. Returns
    /// a reference to the buffer containing the mixed samples.
    pub fn process_frame(&mut self) -> &[i16] {
        if self.state != PipelineState::Running {
            // Not running: return silence
            for sample in self.output_buffer.iter_mut() {
                *sample = 0;
            }
            return &self.output_buffer;
        }

        // Ask the mixer to fill our output buffer
        let result = crate::audio::mixer::with_mixer(|mixer| {
            mixer.mix_to_output(&mut self.output_buffer);
        });

        match result {
            Ok(()) => {
                // Check if the output is all silence (possible underrun)
                let all_silence = self.output_buffer.iter().all(|&s| s == 0);
                if all_silence {
                    self.underruns.fetch_add(1, Ordering::Relaxed);
                }

                // Submit mixed audio to VirtIO-Sound hardware (if present)
                if !all_silence {
                    let _ = crate::audio::virtio_sound::with_device(|dev| {
                        let _ = dev.write_pcm(0, &self.output_buffer);
                    });
                }

                let frames = self.output_config.buffer_frames as u64;
                self.frames_processed.fetch_add(frames, Ordering::Relaxed);
            }
            Err(_) => {
                // Mixer not available: output silence
                for sample in self.output_buffer.iter_mut() {
                    *sample = 0;
                }
                self.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }

        &self.output_buffer
    }

    /// Start the pipeline
    pub fn start(&mut self) {
        if self.state == PipelineState::Idle {
            self.state = PipelineState::Running;
            self.frames_processed.store(0, Ordering::Relaxed);
            self.underruns.store(0, Ordering::Relaxed);
            println!("[AUDIO] Pipeline started");
        }
    }

    /// Stop the pipeline immediately
    pub fn stop(&mut self) {
        self.state = PipelineState::Idle;
        // Clear output buffer
        for sample in self.output_buffer.iter_mut() {
            *sample = 0;
        }
        println!("[AUDIO] Pipeline stopped");
    }

    /// Drain the pipeline: process remaining buffered data then stop
    pub fn drain(&mut self) {
        if self.state == PipelineState::Running {
            self.state = PipelineState::Draining;
            println!("[AUDIO] Pipeline draining...");

            // Process one last frame to flush any remaining data
            self.process_frame_internal();

            self.state = PipelineState::Idle;
            println!("[AUDIO] Pipeline drain complete");
        }
    }

    /// Internal frame processing (used during drain)
    fn process_frame_internal(&mut self) {
        let _ = crate::audio::mixer::with_mixer(|mixer| {
            mixer.mix_to_output(&mut self.output_buffer);
        });

        let frames = self.output_config.buffer_frames as u64;
        self.frames_processed.fetch_add(frames, Ordering::Relaxed);
    }

    /// Get pipeline statistics
    pub fn stats(&self) -> PipelineStats {
        PipelineStats {
            frames_processed: self.frames_processed.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            state: self.state,
        }
    }

    /// Get the current pipeline state
    pub fn state(&self) -> PipelineState {
        self.state
    }

    /// Get the output configuration
    pub fn config(&self) -> &AudioConfig {
        &self.output_config
    }
}

// ============================================================================
// Global Pipeline State
// ============================================================================

static PIPELINE: spin::Mutex<Option<AudioPipeline>> = spin::Mutex::new(None);

/// Initialize the global audio pipeline
pub fn init(config: AudioConfig) -> Result<(), KernelError> {
    let mut pipeline = PIPELINE.lock();
    if pipeline.is_some() {
        return Err(KernelError::InvalidState {
            expected: "uninitialized",
            actual: "already initialized",
        });
    }

    *pipeline = Some(AudioPipeline::new(config));
    println!(
        "[AUDIO] Pipeline initialized: {} Hz, {} ch, {} frames/period",
        config.sample_rate, config.channels, config.buffer_frames
    );
    Ok(())
}

/// Access the global pipeline through a closure
pub fn with_pipeline<R, F: FnOnce(&mut AudioPipeline) -> R>(f: F) -> Result<R, KernelError> {
    let mut guard = PIPELINE.lock();
    match guard.as_mut() {
        Some(pipeline) => Ok(f(pipeline)),
        None => Err(KernelError::NotInitialized {
            subsystem: "audio pipeline",
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
            buffer_frames: 256,
        }
    }

    #[test]
    fn test_pipeline_creation() {
        let config = test_config();
        let pipeline = AudioPipeline::new(config);
        assert_eq!(pipeline.state(), PipelineState::Idle);
        assert_eq!(pipeline.output_buffer.len(), 256 * 2); // 256 frames * 2
                                                           // channels
    }

    #[test]
    fn test_pipeline_start_stop() {
        let config = test_config();
        let mut pipeline = AudioPipeline::new(config);

        assert_eq!(pipeline.state(), PipelineState::Idle);

        pipeline.start();
        assert_eq!(pipeline.state(), PipelineState::Running);

        pipeline.stop();
        assert_eq!(pipeline.state(), PipelineState::Idle);
    }

    #[test]
    fn test_pipeline_stats_initial() {
        let config = test_config();
        let pipeline = AudioPipeline::new(config);
        let stats = pipeline.stats();

        assert_eq!(stats.frames_processed, 0);
        assert_eq!(stats.underruns, 0);
        assert_eq!(stats.state, PipelineState::Idle);
    }

    #[test]
    fn test_pipeline_idle_silence() {
        let config = test_config();
        let mut pipeline = AudioPipeline::new(config);

        // Pipeline is idle, should output silence
        let output = pipeline.process_frame();
        for &sample in output {
            assert_eq!(sample, 0);
        }
    }
}
