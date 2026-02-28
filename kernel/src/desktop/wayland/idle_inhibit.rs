//! Idle Inhibit Protocol (zwp_idle_inhibit_manager_v1)
//!
//! Allows surfaces to prevent the system from going idle (screen dimming,
//! locking) while they are visible. This is commonly used by video players,
//! presentation software, and games to keep the display active.
//!
//! An inhibitor is associated with a specific surface. The idle state is
//! inhibited as long as at least one active inhibitor exists whose surface
//! is visible (mapped).

#![allow(dead_code)]

use alloc::collections::BTreeMap;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Protocol constants
// ---------------------------------------------------------------------------

/// Wayland global interface name for idle inhibit manager
pub const ZWP_IDLE_INHIBIT_MANAGER_V1: &str = "zwp_idle_inhibit_manager_v1";

/// Protocol version
pub const ZWP_IDLE_INHIBIT_MANAGER_V1_VERSION: u32 = 1;

// Manager request opcodes
/// destroy
pub const ZWP_IDLE_INHIBIT_MANAGER_V1_DESTROY: u16 = 0;
/// create_inhibitor(id: new_id, surface: object)
pub const ZWP_IDLE_INHIBIT_MANAGER_V1_CREATE_INHIBITOR: u16 = 1;

// Inhibitor request opcodes
/// destroy
pub const ZWP_IDLE_INHIBITOR_V1_DESTROY: u16 = 0;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An idle inhibitor tied to a specific surface.
///
/// While active, prevents the system from entering idle state (e.g.,
/// dimming the screen or activating a screen locker). The inhibitor
/// is automatically deactivated when its associated surface is destroyed
/// or unmapped.
pub struct IdleInhibitor {
    /// Inhibitor object ID
    pub id: u32,
    /// Surface ID that this inhibitor is tied to
    pub surface_id: u32,
    /// Whether this inhibitor is currently active
    pub active: bool,
}

impl IdleInhibitor {
    /// Create a new active idle inhibitor for the given surface.
    pub fn new(id: u32, surface_id: u32) -> Self {
        Self {
            id,
            surface_id,
            active: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Idle inhibit manager
// ---------------------------------------------------------------------------

/// Manages idle inhibitors and queries the global idle-inhibited state.
pub struct IdleInhibitManager {
    /// All inhibitors keyed by their object ID
    inhibitors: BTreeMap<u32, IdleInhibitor>,
    /// Next inhibitor ID
    next_id: u32,
}

impl IdleInhibitManager {
    /// Create a new idle inhibit manager.
    pub fn new() -> Self {
        Self {
            inhibitors: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Create a new inhibitor for the specified surface.
    ///
    /// Returns the inhibitor ID assigned by the manager.
    pub fn create_inhibitor(&mut self, surface_id: u32) -> Result<u32, KernelError> {
        let id = self.next_id;
        self.next_id += 1;

        let inhibitor = IdleInhibitor::new(id, surface_id);
        self.inhibitors.insert(id, inhibitor);

        Ok(id)
    }

    /// Destroy an inhibitor by its object ID.
    pub fn destroy_inhibitor(&mut self, id: u32) -> Result<(), KernelError> {
        self.inhibitors.remove(&id).ok_or(KernelError::NotFound {
            resource: "idle_inhibitor",
            id: id as u64,
        })?;
        Ok(())
    }

    /// Remove all inhibitors associated with a specific surface.
    ///
    /// Called when a surface is destroyed to clean up its inhibitors.
    /// Returns the number of inhibitors removed.
    pub fn remove_inhibitors_for_surface(&mut self, surface_id: u32) -> usize {
        let before = self.inhibitors.len();
        self.inhibitors
            .retain(|_, inh| inh.surface_id != surface_id);
        before - self.inhibitors.len()
    }

    /// Check whether idle should be inhibited.
    ///
    /// Returns `true` if at least one active inhibitor exists. In a full
    /// implementation, this would also check whether the inhibitor's
    /// surface is currently visible/mapped.
    pub fn is_idle_inhibited(&self) -> bool {
        self.inhibitors.values().any(|inh| inh.active)
    }

    /// Get the number of active inhibitors.
    pub fn active_count(&self) -> usize {
        self.inhibitors.values().filter(|inh| inh.active).count()
    }

    /// Get a reference to a specific inhibitor.
    pub fn get_inhibitor(&self, id: u32) -> Option<&IdleInhibitor> {
        self.inhibitors.get(&id)
    }

    /// Total number of inhibitors (active and inactive).
    pub fn inhibitor_count(&self) -> usize {
        self.inhibitors.len()
    }
}

impl Default for IdleInhibitManager {
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
    fn test_create_inhibitor() {
        let mut mgr = IdleInhibitManager::new();
        let id = mgr.create_inhibitor(10).unwrap();
        assert!(mgr.is_idle_inhibited());
        assert_eq!(mgr.active_count(), 1);

        let inh = mgr.get_inhibitor(id).unwrap();
        assert_eq!(inh.surface_id, 10);
        assert!(inh.active);
    }

    #[test]
    fn test_destroy_inhibitor() {
        let mut mgr = IdleInhibitManager::new();
        let id = mgr.create_inhibitor(10).unwrap();
        assert!(mgr.is_idle_inhibited());

        mgr.destroy_inhibitor(id).unwrap();
        assert!(!mgr.is_idle_inhibited());
        assert_eq!(mgr.inhibitor_count(), 0);
    }

    #[test]
    fn test_destroy_nonexistent() {
        let mut mgr = IdleInhibitManager::new();
        assert!(mgr.destroy_inhibitor(999).is_err());
    }

    #[test]
    fn test_remove_for_surface() {
        let mut mgr = IdleInhibitManager::new();
        mgr.create_inhibitor(10).unwrap();
        mgr.create_inhibitor(10).unwrap();
        mgr.create_inhibitor(20).unwrap();

        let removed = mgr.remove_inhibitors_for_surface(10);
        assert_eq!(removed, 2);
        assert_eq!(mgr.inhibitor_count(), 1);
        assert!(mgr.is_idle_inhibited()); // surface 20 still active
    }

    #[test]
    fn test_not_inhibited_when_empty() {
        let mgr = IdleInhibitManager::new();
        assert!(!mgr.is_idle_inhibited());
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn test_multiple_inhibitors() {
        let mut mgr = IdleInhibitManager::new();
        mgr.create_inhibitor(1).unwrap();
        mgr.create_inhibitor(2).unwrap();
        mgr.create_inhibitor(3).unwrap();
        assert_eq!(mgr.active_count(), 3);
        assert!(mgr.is_idle_inhibited());
    }
}
