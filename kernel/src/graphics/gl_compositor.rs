//! OpenGL-style Compositor
//!
//! Provides a GPU-accelerated compositing pipeline that manages surfaces,
//! performs alpha-over blending, and renders to a back buffer. Uses the
//! texture atlas for efficient surface packing and supports multiple blend
//! modes with integer-only arithmetic.
//!
//! All blending uses integer math: `result = (src * alpha + dst * (255 -
//! alpha)) / 255`.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec, vec::Vec};

use super::texture_atlas::{AtlasRegion, ShelfAllocator};

// ---------------------------------------------------------------------------
// Blend mode
// ---------------------------------------------------------------------------

/// Blending operation applied when compositing a surface onto the back buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Standard alpha-over compositing (Porter-Duff SRC_OVER).
    #[default]
    Over,
    /// Additive blending: `dst + src * alpha` (clamped to 255).
    Add,
    /// Multiplicative blending: `(dst * src) / 255`.
    Multiply,
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// Unique surface identifier.
pub type SurfaceId = u32;

/// A compositable surface managed by the GL compositor.
#[derive(Debug, Clone)]
pub struct GlSurface {
    /// Unique identifier.
    pub id: SurfaceId,
    /// Texture region in the atlas.
    pub atlas_region: AtlasRegion,
    /// Z ordering (higher = closer to viewer).
    pub z_order: i32,
    /// Whether this surface is visible.
    pub visible: bool,
    /// Surface opacity (0 = transparent, 255 = opaque).
    pub opacity: u8,
    /// Blend mode for compositing.
    pub blend_mode: BlendMode,
    /// Position in viewport coordinates.
    pub x: i32,
    /// Position in viewport coordinates.
    pub y: i32,
    /// Pixel data (ARGB8888), row-major.
    pub pixels: Vec<u32>,
}

// ---------------------------------------------------------------------------
// GL compositor
// ---------------------------------------------------------------------------

/// GPU-style compositor that manages surfaces and composites them to a back
/// buffer.
#[derive(Debug)]
pub struct GlCompositor {
    /// Registered surfaces keyed by ID.
    surfaces: BTreeMap<SurfaceId, GlSurface>,
    /// Texture atlas for surface packing.
    atlas: ShelfAllocator,
    /// Viewport width.
    viewport_width: u32,
    /// Viewport height.
    viewport_height: u32,
    /// Composited back buffer (ARGB8888).
    back_buffer: Vec<u32>,
    /// Next surface ID to assign.
    next_id: SurfaceId,
    /// Background clear colour (ARGB8888).
    clear_color: u32,
}

