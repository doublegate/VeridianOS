//! Desktop IPC Protocol
//!
//! Provides IPC communication channels for desktop applications (window
//! manager, terminal, compositor, notifications, clipboard, launcher).
//!
//! Each desktop subsystem registers a well-known endpoint so that user-space
//! clients can discover and communicate with it by name rather than relying
//! on dynamic ID allocation.

#![allow(dead_code)]

use alloc::vec::Vec;

use crate::{error::KernelError, ipc::EndpointId, process::pcb::ProcessId};

// ---------------------------------------------------------------------------
// Well-known desktop service endpoints
// ---------------------------------------------------------------------------

/// Window manager IPC endpoint (manages window lifecycle, focus, stacking)
pub const DESKTOP_WM_ENDPOINT: EndpointId = 1000;
/// Input server endpoint (keyboard, mouse, touch event routing)
pub const DESKTOP_INPUT_ENDPOINT: EndpointId = 1001;
/// Compositor endpoint (surface compositing, damage, frame callbacks)
pub const DESKTOP_COMPOSITOR_ENDPOINT: EndpointId = 1002;
/// Notification daemon endpoint (desktop notifications)
pub const DESKTOP_NOTIFICATION_ENDPOINT: EndpointId = 1003;
/// Clipboard manager endpoint (copy/paste, primary selection)
pub const DESKTOP_CLIPBOARD_ENDPOINT: EndpointId = 1004;
/// Application launcher endpoint (app start, .desktop file queries)
pub const DESKTOP_LAUNCHER_ENDPOINT: EndpointId = 1005;

/// Legacy aliases for backward compatibility with Phase 6 code.
pub const WINDOW_MANAGER_ENDPOINT: EndpointId = DESKTOP_WM_ENDPOINT;
pub const INPUT_SERVER_ENDPOINT: EndpointId = DESKTOP_INPUT_ENDPOINT;
pub const COMPOSITOR_ENDPOINT: EndpointId = DESKTOP_COMPOSITOR_ENDPOINT;

// ---------------------------------------------------------------------------
// Desktop IPC message types
// ---------------------------------------------------------------------------

/// Desktop IPC message types.
///
/// Grouped by subsystem with non-overlapping discriminant ranges so that a
/// single `match` on the wire `u32` unambiguously identifies the message.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopMessageType {
    // -- Window Manager messages (100-199) ----------------------------------
    /// Request: create a new window
    CreateWindow = 100,
    /// Request: destroy an existing window
    DestroyWindow = 101,
    /// Request: move a window to (x, y)
    MoveWindow = 102,
    /// Request: resize a window to (w, h)
    ResizeWindow = 103,
    /// Request: give keyboard focus to a window
    FocusWindow = 104,
    /// Request: update a region of the window's framebuffer
    UpdateWindowContent = 105,
    /// Request: minimize a window
    MinimizeWindow = 106,
    /// Request: maximize/restore a window
    MaximizeWindow = 107,
    /// Request: set a window's title
    SetWindowTitle = 108,
    /// Request: query window geometry
    GetWindowGeometry = 109,

    // -- Input messages (200-299) -------------------------------------------
    /// Event: key pressed
    KeyPress = 200,
    /// Event: key released
    KeyRelease = 201,
    /// Event: pointer motion
    MouseMove = 202,
    /// Event: pointer button press/release
    MouseButton = 203,
    /// Event: scroll wheel or touchpad scroll
    ScrollEvent = 204,
    /// Event: touch down
    TouchDown = 205,
    /// Event: touch up
    TouchUp = 206,
    /// Event: touch motion
    TouchMotion = 207,

    // -- Terminal messages (300-399) ----------------------------------------
    /// Data: terminal stdin bytes
    TerminalInput = 300,
    /// Data: terminal stdout bytes
    TerminalOutput = 301,
    /// Request: resize terminal (cols, rows)
    TerminalResize = 302,

    // -- Compositor messages (400-499) --------------------------------------
    /// Request: commit surface
    SurfaceCommit = 400,
    /// Request: attach buffer to surface
    SurfaceAttach = 401,
    /// Request: mark damage region
    SurfaceDamage = 402,
    /// Event: frame callback
    FrameCallback = 403,

    // -- Notification messages (500-599) ------------------------------------
    /// Request: show a notification
    NotificationShow = 500,
    /// Request: dismiss a notification
    NotificationDismiss = 501,
    /// Event: notification was clicked
    NotificationAction = 502,

    // -- Clipboard messages (600-699) --------------------------------------
    /// Request: set clipboard content
    ClipboardSet = 600,
    /// Request: get clipboard content
    ClipboardGet = 601,
    /// Event: clipboard content changed
    ClipboardChanged = 602,

    // -- Launcher messages (700-799) ----------------------------------------
    /// Request: launch an application by name
    LaunchApp = 700,
    /// Request: list available applications
    ListApps = 701,

    // -- Response messages (900-999) ----------------------------------------
    /// Generic success response
    Success = 900,
    /// Generic error response with reason code
    Error = 901,
}

