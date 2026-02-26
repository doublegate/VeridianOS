//! Variable expansion for the VeridianOS shell.
//!
//! Handles `$VAR`, `${VAR}`, `${VAR:-default}`, `${VAR:+alternate}`,
//! `${#VAR}` (string length), `${VAR%pattern}` / `${VAR%%pattern}` (suffix
//! removal), `${VAR#pattern}` / `${VAR##pattern}` (prefix removal), special
//! variables (`$?`, `$$`, `$0`), tilde expansion, quote handling,
//! backslash-dollar escaping, and command substitution (`$(command)`).

// Shell variable expansion

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

/// Expand variables, tildes, and quotes in the input string.
///
/// - `$VAR` and `${VAR}` are looked up in `env`.
/// - `${VAR:-default}` returns `default` when VAR is unset or empty.
/// - `${VAR:+alternate}` returns `alternate` when VAR is set and non-empty.
/// - `${#VAR}` returns the length of VAR's value (or `"0"` when unset).
/// - `${VAR%pattern}` removes the shortest trailing match of `pattern`.
/// - `${VAR%%pattern}` removes the longest trailing match of `pattern`.
/// - `${VAR#pattern}` removes the shortest leading match of `pattern`.
/// - `${VAR##pattern}` removes the longest leading match of `pattern`.
/// - `$?` expands to `last_exit_code`.
/// - `$$` expands to `"1"` (kernel PID).
/// - `$0` expands to `"vsh"`.
/// - `~` at the start of a word expands to the value of `HOME`.
/// - Single-quoted regions are passed through literally.
/// - Double-quoted regions expand variables but preserve whitespace.
/// - `\$` produces a literal `$`.
pub fn expand_variables(
    input: &str,
    env: &BTreeMap<String, String>,
    last_exit_code: i32,
) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // Track quoting state
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        let ch = chars[i];

        // ---- Single-quote toggle (not inside double quotes) ----
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

        // ---- Double-quote toggle ----
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            i += 1;
            continue;
        }

        // ---- Backslash-dollar escape ----
        if ch == '\\' && i + 1 < len && chars[i + 1] == '$' {
            result.push('$');
            i += 2;
            continue;
        }

        // ---- Tilde expansion (only at word start, outside quotes) ----
        if ch == '~' && !in_double_quote && (i == 0 || chars[i - 1].is_whitespace()) {
            // Check that tilde is at word boundary (next is / , whitespace, or end)
            let next_is_sep = i + 1 >= len || chars[i + 1] == '/' || chars[i + 1].is_whitespace();
            if next_is_sep {
                if let Some(home) = env.get("HOME") {
                    result.push_str(home);
                } else {
                    result.push('~');
                }
                i += 1;
                continue;
            }
        }

        // ---- Command substitution: $(...) ----
        if ch == '$' && i + 1 < len && chars[i + 1] == '(' {
            i += 2; // skip "$("
            let (expanded, consumed) =
                expand_command_substitution(&chars[i..], env, last_exit_code);
            result.push_str(&expanded);
            i += consumed;
            continue;
        }

        // ---- Variable expansion ----
        if ch == '$' && i + 1 < len {
            let next = chars[i + 1];

            // ${...} — braced expansion
            if next == '{' {
                i += 2; // skip "${"
                let (expanded, consumed) = expand_braced(&chars[i..], env);
                result.push_str(&expanded);
                i += consumed;
                continue;
            }

            // $? — last exit code
            if next == '?' {
                result.push_str(&format!("{}", last_exit_code));
                i += 2;
                continue;
            }

            // $$ — kernel PID (always 1)
            if next == '$' {
                result.push('1');
                i += 2;
                continue;
            }

            // $0 — shell name
            if next == '0' {
                result.push_str("vsh");
                i += 2;
                continue;
            }

            // $VAR — unbraced variable name
            if is_var_start(next) {
                i += 1; // skip '$'
                let start = i;
                while i < len && is_var_char(chars[i]) {
                    i += 1;
                }
                let var_name: String = chars[start..i].iter().collect();
                if let Some(val) = env.get(&var_name) {
                    result.push_str(val);
                }
                // If unset, expand to empty string (Bash behavior)
                continue;
            }

            // Bare '$' at end of string or followed by non-var char — literal
            result.push('$');
            i += 1;
            continue;
        }

        // ---- Default: pass character through ----
        result.push(ch);
        i += 1;
    }

    result
}

