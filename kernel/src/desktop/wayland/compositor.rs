//! Wayland Compositor
//!
//! Manages surfaces and composites them into a back-buffer that can be
//! presented to the hardware framebuffer. Surfaces are drawn in Z-order
//! with per-pixel alpha blending for ARGB8888 and fast memcpy for XRGB8888.

use alloc::{collections::BTreeMap, vec, vec::Vec};

use spin::RwLock;

use super::{
    buffer::{self, PixelFormat},
    surface::Surface,
};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Desktop background color (dark blue-grey)
// ---------------------------------------------------------------------------

/// Desktop background color: ARGB packed as 0xAARRGGBB.
const DESKTOP_BG_COLOR: u32 = 0xFF2D_3436;

// ---------------------------------------------------------------------------
// Compositor
// ---------------------------------------------------------------------------

/// Wayland compositor state.
///
/// Owns all surfaces and the software back-buffer used for compositing.
pub struct Compositor {
    /// All surfaces keyed by surface ID
    surfaces: RwLock<BTreeMap<u32, Surface>>,
    /// Z-order: first element is bottom-most, last is top-most
    z_order: RwLock<Vec<u32>>,
    /// Software back-buffer (XRGB8888, row-major, width*height u32 pixels)
    back_buffer: RwLock<Vec<u32>>,
    /// Back-buffer dimensions (atomic for interior mutability)
    fb_width: core::sync::atomic::AtomicU32,
    fb_height: core::sync::atomic::AtomicU32,
    /// Whether any surface is dirty and compositing is needed
    needs_composite: core::sync::atomic::AtomicBool,
}