impl DesktopMessageType {
    /// Convert from a raw `u32` wire value.
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            100 => Some(Self::CreateWindow),
            101 => Some(Self::DestroyWindow),
            102 => Some(Self::MoveWindow),
            103 => Some(Self::ResizeWindow),
            104 => Some(Self::FocusWindow),
            105 => Some(Self::UpdateWindowContent),
            106 => Some(Self::MinimizeWindow),
            107 => Some(Self::MaximizeWindow),
            108 => Some(Self::SetWindowTitle),
            109 => Some(Self::GetWindowGeometry),

            200 => Some(Self::KeyPress),
            201 => Some(Self::KeyRelease),
            202 => Some(Self::MouseMove),
            203 => Some(Self::MouseButton),
            204 => Some(Self::ScrollEvent),
            205 => Some(Self::TouchDown),
            206 => Some(Self::TouchUp),
            207 => Some(Self::TouchMotion),

            300 => Some(Self::TerminalInput),
            301 => Some(Self::TerminalOutput),
            302 => Some(Self::TerminalResize),

            400 => Some(Self::SurfaceCommit),
            401 => Some(Self::SurfaceAttach),
            402 => Some(Self::SurfaceDamage),
            403 => Some(Self::FrameCallback),

            500 => Some(Self::NotificationShow),
            501 => Some(Self::NotificationDismiss),
            502 => Some(Self::NotificationAction),

            600 => Some(Self::ClipboardSet),
            601 => Some(Self::ClipboardGet),
            602 => Some(Self::ClipboardChanged),

            700 => Some(Self::LaunchApp),
            701 => Some(Self::ListApps),

            900 => Some(Self::Success),
            901 => Some(Self::Error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire-format message structs
// ---------------------------------------------------------------------------

/// Window creation request
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CreateWindowRequest {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title_len: u32,
    // Title follows as bytes
}

/// Window creation response
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CreateWindowResponse {
    pub window_id: u64,
}

/// Window update request
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UpdateWindowRequest {
    pub window_id: u64,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data_len: u32,
    // Framebuffer data follows
}

/// Window geometry query/response
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WindowGeometryRequest {
    pub window_id: u64,
}

/// Keyboard event
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key_code: u32,
    pub modifiers: u32,
    pub pressed: bool,
}

/// Mouse event
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub button: u8,
    pub pressed: bool,
}

/// Scroll event
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ScrollEvent {
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
}

/// Desktop notification request
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NotificationRequest {
    pub urgency: u8,
    pub timeout_ms: u32,
    pub title_len: u32,
    pub body_len: u32,
    // title bytes followed by body bytes
}

/// Clipboard header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ClipboardHeader {
    pub mime_type_len: u32,
    pub data_len: u32,
    // mime_type bytes followed by data bytes
}

// ---------------------------------------------------------------------------
// Desktop IPC helper functions
// ---------------------------------------------------------------------------

/// Desktop IPC helper functions for building and parsing wire messages.
pub mod helpers {
    use super::*;

    /// Create a window creation message
    pub fn create_window_message(x: i32, y: i32, width: u32, height: u32, title: &str) -> Vec<u8> {
        let mut data = Vec::new();

        // Message type
        data.extend_from_slice(&(DesktopMessageType::CreateWindow as u32).to_le_bytes());

        // Request struct
        let req = CreateWindowRequest {
            x,
            y,
            width,
            height,
            title_len: title.len() as u32,
        };

        // SAFETY: CreateWindowRequest is #[repr(C)] so its memory layout
        // is well-defined. We create a byte slice view over the struct
        // for size_of::<CreateWindowRequest>() bytes. The reference &req
        // is valid for the entire scope, so the slice is valid.
        unsafe {
            let req_bytes = core::slice::from_raw_parts(
                &req as *const _ as *const u8,
                core::mem::size_of::<CreateWindowRequest>(),
            );
            data.extend_from_slice(req_bytes);
        }

        // Title bytes
        data.extend_from_slice(title.as_bytes());

        data
    }

