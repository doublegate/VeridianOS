//! TCP Congestion Control: Reno and Cubic
//!
//! Implements RFC 5681 Reno (slow start, congestion avoidance, fast retransmit,
//! fast recovery) and RFC 8312 Cubic congestion control. Uses Jacobson's
//! algorithm (RFC 6298) for RTO estimation. All arithmetic is
//! integer/fixed-point (no floating point) for `no_std` compatibility.

#![allow(dead_code)]

/// Maximum Segment Size (standard Ethernet)
const MSS: u32 = 1460;

/// Minimum RTO in microseconds (1 second per RFC 6298)
const RTO_MIN_US: u64 = 1_000_000;

/// Maximum RTO in microseconds (60 seconds per RFC 6298)
const RTO_MAX_US: u64 = 60_000_000;

/// Initial RTO before any RTT measurement (1 second per RFC 6298)
const RTO_INITIAL_US: u64 = 1_000_000;

/// Clock granularity in microseconds (1ms)
const CLOCK_GRANULARITY_US: u64 = 1_000;

/// Duplicate ACK threshold for fast retransmit
const DUP_ACK_THRESHOLD: u32 = 3;

/// Fixed-point shift for Jacobson's algorithm (alpha = 1/8, beta = 1/4)
/// SRTT and RTTVAR are stored shifted left by SHIFT bits for precision.
const SRTT_SHIFT: u32 = 3; // alpha = 1/8 = 1/(2^3)
const RTTVAR_SHIFT: u32 = 2; // beta = 1/4 = 1/(2^2)

/// Congestion control phase
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CongestionPhase {
    /// Exponential growth: cwnd doubles per RTT
    SlowStart,
    /// Linear growth: cwnd increases by ~MSS per RTT
    CongestionAvoidance,
    /// After fast retransmit: inflate cwnd with dup ACKs, deflate on new ACK
    FastRecovery,
}

/// Congestion controller trait
///
/// Provides a pluggable interface for congestion control algorithms.
/// The default implementation is TCP Reno (`RenoController`).
pub trait CongestionController: Send + Sync {
    /// Called when new data is acknowledged.
    ///
    /// `bytes_acked`: number of newly acknowledged bytes
    /// `rtt_us`: measured round-trip time in microseconds (0 if unavailable)
    fn on_ack(&mut self, bytes_acked: u32, rtt_us: u64);

    /// Called when a duplicate ACK is received.
    fn on_duplicate_ack(&mut self);

    /// Called when a retransmission timeout fires.
    fn on_timeout(&mut self);

    /// Returns the current congestion window in bytes.
    fn congestion_window(&self) -> u32;

    /// Returns the current slow-start threshold in bytes.
    fn slow_start_threshold(&self) -> u32;
}

/// TCP Reno congestion control state
#[derive(Debug, Clone)]
pub struct CongestionState {
    /// Congestion window (bytes)
    pub cwnd: u32,
    /// Slow-start threshold (bytes)
    pub ssthresh: u32,
    /// Smoothed RTT estimate (microseconds, fixed-point shifted left by
    /// SRTT_SHIFT)
    rtt_estimate_shifted: u64,
    /// RTT variance (microseconds, fixed-point shifted left by RTTVAR_SHIFT)
    rtt_variance_shifted: u64,
    /// Retransmission timeout (microseconds)
    pub rto: u64,
    /// Duplicate ACK counter
    pub dup_ack_count: u32,
    /// Current congestion phase
    pub phase: CongestionPhase,
    /// Whether we have taken a first RTT sample
    rtt_initialized: bool,
}

impl Default for CongestionState {
    fn default() -> Self {
        Self::new()
    }
}

impl CongestionState {
    /// Create a new congestion control state with default initial values.
    ///
    /// - cwnd starts at 1 MSS (1460 bytes)
    /// - ssthresh starts at u32::MAX (effectively infinite)
    /// - RTO starts at 1 second
    pub fn new() -> Self {
        Self {
            cwnd: MSS,
            ssthresh: u32::MAX,
            rtt_estimate_shifted: 0,
            rtt_variance_shifted: 0,
            rto: RTO_INITIAL_US,
            dup_ack_count: 0,
            phase: CongestionPhase::SlowStart,
            rtt_initialized: false,
        }
    }

    /// Return the smoothed RTT estimate in microseconds (unshifted).
    pub fn srtt_us(&self) -> u64 {
        self.rtt_estimate_shifted >> SRTT_SHIFT
    }

    /// Return the RTT variance in microseconds (unshifted).
    pub fn rttvar_us(&self) -> u64 {
        self.rtt_variance_shifted >> RTTVAR_SHIFT
    }

