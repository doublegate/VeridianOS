//! XWayland Compatibility Layer
//!
//! Provides the socket infrastructure and window mapping for running X11
//! applications via XWayland on VeridianOS. Full X11 server implementation
//! is deferred to Phase 8; this module establishes the correct architecture
//! and provides the bridge between X11 window IDs and Wayland surface IDs.
//!
//! ## Architecture
//!
//! XWayland runs as a child process of the compositor, speaking the X11
//! protocol over a Unix socket pair. It creates Wayland surfaces for each
//! X11 window and forwards input events.
//!
//! ```text
//! X11 Client -> XWayland Process -> Wayland Compositor
//!                   |                     |
//!              X11 protocol          Wayland protocol
//!              (/tmp/.X11-unix/X0)   (kernel syscalls)
//! ```
//!
//! ## Current Status
//!
//! This is a stub implementation that:
//! - Creates the X11 socket directory structure
//! - Manages X11-to-Wayland window ID mappings
//! - Provides the XWayland server lifecycle (start/stop)
//! - Logs operations for debugging
//!
//! Actual X11 protocol handling requires Phase 8.

#![allow(dead_code)]

use alloc::{format, string::String, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default X11 display number
pub const DEFAULT_DISPLAY: u32 = 0;

/// Maximum display number we will try
pub const MAX_DISPLAY: u32 = 32;

/// X11 socket directory
pub const X11_SOCKET_DIR: &str = "/tmp/.X11-unix";

/// X11 lock file directory
pub const X11_LOCK_DIR: &str = "/tmp";

/// Maximum number of X11 windows we track
const MAX_WINDOWS: usize = 256;

// ---------------------------------------------------------------------------
// XWayland server state
// ---------------------------------------------------------------------------

/// State of the XWayland server process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XWaylandState {
    /// Server has not been started
    NotStarted,
    /// Server is in the process of starting (socket created, waiting for ready)
    Starting,
    /// Server is running and accepting X11 connections
    Running,
    /// Server failed to start
    Failed,
    /// Server has been stopped
    Stopped,
}

// ---------------------------------------------------------------------------
// X11 window mapping
// ---------------------------------------------------------------------------

/// Maps an X11 window to a Wayland compositor surface.
///
/// When XWayland creates a window, it also creates a corresponding Wayland
/// surface. This mapping lets the compositor route events and manage the
/// window's lifecycle across both protocols.
#[derive(Debug, Clone)]
pub struct X11WindowMapping {
    /// X11 window ID (XID)
    pub x11_window_id: u32,
    /// Corresponding Wayland surface ID in the compositor
    pub wayland_surface_id: u32,
    /// Whether this is an override-redirect window (popup/tooltip)
    pub override_redirect: bool,
    /// Window position in X11 coordinate space
    pub x: i32,
    pub y: i32,
    /// Window dimensions
    pub width: u32,
    pub height: u32,
    /// Whether the window is currently mapped (visible)
    pub mapped: bool,
    /// Window title (from _NET_WM_NAME or WM_NAME property)
    pub title: String,
    /// Window class (from WM_CLASS property)
    pub window_class: String,
    /// Whether this window wants input focus
    pub accepts_focus: bool,
}

impl X11WindowMapping {
    /// Create a new window mapping.
    fn new(x11_id: u32, wayland_id: u32) -> Self {
        Self {
            x11_window_id: x11_id,
            wayland_surface_id: wayland_id,
            override_redirect: false,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            mapped: false,
            title: String::new(),
            window_class: String::new(),
            accepts_focus: true,
        }
    }
}

// ---------------------------------------------------------------------------
// X11 event types (stub)
// ---------------------------------------------------------------------------

