//! Software VSync Timer
//!
//! Provides a software-based VSync timer using TSC (Time Stamp Counter)
//! for frame pacing at ~60Hz (16.667ms per frame). Coordinates double-buffer
//! swap timing for smooth presentation without hardware VSync support.
//!
//! All timing uses integer nanoseconds (no FPU required).

#![allow(dead_code)]

use alloc::vec::Vec;
#[cfg(not(target_arch = "x86_64"))]
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// VSync interval in nanoseconds for ~60Hz (1_000_000_000 / 60 =
/// 16_666_666.67). Rounded to nearest integer.
pub(crate) const VSYNC_INTERVAL_NS: u64 = 16_666_667;

/// VSync interval in microseconds (for coarser timing).
const VSYNC_INTERVAL_US: u64 = 16_667;

/// Number of frame times to keep for rolling average.
const FRAME_HISTORY_SIZE: usize = 60;

/// If a frame takes longer than 2x the VSync interval, it counts as dropped.
const DROPPED_FRAME_THRESHOLD_NS: u64 = VSYNC_INTERVAL_NS * 2;

// ---------------------------------------------------------------------------
// SwapState
// ---------------------------------------------------------------------------

/// Double-buffer swap state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwapState {
    /// No swap in progress; ready to begin rendering.
    Idle,
    /// Back buffer is being rendered to.
    Rendering,
    /// Rendering complete, waiting for VSync tick to flip.
    WaitingFlip,
    /// Buffers have been swapped; new frame is on-screen.
    Flipped,
}

// ---------------------------------------------------------------------------
// FrameStats
// ---------------------------------------------------------------------------

/// Accumulated frame timing statistics (integer nanoseconds).
#[derive(Debug, Clone)]
pub(crate) struct FrameStats {
    /// Total number of frames presented.
    pub frame_count: u64,
    /// Number of dropped frames (took > 2x VSync interval).
    pub dropped_frames: u64,
    /// Rolling frame time history (nanoseconds).
    frame_times: Vec<u64>,
    /// Index into circular frame_times buffer.
    write_idx: usize,
    /// Minimum frame time observed (ns).
    pub min_frame_time_ns: u64,
    /// Maximum frame time observed (ns).
    pub max_frame_time_ns: u64,
}

impl FrameStats {
    fn new() -> Self {
        Self {
            frame_count: 0,
            dropped_frames: 0,
            frame_times: Vec::new(),
            write_idx: 0,
            min_frame_time_ns: u64::MAX,
            max_frame_time_ns: 0,
        }
    }

    /// Record a frame's duration.
    fn record(&mut self, frame_time_ns: u64) {
        self.frame_count += 1;

        if frame_time_ns > DROPPED_FRAME_THRESHOLD_NS {
            self.dropped_frames += 1;
        }

        if frame_time_ns < self.min_frame_time_ns {
            self.min_frame_time_ns = frame_time_ns;
        }
        if frame_time_ns > self.max_frame_time_ns {
            self.max_frame_time_ns = frame_time_ns;
        }

        // Circular buffer
        if self.frame_times.len() < FRAME_HISTORY_SIZE {
            self.frame_times.push(frame_time_ns);
        } else {
            self.frame_times[self.write_idx] = frame_time_ns;
        }
        self.write_idx = (self.write_idx + 1) % FRAME_HISTORY_SIZE;
    }

    /// Average frame time in nanoseconds.
    pub(crate) fn average_frame_time_ns(&self) -> u64 {
        if self.frame_times.is_empty() {
            return 0;
        }
        let sum: u64 = self.frame_times.iter().sum();
        sum / self.frame_times.len() as u64
    }

    /// Estimated FPS (integer, based on average frame time).
    pub(crate) fn estimated_fps(&self) -> u32 {
        let avg = self.average_frame_time_ns();
        if avg == 0 {
            return 0;
        }
        // fps = 1_000_000_000 / avg
        (1_000_000_000u64 / avg) as u32
    }

    /// Drop rate as parts-per-thousand (0 = no drops, 1000 = all dropped).
    pub(crate) fn drop_rate_permille(&self) -> u32 {
        if self.frame_count == 0 {
            return 0;
        }
        ((self.dropped_frames * 1000) / self.frame_count) as u32
    }
}

