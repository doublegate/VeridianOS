//! NTP client for VeridianOS time synchronization
//!
//! Implements NTPv4 (RFC 5905) client mode with:
//! - 48-byte NTP packet serialization/deserialization
//! - Clock offset and round-trip delay calculation (integer-only)
//! - Marzullo's algorithm for multi-source time selection
//! - Clock filter (best-of-8 by minimum delay)
//! - Drift compensation via integer linear regression
//! - RTC integration for system clock adjustment

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// NTP constants
// ---------------------------------------------------------------------------

/// NTP UDP port
pub const NTP_PORT: u16 = 123;

/// NTP packet size in bytes
pub const NTP_PACKET_SIZE: usize = 48;

/// Seconds between NTP epoch (1900-01-01) and Unix epoch (1970-01-01)
/// 70 years including 17 leap years: (70*365 + 17) * 86400
pub const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// Minimum poll interval in seconds (2^6 = 64s)
pub const MIN_POLL_INTERVAL: u32 = 64;

/// Maximum poll interval in seconds (2^10 = 1024s)
pub const MAX_POLL_INTERVAL: u32 = 1024;

/// Number of samples in the clock filter
pub const CLOCK_FILTER_SIZE: usize = 8;

/// Maximum allowed round-trip delay in milliseconds (reject outliers)
const MAX_DELAY_MS: i64 = 5000;

/// Step threshold in milliseconds (offsets above this trigger a step
/// correction)
const STEP_THRESHOLD_MS: i64 = 128;

/// NTPv4 version number
const NTP_VERSION: u8 = 4;

/// Client mode
const MODE_CLIENT: u8 = 3;

/// Server mode
const MODE_SERVER: u8 = 4;

// ---------------------------------------------------------------------------
// Leap Indicator
// ---------------------------------------------------------------------------

/// Leap indicator values (2-bit field)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LeapIndicator {
    /// No warning
    NoWarning = 0,
    /// Last minute of the day has 61 seconds
    AddSecond = 1,
    /// Last minute of the day has 59 seconds
    DeleteSecond = 2,
    /// Clock not synchronized (alarm)
    Unsynchronized = 3,
}

impl LeapIndicator {
    fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            0 => Self::NoWarning,
            1 => Self::AddSecond,
            2 => Self::DeleteSecond,
            _ => Self::Unsynchronized,
        }
    }
}

// ---------------------------------------------------------------------------
// NTP Timestamp (32.32 fixed-point)
// ---------------------------------------------------------------------------

/// NTP timestamp: 32-bit seconds since 1900-01-01 + 32-bit fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NtpTimestamp {
    /// Seconds since 1900-01-01
    pub seconds: u32,
    /// Fractional seconds (2^-32 second units)
    pub fraction: u32,
}

impl NtpTimestamp {
    /// Create a new NTP timestamp.
    pub fn new(seconds: u32, fraction: u32) -> Self {
        Self { seconds, fraction }
    }

    /// Create from Unix epoch seconds + milliseconds.
    pub fn from_unix(epoch_secs: u64, millis: u32) -> Self {
        let ntp_secs = epoch_secs.saturating_add(NTP_UNIX_OFFSET);
        // Convert milliseconds to NTP fraction: frac = millis * 2^32 / 1000
        let fraction = ((millis as u64) * 4_294_967_296 / 1000) as u32;
        Self {
            seconds: ntp_secs as u32,
            fraction,
        }
    }

    /// Convert to Unix epoch seconds (truncated).
    pub fn to_unix_secs(&self) -> u64 {
        (self.seconds as u64).saturating_sub(NTP_UNIX_OFFSET)
    }

    /// Convert to milliseconds since NTP epoch.
    pub fn to_millis(&self) -> u64 {
        let secs_ms = (self.seconds as u64) * 1000;
        // fraction / 2^32 * 1000 = fraction * 1000 / 2^32
        let frac_ms = (self.fraction as u64) * 1000 / 4_294_967_296;
        secs_ms.saturating_add(frac_ms)
    }

    /// Serialize to 8 big-endian bytes.
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&self.seconds.to_be_bytes());
        buf[4..8].copy_from_slice(&self.fraction.to_be_bytes());
        buf
    }

    /// Deserialize from 8 big-endian bytes.
    pub fn from_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            seconds: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            fraction: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }
}

// ---------------------------------------------------------------------------
// NTP Packet (48 bytes)
// ---------------------------------------------------------------------------

