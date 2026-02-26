//! Package Statistics, Update Notifications, and Security Advisories
//!
//! Tracks package installation metrics, detects available updates by comparing
//! installed vs available package versions, and checks installed packages
//! against security advisories.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space API forward declarations -- see NOTE above

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[cfg(feature = "alloc")]
use super::{PackageMetadata, Version};

// ---------------------------------------------------------------------------
// Package Statistics
// ---------------------------------------------------------------------------

/// Per-package usage and installation statistics.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PackageStats {
    /// Number of times this package has been installed
    pub install_count: u64,
    /// Timestamp of the most recent installation (seconds since epoch)
    pub last_installed: u64,
    /// Timestamp of the most recent update (seconds since epoch)
    pub last_updated: u64,
    /// Total number of times the package has been downloaded
    pub total_downloads: u64,
}

#[cfg(feature = "alloc")]
impl PackageStats {
    /// Create a new zeroed statistics entry.
    pub fn new() -> Self {
        Self {
            install_count: 0,
            last_installed: 0,
            last_updated: 0,
            total_downloads: 0,
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for PackageStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Collects and queries per-package statistics.
#[cfg(feature = "alloc")]
pub struct StatsCollector {
    /// Per-package statistics keyed by package name.
    stats: BTreeMap<String, PackageStats>,
}

#[cfg(feature = "alloc")]
impl StatsCollector {
    /// Create a new empty stats collector.
    pub fn new() -> Self {
        Self {
            stats: BTreeMap::new(),
        }
    }

    /// Record a package installation event.
    pub fn record_install(&mut self, package_name: &str, timestamp: u64) {
        let entry = self
            .stats
            .entry(String::from(package_name))
            .or_insert_with(PackageStats::new);
        entry.install_count += 1;
        entry.last_installed = timestamp;
    }

    /// Record a package update event.
    pub fn record_update(&mut self, package_name: &str, timestamp: u64) {
        let entry = self
            .stats
            .entry(String::from(package_name))
            .or_insert_with(PackageStats::new);
        entry.last_updated = timestamp;
    }

    /// Record a package download event.
    pub fn record_download(&mut self, package_name: &str) {
        let entry = self
            .stats
            .entry(String::from(package_name))
            .or_insert_with(PackageStats::new);
        entry.total_downloads += 1;
    }

    /// Retrieve statistics for a specific package, if any.
    pub fn get_stats(&self, package_name: &str) -> Option<&PackageStats> {
        self.stats.get(package_name)
    }

    /// Return the top `n` most-installed packages sorted by install count
    /// (descending).
    pub fn get_most_installed(&self, n: usize) -> Vec<(&str, u64)> {
        let mut entries: Vec<(&str, u64)> = self
            .stats
            .iter()
            .map(|(name, s)| (name.as_str(), s.install_count))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }

    /// Return the total number of tracked packages.
    pub fn total_packages(&self) -> usize {
        self.stats.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for StatsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Update Notifications
// ---------------------------------------------------------------------------

/// Notification that an installed package has a newer version available.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct UpdateNotification {
    /// Package name
    pub package: String,
    /// Currently installed version
    pub current_version: Version,
    /// Version available in the repository
    pub available_version: Version,
    /// Whether this update addresses a security vulnerability
    pub is_security: bool,
    /// Summary of changes in the available version
    pub changelog: String,
}

/// Compare installed packages against available packages and return
/// notifications for packages that have newer versions.
///
/// A notification is flagged as a security update if the available
/// package's description contains "security" (case-insensitive).
#[cfg(feature = "alloc")]
pub fn check_for_updates(
    installed: &[PackageMetadata],
    available: &[PackageMetadata],
) -> Vec<UpdateNotification> {
    let mut notifications = Vec::new();

    // Build a lookup map from available packages by name, keeping the
    // highest version for each name.
    let mut available_map: BTreeMap<&str, &PackageMetadata> = BTreeMap::new();
    for pkg in available {
        let entry = available_map.entry(pkg.name.as_str()).or_insert(pkg);
        if pkg.version > entry.version {
            *entry = pkg;
        }
    }

    for inst in installed {
        if let Some(avail) = available_map.get(inst.name.as_str()) {
            if avail.version > inst.version {
                // Heuristic: flag as security if description mentions "security"
                let desc_lower = avail.description.as_bytes();
                let is_security = contains_ignore_case(desc_lower, b"security");

                notifications.push(UpdateNotification {
                    package: inst.name.clone(),
                    current_version: inst.version.clone(),
                    available_version: avail.version.clone(),
                    is_security,
                    changelog: avail.description.clone(),
                });
            }
        }
    }

    notifications
}

/// Case-insensitive substring search in byte slices.
#[cfg(feature = "alloc")]
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return needle.is_empty();
    }
    for i in 0..=(haystack.len() - needle.len()) {
        let mut matched = true;
        for j in 0..needle.len() {
            if to_ascii_lower(haystack[i + j]) != to_ascii_lower(needle[j]) {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

/// Convert a single ASCII byte to lowercase.
#[cfg(feature = "alloc")]
fn to_ascii_lower(b: u8) -> u8 {
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// Security Advisories
// ---------------------------------------------------------------------------

/// Severity level for a security advisory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdvisorySeverity {
    /// Low impact
    Low,
    /// Medium impact
    Medium,
    /// High impact
    High,
    /// Critical impact -- immediate action recommended
    Critical,
}

/// A security advisory describing a vulnerability that affects one or more
/// packages.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SecurityAdvisory {
    /// Advisory identifier (e.g. "VSA-2026-001")
    pub id: String,
    /// Names of packages affected by this advisory
    pub affected_packages: Vec<String>,
    /// Severity of the vulnerability
    pub severity: AdvisorySeverity,
    /// Human-readable description of the vulnerability
    pub description: String,
    /// Version that fixes the vulnerability, if known
    pub fixed_version: Option<Version>,
}

/// Check installed packages against a list of security advisories.
///
/// Returns all advisories that affect at least one installed package (matched
/// by name).
#[cfg(feature = "alloc")]
pub fn check_advisories(
    installed: &[PackageMetadata],
    advisories: &[SecurityAdvisory],
) -> Vec<SecurityAdvisory> {
    let installed_names: Vec<&str> = installed.iter().map(|p| p.name.as_str()).collect();

    advisories
        .iter()
        .filter(|adv| {
            adv.affected_packages
                .iter()
                .any(|name| installed_names.contains(&name.as_str()))
        })
        .cloned()
        .collect()
}
