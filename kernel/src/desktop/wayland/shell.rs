//! XDG Shell Protocol
//!
//! Implements xdg_wm_base, xdg_surface, and xdg_toplevel -- the standard
//! desktop shell interface for managing windows, popups, and positioners.

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use spin::RwLock;

use super::protocol::{self, Argument, WaylandMessage};
use crate::error::KernelError;

// ---------------------------------------------------------------------------
// XDG constants
// ---------------------------------------------------------------------------

// xdg_wm_base opcodes (requests)
/// destroy
#[allow(dead_code)] // Phase 6: xdg_wm_base teardown
pub const XDG_WM_BASE_DESTROY: u16 = 0;
/// create_positioner
#[allow(dead_code)] // Phase 6: popup positioning
pub const XDG_WM_BASE_CREATE_POSITIONER: u16 = 1;
/// get_xdg_surface
pub const XDG_WM_BASE_GET_XDG_SURFACE: u16 = 2;
/// pong
pub const XDG_WM_BASE_PONG: u16 = 3;

// xdg_wm_base event opcodes (server -> client)
/// ping
pub const XDG_WM_BASE_PING: u16 = 0;

// xdg_surface opcodes (requests)
/// destroy
#[allow(dead_code)] // Phase 6: xdg_surface cleanup
pub const XDG_SURFACE_DESTROY: u16 = 0;
/// get_toplevel
pub const XDG_SURFACE_GET_TOPLEVEL: u16 = 1;
/// get_popup
#[allow(dead_code)] // Phase 6: popup surfaces
pub const XDG_SURFACE_GET_POPUP: u16 = 2;
/// set_window_geometry
pub const XDG_SURFACE_SET_WINDOW_GEOMETRY: u16 = 3;
/// ack_configure
pub const XDG_SURFACE_ACK_CONFIGURE: u16 = 4;

// xdg_surface event opcodes
/// configure
pub const XDG_SURFACE_CONFIGURE: u16 = 0;

// xdg_toplevel opcodes (requests)
/// destroy
#[allow(dead_code)] // Phase 6: toplevel teardown
pub const XDG_TOPLEVEL_DESTROY: u16 = 0;
/// set_parent
#[allow(dead_code)] // Phase 6: transient window chains
pub const XDG_TOPLEVEL_SET_PARENT: u16 = 1;
/// set_title
pub const XDG_TOPLEVEL_SET_TITLE: u16 = 2;
/// set_app_id
pub const XDG_TOPLEVEL_SET_APP_ID: u16 = 3;
/// move (interactive)
#[allow(dead_code)] // Phase 6: interactive move via pointer grab
pub const XDG_TOPLEVEL_MOVE: u16 = 5;
/// resize (interactive)
#[allow(dead_code)] // Phase 6: interactive resize via pointer grab
pub const XDG_TOPLEVEL_RESIZE: u16 = 6;
/// set_max_size
#[allow(dead_code)] // Phase 6: size constraints
pub const XDG_TOPLEVEL_SET_MAX_SIZE: u16 = 7;
/// set_min_size
#[allow(dead_code)] // Phase 6: size constraints
pub const XDG_TOPLEVEL_SET_MIN_SIZE: u16 = 8;
/// set_maximized
pub const XDG_TOPLEVEL_SET_MAXIMIZED: u16 = 9;
/// unset_maximized
#[allow(dead_code)] // Phase 6: unmaximize
pub const XDG_TOPLEVEL_UNSET_MAXIMIZED: u16 = 10;
/// set_fullscreen
pub const XDG_TOPLEVEL_SET_FULLSCREEN: u16 = 11;
/// unset_fullscreen
#[allow(dead_code)] // Phase 6: unfullscreen
pub const XDG_TOPLEVEL_UNSET_FULLSCREEN: u16 = 12;
/// set_minimized
pub const XDG_TOPLEVEL_SET_MINIMIZED: u16 = 13;

// xdg_toplevel event opcodes
/// configure
pub const XDG_TOPLEVEL_CONFIGURE: u16 = 0;
/// close
pub const XDG_TOPLEVEL_CLOSE: u16 = 1;

