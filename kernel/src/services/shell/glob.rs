//! Glob pattern matching and expansion for the VeridianOS shell.
//!
//! Provides `glob_match` for testing whether a pattern matches a given
//! string, and `expand_globs` for expanding glob tokens against the VFS
//! directory tree.
//!
//! Supported patterns:
//! - `*` matches any sequence of characters (except `/`)
//! - `?` matches exactly one character (except `/`)
//! - `[abc]` matches any character in the set
//! - `[a-z]` matches a character range
//! - `[!abc]` / `[^abc]` negated character class

#![allow(dead_code)]

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// Expand glob tokens against the VFS.
///
/// For each token that contains `*`, `?`, or `[`, attempt to list matching
/// entries from the directory implied by the token's path prefix. If no
/// matches are found, the original token is returned unchanged (standard
/// Bash behaviour).
///
/// `cwd` is the shell's current working directory, used to resolve relative
/// glob patterns.
pub fn expand_globs(tokens: Vec<String>, cwd: &str) -> Vec<String> {
    let mut result = Vec::new();

    for token in &tokens {
        if !contains_glob_chars(token) {
            result.push(token.clone());
            continue;
        }

        // Split the token into directory prefix and the glob pattern part.
        // e.g. "/tmp/*.txt" -> dir="/tmp", pattern="*.txt"
        // e.g. "*.rs"      -> dir=cwd,    pattern="*.rs"
        let (dir, pattern) = split_dir_pattern(token, cwd);

        match list_directory(&dir) {
            Some(entries) => {
                let mut matches: Vec<String> = entries
                    .iter()
                    .filter(|name| glob_match(pattern.as_str(), name))
                    .map(|name| {
                        if dir == "/" {
                            alloc::format!("/{}", name)
                        } else if dir.ends_with('/') {
                            alloc::format!("{}{}", dir, name)
                        } else {
                            alloc::format!("{}/{}", dir, name)
                        }
                    })
                    .collect();

                if matches.is_empty() {
                    // No matches — return pattern unchanged
                    result.push(token.clone());
                } else {
                    // Sort alphabetically
                    matches.sort();
                    result.extend(matches);
                }
            }
            None => {
                // Directory not accessible — return pattern unchanged
                result.push(token.clone());
            }
        }
    }

    result
}

