//! Command alias support for the VeridianOS shell.
//!
//! Provides an alias registry that maps short names to full command strings,
//! with recursive expansion and loop detection.

// Shell alias registry -- API complete, not yet wired to executor
#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// AliasRegistry
// ---------------------------------------------------------------------------

/// Registry of command aliases.
///
/// Aliases are simple string-to-string mappings. When the shell encounters
/// a command whose first word matches an alias name, the alias value is
/// substituted in. Expansion is recursive up to a configurable depth limit
/// to guard against alias loops.
pub struct AliasRegistry {
    /// Alias name -> expansion value.
    aliases: BTreeMap<String, String>,
}

impl Default for AliasRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AliasRegistry {
    /// Create an empty alias registry.
    pub fn new() -> Self {
        Self {
            aliases: BTreeMap::new(),
        }
    }

    /// Define or update an alias.
    pub fn set(&mut self, name: String, value: String) {
        self.aliases.insert(name, value);
    }

    /// Look up an alias by name.
    pub fn get(&self, name: &str) -> Option<&String> {
        self.aliases.get(name)
    }

    /// Remove an alias. Returns the old value, if any.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.aliases.remove(name)
    }

    /// List all aliases as `(name, value)` pairs, sorted by name.
    pub fn list(&self) -> Vec<(&String, &String)> {
        self.aliases.iter().collect()
    }

    /// Return the number of defined aliases.
    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    /// Check whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }

    /// Check whether an alias with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.aliases.contains_key(name)
    }

    /// Remove all aliases.
    pub fn clear(&mut self) {
        self.aliases.clear();
    }
}

// ---------------------------------------------------------------------------
// Alias expansion
// ---------------------------------------------------------------------------

/// Maximum number of recursive alias expansions before we stop
/// (prevents infinite loops such as `alias a=b; alias b=a`).
const MAX_EXPANSION_DEPTH: usize = 10;

/// Expand aliases in a command string.
///
/// Only the **first word** of the command is eligible for alias expansion.
/// Expansion is applied recursively: if the replacement itself starts with
/// an alias, it is expanded again, up to [`MAX_EXPANSION_DEPTH`] iterations.
///
/// Returns the fully-expanded command string.
pub fn expand_aliases(command: &str, registry: &AliasRegistry) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut result = String::from(trimmed);

    for _ in 0..MAX_EXPANSION_DEPTH {
        // Extract the first word
        let first_word_end = result
            .find(|c: char| c.is_whitespace())
            .unwrap_or(result.len());
        let first_word = &result[..first_word_end];

        if let Some(expansion) = registry.get(first_word) {
            // Replace the first word with the alias expansion
            let rest = &result[first_word_end..];
            result = if rest.is_empty() {
                expansion.clone()
            } else {
                let mut expanded = expansion.clone();
                expanded.push_str(rest);
                expanded
            };
        } else {
            // No further expansion possible
            break;
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));

        assert_eq!(reg.get("ll"), Some(&String::from("ls -la")));
        assert_eq!(reg.get("missing"), None);
    }

    #[test]
    fn test_remove() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));

        let removed = reg.remove("ll");
        assert_eq!(removed, Some(String::from("ls -la")));
        assert!(reg.get("ll").is_none());

        assert_eq!(reg.remove("ll"), None);
    }

    #[test]
    fn test_list() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("b"), String::from("beta"));
        reg.set(String::from("a"), String::from("alpha"));

        let list = reg.list();
        assert_eq!(list.len(), 2);
        // BTreeMap is sorted by key
        assert_eq!(list[0].0, "a");
        assert_eq!(list[1].0, "b");
    }

    #[test]
    fn test_len_and_empty() {
        let mut reg = AliasRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);

        reg.set(String::from("x"), String::from("y"));
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_contains() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));

        assert!(reg.contains("ll"));
        assert!(!reg.contains("nope"));
    }

    #[test]
    fn test_clear() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("a"), String::from("b"));
        reg.set(String::from("c"), String::from("d"));
        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_expand_no_alias() {
        let reg = AliasRegistry::new();
        assert_eq!(expand_aliases("ls -la", &reg), "ls -la");
    }

    #[test]
    fn test_expand_simple_alias() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));

        assert_eq!(expand_aliases("ll /tmp", &reg), "ls -la /tmp");
    }

    #[test]
    fn test_expand_alias_no_args() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));

        assert_eq!(expand_aliases("ll", &reg), "ls -la");
    }

    #[test]
    fn test_expand_recursive_alias() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("l"), String::from("ll"));
        reg.set(String::from("ll"), String::from("ls -la"));

        assert_eq!(expand_aliases("l /tmp", &reg), "ls -la /tmp");
    }

    #[test]
    fn test_expand_loop_detection() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("a"), String::from("b"));
        reg.set(String::from("b"), String::from("a"));

        // Should not hang — stops after MAX_EXPANSION_DEPTH
        let result = expand_aliases("a", &reg);
        // After 10 expansions of a->b->a->b... the result alternates.
        // The important thing is it terminates.
        assert!(!result.is_empty());
    }

    #[test]
    fn test_expand_empty_command() {
        let reg = AliasRegistry::new();
        assert_eq!(expand_aliases("", &reg), "");
        assert_eq!(expand_aliases("   ", &reg), "");
    }

    #[test]
    fn test_overwrite_alias() {
        let mut reg = AliasRegistry::new();
        reg.set(String::from("ll"), String::from("ls -la"));
        reg.set(String::from("ll"), String::from("ls -lah"));

        assert_eq!(reg.get("ll"), Some(&String::from("ls -lah")));
    }
}
