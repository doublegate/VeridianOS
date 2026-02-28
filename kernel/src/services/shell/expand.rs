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

/// Execute a command for substitution and return its captured output.
///
/// Supports inline evaluation for common shell builtins and utilities:
/// - `echo [args...]` -- returns the arguments joined by spaces
/// - `cat <path>` -- reads file contents from VFS
/// - `pwd` -- returns current working directory
/// - `uname [-s|-n|-r|-m|-a]` -- returns system information
/// - `whoami` -- returns current user name
/// - `hostname` -- returns system hostname
/// - `basename <path> [suffix]` -- strips directory and optional suffix
/// - `dirname <path>` -- strips last path component
/// - `printf <format> [args...]` -- formatted output
/// - `true` / `false` -- return empty string (side-effect only)
/// - `seq <first> [increment] <last>` -- number sequence
/// - `wc [-l|-w|-c] <file>` -- word/line/byte count
/// - `head [-n N] <file>` -- first N lines of file
/// - `tail [-n N] <file>` -- last N lines of file
/// - `date` -- current date string
/// - `test`/`[` -- exit status only, returns empty
/// - `expr <args>` -- simple integer arithmetic
///
/// For unrecognized commands, returns an empty string (full pipe-based
/// stdout capture would require fork+exec+pipe infrastructure).
fn execute_substitution_command(command: &str) -> String {
    if command.is_empty() {
        return String::new();
    }

    // Split into words (simple whitespace split)
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return String::new();
    }

    let output = match parts[0] {
        "echo" => {
            // echo: return remaining args joined by spaces
            if parts.len() > 1 {
                // Handle -n (no trailing newline) and -e (escape sequences)
                // For command substitution, trailing newlines are stripped anyway
                let mut start = 1;
                while start < parts.len() && parts[start].starts_with('-') {
                    start += 1;
                }
                if start < parts.len() {
                    parts[start..].join(" ")
                } else if start > 1 {
                    // Only flags, no text
                    String::new()
                } else {
                    parts[1..].join(" ")
                }
            } else {
                String::new()
            }
        }

        "cat" => {
            // cat: read file contents from VFS
            if parts.len() > 1 {
                let mut result = String::new();
                for &path in &parts[1..] {
                    if path.starts_with('-') {
                        continue; // Skip flags
                    }
                    if let Ok(data) = crate::fs::read_file(path) {
                        let s = String::from_utf8_lossy(&data).into_owned();
                        result.push_str(&s);
                    }
                }
                result
            } else {
                String::new()
            }
        }

        "pwd" => {
            // pwd: current working directory
            super::try_get_shell()
                .map(|shell| shell.get_cwd())
                .unwrap_or_else(|| String::from("/"))
        }

        "uname" => {
            // uname: system information
            subst_uname(&parts[1..])
        }

        "whoami" => {
            // whoami: current user
            super::try_get_shell()
                .and_then(|shell| shell.get_env("USER"))
                .unwrap_or_else(|| String::from("root"))
        }

        "hostname" => {
            // hostname: system hostname
            String::from("veridian")
        }

        "basename" => {
            // basename: strip directory (and optional suffix)
            if parts.len() > 1 {
                subst_basename(parts[1], parts.get(2).copied())
            } else {
                String::new()
            }
        }

        "dirname" => {
            // dirname: strip last component
            if parts.len() > 1 {
                subst_dirname(parts[1])
            } else {
                String::from(".")
            }
        }

        "printf" => {
            // printf: simple format string expansion
            if parts.len() > 1 {
                subst_printf(&parts[1..])
            } else {
                String::new()
            }
        }

        "true" | "false" => {
            // Side-effect-only commands; return empty for substitution
            String::new()
        }

        "seq" => {
            // seq: number sequence
            subst_seq(&parts[1..])
        }

        "wc" => {
            // wc: word/line/byte count
            subst_wc(&parts[1..])
        }

        "head" => {
            // head: first N lines
            subst_head(&parts[1..])
        }

        "tail" => {
            // tail: last N lines
            subst_tail(&parts[1..])
        }

        "tr" => {
            // tr: character translation requires stdin; not feasible in
            // substitution context without pipe infrastructure
            String::new()
        }

        "date" => {
            // date: current timestamp
            subst_date()
        }

        "test" | "[" => {
            // test/[: condition evaluation produces exit status only
            String::new()
        }

        "expr" => {
            // expr: simple integer arithmetic
            subst_expr(&parts[1..])
        }

        _ => {
            // Unrecognized command -- full pipe-based stdout capture would
            // require fork+exec+pipe infrastructure; return empty for now.
            String::new()
        }
    };

    // Strip trailing newlines (standard command substitution behavior)
    output.trim_end_matches('\n').to_string()
}

