//! Brace expansion.
//!
//! Supports `{a,b,c}`, `{1..10}`, `{a..z}`, and `{01..10..2}`.

use alloc::{format, string::String, vec::Vec};

/// Expand brace expressions in a single word.
///
/// Returns a list of expanded words. If no braces are found, returns a
/// single-element list with the original word.
pub fn expand_braces(word: &str) -> Vec<String> {
    // Find the first unquoted, unnested `{`
    let (prefix, brace_start) = match find_brace_start(word) {
        Some(pos) => (&word[..pos], pos),
        None => return alloc::vec![String::from(word)],
    };

    // Find matching `}`
    let brace_end = match find_brace_end(word, brace_start) {
        Some(pos) => pos,
        None => return alloc::vec![String::from(word)],
    };

    let inner = &word[brace_start + 1..brace_end];
    let suffix = &word[brace_end + 1..];

    // Check if this is a sequence expression: {start..end[..step]}
    if let Some(items) = try_expand_sequence(inner) {
        let mut results = Vec::new();
        for item in &items {
            // Recursively expand braces in the suffix
            let combined = format!("{}{}{}", prefix, item, suffix);
            results.extend(expand_braces(&combined));
        }
        return results;
    }

    // Otherwise, it is a comma-separated list: {a,b,c}
    let alternatives = split_brace_alternatives(inner);
    if alternatives.len() <= 1 {
        // No commas found -- not a valid brace expansion
        return alloc::vec![String::from(word)];
    }

    let mut results = Vec::new();
    for alt in &alternatives {
        let combined = format!("{}{}{}", prefix, alt, suffix);
        results.extend(expand_braces(&combined));
    }
    results
}

/// Find the position of the first unquoted, unnested `{`.
fn find_brace_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            }
            b'{' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the position of the matching `}` for the `{` at `start`.
fn find_brace_end(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split brace alternatives on commas, respecting nesting.
fn split_brace_alternatives(inner: &str) -> Vec<String> {
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                current.push(bytes[i] as char);
                current.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            b'{' => {
                depth += 1;
                current.push('{');
            }
            b'}' => {
                depth -= 1;
                current.push('}');
            }
            b',' if depth == 0 => {
                parts.push(core::mem::take(&mut current));
                i += 1;
                continue;
            }
            b => {
                current.push(b as char);
            }
        }
        i += 1;
    }

    parts.push(current);
    parts
}

/// Try to expand a sequence expression like `1..10` or `a..z` or `01..10..2`.
fn try_expand_sequence(inner: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = inner.split("..").collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }

    let step = if parts.len() == 3 {
        parse_i64(parts[2])?
    } else {
        1
    };

    if step == 0 {
        return None;
    }

    // Try numeric sequence
    if let (Some(start), Some(end)) = (parse_i64(parts[0]), parse_i64(parts[1])) {
        let pad_width = if (parts[0].starts_with('0') && parts[0].len() > 1)
            || (parts[1].starts_with('0') && parts[1].len() > 1)
        {
            parts[0].len().max(parts[1].len())
        } else {
            0
        };

        let mut items = Vec::new();
        let mut i = start;
        let ascending = start <= end;
        let actual_step = if ascending { step.abs() } else { -(step.abs()) };

        loop {
            if ascending && i > end {
                break;
            }
            if !ascending && i < end {
                break;
            }
            if pad_width > 0 {
                items.push(zero_pad(i, pad_width));
            } else {
                items.push(format!("{}", i));
            }
            i += actual_step;
            if items.len() > 10000 {
                break;
            } // safety limit
        }
        return Some(items);
    }

    // Try character sequence
    if parts[0].len() == 1 && parts[1].len() == 1 {
        let start = parts[0].as_bytes()[0];
        let end = parts[1].as_bytes()[0];
        if start.is_ascii_alphabetic() && end.is_ascii_alphabetic() {
            let mut items = Vec::new();
            let step = step.unsigned_abs() as u8;
            if step == 0 {
                return None;
            }

            if start <= end {
                let mut c = start;
                while c <= end {
                    items.push(format!("{}", c as char));
                    c = match c.checked_add(step) {
                        Some(v) => v,
                        None => break,
                    };
                    if items.len() > 10000 {
                        break;
                    }
                }
            } else {
                let mut c = start;
                while c >= end {
                    items.push(format!("{}", c as char));
                    c = match c.checked_sub(step) {
                        Some(v) => v,
                        None => break,
                    };
                    if items.len() > 10000 {
                        break;
                    }
                }
            }
            return Some(items);
        }
    }

    None
}

fn parse_i64(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut n: i64 = 0;
    let mut neg = false;
    let mut i = 0;
    if bytes[0] == b'-' {
        neg = true;
        i = 1;
    } else if bytes[0] == b'+' {
        i = 1;
    }
    if i >= bytes.len() {
        return None;
    }
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((bytes[i] - b'0') as i64)?;
        i += 1;
    }
    Some(if neg { -n } else { n })
}

fn zero_pad(n: i64, width: usize) -> String {
    let s = format!("{}", n.abs());
    let needed = if width > s.len() { width - s.len() } else { 0 };
    let mut result = String::new();
    if n < 0 {
        result.push('-');
    }
    for _ in 0..needed {
        result.push('0');
    }
    result.push_str(&s);
    result
}
