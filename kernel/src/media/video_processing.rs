//! Video processing module for VeridianOS
//!
//! Provides four major subsystems:
//! 1. **AVI container parser** -- RIFF/AVI header parsing, stream demuxing,
//!    index (idx1) parsing, and frame extraction.
//! 2. **Frame rate conversion** -- Frame duplication, frame dropping, 3:2
//!    pulldown (telecine), timestamp-based selection, and motion-compensated
//!    linear blend interpolation. All math is integer-only.
//! 3. **Subtitle overlay** -- SRT parser, timestamp matching, 8x16 bitmap font
//!    text rendering with semi-transparent background, multi-line word
//!    wrapping, and configurable margins.
//! 4. **Real-time audio scheduling** -- Deadline scheduler integration for
//!    audio threads with period-based wake scheduling, latency/jitter tracking,
//!    underrun/overrun statistics, and CPU reservation.
//!
//! All arithmetic is integer or fixed-point. No floating-point is used
//! anywhere.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// AVI Container Parser
// ============================================================================

/// AVI file flags from the main header (avih).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AviFlags(pub u32);

impl AviFlags {
    /// File has an index chunk (idx1).
    pub const AVIF_HASINDEX: u32 = 0x0000_0010;
    /// Interleaved audio/video data.
    pub const AVIF_ISINTERLEAVED: u32 = 0x0000_0100;
    /// Use idx1 offsets from the movi list start (not file start).
    pub const AVIF_MUSTUSEINDEX: u32 = 0x0000_0020;
    /// AVI is copyrighted.
    pub const AVIF_COPYRIGHTED: u32 = 0x0002_0000;

    /// Check whether a specific flag is set.
    pub fn has_flag(&self, flag: u32) -> bool {
        self.0 & flag != 0
    }
}

/// Four-character code (FourCC) used throughout RIFF/AVI.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FourCC(pub [u8; 4]);

impl FourCC {
    pub const RIFF: Self = Self(*b"RIFF");
    pub const AVI: Self = Self(*b"AVI ");
    pub const LIST: Self = Self(*b"LIST");
    pub const AVIH: Self = Self(*b"avih");
    pub const STRH: Self = Self(*b"strh");
    pub const STRF: Self = Self(*b"strf");
    pub const IDX1: Self = Self(*b"idx1");
    pub const MOVI: Self = Self(*b"movi");
    pub const HDRL: Self = Self(*b"hdrl");
    pub const STRL: Self = Self(*b"strl");
    pub const VIDS: Self = Self(*b"vids");
    pub const AUDS: Self = Self(*b"auds");

    /// Create from a byte slice; returns None if slice is too short.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        Some(Self([data[0], data[1], data[2], data[3]]))
    }
}

impl core::fmt::Debug for FourCC {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s: [u8; 4] = self.0;
        write!(
            f,
            "FourCC('{}{}{}{}')",
            s[0] as char, s[1] as char, s[2] as char, s[3] as char
        )
    }
}

/// AVI main header (avih chunk) -- 56 bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct AviMainHeader {
    /// Microseconds per frame (frame period).
    pub microseconds_per_frame: u32,
    /// Maximum bytes per second (approximate data rate).
    pub max_bytes_per_sec: u32,
    /// Padding granularity in bytes.
    pub padding_granularity: u32,
    /// AVI flags (see [`AviFlags`]).
    pub flags: AviFlags,
    /// Total number of frames in the video stream.
    pub total_frames: u32,
    /// Number of streams that require initial frames before playback.
    pub initial_frames: u32,
    /// Number of streams in the file.
    pub streams: u32,
    /// Suggested buffer size for reading the file.
    pub suggested_buffer_size: u32,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
}

impl AviMainHeader {
    /// Parse from a byte buffer (little-endian, expects >= 40 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 40 {
            return None;
        }
        Some(Self {
            microseconds_per_frame: read_u32_le(data, 0),
            max_bytes_per_sec: read_u32_le(data, 4),
            padding_granularity: read_u32_le(data, 8),
            flags: AviFlags(read_u32_le(data, 12)),
            total_frames: read_u32_le(data, 16),
            initial_frames: read_u32_le(data, 20),
            streams: read_u32_le(data, 24),
            suggested_buffer_size: read_u32_le(data, 28),
            width: read_u32_le(data, 32),
            height: read_u32_le(data, 36),
        })
    }

    /// Compute frame rate as a rational number (numerator, denominator).
    /// Returns (fps_num, fps_den) such that fps = fps_num / fps_den.
    pub fn frame_rate(&self) -> (u32, u32) {
        if self.microseconds_per_frame == 0 {
            return (0, 1);
        }
        // fps = 1_000_000 / microseconds_per_frame
        (1_000_000, self.microseconds_per_frame)
    }
}

/// Stream type tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// Video stream (vids).
    Video,
    /// Audio stream (auds).
    Audio,
    /// Unknown / unsupported stream type.
    Unknown,
}

/// AVI stream header (strh chunk) -- 56 bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct AviStreamHeader {
    /// Stream type FourCC (vids, auds, ...).
    pub stream_type: [u8; 4],
    /// Codec handler FourCC (e.g. DIB for uncompressed, MJPG, etc.).
    pub handler: [u8; 4],
    /// Stream flags.
    pub flags: u32,
    /// Priority (used for language selection, etc.).
    pub priority: u16,
    /// Language tag.
    pub language: u16,
    /// Initial frames (delay before interleave).
    pub initial_frames: u32,
    /// Time scale (denominator of sample rate).
    pub scale: u32,
    /// Rate (numerator of sample rate). sample_rate = rate / scale.
    pub rate: u32,
    /// Start time of the stream.
    pub start: u32,
    /// Length of the stream (in `scale` units).
    pub length: u32,
    /// Suggested buffer size.
    pub suggested_buffer_size: u32,
    /// Quality indicator (-1 = default).
    pub quality: u32,
    /// Sample size (0 for variable-size, else fixed).
    pub sample_size: u32,
    /// Frame rectangle: left.
    pub frame_left: u16,
    /// Frame rectangle: top.
    pub frame_top: u16,
    /// Frame rectangle: right.
    pub frame_right: u16,
    /// Frame rectangle: bottom.
    pub frame_bottom: u16,
}

impl AviStreamHeader {
    /// Parse from a byte buffer (little-endian, expects >= 56 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 56 {
            return None;
        }
        let mut stream_type = [0u8; 4];
        stream_type.copy_from_slice(&data[0..4]);
        let mut handler = [0u8; 4];
        handler.copy_from_slice(&data[4..8]);
        Some(Self {
            stream_type,
            handler,
            flags: read_u32_le(data, 8),
            priority: read_u16_le(data, 12),
            language: read_u16_le(data, 14),
            initial_frames: read_u32_le(data, 16),
            scale: read_u32_le(data, 20),
            rate: read_u32_le(data, 24),
            start: read_u32_le(data, 28),
            length: read_u32_le(data, 32),
            suggested_buffer_size: read_u32_le(data, 36),
            quality: read_u32_le(data, 40),
            sample_size: read_u32_le(data, 44),
            frame_left: read_u16_le(data, 48),
            frame_top: read_u16_le(data, 50),
            frame_right: read_u16_le(data, 52),
            frame_bottom: read_u16_le(data, 54),
        })
    }

    /// Determine the stream type from the FourCC tag.
    pub fn get_stream_type(&self) -> StreamType {
        if self.stream_type == *b"vids" {
            StreamType::Video
        } else if self.stream_type == *b"auds" {
            StreamType::Audio
        } else {
            StreamType::Unknown
        }
    }

    /// Compute sample rate as a rational (rate / scale).
    pub fn sample_rate(&self) -> (u32, u32) {
        if self.scale == 0 {
            return (0, 1);
        }
        (self.rate, self.scale)
    }
}

/// BitmapInfoHeader (BITMAPINFOHEADER) -- 40 bytes.
/// Used in video strf chunks to describe the video format.
#[derive(Debug, Clone, Copy, Default)]
pub struct BitmapInfoHeader {
    /// Size of this structure (should be >= 40).
    pub size: u32,
    /// Image width in pixels.
    pub width: i32,
    /// Image height in pixels (positive = bottom-up, negative = top-down).
    pub height: i32,
    /// Number of color planes (must be 1).
    pub planes: u16,
    /// Bits per pixel (1, 4, 8, 16, 24, 32).
    pub bit_count: u16,
    /// Compression FourCC (0 = BI_RGB = uncompressed).
    pub compression: u32,
    /// Size of the image data (may be 0 for BI_RGB).
    pub image_size: u32,
    /// Horizontal resolution (pixels per meter).
    pub x_pels_per_meter: i32,
    /// Vertical resolution (pixels per meter).
    pub y_pels_per_meter: i32,
    /// Number of colors used (0 = all).
    pub colors_used: u32,
    /// Number of important colors (0 = all).
    pub colors_important: u32,
}

impl BitmapInfoHeader {
    /// Parse from a byte buffer (little-endian, expects >= 40 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 40 {
            return None;
        }
        Some(Self {
            size: read_u32_le(data, 0),
            width: read_i32_le(data, 4),
            height: read_i32_le(data, 8),
            planes: read_u16_le(data, 12),
            bit_count: read_u16_le(data, 14),
            compression: read_u32_le(data, 16),
            image_size: read_u32_le(data, 20),
            x_pels_per_meter: read_i32_le(data, 24),
            y_pels_per_meter: read_i32_le(data, 28),
            colors_used: read_u32_le(data, 32),
            colors_important: read_u32_le(data, 36),
        })
    }

    /// Whether the image is stored bottom-up (positive height).
    pub fn is_bottom_up(&self) -> bool {
        self.height > 0
    }

    /// Absolute height (always positive).
    pub fn abs_height(&self) -> u32 {
        if self.height < 0 {
            (-(self.height as i64)) as u32
        } else {
            self.height as u32
        }
    }
}

