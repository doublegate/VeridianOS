//! Video framework for VeridianOS
//!
//! Provides video frame management, pixel format conversion, image decoding
//! (TGA, QOI, PPM, BMP), framebuffer operations (scaling, blitting, color
//! space conversion), and a simple media player for raw frame sequences.

#![allow(dead_code)]

pub mod decode;
pub mod framebuffer;
pub mod player;

use alloc::{vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Pixel format
// ---------------------------------------------------------------------------

/// Pixel format descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit: x(8) R(8) G(8) B(8), alpha ignored.
    XRGB8888,
    /// 32-bit: A(8) R(8) G(8) B(8), premultiplied alpha.
    ARGB8888,
    /// 24-bit: R(8) G(8) B(8), packed.
    RGB888,
    /// 16-bit: R(5) G(6) B(5).
    RGB565,
    /// 24-bit: B(8) G(8) R(8), packed.
    BGR888,
    /// 32-bit: B(8) G(8) R(8) x(8), alpha ignored.
    BGRX8888,
    /// 8-bit grayscale.
    Gray8,
}

impl PixelFormat {
    /// Number of bytes per pixel.
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::XRGB8888 | Self::ARGB8888 | Self::BGRX8888 => 4,
            Self::RGB888 | Self::BGR888 => 3,
            Self::RGB565 => 2,
            Self::Gray8 => 1,
        }
    }

    /// Whether this format carries an alpha channel.
    pub fn has_alpha(&self) -> bool {
        matches!(self, Self::ARGB8888)
    }
}

// ---------------------------------------------------------------------------
// Video frame
// ---------------------------------------------------------------------------

/// A single video/image frame stored in CPU-accessible memory.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: PixelFormat,
    /// Raw pixel data (row-major, tightly packed or with stride padding).
    data: Vec<u8>,
    /// Row stride in bytes (>= width * bpp).
    pub stride: u32,
}

