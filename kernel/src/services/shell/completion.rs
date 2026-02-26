//! Tab completion for the VeridianOS shell.
//!
//! Provides intelligent completions for:
//! - Builtin command names (when completing the first token)
//! - File and directory paths from the VFS (for subsequent tokens)
//! - Environment variable names (for tokens starting with `$`)
//!
//! The completer operates on the current input line and cursor position,
//! returning a sorted list of matching candidates.

// Shell tab completion

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// Compute tab completions for the given input line at the cursor position.
///
/// # Arguments
///
/// - `line` -- the full input line typed so far
/// - `cursor_pos` -- byte offset of the cursor within `line`
/// - `builtin_names` -- slice of registered builtin command names
/// - `env_names` -- slice of environment variable names (without `$` prefix)
/// - `cwd` -- the shell's current working directory
///
/// # Returns
///
/// A sorted list of completion candidates. If only one candidate exists,
/// the caller can substitute it directly. If multiple exist, the caller may
/// display them or insert their longest common prefix.
pub fn complete(
    line: &str,
    cursor_pos: usize,
    builtin_names: &[&str],
    env_names: &[&str],
    cwd: &str,
) -> Vec<String> {
    // Work with the portion of the line up to the cursor
    let before_cursor = if cursor_pos <= line.len() {
        &line[..cursor_pos]
    } else {
        line
    };

    // Tokenize the portion before the cursor
    let tokens = tokenize_for_completion(before_cursor);

    // Determine the word being completed
    let (word, is_first_token) = if before_cursor.ends_with(' ') {
        // Cursor is after a space -- starting a new (empty) token
        (String::new(), tokens.is_empty())
    } else if let Some(last) = tokens.last() {
        (last.clone(), tokens.len() == 1)
    } else {
        (String::new(), true)
    };

    // Variable name completion: $<prefix>
    if let Some(stripped) = word.strip_prefix('$') {
        return complete_variable(stripped, env_names);
    }

    // First token: complete builtin command names
    if is_first_token {
        return complete_command(&word, builtin_names);
    }

    // Subsequent tokens: complete file paths
    complete_path(&word, cwd)
}

/// Compute the longest common prefix of a list of candidates.
///
/// Useful for inserting the shared prefix when multiple completions exist.
pub fn longest_common_prefix(candidates: &[String]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    if candidates.len() == 1 {
        return candidates[0].clone();
    }

    let first = &candidates[0];
    let mut prefix_len = first.len();

    for candidate in &candidates[1..] {
        prefix_len = prefix_len.min(candidate.len());

        for (i, (a, b)) in first.chars().zip(candidate.chars()).enumerate() {
            if a != b || i >= prefix_len {
                prefix_len = i;
                break;
            }
        }
    }

    first[..prefix_len].to_string()
}

// ---------------------------------------------------------------------------
// Completion strategies
// ---------------------------------------------------------------------------

/// Complete a builtin command name from the provided list.
fn complete_command(prefix: &str, builtin_names: &[&str]) -> Vec<String> {
    let mut matches: Vec<String> = builtin_names
        .iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| name.to_string())
        .collect();

    matches.sort();
    matches.dedup();
    matches
}

/// Complete an environment variable name (without the leading `$`).
///
/// Returns candidates with the `$` prefix re-attached so the caller can
/// substitute directly.
fn complete_variable(prefix: &str, env_names: &[&str]) -> Vec<String> {
    let mut matches: Vec<String> = env_names
        .iter()
        .filter(|name| name.starts_with(prefix))
        .map(|name| {
            let mut s = String::with_capacity(name.len() + 1);
            s.push('$');
            s.push_str(name);
            s
        })
        .collect();

    matches.sort();
    matches.dedup();
    matches
}

