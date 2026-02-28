//! Wayland Output Management (wl_output v4)
//!
//! Manages display outputs for multi-monitor support and HiDPI scaling.
//! Each output represents a physical or virtual display with its own
//! resolution, position, scale factor, and mode list.
//!
//! ## Multi-Monitor Layout
//!
//! Outputs are arranged in a global compositor coordinate space. Each
//! output has an (x, y) position representing its top-left corner in
//! this space. Adjacent outputs share edges (e.g., a right-side monitor
//! has x = left_monitor.width). The compositor maps surfaces to outputs
//! based on surface position overlap.
//!
//! ## HiDPI Scaling
//!
//! Each output has an integer scale factor (1 = normal, 2 = HiDPI/Retina).
//! Surfaces rendered for a scaled output should produce pixels at
//! `scale * logical_size`. The compositor handles downscaling when
//! displaying on lower-scale outputs.
//!
//! ## Hotplug
//!
//! Outputs can be added and removed at runtime. When an output is added,
//! a `wl_output` global is advertised to all connected clients. When
//! removed, the global is withdrawn and any surfaces on that output
//! should be moved to the primary output.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Output transform
// ---------------------------------------------------------------------------

/// Output transform applied to the output's content.
///
/// Matches the Wayland wl_output.transform enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum OutputTransform {
    /// No transform
    Normal = 0,
    /// 90 degrees counter-clockwise
    Rotate90 = 1,
    /// 180 degrees
    Rotate180 = 2,
    /// 270 degrees counter-clockwise (90 clockwise)
    Rotate270 = 3,
    /// Horizontal flip
    Flipped = 4,
    /// Flip + 90 degrees counter-clockwise
    FlippedRotate90 = 5,
    /// Flip + 180 degrees
    FlippedRotate180 = 6,
    /// Flip + 270 degrees counter-clockwise
    FlippedRotate270 = 7,
}

impl OutputTransform {
    /// Create from Wayland protocol value.
    pub fn from_wl(value: u32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::Rotate90,
            2 => Self::Rotate180,
            3 => Self::Rotate270,
            4 => Self::Flipped,
            5 => Self::FlippedRotate90,
            6 => Self::FlippedRotate180,
            7 => Self::FlippedRotate270,
            _ => Self::Normal,
        }
    }

    /// Returns true if the transform includes a 90 or 270 degree rotation,
    /// which swaps width and height.
    pub fn swaps_dimensions(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::FlippedRotate90 | Self::FlippedRotate270
        )
    }
}

// ---------------------------------------------------------------------------
// Subpixel layout
// ---------------------------------------------------------------------------

/// Subpixel geometry of the display panel.
///
/// Used by font renderers for sub-pixel anti-aliasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SubpixelLayout {
    /// Unknown or no subpixel information
    Unknown = 0,
    /// No subpixel rendering (e.g., CRT or projector)
    None = 1,
    /// Horizontal RGB subpixels (most common LCD)
    HorizontalRgb = 2,
    /// Horizontal BGR subpixels
    HorizontalBgr = 3,
    /// Vertical RGB subpixels
    VerticalRgb = 4,
    /// Vertical BGR subpixels
    VerticalBgr = 5,
}

impl SubpixelLayout {
    /// Create from Wayland protocol value.
    pub fn from_wl(value: u32) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::None,
            2 => Self::HorizontalRgb,
            3 => Self::HorizontalBgr,
            4 => Self::VerticalRgb,
            5 => Self::VerticalBgr,
            _ => Self::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Output mode
// ---------------------------------------------------------------------------

/// A display mode supported by an output.
///
/// Each output can support multiple modes (resolutions and refresh rates).
/// Exactly one mode should be marked as `current`, and one as `preferred`.
#[derive(Debug, Clone)]
pub struct OutputMode {
    /// Horizontal resolution in pixels
    pub width: u32,
    /// Vertical resolution in pixels
    pub height: u32,
    /// Refresh rate in millihertz (e.g., 60000 = 60Hz)
    pub refresh_mhz: u32,
    /// Whether this is the preferred (native) mode
    pub preferred: bool,
    /// Whether this is the currently active mode
    pub current: bool,
}

impl OutputMode {
    /// Create a new output mode.
    pub fn new(width: u32, height: u32, refresh_mhz: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz,
            preferred: false,
            current: false,
        }
    }