/// NTPv4 packet (48 bytes, RFC 5905).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtpPacket {
    /// Leap indicator (2 bits)
    pub leap: LeapIndicator,
    /// Version number (3 bits)
    pub version: u8,
    /// Mode (3 bits)
    pub mode: u8,
    /// Stratum level (0 = unspecified, 1 = primary, 2-15 = secondary)
    pub stratum: u8,
    /// Maximum interval between successive messages (log2 seconds)
    pub poll: i8,
    /// Precision of the system clock (log2 seconds)
    pub precision: i8,
    /// Total round-trip delay to the reference source (NTP short format, 16.16)
    pub root_delay: u32,
    /// Maximum error relative to the reference source (NTP short format, 16.16)
    pub root_dispersion: u32,
    /// Reference identifier (stratum 1: 4-char ASCII, else: IP address)
    pub reference_id: [u8; 4],
    /// Time when the system clock was last set
    pub reference_ts: NtpTimestamp,
    /// Time at the client when the request departed
    pub origin_ts: NtpTimestamp,
    /// Time at the server when the request arrived
    pub receive_ts: NtpTimestamp,
    /// Time at the server when the response departed
    pub transmit_ts: NtpTimestamp,
}

impl NtpPacket {
    /// Create a new client request packet.
    pub fn new_request(transmit_ts: NtpTimestamp) -> Self {
        Self {
            leap: LeapIndicator::NoWarning,
            version: NTP_VERSION,
            mode: MODE_CLIENT,
            stratum: 0,
            poll: 6,        // 2^6 = 64s
            precision: -18, // ~3.8 microseconds
            root_delay: 0,
            root_dispersion: 0,
            reference_id: [0; 4],
            reference_ts: NtpTimestamp::default(),
            origin_ts: NtpTimestamp::default(),
            receive_ts: NtpTimestamp::default(),
            transmit_ts,
        }
    }

    /// Serialize to a 48-byte array.
    pub fn to_bytes(&self) -> [u8; NTP_PACKET_SIZE] {
        let mut buf = [0u8; NTP_PACKET_SIZE];
        // Byte 0: LI (2) | VN (3) | Mode (3)
        buf[0] = ((self.leap as u8) << 6) | ((self.version & 0x07) << 3) | (self.mode & 0x07);
        buf[1] = self.stratum;
        buf[2] = self.poll as u8;
        buf[3] = self.precision as u8;
        buf[4..8].copy_from_slice(&self.root_delay.to_be_bytes());
        buf[8..12].copy_from_slice(&self.root_dispersion.to_be_bytes());
        buf[12..16].copy_from_slice(&self.reference_id);
        buf[16..24].copy_from_slice(&self.reference_ts.to_bytes());
        buf[24..32].copy_from_slice(&self.origin_ts.to_bytes());
        buf[32..40].copy_from_slice(&self.receive_ts.to_bytes());
        buf[40..48].copy_from_slice(&self.transmit_ts.to_bytes());
        buf
    }