/// WaveFormatEx (WAVEFORMATEX) -- 18 bytes minimum.
/// Used in audio strf chunks to describe the audio format.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveFormatEx {
    /// Format tag (1 = PCM, 3 = IEEE Float, etc.).
    pub format_tag: u16,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Samples per second (Hz).
    pub samples_per_sec: u32,
    /// Average bytes per second.
    pub avg_bytes_per_sec: u32,
    /// Block alignment (channels * bits_per_sample / 8).
    pub block_align: u16,
    /// Bits per sample (8, 16, 24, 32).
    pub bits_per_sample: u16,
    /// Size of extra format data following this structure.
    pub cb_size: u16,
}

impl WaveFormatEx {
    /// PCM format tag.
    pub const WAVE_FORMAT_PCM: u16 = 1;

    /// Parse from a byte buffer (little-endian, expects >= 16 bytes).
    /// The cb_size field is optional (only present if data >= 18 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 16 {
            return None;
        }
        let cb_size = if data.len() >= 18 {
            read_u16_le(data, 16)
        } else {
            0
        };
        Some(Self {
            format_tag: read_u16_le(data, 0),
            channels: read_u16_le(data, 2),
            samples_per_sec: read_u32_le(data, 4),
            avg_bytes_per_sec: read_u32_le(data, 8),
            block_align: read_u16_le(data, 12),
            bits_per_sample: read_u16_le(data, 14),
            cb_size,
        })
    }

    /// Whether this is PCM (uncompressed) audio.
    pub fn is_pcm(&self) -> bool {
        self.format_tag == Self::WAVE_FORMAT_PCM
    }
}

/// An entry in the AVI index (idx1 chunk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AviIndexEntry {
    /// Stream chunk identifier (e.g., "00dc" for video, "01wb" for audio).
    pub chunk_id: [u8; 4],
    /// Flags -- bit 4 (0x10) = AVIIF_KEYFRAME.
    pub flags: u32,
    /// Byte offset of the chunk (from start of movi list or file).
    pub offset: u32,
    /// Size of the chunk data in bytes.
    pub size: u32,
}

impl AviIndexEntry {
    /// AVIIF_KEYFRAME flag.
    pub const AVIIF_KEYFRAME: u32 = 0x0000_0010;

    /// Parse a single index entry (16 bytes).
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 16 {
            return None;
        }
        let mut chunk_id = [0u8; 4];
        chunk_id.copy_from_slice(&data[0..4]);
        Some(Self {
            chunk_id,
            flags: read_u32_le(data, 4),
            offset: read_u32_le(data, 8),
            size: read_u32_le(data, 12),
        })
    }

    /// Whether this entry is a keyframe.
    pub fn is_keyframe(&self) -> bool {
        self.flags & Self::AVIIF_KEYFRAME != 0
    }

    /// Get the stream number from the chunk_id (first two ASCII digits).
    /// E.g., "00dc" -> 0, "01wb" -> 1.
    pub fn stream_number(&self) -> u8 {
        let d0 = self.chunk_id[0].wrapping_sub(b'0');
        let d1 = self.chunk_id[1].wrapping_sub(b'0');
        if d0 <= 9 && d1 <= 9 {
            d0 * 10 + d1
        } else {
            0
        }
    }

    /// Whether this is a video chunk (ends with "dc" or "db").
    pub fn is_video(&self) -> bool {
        self.chunk_id[2] == b'd' && (self.chunk_id[3] == b'c' || self.chunk_id[3] == b'b')
    }

    /// Whether this is an audio chunk (ends with "wb").
    pub fn is_audio(&self) -> bool {
        self.chunk_id[2] == b'w' && self.chunk_id[3] == b'b'
    }
}

/// Information about a parsed AVI stream.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct AviStreamInfo {
    /// Zero-based stream index.
    pub index: u32,
    /// Stream type.
    pub stream_type: StreamType,
    /// Stream header.
    pub header: AviStreamHeader,
    /// Video format (present if stream_type == Video).
    pub video_format: Option<BitmapInfoHeader>,
    /// Audio format (present if stream_type == Audio).
    pub audio_format: Option<WaveFormatEx>,
}

/// Parsed AVI container.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct AviContainer {
    /// Main AVI header.
    pub main_header: AviMainHeader,
    /// Stream information.
    pub streams: Vec<AviStreamInfo>,
    /// Index entries from idx1 chunk.
    pub index: Vec<AviIndexEntry>,
    /// Byte offset of the movi list data start within the file.
    pub movi_offset: u32,
    /// Total file size in bytes.
    pub file_size: u32,
}

#[cfg(feature = "alloc")]
impl AviContainer {
    /// Parse an AVI container from a byte buffer.
    ///
    /// Reads RIFF header, avih, all strh/strf pairs, and the idx1 index.
    /// Does NOT load frame data -- use [`extract_frame`] for that.
    pub fn parse(data: &[u8]) -> Option<Self> {
        // RIFF header: "RIFF" + size(4) + "AVI "
        if data.len() < 12 {
            return None;
        }
        let riff = FourCC::from_bytes(data)?;
        if riff != FourCC::RIFF {
            return None;
        }
        let _file_size = read_u32_le(data, 4);
        let form = FourCC::from_bytes(&data[8..])?;
        if form != FourCC::AVI {
            return None;
        }

        let mut main_header = AviMainHeader::default();
        let mut streams = Vec::new();
        let mut index = Vec::new();
        let mut movi_offset: u32 = 0;
        let mut pos: usize = 12;

        // Walk top-level chunks
        while pos + 8 <= data.len() {
            let chunk_id = FourCC::from_bytes(&data[pos..])?;
            let chunk_size = read_u32_le(data, pos + 4) as usize;
            let chunk_data_start = pos + 8;
            let chunk_data_end = chunk_data_start.saturating_add(chunk_size).min(data.len());

            if chunk_id == FourCC::LIST {
                if chunk_data_end < chunk_data_start + 4 {
                    pos = aligned_next(chunk_data_end);
                    continue;
                }
                let list_type = FourCC::from_bytes(&data[chunk_data_start..])?;

                if list_type == FourCC::HDRL {
                    // Parse header list
                    Self::parse_hdrl(
                        &data[chunk_data_start + 4..chunk_data_end],
                        &mut main_header,
                        &mut streams,
                    );
                } else if list_type == FourCC::MOVI {
                    movi_offset = (chunk_data_start + 4) as u32;
                }
            } else if chunk_id == FourCC::IDX1 {
                // Parse index
                Self::parse_idx1(&data[chunk_data_start..chunk_data_end], &mut index);
            }

            pos = aligned_next(chunk_data_end);
        }

        Some(Self {
            main_header,
            streams,
            index,
            movi_offset,
            file_size: data.len() as u32,
        })
    }

    /// Parse the hdrl LIST contents (avih + strl lists).
    fn parse_hdrl(data: &[u8], main_header: &mut AviMainHeader, streams: &mut Vec<AviStreamInfo>) {
        let mut pos: usize = 0;
        let mut stream_index: u32 = 0;

        while pos + 8 <= data.len() {
            let chunk_id_opt = FourCC::from_bytes(&data[pos..]);
            let chunk_id = match chunk_id_opt {
                Some(id) => id,
                None => break,
            };
            let chunk_size = read_u32_le(data, pos + 4) as usize;
            let chunk_data_start = pos + 8;
            let chunk_data_end = chunk_data_start.saturating_add(chunk_size).min(data.len());

            if chunk_id == FourCC::AVIH {
                if let Some(hdr) = AviMainHeader::parse(&data[chunk_data_start..chunk_data_end]) {
                    *main_header = hdr;
                }
            } else if chunk_id == FourCC::LIST {
                // Check for strl sub-list
                if chunk_data_end >= chunk_data_start + 4 {
                    let list_type = FourCC::from_bytes(&data[chunk_data_start..]);
                    if list_type == Some(FourCC::STRL) {
                        if let Some(info) = Self::parse_strl(
                            &data[chunk_data_start + 4..chunk_data_end],
                            stream_index,
                        ) {
                            streams.push(info);
                            stream_index += 1;
                        }
                    }
                }
            }

            // Advance past the chunk (word-aligned)
            pos = aligned_next_rel(chunk_data_end);
        }
    }

    /// Parse a stream list (strl) containing strh + strf.
    fn parse_strl(data: &[u8], stream_index: u32) -> Option<AviStreamInfo> {
        let mut header: Option<AviStreamHeader> = None;
        let mut video_format: Option<BitmapInfoHeader> = None;
        let mut audio_format: Option<WaveFormatEx> = None;
        let mut pos: usize = 0;

        while pos + 8 <= data.len() {
            let chunk_id = FourCC::from_bytes(&data[pos..])?;
            let chunk_size = read_u32_le(data, pos + 4) as usize;
            let chunk_data_start = pos + 8;
            let chunk_data_end = chunk_data_start.saturating_add(chunk_size).min(data.len());

            if chunk_id == FourCC::STRH {
                header = AviStreamHeader::parse(&data[chunk_data_start..chunk_data_end]);
            } else if chunk_id == FourCC::STRF {
                if let Some(ref hdr) = header {
                    match hdr.get_stream_type() {
                        StreamType::Video => {
                            video_format =
                                BitmapInfoHeader::parse(&data[chunk_data_start..chunk_data_end]);
                        }
                        StreamType::Audio => {
                            audio_format =
                                WaveFormatEx::parse(&data[chunk_data_start..chunk_data_end]);
                        }
                        StreamType::Unknown => {}
                    }
                }
            }

            pos = aligned_next_rel(chunk_data_end);
        }

        let hdr = header?;
        let stream_type = hdr.get_stream_type();
        Some(AviStreamInfo {
            index: stream_index,
            stream_type,
            header: hdr,
            video_format,
            audio_format,
        })
    }

    /// Parse the idx1 chunk.
    fn parse_idx1(data: &[u8], index: &mut Vec<AviIndexEntry>) {
        let mut pos: usize = 0;
        while pos + 16 <= data.len() {
            if let Some(entry) = AviIndexEntry::parse(&data[pos..]) {
                index.push(entry);
            }
            pos += 16;
        }
    }