    /// Update RTT estimates using Jacobson's algorithm (RFC 6298).
    ///
    /// All arithmetic uses integer/fixed-point operations (no floats).
    ///
    /// On the first sample:
    ///   SRTT = R
    ///   RTTVAR = R / 2
    ///
    /// On subsequent samples:
    ///   RTTVAR = (1 - beta) * RTTVAR + beta * |SRTT - R|
    ///          = RTTVAR - RTTVAR/4 + |SRTT - R|/4
    ///   SRTT   = (1 - alpha) * SRTT + alpha * R
    ///          = SRTT - SRTT/8 + R/8
    ///
    /// RTO = SRTT + max(G, 4 * RTTVAR)
    fn update_rtt(&mut self, rtt_us: u64) {
        if rtt_us == 0 {
            return;
        }

        if !self.rtt_initialized {
            // First RTT measurement (RFC 6298 Section 2.2)
            self.rtt_estimate_shifted = rtt_us << SRTT_SHIFT;
            self.rtt_variance_shifted = (rtt_us / 2) << RTTVAR_SHIFT;
            self.rtt_initialized = true;
        } else {
            // Subsequent measurements (RFC 6298 Section 2.3)
            // Work with shifted values for precision

            // |SRTT - R| in unshifted microseconds
            let srtt_unshifted = self.rtt_estimate_shifted >> SRTT_SHIFT;
            let abs_delta = srtt_unshifted.abs_diff(rtt_us);

            // RTTVAR = (1 - beta) * RTTVAR + beta * |SRTT - R|
            // In shifted form: RTTVAR_s = RTTVAR_s - RTTVAR_s/4 + (|delta| <<
            // RTTVAR_SHIFT)/4 = RTTVAR_s - RTTVAR_s/4 + |delta|
            // (since |delta| << RTTVAR_SHIFT >> RTTVAR_SHIFT = |delta|)
            let rttvar_adj = self.rtt_variance_shifted >> RTTVAR_SHIFT;
            self.rtt_variance_shifted = self
                .rtt_variance_shifted
                .saturating_sub(rttvar_adj)
                .saturating_add(abs_delta);

            // SRTT = (1 - alpha) * SRTT + alpha * R
            // In shifted form: SRTT_s = SRTT_s - SRTT_s/8 + (R << SRTT_SHIFT)/8
            // = SRTT_s - SRTT_s/8 + R
            let srtt_adj = self.rtt_estimate_shifted >> SRTT_SHIFT;
            self.rtt_estimate_shifted = self
                .rtt_estimate_shifted
                .saturating_sub(srtt_adj)
                .saturating_add(rtt_us);
        }

        // RTO = SRTT + max(G, 4 * RTTVAR)
        let srtt = self.rtt_estimate_shifted >> SRTT_SHIFT;
        let rttvar = self.rtt_variance_shifted >> RTTVAR_SHIFT;
        let k_rttvar = rttvar.saturating_mul(4);
        let rto = srtt.saturating_add(core::cmp::max(CLOCK_GRANULARITY_US, k_rttvar));

        // Clamp to [RTO_MIN, RTO_MAX]
        self.rto = rto.clamp(RTO_MIN_US, RTO_MAX_US);
    }
}

/// TCP Reno congestion controller
#[derive(Debug, Clone)]
pub struct RenoController {
    state: CongestionState,
}

impl Default for RenoController {
    fn default() -> Self {
        Self::new()
    }
}

impl RenoController {
    /// Create a new Reno congestion controller.
    pub fn new() -> Self {
        Self {
            state: CongestionState::new(),
        }
    }

    /// Access the underlying congestion state.
    pub fn state(&self) -> &CongestionState {
        &self.state
    }

    /// Return the current RTO in microseconds.
    pub fn rto_us(&self) -> u64 {
        self.state.rto
    }

    /// Return the current congestion phase.
    pub fn phase(&self) -> CongestionPhase {
        self.state.phase
    }

    /// Return the duplicate ACK count.
    pub fn dup_ack_count(&self) -> u32 {
        self.state.dup_ack_count
    }

    /// Return the smoothed RTT in microseconds.
    pub fn srtt_us(&self) -> u64 {
        self.state.srtt_us()
    }

    /// Return the RTT variance in microseconds.
    pub fn rttvar_us(&self) -> u64 {
        self.state.rttvar_us()
    }
}

impl CongestionController for RenoController {
    /// Handle acknowledgment of new data.
    ///
    /// In SlowStart: cwnd += MSS per ACK (doubles per RTT)
    /// In CongestionAvoidance: cwnd += MSS * MSS / cwnd per ACK (~MSS per RTT)
    /// In FastRecovery: transition to CongestionAvoidance, cwnd = ssthresh
    /// (deflate)
    fn on_ack(&mut self, bytes_acked: u32, rtt_us: u64) {
        // Update RTT estimate
        self.state.update_rtt(rtt_us);

        // Reset duplicate ACK counter on new ACK
        self.state.dup_ack_count = 0;

        match self.state.phase {
            CongestionPhase::SlowStart => {
                // Exponential growth: add MSS for each ACK
                // (This effectively doubles cwnd per RTT when all segments are acked)
                self.state.cwnd = self.state.cwnd.saturating_add(MSS);

                // Transition to congestion avoidance when cwnd >= ssthresh
                if self.state.cwnd >= self.state.ssthresh {
                    self.state.phase = CongestionPhase::CongestionAvoidance;
                }
            }
            CongestionPhase::CongestionAvoidance => {
                // Linear growth: increment by MSS * MSS / cwnd per ACK
                // This gives approximately MSS bytes growth per RTT.
                // Use u64 intermediate to avoid overflow in MSS * MSS.
                let increment = ((MSS as u64) * (MSS as u64) / (self.state.cwnd as u64)) as u32;
                // Ensure at least 1 byte increase to guarantee progress
                let increment = core::cmp::max(increment, 1);
                self.state.cwnd = self.state.cwnd.saturating_add(increment);
            }
            CongestionPhase::FastRecovery => {
                // New ACK received during fast recovery: deflate cwnd
                self.state.cwnd = self.state.ssthresh;
                self.state.phase = CongestionPhase::CongestionAvoidance;
            }
        }

        let _ = bytes_acked; // Available for future use (e.g., ABC)
    }

