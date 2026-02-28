//! Simple video playback (raw frame sequences)
//!
//! Provides `RawVideoStream` for sequential frame playback and
//! `MediaPlayer` for higher-level control including image loading
//! and display-rect management.

#![allow(dead_code)]

use alloc::vec::Vec;

use spin::Mutex;

use super::{decode, VideoFrame, VideoInfo};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Playback state
// ---------------------------------------------------------------------------

/// Current state of a video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}

// ---------------------------------------------------------------------------
// Raw video stream
// ---------------------------------------------------------------------------

/// A raw, in-memory video stream made of pre-decoded frames.
///
/// Frame advancement is driven by calling `next_frame()` which
/// uses a simple tick-based timing model (caller provides tick count).
pub struct RawVideoStream {
    frames: Vec<VideoFrame>,
    current_frame: usize,
    info: VideoInfo,
    state: PlaybackState,
    frames_displayed: u64,
    /// Tick at which playback started (set by `play()`).
    start_tick: u64,
}

impl RawVideoStream {
    /// Create an empty stream with the given metadata.
    pub fn new(info: VideoInfo) -> Self {
        Self {
            frames: Vec::new(),
            current_frame: 0,
            info,
            state: PlaybackState::Stopped,
            frames_displayed: 0,
            start_tick: 0,
        }
    }

    /// Append a decoded frame to the stream.
    pub fn add_frame(&mut self, frame: VideoFrame) {
        self.frames.push(frame);
    }

    /// Begin (or resume) playback.
    pub fn play(&mut self) {
        match self.state {
            PlaybackState::Stopped | PlaybackState::Finished => {
                self.current_frame = 0;
                self.frames_displayed = 0;
                self.start_tick = get_tick();
                self.state = PlaybackState::Playing;
            }
            PlaybackState::Paused => {
                // Adjust start_tick to exclude paused duration
                self.start_tick = get_tick();
                self.state = PlaybackState::Playing;
            }
            PlaybackState::Playing => {} // already playing
        }
    }

    /// Pause playback.
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    /// Stop playback and reset to the beginning.
    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.current_frame = 0;
        self.frames_displayed = 0;
        self.start_tick = 0;
    }

    /// Seek to a specific frame index.
    pub fn seek(&mut self, frame_index: usize) {
        if frame_index < self.frames.len() {
            self.current_frame = frame_index;
        }
    }

    /// Advance to the next frame based on frame-rate timing.
    ///
    /// Returns a reference to the current frame if one should be displayed,
    /// or `None` if the stream is not playing or has finished.
    pub fn next_frame(&mut self) -> Option<&VideoFrame> {
        if self.state != PlaybackState::Playing {
            // If paused or stopped, return the current frame without advancing
            return if self.state == PlaybackState::Paused {
                self.frames.get(self.current_frame)
            } else {
                None
            };
        }

        if self.frames.is_empty() {
            self.state = PlaybackState::Finished;
            return None;
        }

        // Calculate which frame we should be on based on elapsed ticks.
        // We treat each tick as 1 millisecond.
        let elapsed_ms = get_tick().saturating_sub(self.start_tick);

        // Frame duration in ms: 1000 * den / num
        let frame_dur_ms = if self.info.frame_rate_num > 0 {
            (1000u64 * self.info.frame_rate_den as u64) / self.info.frame_rate_num as u64
        } else {
            // Default to ~30 fps if unspecified
            33
        };

        let target_frame = if frame_dur_ms > 0 {
            (elapsed_ms / frame_dur_ms) as usize
        } else {
            0
        };

        if target_frame >= self.frames.len() {
            self.state = PlaybackState::Finished;
            self.current_frame = self.frames.len() - 1;
            return self.frames.last();
        }

        self.current_frame = target_frame;
        self.frames_displayed += 1;

        self.frames.get(self.current_frame)
    }

    /// Current playback position in milliseconds.
    pub fn current_position_ms(&self) -> u64 {
        if self.info.frame_rate_num == 0 || self.frames.is_empty() {
            return 0;
        }
        let frame_dur_ms =
            (1000u64 * self.info.frame_rate_den as u64) / self.info.frame_rate_num as u64;
        (self.current_frame as u64) * frame_dur_ms
    }

    /// Total stream duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.info.frame_rate_num == 0 || self.frames.is_empty() {
            return 0;
        }
        let frame_dur_ms =
            (1000u64 * self.info.frame_rate_den as u64) / self.info.frame_rate_num as u64;
        (self.frames.len() as u64) * frame_dur_ms
    }

    /// Whether the stream has reached the end.
    pub fn is_finished(&self) -> bool {
        self.state == PlaybackState::Finished
    }

    /// Get current playback state.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Number of frames in the stream.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Get video info.
    pub fn info(&self) -> &VideoInfo {
        &self.info
    }
}

