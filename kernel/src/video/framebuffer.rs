//! Video frame buffer operations
//!
//! Provides scaling (nearest-neighbor, bilinear), pixel format conversion,
//! color space conversion (YUV <-> RGB via BT.601), alpha blending, and
//! framebuffer blitting. All math is integer-only (no FPU).

#![allow(dead_code)]

use super::{PixelFormat, VideoFrame};

// ---------------------------------------------------------------------------
// Scale mode
// ---------------------------------------------------------------------------

/// Scaling algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleMode {
    /// Point sampling -- fastest, blocky.
    NearestNeighbor,
    /// Fixed-point 8.8 bilinear interpolation -- smoother.
    Bilinear,
}

// ---------------------------------------------------------------------------
// Color space
// ---------------------------------------------------------------------------

/// Color space descriptor (for future pipeline use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    SRGB,
    LinearRGB,
    YUV420,
    YUV422,
}

// ---------------------------------------------------------------------------
// Scaling
// ---------------------------------------------------------------------------

/// Scale a `VideoFrame` to the requested dimensions.
///
/// Source pixels are read through `get_pixel()` so any pixel format is
/// supported.  The output frame uses the same `PixelFormat` as the source.
pub fn scale_frame(
    src: &VideoFrame,
    dst_width: u32,
    dst_height: u32,
    mode: ScaleMode,
) -> VideoFrame {
    if dst_width == 0 || dst_height == 0 || src.width == 0 || src.height == 0 {
        return VideoFrame::new(dst_width, dst_height, src.format);
    }

    let mut dst = VideoFrame::new(dst_width, dst_height, src.format);

    match mode {
        ScaleMode::NearestNeighbor => {
            for dy in 0..dst_height {
                let sy = (dy as u64 * src.height as u64 / dst_height as u64) as u32;
                let sy = sy.min(src.height - 1);
                for dx in 0..dst_width {
                    let sx = (dx as u64 * src.width as u64 / dst_width as u64) as u32;
                    let sx = sx.min(src.width - 1);
                    let (r, g, b, a) = src.get_pixel(sx, sy);
                    dst.set_pixel(dx, dy, r, g, b, a);
                }
            }
        }
        ScaleMode::Bilinear => {
            // Fixed-point 8.8 bilinear interpolation.
            // scale_x_fp = src_width * 256 / dst_width  (8.8 step)
            let scale_x_fp = (src.width as u64 * 256) / dst_width as u64;
            let scale_y_fp = (src.height as u64 * 256) / dst_height as u64;

            for dy in 0..dst_height {
                let src_y_fp = (dy as u64 * scale_y_fp) as u32;
                let sy0 = (src_y_fp >> 8).min(src.height - 1);
                let sy1 = (sy0 + 1).min(src.height - 1);
                let fy = src_y_fp & 0xFF; // fractional part 0..255

                for dx in 0..dst_width {
                    let src_x_fp = (dx as u64 * scale_x_fp) as u32;
                    let sx0 = (src_x_fp >> 8).min(src.width - 1);
                    let sx1 = (sx0 + 1).min(src.width - 1);
                    let fx = src_x_fp & 0xFF;

                    // Fetch four neighbouring pixels
                    let (r00, g00, b00, a00) = src.get_pixel(sx0, sy0);
                    let (r10, g10, b10, a10) = src.get_pixel(sx1, sy0);
                    let (r01, g01, b01, a01) = src.get_pixel(sx0, sy1);
                    let (r11, g11, b11, a11) = src.get_pixel(sx1, sy1);

                    // Bilinear weights (all in 0..255)
                    let inv_fx = 256 - fx;
                    let inv_fy = 256 - fy;

                    let w00 = inv_fx * inv_fy; // max 256*256 = 65536
                    let w10 = fx * inv_fy;
                    let w01 = inv_fx * fy;
                    let w11 = fx * fy;

                    let r = ((r00 as u32 * w00
                        + r10 as u32 * w10
                        + r01 as u32 * w01
                        + r11 as u32 * w11)
                        >> 16) as u8;
                    let g = ((g00 as u32 * w00
                        + g10 as u32 * w10
                        + g01 as u32 * w01
                        + g11 as u32 * w11)
                        >> 16) as u8;
                    let b = ((b00 as u32 * w00
                        + b10 as u32 * w10
                        + b01 as u32 * w01
                        + b11 as u32 * w11)
                        >> 16) as u8;
                    let a = ((a00 as u32 * w00
                        + a10 as u32 * w10
                        + a01 as u32 * w01
                        + a11 as u32 * w11)
                        >> 16) as u8;

                    dst.set_pixel(dx, dy, r, g, b, a);
                }
            }
        }
    }

    dst
}