impl VideoFrame {
    /// Allocate a new, zeroed frame.
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        let bpp = format.bytes_per_pixel() as u32;
        let stride = width * bpp;
        let size = (stride as usize) * (height as usize);
        Self {
            width,
            height,
            format,
            data: vec![0u8; size],
            stride,
        }
    }

    /// Byte offset of pixel (x, y) in the data buffer.
    #[inline]
    pub fn pixel_offset(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.stride as usize) + (x as usize) * self.format.bytes_per_pixel()
    }

    /// Write a pixel at (x, y).  Out-of-bounds writes are silently ignored.
    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let off = self.pixel_offset(x, y);
        match self.format {
            PixelFormat::XRGB8888 => {
                if off + 3 < self.data.len() {
                    self.data[off] = b;
                    self.data[off + 1] = g;
                    self.data[off + 2] = r;
                    self.data[off + 3] = 0xFF;
                }
            }
            PixelFormat::ARGB8888 => {
                if off + 3 < self.data.len() {
                    self.data[off] = b;
                    self.data[off + 1] = g;
                    self.data[off + 2] = r;
                    self.data[off + 3] = a;
                }
            }
            PixelFormat::RGB888 => {
                if off + 2 < self.data.len() {
                    self.data[off] = r;
                    self.data[off + 1] = g;
                    self.data[off + 2] = b;
                }
            }
            PixelFormat::RGB565 => {
                if off + 1 < self.data.len() {
                    let val: u16 =
                        ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3);
                    self.data[off] = (val & 0xFF) as u8;
                    self.data[off + 1] = (val >> 8) as u8;
                }
            }
            PixelFormat::BGR888 => {
                if off + 2 < self.data.len() {
                    self.data[off] = b;
                    self.data[off + 1] = g;
                    self.data[off + 2] = r;
                }
            }
            PixelFormat::BGRX8888 => {
                if off + 3 < self.data.len() {
                    self.data[off] = b;
                    self.data[off + 1] = g;
                    self.data[off + 2] = r;
                    self.data[off + 3] = 0xFF;
                }
            }
            PixelFormat::Gray8 => {
                if off < self.data.len() {
                    // ITU-R BT.601 luma (integer approximation)
                    self.data[off] = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
                }
            }
        }
    }

    /// Read a pixel at (x, y) as (R, G, B, A).
    /// Returns (0,0,0,0) for out-of-bounds coordinates.
    pub fn get_pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        if x >= self.width || y >= self.height {
            return (0, 0, 0, 0);
        }
        let off = self.pixel_offset(x, y);
        match self.format {
            PixelFormat::XRGB8888 => {
                if off + 3 < self.data.len() {
                    (self.data[off + 2], self.data[off + 1], self.data[off], 0xFF)
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::ARGB8888 => {
                if off + 3 < self.data.len() {
                    (
                        self.data[off + 2],
                        self.data[off + 1],
                        self.data[off],
                        self.data[off + 3],
                    )
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::RGB888 => {
                if off + 2 < self.data.len() {
                    (self.data[off], self.data[off + 1], self.data[off + 2], 0xFF)
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::RGB565 => {
                if off + 1 < self.data.len() {
                    let val = (self.data[off] as u16) | ((self.data[off + 1] as u16) << 8);
                    let r = ((val >> 11) & 0x1F) as u8;
                    let g = ((val >> 5) & 0x3F) as u8;
                    let b = (val & 0x1F) as u8;
                    // Expand to 8-bit
                    (
                        (r << 3) | (r >> 2),
                        (g << 2) | (g >> 4),
                        (b << 3) | (b >> 2),
                        0xFF,
                    )
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::BGR888 => {
                if off + 2 < self.data.len() {
                    (self.data[off + 2], self.data[off + 1], self.data[off], 0xFF)
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::BGRX8888 => {
                if off + 3 < self.data.len() {
                    (self.data[off + 2], self.data[off + 1], self.data[off], 0xFF)
                } else {
                    (0, 0, 0, 0)
                }
            }
            PixelFormat::Gray8 => {
                if off < self.data.len() {
                    let v = self.data[off];
                    (v, v, v, 0xFF)
                } else {
                    (0, 0, 0, 0)
                }
            }
        }
    }

    /// Fill the entire frame with a solid color.
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.set_pixel(x, y, r, g, b, 0xFF);
            }
        }
    }

    /// Immutable access to the raw pixel data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Mutable access to the raw pixel data.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

// ---------------------------------------------------------------------------
// Video info
// ---------------------------------------------------------------------------

/// Metadata describing a video stream.
#[derive(Debug, Clone, Copy)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    /// Frame-rate numerator (e.g. 30 for 30 fps).
    pub frame_rate_num: u32,
    /// Frame-rate denominator (e.g. 1 for 30 fps).
    pub frame_rate_den: u32,
}

// ---------------------------------------------------------------------------
// Subsystem init
// ---------------------------------------------------------------------------

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the video subsystem.
pub fn init() -> Result<(), KernelError> {
    if INITIALIZED.load(Ordering::Acquire) {
        return Ok(());
    }

    println!("[VIDEO] Initializing video subsystem...");

    player::init()?;

    INITIALIZED.store(true, Ordering::Release);
    println!("[VIDEO] Video subsystem initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_format_bpp() {
        assert_eq!(PixelFormat::XRGB8888.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::ARGB8888.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::RGB888.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::RGB565.bytes_per_pixel(), 2);
        assert_eq!(PixelFormat::Gray8.bytes_per_pixel(), 1);
        assert_eq!(PixelFormat::BGR888.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::BGRX8888.bytes_per_pixel(), 4);
    }

    #[test]
    fn test_pixel_format_alpha() {
        assert!(!PixelFormat::XRGB8888.has_alpha());
        assert!(PixelFormat::ARGB8888.has_alpha());
        assert!(!PixelFormat::RGB888.has_alpha());
        assert!(!PixelFormat::Gray8.has_alpha());
    }

    #[test]
    fn test_video_frame_new() {
        let f = VideoFrame::new(320, 240, PixelFormat::XRGB8888);
        assert_eq!(f.width, 320);
        assert_eq!(f.height, 240);
        assert_eq!(f.stride, 320 * 4);
        assert_eq!(f.data.len(), 320 * 240 * 4);
    }

    #[test]
    fn test_video_frame_set_get_pixel() {
        let mut f = VideoFrame::new(4, 4, PixelFormat::ARGB8888);
        f.set_pixel(1, 2, 0xAA, 0xBB, 0xCC, 0xDD);
        let (r, g, b, a) = f.get_pixel(1, 2);
        assert_eq!((r, g, b, a), (0xAA, 0xBB, 0xCC, 0xDD));
    }

    #[test]
    fn test_video_frame_out_of_bounds() {
        let mut f = VideoFrame::new(2, 2, PixelFormat::RGB888);
        // Should not panic
        f.set_pixel(10, 10, 255, 0, 0, 255);
        let px = f.get_pixel(10, 10);
        assert_eq!(px, (0, 0, 0, 0));
    }
}
