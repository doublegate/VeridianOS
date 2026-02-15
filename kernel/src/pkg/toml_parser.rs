//! Minimal TOML Parser for Portfile.toml
//!
//! A no_std-compatible TOML parser supporting key-value pairs, sections,
//! arrays, and inline tables. Designed specifically for parsing port
//! definition files in the VeridianOS ports system.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[cfg(feature = "alloc")]
use crate::error::KernelError;

/// A parsed TOML value.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    /// A string value (e.g., `"hello"`)
    String(String),
    /// A 64-bit signed integer (e.g., `42`)
    Integer(i64),
    /// A boolean value (e.g., `true`)
    Boolean(bool),
    /// An array of values (e.g., `["a", "b"]`)
    Array(Vec<TomlValue>),
    /// A table / map of key-value pairs
    Table(BTreeMap<String, TomlValue>),
}

#[cfg(feature = "alloc")]
impl TomlValue {
    /// Try to interpret this value as a string reference.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            TomlValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to interpret this value as an integer.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            TomlValue::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to interpret this value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            TomlValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to interpret this value as an array.
    pub fn as_array(&self) -> Option<&[TomlValue]> {
        match self {
            TomlValue::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Try to interpret this value as a table.
    pub fn as_table(&self) -> Option<&BTreeMap<String, TomlValue>> {
        match self {
            TomlValue::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// Parse a TOML string into a nested `BTreeMap<String, TomlValue>`.
///
/// Supports:
/// - Key-value pairs: `key = "value"`, `key = 42`, `key = true`
/// - Sections: `[section]`
/// - Arrays: `["a", "b", "c"]`
/// - Inline tables: `{ key = "value", key2 = 42 }`
#[cfg(feature = "alloc")]
pub fn parse_toml(input: &str) -> Result<BTreeMap<String, TomlValue>, KernelError> {
    let mut root = BTreeMap::new();
    let mut current_section: Option<String> = None;

    for raw_line in input.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        // Section header: [section]
        if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
            let section_name = line[1..line.len() - 1].trim();
            if section_name.is_empty() {
                return Err(KernelError::InvalidArgument {
                    name: "toml_section",
                    value: "empty_section_name",
                });
            }
            current_section = Some(String::from(section_name));
            // Ensure the section table exists
            root.entry(String::from(section_name))
                .or_insert_with(|| TomlValue::Table(BTreeMap::new()));
            continue;
        }

        // Key-value pair: key = value
        if let Some((key, value)) = split_key_value(line) {
            let key = key.trim();
            let value = value.trim();

            let parsed_value = parse_value(value)?;

            if let Some(ref section) = current_section {
                // Insert into current section table
                if let Some(TomlValue::Table(table)) = root.get_mut(section) {
                    table.insert(String::from(key), parsed_value);
                }
            } else {
                // Insert into root
                root.insert(String::from(key), parsed_value);
            }
        }
    }

    Ok(root)
}

/// Strip inline comments (everything after an unquoted `#`).
#[cfg(feature = "alloc")]
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Split `key = value` at the first unquoted `=`.
#[cfg(feature = "alloc")]
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '=' if !in_string => {
                return Some((&line[..i], &line[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

/// Parse a single TOML value from its string representation.
#[cfg(feature = "alloc")]
fn parse_value(s: &str) -> Result<TomlValue, KernelError> {
    let s = s.trim();

    // Quoted string
    if s.starts_with('"') {
        return parse_string(s);
    }

    // Boolean
    if s == "true" {
        return Ok(TomlValue::Boolean(true));
    }
    if s == "false" {
        return Ok(TomlValue::Boolean(false));
    }

    // Array
    if s.starts_with('[') {
        return parse_array(s);
    }

    // Inline table
    if s.starts_with('{') {
        return parse_inline_table(s);
    }

    // Integer (try last to avoid misinterpreting other tokens)
    if let Some(n) = try_parse_integer(s) {
        return Ok(TomlValue::Integer(n));
    }

    Err(KernelError::InvalidArgument {
        name: "toml_value",
        value: "unrecognised_value",
    })
}

/// Parse a quoted string value. Handles basic escape sequences.
#[cfg(feature = "alloc")]
fn parse_string(s: &str) -> Result<TomlValue, KernelError> {
    if !s.starts_with('"') {
        return Err(KernelError::InvalidArgument {
            name: "toml_string",
            value: "missing_opening_quote",
        });
    }

    let mut result = String::new();
    let mut chars = s[1..].chars();
    let mut closed = false;

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                closed = true;
                break;
            }
            '\\' => {
                let escaped = chars.next().ok_or(KernelError::InvalidArgument {
                    name: "toml_string",
                    value: "unterminated_escape",
                })?;
                match escaped {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    _ => {
                        result.push('\\');
                        result.push(escaped);
                    }
                }
            }
            _ => result.push(c),
        }
    }

    if !closed {
        return Err(KernelError::InvalidArgument {
            name: "toml_string",
            value: "unterminated_string",
        });
    }

    Ok(TomlValue::String(result))
}

/// Parse an array value: `[val1, val2, ...]`
#[cfg(feature = "alloc")]
fn parse_array(s: &str) -> Result<TomlValue, KernelError> {
    if !s.starts_with('[') || !s.ends_with(']') {
        return Err(KernelError::InvalidArgument {
            name: "toml_array",
            value: "malformed_array",
        });
    }

    let inner = s[1..s.len() - 1].trim();
    if inner.is_empty() {
        return Ok(TomlValue::Array(Vec::new()));
    }

    let elements = split_top_level(inner, ',');
    let mut values = Vec::new();
    for elem in elements {
        let elem = elem.trim();
        if !elem.is_empty() {
            values.push(parse_value(elem)?);
        }
    }

    Ok(TomlValue::Array(values))
}

/// Parse an inline table: `{ key = val, key2 = val2 }`
#[cfg(feature = "alloc")]
fn parse_inline_table(s: &str) -> Result<TomlValue, KernelError> {
    if !s.starts_with('{') || !s.ends_with('}') {
        return Err(KernelError::InvalidArgument {
            name: "toml_table",
            value: "malformed_inline_table",
        });
    }

    let inner = s[1..s.len() - 1].trim();
    if inner.is_empty() {
        return Ok(TomlValue::Table(BTreeMap::new()));
    }

    let pairs = split_top_level(inner, ',');
    let mut table = BTreeMap::new();
    for pair in pairs {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        if let Some((key, value)) = split_key_value(pair) {
            table.insert(String::from(key.trim()), parse_value(value)?);
        } else {
            return Err(KernelError::InvalidArgument {
                name: "toml_table",
                value: "missing_equals_in_inline_table",
            });
        }
    }

    Ok(TomlValue::Table(table))
}

/// Split a string by `delimiter`, respecting quoted strings and nested
/// brackets / braces.
#[cfg(feature = "alloc")]
fn split_top_level(s: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut start = 0;

    for (i, c) in s.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '[' | '{' if !in_string => depth += 1,
            ']' | '}' if !in_string => depth -= 1,
            c if c == delimiter && !in_string && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    // Push the final segment
    if start <= s.len() {
        parts.push(&s[start..]);
    }

    parts
}

/// Try to parse a string as a signed 64-bit integer.
#[cfg(feature = "alloc")]
fn try_parse_integer(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Handle hex, octal, binary prefixes
    if s.starts_with("0x") || s.starts_with("0X") {
        return i64::from_str_radix(&s[2..].replace('_', ""), 16).ok();
    }
    if s.starts_with("0o") || s.starts_with("0O") {
        return i64::from_str_radix(&s[2..].replace('_', ""), 8).ok();
    }
    if s.starts_with("0b") || s.starts_with("0B") {
        return i64::from_str_radix(&s[2..].replace('_', ""), 2).ok();
    }

    // Decimal -- allow underscores as visual separators
    let cleaned: String = s.chars().filter(|&c| c != '_').collect();
    cleaned.parse::<i64>().ok()
}
