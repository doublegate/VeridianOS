// Methods are progressively wired across sprints (tab completion, etc.)
#![allow(dead_code)]

//! Line editor with cursor movement, history navigation, and kill ring.
//!
//! Replaces the basic `read_line()` method with a full-featured line editor
//! supporting:
//! - Cursor movement (left/right, Home/End, Ctrl-A/Ctrl-E)
//! - Character insertion and deletion at cursor position
//! - Kill operations (Ctrl-K, Ctrl-U, Ctrl-W)
//! - Command history navigation (Up/Down arrows)
//! - ANSI escape code output for terminal redrawing

use alloc::{string::String, vec::Vec};

use super::ansi::{AnsiEvent, AnsiParser};

/// Result of processing a single input event in the line editor.
#[derive(Debug, PartialEq, Eq)]
pub enum EditResult {
    /// Continue reading input.
    Continue,
    /// The user pressed Enter — the line is ready.
    Done,
    /// The user pressed Ctrl-C — cancel the current line.
    Cancel,
    /// The user pressed Ctrl-D on an empty line — exit.
    Eof,
    /// The user pressed Ctrl-L — clear screen and redraw.
    ClearScreen,
    /// The user pressed Tab — request tab completion.
    TabComplete,
    /// The user pressed Ctrl-Z — suspend the foreground job.
    Suspend,
}

/// A line editor with cursor tracking, inline editing, and history support.
pub struct LineEditor {
    /// The current line buffer.
    buffer: Vec<u8>,
    /// Cursor position within the buffer (0 = before first char).
    cursor: usize,
    /// ANSI escape sequence parser.
    parser: AnsiParser,
    /// Saved line when navigating history (so the user can return to it).
    saved_line: Option<Vec<u8>>,
    /// Current position in the history list (-1 = current line, 0 = most
    /// recent).
    history_index: Option<usize>,
}

impl LineEditor {
    /// Create a new line editor.
    pub const fn new() -> Self {
        Self {
            buffer: Vec::new(),
            cursor: 0,
            parser: AnsiParser::new(),
            saved_line: None,
            history_index: None,
        }
    }

