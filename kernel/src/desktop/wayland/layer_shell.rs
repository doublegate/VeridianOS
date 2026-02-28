//! Layer Shell Protocol (zwlr_layer_shell_v1)
//!
//! Provides surfaces anchored to screen edges for panels, notifications,
//! screen locks, and overlays. Based on wlr-layer-shell-unstable-v1.
//!
//! Layer surfaces are positioned relative to the output edges using anchor
//! flags and can claim exclusive zones that reduce the usable area for
//! normal windows. The rendering order from bottom to top is:
//!
//!   Background -> Bottom -> Top -> Overlay
//!
//! This ensures that overlay surfaces (screen locks) always appear above
//! everything, while background surfaces (wallpaper) sit behind all windows.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Layer shell protocol constants
// ---------------------------------------------------------------------------

/// Wayland global interface name for layer shell
pub const ZWLR_LAYER_SHELL_V1: &str = "zwlr_layer_shell_v1";

/// Protocol version
pub const ZWLR_LAYER_SHELL_V1_VERSION: u32 = 4;

// Layer shell request opcodes
/// get_layer_surface(id, surface, output, layer, namespace)
pub const ZWLR_LAYER_SHELL_V1_GET_LAYER_SURFACE: u16 = 0;
/// destroy
pub const ZWLR_LAYER_SHELL_V1_DESTROY: u16 = 1;

// Layer surface request opcodes
/// set_size(width, height)
pub const ZWLR_LAYER_SURFACE_V1_SET_SIZE: u16 = 0;
/// set_anchor(anchor)
pub const ZWLR_LAYER_SURFACE_V1_SET_ANCHOR: u16 = 1;
/// set_exclusive_zone(zone)
pub const ZWLR_LAYER_SURFACE_V1_SET_EXCLUSIVE_ZONE: u16 = 2;
/// set_margin(top, right, bottom, left)
pub const ZWLR_LAYER_SURFACE_V1_SET_MARGIN: u16 = 3;
/// set_keyboard_interactivity(mode)
pub const ZWLR_LAYER_SURFACE_V1_SET_KEYBOARD_INTERACTIVITY: u16 = 4;
/// get_popup(popup)
pub const ZWLR_LAYER_SURFACE_V1_GET_POPUP: u16 = 5;
/// ack_configure(serial)
pub const ZWLR_LAYER_SURFACE_V1_ACK_CONFIGURE: u16 = 6;
/// destroy
pub const ZWLR_LAYER_SURFACE_V1_DESTROY: u16 = 7;
/// set_layer(layer) -- since version 2
pub const ZWLR_LAYER_SURFACE_V1_SET_LAYER: u16 = 8;

// Layer surface event opcodes
/// configure(serial, width, height)
pub const ZWLR_LAYER_SURFACE_V1_CONFIGURE: u16 = 0;
/// closed
pub const ZWLR_LAYER_SURFACE_V1_CLOSED: u16 = 1;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Layer shell layers (bottom to top rendering order).
///
/// Surfaces in higher layers are always rendered above surfaces in lower
/// layers. Normal windows sit between Bottom and Top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Layer {
    /// Below all windows (e.g., desktop wallpaper)
    Background = 0,
    /// Below normal windows but above background (e.g., bottom panels)
    Bottom = 1,
    /// Above normal windows (e.g., top panels, notification popups)
    Top = 2,
    /// Above everything including fullscreen (e.g., screen lock, OSD)
    Overlay = 3,
}

impl Layer {
    /// Convert a raw protocol value to a Layer.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Background),
            1 => Some(Self::Bottom),
            2 => Some(Self::Top),
            3 => Some(Self::Overlay),
            _ => None,
        }
    }
}

/// Anchor edges for layer surface positioning.
///
/// When opposite edges are anchored (e.g., left + right), the surface
/// stretches to fill that axis. When only one edge is anchored, the
/// surface is placed at that edge with its requested size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Anchor {
    /// Create an Anchor with no edges set.
    pub fn none() -> Self {
        Self {
            top: false,
            bottom: false,
            left: false,
            right: false,
        }
    }

    /// Create an Anchor from a bitfield (protocol wire format).
    ///
    /// Bit 0 = top, bit 1 = bottom, bit 2 = left, bit 3 = right.
    pub fn from_bits(bits: u32) -> Self {
        Self {
            top: bits & 1 != 0,
            bottom: bits & 2 != 0,
            left: bits & 4 != 0,
            right: bits & 8 != 0,
        }
    }

    /// Convert to bitfield representation.
    pub fn to_bits(&self) -> u32 {
        let mut bits = 0u32;
        if self.top {
            bits |= 1;
        }
        if self.bottom {
            bits |= 2;
        }
        if self.left {
            bits |= 4;
        }
        if self.right {
            bits |= 8;
        }
        bits
    }

    /// Whether this anchor stretches horizontally (left + right).
    pub fn stretches_horizontal(&self) -> bool {
        self.left && self.right
    }

    /// Whether this anchor stretches vertically (top + bottom).
    pub fn stretches_vertical(&self) -> bool {
        self.top && self.bottom
    }
}

