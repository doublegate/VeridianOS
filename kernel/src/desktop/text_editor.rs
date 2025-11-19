//! GUI Text Editor Application
//!
//! Simple text editor with basic editing capabilities.

use alloc::{format, string::String, vec, vec::Vec};

use spin::RwLock;

use crate::{
    desktop::{
        font::{get_font_manager, FontSize, FontStyle},
        window_manager::{with_window_manager, InputEvent, WindowId},
    },
    error::KernelError,
    fs::{get_vfs, OpenFlags},
    sync::once_lock::GlobalState,
};

/// Text buffer line
type Line = Vec<char>;

/// Text editor state
pub struct TextEditor {
    /// Window ID
    window_id: WindowId,

    /// File path (None if new file)
    file_path: Option<String>,

    /// Text buffer (lines of characters)
    buffer: Vec<Line>,

    /// Cursor position (line, column)
    cursor_line: usize,
    cursor_col: usize,

    /// Scroll offset (top line visible)
    scroll_line: usize,

    /// Modified flag
    modified: bool,

    /// Window dimensions
    width: u32,
    height: u32,

    /// Visible rows
    visible_rows: usize,

    /// Visible columns
    visible_cols: usize,
}

impl TextEditor {
    /// Create a new text editor
    pub fn new(file_path: Option<String>) -> Result<Self, KernelError> {
        let width = 800;
        let height = 600;

        // Create window
        let window_id = with_window_manager(|wm| wm.create_window(150, 80, width, height, 0))
            .ok_or(KernelError::InvalidState {
                expected: "initialized",
                actual: "uninitialized",
            })??;

        // Calculate visible area
        let char_width = 8;
        let char_height = 12;
        let visible_cols = (width as usize) / char_width;
        let visible_rows = ((height as usize) - 40) / char_height; // -40 for status bar

        let mut editor = Self {
            window_id,
            file_path: file_path.clone(),
            buffer: vec![Vec::new()], // Start with one empty line
            cursor_line: 0,
            cursor_col: 0,
            scroll_line: 0,
            modified: false,
            width,
            height,
            visible_rows,
            visible_cols,
        };

        // Load file if specified
        if let Some(ref path) = file_path {
            editor.load_file(path)?;
        }

        println!("[TEXT-EDITOR] Created editor window {}", window_id);

        Ok(editor)
    }

    /// Load file from filesystem
    pub fn load_file(&mut self, path: &str) -> Result<(), KernelError> {
        println!("[TEXT-EDITOR] Loading file: {}", path);

        let vfs = get_vfs();

        // Open file
        match vfs.read().open(path, OpenFlags::read_only()) {
            Ok(node) => {
                // Read file
                let metadata = node.metadata().map_err(|_| KernelError::InvalidArgument {
                    name: "file_metadata",
                    value: "failed_to_read",
                })?;
                let mut file_buffer = vec![0u8; metadata.size];

                match node.read(0, &mut file_buffer) {
                    Ok(_bytes_read) => {
                        // Parse content into lines
                        self.buffer.clear();
                        let content = core::str::from_utf8(&file_buffer).map_err(|_| {
                            KernelError::InvalidArgument {
                                name: "file_content",
                                value: "invalid_utf8",
                            }
                        })?;

                        for line in content.lines() {
                            self.buffer.push(line.chars().collect());
                        }

                        if self.buffer.is_empty() {
                            self.buffer.push(Vec::new());
                        }

                        self.modified = false;
                        println!("[TEXT-EDITOR] Loaded {} lines", self.buffer.len());
                    }
                    Err(_e) => {
                        println!("[TEXT-EDITOR] Failed to read file");
                        return Err(KernelError::InvalidArgument {
                            name: "file_read",
                            value: "failed",
                        });
                    }
                }
            }
            Err(_e) => {
                println!("[TEXT-EDITOR] Failed to open file");
                return Err(KernelError::InvalidArgument {
                    name: "file_open",
                    value: "failed",
                });
            }
        }

        Ok(())
    }

