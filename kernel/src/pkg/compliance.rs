//! License Compliance and Dependency Graph Analysis
//!
//! Provides license detection from text, compatibility checking between
//! license pairs, and dependency graph operations including reverse
//! dependency lookup, circular dependency detection, and depth calculation.
//!
//! NOTE: Many types in this module are forward declarations for user-space
//! APIs. They will be exercised when user-space process execution is
//! functional. See TODO(user-space) markers for specific activation points.

// User-space API forward declarations -- see NOTE above

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ============================================================================
// License Types and Detection
// ============================================================================

/// Known open-source and proprietary license identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum License {
    /// MIT License
    MIT,
    /// Apache License 2.0
    Apache2,
    /// GNU General Public License v2.0
    GPL2,
    /// GNU General Public License v3.0
    GPL3,
    /// GNU Lesser General Public License v2.1
    LGPL21,
    /// BSD 2-Clause "Simplified" License
    BSD2,
    /// BSD 3-Clause "New" or "Revised" License
    BSD3,
    /// ISC License
    ISC,
    /// Mozilla Public License 2.0
    MPL2,
    /// Proprietary / closed-source
    Proprietary,
    /// License could not be determined
    Unknown,
}

#[cfg(feature = "alloc")]
impl License {
    /// Return the SPDX-style identifier string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MIT => "MIT",
            Self::Apache2 => "Apache-2.0",
            Self::GPL2 => "GPL-2.0",
            Self::GPL3 => "GPL-3.0",
            Self::LGPL21 => "LGPL-2.1",
            Self::BSD2 => "BSD-2-Clause",
            Self::BSD3 => "BSD-3-Clause",
            Self::ISC => "ISC",
            Self::MPL2 => "MPL-2.0",
            Self::Proprietary => "Proprietary",
            Self::Unknown => "Unknown",
        }
    }

    /// Parse a license from an SPDX-style identifier string.
    pub fn from_spdx(s: &str) -> Self {
        match s.trim() {
            "MIT" => Self::MIT,
            "Apache-2.0" | "Apache2" => Self::Apache2,
            "GPL-2.0" | "GPL-2.0-only" | "GPL2" => Self::GPL2,
            "GPL-3.0" | "GPL-3.0-only" | "GPL3" => Self::GPL3,
            "LGPL-2.1" | "LGPL-2.1-only" => Self::LGPL21,
            "BSD-2-Clause" | "BSD2" => Self::BSD2,
            "BSD-3-Clause" | "BSD3" => Self::BSD3,
            "ISC" => Self::ISC,
            "MPL-2.0" | "MPL2" => Self::MPL2,
            "Proprietary" => Self::Proprietary,
            _ => Self::Unknown,
        }
    }

    /// Return whether this license is copyleft (restricts derivative works).
    pub fn is_copyleft(self) -> bool {
        matches!(self, Self::GPL2 | Self::GPL3 | Self::LGPL21 | Self::MPL2)
    }

    /// Return whether this license is permissive.
    pub fn is_permissive(self) -> bool {
        matches!(
            self,
            Self::MIT | Self::Apache2 | Self::BSD2 | Self::BSD3 | Self::ISC
        )
    }
}

/// Detect a license from the full text of a LICENSE file.
///
/// Uses keyword matching to identify the license. Returns `License::Unknown`
/// if no known license pattern is matched.
#[cfg(feature = "alloc")]
pub fn detect_license(text: &str) -> License {
    // Normalize to lowercase for case-insensitive matching
    let lower = text.to_lowercase();

    // Check for GPL variants first (more specific before less specific)
    if lower.contains("gnu general public license") || lower.contains("gpl") {
        if lower.contains("version 3") || lower.contains("gpl-3") || lower.contains("gplv3") {
            return License::GPL3;
        }
        if lower.contains("version 2") || lower.contains("gpl-2") || lower.contains("gplv2") {
            return License::GPL2;
        }
        // Generic GPL reference without version defaults to GPL3
        if lower.contains("general public license") {
            return License::GPL3;
        }
    }

    // Check for LGPL
    if (lower.contains("lesser general public license") || lower.contains("lgpl"))
        && (lower.contains("2.1") || lower.contains("lgpl-2.1"))
    {
        return License::LGPL21;
    }

    // Check for Apache
    if (lower.contains("apache license") || lower.contains("apache-2"))
        && (lower.contains("version 2") || lower.contains("2.0") || lower.contains("apache-2"))
    {
        return License::Apache2;
    }

    // Check for MIT
    if lower.contains("mit license")
        || lower.contains("permission is hereby granted, free of charge")
    {
        return License::MIT;
    }

    // Check for BSD variants
    if lower.contains("bsd") {
        if lower.contains("2-clause") || lower.contains("simplified") {
            return License::BSD2;
        }
        if lower.contains("3-clause") || lower.contains("new") || lower.contains("revised") {
            return License::BSD3;
        }
        // Generic BSD reference defaults to BSD3
        return License::BSD3;
    }

    // Check for ISC
    if lower.contains("isc license")
        || lower.contains("permission to use, copy, modify, and/or distribute")
    {
        return License::ISC;
    }

    // Check for MPL
    if lower.contains("mozilla public license") || lower.contains("mpl-2") {
        return License::MPL2;
    }

    // Check for proprietary indicators
    if lower.contains("proprietary")
        || lower.contains("all rights reserved")
        || lower.contains("no permission")
    {
        return License::Proprietary;
    }

    License::Unknown
}

