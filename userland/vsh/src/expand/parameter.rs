//! Parameter expansion.
//!
//! Handles `$VAR`, `${VAR}`, and all the `${VAR...}` operators from Bash.

use alloc::{collections::BTreeMap, format, string::String};

use crate::expand::glob;

/// Expand parameters/variables in a word.
///
/// Handles: `$VAR`, `${VAR}`, `${VAR:-default}`, `${VAR:=assign}`,
/// `${VAR:+alt}`, `${VAR:?error}`, `${#VAR}`, `${VAR%pat}`,
/// `${VAR%%pat}`, `${VAR#pat}`, `${VAR##pat}`, `${VAR/pat/rep}`,
/// `${VAR//pat/rep}`, `${VAR^pat}`, `${VAR^^pat}`, `${VAR,pat}`,
/// `${VAR,,pat}`, `${VAR:offset:length}`, and special variables.
pub fn expand_parameters(
    input: &str,
    vars: &BTreeMap<String, String>,
    special: &SpecialVars,
) -> String {
    let chars: alloc::vec::Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let ch = chars[i];

        // Single-quote toggle (not inside double quotes)
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            i += 1;
            continue;
        }

        // Inside single quotes: literal pass-through
        if in_single_quote {
            result.push(ch);
            i += 1;
            continue;
        }

        // Double-quote toggle
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }

        // Backslash escape
        if ch == '\\' && i + 1 < len {
            if in_double_quote {
                let next = chars[i + 1];
                if matches!(next, '$' | '`' | '"' | '\\') {
                    result.push(next);
                    i += 2;
                    continue;
                }
            } else {
                result.push(chars[i + 1]);
                i += 2;
                continue;
            }
        }

        // Dollar expansion
        if ch == '$' && i + 1 < len {
            let next = chars[i + 1];

            // ${...} braced expansion
            if next == '{' {
                i += 2;
                let (expanded, consumed) = expand_braced_param(&chars[i..], vars, special);
                result.push_str(&expanded);
                i += consumed;
                continue;
            }

            // $((expr)) arithmetic
            if next == '(' && i + 2 < len && chars[i + 2] == '(' {
                // Skip `$((`, find matching `))`
                i += 3;
                let start = i;
                let mut depth = 1;
                while i < len && depth > 0 {
                    if i + 1 < len && chars[i] == ')' && chars[i + 1] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    if i + 1 < len
                        && chars[i] == '$'
                        && chars[i + 1] == '('
                        && i + 2 < len
                        && chars[i + 2] == '('
                    {
                        depth += 1;
                    }
                    i += 1;
                }
                let expr: String = chars[start..i].iter().collect();
                // Skip the closing `))`
                if i + 1 < len {
                    i += 2;
                }
                let val = crate::parser::arithmetic::eval_arithmetic(&expr, vars).unwrap_or(0);
                result.push_str(&format!("{}", val));
                continue;
            }

            // $(cmd) command substitution placeholder
            if next == '(' {
                // For now, emit the raw $(...) -- command substitution is
                // handled during execution, not during expansion.
                result.push('$');
                i += 1;
                continue;
            }

            // Special variables
            match next {
                '?' => {
                    result.push_str(&format!("{}", special.exit_status));
                    i += 2;
                    continue;
                }
                '$' => {
                    result.push_str(&format!("{}", special.pid));
                    i += 2;
                    continue;
                }
                '!' => {
                    result.push_str(&format!("{}", special.last_bg_pid));
                    i += 2;
                    continue;
                }
                '#' => {
                    result.push_str(&format!("{}", special.argc));
                    i += 2;
                    continue;
                }
                '0' => {
                    result.push_str(&special.arg0);
                    i += 2;
                    continue;
                }
                '-' => {
                    result.push_str(&special.flags);
                    i += 2;
                    continue;
                }
                '_' => {
                    result.push_str(&special.last_arg);
                    i += 2;
                    continue;
                }
                '@' | '*' => {
                    let sep = " ";
                    result.push_str(&special.positional.join(sep));
                    i += 2;
                    continue;
                }
                _ => {}
            }

            // Positional parameters: $1 - $9
            if next.is_ascii_digit() && next != '0' {
                let idx = (next as u8 - b'1') as usize;
                if idx < special.positional.len() {
                    result.push_str(&special.positional[idx]);
                }
                i += 2;
                continue;
            }

            // Bare $VAR
            if is_var_start(next) {
                i += 1; // skip $
                let start = i;
                while i < len && is_var_char(chars[i]) {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                if let Some(val) = vars.get(&name) {
                    result.push_str(val);
                }
                continue;
            }

            // Bare $ -- literal
            result.push('$');
            i += 1;
            continue;
        }

        // Tilde expansion (only at word start or after : in assignments)
        if ch == '~'
            && i == 0
            && !in_double_quote
            && (i + 1 >= len || chars[i + 1] == '/' || chars[i + 1] == ' ')
        {
            if let Some(home) = vars.get("HOME") {
                result.push_str(home);
                i += 1;
                continue;
            }
        }

        result.push(ch);
        i += 1;
    }

    result
}

