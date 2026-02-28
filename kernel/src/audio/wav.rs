//! WAV file parser (PCM only)
//!
//! Parses RIFF/WAVE files and extracts PCM audio data. Supports common
//! PCM formats (8-bit unsigned, 16-bit signed, 24-bit signed) and
//! provides conversion utilities to normalize audio to S16Le for mixing.
//!
//! ## Supported Formats
//!
//! - PCM 8-bit unsigned (audio_format=1, bits_per_sample=8)
//! - PCM 16-bit signed little-endian (audio_format=1, bits_per_sample=16)
//! - PCM 24-bit signed little-endian (audio_format=1, bits_per_sample=24)
//! - PCM 32-bit signed little-endian (audio_format=1, bits_per_sample=32)

#![allow(dead_code)]

use alloc::vec::Vec;

use crate::audio::{AudioConfig, SampleFormat};
use crate::error::KernelError;

// ============================================================================
// WAV File Constants
// ============================================================================

/// "RIFF" magic bytes
const RIFF_MAGIC: [u8; 4] = [b'R', b'I', b'F', b'F'];

/// "WAVE" format identifier
const WAVE_MAGIC: [u8; 4] = [b'W', b'A', b'V', b'E'];

/// "fmt " chunk identifier
const FMT_CHUNK_ID: [u8; 4] = [b'f', b'm', b't', b' '];

/// "data" chunk identifier
const DATA_CHUNK_ID: [u8; 4] = [b'd', b'a', b't', b'a'];

/// PCM audio format (uncompressed)
const AUDIO_FORMAT_PCM: u16 = 1;

// ============================================================================
// WAV Header Structures
// ============================================================================

/// RIFF file header (12 bytes)
#[derive(Debug, Clone, Copy)]
pub struct WavHeader {
    /// Must be "RIFF"
    pub riff_magic: [u8; 4],
    /// File size minus 8 bytes (RIFF header)
    pub file_size: u32,
    /// Must be "WAVE"
    pub wave_magic: [u8; 4],
}

/// Format chunk describing the audio data layout
#[derive(Debug, Clone, Copy)]
pub struct FmtChunk {
    /// Chunk identifier: "fmt "
    pub chunk_id: [u8; 4],
    /// Chunk data size (16 for PCM)
    pub chunk_size: u32,
    /// Audio format (1 = PCM)
    pub audio_format: u16,
    /// Number of channels (1 = mono, 2 = stereo)
    pub num_channels: u16,
    /// Samples per second (e.g., 44100, 48000)
    pub sample_rate: u32,
    /// Bytes per second (sample_rate * block_align)
    pub byte_rate: u32,
    /// Block alignment (num_channels * bits_per_sample / 8)
    pub block_align: u16,
    /// Bits per sample (8, 16, 24, 32)
    pub bits_per_sample: u16,
}

// ============================================================================
// Parsed WAV File
// ============================================================================

/// Parsed WAV file with metadata and data location
#[derive(Debug, Clone)]
pub struct WavFile {
    /// Number of audio channels
    pub num_channels: u16,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Bits per sample
    pub bits_per_sample: u16,
    /// Audio format code (1 = PCM)
    pub audio_format: u16,
    /// Byte offset to the start of PCM data in the source buffer
    pub data_offset: usize,
    /// Size of the PCM data section in bytes
    pub data_size: usize,
    /// Block alignment
    pub block_align: u16,
}

impl WavFile {
    /// Parse a WAV file from a byte buffer
    ///
    /// Validates the RIFF/WAVE header, locates the "fmt " and "data" chunks,
    /// and extracts metadata.
    pub fn parse(data: &[u8]) -> Result<WavFile, KernelError> {
        // Minimum WAV file: 12 (RIFF header) + 24 (fmt chunk) + 8 (data header) = 44
        if data.len() < 44 {
            return Err(KernelError::InvalidArgument {
                name: "wav_data",
                value: "file too small for WAV header",
            });
        }

        // Validate RIFF header
        if data[0..4] != RIFF_MAGIC {
            return Err(KernelError::InvalidArgument {
                name: "wav_data",
                value: "missing RIFF magic",
            });
        }

        if data[8..12] != WAVE_MAGIC {
            return Err(KernelError::InvalidArgument {
                name: "wav_data",
                value: "missing WAVE magic",
            });
        }

        // Search for fmt and data chunks
        let mut pos = 12;
        let mut fmt_found = false;
        let mut num_channels: u16 = 0;
        let mut sample_rate: u32 = 0;
        let mut bits_per_sample: u16 = 0;
        let mut audio_format: u16 = 0;
        let mut block_align: u16 = 0;
        let mut data_offset: usize = 0;
        let mut data_size: usize = 0;
        let mut data_found = false;

        while pos + 8 <= data.len() {
            let chunk_id = [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]];
            let chunk_size = read_u32_le(&data[pos + 4..pos + 8]) as usize;

            if chunk_id == FMT_CHUNK_ID {
                if pos + 8 + 16 > data.len() {
                    return Err(KernelError::InvalidArgument {
                        name: "wav_data",
                        value: "fmt chunk truncated",
                    });
                }

                let fmt_data = &data[pos + 8..];
                audio_format = read_u16_le(&fmt_data[0..2]);
                num_channels = read_u16_le(&fmt_data[2..4]);
                sample_rate = read_u32_le(&fmt_data[4..8]);
                // byte_rate at [8..12]
                block_align = read_u16_le(&fmt_data[12..14]);
                bits_per_sample = read_u16_le(&fmt_data[14..16]);

                if audio_format != AUDIO_FORMAT_PCM {
                    return Err(KernelError::InvalidArgument {
                        name: "audio_format",
                        value: "only PCM format supported",
                    });
                }

                fmt_found = true;
            } else if chunk_id == DATA_CHUNK_ID {
                data_offset = pos + 8;
                data_size = chunk_size.min(data.len() - data_offset);
                data_found = true;
            }

            // Move to next chunk (chunks are word-aligned)
            let padded_size = (chunk_size + 1) & !1;
            pos += 8 + padded_size;
        }

