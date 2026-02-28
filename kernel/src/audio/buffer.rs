//! Ring buffer for audio data transport
//!
//! Provides lock-free single-producer single-consumer ring buffers for
//! efficient audio data transfer between producer (client) and consumer
//! (mixer/pipeline) threads.
//!
//! ## Design
//!
//! `AudioRingBuffer` is a byte-level ring buffer using atomic read/write
//! positions for lock-free operation in the SPSC case. `SharedAudioBuffer`
//! wraps it with a `Mutex` to support multi-producer scenarios and provides
//! a typed i16 sample interface.

#![allow(dead_code)]

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::audio::AudioConfig;

// ============================================================================
// Audio Ring Buffer (byte-level, SPSC lock-free)
// ============================================================================

/// Lock-free single-producer single-consumer ring buffer for audio data
///
/// Uses atomic read/write positions to allow concurrent access from one
/// producer and one consumer without locking.
pub struct AudioRingBuffer {
    /// Backing storage for audio data
    data: Vec<u8>,
    /// Current read position (byte offset, wraps around capacity)
    read_pos: AtomicU32,
    /// Current write position (byte offset, wraps around capacity)
    write_pos: AtomicU32,
    /// Total capacity in bytes
    capacity: u32,
    /// Size of one audio frame in bytes (channels * bytes_per_sample)
    frame_size: u16,
}

impl AudioRingBuffer {
    /// Create a new ring buffer with the given capacity in frames
    ///
    /// The actual byte capacity is `capacity_frames * frame_size`.
    pub fn new(capacity_frames: u32, frame_size: u16) -> Self {
        let byte_capacity = capacity_frames * frame_size as u32;
        Self {
            data: alloc::vec![0u8; byte_capacity as usize],
            read_pos: AtomicU32::new(0),
            write_pos: AtomicU32::new(0),
            capacity: byte_capacity,
            frame_size,
        }
    }

    /// Write data into the ring buffer
    ///
    /// Returns the number of bytes actually written. May be less than
    /// `data.len()` if the buffer is nearly full.
    pub fn write(&self, data: &[u8]) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        let cap = self.capacity;

        // Available space: capacity - 1 - used (reserve 1 byte to distinguish
        // full from empty)
        let used = if write >= read {
            write - read
        } else {
            cap - read + write
        };
        let available = (cap - 1 - used) as usize;
        let to_write = data.len().min(available);

        if to_write == 0 {
            return 0;
        }

        let w = write as usize;
        let c = cap as usize;

        // Write in one or two chunks depending on wrap-around
        let first_chunk = (c - w).min(to_write);
        let second_chunk = to_write - first_chunk;