// ============================================================================
// License Compatibility
// ============================================================================

/// A conflict between two package licenses.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct LicenseConflict {
    /// Name of the first package.
    pub package_a: String,
    /// License of the first package.
    pub license_a: License,
    /// Name of the second package.
    pub package_b: String,
    /// License of the second package.
    pub license_b: License,
    /// Explanation of why these licenses conflict.
    pub reason: String,
}

/// License compatibility checker.
///
/// Determines whether two licenses can coexist in the same dependency tree
/// based on their distribution requirements.
#[cfg(feature = "alloc")]
pub struct LicenseCompatibility;

#[cfg(feature = "alloc")]
impl LicenseCompatibility {
    /// Check if two licenses are compatible for co-distribution.
    ///
    /// Rules:
    /// - Proprietary is incompatible with GPL2, GPL3, and LGPL21.
    /// - GPL3 is incompatible with GPL2 (GPL2-only cannot upgrade to GPL3).
    /// - Permissive licenses (MIT, BSD, ISC, Apache2) are compatible with
    ///   everything except Proprietary.
    /// - Unknown licenses are treated as compatible (best-effort).
    pub fn is_compatible(a: &License, b: &License) -> bool {
        // Unknown is treated as compatible (best-effort)
        if *a == License::Unknown || *b == License::Unknown {
            return true;
        }

        // Proprietary is incompatible with copyleft
        if *a == License::Proprietary {
            return !matches!(b, License::GPL2 | License::GPL3 | License::LGPL21);
        }
        if *b == License::Proprietary {
            return !matches!(a, License::GPL2 | License::GPL3 | License::LGPL21);
        }

        // GPL3 is incompatible with GPL2-only
        if (*a == License::GPL3 && *b == License::GPL2)
            || (*a == License::GPL2 && *b == License::GPL3)
        {
            return false;
        }

        // All other combinations are compatible
        true
    }
}

/// Check all license pairs in a dependency list for compatibility.
///
/// Returns `Ok(())` if all pairs are compatible, or `Err` with a list of
/// conflicts found.
#[cfg(feature = "alloc")]
pub fn check_compatibility(deps: &[(String, License)]) -> Result<(), Vec<LicenseConflict>> {
    let mut conflicts = Vec::new();

    for i in 0..deps.len() {
        for j in (i + 1)..deps.len() {
            let (ref name_a, ref license_a) = deps[i];
            let (ref name_b, ref license_b) = deps[j];

            if !LicenseCompatibility::is_compatible(license_a, license_b) {
                let reason = alloc::format!(
                    "{} ({}) is incompatible with {} ({})",
                    license_a.as_str(),
                    name_a,
                    license_b.as_str(),
                    name_b
                );
                conflicts.push(LicenseConflict {
                    package_a: name_a.clone(),
                    license_a: *license_a,
                    package_b: name_b.clone(),
                    license_b: *license_b,
                    reason,
                });
            }
        }
    }

    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(conflicts)
    }
}

// ============================================================================
// Dependency Graph
// ============================================================================

/// A directed dependency graph for packages.
///
/// Nodes are package names; edges point from a package to its dependencies.
/// Supports reverse dependency lookup, circular dependency detection, and
/// dependency depth calculation.
#[cfg(feature = "alloc")]
pub struct DependencyGraph {
    /// Adjacency list: package -> list of its dependencies.
    nodes: BTreeMap<String, Vec<String>>,
}

