//! Window Manager with Event Loop
//!
//! Manages windows, input events, and coordinates desktop applications.

use alloc::{collections::BTreeMap, vec::Vec};

use spin::RwLock;

use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Window ID type
pub type WindowId = u32;

/// Window state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
    Hidden,
}

/// Window structure
#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: [u8; 64],
    pub title_len: usize,
    pub state: WindowState,
    pub visible: bool,
    pub focused: bool,
    pub owner_pid: u64,
}

impl Window {
    /// Create a new window
    pub fn new(id: WindowId, x: i32, y: i32, width: u32, height: u32, owner_pid: u64) -> Self {
        Self {
            id,
            x,
            y,
            width,
            height,
            title: [0; 64],
            title_len: 0,
            state: WindowState::Normal,
            visible: true,
            focused: false,
            owner_pid,
        }
    }

    /// Set window title
    pub fn set_title(&mut self, title: &str) {
        let bytes = title.as_bytes();
        let len = bytes.len().min(64);
        self.title[..len].copy_from_slice(&bytes[..len]);
        self.title_len = len;
    }

    /// Get window title as string slice
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }
}

/// Input event types
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    KeyPress {
        scancode: u8,
        character: char,
    },
    KeyRelease {
        scancode: u8,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u8,
        pressed: bool,
        x: i32,
        y: i32,
    },
    MouseScroll {
        delta_x: i16,
        delta_y: i16,
    },
}

/// Window event
#[derive(Debug, Clone)]
pub struct WindowEvent {
    pub window_id: WindowId,
    pub event: InputEvent,
}

/// Window Manager
pub struct WindowManager {
    /// All windows indexed by ID
    windows: RwLock<BTreeMap<WindowId, Window>>,

    /// Window Z-order (bottom to top)
    z_order: RwLock<Vec<WindowId>>,

    /// Currently focused window
    focused_window: RwLock<Option<WindowId>>,

    /// Event queue
    event_queue: RwLock<Vec<WindowEvent>>,

    /// Next window ID
    next_window_id: RwLock<WindowId>,

    /// Mouse cursor position
    mouse_x: RwLock<i32>,
    mouse_y: RwLock<i32>,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new() -> Self {
        Self {
            windows: RwLock::new(BTreeMap::new()),
            z_order: RwLock::new(Vec::new()),
            focused_window: RwLock::new(None),
            event_queue: RwLock::new(Vec::new()),
            next_window_id: RwLock::new(1),
            mouse_x: RwLock::new(0),
            mouse_y: RwLock::new(0),
        }
    }

