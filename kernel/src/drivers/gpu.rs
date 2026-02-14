//! GPU Driver Module
//!
//! Supports VBE (VESA BIOS Extensions) and GOP (Graphics Output Protocol) for
//! framebuffer access

// Allow dead code for GPU mode info fields not yet fully utilized
#![allow(dead_code)]

use core::slice;

use spin::Mutex;

use crate::{error::KernelError, graphics::Color};

/// VBE Mode Info Block
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct VbeModeInfo {
    pub attributes: u16,
    pub window_a: u8,
    pub window_b: u8,
    pub granularity: u16,
    pub window_size: u16,
    pub segment_a: u16,
    pub segment_b: u16,
    pub win_func_ptr: u32,
    pub pitch: u16,
    pub width: u16,
    pub height: u16,
    pub w_char: u8,
    pub y_char: u8,
    pub planes: u8,
    pub bpp: u8,
    pub banks: u8,
    pub memory_model: u8,
    pub bank_size: u8,
    pub image_pages: u8,
    pub reserved0: u8,
    pub red_mask: u8,
    pub red_position: u8,
    pub green_mask: u8,
    pub green_position: u8,
    pub blue_mask: u8,
    pub blue_position: u8,
    pub reserved_mask: u8,
    pub reserved_position: u8,
    pub direct_color_attributes: u8,
    pub framebuffer: u32,
    pub off_screen_mem_off: u32,
    pub off_screen_mem_size: u16,
}

/// GOP Pixel Format
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GopPixelFormat {
    RedGreenBlueReserved = 0,
    BlueGreenRedReserved = 1,
    BitMask = 2,
    BltOnly = 3,
}

/// GOP Mode Info
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GopModeInfo {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: GopPixelFormat,
    pub pixel_information: [u32; 4],
    pub pixels_per_scan_line: u32,
}

/// GPU Driver
pub struct GpuDriver {
    framebuffer_addr: usize,
    width: usize,
    height: usize,
    pitch: usize,
    bpp: usize,
    pixel_format: PixelFormat,
}

/// Pixel Format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb888,
    Bgr888,
    Rgba8888,
    Bgra8888,
}

impl GpuDriver {
    /// Create GPU driver from VBE mode info
    pub fn from_vbe(mode_info: &VbeModeInfo) -> Result<Self, KernelError> {
        if mode_info.framebuffer == 0 {
            return Err(KernelError::HardwareError {
                device: "vbe",
                code: 1,
            });
        }

        let pixel_format = if mode_info.red_position == 0 {
            PixelFormat::Bgr888
        } else {
            PixelFormat::Rgb888
        };

        Ok(Self {
            framebuffer_addr: mode_info.framebuffer as usize,
            width: mode_info.width as usize,
            height: mode_info.height as usize,
            pitch: mode_info.pitch as usize,
            bpp: mode_info.bpp as usize / 8,
            pixel_format,
        })
    }

    /// Create GPU driver from GOP mode info
    pub fn from_gop(framebuffer_addr: usize, mode_info: &GopModeInfo) -> Result<Self, KernelError> {
        if framebuffer_addr == 0 {
            return Err(KernelError::HardwareError {
                device: "gop",
                code: 1,
            });
        }

        let pixel_format = match mode_info.pixel_format {
            GopPixelFormat::RedGreenBlueReserved => PixelFormat::Rgba8888,
            GopPixelFormat::BlueGreenRedReserved => PixelFormat::Bgra8888,
            _ => PixelFormat::Rgba8888,
        };

        Ok(Self {
            framebuffer_addr,
            width: mode_info.horizontal_resolution as usize,
            height: mode_info.vertical_resolution as usize,
            pitch: (mode_info.pixels_per_scan_line * 4) as usize,
            bpp: 4,
            pixel_format,
        })
    }

    /// Create a simple framebuffer driver (for testing)
    pub fn simple(framebuffer_addr: usize, width: usize, height: usize) -> Self {
        Self {
            framebuffer_addr,
            width,
            height,
            pitch: width * 4,
            bpp: 4,
            pixel_format: PixelFormat::Rgba8888,
        }
    }