        if !fmt_found {
            return Err(KernelError::InvalidArgument {
                name: "wav_data",
                value: "missing fmt chunk",
            });
        }

        if !data_found {
            return Err(KernelError::InvalidArgument {
                name: "wav_data",
                value: "missing data chunk",
            });
        }

        Ok(WavFile {
            num_channels,
            sample_rate,
            bits_per_sample,
            audio_format,
            data_offset,
            data_size,
            block_align,
        })
    }

    /// Get a reference to the PCM data within the source buffer
    pub fn sample_data<'a>(&self, source: &'a [u8]) -> &'a [u8] {
        let end = (self.data_offset + self.data_size).min(source.len());
        &source[self.data_offset..end]
    }

    /// Convert this WAV file's format info to an `AudioConfig`
    pub fn to_audio_config(&self) -> AudioConfig {
        let format = match self.bits_per_sample {
            8 => SampleFormat::U8,
            16 => SampleFormat::S16Le,
            24 => SampleFormat::S24Le,
            32 => SampleFormat::S32Le,
            _ => SampleFormat::S16Le, // fallback
        };

        AudioConfig {
            sample_rate: self.sample_rate,
            channels: self.num_channels as u8,
            format,
            buffer_frames: 1024,
        }
    }

    /// Calculate the duration of the audio in milliseconds
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 || self.block_align == 0 {
            return 0;
        }
        let total_frames = self.data_size as u64 / self.block_align as u64;
        (total_frames * 1000) / self.sample_rate as u64
    }

    /// Calculate the total number of samples (all channels)
    pub fn total_samples(&self) -> usize {
        if self.bits_per_sample == 0 {
            return 0;
        }
        let bytes_per_sample = self.bits_per_sample as usize / 8;
        if bytes_per_sample == 0 {
            return 0;
        }
        self.data_size / bytes_per_sample
    }
}

// ============================================================================
// Format Conversion Utilities
// ============================================================================

/// Convert unsigned 8-bit PCM to signed 16-bit PCM
///
/// U8 samples are centered at 128 (silence). This function shifts to signed
/// and scales to 16-bit range.
pub fn convert_u8_to_s16(data: &[u8]) -> Vec<i16> {
    let mut output = Vec::with_capacity(data.len());
    for &sample in data {
        // U8 is unsigned 0..255 centered at 128
        // Convert to signed: subtract 128, then scale to 16-bit
        let signed = (sample as i16) - 128;
        output.push(signed << 8); // Scale 8-bit to 16-bit range
    }
    output
}

/// Convert signed 24-bit little-endian PCM to signed 16-bit PCM
///
/// Reads packed 3-byte samples and truncates to 16-bit by discarding the
/// least significant 8 bits.
pub fn convert_s24_to_s16(data: &[u8]) -> Vec<i16> {
    let num_samples = data.len() / 3;
    let mut output = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let offset = i * 3;
        if offset + 2 >= data.len() {
            break;
        }

        // 24-bit LE: [low, mid, high]
        // Reconstruct as i32, then sign-extend from 24 bits
        let low = data[offset] as i32;
        let mid = data[offset + 1] as i32;
        let high = data[offset + 2] as i32;
        let sample_24 = low | (mid << 8) | (high << 16);

        // Sign-extend from 24 bits
        let signed = if sample_24 & 0x800000 != 0 {
            sample_24 | !0xFFFFFF_u32 as i32
        } else {
            sample_24
        };

        // Truncate to 16-bit: shift right by 8
        output.push((signed >> 8) as i16);
    }
    output
}

