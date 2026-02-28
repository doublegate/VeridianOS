//! Indexed array implementation.
//!
//! Bash indexed arrays: `declare -a arr`, `arr[0]=val`, `${arr[0]}`,
//! `${arr[@]}`, `${#arr[@]}`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// An indexed array (sparse, like Bash).
#[derive(Debug, Clone)]
pub struct IndexedArray {
    elements: BTreeMap<usize, String>,
}

impl IndexedArray {
    pub fn new() -> Self {
        Self {
            elements: BTreeMap::new(),
        }
    }

    /// Create from a list of values (indices 0, 1, 2, ...).
    pub fn from_values(values: &[String]) -> Self {
        let mut arr = Self::new();
        for (i, v) in values.iter().enumerate() {
            arr.set(i, v.clone());
        }
        arr
    }

    /// Get element at index.
    pub fn get(&self, index: usize) -> Option<&String> {
        self.elements.get(&index)
    }

    /// Set element at index.
    pub fn set(&mut self, index: usize, value: String) {
        self.elements.insert(index, value);
    }

    /// Remove element at index.
    pub fn unset(&mut self, index: usize) {
        self.elements.remove(&index);
    }

    /// Number of set elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the array is empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get all values (in index order).
    pub fn values(&self) -> Vec<&String> {
        self.elements.values().collect()
    }

    /// Get all indices.
    pub fn indices(&self) -> Vec<usize> {
        self.elements.keys().copied().collect()
    }

    /// Append a value at the next available index.
    pub fn push(&mut self, value: String) {
        let next = self.elements.keys().last().map(|k| k + 1).unwrap_or(0);
        self.elements.insert(next, value);
    }

    /// Join all values with a separator.
    pub fn join(&self, sep: &str) -> String {
        let vals: Vec<&str> = self.elements.values().map(|s| s.as_str()).collect();
        let mut result = String::new();
        for (i, v) in vals.iter().enumerate() {
            if i > 0 {
                result.push_str(sep);
            }
            result.push_str(v);
        }
        result
    }
}