// ---------------------------------------------------------------------------
// Braced expansion helpers
// ---------------------------------------------------------------------------

/// Parse a `${...}` expression starting right after the `{`.
///
/// Returns `(expanded_value, chars_consumed)` where `chars_consumed` includes
/// the closing `}`.
fn expand_braced(chars: &[char], env: &BTreeMap<String, String>) -> (String, usize) {
    let len = chars.len();

    // Find closing brace
    let close = match find_closing_brace(chars) {
        Some(pos) => pos,
        None => {
            // No closing brace — return literal "${" + rest
            let literal: String = chars.iter().collect();
            return (format!("${{{}", literal), len);
        }
    };

    let inner: String = chars[..close].iter().collect();
    let consumed = close + 1; // +1 for '}'

    // ${#VAR} — string length
    if let Some(var_name) = inner.strip_prefix('#') {
        let length = env.get(var_name).map(|v| v.len()).unwrap_or(0);
        return (format!("{}", length), consumed);
    }

    // ${VAR%%pattern} — longest suffix removal (check before single %)
    if let Some(pos) = inner.find("%%") {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 2..];
        let value = env.get(var_name).cloned().unwrap_or_default();
        let trimmed = remove_suffix_longest(&value, pattern);
        return (trimmed, consumed);
    }

    // ${VAR%pattern} — shortest suffix removal
    if let Some(pos) = inner.find('%') {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 1..];
        let value = env.get(var_name).cloned().unwrap_or_default();
        let trimmed = remove_suffix_shortest(&value, pattern);
        return (trimmed, consumed);
    }

    // ${VAR##pattern} — longest prefix removal (check before single #)
    if let Some(pos) = inner.find("##") {
        let var_name = &inner[..pos];
        let pattern = &inner[pos + 2..];
        let value = env.get(var_name).cloned().unwrap_or_default();
        let trimmed = remove_prefix_longest(&value, pattern);
        return (trimmed, consumed);
    }

    // ${VAR#pattern} — shortest prefix removal
    // Must not misinterpret ${#VAR} which is handled above.
    if let Some(pos) = inner.find('#') {
        // Only treat as prefix removal if '#' is not at position 0
        // (position 0 is the ${#VAR} length syntax, already handled).
        if pos > 0 {
            let var_name = &inner[..pos];
            let pattern = &inner[pos + 1..];
            let value = env.get(var_name).cloned().unwrap_or_default();
            let trimmed = remove_prefix_shortest(&value, pattern);
            return (trimmed, consumed);
        }
    }

    // ${VAR:-default} — default value if unset or empty
    if let Some(pos) = inner.find(":-") {
        let var_name = &inner[..pos];
        let default_val = &inner[pos + 2..];
        let value = env.get(var_name).cloned().unwrap_or_default();
        if value.is_empty() {
            return (default_val.to_string(), consumed);
        }
        return (value, consumed);
    }

    // ${VAR:+alternate} — alternate value if set and non-empty
    if let Some(pos) = inner.find(":+") {
        let var_name = &inner[..pos];
        let alternate = &inner[pos + 2..];
        let value = env.get(var_name).cloned().unwrap_or_default();
        if !value.is_empty() {
            return (alternate.to_string(), consumed);
        }
        return (String::new(), consumed);
    }

    // Plain ${VAR}
    let value = env.get(inner.as_str()).cloned().unwrap_or_default();
    (value, consumed)
}

/// Find the index of the closing `}` that matches the opening `{`.
/// The input slice starts right after the opening `{`.
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

/// Returns true if `ch` can be the first character of a variable name.
fn is_var_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

/// Returns true if `ch` can appear in a variable name (after the first char).
fn is_var_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// ---------------------------------------------------------------------------
// Pattern matching helpers for suffix/prefix removal
// ---------------------------------------------------------------------------