    /// Handle a duplicate ACK.
    ///
    /// After 3 duplicate ACKs: enter fast retransmit / fast recovery.
    /// During fast recovery: inflate cwnd by MSS per additional dup ACK.
    fn on_duplicate_ack(&mut self) {
        self.state.dup_ack_count += 1;

        match self.state.phase {
            CongestionPhase::SlowStart | CongestionPhase::CongestionAvoidance => {
                if self.state.dup_ack_count == DUP_ACK_THRESHOLD {
                    // Enter fast retransmit / fast recovery
                    // ssthresh = max(cwnd / 2, 2 * MSS)
                    self.state.ssthresh = core::cmp::max(self.state.cwnd / 2, 2 * MSS);
                    // cwnd = ssthresh + 3 * MSS (account for the 3 dup ACKs)
                    self.state.cwnd = self.state.ssthresh + 3 * MSS;
                    self.state.phase = CongestionPhase::FastRecovery;
                }
            }
            CongestionPhase::FastRecovery => {
                // Each additional dup ACK: inflate cwnd by MSS
                self.state.cwnd = self.state.cwnd.saturating_add(MSS);
            }
        }
    }

    /// Handle a retransmission timeout.
    ///
    /// This is the most severe congestion signal:
    /// - ssthresh = max(cwnd / 2, 2 * MSS)
    /// - cwnd = 1 MSS
    /// - Back to slow start
    /// - Double the RTO (exponential backoff)
    fn on_timeout(&mut self) {
        // ssthresh = max(cwnd / 2, 2 * MSS)
        self.state.ssthresh = core::cmp::max(self.state.cwnd / 2, 2 * MSS);
        // Reset cwnd to 1 MSS
        self.state.cwnd = MSS;
        // Reset dup ACK counter
        self.state.dup_ack_count = 0;
        // Return to slow start
        self.state.phase = CongestionPhase::SlowStart;
        // Exponential backoff: double the RTO, clamped to max
        self.state.rto = core::cmp::min(self.state.rto.saturating_mul(2), RTO_MAX_US);
    }

    fn congestion_window(&self) -> u32 {
        self.state.cwnd
    }

    fn slow_start_threshold(&self) -> u32 {
        self.state.ssthresh
    }
}

// ---------------------------------------------------------------------------
// TCP Cubic Congestion Control (RFC 8312)
// ---------------------------------------------------------------------------

/// Cubic parameter C = 0.4, represented as 410/1024 in fixed-point.
const CUBIC_C_NUM: u64 = 410;
const CUBIC_C_DEN: u64 = 1024;

/// Cubic parameter beta = 0.7, represented as 717/1024.
const CUBIC_BETA_NUM: u64 = 717;
const CUBIC_BETA_DEN: u64 = 1024;

/// One minus beta = 0.3, represented as 307/1024.
const CUBIC_ONE_MINUS_BETA_NUM: u64 = 307;
const CUBIC_ONE_MINUS_BETA_DEN: u64 = 1024;

/// Integer cube root via Newton's method.
///
/// Returns the integer cube root of `n` (i.e., floor(n^(1/3))).
/// Uses pure integer arithmetic -- no floating point.
fn integer_cbrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    if n < 8 {
        return 1;
    }

    // Initial estimate: start from a power-of-two upper bound.
    // bit_length / 3 gives a reasonable starting exponent.
    let bits = 64 - n.leading_zeros() as u64;
    let mut x = 1u64 << bits.div_ceil(3);

    // Newton's iteration: x_{n+1} = (2*x_n + n / x_n^2) / 3
    loop {
        let x2 = x.saturating_mul(x);
        let x_new = if x2 == 0 {
            // Overflow guard
            x >> 1
        } else {
            (2 * x + n / x2) / 3
        };
        if x_new >= x {
            break;
        }
        x = x_new;
    }

    // Newton can overshoot by 1; verify and correct.
    if x.saturating_mul(x).saturating_mul(x) > n {
        x -= 1;
    }
    x
}

/// TCP Cubic congestion controller (RFC 8312).
///
/// Cubic uses a cubic function of elapsed time since the last congestion event
/// to set the congestion window, providing better bandwidth utilization on
/// high-BDP networks than Reno while remaining TCP-friendly on low-BDP paths.
#[derive(Debug, Clone)]
pub struct CubicController {
    /// Underlying congestion state (cwnd, ssthresh, RTT, RTO, phase).
    state: CongestionState,
    /// cwnd at last loss event (in bytes).
    w_max: u32,
    /// Timestamp of last congestion event (microseconds since boot).
    epoch_start: u64,
    /// K value in microseconds (time for cubic to reach w_max).
    k_us: u64,
    /// Origin point (W_max) for the current cubic epoch, in segments.
    origin_point: u32,
    /// TCP-friendly window estimate (bytes).
    tcp_cwnd: u32,
    /// Previous w_max for fast convergence.
    prev_w_max: u32,
    /// Elapsed time accumulator in microseconds (driven by RTT samples).
    elapsed_us: u64,
}

impl Default for CubicController {
    fn default() -> Self {
        Self::new()
    }
}

impl CubicController {
    /// Create a new Cubic congestion controller.
    pub fn new() -> Self {
        Self {
            state: CongestionState::new(),
            w_max: 0,
            epoch_start: 0,
            k_us: 0,
            origin_point: 0,
            tcp_cwnd: 0,
            prev_w_max: 0,
            elapsed_us: 0,
        }
    }

    /// Access the underlying congestion state.
    pub fn state(&self) -> &CongestionState {
        &self.state
    }

    /// Return the current RTO in microseconds.
    pub fn rto_us(&self) -> u64 {
        self.state.rto
    }

    /// Return the current congestion phase.
    pub fn phase(&self) -> CongestionPhase {
        self.state.phase
    }

    /// Return the duplicate ACK count.
    pub fn dup_ack_count(&self) -> u32 {
        self.state.dup_ack_count
    }

    /// Return the smoothed RTT in microseconds.
    pub fn srtt_us(&self) -> u64 {
        self.state.srtt_us()
    }