/// Convert signed 32-bit little-endian PCM to signed 16-bit PCM
///
/// Discards the lower 16 bits of each 32-bit sample.
pub fn convert_s32_to_s16(data: &[u8]) -> Vec<i16> {
    let num_samples = data.len() / 4;
    let mut output = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let offset = i * 4;
        if offset + 3 >= data.len() {
            break;
        }

        let sample_32 = read_i32_le(&data[offset..offset + 4]);
        // Truncate to 16-bit: shift right by 16
        output.push((sample_32 >> 16) as i16);
    }
    output
}

// ============================================================================
// Little-Endian Reading Helpers
// ============================================================================

#[inline]
fn read_u16_le(data: &[u8]) -> u16 {
    u16::from_le_bytes([data[0], data[1]])
}

#[inline]
fn read_u32_le(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

#[inline]
fn read_i32_le(data: &[u8]) -> i32 {
    i32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid WAV file in memory
    fn build_test_wav(
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
        pcm_data: &[u8],
    ) -> Vec<u8> {
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * block_align as u32;
        let data_size = pcm_data.len() as u32;
        let fmt_chunk_size: u32 = 16;
        let file_size = 4 + (8 + fmt_chunk_size) + (8 + data_size);

        let mut wav = Vec::new();

        // RIFF header
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&fmt_chunk_size.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(pcm_data);

        wav
    }

    #[test]
    fn test_parse_valid_wav() {
        let pcm = [0u8; 100]; // 50 samples of 16-bit mono
        let wav_data = build_test_wav(1, 44100, 16, &pcm);

        let wav = WavFile::parse(&wav_data).unwrap();
        assert_eq!(wav.num_channels, 1);
        assert_eq!(wav.sample_rate, 44100);
        assert_eq!(wav.bits_per_sample, 16);
        assert_eq!(wav.audio_format, 1);
        assert_eq!(wav.data_size, 100);
    }

    #[test]
    fn test_parse_stereo_wav() {
        let pcm = [0u8; 200]; // 50 frames of 16-bit stereo
        let wav_data = build_test_wav(2, 48000, 16, &pcm);

        let wav = WavFile::parse(&wav_data).unwrap();
        assert_eq!(wav.num_channels, 2);
        assert_eq!(wav.sample_rate, 48000);
        assert_eq!(wav.total_samples(), 100); // 200 bytes / 2 bytes per sample
    }

    #[test]
    fn test_parse_invalid_magic() {
        let mut wav_data = build_test_wav(1, 44100, 16, &[0u8; 10]);
        wav_data[0] = b'X'; // Corrupt RIFF magic

        let result = WavFile::parse(&wav_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_wave_magic() {
        let mut wav_data = build_test_wav(1, 44100, 16, &[0u8; 10]);
        wav_data[8] = b'X'; // Corrupt WAVE magic

        let result = WavFile::parse(&wav_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_too_small() {
        let data = [0u8; 20]; // Too small for a WAV header
        let result = WavFile::parse(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_sample_data() {
        let pcm = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let wav_data = build_test_wav(1, 44100, 16, &pcm);
        let wav = WavFile::parse(&wav_data).unwrap();
        let samples = wav.sample_data(&wav_data);
        assert_eq!(samples, &pcm);
    }

    #[test]
    fn test_to_audio_config() {
        let pcm = [0u8; 16];
        let wav_data = build_test_wav(2, 48000, 16, &pcm);
        let wav = WavFile::parse(&wav_data).unwrap();
        let config = wav.to_audio_config();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.format, SampleFormat::S16Le);
    }

    #[test]
    fn test_duration_ms() {
        // 44100 Hz, mono, 16-bit: 2 bytes per frame
        // 88200 bytes = 44100 frames = 1000ms
        let pcm = [0u8; 88200];
        let wav_data = build_test_wav(1, 44100, 16, &pcm);
        let wav = WavFile::parse(&wav_data).unwrap();
        assert_eq!(wav.duration_ms(), 1000);
    }

    #[test]
    fn test_convert_u8_to_s16() {
        let data = [128u8, 0, 255]; // Silence, min, max
        let result = convert_u8_to_s16(&data);
        assert_eq!(result[0], 0);        // 128 -> 0
        assert_eq!(result[1], -128 << 8); // 0 -> -128*256 = -32768
        assert_eq!(result[2], 127 << 8);  // 255 -> 127*256 = 32512
    }

    #[test]
    fn test_convert_s24_to_s16() {
        // 24-bit sample: 0x007FFF (positive max / 256)
        let data = [0xFF, 0x7F, 0x00]; // LE: low=0xFF, mid=0x7F, high=0x00
        let result = convert_s24_to_s16(&data);
        assert_eq!(result[0], 0x7F); // 0x007FFF >> 8 = 0x7F
    }

    #[test]
    fn test_total_samples() {
        let pcm = [0u8; 40]; // 40 bytes
        let wav_data = build_test_wav(2, 44100, 16, &pcm);
        let wav = WavFile::parse(&wav_data).unwrap();
        // 40 bytes / 2 bytes per sample = 20 samples
        assert_eq!(wav.total_samples(), 20);
    }
}