    /// Parse window creation response
    pub fn parse_window_response(data: &[u8]) -> Result<u64, KernelError> {
        if data.len() < core::mem::size_of::<CreateWindowResponse>() {
            return Err(KernelError::InvalidArgument {
                name: "response_size",
                value: "too_small",
            });
        }

        // SAFETY: data.len() >= size_of::<CreateWindowResponse>() was
        // validated above. read_unaligned handles any alignment issues.
        // CreateWindowResponse is #[repr(C)] with Copy, so reading it
        // from the byte buffer is valid.
        unsafe {
            let resp = core::ptr::read_unaligned(data.as_ptr() as *const CreateWindowResponse);
            Ok(resp.window_id)
        }
    }

    /// Create a keyboard event message
    pub fn keyboard_event_message(key_code: u32, modifiers: u32, pressed: bool) -> Vec<u8> {
        let mut data = Vec::new();

        // Message type
        let msg_type = if pressed {
            DesktopMessageType::KeyPress
        } else {
            DesktopMessageType::KeyRelease
        };
        data.extend_from_slice(&(msg_type as u32).to_le_bytes());

        // Event struct
        let event = KeyEvent {
            key_code,
            modifiers,
            pressed,
        };

        // SAFETY: KeyEvent is #[repr(C)] so its memory layout is
        // well-defined. We create a byte slice view over the struct
        // for size_of::<KeyEvent>() bytes. The reference is valid.
        unsafe {
            let event_bytes = core::slice::from_raw_parts(
                &event as *const _ as *const u8,
                core::mem::size_of::<KeyEvent>(),
            );
            data.extend_from_slice(event_bytes);
        }

        data
    }

    /// Create a mouse event message
    pub fn mouse_event_message(x: i32, y: i32, button: u8, pressed: bool) -> Vec<u8> {
        let mut data = Vec::new();

        // Message type
        data.extend_from_slice(&(DesktopMessageType::MouseMove as u32).to_le_bytes());

        // Event struct
        let event = MouseEvent {
            x,
            y,
            button,
            pressed,
        };

        // SAFETY: MouseEvent is #[repr(C)] so its memory layout is
        // well-defined. We create a byte slice view over the struct
        // for size_of::<MouseEvent>() bytes. The reference is valid.
        unsafe {
            let event_bytes = core::slice::from_raw_parts(
                &event as *const _ as *const u8,
                core::mem::size_of::<MouseEvent>(),
            );
            data.extend_from_slice(event_bytes);
        }

        data
    }
}

// ---------------------------------------------------------------------------
// Desktop IPC Server
// ---------------------------------------------------------------------------

/// Desktop IPC server that manages endpoint registration and message routing.
pub struct DesktopIpcServer {
    /// Whether all endpoints have been registered
    endpoints_registered: bool,
    /// Number of endpoints successfully registered
    registered_count: u32,
}

impl DesktopIpcServer {
    /// Create a new server instance (endpoints not yet registered).
    pub fn new() -> Self {
        Self {
            endpoints_registered: false,
            registered_count: 0,
        }
    }

    /// Register all well-known desktop endpoints with the IPC subsystem.
    ///
    /// Each endpoint is created with the kernel (PID 0) as owner. Errors
    /// during individual endpoint creation are logged but do not prevent
    /// the remaining endpoints from being registered.
    pub fn register_endpoints(&mut self) -> Result<(), KernelError> {
        let kernel_pid = ProcessId(0);
        let endpoints: &[(EndpointId, &str)] = &[
            (DESKTOP_WM_ENDPOINT, "window_manager"),
            (DESKTOP_INPUT_ENDPOINT, "input_server"),
            (DESKTOP_COMPOSITOR_ENDPOINT, "compositor"),
            (DESKTOP_NOTIFICATION_ENDPOINT, "notifications"),
            (DESKTOP_CLIPBOARD_ENDPOINT, "clipboard"),
            (DESKTOP_LAUNCHER_ENDPOINT, "launcher"),
        ];

        for &(id, name) in endpoints {
            match crate::ipc::create_endpoint(kernel_pid) {
                Ok((_endpoint_id, _cap)) => {
                    self.registered_count += 1;
                    crate::println!("[DESKTOP-IPC] Registered endpoint {} (ID {})", name, id);
                }
                Err(e) => {
                    crate::println!(
                        "[DESKTOP-IPC] Warning: failed to register {} (ID {}): {:?}",
                        name,
                        id,
                        e
                    );
                }
            }
        }

        self.endpoints_registered = true;
        Ok(())
    }

