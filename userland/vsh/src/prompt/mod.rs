//! Prompt rendering for vsh.
//!
//! Expands PS1/PS2/PS3/PS4 prompt strings using Bash-compatible escape
//! sequences:
//! - `\u` -- username
//! - `\h` -- hostname (short)
//! - `\H` -- hostname (full)
//! - `\w` -- current working directory (with `~` abbreviation)
//! - `\W` -- basename of current working directory
//! - `\d` -- date in "Weekday Month Day" format
//! - `\t` -- time in HH:MM:SS (24-hour)
//! - `\T` -- time in HH:MM:SS (12-hour)
//! - `\@` -- time in HH:MM AM/PM
//! - `\n` -- newline
//! - `\r` -- carriage return
//! - `\a` -- bell
//! - `\e` -- escape (0x1B)
//! - `\$` -- `#` if uid 0, `$` otherwise
//! - `\\` -- literal backslash
//! - `\[` -- begin non-printing characters
//! - `\]` -- end non-printing characters
//! - `\nnn` -- octal character code

extern crate alloc;

use alloc::{string::String, vec::Vec};

/// Context needed for prompt expansion.
pub struct PromptContext<'a> {
    /// Current username.
    pub user: &'a str,
    /// Short hostname.
    pub hostname: &'a str,
    /// Current working directory (absolute path).
    pub cwd: &'a str,
    /// Home directory (for `~` abbreviation).
    pub home: &'a str,
    /// Whether the user is root (uid 0).
    pub is_root: bool,
    /// Shell name.
    pub shell_name: &'a str,
}

/// Expand a prompt string.
pub fn expand_prompt(ps: &str, ctx: &PromptContext) -> String {
    let chars: Vec<char> = ps.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len * 2);
    let mut i = 0;

    while i < len {
        if chars[i] == '\\' && i + 1 < len {
            i += 1;
            match chars[i] {
                'u' => {
                    result.push_str(ctx.user);
                    i += 1;
                }
                'h' => {
                    // Short hostname (up to first `.`)
                    let short = ctx.hostname.split('.').next().unwrap_or(ctx.hostname);
                    result.push_str(short);
                    i += 1;
                }
                'H' => {
                    result.push_str(ctx.hostname);
                    i += 1;
                }
                'w' => {
                    // CWD with ~ abbreviation
                    result.push_str(&abbreviate_home(ctx.cwd, ctx.home));
                    i += 1;
                }
                'W' => {
                    // Basename of CWD
                    let base = basename(ctx.cwd);
                    if base.is_empty() || base == "/" {
                        result.push('/');
                    } else {
                        result.push_str(base);
                    }
                    i += 1;
                }
                'd' => {
                    // Date -- placeholder since we lack a time syscall
                    result.push_str("Mon Jan 01");
                    i += 1;
                }
                't' => {
                    // 24-hour time placeholder
                    result.push_str("00:00:00");
                    i += 1;
                }
                'T' => {
                    // 12-hour time placeholder
                    result.push_str("12:00:00");
                    i += 1;
                }
                '@' => {
                    result.push_str("12:00 AM");
                    i += 1;
                }
                'n' => {
                    result.push('\n');
                    i += 1;
                }
                'r' => {
                    result.push('\r');
                    i += 1;
                }
                'a' => {
                    result.push('\x07');
                    i += 1;
                }
                'e' => {
                    result.push('\x1B');
                    i += 1;
                }
                '$' => {
                    if ctx.is_root {
                        result.push('#');
                    } else {
                        result.push('$');
                    }
                    i += 1;
                }
                '\\' => {
                    result.push('\\');
                    i += 1;
                }
                '[' => {
                    // Begin non-printing sequence (used for terminal width calc)
                    result.push('\x01');
                    i += 1;
                }
                ']' => {
                    // End non-printing sequence
                    result.push('\x02');
                    i += 1;
                }
                '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' => {
                    // Octal character code (\nnn)
                    let start = i;
                    let mut val: u8 = 0;
                    let mut count = 0;
                    while i < len && count < 3 && chars[i] >= '0' && chars[i] <= '7' {
                        val = val * 8 + (chars[i] as u8 - b'0');
                        i += 1;
                        count += 1;
                    }
                    if count > 0 {
                        result.push(val as char);
                    } else {
                        // Invalid octal, output literally
                        result.push('\\');
                        i = start;
                    }
                }
                'v' => {
                    result.push_str("0.1.0"); // shell version
                    i += 1;
                }
                's' => {
                    result.push_str(ctx.shell_name);
                    i += 1;
                }
                '#' => {
                    result.push('1'); // command number placeholder
                    i += 1;
                }
                '!' => {
                    result.push('1'); // history number placeholder
                    i += 1;
                }
                _ => {
                    // Unknown escape -- output literally
                    result.push('\\');
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Replace a leading $HOME prefix with `~`.
fn abbreviate_home(path: &str, home: &str) -> String {
    if !home.is_empty() && path.starts_with(home) {
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            let mut s = String::from("~");
            s.push_str(rest);
            return s;
        }
    }
    String::from(path)
}

/// Get the last component of a path.
fn basename(path: &str) -> &str {
    if path == "/" {
        return "/";
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    }
}

/// Build the default PS1 prompt: `\u@\h:\w\$ `.
pub fn default_ps1() -> String {
    String::from("\\u@\\h:\\w\\$ ")
}

/// Build the default PS2 prompt: `> `.
#[allow(dead_code)] // Used for continuation prompts
pub fn default_ps2() -> String {
    String::from("> ")
}

/// Build the default PS4 prompt: `+ `.
#[allow(dead_code)] // Used for xtrace (set -x) output
pub fn default_ps4() -> String {
    String::from("+ ")
}
