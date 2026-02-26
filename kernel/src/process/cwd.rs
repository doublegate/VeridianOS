//! Per-Process Working Directory
//!
//! Tracks and resolves the current working directory for each process.
//! Provides path normalization and resolution of relative paths.

// Per-process working directory

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// ProcessCwd
// ---------------------------------------------------------------------------

/// Per-process current working directory state.
#[cfg(feature = "alloc")]
pub struct ProcessCwd {
    /// The current working directory as an absolute path.
    path: String,
}

#[cfg(feature = "alloc")]
impl Default for ProcessCwd {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl ProcessCwd {
    /// Create a new `ProcessCwd` with the root directory as the default.
    pub fn new() -> Self {
        Self {
            path: String::from("/"),
        }
    }

    /// Create a `ProcessCwd` with a specific initial directory.
    pub fn with_path(path: &str) -> Result<Self, KernelError> {
        if !path.starts_with('/') {
            return Err(KernelError::InvalidArgument {
                name: "path",
                value: "initial CWD must be an absolute path",
            });
        }
        let normalized = normalize_path(path);
        Ok(Self { path: normalized })
    }

    /// Get the current working directory.
    pub fn get(&self) -> &str {
        &self.path
    }

    /// Set the current working directory.
    ///
    /// The path must be absolute. It is normalized before storage.
    pub fn set(&mut self, path: &str) -> Result<(), KernelError> {
        if !path.starts_with('/') {
            return Err(KernelError::InvalidArgument {
                name: "path",
                value: "CWD must be an absolute path",
            });
        }

        if path.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "path",
                value: "path cannot be empty",
            });
        }

        self.path = normalize_path(path);
        Ok(())
    }

    /// Resolve a potentially relative path against this CWD.
    ///
    /// - Absolute paths (starting with `/`) are returned normalized.
    /// - Relative paths are joined with the CWD and normalized.
    pub fn resolve(&self, relative: &str) -> String {
        resolve_path(relative, &self.path)
    }
}

// ---------------------------------------------------------------------------
// Path Resolution and Normalization (free functions)
// ---------------------------------------------------------------------------

/// Resolve a potentially relative path against a given working directory.
///
/// - If `path` starts with `/`, it is treated as absolute and normalized.
/// - Otherwise, `path` is appended to `cwd` with a `/` separator and
///   normalized.
#[cfg(feature = "alloc")]
pub fn resolve_path(path: &str, cwd: &str) -> String {
    if path.starts_with('/') {
        // Absolute path -- just normalize.
        normalize_path(path)
    } else {
        // Relative path -- join with CWD.
        let mut combined = String::with_capacity(cwd.len() + 1 + path.len());
        combined.push_str(cwd);
        if !cwd.ends_with('/') {
            combined.push('/');
        }
        combined.push_str(path);
        normalize_path(&combined)
    }
}

