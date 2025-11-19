//! Window compositor

use super::Rect;
use crate::error::KernelError;
use crate::sync::once_lock::GlobalState;
use alloc::vec::Vec;
use spin::RwLock;

/// Window handle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowId(u32);

/// Window
#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub rect: Rect,
    pub title: &'static str,
    pub visible: bool,
    pub focused: bool,
}

impl Window {
    pub fn new(id: WindowId, rect: Rect, title: &'static str) -> Self {
        Self {
            id,
            rect,
            title,
            visible: true,
            focused: false,
        }
    }
}

/// Compositor state
pub struct Compositor {
    windows: Vec<Window>,
    next_id: u32,
    focused_window: Option<WindowId>,
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            focused_window: None,
        }
    }

    /// Create a new window
    pub fn create_window(&mut self, rect: Rect, title: &'static str) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;

        let window = Window::new(id, rect, title);
        self.windows.push(window);

        if self.focused_window.is_none() {
            self.focused_window = Some(id);
        }

        id
    }

    /// Destroy a window
    pub fn destroy_window(&mut self, id: WindowId) {
        self.windows.retain(|w| w.id != id);
        if self.focused_window == Some(id) {
            self.focused_window = self.windows.first().map(|w| w.id);
        }
    }

    /// Get window
    pub fn get_window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Focus window
    pub fn focus_window(&mut self, id: WindowId) {
        for window in &mut self.windows {
            window.focused = window.id == id;
        }
        self.focused_window = Some(id);
    }

    /// Render all windows
    pub fn render(&mut self) {
        // TODO: Implement actual rendering
        // For now, just iterate windows
        for _window in &self.windows {
            // Would draw window to framebuffer
        }
    }
}

static COMPOSITOR: GlobalState<RwLock<Compositor>> = GlobalState::new();

/// Execute a function with the compositor
pub fn with_compositor<R, F: FnOnce(&mut Compositor) -> R>(f: F) -> Option<R> {
    COMPOSITOR.with(|comp| {
        let mut compositor = comp.write();
        f(&mut compositor)
    })
}

/// Initialize compositor
pub fn init() -> Result<(), KernelError> {
    println!("[COMP] Initializing compositor...");

    COMPOSITOR.init(RwLock::new(Compositor::new())).map_err(|_| KernelError::InvalidState {
        expected: "uninitialized",
        actual: "initialized",
    })?;

    // Create a test window
    with_compositor(|comp| {
        comp.create_window(
            Rect {
                x: 100,
                y: 100,
                width: 640,
                height: 480,
            },
            "VeridianOS",
        )
    });

    println!("[COMP] Compositor initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_compositor_create_window() {
        let mut comp = Compositor::new();
        let id = comp.create_window(
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            "Test",
        );
        assert!(comp.get_window(id).is_some());
    }

    #[test_case]
    fn test_compositor_destroy_window() {
        let mut comp = Compositor::new();
        let id = comp.create_window(
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            "Test",
        );
        comp.destroy_window(id);
        assert!(comp.get_window(id).is_none());
    }
}
