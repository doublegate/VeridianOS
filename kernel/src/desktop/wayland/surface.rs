//! Wayland Surface
//!
//! Represents a renderable rectangular area.

use super::buffer::Buffer;
use crate::error::KernelError;

/// Wayland surface
pub struct Surface {
    /// Surface ID
    pub id: u32,
    /// Attached buffer
    pub buffer: Option<Buffer>,
    /// Position (x, y)
    pub position: (i32, i32),
    /// Size (width, height)
    pub size: (u32, u32),
}

impl Surface {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            buffer: None,
            position: (0, 0),
            size: (0, 0),
        }
    }

    pub fn attach_buffer(&mut self, buffer: Buffer) {
        self.size = (buffer.width, buffer.height);
        self.buffer = Some(buffer);
    }

    pub fn commit(&mut self) -> Result<(), KernelError> {
        // TODO(phase6): Submit surface to compositor for rendering
        Ok(())
    }
}