// ---------------------------------------------------------------------------
// Media player
// ---------------------------------------------------------------------------

/// Higher-level media player that wraps a video stream and a display
/// rectangle.
pub struct MediaPlayer {
    video_stream: Option<RawVideoStream>,
    audio_stream_id: Option<u32>,
    display_x: u32,
    display_y: u32,
    display_width: u32,
    display_height: u32,
}

impl MediaPlayer {
    /// Create a new, empty media player.
    pub fn new() -> Self {
        Self {
            video_stream: None,
            audio_stream_id: None,
            display_x: 0,
            display_y: 0,
            display_width: 0,
            display_height: 0,
        }
    }

    /// Load a single image (TGA, QOI) and present it as a one-frame video.
    pub fn load_image(&mut self, data: &[u8]) -> Result<(), KernelError> {
        let frame = decode::decode_image(data)?;

        let info = VideoInfo {
            width: frame.width,
            height: frame.height,
            format: frame.format,
            frame_rate_num: 1,
            frame_rate_den: 1,
        };

        let mut stream = RawVideoStream::new(info);
        stream.add_frame(frame);

        self.video_stream = Some(stream);
        Ok(())
    }

    /// Load a pre-decoded frame sequence as a video.
    pub fn load_video(
        &mut self,
        frames: Vec<VideoFrame>,
        info: VideoInfo,
    ) -> Result<(), KernelError> {
        if frames.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "frames",
                value: "empty frame list",
            });
        }

        let mut stream = RawVideoStream::new(info);
        for frame in frames {
            stream.add_frame(frame);
        }

        self.video_stream = Some(stream);
        Ok(())
    }

    /// Start playback.
    pub fn play(&mut self) -> Result<(), KernelError> {
        match self.video_stream.as_mut() {
            Some(stream) => {
                stream.play();
                Ok(())
            }
            None => Err(KernelError::InvalidState {
                expected: "loaded",
                actual: "no video loaded",
            }),
        }
    }

    /// Pause playback.
    pub fn pause(&mut self) {
        if let Some(stream) = self.video_stream.as_mut() {
            stream.pause();
        }
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        if let Some(stream) = self.video_stream.as_mut() {
            stream.stop();
        }
    }

    /// Get the current frame for rendering.
    pub fn render_current_frame(&self) -> Option<&VideoFrame> {
        self.video_stream
            .as_ref()
            .and_then(|stream| stream.frames.get(stream.current_frame))
    }

    /// Set the display rectangle on screen.
    pub fn set_display_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.display_x = x;
        self.display_y = y;
        self.display_width = width;
        self.display_height = height;
    }

    /// Get display rectangle.
    pub fn display_rect(&self) -> (u32, u32, u32, u32) {
        (
            self.display_x,
            self.display_y,
            self.display_width,
            self.display_height,
        )
    }

    /// Check if a video is loaded.
    pub fn is_loaded(&self) -> bool {
        self.video_stream.is_some()
    }

    /// Get the playback state.
    pub fn playback_state(&self) -> Option<PlaybackState> {
        self.video_stream.as_ref().map(|s| s.state())
    }
}

impl Default for MediaPlayer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global player instance
// ---------------------------------------------------------------------------

static PLAYER: Mutex<Option<MediaPlayer>> = Mutex::new(None);

/// Initialize the player subsystem.
pub fn init() -> Result<(), KernelError> {
    let mut guard = PLAYER.lock();
    if guard.is_none() {
        *guard = Some(MediaPlayer::new());
    }
    Ok(())
}

/// Execute a closure with the global media player.
pub fn with_player<R, F: FnOnce(&mut MediaPlayer) -> R>(f: F) -> Result<R, KernelError> {
    let mut guard = PLAYER.lock();
    match guard.as_mut() {
        Some(player) => Ok(f(player)),
        None => Err(KernelError::NotInitialized {
            subsystem: "video player",
        }),
    }
}

