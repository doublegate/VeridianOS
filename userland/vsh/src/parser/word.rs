//! Word parsing utilities.
//!
//! Handles splitting, quoting detection, and preliminary processing of
//! words before they are sent through the expansion pipeline.

use alloc::{string::String, vec::Vec};

/// Check if a word contains any expansion syntax that needs processing.
#[allow(dead_code)] // Used for optimization: skip expansion when not needed
pub fn needs_expansion(word: &str) -> bool {
    let bytes = word.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'$' | b'`' | b'~' | b'*' | b'?' | b'[' => return true,
            b'\\' => {
                i += 2; // skip escaped character
                continue;
            }
            b'\'' => {
                // Skip single-quoted region (no expansion)
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // closing quote
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Remove quotes from a word (quote removal -- the final step of expansion).
///
/// Removes unescaped single quotes, double quotes, and resolves backslash
/// escapes.
pub fn remove_quotes(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;

    while i < len {
        let ch = chars[i];

        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                result.push(ch);
            }
            i += 1;
            continue;
        }

        if in_double {
            if ch == '"' {
                in_double = false;
                i += 1;
                continue;
            }
            if ch == '\\' && i + 1 < len {
                let next = chars[i + 1];
                if matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    result.push(next);
                    i += 2;
                    continue;
                }
            }
            result.push(ch);
            i += 1;
            continue;
        }

        // Unquoted context
        match ch {
            '\'' => {
                in_single = true;
                i += 1;
            }
            '"' => {
                in_double = true;
                i += 1;
            }
            '\\' if i + 1 < len => {
                result.push(chars[i + 1]);
                i += 2;
            }
            _ => {
                result.push(ch);
                i += 1;
            }
        }
    }

    result
}

/// Perform word splitting on a string using IFS characters.
///
/// Default IFS is space, tab, newline. Leading/trailing IFS chars are
/// trimmed, and sequences of IFS chars between words are collapsed.
pub fn word_split(value: &str, ifs: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }

    let ifs_chars: Vec<char> = if ifs.is_empty() {
        // Empty IFS: no splitting
        return alloc::vec![String::from(value)];
    } else {
        ifs.chars().collect()
    };

    let is_ifs = |ch: char| -> bool { ifs_chars.contains(&ch) };
    let is_ifs_whitespace =
        |ch: char| -> bool { (ch == ' ' || ch == '\t' || ch == '\n') && ifs_chars.contains(&ch) };

    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;

    // Skip leading IFS whitespace
    while i < chars.len() && is_ifs_whitespace(chars[i]) {
        i += 1;
    }

    while i < chars.len() {
        let ch = chars[i];

        if is_ifs(ch) {
            if !current.is_empty() {
                words.push(core::mem::take(&mut current));
            }
            // Skip whitespace IFS chars
            while i < chars.len() && is_ifs_whitespace(chars[i]) {
                i += 1;
            }
            // If a non-whitespace IFS char follows, it is a delimiter
            if i < chars.len() && is_ifs(chars[i]) && !is_ifs_whitespace(chars[i]) {
                i += 1;
                // Skip trailing IFS whitespace
                while i < chars.len() && is_ifs_whitespace(chars[i]) {
                    i += 1;
                }
            }
        } else {
            current.push(ch);
            i += 1;
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}