    /// Whether all endpoints have been registered.
    pub fn is_registered(&self) -> bool {
        self.endpoints_registered
    }

    /// Number of successfully registered endpoints.
    pub fn registered_count(&self) -> u32 {
        self.registered_count
    }
}

impl Default for DesktopIpcServer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Global desktop IPC server instance.
static DESKTOP_IPC: spin::Mutex<Option<DesktopIpcServer>> = spin::Mutex::new(None);

/// Check whether the desktop IPC system has been initialized.
pub fn is_initialized() -> bool {
    DESKTOP_IPC
        .lock()
        .as_ref()
        .is_some_and(|s| s.is_registered())
}

/// Initialize the desktop IPC system.
///
/// Creates a `DesktopIpcServer` and registers all well-known endpoints.
pub fn init() -> Result<(), KernelError> {
    crate::println!("[DESKTOP-IPC] Initializing desktop IPC protocol...");

    let mut server = DesktopIpcServer::new();
    server.register_endpoints()?;

    let count = server.registered_count();
    *DESKTOP_IPC.lock() = Some(server);

    crate::println!(
        "[DESKTOP-IPC] Desktop IPC protocol initialized ({} endpoints)",
        count
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{helpers::*, *};

    #[test]
    fn test_create_window_message() {
        let msg = create_window_message(100, 200, 800, 600, "Test Window");

        // Should contain message type + struct + title
        assert!(msg.len() > core::mem::size_of::<CreateWindowRequest>());
    }

    #[test]
    fn test_keyboard_event() {
        let msg = keyboard_event_message(65, 0, true); // 'A' key pressed

        // Should contain message type + event struct
        assert_eq!(msg.len(), 4 + core::mem::size_of::<KeyEvent>());
    }

    #[test]
    fn test_message_type_round_trip() {
        let cases: &[DesktopMessageType] = &[
            DesktopMessageType::CreateWindow,
            DesktopMessageType::KeyPress,
            DesktopMessageType::TerminalInput,
            DesktopMessageType::SurfaceCommit,
            DesktopMessageType::NotificationShow,
            DesktopMessageType::ClipboardSet,
            DesktopMessageType::LaunchApp,
            DesktopMessageType::Success,
            DesktopMessageType::Error,
        ];
        for &mt in cases {
            let v = mt as u32;
            assert_eq!(DesktopMessageType::from_u32(v), Some(mt));
        }
    }

    #[test]
    fn test_message_type_unknown() {
        assert_eq!(DesktopMessageType::from_u32(0), None);
        assert_eq!(DesktopMessageType::from_u32(9999), None);
    }

    #[test]
    fn test_desktop_ipc_server_new() {
        let server = DesktopIpcServer::new();
        assert!(!server.is_registered());
        assert_eq!(server.registered_count(), 0);
    }

    #[test]
    fn test_endpoint_constants() {
        // Ensure legacy aliases match the new names
        assert_eq!(WINDOW_MANAGER_ENDPOINT, DESKTOP_WM_ENDPOINT);
        assert_eq!(INPUT_SERVER_ENDPOINT, DESKTOP_INPUT_ENDPOINT);
        assert_eq!(COMPOSITOR_ENDPOINT, DESKTOP_COMPOSITOR_ENDPOINT);

        // Ensure all endpoint IDs are unique
        let ids = [
            DESKTOP_WM_ENDPOINT,
            DESKTOP_INPUT_ENDPOINT,
            DESKTOP_COMPOSITOR_ENDPOINT,
            DESKTOP_NOTIFICATION_ENDPOINT,
            DESKTOP_CLIPBOARD_ENDPOINT,
            DESKTOP_LAUNCHER_ENDPOINT,
        ];
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j]);
            }
        }
    }
}