    /// Get the first video stream info, if any.
    pub fn video_stream(&self) -> Option<&AviStreamInfo> {
        self.streams
            .iter()
            .find(|s| s.stream_type == StreamType::Video)
    }

    /// Get the first audio stream info, if any.
    pub fn audio_stream(&self) -> Option<&AviStreamInfo> {
        self.streams
            .iter()
            .find(|s| s.stream_type == StreamType::Audio)
    }

    /// Extract frame data by index from the original AVI data buffer.
    ///
    /// Returns a slice into the provided data pointing to the frame payload.
    /// `frame_index` is the zero-based video frame number in the idx1.
    pub fn extract_frame<'a>(&self, data: &'a [u8], frame_index: usize) -> Option<&'a [u8]> {
        let video_entries: Vec<&AviIndexEntry> =
            self.index.iter().filter(|e| e.is_video()).collect();
        let entry = video_entries.get(frame_index)?;

        // Offset is relative to movi list start (after "movi" tag)
        // Each chunk has an 8-byte header (fourcc + size)
        let abs_offset = (self.movi_offset as usize)
            .checked_add(entry.offset as usize)?
            .checked_add(8)?; // skip chunk header
        let end = abs_offset.checked_add(entry.size as usize)?;

        if end > data.len() {
            return None;
        }
        Some(&data[abs_offset..end])
    }

    /// Count video frames in the index.
    pub fn video_frame_count(&self) -> usize {
        self.index.iter().filter(|e| e.is_video()).count()
    }

    /// Count audio chunks in the index.
    pub fn audio_chunk_count(&self) -> usize {
        self.index.iter().filter(|e| e.is_audio()).count()
    }

    /// Get all video index entries.
    pub fn video_index_entries(&self) -> Vec<&AviIndexEntry> {
        self.index.iter().filter(|e| e.is_video()).collect()
    }

    /// Get all audio index entries.
    pub fn audio_index_entries(&self) -> Vec<&AviIndexEntry> {
        self.index.iter().filter(|e| e.is_audio()).collect()
    }

    /// Demux: separate video and audio index entries for interleaved playback.
    /// Returns (video_entries, audio_entries).
    pub fn demux_streams(&self) -> (Vec<AviIndexEntry>, Vec<AviIndexEntry>) {
        let mut video = Vec::new();
        let mut audio = Vec::new();
        for entry in &self.index {
            if entry.is_video() {
                video.push(*entry);
            } else if entry.is_audio() {
                audio.push(*entry);
            }
        }
        (video, audio)
    }
}

// ============================================================================
// Frame Rate Conversion
// ============================================================================

/// Frame rate conversion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRateMode {
    /// Duplicate frames to increase frame rate.
    Duplicate,
    /// Drop frames to decrease frame rate.
    Drop,
    /// 3:2 pulldown (telecine) for 24fps -> ~30fps (29.97 interlaced).
    /// Pattern repeats every 5 output frames from 4 source frames.
    Pulldown32,
    /// Timestamp-based selection (nearest source frame).
    TimestampSelect,
    /// Linear blend between adjacent frames (integer weighted average).
    LinearBlend,
}

/// Frame rate converter state.
#[derive(Debug, Clone)]
pub struct FrameRateConverter {
    /// Source frame rate numerator.
    pub src_fps_num: u32,
    /// Source frame rate denominator.
    pub src_fps_den: u32,
    /// Target frame rate numerator.
    pub dst_fps_num: u32,
    /// Target frame rate denominator.
    pub dst_fps_den: u32,
    /// Conversion mode.
    pub mode: FrameRateMode,
}

impl FrameRateConverter {
    /// Create a new frame rate converter.
    pub fn new(
        src_fps_num: u32,
        src_fps_den: u32,
        dst_fps_num: u32,
        dst_fps_den: u32,
        mode: FrameRateMode,
    ) -> Self {
        Self {
            src_fps_num,
            src_fps_den,
            dst_fps_num,
            dst_fps_den,
            mode,
        }
    }

    /// Compute the source frame index for a given output frame index.
    ///
    /// Uses timestamp-based selection:
    ///   source_index = output_index * src_fps_den * dst_fps_num
    ///                  / (dst_fps_den * src_fps_num)
    ///
    /// All integer arithmetic; rounds down to nearest source frame.
    pub fn source_frame_for_output(&self, output_index: u32) -> u32 {
        if self.src_fps_num == 0 || self.dst_fps_den == 0 {
            return 0;
        }
        // output_pts = output_index * dst_fps_den / dst_fps_num (in seconds *
        // dst_fps_den) source_index = output_pts * src_fps_num / src_fps_den
        // Combined: output_index * dst_fps_den * src_fps_num / (dst_fps_num *
        // src_fps_den) Wait, we want: output_index * src_fps_den * dst_fps_num
        // / (dst_fps_den * src_fps_num) Actually: source_pts * src_fps =
        // output_pts * dst_fps => source_index = output_index * dst_fps /
        // src_fps (when fps = num/den) => source_index = output_index *
        // (dst_fps_num / dst_fps_den) / (src_fps_num / src_fps_den)
        // => source_index = output_index * dst_fps_num * src_fps_den / (dst_fps_den *
        // src_fps_num) Wrong direction for pulldown/duplication -- re-derive:
        // If we output more frames, each output frame maps to an earlier source frame.
        // source_index = output_index * src_fps / dst_fps
        //              = output_index * (src_fps_num / src_fps_den) / (dst_fps_num /
        // dst_fps_den)              = output_index * src_fps_num * dst_fps_den
        // / (src_fps_den * dst_fps_num)
        let numerator = (output_index as u64)
            .checked_mul(self.src_fps_num as u64)
            .and_then(|v| v.checked_mul(self.dst_fps_den as u64))
            .unwrap_or(u64::MAX);
        let denominator = (self.src_fps_den as u64)
            .checked_mul(self.dst_fps_num as u64)
            .max(Some(1))
            .unwrap_or(1);
        (numerator / denominator) as u32
    }

    /// Compute the 3:2 pulldown pattern for a given output frame index.
    ///
    /// 3:2 pulldown maps 4 source frames to 5 output frames:
    ///   Output 0 -> Source 0  (A)
    ///   Output 1 -> Source 0  (A) -- repeated
    ///   Output 2 -> Source 1  (B)
    ///   Output 3 -> Source 2  (C)
    ///   Output 4 -> Source 2  (C) -- repeated
    ///   ... then pattern repeats with next 4 source frames.
    ///
    /// Returns (source_frame_index, is_repeated_frame).
    pub fn pulldown_32_source(&self, output_index: u32) -> (u32, bool) {
        let cycle = output_index / 5;
        let phase = output_index % 5;
        let base = cycle * 4;
        match phase {
            0 => (base, false),
            1 => (base, true),      // A repeated
            2 => (base + 1, false), // B
            3 => (base + 2, false), // C
            4 => (base + 2, true),  // C repeated
            _ => unreachable!(),
        }
    }

    /// Generate the output frame sequence for `total_output_frames`.
    ///
    /// Returns a list of (source_frame_index, blend_weight) pairs.
    /// For non-blend modes, blend_weight is always 256 (fully opaque = source
    /// frame). For LinearBlend, blend_weight is 0..256 indicating how much
    /// of the *next* source frame to blend (0 = 100% current, 256 = 100%
    /// next).
    #[cfg(feature = "alloc")]
    pub fn build_frame_map(
        &self,
        total_source_frames: u32,
        total_output_frames: u32,
    ) -> Vec<FrameMapEntry> {
        let mut map = Vec::with_capacity(total_output_frames as usize);

        for out_idx in 0..total_output_frames {
            let entry = match self.mode {
                FrameRateMode::Duplicate | FrameRateMode::Drop | FrameRateMode::TimestampSelect => {
                    let src = self
                        .source_frame_for_output(out_idx)
                        .min(total_source_frames.saturating_sub(1));
                    FrameMapEntry {
                        source_index: src,
                        blend_weight: 256,
                    }
                }
                FrameRateMode::Pulldown32 => {
                    let (src, _repeated) = self.pulldown_32_source(out_idx);
                    FrameMapEntry {
                        source_index: src.min(total_source_frames.saturating_sub(1)),
                        blend_weight: 256,
                    }
                }
                FrameRateMode::LinearBlend => {
                    self.compute_blend_entry(out_idx, total_source_frames)
                }
            };
            map.push(entry);
        }

        map
    }

    /// Compute a linear blend frame map entry.
    ///
    /// Determines which two source frames to blend and the blend weight.
    /// Uses 8.8 fixed-point for sub-frame position.
    fn compute_blend_entry(&self, output_index: u32, total_source_frames: u32) -> FrameMapEntry {
        if self.dst_fps_num == 0 || self.src_fps_den == 0 || total_source_frames == 0 {
            return FrameMapEntry {
                source_index: 0,
                blend_weight: 256,
            };
        }

        // Source position in 8.8 fixed-point
        let numerator = (output_index as u64)
            .checked_mul(self.src_fps_num as u64)
            .and_then(|v| v.checked_mul(self.dst_fps_den as u64))
            .and_then(|v| v.checked_mul(256)) // 8.8 scale
            .unwrap_or(u64::MAX);
        let denominator = (self.src_fps_den as u64)
            .checked_mul(self.dst_fps_num as u64)
            .max(Some(1))
            .unwrap_or(1);
        let src_pos_fp = (numerator / denominator) as u32;

        let src_index = (src_pos_fp >> 8).min(total_source_frames.saturating_sub(1));
        let frac = src_pos_fp & 0xFF; // 0..255

        FrameMapEntry {
            source_index: src_index,
            blend_weight: frac as u16, // 0 = 100% current, 255 = ~100% next
        }
    }
}

/// A single entry in the frame map produced by
/// [`FrameRateConverter::build_frame_map`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameMapEntry {
    /// Index of the (primary) source frame.
    pub source_index: u32,
    /// Blend weight toward the *next* source frame (0..256).
    /// 0 means 100% this frame, 256 means 100% next frame.
    /// For non-blend modes this is always 256 (use source_index as-is).
    pub blend_weight: u16,
}

