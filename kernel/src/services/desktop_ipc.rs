//! Desktop IPC Protocol
//!
//! Provides IPC communication channels for desktop applications (window manager, terminal, etc.)

use crate::error::KernelError;
use crate::ipc::EndpointId;
use alloc::vec::Vec;

/// Desktop IPC message types
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopMessageType {
    // Window Manager Messages
    CreateWindow = 1,
    DestroyWindow = 2,
    MoveWindow = 3,
    ResizeWindow = 4,
    FocusWindow = 5,
    UpdateWindowContent = 6,

    // Input Messages
    KeyPress = 10,
    KeyRelease = 11,
    MouseMove = 12,
    MouseButton = 13,

    // Terminal Messages
    TerminalInput = 20,
    TerminalOutput = 21,

    // Response Messages
    Success = 100,
    Error = 101,
}

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

/// Desktop IPC helper functions
pub mod helpers {
    use super::*;

    /// Create a window creation message
    pub fn create_window_message(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        title: &str,
    ) -> Vec<u8> {
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

        // Unsafe conversion of struct to bytes
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
        let event = MouseEvent { x, y, button, pressed };

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

/// Desktop service endpoints (well-known endpoint IDs)
pub const WINDOW_MANAGER_ENDPOINT: EndpointId = 1000;
pub const INPUT_SERVER_ENDPOINT: EndpointId = 1001;
pub const COMPOSITOR_ENDPOINT: EndpointId = 1002;

/// Initialize desktop IPC system
pub fn init() -> Result<(), KernelError> {
    println!("[DESKTOP-IPC] Initializing desktop IPC protocol...");

    // TODO: Register well-known endpoints
    // TODO: Create IPC channels for window manager, input server, compositor

    println!("[DESKTOP-IPC] Desktop IPC protocol initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::helpers::*;

    #[test_case]
    fn test_create_window_message() {
        let msg = create_window_message(100, 200, 800, 600, "Test Window");

        // Should contain message type + struct + title
        assert!(msg.len() > core::mem::size_of::<CreateWindowRequest>());
    }

    #[test_case]
    fn test_keyboard_event() {
        let msg = keyboard_event_message(65, 0, true); // 'A' key pressed

        // Should contain message type + event struct
        assert_eq!(
            msg.len(),
            4 + core::mem::size_of::<KeyEvent>()
        );
    }
}