/// Keyboard interactivity mode for layer surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum KeyboardInteractivity {
    /// Surface does not receive keyboard events
    None = 0,
    /// Surface grabs keyboard focus exclusively (e.g., screen lock)
    Exclusive = 1,
    /// Surface receives keyboard focus on demand (e.g., when clicked)
    OnDemand = 2,
}

impl KeyboardInteractivity {
    /// Convert from raw protocol value.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Exclusive),
            2 => Some(Self::OnDemand),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Layer surface
// ---------------------------------------------------------------------------

/// Layer surface configuration.
///
/// A layer surface is a surface that is anchored to screen edges and
/// belongs to a specific rendering layer. It can claim an exclusive zone
/// to prevent normal windows from occupying that screen area.
pub struct LayerSurface {
    /// Layer surface object ID
    pub id: u32,
    /// Underlying compositor surface ID
    pub surface_id: u32,
    /// Which layer this surface belongs to
    pub layer: Layer,
    /// Which screen edges the surface is anchored to
    pub anchor: Anchor,
    /// Exclusive zone in pixels (-1 = no exclusive zone, 0+ = reserve space)
    pub exclusive_zone: i32,
    /// Margin from top edge in pixels
    pub margin_top: i32,
    /// Margin from bottom edge in pixels
    pub margin_bottom: i32,
    /// Margin from left edge in pixels
    pub margin_left: i32,
    /// Margin from right edge in pixels
    pub margin_right: i32,
    /// Keyboard focus mode
    pub keyboard_interactivity: KeyboardInteractivity,
    /// Requested width (0 = fill available width when anchored)
    pub desired_width: u32,
    /// Requested height (0 = fill available height when anchored)
    pub desired_height: u32,
    /// Actual configured width (assigned by compositor)
    pub actual_width: u32,
    /// Actual configured height (assigned by compositor)
    pub actual_height: u32,
    /// Namespace for grouping (e.g., "panel", "notifications", "wallpaper")
    pub namespace: [u8; 64],
    /// Length of the namespace string
    pub namespace_len: usize,
    /// Whether the client has acknowledged the latest configure
    pub configured: bool,
    /// Last configure serial sent to client
    pub configure_serial: u32,
    /// Whether the surface is currently mapped (has a buffer and is visible)
    pub mapped: bool,
}

impl LayerSurface {
    /// Create a new layer surface with default settings.
    pub fn new(id: u32, surface_id: u32, layer: Layer) -> Self {
        Self {
            id,
            surface_id,
            layer,
            anchor: Anchor::none(),
            exclusive_zone: 0,
            margin_top: 0,
            margin_bottom: 0,
            margin_left: 0,
            margin_right: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
            desired_width: 0,
            desired_height: 0,
            actual_width: 0,
            actual_height: 0,
            namespace: [0u8; 64],
            namespace_len: 0,
            configured: false,
            configure_serial: 0,
            mapped: false,
        }
    }

    /// Set the namespace string for this layer surface.
    pub fn set_namespace(&mut self, ns: &[u8]) {
        let copy_len = ns.len().min(self.namespace.len());
        self.namespace[..copy_len].copy_from_slice(&ns[..copy_len]);
        self.namespace_len = copy_len;
    }

    /// Get the namespace as a byte slice.
    pub fn namespace_bytes(&self) -> &[u8] {
        &self.namespace[..self.namespace_len]
    }
}

// ---------------------------------------------------------------------------
// Layer shell manager
// ---------------------------------------------------------------------------

/// Usable screen area after exclusive zones are subtracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsableArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Layer shell manager.
///
/// Tracks all layer surfaces and computes exclusive zones for the output.
pub struct LayerShellManager {
    /// All layer surfaces keyed by their object ID
    surfaces: BTreeMap<u32, LayerSurface>,
    /// Next layer surface ID
    next_id: u32,
    /// Next configure serial
    next_serial: u32,
}

impl LayerShellManager {
    /// Create a new layer shell manager.
    pub fn new() -> Self {
        Self {
            surfaces: BTreeMap::new(),
            next_id: 1,
            next_serial: 1,
        }
    }

    /// Allocate the next serial number.
    fn alloc_serial(&mut self) -> u32 {
        let s = self.next_serial;
        self.next_serial += 1;
        s
    }