/// Special shell variables.
#[derive(Debug, Clone)]
pub struct SpecialVars {
    /// `$?` -- exit status of last command.
    pub exit_status: i32,
    /// `$$` -- PID of the shell.
    pub pid: u32,
    /// `$!` -- PID of last background process.
    pub last_bg_pid: u32,
    /// `$#` -- number of positional parameters.
    pub argc: usize,
    /// `$0` -- shell name or script name.
    pub arg0: String,
    /// `$-` -- current option flags.
    pub flags: String,
    /// `$_` -- last argument of previous command.
    pub last_arg: String,
    /// Positional parameters `$1`, `$2`, ...
    pub positional: alloc::vec::Vec<String>,
}

impl Default for SpecialVars {
    fn default() -> Self {
        Self {
            exit_status: 0,
            pid: 0,
            last_bg_pid: 0,
            argc: 0,
            arg0: String::from("vsh"),
            flags: String::new(),
            last_arg: String::new(),
            positional: alloc::vec::Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Braced parameter expansion
// ---------------------------------------------------------------------------

fn expand_braced_param(
    chars: &[char],
    vars: &BTreeMap<String, String>,
    special: &SpecialVars,
) -> (String, usize) {
    let len = chars.len();

    // Find closing brace
    let close = match find_closing_brace(chars) {
        Some(pos) => pos,
        None => {
            let literal: String = chars.iter().collect();
            return (format!("${{{}", literal), len);
        }
    };

    let inner: String = chars[..close].iter().collect();
    let consumed = close + 1;

    // ${#VAR} -- string length
    if let Some(var_name) = inner.strip_prefix('#') {
        let length = vars.get(var_name).map(|v| v.len()).unwrap_or(0);
        return (format!("{}", length), consumed);
    }

    // ${VAR%%pattern} -- longest suffix removal
    if let Some(pos) = inner.find("%%") {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 2..];
        let value = get_var(var_name, vars, special);
        return (remove_suffix_longest(&value, pattern), consumed);
    }

    // ${VAR%pattern} -- shortest suffix removal
    if let Some(pos) = inner.find('%') {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 1..];
        let value = get_var(var_name, vars, special);
        return (remove_suffix_shortest(&value, pattern), consumed);
    }

    // ${VAR##pattern} -- longest prefix removal
    if let Some(pos) = inner.find("##") {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 2..];
        let value = get_var(var_name, vars, special);
        return (remove_prefix_longest(&value, pattern), consumed);
    }

    // ${VAR#pattern} -- shortest prefix removal
    if let Some(pos) = inner.find('#') {
        if pos > 0 {
            let var_name = &inner[..pos];
            let pattern = &inner[pos + 1..];
            let value = get_var(var_name, vars, special);
            return (remove_prefix_shortest(&value, pattern), consumed);
        }
    }

    // ${VAR//pat/rep} -- global replacement (check before single /)
    if let Some(pos) = inner.find("//") {
        let var_name = &inner[..pos];
        let rest = &inner[pos + 2..];
        let (pattern, replacement) = split_at_unescaped_slash(rest);
        let value = get_var(var_name, vars, special);
        return (replace_all(&value, &pattern, &replacement), consumed);
    }

