//! Graphics and GUI subsystem
//!
//! Provides basic graphics support including framebuffer and compositor.

use crate::error::KernelError;

pub mod framebuffer;
pub mod compositor;

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

    #[test_case]
    fn test_color() {
        let c = Color::rgb(128, 64, 32);
        assert_eq!(c.r, 128);
        assert_eq!(c.g, 64);
        assert_eq!(c.b, 32);
        assert_eq!(c.a, 255);
    }

    #[test_case]
    fn test_color_to_u32() {
        let c = Color::rgb(255, 0, 128);
        let u = c.to_u32();
        assert_ne!(u, 0);
    }
}