/// Simplified X11 event for the compatibility layer.
///
/// These are the events that XWayland would forward from X11 clients
/// to the Wayland compositor.
#[derive(Debug, Clone)]
pub enum X11Event {
    /// Window creation (CreateNotify equivalent)
    WindowCreated {
        window_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        override_redirect: bool,
    },
    /// Window destruction (DestroyNotify equivalent)
    WindowDestroyed { window_id: u32 },
    /// Window mapped (visible)
    WindowMapped { window_id: u32 },
    /// Window unmapped (hidden)
    WindowUnmapped { window_id: u32 },
    /// Window reconfigured (position/size change)
    WindowConfigured {
        window_id: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    /// Window title changed
    TitleChanged { window_id: u32, title: String },
    /// Focus request
    FocusRequest { window_id: u32 },
}

// ---------------------------------------------------------------------------
// XWayland server
// ---------------------------------------------------------------------------

/// XWayland compatibility server.
///
/// Manages the lifecycle of the XWayland process and maintains the mapping
/// between X11 window IDs and Wayland surface IDs.
pub struct XWaylandServer {
    /// Current server state
    state: XWaylandState,
    /// X11 display number (typically 0)
    display_number: u32,
    /// Path to the X11 socket (e.g., /tmp/.X11-unix/X0)
    socket_path: String,
    /// Path to the X11 lock file (e.g., /tmp/.X0-lock)
    lock_path: String,
    /// Active window mappings
    window_mappings: Vec<X11WindowMapping>,
    /// Next X11 window ID to assign (for stub purposes)
    next_window_id: u32,
    /// Window manager socket file descriptor (for WM <-> XWayland comms)
    wm_fd: Option<i32>,
    /// PID of the XWayland process (0 = not running)
    xwayland_pid: u64,
}

impl XWaylandServer {
    /// Create a new XWayland server (not yet started).
    pub fn new() -> Self {
        Self::with_display(DEFAULT_DISPLAY)
    }

    /// Create a new XWayland server with a specific display number.
    pub fn with_display(display: u32) -> Self {
        Self {
            state: XWaylandState::NotStarted,
            display_number: display,
            socket_path: format!("{}/X{}", X11_SOCKET_DIR, display),
            lock_path: format!("{}/.X{}-lock", X11_LOCK_DIR, display),
            window_mappings: Vec::new(),
            next_window_id: 1,
            wm_fd: None,
            xwayland_pid: 0,
        }
    }

    /// Start the XWayland server.
    ///
    /// In this stub implementation, this creates the socket directory
    /// structure and transitions to the Running state. Actual XWayland
    /// process spawning is deferred to Phase 8.
    pub fn start(&mut self) -> Result<(), KernelError> {
        if self.state == XWaylandState::Running {
            return Err(KernelError::InvalidState {
                expected: "not running",
                actual: "already running",
            });
        }

        self.state = XWaylandState::Starting;

        // Create socket directory (in real implementation, this would use
        // the VFS to create /tmp/.X11-unix/)
        crate::println!(
            "[XWAYLAND] Stub: creating X11 socket directory at {}",
            X11_SOCKET_DIR
        );
        crate::println!(
            "[XWAYLAND] Stub: display :{} socket at {}",
            self.display_number,
            self.socket_path
        );
        crate::println!("[XWAYLAND] Stub: lock file at {}", self.lock_path);

        // In Phase 8, this would:
        // 1. Create a socketpair for WM communication
        // 2. Fork and exec the XWayland binary
        // 3. Wait for the SIGUSR1 ready signal
        // 4. Set up the X11 window manager (reparenting, decorations)

        self.state = XWaylandState::Running;
        crate::println!(
            "[XWAYLAND] Stub server running on display :{}",
            self.display_number
        );
        Ok(())
    }

    /// Stop the XWayland server.
    pub fn stop(&mut self) -> Result<(), KernelError> {
        if self.state != XWaylandState::Running && self.state != XWaylandState::Starting {
            return Err(KernelError::InvalidState {
                expected: "running or starting",
                actual: "not running",
            });
        }

        // In Phase 8, this would:
        // 1. Send SIGTERM to the XWayland process
        // 2. Wait for exit
        // 3. Clean up sockets and lock file

        // Destroy all window mappings
        self.window_mappings.clear();

        self.wm_fd = None;
        self.xwayland_pid = 0;
        self.state = XWaylandState::Stopped;

        crate::println!(
            "[XWAYLAND] Server stopped on display :{}",
            self.display_number
        );
        Ok(())
    }