// xdg_toplevel state enum values (used in configure event's states array)
/// maximized state
pub const XDG_TOPLEVEL_STATE_MAXIMIZED: u32 = 1;
/// fullscreen state
pub const XDG_TOPLEVEL_STATE_FULLSCREEN: u32 = 2;
/// resizing state
#[allow(dead_code)] // Phase 6: interactive resize feedback
pub const XDG_TOPLEVEL_STATE_RESIZING: u32 = 3;
/// activated (focused) state
pub const XDG_TOPLEVEL_STATE_ACTIVATED: u32 = 4;

// ---------------------------------------------------------------------------
// XDG Decoration constants (zxdg_decoration_manager_v1)
// ---------------------------------------------------------------------------

/// Wayland global interface name for decoration manager
pub const ZXDG_DECORATION_MANAGER_V1: &str = "zxdg_decoration_manager_v1";

/// Protocol version
pub const ZXDG_DECORATION_MANAGER_V1_VERSION: u32 = 1;

// Manager request opcodes
/// destroy
#[allow(dead_code)] // Phase 7: decoration manager teardown
pub const ZXDG_DECORATION_MANAGER_V1_DESTROY: u16 = 0;
/// get_toplevel_decoration(id: new_id, toplevel: object)
#[allow(dead_code)] // Phase 7: decoration negotiation
pub const ZXDG_DECORATION_MANAGER_V1_GET_TOPLEVEL_DECORATION: u16 = 1;

// Toplevel decoration request opcodes
/// destroy
#[allow(dead_code)] // Phase 7: decoration cleanup
pub const ZXDG_TOPLEVEL_DECORATION_V1_DESTROY: u16 = 0;
/// set_mode(mode: uint)
#[allow(dead_code)] // Phase 7: client decoration preference
pub const ZXDG_TOPLEVEL_DECORATION_V1_SET_MODE: u16 = 1;
/// unset_mode
#[allow(dead_code)] // Phase 7: client reverts to compositor preference
pub const ZXDG_TOPLEVEL_DECORATION_V1_UNSET_MODE: u16 = 2;

// Toplevel decoration event opcodes
/// configure(mode: uint)
#[allow(dead_code)] // Phase 7: compositor announces chosen mode
pub const ZXDG_TOPLEVEL_DECORATION_V1_CONFIGURE: u16 = 0;

// Decoration mode constants
/// Decorations are drawn by the client
#[allow(dead_code)] // Phase 7: CSD mode
pub const ZXDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE: u32 = 1;
/// Decorations are drawn by the compositor (server)
#[allow(dead_code)] // Phase 7: SSD mode
pub const ZXDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE: u32 = 2;

/// Server-side vs client-side decoration preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationMode {
    /// Client draws its own title bar, borders, etc.
    ClientSide,
    /// Compositor draws title bar, borders, etc.
    ServerSide,
}

impl DecorationMode {
    /// Parse from the wire `u32` value.
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            ZXDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE => Some(Self::ClientSide),
            ZXDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE => Some(Self::ServerSide),
            _ => None,
        }
    }

    /// Convert to the wire `u32` value.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::ClientSide => ZXDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE,
            Self::ServerSide => ZXDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE,
        }
    }
}

/// Per-toplevel decoration state negotiated between client and compositor.
#[allow(dead_code)] // Phase 7: full decoration negotiation lifecycle
pub struct ToplevelDecoration {
    /// Decoration object ID
    pub id: u32,
    /// Associated toplevel ID
    pub toplevel_id: u32,
    /// The mode that the compositor decided on
    pub mode: DecorationMode,
}

/// Negotiate decoration mode for a toplevel.
///
/// VeridianOS always prefers server-side decorations so that the compositor
/// draws title bars, close buttons, and window borders uniformly.
pub fn negotiate_decoration(_client_preference: Option<DecorationMode>) -> DecorationMode {
    // The compositor always chooses SSD for a consistent desktop appearance.
    DecorationMode::ServerSide
}

// ---------------------------------------------------------------------------
// Window state
// ---------------------------------------------------------------------------

/// Window state for an xdg_toplevel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
}

// ---------------------------------------------------------------------------
// XDG Surface
// ---------------------------------------------------------------------------

