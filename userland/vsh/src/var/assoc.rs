//! Associative array implementation.
//!
//! Bash associative arrays: `declare -A map`, `map[key]=val`,
//! `${map[key]}`, `${map[@]}`, `${#map[@]}`, `${!map[@]}`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// An associative array (string-keyed).
#[derive(Debug, Clone)]
pub struct AssocArray {
    elements: BTreeMap<String, String>,
}

impl AssocArray {
    pub fn new() -> Self {
        Self {
            elements: BTreeMap::new(),
        }
    }

    /// Get element by key.
    pub fn get(&self, key: &str) -> Option<&String> {
        self.elements.get(key)
    }

    /// Set element by key.
    pub fn set(&mut self, key: String, value: String) {
        self.elements.insert(key, value);
    }

    /// Remove element by key.
    pub fn unset(&mut self, key: &str) {
        self.elements.remove(key);
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the array is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get all values.
    pub fn values(&self) -> Vec<&String> {
        self.elements.values().collect()
    }

    /// Get all keys.
    pub fn keys(&self) -> Vec<&String> {
        self.elements.keys().collect()
    }

    /// Check if a key exists.
    pub fn contains_key(&self, key: &str) -> bool {
        self.elements.contains_key(key)
    }

    /// Join all values with a separator.
    pub fn join(&self, sep: &str) -> String {
        let vals: Vec<&str> = self.elements.values().map(|s| s.as_str()).collect();
        let mut result = String::new();
        for (i, v) in vals.iter().enumerate() {
            if i > 0 { result.push_str(sep); }
            result.push_str(v);
        }
        result
    }
}
