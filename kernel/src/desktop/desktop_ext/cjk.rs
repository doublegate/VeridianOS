//! CJK Unicode / Wide Character Support
//!
//! Wide character detection, double-width cell rendering, and IME framework.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

/// Check if a character is a CJK wide character (occupies 2 cells).
///
/// Based on Unicode East Asian Width property and common CJK ranges:
/// - CJK Unified Ideographs (U+4E00-U+9FFF)
/// - CJK Unified Ideographs Extension A (U+3400-U+4DBF)
/// - CJK Compatibility Ideographs (U+F900-U+FAFF)
/// - Hangul Syllables (U+AC00-U+D7AF)
/// - Katakana (U+30A0-U+30FF)
/// - Hiragana (U+3040-U+309F)
/// - CJK Symbols and Punctuation (U+3000-U+303F)
/// - Fullwidth Forms (U+FF01-U+FF60, U+FFE0-U+FFE6)
/// - Bopomofo (U+3100-U+312F)
/// - Enclosed CJK (U+3200-U+32FF)
/// - CJK Compatibility (U+3300-U+33FF)
/// - CJK Unified Ideographs Extension B+ (U+20000-U+2A6DF)
pub fn is_cjk_wide(ch: char) -> bool {
    let cp = ch as u32;

    // Check the most common ranges first for performance.
    if (0x4E00..=0x9FFF).contains(&cp) {
        return true;
    }
    if (0xAC00..=0xD7AF).contains(&cp) {
        return true;
    }
    if (0x3040..=0x30FF).contains(&cp) {
        return true;
    }
    if (0xFF01..=0xFF60).contains(&cp) {
        return true;
    }
    if (0xFFE0..=0xFFE6).contains(&cp) {
        return true;
    }
    if (0x3400..=0x4DBF).contains(&cp) {
        return true;
    }
    if (0x3000..=0x303F).contains(&cp) {
        return true;
    }
    if (0x3100..=0x312F).contains(&cp) {
        return true;
    }
    if (0x3200..=0x33FF).contains(&cp) {
        return true;
    }
    if (0xF900..=0xFAFF).contains(&cp) {
        return true;
    }
    if (0x20000..=0x2A6DF).contains(&cp) {
        return true;
    }

    false
}

/// Get the display width of a character in terminal cells.
///
/// Returns 2 for wide (CJK) characters, 0 for zero-width characters
/// (combining marks, control chars), and 1 for everything else.
pub fn char_width(ch: char) -> u8 {
    let cp = ch as u32;

    // Control characters and zero-width.
    if cp == 0 || (0x01..=0x1F).contains(&cp) || cp == 0x7F {
        return 0;
    }

    // Combining marks (general category Mn/Mc/Me).
    if (0x0300..=0x036F).contains(&cp) {
        return 0; // Combining Diacritical Marks
    }
    if (0x1AB0..=0x1AFF).contains(&cp) {
        return 0; // Combining Diacritical Marks Extended
    }
    if (0x1DC0..=0x1DFF).contains(&cp) {
        return 0; // Combining Diacritical Marks Supplement
    }
    if (0x20D0..=0x20FF).contains(&cp) {
        return 0; // Combining Diacritical Marks for Symbols
    }
    if (0xFE20..=0xFE2F).contains(&cp) {
        return 0; // Combining Half Marks
    }

    // Soft hyphen.
    if cp == 0x00AD {
        return 1;
    }

    // Zero-width joiner / non-joiner / space.
    if cp == 0x200B || cp == 0x200C || cp == 0x200D || cp == 0xFEFF {
        return 0;
    }

    if is_cjk_wide(ch) {
        return 2;
    }

    1
}

/// Calculate the display width of a string in terminal cells.
#[cfg(feature = "alloc")]
pub fn string_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c) as usize).sum()
}

/// Truncate a string to fit within `max_width` terminal cells.
/// Appends "..." if truncated.
#[cfg(feature = "alloc")]
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width < 3 {
        return String::new();
    }

    let mut width = 0usize;
    let mut result = String::new();

    for ch in s.chars() {
        let cw = char_width(ch) as usize;
        if width + cw > max_width - 3 {
            result.push_str("...");
            return result;
        }
        result.push(ch);
        width += cw;
    }

    result
}

