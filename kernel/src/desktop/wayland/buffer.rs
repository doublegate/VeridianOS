//! Wayland Buffer
//!
//! Contains pixel data for surface rendering.

/// Pixel format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    ARGB8888,
    XRGB8888,
    RGB565,
}

/// Wayland buffer (shared memory)
pub struct Buffer {
    /// Buffer ID
    pub id: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Stride (bytes per row)
    pub stride: u32,
    /// Pixel format
    pub format: PixelFormat,
    /// Shared memory handle
    pub shm_handle: u64,
}

impl Buffer {
    pub fn new(id: u32, width: u32, height: u32, format: PixelFormat) -> Self {
        let stride = width
            * match format {
                PixelFormat::ARGB8888 | PixelFormat::XRGB8888 => 4,
                PixelFormat::RGB565 => 2,
            };

        Self {
            id,
            width,
            height,
            stride,
            format,
            shm_handle: 0,
        }
    }
}