    /// Create a new layer surface.
    ///
    /// Returns the layer surface ID assigned by the manager.
    pub fn create_surface(
        &mut self,
        surface_id: u32,
        layer: Layer,
        namespace: &[u8],
    ) -> Result<u32, KernelError> {
        let id = self.next_id;
        self.next_id += 1;

        let mut ls = LayerSurface::new(id, surface_id, layer);
        ls.set_namespace(namespace);

        self.surfaces.insert(id, ls);
        Ok(id)
    }

    /// Destroy a layer surface.
    pub fn destroy_surface(&mut self, id: u32) -> Result<(), KernelError> {
        self.surfaces.remove(&id).ok_or(KernelError::NotFound {
            resource: "layer_surface",
            id: id as u64,
        })?;
        Ok(())
    }

    /// Get a reference to a layer surface.
    pub fn get_surface(&self, id: u32) -> Option<&LayerSurface> {
        self.surfaces.get(&id)
    }

    /// Get a mutable reference to a layer surface.
    pub fn get_surface_mut(&mut self, id: u32) -> Option<&mut LayerSurface> {
        self.surfaces.get_mut(&id)
    }

    /// Configure a layer surface with its actual dimensions and send a
    /// configure serial to the client.
    ///
    /// Returns the serial number to be sent in the configure event.
    pub fn configure_surface(
        &mut self,
        id: u32,
        width: u32,
        height: u32,
    ) -> Result<u32, KernelError> {
        let serial = self.alloc_serial();

        let ls = self.surfaces.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "layer_surface",
            id: id as u64,
        })?;

        ls.actual_width = width;
        ls.actual_height = height;
        ls.configure_serial = serial;
        ls.configured = false;

        Ok(serial)
    }

    /// Handle ack_configure from the client.
    pub fn ack_configure(&mut self, id: u32, serial: u32) -> bool {
        if let Some(ls) = self.surfaces.get_mut(&id) {
            if ls.configure_serial == serial {
                ls.configured = true;
                return true;
            }
        }
        false
    }

    /// Get all layer surfaces belonging to a specific layer, sorted by
    /// creation order.
    pub fn get_surfaces_for_layer(&self, layer: Layer) -> Vec<&LayerSurface> {
        self.surfaces
            .values()
            .filter(|ls| ls.layer == layer)
            .collect()
    }

    /// Calculate the total exclusive zone offsets for an output.
    ///
    /// Returns (top, bottom, left, right) pixel offsets that normal
    /// windows should respect.
    pub fn calculate_exclusive_zones(&self) -> (i32, i32, i32, i32) {
        let mut top = 0i32;
        let mut bottom = 0i32;
        let mut left = 0i32;
        let mut right = 0i32;

        for ls in self.surfaces.values() {
            if ls.exclusive_zone <= 0 || !ls.mapped {
                continue;
            }

            let zone = ls.exclusive_zone;

            // Determine which edge the exclusive zone applies to based on
            // the anchor configuration.
            if ls.anchor.top && !ls.anchor.bottom {
                // Anchored to top only -> reserves top space
                top = top.max(zone + ls.margin_top);
            } else if ls.anchor.bottom && !ls.anchor.top {
                // Anchored to bottom only -> reserves bottom space
                bottom = bottom.max(zone + ls.margin_bottom);
            } else if ls.anchor.left && !ls.anchor.right {
                // Anchored to left only -> reserves left space
                left = left.max(zone + ls.margin_left);
            } else if ls.anchor.right && !ls.anchor.left {
                // Anchored to right only -> reserves right space
                right = right.max(zone + ls.margin_right);
            } else if ls.anchor.top && ls.anchor.bottom {
                // Vertically stretched -- exclusive zone applies to the
                // narrowest horizontal anchor
                if ls.anchor.left && !ls.anchor.right {
                    left = left.max(zone + ls.margin_left);
                } else if ls.anchor.right && !ls.anchor.left {
                    right = right.max(zone + ls.margin_right);
                }
            } else if ls.anchor.left && ls.anchor.right {
                // Horizontally stretched -- exclusive zone applies to the
                // narrowest vertical anchor
                if ls.anchor.top && !ls.anchor.bottom {
                    top = top.max(zone + ls.margin_top);
                } else if ls.anchor.bottom && !ls.anchor.top {
                    bottom = bottom.max(zone + ls.margin_bottom);
                }
            }
        }

        (top, bottom, left, right)
    }

    /// Compute the usable area after subtracting all exclusive zones from
    /// the full output dimensions.
    pub fn get_usable_area(&self, output_width: u32, output_height: u32) -> UsableArea {
        let (top, bottom, left, right) = self.calculate_exclusive_zones();

        let x = left;
        let y = top;
        let w = (output_width as i32 - left - right).max(0) as u32;
        let h = (output_height as i32 - top - bottom).max(0) as u32;

        UsableArea {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Return the total number of layer surfaces.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }
}

