//! Wayland Compositor
//!
//! Manages surfaces and composites them into final framebuffer.

use alloc::{collections::BTreeMap, vec::Vec};

use spin::RwLock;

use super::surface::Surface;
use crate::error::KernelError;

/// Compositor state
pub struct Compositor {
    /// All surfaces
    surfaces: RwLock<BTreeMap<u32, Surface>>,
    /// Z-order (top to bottom)
    z_order: RwLock<Vec<u32>>,
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            surfaces: RwLock::new(BTreeMap::new()),
            z_order: RwLock::new(Vec::new()),
        }
    }

    pub fn create_surface(&self, id: u32) -> Result<(), KernelError> {
        let surface = Surface::new(id);
        self.surfaces.write().insert(id, surface);
        self.z_order.write().push(id);
        Ok(())
    }

    pub fn destroy_surface(&self, id: u32) -> Result<(), KernelError> {
        self.surfaces.write().remove(&id);
        self.z_order.write().retain(|&sid| sid != id);
        Ok(())
    }

    pub fn composite(&self) -> Result<(), KernelError> {
        // TODO(phase6): Composite all surfaces in Z-order to framebuffer
        Ok(())
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}
