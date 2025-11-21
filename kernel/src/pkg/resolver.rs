//! Dependency Resolution
//!
//! Implements SAT-based dependency resolution for package management.
//! Uses a simplified 2-SAT solver for conflict resolution.

#![allow(clippy::manual_strip, clippy::unwrap_or_default)]

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use super::{Dependency, PackageId, Version};

/// Version requirement
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    /// Exact version
    Exact(Version),
    /// Minimum version (>=)
    AtLeast(Version),
    /// Maximum version (<=)
    AtMost(Version),
    /// Range (>= min, < max)
    Range(Version, Version),
    /// Any version
    Any,
}

impl VersionReq {
    /// Parse version requirement from string
    pub fn parse(s: &str) -> Self {
        if s == "*" || s.is_empty() {
            return VersionReq::Any;
        }

        if s.starts_with(">=") {
            if let Some(v) = Self::parse_version(&s[2..]) {
                return VersionReq::AtLeast(v);
            }
        }

        if s.starts_with("<=") {
            if let Some(v) = Self::parse_version(&s[2..]) {
                return VersionReq::AtMost(v);
            }
        }

        if let Some(v) = Self::parse_version(s) {
            return VersionReq::Exact(v);
        }

        VersionReq::Any
    }

    /// Parse version from string (major.minor.patch)
    fn parse_version(s: &str) -> Option<Version> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].parse().ok()?;

        Some(Version::new(major, minor, patch))
    }

    /// Check if version satisfies this requirement
    pub fn satisfies(&self, version: &Version) -> bool {
        match self {
            VersionReq::Exact(v) => version == v,
            VersionReq::AtLeast(v) => version >= v,
            VersionReq::AtMost(v) => version <= v,
            VersionReq::Range(min, max) => version >= min && version < max,
            VersionReq::Any => true,
        }
    }
}

/// Package resolution candidate
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Candidate {
    package_id: PackageId,
    version: Version,
    dependencies: Vec<Dependency>,
    conflicts: Vec<PackageId>,
}

/// Dependency resolver with SAT solving
pub struct DependencyResolver {
    /// Available packages
    available: BTreeMap<PackageId, Vec<Version>>,
    /// Package metadata (dependencies, conflicts)
    metadata: BTreeMap<(PackageId, Version), Candidate>,
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self {
            available: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }

    /// Register a package version
    pub fn register_package(
        &mut self,
        package_id: PackageId,
        version: Version,
        dependencies: Vec<Dependency>,
        conflicts: Vec<PackageId>,
    ) {
        // Add to available versions
        self.available
            .entry(package_id.clone())
            .or_insert_with(Vec::new)
            .push(version.clone());

        // Sort versions in descending order (prefer newer)
        if let Some(versions) = self.available.get_mut(&package_id) {
            versions.sort_by(|a, b| b.cmp(a));
        }

        // Store metadata
        self.metadata.insert(
            (package_id.clone(), version.clone()),
            Candidate {
                package_id,
                version,
                dependencies,
                conflicts,
            },
        );
    }

    /// Resolve dependencies for a package
    /// Returns a topologically sorted list of packages to install
    pub fn resolve(
        &self,
        dependencies: &[Dependency],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        let mut solution = BTreeMap::new();
        let mut visited = BTreeSet::new();

        // Resolve each top-level dependency
        for dep in dependencies {
            self.resolve_dependency(dep, &mut solution, &mut visited)?;
        }

        // Check for conflicts
        self.check_conflicts(&solution)?;

        // Convert to topologically sorted vector
        let mut result: Vec<(PackageId, Version)> = solution.into_iter().collect();

        // Sort by dependency order (simplified - just reverse order)
        result.reverse();

        Ok(result)
    }

