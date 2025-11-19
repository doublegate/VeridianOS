//! Framebuffer implementation

use super::{Color, GraphicsContext, Rect};
use crate::error::KernelError;

/// Framebuffer configuration
pub struct Framebuffer {
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u8,
    buffer: Option<*mut u32>,
}

impl Framebuffer {
    pub const fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 32,
            buffer: None,
        }
    }

    pub fn configure(&mut self, width: u32, height: u32, buffer: *mut u32) {
        self.width = width;
        self.height = height;
        self.pitch = width * 4;
        self.buffer = Some(buffer);
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

impl GraphicsContext for Framebuffer {
    fn draw_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }

        if let Some(buffer) = self.buffer {
            unsafe {
                let offset = (y as u32 * self.width + x as u32) as isize;
                *buffer.offset(offset) = color.to_u32();
            }
        }
    }

    fn draw_rect(&mut self, rect: Rect, color: Color) {
        // Draw top
        for x in rect.x..(rect.x + rect.width as i32) {
            self.draw_pixel(x, rect.y, color);
            self.draw_pixel(x, rect.y + rect.height as i32 - 1, color);
        }
        // Draw sides
        for y in rect.y..(rect.y + rect.height as i32) {
            self.draw_pixel(rect.x, y, color);
            self.draw_pixel(rect.x + rect.width as i32 - 1, y, color);
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        for y in rect.y..(rect.y + rect.height as i32) {
            for x in rect.x..(rect.x + rect.width as i32) {
                self.draw_pixel(x, y, color);
            }
        }
    }

    fn clear(&mut self, color: Color) {
        self.fill_rect(
            Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            color,
        );
    }
}

static mut FRAMEBUFFER: Framebuffer = Framebuffer::new();

/// Get framebuffer instance
pub fn get() -> &'static mut Framebuffer {
    unsafe { &mut FRAMEBUFFER }
}

/// Initialize framebuffer
pub fn init() -> Result<(), KernelError> {
    println!("[FB] Initializing framebuffer...");

    // TODO: Get framebuffer info from bootloader
    // For now, create a dummy configuration
    let fb = get();
    // fb.configure(1024, 768, null_mut()); // Would need actual buffer

    println!(
        "[FB] Framebuffer initialized ({}x{})",
        fb.width(),
        fb.height()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_framebuffer_create() {
        let fb = Framebuffer::new();
        assert_eq!(fb.width(), 0);
        assert_eq!(fb.height(), 0);
    }
}