    // ${VAR/pat/rep} -- single replacement
    if let Some(pos) = inner.find('/') {
        let var_name = &inner[..pos];
        let rest = &inner[pos + 1..];
        let (pattern, replacement) = split_at_unescaped_slash(rest);
        let value = get_var(var_name, vars, special);
        return (replace_first(&value, &pattern, &replacement), consumed);
    }

    // ${VAR^^pattern} -- uppercase all
    if let Some(pos) = inner.find("^^") {
        let var_name = &inner[..pos];
        let value = get_var(var_name, vars, special);
        return (value.to_ascii_uppercase(), consumed);
    }

    // ${VAR^pattern} -- uppercase first
    if let Some(pos) = inner.find('^') {
        if pos > 0 || (pos == 0 && inner.len() > 1) {
            let var_name = &inner[..pos];
            let value = get_var(var_name, vars, special);
            return (uppercase_first(&value), consumed);
        }
    }

    // ${VAR,,pattern} -- lowercase all
    if let Some(pos) = inner.find(",,") {
        let var_name = &inner[..pos];
        let value = get_var(var_name, vars, special);
        return (value.to_ascii_lowercase(), consumed);
    }

    // ${VAR,pattern} -- lowercase first
    if let Some(pos) = inner.find(',') {
        if pos > 0 {
            let var_name = &inner[..pos];
            let value = get_var(var_name, vars, special);
            return (lowercase_first(&value), consumed);
        }
    }

    // ${VAR:offset:length} -- substring
    if let Some(colon_pos) = inner.find(':') {
        let var_name = &inner[..colon_pos];
        let rest = &inner[colon_pos + 1..];

        // Check for ${VAR:-default}, ${VAR:=assign}, ${VAR:+alt}, ${VAR:?error}
        if let Some(default_val) = rest.strip_prefix('-') {
            let value = get_var(var_name, vars, special);
            if value.is_empty() {
                return (String::from(default_val), consumed);
            }
            return (value, consumed);
        }
        if let Some(assign_val) = rest.strip_prefix('=') {
            let value = get_var(var_name, vars, special);
            if value.is_empty() {
                // In a real shell, we would assign the value to the variable
                return (String::from(assign_val), consumed);
            }
            return (value, consumed);
        }
        if let Some(alt_val) = rest.strip_prefix('+') {
            let value = get_var(var_name, vars, special);
            if !value.is_empty() {
                return (String::from(alt_val), consumed);
            }
            return (String::new(), consumed);
        }
        if let Some(_err_msg) = rest.strip_prefix('?') {
            let value = get_var(var_name, vars, special);
            if value.is_empty() {
                // In a real shell, we would print the error and exit
                return (String::new(), consumed);
            }
            return (value, consumed);
        }

        // Substring: ${VAR:offset} or ${VAR:offset:length}
        let value = get_var(var_name, vars, special);
        if let Some(substr) = substring(&value, rest) {
            return (substr, consumed);
        }
    }

    // Plain ${VAR}
    let value = get_var(&inner, vars, special);
    (value, consumed)
}

fn get_var(name: &str, vars: &BTreeMap<String, String>, special: &SpecialVars) -> String {
    match name {
        "?" => format!("{}", special.exit_status),
        "$" => format!("{}", special.pid),
        "!" => format!("{}", special.last_bg_pid),
        "#" => format!("{}", special.argc),
        "0" => special.arg0.clone(),
        "-" => special.flags.clone(),
        "_" => special.last_arg.clone(),
        "@" | "*" => special.positional.join(" "),
        _ => {
            // Positional $1-$9
            if name.len() == 1 {
                let b = name.as_bytes()[0];
                if (b'1'..=b'9').contains(&b) {
                    let idx = (b - b'1') as usize;
                    if idx < special.positional.len() {
                        return special.positional[idx].clone();
                    }
                    return String::new();
                }
            }
            vars.get(name).cloned().unwrap_or_default()
        }
    }
}

fn find_closing_brace(chars: &[char]) -> Option<usize> {
    let mut depth = 1u32;
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_var_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_var_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// ---------------------------------------------------------------------------
// Pattern matching helpers
// ---------------------------------------------------------------------------

fn pattern_match(pattern: &str, text: &str) -> bool {
    glob::glob_match(pattern, text)
}

fn remove_suffix_shortest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return String::from(value);
    }
    for (i, _) in value.char_indices().rev() {
        if pattern_match(pattern, &value[i..]) {
            return String::from(&value[..i]);
        }
    }
    if pattern_match(pattern, value) {
        return String::new();
    }
    String::from(value)
}

