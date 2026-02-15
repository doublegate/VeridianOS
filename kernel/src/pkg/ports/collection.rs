//! Port Collection Management
//!
//! Organises ports into categories (e.g., core, devel, libs, net,
//! security, utils) and provides search and synchronisation capabilities.
//! A `PortCollection` is the top-level directory of available ports,
//! analogous to the ports tree in BSD-like operating systems.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[cfg(feature = "alloc")]
use crate::error::KernelError;

/// Standard port categories shipped with VeridianOS.
#[cfg(feature = "alloc")]
pub const STANDARD_CATEGORIES: &[&str] = &[
    "core",     // Essential system packages
    "devel",    // Development tools and libraries
    "libs",     // Shared / static libraries
    "net",      // Networking utilities and daemons
    "security", // Security tools and cryptographic software
    "utils",    // General-purpose utilities
];

/// A categorised collection of available ports.
///
/// Each category maps to a list of port names that belong to it. The
/// collection is typically populated by scanning `/usr/ports/` or by
/// syncing with a remote ports index.
#[cfg(feature = "alloc")]
pub struct PortCollection {
    /// Category -> list of port names
    categories: BTreeMap<String, Vec<String>>,
    /// Last sync timestamp (seconds since boot / epoch)
    last_sync: u64,
}

#[cfg(feature = "alloc")]
impl PortCollection {
    /// Create a new, empty collection with the standard categories
    /// pre-registered.
    pub fn new() -> Self {
        let mut categories = BTreeMap::new();
        for &cat in STANDARD_CATEGORIES {
            categories.insert(String::from(cat), Vec::new());
        }
        Self {
            categories,
            last_sync: 0,
        }
    }

    /// Add a port to a category. If the category does not exist it is
    /// created automatically.
    pub fn add_port(&mut self, category: &str, port_name: &str) {
        let list = self
            .categories
            .entry(String::from(category))
            .or_insert_with(Vec::new);

        // Avoid duplicates
        if !list.iter().any(|n| n == port_name) {
            list.push(String::from(port_name));
        }
    }

    /// Remove a port from a category. Returns `true` if the port was found
    /// and removed, `false` otherwise.
    pub fn remove_port(&mut self, category: &str, port_name: &str) -> bool {
        if let Some(list) = self.categories.get_mut(category) {
            if let Some(pos) = list.iter().position(|n| n == port_name) {
                list.remove(pos);
                return true;
            }
        }
        false
    }

    /// List all known category names (sorted).
    pub fn list_categories(&self) -> Vec<&str> {
        self.categories.keys().map(|s| s.as_str()).collect()
    }

    /// List port names within a specific category.
    pub fn list_ports_in_category(&self, category: &str) -> Option<&[String]> {
        self.categories.get(category).map(|v| v.as_slice())
    }

    /// Search all categories for ports whose name contains `query`
    /// (case-insensitive). Returns `(category, port_name)` pairs.
    pub fn search_ports(&self, query: &str) -> Vec<(String, String)> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for (category, ports) in &self.categories {
            for port_name in ports {
                if port_name.to_lowercase().contains(&query_lower) {
                    results.push((category.clone(), port_name.clone()));
                }
            }
        }