    /// Compute K (time to reach w_max) in microseconds.
    ///
    /// K = cbrt(W_max * beta / C) in segments, then converted to microseconds
    /// using the current SRTT.
    ///
    /// All math is integer-only with intermediate u64 to prevent overflow.
    fn compute_k(&self, w_max_segs: u64) -> u64 {
        // K_segs_cubed = w_max_segs * beta / C
        //              = w_max_segs * (CUBIC_BETA_NUM / CUBIC_BETA_DEN) / (CUBIC_C_NUM
        // / CUBIC_C_DEN)              = w_max_segs * CUBIC_BETA_NUM *
        // CUBIC_C_DEN / (CUBIC_BETA_DEN * CUBIC_C_NUM)
        let numerator = w_max_segs
            .saturating_mul(CUBIC_BETA_NUM)
            .saturating_mul(CUBIC_C_DEN);
        let denominator = CUBIC_BETA_DEN.saturating_mul(CUBIC_C_NUM);
        let k_cubed = if denominator == 0 {
            0
        } else {
            numerator / denominator
        };
        let k_segs = integer_cbrt(k_cubed);

        // Convert from RTT units to microseconds.
        let srtt = self.state.srtt_us();
        let rtt = if srtt > 0 { srtt } else { 100_000 }; // default 100ms if unknown
        k_segs.saturating_mul(rtt)
    }

    /// Compute the Cubic window W(t) in bytes.
    ///
    /// W(t) = C * (t - K)^3 + W_max
    ///
    /// `t_us` is elapsed time in microseconds since the congestion event.
    fn cubic_window(&self, t_us: u64) -> u32 {
        let rtt = {
            let s = self.state.srtt_us();
            if s > 0 {
                s
            } else {
                100_000
            }
        };

        // Convert t from microseconds to RTT units (fixed-point 10-bit fraction).
        // t_rtt_fp = t_us * 1024 / rtt
        let t_rtt_fp = if rtt > 0 {
            t_us.saturating_mul(1024) / rtt
        } else {
            0
        };
        let k_rtt_fp = if rtt > 0 {
            self.k_us.saturating_mul(1024) / rtt
        } else {
            0
        };

        // (t - K) in fixed-point RTT units, may be negative
        let (diff_fp, negative) = if t_rtt_fp >= k_rtt_fp {
            (t_rtt_fp - k_rtt_fp, false)
        } else {
            (k_rtt_fp - t_rtt_fp, true)
        };

        // (t - K)^3 in fixed-point: cube then un-scale by 1024^3 -> 1024^2
        // We compute diff^3 / 1024^2 to stay in 1024-scaled result.
        // To avoid overflow: diff is at most ~10^7 for 1024-scaled,
        // diff^3 can be huge. Use step-by-step division.
        let diff2 = diff_fp.saturating_mul(diff_fp) / 1024; // scale back one factor
        let diff3 = diff2.saturating_mul(diff_fp) / 1024; // scale back another factor
                                                          // diff3 is now in base (RTT) units (unscaled)

        // W_cubic = C * diff3 + origin_point  (all in segments)
        let c_diff3 = diff3.saturating_mul(CUBIC_C_NUM) / CUBIC_C_DEN;

        let origin_segs = self.origin_point as u64;
        let w_segs = if negative {
            origin_segs.saturating_sub(c_diff3)
        } else {
            origin_segs.saturating_add(c_diff3)
        };

        // Convert segments to bytes, clamp to u32
        let w_bytes = w_segs.saturating_mul(MSS as u64);
        if w_bytes > u32::MAX as u64 {
            u32::MAX
        } else {
            core::cmp::max(w_bytes as u32, MSS)
        }
    }

    /// Compute the TCP-friendly (Reno-equivalent) window in bytes.
    ///
    /// W_tcp = W_max * (1 - beta) + 3 * beta / (2 - beta) * t / RTT
    fn tcp_friendly_window(&self, t_us: u64) -> u32 {
        let rtt = {
            let s = self.state.srtt_us();
            if s > 0 {
                s
            } else {
                100_000
            }
        };
        let w_max_segs = self.origin_point as u64;

        // Base: W_max * (1 - beta) in segments
        let base = w_max_segs.saturating_mul(CUBIC_ONE_MINUS_BETA_NUM) / CUBIC_ONE_MINUS_BETA_DEN;

        // Slope: 3 * beta / (2 - beta) per RTT
        // = 3 * 717 / (2 * 1024 - 717) = 2151 / 1331
        let slope_num: u64 = 3 * CUBIC_BETA_NUM; // 2151
        let slope_den: u64 = 2 * CUBIC_BETA_DEN - CUBIC_BETA_NUM; // 1331

        // t in RTT units
        let t_rtts = if rtt > 0 { t_us / rtt } else { 0 };

        let increment = t_rtts.saturating_mul(slope_num) / slope_den;

        let w_segs = base.saturating_add(increment);
        let w_bytes = w_segs.saturating_mul(MSS as u64);
        if w_bytes > u32::MAX as u64 {
            u32::MAX
        } else {
            core::cmp::max(w_bytes as u32, MSS)
        }
    }

    /// Start a new cubic epoch after a loss event.
    fn start_epoch(&mut self) {
        let w_max_segs = (self.w_max as u64) / (MSS as u64);
        self.origin_point = w_max_segs as u32;
        self.k_us = self.compute_k(w_max_segs);
        self.elapsed_us = 0;
        self.tcp_cwnd = self.state.cwnd;
    }
}

