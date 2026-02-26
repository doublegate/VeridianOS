//! Wayland Compositor
//!
//! Implements the Wayland display protocol for VeridianOS.
//!
//! ## Wayland Overview
//!
//! Wayland is a modern replacement for X11, designed for:
//! - Direct rendering: Clients draw directly to surfaces
//! - Asynchronous updates: No blocking on server
//! - Security: No global coordinate space, isolated clients

// Phase 6 (desktop) -- Wayland protocol structures are defined but the
// compositor is not yet connected to actual display hardware.
//! - Efficiency: Minimal data copies, GPU acceleration
//!
//! ## Core Concepts
//!
//! - **Display**: Connection to compositor
//! - **Surface**: Renderable area
//! - **Buffer**: Pixel data attached to surface
//! - **Compositor**: Window manager
//! - **Shell**: Desktop interface (xdg-shell)

pub mod buffer;
pub mod compositor;
pub mod protocol;
pub mod shell;
pub mod surface;

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Wayland object ID
pub type ObjectId = u32;

/// Wayland display server
pub struct WaylandDisplay {
    /// Connected clients
    clients: RwLock<BTreeMap<u32, WaylandClient>>,
    /// Next client ID
    next_client_id: core::sync::atomic::AtomicU32,
    /// Global objects (compositor, shell, etc.)
    globals: RwLock<Vec<GlobalObject>>,
}

impl WaylandDisplay {
    /// Create new Wayland display
    pub fn new() -> Self {
        let mut display = Self {
            clients: RwLock::new(BTreeMap::new()),
            next_client_id: core::sync::atomic::AtomicU32::new(1),
            globals: RwLock::new(Vec::new()),
        };

        // Register global objects
        display.register_global("wl_compositor", 4);
        display.register_global("wl_shm", 1);
        display.register_global("xdg_wm_base", 2);

        display
    }

    /// Register a global object
    fn register_global(&mut self, interface: &str, version: u32) {
        self.globals.write().push(GlobalObject {
            interface: String::from(interface),
            version,
        });
    }

    /// Connect a new client
    pub fn connect_client(&self) -> Result<u32, KernelError> {
        let client_id = self
            .next_client_id
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let client = WaylandClient::new(client_id);
        self.clients.write().insert(client_id, client);

        Ok(client_id)
    }

    /// Disconnect client
    pub fn disconnect_client(&self, client_id: u32) -> Result<(), KernelError> {
        self.clients.write().remove(&client_id);
        Ok(())
    }

    /// Process client message
    pub fn process_message(&self, client_id: u32, data: &[u8]) -> Result<Vec<u8>, KernelError> {
        let clients = self.clients.read();
        let client = clients.get(&client_id).ok_or(KernelError::NotFound {
            resource: "client",
            id: client_id as u64,
        })?;

        client.handle_message(data)
    }
}

impl Default for WaylandDisplay {
    fn default() -> Self {
        Self::new()
    }
}

/// Global object announcement
#[derive(Debug, Clone)]
struct GlobalObject {
    interface: String,
    version: u32,
}

/// Wayland client connection
pub struct WaylandClient {
    id: u32,
    /// Client's object map
    objects: RwLock<BTreeMap<ObjectId, Object>>,
    /// Next object ID
    next_object_id: core::sync::atomic::AtomicU32,
}

impl WaylandClient {
    fn new(id: u32) -> Self {
        Self {
            id,
            objects: RwLock::new(BTreeMap::new()),
            next_object_id: core::sync::atomic::AtomicU32::new(1),
        }
    }

    fn handle_message(&self, _data: &[u8]) -> Result<Vec<u8>, KernelError> {
        // TODO(phase6): Parse Wayland protocol message (object_id, opcode, size,
        // arguments)

        Ok(Vec::new())
    }

    /// Create new object
    pub fn create_object(&self, interface: &str) -> ObjectId {
        let id = self
            .next_object_id
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        let object = Object {
            id,
            interface: String::from(interface),
        };

        self.objects.write().insert(id, object);
        id
    }

    /// Destroy object
    pub fn destroy_object(&self, object_id: ObjectId) {
        self.objects.write().remove(&object_id);
    }
}

/// Wayland object
#[derive(Debug, Clone)]
struct Object {
    id: ObjectId,
    interface: String,
}

/// Global Wayland display instance
static WAYLAND_DISPLAY: GlobalState<WaylandDisplay> = GlobalState::new();

/// Initialize Wayland compositor
pub fn init() -> Result<(), KernelError> {
    WAYLAND_DISPLAY
        .init(WaylandDisplay::new())
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    crate::println!("[WAYLAND] Wayland compositor initialized");
    Ok(())
}

/// Execute a function with the Wayland display
pub fn with_display<R, F: FnOnce(&WaylandDisplay) -> R>(f: F) -> Option<R> {
    WAYLAND_DISPLAY.with(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_creation() {
        let display = WaylandDisplay::new();
        assert_eq!(display.globals.read().len(), 3); // compositor, shm,
                                                     // xdg_wm_base
    }

    #[test]
    fn test_client_connection() {
        let display = WaylandDisplay::new();
        let client_id = display.connect_client().unwrap();
        assert!(client_id > 0);

        assert!(display.disconnect_client(client_id).is_ok());
    }

    #[test]
    fn test_object_creation() {
        let client = WaylandClient::new(1);
        let obj_id = client.create_object("wl_surface");
        assert_eq!(obj_id, 1);

        client.destroy_object(obj_id);
    }
}