/// Test whether `pattern` matches `text`.
///
/// The pattern language supports:
/// - `*` — matches zero or more characters (not `/`)
/// - `?` — matches exactly one character (not `/`)
/// - `[abc]` — matches any one of the listed characters
/// - `[a-z]` — matches a character in the range
/// - `[!...]` or `[^...]` — negated character class
/// - Any other character matches itself literally.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_recursive(&pat, 0, &txt, 0)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Recursive glob matcher with backtracking for `*`.
fn glob_match_recursive(pat: &[char], pi: usize, txt: &[char], ti: usize) -> bool {
    let plen = pat.len();
    let tlen = txt.len();
    let mut pi = pi;
    let mut ti = ti;

    // Track the last `*` position for backtracking
    let mut star_pi: Option<usize> = None;
    let mut star_ti: usize = 0;

    while ti < tlen {
        if pi < plen && pat[pi] == '?' && txt[ti] != '/' {
            // '?' matches any single non-slash character
            pi += 1;
            ti += 1;
        } else if pi < plen && pat[pi] == '*' {
            // '*' matches zero or more non-slash characters
            // Record position for backtracking
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
            // Try matching zero characters first (advance pattern only)
        } else if pi < plen && pat[pi] == '[' {
            // Character class
            let (matched, end) = match_char_class(&pat[pi..], txt[ti]);
            if matched {
                pi += end;
                ti += 1;
            } else if let Some(sp) = star_pi {
                // Backtrack: let '*' consume one more character, but not '/'
                if txt[star_ti] == '/' {
                    return false;
                }
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        } else if pi < plen && pat[pi] == txt[ti] {
            // Exact character match
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_pi {
            // Mismatch — backtrack to last '*', but '*' must not consume '/'
            if txt[star_ti] == '/' {
                return false;
            }
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    // Consume any remaining '*' in pattern
    while pi < plen && pat[pi] == '*' {
        pi += 1;
    }

    pi == plen
}

/// Match a character class `[...]` starting at `chars[0]` which must be `[`.
///
/// Returns `(matched, consumed)` where `consumed` is the number of chars in
/// the pattern consumed by the class (including the closing `]`).
fn match_char_class(chars: &[char], ch: char) -> (bool, usize) {
    let len = chars.len();
    if len < 2 || chars[0] != '[' {
        return (false, 0);
    }

    let mut i = 1;
    let negated = if i < len && (chars[i] == '!' || chars[i] == '^') {
        i += 1;
        true
    } else {
        false
    };

    let mut matched = false;

    // A leading ']' right after '[' or '[!' is treated as a literal ']'
    if i < len && chars[i] == ']' {
        if ch == ']' {
            matched = true;
        }
        i += 1;
    }

    while i < len && chars[i] != ']' {
        // Check for range: a-z
        if i + 2 < len && chars[i + 1] == '-' && chars[i + 2] != ']' {
            let lo = chars[i];
            let hi = chars[i + 2];
            if ch >= lo && ch <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if chars[i] == ch {
                matched = true;
            }
            i += 1;
        }
    }

    // Skip closing ']'
    if i < len && chars[i] == ']' {
        i += 1;
    } else {
        // No closing bracket — treat the whole thing as literal (no match)
        return (false, 0);
    }

    let result = if negated { !matched } else { matched };
    (result, i)
}

/// Return true if the string contains any unescaped glob metacharacters.
fn contains_glob_chars(s: &str) -> bool {
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '*' || ch == '?' || ch == '[' {
            return true;
        }
    }
    false
}

/// Split a glob token into (directory, pattern).
///
/// For example:
/// - `/tmp/*.txt` -> (`/tmp`, `*.txt`)
/// - `*.rs`       -> (cwd, `*.rs`)
/// - `/etc/conf.d/[a-z]*` -> (`/etc/conf.d`, `[a-z]*`)
fn split_dir_pattern(token: &str, cwd: &str) -> (String, String) {
    // Find the last '/' before any glob character
    let first_glob = token
        .char_indices()
        .find(|&(_, ch)| ch == '*' || ch == '?' || ch == '[')
        .map(|(i, _)| i)
        .unwrap_or(token.len());

    let prefix = &token[..first_glob];
    if let Some(slash_pos) = prefix.rfind('/') {
        let dir = if slash_pos == 0 {
            String::from("/")
        } else {
            token[..slash_pos].to_string()
        };
        let pattern = token[slash_pos + 1..].to_string();
        (dir, pattern)
    } else {
        // No directory separator before the glob — use cwd
        (cwd.to_string(), token.to_string())
    }
}

/// List directory entries from the VFS.
///
/// Returns `Some(names)` on success, `None` if the directory cannot be read.
fn list_directory(path: &str) -> Option<Vec<String>> {
    let vfs = crate::fs::try_get_vfs()?;
    let vfs_guard = vfs.read();
    let node = vfs_guard.resolve_path(path).ok()?;
    let entries = node.readdir().ok()?;

    let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    Some(names)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- glob_match: basic ----

    #[test]
    fn test_exact_match() {
        assert!(glob_match("hello", "hello"));
    }

    #[test]
    fn test_exact_mismatch() {
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn test_empty_pattern_empty_text() {
        assert!(glob_match("", ""));
    }

    #[test]
    fn test_empty_pattern_nonempty_text() {
        assert!(!glob_match("", "a"));
    }

    // ---- glob_match: * wildcard ----

    #[test]
    fn test_star_matches_everything() {
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn test_star_matches_empty() {
        assert!(glob_match("*", ""));
    }

    #[test]
    fn test_star_suffix() {
        assert!(glob_match("*.txt", "readme.txt"));
        assert!(!glob_match("*.txt", "readme.md"));
    }

    #[test]
    fn test_star_prefix() {
        assert!(glob_match("test*", "testing"));
        assert!(!glob_match("test*", "best"));
    }

    #[test]
    fn test_star_middle() {
        assert!(glob_match("a*c", "abc"));
        assert!(glob_match("a*c", "aXYZc"));
        assert!(!glob_match("a*c", "aXYZd"));
    }

    #[test]
    fn test_multiple_stars() {
        assert!(glob_match("*.*", "file.txt"));
        assert!(glob_match("*.*", "a.b"));
        assert!(!glob_match("*.*", "noext"));
    }

    #[test]
    fn test_star_does_not_match_slash() {
        assert!(!glob_match("*", "a/b"));
    }

    // ---- glob_match: ? wildcard ----

    #[test]
    fn test_question_mark_single_char() {
        assert!(glob_match("?", "a"));
        assert!(!glob_match("?", ""));
        assert!(!glob_match("?", "ab"));
    }

    #[test]
    fn test_question_in_pattern() {
        assert!(glob_match("f?o", "foo"));
        assert!(glob_match("f?o", "fXo"));
        assert!(!glob_match("f?o", "fo"));
    }

    // ---- glob_match: character classes ----

    #[test]
    fn test_char_class_basic() {
        assert!(glob_match("[abc]", "a"));
        assert!(glob_match("[abc]", "b"));
        assert!(!glob_match("[abc]", "d"));
    }

    #[test]
    fn test_char_class_range() {
        assert!(glob_match("[a-z]", "m"));
        assert!(!glob_match("[a-z]", "A"));
        assert!(!glob_match("[a-z]", "5"));
    }

    #[test]
    fn test_char_class_negated() {
        assert!(!glob_match("[!abc]", "a"));
        assert!(glob_match("[!abc]", "d"));
    }

    #[test]
    fn test_char_class_caret_negated() {
        assert!(!glob_match("[^abc]", "b"));
        assert!(glob_match("[^abc]", "z"));
    }

    #[test]
    fn test_char_class_in_pattern() {
        assert!(glob_match("[a-z]*.txt", "readme.txt"));
        assert!(!glob_match("[a-z]*.txt", "README.txt"));
    }

    // ---- glob_match: combined ----

    #[test]
    fn test_complex_pattern() {
        assert!(glob_match("*.tar.gz", "archive.tar.gz"));
        assert!(!glob_match("*.tar.gz", "archive.tar.bz2"));
    }

    #[test]
    fn test_question_and_star() {
        assert!(glob_match("?est*", "testing"));
        assert!(!glob_match("?est*", "est"));
    }

    // ---- contains_glob_chars ----

    #[test]
    fn test_contains_glob_star() {
        assert!(contains_glob_chars("*.txt"));
    }

    #[test]
    fn test_contains_glob_question() {
        assert!(contains_glob_chars("file?.rs"));
    }

    #[test]
    fn test_contains_glob_bracket() {
        assert!(contains_glob_chars("[abc]"));
    }

    #[test]
    fn test_no_glob_chars() {
        assert!(!contains_glob_chars("plain.txt"));
    }

    // ---- split_dir_pattern ----

    #[test]
    fn test_split_absolute_path() {
        let (dir, pat) = split_dir_pattern("/tmp/*.txt", "/home");
        assert_eq!(dir, "/tmp");
        assert_eq!(pat, "*.txt");
    }

    #[test]
    fn test_split_relative_pattern() {
        let (dir, pat) = split_dir_pattern("*.rs", "/home/user");
        assert_eq!(dir, "/home/user");
        assert_eq!(pat, "*.rs");
    }

    #[test]
    fn test_split_root_dir() {
        let (dir, pat) = split_dir_pattern("/*.conf", "/");
        assert_eq!(dir, "/");
        assert_eq!(pat, "*.conf");
    }

    #[test]
    fn test_split_nested_path() {
        let (dir, pat) = split_dir_pattern("/etc/conf.d/[a-z]*", "/");
        assert_eq!(dir, "/etc/conf.d");
        assert_eq!(pat, "[a-z]*");
    }
}