/// Blend two pixel buffers using integer weighted average.
///
/// `weight` is 0..256 (0 = 100% frame_a, 256 = 100% frame_b).
/// Both buffers must have the same length. Output is written to `out`.
#[cfg(feature = "alloc")]
pub fn blend_frames(frame_a: &[u8], frame_b: &[u8], out: &mut [u8], weight: u16) {
    let len = frame_a.len().min(frame_b.len()).min(out.len());
    let w = weight as u32;
    let inv_w = 256u32.saturating_sub(w);

    for i in 0..len {
        let a = frame_a[i] as u32;
        let b = frame_b[i] as u32;
        out[i] = ((a * inv_w + b * w) >> 8) as u8;
    }
}

// ============================================================================
// Subtitle (SRT) Parser and Overlay
// ============================================================================

/// A single subtitle entry parsed from SRT format.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleEntry {
    /// Sequence number (1-based).
    pub sequence: u32,
    /// Start time in milliseconds.
    pub start_ms: u64,
    /// End time in milliseconds.
    pub end_ms: u64,
    /// Text content (may contain multiple lines separated by '\n').
    pub text: String,
}

/// Subtitle overlay configuration.
#[derive(Debug, Clone, Copy)]
pub struct SubtitleConfig {
    /// Bottom margin in pixels from the bottom of the frame.
    pub bottom_margin: u32,
    /// Left/right margin in pixels.
    pub horizontal_margin: u32,
    /// Font width in pixels (8 for the 8x16 bitmap font).
    pub font_width: u32,
    /// Font height in pixels (16 for the 8x16 bitmap font).
    pub font_height: u32,
    /// Background box opacity: 0 = transparent, 255 = fully opaque.
    pub bg_opacity: u8,
    /// Background color (R, G, B).
    pub bg_color: (u8, u8, u8),
    /// Text color (R, G, B).
    pub text_color: (u8, u8, u8),
    /// Padding inside background box in pixels.
    pub padding: u32,
}

impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            bottom_margin: 40,
            horizontal_margin: 20,
            font_width: 8,
            font_height: 16,
            bg_opacity: 180,
            bg_color: (0, 0, 0),
            text_color: (255, 255, 255),
            padding: 4,
        }
    }
}

/// Subtitle track holding all parsed entries.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    /// All subtitle entries, sorted by start_ms.
    pub entries: Vec<SubtitleEntry>,
}

#[cfg(feature = "alloc")]
impl SubtitleTrack {
    /// Parse an SRT file from a string.
    ///
    /// SRT format:
    /// ```text
    /// 1
    /// 00:00:01,000 --> 00:00:04,000
    /// Hello, world!
    ///
    /// 2
    /// 00:00:05,500 --> 00:00:08,000
    /// Second subtitle
    /// with multiple lines.
    /// ```
    pub fn parse_srt(input: &str) -> Self {
        let mut entries = Vec::new();
        let mut lines = input.lines().peekable();

        while lines.peek().is_some() {
            // Skip blank lines
            while let Some(&line) = lines.peek() {
                if line.trim().is_empty() {
                    lines.next();
                } else {
                    break;
                }
            }

            // Sequence number
            let seq_line = match lines.next() {
                Some(l) => l.trim(),
                None => break,
            };
            let sequence = match parse_u32_from_str(seq_line) {
                Some(n) => n,
                None => continue,
            };

            // Timestamp line: "HH:MM:SS,mmm --> HH:MM:SS,mmm"
            let ts_line = match lines.next() {
                Some(l) => l.trim(),
                None => break,
            };
            let (start_ms, end_ms) = match parse_srt_timestamp_line(ts_line) {
                Some(t) => t,
                None => continue,
            };

            // Text lines (until blank line or EOF)
            let mut text = String::new();
            while let Some(&line) = lines.peek() {
                if line.trim().is_empty() {
                    break;
                }
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(lines.next().unwrap_or(""));
            }

            entries.push(SubtitleEntry {
                sequence,
                start_ms,
                end_ms,
                text,
            });
        }

        // Sort by start time (should already be sorted in valid SRT)
        entries.sort_by_key(|e| e.start_ms);

        Self { entries }
    }

    /// Find the active subtitle at the given time (in milliseconds).
    ///
    /// Returns the first entry where start_ms <= time_ms < end_ms.
    pub fn active_at(&self, time_ms: u64) -> Option<&SubtitleEntry> {
        self.entries
            .iter()
            .find(|e| time_ms >= e.start_ms && time_ms < e.end_ms)
    }

    /// Find all active subtitles at the given time.
    pub fn all_active_at(&self, time_ms: u64) -> Vec<&SubtitleEntry> {
        self.entries
            .iter()
            .filter(|e| time_ms >= e.start_ms && time_ms < e.end_ms)
            .collect()
    }

