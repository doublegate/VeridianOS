//! Multi-Output Display Manager
//!
//! Manages multiple display outputs for multi-monitor configurations.
//! Coordinates output enumeration, positioning, per-output page flips,
//! and hotplug events for DRM connectors.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of simultaneous display outputs
const MAX_OUTPUTS: usize = 8;

/// Default refresh rate in millihertz (60 Hz)
const DEFAULT_REFRESH_MHZ: u32 = 60000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Represents a single display output (physical monitor/connector)
#[derive(Debug, Clone, Copy)]
pub struct DisplayOutput {
    /// Unique output identifier
    pub id: u32,
    /// DRM connector ID
    pub connector_id: u32,
    /// DRM CRTC ID assigned to this output
    pub crtc_id: u32,
    /// Display width in pixels
    pub width: u32,
    /// Display height in pixels
    pub height: u32,
    /// Refresh rate in millihertz
    pub refresh_hz: u32,
    /// X offset in virtual desktop coordinates
    pub x_offset: i32,
    /// Y offset in virtual desktop coordinates
    pub y_offset: i32,
    /// Whether this output is enabled
    pub enabled: bool,
    /// Whether this is the primary output
    pub primary: bool,
    /// DRM connector connection status
    pub connected: bool,
    /// Physical width in mm (from EDID)
    pub physical_width_mm: u32,
    /// Physical height in mm (from EDID)
    pub physical_height_mm: u32,
}

impl Default for DisplayOutput {
    fn default() -> Self {
        Self {
            id: 0,
            connector_id: 0,
            crtc_id: 0,
            width: 0,
            height: 0,
            refresh_hz: DEFAULT_REFRESH_MHZ,
            x_offset: 0,
            y_offset: 0,
            enabled: false,
            primary: false,
            connected: false,
            physical_width_mm: 0,
            physical_height_mm: 0,
        }
    }
}

impl DisplayOutput {
    /// Get the right edge of this output in virtual desktop coordinates
    pub fn right_edge(&self) -> i32 {
        self.x_offset.saturating_add(self.width as i32)
    }

    /// Get the bottom edge of this output in virtual desktop coordinates
    pub fn bottom_edge(&self) -> i32 {
        self.y_offset.saturating_add(self.height as i32)
    }

    /// Check if a point falls within this output
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        if !self.enabled {
            return false;
        }
        x >= self.x_offset && x < self.right_edge() && y >= self.y_offset && y < self.bottom_edge()
    }
}

/// Multi-output display manager
pub struct MultiOutputManager {
    /// Array of display outputs
    outputs: [DisplayOutput; MAX_OUTPUTS],
    /// Number of active outputs
    num_outputs: usize,
    /// Next output ID to assign
    next_id: u32,
    /// Total virtual desktop width
    total_width: u32,
    /// Total virtual desktop height
    total_height: u32,
    /// Whether the manager has been initialized
    initialized: bool,
}

impl Default for MultiOutputManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiOutputManager {
    /// Create a new multi-output manager
    pub const fn new() -> Self {
        const DEFAULT: DisplayOutput = DisplayOutput {
            id: 0,
            connector_id: 0,
            crtc_id: 0,
            width: 0,
            height: 0,
            refresh_hz: DEFAULT_REFRESH_MHZ,
            x_offset: 0,
            y_offset: 0,
            enabled: false,
            primary: false,
            connected: false,
            physical_width_mm: 0,
            physical_height_mm: 0,
        };
        Self {
            outputs: [DEFAULT; MAX_OUTPUTS],
            num_outputs: 0,
            next_id: 1,
            total_width: 0,
            total_height: 0,
            initialized: false,
        }
    }

    /// Initialize the multi-output manager
    pub fn init(&mut self) {
        self.num_outputs = 0;
        self.next_id = 1;
        self.total_width = 0;
        self.total_height = 0;
        self.initialized = true;
    }