/// An xdg_surface wraps a wl_surface with desktop shell semantics.
pub struct XdgSurface {
    /// xdg_surface object ID
    pub id: u32,
    /// Underlying wl_surface ID
    pub surface_id: u32,
    /// Whether the client has ack'd the latest configure
    pub configured: bool,
    /// Last sent configure serial
    pub configure_serial: u32,
    /// Window geometry (client-set visible bounds)
    pub geometry: Option<(i32, i32, u32, u32)>,
    /// Associated toplevel (if any)
    pub toplevel: Option<XdgToplevel>,
}

impl XdgSurface {
    pub fn new(id: u32, surface_id: u32) -> Self {
        Self {
            id,
            surface_id,
            configured: false,
            configure_serial: 0,
            geometry: None,
            toplevel: None,
        }
    }

    /// Handle ack_configure from the client.
    pub fn ack_configure(&mut self, serial: u32) -> bool {
        if serial == self.configure_serial {
            self.configured = true;
            true
        } else {
            false
        }
    }

    /// Set window geometry.
    pub fn set_geometry(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.geometry = Some((x, y, width, height));
    }
}

// ---------------------------------------------------------------------------
// XDG Toplevel
// ---------------------------------------------------------------------------

/// An xdg_toplevel represents a standard desktop window.
pub struct XdgToplevel {
    /// Toplevel object ID
    pub id: u32,
    /// Parent xdg_surface ID
    pub xdg_surface_id: u32,
    /// Window title (set by client)
    pub title: String,
    /// Application identifier
    pub app_id: String,
    /// Current window state
    pub state: WindowState,
    /// Whether this toplevel is activated (focused)
    pub activated: bool,
    /// Minimum size constraint (0,0 = no constraint)
    #[allow(dead_code)] // Phase 6: size constraint enforcement
    pub min_size: (u32, u32),
    /// Maximum size constraint (0,0 = no constraint)
    #[allow(dead_code)] // Phase 6: size constraint enforcement
    pub max_size: (u32, u32),
}

impl XdgToplevel {
    pub fn new(id: u32, xdg_surface_id: u32) -> Self {
        Self {
            id,
            xdg_surface_id,
            title: String::new(),
            app_id: String::new(),
            state: WindowState::Normal,
            activated: false,
            min_size: (0, 0),
            max_size: (0, 0),
        }
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn set_app_id(&mut self, app_id: String) {
        self.app_id = app_id;
    }

    pub fn set_maximized(&mut self) {
        self.state = WindowState::Maximized;
    }

    pub fn set_fullscreen(&mut self) {
        self.state = WindowState::Fullscreen;
    }

    pub fn set_minimized(&mut self) {
        self.state = WindowState::Minimized;
    }

    #[allow(dead_code)] // Phase 6: unmaximize/unfullscreen restore
    pub fn set_normal(&mut self) {
        self.state = WindowState::Normal;
    }
}

// ---------------------------------------------------------------------------
// XDG Shell Manager (xdg_wm_base)
// ---------------------------------------------------------------------------

/// Manages the xdg_wm_base protocol state: ping/pong, xdg_surface and
/// xdg_toplevel lifecycle.
pub struct XdgShell {
    /// All xdg_surfaces keyed by their object ID
    xdg_surfaces: BTreeMap<u32, XdgSurface>,
    /// Next configure serial
    next_serial: u32,
    /// Pending ping serial (None = no outstanding ping)
    pending_ping: Option<u32>,
    /// Whether the client responded to the last ping
    #[allow(dead_code)] // Phase 6: unresponsive client detection
    client_alive: bool,
}

impl XdgShell {
    pub fn new() -> Self {
        Self {
            xdg_surfaces: BTreeMap::new(),
            next_serial: 1,
            pending_ping: None,
            client_alive: true,
        }
    }

    /// Allocate the next serial number.
    fn next_serial(&mut self) -> u32 {
        let s = self.next_serial;
        self.next_serial += 1;
        s
    }

    // -- xdg_wm_base requests -----------------------------------------------

    /// Handle xdg_wm_base.pong from the client.
    pub fn handle_pong(&mut self, serial: u32) -> bool {
        if self.pending_ping == Some(serial) {
            self.pending_ping = None;
            self.client_alive = true;
            true
        } else {
            false
        }
    }