    /// Number of subtitle entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the track is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Render a subtitle onto a pixel buffer (XRGB8888 / BGRX8888 / ARGB8888
/// format).
///
/// `buf` is the framebuffer in 32-bit pixel format (4 bytes per pixel).
/// `stride` is the row stride in bytes.
/// `width` and `height` are the frame dimensions.
/// `text` is the subtitle text (may contain newlines).
/// `config` controls positioning, colors, and opacity.
///
/// Uses a simple 8x16 bitmap font renderer. Characters outside printable ASCII
/// are skipped. Multi-line text is word-wrapped at the frame width boundary.
#[cfg(feature = "alloc")]
pub fn render_subtitle_overlay(
    buf: &mut [u8],
    stride: u32,
    width: u32,
    height: u32,
    text: &str,
    config: &SubtitleConfig,
) {
    if text.is_empty() || width == 0 || height == 0 {
        return;
    }

    let fw = config.font_width;
    let fh = config.font_height;
    let pad = config.padding;

    // Compute available text area width
    let text_area_width = width
        .saturating_sub(config.horizontal_margin * 2)
        .saturating_sub(pad * 2);
    if text_area_width < fw {
        return;
    }
    let max_chars_per_line = text_area_width / fw;
    if max_chars_per_line == 0 {
        return;
    }

    // Word-wrap lines
    let wrapped_lines = wrap_text(text, max_chars_per_line as usize);
    let num_lines = wrapped_lines.len() as u32;
    if num_lines == 0 {
        return;
    }

    // Compute box dimensions
    let box_text_height = num_lines * fh;
    let box_height = box_text_height + pad * 2;
    let max_line_len = wrapped_lines
        .iter()
        .map(|l| l.len() as u32)
        .max()
        .unwrap_or(0);
    let box_text_width = max_line_len * fw;
    let box_width = box_text_width + pad * 2;

    // Position: bottom-center
    let box_x = if width > box_width {
        (width - box_width) / 2
    } else {
        0
    };
    let box_y = if height > box_height + config.bottom_margin {
        height - box_height - config.bottom_margin
    } else {
        0
    };

    // Draw semi-transparent background box
    draw_bg_box(
        buf, stride, width, height, box_x, box_y, box_width, box_height, config,
    );

    // Draw text
    let text_x = box_x + pad;
    let mut text_y = box_y + pad;

    for line in &wrapped_lines {
        // Center each line horizontally within the box
        let line_width = line.len() as u32 * fw;
        let line_x = if box_text_width > line_width {
            text_x + (box_text_width - line_width) / 2
        } else {
            text_x
        };

        draw_text_line(buf, stride, width, height, line_x, text_y, line, config);
        text_y += fh;
    }
}

/// Draw a semi-transparent background rectangle.
#[allow(clippy::too_many_arguments)]
fn draw_bg_box(
    buf: &mut [u8],
    stride: u32,
    _width: u32,
    height: u32,
    bx: u32,
    by: u32,
    bw: u32,
    bh: u32,
    config: &SubtitleConfig,
) {
    let alpha = config.bg_opacity as u32;
    let inv_alpha = 255u32.saturating_sub(alpha);
    let (br, bg, bb) = config.bg_color;

    for dy in 0..bh {
        let py = by + dy;
        if py >= height {
            break;
        }
        let row_offset = (py * stride) as usize;

        for dx in 0..bw {
            let px = bx + dx;
            let pixel_offset = row_offset + (px as usize) * 4;
            if pixel_offset + 3 >= buf.len() {
                continue;
            }

            // Alpha blend: out = bg * alpha + existing * (255 - alpha), all / 255
            let existing_b = buf[pixel_offset] as u32;
            let existing_g = buf[pixel_offset + 1] as u32;
            let existing_r = buf[pixel_offset + 2] as u32;

            buf[pixel_offset] = ((bb as u32 * alpha + existing_b * inv_alpha) / 255) as u8;
            buf[pixel_offset + 1] = ((bg as u32 * alpha + existing_g * inv_alpha) / 255) as u8;
            buf[pixel_offset + 2] = ((br as u32 * alpha + existing_r * inv_alpha) / 255) as u8;
            buf[pixel_offset + 3] = 0xFF;
        }
    }
}

/// Draw a single line of text using an 8x16 bitmap font.
fn draw_text_line(
    buf: &mut [u8],
    stride: u32,
    _width: u32,
    height: u32,
    start_x: u32,
    start_y: u32,
    text: &str,
    config: &SubtitleConfig,
) {
    let (tr, tg, tb) = config.text_color;

    for (i, ch) in text.chars().enumerate() {
        let gx = start_x + (i as u32) * config.font_width;
        let glyph = get_glyph(ch);

        for row in 0..config.font_height.min(16) {
            let py = start_y + row;
            if py >= height {
                break;
            }
            let bits = glyph[row as usize];

            for col in 0..config.font_width.min(8) {
                if bits & (0x80 >> col) != 0 {
                    let px = gx + col;
                    let pixel_offset = (py * stride) as usize + (px as usize) * 4;
                    if pixel_offset + 3 < buf.len() {
                        buf[pixel_offset] = tb;
                        buf[pixel_offset + 1] = tg;
                        buf[pixel_offset + 2] = tr;
                        buf[pixel_offset + 3] = 0xFF;
                    }
                }
            }
        }
    }
}

/// Minimal 8x16 glyph lookup.
///
/// Returns a 16-byte bitmap where each byte represents one row of 8 pixels.
/// Bit 7 is the leftmost pixel. Only printable ASCII (0x20..0x7E) is supported;
/// all other characters return a blank glyph.
fn get_glyph(ch: char) -> [u8; 16] {
    let code = ch as u32;
    if !(0x20..=0x7E).contains(&code) {
        return [0u8; 16];
    }

    // We use a tiny built-in font for a few essential characters.
    // In production, this would reference the kernel's full font8x16 table.
    // Here we provide minimal glyphs for common characters used in subtitles.
    match ch {
        ' ' => [0; 16],
        'A' => [
            0x00, 0x00, 0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'H' => [
            0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x66, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'e' => [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'l' => [
            0x00, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x0E, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        'o' => [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        '!' => [
            0x00, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ],
        _ => {
            // Generic block glyph for characters without specific definitions
            [
                0x00, 0x00, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ]
        }
    }
}

/// Word-wrap text to fit within `max_chars` characters per line.
/// Splits on existing newlines first, then wraps long lines at word boundaries.
#[cfg(feature = "alloc")]
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    if max_chars == 0 {
        return result;
    }

    for raw_line in text.split('\n') {
        if raw_line.len() <= max_chars {
            result.push(String::from(raw_line));
            continue;
        }

        // Word-wrap this line
        let mut current_line = String::new();
        for word in raw_line.split(' ') {
            if current_line.is_empty() {
                if word.len() > max_chars {
                    // Word longer than line -- force break
                    let mut start = 0;
                    while start < word.len() {
                        let end = (start + max_chars).min(word.len());
                        result.push(String::from(&word[start..end]));
                        start = end;
                    }
                } else {
                    current_line.push_str(word);
                }
            } else if current_line.len() + 1 + word.len() <= max_chars {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                result.push(current_line);
                current_line = String::from(word);
            }
        }
        if !current_line.is_empty() {
            result.push(current_line);
        }
    }

    result
}

// ============================================================================
// Real-Time Audio Scheduling
// ============================================================================

/// Audio thread priority class, mapped to the kernel deadline scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioPriorityClass {
    /// Critical audio path (lowest latency, highest priority).
    /// Period: 1ms, runtime budget: 500us.
    Critical,
    /// Normal audio processing (standard latency).
    /// Period: 5ms, runtime budget: 2ms.
    Normal,
    /// Background audio tasks (bulk processing, not latency-sensitive).
    /// Period: 20ms, runtime budget: 10ms.
    Background,
}

impl AudioPriorityClass {
    /// Get the period in nanoseconds for this priority class.
    pub fn period_ns(&self) -> u64 {
        match self {
            Self::Critical => 1_000_000,    // 1ms
            Self::Normal => 5_000_000,      // 5ms
            Self::Background => 20_000_000, // 20ms
        }
    }

    /// Get the runtime budget in nanoseconds for this priority class.
    pub fn runtime_ns(&self) -> u64 {
        match self {
            Self::Critical => 500_000,      // 500us
            Self::Normal => 2_000_000,      // 2ms
            Self::Background => 10_000_000, // 10ms
        }
    }

    /// Get the deadline in nanoseconds (same as period for audio).
    pub fn deadline_ns(&self) -> u64 {
        self.period_ns()
    }

    /// Compute CPU utilization in permille (parts per 1000).
    pub fn utilization_permille(&self) -> u64 {
        let period = self.period_ns();
        if period == 0 {
            return 1000;
        }
        self.runtime_ns()
            .checked_mul(1000)
            .map(|v| v / period)
            .unwrap_or(1000)
    }
}

/// Audio scheduling parameters for a single audio thread.
#[derive(Debug, Clone, Copy)]
pub struct AudioSchedParams {
    /// Process/thread ID.
    pub pid: u64,
    /// Priority class.
    pub priority: AudioPriorityClass,
    /// Period in nanoseconds (wake interval for buffer fill).
    pub period_ns: u64,
    /// Runtime budget in nanoseconds per period.
    pub runtime_ns: u64,
    /// CPU reservation in permille (0..1000).
    pub cpu_reservation_permille: u32,
}

impl AudioSchedParams {
    /// Create scheduling parameters for a given priority class.
    pub fn from_priority(pid: u64, priority: AudioPriorityClass) -> Self {
        Self {
            pid,
            priority,
            period_ns: priority.period_ns(),
            runtime_ns: priority.runtime_ns(),
            cpu_reservation_permille: priority.utilization_permille() as u32,
        }
    }

    /// Create custom scheduling parameters.
    pub fn custom(pid: u64, period_ns: u64, runtime_ns: u64) -> Self {
        let cpu_reservation_permille = if period_ns > 0 {
            (runtime_ns.saturating_mul(1000) / period_ns) as u32
        } else {
            1000
        };
        Self {
            pid,
            priority: AudioPriorityClass::Normal,
            period_ns,
            runtime_ns,
            cpu_reservation_permille,
        }
    }
}

/// Statistics for a single audio thread's scheduling behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct AudioSchedStats {
    /// Total number of scheduling periods completed.
    pub periods_completed: u64,
    /// Number of times the thread was woken on time.
    pub on_time_wakes: u64,
    /// Number of times the thread was woken late (missed deadline).
    pub late_wakes: u64,
    /// Number of buffer underruns (thread did not fill buffer in time).
    pub underruns: u64,
    /// Number of buffer overruns (buffer full, data lost).
    pub overruns: u64,
    /// Maximum observed scheduling jitter in nanoseconds.
    pub max_jitter_ns: u64,
    /// Minimum observed scheduling jitter in nanoseconds.
    pub min_jitter_ns: u64,
    /// Cumulative jitter (for computing average).
    pub total_jitter_ns: u64,
    /// Last wake timestamp (nanoseconds since boot).
    pub last_wake_ns: u64,
    /// Expected next wake timestamp.
    pub next_expected_wake_ns: u64,
}

impl AudioSchedStats {
    /// Compute average jitter in nanoseconds.
    pub fn avg_jitter_ns(&self) -> u64 {
        if self.periods_completed == 0 {
            return 0;
        }
        self.total_jitter_ns / self.periods_completed
    }

    /// Record a wake event.
    ///
    /// `actual_wake_ns` is the actual wake time. `expected_wake_ns` is when
    /// the wake was scheduled. The difference is the jitter.
    pub fn record_wake(&mut self, actual_wake_ns: u64, expected_wake_ns: u64) {
        self.periods_completed += 1;

        let jitter = actual_wake_ns.abs_diff(expected_wake_ns);

        self.total_jitter_ns = self.total_jitter_ns.saturating_add(jitter);

        if jitter > self.max_jitter_ns {
            self.max_jitter_ns = jitter;
        }
        if self.min_jitter_ns == 0 || jitter < self.min_jitter_ns {
            self.min_jitter_ns = jitter;
        }

        if actual_wake_ns <= expected_wake_ns {
            self.on_time_wakes += 1;
        } else {
            self.late_wakes += 1;
        }

        self.last_wake_ns = actual_wake_ns;
        self.next_expected_wake_ns =
            actual_wake_ns.saturating_add(expected_wake_ns.saturating_sub(self.last_wake_ns));
    }

    /// Record a buffer underrun event.
    pub fn record_underrun(&mut self) {
        self.underruns += 1;
    }

    /// Record a buffer overrun event.
    pub fn record_overrun(&mut self) {
        self.overruns += 1;
    }
}

/// Real-time audio scheduler manager.
///
/// Tracks all registered audio threads and their scheduling statistics.
/// Integrates with the kernel's EDF deadline scheduler for actual scheduling.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct AudioScheduler {
    /// Registered audio threads and their parameters.
    threads: Vec<AudioSchedParams>,
    /// Per-thread scheduling statistics (parallel to `threads`).
    stats: Vec<AudioSchedStats>,
    /// Total CPU reservation in permille across all audio threads.
    total_reservation_permille: u32,
    /// Maximum total CPU reservation allowed (default: 800 = 80%).
    max_reservation_permille: u32,
}

#[cfg(feature = "alloc")]
impl Default for AudioScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl AudioScheduler {
    /// Maximum number of concurrent audio threads.
    const MAX_AUDIO_THREADS: usize = 32;

    /// Create a new audio scheduler with default settings.
    pub fn new() -> Self {
        Self {
            threads: Vec::new(),
            stats: Vec::new(),
            total_reservation_permille: 0,
            max_reservation_permille: 800, // 80% max for audio
        }
    }

    /// Create with a custom maximum CPU reservation.
    pub fn with_max_reservation(max_permille: u32) -> Self {
        Self {
            threads: Vec::new(),
            stats: Vec::new(),
            total_reservation_permille: 0,
            max_reservation_permille: max_permille.min(1000),
        }
    }

    /// Register an audio thread for real-time scheduling.
    ///
    /// Returns `Err` if:
    /// - Maximum thread count exceeded
    /// - CPU reservation would exceed the maximum
    /// - Thread already registered
    pub fn register_thread(&mut self, params: AudioSchedParams) -> Result<(), AudioSchedError> {
        // Check duplicates
        if self.threads.iter().any(|t| t.pid == params.pid) {
            return Err(AudioSchedError::AlreadyRegistered);
        }

        // Check capacity
        if self.threads.len() >= Self::MAX_AUDIO_THREADS {
            return Err(AudioSchedError::TooManyThreads);
        }

        // Check CPU reservation
        let new_total = self
            .total_reservation_permille
            .saturating_add(params.cpu_reservation_permille);
        if new_total > self.max_reservation_permille {
            return Err(AudioSchedError::InsufficientCpuBudget);
        }

        self.total_reservation_permille = new_total;
        self.threads.push(params);
        self.stats.push(AudioSchedStats::default());
        Ok(())
    }

    /// Unregister an audio thread.
    pub fn unregister_thread(&mut self, pid: u64) -> Result<(), AudioSchedError> {
        let idx = self
            .threads
            .iter()
            .position(|t| t.pid == pid)
            .ok_or(AudioSchedError::NotFound)?;

        let params = self.threads.remove(idx);
        self.stats.remove(idx);
        self.total_reservation_permille = self
            .total_reservation_permille
            .saturating_sub(params.cpu_reservation_permille);
        Ok(())
    }

    /// Get the scheduling parameters for a thread.
    pub fn get_params(&self, pid: u64) -> Option<&AudioSchedParams> {
        self.threads.iter().find(|t| t.pid == pid)
    }

    /// Get mutable scheduling statistics for a thread.
    pub fn get_stats_mut(&mut self, pid: u64) -> Option<&mut AudioSchedStats> {
        let idx = self.threads.iter().position(|t| t.pid == pid)?;
        self.stats.get_mut(idx)
    }

    /// Get scheduling statistics for a thread.
    pub fn get_stats(&self, pid: u64) -> Option<&AudioSchedStats> {
        let idx = self.threads.iter().position(|t| t.pid == pid)?;
        self.stats.get(idx)
    }

    /// Record a wake event for a thread.
    pub fn record_wake(
        &mut self,
        pid: u64,
        actual_ns: u64,
        expected_ns: u64,
    ) -> Result<(), AudioSchedError> {
        let stats = self.get_stats_mut(pid).ok_or(AudioSchedError::NotFound)?;
        stats.record_wake(actual_ns, expected_ns);
        Ok(())
    }

    /// Total CPU reservation in permille.
    pub fn total_reservation(&self) -> u32 {
        self.total_reservation_permille
    }

    /// Number of registered audio threads.
    pub fn thread_count(&self) -> usize {
        self.threads.len()
    }

    /// Available CPU budget in permille.
    pub fn available_budget(&self) -> u32 {
        self.max_reservation_permille
            .saturating_sub(self.total_reservation_permille)
    }

    /// Compute the next wake time for a thread based on its period.
    pub fn next_wake_time(&self, pid: u64, current_ns: u64) -> Option<u64> {
        let params = self.get_params(pid)?;
        Some(current_ns.saturating_add(params.period_ns))
    }

    /// Get aggregate statistics across all audio threads.
    pub fn aggregate_stats(&self) -> AudioSchedStats {
        let mut agg = AudioSchedStats::default();
        for stats in &self.stats {
            agg.periods_completed = agg
                .periods_completed
                .saturating_add(stats.periods_completed);
            agg.on_time_wakes = agg.on_time_wakes.saturating_add(stats.on_time_wakes);
            agg.late_wakes = agg.late_wakes.saturating_add(stats.late_wakes);
            agg.underruns = agg.underruns.saturating_add(stats.underruns);
            agg.overruns = agg.overruns.saturating_add(stats.overruns);
            if stats.max_jitter_ns > agg.max_jitter_ns {
                agg.max_jitter_ns = stats.max_jitter_ns;
            }
            agg.total_jitter_ns = agg.total_jitter_ns.saturating_add(stats.total_jitter_ns);
        }
        agg
    }
}

/// Errors from the audio real-time scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSchedError {
    /// Thread already registered.
    AlreadyRegistered,
    /// Maximum audio thread count exceeded.
    TooManyThreads,
    /// Not enough CPU budget for the requested reservation.
    InsufficientCpuBudget,
    /// Thread not found.
    NotFound,
    /// Invalid scheduling parameters.
    InvalidParams,
}

/// Global audio scheduling statistics counters (lock-free).
pub(crate) static AUDIO_TOTAL_UNDERRUNS: AtomicU64 = AtomicU64::new(0);
pub(crate) static AUDIO_TOTAL_OVERRUNS: AtomicU64 = AtomicU64::new(0);
pub(crate) static AUDIO_TOTAL_LATE_WAKES: AtomicU64 = AtomicU64::new(0);

/// Increment global underrun counter.
pub fn count_audio_underrun() {
    AUDIO_TOTAL_UNDERRUNS.fetch_add(1, Ordering::Relaxed);
}

/// Increment global overrun counter.
pub fn count_audio_overrun() {
    AUDIO_TOTAL_OVERRUNS.fetch_add(1, Ordering::Relaxed);
}

/// Increment global late wake counter.
pub fn count_audio_late_wake() {
    AUDIO_TOTAL_LATE_WAKES.fetch_add(1, Ordering::Relaxed);
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Read a little-endian u32 from a byte slice at the given offset.
#[inline]
fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    if offset + 4 > data.len() {
        return 0;
    }
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read a little-endian i32 from a byte slice at the given offset.
#[inline]
fn read_i32_le(data: &[u8], offset: usize) -> i32 {
    read_u32_le(data, offset) as i32
}

/// Read a little-endian u16 from a byte slice at the given offset.
#[inline]
fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    if offset + 2 > data.len() {
        return 0;
    }
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

/// Advance to the next 2-byte aligned position.
fn aligned_next(pos: usize) -> usize {
    (pos + 1) & !1
}

/// Advance to the next 2-byte aligned position (relative offset).
fn aligned_next_rel(pos: usize) -> usize {
    (pos + 1) & !1
}

/// Parse a u32 from a decimal string.
fn parse_u32_from_str(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}

/// Parse an SRT timestamp "HH:MM:SS,mmm" to milliseconds.
fn parse_srt_timestamp(s: &str) -> Option<u64> {
    // Expected: "HH:MM:SS,mmm" or "HH:MM:SS.mmm"
    let s = s.trim();
    if s.len() < 12 {
        return None;
    }
    let bytes = s.as_bytes();

    let hours = parse_two_digits(bytes, 0)? as u64;
    // bytes[2] should be ':'
    if bytes[2] != b':' {
        return None;
    }
    let minutes = parse_two_digits(bytes, 3)? as u64;
    if bytes[5] != b':' {
        return None;
    }
    let seconds = parse_two_digits(bytes, 6)? as u64;
    // bytes[8] should be ',' or '.'
    if bytes[8] != b',' && bytes[8] != b'.' {
        return None;
    }
    let millis = parse_three_digits(bytes, 9)? as u64;

    hours
        .checked_mul(3_600_000)?
        .checked_add(minutes.checked_mul(60_000)?)?
        .checked_add(seconds.checked_mul(1_000)?)?
        .checked_add(millis)
}

/// Parse a timestamp line "start --> end".
fn parse_srt_timestamp_line(line: &str) -> Option<(u64, u64)> {
    let parts: Vec<&str> = line.split("-->").collect();
    if parts.len() != 2 {
        return None;
    }
    let start = parse_srt_timestamp(parts[0])?;
    let end = parse_srt_timestamp(parts[1])?;
    Some((start, end))
}

/// Parse two ASCII decimal digits at `offset`.
fn parse_two_digits(bytes: &[u8], offset: usize) -> Option<u32> {
    if offset + 2 > bytes.len() {
        return None;
    }
    let d0 = bytes[offset].wrapping_sub(b'0');
    let d1 = bytes[offset + 1].wrapping_sub(b'0');
    if d0 > 9 || d1 > 9 {
        return None;
    }
    Some(d0 as u32 * 10 + d1 as u32)
}

/// Parse three ASCII decimal digits at `offset`.
fn parse_three_digits(bytes: &[u8], offset: usize) -> Option<u32> {
    if offset + 3 > bytes.len() {
        return None;
    }
    let d0 = bytes[offset].wrapping_sub(b'0');
    let d1 = bytes[offset + 1].wrapping_sub(b'0');
    let d2 = bytes[offset + 2].wrapping_sub(b'0');
    if d0 > 9 || d1 > 9 || d2 > 9 {
        return None;
    }
    let hundreds = d0 as u32;
    let tens = d1 as u32;
    hundreds
        .checked_mul(100)?
        .checked_add(tens.checked_mul(10)?)?
        .checked_add(d2 as u32)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // ---- AVI Parser Tests ----

    #[test]
    fn test_fourcc_from_bytes() {
        let data = b"RIFF";
        let fcc = FourCC::from_bytes(data).unwrap();
        assert_eq!(fcc, FourCC::RIFF);
    }

    #[test]
    fn test_fourcc_from_bytes_too_short() {
        let data = b"RI";
        assert!(FourCC::from_bytes(data).is_none());
    }

    #[test]
    fn test_avi_flags() {
        let flags = AviFlags(AviFlags::AVIF_HASINDEX | AviFlags::AVIF_ISINTERLEAVED);
        assert!(flags.has_flag(AviFlags::AVIF_HASINDEX));
        assert!(flags.has_flag(AviFlags::AVIF_ISINTERLEAVED));
        assert!(!flags.has_flag(AviFlags::AVIF_COPYRIGHTED));
    }

    #[test]
    fn test_avi_main_header_parse() {
        let mut data = [0u8; 40];
        // microseconds_per_frame = 33333 (~30fps)
        data[0..4].copy_from_slice(&33333u32.to_le_bytes());
        // total_frames = 100
        data[16..20].copy_from_slice(&100u32.to_le_bytes());
        // streams = 2
        data[24..28].copy_from_slice(&2u32.to_le_bytes());
        // width = 640
        data[32..36].copy_from_slice(&640u32.to_le_bytes());
        // height = 480
        data[36..40].copy_from_slice(&480u32.to_le_bytes());

        let hdr = AviMainHeader::parse(&data).unwrap();
        assert_eq!(hdr.microseconds_per_frame, 33333);
        assert_eq!(hdr.total_frames, 100);
        assert_eq!(hdr.streams, 2);
        assert_eq!(hdr.width, 640);
        assert_eq!(hdr.height, 480);
    }

    #[test]
    fn test_avi_main_header_frame_rate() {
        let hdr = AviMainHeader {
            microseconds_per_frame: 33333,
            ..Default::default()
        };
        let (num, den) = hdr.frame_rate();
        // 1_000_000 / 33333 ~= 30.0003 fps
        assert_eq!(num, 1_000_000);
        assert_eq!(den, 33333);
    }

    #[test]
    fn test_avi_main_header_parse_too_short() {
        let data = [0u8; 20];
        assert!(AviMainHeader::parse(&data).is_none());
    }

    #[test]
    fn test_avi_stream_header_parse() {
        let mut data = [0u8; 56];
        data[0..4].copy_from_slice(b"vids");
        data[4..8].copy_from_slice(b"DIB ");
        // scale = 1
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        // rate = 30
        data[24..28].copy_from_slice(&30u32.to_le_bytes());

        let hdr = AviStreamHeader::parse(&data).unwrap();
        assert_eq!(hdr.get_stream_type(), StreamType::Video);
        assert_eq!(hdr.sample_rate(), (30, 1));
    }

    #[test]
    fn test_avi_stream_header_audio() {
        let mut data = [0u8; 56];
        data[0..4].copy_from_slice(b"auds");

        let hdr = AviStreamHeader::parse(&data).unwrap();
        assert_eq!(hdr.get_stream_type(), StreamType::Audio);
    }

    #[test]
    fn test_bitmap_info_header_parse() {
        let mut data = [0u8; 40];
        data[0..4].copy_from_slice(&40u32.to_le_bytes()); // size
        data[4..8].copy_from_slice(&320i32.to_le_bytes()); // width
        data[8..12].copy_from_slice(&240i32.to_le_bytes()); // height (positive = bottom-up)
        data[12..14].copy_from_slice(&1u16.to_le_bytes()); // planes
        data[14..16].copy_from_slice(&24u16.to_le_bytes()); // bit_count

        let bih = BitmapInfoHeader::parse(&data).unwrap();
        assert_eq!(bih.width, 320);
        assert_eq!(bih.height, 240);
        assert_eq!(bih.bit_count, 24);
        assert!(bih.is_bottom_up());
        assert_eq!(bih.abs_height(), 240);
    }

    #[test]
    fn test_bitmap_info_header_top_down() {
        let mut data = [0u8; 40];
        data[0..4].copy_from_slice(&40u32.to_le_bytes());
        data[4..8].copy_from_slice(&320i32.to_le_bytes());
        data[8..12].copy_from_slice(&(-240i32).to_le_bytes()); // negative = top-down

        let bih = BitmapInfoHeader::parse(&data).unwrap();
        assert!(!bih.is_bottom_up());
        assert_eq!(bih.abs_height(), 240);
    }

    #[test]
    fn test_wave_format_ex_parse() {
        let mut data = [0u8; 18];
        data[0..2].copy_from_slice(&1u16.to_le_bytes()); // PCM
        data[2..4].copy_from_slice(&2u16.to_le_bytes()); // stereo
        data[4..8].copy_from_slice(&44100u32.to_le_bytes()); // 44.1kHz
        data[8..12].copy_from_slice(&176400u32.to_le_bytes()); // byte rate
        data[12..14].copy_from_slice(&4u16.to_le_bytes()); // block align
        data[14..16].copy_from_slice(&16u16.to_le_bytes()); // 16-bit

        let wfx = WaveFormatEx::parse(&data).unwrap();
        assert!(wfx.is_pcm());
        assert_eq!(wfx.channels, 2);
        assert_eq!(wfx.samples_per_sec, 44100);
        assert_eq!(wfx.bits_per_sample, 16);
    }

    #[test]
    fn test_avi_index_entry() {
        let mut data = [0u8; 16];
        data[0..4].copy_from_slice(b"00dc");
        data[4..8].copy_from_slice(&0x10u32.to_le_bytes()); // keyframe
        data[8..12].copy_from_slice(&1024u32.to_le_bytes()); // offset
        data[12..16].copy_from_slice(&4096u32.to_le_bytes()); // size

        let entry = AviIndexEntry::parse(&data).unwrap();
        assert!(entry.is_keyframe());
        assert!(entry.is_video());
        assert!(!entry.is_audio());
        assert_eq!(entry.stream_number(), 0);
        assert_eq!(entry.offset, 1024);
        assert_eq!(entry.size, 4096);
    }

    #[test]
    fn test_avi_index_entry_audio() {
        let mut data = [0u8; 16];
        data[0..4].copy_from_slice(b"01wb");

        let entry = AviIndexEntry::parse(&data).unwrap();
        assert!(!entry.is_video());
        assert!(entry.is_audio());
        assert_eq!(entry.stream_number(), 1);
    }

    // ---- Frame Rate Conversion Tests ----

    #[test]
    fn test_source_frame_for_output_identity() {
        // Same rate: 30fps -> 30fps, should map 1:1
        let conv = FrameRateConverter::new(30, 1, 30, 1, FrameRateMode::Duplicate);
        assert_eq!(conv.source_frame_for_output(0), 0);
        assert_eq!(conv.source_frame_for_output(1), 1);
        assert_eq!(conv.source_frame_for_output(10), 10);
    }

    #[test]
    fn test_source_frame_for_output_downsample() {
        // 60fps -> 30fps: every other source frame
        let conv = FrameRateConverter::new(60, 1, 30, 1, FrameRateMode::Drop);
        assert_eq!(conv.source_frame_for_output(0), 0);
        assert_eq!(conv.source_frame_for_output(1), 2);
        assert_eq!(conv.source_frame_for_output(2), 4);
    }

    #[test]
    fn test_source_frame_for_output_upsample() {
        // 24fps -> 48fps: each source frame used twice
        let conv = FrameRateConverter::new(24, 1, 48, 1, FrameRateMode::Duplicate);
        assert_eq!(conv.source_frame_for_output(0), 0);
        assert_eq!(conv.source_frame_for_output(1), 0);
        assert_eq!(conv.source_frame_for_output(2), 1);
        assert_eq!(conv.source_frame_for_output(3), 1);
    }

    #[test]
    fn test_pulldown_32_pattern() {
        let conv = FrameRateConverter::new(24, 1, 30, 1, FrameRateMode::Pulldown32);
        // First cycle of 5 output frames from 4 source frames
        assert_eq!(conv.pulldown_32_source(0), (0, false)); // A
        assert_eq!(conv.pulldown_32_source(1), (0, true)); // A repeated
        assert_eq!(conv.pulldown_32_source(2), (1, false)); // B
        assert_eq!(conv.pulldown_32_source(3), (2, false)); // C
        assert_eq!(conv.pulldown_32_source(4), (2, true)); // C repeated

        // Second cycle
        assert_eq!(conv.pulldown_32_source(5), (4, false)); // D
        assert_eq!(conv.pulldown_32_source(6), (4, true)); // D repeated
    }

    #[test]
    fn test_blend_frames_50_50() {
        let frame_a = vec![0u8, 100, 200, 50];
        let frame_b = vec![100u8, 200, 0, 150];
        let mut out = vec![0u8; 4];

        blend_frames(&frame_a, &frame_b, &mut out, 128);
        // 50/50 blend: (0*128 + 100*128)/256 = 50
        assert_eq!(out[0], 50);
        // (100*128 + 200*128)/256 = 150
        assert_eq!(out[1], 150);
        // (200*128 + 0*128)/256 = 100
        assert_eq!(out[2], 100);
        // (50*128 + 150*128)/256 = 100
        assert_eq!(out[3], 100);
    }

    #[test]
    fn test_blend_frames_all_a() {
        let frame_a = vec![100u8; 4];
        let frame_b = vec![200u8; 4];
        let mut out = vec![0u8; 4];

        blend_frames(&frame_a, &frame_b, &mut out, 0);
        // weight=0: 100% frame_a
        assert_eq!(out, vec![100u8; 4]);
    }

    #[test]
    fn test_blend_frames_all_b() {
        let frame_a = vec![100u8; 4];
        let frame_b = vec![200u8; 4];
        let mut out = vec![0u8; 4];

        blend_frames(&frame_a, &frame_b, &mut out, 256);
        // weight=256: 100% frame_b
        assert_eq!(out, vec![200u8; 4]);
    }

    #[test]
    fn test_frame_map_duplicate() {
        let conv = FrameRateConverter::new(24, 1, 48, 1, FrameRateMode::Duplicate);
        let map = conv.build_frame_map(4, 8);
        assert_eq!(map.len(), 8);
        // Each source frame should appear twice
        assert_eq!(map[0].source_index, 0);
        assert_eq!(map[1].source_index, 0);
        assert_eq!(map[2].source_index, 1);
        assert_eq!(map[3].source_index, 1);
    }

    // ---- SRT Parser Tests ----

    #[test]
    fn test_parse_srt_timestamp() {
        let ts = parse_srt_timestamp("01:23:45,678").unwrap();
        // 1*3600000 + 23*60000 + 45*1000 + 678 = 3600000 + 1380000 + 45000 + 678 =
        // 5025678
        assert_eq!(ts, 5_025_678);
    }

    #[test]
    fn test_parse_srt_timestamp_zero() {
        let ts = parse_srt_timestamp("00:00:00,000").unwrap();
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_parse_srt_timestamp_dot_separator() {
        let ts = parse_srt_timestamp("00:00:01.500").unwrap();
        assert_eq!(ts, 1500);
    }

    #[test]
    fn test_parse_srt_basic() {
        let srt_text = "1\n00:00:01,000 --> 00:00:04,000\nHello, world!\n\n2\n00:00:05,500 --> \
                        00:00:08,000\nSecond subtitle\nwith two lines.\n";
        let track = SubtitleTrack::parse_srt(srt_text);
        assert_eq!(track.len(), 2);

        assert_eq!(track.entries[0].sequence, 1);
        assert_eq!(track.entries[0].start_ms, 1000);
        assert_eq!(track.entries[0].end_ms, 4000);
        assert_eq!(track.entries[0].text, "Hello, world!");

        assert_eq!(track.entries[1].sequence, 2);
        assert_eq!(track.entries[1].start_ms, 5500);
        assert_eq!(track.entries[1].end_ms, 8000);
        assert_eq!(track.entries[1].text, "Second subtitle\nwith two lines.");
    }

    #[test]
    fn test_subtitle_active_at() {
        let srt_text =
            "1\n00:00:01,000 --> 00:00:04,000\nFirst\n\n2\n00:00:05,000 --> 00:00:08,000\nSecond\n";
        let track = SubtitleTrack::parse_srt(srt_text);

        assert!(track.active_at(0).is_none());
        assert_eq!(track.active_at(1000).unwrap().text, "First");
        assert_eq!(track.active_at(3999).unwrap().text, "First");
        assert!(track.active_at(4000).is_none());
        assert_eq!(track.active_at(5000).unwrap().text, "Second");
        assert!(track.active_at(8000).is_none());
    }

    #[test]
    fn test_wrap_text_short() {
        let lines = wrap_text("Hello world", 20);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "Hello world");
    }

    #[test]
    fn test_wrap_text_multiline() {
        let lines = wrap_text("This is a longer line of text", 15);
        assert!(lines.len() >= 2);
        for line in &lines {
            assert!(line.len() <= 15);
        }
    }

    #[test]
    fn test_wrap_text_existing_newlines() {
        let lines = wrap_text("Line one\nLine two", 50);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Line one");
        assert_eq!(lines[1], "Line two");
    }

    // ---- Audio Scheduling Tests ----

    #[test]
    fn test_audio_priority_class_params() {
        assert_eq!(AudioPriorityClass::Critical.period_ns(), 1_000_000);
        assert_eq!(AudioPriorityClass::Critical.runtime_ns(), 500_000);
        assert_eq!(AudioPriorityClass::Critical.utilization_permille(), 500);

        assert_eq!(AudioPriorityClass::Normal.period_ns(), 5_000_000);
        assert_eq!(AudioPriorityClass::Normal.runtime_ns(), 2_000_000);
        assert_eq!(AudioPriorityClass::Normal.utilization_permille(), 400);

        assert_eq!(AudioPriorityClass::Background.period_ns(), 20_000_000);
        assert_eq!(AudioPriorityClass::Background.runtime_ns(), 10_000_000);
        assert_eq!(AudioPriorityClass::Background.utilization_permille(), 500);
    }

    #[test]
    fn test_audio_scheduler_register() {
        let mut sched = AudioScheduler::new();
        let params = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        assert!(sched.register_thread(params).is_ok());
        assert_eq!(sched.thread_count(), 1);
        assert_eq!(sched.total_reservation(), 400);
    }

    #[test]
    fn test_audio_scheduler_register_duplicate() {
        let mut sched = AudioScheduler::new();
        let params = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        assert!(sched.register_thread(params).is_ok());
        assert_eq!(
            sched.register_thread(params),
            Err(AudioSchedError::AlreadyRegistered)
        );
    }

    #[test]
    fn test_audio_scheduler_cpu_budget() {
        let mut sched = AudioScheduler::with_max_reservation(500);
        // Normal = 400 permille, should fit
        let p1 = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        assert!(sched.register_thread(p1).is_ok());
        assert_eq!(sched.available_budget(), 100);

        // Another Normal = 400, total would be 800 > 500
        let p2 = AudioSchedParams::from_priority(2, AudioPriorityClass::Normal);
        assert_eq!(
            sched.register_thread(p2),
            Err(AudioSchedError::InsufficientCpuBudget)
        );
    }

    #[test]
    fn test_audio_scheduler_unregister() {
        let mut sched = AudioScheduler::new();
        let params = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        assert!(sched.register_thread(params).is_ok());
        assert!(sched.unregister_thread(1).is_ok());
        assert_eq!(sched.thread_count(), 0);
        assert_eq!(sched.total_reservation(), 0);
    }

    #[test]
    fn test_audio_scheduler_unregister_not_found() {
        let mut sched = AudioScheduler::new();
        assert_eq!(sched.unregister_thread(42), Err(AudioSchedError::NotFound));
    }

    #[test]
    fn test_audio_sched_stats_record_wake() {
        let mut stats = AudioSchedStats::default();

        // On-time wake (actual <= expected)
        stats.record_wake(1_000_000, 1_000_000);
        assert_eq!(stats.periods_completed, 1);
        assert_eq!(stats.on_time_wakes, 1);
        assert_eq!(stats.late_wakes, 0);
        assert_eq!(stats.max_jitter_ns, 0);

        // Late wake (actual > expected)
        stats.record_wake(2_100_000, 2_000_000);
        assert_eq!(stats.periods_completed, 2);
        assert_eq!(stats.on_time_wakes, 1);
        assert_eq!(stats.late_wakes, 1);
        assert_eq!(stats.max_jitter_ns, 100_000);
    }

    #[test]
    fn test_audio_sched_stats_underrun_overrun() {
        let mut stats = AudioSchedStats::default();
        stats.record_underrun();
        stats.record_underrun();
        stats.record_overrun();
        assert_eq!(stats.underruns, 2);
        assert_eq!(stats.overruns, 1);
    }

    #[test]
    fn test_audio_sched_stats_avg_jitter() {
        let mut stats = AudioSchedStats::default();
        stats.record_wake(1_000_100, 1_000_000); // 100ns jitter
        stats.record_wake(2_000_200, 2_000_000); // 200ns jitter
        assert_eq!(stats.avg_jitter_ns(), 150); // (100 + 200) / 2
    }

    #[test]
    fn test_audio_scheduler_next_wake_time() {
        let mut sched = AudioScheduler::new();
        let params = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        assert!(sched.register_thread(params).is_ok());

        let next = sched.next_wake_time(1, 10_000_000).unwrap();
        assert_eq!(next, 15_000_000); // current + 5ms period
    }

    #[test]
    fn test_audio_scheduler_aggregate_stats() {
        let mut sched = AudioScheduler::with_max_reservation(1000);
        let p1 = AudioSchedParams::from_priority(1, AudioPriorityClass::Normal);
        let p2 = AudioSchedParams::from_priority(2, AudioPriorityClass::Background);
        assert!(sched.register_thread(p1).is_ok());
        assert!(sched.register_thread(p2).is_ok());

        // Record events
        assert!(sched.record_wake(1, 1_000_000, 1_000_000).is_ok());
        assert!(sched.record_wake(2, 2_100_000, 2_000_000).is_ok());
        sched.get_stats_mut(1).unwrap().record_underrun();

        let agg = sched.aggregate_stats();
        assert_eq!(agg.periods_completed, 2);
        assert_eq!(agg.on_time_wakes, 1);
        assert_eq!(agg.late_wakes, 1);
        assert_eq!(agg.underruns, 1);
    }

    #[test]
    fn test_global_audio_counters() {
        // These are global atomics, just verify they increment
        let before = AUDIO_TOTAL_UNDERRUNS.load(Ordering::Relaxed);
        count_audio_underrun();
        let after = AUDIO_TOTAL_UNDERRUNS.load(Ordering::Relaxed);
        assert_eq!(after, before + 1);
    }

    // ---- AVI Container Integration Test ----

    #[test]
    fn test_avi_container_parse_minimal() {
        // Build a minimal valid AVI file in memory
        let mut avi = Vec::new();

        // RIFF header
        avi.extend_from_slice(b"RIFF");
        let size_pos = avi.len();
        avi.extend_from_slice(&0u32.to_le_bytes()); // placeholder
        avi.extend_from_slice(b"AVI ");

        // hdrl LIST
        avi.extend_from_slice(b"LIST");
        let hdrl_size_pos = avi.len();
        avi.extend_from_slice(&0u32.to_le_bytes()); // placeholder
        avi.extend_from_slice(b"hdrl");

        // avih chunk
        avi.extend_from_slice(b"avih");
        avi.extend_from_slice(&40u32.to_le_bytes()); // chunk size
        let mut avih_data = [0u8; 40];
        avih_data[0..4].copy_from_slice(&33333u32.to_le_bytes()); // ~30fps
        avih_data[16..20].copy_from_slice(&1u32.to_le_bytes()); // total_frames
        avih_data[24..28].copy_from_slice(&1u32.to_le_bytes()); // streams
        avih_data[32..36].copy_from_slice(&320u32.to_le_bytes()); // width
        avih_data[36..40].copy_from_slice(&240u32.to_le_bytes()); // height
        avi.extend_from_slice(&avih_data);

        // Fix hdrl LIST size
        let hdrl_size = (avi.len() - hdrl_size_pos - 4) as u32;
        avi[hdrl_size_pos..hdrl_size_pos + 4].copy_from_slice(&hdrl_size.to_le_bytes());

        // movi LIST (empty)
        avi.extend_from_slice(b"LIST");
        avi.extend_from_slice(&4u32.to_le_bytes());
        avi.extend_from_slice(b"movi");

        // Fix RIFF size
        let riff_size = (avi.len() - 8) as u32;
        avi[size_pos..size_pos + 4].copy_from_slice(&riff_size.to_le_bytes());

        let container = AviContainer::parse(&avi).unwrap();
        assert_eq!(container.main_header.width, 320);
        assert_eq!(container.main_header.height, 240);
        assert_eq!(container.main_header.microseconds_per_frame, 33333);
        assert_eq!(container.main_header.total_frames, 1);
    }

    #[test]
    fn test_avi_container_parse_invalid() {
        // Not a RIFF file
        let data = b"NOT_RIFF_DATA";
        assert!(AviContainer::parse(data).is_none());
    }

    #[test]
    fn test_avi_demux_streams() {
        let mut container = AviContainer {
            main_header: AviMainHeader::default(),
            streams: Vec::new(),
            index: vec![
                AviIndexEntry {
                    chunk_id: *b"00dc",
                    flags: 0x10,
                    offset: 0,
                    size: 100,
                },
                AviIndexEntry {
                    chunk_id: *b"01wb",
                    flags: 0,
                    offset: 108,
                    size: 50,
                },
                AviIndexEntry {
                    chunk_id: *b"00dc",
                    flags: 0,
                    offset: 166,
                    size: 100,
                },
            ],
            movi_offset: 0,
            file_size: 0,
        };

        let (video, audio) = container.demux_streams();
        assert_eq!(video.len(), 2);
        assert_eq!(audio.len(), 1);
        assert_eq!(container.video_frame_count(), 2);
        assert_eq!(container.audio_chunk_count(), 1);
    }
}