    /// Add a new display output
    ///
    /// The output is placed to the right of all existing outputs by default.
    pub fn add_output(
        &mut self,
        connector_id: u32,
        crtc_id: u32,
        width: u32,
        height: u32,
        refresh_hz: u32,
    ) -> Result<u32, KernelError> {
        if self.num_outputs >= MAX_OUTPUTS {
            return Err(KernelError::ResourceExhausted {
                resource: "display outputs",
            });
        }

        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        // Place output to the right of all existing outputs
        let x_offset = self.total_width as i32;
        let is_primary = self.num_outputs == 0;

        self.outputs[self.num_outputs] = DisplayOutput {
            id,
            connector_id,
            crtc_id,
            width,
            height,
            refresh_hz,
            x_offset,
            y_offset: 0,
            enabled: true,
            primary: is_primary,
            connected: true,
            physical_width_mm: 0,
            physical_height_mm: 0,
        };

        self.num_outputs += 1;
        self.recalculate_total_size();

        Ok(id)
    }

    /// Remove a display output by ID
    pub fn remove_output(&mut self, output_id: u32) -> Result<(), KernelError> {
        let idx = self.find_output_index(output_id)?;

        // Shift remaining outputs down
        for i in idx..self.num_outputs.saturating_sub(1) {
            self.outputs[i] = self.outputs[i + 1];
        }

        if self.num_outputs > 0 {
            self.num_outputs -= 1;
            self.outputs[self.num_outputs] = DisplayOutput::default();
        }

        // If we removed the primary, promote the first remaining output
        if self.num_outputs > 0 {
            let has_primary = self.outputs[..self.num_outputs].iter().any(|o| o.primary);
            if !has_primary {
                self.outputs[0].primary = true;
            }
        }

        self.recalculate_total_size();
        Ok(())
    }

    /// Set the position of an output in virtual desktop coordinates
    pub fn set_position(&mut self, output_id: u32, x: i32, y: i32) -> Result<(), KernelError> {
        let idx = self.find_output_index(output_id)?;
        self.outputs[idx].x_offset = x;
        self.outputs[idx].y_offset = y;
        self.recalculate_total_size();
        Ok(())
    }

    /// Set the primary output
    pub fn set_primary(&mut self, output_id: u32) -> Result<(), KernelError> {
        let idx = self.find_output_index(output_id)?;

        // Clear primary on all outputs
        for i in 0..self.num_outputs {
            self.outputs[i].primary = false;
        }

        self.outputs[idx].primary = true;
        Ok(())
    }

    /// Get list of active outputs
    pub fn get_outputs(&self) -> &[DisplayOutput] {
        &self.outputs[..self.num_outputs]
    }

    /// Get total virtual desktop size
    pub fn get_total_size(&self) -> (u32, u32) {
        (self.total_width, self.total_height)
    }

    /// Map a point in virtual desktop coordinates to a specific output
    ///
    /// Returns (output_id, local_x, local_y) or None if the point is
    /// outside all outputs.
    pub fn point_to_output(&self, x: i32, y: i32) -> Option<(u32, i32, i32)> {
        for i in 0..self.num_outputs {
            let output = &self.outputs[i];
            if output.enabled && output.contains_point(x, y) {
                let local_x = x - output.x_offset;
                let local_y = y - output.y_offset;
                return Some((output.id, local_x, local_y));
            }
        }
        None
    }

    /// Get the primary output
    pub fn primary_output(&self) -> Option<&DisplayOutput> {
        self.outputs[..self.num_outputs].iter().find(|o| o.primary)
    }

    /// Get a specific output by ID
    pub fn get_output(&self, output_id: u32) -> Option<&DisplayOutput> {
        self.outputs[..self.num_outputs]
            .iter()
            .find(|o| o.id == output_id)
    }

    /// Get a mutable reference to a specific output by ID
    pub fn get_output_mut(&mut self, output_id: u32) -> Option<&mut DisplayOutput> {
        self.outputs[..self.num_outputs]
            .iter_mut()
            .find(|o| o.id == output_id)
    }

    /// Schedule a page flip on a specific output
    ///
    /// In a real implementation this would issue a DRM page flip ioctl.
    pub fn flip(&self, output_id: u32, _buffer_id: u32) -> Result<(), KernelError> {
        let _output = self.get_output(output_id).ok_or(KernelError::NotFound {
            resource: "output",
            id: output_id as u64,
        })?;
        // Page flip via DRM would happen here
        Ok(())
    }

