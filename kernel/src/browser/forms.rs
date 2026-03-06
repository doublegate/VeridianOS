//! HTML Form Elements and Scroll State
//!
//! Handles interactive form inputs (text fields, checkboxes, buttons),
//! link navigation, and scroll state management. All dimensions use
//! pixel coordinates (i32).

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::events::NodeId;

// ---------------------------------------------------------------------------
// Input types and form elements
// ---------------------------------------------------------------------------

/// HTML input element types
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputType {
    #[default]
    Text,
    Password,
    Submit,
    Hidden,
    Checkbox,
    Radio,
    Button,
    Reset,
}

/// HTTP method for form submission
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormMethod {
    #[default]
    Get,
    Post,
}

/// A <form> element's state
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct FormElement {
    /// Form node ID in the DOM
    pub node_id: NodeId,
    /// Action URL
    pub action: String,
    /// Submission method
    pub method: FormMethod,
    /// Input element node IDs belonging to this form
    pub inputs: Vec<NodeId>,
}

impl FormElement {
    pub fn new(node_id: NodeId, action: &str, method: FormMethod) -> Self {
        Self {
            node_id,
            action: action.to_string(),
            method,
            inputs: Vec::new(),
        }
    }

    /// Add an input element to this form
    pub fn add_input(&mut self, input_node: NodeId) {
        self.inputs.push(input_node);
    }

    /// Build URL-encoded form data from input elements
    pub fn encode_form_data(&self, inputs: &[InputElement]) -> String {
        let mut result = String::new();
        for input in inputs {
            if input.input_type == InputType::Submit
                || input.input_type == InputType::Button
                || input.input_type == InputType::Reset
            {
                continue;
            }
            if input.input_type == InputType::Checkbox && !input.checked {
                continue;
            }
            if !result.is_empty() {
                result.push('&');
            }
            result.push_str(&url_encode(&input.name));
            result.push('=');
            result.push_str(&url_encode(&input.value));
        }
        result
    }
}

/// An <input> element's state
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct InputElement {
    /// Node ID in the DOM
    pub node_id: NodeId,
    /// Input type
    pub input_type: InputType,
    /// Name attribute
    pub name: String,
    /// Current value
    pub value: String,
    /// Whether the input is checked (checkbox/radio)
    pub checked: bool,
    /// Cursor position within value (for text inputs)
    pub cursor_pos: usize,
    /// Whether the input has focus
    pub focused: bool,
    /// Placeholder text
    pub placeholder: String,
    /// Whether the input is disabled
    pub disabled: bool,
    /// Maximum length (0 = unlimited)
    pub max_length: usize,
}

impl InputElement {
    pub fn new(node_id: NodeId, input_type: InputType, name: &str) -> Self {
        Self {
            node_id,
            input_type,
            name: name.to_string(),
            value: String::new(),
            checked: false,
            cursor_pos: 0,
            focused: false,
            placeholder: String::new(),
            disabled: false,
            max_length: 0,
        }
    }

    /// Set the value and clamp cursor
    pub fn set_value(&mut self, value: &str) {
        self.value = value.to_string();
        if self.cursor_pos > self.value.len() {
            self.cursor_pos = self.value.len();
        }
    }

    /// Toggle checked state (for checkbox/radio)
    pub fn toggle_checked(&mut self) {
        if self.input_type == InputType::Checkbox || self.input_type == InputType::Radio {
            self.checked = !self.checked;
        }
    }