// ---------------------------------------------------------------------------
// Tick source (monotonic milliseconds)
// ---------------------------------------------------------------------------

/// Simple monotonic tick counter (milliseconds).
///
/// In a full implementation this would read the kernel's timer. For now
/// we use an atomic counter that can be bumped externally or defaults to 0.
static TICK_COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Get the current tick value (milliseconds).
fn get_tick() -> u64 {
    TICK_COUNTER.load(core::sync::atomic::Ordering::Relaxed)
}

/// Advance the tick counter (called by the kernel timer or test harness).
pub fn advance_tick(ms: u64) {
    TICK_COUNTER.fetch_add(ms, core::sync::atomic::Ordering::Relaxed);
}

/// Set the tick counter to an absolute value.
pub fn set_tick(ms: u64) {
    TICK_COUNTER.store(ms, core::sync::atomic::Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::PixelFormat;

    fn make_info(fps: u32) -> VideoInfo {
        VideoInfo {
            width: 4,
            height: 4,
            format: PixelFormat::ARGB8888,
            frame_rate_num: fps,
            frame_rate_den: 1,
        }
    }

    fn make_frame(r: u8) -> VideoFrame {
        let mut f = VideoFrame::new(4, 4, PixelFormat::ARGB8888);
        f.set_pixel(0, 0, r, 0, 0, 255);
        f
    }

    #[test]
    fn test_stream_lifecycle() {
        set_tick(0);
        let mut stream = RawVideoStream::new(make_info(10)); // 10 fps = 100ms/frame
        stream.add_frame(make_frame(10));
        stream.add_frame(make_frame(20));
        stream.add_frame(make_frame(30));

        assert_eq!(stream.state(), PlaybackState::Stopped);
        assert_eq!(stream.duration_ms(), 300); // 3 frames * 100ms

        stream.play();
        assert_eq!(stream.state(), PlaybackState::Playing);

        // At t=0 should show frame 0
        let f = stream.next_frame().unwrap();
        assert_eq!(f.get_pixel(0, 0).0, 10);

        // Advance to t=150ms -> frame 1
        advance_tick(150);
        let f = stream.next_frame().unwrap();
        assert_eq!(f.get_pixel(0, 0).0, 20);

        // Pause
        stream.pause();
        assert_eq!(stream.state(), PlaybackState::Paused);

        // Should still return current frame when paused
        let f = stream.next_frame().unwrap();
        assert_eq!(f.get_pixel(0, 0).0, 20);

        // Stop
        stream.stop();
        assert_eq!(stream.state(), PlaybackState::Stopped);
        assert!(stream.next_frame().is_none());

        // Reset tick for other tests
        set_tick(0);
    }

    #[test]
    fn test_stream_finished() {
        set_tick(0);
        let mut stream = RawVideoStream::new(make_info(10));
        stream.add_frame(make_frame(10));

        stream.play();
        // Advance well past the single frame
        advance_tick(500);
        let _f = stream.next_frame();
        assert!(stream.is_finished());

        set_tick(0);
    }

    #[test]
    fn test_media_player_load_video() {
        let info = make_info(30);
        let frames = alloc::vec![make_frame(1), make_frame(2)];
        let mut player = MediaPlayer::new();

        player.load_video(frames, info).expect("load should work");
        assert!(player.is_loaded());

        let frame = player.render_current_frame().unwrap();
        assert_eq!(frame.width, 4);
    }

    #[test]
    fn test_media_player_display_rect() {
        let mut player = MediaPlayer::new();
        player.set_display_rect(10, 20, 640, 480);
        assert_eq!(player.display_rect(), (10, 20, 640, 480));
    }

    #[test]
    fn test_media_player_play_without_load() {
        let mut player = MediaPlayer::new();
        let result = player.play();
        assert!(result.is_err());
    }

    #[test]
    fn test_seek() {
        let mut stream = RawVideoStream::new(make_info(30));
        stream.add_frame(make_frame(10));
        stream.add_frame(make_frame(20));
        stream.add_frame(make_frame(30));

        stream.seek(2);
        assert_eq!(stream.current_frame, 2);

        // Out of range: should stay at 2
        stream.seek(100);
        assert_eq!(stream.current_frame, 2);
    }
}
