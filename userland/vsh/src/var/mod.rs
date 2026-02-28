//! Shell variable and environment management.
//!
//! Manages regular variables, exported environment variables, special
//! parameters (`$?`, `$$`, `$!`, etc.), arrays, associative arrays,
//! variable attributes (export, readonly, integer, nameref), and
//! scope management for functions.

extern crate alloc;

use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

/// Attributes that can be set on a variable.
#[derive(Debug, Clone, Copy, Default)]
pub struct VarAttrs {
    /// `-x`: Variable is exported to child processes.
    pub exported: bool,
    /// `-r`: Variable is readonly.
    pub readonly: bool,
    /// `-i`: Variable is treated as integer in arithmetic contexts.
    pub integer: bool,
    /// `-l`: Lowercase on assignment.
    pub lowercase: bool,
    /// `-u`: Uppercase on assignment.
    pub uppercase: bool,
    /// `-n`: Nameref (variable is a reference to another variable).
    pub nameref: bool,
    /// `-a`: Indexed array.
    pub is_array: bool,
    /// `-A`: Associative array.
    pub is_assoc: bool,
    /// `-t`: Trace attribute.
    pub trace: bool,
}

/// A shell variable value.
#[derive(Debug, Clone)]
pub enum VarValue {
    /// A scalar string value.
    Scalar(String),
    /// An indexed array.
    Array(Vec<String>),
    /// An associative array (key -> value).
    Assoc(BTreeMap<String, String>),
}

impl VarValue {
    /// Get the scalar value, or element 0 of an array.
    pub fn as_str(&self) -> &str {
        match self {
            VarValue::Scalar(s) => s.as_str(),
            VarValue::Array(a) => {
                if a.is_empty() {
                    ""
                } else {
                    a[0].as_str()
                }
            }
            VarValue::Assoc(_) => "",
        }
    }

    /// Convert to owned string.
    pub fn to_string_val(&self) -> String {
        String::from(self.as_str())
    }
}

/// A variable entry.
#[derive(Debug, Clone)]
pub struct Variable {
    pub value: VarValue,
    pub attrs: VarAttrs,
}

/// A single variable scope.
#[derive(Debug, Clone)]
struct Scope {
    vars: BTreeMap<String, Variable>,
}

impl Scope {
    fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }
}

/// Shell environment: manages all variables across scopes.
pub struct ShellEnv {
    /// Stack of scopes. Index 0 is the global scope, last is current.
    scopes: Vec<Scope>,
    /// Positional parameters ($1, $2, ...).
    pub positional: Vec<String>,
    /// Last exit status ($?).
    pub last_status: i32,
    /// PID of the shell ($$).
    pub shell_pid: i32,
    /// PID of the last background process ($!).
    pub last_bg_pid: i32,
    /// The name of the shell or script ($0).
    pub arg0: String,
    /// The current option flags ($-).
    pub option_flags: String,
    /// The last argument of the previous command ($_).
    pub last_arg: String,
    /// Shell functions.
    pub functions: BTreeMap<String, ShellFunction>,
    /// Alias definitions.
    pub aliases: BTreeMap<String, String>,
    /// Hash table of command paths.
    pub hash_table: BTreeMap<String, String>,
}

/// A shell function definition.
#[derive(Debug, Clone)]
pub struct ShellFunction {
    /// The function name.
    pub name: String,
    /// The raw AST body, stored as a serialized form. In practice the
    /// executor keeps the parsed AST but we store the source text here
    /// for `declare -f` display.
    pub body_source: String,
}

impl ShellEnv {
    /// Create a new shell environment with sensible defaults.
    pub fn new() -> Self {
        let mut env = Self {
            scopes: Vec::new(),
            positional: Vec::new(),
            last_status: 0,
            shell_pid: 0,
            last_bg_pid: 0,
            arg0: String::from("vsh"),
            option_flags: String::new(),
            last_arg: String::new(),
            functions: BTreeMap::new(),
            aliases: BTreeMap::new(),
            hash_table: BTreeMap::new(),
        };
        env.scopes.push(Scope::new());
        env
    }

