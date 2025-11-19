//! GUI File Manager Application
//!
//! Provides graphical file browsing and management using the window manager and VFS.

use crate::error::KernelError;
use crate::desktop::window_manager::{WindowId, get_window_manager, InputEvent};
use crate::desktop::font::{get_font_manager, FontSize, FontStyle};
use crate::fs::{get_vfs, NodeType};
use crate::sync::once_lock::GlobalState;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;
use spin::RwLock;

/// File entry in the browser
#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    node_type: NodeType,
    size: usize,
    selected: bool,
}

/// File manager state
pub struct FileManager {
    /// Window ID
    window_id: WindowId,

    /// Current directory path
    current_path: String,

    /// File entries in current directory
    entries: Vec<FileEntry>,

    /// Selected entry index
    selected_index: usize,

    /// Scroll offset
    scroll_offset: usize,

    /// Window dimensions
    width: u32,
    height: u32,
}

impl FileManager {
    /// Create a new file manager
    pub fn new() -> Result<Self, KernelError> {
        let width = 640;
        let height = 480;

        // Create window
        let window_id = with_window_manager(|wm| wm.create_window(200, 100, width, height, 0))
            .ok_or(KernelError::InvalidState {
                expected: "initialized",
                actual: "uninitialized",
            })??;

        let mut fm = Self {
            window_id,
            current_path: String::from("/"),
            entries: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            width,
            height,
        };

        // Load initial directory
        fm.refresh_directory()?;

        println!("[FILE-MANAGER] Created file manager window {}", window_id);

        Ok(fm)
    }