    /// Deserialize from a 48-byte slice.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < NTP_PACKET_SIZE {
            return None;
        }
        let li = (buf[0] >> 6) & 0x03;
        let vn = (buf[0] >> 3) & 0x07;
        let mode = buf[0] & 0x07;

        Some(Self {
            leap: LeapIndicator::from_u8(li),
            version: vn,
            mode,
            stratum: buf[1],
            poll: buf[2] as i8,
            precision: buf[3] as i8,
            root_delay: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
            root_dispersion: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
            reference_id: [buf[12], buf[13], buf[14], buf[15]],
            reference_ts: NtpTimestamp::from_bytes(buf[16..24].try_into().ok()?),
            origin_ts: NtpTimestamp::from_bytes(buf[24..32].try_into().ok()?),
            receive_ts: NtpTimestamp::from_bytes(buf[32..40].try_into().ok()?),
            transmit_ts: NtpTimestamp::from_bytes(buf[40..48].try_into().ok()?),
        })
    }

    /// Check if this is a Kiss-o'-Death (KoD) packet.
    ///
    /// KoD packets have stratum == 0 and a 4-char ASCII code in reference_id.
    pub fn is_kod(&self) -> bool {
        self.stratum == 0
            && self
                .reference_id
                .iter()
                .all(|&b| (0x20..=0x7E).contains(&b))
    }

    /// Get the KoD code (e.g., "DENY", "RATE", "RSTR") if this is a KoD packet.
    pub fn kod_code(&self) -> Option<[u8; 4]> {
        if self.is_kod() {
            Some(self.reference_id)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Clock offset / delay calculation
// ---------------------------------------------------------------------------

/// Result of processing an NTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NtpMeasurement {
    /// Clock offset in milliseconds (positive = local behind server)
    pub offset_ms: i64,
    /// Round-trip delay in milliseconds
    pub delay_ms: i64,
}

/// Calculate clock offset and round-trip delay from NTP timestamps.
///
/// Using the standard NTP formulas (all in milliseconds):
/// - offset = ((t2 - t1) + (t3 - t4)) / 2
/// - delay  = (t4 - t1) - (t3 - t2)
///
/// Where:
/// - t1 = client transmit time (origin)
/// - t2 = server receive time
/// - t3 = server transmit time
/// - t4 = client receive time
pub fn calculate_offset_delay(
    t1: &NtpTimestamp,
    t2: &NtpTimestamp,
    t3: &NtpTimestamp,
    t4: &NtpTimestamp,
) -> NtpMeasurement {
    let t1_ms = t1.to_millis() as i64;
    let t2_ms = t2.to_millis() as i64;
    let t3_ms = t3.to_millis() as i64;
    let t4_ms = t4.to_millis() as i64;

    // offset = ((t2 - t1) + (t3 - t4)) / 2
    let offset_ms = ((t2_ms - t1_ms) + (t3_ms - t4_ms)) / 2;

    // delay = (t4 - t1) - (t3 - t2)
    let delay_ms = (t4_ms - t1_ms) - (t3_ms - t2_ms);

    NtpMeasurement {
        offset_ms,
        delay_ms,
    }
}

// ---------------------------------------------------------------------------
// Clock Filter (best of N samples by minimum delay)
// ---------------------------------------------------------------------------

/// Clock filter: keeps the last CLOCK_FILTER_SIZE samples and selects the
/// one with the smallest round-trip delay as the best estimate.
#[derive(Debug)]
pub struct ClockFilter {
    samples: [Option<NtpMeasurement>; CLOCK_FILTER_SIZE],
    next_idx: usize,
    count: usize,
}

impl Default for ClockFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockFilter {
    /// Create a new empty clock filter.
    pub fn new() -> Self {
        Self {
            samples: [None; CLOCK_FILTER_SIZE],
            next_idx: 0,
            count: 0,
        }
    }

    /// Add a new measurement to the filter.
    pub fn add_sample(&mut self, sample: NtpMeasurement) {
        self.samples[self.next_idx] = Some(sample);
        self.next_idx = (self.next_idx + 1) % CLOCK_FILTER_SIZE;
        if self.count < CLOCK_FILTER_SIZE {
            self.count += 1;
        }
    }

    /// Select the best sample (lowest absolute delay).
    pub fn best_sample(&self) -> Option<NtpMeasurement> {
        let mut best: Option<NtpMeasurement> = None;
        for s in self.samples.iter().flatten() {
            match best {
                None => best = Some(*s),
                Some(b) if s.delay_ms.unsigned_abs() < b.delay_ms.unsigned_abs() => {
                    best = Some(*s);
                }
                _ => {}
            }
        }
        best
    }

    /// Compute jitter as root-mean-square of successive offset differences
    /// (integer square root approximation).
    pub fn jitter_ms(&self) -> u64 {
        if self.count < 2 {
            return 0;
        }

        // Collect valid samples in order
        let mut offsets = [0i64; CLOCK_FILTER_SIZE];
        let mut n = 0usize;
        for s in self.samples.iter().flatten() {
            if n < CLOCK_FILTER_SIZE {
                offsets[n] = s.offset_ms;
                n += 1;
            }
        }

        if n < 2 {
            return 0;
        }

        // Sum of squared differences
        let mut sum_sq: u64 = 0;
        for i in 1..n {
            let diff = offsets[i] - offsets[i - 1];
            // Use checked arithmetic to avoid overflow
            let sq = (diff as i128) * (diff as i128);
            sum_sq = sum_sq.saturating_add(sq as u64);
        }

        let mean_sq = sum_sq / (n as u64 - 1);
        // Integer square root via Newton's method
        isqrt(mean_sq)
    }

    /// Number of valid samples in the filter.
    pub fn sample_count(&self) -> usize {
        self.count
    }
}

/// Integer square root using Newton's method.
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ---------------------------------------------------------------------------
// Marzullo's Algorithm
// ---------------------------------------------------------------------------

/// An interval from a time source for Marzullo's algorithm.
#[derive(Debug, Clone, Copy)]
pub struct TimeInterval {
    /// Lower bound (offset - delay/2) in milliseconds
    pub low: i64,
    /// Upper bound (offset + delay/2) in milliseconds
    pub high: i64,
}

/// Run Marzullo's algorithm on a set of time intervals to find the
/// tightest intersection supported by a majority of sources.
///
/// Returns the best offset estimate (midpoint of the tightest interval)
/// or None if no majority intersection exists.
#[cfg(feature = "alloc")]
pub fn marzullo_select(intervals: &[TimeInterval]) -> Option<i64> {
    if intervals.is_empty() {
        return None;
    }
    if intervals.len() == 1 {
        return Some((intervals[0].low + intervals[0].high) / 2);
    }

    // Build sorted endpoint list: (value, type)
    // type: -1 for interval start, +1 for interval end
    let mut endpoints: Vec<(i64, i32)> = Vec::with_capacity(intervals.len() * 2);
    for iv in intervals {
        endpoints.push((iv.low, -1));
        endpoints.push((iv.high, 1));
    }
    // Sort by value, then by type (starts before ends at same point)
    endpoints.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let n = intervals.len() as i32;
    let majority = (n + 1) / 2; // ceil(n/2)

    let mut best_low = i64::MIN;
    let mut best_high = i64::MAX;
    let mut best_count = 0i32;
    let mut count = 0i32;

    for &(val, kind) in &endpoints {
        // -1 means entering an interval, +1 means leaving
        count -= kind; // entering: count += 1, leaving: count -= 1
        if count >= majority && count > best_count {
            best_count = count;
            best_low = val;
        }
        if count < best_count && best_count >= majority {
            best_high = val;
            break;
        }
    }

    if best_count >= majority && best_low != i64::MIN && best_high != i64::MAX {
        Some((best_low + best_high) / 2)
    } else {
        // Fallback: use interval with smallest delay (closest to true time)
        let mut min_span = i64::MAX;
        let mut best_mid = 0i64;
        for iv in intervals {
            let span = iv.high - iv.low;
            if span < min_span {
                min_span = span;
                best_mid = (iv.low + iv.high) / 2;
            }
        }
        Some(best_mid)
    }
}

// ---------------------------------------------------------------------------
// Drift compensation (integer linear regression)
// ---------------------------------------------------------------------------

/// Drift estimator using integer linear regression over recent measurements.
///
/// Tracks (time, offset) pairs and computes drift rate in parts-per-million
/// (PPM) using integer arithmetic only.
#[derive(Debug)]
pub struct DriftEstimator {
    /// Ring buffer of (elapsed_ms_since_start, offset_ms) pairs
    samples: [(u64, i64); CLOCK_FILTER_SIZE],
    count: usize,
    next_idx: usize,
}

impl Default for DriftEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl DriftEstimator {
    /// Create a new drift estimator.
    pub fn new() -> Self {
        Self {
            samples: [(0, 0); CLOCK_FILTER_SIZE],
            count: 0,
            next_idx: 0,
        }
    }

    /// Add a data point: elapsed time in ms since first measurement, and offset
    /// in ms.
    pub fn add(&mut self, elapsed_ms: u64, offset_ms: i64) {
        self.samples[self.next_idx] = (elapsed_ms, offset_ms);
        self.next_idx = (self.next_idx + 1) % CLOCK_FILTER_SIZE;
        if self.count < CLOCK_FILTER_SIZE {
            self.count += 1;
        }
    }

    /// Compute drift rate in parts-per-billion (PPB) using integer-only
    /// linear regression (slope = sum(dx*dy) / sum(dx^2)).
    ///
    /// Returns None if fewer than 2 samples.
    pub fn drift_ppb(&self) -> Option<i64> {
        if self.count < 2 {
            return None;
        }

        // Compute means (integer approximation)
        let mut sum_x: i128 = 0;
        let mut sum_y: i128 = 0;
        for i in 0..self.count {
            sum_x += self.samples[i].0 as i128;
            sum_y += self.samples[i].1 as i128;
        }
        let n = self.count as i128;
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;

        // Linear regression: slope = sum((xi - mean_x)(yi - mean_y)) / sum((xi -
        // mean_x)^2)
        let mut num: i128 = 0;
        let mut den: i128 = 0;
        for i in 0..self.count {
            let dx = self.samples[i].0 as i128 - mean_x;
            let dy = self.samples[i].1 as i128 - mean_y;
            num += dx * dy;
            den += dx * dx;
        }

        if den == 0 {
            return Some(0);
        }

        // slope = num/den is in ms_offset / ms_elapsed = dimensionless ratio
        // Convert to PPB: slope * 1_000_000_000
        // = (num * 1_000_000_000) / den
        let ppb = (num * 1_000_000_000) / den;
        Some(ppb as i64)
    }
}

// ---------------------------------------------------------------------------
// NTP Client
// ---------------------------------------------------------------------------

/// Global NTP state: last known offset in milliseconds
static NTP_OFFSET_MS: AtomicI64 = AtomicI64::new(0);

/// Global NTP state: last sync timestamp (Unix epoch seconds)
static LAST_SYNC_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Global NTP state: current poll interval in seconds
static POLL_INTERVAL: AtomicU64 = AtomicU64::new(MIN_POLL_INTERVAL as u64);

/// NTP client managing time synchronization with one or more servers.
pub struct NtpClient {
    /// Clock filter for selecting best sample
    pub filter: ClockFilter,
    /// Drift estimator for frequency compensation
    pub drift: DriftEstimator,
    /// Current poll interval in seconds
    pub poll_interval: u32,
    /// Our stratum level (one more than best server's stratum)
    pub stratum: u8,
    /// Whether a step correction has been applied since boot
    pub step_applied: bool,
    /// Measurement start time in ms (for drift tracking)
    pub start_ms: u64,
}

impl Default for NtpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NtpClient {
    /// Create a new NTP client.
    pub fn new() -> Self {
        Self {
            filter: ClockFilter::new(),
            drift: DriftEstimator::new(),
            poll_interval: MIN_POLL_INTERVAL,
            stratum: 16, // unsynchronized
            step_applied: false,
            start_ms: 0,
        }
    }

    /// Create a client request packet for the given transmit timestamp.
    pub fn create_request(&self, transmit_ts: NtpTimestamp) -> [u8; NTP_PACKET_SIZE] {
        let pkt = NtpPacket::new_request(transmit_ts);
        pkt.to_bytes()
    }

    /// Process a server response and return the computed measurement.
    ///
    /// - `response`: the raw 48-byte NTP response
    /// - `t1`: our original transmit timestamp
    /// - `t4`: our receive timestamp (when we got the response)
    pub fn process_response(
        &mut self,
        response: &[u8],
        t1: &NtpTimestamp,
        t4: &NtpTimestamp,
    ) -> Option<NtpMeasurement> {
        let pkt = NtpPacket::from_bytes(response)?;

        // Reject KoD packets
        if pkt.is_kod() {
            return None;
        }

        // Reject non-server responses
        if pkt.mode != MODE_SERVER {
            return None;
        }

        // Reject unsynchronized servers
        if pkt.stratum == 0 || pkt.stratum >= 16 {
            return None;
        }

        let t2 = &pkt.receive_ts;
        let t3 = &pkt.transmit_ts;
        let measurement = calculate_offset_delay(t1, t2, t3, t4);

        // Reject if delay is unreasonable
        if measurement.delay_ms.unsigned_abs() > MAX_DELAY_MS as u64 {
            return None;
        }

        // Update our stratum
        self.stratum = pkt.stratum.saturating_add(1).min(15);

        // Add to clock filter
        self.filter.add_sample(measurement);

        // Add to drift estimator
        let elapsed = t4.to_millis().saturating_sub(self.start_ms);
        self.drift.add(elapsed, measurement.offset_ms);

        // Handle leap indicator
        if pkt.leap == LeapIndicator::AddSecond || pkt.leap == LeapIndicator::DeleteSecond {
            // Log leap second warning (actual adjustment happens at midnight)
            // In a full implementation this would schedule the leap second
            // insertion
        }

        Some(measurement)
    }

    /// Decide whether to step (instant jump) or slew (gradual adjust) the
    /// clock, and apply the correction via the RTC integration point.
    ///
    /// Returns the applied correction in milliseconds.
    pub fn adjust_clock(&mut self) -> i64 {
        let best = match self.filter.best_sample() {
            Some(s) => s,
            None => return 0,
        };

        let offset = best.offset_ms;

        // Apply drift compensation
        let drift_correction = match self.drift.drift_ppb() {
            Some(ppb) => {
                // drift_correction_ms = ppb * poll_interval_ms / 1_000_000_000
                let poll_ms = (self.poll_interval as i64) * 1000;
                (ppb * poll_ms) / 1_000_000_000
            }
            None => 0,
        };

        let total_correction = offset + drift_correction;

        // Step or slew decision
        if total_correction.unsigned_abs() > STEP_THRESHOLD_MS as u64 && !self.step_applied {
            // Step: apply full correction immediately
            apply_time_correction(total_correction);
            self.step_applied = true;
            // Reset poll interval after step
            self.poll_interval = MIN_POLL_INTERVAL;
        } else {
            // Slew: apply correction gradually
            apply_time_correction(total_correction);
            // Increase poll interval on stable corrections
            if total_correction.unsigned_abs() < 10 {
                self.increase_poll_interval();
            } else if total_correction.unsigned_abs() > 50 {
                self.decrease_poll_interval();
            }
        }

        NTP_OFFSET_MS.store(total_correction, Ordering::Relaxed);
        total_correction
    }

    /// Increase poll interval (double, up to maximum).
    fn increase_poll_interval(&mut self) {
        let new_interval = self.poll_interval.saturating_mul(2).min(MAX_POLL_INTERVAL);
        self.poll_interval = new_interval;
        POLL_INTERVAL.store(new_interval as u64, Ordering::Relaxed);
    }

    /// Decrease poll interval (halve, down to minimum).
    fn decrease_poll_interval(&mut self) {
        let new_interval = (self.poll_interval / 2).max(MIN_POLL_INTERVAL);
        self.poll_interval = new_interval;
        POLL_INTERVAL.store(new_interval as u64, Ordering::Relaxed);
    }

    /// Get the current poll interval in seconds.
    pub fn get_poll_interval(&self) -> u32 {
        self.poll_interval
    }
}

// ---------------------------------------------------------------------------
// RTC Integration
// ---------------------------------------------------------------------------

/// Apply a time correction to the system clock via the RTC subsystem.
///
/// On x86_64, this calls into the RTC module's NTP correction interface.
/// On other architectures, the offset is stored in the global atomic.
fn apply_time_correction(offset_ms: i64) {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::x86_64::rtc::set_time_correction(offset_ms);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        NTP_OFFSET_MS.store(offset_ms, Ordering::Relaxed);
    }
}