    /// Create a new window
    pub fn create_window(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        owner_pid: u64,
    ) -> Result<WindowId, KernelError> {
        let id = {
            let mut next_id = self.next_window_id.write();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let window = Window::new(id, x, y, width, height, owner_pid);

        self.windows.write().insert(id, window);
        self.z_order.write().push(id);

        println!("[WM] Created window {} for PID {}", id, owner_pid);

        Ok(id)
    }

    /// Destroy a window
    pub fn destroy_window(&self, window_id: WindowId) -> Result<(), KernelError> {
        self.windows.write().remove(&window_id);
        self.z_order.write().retain(|&id| id != window_id);

        if *self.focused_window.read() == Some(window_id) {
            *self.focused_window.write() = None;
        }

        println!("[WM] Destroyed window {}", window_id);

        Ok(())
    }

    /// Move a window
    pub fn move_window(&self, window_id: WindowId, x: i32, y: i32) -> Result<(), KernelError> {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.x = x;
            window.y = y;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Resize a window
    pub fn resize_window(
        &self,
        window_id: WindowId,
        width: u32,
        height: u32,
    ) -> Result<(), KernelError> {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.width = width;
            window.height = height;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Focus a window
    pub fn focus_window(&self, window_id: WindowId) -> Result<(), KernelError> {
        // Unfocus previous window
        if let Some(prev_id) = *self.focused_window.read() {
            if let Some(prev_window) = self.windows.write().get_mut(&prev_id) {
                prev_window.focused = false;
            }
        }

        // Focus new window
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.focused = true;
            *self.focused_window.write() = Some(window_id);

            // Bring to front
            let mut z_order = self.z_order.write();
            z_order.retain(|&id| id != window_id);
            z_order.push(window_id);

            println!("[WM] Focused window {}", window_id);
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Get window at position
    pub fn window_at_position(&self, x: i32, y: i32) -> Option<WindowId> {
        let windows = self.windows.read();
        let z_order = self.z_order.read();

        // Search from top to bottom
        for &window_id in z_order.iter().rev() {
            if let Some(window) = windows.get(&window_id) {
                if window.visible
                    && x >= window.x
                    && x < window.x + window.width as i32
                    && y >= window.y
                    && y < window.y + window.height as i32
                {
                    return Some(window_id);
                }
            }
        }

        None
    }

    /// Process input event
    pub fn process_input(&self, event: InputEvent) {
        match event {
            InputEvent::MouseMove { x, y } => {
                *self.mouse_x.write() = x;
                *self.mouse_y.write() = y;

                // Send to focused window
                if let Some(window_id) = *self.focused_window.read() {
                    self.queue_event(WindowEvent { window_id, event });
                }
            }
            InputEvent::MouseButton {
                button: _button,
                pressed,
                x,
                y,
            } => {
                if pressed {
                    // Click - focus window at position
                    if let Some(window_id) = self.window_at_position(x, y) {
                        if let Err(_e) = self.focus_window(window_id) {
                            crate::println!(
                                "[WM] Warning: failed to focus window {}: {:?}",
                                window_id,
                                _e
                            );
                        }

                        // Send click event to window
                        self.queue_event(WindowEvent { window_id, event });
                    }
                } else {
                    // Release - send to focused window
                    if let Some(window_id) = *self.focused_window.read() {
                        self.queue_event(WindowEvent { window_id, event });
                    }
                }
            }
            InputEvent::KeyPress { .. }
            | InputEvent::KeyRelease { .. }
            | InputEvent::MouseScroll { .. } => {
                // Send keyboard events to focused window
                if let Some(window_id) = *self.focused_window.read() {
                    self.queue_event(WindowEvent { window_id, event });
                }
            }
        }
    }

    /// Queue an event for delivery
    pub fn queue_event(&self, event: WindowEvent) {
        self.event_queue.write().push(event);
    }

    /// Get pending events for a window
    pub fn get_events(&self, window_id: WindowId) -> Vec<InputEvent> {
        let mut queue = self.event_queue.write();
        let mut events = Vec::new();

        // Extract events for this window
        let mut i = 0;
        while i < queue.len() {
            if queue[i].window_id == window_id {
                events.push(queue.remove(i).event);
            } else {
                i += 1;
            }
        }

        events
    }

    /// Set a window's title.
    pub fn set_window_title(&self, window_id: WindowId, title: &str) {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.set_title(title);
        }
    }

    /// Get a clone of a window by ID.
    pub fn get_window(&self, window_id: WindowId) -> Option<Window> {
        self.windows.read().get(&window_id).cloned()
    }

    /// Get the currently focused window ID.
    pub fn get_focused_window_id(&self) -> Option<WindowId> {
        *self.focused_window.read()
    }

    /// Get all windows
    pub fn get_all_windows(&self) -> Vec<Window> {
        self.windows.read().values().cloned().collect()
    }

    /// Event loop iteration
    pub fn event_loop_iteration(&self) {
        // Process any pending hardware events
        // This would integrate with keyboard/mouse drivers

        // For now, this is a stub showing the structure
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global window manager
static WINDOW_MANAGER: GlobalState<WindowManager> = GlobalState::new();

/// Initialize window manager
pub fn init() -> Result<(), KernelError> {
    let wm = WindowManager::new();
    WINDOW_MANAGER
        .init(wm)
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    println!("[WM] Window manager initialized");
    Ok(())
}

/// Execute a function with the window manager
pub fn with_window_manager<R, F: FnOnce(&WindowManager) -> R>(f: F) -> Option<R> {
    WINDOW_MANAGER.with(f)
}

/// Get the global window manager (deprecated - use with_window_manager instead)
pub fn get_window_manager() -> Result<(), KernelError> {
    WINDOW_MANAGER
        .with(|_| ())
        .ok_or(KernelError::InvalidState {
            expected: "initialized",
            actual: "uninitialized",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_creation() {
        let wm = WindowManager::new();
        let id = wm.create_window(0, 0, 640, 480, 1).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_window_focus() {
        let wm = WindowManager::new();
        let id1 = wm.create_window(0, 0, 640, 480, 1).unwrap();
        let id2 = wm.create_window(100, 100, 640, 480, 1).unwrap();

        wm.focus_window(id1).unwrap();
        assert_eq!(*wm.focused_window.read(), Some(id1));

        wm.focus_window(id2).unwrap();
        assert_eq!(*wm.focused_window.read(), Some(id2));
    }

    #[test]
    fn test_window_at_position() {
        let wm = WindowManager::new();
        let id = wm.create_window(100, 100, 200, 150, 1).unwrap();

        assert_eq!(wm.window_at_position(150, 150), Some(id));
        assert_eq!(wm.window_at_position(50, 50), None);
    }
}