#[cfg(feature = "alloc")]
impl DependencyGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
        }
    }

    /// Add a package node to the graph (with no dependencies initially).
    pub fn add_package(&mut self, name: &str) {
        self.nodes
            .entry(String::from(name))
            .or_insert_with(Vec::new);
    }

    /// Add a dependency edge: `package` depends on `dependency`.
    ///
    /// Both nodes are created if they do not already exist.
    pub fn add_dependency(&mut self, package: &str, dependency: &str) {
        self.nodes
            .entry(String::from(package))
            .or_insert_with(Vec::new)
            .push(String::from(dependency));
        // Ensure the dependency node exists
        self.nodes
            .entry(String::from(dependency))
            .or_insert_with(Vec::new);
    }

    /// Find all packages that depend on the given package (reverse
    /// dependencies).
    pub fn find_reverse_deps(&self, package: &str) -> Vec<String> {
        let mut reverse = Vec::new();
        for (node, deps) in &self.nodes {
            if deps.iter().any(|d| d == package) {
                reverse.push(node.clone());
            }
        }
        reverse
    }

    /// Detect circular dependencies in the graph via DFS.
    ///
    /// Returns a list of cycles, where each cycle is a list of package names
    /// forming the cycle. An empty result means no cycles exist.
    pub fn detect_circular_deps(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited: BTreeMap<String, bool> = BTreeMap::new();
        let mut in_stack: BTreeMap<String, bool> = BTreeMap::new();
        let mut path: Vec<String> = Vec::new();

        for node in self.nodes.keys() {
            if !visited.get(node).copied().unwrap_or(false) {
                self.dfs_detect_cycles(node, &mut visited, &mut in_stack, &mut path, &mut cycles);
            }
        }

        cycles
    }

    /// DFS helper for cycle detection.
    fn dfs_detect_cycles(
        &self,
        node: &str,
        visited: &mut BTreeMap<String, bool>,
        in_stack: &mut BTreeMap<String, bool>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(String::from(node), true);
        in_stack.insert(String::from(node), true);
        path.push(String::from(node));

        if let Some(deps) = self.nodes.get(node) {
            for dep in deps {
                if !visited.get(dep).copied().unwrap_or(false) {
                    self.dfs_detect_cycles(dep, visited, in_stack, path, cycles);
                } else if in_stack.get(dep).copied().unwrap_or(false) {
                    // Found a cycle: extract the cycle from the path
                    let mut cycle = Vec::new();
                    let mut found = false;
                    for p in path.iter() {
                        if p == dep {
                            found = true;
                        }
                        if found {
                            cycle.push(p.clone());
                        }
                    }
                    cycle.push(dep.clone()); // Close the cycle
                    cycles.push(cycle);
                }
            }
        }

        path.pop();
        in_stack.insert(String::from(node), false);
    }

    /// Compute the dependency depth of a package.
    ///
    /// The depth is the longest path from any root (a package with no
    /// reverse dependencies) to this package. Returns 0 if the package
    /// is a root or is not in the graph.
    pub fn dependency_depth(&self, package: &str) -> usize {
        if !self.nodes.contains_key(package) {
            return 0;
        }

        // BFS from all roots, tracking distance
        let roots = self.find_roots();
        if roots.is_empty() {
            return 0;
        }

        let mut max_depth = 0;

        for root in &roots {
            let depth = self.bfs_depth(root, package);
            if depth > max_depth {
                max_depth = depth;
            }
        }

        max_depth
    }

    /// Find root nodes (packages that no other package depends on).
    pub fn find_roots(&self) -> Vec<String> {
        let mut roots = Vec::new();
        for node in self.nodes.keys() {
            let has_reverse = self
                .nodes
                .values()
                .any(|deps| deps.iter().any(|d| d == node));
            if !has_reverse {
                roots.push(node.clone());
            }
        }
        roots
    }

    /// BFS to find the longest path from `start` to `target`.
    ///
    /// Returns 0 if `target` is not reachable from `start`.
    fn bfs_depth(&self, start: &str, target: &str) -> usize {
        // Use a simple recursive DFS to find the longest path
        let mut visited = BTreeMap::new();
        self.dfs_longest_path(start, target, &mut visited)
    }

    /// DFS helper to find the longest path length from `current` to `target`.
    fn dfs_longest_path(
        &self,
        current: &str,
        target: &str,
        visited: &mut BTreeMap<String, bool>,
    ) -> usize {
        if current == target {
            return 0;
        }

        if visited.get(current).copied().unwrap_or(false) {
            return 0;
        }

        visited.insert(String::from(current), true);

        let mut max_depth = 0;
        let mut found = false;

        if let Some(deps) = self.nodes.get(current) {
            for dep in deps {
                let depth = self.dfs_longest_path(dep, target, visited);
                if dep == target || depth > 0 {
                    found = true;
                    let candidate = 1 + depth;
                    if candidate > max_depth {
                        max_depth = candidate;
                    }
                }
            }
        }

        visited.insert(String::from(current), false);

        if found {
            max_depth
        } else {
            0
        }
    }

    /// Return the total number of packages in the graph.
    pub fn package_count(&self) -> usize {
        self.nodes.len()
    }

    /// Return the total number of dependency edges.
    pub fn edge_count(&self) -> usize {
        self.nodes.values().map(|deps| deps.len()).sum()
    }

    /// Return the direct dependencies of a package.
    pub fn dependencies(&self, package: &str) -> Option<&[String]> {
        self.nodes.get(package).map(|v| v.as_slice())
    }
}

#[cfg(feature = "alloc")]
impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}
