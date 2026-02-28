//! Tilde expansion.
//!
//! Supports `~`, `~user`, `~+` (PWD), and `~-` (OLDPWD).

use alloc::{collections::BTreeMap, string::String};

/// Expand tilde prefix in a word.
///
/// - `~` expands to `$HOME`
/// - `~+` expands to `$PWD`
/// - `~-` expands to `$OLDPWD`
/// - `~user` would expand to user's home directory (not yet supported in
///   VeridianOS)
pub fn expand_tilde(word: &str, vars: &BTreeMap<String, String>) -> String {
    if !word.starts_with('~') {
        return String::from(word);
    }

    // Find the end of the tilde-prefix (first unquoted `/` or end of word)
    let prefix_end = word[1..].find('/').map(|i| i + 1).unwrap_or(word.len());
    let tilde_part = &word[1..prefix_end];
    let rest = &word[prefix_end..];

    let expanded = match tilde_part {
        "" => {
            // ~ alone -> $HOME
            vars.get("HOME")
                .cloned()
                .unwrap_or_else(|| String::from("~"))
        }
        "+" => {
            // ~+ -> $PWD
            vars.get("PWD")
                .cloned()
                .unwrap_or_else(|| String::from("~+"))
        }
        "-" => {
            // ~- -> $OLDPWD
            vars.get("OLDPWD")
                .cloned()
                .unwrap_or_else(|| String::from("~-"))
        }
        _user => {
            // ~user: not yet supported, return as-is
            return String::from(word);
        }
    };

    let mut result = expanded;
    result.push_str(rest);
    result
}