// ---------------------------------------------------------------------------
// Command substitution inline evaluators
// ---------------------------------------------------------------------------

/// Evaluate `uname` with optional flags.
fn subst_uname(args: &[&str]) -> String {
    let show_all = args.contains(&"-a");
    let show_sysname = args.is_empty() || show_all || args.contains(&"-s");
    let show_nodename = show_all || args.contains(&"-n");
    let show_release = show_all || args.contains(&"-r");
    let show_machine = show_all || args.contains(&"-m");

    let mut parts_out: Vec<&str> = Vec::new();
    if show_sysname {
        parts_out.push("VeridianOS");
    }
    if show_nodename {
        parts_out.push("veridian");
    }
    if show_release {
        parts_out.push("0.7.1");
    }
    if show_machine {
        #[cfg(target_arch = "x86_64")]
        parts_out.push("x86_64");
        #[cfg(target_arch = "aarch64")]
        parts_out.push("aarch64");
        #[cfg(target_arch = "riscv64")]
        parts_out.push("riscv64");
    }

    parts_out.join(" ")
}

/// Evaluate `basename <path> [suffix]`.
fn subst_basename(path: &str, suffix: Option<&str>) -> String {
    // Find the last non-trailing-slash component
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::from("/");
    }

    let base = match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    };

    // Strip optional suffix
    if let Some(sfx) = suffix {
        if !sfx.is_empty() && base.len() > sfx.len() && base.ends_with(sfx) {
            return base[..base.len() - sfx.len()].to_string();
        }
    }

    base.to_string()
}

/// Evaluate `dirname <path>`.
fn subst_dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::from("/");
    }

    match trimmed.rfind('/') {
        Some(0) => String::from("/"),
        Some(pos) => trimmed[..pos].to_string(),
        None => String::from("."),
    }
}