// ---------------------------------------------------------------------------
// Pixel format conversion
// ---------------------------------------------------------------------------

/// Convert a frame from one pixel format to another.
///
/// Pixels are read with `get_pixel()` (format-aware) and written with
/// `set_pixel()`, so any combination of source/destination formats works.
pub fn convert_pixel_format(src: &VideoFrame, dst_format: PixelFormat) -> VideoFrame {
    let mut dst = VideoFrame::new(src.width, src.height, dst_format);
    for y in 0..src.height {
        for x in 0..src.width {
            let (r, g, b, a) = src.get_pixel(x, y);
            dst.set_pixel(x, y, r, g, b, a);
        }
    }
    dst
}

// ---------------------------------------------------------------------------
// Framebuffer blitting
// ---------------------------------------------------------------------------

/// Blit a `VideoFrame` onto a raw framebuffer (BGRA u32 layout).
///
/// The frame is placed at `(x, y)` and clipped to `(fb_width, fb_height)`.
/// `fb_addr` must point to valid, writeable memory of at least
/// `fb_stride * fb_height` bytes.
///
/// # Safety
///
/// The caller must ensure `fb_addr` points to a valid writable region of
/// at least `fb_stride * fb_height` bytes.
pub unsafe fn blit_to_framebuffer(
    frame: &VideoFrame,
    fb_addr: usize,
    fb_width: u32,
    fb_height: u32,
    fb_stride: u32,
    x: u32,
    y: u32,
) {
    if frame.width == 0 || frame.height == 0 {
        return;
    }

    let fb_ptr = fb_addr as *mut u8;

    // Clip region
    let clip_x_start = x;
    let clip_y_start = y;
    let clip_x_end = (x + frame.width).min(fb_width);
    let clip_y_end = (y + frame.height).min(fb_height);

    if clip_x_start >= clip_x_end || clip_y_start >= clip_y_end {
        return;
    }

    for fy in 0..(clip_y_end - clip_y_start) {
        let dst_y = clip_y_start + fy;
        let dst_row_offset = (dst_y as usize) * (fb_stride as usize);

        for fx in 0..(clip_x_end - clip_x_start) {
            let (r, g, b, a) = frame.get_pixel(fx, fy);
            let dst_x = clip_x_start + fx;
            // Framebuffer is BGRA u32 (native VeridianOS format)
            let dst_off = dst_row_offset + (dst_x as usize) * 4;

            if a == 0xFF {
                // Opaque -- direct write
                // SAFETY: dst_off is within the framebuffer region (clipped above).
                let dst = fb_ptr.add(dst_off);
                *dst = b;
                *dst.add(1) = g;
                *dst.add(2) = r;
                *dst.add(3) = 0xFF;
            } else if a > 0 {
                // Alpha blend with existing framebuffer content
                // SAFETY: dst_off is within the framebuffer region (clipped above).
                let dst = fb_ptr.add(dst_off);
                let bg_b = *dst;
                let bg_g = *dst.add(1);
                let bg_r = *dst.add(2);
                let (br, bg, bb) = alpha_blend(r, g, b, a, bg_r, bg_g, bg_b);
                *dst = bb;
                *dst.add(1) = bg;
                *dst.add(2) = br;
                *dst.add(3) = 0xFF;
            }
            // a == 0: fully transparent, skip
        }
    }
}

