//! Drag-and-Drop
//!
//! wl_data_offer protocol with enter/leave/drop/motion events.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::clipboard::ClipboardMime;

/// Errors that can occur during drag-and-drop operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndError {
    /// No drag operation is in progress.
    NotDragging,
    /// A drag is already in progress.
    AlreadyDragging,
    /// The drop target rejected the drop.
    DropRejected,
    /// No MIME type matches between source and target.
    NoMimeMatch,
    /// Invalid surface ID.
    InvalidSurface,
    /// The drag operation was cancelled.
    Cancelled,
}

impl core::fmt::Display for DndError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotDragging => write!(f, "not dragging"),
            Self::AlreadyDragging => write!(f, "already dragging"),
            Self::DropRejected => write!(f, "drop rejected"),
            Self::NoMimeMatch => write!(f, "no MIME match"),
            Self::InvalidSurface => write!(f, "invalid surface"),
            Self::Cancelled => write!(f, "drag cancelled"),
        }
    }
}

/// State machine for drag-and-drop operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DndState {
    /// No drag operation active.
    #[default]
    Idle,
    /// A drag is in progress (user holding mouse button).
    Dragging,
    /// Over a valid drop target, waiting for drop confirmation.
    DropPending,
    /// Drop was accepted and data transfer is happening.
    Transferring,
}

/// Events emitted by the DnD subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndEvent {
    /// Drag entered a surface.
    Enter { surface_id: u32, x: i32, y: i32 },
    /// Drag left a surface.
    Leave { surface_id: u32 },
    /// Drag moved within a surface.
    Motion { surface_id: u32, x: i32, y: i32 },
    /// Drop occurred on a surface.
    Drop { surface_id: u32, x: i32, y: i32 },
    /// Drag was cancelled.
    Cancelled,
}

/// Information about a drag source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct DragSource {
    /// Surface ID that initiated the drag.
    pub source_surface: u32,
    /// MIME types offered by the source.
    pub offered_mimes: Vec<ClipboardMime>,
    /// Position where drag started.
    pub origin_x: i32,
    pub origin_y: i32,
    /// Visual feedback: ghost image dimensions (width, height).
    pub ghost_width: u32,
    pub ghost_height: u32,
}

/// Information about a drop target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct DropTarget {
    /// Surface ID that can receive drops.
    pub surface_id: u32,
    /// MIME types accepted by this target.
    pub accepted_mimes: Vec<ClipboardMime>,
    /// Bounding box (x, y, width, height).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "alloc")]
impl DropTarget {
    /// Check if a point is within this drop target's bounds.
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width as i32)
            && py >= self.y
            && py < self.y.saturating_add(self.height as i32)
    }
}

/// Drag-and-drop manager.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct DndManager {
    /// Current DnD state.
    state: DndState,
    /// Active drag source (if dragging).
    source: Option<DragSource>,
    /// Currently hovered surface.
    hover_surface: Option<u32>,
    /// Current cursor position during drag.
    cursor_x: i32,
    cursor_y: i32,
    /// Registered drop targets.
    targets: Vec<DropTarget>,
    /// Pending events.
    events: Vec<DndEvent>,
}