    /// Reset the editor for a new line.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.saved_line = None;
        self.history_index = None;
    }

    /// Get the current line content as a string.
    pub fn line(&self) -> String {
        String::from_utf8_lossy(&self.buffer).into_owned()
    }

    /// Get current buffer length.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Get cursor position.
    pub fn cursor_pos(&self) -> usize {
        self.cursor
    }

    /// Feed a raw byte from serial input and process it.
    ///
    /// Returns `None` if the byte is part of an incomplete escape sequence.
    /// Returns `Some(EditResult)` when an action should be taken.
    pub fn feed(&mut self, byte: u8, history: &[String]) -> Option<EditResult> {
        let event = self.parser.feed(byte)?;
        Some(self.handle_event(event, history))
    }

    /// Handle a parsed ANSI event and return the appropriate action.
    fn handle_event(&mut self, event: AnsiEvent, history: &[String]) -> EditResult {
        match event {
            AnsiEvent::Char(ch) => {
                self.insert_char(ch);
                EditResult::Continue
            }
            AnsiEvent::Enter => EditResult::Done,
            AnsiEvent::Backspace => {
                self.backspace();
                EditResult::Continue
            }
            AnsiEvent::Delete => {
                self.delete_at_cursor();
                EditResult::Continue
            }
            AnsiEvent::ArrowLeft => {
                self.move_left();
                EditResult::Continue
            }
            AnsiEvent::ArrowRight => {
                self.move_right();
                EditResult::Continue
            }
            AnsiEvent::ArrowUp => {
                self.history_prev(history);
                EditResult::Continue
            }
            AnsiEvent::ArrowDown => {
                self.history_next(history);
                EditResult::Continue
            }
            AnsiEvent::Home | AnsiEvent::CtrlA => {
                self.move_to_start();
                EditResult::Continue
            }
            AnsiEvent::End | AnsiEvent::CtrlE => {
                self.move_to_end();
                EditResult::Continue
            }
            AnsiEvent::CtrlK => {
                self.kill_to_end();
                EditResult::Continue
            }
            AnsiEvent::CtrlU => {
                self.kill_to_start();
                EditResult::Continue
            }
            AnsiEvent::CtrlW => {
                self.kill_word();
                EditResult::Continue
            }
            AnsiEvent::CtrlC => EditResult::Cancel,
            AnsiEvent::CtrlD => {
                if self.buffer.is_empty() {
                    EditResult::Eof
                } else {
                    // Ctrl-D with content: delete char at cursor (like Delete)
                    self.delete_at_cursor();
                    EditResult::Continue
                }
            }
            AnsiEvent::CtrlL => EditResult::ClearScreen,
            AnsiEvent::Tab => EditResult::TabComplete,
            AnsiEvent::CtrlZ => EditResult::Suspend,
            _ => EditResult::Continue,
        }
    }

    /// Insert a character at the cursor position.
    fn insert_char(&mut self, ch: u8) {
        if self.cursor >= self.buffer.len() {
            // Append at end — simple case
            self.buffer.push(ch);
            self.cursor += 1;
            // Print just the character
            crate::print!("{}", ch as char);
        } else {
            // Insert in the middle
            self.buffer.insert(self.cursor, ch);
            self.cursor += 1;
            // Redraw from cursor to end, then reposition
            self.redraw_from_cursor();
        }
    }

    /// Delete the character before the cursor (backspace).
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buffer.remove(self.cursor);
            if self.cursor == self.buffer.len() {
                // At end of line — simple erase
                crate::print!("\x08 \x08");
            } else {
                // In the middle — need to redraw
                // Move cursor back one position
                crate::print!("\x08");
                self.redraw_from_cursor();
            }
        }
    }

    /// Delete the character at the cursor position.
    fn delete_at_cursor(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.remove(self.cursor);
            self.redraw_from_cursor();
        }
    }

    /// Move cursor left.
    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            crate::print!("\x1b[D");
        }
    }

    /// Move cursor right.
    fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor += 1;
            crate::print!("\x1b[C");
        }
    }

    /// Move cursor to start of line.
    fn move_to_start(&mut self) {
        if self.cursor > 0 {
            // Move left by `cursor` positions
            crate::print!("\x1b[{}D", self.cursor);
            self.cursor = 0;
        }
    }

    /// Move cursor to end of line.
    fn move_to_end(&mut self) {
        if self.cursor < self.buffer.len() {
            let distance = self.buffer.len() - self.cursor;
            crate::print!("\x1b[{}C", distance);
            self.cursor = self.buffer.len();
        }
    }

    /// Kill (delete) from cursor to end of line.
    fn kill_to_end(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.truncate(self.cursor);
            // Clear from cursor to end of line
            crate::print!("\x1b[K");
        }
    }

    /// Kill (delete) from start of line to cursor.
    fn kill_to_start(&mut self) {
        if self.cursor > 0 {
            let removed = self.cursor;
            self.buffer.drain(..self.cursor);
            self.cursor = 0;
            // Move cursor to start, redraw entire line, clear remainder
            crate::print!("\x1b[{}D", removed);
            self.redraw_full_line();
        }
    }

    /// Kill the previous word (back to previous whitespace).
    fn kill_word(&mut self) {
        if self.cursor == 0 {
            return;
        }

        // Skip trailing whitespace
        let mut pos = self.cursor;
        while pos > 0 && self.buffer[pos - 1] == b' ' {
            pos -= 1;
        }
        // Skip word characters
        while pos > 0 && self.buffer[pos - 1] != b' ' {
            pos -= 1;
        }

        let removed = self.cursor - pos;
        if removed > 0 {
            self.buffer.drain(pos..self.cursor);
            self.cursor = pos;
            // Move back and redraw
            crate::print!("\x1b[{}D", removed);
            self.redraw_from_cursor();
        }
    }

    /// Navigate to the previous history entry.
    fn history_prev(&mut self, history: &[String]) {
        if history.is_empty() {
            return;
        }

        let new_index = match self.history_index {
            None => {
                // Save current line before entering history
                self.saved_line = Some(self.buffer.clone());
                history.len() - 1
            }
            Some(idx) => {
                if idx == 0 {
                    return; // Already at oldest entry
                }
                idx - 1
            }
        };

        self.history_index = Some(new_index);
        self.load_history_entry(&history[new_index]);
    }

    /// Navigate to the next history entry (towards present).
    fn history_next(&mut self, history: &[String]) {
        match self.history_index {
            None => {} // Already at current line
            Some(idx) => {
                if idx + 1 >= history.len() {
                    // Return to saved current line
                    self.history_index = None;
                    if let Some(saved) = self.saved_line.take() {
                        self.load_buffer(saved);
                    }
                } else {
                    self.history_index = Some(idx + 1);
                    self.load_history_entry(&history[idx + 1]);
                }
            }
        }
    }

    /// Load a history entry into the buffer, redrawing the line.
    fn load_history_entry(&mut self, entry: &str) {
        self.load_buffer(entry.as_bytes().to_vec());
    }

    /// Replace the entire buffer with new content and redraw.
    fn load_buffer(&mut self, new_buffer: Vec<u8>) {
        // Move cursor to start
        if self.cursor > 0 {
            crate::print!("\x1b[{}D", self.cursor);
        }
        // Clear current line
        crate::print!("\x1b[K");
        // Set new buffer
        self.buffer = new_buffer;
        self.cursor = self.buffer.len();
        // Print new content
        if let Ok(s) = core::str::from_utf8(&self.buffer) {
            crate::print!("{}", s);
        }
    }

    /// Insert a string at the cursor position (used for tab completion).
    pub fn insert_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.buffer.insert(self.cursor, b);
            self.cursor += 1;
        }
        self.redraw_from_cursor();
    }

    /// Replace the current word without terminal output (for manual redraw).
    pub fn replace_word_silent(&mut self, start: usize, replacement: &str) {
        if start < self.cursor {
            self.buffer.drain(start..self.cursor);
            self.cursor = start;
        }
        for &b in replacement.as_bytes() {
            self.buffer.insert(self.cursor, b);
            self.cursor += 1;
        }
    }

    /// Replace the current word at cursor with a completion string.
    pub fn replace_word(&mut self, start: usize, replacement: &str) {
        // Remove old word
        if start < self.cursor {
            self.buffer.drain(start..self.cursor);
            self.cursor = start;
        }
        // Insert replacement
        for &b in replacement.as_bytes() {
            self.buffer.insert(self.cursor, b);
            self.cursor += 1;
        }
        // Move to start and redraw entire line
        if self.cursor > replacement.len() {
            crate::print!("\x1b[{}D", self.cursor);
        }
        self.redraw_full_line();
    }

    /// Redraw the line from the current cursor position to the end.
    ///
    /// Prints characters from cursor to end of buffer, then clears any
    /// remaining characters from the previous content, then moves cursor back.
    fn redraw_from_cursor(&mut self) {
        // Print from cursor to end
        let tail = &self.buffer[self.cursor..];
        if let Ok(s) = core::str::from_utf8(tail) {
            crate::print!("{}", s);
        }
        // Clear any leftover characters
        crate::print!("\x1b[K");
        // Move cursor back to its position
        let distance = self.buffer.len() - self.cursor;
        if distance > 0 {
            crate::print!("\x1b[{}D", distance);
        }
    }

    /// Redraw the entire line from position 0.
    fn redraw_full_line(&mut self) {
        if let Ok(s) = core::str::from_utf8(&self.buffer) {
            crate::print!("{}", s);
        }
        crate::print!("\x1b[K");
        // Reposition cursor
        let distance = self.buffer.len() - self.cursor;
        if distance > 0 {
            crate::print!("\x1b[{}D", distance);
        }
    }
}