    /// Handle a DRM connector hotplug event
    ///
    /// If connected=true and the connector is new, adds an output.
    /// If connected=false, removes the associated output.
    pub fn handle_hotplug(
        &mut self,
        connector_id: u32,
        connected: bool,
        width: u32,
        height: u32,
        refresh_hz: u32,
    ) -> Result<(), KernelError> {
        if connected {
            // Check if we already have this connector
            let existing = self.outputs[..self.num_outputs]
                .iter()
                .any(|o| o.connector_id == connector_id);

            if !existing {
                self.add_output(connector_id, 0, width, height, refresh_hz)?;
                crate::println!(
                    "[MULTI-OUTPUT] Connector {} connected ({}x{}@{}Hz)",
                    connector_id,
                    width,
                    height,
                    refresh_hz / 1000
                );
            }
        } else {
            // Find and remove the output for this connector
            if let Some(output_id) = self.outputs[..self.num_outputs]
                .iter()
                .find(|o| o.connector_id == connector_id)
                .map(|o| o.id)
            {
                self.remove_output(output_id)?;
                crate::println!("[MULTI-OUTPUT] Connector {} disconnected", connector_id);
            }
        }
        Ok(())
    }

    /// Arrange outputs left-to-right based on their current order
    pub fn auto_layout(&mut self) {
        let mut x_offset: i32 = 0;
        for i in 0..self.num_outputs {
            self.outputs[i].x_offset = x_offset;
            self.outputs[i].y_offset = 0;
            x_offset = x_offset.saturating_add(self.outputs[i].width as i32);
        }
        self.recalculate_total_size();
    }

    /// Get the number of active outputs
    pub fn num_outputs(&self) -> usize {
        self.num_outputs
    }

    /// Check if the manager is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    // ----- Private helpers -----

    /// Find the index of an output by ID
    fn find_output_index(&self, output_id: u32) -> Result<usize, KernelError> {
        for i in 0..self.num_outputs {
            if self.outputs[i].id == output_id {
                return Ok(i);
            }
        }
        Err(KernelError::NotFound {
            resource: "output",
            id: output_id as u64,
        })
    }

    /// Recalculate the total virtual desktop size
    fn recalculate_total_size(&mut self) {
        let mut max_right: i32 = 0;
        let mut max_bottom: i32 = 0;

        for i in 0..self.num_outputs {
            let output = &self.outputs[i];
            if output.enabled {
                let right = output.right_edge();
                let bottom = output.bottom_edge();
                if right > max_right {
                    max_right = right;
                }
                if bottom > max_bottom {
                    max_bottom = bottom;
                }
            }
        }

        self.total_width = if max_right > 0 { max_right as u32 } else { 0 };
        self.total_height = if max_bottom > 0 { max_bottom as u32 } else { 0 };
    }
}

// ---------------------------------------------------------------------------
// Global Multi-Output Manager
// ---------------------------------------------------------------------------

static MULTI_OUTPUT: Mutex<MultiOutputManager> = Mutex::new(MultiOutputManager::new());

static MULTI_OUTPUT_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the multi-output display manager
pub fn multi_output_init() {
    let mut mgr = MULTI_OUTPUT.lock();
    mgr.init();
    MULTI_OUTPUT_INITIALIZED.store(true, Ordering::Release);
    crate::println!(
        "[MULTI-OUTPUT] Display manager initialized (max {} outputs)",
        MAX_OUTPUTS
    );
}

/// Add a display output
pub fn multi_output_add(
    connector_id: u32,
    crtc_id: u32,
    width: u32,
    height: u32,
    refresh_hz: u32,
) -> Result<u32, KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    let mut mgr = MULTI_OUTPUT.lock();
    let id = mgr.add_output(connector_id, crtc_id, width, height, refresh_hz)?;
    // Update CRTC ID on the output
    if let Some(output) = mgr.get_output_mut(id) {
        output.crtc_id = crtc_id;
    }
    Ok(id)
}

/// Remove a display output
pub fn multi_output_remove(output_id: u32) -> Result<(), KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    MULTI_OUTPUT.lock().remove_output(output_id)
}