// ---------------------------------------------------------------------------
// SwVsyncState
// ---------------------------------------------------------------------------

/// Software VSync state tracking frame timing and buffer swaps.
pub(crate) struct SwVsyncState {
    /// Monotonic TSC-based timestamp of last VSync tick (nanoseconds).
    last_vsync_ns: u64,
    /// Target interval between frames (nanoseconds).
    interval_ns: u64,
    /// Current swap state.
    swap_state: SwapState,
    /// Frame statistics.
    stats: FrameStats,
    /// TSC ticks per nanosecond (calibrated at init), stored as
    /// fixed-point 32.32 to avoid FP. 0 means uncalibrated (use
    /// fallback).
    tsc_per_ns_fp32: u64,
    /// Whether VSync is enabled.
    enabled: bool,
    /// Frame start timestamp (ns) for measuring frame duration.
    frame_start_ns: u64,
}

impl SwVsyncState {
    /// Create a new VSync state (uncalibrated).
    pub(crate) fn new() -> Self {
        Self {
            last_vsync_ns: 0,
            interval_ns: VSYNC_INTERVAL_NS,
            swap_state: SwapState::Idle,
            stats: FrameStats::new(),
            tsc_per_ns_fp32: 0,
            enabled: true,
            frame_start_ns: 0,
        }
    }

    /// Create with a custom refresh interval.
    pub(crate) fn with_interval_ns(interval_ns: u64) -> Self {
        let mut s = Self::new();
        s.interval_ns = interval_ns;
        s
    }

    /// Set TSC calibration (ticks per nanosecond as 32.32 fixed-point).
    pub(crate) fn set_tsc_calibration(&mut self, tsc_per_ns_fp32: u64) {
        self.tsc_per_ns_fp32 = tsc_per_ns_fp32;
    }

    /// Convert a TSC reading to nanoseconds using calibration.
    fn tsc_to_ns(&self, tsc: u64) -> u64 {
        if self.tsc_per_ns_fp32 == 0 {
            // Fallback: assume ~3 GHz (1 tick = ~0.33 ns)
            // ns = tsc / 3
            tsc / 3
        } else {
            // ns = tsc / (tsc_per_ns_fp32 >> 32) approximately
            // More precisely: ns = (tsc << 32) / tsc_per_ns_fp32
            // but that overflows for large tsc values. Use u128.
            let numerator = (tsc as u128) << 32;
            (numerator / self.tsc_per_ns_fp32 as u128) as u64
        }
    }