fn remove_suffix_longest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return String::from(value);
    }
    if pattern_match(pattern, value) {
        return String::new();
    }
    for (i, _) in value.char_indices() {
        if pattern_match(pattern, &value[i..]) {
            return String::from(&value[..i]);
        }
    }
    String::from(value)
}

fn remove_prefix_shortest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return String::from(value);
    }
    for i in value
        .char_indices()
        .map(|(i, _)| i)
        .chain(core::iter::once(value.len()))
        .skip(1)
    {
        if pattern_match(pattern, &value[..i]) {
            return String::from(&value[i..]);
        }
    }
    String::from(value)
}

fn remove_prefix_longest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return String::from(value);
    }
    if pattern_match(pattern, value) {
        return String::new();
    }
    for (i, _) in value.char_indices().rev() {
        let end = i + value[i..].chars().next().map_or(0, |c| c.len_utf8());
        if pattern_match(pattern, &value[..end]) {
            return String::from(&value[end..]);
        }
    }
    String::from(value)
}

fn split_at_unescaped_slash(s: &str) -> (String, String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'/' {
            return (String::from(&s[..i]), String::from(&s[i + 1..]));
        }
        i += 1;
    }
    (String::from(s), String::new())
}

fn replace_first(value: &str, pattern: &str, replacement: &str) -> String {
    // Simple glob-based replacement
    for (i, _) in value.char_indices() {
        for end in (i + 1..=value.len()).rev() {
            if pattern_match(pattern, &value[i..end]) {
                let mut result = String::from(&value[..i]);
                result.push_str(replacement);
                result.push_str(&value[end..]);
                return result;
            }
        }
    }
    String::from(value)
}

fn replace_all(value: &str, pattern: &str, replacement: &str) -> String {
    let mut result = String::from(value);
    let mut changed = true;
    let mut iterations = 0;
    while changed && iterations < 1000 {
        changed = false;
        iterations += 1;
        let new = replace_first(&result, pattern, replacement);
        if new != result {
            result = new;
            changed = true;
        }
    }
    result
}

fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let mut result = String::with_capacity(s.len());
            for uc in c.to_uppercase() {
                result.push(uc);
            }
            for c in chars {
                result.push(c);
            }
            result
        }
    }
}

fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let mut result = String::with_capacity(s.len());
            for lc in c.to_lowercase() {
                result.push(lc);
            }
            for c in chars {
                result.push(c);
            }
            result
        }
    }
}

fn substring(value: &str, spec: &str) -> Option<String> {
    let parts: alloc::vec::Vec<&str> = spec.splitn(2, ':').collect();
    let offset = parse_i64(parts[0])?;
    let vlen = value.len() as i64;

    let start = if offset < 0 {
        (vlen + offset).max(0) as usize
    } else {
        (offset as usize).min(value.len())
    };

    if parts.len() == 2 {
        let length = parse_i64(parts[1])?;
        if length < 0 {
            let end = (vlen + length).max(start as i64) as usize;
            Some(String::from(&value[start..end]))
        } else {
            let end = (start + length as usize).min(value.len());
            Some(String::from(&value[start..end]))
        }
    } else {
        Some(String::from(&value[start..]))
    }
}

fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
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

trait ToAsciiUppercase {
    fn to_ascii_uppercase(&self) -> String;
}

impl ToAsciiUppercase for String {
    fn to_ascii_uppercase(&self) -> String {
        self.chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    (c as u8 - b'a' + b'A') as char
                } else {
                    c
                }
            })
            .collect()
    }
}

trait ToAsciiLowercase {
    fn to_ascii_lowercase(&self) -> String;
}

impl ToAsciiLowercase for String {
    fn to_ascii_lowercase(&self) -> String {
        self.chars()
            .map(|c| {
                if c.is_ascii_uppercase() {
                    (c as u8 - b'A' + b'a') as char
                } else {
                    c
                }
            })
            .collect()
    }
}