/// Set output position in virtual desktop
pub fn multi_output_set_position(output_id: u32, x: i32, y: i32) -> Result<(), KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    MULTI_OUTPUT.lock().set_position(output_id, x, y)
}

/// Set the primary display output
pub fn multi_output_set_primary(output_id: u32) -> Result<(), KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    MULTI_OUTPUT.lock().set_primary(output_id)
}

/// Get total virtual desktop size
pub fn multi_output_get_total_size() -> (u32, u32) {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return (0, 0);
    }
    MULTI_OUTPUT.lock().get_total_size()
}

/// Map a point to a specific output
pub fn multi_output_point_to_output(x: i32, y: i32) -> Option<(u32, i32, i32)> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return None;
    }
    MULTI_OUTPUT.lock().point_to_output(x, y)
}

/// Handle a DRM connector hotplug event
pub fn multi_output_handle_hotplug(
    connector_id: u32,
    connected: bool,
    width: u32,
    height: u32,
    refresh_hz: u32,
) -> Result<(), KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    MULTI_OUTPUT
        .lock()
        .handle_hotplug(connector_id, connected, width, height, refresh_hz)
}

/// Schedule a page flip on a specific output
pub fn multi_output_flip(output_id: u32, buffer_id: u32) -> Result<(), KernelError> {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "multi_output",
        });
    }
    MULTI_OUTPUT.lock().flip(output_id, buffer_id)
}