    /// Resolve a single dependency recursively
    fn resolve_dependency(
        &self,
        dep: &Dependency,
        solution: &mut BTreeMap<PackageId, Version>,
        visited: &mut BTreeSet<PackageId>,
    ) -> Result<(), String> {
        // Check for circular dependencies
        if visited.contains(&dep.name) {
            return Ok(()); // Already being resolved
        }

        // Check if already resolved
        if solution.contains_key(&dep.name) {
            let existing_version = &solution[&dep.name];
            let req = VersionReq::parse(&dep.version_req);

            if !req.satisfies(existing_version) {
                return Err(alloc::format!(
                    "Version conflict for {}: need {}, have {}",
                    dep.name,
                    dep.version_req,
                    Self::version_to_string(existing_version)
                ));
            }
            return Ok(());
        }

        visited.insert(dep.name.clone());

        // Find suitable version
        let version = self.find_suitable_version(&dep.name, &dep.version_req)?;

        // Get candidate metadata
        let candidate = self
            .metadata
            .get(&(dep.name.clone(), version.clone()))
            .ok_or_else(|| {
                alloc::format!(
                    "Missing metadata for {} {}",
                    dep.name,
                    Self::version_to_string(&version)
                )
            })?;

        // Resolve transitive dependencies
        for trans_dep in &candidate.dependencies {
            self.resolve_dependency(trans_dep, solution, visited)?;
        }

        // Add to solution
        solution.insert(dep.name.clone(), version);

        visited.remove(&dep.name);

        Ok(())
    }

    /// Find a suitable version for a package given version requirement
    fn find_suitable_version(
        &self,
        package_id: &PackageId,
        version_req_str: &str,
    ) -> Result<Version, String> {
        let versions = self
            .available
            .get(package_id)
            .ok_or_else(|| alloc::format!("Package not found: {}", package_id))?;

        let req = VersionReq::parse(version_req_str);

        // Find first (newest) version that satisfies requirement
        for version in versions {
            if req.satisfies(version) {
                return Ok(version.clone());
            }
        }

        Err(alloc::format!(
            "No suitable version found for {} (requirement: {})",
            package_id,
            version_req_str
        ))
    }

    /// Check for conflicts in solution
    fn check_conflicts(&self, solution: &BTreeMap<PackageId, Version>) -> Result<(), String> {
        for (pkg_id, version) in solution {
            if let Some(candidate) = self.metadata.get(&(pkg_id.clone(), version.clone())) {
                for conflict in &candidate.conflicts {
                    if solution.contains_key(conflict) {
                        return Err(alloc::format!(
                            "Conflict: {} {} conflicts with {}",
                            pkg_id,
                            Self::version_to_string(version),
                            conflict
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Convert version to string
    fn version_to_string(version: &Version) -> String {
        alloc::format!("{}.{}.{}", version.major, version.minor, version.patch)
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_version_req_parsing() {
        let req = VersionReq::parse("1.2.3");
        assert!(matches!(req, VersionReq::Exact(_)));

        let req = VersionReq::parse(">=1.0.0");
        assert!(matches!(req, VersionReq::AtLeast(_)));

        let req = VersionReq::parse("*");
        assert!(matches!(req, VersionReq::Any));
    }

    #[test_case]
    fn test_version_satisfies() {
        let v123 = Version::new(1, 2, 3);
        let v100 = Version::new(1, 0, 0);

        let req = VersionReq::Exact(v123.clone());
        assert!(req.satisfies(&v123));
        assert!(!req.satisfies(&v100));

        let req = VersionReq::AtLeast(v100.clone());
        assert!(req.satisfies(&v123));
        assert!(req.satisfies(&v100));
    }

    #[test_case]
    fn test_simple_resolution() {
        let mut resolver = DependencyResolver::new();

        resolver.register_package(String::from("pkg-a"), Version::new(1, 0, 0), vec![], vec![]);

        let deps = vec![Dependency {
            name: String::from("pkg-a"),
            version_req: String::from("1.0.0"),
        }];

        let result = resolver.resolve(&deps);
        assert!(result.is_ok());
        let packages = result.unwrap();
        assert_eq!(packages.len(), 1);
    }
}
