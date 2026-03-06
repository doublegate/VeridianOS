//! Text Editor with Gap Buffer
//!
//! Core text editing engine with gap buffer data structure, multiple buffer
//! support, undo/redo history, and syntax highlighting integration.

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

/// Gap buffer for efficient text editing
pub struct GapBuffer {
    buf: Vec<u8>,
    gap_start: usize,
    gap_end: usize,
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl GapBuffer {
    const INITIAL_GAP: usize = 64;

    pub fn new() -> Self {
        Self {
            buf: vec![0; Self::INITIAL_GAP],
            gap_start: 0,
            gap_end: Self::INITIAL_GAP,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let bytes = text.as_bytes();
        let gap_size = Self::INITIAL_GAP;
        let mut buf = Vec::with_capacity(bytes.len() + gap_size);
        buf.extend_from_slice(bytes);
        buf.resize(bytes.len() + gap_size, 0);

        Self {
            buf,
            gap_start: bytes.len(),
            gap_end: bytes.len() + gap_size,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len() - self.gap_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    fn move_gap_to(&mut self, pos: usize) {
        if pos == self.gap_start {
            return;
        }
        if pos < self.gap_start {
            let count = self.gap_start - pos;
            let src = pos;
            let dst = self.gap_end - count;
            self.buf.copy_within(src..src + count, dst);
            self.gap_start = pos;
            self.gap_end -= count;
        } else {
            let count = pos - self.gap_start;
            let src = self.gap_end;
            let dst = self.gap_start;
            self.buf.copy_within(src..src + count, dst);
            self.gap_start += count;
            self.gap_end += count;
        }
    }

    fn ensure_gap(&mut self, needed: usize) {
        if self.gap_len() >= needed {
            return;
        }
        let extra = core::cmp::max(needed * 2, 64);
        let old_gap_end = self.gap_end;
        let after_gap = self.buf.len() - old_gap_end;

        self.buf.resize(self.buf.len() + extra, 0);
        // Move content after gap
        if after_gap > 0 {
            self.buf
                .copy_within(old_gap_end..old_gap_end + after_gap, old_gap_end + extra);
        }
        self.gap_end += extra;
    }

    /// Insert a character at position
    pub fn insert(&mut self, pos: usize, ch: u8) {
        self.move_gap_to(pos);
        self.ensure_gap(1);
        self.buf[self.gap_start] = ch;
        self.gap_start += 1;
    }

    /// Insert a string at position
    pub fn insert_str(&mut self, pos: usize, text: &str) {
        self.move_gap_to(pos);
        self.ensure_gap(text.len());
        self.buf[self.gap_start..self.gap_start + text.len()].copy_from_slice(text.as_bytes());
        self.gap_start += text.len();
    }

    /// Delete a character at position
    pub fn delete(&mut self, pos: usize) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        self.move_gap_to(pos);
        let ch = self.buf[self.gap_end];
        self.gap_end += 1;
        Some(ch)
    }

    /// Delete a range of characters
    pub fn delete_range(&mut self, start: usize, end: usize) -> usize {
        if start >= end || start >= self.len() {
            return 0;
        }
        let end = core::cmp::min(end, self.len());
        self.move_gap_to(start);
        let count = end - start;
        self.gap_end += count;
        count
    }

    /// Get character at position
    pub fn char_at(&self, pos: usize) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        let actual_pos = if pos < self.gap_start {
            pos
        } else {
            pos + self.gap_len()
        };
        Some(self.buf[actual_pos])
    }

    /// Convert to String
    pub fn to_text(&self) -> String {
        let mut result = Vec::with_capacity(self.len());
        for i in 0..self.gap_start {
            result.push(self.buf[i]);
        }
        for i in self.gap_end..self.buf.len() {
            result.push(self.buf[i]);
        }
        String::from_utf8(result).unwrap_or_default()
    }

    /// Get a line by number (0-based)
    pub fn line(&self, line_num: usize) -> Option<String> {
        let text = self.to_text();
        text.lines().nth(line_num).map(|s| s.to_string())
    }

    /// Count lines
    pub fn line_count(&self) -> usize {
        let text = self.to_text();
        text.lines().count().max(1)
    }
}

/// Undo/redo operation
#[derive(Debug, Clone)]
enum EditOp {
    Insert { pos: usize, text: String },
    Delete { pos: usize, text: String },
}

/// Editor buffer (file being edited)
pub struct EditorBuffer {
    pub name: String,
    pub gap_buf: GapBuffer,
    pub cursor_pos: usize,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub modified: bool,
    undo_stack: Vec<EditOp>,
    redo_stack: Vec<EditOp>,
    pub scroll_top: usize,
}

impl EditorBuffer {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            gap_buf: GapBuffer::new(),
            cursor_pos: 0,
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_top: 0,
        }
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            gap_buf: GapBuffer::from_text(text),
            cursor_pos: 0,
            cursor_line: 0,
            cursor_col: 0,
            modified: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            scroll_top: 0,
        }
    }

    pub fn insert_char(&mut self, ch: u8) {
        self.gap_buf.insert(self.cursor_pos, ch);
        let text = String::from(ch as char);
        self.undo_stack.push(EditOp::Insert {
            pos: self.cursor_pos,
            text,
        });
        self.redo_stack.clear();
        self.cursor_pos += 1;
        self.modified = true;
        self.update_cursor_pos();
    }

    pub fn insert_text(&mut self, text: &str) {
        self.gap_buf.insert_str(self.cursor_pos, text);
        self.undo_stack.push(EditOp::Insert {
            pos: self.cursor_pos,
            text: text.to_string(),
        });
        self.redo_stack.clear();
        self.cursor_pos += text.len();
        self.modified = true;
        self.update_cursor_pos();
    }

    pub fn delete_char(&mut self) -> Option<u8> {
        if self.cursor_pos >= self.gap_buf.len() {
            return None;
        }
        let ch = self.gap_buf.delete(self.cursor_pos)?;
        self.undo_stack.push(EditOp::Delete {
            pos: self.cursor_pos,
            text: String::from(ch as char),
        });
        self.redo_stack.clear();
        self.modified = true;
        Some(ch)
    }

    pub fn backspace(&mut self) -> Option<u8> {
        if self.cursor_pos == 0 {
            return None;
        }
        self.cursor_pos -= 1;
        self.delete_char()
    }

    pub fn undo(&mut self) -> bool {
        let op = match self.undo_stack.pop() {
            Some(op) => op,
            None => return false,
        };

        match &op {
            EditOp::Insert { pos, text } => {
                self.gap_buf.delete_range(*pos, pos + text.len());
                self.cursor_pos = *pos;
                self.redo_stack.push(op);
            }
            EditOp::Delete { pos, text } => {
                self.gap_buf.insert_str(*pos, text);
                self.cursor_pos = pos + text.len();
                self.redo_stack.push(op);
            }
        }

        self.update_cursor_pos();
        true
    }

    pub fn redo(&mut self) -> bool {
        let op = match self.redo_stack.pop() {
            Some(op) => op,
            None => return false,
        };

        match &op {
            EditOp::Insert { pos, text } => {
                self.gap_buf.insert_str(*pos, text);
                self.cursor_pos = pos + text.len();
                self.undo_stack.push(op);
            }
            EditOp::Delete { pos, text: _ } => {
                self.gap_buf.delete(self.cursor_pos);
                self.cursor_pos = *pos;
                self.undo_stack.push(op);
            }
        }

        self.update_cursor_pos();
        true
    }

    fn update_cursor_pos(&mut self) {
        let text = self.gap_buf.to_text();
        let before = &text[..self.cursor_pos.min(text.len())];
        self.cursor_line = before.matches('\n').count();
        self.cursor_col = before
            .rfind('\n')
            .map_or(before.len(), |p| before.len() - p - 1);
    }

    pub fn content(&self) -> String {
        self.gap_buf.to_text()
    }

    pub fn line_count(&self) -> usize {
        self.gap_buf.line_count()
    }
}