/// Get number of active outputs
pub fn multi_output_count() -> usize {
    if !MULTI_OUTPUT_INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    MULTI_OUTPUT.lock().num_outputs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_output_default() {
        let output = DisplayOutput::default();
        assert!(!output.enabled);
        assert!(!output.primary);
        assert_eq!(output.width, 0);
        assert_eq!(output.height, 0);
    }

    #[test]
    fn test_display_output_contains_point() {
        let mut output = DisplayOutput::default();
        output.enabled = true;
        output.width = 1920;
        output.height = 1080;
        output.x_offset = 100;
        output.y_offset = 50;

        assert!(output.contains_point(100, 50));
        assert!(output.contains_point(500, 500));
        assert!(output.contains_point(2019, 1129));
        assert!(!output.contains_point(99, 50));
        assert!(!output.contains_point(100, 49));
        assert!(!output.contains_point(2020, 500));
    }

    #[test]
    fn test_display_output_disabled_no_contains() {
        let mut output = DisplayOutput::default();
        output.width = 1920;
        output.height = 1080;
        // Not enabled
        assert!(!output.contains_point(500, 500));
    }

    #[test]
    fn test_display_output_edges() {
        let mut output = DisplayOutput::default();
        output.x_offset = 100;
        output.y_offset = 200;
        output.width = 1920;
        output.height = 1080;

        assert_eq!(output.right_edge(), 2020);
        assert_eq!(output.bottom_edge(), 1280);
    }

    #[test]
    fn test_multi_output_manager_new() {
        let mgr = MultiOutputManager::new();
        assert!(!mgr.is_initialized());
        assert_eq!(mgr.num_outputs(), 0);
    }

    #[test]
    fn test_multi_output_manager_init() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();
        assert!(mgr.is_initialized());
        assert_eq!(mgr.num_outputs(), 0);
        assert_eq!(mgr.get_total_size(), (0, 0));
    }

    #[test]
    fn test_add_output() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        assert_eq!(mgr.num_outputs(), 1);
        assert_eq!(mgr.get_total_size(), (1920, 1080));

        let output = mgr.get_output(id).unwrap();
        assert!(output.primary);
        assert!(output.enabled);
        assert_eq!(output.x_offset, 0);
    }

    #[test]
    fn test_add_two_outputs() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        assert_eq!(mgr.num_outputs(), 2);
        assert_eq!(mgr.get_total_size(), (4480, 1440));

        let o1 = mgr.get_output(id1).unwrap();
        assert_eq!(o1.x_offset, 0);
        assert!(o1.primary);

        let o2 = mgr.get_output(id2).unwrap();
        assert_eq!(o2.x_offset, 1920);
        assert!(!o2.primary);
    }

    #[test]
    fn test_remove_output() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let _id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        mgr.remove_output(id1).unwrap();
        assert_eq!(mgr.num_outputs(), 1);
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        assert!(mgr.remove_output(999).is_err());
    }

    #[test]
    fn test_max_outputs() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        for i in 0..MAX_OUTPUTS {
            assert!(mgr
                .add_output(i as u32, i as u32 * 10, 1920, 1080, 60000)
                .is_ok());
        }
        // 9th should fail
        assert!(mgr.add_output(99, 990, 1920, 1080, 60000).is_err());
    }

    #[test]
    fn test_set_position() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        mgr.set_position(id, 500, 300).unwrap();

        let output = mgr.get_output(id).unwrap();
        assert_eq!(output.x_offset, 500);
        assert_eq!(output.y_offset, 300);
    }

    #[test]
    fn test_set_primary() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        mgr.set_primary(id2).unwrap();

        assert!(!mgr.get_output(id1).unwrap().primary);
        assert!(mgr.get_output(id2).unwrap().primary);
    }

    #[test]
    fn test_point_to_output() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        // Point in first output
        let result = mgr.point_to_output(500, 500);
        assert!(result.is_some());
        let (oid, lx, ly) = result.unwrap();
        assert_eq!(oid, id1);
        assert_eq!(lx, 500);
        assert_eq!(ly, 500);

        // Point in second output
        let result = mgr.point_to_output(2000, 500);
        assert!(result.is_some());
        let (oid, lx, _ly) = result.unwrap();
        assert_eq!(oid, id2);
        assert_eq!(lx, 80); // 2000 - 1920

        // Point outside all outputs
        assert!(mgr.point_to_output(5000, 5000).is_none());
    }

    #[test]
    fn test_auto_layout() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        // Move outputs to weird positions
        mgr.set_position(id1, 500, 300).unwrap();
        mgr.set_position(id2, -100, 200).unwrap();

        // Auto-layout should reset to left-to-right
        mgr.auto_layout();

        let o1 = mgr.get_output(id1).unwrap();
        assert_eq!(o1.x_offset, 0);
        assert_eq!(o1.y_offset, 0);

        let o2 = mgr.get_output(id2).unwrap();
        assert_eq!(o2.x_offset, 1920);
        assert_eq!(o2.y_offset, 0);
    }

    #[test]
    fn test_handle_hotplug_connect() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        mgr.handle_hotplug(1, true, 1920, 1080, 60000).unwrap();
        assert_eq!(mgr.num_outputs(), 1);
    }

    #[test]
    fn test_handle_hotplug_disconnect() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        mgr.handle_hotplug(1, true, 1920, 1080, 60000).unwrap();
        assert_eq!(mgr.num_outputs(), 1);

        mgr.handle_hotplug(1, false, 0, 0, 0).unwrap();
        assert_eq!(mgr.num_outputs(), 0);
    }

    #[test]
    fn test_handle_hotplug_duplicate_connect() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        mgr.handle_hotplug(1, true, 1920, 1080, 60000).unwrap();
        mgr.handle_hotplug(1, true, 1920, 1080, 60000).unwrap();
        assert_eq!(mgr.num_outputs(), 1); // should not duplicate
    }

    #[test]
    fn test_flip() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        assert!(mgr.flip(id, 0).is_ok());
        assert!(mgr.flip(999, 0).is_err());
    }

    #[test]
    fn test_primary_output() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        assert!(mgr.primary_output().is_none());

        mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        assert!(mgr.primary_output().is_some());
        assert!(mgr.primary_output().unwrap().primary);
    }

    #[test]
    fn test_primary_promotion_on_remove() {
        let mut mgr = MultiOutputManager::new();
        mgr.init();

        let id1 = mgr.add_output(1, 10, 1920, 1080, 60000).unwrap();
        let _id2 = mgr.add_output(2, 20, 2560, 1440, 60000).unwrap();

        // Remove primary
        mgr.remove_output(id1).unwrap();

        // Second output should be promoted
        assert!(mgr.primary_output().is_some());
        assert!(mgr.primary_output().unwrap().primary);
    }
}