    /// Build a ping event to send to the client.
    pub fn build_ping(&mut self, wm_base_id: u32) -> Vec<u8> {
        let serial = self.next_serial();
        self.pending_ping = Some(serial);
        protocol::serialize_message(&WaylandMessage::new(
            wm_base_id,
            XDG_WM_BASE_PING,
            vec![Argument::Uint(serial)],
        ))
    }

    // -- xdg_surface lifecycle ----------------------------------------------

    /// Create a new xdg_surface wrapping a wl_surface.
    pub fn create_xdg_surface(
        &mut self,
        xdg_surface_id: u32,
        surface_id: u32,
    ) -> Result<(), KernelError> {
        let xdg = XdgSurface::new(xdg_surface_id, surface_id);
        self.xdg_surfaces.insert(xdg_surface_id, xdg);
        Ok(())
    }

    /// Get a reference to an xdg_surface.
    pub fn get_xdg_surface(&self, id: u32) -> Option<&XdgSurface> {
        self.xdg_surfaces.get(&id)
    }

    /// Get a mutable reference to an xdg_surface.
    pub fn get_xdg_surface_mut(&mut self, id: u32) -> Option<&mut XdgSurface> {
        self.xdg_surfaces.get_mut(&id)
    }

    /// Destroy an xdg_surface.
    #[allow(dead_code)] // Phase 6: surface cleanup
    pub fn destroy_xdg_surface(&mut self, id: u32) -> bool {
        self.xdg_surfaces.remove(&id).is_some()
    }

    /// Find and mutate a toplevel by its object ID.
    ///
    /// Scans all xdg_surfaces for a toplevel matching `toplevel_id` and calls
    /// `f` on it if found.
    pub fn with_toplevel_mut<R, F: FnOnce(&mut XdgToplevel) -> R>(
        &mut self,
        toplevel_id: u32,
        f: F,
    ) -> Option<R> {
        for xdg in self.xdg_surfaces.values_mut() {
            if let Some(ref mut tl) = xdg.toplevel {
                if tl.id == toplevel_id {
                    return Some(f(tl));
                }
            }
        }
        None
    }

    // -- xdg_toplevel lifecycle ---------------------------------------------

    /// Create a toplevel role on an xdg_surface.
    pub fn create_toplevel(
        &mut self,
        xdg_surface_id: u32,
        toplevel_id: u32,
    ) -> Result<(), KernelError> {
        let xdg = self
            .xdg_surfaces
            .get_mut(&xdg_surface_id)
            .ok_or(KernelError::NotFound {
                resource: "xdg_surface",
                id: xdg_surface_id as u64,
            })?;

        if xdg.toplevel.is_some() {
            return Err(KernelError::AlreadyExists {
                resource: "xdg_toplevel",
                id: toplevel_id as u64,
            });
        }

        xdg.toplevel = Some(XdgToplevel::new(toplevel_id, xdg_surface_id));
        Ok(())
    }

    // -- Configure events ---------------------------------------------------

    /// Build an xdg_toplevel.configure event followed by xdg_surface.configure.
    ///
    /// This is the initial configure sequence that must be sent before the
    /// client can attach buffers.
    pub fn build_initial_configure(
        &mut self,
        xdg_surface_id: u32,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let serial = self.next_serial();

        if let Some(xdg) = self.xdg_surfaces.get_mut(&xdg_surface_id) {
            xdg.configure_serial = serial;
        }

        let mut events = Vec::new();

        // xdg_toplevel.configure(width, height, states)
        if let Some(xdg) = self.xdg_surfaces.get(&xdg_surface_id) {
            if let Some(ref toplevel) = xdg.toplevel {
                let mut states_data = Vec::new();
                if toplevel.activated {
                    states_data.extend_from_slice(&XDG_TOPLEVEL_STATE_ACTIVATED.to_ne_bytes());
                }
                match toplevel.state {
                    WindowState::Maximized => {
                        states_data.extend_from_slice(&XDG_TOPLEVEL_STATE_MAXIMIZED.to_ne_bytes());
                    }
                    WindowState::Fullscreen => {
                        states_data.extend_from_slice(&XDG_TOPLEVEL_STATE_FULLSCREEN.to_ne_bytes());
                    }
                    _ => {}
                }

                let toplevel_configure = WaylandMessage::new(
                    toplevel.id,
                    XDG_TOPLEVEL_CONFIGURE,
                    vec![
                        Argument::Int(width as i32),
                        Argument::Int(height as i32),
                        Argument::Array(states_data),
                    ],
                );
                events.extend_from_slice(&protocol::serialize_message(&toplevel_configure));
            }
        }

        // xdg_surface.configure(serial)
        let surface_configure = WaylandMessage::new(
            xdg_surface_id,
            XDG_SURFACE_CONFIGURE,
            vec![Argument::Uint(serial)],
        );
        events.extend_from_slice(&protocol::serialize_message(&surface_configure));

        events
    }