/// Simple `printf` format string expansion.
///
/// Supports `%s` (string), `%d` (integer), `%%` (literal %), and `\n`, `\t`
/// escape sequences. This is a simplified version that handles the most common
/// use cases in shell scripts.
fn subst_printf(args: &[&str]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let fmt = args[0];
    let mut result = String::new();
    let mut arg_idx = 1usize;
    let fmt_bytes = fmt.as_bytes();
    let mut i = 0;

    while i < fmt_bytes.len() {
        if fmt_bytes[i] == b'%' && i + 1 < fmt_bytes.len() {
            match fmt_bytes[i + 1] {
                b's' => {
                    if arg_idx < args.len() {
                        result.push_str(args[arg_idx]);
                        arg_idx += 1;
                    }
                    i += 2;
                }
                b'd' => {
                    if arg_idx < args.len() {
                        result.push_str(args[arg_idx]);
                        arg_idx += 1;
                    }
                    i += 2;
                }
                b'%' => {
                    result.push('%');
                    i += 2;
                }
                _ => {
                    result.push('%');
                    i += 1;
                }
            }
        } else if fmt_bytes[i] == b'\\' && i + 1 < fmt_bytes.len() {
            match fmt_bytes[i + 1] {
                b'n' => {
                    result.push('\n');
                    i += 2;
                }
                b't' => {
                    result.push('\t');
                    i += 2;
                }
                b'\\' => {
                    result.push('\\');
                    i += 2;
                }
                _ => {
                    result.push('\\');
                    i += 1;
                }
            }
        } else {
            result.push(fmt_bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Evaluate `seq [first [increment]] last`.
fn subst_seq(args: &[&str]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let (first, increment, last) = match args.len() {
        1 => (1i64, 1i64, parse_i64(args[0]).unwrap_or(1)),
        2 => {
            let f = parse_i64(args[0]).unwrap_or(1);
            let l = parse_i64(args[1]).unwrap_or(1);
            (f, if f <= l { 1 } else { -1 }, l)
        }
        _ => {
            let f = parse_i64(args[0]).unwrap_or(1);
            let inc = parse_i64(args[1]).unwrap_or(1);
            let l = parse_i64(args[2]).unwrap_or(1);
            (f, inc, l)
        }
    };

    if increment == 0 {
        return String::new();
    }

    let mut result = String::new();
    let mut current = first;
    let mut first_line = true;

    // Safety limit to prevent infinite loops
    let max_iterations = 10000u32;
    let mut count = 0u32;

    loop {
        if increment > 0 && current > last {
            break;
        }
        if increment < 0 && current < last {
            break;
        }
        if count >= max_iterations {
            break;
        }

        if !first_line {
            result.push('\n');
        }
        // Format the number
        let num_str = format_i64(current);
        result.push_str(&num_str);

        first_line = false;
        current += increment;
        count += 1;
    }

    result
}

/// Evaluate `wc [-l|-w|-c] <file>`.
fn subst_wc(args: &[&str]) -> String {
    let mut count_lines = false;
    let mut count_words = false;
    let mut count_bytes = false;
    let mut file_path = None;

    for &arg in args {
        match arg {
            "-l" => count_lines = true,
            "-w" => count_words = true,
            "-c" | "-m" => count_bytes = true,
            _ if !arg.starts_with('-') => file_path = Some(arg),
            _ => {}
        }
    }

    // Default: show all three if no specific flag
    if !count_lines && !count_words && !count_bytes {
        count_lines = true;
        count_words = true;
        count_bytes = true;
    }

    let path = match file_path {
        Some(p) => p,
        None => return String::from("0"),
    };

    let data = match crate::fs::read_file(path) {
        Ok(d) => d,
        Err(_) => return String::from("0"),
    };

    let content = String::from_utf8_lossy(&data);
    let mut parts_out: Vec<String> = Vec::new();

    if count_lines {
        let lines = content.as_bytes().iter().filter(|&&b| b == b'\n').count();
        parts_out.push(format!("{}", lines));
    }
    if count_words {
        let words = content.split_whitespace().count();
        parts_out.push(format!("{}", words));
    }
    if count_bytes {
        parts_out.push(format!("{}", data.len()));
    }

    parts_out.join(" ")
}

/// Evaluate `head [-n N] <file>`.
fn subst_head(args: &[&str]) -> String {
    let mut num_lines = 10usize;
    let mut file_path = None;
    let mut i = 0;

    while i < args.len() {
        if args[i] == "-n" && i + 1 < args.len() {
            num_lines = parse_i64(args[i + 1]).unwrap_or(10) as usize;
            i += 2;
        } else if !args[i].starts_with('-') {
            file_path = Some(args[i]);
            i += 1;
        } else {
            i += 1;
        }
    }

    let path = match file_path {
        Some(p) => p,
        None => return String::new(),
    };

    let data = match crate::fs::read_file(path) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let content = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = content.lines().take(num_lines).collect();
    lines.join("\n")
}

/// Evaluate `tail [-n N] <file>`.
fn subst_tail(args: &[&str]) -> String {
    let mut num_lines = 10usize;
    let mut file_path = None;
    let mut i = 0;

    while i < args.len() {
        if args[i] == "-n" && i + 1 < args.len() {
            num_lines = parse_i64(args[i + 1]).unwrap_or(10) as usize;
            i += 2;
        } else if !args[i].starts_with('-') {
            file_path = Some(args[i]);
            i += 1;
        } else {
            i += 1;
        }
    }

    let path = match file_path {
        Some(p) => p,
        None => return String::new(),
    };

    let data = match crate::fs::read_file(path) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    let content = String::from_utf8_lossy(&data);
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(num_lines);
    all_lines[start..].join("\n")
}

/// Evaluate `date` (simplified UTC timestamp).
fn subst_date() -> String {
    let total_secs = crate::arch::timer::get_timestamp_secs();

    let secs_per_day: u64 = 86400;
    let mut days = total_secs / secs_per_day;
    let day_secs = total_secs % secs_per_day;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    let mut year: u64 = 1970;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let month_days: [u64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month: u64 = 1;
    for &mdays in &month_days {
        if days < mdays {
            break;
        }
        days -= mdays;
        month += 1;
    }

    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hours, minutes, seconds
    )
}

/// Check if a year is a leap year (for date calculation).
fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Evaluate `expr` -- simple integer arithmetic.
///
/// Supports: `expr <a> + <b>`, `expr <a> - <b>`, `expr <a> * <b>`,
/// `expr <a> / <b>`, `expr <a> % <b>`.
fn subst_expr(args: &[&str]) -> String {
    if args.len() == 3 {
        let a = parse_i64(args[0]);
        let b = parse_i64(args[2]);
        if let (Some(a), Some(b)) = (a, b) {
            let result = match args[1] {
                "+" => Some(a + b),
                "-" => Some(a - b),
                "*" => Some(a * b),
                "/" => {
                    if b != 0 {
                        Some(a / b)
                    } else {
                        None
                    }
                }
                "%" => {
                    if b != 0 {
                        Some(a % b)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(r) = result {
                return format_i64(r);
            }
        }
    }

    // Single argument: return it as-is (expr identity)
    if args.len() == 1 {
        return args[0].to_string();
    }

    String::from("0")
}

/// Parse a decimal integer from a string.
fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (negative, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    };

    if digits.is_empty() {
        return None;
    }

    let mut result: i64 = 0;
    for &b in digits.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?;
        result = result.checked_add((b - b'0') as i64)?;
    }

    if negative {
        Some(-result)
    } else {
        Some(result)
    }
}

/// Format an i64 as a decimal string.
fn format_i64(val: i64) -> String {
    format!("{}", val)
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

    // ---- Extended command substitution ----

    #[test]
    fn test_command_sub_hostname() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(hostname)", &env, 0), "veridian");
    }

    #[test]
    fn test_command_sub_uname_default() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(uname)", &env, 0), "VeridianOS");
    }

    #[test]
    fn test_command_sub_uname_release() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(uname -r)", &env, 0), "0.7.1");
    }

    #[test]
    fn test_command_sub_basename() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("$(basename /usr/local/bin/gcc)", &env, 0),
            "gcc"
        );
    }

    #[test]
    fn test_command_sub_basename_with_suffix() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("$(basename archive.tar.gz .tar.gz)", &env, 0),
            "archive"
        );
    }

    #[test]
    fn test_command_sub_dirname() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("$(dirname /usr/local/bin/gcc)", &env, 0),
            "/usr/local/bin"
        );
    }

    #[test]
    fn test_command_sub_dirname_root() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(dirname /file)", &env, 0), "/");
    }

    #[test]
    fn test_command_sub_dirname_no_slash() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(dirname file.txt)", &env, 0), ".");
    }

    #[test]
    fn test_command_sub_expr_add() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(expr 3 + 4)", &env, 0), "7");
    }

    #[test]
    fn test_command_sub_expr_subtract() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(expr 10 - 3)", &env, 0), "7");
    }

    #[test]
    fn test_command_sub_expr_multiply() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(expr 6 * 7)", &env, 0), "42");
    }

    #[test]
    fn test_command_sub_seq() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(seq 3)", &env, 0), "1\n2\n3");
    }

    #[test]
    fn test_command_sub_seq_range() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(seq 2 5)", &env, 0), "2\n3\n4\n5");
    }

    #[test]
    fn test_command_sub_printf_simple() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(printf hello)", &env, 0), "hello");
    }

    #[test]
    fn test_command_sub_printf_format() {
        let env = BTreeMap::new();
        assert_eq!(
            expand_variables("$(printf %s-%s foo bar)", &env, 0),
            "foo-bar"
        );
    }

    #[test]
    fn test_command_sub_true_false() {
        let env = BTreeMap::new();
        assert_eq!(expand_variables("$(true)", &env, 0), "");
        assert_eq!(expand_variables("$(false)", &env, 0), "");
    }

    // ---- Inline helper unit tests ----

    #[test]
    fn test_parse_i64() {
        assert_eq!(parse_i64("42"), Some(42));
        assert_eq!(parse_i64("-7"), Some(-7));
        assert_eq!(parse_i64("+10"), Some(10));
        assert_eq!(parse_i64("0"), Some(0));
        assert_eq!(parse_i64(""), None);
        assert_eq!(parse_i64("abc"), None);
    }

    #[test]
    fn test_subst_basename_cases() {
        assert_eq!(subst_basename("/usr/bin/gcc", None), "gcc");
        assert_eq!(subst_basename("/", None), "/");
        assert_eq!(subst_basename("file.c", Some(".c")), "file");
        assert_eq!(subst_basename("a/b/c.rs", Some(".rs")), "c");
        assert_eq!(subst_basename("/trailing/", None), "trailing");
    }

    #[test]
    fn test_subst_dirname_cases() {
        assert_eq!(subst_dirname("/usr/bin/gcc"), "/usr/bin");
        assert_eq!(subst_dirname("/file"), "/");
        assert_eq!(subst_dirname("plain"), ".");
        assert_eq!(subst_dirname("/"), "/");
    }

    #[test]
    fn test_subst_expr_cases() {
        assert_eq!(subst_expr(&["5", "+", "3"]), "8");
        assert_eq!(subst_expr(&["10", "-", "4"]), "6");
        assert_eq!(subst_expr(&["6", "*", "7"]), "42");
        assert_eq!(subst_expr(&["15", "/", "3"]), "5");
        assert_eq!(subst_expr(&["17", "%", "5"]), "2");
        assert_eq!(subst_expr(&["10", "/", "0"]), "0"); // division by zero
    }

    #[test]
    fn test_subst_seq_cases() {
        assert_eq!(subst_seq(&["3"]), "1\n2\n3");
        assert_eq!(subst_seq(&["2", "4"]), "2\n3\n4");
        assert_eq!(subst_seq(&["1", "2", "5"]), "1\n3\n5");
    }

    #[test]
    fn test_subst_printf_cases() {
        assert_eq!(subst_printf(&["hello"]), "hello");
        assert_eq!(subst_printf(&["%s", "world"]), "world");
        assert_eq!(subst_printf(&["%%"]), "%");
        assert_eq!(subst_printf(&["a\\nb"]), "a\nb");
    }
}