    /// Read the current TSC (x86_64 RDTSC).
    #[inline]
    fn read_tsc(&self) -> u64 {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: RDTSC is always available on x86_64.
            unsafe { core::arch::x86_64::_rdtsc() }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            // Fallback: use a simple counter (not accurate but compiles).
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            COUNTER.fetch_add(1, Ordering::Relaxed)
        }
    }

    /// Get current monotonic time in nanoseconds.
    fn now_ns(&self) -> u64 {
        self.tsc_to_ns(self.read_tsc())
    }

    // -- VSync control -------------------------------------------------------

    /// Enable or disable VSync pacing.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Whether VSync is enabled.
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the target refresh rate in mHz (e.g. 60000 for 60 Hz).
    pub(crate) fn set_refresh_mhz(&mut self, mhz: u32) {
        if mhz > 0 {
            // interval_ns = 1_000_000_000_000 / mhz (since mhz is milliHz)
            self.interval_ns = 1_000_000_000_000u64 / mhz as u64;
        }
    }

    // -- Frame timing --------------------------------------------------------

    /// Wait until the next VSync tick. Busy-waits using TSC.
    ///
    /// Call this after rendering is complete and before flipping buffers.
    pub(crate) fn wait(&mut self) {
        if !self.enabled {
            return;
        }

        let now = self.now_ns();

        if self.last_vsync_ns == 0 {
            // First frame: no wait needed, just record the time.
            self.last_vsync_ns = now;
            return;
        }

        // Calculate next VSync deadline
        let mut next_vsync = self.last_vsync_ns.saturating_add(self.interval_ns);

        // If we already missed the deadline, skip to the next one
        if now > next_vsync {
            let elapsed = now - self.last_vsync_ns;
            let skipped = elapsed / self.interval_ns;
            next_vsync = self
                .last_vsync_ns
                .saturating_add(self.interval_ns.saturating_mul(skipped + 1));
        }

        // Busy-wait until deadline
        while self.now_ns() < next_vsync {
            core::hint::spin_loop();
        }

        self.last_vsync_ns = next_vsync;
    }

    /// Signal that a frame has started rendering.
    pub(crate) fn begin_frame(&mut self) {
        self.frame_start_ns = self.now_ns();
        self.swap_state = SwapState::Rendering;
    }

    /// Signal that rendering is complete (ready to flip).
    pub(crate) fn end_render(&mut self) {
        self.swap_state = SwapState::WaitingFlip;
    }

    /// Signal that the frame has been presented.
    pub(crate) fn signal_complete(&mut self) {
        let now = self.now_ns();
        if self.frame_start_ns > 0 {
            let frame_time = now.saturating_sub(self.frame_start_ns);
            self.stats.record(frame_time);
        }
        self.swap_state = SwapState::Flipped;
    }

    // -- Swap coordination ---------------------------------------------------

    /// Request a buffer swap. Returns true if the swap can proceed.
    pub(crate) fn request_swap(&mut self) -> bool {
        match self.swap_state {
            SwapState::WaitingFlip => {
                if self.enabled {
                    self.wait();
                }
                self.swap_state = SwapState::Flipped;
                true
            }
            SwapState::Idle => {
                // No active render, immediate swap OK
                self.swap_state = SwapState::Flipped;
                true
            }
            _ => false,
        }
    }

    /// Mark swap as complete; return to idle for next frame.
    pub(crate) fn swap_complete(&mut self) {
        self.signal_complete();
        self.swap_state = SwapState::Idle;
    }

    // -- Queries -------------------------------------------------------------

    /// Current swap state.
    pub(crate) fn swap_state(&self) -> SwapState {
        self.swap_state
    }

    /// Frame statistics.
    pub(crate) fn stats(&self) -> &FrameStats {
        &self.stats
    }

    /// Target VSync interval in nanoseconds.
    pub(crate) fn interval_ns(&self) -> u64 {
        self.interval_ns
    }
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

static SW_VSYNC: Mutex<Option<SwVsyncState>> = Mutex::new(None);

/// Initialize the software VSync timer.
pub(crate) fn sw_vsync_init() {
    let mut guard = SW_VSYNC.lock();
    if guard.is_none() {
        *guard = Some(SwVsyncState::new());
    }
}

/// Initialize with a specific refresh rate (mHz).
pub(crate) fn sw_vsync_init_with_refresh(refresh_mhz: u32) {
    let mut guard = SW_VSYNC.lock();
    let mut state = SwVsyncState::new();
    state.set_refresh_mhz(refresh_mhz);
    *guard = Some(state);
}

/// Wait for next VSync tick.
pub(crate) fn sw_vsync_wait() {
    if let Some(ref mut state) = *SW_VSYNC.lock() {
        state.wait();
    }
}

/// Signal frame complete.
pub(crate) fn sw_vsync_signal() {
    if let Some(ref mut state) = *SW_VSYNC.lock() {
        state.signal_complete();
    }
}

