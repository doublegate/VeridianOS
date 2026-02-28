//! Here-document handling.
//!
//! Supports `<<DELIM ... DELIM`, `<<-DELIM ... DELIM` (tab stripping),
//! and `<<<'here string'` (here-strings).

use alloc::{string::String, vec::Vec};

/// A pending here-document that needs its body collected.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields read during heredoc body collection
pub struct PendingHereDoc {
    /// The delimiter word (without quotes).
    pub delimiter: String,
    /// Whether to strip leading tabs (`<<-`).
    pub strip_tabs: bool,
    /// Whether the delimiter was quoted (suppresses expansion in the body).
    pub quoted: bool,
}

impl PendingHereDoc {
    pub fn new(raw_delimiter: &str, strip_tabs: bool) -> Self {
        let (delimiter, quoted) = strip_heredoc_quotes(raw_delimiter);
        Self {
            delimiter,
            strip_tabs,
            quoted,
        }
    }
}

/// Strip quotes from a here-document delimiter and determine if it was
/// quoted.
///
/// Bash rules:
/// - `<<'EOF'` or `<<"EOF"` or `<<\EOF`: the delimiter is `EOF`, and the body
///   is NOT subject to expansion.
/// - `<<EOF`: the delimiter is `EOF`, and the body IS subject to expansion.
fn strip_heredoc_quotes(raw: &str) -> (String, bool) {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return (String::new(), false);
    }

    // Check for surrounding single quotes
    if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        let inner = &raw[1..raw.len() - 1];
        return (String::from(inner), true);
    }

    // Check for surrounding double quotes
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        let inner = &raw[1..raw.len() - 1];
        return (String::from(inner), true);
    }

    // Check for leading backslash on any character
    let mut result = String::with_capacity(raw.len());
    let mut had_backslash = false;
    let mut i = 0;
    let chars: Vec<char> = raw.chars().collect();
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            result.push(chars[i + 1]);
            had_backslash = true;
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    (result, had_backslash)
}

/// Collect a here-document body from input lines.
///
/// `lines` is an iterator over input lines (without trailing newlines).
/// Reads lines until `delimiter` is found on a line by itself. If
/// `strip_tabs` is true, leading tabs are removed from each line and
/// the delimiter line.
///
/// Returns the collected body (including embedded newlines).
#[allow(dead_code)] // Used when heredoc bodies are collected during parsing
pub fn collect_heredoc_body(
    lines: &[&str],
    start_line: usize,
    delimiter: &str,
    strip_tabs: bool,
) -> (String, usize) {
    let mut body = String::new();
    let mut line_idx = start_line;

    while line_idx < lines.len() {
        let mut line = lines[line_idx];
        line_idx += 1;

        // If stripping tabs, remove leading tabs
        if strip_tabs {
            line = line.trim_start_matches('\t');
        }

        // Check if this line is the delimiter
        if line == delimiter {
            return (body, line_idx);
        }

        // Append line to body
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(line);
    }

    // Reached end of input without finding delimiter -- return what we have
    (body, line_idx)
}

/// Parse a here-string value from `<<<word`.
///
/// The word may be quoted (single or double) or unquoted.
#[allow(dead_code)] // Used when here-strings are expanded
pub fn parse_here_string(word: &str) -> String {
    let bytes = word.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }

    // Strip surrounding quotes if present
    if bytes.len() >= 2
        && ((bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"'))
    {
        return String::from(&word[1..word.len() - 1]);
    }

    String::from(word)
}