#[cfg(feature = "alloc")]
impl Default for DndManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl DndManager {
    /// Create a new DnD manager.
    pub fn new() -> Self {
        Self {
            state: DndState::Idle,
            source: None,
            hover_surface: None,
            cursor_x: 0,
            cursor_y: 0,
            targets: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Begin a drag operation.
    pub fn start_drag(
        &mut self,
        source_surface: u32,
        offered_mimes: Vec<ClipboardMime>,
        origin_x: i32,
        origin_y: i32,
        ghost_width: u32,
        ghost_height: u32,
    ) -> Result<(), DndError> {
        if self.state != DndState::Idle {
            return Err(DndError::AlreadyDragging);
        }

        self.source = Some(DragSource {
            source_surface,
            offered_mimes,
            origin_x,
            origin_y,
            ghost_width,
            ghost_height,
        });
        self.state = DndState::Dragging;
        self.cursor_x = origin_x;
        self.cursor_y = origin_y;

        Ok(())
    }

    /// Update cursor position during drag. Performs hit testing and emits
    /// events.
    pub fn motion(&mut self, x: i32, y: i32) -> Result<(), DndError> {
        if self.state != DndState::Dragging && self.state != DndState::DropPending {
            return Err(DndError::NotDragging);
        }

        self.cursor_x = x;
        self.cursor_y = y;

        // Hit-test against registered drop targets.
        let hit = self.targets.iter().find(|t| t.contains(x, y));

        let new_surface = hit.map(|t| t.surface_id);
        let old_surface = self.hover_surface;

        // Emit leave/enter events on surface change.
        if new_surface != old_surface {
            if let Some(old_id) = old_surface {
                self.events.push(DndEvent::Leave { surface_id: old_id });
            }
            if let Some(new_id) = new_surface {
                self.events.push(DndEvent::Enter {
                    surface_id: new_id,
                    x,
                    y,
                });
                self.state = DndState::DropPending;
            } else {
                self.state = DndState::Dragging;
            }
            self.hover_surface = new_surface;
        } else if let Some(sid) = new_surface {
            self.events.push(DndEvent::Motion {
                surface_id: sid,
                x,
                y,
            });
        }

        Ok(())
    }

    /// Perform a drop at the current position.
    pub fn drop_action(&mut self) -> Result<DndEvent, DndError> {
        if self.state != DndState::DropPending {
            return Err(DndError::NotDragging);
        }

        let surface_id = self.hover_surface.ok_or(DndError::InvalidSurface)?;
        let source = self.source.as_ref().ok_or(DndError::NotDragging)?;

        // Check MIME compatibility.
        let target = self
            .targets
            .iter()
            .find(|t| t.surface_id == surface_id)
            .ok_or(DndError::InvalidSurface)?;

        let has_match = source
            .offered_mimes
            .iter()
            .any(|m| target.accepted_mimes.contains(m));

        if !has_match {
            self.cancel();
            return Err(DndError::NoMimeMatch);
        }

        let event = DndEvent::Drop {
            surface_id,
            x: self.cursor_x,
            y: self.cursor_y,
        };
        self.events.push(event);

        self.state = DndState::Transferring;

        Ok(event)
    }

    /// Cancel the current drag operation.
    pub fn cancel(&mut self) {
        if let Some(sid) = self.hover_surface.take() {
            self.events.push(DndEvent::Leave { surface_id: sid });
        }
        self.events.push(DndEvent::Cancelled);
        self.source = None;
        self.state = DndState::Idle;
    }

    /// Complete the data transfer (called after successful drop).
    pub fn finish_transfer(&mut self) {
        self.source = None;
        self.hover_surface = None;
        self.state = DndState::Idle;
    }

    /// Register a drop target.
    pub fn register_target(&mut self, target: DropTarget) {
        // Remove existing target with same surface ID.
        self.targets.retain(|t| t.surface_id != target.surface_id);
        self.targets.push(target);
    }

    /// Unregister a drop target.
    pub fn unregister_target(&mut self, surface_id: u32) {
        self.targets.retain(|t| t.surface_id != surface_id);
    }

    /// Get current DnD state.
    pub fn state(&self) -> DndState {
        self.state
    }

    /// Get cursor position during drag.
    pub fn cursor_position(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
    }

    /// Get the active drag source info.
    pub fn source(&self) -> Option<&DragSource> {
        self.source.as_ref()
    }

    /// Get the ghost image position (centered on cursor).
    pub fn ghost_position(&self) -> Option<(i32, i32, u32, u32)> {
        self.source.as_ref().map(|s| {
            (
                self.cursor_x - (s.ghost_width as i32 / 2),
                self.cursor_y - (s.ghost_height as i32 / 2),
                s.ghost_width,
                s.ghost_height,
            )
        })
    }

    /// Drain pending events.
    pub fn drain_events(&mut self) -> Vec<DndEvent> {
        core::mem::take(&mut self.events)
    }

    /// Find the best matching MIME between source and a specific target.
    pub fn negotiate_mime(&self, surface_id: u32) -> Option<ClipboardMime> {
        let source = self.source.as_ref()?;
        let target = self.targets.iter().find(|t| t.surface_id == surface_id)?;
        source
            .offered_mimes
            .iter()
            .find(|m| target.accepted_mimes.contains(m))
            .copied()
    }
}