    /// Create a mode marked as both current and preferred.
    pub fn new_current_preferred(width: u32, height: u32, refresh_mhz: u32) -> Self {
        Self {
            width,
            height,
            refresh_mhz,
            preferred: true,
            current: true,
        }
    }

    /// Get the Wayland mode flags (bitmask).
    pub fn wl_flags(&self) -> u32 {
        let mut flags = 0u32;
        if self.current {
            flags |= 0x1; // WL_OUTPUT_MODE_CURRENT
        }
        if self.preferred {
            flags |= 0x2; // WL_OUTPUT_MODE_PREFERRED
        }
        flags
    }

    /// Pixel count for this mode.
    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// A display output (physical monitor or virtual display).
pub struct Output {
    /// Unique output ID
    pub id: u32,
    /// Human-readable output name (e.g., "HDMI-A-1", "eDP-1")
    pub name: String,
    /// Description of the output (e.g., "Dell U2720Q")
    pub description: String,
    /// Manufacturer name
    pub make: String,
    /// Model name
    pub model: String,
    /// Position in global compositor coordinate space
    pub x: i32,
    pub y: i32,
    /// Physical width in millimeters (0 = unknown)
    pub physical_width_mm: u32,
    /// Physical height in millimeters (0 = unknown)
    pub physical_height_mm: u32,
    /// Subpixel layout
    pub subpixel: SubpixelLayout,
    /// Applied transform (rotation/flip)
    pub transform: OutputTransform,
    /// Integer scale factor (1 = normal, 2 = HiDPI)
    pub scale: u32,
    /// Supported display modes
    pub modes: Vec<OutputMode>,
    /// Whether this output is enabled
    pub enabled: bool,
}

impl Output {
    /// Create a new output with default settings.
    pub fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: String::from(name),
            description: String::new(),
            make: String::from("VeridianOS"),
            model: String::from("Virtual Display"),
            x: 0,
            y: 0,
            physical_width_mm: 0,
            physical_height_mm: 0,
            subpixel: SubpixelLayout::Unknown,
            transform: OutputTransform::Normal,
            scale: 1,
            modes: Vec::new(),
            enabled: true,
        }
    }

    /// Create a virtual output with a single mode.
    pub fn new_virtual(id: u32, name: &str, width: u32, height: u32) -> Self {
        let mut output = Self::new(id, name);
        output
            .modes
            .push(OutputMode::new_current_preferred(width, height, 60000));
        output
    }

    /// Get the current mode (if any).
    pub fn current_mode(&self) -> Option<&OutputMode> {
        self.modes.iter().find(|m| m.current)
    }

    /// Get the logical width (after transform and scale).
    pub fn logical_width(&self) -> u32 {
        let mode = match self.current_mode() {
            Some(m) => m,
            None => return 0,
        };
        let (w, h) = if self.transform.swaps_dimensions() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        let _ = h; // suppress unused warning
        w / self.scale
    }

    /// Get the logical height (after transform and scale).
    pub fn logical_height(&self) -> u32 {
        let mode = match self.current_mode() {
            Some(m) => m,
            None => return 0,
        };
        let (w, h) = if self.transform.swaps_dimensions() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        let _ = w; // suppress unused warning
        h / self.scale
    }

    /// Get the physical pixel width (current mode, no scaling).
    pub fn pixel_width(&self) -> u32 {
        self.current_mode().map(|m| m.width).unwrap_or(0)
    }

    /// Get the physical pixel height (current mode, no scaling).
    pub fn pixel_height(&self) -> u32 {
        self.current_mode().map(|m| m.height).unwrap_or(0)
    }

    /// Calculate DPI from physical dimensions and pixel resolution.
    ///
    /// Returns (dpi_x, dpi_y) or (0, 0) if physical dimensions are unknown.
    pub fn dpi(&self) -> (u32, u32) {
        if self.physical_width_mm == 0 || self.physical_height_mm == 0 {
            return (0, 0);
        }
        let mode = match self.current_mode() {
            Some(m) => m,
            None => return (0, 0),
        };
        // DPI = pixels / (mm / 25.4)
        let dpi_x = (mode.width * 254) / (self.physical_width_mm * 10);
        let dpi_y = (mode.height * 254) / (self.physical_height_mm * 10);
        (dpi_x, dpi_y)
    }

    /// Check if a point (in global compositor coordinates) falls within
    /// this output's logical area.
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        let w = self.logical_width() as i32;
        let h = self.logical_height() as i32;
        px >= self.x && px < self.x + w && py >= self.y && py < self.y + h
    }

    /// Get the bounding rectangle (x, y, width, height) in global coords.
    pub fn bounds(&self) -> (i32, i32, u32, u32) {
        (self.x, self.y, self.logical_width(), self.logical_height())
    }

    /// Set the active mode by index. Marks all other modes as non-current.
    pub fn set_current_mode(&mut self, index: usize) -> Result<(), KernelError> {
        if index >= self.modes.len() {
            return Err(KernelError::InvalidArgument {
                name: "mode_index",
                value: "out of range",
            });
        }
        for mode in self.modes.iter_mut() {
            mode.current = false;
        }
        self.modes[index].current = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Output manager
// ---------------------------------------------------------------------------

/// Manages all display outputs and their configuration.
///
/// The output manager tracks the global coordinate layout, handles hotplug
/// events, and provides queries for surface-to-output mapping.
pub struct OutputManager {
    /// All outputs keyed by ID
    outputs: BTreeMap<u32, Output>,
    /// Next output ID
    next_id: AtomicU32,
    /// ID of the primary output (receives new windows by default)
    primary_output: Option<u32>,
}

impl OutputManager {
    /// Create a new output manager with no outputs.
    pub fn new() -> Self {
        Self {
            outputs: BTreeMap::new(),
            next_id: AtomicU32::new(1),
            primary_output: None,
        }
    }

    /// Create an output manager with a single virtual output matching the
    /// framebuffer dimensions.
    pub fn new_with_framebuffer(width: u32, height: u32) -> Self {
        let mut manager = Self::new();
        let id = manager.next_id.fetch_add(1, Ordering::Relaxed);
        let output = Output::new_virtual(id, "FBCON-1", width, height);
        manager.outputs.insert(id, output);
        manager.primary_output = Some(id);
        manager
    }

    /// Add a new output to the manager. Returns the assigned output ID.
    pub fn add_output(&mut self, mut output: Output) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        output.id = id;

        // If this is the first output, make it primary
        if self.primary_output.is_none() {
            self.primary_output = Some(id);
        }

        self.outputs.insert(id, output);
        id
    }

    /// Remove an output by ID.
    ///
    /// If the removed output was primary, the next available output becomes
    /// primary. Returns the removed output if it existed.
    pub fn remove_output(&mut self, id: u32) -> Option<Output> {
        let output = self.outputs.remove(&id);

        // Update primary if we removed it
        if self.primary_output == Some(id) {
            self.primary_output = self.outputs.keys().next().copied();
        }

        output
    }

    /// Get a reference to an output by ID.
    pub fn get_output(&self, id: u32) -> Option<&Output> {
        self.outputs.get(&id)
    }

    /// Get a mutable reference to an output by ID.
    pub fn get_output_mut(&mut self, id: u32) -> Option<&mut Output> {
        self.outputs.get_mut(&id)
    }

    /// Get the primary output.
    pub fn get_primary(&self) -> Option<&Output> {
        self.primary_output.and_then(|id| self.outputs.get(&id))
    }

    /// Get the primary output ID.
    pub fn get_primary_id(&self) -> Option<u32> {
        self.primary_output
    }

    /// Set the primary output.
    pub fn set_primary(&mut self, id: u32) -> Result<(), KernelError> {
        if !self.outputs.contains_key(&id) {
            return Err(KernelError::NotFound {
                resource: "output",
                id: id as u64,
            });
        }
        self.primary_output = Some(id);
        Ok(())
    }

    /// Get all outputs as a list of references.
    pub fn get_all_outputs(&self) -> Vec<&Output> {
        self.outputs.values().collect()
    }

    /// Get the number of active outputs.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Calculate the total bounding rectangle across all outputs.
    ///
    /// Returns (min_x, min_y, total_width, total_height) in global coords.
    pub fn get_total_area(&self) -> (i32, i32, u32, u32) {
        if self.outputs.is_empty() {
            return (0, 0, 0, 0);
        }

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for output in self.outputs.values() {
            if !output.enabled {
                continue;
            }
            let (ox, oy, ow, oh) = output.bounds();
            if ox < min_x {
                min_x = ox;
            }
            if oy < min_y {
                min_y = oy;
            }
            let right = ox + ow as i32;
            let bottom = oy + oh as i32;
            if right > max_x {
                max_x = right;
            }
            if bottom > max_y {
                max_y = bottom;
            }
        }

        if min_x == i32::MAX {
            return (0, 0, 0, 0);
        }

        (min_x, min_y, (max_x - min_x) as u32, (max_y - min_y) as u32)
    }

    /// Find the output at a given point in global coordinates.
    ///
    /// Returns the output ID if found.
    pub fn get_output_at_point(&self, x: i32, y: i32) -> Option<u32> {
        for output in self.outputs.values() {
            if output.enabled && output.contains_point(x, y) {
                return Some(output.id);
            }
        }
        None
    }

    /// Set the scale factor for an output.
    pub fn set_scale(&mut self, id: u32, scale: u32) -> Result<(), KernelError> {
        let output = self.outputs.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "output",
            id: id as u64,
        })?;

        if scale == 0 {
            return Err(KernelError::InvalidArgument {
                name: "scale",
                value: "must be >= 1",
            });
        }

        output.scale = scale;
        Ok(())
    }

    /// Set the position of an output in global coordinates.
    pub fn set_position(&mut self, id: u32, x: i32, y: i32) -> Result<(), KernelError> {
        let output = self.outputs.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "output",
            id: id as u64,
        })?;
        output.x = x;
        output.y = y;
        Ok(())
    }

    /// Set the transform for an output.
    pub fn set_transform(
        &mut self,
        id: u32,
        transform: OutputTransform,
    ) -> Result<(), KernelError> {
        let output = self.outputs.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "output",
            id: id as u64,
        })?;
        output.transform = transform;
        Ok(())
    }

    /// Handle a hotplug event: add a new output to the right of existing
    /// outputs.
    ///
    /// Returns the assigned output ID.
    pub fn handle_hotplug(&mut self, output: Output) -> u32 {
        // Calculate position: to the right of all existing outputs
        let (_, _, total_w, _) = self.get_total_area();
        let mut new_output = output;
        new_output.x = total_w as i32;
        new_output.y = 0;
        self.add_output(new_output)
    }

    /// Arrange outputs side by side (left to right) in the order they
    /// were added.
    pub fn arrange_horizontal(&mut self) {
        let mut x_offset = 0i32;
        let ids: Vec<u32> = self.outputs.keys().copied().collect();
        for id in ids {
            if let Some(output) = self.outputs.get_mut(&id) {
                if !output.enabled {
                    continue;
                }
                output.x = x_offset;
                output.y = 0;
                x_offset += output.logical_width() as i32;
            }
        }
    }

    /// Arrange outputs vertically (top to bottom).
    pub fn arrange_vertical(&mut self) {
        let mut y_offset = 0i32;
        let ids: Vec<u32> = self.outputs.keys().copied().collect();
        for id in ids {
            if let Some(output) = self.outputs.get_mut(&id) {
                if !output.enabled {
                    continue;
                }
                output.x = 0;
                output.y = y_offset;
                y_offset += output.logical_height() as i32;
            }
        }
    }

    /// Enable or disable an output.
    pub fn set_enabled(&mut self, id: u32, enabled: bool) -> Result<(), KernelError> {
        let output = self.outputs.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "output",
            id: id as u64,
        })?;
        output.enabled = enabled;

        // If we disabled the primary, pick a new one
        if !enabled && self.primary_output == Some(id) {
            self.primary_output = self
                .outputs
                .values()
                .find(|o| o.enabled && o.id != id)
                .map(|o| o.id);
        }
        Ok(())
    }

    /// Get the effective scale for a point in global coordinates.
    ///
    /// Returns the scale factor of the output containing the point,
    /// or 1 if no output contains the point.
    pub fn scale_at_point(&self, x: i32, y: i32) -> u32 {
        self.get_output_at_point(x, y)
            .and_then(|id| self.outputs.get(&id))
            .map(|o| o.scale)
            .unwrap_or(1)
    }

    /// Get all outputs that overlap with a given rectangle.
    pub fn outputs_for_rect(&self, x: i32, y: i32, w: u32, h: u32) -> Vec<u32> {
        let rect_right = x + w as i32;
        let rect_bottom = y + h as i32;
        let mut result = Vec::new();

        for output in self.outputs.values() {
            if !output.enabled {
                continue;
            }
            let (ox, oy, ow, oh) = output.bounds();
            let out_right = ox + ow as i32;
            let out_bottom = oy + oh as i32;

            // Standard rectangle overlap test
            if x < out_right && rect_right > ox && y < out_bottom && rect_bottom > oy {
                result.push(output.id);
            }
        }
        result
    }
}

