//! Wayland Surface
//!
//! Represents a renderable rectangular area. Each surface has pending and
//! committed state (double-buffered protocol state) and tracks attached
//! buffers, position, damage, and opaque regions.

use alloc::vec::Vec;

use super::buffer::Buffer;
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Damage region
// ---------------------------------------------------------------------------

/// A rectangular damage region on a surface (pixels that changed).
#[derive(Debug, Clone, Copy)]
pub struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Surface state (pending / committed)
// ---------------------------------------------------------------------------

/// Per-surface state that is applied atomically on commit.
#[derive(Debug, Clone)]
pub struct SurfaceState {
    /// Attached buffer (None = no buffer / transparent)
    pub buffer: Option<Buffer>,
    /// Buffer offset from surface origin
    pub buffer_offset: (i32, i32),
    /// Accumulated damage regions since last commit
    pub damage: Vec<DamageRect>,
    /// Opaque region hint (currently unused, reserved for Phase 6)
    #[allow(dead_code)] // Phase 6: opaque region optimization
    pub opaque: Vec<DamageRect>,
    /// Input region (where the surface accepts pointer/touch)
    #[allow(dead_code)] // Phase 6: input region clipping
    pub input: Vec<DamageRect>,
}

impl SurfaceState {
    fn new() -> Self {
        Self {
            buffer: None,
            buffer_offset: (0, 0),
            damage: Vec::new(),
            opaque: Vec::new(),
            input: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// A Wayland surface representing a renderable area.
pub struct Surface {
    /// Surface ID (Wayland object ID)
    pub id: u32,
    /// Committed (current) state -- what the compositor reads
    pub committed: SurfaceState,
    /// Pending state -- accumulated by attach/damage before commit
    pub pending: SurfaceState,
    /// Position in compositor coordinate space (set by shell role)
    pub position: (i32, i32),
    /// Committed size (derived from buffer dimensions)
    pub size: (u32, u32),
    /// Whether this surface has new content since last composite
    pub dirty: bool,
    /// Whether the surface is mapped (has a committed buffer and a role)
    pub mapped: bool,
    /// Owning client ID
    pub client_id: u32,
}

impl Surface {
    /// Create a new unmapped surface.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            committed: SurfaceState::new(),
            pending: SurfaceState::new(),
            position: (0, 0),
            size: (0, 0),
            dirty: false,
            mapped: false,
            client_id: 0,
        }
    }

    /// Create a new surface with a specific client ID.
    pub fn with_client(id: u32, client_id: u32) -> Self {
        let mut s = Self::new(id);
        s.client_id = client_id;
        s
    }

    /// Attach a buffer to the pending state (wl_surface.attach).
    pub fn attach_buffer(&mut self, buffer: Buffer) {
        self.pending.buffer = Some(buffer);
    }

    /// Attach with an explicit offset.
    pub fn attach_buffer_at(&mut self, buffer: Buffer, dx: i32, dy: i32) {
        self.pending.buffer_offset = (dx, dy);
        self.pending.buffer = Some(buffer);
    }

    /// Mark a damage region on the pending state (wl_surface.damage).
    pub fn damage(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.pending.damage.push(DamageRect {
            x,
            y,
            width,
            height,
        });
    }

    /// Mark the entire surface as damaged.
    pub fn damage_full(&mut self) {
        if let Some(ref buf) = self.pending.buffer {
            self.pending.damage.push(DamageRect {
                x: 0,
                y: 0,
                width: buf.width,
                height: buf.height,
            });
        } else if self.size.0 > 0 && self.size.1 > 0 {
            self.pending.damage.push(DamageRect {
                x: 0,
                y: 0,
                width: self.size.0,
                height: self.size.1,
            });
        }
    }

    /// Commit pending state to committed state (wl_surface.commit).
    ///
    /// This is the atomic state-swap that the Wayland protocol requires.
    pub fn commit(&mut self) -> Result<(), KernelError> {
        // Swap buffer
        if self.pending.buffer.is_some() {
            self.committed.buffer = self.pending.buffer.take();
            self.committed.buffer_offset = self.pending.buffer_offset;

            // Update size from committed buffer
            if let Some(ref buf) = self.committed.buffer {
                self.size = (buf.width, buf.height);
                self.mapped = true;
            }
        }

        // Merge damage
        if !self.pending.damage.is_empty() {
            self.committed.damage.clear();
            core::mem::swap(&mut self.committed.damage, &mut self.pending.damage);
            self.dirty = true;
        }

        // Clear pending damage after commit
        self.pending.damage.clear();
        self.pending.buffer_offset = (0, 0);

        Ok(())
    }

    /// Check whether this surface has a committed buffer.
    pub fn has_buffer(&self) -> bool {
        self.committed.buffer.is_some()
    }

    /// Clear the dirty flag after the compositor has rendered this surface.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
        self.committed.damage.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{super::buffer::PixelFormat, *};

    #[test]
    fn test_surface_lifecycle() {
        let mut surface = Surface::new(1);
        assert!(!surface.mapped);
        assert!(!surface.dirty);

        let buf = Buffer::new(1, 640, 480, PixelFormat::Xrgb8888);
        surface.attach_buffer(buf);
        surface.damage(0, 0, 640, 480);
        surface.commit().unwrap();

        assert!(surface.mapped);
        assert!(surface.dirty);
        assert_eq!(surface.size, (640, 480));
    }

    #[test]
    fn test_surface_clear_dirty() {
        let mut surface = Surface::new(1);
        let buf = Buffer::new(1, 100, 100, PixelFormat::Argb8888);
        surface.attach_buffer(buf);
        surface.damage(0, 0, 100, 100);
        surface.commit().unwrap();
        assert!(surface.dirty);

        surface.clear_dirty();
        assert!(!surface.dirty);
    }
}