/// Glob-like pattern match for parameter expansion.
///
/// Unlike filename glob where `*` does not match `/`, parameter expansion
/// patterns allow `*` and `?` to match ANY character (POSIX spec).
fn pattern_match(pattern: &str, text: &str) -> bool {
    let pat = pattern.as_bytes();
    let txt = text.as_bytes();
    let (plen, tlen) = (pat.len(), txt.len());
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < tlen {
        if pi < plen && (pat[pi] == b'?' || pat[pi] == txt[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < plen && pat[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < plen && pat[pi] == b'*' {
        pi += 1;
    }
    pi == plen
}

/// Remove the shortest trailing substring that matches `pattern`.
fn remove_suffix_shortest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }
    // Try suffixes from shortest to longest (scan from end toward start)
    for (i, _) in value.char_indices().rev() {
        if pattern_match(pattern, &value[i..]) {
            return value[..i].to_string();
        }
    }
    // Try the entire string as suffix
    if pattern_match(pattern, value) {
        return String::new();
    }
    value.to_string()
}

/// Remove the longest trailing substring that matches `pattern`.
fn remove_suffix_longest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }
    // Try the entire string first (longest), then shorter
    if pattern_match(pattern, value) {
        return String::new();
    }
    for (i, _) in value.char_indices() {
        if pattern_match(pattern, &value[i..]) {
            return value[..i].to_string();
        }
    }
    value.to_string()
}

/// Remove the shortest leading substring that matches `pattern`.
fn remove_prefix_shortest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }
    // Try prefixes from shortest to longest (skip empty prefix at 0)
    for i in value
        .char_indices()
        .map(|(i, _)| i)
        .chain(core::iter::once(value.len()))
        .skip(1)
    {
        if pattern_match(pattern, &value[..i]) {
            return value[i..].to_string();
        }
    }
    value.to_string()
}

/// Remove the longest leading substring that matches `pattern`.
fn remove_prefix_longest(value: &str, pattern: &str) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }
    // Try prefixes from longest to shortest
    if pattern_match(pattern, value) {
        return String::new();
    }
    for (i, _) in value.char_indices().rev() {
        let end = i + value[i..].chars().next().map_or(0, |c| c.len_utf8());
        if pattern_match(pattern, &value[..end]) {
            return value[end..].to_string();
        }
    }
    value.to_string()
}

// ---------------------------------------------------------------------------
// Command substitution helpers
// ---------------------------------------------------------------------------

/// Expand a `$(...)` command substitution starting right after the `(`.
///
/// Returns `(expanded_output, chars_consumed)` where `chars_consumed` includes
/// the closing `)`.
///
/// In kernel space without full stdout capture, we support a limited set of
/// inline commands:
/// - `$(echo ...)`: returns the echo arguments (with variable expansion)
/// - `$(cat /path)`: reads the file contents from VFS
/// - Other commands: return empty string (full stdout capture requires process
///   infrastructure that is not yet available)
///
/// Nested `$(...)` are handled recursively.
fn expand_command_substitution(
    chars: &[char],
    env: &BTreeMap<String, String>,
    last_exit_code: i32,
) -> (String, usize) {
    let len = chars.len();

    // Find closing paren, respecting nesting
    let close = match find_closing_paren(chars) {
        Some(pos) => pos,
        None => {
            // No closing paren — return literal "$(" + rest
            let literal: String = chars.iter().collect();
            return (format!("$({}", literal), len);
        }
    };

    let inner: String = chars[..close].iter().collect();
    let consumed = close + 1; // +1 for ')'

    // Recursively expand any nested $() in the inner command first
    let expanded_inner = expand_variables(&inner, env, last_exit_code);

    // Parse the command
    let trimmed = expanded_inner.trim();
    let output = execute_substitution_command(trimmed);

    (output, consumed)
}