impl Default for OutputManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_manager_basic() {
        let mut mgr = OutputManager::new();
        assert_eq!(mgr.output_count(), 0);

        let output = Output::new_virtual(0, "TEST-1", 1920, 1080);
        let id = mgr.add_output(output);
        assert_eq!(mgr.output_count(), 1);
        assert_eq!(mgr.get_primary_id(), Some(id));
    }

    #[test]
    fn test_output_manager_with_framebuffer() {
        let mgr = OutputManager::new_with_framebuffer(1280, 800);
        assert_eq!(mgr.output_count(), 1);
        let primary = mgr.get_primary().unwrap();
        assert_eq!(primary.pixel_width(), 1280);
        assert_eq!(primary.pixel_height(), 800);
    }

    #[test]
    fn test_total_area_multi_output() {
        let mut mgr = OutputManager::new();
        let out1 = Output::new_virtual(0, "LEFT", 1920, 1080);
        mgr.add_output(out1);
        let mut out2 = Output::new_virtual(0, "RIGHT", 1920, 1080);
        out2.x = 1920;
        mgr.add_output(out2);

        let (x, y, w, h) = mgr.get_total_area();
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 3840);
        assert_eq!(h, 1080);
    }

    #[test]
    fn test_output_at_point() {
        let mut mgr = OutputManager::new();
        let out1 = Output::new_virtual(0, "LEFT", 1920, 1080);
        let id1 = mgr.add_output(out1);
        let mut out2 = Output::new_virtual(0, "RIGHT", 1920, 1080);
        out2.x = 1920;
        let id2 = mgr.add_output(out2);

        assert_eq!(mgr.get_output_at_point(100, 100), Some(id1));
        assert_eq!(mgr.get_output_at_point(2000, 100), Some(id2));
        assert_eq!(mgr.get_output_at_point(5000, 100), None);
    }

    #[test]
    fn test_hidpi_scale() {
        let mut mgr = OutputManager::new();
        let output = Output::new_virtual(0, "HIDPI", 3840, 2160);
        let id = mgr.add_output(output);
        mgr.set_scale(id, 2).unwrap();

        let out = mgr.get_output(id).unwrap();
        assert_eq!(out.logical_width(), 1920);
        assert_eq!(out.logical_height(), 1080);
        assert_eq!(out.pixel_width(), 3840);
        assert_eq!(out.pixel_height(), 2160);
    }

    #[test]
    fn test_output_transform_swap() {
        assert!(!OutputTransform::Normal.swaps_dimensions());
        assert!(OutputTransform::Rotate90.swaps_dimensions());
        assert!(!OutputTransform::Rotate180.swaps_dimensions());
        assert!(OutputTransform::Rotate270.swaps_dimensions());
    }

    #[test]
    fn test_remove_primary() {
        let mut mgr = OutputManager::new();
        let out1 = Output::new_virtual(0, "A", 1920, 1080);
        let id1 = mgr.add_output(out1);
        let out2 = Output::new_virtual(0, "B", 1920, 1080);
        let id2 = mgr.add_output(out2);

        assert_eq!(mgr.get_primary_id(), Some(id1));
        mgr.remove_output(id1);
        assert_eq!(mgr.get_primary_id(), Some(id2));
    }

    #[test]
    fn test_outputs_for_rect() {
        let mut mgr = OutputManager::new();
        let out1 = Output::new_virtual(0, "LEFT", 1920, 1080);
        let id1 = mgr.add_output(out1);
        let mut out2 = Output::new_virtual(0, "RIGHT", 1920, 1080);
        out2.x = 1920;
        let id2 = mgr.add_output(out2);

        // Rect spanning both outputs
        let ids = mgr.outputs_for_rect(1800, 0, 240, 100);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));

        // Rect on left only
        let ids = mgr.outputs_for_rect(0, 0, 100, 100);
        assert!(ids.contains(&id1));
        assert!(!ids.contains(&id2));
    }

    #[test]
    fn test_output_dpi() {
        let mut output = Output::new_virtual(1, "TEST", 3840, 2160);
        output.physical_width_mm = 600; // ~24 inches wide
        output.physical_height_mm = 340;

        let (dpi_x, dpi_y) = output.dpi();
        // 3840 / (600/25.4) = 3840 / 23.6 = ~162 DPI
        assert!(dpi_x > 150 && dpi_x < 170);
        assert!(dpi_y > 150 && dpi_y < 170);
    }
}