impl GlCompositor {
    /// Create a new compositor with the given viewport dimensions.
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        let pixel_count = (viewport_width as usize) * (viewport_height as usize);
        Self {
            surfaces: BTreeMap::new(),
            atlas: ShelfAllocator::new(viewport_width.max(2048), viewport_height.max(2048)),
            viewport_width,
            viewport_height,
            back_buffer: vec![0xFF000000u32; pixel_count],
            next_id: 1,
            clear_color: 0xFF000000,
        }
    }

    /// Set the background clear colour.
    pub fn set_clear_color(&mut self, color: u32) {
        self.clear_color = color;
    }

    /// Viewport width.
    pub fn width(&self) -> u32 {
        self.viewport_width
    }

    /// Viewport height.
    pub fn height(&self) -> u32 {
        self.viewport_height
    }

    /// Number of registered surfaces.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Create a new surface with the given dimensions.
    ///
    /// Returns the surface ID, or `None` if the atlas is full.
    pub fn create_surface(&mut self, width: u32, height: u32) -> Option<SurfaceId> {
        let region = self.atlas.allocate(width, height)?;
        let pixel_count = (width as usize) * (height as usize);
        let id = self.next_id;
        self.next_id += 1;

        let surface = GlSurface {
            id,
            atlas_region: region,
            z_order: 0,
            visible: true,
            opacity: 255,
            blend_mode: BlendMode::Over,
            x: 0,
            y: 0,
            pixels: vec![0x00000000u32; pixel_count],
        };

        self.surfaces.insert(id, surface);
        Some(id)
    }

    /// Destroy a surface and free its atlas region.
    pub fn destroy_surface(&mut self, id: SurfaceId) -> bool {
        if let Some(surface) = self.surfaces.remove(&id) {
            self.atlas.deallocate(&surface.atlas_region);
            true
        } else {
            false
        }
    }

    /// Update a surface's pixel data.
    ///
    /// `pixels` must be exactly `width * height` ARGB8888 values.
    pub fn update_surface(&mut self, id: SurfaceId, pixels: &[u32]) -> bool {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            let expected =
                (surface.atlas_region.width as usize) * (surface.atlas_region.height as usize);
            if pixels.len() == expected {
                surface.pixels.clear();
                surface.pixels.extend_from_slice(pixels);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Set a surface's position.
    pub fn set_surface_position(&mut self, id: SurfaceId, x: i32, y: i32) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.x = x;
            surface.y = y;
        }
    }

    /// Set a surface's z-order.
    pub fn set_surface_z_order(&mut self, id: SurfaceId, z: i32) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.z_order = z;
        }
    }

    /// Set a surface's visibility.
    pub fn set_surface_visible(&mut self, id: SurfaceId, visible: bool) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.visible = visible;
        }
    }

    /// Set a surface's opacity.
    pub fn set_surface_opacity(&mut self, id: SurfaceId, opacity: u8) {
        if let Some(surface) = self.surfaces.get_mut(&id) {
            surface.opacity = opacity;
        }
    }

    /// Composite all visible surfaces to the back buffer.
    ///
    /// Surfaces are drawn in z-order (lowest first). The back buffer is
    /// cleared before compositing.
    pub fn composite(&mut self) {
        // Clear
        for px in self.back_buffer.iter_mut() {
            *px = self.clear_color;
        }

        // Collect and sort surfaces by z-order
        let mut order: Vec<SurfaceId> = self
            .surfaces
            .values()
            .filter(|s| s.visible)
            .map(|s| s.id)
            .collect();

        // Sort by z_order using surface lookup
        order.sort_by(|a, b| {
            let za = self.surfaces.get(a).map_or(0, |s| s.z_order);
            let zb = self.surfaces.get(b).map_or(0, |s| s.z_order);
            za.cmp(&zb)
        });

        let vw = self.viewport_width as i32;
        let vh = self.viewport_height as i32;

        for sid in &order {
            let surface = match self.surfaces.get(sid) {
                Some(s) => s,
                None => continue,
            };

            let sw = surface.atlas_region.width as i32;
            let sh = surface.atlas_region.height as i32;
            let sx = surface.x;
            let sy = surface.y;
            let opacity = surface.opacity;
            let blend = surface.blend_mode;

            for row in 0..sh {
                let dst_y = sy + row;
                if dst_y < 0 || dst_y >= vh {
                    continue;
                }
                for col in 0..sw {
                    let dst_x = sx + col;
                    if dst_x < 0 || dst_x >= vw {
                        continue;
                    }

                    let src_pixel = surface.pixels[(row * sw + col) as usize];
                    let dst_idx = (dst_y * vw + dst_x) as usize;

                    let blended = blend_pixel(src_pixel, self.back_buffer[dst_idx], opacity, blend);
                    self.back_buffer[dst_idx] = blended;
                }
            }
        }
    }

    /// Return a reference to the composited back buffer.
    pub fn present(&self) -> &[u32] {
        &self.back_buffer
    }

    /// Get a surface by ID.
    pub fn get_surface(&self, id: SurfaceId) -> Option<&GlSurface> {
        self.surfaces.get(&id)
    }

    /// Get a mutable reference to a surface by ID.
    pub fn get_surface_mut(&mut self, id: SurfaceId) -> Option<&mut GlSurface> {
        self.surfaces.get_mut(&id)
    }
}

// ---------------------------------------------------------------------------
// Blending
// ---------------------------------------------------------------------------