/// Find the index of the closing `)` that matches the opening `(`.
/// The input slice starts right after the opening `(`.
fn find_closing_paren(chars: &[char]) -> Option<usize> {
    let mut depth = 1u32;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                // Check for nested $( by looking at preceding char
                depth += 1;
            }
            ')' => {
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

/// Execute a simple command for substitution and return its output.
///
/// Supports:
/// - `echo [args...]` — returns the arguments joined by spaces
/// - `cat <path>` — reads file contents from VFS
/// - `pwd` — returns current working directory
/// - Other commands — returns empty string
fn execute_substitution_command(command: &str) -> String {
    if command.is_empty() {
        return String::new();
    }

    // Split into words (simple whitespace split)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return String::new();
    }

    match parts[0] {
        "echo" => {
            // Return the remaining args joined by spaces, like echo does
            if parts.len() > 1 {
                parts[1..].join(" ")
            } else {
                String::new()
            }
        }
        "cat" => {
            // Read file contents from VFS
            if parts.len() > 1 {
                match crate::fs::read_file(parts[1]) {
                    Ok(data) => {
                        // Convert bytes to string, trimming trailing newline
                        let s = String::from_utf8_lossy(&data).into_owned();
                        // Trim trailing newline like shell command substitution does
                        s.trim_end_matches('\n').to_string()
                    }
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        }
        "pwd" => {
            // Try to get CWD from shell state
            super::try_get_shell()
                .map(|shell| shell.get_cwd())
                .unwrap_or_else(|| String::from("/"))
        }
        // TODO(phase6): Full stdout capture requires process pipe infrastructure
        // (fork + exec + pipe fd redirection).  Other commands return empty for now.
        _ => String::new(),
    }
}

/// Expand all `$(...)` command substitutions in the input string.
///
/// This is the public entry point for command substitution. It delegates
/// to `expand_variables()` which handles `$(...)` as part of its expansion
/// pass.
#[allow(dead_code)] // Public API, not yet called externally
pub fn expand_command_substitutions(
    input: &str,
    env: &BTreeMap<String, String>,
    last_exit_code: i32,
) -> String {
    // expand_variables already handles $() inline, so just delegate
    expand_variables(input, env, last_exit_code)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for &(k, v) in pairs {
            map.insert(k.to_string(), v.to_string());
        }
        map
    }

    // ---- Basic variable expansion ----

    #[test]
    fn test_simple_var() {
        let env = env_with(&[("FOO", "bar")]);
        assert_eq!(expand_variables("$FOO", &env, 0), "bar");
    }

    #[test]
    fn test_braced_var() {
        let env = env_with(&[("FOO", "bar")]);
        assert_eq!(expand_variables("${FOO}", &env, 0), "bar");
    }

    #[test]
    fn test_unset_var_empty() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$MISSING", &env, 0), "");
    }

    #[test]
    fn test_var_in_text() {
        let env = env_with(&[("USER", "root")]);
        assert_eq!(expand_variables("hello $USER!", &env, 0), "hello root!");
    }

    #[test]
    fn test_braced_var_adjacent_text() {
        let env = env_with(&[("X", "abc")]);
        assert_eq!(expand_variables("${X}def", &env, 0), "abcdef");
    }

    // ---- Default and alternate values ----

    #[test]
    fn test_default_when_unset() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("${VAR:-fallback}", &env, 0), "fallback");
    }

    #[test]
    fn test_default_when_empty() {
        let env = env_with(&[("VAR", "")]);
        assert_eq!(expand_variables("${VAR:-fallback}", &env, 0), "fallback");
    }

    #[test]
    fn test_default_when_set() {
        let env = env_with(&[("VAR", "hello")]);
        assert_eq!(expand_variables("${VAR:-fallback}", &env, 0), "hello");
    }

    #[test]
    fn test_alternate_when_set() {
        let env = env_with(&[("VAR", "anything")]);
        assert_eq!(expand_variables("${VAR:+yes}", &env, 0), "yes");
    }

    #[test]
    fn test_alternate_when_unset() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("${VAR:+yes}", &env, 0), "");
    }

    // ---- String length ----

    #[test]
    fn test_string_length() {
        let env = env_with(&[("NAME", "hello")]);
        assert_eq!(expand_variables("${#NAME}", &env, 0), "5");
    }

    #[test]
    fn test_string_length_unset() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("${#MISSING}", &env, 0), "0");
    }

    // ---- Suffix removal ----

    #[test]
    fn test_suffix_shortest() {
        let env = env_with(&[("FILE", "archive.tar.gz")]);
        assert_eq!(expand_variables("${FILE%.*}", &env, 0), "archive.tar");
    }

    #[test]
    fn test_suffix_longest() {
        let env = env_with(&[("FILE", "archive.tar.gz")]);
        assert_eq!(expand_variables("${FILE%%.*}", &env, 0), "archive");
    }

    // ---- Prefix removal ----

    #[test]
    fn test_prefix_shortest() {
        let env = env_with(&[("PATH", "/usr/local/bin")]);
        assert_eq!(expand_variables("${PATH#/*/}", &env, 0), "local/bin");
    }

    #[test]
    fn test_prefix_longest() {
        let env = env_with(&[("PATH", "/usr/local/bin")]);
        assert_eq!(expand_variables("${PATH##/*/}", &env, 0), "bin");
    }

    // ---- Special variables ----

    #[test]
    fn test_exit_code() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$?", &env, 42), "42");
    }

    #[test]
    fn test_pid() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$$", &env, 0), "1");
    }

    #[test]
    fn test_shell_name() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$0", &env, 0), "vsh");
    }

    // ---- Tilde expansion ----

    #[test]
    fn test_tilde_expansion() {
        let env = env_with(&[("HOME", "/root")]);
        assert_eq!(expand_variables("~/docs", &env, 0), "/root/docs");
    }

    #[test]
    fn test_tilde_alone() {
        let env = env_with(&[("HOME", "/home/user")]);
        assert_eq!(expand_variables("~", &env, 0), "/home/user");
    }

    #[test]
    fn test_tilde_no_home() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("~", &env, 0), "~");
    }

    // ---- Quoting ----

    #[test]
    fn test_single_quotes_no_expansion() {
        let env = env_with(&[("FOO", "bar")]);
        assert_eq!(expand_variables("'$FOO'", &env, 0), "$FOO");
    }

    #[test]
    fn test_double_quotes_expand() {
        let env = env_with(&[("FOO", "bar")]);
        assert_eq!(expand_variables("\"$FOO\"", &env, 0), "bar");
    }

    // ---- Escape ----

    #[test]
    fn test_escaped_dollar() {
        let env = env_with(&[("FOO", "bar")]);
        assert_eq!(expand_variables("\\$FOO", &env, 0), "$FOO");
    }

    // ---- Mixed ----

    #[test]
    fn test_multiple_vars() {
        let env = env_with(&[("A", "hello"), ("B", "world")]);
        assert_eq!(expand_variables("$A $B", &env, 0), "hello world");
    }

    #[test]
    fn test_empty_input() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("", &env, 0), "");
    }

    #[test]
    fn test_no_vars() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("plain text", &env, 0), "plain text");
    }

    #[test]
    fn test_bare_dollar_at_end() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("cost is $", &env, 0), "cost is $");
    }

    // ---- Command substitution ----

    #[test]
    fn test_command_sub_echo() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(echo hello)", &env, 0), "hello");
    }

    #[test]
    fn test_command_sub_echo_multiple_words() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("$(echo hello world)", &env, 0),
            "hello world"
        );
    }

    #[test]
    fn test_command_sub_echo_with_var() {
        let env = env_with(&[("NAME", "veridian")]);
        assert_eq!(expand_variables("$(echo $NAME)", &env, 0), "veridian");
    }

    #[test]
    fn test_command_sub_in_text() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("hello $(echo world)!", &env, 0),
            "hello world!"
        );
    }

    #[test]
    fn test_command_sub_unknown_cmd() {
        let env = BTreeMap::new();
        // Unknown commands return empty string
        assert_eq!(expand_variables("$(unknown_cmd arg)", &env, 0), "");
    }

    #[test]
    fn test_command_sub_empty() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$()", &env, 0), "");
    }

    #[test]
    fn test_command_sub_unclosed() {
        let env = BTreeMap::new();
        // Unclosed $( returns literal
        assert_eq!(expand_variables("$(echo hello", &env, 0), "$(echo hello");
    }

    #[test]
    fn test_command_sub_nested() {
        let env = BTreeMap::new();
        // Nested: $(echo $(echo inner))
        assert_eq!(expand_variables("$(echo $(echo inner))", &env, 0), "inner");
    }
}
