//! Graphics and GUI subsystem
//!
//! Provides basic graphics support including framebuffer and compositor.

use crate::error::KernelError;

pub mod compositor;
pub mod cursor;
pub mod drm_ioctl;
pub mod fbcon;
pub mod font8x16;
pub mod framebuffer;
pub mod gl_compositor;
pub mod gpu;
pub mod gpu_accel;
pub mod shader;
pub mod texture_atlas;

/// Canonical pixel format descriptor.
///
/// This is the single source of truth for pixel formats across all subsystems
/// (drivers, video, graphics acceleration, Wayland buffers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 24-bit: R(8) G(8) B(8), packed.
    Rgb888,
    /// 24-bit: B(8) G(8) R(8), packed.
    Bgr888,
    /// 32-bit: R(8) G(8) B(8) A(8).
    Rgba8888,
    /// 32-bit: B(8) G(8) R(8) A(8).
    Bgra8888,
    /// 32-bit: x(8) R(8) G(8) B(8), alpha ignored.
    Xrgb8888,
    /// 32-bit: A(8) R(8) G(8) B(8), premultiplied alpha.
    Argb8888,
    /// 16-bit: R(5) G(6) B(5).
    Rgb565,
    /// 32-bit: B(8) G(8) R(8) x(8), alpha ignored.
    Bgrx8888,
    /// 8-bit grayscale.
    Gray8,
    /// 32-bit: x(8) B(8) G(8) R(8), alpha ignored.
    Xbgr8888,
    /// 32-bit: A(8) B(8) G(8) R(8).
    Abgr8888,
}

impl PixelFormat {
    /// Number of bytes per pixel.
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Xrgb8888
            | Self::Argb8888
            | Self::Bgrx8888
            | Self::Xbgr8888
            | Self::Abgr8888
            | Self::Rgba8888
            | Self::Bgra8888 => 4,
            Self::Rgb888 | Self::Bgr888 => 3,
            Self::Rgb565 => 2,
            Self::Gray8 => 1,
        }
    }

    /// Bytes per pixel (alias for `bytes_per_pixel`, returns `u32`).
    pub fn bpp(&self) -> u32 {
        self.bytes_per_pixel() as u32
    }

    /// Whether this format carries an alpha channel.
    pub fn has_alpha(&self) -> bool {
        matches!(
            self,
            Self::Argb8888 | Self::Abgr8888 | Self::Rgba8888 | Self::Bgra8888
        )
    }

    /// Convert a Wayland wl_shm format code to our enum.
    pub fn from_wl_format(code: u32) -> Option<Self> {
        match code {
            0 => Some(PixelFormat::Argb8888), // WL_SHM_FORMAT_ARGB8888
            1 => Some(PixelFormat::Xrgb8888), // WL_SHM_FORMAT_XRGB8888
            _ => None,
        }
    }

    /// Convert our format to the Wayland wl_shm format code.
    pub fn to_wl_format(self) -> u32 {
        match self {
            PixelFormat::Argb8888 => 0,
            PixelFormat::Xrgb8888 => 1,
            PixelFormat::Rgb565 => 0x20363154, // WL_SHM_FORMAT_RGB565
            _ => 0,                            // Default to ARGB8888 for unsupported formats
        }
    }
}

/// Color representation (RGBA)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 255, 0);
    pub const BLUE: Self = Self::rgb(0, 0, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_u32(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

/// Rectangle
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Graphics context
pub trait GraphicsContext {
    fn draw_pixel(&mut self, x: i32, y: i32, color: Color);
    fn draw_rect(&mut self, rect: Rect, color: Color);
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn clear(&mut self, color: Color);
}

/// Initialize graphics subsystem
pub fn init() -> Result<(), KernelError> {
    println!("[GFX] Initializing graphics subsystem...");

    // Initialize framebuffer
    framebuffer::init()?;

    // Initialize compositor
    compositor::init()?;

    println!("[GFX] Graphics subsystem initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color() {
        let c = Color::rgb(128, 64, 32);
        assert_eq!(c.r, 128);
        assert_eq!(c.g, 64);
        assert_eq!(c.b, 32);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn test_color_to_u32() {
        let c = Color::rgb(255, 0, 128);
        let u = c.to_u32();
        assert_ne!(u, 0);
    }
}
