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
    /// In a full implementation this would scan `/usr/ports/` via the VFS,
    /// reading each `Portfile.toml` to populate category and port data.
    /// Returns the number of ports discovered.
    pub fn sync_collection(&mut self) -> Result<usize, KernelError> {
        // In a running system:
        // 1. Walk /usr/ports/<category>/<port>/Portfile.toml
        // 2. Parse each Portfile.toml header to extract name + category
        // 3. Populate self.categories

        crate::println!("[PORTS] Syncing port collection from /usr/ports/ ...");

        // Register a set of well-known ports for demonstration so the
        // framework is exercisable even without a real filesystem.
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
            "[PORTS] Sync complete: {} ports in {} categories",
            self.total_ports(),
            self.category_count()
        );

        Ok(count)
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