    /// Get framebuffer as mutable slice
    fn framebuffer_mut(&mut self) -> &mut [u32] {
        // SAFETY: framebuffer_addr points to a memory-mapped framebuffer region
        // provided by the VBE/GOP firmware or set during initialization. The
        // slice covers exactly (pitch * height / 4) u32 entries, which
        // corresponds to the full framebuffer. We hold &mut self so no other
        // reference to this data exists.
        unsafe {
            slice::from_raw_parts_mut(
                self.framebuffer_addr as *mut u32,
                (self.pitch * self.height) / 4,
            )
        }
    }

    /// Set a pixel at (x, y)
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) -> Result<(), KernelError> {
        if x >= self.width || y >= self.height {
            return Err(KernelError::InvalidArgument {
                name: "coordinates",
                value: "out_of_bounds",
            });
        }

        let offset = y * (self.pitch / 4) + x;
        let pixel_value = self.color_to_pixel(color);

        let fb = self.framebuffer_mut();
        fb[offset] = pixel_value;

        Ok(())
    }

    /// Fill a rectangle
    pub fn fill_rect(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
    ) -> Result<(), KernelError> {
        let pixel_value = self.color_to_pixel(color);
        let width = self.width;
        let height = self.height;
        let pitch = self.pitch;
        let fb = self.framebuffer_mut();

        for dy in 0..h {
            let row_y = y + dy;
            if row_y >= height {
                break;
            }

            for dx in 0..w {
                let col_x = x + dx;
                if col_x >= width {
                    break;
                }

                let offset = row_y * (pitch / 4) + col_x;
                fb[offset] = pixel_value;
            }
        }

        Ok(())
    }

    /// Clear the screen
    pub fn clear(&mut self, color: Color) {
        let _ = self.fill_rect(0, 0, self.width, self.height, color);
    }

    /// Convert Color to pixel value based on format
    fn color_to_pixel(&self, color: Color) -> u32 {
        match self.pixel_format {
            PixelFormat::Rgb888 | PixelFormat::Rgba8888 => {
                ((color.a as u32) << 24)
                    | ((color.r as u32) << 16)
                    | ((color.g as u32) << 8)
                    | (color.b as u32)
            }
            PixelFormat::Bgr888 | PixelFormat::Bgra8888 => {
                ((color.a as u32) << 24)
                    | ((color.b as u32) << 16)
                    | ((color.g as u32) << 8)
                    | (color.r as u32)
            }
        }
    }

    /// Get width
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get height
    pub fn height(&self) -> usize {
        self.height
    }

    /// Blit buffer to screen
    pub fn blit(
        &mut self,
        buffer: &[u32],
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> Result<(), KernelError> {
        let width = self.width;
        let height = self.height;
        let pitch = self.pitch;
        let fb = self.framebuffer_mut();

        for dy in 0..h {
            let row_y = y + dy;
            if row_y >= height {
                break;
            }

            for dx in 0..w {
                let col_x = x + dx;
                if col_x >= width {
                    break;
                }

                let buf_offset = dy * w + dx;
                let fb_offset = row_y * (pitch / 4) + col_x;

                if buf_offset < buffer.len() {
                    fb[fb_offset] = buffer[buf_offset];
                }
            }
        }

        Ok(())
    }
}

/// Global GPU driver instance protected by Mutex
static GPU_DRIVER: Mutex<Option<GpuDriver>> = Mutex::new(None);

/// Initialize GPU driver
pub fn init() -> Result<(), KernelError> {
    println!("[GPU] Initializing GPU driver...");

    // For now, create a simple framebuffer (would normally detect VBE/GOP)
    // TODO(phase6): Detect actual framebuffer from bootloader (VBE/GOP)
    let driver = GpuDriver::simple(0xFD000000, 1024, 768);

    *GPU_DRIVER.lock() = Some(driver);

    println!("[GPU] GPU driver initialized (1024x768)");
    Ok(())
}

/// Execute a closure with the GPU driver (mutable access)
pub fn with_driver<R, F: FnOnce(&mut GpuDriver) -> R>(f: F) -> Option<R> {
    GPU_DRIVER.lock().as_mut().map(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_pixel_format() {
        let driver = GpuDriver::simple(0x1000000, 800, 600);
        let color = Color {
            r: 255,
            g: 128,
            b: 64,
            a: 255,
        };
        let pixel = driver.color_to_pixel(color);

        // RGBA8888 format
        assert_eq!(pixel, 0xFF_FF_80_40);
    }
}