/// Normalize a path by collapsing redundant separators and resolving `.` and
/// `..`.
///
/// The result is always an absolute path starting with `/`. Trailing slashes
/// are removed (except for the root `/` itself).
#[cfg(feature = "alloc")]
pub fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => {
                // Skip empty segments (from `//`) and current-dir markers.
            }
            ".." => {
                // Go up one level, but never above root.
                components.pop();
            }
            other => {
                components.push(other);
            }
        }
    }

    if components.is_empty() {
        return String::from("/");
    }

    let mut result = String::with_capacity(path.len());
    for component in &components {
        result.push('/');
        result.push_str(component);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- normalize_path tests ---

    #[test]
    fn test_normalize_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn test_normalize_simple() {
        assert_eq!(normalize_path("/usr/bin"), "/usr/bin");
    }

    #[test]
    fn test_normalize_trailing_slash() {
        assert_eq!(normalize_path("/usr/bin/"), "/usr/bin");
    }

    #[test]
    fn test_normalize_double_slash() {
        assert_eq!(normalize_path("/usr//bin"), "/usr/bin");
    }

    #[test]
    fn test_normalize_triple_slash() {
        assert_eq!(normalize_path("///"), "/");
    }

    #[test]
    fn test_normalize_dot() {
        assert_eq!(normalize_path("/usr/./bin"), "/usr/bin");
    }

    #[test]
    fn test_normalize_dotdot() {
        assert_eq!(normalize_path("/usr/local/../bin"), "/usr/bin");
    }

    #[test]
    fn test_normalize_dotdot_at_root() {
        assert_eq!(normalize_path("/.."), "/");
    }

    #[test]
    fn test_normalize_multiple_dotdot() {
        assert_eq!(normalize_path("/a/b/c/../../d"), "/a/d");
    }

    #[test]
    fn test_normalize_complex() {
        assert_eq!(normalize_path("/usr//local/../bin/./gcc"), "/usr/bin/gcc");
    }

    #[test]
    fn test_normalize_all_dotdot() {
        assert_eq!(normalize_path("/a/b/../../.."), "/");
    }

    // --- resolve_path tests ---

    #[test]
    fn test_resolve_absolute() {
        assert_eq!(resolve_path("/etc/hosts", "/home"), "/etc/hosts");
    }

    #[test]
    fn test_resolve_relative_simple() {
        assert_eq!(resolve_path("foo", "/home"), "/home/foo");
    }

    #[test]
    fn test_resolve_relative_nested() {
        assert_eq!(resolve_path("foo/bar", "/home"), "/home/foo/bar");
    }

    #[test]
    fn test_resolve_relative_dotdot() {
        assert_eq!(resolve_path("../bin", "/usr/local"), "/usr/bin");
    }

    #[test]
    fn test_resolve_dot() {
        assert_eq!(resolve_path(".", "/var/log"), "/var/log");
    }

    #[test]
    fn test_resolve_relative_from_root() {
        assert_eq!(resolve_path("usr/bin", "/"), "/usr/bin");
    }

    #[test]
    fn test_resolve_dotdot_past_root() {
        assert_eq!(resolve_path("../../..", "/a"), "/");
    }

    // --- ProcessCwd tests ---

    #[test]
    fn test_process_cwd_default() {
        let cwd = ProcessCwd::new();
        assert_eq!(cwd.get(), "/");
    }

    #[test]
    fn test_process_cwd_with_path() {
        let cwd = ProcessCwd::with_path("/home/user").unwrap();
        assert_eq!(cwd.get(), "/home/user");
    }

    #[test]
    fn test_process_cwd_with_path_relative_fails() {
        let result = ProcessCwd::with_path("relative/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_process_cwd_set() {
        let mut cwd = ProcessCwd::new();
        assert!(cwd.set("/home/user").is_ok());
        assert_eq!(cwd.get(), "/home/user");
    }

    #[test]
    fn test_process_cwd_set_normalizes() {
        let mut cwd = ProcessCwd::new();
        assert!(cwd.set("/home//user/../admin/./docs").is_ok());
        assert_eq!(cwd.get(), "/home/admin/docs");
    }

    #[test]
    fn test_process_cwd_set_relative_fails() {
        let mut cwd = ProcessCwd::new();
        let result = cwd.set("relative");
        assert!(result.is_err());
    }

    #[test]
    fn test_process_cwd_resolve_absolute() {
        let cwd = ProcessCwd::with_path("/home/user").unwrap();
        assert_eq!(cwd.resolve("/etc/passwd"), "/etc/passwd");
    }

    #[test]
    fn test_process_cwd_resolve_relative() {
        let cwd = ProcessCwd::with_path("/home/user").unwrap();
        assert_eq!(
            cwd.resolve("Documents/file.txt"),
            "/home/user/Documents/file.txt"
        );
    }

    #[test]
    fn test_process_cwd_resolve_dotdot() {
        let cwd = ProcessCwd::with_path("/home/user").unwrap();
        assert_eq!(cwd.resolve("../admin"), "/home/admin");
    }
}