    /// Get display text (masked for password fields)
    pub fn display_text(&self) -> String {
        if self.value.is_empty() && !self.placeholder.is_empty() {
            return self.placeholder.clone();
        }
        match self.input_type {
            InputType::Password => {
                let mut masked = String::with_capacity(self.value.len());
                for _ in 0..self.value.len() {
                    masked.push('*');
                }
                masked
            }
            _ => self.value.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Text input handling
// ---------------------------------------------------------------------------

/// Text input buffer with cursor and selection
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    /// Text buffer
    pub buffer: String,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Selection start (byte offset, None if no selection)
    pub selection_start: Option<usize>,
    /// Selection end (byte offset)
    pub selection_end: Option<usize>,
}

impl TextInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            buffer: text.to_string(),
            cursor: text.len(),
            selection_start: None,
            selection_end: None,
        }
    }

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        if self.cursor > self.buffer.len() {
            self.cursor = self.buffer.len();
        }
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Insert a string at the cursor position
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        if self.cursor > self.buffer.len() {
            self.cursor = self.buffer.len();
        }
        self.buffer.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor == 0 {
            return false;
        }
        // Find the previous character boundary
        let prev = self.prev_char_boundary(self.cursor);
        self.buffer.drain(prev..self.cursor);
        self.cursor = prev;
        true
    }

    /// Delete character at cursor (delete key)
    pub fn delete(&mut self) -> bool {
        if self.delete_selection() {
            return true;
        }
        if self.cursor >= self.buffer.len() {
            return false;
        }
        let next = self.next_char_boundary(self.cursor);
        self.buffer.drain(self.cursor..next);
        true
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        self.clear_selection();
        if self.cursor > 0 {
            self.cursor = self.prev_char_boundary(self.cursor);
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        self.clear_selection();
        if self.cursor < self.buffer.len() {
            self.cursor = self.next_char_boundary(self.cursor);
        }
    }

    /// Move cursor to beginning
    pub fn move_home(&mut self) {
        self.clear_selection();
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_end(&mut self) {
        self.clear_selection();
        self.cursor = self.buffer.len();
    }

    /// Select all text
    pub fn select_all(&mut self) {
        self.selection_start = Some(0);
        self.selection_end = Some(self.buffer.len());
        self.cursor = self.buffer.len();
    }

    /// Get selected text
    pub fn selected_text(&self) -> Option<&str> {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) if start != end => {
                let (s, e) = if start < end {
                    (start, end)
                } else {
                    (end, start)
                };
                Some(&self.buffer[s..e])
            }
            _ => None,
        }
    }

    /// Delete selected text
    pub fn delete_selection(&mut self) -> bool {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) if start != end => {
                let (s, e) = if start < end {
                    (start, end)
                } else {
                    (end, start)
                };
                self.buffer.drain(s..e);
                self.cursor = s;
                self.clear_selection();
                true
            }
            _ => false,
        }
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }

    /// Handle a key event. Returns true if text changed.
    pub fn handle_key(&mut self, key_code: u32, char_code: u32, modifiers: u8) -> bool {
        let ctrl = modifiers & 2 != 0;

        // Ctrl+A = select all
        if ctrl && (key_code == 65 || key_code == 97) {
            self.select_all();
            return false;
        }

        match key_code {
            8 => self.backspace(), // Backspace
            46 => self.delete(),   // Delete
            37 => {
                // Left arrow
                self.move_left();
                false
            }
            39 => {
                // Right arrow
                self.move_right();
                false
            }
            36 => {
                // Home
                self.move_home();
                false
            }
            35 => {
                // End
                self.move_end();
                false
            }
            _ => {
                // Printable character
                if (32..127).contains(&char_code) {
                    if let Some(ch) = char::from_u32(char_code) {
                        self.insert_char(ch);
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Get the display string with cursor marker for rendering.
    /// Returns (text_before_cursor, text_after_cursor).
    pub fn render_parts(&self) -> (&str, &str) {
        let pos = self.cursor.min(self.buffer.len());
        (&self.buffer[..pos], &self.buffer[pos..])
    }

    // -- helpers --

    fn prev_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.saturating_sub(1);
        while p > 0 && !self.buffer.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn next_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos + 1;
        while p < self.buffer.len() && !self.buffer.is_char_boundary(p) {
            p += 1;
        }
        p.min(self.buffer.len())
    }
}

// ---------------------------------------------------------------------------
// Link handling
// ---------------------------------------------------------------------------

/// Result of clicking on a link
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum LinkAction {
    /// Navigate to a new URL
    Navigate(String),
    /// Navigate within the same page (anchor)
    ScrollToAnchor(String),
    /// JavaScript URL
    JavaScript(String),
    /// No action
    None,
}

/// Extract href from a link node and determine navigation action
pub fn handle_click_on_link(href: &str) -> LinkAction {
    if href.is_empty() {
        return LinkAction::None;
    }
    if let Some(anchor) = href.strip_prefix('#') {
        return LinkAction::ScrollToAnchor(anchor.to_string());
    }
    if let Some(js) = href.strip_prefix("javascript:") {
        return LinkAction::JavaScript(js.to_string());
    }
    LinkAction::Navigate(href.to_string())
}

// ---------------------------------------------------------------------------
// Scroll state
// ---------------------------------------------------------------------------

/// Viewport scroll state
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    /// Current vertical scroll offset (pixels)
    pub scroll_y: i32,
    /// Maximum scroll offset (content_height - viewport_height)
    pub max_scroll_y: i32,
    /// Viewport height (pixels)
    pub viewport_height: i32,
    /// Total content height (pixels)
    pub content_height: i32,
    /// Horizontal scroll offset (pixels)
    pub scroll_x: i32,
    /// Maximum horizontal scroll
    pub max_scroll_x: i32,
    /// Viewport width
    pub viewport_width: i32,
    /// Total content width
    pub content_width: i32,
}

impl ScrollState {
    pub fn new(viewport_width: i32, viewport_height: i32) -> Self {
        Self {
            viewport_width,
            viewport_height,
            ..Default::default()
        }
    }

    /// Update content dimensions and recalculate max scroll
    pub fn set_content_size(&mut self, width: i32, height: i32) {
        self.content_width = width;
        self.content_height = height;
        self.max_scroll_y = (height - self.viewport_height).max(0);
        self.max_scroll_x = (width - self.viewport_width).max(0);
        self.clamp();
    }

    /// Update viewport dimensions
    pub fn set_viewport_size(&mut self, width: i32, height: i32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.max_scroll_y = (self.content_height - height).max(0);
        self.max_scroll_x = (self.content_width - width).max(0);
        self.clamp();
    }

    /// Scroll by a delta (positive = down/right)
    pub fn scroll_by(&mut self, dx: i32, dy: i32) {
        self.scroll_x += dx;
        self.scroll_y += dy;
        self.clamp();
    }

    /// Scroll to an absolute position
    pub fn scroll_to(&mut self, x: i32, y: i32) {
        self.scroll_x = x;
        self.scroll_y = y;
        self.clamp();
    }

    /// Scroll to make a vertical position visible
    pub fn ensure_visible_y(&mut self, y: i32, height: i32) {
        if y < self.scroll_y {
            self.scroll_y = y;
        } else if y + height > self.scroll_y + self.viewport_height {
            self.scroll_y = y + height - self.viewport_height;
        }
        self.clamp();
    }

    /// Whether a vertical scrollbar is needed
    pub fn needs_v_scrollbar(&self) -> bool {
        self.content_height > self.viewport_height
    }

    /// Whether a horizontal scrollbar is needed
    pub fn needs_h_scrollbar(&self) -> bool {
        self.content_width > self.viewport_width
    }

    /// Get scrollbar thumb position and size for vertical scrollbar.
    /// Returns (thumb_y, thumb_height) in pixels within the track.
    pub fn v_scrollbar_thumb(&self, track_height: i32) -> (i32, i32) {
        if self.content_height <= 0 || !self.needs_v_scrollbar() {
            return (0, track_height);
        }
        let thumb_h =
            (self.viewport_height as i64 * track_height as i64 / self.content_height as i64) as i32;
        let thumb_h = thumb_h.max(20); // minimum thumb size
        let scrollable = track_height - thumb_h;
        let thumb_y = if self.max_scroll_y > 0 {
            (self.scroll_y as i64 * scrollable as i64 / self.max_scroll_y as i64) as i32
        } else {
            0
        };
        (thumb_y, thumb_h)
    }

    fn clamp(&mut self) {
        self.scroll_x = self.scroll_x.clamp(0, self.max_scroll_x);
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll_y);
    }
}

// ---------------------------------------------------------------------------
// URL encoding helper
// ---------------------------------------------------------------------------

/// Simple URL encoding (percent-encoding)
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex_upper((b >> 4) & 0x0F));
                result.push(hex_upper(b & 0x0F));
            }
        }
    }
    result
}

