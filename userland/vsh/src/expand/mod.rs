//! Word expansion pipeline.
//!
//! Implements the Bash expansion order:
//! 1. Brace expansion
//! 2. Tilde expansion
//! 3. Parameter/variable expansion
//! 4. Command substitution (during execution)
//! 5. Arithmetic expansion
//! 6. Word splitting
//! 7. Pathname expansion (globbing)
//! 8. Quote removal

pub mod brace;
pub mod glob;
pub mod parameter;
pub mod tilde;

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use parameter::SpecialVars;

use crate::parser::word;

/// Expand a single word through the full expansion pipeline.
///
/// Returns one or more words (due to word splitting and brace expansion).
pub fn expand_word(
    raw: &str,
    vars: &BTreeMap<String, String>,
    special: &SpecialVars,
    do_glob: bool,
) -> Vec<String> {
    // 1. Brace expansion
    let braced = brace::expand_braces(raw);

    let mut results = Vec::new();

    for w in &braced {
        // 2. Tilde expansion
        let tilded = tilde::expand_tilde(w, vars);

        // 3+5. Parameter expansion (includes arithmetic)
        let expanded = parameter::expand_parameters(&tilded, vars, special);

        // 6. Word splitting (only for unquoted results)
        let ifs = vars.get("IFS").map(|s| s.as_str()).unwrap_or(" \t\n");
        let split = word::word_split(&expanded, ifs);

        for part in &split {
            // 7. Pathname expansion (globbing)
            if do_glob && glob::contains_glob_chars(part) {
                // In a real shell, we would enumerate the filesystem.
                // For now, return the pattern as-is.
                results.push(word::remove_quotes(part));
            } else {
                // 8. Quote removal
                results.push(word::remove_quotes(part));
            }
        }
    }

    // If expansion produced nothing, return a single empty string
    if results.is_empty() {
        results.push(String::new());
    }

    results
}

/// Expand a list of words (e.g., command arguments).
pub fn expand_words(
    words: &[crate::parser::ast::Word],
    vars: &BTreeMap<String, String>,
    special: &SpecialVars,
    do_glob: bool,
) -> Vec<String> {
    let mut result = Vec::new();
    for w in words {
        result.extend(expand_word(&w.raw, vars, special, do_glob));
    }
    result
}