impl CongestionController for CubicController {
    fn on_ack(&mut self, bytes_acked: u32, rtt_us: u64) {
        self.state.update_rtt(rtt_us);
        self.state.dup_ack_count = 0;

        match self.state.phase {
            CongestionPhase::SlowStart => {
                self.state.cwnd = self.state.cwnd.saturating_add(MSS);
                if self.state.cwnd >= self.state.ssthresh {
                    self.state.phase = CongestionPhase::CongestionAvoidance;
                    // Begin cubic epoch when entering CA
                    if self.w_max == 0 {
                        self.w_max = self.state.cwnd;
                    }
                    self.start_epoch();
                }
            }
            CongestionPhase::CongestionAvoidance => {
                // Advance elapsed time by one RTT sample
                let rtt_sample = if rtt_us > 0 {
                    rtt_us
                } else {
                    self.state.srtt_us()
                };
                if rtt_sample > 0 {
                    self.elapsed_us = self.elapsed_us.saturating_add(rtt_sample);
                }

                // Compute cubic window and TCP-friendly window
                let w_cubic = self.cubic_window(self.elapsed_us);
                let w_tcp = self.tcp_friendly_window(self.elapsed_us);

                // Use the larger of cubic and TCP-friendly (ensures fairness)
                let target = core::cmp::max(w_cubic, w_tcp);

                // Increase toward target: add MSS * MSS / cwnd per ACK (bounded)
                if target > self.state.cwnd {
                    let delta = target - self.state.cwnd;
                    let increment = ((MSS as u64) * (MSS as u64) / (self.state.cwnd as u64)) as u32;
                    let increment = core::cmp::min(core::cmp::max(increment, 1), delta);
                    self.state.cwnd = self.state.cwnd.saturating_add(increment);
                } else {
                    // Cubic says reduce, but don't decrease below current -
                    // just hold (Cubic only decreases on
                    // loss, not proactively)
                }

                self.tcp_cwnd = w_tcp;
            }
            CongestionPhase::FastRecovery => {
                self.state.cwnd = self.state.ssthresh;
                self.state.phase = CongestionPhase::CongestionAvoidance;
                self.start_epoch();
            }
        }

        let _ = bytes_acked;
    }

    fn on_duplicate_ack(&mut self) {
        self.state.dup_ack_count += 1;

        match self.state.phase {
            CongestionPhase::SlowStart | CongestionPhase::CongestionAvoidance => {
                if self.state.dup_ack_count == DUP_ACK_THRESHOLD {
                    // Save w_max before reduction
                    let current_cwnd = self.state.cwnd;

                    // Fast convergence: if new w_max < previous w_max,
                    // reduce w_max further to converge faster.
                    if current_cwnd < self.prev_w_max {
                        // w_max = cwnd * (1 + beta) / 2 = cwnd * 1717 / 2048
                        self.w_max = ((current_cwnd as u64) * (CUBIC_BETA_DEN + CUBIC_BETA_NUM)
                            / (2 * CUBIC_BETA_DEN)) as u32;
                    } else {
                        self.w_max = current_cwnd;
                    }
                    self.prev_w_max = current_cwnd;

                    // Multiplicative decrease: cwnd = cwnd * beta
                    // ssthresh = cwnd * beta = cwnd * 717 / 1024
                    let new_cwnd = ((current_cwnd as u64) * CUBIC_BETA_NUM / CUBIC_BETA_DEN) as u32;
                    self.state.ssthresh = core::cmp::max(new_cwnd, 2 * MSS);
                    self.state.cwnd = self.state.ssthresh + 3 * MSS;
                    self.state.phase = CongestionPhase::FastRecovery;

                    self.start_epoch();
                }
            }
            CongestionPhase::FastRecovery => {
                self.state.cwnd = self.state.cwnd.saturating_add(MSS);
            }
        }
    }

    fn on_timeout(&mut self) {
        // Save w_max
        self.prev_w_max = self.w_max;
        self.w_max = self.state.cwnd;

        // ssthresh = cwnd * beta
        let new_ssthresh = ((self.state.cwnd as u64) * CUBIC_BETA_NUM / CUBIC_BETA_DEN) as u32;
        self.state.ssthresh = core::cmp::max(new_ssthresh, 2 * MSS);
        self.state.cwnd = MSS;
        self.state.dup_ack_count = 0;
        self.state.phase = CongestionPhase::SlowStart;
        self.state.rto = core::cmp::min(self.state.rto.saturating_mul(2), RTO_MAX_US);

        // Reset epoch
        self.epoch_start = 0;
        self.elapsed_us = 0;
    }

    fn congestion_window(&self) -> u32 {
        self.state.cwnd
    }

    fn slow_start_threshold(&self) -> u32 {
        self.state.ssthresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let cc = RenoController::new();
        assert_eq!(cc.congestion_window(), MSS);
        assert_eq!(cc.slow_start_threshold(), u32::MAX);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
        assert_eq!(cc.dup_ack_count(), 0);
        assert_eq!(cc.rto_us(), RTO_INITIAL_US);
    }

