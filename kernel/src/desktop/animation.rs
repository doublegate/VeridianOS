//! Animation Framework
//!
//! Provides transition timers and easing functions for smooth window
//! animations (opacity changes, position moves, resize transitions).
//! All math is integer-only (fixed-point 8.8) to avoid floating-point
//! operations in the kernel.

#![allow(dead_code)]

use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Easing functions
// ---------------------------------------------------------------------------

/// Easing function type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EasingFunction {
    Linear,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    /// Slight overshoot at the end
    EaseOutBack,
    /// Bounce at the end
    EaseOutBounce,
}

/// Evaluate an easing function with fixed-point arithmetic (no floating point).
///
/// `t_256` is the progress from 0 to 256 (representing 0.0 to 1.0).
/// Returns a value in approximately 0..256 (may slightly exceed 256 for
/// overshoot easings like `EaseOutBack`).
pub fn evaluate_easing_fixed(easing: EasingFunction, t_256: u32) -> u32 {
    // Clamp input
    let t = t_256.min(256);

    match easing {
        EasingFunction::Linear => t,

        EasingFunction::EaseInQuad => {
            // t^2
            (t * t) >> 8
        }

        EasingFunction::EaseOutQuad => {
            // 1 - (1-t)^2
            let inv = 256 - t;
            256 - ((inv * inv) >> 8)
        }

        EasingFunction::EaseInOutQuad => {
            // Piecewise: 2t^2 for first half, 1-(-2t+2)^2/2 for second half
            if t < 128 {
                // 2 * (t/256)^2 * 256 = 2*t^2/256
                (2 * t * t) >> 8
            } else {
                let inv = 256 - t;
                256 - ((2 * inv * inv) >> 8)
            }
        }

        EasingFunction::EaseInCubic => {
            // t^3 in fixed point: (t * t * t) >> 16
            let t2 = (t * t) >> 8;
            (t2 * t) >> 8
        }

        EasingFunction::EaseOutCubic => {
            // 1 - (1-t)^3
            let inv = 256 - t;
            let inv2 = (inv * inv) >> 8;
            let inv3 = (inv2 * inv) >> 8;
            256 - inv3
        }

        EasingFunction::EaseInOutCubic => {
            if t < 128 {
                // 4 * t^3 in fixed point
                let t2 = (t * t) >> 8;
                let t3 = (t2 * t) >> 8;
                (4 * t3) >> 8
            } else {
                let inv = 256 - t;
                let inv2 = (inv * inv) >> 8;
                let inv3 = (inv2 * inv) >> 8;
                256 - ((4 * inv3) >> 8)
            }
        }

        EasingFunction::EaseOutBack => {
            // Overshoot: goes slightly past 256 then settles.
            // Approximation: EaseOutQuad + 10% overshoot bump.
            let inv = 256 - t;
            let base = 256 - ((inv * inv) >> 8);
            // Add a small overshoot that peaks at t~192 (3/4)
            // overshoot = sin-like bump approximated as parabola
            let mid_dist = if t > 128 { 256 - t } else { t };
            let bump = (mid_dist * 20) >> 8; // ~8% overshoot
                                             // Only apply bump in second half
            if t > 128 {
                base + bump
            } else {
                base
            }
        }

        EasingFunction::EaseOutBounce => {
            // Simplified bounce using 3 segments.
            // Each segment is a parabola opening downward.
            if t < 192 {
                // Main arc: covers 0..75% of time, reaches ~100% height
                // Parabola: 4*(t/192)*(1 - t/192) scaled to 256
                let t_seg = (t * 256) / 192;
                let inv = 256 - t_seg;
                let val = (4 * t_seg * inv) >> 8;
                // Scale to reach 256 at peak
                (val * 256) >> 8
            } else if t < 240 {
                // First bounce: smaller arc
                let seg_start = 192;
                let seg_len = 48;
                let t_seg = ((t - seg_start) * 256) / seg_len;
                let inv = 256 - t_seg;
                let bounce_h = 32; // bounce height ~12.5%
                let base = 256 - bounce_h;
                let val = (4 * t_seg * inv) >> 8;
                base + ((val * bounce_h) >> 8)
            } else {
                // Final settle: tiny bounce
                let seg_start = 240;
                let seg_len = 16;
                let t_local = t - seg_start;
                let t_seg = if seg_len > 0 {
                    (t_local * 256) / seg_len
                } else {
                    256
                };
                let inv = 256 - t_seg;
                let bounce_h = 8; // tiny bounce
                let base = 256 - bounce_h;
                let val = (4 * t_seg * inv) >> 8;
                base + ((val * bounce_h) >> 8)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Animation properties
// ---------------------------------------------------------------------------

/// Animation target property
#[derive(Debug, Clone, Copy)]
pub enum AnimationProperty {
    Opacity {
        from: u8,
        to: u8,
    },
    PositionX {
        from: i32,
        to: i32,
    },
    PositionY {
        from: i32,
        to: i32,
    },
    Width {
        from: u32,
        to: u32,
    },
    Height {
        from: u32,
        to: u32,
    },
    /// Fixed-point 8.8 scale factor (256 = 1.0x)
    Scale {
        from_256: u32,
        to_256: u32,
    },
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

/// Active animation instance.
pub struct Animation {
    pub id: u32,
    pub window_id: u32,
    pub property: AnimationProperty,
    pub easing: EasingFunction,
    /// Total duration in milliseconds
    pub duration_ms: u32,
    /// Elapsed time in milliseconds
    pub elapsed_ms: u32,
    /// Whether this animation has finished
    pub completed: bool,
}

impl Animation {
    /// Compute the current interpolated value of this animation.
    ///
    /// Returns the value as an i64 (covers both signed and unsigned
    /// properties).
    pub fn current_value(&self) -> i64 {
        if self.completed || self.duration_ms == 0 {
            return self.target_value();
        }

        // Progress 0..256
        let t_256 = ((self.elapsed_ms as u64 * 256) / self.duration_ms as u64).min(256) as u32;
        let eased = evaluate_easing_fixed(self.easing, t_256);

        // Interpolate: from + (to - from) * eased / 256
        match self.property {
            AnimationProperty::Opacity { from, to } => {
                let from_i = from as i64;
                let to_i = to as i64;
                from_i + ((to_i - from_i) * eased as i64) / 256
            }
            AnimationProperty::PositionX { from, to }
            | AnimationProperty::PositionY { from, to } => {
                let from_i = from as i64;
                let to_i = to as i64;
                from_i + ((to_i - from_i) * eased as i64) / 256
            }
            AnimationProperty::Width { from, to } | AnimationProperty::Height { from, to } => {
                let from_i = from as i64;
                let to_i = to as i64;
                from_i + ((to_i - from_i) * eased as i64) / 256
            }
            AnimationProperty::Scale { from_256, to_256 } => {
                let from_i = from_256 as i64;
                let to_i = to_256 as i64;
                from_i + ((to_i - from_i) * eased as i64) / 256
            }
        }
    }

    /// Get the final target value.
    fn target_value(&self) -> i64 {
        match self.property {
            AnimationProperty::Opacity { to, .. } => to as i64,
            AnimationProperty::PositionX { to, .. } | AnimationProperty::PositionY { to, .. } => {
                to as i64
            }
            AnimationProperty::Width { to, .. } | AnimationProperty::Height { to, .. } => to as i64,
            AnimationProperty::Scale { to_256, .. } => to_256 as i64,
        }
    }
}

// ---------------------------------------------------------------------------
// AnimationManager
// ---------------------------------------------------------------------------

/// Manages all active animations and advances them each frame.
pub struct AnimationManager {
    animations: Vec<Animation>,
    next_id: u32,
}

impl AnimationManager {
    /// Create a new animation manager.
    pub fn new() -> Self {
        Self {
            animations: Vec::new(),
            next_id: 1,
        }
    }

    /// Start a new animation. Returns the animation ID.
    pub fn start(
        &mut self,
        window_id: u32,
        property: AnimationProperty,
        easing: EasingFunction,
        duration_ms: u32,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        self.animations.push(Animation {
            id,
            window_id,
            property,
            easing,
            duration_ms,
            elapsed_ms: 0,
            completed: false,
        });

        id
    }

    /// Advance all animations by `delta_ms` milliseconds.
    pub fn tick(&mut self, delta_ms: u32) {
        for anim in &mut self.animations {
            if anim.completed {
                continue;
            }
            anim.elapsed_ms += delta_ms;
            if anim.elapsed_ms >= anim.duration_ms {
                anim.elapsed_ms = anim.duration_ms;
                anim.completed = true;
            }
        }
    }

    /// Get the current interpolated value for an animation by ID.
    pub fn get_current_value(&self, animation_id: u32) -> Option<i64> {
        self.animations
            .iter()
            .find(|a| a.id == animation_id)
            .map(|a| a.current_value())
    }

    /// Check whether an animation has completed.
    pub fn is_complete(&self, animation_id: u32) -> bool {
        self.animations
            .iter()
            .find(|a| a.id == animation_id)
            .is_none_or(|a| a.completed)
    }

    /// Remove all completed animations.
    pub fn remove_completed(&mut self) {
        self.animations.retain(|a| !a.completed);
    }

    /// Cancel all animations for a specific window.
    pub fn cancel(&mut self, window_id: u32) {
        self.animations.retain(|a| a.window_id != window_id);
    }

    /// Returns `true` if there are any active (non-completed) animations.
    pub fn has_active_animations(&self) -> bool {
        self.animations.iter().any(|a| !a.completed)
    }

    /// Get the number of active animations.
    pub fn active_count(&self) -> usize {
        self.animations.iter().filter(|a| !a.completed).count()
    }

    /// Get all animations for a specific window.
    pub fn get_window_animations(&self, window_id: u32) -> Vec<&Animation> {
        self.animations
            .iter()
            .filter(|a| a.window_id == window_id && !a.completed)
            .collect()
    }
}

impl Default for AnimationManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Compositing effects helper
// ---------------------------------------------------------------------------

/// Render a soft box shadow behind a window.
///
/// The shadow is drawn into `shadow_buffer`, which should be pre-allocated
/// with `(width + 2 * radius) * (height + 2 * radius)` u32 elements.
/// The shadow is centered, so the window content starts at offset
/// `(radius, radius)` within the shadow buffer.
///
/// Uses a 3-pass box blur approximating a Gaussian shadow.
///
/// `buf_width` is the stride of `shadow_buffer` in pixels.
pub fn render_shadow(
    shadow_buffer: &mut [u32],
    buf_width: u32,
    width: u32,
    height: u32,
    radius: u32,
    opacity: u8,
) {
    let bw = buf_width as usize;
    let total_w = width + 2 * radius;
    let total_h = height + 2 * radius;
    let r = radius as usize;
    let w = width as usize;
    let h = height as usize;

    // Clear to transparent
    for px in shadow_buffer.iter_mut().take(bw * total_h as usize) {
        *px = 0;
    }

    // Seed: fill the window footprint area with full opacity
    for y in 0..h {
        for x in 0..w {
            let px = r + x;
            let py = r + y;
            let idx = py * bw + px;
            if idx < shadow_buffer.len() {
                shadow_buffer[idx] = opacity as u32;
            }
        }
    }

    // 3-pass box blur (horizontal + vertical + horizontal) approximating Gaussian
    if radius == 0 {
        // Convert alpha values to shadow pixels
        convert_alpha_to_shadow(shadow_buffer, bw, total_w as usize, total_h as usize);
        return;
    }

    // Temporary buffer for blur passes (stores alpha channel only as u32)
    let buf_len = bw * total_h as usize;
    let mut temp = alloc::vec![0u32; buf_len];

    // Pass 1: horizontal blur
    box_blur_h(
        shadow_buffer,
        &mut temp,
        bw,
        total_w as usize,
        total_h as usize,
        r,
    );

    // Pass 2: vertical blur
    box_blur_v(
        &temp,
        shadow_buffer,
        bw,
        total_w as usize,
        total_h as usize,
        r,
    );

    // Pass 3: horizontal blur again
    let shadow_copy: Vec<u32> = shadow_buffer[..buf_len].to_vec();
    box_blur_h(
        &shadow_copy,
        shadow_buffer,
        bw,
        total_w as usize,
        total_h as usize,
        r,
    );

    // Convert alpha values to ARGB shadow color
    convert_alpha_to_shadow(shadow_buffer, bw, total_w as usize, total_h as usize);
}

/// Horizontal box blur pass.
fn box_blur_h(src: &[u32], dst: &mut [u32], bw: usize, w: usize, h: usize, radius: usize) {
    let diam = 2 * radius + 1;
    for y in 0..h {
        let row_off = y * bw;
        let mut sum: u32 = 0;

        // Initialize sum for first pixel (include left padding)
        for x in 0..=radius.min(w.saturating_sub(1)) {
            sum += src.get(row_off + x).copied().unwrap_or(0);
        }
        // Mirror left edge
        for _ in 0..radius.saturating_sub(0) {
            sum += src.get(row_off).copied().unwrap_or(0);
        }

        for x in 0..w {
            let idx = row_off + x;
            if idx < dst.len() {
                dst[idx] = sum / diam as u32;
            }

            // Slide window: add right pixel, remove left pixel
            let add_x = x + radius + 1;
            let rem_x = x.wrapping_sub(radius);

            let add_val = if add_x < w {
                src.get(row_off + add_x).copied().unwrap_or(0)
            } else {
                // Clamp to edge
                src.get(row_off + w.saturating_sub(1)).copied().unwrap_or(0)
            };

            let rem_val = if rem_x < w {
                src.get(row_off + rem_x).copied().unwrap_or(0)
            } else {
                src.get(row_off).copied().unwrap_or(0)
            };

            sum = sum.wrapping_add(add_val).wrapping_sub(rem_val);
        }
    }
}

/// Vertical box blur pass.
fn box_blur_v(src: &[u32], dst: &mut [u32], bw: usize, w: usize, h: usize, radius: usize) {
    let diam = 2 * radius + 1;
    for x in 0..w {
        let mut sum: u32 = 0;

        // Initialize sum
        for y in 0..=radius.min(h.saturating_sub(1)) {
            sum += src.get(y * bw + x).copied().unwrap_or(0);
        }
        for _ in 0..radius {
            sum += src.get(x).copied().unwrap_or(0);
        }

        for y in 0..h {
            let idx = y * bw + x;
            if idx < dst.len() {
                dst[idx] = sum / diam as u32;
            }

            let add_y = y + radius + 1;
            let rem_y = y.wrapping_sub(radius);

            let add_val = if add_y < h {
                src.get(add_y * bw + x).copied().unwrap_or(0)
            } else {
                src.get(h.saturating_sub(1) * bw + x).copied().unwrap_or(0)
            };

            let rem_val = if rem_y < h {
                src.get(rem_y * bw + x).copied().unwrap_or(0)
            } else {
                src.get(x).copied().unwrap_or(0)
            };

            sum = sum.wrapping_add(add_val).wrapping_sub(rem_val);
        }
    }
}

/// Convert alpha-only values in the buffer to ARGB shadow pixels.
/// Shadow color is black (0x00000000) with the alpha from the blur.
fn convert_alpha_to_shadow(buffer: &mut [u32], bw: usize, w: usize, h: usize) {
    for y in 0..h {
        for x in 0..w {
            let idx = y * bw + x;
            if idx < buffer.len() {
                let alpha = buffer[idx].min(255);
                buffer[idx] = alpha << 24; // Black shadow with alpha
            }
        }
    }
}