/// Blit a `VideoFrame` onto a u32 slice buffer (BGRA layout).
///
/// Safe variant that works with Rust slices instead of raw pointers.
pub fn blit_to_buffer(
    frame: &VideoFrame,
    buffer: &mut [u32],
    buf_width: u32,
    buf_height: u32,
    x: u32,
    y: u32,
) {
    if frame.width == 0 || frame.height == 0 {
        return;
    }

    let clip_x_end = (x + frame.width).min(buf_width);
    let clip_y_end = (y + frame.height).min(buf_height);

    if x >= clip_x_end || y >= clip_y_end {
        return;
    }

    for fy in 0..(clip_y_end - y) {
        let dst_y = y + fy;
        for fx in 0..(clip_x_end - x) {
            let dst_x = x + fx;
            let idx = (dst_y as usize) * (buf_width as usize) + (dst_x as usize);
            if idx >= buffer.len() {
                continue;
            }

            let (r, g, b, a) = frame.get_pixel(fx, fy);
            if a == 0xFF {
                buffer[idx] = 0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            } else if a > 0 {
                let existing = buffer[idx];
                let bg_r = ((existing >> 16) & 0xFF) as u8;
                let bg_g = ((existing >> 8) & 0xFF) as u8;
                let bg_b = (existing & 0xFF) as u8;
                let (br, bg, bb) = alpha_blend(r, g, b, a, bg_r, bg_g, bg_b);
                buffer[idx] = 0xFF00_0000 | ((br as u32) << 16) | ((bg as u32) << 8) | (bb as u32);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// YUV <-> RGB  (BT.601)
// ---------------------------------------------------------------------------

/// Convert YUV to RGB using BT.601 integer coefficients.
///
/// ```text
/// R = Y + 1.402 * (V - 128)     ~  Y + (359 * (V-128)) >> 8
/// G = Y - 0.344 * (U - 128)
///       - 0.714 * (V - 128)     ~  Y - (88*(U-128) + 183*(V-128)) >> 8
/// B = Y + 1.772 * (U - 128)     ~  Y + (454 * (U-128)) >> 8
/// ```
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let y = y as i32;
    let cb = u as i32 - 128;
    let cr = v as i32 - 128;

    let r = y + ((359 * cr) >> 8);
    let g = y - ((88 * cb + 183 * cr) >> 8);
    let b = y + ((454 * cb) >> 8);

    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

/// Convert RGB to YUV using BT.601 integer coefficients.
///
/// ```text
/// Y  =  0.299*R + 0.587*G + 0.114*B   ~ (77*R + 150*G + 29*B) >> 8
/// U  = -0.169*R - 0.331*G + 0.500*B + 128  ~ ((-43*R - 85*G + 128*B) >> 8) + 128
/// V  =  0.500*R - 0.419*G - 0.081*B + 128  ~ ((128*R - 107*G - 21*B) >> 8) + 128
/// ```
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let ri = r as i32;
    let gi = g as i32;
    let bi = b as i32;

    let y = (77 * ri + 150 * gi + 29 * bi) >> 8;
    let u = ((-43 * ri - 85 * gi + 128 * bi) >> 8) + 128;
    let v = ((128 * ri - 107 * gi - 21 * bi) >> 8) + 128;

    (clamp_u8(y), clamp_u8(u), clamp_u8(v))
}

// ---------------------------------------------------------------------------
// Alpha blending
// ---------------------------------------------------------------------------

/// Alpha-blend a source pixel over a destination pixel.
///
/// Uses integer math: `result = (src * alpha + dst * (255 - alpha)) / 255`.
/// This is the standard "over" compositing operation.
pub fn alpha_blend(
    src_r: u8,
    src_g: u8,
    src_b: u8,
    src_a: u8,
    dst_r: u8,
    dst_g: u8,
    dst_b: u8,
) -> (u8, u8, u8) {
    let a = src_a as u16;
    let inv_a = 255 - a;

    let r = ((src_r as u16 * a + dst_r as u16 * inv_a) / 255) as u8;
    let g = ((src_g as u16 * a + dst_g as u16 * inv_a) / 255) as u8;
    let b = ((src_b as u16 * a + dst_b as u16 * inv_a) / 255) as u8;

    (r, g, b)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clamp an i32 to the 0..=255 range and return as u8.
#[inline]
fn clamp_u8(val: i32) -> u8 {
    if val < 0 {
        0
    } else if val > 255 {
        255
    } else {
        val as u8
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nearest_neighbor_identity() {
        let mut src = VideoFrame::new(4, 4, PixelFormat::ARGB8888);
        src.set_pixel(0, 0, 255, 0, 0, 255);
        src.set_pixel(3, 3, 0, 255, 0, 255);

        let dst = scale_frame(&src, 4, 4, ScaleMode::NearestNeighbor);
        assert_eq!(dst.get_pixel(0, 0), (255, 0, 0, 255));
        assert_eq!(dst.get_pixel(3, 3), (0, 255, 0, 255));
    }

    #[test]
    fn test_nearest_neighbor_upscale() {
        let mut src = VideoFrame::new(2, 2, PixelFormat::ARGB8888);
        src.set_pixel(0, 0, 100, 200, 50, 255);
        src.set_pixel(1, 1, 10, 20, 30, 255);

        let dst = scale_frame(&src, 4, 4, ScaleMode::NearestNeighbor);
        // (0,0) should map to src(0,0)
        assert_eq!(dst.get_pixel(0, 0), (100, 200, 50, 255));
        // (3,3) should map to src(1,1)
        assert_eq!(dst.get_pixel(3, 3), (10, 20, 30, 255));
    }

    #[test]
    fn test_bilinear_identity() {
        let mut src = VideoFrame::new(4, 4, PixelFormat::ARGB8888);
        src.set_pixel(0, 0, 255, 0, 0, 255);

        let dst = scale_frame(&src, 4, 4, ScaleMode::Bilinear);
        let (r, _g, _b, _a) = dst.get_pixel(0, 0);
        // Should be close to the original (may differ slightly due to fixed-point)
        assert!(r > 240);
    }

    #[test]
    fn test_convert_xrgb_to_rgb888() {
        let mut src = VideoFrame::new(2, 2, PixelFormat::XRGB8888);
        src.set_pixel(0, 0, 0xAA, 0xBB, 0xCC, 0xFF);

        let dst = convert_pixel_format(&src, PixelFormat::RGB888);
        assert_eq!(dst.format, PixelFormat::RGB888);
        let (r, g, b, a) = dst.get_pixel(0, 0);
        assert_eq!((r, g, b), (0xAA, 0xBB, 0xCC));
        assert_eq!(a, 0xFF);
    }

    #[test]
    fn test_yuv_rgb_roundtrip() {
        // Test that RGB -> YUV -> RGB is approximately identity
        let r0: u8 = 180;
        let g0: u8 = 100;
        let b0: u8 = 60;

        let (y, u, v) = rgb_to_yuv(r0, g0, b0);
        let (r1, g1, b1) = yuv_to_rgb(y, u, v);

        // Allow +/- 2 rounding error
        assert!((r1 as i16 - r0 as i16).unsigned_abs() <= 2);
        assert!((g1 as i16 - g0 as i16).unsigned_abs() <= 2);
        assert!((b1 as i16 - b0 as i16).unsigned_abs() <= 2);
    }

    #[test]
    fn test_yuv_black_white() {
        // Black: Y=0, U=128, V=128 -> R=0, G=0, B=0
        let (r, g, b) = yuv_to_rgb(0, 128, 128);
        assert_eq!((r, g, b), (0, 0, 0));

        // White: Y=255, U=128, V=128 -> R=255, G=255, B=255
        let (r, g, b) = yuv_to_rgb(255, 128, 128);
        assert_eq!((r, g, b), (255, 255, 255));
    }

    #[test]
    fn test_alpha_blend_opaque() {
        let (r, g, b) = alpha_blend(100, 200, 50, 255, 0, 0, 0);
        assert_eq!((r, g, b), (100, 200, 50));
    }

    #[test]
    fn test_alpha_blend_transparent() {
        let (r, g, b) = alpha_blend(100, 200, 50, 0, 10, 20, 30);
        assert_eq!((r, g, b), (10, 20, 30));
    }

    #[test]
    fn test_alpha_blend_half() {
        let (r, g, b) = alpha_blend(200, 100, 0, 128, 0, 0, 200);
        // r ~ (200*128 + 0*127) / 255 ~ 100
        // b ~ (0*128 + 200*127) / 255 ~ 99
        assert!(r > 90 && r < 110);
        assert!(b > 90 && b < 110);
    }
}