/// Blend a source pixel onto a destination pixel using the given mode and
/// opacity.
///
/// All channels use integer arithmetic only.
fn blend_pixel(src: u32, dst: u32, opacity: u8, mode: BlendMode) -> u32 {
    let sa = (src >> 24) & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;

    let da = (dst >> 24) & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;

    // Effective alpha = src_alpha * opacity / 255
    let alpha = (sa * opacity as u32) / 255;
    let inv_alpha = 255 - alpha;

    let (ra, rr, rg, rb) = match mode {
        BlendMode::Over => {
            let oa = alpha + (da * inv_alpha) / 255;
            let or = (sr * alpha + dr * inv_alpha) / 255;
            let og = (sg * alpha + dg * inv_alpha) / 255;
            let ob = (sb * alpha + db * inv_alpha) / 255;
            (oa.min(255), or.min(255), og.min(255), ob.min(255))
        }
        BlendMode::Add => {
            let oa = (alpha + da).min(255);
            let or = (sr * alpha / 255 + dr).min(255);
            let og = (sg * alpha / 255 + dg).min(255);
            let ob = (sb * alpha / 255 + db).min(255);
            (oa, or, og, ob)
        }
        BlendMode::Multiply => {
            let oa = (da * alpha) / 255;
            let or = (dr * sr) / 255;
            let og = (dg * sg) / 255;
            let ob = (db * sb) / 255;
            (oa.min(255), or.min(255), og.min(255), ob.min(255))
        }
    };

    (ra << 24) | (rr << 16) | (rg << 8) | rb
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_surface() {
        let mut comp = GlCompositor::new(640, 480);
        let id = comp.create_surface(100, 50);
        assert!(id.is_some());
        assert_eq!(comp.surface_count(), 1);
    }

    #[test]
    fn test_destroy_surface() {
        let mut comp = GlCompositor::new(640, 480);
        let id = comp.create_surface(100, 50).unwrap();
        assert!(comp.destroy_surface(id));
        assert_eq!(comp.surface_count(), 0);
        assert!(!comp.destroy_surface(id)); // already gone
    }

    #[test]
    fn test_update_surface() {
        let mut comp = GlCompositor::new(640, 480);
        let id = comp.create_surface(4, 4).unwrap();
        let pixels = vec![0xFFFF0000u32; 16];
        assert!(comp.update_surface(id, &pixels));
        // Wrong size should fail
        let bad = vec![0u32; 10];
        assert!(!comp.update_surface(id, &bad));
    }

    #[test]
    fn test_composite_empty() {
        let mut comp = GlCompositor::new(8, 8);
        comp.set_clear_color(0xFF112233);
        comp.composite();
        assert_eq!(comp.present()[0], 0xFF112233);
    }

    #[test]
    fn test_composite_opaque_surface() {
        let mut comp = GlCompositor::new(8, 8);
        comp.set_clear_color(0xFF000000);
        let id = comp.create_surface(4, 4).unwrap();
        let pixels = vec![0xFFFF0000u32; 16]; // opaque red
        comp.update_surface(id, &pixels);
        comp.set_surface_position(id, 0, 0);
        comp.composite();
        // Top-left pixel should be red
        assert_eq!(comp.present()[0], 0xFFFF0000);
        // Pixel at (4,0) should be clear colour
        assert_eq!(comp.present()[4], 0xFF000000);
    }

    #[test]
    fn test_z_order() {
        let mut comp = GlCompositor::new(8, 8);
        let id1 = comp.create_surface(4, 4).unwrap();
        let id2 = comp.create_surface(4, 4).unwrap();
        comp.update_surface(id1, &vec![0xFFFF0000u32; 16]);
        comp.update_surface(id2, &vec![0xFF00FF00u32; 16]);
        comp.set_surface_position(id1, 0, 0);
        comp.set_surface_position(id2, 0, 0);
        comp.set_surface_z_order(id1, 0);
        comp.set_surface_z_order(id2, 1);
        comp.composite();
        // Surface 2 (green) is on top
        assert_eq!(comp.present()[0], 0xFF00FF00);
    }

    #[test]
    fn test_visibility() {
        let mut comp = GlCompositor::new(8, 8);
        comp.set_clear_color(0xFF000000);
        let id = comp.create_surface(4, 4).unwrap();
        comp.update_surface(id, &vec![0xFFFF0000u32; 16]);
        comp.set_surface_visible(id, false);
        comp.composite();
        assert_eq!(comp.present()[0], 0xFF000000);
    }

    #[test]
    fn test_blend_over_semitransparent() {
        // 50% alpha red over black
        let result = blend_pixel(0x80FF0000, 0xFF000000, 255, BlendMode::Over);
        let r = (result >> 16) & 0xFF;
        // Should be roughly half red
        assert!(r > 100 && r < 140);
    }

    #[test]
    fn test_blend_add() {
        let result = blend_pixel(0xFF800000, 0xFF400000, 255, BlendMode::Add);
        let r = (result >> 16) & 0xFF;
        assert_eq!(r, 192); // 0x40 + 0x80
    }

    #[test]
    fn test_blend_multiply() {
        let result = blend_pixel(0xFF800000, 0xFF800000, 255, BlendMode::Multiply);
        let r = (result >> 16) & 0xFF;
        // (128 * 128) / 255 ~ 64
        assert!(r > 60 && r < 68);
    }
}
