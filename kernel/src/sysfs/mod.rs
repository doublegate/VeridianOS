//! Virtual sysfs filesystem for VeridianOS.
//!
//! Provides a Linux-compatible sysfs interface for kernel subsystem
//! configuration and status. Virtual files under `/sys/` expose kernel
//! parameters to user-space tools and desktop environments (e.g., KDE
//! PowerDevil reads `/sys/power/state` and `/sys/class/backlight/`).
//!
//! Each sysfs node is a `SysfsNode` with read and/or write handlers.
//! Nodes are registered at init time and looked up by path.

#![allow(dead_code)]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

pub mod power;

// ---------------------------------------------------------------------------
// Sysfs node definitions
// ---------------------------------------------------------------------------

/// Maximum number of sysfs nodes.
const MAX_SYSFS_NODES: usize = 64;

/// Type alias for sysfs read handler.
/// Returns the contents of the virtual file as a String.
type SysfsReadFn = fn() -> String;

/// Type alias for sysfs write handler.
/// Receives the value written to the virtual file.
/// Returns Ok(()) on success or an error.
type SysfsWriteFn = fn(&str) -> KernelResult<()>;

/// A virtual sysfs filesystem node.
#[derive(Clone)]
pub struct SysfsNode {
    /// Full path (e.g., "/sys/power/state").
    pub path: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Read handler (None = write-only).
    pub read_fn: Option<SysfsReadFn>,
    /// Write handler (None = read-only).
    pub write_fn: Option<SysfsWriteFn>,
}

impl SysfsNode {
    /// Create a new read-only sysfs node.
    pub const fn read_only(
        path: &'static str,
        description: &'static str,
        read_fn: SysfsReadFn,
    ) -> Self {
        Self {
            path,
            description,
            read_fn: Some(read_fn),
            write_fn: None,
        }
    }

    /// Create a new read-write sysfs node.
    pub const fn read_write(
        path: &'static str,
        description: &'static str,
        read_fn: SysfsReadFn,
        write_fn: SysfsWriteFn,
    ) -> Self {
        Self {
            path,
            description,
            read_fn: Some(read_fn),
            write_fn: Some(write_fn),
        }
    }

    /// Whether this node is writable.
    pub fn is_writable(&self) -> bool {
        self.write_fn.is_some()
    }

    /// Whether this node is readable.
    pub fn is_readable(&self) -> bool {
        self.read_fn.is_some()
    }
}

/// Trait for sysfs entry registration.
pub trait SysfsEntry {
    /// Returns the path of this sysfs entry.
    fn path(&self) -> &str;

    /// Read the sysfs entry value.
    fn read(&self) -> KernelResult<String>;

    /// Write a value to the sysfs entry.
    fn write(&self, value: &str) -> KernelResult<()>;
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

struct SysfsRegistry {
    nodes: Vec<SysfsNode>,
}

impl SysfsRegistry {
    const fn new() -> Self {
        Self { nodes: Vec::new() }
    }
}

static SYSFS_REGISTRY: Mutex<SysfsRegistry> = Mutex::new(SysfsRegistry::new());

// ---------------------------------------------------------------------------
// Registration API
// ---------------------------------------------------------------------------

/// Register a sysfs node.
pub fn register_node(node: SysfsNode) -> KernelResult<()> {
    let mut registry = SYSFS_REGISTRY.lock();

    if registry.nodes.len() >= MAX_SYSFS_NODES {
        return Err(KernelError::ResourceExhausted {
            resource: "sysfs nodes",
        });
    }

    // Check for duplicate paths.
    for existing in &registry.nodes {
        if existing.path == node.path {
            return Err(KernelError::AlreadyExists {
                resource: "sysfs node",
                id: 0,
            });
        }
    }

    println!("[SYSFS] Registered: {}", node.path);
    registry.nodes.push(node);
    Ok(())
}

/// Look up a sysfs node by path.
pub fn lookup(path: &str) -> Option<SysfsNode> {
    let registry = SYSFS_REGISTRY.lock();
    for node in &registry.nodes {
        if node.path == path {
            return Some(node.clone());
        }
    }
    None
}

/// Read a sysfs virtual file.
pub fn sysfs_read(path: &str) -> KernelResult<String> {
    let node = lookup(path).ok_or(KernelError::NotFound {
        resource: "sysfs",
        id: 0,
    })?;

    let read_fn = node.read_fn.ok_or(KernelError::PermissionDenied {
        operation: "sysfs read (write-only node)",
    })?;

    Ok(read_fn())
}

/// Write to a sysfs virtual file.
pub fn sysfs_write(path: &str, value: &str) -> KernelResult<()> {
    let node = lookup(path).ok_or(KernelError::NotFound {
        resource: "sysfs",
        id: 0,
    })?;

    let write_fn = node.write_fn.ok_or(KernelError::PermissionDenied {
        operation: "sysfs write (read-only node)",
    })?;

    write_fn(value)
}

/// List all registered sysfs nodes.
pub fn list_nodes() -> Vec<&'static str> {
    let registry = SYSFS_REGISTRY.lock();
    registry.nodes.iter().map(|n| n.path).collect()
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the sysfs subsystem and register all built-in nodes.
pub fn sysfs_init() -> KernelResult<()> {
    println!("[SYSFS] Initializing virtual filesystem...");

    // Register power-related sysfs nodes.
    power::sysfs_power_init()?;

    let count = {
        let registry = SYSFS_REGISTRY.lock();
        registry.nodes.len()
    };

    println!("[SYSFS] Initialized: {} nodes registered", count);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_read() -> String {
        String::from("test_value")
    }

    fn test_write(value: &str) -> KernelResult<()> {
        if value.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "value",
                value: "empty",
            });
        }
        Ok(())
    }

    #[test]
    fn test_sysfs_node_read_only() {
        let node = SysfsNode::read_only("/test/ro", "test read-only", test_read);
        assert!(node.is_readable());
        assert!(!node.is_writable());
        assert_eq!(node.path, "/test/ro");
    }

    #[test]
    fn test_sysfs_node_read_write() {
        let node = SysfsNode::read_write("/test/rw", "test read-write", test_read, test_write);
        assert!(node.is_readable());
        assert!(node.is_writable());
    }

    #[test]
    fn test_sysfs_node_read_fn() {
        let node = SysfsNode::read_only("/test/read", "test", test_read);
        let val = (node.read_fn.unwrap())();
        assert_eq!(val, "test_value");
    }

    #[test]
    fn test_sysfs_node_write_fn() {
        let node = SysfsNode::read_write("/test/write", "test", test_read, test_write);
        let result = (node.write_fn.unwrap())("hello");
        assert!(result.is_ok());
    }

    #[test]
    fn test_sysfs_node_write_empty_error() {
        let node = SysfsNode::read_write("/test/write_err", "test", test_read, test_write);
        let result = (node.write_fn.unwrap())("");
        assert!(result.is_err());
    }
}