        results
    }

    /// Return the total number of ports across all categories.
    pub fn total_ports(&self) -> usize {
        self.categories.values().map(|v| v.len()).sum()
    }

    /// Return the number of categories.
    pub fn category_count(&self) -> usize {
        self.categories.len()
    }

    /// Synchronise the collection from the ports tree on disk.
    ///
    /// First attempts to scan `/usr/ports/` via the VFS to discover real
    /// ports. If the VFS is not available or no ports are found, falls back
    /// to a set of well-known demo ports so the framework is exercisable
    /// even without a real filesystem.
    ///
    /// Returns the number of ports discovered.
    pub fn sync_collection(&mut self) -> Result<usize, KernelError> {
        crate::println!("[PORTS] Syncing port collection from /usr/ports/ ...");

        // Try scanning the VFS first
        let vfs_count = self.scan_ports_directory();

        if vfs_count > 0 {
            // VFS scan found ports -- use those
            self.last_sync = crate::arch::timer::get_timestamp_secs();

            crate::println!(
                "[PORTS] Sync complete (VFS): {} ports in {} categories",
                self.total_ports(),
                self.category_count()
            );

            return Ok(vfs_count);
        }

        // Fallback: register a set of well-known ports for demonstration
        // so the framework is exercisable even without a real filesystem.
        crate::println!("[PORTS] No VFS ports found, loading demo ports");

        let demo_ports: &[(&str, &str)] = &[
            ("core", "coreutils"),
            ("core", "bash"),
            ("core", "grep"),
            ("core", "sed"),
            ("devel", "gcc"),
            ("devel", "make"),
            ("devel", "cmake"),
            ("devel", "git"),
            ("libs", "openssl"),
            ("libs", "zlib"),
            ("libs", "libpng"),
            ("net", "curl"),
            ("net", "wget"),
            ("net", "openssh"),
            ("security", "gnupg"),
            ("security", "nmap"),
            ("utils", "vim"),
            ("utils", "tmux"),
            ("utils", "htop"),
        ];

        let mut count = 0;
        for &(category, name) in demo_ports {
            self.add_port(category, name);
            count += 1;
        }

        self.last_sync = crate::arch::timer::get_timestamp_secs();

        crate::println!(
            "[PORTS] Sync complete (demo): {} ports in {} categories",
            self.total_ports(),
            self.category_count()
        );

        Ok(count)
    }

    /// Scan the `/usr/ports/` directory tree via VFS to discover ports.
    ///
    /// Expects the layout `/usr/ports/<category>/<port_name>/`. Each
    /// sub-directory under a category is registered as a port in that
    /// category. Returns the number of ports discovered (0 if VFS is
    /// unavailable or the directory does not exist).
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
    fn scan_ports_directory(&mut self) -> usize {
        let vfs_lock = match crate::fs::try_get_vfs() {
            Some(lock) => lock,
            None => {
                crate::println!("[PORTS] VFS not available, skipping directory scan");
                return 0;
            }
        };

        let vfs = vfs_lock.read();
        let ports_root = match vfs.resolve_path("/usr/ports") {
            Ok(node) => node,
            Err(_) => {
                crate::println!("[PORTS] /usr/ports not found in VFS");
                return 0;
            }
        };

        // List category directories
        let category_entries = match ports_root.readdir() {
            Ok(entries) => entries,
            Err(_) => {
                crate::println!("[PORTS] Cannot list /usr/ports directory");
                return 0;
            }
        };

        let mut total = 0;

        for cat_entry in &category_entries {
            // Only process directories (categories)
            if cat_entry.node_type != crate::fs::NodeType::Directory {
                continue;
            }

            let category = &cat_entry.name;
            let cat_path = alloc::format!("/usr/ports/{}", category);

            // List port directories within this category
            let cat_node = match vfs.resolve_path(&cat_path) {
                Ok(n) => n,
                Err(_) => continue,
            };

            let port_entries = match cat_node.readdir() {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for port_entry in &port_entries {
                if port_entry.node_type != crate::fs::NodeType::Directory {
                    continue;
                }

                self.add_port(category, &port_entry.name);
                total += 1;
            }
        }

        if total > 0 {
            crate::println!(
                "[PORTS] VFS scan discovered {} ports under /usr/ports/",
                total
            );
        }

        total
    }

    /// Return the timestamp (seconds) of the last successful sync.
    pub fn last_sync_time(&self) -> u64 {
        self.last_sync
    }

    /// Check whether the collection has been synced at least once.
    pub fn is_synced(&self) -> bool {
        self.last_sync > 0 || self.total_ports() > 0
    }
}

#[cfg(feature = "alloc")]
impl Default for PortCollection {
    fn default() -> Self {
        Self::new()
    }
}