/// Access the software VSync state.
pub(crate) fn with_sw_vsync<R, F: FnOnce(&mut SwVsyncState) -> R>(f: F) -> Option<R> {
    SW_VSYNC.lock().as_mut().map(f)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vsync_interval_ns() {
        // ~60Hz
        assert_eq!(VSYNC_INTERVAL_NS, 16_666_667);
    }

    #[test]
    fn test_swap_state_transitions() {
        let mut state = SwVsyncState::new();
        state.set_enabled(false); // disable wait for test speed

        assert_eq!(state.swap_state(), SwapState::Idle);

        state.begin_frame();
        assert_eq!(state.swap_state(), SwapState::Rendering);

        state.end_render();
        assert_eq!(state.swap_state(), SwapState::WaitingFlip);

        assert!(state.request_swap());
        assert_eq!(state.swap_state(), SwapState::Flipped);

        state.swap_complete();
        assert_eq!(state.swap_state(), SwapState::Idle);
    }

    #[test]
    fn test_swap_request_from_idle() {
        let mut state = SwVsyncState::new();
        state.set_enabled(false);
        assert!(state.request_swap());
    }

    #[test]
    fn test_swap_request_during_render() {
        let mut state = SwVsyncState::new();
        state.begin_frame();
        // Cannot swap while rendering
        assert!(!state.request_swap());
    }

    #[test]
    fn test_frame_stats_new() {
        let stats = FrameStats::new();
        assert_eq!(stats.frame_count, 0);
        assert_eq!(stats.dropped_frames, 0);
        assert_eq!(stats.average_frame_time_ns(), 0);
        assert_eq!(stats.estimated_fps(), 0);
    }

    #[test]
    fn test_frame_stats_record() {
        let mut stats = FrameStats::new();
        stats.record(16_666_667); // ~60fps frame
        assert_eq!(stats.frame_count, 1);
        assert_eq!(stats.dropped_frames, 0);
        assert_eq!(stats.min_frame_time_ns, 16_666_667);
        assert_eq!(stats.max_frame_time_ns, 16_666_667);
    }

    #[test]
    fn test_frame_stats_dropped() {
        let mut stats = FrameStats::new();
        // A frame that takes 40ms (> 2 * 16.67ms)
        stats.record(40_000_000);
        assert_eq!(stats.dropped_frames, 1);
    }

    #[test]
    fn test_frame_stats_average() {
        let mut stats = FrameStats::new();
        stats.record(10_000_000);
        stats.record(20_000_000);
        assert_eq!(stats.average_frame_time_ns(), 15_000_000);
    }

    #[test]
    fn test_frame_stats_fps() {
        let mut stats = FrameStats::new();
        // Record 60 frames at exactly 16.666ms
        for _ in 0..60 {
            stats.record(16_666_667);
        }
        let fps = stats.estimated_fps();
        // Should be ~60
        assert!(fps >= 59 && fps <= 60, "fps was {}", fps);
    }

    #[test]
    fn test_frame_stats_drop_rate() {
        let mut stats = FrameStats::new();
        for _ in 0..9 {
            stats.record(16_000_000); // normal
        }
        stats.record(40_000_000); // dropped
                                  // 1/10 dropped = 100 permille
        assert_eq!(stats.drop_rate_permille(), 100);
    }

    #[test]
    fn test_set_refresh_60hz() {
        let mut state = SwVsyncState::new();
        state.set_refresh_mhz(60_000);
        // 1_000_000_000_000 / 60000 = 16_666_666
        assert_eq!(state.interval_ns(), 16_666_666);
    }

    #[test]
    fn test_set_refresh_144hz() {
        let mut state = SwVsyncState::new();
        state.set_refresh_mhz(144_000);
        // 1_000_000_000_000 / 144000 = 6_944_444
        assert_eq!(state.interval_ns(), 6_944_444);
    }

    #[test]
    fn test_custom_interval() {
        let state = SwVsyncState::with_interval_ns(8_333_333); // ~120Hz
        assert_eq!(state.interval_ns(), 8_333_333);
    }

    #[test]
    fn test_enable_disable() {
        let mut state = SwVsyncState::new();
        assert!(state.is_enabled());
        state.set_enabled(false);
        assert!(!state.is_enabled());
    }

    #[test]
    fn test_frame_stats_circular_buffer() {
        let mut stats = FrameStats::new();
        // Fill beyond FRAME_HISTORY_SIZE
        for i in 0..FRAME_HISTORY_SIZE + 10 {
            stats.record((i as u64 + 1) * 1_000_000);
        }
        assert_eq!(stats.frame_times.len(), FRAME_HISTORY_SIZE);
        assert_eq!(stats.frame_count, (FRAME_HISTORY_SIZE + 10) as u64);
    }
}