    // --- Scope management ---

    /// Push a new local scope (for function calls).
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    /// Pop the current local scope. Variables marked `export` are merged
    /// into the parent.
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            let scope = self.scopes.pop().unwrap();
            // Exported variables propagate to parent
            if let Some(parent) = self.scopes.last_mut() {
                for (name, var) in scope.vars {
                    if var.attrs.exported {
                        parent.vars.insert(name, var);
                    }
                }
            }
        }
    }

    // --- Variable access ---

    /// Look up a variable by name. Searches from innermost scope outward.
    pub fn get(&self, name: &str) -> Option<&Variable> {
        // Handle nameref resolution (one level)
        for scope in self.scopes.iter().rev() {
            if let Some(var) = scope.vars.get(name) {
                if var.attrs.nameref {
                    if let VarValue::Scalar(ref target) = var.value {
                        return self.get_direct(target);
                    }
                }
                return Some(var);
            }
        }
        None
    }

    /// Direct lookup without nameref resolution.
    fn get_direct(&self, name: &str) -> Option<&Variable> {
        for scope in self.scopes.iter().rev() {
            if let Some(var) = scope.vars.get(name) {
                return Some(var);
            }
        }
        None
    }

    /// Get the value of a variable as a string. Returns empty string if
    /// the variable is not set.
    pub fn get_str(&self, name: &str) -> &str {
        match self.get(name) {
            Some(var) => var.value.as_str(),
            None => "",
        }
    }

    /// Set a variable in the current (innermost) scope.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), &'static str> {
        // Check readonly in all scopes
        if let Some(var) = self.get(name) {
            if var.attrs.readonly {
                return Err("readonly variable");
            }
        }

        let scope = self.scopes.last_mut().unwrap();

        if let Some(existing) = scope.vars.get_mut(name) {
            // Copy attrs to avoid borrow conflict with self.transform_value()
            let attrs = existing.attrs;
            let transformed = Self::transform_value_static(value, &attrs);
            existing.value = VarValue::Scalar(transformed);
        } else {
            scope.vars.insert(
                String::from(name),
                Variable {
                    value: VarValue::Scalar(String::from(value)),
                    attrs: VarAttrs::default(),
                },
            );
        }
        Ok(())
    }

    /// Set a variable in the global scope.
    pub fn set_global(&mut self, name: &str, value: &str) -> Result<(), &'static str> {
        // Check readonly
        if let Some(var) = self.get(name) {
            if var.attrs.readonly {
                return Err("readonly variable");
            }
        }

        let scope = &mut self.scopes[0];
        if let Some(existing) = scope.vars.get_mut(name) {
            let attrs = existing.attrs;
            let transformed = Self::transform_value_static(value, &attrs);
            existing.value = VarValue::Scalar(transformed);
        } else {
            scope.vars.insert(
                String::from(name),
                Variable {
                    value: VarValue::Scalar(String::from(value)),
                    attrs: VarAttrs::default(),
                },
            );
        }
        Ok(())
    }

    /// Set a local variable in the innermost scope only (for `local` builtin).
    pub fn set_local(&mut self, name: &str, value: &str) -> Result<(), &'static str> {
        let scope = self.scopes.last_mut().unwrap();
        scope.vars.insert(
            String::from(name),
            Variable {
                value: VarValue::Scalar(String::from(value)),
                attrs: VarAttrs::default(),
            },
        );
        Ok(())
    }

    /// Unset a variable.
    pub fn unset(&mut self, name: &str) -> Result<(), &'static str> {
        // Check readonly
        if let Some(var) = self.get(name) {
            if var.attrs.readonly {
                return Err("readonly variable");
            }
        }
        for scope in self.scopes.iter_mut().rev() {
            if scope.vars.remove(name).is_some() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Export a variable (mark it for child process inheritance).
    pub fn export(&mut self, name: &str, value: Option<&str>) {
        if let Some(val) = value {
            let _ = self.set(name, val);
        }
        // Find and mark exported
        for scope in self.scopes.iter_mut().rev() {
            if let Some(var) = scope.vars.get_mut(name) {
                var.attrs.exported = true;
                return;
            }
        }
        // Variable not found -- create it empty and exported
        let scope = self.scopes.last_mut().unwrap();
        scope.vars.insert(
            String::from(name),
            Variable {
                value: VarValue::Scalar(String::new()),
                attrs: VarAttrs {
                    exported: true,
                    ..VarAttrs::default()
                },
            },
        );
    }

    /// Mark a variable as readonly.
    pub fn set_readonly(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(var) = scope.vars.get_mut(name) {
                var.attrs.readonly = true;
                return;
            }
        }
    }

    // --- Array operations ---

    /// Set an indexed array element.
    pub fn set_array_element(
        &mut self,
        name: &str,
        index: usize,
        value: &str,
    ) -> Result<(), &'static str> {
        if let Some(var) = self.get(name) {
            if var.attrs.readonly {
                return Err("readonly variable");
            }
        }

        let scope = self.scopes.last_mut().unwrap();
        let entry = scope
            .vars
            .entry(String::from(name))
            .or_insert_with(|| Variable {
                value: VarValue::Array(Vec::new()),
                attrs: VarAttrs {
                    is_array: true,
                    ..VarAttrs::default()
                },
            });

        match &mut entry.value {
            VarValue::Array(arr) => {
                while arr.len() <= index {
                    arr.push(String::new());
                }
                arr[index] = String::from(value);
            }
            _ => {
                // Convert scalar to array
                let old = entry.value.to_string_val();
                let mut arr = Vec::new();
                if !old.is_empty() {
                    arr.push(old);
                }
                while arr.len() <= index {
                    arr.push(String::new());
                }
                arr[index] = String::from(value);
                entry.value = VarValue::Array(arr);
                entry.attrs.is_array = true;
            }
        }
        Ok(())
    }

    /// Get an indexed array element.
    pub fn get_array_element(&self, name: &str, index: usize) -> Option<&str> {
        match self.get(name) {
            Some(Variable {
                value: VarValue::Array(arr),
                ..
            }) => arr.get(index).map(|s| s.as_str()),
            Some(Variable {
                value: VarValue::Scalar(s),
                ..
            }) if index == 0 => Some(s.as_str()),
            _ => None,
        }
    }

    /// Get all elements of an array (for `${arr[@]}` / `${arr[*]}`).
    pub fn get_array_all(&self, name: &str) -> Option<&[String]> {
        match self.get(name) {
            Some(Variable {
                value: VarValue::Array(arr),
                ..
            }) => Some(arr.as_slice()),
            _ => None,
        }
    }

    /// Get the length of an array.
    pub fn get_array_len(&self, name: &str) -> usize {
        match self.get(name) {
            Some(Variable {
                value: VarValue::Array(arr),
                ..
            }) => arr.len(),
            Some(Variable {
                value: VarValue::Scalar(s),
                ..
            }) => {
                if s.is_empty() {
                    0
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    /// Set an associative array element.
    pub fn set_assoc_element(
        &mut self,
        name: &str,
        key: &str,
        value: &str,
    ) -> Result<(), &'static str> {
        if let Some(var) = self.get(name) {
            if var.attrs.readonly {
                return Err("readonly variable");
            }
        }

        let scope = self.scopes.last_mut().unwrap();
        let entry = scope
            .vars
            .entry(String::from(name))
            .or_insert_with(|| Variable {
                value: VarValue::Assoc(BTreeMap::new()),
                attrs: VarAttrs {
                    is_assoc: true,
                    ..VarAttrs::default()
                },
            });

        match &mut entry.value {
            VarValue::Assoc(map) => {
                map.insert(String::from(key), String::from(value));
            }
            _ => {
                let mut map = BTreeMap::new();
                map.insert(String::from(key), String::from(value));
                entry.value = VarValue::Assoc(map);
                entry.attrs.is_assoc = true;
            }
        }
        Ok(())
    }

    /// Get an associative array element.
    pub fn get_assoc_element(&self, name: &str, key: &str) -> Option<&str> {
        match self.get(name) {
            Some(Variable {
                value: VarValue::Assoc(map),
                ..
            }) => map.get(key).map(|s| s.as_str()),
            _ => None,
        }
    }

    // --- Special parameters ---

    /// Resolve a special parameter by name.
    pub fn get_special(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(format!("{}", self.last_status)),
            "$" => Some(format!("{}", self.shell_pid)),
            "!" => Some(format!("{}", self.last_bg_pid)),
            "#" => Some(format!("{}", self.positional.len())),
            "0" => Some(self.arg0.clone()),
            "-" => Some(self.option_flags.clone()),
            "_" => Some(self.last_arg.clone()),
            "@" | "*" => {
                // $@ and $* both expand to positional params, but differ in
                // quoting behavior (handled by expansion, not here).
                let joined = self
                    .positional
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>();
                Some(joined.join(" "))
            }
            _ => {
                // Positional parameter $1, $2, ...
                if let Some(n) = parse_usize(name) {
                    if n >= 1 && n <= self.positional.len() {
                        return Some(self.positional[n - 1].clone());
                    }
                    return Some(String::new());
                }
                None
            }
        }
    }

    /// Check if a name is a special parameter.
    pub fn is_special(name: &str) -> bool {
        matches!(name, "?" | "$" | "!" | "#" | "0" | "-" | "_" | "@" | "*")
            || name.as_bytes().iter().all(|b| b.is_ascii_digit())
    }

    // --- Environment collection ---

    /// Collect all exported variables as `KEY=VALUE` strings, suitable for
    /// passing to `execve`.
    pub fn collect_env(&self) -> Vec<String> {
        let mut env = BTreeMap::new();
        for scope in &self.scopes {
            for (name, var) in &scope.vars {
                if var.attrs.exported {
                    env.insert(name.clone(), format!("{}={}", name, var.value.as_str()));
                }
            }
        }
        env.into_values().collect()
    }

    /// Collect all variable names (for tab completion, etc.).
    pub fn all_var_names(&self) -> Vec<String> {
        let mut names = BTreeMap::new();
        for scope in &self.scopes {
            for name in scope.vars.keys() {
                names.insert(name.clone(), ());
            }
        }
        names.into_keys().collect()
    }

    /// Import an environment string (`NAME=VALUE`) into the global scope
    /// as an exported variable.
    pub fn import_env(&mut self, entry: &str) {
        if let Some(eq_pos) = entry.find('=') {
            let name = &entry[..eq_pos];
            let value = &entry[eq_pos + 1..];
            let _ = self.set_global(name, value);
            self.export(name, None);
        }
    }

    // --- Helpers ---

    fn transform_value_static(value: &str, attrs: &VarAttrs) -> String {
        if attrs.lowercase {
            return value
                .chars()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        (c as u8 + 32) as char
                    } else {
                        c
                    }
                })
                .collect();
        }
        if attrs.uppercase {
            return value
                .chars()
                .map(|c| {
                    if c.is_ascii_lowercase() {
                        (c as u8 - 32) as char
                    } else {
                        c
                    }
                })
                .collect();
        }
        String::from(value)
    }

    /// Check whether a variable is set (exists with any value including empty).
    pub fn is_set(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Get the length of a variable's value (`${#var}`).
    pub fn var_len(&self, name: &str) -> usize {
        match self.get(name) {
            Some(var) => var.value.as_str().len(),
            None => 0,
        }
    }
}

/// Parse an unsigned integer from a string.
fn parse_usize(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}