/// Get the last NTP-computed offset in milliseconds.
pub fn get_ntp_offset_ms() -> i64 {
    NTP_OFFSET_MS.load(Ordering::Relaxed)
}

/// Get the last NTP sync time as Unix epoch seconds (0 = never synced).
pub fn get_last_sync_epoch() -> u64 {
    LAST_SYNC_EPOCH.load(Ordering::Relaxed)
}

/// Record that a successful NTP sync occurred at the given Unix epoch time.
pub fn record_sync(epoch_secs: u64) {
    LAST_SYNC_EPOCH.store(epoch_secs, Ordering::Relaxed);
}

/// Get the current poll interval in seconds.
pub fn get_poll_interval() -> u64 {
    POLL_INTERVAL.load(Ordering::Relaxed)
}

/// Trigger a boot-time NTP synchronization.
///
/// In a full implementation this would send a request to a configured
/// NTP server via UDP port 123. Here we initialize the client state.
pub fn boot_sync() -> NtpClient {
    #[allow(unused_mut)]
    let mut client = NtpClient::new();
    // Set start time from system clock if available
    #[cfg(target_arch = "x86_64")]
    {
        let epoch = crate::arch::x86_64::rtc::current_epoch_secs();
        client.start_ms = epoch * 1000;
    }
    client
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -- Timestamp tests --

    #[test]
    fn test_timestamp_to_from_bytes_roundtrip() {
        let ts = NtpTimestamp::new(0xAABBCCDD, 0x11223344);
        let bytes = ts.to_bytes();
        let ts2 = NtpTimestamp::from_bytes(&bytes);
        assert_eq!(ts, ts2);
    }

    #[test]
    fn test_timestamp_zero() {
        let ts = NtpTimestamp::new(0, 0);
        let bytes = ts.to_bytes();
        assert_eq!(bytes, [0u8; 8]);
        let ts2 = NtpTimestamp::from_bytes(&bytes);
        assert_eq!(ts2.seconds, 0);
        assert_eq!(ts2.fraction, 0);
    }

    #[test]
    fn test_timestamp_ntp_to_unix_epoch_conversion() {
        // NTP epoch is 1900-01-01, Unix is 1970-01-01
        // At NTP seconds = NTP_UNIX_OFFSET, Unix should be 0
        let ts = NtpTimestamp::new(NTP_UNIX_OFFSET as u32, 0);
        assert_eq!(ts.to_unix_secs(), 0);
    }

    #[test]
    fn test_timestamp_unix_to_ntp_conversion() {
        // Unix epoch 0 -> NTP seconds = NTP_UNIX_OFFSET
        let ts = NtpTimestamp::from_unix(0, 0);
        assert_eq!(ts.seconds, NTP_UNIX_OFFSET as u32);
        assert_eq!(ts.fraction, 0);
    }

    #[test]
    fn test_timestamp_millis_fraction() {
        // 500ms = 0.5s -> fraction ~= 2^31 = 2147483648
        let ts = NtpTimestamp::from_unix(0, 500);
        // Allow small rounding: should be close to 2^31
        let expected = 2_147_483_648u32;
        let diff = if ts.fraction > expected {
            ts.fraction - expected
        } else {
            expected - ts.fraction
        };
        assert!(
            diff < 5000,
            "fraction {} not close to {}",
            ts.fraction,
            expected
        );
    }

    // -- Packet tests --

    #[test]
    fn test_packet_serialization_roundtrip() {
        let ts = NtpTimestamp::new(1000, 2000);
        let pkt = NtpPacket::new_request(ts);
        let bytes = pkt.to_bytes();
        let pkt2 = NtpPacket::from_bytes(&bytes).unwrap();
        assert_eq!(pkt, pkt2);
    }

    #[test]
    fn test_packet_too_short() {
        let buf = [0u8; 10];
        assert!(NtpPacket::from_bytes(&buf).is_none());
    }

    #[test]
    fn test_packet_header_fields() {
        let ts = NtpTimestamp::new(100, 200);
        let pkt = NtpPacket::new_request(ts);
        let bytes = pkt.to_bytes();
        // Byte 0: LI=0(2), VN=4(3), Mode=3(3) -> 0b00_100_011 = 0x23
        assert_eq!(bytes[0], 0x23);
        assert_eq!(bytes[1], 0); // stratum
        assert_eq!(bytes[2], 6); // poll
    }

    #[test]
    fn test_kod_detection() {
        let mut pkt = NtpPacket::new_request(NtpTimestamp::default());
        pkt.mode = MODE_SERVER;
        pkt.stratum = 0;
        pkt.reference_id = *b"DENY";
        assert!(pkt.is_kod());
        assert_eq!(pkt.kod_code(), Some(*b"DENY"));
    }

    #[test]
    fn test_kod_not_triggered_normal_packet() {
        let mut pkt = NtpPacket::new_request(NtpTimestamp::default());
        pkt.mode = MODE_SERVER;
        pkt.stratum = 2;
        pkt.reference_id = [192, 168, 1, 1]; // IP address, not ASCII
        assert!(!pkt.is_kod());
        assert_eq!(pkt.kod_code(), None);
    }

    // -- Clock offset / delay tests --

    #[test]
    fn test_clock_offset_symmetric() {
        // Symmetric delay: server 100ms ahead
        // t1=1000, t2=1100, t3=1100, t4=1000 (all in NTP seconds-as-ms)
        let t1 = NtpTimestamp::new(1, 0); // 1000ms
        let t2 = NtpTimestamp::new(2, 0); // 2000ms (server +1s)
        let t3 = NtpTimestamp::new(2, 0); // 2000ms
        let t4 = NtpTimestamp::new(1, 0); // 1000ms
        let m = calculate_offset_delay(&t1, &t2, &t3, &t4);
        // offset = ((2000-1000) + (2000-1000)) / 2 = 1000ms
        assert_eq!(m.offset_ms, 1000);
        // delay = (1000-1000) - (2000-2000) = 0
        assert_eq!(m.delay_ms, 0);
    }

    #[test]
    fn test_round_trip_delay() {
        // t1=0s, t2=0.05s, t3=0.05s, t4=0.1s -> RTT=0.1s, offset=0
        let t1 = NtpTimestamp::new(0, 0);
        let t2 = NtpTimestamp::new(0, 214748365); // ~50ms
        let t3 = NtpTimestamp::new(0, 214748365); // ~50ms
        let t4 = NtpTimestamp::new(0, 429496730); // ~100ms
        let m = calculate_offset_delay(&t1, &t2, &t3, &t4);
        // offset should be ~0, delay should be ~100ms
        assert!(m.offset_ms.unsigned_abs() <= 1, "offset: {}", m.offset_ms);
        assert!(
            m.delay_ms >= 95 && m.delay_ms <= 105,
            "delay: {}",
            m.delay_ms
        );
    }

    // -- Clock filter tests --

    #[test]
    fn test_clock_filter_best_sample() {
        let mut filter = ClockFilter::new();
        filter.add_sample(NtpMeasurement {
            offset_ms: 10,
            delay_ms: 100,
        });
        filter.add_sample(NtpMeasurement {
            offset_ms: 12,
            delay_ms: 50,
        });
        filter.add_sample(NtpMeasurement {
            offset_ms: 8,
            delay_ms: 200,
        });
        let best = filter.best_sample().unwrap();
        // Best = lowest delay = 50ms
        assert_eq!(best.delay_ms, 50);
        assert_eq!(best.offset_ms, 12);
    }

    #[test]
    fn test_clock_filter_empty() {
        let filter = ClockFilter::new();
        assert!(filter.best_sample().is_none());
        assert_eq!(filter.sample_count(), 0);
    }

    #[test]
    fn test_clock_filter_jitter() {
        let mut filter = ClockFilter::new();
        // Add samples with increasing offsets: 10, 20, 30, 40
        for i in 1..=4 {
            filter.add_sample(NtpMeasurement {
                offset_ms: i * 10,
                delay_ms: 50,
            });
        }
        let jitter = filter.jitter_ms();
        // Differences: 10, 10, 10 -> RMS = sqrt(100) = 10
        assert_eq!(jitter, 10);
    }

    // -- Marzullo's algorithm tests --

    #[cfg(feature = "alloc")]
    #[test]
    fn test_marzullo_single_source() {
        let intervals = vec![TimeInterval { low: 90, high: 110 }];
        let result = marzullo_select(&intervals);
        assert_eq!(result, Some(100));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_marzullo_overlapping_sources() {
        // Three sources agreeing roughly on offset=100
        let intervals = vec![
            TimeInterval { low: 90, high: 110 },
            TimeInterval { low: 95, high: 115 },
            TimeInterval { low: 85, high: 105 },
        ];
        let result = marzullo_select(&intervals).unwrap();
        // Majority intersection should be around 95..105, midpoint ~100
        assert!(result >= 90 && result <= 110, "result: {}", result);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_marzullo_empty() {
        let intervals: Vec<TimeInterval> = vec![];
        assert!(marzullo_select(&intervals).is_none());
    }

    // -- Drift estimator tests --

    #[test]
    fn test_drift_estimator_too_few_samples() {
        let est = DriftEstimator::new();
        assert!(est.drift_ppb().is_none());
    }

    #[test]
    fn test_drift_estimator_constant_offset() {
        let mut est = DriftEstimator::new();
        // Constant offset = no drift
        est.add(0, 100);
        est.add(1000, 100);
        est.add(2000, 100);
        let ppb = est.drift_ppb().unwrap();
        assert_eq!(ppb, 0);
    }

    #[test]
    fn test_drift_estimator_linear_drift() {
        let mut est = DriftEstimator::new();
        // 1ms offset per 1000ms elapsed = 1 PPM = 1000 PPB
        est.add(0, 0);
        est.add(1000, 1);
        est.add(2000, 2);
        est.add(3000, 3);
        let ppb = est.drift_ppb().unwrap();
        assert_eq!(ppb, 1_000_000);
    }

    // -- Poll interval tests --

    #[test]
    fn test_poll_interval_bounds() {
        let mut client = NtpClient::new();
        assert_eq!(client.get_poll_interval(), MIN_POLL_INTERVAL);

        // Increase multiple times
        for _ in 0..20 {
            client.increase_poll_interval();
        }
        assert!(client.get_poll_interval() <= MAX_POLL_INTERVAL);
        assert_eq!(client.get_poll_interval(), MAX_POLL_INTERVAL);

        // Decrease multiple times
        for _ in 0..20 {
            client.decrease_poll_interval();
        }
        assert!(client.get_poll_interval() >= MIN_POLL_INTERVAL);
        assert_eq!(client.get_poll_interval(), MIN_POLL_INTERVAL);
    }

    // -- Leap indicator tests --

    #[test]
    fn test_leap_indicator_roundtrip() {
        for val in 0..=3u8 {
            let li = LeapIndicator::from_u8(val);
            assert_eq!(li as u8, val);
        }
    }

    #[test]
    fn test_leap_indicator_in_packet() {
        let ts = NtpTimestamp::default();
        let mut pkt = NtpPacket::new_request(ts);
        pkt.leap = LeapIndicator::AddSecond;
        let bytes = pkt.to_bytes();
        let pkt2 = NtpPacket::from_bytes(&bytes).unwrap();
        assert_eq!(pkt2.leap, LeapIndicator::AddSecond);
    }

    // -- Integer sqrt tests --

    #[test]
    fn test_isqrt() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(99), 9); // floor
        assert_eq!(isqrt(1_000_000), 1000);
    }
}