    /// Save file to filesystem
    pub fn save_file(&mut self) -> Result<(), KernelError> {
        let path = self
            .file_path
            .as_ref()
            .ok_or(KernelError::InvalidArgument {
                name: "file_path",
                value: "no_path_specified",
            })?;

        println!("[TEXT-EDITOR] Saving file: {}", path);

        // Convert buffer to bytes
        let mut content = String::new();
        for line in &self.buffer {
            for &ch in line {
                content.push(ch);
            }
            content.push('\n');
        }

        let bytes = content.as_bytes();

        // Write to filesystem
        let vfs = get_vfs();

        // First check if file exists, otherwise create it
        match vfs.read().open(path, OpenFlags::read_only()) {
            Ok(node) => {
                // File exists, write to it
                node.write(0, bytes)
                    .map_err(|_| KernelError::InvalidArgument {
                        name: "file_write",
                        value: "failed",
                    })?;
                self.modified = false;
                println!("[TEXT-EDITOR] File saved ({} bytes)", bytes.len());
                Ok(())
            }
            Err(_) => {
                // File doesn't exist, need to create it
                // For now, return an error since we need parent directory
                println!("[TEXT-EDITOR] Failed to save file: file does not exist");
                Err(KernelError::InvalidArgument {
                    name: "file_save",
                    value: "file_not_found",
                })
            }
        }
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
                        // Insert newline
                        self.insert_newline();
                    }
                    '\x08' => {
                        // Backspace
                        self.delete_char();
                    }
                    '\t' => {
                        // Tab - insert 4 spaces
                        for _ in 0..4 {
                            self.insert_char(' ');
                        }
                    }
                    ch if ch >= ' ' && ch <= '~' => {
                        // Printable character
                        self.insert_char(ch);
                    }
                    _ => {
                        // Handle special keys via scancode
                        match scancode {
                            72 => self.move_cursor_up(),    // Up arrow
                            80 => self.move_cursor_down(),  // Down arrow
                            75 => self.move_cursor_left(),  // Left arrow
                            77 => self.move_cursor_right(), // Right arrow
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Insert character at cursor
    fn insert_char(&mut self, ch: char) {
        if self.cursor_line < self.buffer.len() {
            self.buffer[self.cursor_line].insert(self.cursor_col, ch);
            self.cursor_col += 1;
            self.modified = true;
        }
    }

    /// Delete character before cursor
    fn delete_char(&mut self) {
        if self.cursor_col > 0 {
            self.buffer[self.cursor_line].remove(self.cursor_col - 1);
            self.cursor_col -= 1;
            self.modified = true;
        } else if self.cursor_line > 0 {
            // Join with previous line
            let current_line = self.buffer.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.buffer[self.cursor_line].len();
            self.buffer[self.cursor_line].extend(current_line);
            self.modified = true;
        }
    }

    /// Insert newline at cursor
    fn insert_newline(&mut self) {
        if self.cursor_line < self.buffer.len() {
            let rest = self.buffer[self.cursor_line].split_off(self.cursor_col);
            self.cursor_line += 1;
            self.buffer.insert(self.cursor_line, rest);
            self.cursor_col = 0;
            self.modified = true;
        }
    }

    /// Move cursor up
    fn move_cursor_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.buffer[self.cursor_line].len());
        }
    }

    /// Move cursor down
    fn move_cursor_down(&mut self) {
        if self.cursor_line < self.buffer.len() - 1 {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.buffer[self.cursor_line].len());
        }
    }

    /// Move cursor left
    fn move_cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.buffer[self.cursor_line].len();
        }
    }

    /// Move cursor right
    fn move_cursor_right(&mut self) {
        if self.cursor_col < self.buffer[self.cursor_line].len() {
            self.cursor_col += 1;
        } else if self.cursor_line < self.buffer.len() - 1 {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    /// Render text editor
    pub fn render(
        &self,
        framebuffer: &mut [u8],
        fb_width: usize,
        fb_height: usize,
    ) -> Result<(), KernelError> {
        // Clear background
        for pixel in framebuffer.iter_mut() {
            *pixel = 16; // Very dark gray
        }

        // Get font
        let font_manager = get_font_manager()?;
        let font = font_manager
            .get_font(FontSize::Medium, FontStyle::Regular)
            .ok_or(KernelError::NotFound {
                resource: "font",
                id: 0,
            })?;

        // Render status bar
        let status = if let Some(ref path) = self.file_path {
            if self.modified {
                format!(
                    "{}* - Line {}, Col {}",
                    path,
                    self.cursor_line + 1,
                    self.cursor_col + 1
                )
            } else {
                format!(
                    "{} - Line {}, Col {}",
                    path,
                    self.cursor_line + 1,
                    self.cursor_col + 1
                )
            }
        } else {
            format!(
                "[New File] - Line {}, Col {}",
                self.cursor_line + 1,
                self.cursor_col + 1
            )
        };

        let _ = font.render_text(&status, framebuffer, fb_width, fb_height, 5, 5);

        // Render text
        let char_height = 12;
        let start_y = 30;

        for (i, line) in self
            .buffer
            .iter()
            .enumerate()
            .skip(self.scroll_line)
            .take(self.visible_rows)
        {
            let y = start_y + ((i - self.scroll_line) * char_height) as i32;

            // Convert line to string
            let line_str: String = line.iter().collect();

            // Render line
            let _ = font.render_text(&line_str, framebuffer, fb_width, fb_height, 5, y);

            // Render cursor if on this line
            if i == self.cursor_line {
                let cursor_x = 5 + (self.cursor_col * 8) as i32;
                // Draw cursor (vertical bar)
                for dy in 0..char_height {
                    for dx in 0..2 {
                        let px = cursor_x + dx as i32;
                        let py = y + dy as i32;

                        if px >= 0
                            && py >= 0
                            && (px as usize) < fb_width
                            && (py as usize) < fb_height
                        {
                            let index = py as usize * fb_width + px as usize;
                            if index < framebuffer.len() {
                                framebuffer[index] = 255; // White cursor
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Get window ID
    pub fn window_id(&self) -> WindowId {
        self.window_id
    }
}

/// Global text editor (can support multiple instances)
static TEXT_EDITOR: GlobalState<RwLock<TextEditor>> = GlobalState::new();

/// Initialize text editor
pub fn init() -> Result<(), KernelError> {
    println!("[TEXT-EDITOR] Text editor initialized");
    Ok(())
}

/// Create a new text editor instance
pub fn create_text_editor(file_path: Option<String>) -> Result<(), KernelError> {
    let editor = TextEditor::new(file_path)?;
    TEXT_EDITOR
        .init(RwLock::new(editor))
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;
    Ok(())
}

/// Execute a function with the text editor
pub fn with_text_editor<R, F: FnOnce(&RwLock<TextEditor>) -> R>(f: F) -> Option<R> {
    TEXT_EDITOR.with(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_char_insertion() {
        // Would test character insertion here
    }

    #[test_case]
    fn test_newline_insertion() {
        // Would test newline handling
    }
}