impl Compositor {
    /// Create a new compositor. Dimensions default to 0 until
    /// `set_output_size`.
    pub fn new() -> Self {
        Self {
            surfaces: RwLock::new(BTreeMap::new()),
            z_order: RwLock::new(Vec::new()),
            back_buffer: RwLock::new(Vec::new()),
            fb_width: core::sync::atomic::AtomicU32::new(0),
            fb_height: core::sync::atomic::AtomicU32::new(0),
            needs_composite: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Configure the output resolution. Allocates the back-buffer.
    pub fn set_output_size(&self, width: u32, height: u32) {
        self.fb_width
            .store(width, core::sync::atomic::Ordering::Release);
        self.fb_height
            .store(height, core::sync::atomic::Ordering::Release);
        let pixel_count = (width as usize) * (height as usize);
        *self.back_buffer.write() = vec![DESKTOP_BG_COLOR; pixel_count];
    }

    /// Register a new surface.
    pub fn create_surface(&self, id: u32) -> Result<(), KernelError> {
        let surface = Surface::new(id);
        self.surfaces.write().insert(id, surface);
        self.z_order.write().push(id);
        Ok(())
    }

    /// Register a new surface owned by a specific client.
    pub fn create_surface_for_client(&self, id: u32, client_id: u32) -> Result<(), KernelError> {
        let surface = Surface::with_client(id, client_id);
        self.surfaces.write().insert(id, surface);
        self.z_order.write().push(id);
        Ok(())
    }

    /// Destroy a surface and remove it from Z-order.
    pub fn destroy_surface(&self, id: u32) -> Result<(), KernelError> {
        self.surfaces.write().remove(&id);
        self.z_order.write().retain(|&sid| sid != id);
        Ok(())
    }

    /// Get a read reference to a surface.
    pub fn get_surface(&self, id: u32) -> Option<Surface> {
        let surfaces = self.surfaces.read();
        // Clone the surface data we need -- avoids holding the lock across
        // caller code.
        surfaces.get(&id).map(|s| Surface {
            id: s.id,
            committed: s.committed.clone(),
            pending: s.pending.clone(),
            position: s.position,
            size: s.size,
            dirty: s.dirty,
            mapped: s.mapped,
            client_id: s.client_id,
        })
    }

    /// Execute a closure with mutable access to a surface.
    pub fn with_surface_mut<R, F: FnOnce(&mut Surface) -> R>(&self, id: u32, f: F) -> Option<R> {
        let mut surfaces = self.surfaces.write();
        surfaces.get_mut(&id).map(f)
    }

    /// Raise a surface to the top of the Z-order stack.
    pub fn raise_surface(&self, id: u32) {
        let mut z = self.z_order.write();
        z.retain(|&sid| sid != id);
        z.push(id);
    }

    /// Set a surface's position in compositor coordinates.
    pub fn set_surface_position(&self, id: u32, x: i32, y: i32) {
        if let Some(surface) = self.surfaces.write().get_mut(&id) {
            surface.position = (x, y);
        }
    }

    /// Mark that compositing is needed.
    pub fn request_composite(&self) {
        self.needs_composite
            .store(true, core::sync::atomic::Ordering::Release);
    }

    /// Composite all visible, mapped surfaces in Z-order into the back-buffer.
    ///
    /// 1. Clear back-buffer to desktop background color.
    /// 2. For each surface (bottom to top), blit its committed buffer.
    /// 3. Mark surfaces as clean.
    ///
    /// Returns `true` if any pixels were actually drawn.
    pub fn composite(&self) -> Result<bool, KernelError> {
        let fb_w_val = self.fb_width.load(core::sync::atomic::Ordering::Acquire);
        let fb_h_val = self.fb_height.load(core::sync::atomic::Ordering::Acquire);
        if fb_w_val == 0 || fb_h_val == 0 {
            return Ok(false);
        }

        let z_order = self.z_order.read().clone();
        let mut surfaces = self.surfaces.write();
        let mut bb = self.back_buffer.write();

        let fb_w = fb_w_val as usize;
        let fb_h = fb_h_val as usize;

        // Step 1: clear to background
        for pixel in bb.iter_mut() {
            *pixel = DESKTOP_BG_COLOR;
        }

        let mut any_drawn = false;

        // Step 2: blit surfaces in Z-order
        for &sid in &z_order {
            let surface = match surfaces.get(&sid) {
                Some(s) => s,
                None => continue,
            };

            if !surface.mapped || surface.size.0 == 0 || surface.size.1 == 0 {
                continue;
            }

            let buf = match &surface.committed.buffer {
                Some(b) => b,
                None => continue,
            };

            // Blit the surface buffer into the back-buffer directly from
            // the SHM pool, avoiding a multi-MB `.to_vec()` clone per surface.
            let sx = surface.position.0;
            let sy = surface.position.1;
            let sw = buf.width as usize;
            let sh = buf.height as usize;
            let stride = buf.stride as usize;
            let format = buf.format;
            let pool_id = buf.pool_id;
            let pool_buffer_id = buf.pool_buffer_id;

            if pool_id == 0 {
                continue;
            }

            let drew = buffer::with_pool(pool_id, |pool| {
                let pixels = match pool.read_buffer_pixels(pool_buffer_id) {
                    Some(p) => p,
                    None => return false,
                };

                for row in 0..sh {
                    let dst_y = sy as isize + row as isize;
                    if dst_y < 0 || dst_y >= fb_h as isize {
                        continue;
                    }
                    let dst_y = dst_y as usize;

                    for col in 0..sw {
                        let dst_x = sx as isize + col as isize;
                        if dst_x < 0 || dst_x >= fb_w as isize {
                            continue;
                        }
                        let dst_x = dst_x as usize;

                        let src_off = row * stride + col * format.bpp() as usize;
                        if src_off + 3 >= pixels.len() {
                            break;
                        }

                        let dst_idx = dst_y * fb_w + dst_x;

                        match format {
                            PixelFormat::Xrgb8888 => {
                                let b_val = pixels[src_off] as u32;
                                let g_val = pixels[src_off + 1] as u32;
                                let r_val = pixels[src_off + 2] as u32;
                                bb[dst_idx] = 0xFF00_0000 | (r_val << 16) | (g_val << 8) | b_val;
                            }
                            PixelFormat::Argb8888 => {
                                let b_src = pixels[src_off] as u32;
                                let g_src = pixels[src_off + 1] as u32;
                                let r_src = pixels[src_off + 2] as u32;
                                let a_src = pixels[src_off + 3] as u32;

                                if a_src == 255 {
                                    bb[dst_idx] =
                                        0xFF00_0000 | (r_src << 16) | (g_src << 8) | b_src;
                                } else if a_src > 0 {
                                    let dst_pixel = bb[dst_idx];
                                    let r_dst = (dst_pixel >> 16) & 0xFF;
                                    let g_dst = (dst_pixel >> 8) & 0xFF;
                                    let b_dst = dst_pixel & 0xFF;

                                    let inv_a = 255 - a_src;
                                    let r_out = (r_src * a_src + r_dst * inv_a) / 255;
                                    let g_out = (g_src * a_src + g_dst * inv_a) / 255;
                                    let b_out = (b_src * a_src + b_dst * inv_a) / 255;

                                    bb[dst_idx] =
                                        0xFF00_0000 | (r_out << 16) | (g_out << 8) | b_out;
                                }
                            }
                            PixelFormat::Rgb565 => {
                                if src_off + 1 < pixels.len() {
                                    let raw = (pixels[src_off] as u16)
                                        | ((pixels[src_off + 1] as u16) << 8);
                                    let r5 = ((raw >> 11) & 0x1F) as u32;
                                    let g6 = ((raw >> 5) & 0x3F) as u32;
                                    let b5 = (raw & 0x1F) as u32;
                                    let r8 = (r5 * 255 + 15) / 31;
                                    let g8 = (g6 * 255 + 31) / 63;
                                    let b8 = (b5 * 255 + 15) / 31;
                                    bb[dst_idx] = 0xFF00_0000 | (r8 << 16) | (g8 << 8) | b8;
                                }
                            }
                        }
                    }
                }
                true
            });
            if drew == Some(true) {
                any_drawn = true;
            }
        }

        // Step 3: clear dirty flags
        for surface in surfaces.values_mut() {
            surface.clear_dirty();
        }

        self.needs_composite
            .store(false, core::sync::atomic::Ordering::Release);

        Ok(any_drawn)
    }

    /// Get a snapshot of the back-buffer for presentation to hardware.
    #[allow(dead_code)] // Kept for API compatibility; prefer with_back_buffer()
    pub fn back_buffer(&self) -> Vec<u32> {
        self.back_buffer.read().clone()
    }

    /// Execute a closure with a read-only reference to the back-buffer.
    ///
    /// Avoids the 4MB clone that `back_buffer()` performs each call.
    pub fn with_back_buffer<R, F: FnOnce(&[u32]) -> R>(&self, f: F) -> R {
        let bb = self.back_buffer.read();
        f(&bb)
    }

    /// Back-buffer dimensions.
    pub fn output_size(&self) -> (u32, u32) {
        (
            self.fb_width.load(core::sync::atomic::Ordering::Acquire),
            self.fb_height.load(core::sync::atomic::Ordering::Acquire),
        )
    }

    /// Number of surfaces.
    pub fn surface_count(&self) -> usize {
        self.surfaces.read().len()
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::buffer::{Buffer, PixelFormat},
        *,
    };

    #[test]
    fn test_create_destroy_surface() {
        let comp = Compositor::new();
        comp.create_surface(1).unwrap();
        assert_eq!(comp.surface_count(), 1);
        comp.destroy_surface(1).unwrap();
        assert_eq!(comp.surface_count(), 0);
    }

    #[test]
    fn test_composite_empty() {
        let comp = Compositor::new();
        comp.set_output_size(64, 64);
        let drawn = comp.composite().unwrap();
        assert!(!drawn); // no surfaces
                         // Back-buffer should be filled with background color
        let bb = comp.back_buffer();
        assert_eq!(bb.len(), 64 * 64);
        assert_eq!(bb[0], DESKTOP_BG_COLOR);
    }

    #[test]
    fn test_z_order_raise() {
        let comp = Compositor::new();
        comp.create_surface(1).unwrap();
        comp.create_surface(2).unwrap();
        comp.create_surface(3).unwrap();

        comp.raise_surface(1);
        let z = comp.z_order.read().clone();
        assert_eq!(z, vec![2, 3, 1]);
    }
}