/// Complete a file or directory path from the VFS.
///
/// The `word` may be:
/// - Empty (list cwd contents)
/// - A relative name like `src` (list cwd entries starting with `src`)
/// - An absolute path like `/etc/` (list entries in /etc)
/// - A partial like `/etc/ho` (list /etc entries starting with `ho`)
fn complete_path(word: &str, cwd: &str) -> Vec<String> {
    // Split into directory part and name prefix
    let (dir_path, name_prefix) = if word.is_empty() {
        // Complete from cwd
        (cwd.to_string(), String::new())
    } else if word.ends_with('/') {
        // Complete inside the given directory
        (word.to_string(), String::new())
    } else if let Some(slash_pos) = word.rfind('/') {
        // Split at last slash
        let dir = if slash_pos == 0 {
            String::from("/")
        } else {
            word[..slash_pos].to_string()
        };
        let prefix = word[slash_pos + 1..].to_string();
        (dir, prefix)
    } else {
        // Relative name -- complete in cwd
        (cwd.to_string(), word.to_string())
    };

    // List directory entries from VFS
    let entries = list_directory_entries(&dir_path);
    if entries.is_empty() {
        return Vec::new();
    }

    // Filter by prefix and build full paths
    let mut matches: Vec<String> = Vec::new();

    for (name, is_dir) in &entries {
        if name.starts_with(name_prefix.as_str()) {
            let full = if word.is_empty() {
                // No prefix typed -- show name only
                if *is_dir {
                    let mut s = name.clone();
                    s.push('/');
                    s
                } else {
                    name.clone()
                }
            } else if word.contains('/') {
                // Rebuild with directory prefix
                let dir_prefix = if let Some(pos) = word.rfind('/') {
                    &word[..=pos]
                } else {
                    ""
                };
                if *is_dir {
                    let mut s = String::from(dir_prefix);
                    s.push_str(name);
                    s.push('/');
                    s
                } else {
                    let mut s = String::from(dir_prefix);
                    s.push_str(name);
                    s
                }
            } else if *is_dir {
                let mut s = name.clone();
                s.push('/');
                s
            } else {
                name.clone()
            };

            matches.push(full);
        }
    }

    matches.sort();
    matches
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Tokenize the input line for completion purposes.
///
/// Simpler than the shell's main tokenizer -- we just need to split on
/// whitespace while respecting quotes.
fn tokenize_for_completion(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in input.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quote => {
                escape_next = true;
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// List entries in a directory from the VFS.
///
/// Returns a vector of `(name, is_directory)` pairs. Returns an empty
/// vector if the VFS is unavailable or the directory cannot be read.
fn list_directory_entries(dir_path: &str) -> Vec<(String, bool)> {
    if let Some(vfs) = crate::fs::try_get_vfs() {
        let vfs_guard = vfs.read();
        if let Ok(node) = vfs_guard.resolve_path(dir_path) {
            if let Ok(entries) = node.readdir() {
                return entries
                    .iter()
                    .map(|e| {
                        let is_dir = e.node_type == crate::fs::NodeType::Directory;
                        (e.name.clone(), is_dir)
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    // ---- longest_common_prefix ----

    #[test]
    fn test_lcp_empty_list() {
        let candidates: Vec<String> = Vec::new();
        assert_eq!(longest_common_prefix(&candidates), "");
    }

    #[test]
    fn test_lcp_single_candidate() {
        let candidates = vec!["hello".to_string()];
        assert_eq!(longest_common_prefix(&candidates), "hello");
    }

    #[test]
    fn test_lcp_common_prefix() {
        let candidates = vec!["export".to_string(), "exit".to_string(), "exec".to_string()];
        assert_eq!(longest_common_prefix(&candidates), "ex");
    }

    #[test]
    fn test_lcp_identical() {
        let candidates = vec!["same".to_string(), "same".to_string()];
        assert_eq!(longest_common_prefix(&candidates), "same");
    }

    #[test]
    fn test_lcp_no_common_prefix() {
        let candidates = vec!["abc".to_string(), "xyz".to_string()];
        assert_eq!(longest_common_prefix(&candidates), "");
    }

    #[test]
    fn test_lcp_one_empty() {
        let candidates = vec!["hello".to_string(), String::new()];
        assert_eq!(longest_common_prefix(&candidates), "");
    }

    // ---- tokenize_for_completion ----

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize_for_completion("ls -la /tmp");
        assert_eq!(tokens, vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize_for_completion("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_quoted() {
        let tokens = tokenize_for_completion("echo \"hello world\"");
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    #[test]
    fn test_tokenize_single_quoted() {
        let tokens = tokenize_for_completion("echo 'one two'");
        assert_eq!(tokens, vec!["echo", "one two"]);
    }

    #[test]
    fn test_tokenize_escaped_space() {
        let tokens = tokenize_for_completion("echo hello\\ world");
        assert_eq!(tokens, vec!["echo", "hello world"]);
    }

    // ---- complete_command ----

    #[test]
    fn test_complete_command_prefix() {
        let builtins = &["echo", "env", "exit", "export", "help"];
        let result = complete_command("ex", builtins);
        assert_eq!(result, vec!["exit", "export"]);
    }

    #[test]
    fn test_complete_command_empty_prefix() {
        let builtins = &["cd", "echo", "help"];
        let result = complete_command("", builtins);
        assert_eq!(result, vec!["cd", "echo", "help"]);
    }

    #[test]
    fn test_complete_command_no_match() {
        let builtins = &["cd", "echo", "help"];
        let result = complete_command("zz", builtins);
        assert!(result.is_empty());
    }

    #[test]
    fn test_complete_command_exact_match() {
        let builtins = &["echo", "exit"];
        let result = complete_command("echo", builtins);
        assert_eq!(result, vec!["echo"]);
    }

    // ---- complete_variable ----

    #[test]
    fn test_complete_variable_prefix() {
        let env = &["HOME", "HOST", "PATH"];
        let result = complete_variable("HO", env);
        assert_eq!(result, vec!["$HOME", "$HOST"]);
    }

    #[test]
    fn test_complete_variable_no_match() {
        let env = &["HOME"];
        let result = complete_variable("ZZ", env);
        assert!(result.is_empty());
    }

    #[test]
    fn test_complete_variable_all() {
        let env = &["A", "B"];
        let result = complete_variable("", env);
        assert_eq!(result, vec!["$A", "$B"]);
    }

    #[test]
    fn test_complete_variable_single_match() {
        let env = &["PATH", "PWD", "SHELL"];
        let result = complete_variable("SH", env);
        assert_eq!(result, vec!["$SHELL"]);
    }

    // ---- complete (integration, no VFS) ----

    #[test]
    fn test_complete_first_token_command() {
        let builtins = &["echo", "exit", "export", "help"];
        let env = &["HOME", "PATH"];
        let result = complete("ex", 2, builtins, env, "/");
        assert_eq!(result, vec!["exit", "export"]);
    }

    #[test]
    fn test_complete_variable_token() {
        let builtins = &["echo"];
        let env = &["HOME", "HOST", "PATH"];
        let result = complete("echo $HO", 8, builtins, env, "/");
        assert_eq!(result, vec!["$HOME", "$HOST"]);
    }

    #[test]
    fn test_complete_empty_line() {
        let builtins = &["cd", "echo", "help"];
        let env = &["PATH"];
        let result = complete("", 0, builtins, env, "/");
        assert_eq!(result, vec!["cd", "echo", "help"]);
    }

    #[test]
    fn test_complete_variable_first_token() {
        // Even if it is the first token, $-prefix triggers variable completion
        let builtins = &["echo"];
        let env = &["PATH", "PWD"];
        let result = complete("$P", 2, builtins, env, "/");
        assert_eq!(result, vec!["$PATH", "$PWD"]);
    }

    #[test]
    fn test_complete_cursor_mid_line() {
        // Cursor at position 2 in "exit foo" -- only "ex" is considered
        let builtins = &["echo", "exit", "export"];
        let env: &[&str] = &[];
        let result = complete("exit foo", 2, builtins, env, "/");
        assert_eq!(result, vec!["exit", "export"]);
    }
}