        // SAFETY: We only write within bounds of self.data, and the atomic
        // positions ensure the reader does not access these regions until
        // write_pos is updated.
        let data_ptr = self.data.as_ptr() as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr.add(w), first_chunk);
            if second_chunk > 0 {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(first_chunk),
                    data_ptr,
                    second_chunk,
                );
            }
        }

        let new_write = ((w + to_write) % c) as u32;
        self.write_pos.store(new_write, Ordering::Release);

        to_write
    }

    /// Read data from the ring buffer
    ///
    /// Returns the number of bytes actually read. May be less than
    /// `output.len()` if the buffer does not contain enough data.
    pub fn read(&self, output: &mut [u8]) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        let cap = self.capacity;

        let used = if write >= read {
            (write - read) as usize
        } else {
            (cap - read + write) as usize
        };
        let to_read = output.len().min(used);

        if to_read == 0 {
            return 0;
        }

        let r = read as usize;
        let c = cap as usize;

        let first_chunk = (c - r).min(to_read);
        let second_chunk = to_read - first_chunk;

        // SAFETY: We only read within bounds of self.data, and the atomic
        // positions ensure the writer does not overwrite these regions until
        // read_pos is updated.
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.data.as_ptr().add(r),
                output.as_mut_ptr(),
                first_chunk,
            );
            if second_chunk > 0 {
                core::ptr::copy_nonoverlapping(
                    self.data.as_ptr(),
                    output.as_mut_ptr().add(first_chunk),
                    second_chunk,
                );
            }
        }

        let new_read = ((r + to_read) % c) as u32;
        self.read_pos.store(new_read, Ordering::Release);

        to_read
    }

    /// Number of frames available to read
    pub fn available_read(&self) -> u32 {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        let cap = self.capacity;

        let used_bytes = if write >= read {
            write - read
        } else {
            cap - read + write
        };

        if self.frame_size > 0 {
            used_bytes / self.frame_size as u32
        } else {
            0
        }
    }

    /// Number of frames available to write
    pub fn available_write(&self) -> u32 {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        let cap = self.capacity;

        let used_bytes = if write >= read {
            write - read
        } else {
            cap - read + write
        };
        let available_bytes = cap - 1 - used_bytes;

        if self.frame_size > 0 {
            available_bytes / self.frame_size as u32
        } else {
            0
        }
    }

    /// Clear the buffer (reset read/write positions)
    pub fn clear(&mut self) {
        self.read_pos.store(0, Ordering::Release);
        self.write_pos.store(0, Ordering::Release);
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.read_pos.load(Ordering::Acquire) == self.write_pos.load(Ordering::Acquire)
    }

    /// Check if the buffer is full (only 1 byte of slack remaining)
    pub fn is_full(&self) -> bool {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        let cap = self.capacity;
        let used = if write >= read {
            write - read
        } else {
            cap - read + write
        };
        used >= cap - 1
    }
}

// ============================================================================
// Shared Audio Buffer (mutex-protected, typed i16 interface)
// ============================================================================

/// Mutex-protected audio ring buffer with typed sample interface
///
/// Wraps an `AudioRingBuffer` with a spin mutex to support multiple
/// producers writing interleaved i16 samples.
pub struct SharedAudioBuffer {
    /// Inner ring buffer, protected by a spin mutex
    inner: spin::Mutex<AudioRingBuffer>,
    /// Audio configuration for this buffer
    config: AudioConfig,
}

impl SharedAudioBuffer {
    /// Create a new shared audio buffer
    ///
    /// # Arguments
    /// * `capacity_frames` - Number of audio frames the buffer can hold
    /// * `config` - Audio configuration (determines frame size)
    pub fn new(capacity_frames: u32, config: AudioConfig) -> Self {
        let frame_size = config.frame_size();
        Self {
            inner: spin::Mutex::new(AudioRingBuffer::new(capacity_frames, frame_size)),
            config,
        }
    }

