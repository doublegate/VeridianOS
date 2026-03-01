//! GUI File Manager Application
//!
//! Provides graphical file browsing and management using the window manager and
//! VFS.

use alloc::{format, string::String, vec, vec::Vec};

use spin::RwLock;

use crate::{
    desktop::window_manager::{with_window_manager, InputEvent, WindowId},
    error::KernelError,
    fs::{get_vfs, NodeType},
    sync::once_lock::GlobalState,
};

/// File entry in the browser
#[derive(Debug, Clone)]
struct FileEntry {
    name: String,
    node_type: NodeType,
    #[allow(dead_code)] // Set during directory scan; displayed when file details view is added
    size: usize,
    #[allow(dead_code)] // Set via UI interaction; used in multi-select operations (future)
    selected: bool,
}

/// File manager state
pub struct FileManager {
    /// Window ID
    window_id: WindowId,

    /// Compositor surface ID
    surface_id: u32,
    /// SHM pool ID
    pool_id: u32,
    /// Pool buffer ID
    pool_buf_id: u32,

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

        // Create compositor surface
        let (surface_id, pool_id, pool_buf_id) =
            super::renderer::create_app_surface(200, 100, width, height);

        let mut fm = Self {
            window_id,
            surface_id,
            pool_id,
            pool_buf_id,
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
        match vfs
            .read()
            .open(&self.current_path, crate::fs::file::OpenFlags::read_only())
        {
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
        self.entries
            .sort_by(|a, b| match (a.node_type, b.node_type) {
                (NodeType::Directory, NodeType::File) => core::cmp::Ordering::Less,
                (NodeType::File, NodeType::Directory) => core::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });

        // Insert ".." parent directory entry at the top for non-root dirs
        if self.current_path != "/" {
            self.entries.insert(
                0,
                FileEntry {
                    name: String::from(".."),
                    node_type: NodeType::Directory,
                    size: 0,
                    selected: false,
                },
            );
        }

        println!("[FILE-MANAGER] Loaded {} entries", self.entries.len());

        Ok(())
    }

    /// Process input event
    pub fn process_input(&mut self, event: InputEvent) -> Result<(), KernelError> {
        match event {
            InputEvent::KeyPress {
                character,
                scancode,
            } => {
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
                    _ => {
                        // Arrow keys in GUI mode (single-byte 0x80+ codes)
                        match scancode {
                            0x80 => {
                                // KEY_UP
                                if self.selected_index > 0 {
                                    self.selected_index -= 1;
                                }
                            }
                            0x81 => {
                                // KEY_DOWN
                                if self.selected_index < self.entries.len().saturating_sub(1) {
                                    self.selected_index += 1;
                                }
                            }
                            0x82 => {
                                // KEY_LEFT - parent directory
                                self.navigate_parent()?;
                            }
                            0x83 => {
                                // KEY_RIGHT - open selected
                                self.open_selected()?;
                            }
                            _ => {}
                        }
                    }
                }
            }
            InputEvent::MouseButton {
                button: 0,
                pressed: true,
                x,
                y,
            } => {
                // Left click - select item
                self.handle_click(x, y)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle mouse click
    fn handle_click(&mut self, _x: i32, y: i32) -> Result<(), KernelError> {
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
                // Handle ".." parent directory
                if entry.name == ".." {
                    return self.navigate_parent();
                }
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
                // Build full file path
                let file_path = if self.current_path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", self.current_path, entry.name)
                };

                // Read file header bytes for magic-based MIME detection
                let header_bytes = crate::fs::read_file(&file_path).ok().map(|data| {
                    let len = core::cmp::min(data.len(), 512);
                    data[..len].to_vec()
                });

                // Detect MIME type via extension + magic bytes
                let mime = crate::desktop::mime::MimeDatabase::detect_mime(
                    &entry.name,
                    header_bytes.as_deref(),
                );

                // Look up the associated application
                let db = crate::desktop::mime::MimeDatabase::new();
                if let Some(assoc) = db.open_with(&mime) {
                    let mime_str = crate::desktop::mime::MimeDatabase::mime_to_str(&mime);
                    println!(
                        "[FILE-MANAGER] Opening '{}' ({}) with {} ({})",
                        entry.name, mime_str, assoc.app_name, assoc.app_exec
                    );

                    // Attempt to launch the associated application with the
                    // file path as argument. This uses the same load+exec
                    // infrastructure as the shell's external command execution.
                    //
                    // Check if the executable exists first (read guard is
                    // dropped after resolve_path returns).
                    let app_exists = crate::fs::get_vfs()
                        .read()
                        .resolve_path(&assoc.app_exec)
                        .is_ok();

                    if app_exists {
                        match crate::userspace::load_user_program(
                            &assoc.app_exec,
                            &[&assoc.app_exec, &file_path],
                            &[],
                        ) {
                            Ok(pid) => {
                                println!(
                                    "[FILE-MANAGER] Launched {} (PID {}) for '{}'",
                                    assoc.app_name, pid.0, entry.name
                                );
                            }
                            Err(e) => {
                                println!(
                                    "[FILE-MANAGER] Failed to launch {}: {:?}",
                                    assoc.app_exec, e
                                );
                            }
                        }
                    } else {
                        println!(
                            "[FILE-MANAGER] Application '{}' not found at '{}'",
                            assoc.app_name, assoc.app_exec
                        );
                    }
                } else {
                    println!(
                        "[FILE-MANAGER] No application associated with '{}'",
                        entry.name
                    );
                }
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

    /// Render file manager to a BGRA pixel buffer.
    ///
    /// `buf` is width*height*4 bytes in BGRA format.
    pub fn render(&self, buf: &mut [u8], width: usize, _height: usize) -> Result<(), KernelError> {
        use super::renderer::draw_char_into_buffer;

        // Clear to dark gray background (BGRA: 0x2A2A2A)
        for chunk in buf.chunks_exact_mut(4) {
            chunk[0] = 0x2A; // B
            chunk[1] = 0x2A; // G
            chunk[2] = 0x2A; // R
            chunk[3] = 0xFF; // A
        }

        // Draw header: current path
        let header = self.current_path.as_bytes();
        let prefix = b"Path: ";
        for (i, &ch) in prefix.iter().chain(header.iter()).enumerate() {
            draw_char_into_buffer(buf, width, ch, 8 + i * 8, 6, 0xDDDDDD);
        }

        // Draw separator line at y=24
        for x in 0..width {
            let offset = (24 * width + x) * 4;
            if offset + 3 < buf.len() {
                buf[offset] = 0x55;
                buf[offset + 1] = 0x55;
                buf[offset + 2] = 0x55;
                buf[offset + 3] = 0xFF;
            }
        }

        // Draw entries
        let line_height = 18;
        let start_y = 28;

        for (i, entry) in self.entries.iter().enumerate().skip(self.scroll_offset) {
            let row = i - self.scroll_offset;
            let y = start_y + row * line_height;

            // Highlight selected row
            if i == self.selected_index {
                for dy in 0..line_height {
                    for x in 0..width {
                        let offset = ((y + dy) * width + x) * 4;
                        if offset + 3 < buf.len() {
                            buf[offset] = 0x50;
                            buf[offset + 1] = 0x40;
                            buf[offset + 2] = 0x30;
                            buf[offset + 3] = 0xFF;
                        }
                    }
                }
            }

            // Draw prefix [DIR] or [FILE]
            let prefix: &[u8] = match entry.node_type {
                NodeType::Directory => b"[DIR]  ",
                NodeType::File => b"[FILE] ",
                _ => b"[?]    ",
            };

            let (text_color, prefix_color) = match entry.node_type {
                NodeType::Directory => (0x55AAFF_u32, 0x55AAFF_u32),
                _ => (0xCCCCCC, 0x888888),
            };

            // Draw prefix
            for (j, &ch) in prefix.iter().enumerate() {
                draw_char_into_buffer(buf, width, ch, 8 + j * 8, y + 1, prefix_color);
            }

            // Draw entry name
            let name_x = 8 + prefix.len() * 8;
            for (j, &ch) in entry.name.as_bytes().iter().enumerate() {
                draw_char_into_buffer(buf, width, ch, name_x + j * 8, y + 1, text_color);
            }
        }

        Ok(())
    }

    /// Get window ID
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    /// Get compositor surface ID
    pub fn surface_id(&self) -> u32 {
        self.surface_id
    }

    /// Render file manager contents to its compositor surface.
    pub fn render_to_surface(&self) {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut pixels = vec![0u8; w * h * 4];
        let _ = self.render(&mut pixels, w, h);
        super::renderer::update_surface_pixels(
            self.surface_id,
            self.pool_id,
            self.pool_buf_id,
            &pixels,
        );
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
    FILE_MANAGER
        .init(RwLock::new(fm))
        .map_err(|_| KernelError::InvalidState {
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

    #[test]
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