    /// Get the current server state.
    pub fn state(&self) -> XWaylandState {
        self.state
    }

    /// Get the display number.
    pub fn display_number(&self) -> u32 {
        self.display_number
    }

    /// Get the socket path.
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }

    /// Check if the server is running.
    pub fn is_running(&self) -> bool {
        self.state == XWaylandState::Running
    }

    /// Create a new X11-to-Wayland window mapping.
    ///
    /// Called when XWayland creates a new X11 window and its corresponding
    /// Wayland surface.
    pub fn create_window_mapping(&mut self, wayland_surface_id: u32) -> Result<u32, KernelError> {
        if self.window_mappings.len() >= MAX_WINDOWS {
            return Err(KernelError::ResourceExhausted {
                resource: "x11_windows",
            });
        }

        let x11_id = self.next_window_id;
        self.next_window_id += 1;

        let mapping = X11WindowMapping::new(x11_id, wayland_surface_id);
        self.window_mappings.push(mapping);

        Ok(x11_id)
    }

    /// Destroy a window mapping by X11 window ID.
    pub fn destroy_window_mapping(&mut self, x11_window_id: u32) -> Result<(), KernelError> {
        let idx = self
            .window_mappings
            .iter()
            .position(|m| m.x11_window_id == x11_window_id)
            .ok_or(KernelError::NotFound {
                resource: "x11_window",
                id: x11_window_id as u64,
            })?;

        self.window_mappings.remove(idx);
        Ok(())
    }

    /// Get the Wayland surface ID for an X11 window.
    pub fn get_surface_for_x11_window(&self, x11_window_id: u32) -> Option<u32> {
        self.window_mappings
            .iter()
            .find(|m| m.x11_window_id == x11_window_id)
            .map(|m| m.wayland_surface_id)
    }

    /// Get the X11 window ID for a Wayland surface.
    pub fn get_x11_window_for_surface(&self, wayland_surface_id: u32) -> Option<u32> {
        self.window_mappings
            .iter()
            .find(|m| m.wayland_surface_id == wayland_surface_id)
            .map(|m| m.x11_window_id)
    }

    /// Get a reference to a window mapping.
    pub fn get_window_mapping(&self, x11_window_id: u32) -> Option<&X11WindowMapping> {
        self.window_mappings
            .iter()
            .find(|m| m.x11_window_id == x11_window_id)
    }

    /// Get a mutable reference to a window mapping.
    pub fn get_window_mapping_mut(&mut self, x11_window_id: u32) -> Option<&mut X11WindowMapping> {
        self.window_mappings
            .iter_mut()
            .find(|m| m.x11_window_id == x11_window_id)
    }

    /// Get all active window mappings.
    pub fn get_all_mappings(&self) -> &[X11WindowMapping] {
        &self.window_mappings
    }

    /// Get the number of active window mappings.
    pub fn window_count(&self) -> usize {
        self.window_mappings.len()
    }