impl Default for LayerShellManager {
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
    fn test_layer_ordering() {
        assert!(Layer::Background < Layer::Bottom);
        assert!(Layer::Bottom < Layer::Top);
        assert!(Layer::Top < Layer::Overlay);
    }

    #[test]
    fn test_anchor_from_bits() {
        let a = Anchor::from_bits(0b1111);
        assert!(a.top && a.bottom && a.left && a.right);

        let b = Anchor::from_bits(0b0001);
        assert!(b.top && !b.bottom && !b.left && !b.right);

        let c = Anchor::from_bits(0);
        assert!(!c.top && !c.bottom && !c.left && !c.right);
    }

    #[test]
    fn test_anchor_roundtrip() {
        let a = Anchor {
            top: true,
            bottom: false,
            left: true,
            right: false,
        };
        let bits = a.to_bits();
        let b = Anchor::from_bits(bits);
        assert_eq!(a, b);
    }

    #[test]
    fn test_layer_surface_creation() {
        let mut mgr = LayerShellManager::new();
        let id = mgr.create_surface(10, Layer::Top, b"panel").unwrap();
        assert_eq!(mgr.surface_count(), 1);

        let ls = mgr.get_surface(id).unwrap();
        assert_eq!(ls.surface_id, 10);
        assert_eq!(ls.layer, Layer::Top);
        assert_eq!(ls.namespace_bytes(), b"panel");
    }

    #[test]
    fn test_layer_surface_destroy() {
        let mut mgr = LayerShellManager::new();
        let id = mgr.create_surface(10, Layer::Bottom, b"dock").unwrap();
        assert_eq!(mgr.surface_count(), 1);
        mgr.destroy_surface(id).unwrap();
        assert_eq!(mgr.surface_count(), 0);
    }

    #[test]
    fn test_configure_ack() {
        let mut mgr = LayerShellManager::new();
        let id = mgr.create_surface(10, Layer::Top, b"bar").unwrap();

        let serial = mgr.configure_surface(id, 1280, 32).unwrap();
        assert!(!mgr.get_surface(id).unwrap().configured);

        assert!(mgr.ack_configure(id, serial));
        assert!(mgr.get_surface(id).unwrap().configured);

        // Wrong serial
        assert!(!mgr.ack_configure(id, serial + 999));
    }

    #[test]
    fn test_exclusive_zones() {
        let mut mgr = LayerShellManager::new();

        // Top panel: anchored top, left, right with 32px exclusive zone
        let id = mgr.create_surface(10, Layer::Top, b"panel").unwrap();
        {
            let ls = mgr.get_surface_mut(id).unwrap();
            ls.anchor = Anchor {
                top: true,
                bottom: false,
                left: true,
                right: true,
            };
            ls.exclusive_zone = 32;
            ls.mapped = true;
        }

        let (top, bottom, left, right) = mgr.calculate_exclusive_zones();
        assert_eq!(top, 32);
        assert_eq!(bottom, 0);
        assert_eq!(left, 0);
        assert_eq!(right, 0);

        let usable = mgr.get_usable_area(1280, 800);
        assert_eq!(usable.x, 0);
        assert_eq!(usable.y, 32);
        assert_eq!(usable.width, 1280);
        assert_eq!(usable.height, 768);
    }

    #[test]
    fn test_usable_area_no_exclusions() {
        let mgr = LayerShellManager::new();
        let area = mgr.get_usable_area(1920, 1080);
        assert_eq!(area.x, 0);
        assert_eq!(area.y, 0);
        assert_eq!(area.width, 1920);
        assert_eq!(area.height, 1080);
    }

    #[test]
    fn test_get_surfaces_for_layer() {
        let mut mgr = LayerShellManager::new();
        mgr.create_surface(1, Layer::Top, b"a").unwrap();
        mgr.create_surface(2, Layer::Bottom, b"b").unwrap();
        mgr.create_surface(3, Layer::Top, b"c").unwrap();

        let top_surfaces = mgr.get_surfaces_for_layer(Layer::Top);
        assert_eq!(top_surfaces.len(), 2);

        let bottom_surfaces = mgr.get_surfaces_for_layer(Layer::Bottom);
        assert_eq!(bottom_surfaces.len(), 1);

        let bg_surfaces = mgr.get_surfaces_for_layer(Layer::Background);
        assert_eq!(bg_surfaces.len(), 0);
    }
}