fn hex_upper(n: u8) -> char {
    if n < 10 {
        (b'0' + n) as char
    } else {
        (b'A' + n - 10) as char
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_text_input_insert() {
        let mut ti = TextInput::new();
        ti.insert_char('h');
        ti.insert_char('i');
        assert_eq!(ti.buffer, "hi");
        assert_eq!(ti.cursor, 2);
    }

    #[test]
    fn test_text_input_insert_str() {
        let mut ti = TextInput::new();
        ti.insert_str("hello");
        assert_eq!(ti.buffer, "hello");
        assert_eq!(ti.cursor, 5);
    }

    #[test]
    fn test_text_input_backspace() {
        let mut ti = TextInput::from_text("abc");
        assert!(ti.backspace());
        assert_eq!(ti.buffer, "ab");
        assert_eq!(ti.cursor, 2);
    }

    #[test]
    fn test_text_input_backspace_empty() {
        let mut ti = TextInput::new();
        assert!(!ti.backspace());
    }

    #[test]
    fn test_text_input_delete() {
        let mut ti = TextInput::from_text("abc");
        ti.cursor = 0;
        assert!(ti.delete());
        assert_eq!(ti.buffer, "bc");
    }

    #[test]
    fn test_text_input_delete_at_end() {
        let mut ti = TextInput::from_text("abc");
        assert!(!ti.delete());
    }

    #[test]
    fn test_text_input_move_left_right() {
        let mut ti = TextInput::from_text("abc");
        ti.move_left();
        assert_eq!(ti.cursor, 2);
        ti.move_left();
        assert_eq!(ti.cursor, 1);
        ti.move_right();
        assert_eq!(ti.cursor, 2);
    }

    #[test]
    fn test_text_input_home_end() {
        let mut ti = TextInput::from_text("hello");
        ti.move_home();
        assert_eq!(ti.cursor, 0);
        ti.move_end();
        assert_eq!(ti.cursor, 5);
    }

    #[test]
    fn test_text_input_select_all() {
        let mut ti = TextInput::from_text("hello");
        ti.select_all();
        assert_eq!(ti.selection_start, Some(0));
        assert_eq!(ti.selection_end, Some(5));
        assert_eq!(ti.selected_text(), Some("hello"));
    }

    #[test]
    fn test_text_input_delete_selection() {
        let mut ti = TextInput::from_text("hello world");
        ti.selection_start = Some(5);
        ti.selection_end = Some(11);
        assert!(ti.delete_selection());
        assert_eq!(ti.buffer, "hello");
        assert_eq!(ti.cursor, 5);
    }

    #[test]
    fn test_text_input_render_parts() {
        let mut ti = TextInput::from_text("hello");
        ti.cursor = 3;
        let (before, after) = ti.render_parts();
        assert_eq!(before, "hel");
        assert_eq!(after, "lo");
    }

    #[test]
    fn test_input_element_password_display() {
        let mut ie = InputElement::new(0, InputType::Password, "pw");
        ie.set_value("secret");
        assert_eq!(ie.display_text(), "******");
    }

    #[test]
    fn test_input_element_placeholder() {
        let mut ie = InputElement::new(0, InputType::Text, "name");
        ie.placeholder = "Enter name".to_string();
        assert_eq!(ie.display_text(), "Enter name");
        ie.set_value("Alice");
        assert_eq!(ie.display_text(), "Alice");
    }

    #[test]
    fn test_input_element_toggle_checkbox() {
        let mut ie = InputElement::new(0, InputType::Checkbox, "agree");
        assert!(!ie.checked);
        ie.toggle_checked();
        assert!(ie.checked);
        ie.toggle_checked();
        assert!(!ie.checked);
    }

    #[test]
    fn test_form_encode() {
        let form = FormElement::new(0, "/submit", FormMethod::Post);
        let inputs = vec![
            {
                let mut ie = InputElement::new(1, InputType::Text, "user");
                ie.set_value("alice");
                ie
            },
            {
                let mut ie = InputElement::new(2, InputType::Text, "pass");
                ie.set_value("s&cr=t");
                ie
            },
        ];
        let encoded = form.encode_form_data(&inputs);
        assert!(encoded.contains("user=alice"));
        assert!(encoded.contains("pass=s%26cr%3Dt"));
    }

    #[test]
    fn test_link_navigate() {
        match handle_click_on_link("https://example.com") {
            LinkAction::Navigate(url) => assert_eq!(url, "https://example.com"),
            _ => panic!("expected Navigate"),
        }
    }

    #[test]
    fn test_link_anchor() {
        match handle_click_on_link("#top") {
            LinkAction::ScrollToAnchor(a) => assert_eq!(a, "top"),
            _ => panic!("expected ScrollToAnchor"),
        }
    }

    #[test]
    fn test_link_javascript() {
        match handle_click_on_link("javascript:alert(1)") {
            LinkAction::JavaScript(js) => assert_eq!(js, "alert(1)"),
            _ => panic!("expected JavaScript"),
        }
    }

    #[test]
    fn test_link_empty() {
        match handle_click_on_link("") {
            LinkAction::None => {}
            _ => panic!("expected None"),
        }
    }

    #[test]
    fn test_scroll_state_basic() {
        let mut s = ScrollState::new(800, 600);
        s.set_content_size(800, 1200);
        assert_eq!(s.max_scroll_y, 600);
        assert!(s.needs_v_scrollbar());
        assert!(!s.needs_h_scrollbar());
    }

    #[test]
    fn test_scroll_by() {
        let mut s = ScrollState::new(800, 600);
        s.set_content_size(800, 1200);
        s.scroll_by(0, 100);
        assert_eq!(s.scroll_y, 100);
        s.scroll_by(0, 1000);
        assert_eq!(s.scroll_y, 600); // clamped
    }

    #[test]
    fn test_scroll_to() {
        let mut s = ScrollState::new(800, 600);
        s.set_content_size(800, 1200);
        s.scroll_to(0, 300);
        assert_eq!(s.scroll_y, 300);
        s.scroll_to(0, -10);
        assert_eq!(s.scroll_y, 0); // clamped
    }

    #[test]
    fn test_scrollbar_thumb() {
        let mut s = ScrollState::new(800, 600);
        s.set_content_size(800, 1200);
        let (y, h) = s.v_scrollbar_thumb(600);
        // thumb should be about half the track (600/1200)
        assert_eq!(h, 300);
        assert_eq!(y, 0);
    }

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello"), "hello");
        assert_eq!(url_encode("a b"), "a+b");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn test_handle_key_printable() {
        let mut ti = TextInput::new();
        let changed = ti.handle_key(65, 65, 0); // 'A'
        assert!(changed);
        assert_eq!(ti.buffer, "A");
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut ti = TextInput::from_text("ab");
        let changed = ti.handle_key(8, 0, 0);
        assert!(changed);
        assert_eq!(ti.buffer, "a");
    }
}