    /// Write i16 samples into the buffer
    ///
    /// Converts the sample slice to bytes and writes into the ring buffer.
    /// Returns the number of samples actually written.
    pub fn write_samples(&self, samples: &[i16]) -> usize {
        let bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                samples.as_ptr() as *const u8,
                samples.len() * 2,
            )
        };
        let ring = self.inner.lock();
        let bytes_written = ring.write(bytes);
        bytes_written / 2
    }

    /// Read i16 samples from the buffer
    ///
    /// Reads bytes from the ring buffer and reinterprets as i16 samples.
    /// Returns the number of samples actually read.
    pub fn read_samples(&self, output: &mut [i16]) -> usize {
        let bytes: &mut [u8] = unsafe {
            core::slice::from_raw_parts_mut(
                output.as_mut_ptr() as *mut u8,
                output.len() * 2,
            )
        };
        let ring = self.inner.lock();
        let bytes_read = ring.read(bytes);
        bytes_read / 2
    }

    /// Number of frames available to read
    pub fn available_read_frames(&self) -> u32 {
        self.inner.lock().available_read()
    }

    /// Number of frames available to write
    pub fn available_write_frames(&self) -> u32 {
        self.inner.lock().available_write()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }

    /// Get the audio configuration for this buffer
    pub fn config(&self) -> &AudioConfig {
        &self.config
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
            channels: 1,
            format: SampleFormat::S16Le,
            buffer_frames: 256,
        }
    }

    #[test]
    fn test_ring_buffer_write_read() {
        let ring = AudioRingBuffer::new(64, 2);
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];

        let written = ring.write(&data);
        assert_eq!(written, 8);

        let mut output = [0u8; 8];
        let read = ring.read(&mut output);
        assert_eq!(read, 8);
        assert_eq!(output, data);
    }

    #[test]
    fn test_ring_buffer_wrap_around() {
        // Small buffer to force wrap-around
        let ring = AudioRingBuffer::new(4, 1); // 4 bytes capacity

        // Write 3 bytes (leaving 1 byte slack)
        let data1 = [10u8, 20, 30];
        let written1 = ring.write(&data1);
        assert_eq!(written1, 3);

        // Read 2 bytes to free space
        let mut out1 = [0u8; 2];
        let read1 = ring.read(&mut out1);
        assert_eq!(read1, 2);
        assert_eq!(out1, [10, 20]);

        // Write 2 more bytes (should wrap around)
        let data2 = [40u8, 50];
        let written2 = ring.write(&data2);
        assert_eq!(written2, 2);

        // Read remaining 3 bytes
        let mut out2 = [0u8; 3];
        let read2 = ring.read(&mut out2);
        assert_eq!(read2, 3);
        assert_eq!(out2, [30, 40, 50]);
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let ring = AudioRingBuffer::new(4, 1); // 4 bytes capacity

        // Try to write more than capacity
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let written = ring.write(&data);
        // Only 3 bytes should fit (capacity - 1 for full/empty distinction)
        assert_eq!(written, 3);

        assert!(ring.is_full());
    }

    #[test]
    fn test_ring_buffer_empty_read() {
        let ring = AudioRingBuffer::new(32, 2);

        assert!(ring.is_empty());

        let mut output = [0u8; 16];
        let read = ring.read(&mut output);
        assert_eq!(read, 0);
    }

    #[test]
    fn test_ring_buffer_available() {
        let ring = AudioRingBuffer::new(8, 2); // 8 frames * 2 bytes = 16 bytes capacity

        assert_eq!(ring.available_read(), 0);
        assert_eq!(ring.available_write(), 7); // capacity - 1 = 15 bytes = 7 frames

        let data = [0u8; 6]; // 3 frames
        ring.write(&data);

        assert_eq!(ring.available_read(), 3);
        assert_eq!(ring.available_write(), 4); // 15-6=9 bytes = 4 frames
    }

    #[test]
    fn test_shared_buffer_samples() {
        let config = test_config();
        let buf = SharedAudioBuffer::new(64, config);

        let samples = [100i16, 200, 300, 400];
        let written = buf.write_samples(&samples);
        assert_eq!(written, 4);

        let mut output = [0i16; 4];
        let read = buf.read_samples(&mut output);
        assert_eq!(read, 4);
        assert_eq!(output, samples);
    }

    #[test]
    fn test_shared_buffer_partial_read() {
        let config = test_config();
        let buf = SharedAudioBuffer::new(64, config);

        let samples = [10i16, 20, 30];
        buf.write_samples(&samples);

        // Read only 2 samples
        let mut output = [0i16; 2];
        let read = buf.read_samples(&mut output);
        assert_eq!(read, 2);
        assert_eq!(output, [10, 20]);

        // Read the remaining 1 sample
        let mut output2 = [0i16; 4];
        let read2 = buf.read_samples(&mut output2);
        assert_eq!(read2, 1);
        assert_eq!(output2[0], 30);
    }

    #[test]
    fn test_shared_buffer_empty() {
        let config = test_config();
        let buf = SharedAudioBuffer::new(64, config);

        assert!(buf.is_empty());
        assert_eq!(buf.available_read_frames(), 0);
    }
}