/// Multi-buffer editor
pub struct Editor {
    buffers: Vec<EditorBuffer>,
    active: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffers: Vec::new(),
            active: 0,
        }
    }

    pub fn open(&mut self, name: &str, content: &str) -> usize {
        let idx = self.buffers.len();
        self.buffers.push(EditorBuffer::from_text(name, content));
        self.active = idx;
        idx
    }

    pub fn close(&mut self, idx: usize) -> bool {
        if idx >= self.buffers.len() {
            return false;
        }
        self.buffers.remove(idx);
        if self.active >= self.buffers.len() && !self.buffers.is_empty() {
            self.active = self.buffers.len() - 1;
        }
        true
    }

    pub fn active_buffer(&self) -> Option<&EditorBuffer> {
        self.buffers.get(self.active)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut EditorBuffer> {
        self.buffers.get_mut(self.active)
    }

    pub fn switch_to(&mut self, idx: usize) -> bool {
        if idx < self.buffers.len() {
            self.active = idx;
            true
        } else {
            false
        }
    }

    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gap_buffer_new() {
        let buf = GapBuffer::new();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_gap_buffer_insert() {
        let mut buf = GapBuffer::new();
        buf.insert(0, b'h');
        buf.insert(1, b'i');
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.to_text(), "hi");
    }

    #[test]
    fn test_gap_buffer_insert_str() {
        let mut buf = GapBuffer::new();
        buf.insert_str(0, "hello");
        assert_eq!(buf.to_text(), "hello");
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn test_gap_buffer_delete() {
        let mut buf = GapBuffer::from_text("hello");
        let ch = buf.delete(0);
        assert_eq!(ch, Some(b'h'));
        assert_eq!(buf.to_text(), "ello");
    }

    #[test]
    fn test_gap_buffer_delete_range() {
        let mut buf = GapBuffer::from_text("hello world");
        let count = buf.delete_range(5, 11);
        assert_eq!(count, 6);
        assert_eq!(buf.to_text(), "hello");
    }

    #[test]
    fn test_gap_buffer_char_at() {
        let buf = GapBuffer::from_text("abc");
        assert_eq!(buf.char_at(0), Some(b'a'));
        assert_eq!(buf.char_at(2), Some(b'c'));
        assert_eq!(buf.char_at(3), None);
    }

    #[test]
    fn test_gap_buffer_line() {
        let buf = GapBuffer::from_text("line1\nline2\nline3");
        assert_eq!(buf.line(0), Some("line1".to_string()));
        assert_eq!(buf.line(1), Some("line2".to_string()));
        assert_eq!(buf.line(2), Some("line3".to_string()));
    }

    #[test]
    fn test_gap_buffer_line_count() {
        let buf = GapBuffer::from_text("a\nb\nc");
        assert_eq!(buf.line_count(), 3);
    }

    #[test]
    fn test_editor_buffer_insert() {
        let mut buf = EditorBuffer::new("test.txt");
        buf.insert_text("hello");
        assert_eq!(buf.content(), "hello");
        assert!(buf.modified);
    }

    #[test]
    fn test_editor_buffer_undo_redo() {
        let mut buf = EditorBuffer::new("test.txt");
        buf.insert_text("hello");
        assert_eq!(buf.content(), "hello");

        assert!(buf.undo());
        assert_eq!(buf.content(), "");

        assert!(buf.redo());
        assert_eq!(buf.content(), "hello");
    }

    #[test]
    fn test_editor_multi_buffer() {
        let mut editor = Editor::new();
        editor.open("a.txt", "file A");
        editor.open("b.txt", "file B");

        assert_eq!(editor.buffer_count(), 2);
        assert_eq!(editor.active_buffer().unwrap().name, "b.txt");

        editor.switch_to(0);
        assert_eq!(editor.active_buffer().unwrap().name, "a.txt");
    }

    #[test]
    fn test_editor_close() {
        let mut editor = Editor::new();
        editor.open("a.txt", "");
        editor.open("b.txt", "");
        assert!(editor.close(0));
        assert_eq!(editor.buffer_count(), 1);
    }
}