    /// Refresh directory listing
    pub fn refresh_directory(&mut self) -> Result<(), KernelError> {
        println!("[FILE-MANAGER] Refreshing directory: {}", self.current_path);

        self.entries.clear();

        // Get VFS
        let vfs = get_vfs();

        // Open directory
        match vfs.read().open(&self.current_path, crate::fs::file::OpenFlags::read_only()) {
            Ok(dir_node) => {
                // List directory contents
                match dir_node.readdir() {
                    Ok(entries) => {
                        for entry in entries {
                            self.entries.push(FileEntry {
                                name: entry.name,
                                node_type: entry.node_type,
                                size: 0, // Size not available in DirEntry
                                selected: false,
                            });
                        }
                    }
                    Err(_) => {
                        println!("[FILE-MANAGER] Failed to read directory");
                    }
                }
            }
            Err(_) => {
                println!("[FILE-MANAGER] Failed to open directory");
            }
        }

        // Sort entries: directories first, then files
        self.entries.sort_by(|a, b| {
            match (a.node_type, b.node_type) {
                (NodeType::Directory, NodeType::File) => core::cmp::Ordering::Less,
                (NodeType::File, NodeType::Directory) => core::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        println!("[FILE-MANAGER] Loaded {} entries", self.entries.len());

        Ok(())
    }

    /// Process input event
    pub fn process_input(&mut self, event: InputEvent) -> Result<(), KernelError> {
        match event {
            InputEvent::KeyPress { character, .. } => {
                match character {
                    '\n' | '\r' => {
                        // Enter - open selected entry
                        self.open_selected()?;
                    }
                    'j' | 'J' => {
                        // Down
                        if self.selected_index < self.entries.len().saturating_sub(1) {
                            self.selected_index += 1;
                        }
                    }
                    'k' | 'K' => {
                        // Up
                        if self.selected_index > 0 {
                            self.selected_index -= 1;
                        }
                    }
                    'h' | 'H' => {
                        // Back / parent directory
                        self.navigate_parent()?;
                    }
                    'r' | 'R' => {
                        // Refresh
                        self.refresh_directory()?;
                    }
                    _ => {}
                }
            }
            InputEvent::MouseButton { button: 0, pressed: true, x, y } => {
                // Left click - select item
                self.handle_click(x, y)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle mouse click
    fn handle_click(&mut self, x: i32, y: i32) -> Result<(), KernelError> {
        // Calculate which entry was clicked
        let line_height = 20;
        let header_height = 40;

        if y > header_height {
            let entry_index = ((y - header_height) / line_height) as usize + self.scroll_offset;
            if entry_index < self.entries.len() {
                self.selected_index = entry_index;
            }
        }

        Ok(())
    }

    /// Open selected entry
    fn open_selected(&mut self) -> Result<(), KernelError> {
        if self.selected_index >= self.entries.len() {
            return Ok(());
        }

        let entry = &self.entries[self.selected_index];

        match entry.node_type {
            NodeType::Directory => {
                // Navigate into directory
                if self.current_path == "/" {
                    self.current_path = format!("/{}", entry.name);
                } else {
                    self.current_path = format!("{}/{}", self.current_path, entry.name);
                }
                self.selected_index = 0;
                self.scroll_offset = 0;
                self.refresh_directory()?;
            }
            NodeType::File => {
                // TODO: Open file in appropriate application
                println!("[FILE-MANAGER] Opening file: {}", entry.name);
            }
            _ => {}
        }

        Ok(())
    }

    /// Navigate to parent directory
    fn navigate_parent(&mut self) -> Result<(), KernelError> {
        if self.current_path == "/" {
            return Ok(()); // Already at root
        }

        // Find last '/' and truncate
        if let Some(pos) = self.current_path.rfind('/') {
            if pos == 0 {
                self.current_path = String::from("/");
            } else {
                self.current_path.truncate(pos);
            }
            self.selected_index = 0;
            self.scroll_offset = 0;
            self.refresh_directory()?;
        }

        Ok(())
    }

    /// Render file manager
    pub fn render(&self, framebuffer: &mut [u8], fb_width: usize, fb_height: usize) -> Result<(), KernelError> {
        // Clear background
        for pixel in framebuffer.iter_mut() {
            *pixel = 32; // Dark gray
        }

        // Get font
        let font_manager = get_font_manager()?;
        let font = font_manager.get_font(FontSize::Medium, FontStyle::Regular)
            .ok_or(KernelError::NotFound { resource: "font", id: 0 })?;

        // Render header
        let header = format!("File Manager - {}", self.current_path);
        let _ = font.render_text(&header, framebuffer, fb_width, fb_height, 10, 10);

        // Render entries
        let line_height = 20;
        let start_y = 40;

        for (i, entry) in self.entries.iter().enumerate().skip(self.scroll_offset) {
            let y = start_y + (i - self.scroll_offset) as i32 * line_height;

            // Highlight selected
            if i == self.selected_index {
                // Draw selection highlight (simplified)
                for dy in 0..line_height {
                    for dx in 0..fb_width.min(self.width as usize) {
                        let py = y + dy;
                        if py >= 0 && (py as usize) < fb_height {
                            let index = py as usize * fb_width + dx;
                            if index < framebuffer.len() {
                                framebuffer[index] = 64; // Lighter gray
                            }
                        }
                    }
                }
            }

            // Render entry
            let prefix = match entry.node_type {
                NodeType::Directory => "[DIR]  ",
                NodeType::File => "[FILE] ",
                _ => "[?]    ",
            };

            let entry_text = format!("{}{}", prefix, entry.name);
            let _ = font.render_text(&entry_text, framebuffer, fb_width, fb_height, 15, y);
        }

        Ok(())
    }

    /// Get window ID
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }
}

/// Global file manager (can support multiple instances)
static FILE_MANAGER: GlobalState<RwLock<FileManager>> = GlobalState::new();

/// Initialize file manager
pub fn init() -> Result<(), KernelError> {
    println!("[FILE-MANAGER] File manager initialized");
    Ok(())
}

/// Create a new file manager instance
pub fn create_file_manager() -> Result<(), KernelError> {
    let fm = FileManager::new()?;
    FILE_MANAGER.init(RwLock::new(fm)).map_err(|_| KernelError::InvalidState {
        expected: "uninitialized",
        actual: "initialized",
    })?;
    Ok(())
}

/// Execute a function with the file manager
pub fn with_file_manager<R, F: FnOnce(&RwLock<FileManager>) -> R>(f: F) -> Option<R> {
    FILE_MANAGER.with(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_file_entry_creation() {
        let entry = FileEntry {
            name: String::from("test.txt"),
            node_type: NodeType::File,
            size: 1024,
            selected: false,
        };

        assert_eq!(entry.name, "test.txt");
        assert_eq!(entry.size, 1024);
    }
}