    /// Build an xdg_toplevel.close event.
    pub fn build_close_event(&self, xdg_surface_id: u32) -> Option<Vec<u8>> {
        let xdg = self.xdg_surfaces.get(&xdg_surface_id)?;
        let toplevel = xdg.toplevel.as_ref()?;

        Some(protocol::serialize_message(&WaylandMessage::new(
            toplevel.id,
            XDG_TOPLEVEL_CLOSE,
            vec![],
        )))
    }
}

impl Default for XdgShell {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global XDG Shell instance
// ---------------------------------------------------------------------------

static XDG_SHELL: RwLock<Option<XdgShell>> = RwLock::new(None);

/// Initialize the XDG shell manager.
pub fn init_xdg_shell() {
    let mut shell = XDG_SHELL.write();
    if shell.is_none() {
        *shell = Some(XdgShell::new());
    }
}

/// Execute a closure with read access to the XDG shell.
#[allow(dead_code)] // Phase 6: xdg queries from window manager
pub fn with_xdg_shell<R, F: FnOnce(&XdgShell) -> R>(f: F) -> Option<R> {
    let guard = XDG_SHELL.read();
    guard.as_ref().map(f)
}

/// Execute a closure with mutable access to the XDG shell.
pub fn with_xdg_shell_mut<R, F: FnOnce(&mut XdgShell) -> R>(f: F) -> Option<R> {
    let mut guard = XDG_SHELL.write();
    guard.as_mut().map(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xdg_toplevel_title() {
        let mut tl = XdgToplevel::new(1, 1);
        tl.set_title(String::from("Hello"));
        assert_eq!(tl.title, "Hello");
    }

    #[test]
    fn test_xdg_toplevel_state() {
        let mut tl = XdgToplevel::new(1, 1);
        assert_eq!(tl.state, WindowState::Normal);
        tl.set_maximized();
        assert_eq!(tl.state, WindowState::Maximized);
        tl.set_fullscreen();
        assert_eq!(tl.state, WindowState::Fullscreen);
    }

    #[test]
    fn test_xdg_shell_ping_pong() {
        let mut shell = XdgShell::new();
        let _ping_event = shell.build_ping(10);
        let serial = shell.pending_ping.unwrap();
        assert!(shell.handle_pong(serial));
        assert!(shell.pending_ping.is_none());
    }

    #[test]
    fn test_xdg_surface_configure() {
        let mut shell = XdgShell::new();
        shell.create_xdg_surface(5, 3).unwrap();
        shell.create_toplevel(5, 6).unwrap();

        let events = shell.build_initial_configure(5, 800, 600);
        assert!(!events.is_empty());

        // ack_configure with correct serial
        let serial = shell.get_xdg_surface(5).unwrap().configure_serial;
        assert!(shell.get_xdg_surface_mut(5).unwrap().ack_configure(serial));
    }

    #[test]
    fn test_xdg_surface_ack_wrong_serial() {
        let mut shell = XdgShell::new();
        shell.create_xdg_surface(5, 3).unwrap();
        // Wrong serial
        assert!(!shell.get_xdg_surface_mut(5).unwrap().ack_configure(999));
    }

    #[test]
    fn test_close_event() {
        let mut shell = XdgShell::new();
        shell.create_xdg_surface(5, 3).unwrap();
        shell.create_toplevel(5, 6).unwrap();
        let close = shell.build_close_event(5);
        assert!(close.is_some());
    }
}