    /// Handle an X11 event from the XWayland process.
    ///
    /// In the stub implementation, this updates internal mappings.
    /// In Phase 8, this would translate X11 events to Wayland compositor
    /// actions.
    pub fn handle_x11_event(&mut self, event: &X11Event) -> Result<(), KernelError> {
        match event {
            X11Event::WindowCreated {
                window_id,
                x,
                y,
                width,
                height,
                override_redirect,
            } => {
                if let Some(mapping) = self.get_window_mapping_mut(*window_id) {
                    mapping.x = *x;
                    mapping.y = *y;
                    mapping.width = *width;
                    mapping.height = *height;
                    mapping.override_redirect = *override_redirect;
                }
                Ok(())
            }

            X11Event::WindowDestroyed { window_id } => {
                let _ = self.destroy_window_mapping(*window_id);
                Ok(())
            }

            X11Event::WindowMapped { window_id } => {
                if let Some(mapping) = self.get_window_mapping_mut(*window_id) {
                    mapping.mapped = true;
                }
                Ok(())
            }

            X11Event::WindowUnmapped { window_id } => {
                if let Some(mapping) = self.get_window_mapping_mut(*window_id) {
                    mapping.mapped = false;
                }
                Ok(())
            }

            X11Event::WindowConfigured {
                window_id,
                x,
                y,
                width,
                height,
            } => {
                if let Some(mapping) = self.get_window_mapping_mut(*window_id) {
                    mapping.x = *x;
                    mapping.y = *y;
                    mapping.width = *width;
                    mapping.height = *height;
                }
                Ok(())
            }

            X11Event::TitleChanged { window_id, title } => {
                if let Some(mapping) = self.get_window_mapping_mut(*window_id) {
                    mapping.title = title.clone();
                }
                Ok(())
            }

            X11Event::FocusRequest { window_id: _ } => {
                // In Phase 8, this would request focus through the Wayland
                // compositor's window manager
                Ok(())
            }
        }
    }

    /// Find a free display number by checking for existing lock files.
    ///
    /// Returns the first available display number, or an error if all
    /// display numbers up to MAX_DISPLAY are taken.
    pub fn find_free_display() -> Result<u32, KernelError> {
        // In the stub, we always return 0 since there is no actual
        // lock file checking.
        Ok(DEFAULT_DISPLAY)
    }

    /// Get the DISPLAY environment variable string for this server.
    ///
    /// Returns a string like ":0" that X11 clients use to connect.
    pub fn display_string(&self) -> String {
        format!(":{}", self.display_number)
    }
}

impl Default for XWaylandServer {
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
    fn test_xwayland_lifecycle() {
        let mut server = XWaylandServer::new();
        assert_eq!(server.state(), XWaylandState::NotStarted);
        assert!(!server.is_running());

        server.start().unwrap();
        assert_eq!(server.state(), XWaylandState::Running);
        assert!(server.is_running());

        server.stop().unwrap();
        assert_eq!(server.state(), XWaylandState::Stopped);
        assert!(!server.is_running());
    }

    #[test]
    fn test_window_mapping() {
        let mut server = XWaylandServer::new();
        server.start().unwrap();

        let x11_id = server.create_window_mapping(2001).unwrap();
        assert_eq!(server.window_count(), 1);
        assert_eq!(server.get_surface_for_x11_window(x11_id), Some(2001));
        assert_eq!(server.get_x11_window_for_surface(2001), Some(x11_id));

        server.destroy_window_mapping(x11_id).unwrap();
        assert_eq!(server.window_count(), 0);
    }

    #[test]
    fn test_x11_events() {
        let mut server = XWaylandServer::new();
        server.start().unwrap();

        let x11_id = server.create_window_mapping(2002).unwrap();

        server
            .handle_x11_event(&X11Event::WindowCreated {
                window_id: x11_id,
                x: 100,
                y: 200,
                width: 640,
                height: 480,
                override_redirect: false,
            })
            .unwrap();

        let mapping = server.get_window_mapping(x11_id).unwrap();
        assert_eq!(mapping.x, 100);
        assert_eq!(mapping.y, 200);
        assert_eq!(mapping.width, 640);
        assert_eq!(mapping.height, 480);
        assert!(!mapping.override_redirect);

        server
            .handle_x11_event(&X11Event::WindowMapped { window_id: x11_id })
            .unwrap();
        assert!(server.get_window_mapping(x11_id).unwrap().mapped);

        server
            .handle_x11_event(&X11Event::TitleChanged {
                window_id: x11_id,
                title: String::from("Test Window"),
            })
            .unwrap();
        assert_eq!(
            server.get_window_mapping(x11_id).unwrap().title,
            "Test Window"
        );
    }

    #[test]
    fn test_display_string() {
        let server = XWaylandServer::with_display(2);
        assert_eq!(server.display_string(), ":2");
        assert_eq!(server.socket_path(), "/tmp/.X11-unix/X2");
    }
}