    #[test]
    fn test_slow_start_growth() {
        let mut cc = RenoController::new();
        // In slow start, each ACK should increase cwnd by MSS
        cc.on_ack(MSS, 50_000); // 50ms RTT
        assert_eq!(cc.congestion_window(), 2 * MSS);
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.congestion_window(), 3 * MSS);
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.congestion_window(), 4 * MSS);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
    }

    #[test]
    fn test_slow_start_to_congestion_avoidance() {
        let mut cc = RenoController::new();
        // Set a low ssthresh to trigger transition
        cc.state.ssthresh = 3 * MSS;
        cc.on_ack(MSS, 50_000); // cwnd = 2 * MSS
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
        cc.on_ack(MSS, 50_000); // cwnd = 3 * MSS >= ssthresh
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
    }

    #[test]
    fn test_congestion_avoidance_linear_growth() {
        let mut cc = RenoController::new();
        // Force into congestion avoidance with cwnd = 4 * MSS
        cc.state.cwnd = 4 * MSS;
        cc.state.ssthresh = 4 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        let initial_cwnd = cc.congestion_window();
        // Each ACK should add approximately MSS * MSS / cwnd bytes
        cc.on_ack(MSS, 50_000);
        let increment = cc.congestion_window() - initial_cwnd;

        // Expected increment: MSS * MSS / (4 * MSS) = MSS / 4 = 365
        let expected = MSS / 4;
        assert_eq!(increment, expected);
    }

    #[test]
    fn test_congestion_avoidance_minimum_increment() {
        let mut cc = RenoController::new();
        // Very large cwnd to test minimum increment
        cc.state.cwnd = u32::MAX / 2;
        cc.state.ssthresh = MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        let initial = cc.congestion_window();
        cc.on_ack(MSS, 50_000);
        // Should increase by at least 1 byte
        assert!(cc.congestion_window() > initial);
    }

    #[test]
    fn test_fast_retransmit_on_3_dup_acks() {
        let mut cc = RenoController::new();
        cc.state.cwnd = 10 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        let original_cwnd = cc.congestion_window();

        // 3 duplicate ACKs trigger fast retransmit / recovery
        cc.on_duplicate_ack();
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
        cc.on_duplicate_ack();
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
        cc.on_duplicate_ack();
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);

        // ssthresh = max(cwnd / 2, 2 * MSS)
        let expected_ssthresh = core::cmp::max(original_cwnd / 2, 2 * MSS);
        assert_eq!(cc.slow_start_threshold(), expected_ssthresh);

        // cwnd = ssthresh + 3 * MSS
        assert_eq!(cc.congestion_window(), expected_ssthresh + 3 * MSS);
    }

    #[test]
    fn test_fast_recovery_inflation() {
        let mut cc = RenoController::new();
        cc.state.cwnd = 10 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        // Trigger fast recovery
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);
        let cwnd_after_fr = cc.congestion_window();

        // Additional dup ACKs inflate cwnd by MSS each
        cc.on_duplicate_ack();
        assert_eq!(cc.congestion_window(), cwnd_after_fr + MSS);
        cc.on_duplicate_ack();
        assert_eq!(cc.congestion_window(), cwnd_after_fr + 2 * MSS);
    }

    #[test]
    fn test_fast_recovery_exit_on_new_ack() {
        let mut cc = RenoController::new();
        cc.state.cwnd = 10 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        // Enter fast recovery
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        let ssthresh = cc.slow_start_threshold();

        // New ACK should deflate cwnd to ssthresh and enter congestion avoidance
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.congestion_window(), ssthresh);
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
        assert_eq!(cc.dup_ack_count(), 0);
    }

    #[test]
    fn test_timeout_resets_to_slow_start() {
        let mut cc = RenoController::new();
        cc.state.cwnd = 20 * MSS;
        cc.state.ssthresh = 15 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        let cwnd_before = cc.congestion_window();
        cc.on_timeout();

        // cwnd should reset to 1 MSS
        assert_eq!(cc.congestion_window(), MSS);
        // ssthresh = max(cwnd / 2, 2 * MSS)
        assert_eq!(
            cc.slow_start_threshold(),
            core::cmp::max(cwnd_before / 2, 2 * MSS)
        );
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
    }

    #[test]
    fn test_timeout_doubles_rto() {
        let mut cc = RenoController::new();
        let rto_initial = cc.rto_us();

        cc.on_timeout();
        assert_eq!(cc.rto_us(), rto_initial * 2);

        cc.on_timeout();
        assert_eq!(cc.rto_us(), rto_initial * 4);
    }

    #[test]
    fn test_rto_clamped_to_max() {
        let mut cc = RenoController::new();
        // Force RTO near max
        cc.state.rto = RTO_MAX_US;
        cc.on_timeout();
        assert_eq!(cc.rto_us(), RTO_MAX_US);
    }

    #[test]
    fn test_rto_first_rtt_sample() {
        let mut cc = RenoController::new();
        let rtt = 100_000u64; // 100ms

        cc.on_ack(MSS, rtt);

        // After first sample: SRTT = R, RTTVAR = R/2
        assert_eq!(cc.srtt_us(), rtt);
        assert_eq!(cc.rttvar_us(), rtt / 2);

        // RTO = SRTT + max(G, 4 * RTTVAR) = 100ms + max(1ms, 4 * 50ms) = 100ms + 200ms
        // = 300ms
        let expected_rto = rtt + 4 * (rtt / 2);
        // Clamp to at least RTO_MIN
        let expected_rto = expected_rto.clamp(RTO_MIN_US, RTO_MAX_US);
        assert_eq!(cc.rto_us(), expected_rto);
    }

    #[test]
    fn test_rto_subsequent_rtt_sample() {
        let mut cc = RenoController::new();

        // First sample: 100ms
        cc.on_ack(MSS, 100_000);
        let srtt_after_first = cc.srtt_us();
        assert_eq!(srtt_after_first, 100_000);

        // Second sample: 120ms
        cc.on_ack(MSS, 120_000);
        let srtt_after_second = cc.srtt_us();

        // SRTT should move toward 120ms but not reach it
        // SRTT = SRTT - SRTT/8 + R/8 = 100000 - 12500 + 15000 = 102500
        assert_eq!(srtt_after_second, 102_500);
    }

    #[test]
    fn test_rto_minimum_enforced() {
        let mut cc = RenoController::new();
        // Very small RTT should still result in RTO >= RTO_MIN
        cc.on_ack(MSS, 100); // 0.1ms
        assert!(cc.rto_us() >= RTO_MIN_US);
    }

    #[test]
    fn test_zero_rtt_ignored() {
        let mut cc = RenoController::new();
        let rto_before = cc.rto_us();
        cc.on_ack(MSS, 0);
        // RTT of 0 should not update RTT estimates; RTO stays at initial
        assert_eq!(cc.rto_us(), rto_before);
    }

    #[test]
    fn test_ssthresh_minimum_2mss() {
        let mut cc = RenoController::new();
        // Very small cwnd: cwnd / 2 < 2 * MSS
        cc.state.cwnd = MSS;
        cc.on_timeout();
        assert_eq!(cc.slow_start_threshold(), 2 * MSS);
    }

    #[test]
    fn test_dup_ack_count_reset_on_new_ack() {
        let mut cc = RenoController::new();
        cc.on_duplicate_ack();
        cc.on_duplicate_ack();
        assert_eq!(cc.dup_ack_count(), 2);

        // New ACK resets dup count
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.dup_ack_count(), 0);
    }

    #[test]
    fn test_full_congestion_cycle() {
        let mut cc = RenoController::new();

        // Phase 1: Slow start from 1 MSS
        for _ in 0..5 {
            cc.on_ack(MSS, 50_000);
        }
        assert_eq!(cc.congestion_window(), 6 * MSS);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);

        // Phase 2: Timeout -- reset to slow start
        cc.on_timeout();
        assert_eq!(cc.congestion_window(), MSS);
        assert_eq!(
            cc.slow_start_threshold(),
            core::cmp::max(6 * MSS / 2, 2 * MSS)
        );
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);

        // Phase 3: Grow back past ssthresh into congestion avoidance
        let ssthresh = cc.slow_start_threshold();
        while cc.phase() == CongestionPhase::SlowStart {
            cc.on_ack(MSS, 50_000);
        }
        assert!(cc.congestion_window() >= ssthresh);
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);

        // Phase 4: 3 dup ACKs -> fast recovery
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);

        // Phase 5: New ACK exits fast recovery
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
    }

    // -----------------------------------------------------------------------
    // Cubic controller tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_integer_cbrt_exact_cubes() {
        assert_eq!(integer_cbrt(0), 0);
        assert_eq!(integer_cbrt(1), 1);
        assert_eq!(integer_cbrt(8), 2);
        assert_eq!(integer_cbrt(27), 3);
        assert_eq!(integer_cbrt(64), 4);
        assert_eq!(integer_cbrt(125), 5);
        assert_eq!(integer_cbrt(1000), 10);
        assert_eq!(integer_cbrt(1_000_000), 100);
        assert_eq!(integer_cbrt(1_000_000_000), 1000);
    }

    #[test]
    fn test_integer_cbrt_non_exact() {
        // floor of cube root
        assert_eq!(integer_cbrt(2), 1);
        assert_eq!(integer_cbrt(7), 1);
        assert_eq!(integer_cbrt(9), 2);
        assert_eq!(integer_cbrt(26), 2);
        assert_eq!(integer_cbrt(63), 3);
        assert_eq!(integer_cbrt(100), 4);
        // Verify: 4^3 = 64 <= 100 < 125 = 5^3
    }

    #[test]
    fn test_integer_cbrt_large_values() {
        // Typical cwnd-related values
        let val = 1_000_000_000_000u64; // 10^12
        let root = integer_cbrt(val);
        assert_eq!(root, 10_000); // 10000^3 = 10^12

        // Maximum-ish value
        let root = integer_cbrt(u64::MAX);
        // 2642245^3 ≈ 1.844 * 10^19 ≈ u64::MAX
        assert!(root >= 2_642_245);
        // Verify root^3 <= u64::MAX (doesn't overflow)
        assert!(root
            .checked_mul(root)
            .and_then(|r2| r2.checked_mul(root))
            .is_some());
        // Verify (root+1)^3 overflows u64 (proves root is the floor cbrt)
        let r1 = root + 1;
        assert!(r1
            .checked_mul(r1)
            .and_then(|r2| r2.checked_mul(r1))
            .is_none());
    }

    #[test]
    fn test_cubic_initial_state() {
        let cc = CubicController::new();
        assert_eq!(cc.congestion_window(), MSS);
        assert_eq!(cc.slow_start_threshold(), u32::MAX);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
        assert_eq!(cc.w_max, 0);
    }

    #[test]
    fn test_cubic_slow_start_identical_to_reno() {
        let mut cubic = CubicController::new();
        let mut reno = RenoController::new();

        // Both should grow identically during slow start
        for _ in 0..5 {
            cubic.on_ack(MSS, 50_000);
            reno.on_ack(MSS, 50_000);
        }
        assert_eq!(cubic.congestion_window(), reno.congestion_window());
        assert_eq!(cubic.phase(), CongestionPhase::SlowStart);
    }

    #[test]
    fn test_cubic_loss_response_beta_07() {
        let mut cc = CubicController::new();
        // Set up: large cwnd in congestion avoidance
        cc.state.cwnd = 100 * MSS;
        cc.state.ssthresh = 50 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;
        cc.w_max = 100 * MSS;

        let cwnd_before = cc.congestion_window();

        // 3 dup ACKs trigger loss
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }

        // ssthresh should be cwnd * 0.7 (beta)
        let expected_ssthresh = ((cwnd_before as u64) * CUBIC_BETA_NUM / CUBIC_BETA_DEN) as u32;
        assert_eq!(cc.slow_start_threshold(), expected_ssthresh);

        // w_max should record the pre-loss cwnd
        assert_eq!(cc.w_max, cwnd_before);
    }

    #[test]
    fn test_cubic_fast_convergence() {
        let mut cc = CubicController::new();
        cc.state.cwnd = 100 * MSS;
        cc.state.ssthresh = 50 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;
        cc.prev_w_max = 120 * MSS; // Previous w_max was higher

        let cwnd_before = cc.congestion_window();

        // Trigger loss: cwnd (100) < prev_w_max (120), so fast convergence applies
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }

        // w_max should be reduced: cwnd * (1 + beta) / 2 = 100 * 1.7 / 2 = 85 segments
        let expected_w_max = ((cwnd_before as u64) * (CUBIC_BETA_DEN + CUBIC_BETA_NUM)
            / (2 * CUBIC_BETA_DEN)) as u32;
        assert_eq!(cc.w_max, expected_w_max);
        // Fast convergence w_max should be less than normal w_max
        assert!(cc.w_max < cwnd_before);
    }

    #[test]
    fn test_cubic_timeout_resets() {
        let mut cc = CubicController::new();
        cc.state.cwnd = 50 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        cc.on_timeout();

        assert_eq!(cc.congestion_window(), MSS);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
        // ssthresh = cwnd * beta = 50 * 717 / 1024 ~= 35 segments
        let expected = ((50 * MSS as u64) * CUBIC_BETA_NUM / CUBIC_BETA_DEN) as u32;
        assert_eq!(cc.slow_start_threshold(), expected);
    }

    #[test]
    fn test_cubic_growth_after_loss() {
        let mut cc = CubicController::new();
        // Initialize RTT
        cc.state.cwnd = 100 * MSS;
        cc.state.ssthresh = 50 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        // Prime RTT estimate
        cc.state.update_rtt(50_000); // 50ms

        // Trigger loss
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);

        // Exit fast recovery with new ACK
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);

        let cwnd_after_loss = cc.congestion_window();

        // Continue with ACKs -- cwnd should grow
        for _ in 0..20 {
            cc.on_ack(MSS, 50_000);
        }
        assert!(
            cc.congestion_window() > cwnd_after_loss,
            "cwnd should grow after loss: {} vs {}",
            cc.congestion_window(),
            cwnd_after_loss
        );
    }

    #[test]
    fn test_cubic_tcp_friendly_region() {
        // In the TCP-friendly region (early after loss with small elapsed time),
        // Cubic should behave at least as well as Reno.
        let mut cc = CubicController::new();
        cc.state.update_rtt(50_000);
        cc.state.cwnd = 10 * MSS;
        cc.state.ssthresh = 10 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;
        cc.w_max = 10 * MSS;
        cc.origin_point = 10;
        cc.k_us = cc.compute_k(10);
        cc.elapsed_us = 0;

        // TCP-friendly window should be at least 1 MSS
        let w_tcp = cc.tcp_friendly_window(0);
        assert!(w_tcp >= MSS);

        // After some time, TCP-friendly should grow
        let w_tcp_later = cc.tcp_friendly_window(500_000); // 500ms
        assert!(w_tcp_later >= w_tcp);
    }

    #[test]
    fn test_cubic_window_concave_then_convex() {
        // The cubic function should be concave (below w_max) before K
        // and convex (above w_max) after K.
        let mut cc = CubicController::new();
        cc.state.update_rtt(50_000);
        cc.w_max = 100 * MSS;
        cc.origin_point = 100;
        cc.k_us = cc.compute_k(100);

        // Well before K: window should be below w_max
        if cc.k_us > 200_000 {
            let w_early = cc.cubic_window(cc.k_us / 4);
            assert!(
                w_early < cc.w_max,
                "early window {} should be below w_max {}",
                w_early,
                cc.w_max
            );
        }

        // At K: window should be approximately w_max
        let w_at_k = cc.cubic_window(cc.k_us);
        let tolerance = 5 * MSS; // Allow some fixed-point rounding
        let diff = if w_at_k > cc.w_max {
            w_at_k - cc.w_max
        } else {
            cc.w_max - w_at_k
        };
        assert!(
            diff <= tolerance,
            "window at K ({}) should be close to w_max ({}), diff={}",
            w_at_k,
            cc.w_max,
            diff
        );

        // Well after K: window should exceed w_max
        let w_late = cc.cubic_window(cc.k_us * 3);
        assert!(
            w_late > cc.w_max,
            "late window {} should exceed w_max {}",
            w_late,
            cc.w_max
        );
    }

    #[test]
    fn test_cubic_full_congestion_cycle() {
        let mut cc = CubicController::new();

        // Phase 1: Slow start
        for _ in 0..5 {
            cc.on_ack(MSS, 50_000);
        }
        assert_eq!(cc.congestion_window(), 6 * MSS);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);

        // Phase 2: Timeout
        cc.on_timeout();
        assert_eq!(cc.congestion_window(), MSS);
        assert_eq!(cc.phase(), CongestionPhase::SlowStart);
        assert!(cc.w_max > 0);

        // Phase 3: Grow through slow start into CA
        while cc.phase() == CongestionPhase::SlowStart {
            cc.on_ack(MSS, 50_000);
        }
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);

        // Phase 4: Grow in CA
        let cwnd_at_ca = cc.congestion_window();
        for _ in 0..10 {
            cc.on_ack(MSS, 50_000);
        }
        assert!(cc.congestion_window() >= cwnd_at_ca);

        // Phase 5: 3 dup ACKs -> fast recovery
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);

        // Phase 6: Exit fast recovery
        cc.on_ack(MSS, 50_000);
        assert_eq!(cc.phase(), CongestionPhase::CongestionAvoidance);
    }

    #[test]
    fn test_cubic_recovery_inflation() {
        let mut cc = CubicController::new();
        cc.state.cwnd = 20 * MSS;
        cc.state.phase = CongestionPhase::CongestionAvoidance;

        // Trigger fast recovery
        for _ in 0..3 {
            cc.on_duplicate_ack();
        }
        assert_eq!(cc.phase(), CongestionPhase::FastRecovery);
        let cwnd_fr = cc.congestion_window();

        // Additional dup ACKs should inflate cwnd by MSS
        cc.on_duplicate_ack();
        assert_eq!(cc.congestion_window(), cwnd_fr + MSS);
        cc.on_duplicate_ack();
        assert_eq!(cc.congestion_window(), cwnd_fr + 2 * MSS);
    }
}
