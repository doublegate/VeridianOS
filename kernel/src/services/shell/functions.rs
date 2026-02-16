//! User-defined shell functions for the VeridianOS shell.
//!
//! Supports defining, looking up, removing, and listing named functions
//! whose bodies are stored as sequences of command lines to be interpreted
//! by the shell when invoked.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// ShellFunction
// ---------------------------------------------------------------------------

/// A user-defined shell function.
///
/// Functions are defined with a name and a body consisting of one or more
/// command lines. When invoked, the body lines are executed sequentially
/// in the current shell context.
#[derive(Debug, Clone)]
pub struct ShellFunction {
    /// The function name (used to invoke it).
    pub name: String,
    /// The body: a list of command lines to execute.
    pub body: Vec<String>,
}

impl ShellFunction {
    /// Create a new shell function.
    pub fn new(name: String, body: Vec<String>) -> Self {
        Self { name, body }
    }

    /// Return the number of lines in the function body.
    pub fn line_count(&self) -> usize {
        self.body.len()
    }

    /// Check whether the function body is empty.
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }
}

impl core::fmt::Display for ShellFunction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "{}() {{", self.name)?;
        for line in &self.body {
            writeln!(f, "    {}", line)?;
        }
        write!(f, "}}")
    }
}

// ---------------------------------------------------------------------------
// FunctionRegistry
// ---------------------------------------------------------------------------

/// Registry of user-defined shell functions.
///
/// Functions are stored by name in a sorted map for deterministic
/// listing order.
pub struct FunctionRegistry {
    /// Function name -> definition.
    functions: BTreeMap<String, ShellFunction>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionRegistry {
    /// Create an empty function registry.
    pub fn new() -> Self {
        Self {
            functions: BTreeMap::new(),
        }
    }

    /// Define (or redefine) a shell function.
    ///
    /// If a function with the same name already exists, it is replaced.
    pub fn define(&mut self, name: String, body: Vec<String>) {
        let func = ShellFunction::new(name.clone(), body);
        self.functions.insert(name, func);
    }

    /// Look up a function by name.
    pub fn get(&self, name: &str) -> Option<&ShellFunction> {
        self.functions.get(name)
    }

    /// Remove a function by name.
    ///
    /// Returns `true` if the function existed and was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.functions.remove(name).is_some()
    }

    /// List all defined function names in sorted order.
    pub fn list(&self) -> Vec<&str> {
        self.functions.keys().map(|k| k.as_str()).collect()
    }

    /// Return the number of defined functions.
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Check whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    /// Check whether a function with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    /// Remove all functions.
    pub fn clear(&mut self) {
        self.functions.clear();
    }

    /// Iterate over all functions.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ShellFunction)> {
        self.functions.iter()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;

    #[test]
    fn test_shell_function_creation() {
        let func = ShellFunction::new(
            String::from("greet"),
            vec![String::from("echo Hello"), String::from("echo World")],
        );
        assert_eq!(func.name, "greet");
        assert_eq!(func.line_count(), 2);
        assert!(!func.is_empty());
    }

    #[test]
    fn test_shell_function_empty() {
        let func = ShellFunction::new(String::from("noop"), Vec::new());
        assert!(func.is_empty());
        assert_eq!(func.line_count(), 0);
    }

    #[test]
    fn test_registry_define_and_get() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("hello"), vec![String::from("echo hello")]);

        let func = reg.get("hello");
        assert!(func.is_some());
        assert_eq!(func.unwrap().name, "hello");
        assert_eq!(func.unwrap().body, vec!["echo hello"]);
    }

    #[test]
    fn test_registry_get_missing() {
        let reg = FunctionRegistry::new();
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn test_registry_redefine() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("f"), vec![String::from("echo v1")]);
        reg.define(String::from("f"), vec![String::from("echo v2")]);

        let func = reg.get("f").unwrap();
        assert_eq!(func.body, vec!["echo v2"]);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_registry_remove() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("f"), vec![String::from("echo")]);

        assert!(reg.remove("f"));
        assert!(!reg.remove("f"));
        assert!(reg.is_empty());
    }

    #[test]
    fn test_registry_list() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("beta"), vec![String::from("cmd")]);
        reg.define(String::from("alpha"), vec![String::from("cmd")]);
        reg.define(String::from("gamma"), vec![String::from("cmd")]);

        let names = reg.list();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_registry_len_and_empty() {
        let mut reg = FunctionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);

        reg.define(String::from("a"), vec![]);
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_registry_contains() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("exists"), vec![]);

        assert!(reg.contains("exists"));
        assert!(!reg.contains("nope"));
    }

    #[test]
    fn test_registry_clear() {
        let mut reg = FunctionRegistry::new();
        reg.define(String::from("a"), vec![]);
        reg.define(String::from("b"), vec![]);
        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_shell_function_display() {
        let func = ShellFunction::new(
            String::from("greet"),
            vec![String::from("echo Hi"), String::from("echo Bye")],
        );
        let display = alloc::format!("{}", func);
        assert!(display.contains("greet() {"));
        assert!(display.contains("    echo Hi"));
        assert!(display.contains("    echo Bye"));
        assert!(display.contains("}"));
    }
}
