//! VeridianOS Text Editor
//!
//! A simple yet powerful text editor for VeridianOS.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::{String, ToString};
use core::panic::PanicInfo;

/// Cursor position
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    pub line: usize,
    pub column: usize,
}

impl CursorPos {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    pub fn origin() -> Self {
        Self { line: 0, column: 0 }
    }
}

/// Text buffer
pub struct TextBuffer {
    lines: Vec<String>,
    cursor: CursorPos,
    selection_start: Option<CursorPos>,
    modified: bool,
    undo_stack: Vec<EditAction>,
    redo_stack: Vec<EditAction>,
}

impl TextBuffer {
    /// Create a new empty text buffer
    pub fn new() -> Self {
        Self {
            lines: alloc::vec![String::new()],
            cursor: CursorPos::origin(),
            selection_start: None,
            modified: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Load from string
    pub fn from_str(content: &str) -> Self {
        let lines: Vec<String> = content.lines()
            .map(|s| s.to_string())
            .collect();

        let lines = if lines.is_empty() {
            alloc::vec![String::new()]
        } else {
            lines
        };

        Self {
            lines,
            cursor: CursorPos::origin(),
            selection_start: None,
            modified: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Get line count
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Get line
    pub fn get_line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(|s| s.as_str())
    }

    /// Get cursor position
    pub fn cursor(&self) -> CursorPos {
        self.cursor
    }

    /// Move cursor up
    pub fn move_up(&mut self) {
        if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.clamp_cursor();
        }
    }

    /// Move cursor down
    pub fn move_down(&mut self) {
        if self.cursor.line < self.lines.len() - 1 {
            self.cursor.line += 1;
            self.clamp_cursor();
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor.column > 0 {
            self.cursor.column -= 1;
        } else if self.cursor.line > 0 {
            // Move to end of previous line
            self.cursor.line -= 1;
            self.cursor.column = self.lines[self.cursor.line].len();
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        let line_len = self.lines[self.cursor.line].len();
        if self.cursor.column < line_len {
            self.cursor.column += 1;
        } else if self.cursor.line < self.lines.len() - 1 {
            // Move to start of next line
            self.cursor.line += 1;
            self.cursor.column = 0;
        }
    }

    /// Move cursor to start of line
    pub fn move_home(&mut self) {
        self.cursor.column = 0;
    }

    /// Move cursor to end of line
    pub fn move_end(&mut self) {
        self.cursor.column = self.lines[self.cursor.line].len();
    }

    /// Clamp cursor to valid position
    fn clamp_cursor(&mut self) {
        if self.cursor.line >= self.lines.len() {
            self.cursor.line = self.lines.len() - 1;
        }
        let line_len = self.lines[self.cursor.line].len();
        if self.cursor.column > line_len {
            self.cursor.column = line_len;
        }
    }

    /// Insert character at cursor
    pub fn insert_char(&mut self, ch: char) {
        let line = &mut self.lines[self.cursor.line];
        line.insert(self.cursor.column, ch);
        self.cursor.column += 1;
        self.modified = true;
        self.redo_stack.clear();
    }

    /// Insert newline at cursor
    pub fn insert_newline(&mut self) {
        let line = &mut self.lines[self.cursor.line];
        let rest = line.split_off(self.cursor.column);

        self.cursor.line += 1;
        self.cursor.column = 0;
        self.lines.insert(self.cursor.line, rest);
        self.modified = true;
        self.redo_stack.clear();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_before(&mut self) {
        if self.cursor.column > 0 {
            let line = &mut self.lines[self.cursor.line];
            line.remove(self.cursor.column - 1);
            self.cursor.column -= 1;
            self.modified = true;
        } else if self.cursor.line > 0 {
            // Join with previous line
            let current_line = self.lines.remove(self.cursor.line);
            self.cursor.line -= 1;
            self.cursor.column = self.lines[self.cursor.line].len();
            self.lines[self.cursor.line].push_str(&current_line);
            self.modified = true;
        }
        self.redo_stack.clear();
    }

    /// Delete character at cursor (delete)
    pub fn delete_at(&mut self) {
        let line_len = self.lines[self.cursor.line].len();
        if self.cursor.column < line_len {
            let line = &mut self.lines[self.cursor.line];
            line.remove(self.cursor.column);
            self.modified = true;
        } else if self.cursor.line < self.lines.len() - 1 {
            // Join with next line
            let next_line = self.lines.remove(self.cursor.line + 1);
            self.lines[self.cursor.line].push_str(&next_line);
            self.modified = true;
        }
        self.redo_stack.clear();
    }

    /// Get entire text content
    pub fn to_string(&self) -> String {
        let mut result = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            result.push_str(line);
            if i < self.lines.len() - 1 {
                result.push('\n');
            }
        }
        result
    }

    /// Check if buffer is modified
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Mark as saved
    pub fn mark_saved(&mut self) {
        self.modified = false;
    }
}

/// Edit action for undo/redo
#[derive(Debug, Clone)]
enum EditAction {
    InsertChar { pos: CursorPos, ch: char },
    DeleteChar { pos: CursorPos, ch: char },
    InsertLine { line: usize, content: String },
    DeleteLine { line: usize, content: String },
}

/// Text editor
pub struct TextEditor {
    buffer: TextBuffer,
    filename: Option<String>,
    view_offset: usize,
    show_line_numbers: bool,
    tab_width: usize,

    // UI dimensions
    width: usize,
    height: usize,

    // Color scheme (Nord theme)
    bg_color: u32,
    fg_color: u32,
    line_num_color: u32,
    cursor_color: u32,
    status_bg_color: u32,
}

impl TextEditor {
    /// Create a new text editor
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            buffer: TextBuffer::new(),
            filename: None,
            view_offset: 0,
            show_line_numbers: true,
            tab_width: 4,
            width,
            height,
            bg_color: 0xFF2E3440,
            fg_color: 0xFFECEFF4,
            line_num_color: 0xFF4C566A,
            cursor_color: 0xFF88C0D0,
            status_bg_color: 0xFF3B4252,
        }
    }

    /// Load file
    pub fn load_file(&mut self, filename: String, content: &str) {
        self.buffer = TextBuffer::from_str(content);
        self.filename = Some(filename);
        self.view_offset = 0;
    }

    /// Save file
    pub fn save_file(&mut self) -> Result<(), &'static str> {
        if self.filename.is_none() {
            return Err("No filename specified");
        }

        // In a real implementation, would use VFS syscalls to write file
        self.buffer.mark_saved();
        Ok(())
    }

    /// Process keyboard input
    pub fn process_key(&mut self, key: char) {
        match key {
            '\x08' => self.buffer.delete_before(), // Backspace
            '\r' | '\n' => self.buffer.insert_newline(),
            '\t' => {
                // Insert spaces for tab
                for _ in 0..self.tab_width {
                    self.buffer.insert_char(' ');
                }
            }
            ch if ch >= ' ' && ch <= '~' => self.buffer.insert_char(ch),
            _ => {} // Ignore other control characters
        }

        self.ensure_cursor_visible();
    }

    /// Process special key
    pub fn process_special_key(&mut self, key: &str) {
        match key {
            "up" => self.buffer.move_up(),
            "down" => self.buffer.move_down(),
            "left" => self.buffer.move_left(),
            "right" => self.buffer.move_right(),
            "home" => self.buffer.move_home(),
            "end" => self.buffer.move_end(),
            "delete" => self.buffer.delete_at(),
            "pageup" => {
                // Move up one page
                for _ in 0..20 {
                    self.buffer.move_up();
                }
            }
            "pagedown" => {
                // Move down one page
                for _ in 0..20 {
                    self.buffer.move_down();
                }
            }
            _ => {}
        }

        self.ensure_cursor_visible();
    }

    /// Ensure cursor is visible in viewport
    fn ensure_cursor_visible(&mut self) {
        let cursor_line = self.buffer.cursor().line;
        let visible_lines = (self.height - 50) / 16; // Approximate

        if cursor_line < self.view_offset {
            self.view_offset = cursor_line;
        } else if cursor_line >= self.view_offset + visible_lines {
            self.view_offset = cursor_line - visible_lines + 1;
        }
    }

    /// Render editor to framebuffer
    pub fn render(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        // Clear background
        for pixel in fb.iter_mut() {
            *pixel = self.bg_color;
        }

        // Calculate dimensions
        let line_number_width = if self.show_line_numbers { 50 } else { 0 };
        let text_area_x = line_number_width + 10;
        let char_width = 8;
        let char_height = 16;
        let visible_lines = (fb_height - 30) / char_height;

        // Draw line numbers
        if self.show_line_numbers {
            self.draw_line_numbers(fb, fb_width, fb_height, visible_lines);
        }

        // Draw text content
        self.draw_text(fb, fb_width, fb_height, text_area_x, visible_lines);

        // Draw cursor
        self.draw_cursor(fb, fb_width, text_area_x, char_width, char_height);

        // Draw status bar
        self.draw_status_bar(fb, fb_width, fb_height);
    }

    /// Draw line numbers
    fn draw_line_numbers(&self, fb: &mut [u32], fb_width: usize, fb_height: usize, visible_lines: usize) {
        let char_height = 16;

        for i in 0..visible_lines {
            let line_num = self.view_offset + i;
            if line_num >= self.buffer.line_count() {
                break;
            }

            let y_offset = i * char_height;

            // Draw line number (simplified - just colored rectangles)
            for y in y_offset..(y_offset + char_height).min(fb_height - 30) {
                for x in 5..45 {
                    let offset = y * fb_width + x;
                    if offset < fb.len() {
                        fb[offset] = self.line_num_color;
                    }
                }
            }
        }
    }

    /// Draw text content
    fn draw_text(&self, fb: &mut [u32], fb_width: usize, fb_height: usize, text_x: usize, visible_lines: usize) {
        let char_width = 8;
        let char_height = 16;

        for i in 0..visible_lines {
            let line_num = self.view_offset + i;
            if line_num >= self.buffer.line_count() {
                break;
            }

            if let Some(line_content) = self.buffer.get_line(line_num) {
                let y_offset = i * char_height;

                // Draw each character (simplified)
                for (col, _ch) in line_content.chars().enumerate() {
                    let x_offset = text_x + col * char_width;

                    // Draw character as colored rectangle
                    for y in (y_offset + 2)..(y_offset + 14).min(fb_height - 30) {
                        for x in (x_offset + 1)..(x_offset + 7).min(fb_width) {
                            let offset = y * fb_width + x;
                            if offset < fb.len() {
                                fb[offset] = self.fg_color;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw cursor
    fn draw_cursor(&self, fb: &mut [u32], fb_width: usize, text_x: usize, char_width: usize, char_height: usize) {
        let cursor = self.buffer.cursor();

        if cursor.line >= self.view_offset {
            let visible_line = cursor.line - self.view_offset;
            let x_offset = text_x + cursor.column * char_width;
            let y_offset = visible_line * char_height;

            // Draw cursor as vertical line
            for y in y_offset..(y_offset + char_height) {
                for x in x_offset..(x_offset + 2) {
                    if x < fb_width && y < self.height - 30 {
                        let offset = y * fb_width + x;
                        if offset < fb.len() {
                            fb[offset] = self.cursor_color;
                        }
                    }
                }
            }
        }
    }

    /// Draw status bar
    fn draw_status_bar(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        let status_height = 25;
        let status_top = fb_height - status_height;

        // Draw status bar background
        for y in status_top..fb_height {
            for x in 0..fb_width {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = self.status_bg_color;
                }
            }
        }

        // Draw status text (simplified - filename and position)
        for y in (status_top + 5)..(status_top + 20) {
            for x in 10..300 {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = self.fg_color;
                }
            }
        }

        // Draw modified indicator if needed
        if self.buffer.is_modified() {
            for y in (status_top + 5)..(status_top + 20) {
                for x in (fb_width - 50)..(fb_width - 20) {
                    let offset = y * fb_width + x;
                    if offset < fb.len() {
                        fb[offset] = 0xFFBF616A; // Red for modified
                    }
                }
            }
        }
    }
}

/// Main entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Create text editor
    let mut editor = TextEditor::new(800, 600);

    // Load example content
    let example_content = "// VeridianOS Example File\n\
                          fn main() {\n\
                          \tprintln!(\"Hello, World!\");\n\
                          }\n";
    editor.load_file("example.rs".to_string(), example_content);

    // In a real implementation, this would:
    // 1. Connect to window manager to get a window
    // 2. Enter event loop:
    //    - Receive keyboard input
    //    - Process editing commands
    //    - Update text buffer
    //    - Render to framebuffer
    //    - Send update to window manager

    // Render initial view
    let mut framebuffer = alloc::vec![0u32; 800 * 600];
    editor.render(&mut framebuffer, 800, 600);

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