/// Double-width cell renderer helper.
///
/// When rendering a wide character at cell (col, row), it occupies
/// cells (col, row) and (col+1, row). The second cell should be marked
/// as a continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellContent {
    /// Normal single-width character.
    Narrow(char),
    /// First cell of a wide character.
    WideStart(char),
    /// Continuation of a wide character (second cell).
    WideContinuation,
    /// Empty cell.
    #[default]
    Empty,
}

/// Input Method Editor (IME) state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeState {
    /// IME is inactive (direct input).
    #[default]
    Inactive,
    /// Composing: user is typing a sequence that will be converted.
    Composing,
    /// Committed: the composed text has been finalized.
    Committed,
}

/// A candidate in the IME candidate list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ImeCandidate {
    /// Display label (e.g., "1", "2").
    pub label: String,
    /// The candidate text.
    pub text: String,
}

/// Input Method Editor framework.
///
/// Provides the state machine and data structures for input composition.
/// Actual input method dictionaries would be loaded from user space.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct InputMethodEditor {
    /// Current IME state.
    state: ImeState,
    /// Preedit (composing) string.
    preedit: String,
    /// Cursor position within preedit.
    preedit_cursor: usize,
    /// Candidate list.
    candidates: Vec<ImeCandidate>,
    /// Selected candidate index.
    selected_candidate: usize,
    /// Committed text (ready for insertion).
    committed: String,
    /// Whether the IME is enabled.
    enabled: bool,
    /// Pinyin lookup table (stub).
    pinyin_table: BTreeMap<String, Vec<String>>,
}

#[cfg(feature = "alloc")]
impl Default for InputMethodEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl InputMethodEditor {
    /// Create a new IME with basic Pinyin stub entries.
    pub fn new() -> Self {
        let mut pinyin_table = BTreeMap::new();

        // Basic Pinyin stub entries for common characters.
        pinyin_table.insert(
            String::from("ni"),
            alloc::vec![String::from("\u{4F60}"), String::from("\u{5C3C}")],
        );
        pinyin_table.insert(
            String::from("hao"),
            alloc::vec![String::from("\u{597D}"), String::from("\u{53F7}")],
        );
        pinyin_table.insert(
            String::from("shi"),
            alloc::vec![
                String::from("\u{662F}"),
                String::from("\u{4E16}"),
                String::from("\u{4E8B}"),
            ],
        );
        pinyin_table.insert(
            String::from("de"),
            alloc::vec![String::from("\u{7684}"), String::from("\u{5F97}")],
        );
        pinyin_table.insert(String::from("wo"), alloc::vec![String::from("\u{6211}")]);
        pinyin_table.insert(
            String::from("ren"),
            alloc::vec![String::from("\u{4EBA}"), String::from("\u{8BA4}")],
        );
        pinyin_table.insert(
            String::from("da"),
            alloc::vec![String::from("\u{5927}"), String::from("\u{6253}")],
        );
        pinyin_table.insert(
            String::from("zhong"),
            alloc::vec![String::from("\u{4E2D}"), String::from("\u{91CD}")],
        );
        pinyin_table.insert(
            String::from("guo"),
            alloc::vec![String::from("\u{56FD}"), String::from("\u{8FC7}")],
        );
        pinyin_table.insert(
            String::from("yi"),
            alloc::vec![
                String::from("\u{4E00}"),
                String::from("\u{4E49}"),
                String::from("\u{5DF2}"),
            ],
        );

        Self {
            state: ImeState::Inactive,
            preedit: String::new(),
            preedit_cursor: 0,
            candidates: Vec::new(),
            selected_candidate: 0,
            committed: String::new(),
            enabled: false,
            pinyin_table,
        }
    }

    /// Enable or disable the IME.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.reset();
        }
    }

    /// Check if IME is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get current IME state.
    pub fn state(&self) -> ImeState {
        self.state
    }

    /// Get the preedit string (what the user is typing).
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// Get the preedit cursor position.
    pub fn preedit_cursor(&self) -> usize {
        self.preedit_cursor
    }

    /// Get the candidate list.
    pub fn candidates(&self) -> &[ImeCandidate] {
        &self.candidates
    }

    /// Get the selected candidate index.
    pub fn selected_candidate(&self) -> usize {
        self.selected_candidate
    }

    /// Get and clear the committed text.
    pub fn take_committed(&mut self) -> String {
        let result = core::mem::take(&mut self.committed);
        if self.state == ImeState::Committed {
            self.state = ImeState::Inactive;
        }
        result
    }

    /// Feed a character into the IME.
    pub fn feed_char(&mut self, ch: char) {
        if !self.enabled {
            self.committed.push(ch);
            self.state = ImeState::Committed;
            return;
        }

        if ch.is_ascii_alphabetic() {
            self.preedit.push(ch.to_ascii_lowercase());
            self.preedit_cursor = self.preedit.len();
            self.state = ImeState::Composing;
            self.update_candidates();
        } else if ch.is_ascii_digit() && self.state == ImeState::Composing {
            // Select candidate by number.
            let idx = (ch as u8 - b'1') as usize;
            self.select_candidate(idx);
        } else if ch == ' ' && self.state == ImeState::Composing {
            // Commit first candidate.
            self.select_candidate(0);
        } else {
            // Non-alphabetic input while not composing: pass through.
            if self.state == ImeState::Composing {
                self.commit_preedit();
            }
            self.committed.push(ch);
            self.state = ImeState::Committed;
        }
    }

    /// Feed a backspace into the IME.
    pub fn feed_backspace(&mut self) {
        if self.state == ImeState::Composing && !self.preedit.is_empty() {
            self.preedit.pop();
            self.preedit_cursor = self.preedit.len();
            if self.preedit.is_empty() {
                self.state = ImeState::Inactive;
                self.candidates.clear();
            } else {
                self.update_candidates();
            }
        }
    }

    /// Feed an Enter key: commit preedit as-is.
    pub fn feed_enter(&mut self) {
        if self.state == ImeState::Composing {
            self.commit_preedit();
        }
    }

    /// Feed Escape: cancel composition.
    pub fn feed_escape(&mut self) {
        self.reset();
    }

    /// Move candidate selection up.
    pub fn candidate_prev(&mut self) {
        if !self.candidates.is_empty() && self.selected_candidate > 0 {
            self.selected_candidate -= 1;
        }
    }

    /// Move candidate selection down.
    pub fn candidate_next(&mut self) {
        if !self.candidates.is_empty() && self.selected_candidate + 1 < self.candidates.len() {
            self.selected_candidate += 1;
        }
    }

    /// Update the candidate list based on current preedit.
    fn update_candidates(&mut self) {
        self.candidates.clear();
        self.selected_candidate = 0;

        if let Some(chars) = self.pinyin_table.get(&self.preedit) {
            for (i, text) in chars.iter().enumerate() {
                self.candidates.push(ImeCandidate {
                    label: String::from(match i {
                        0 => "1",
                        1 => "2",
                        2 => "3",
                        3 => "4",
                        4 => "5",
                        5 => "6",
                        6 => "7",
                        7 => "8",
                        8 => "9",
                        _ => "?",
                    }),
                    text: text.clone(),
                });
            }
        }
    }

    /// Select and commit a candidate by index.
    fn select_candidate(&mut self, idx: usize) {
        if idx < self.candidates.len() {
            self.committed = self.candidates[idx].text.clone();
        } else if !self.preedit.is_empty() {
            // No matching candidate: commit preedit as-is.
            self.committed = core::mem::take(&mut self.preedit);
        }
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.state = ImeState::Committed;
    }

    /// Commit the raw preedit string.
    fn commit_preedit(&mut self) {
        self.committed = core::mem::take(&mut self.preedit);
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.state = ImeState::Committed;
    }

    /// Reset the IME to inactive state.
    pub fn reset(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.committed.clear();
        self.state = ImeState::Inactive;
    }
}
