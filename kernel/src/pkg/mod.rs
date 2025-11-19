//! Package Management System
//!
//! VeridianOS package manager for installing, updating, and managing software packages.

pub mod format;
pub mod repository;
pub mod resolver;

use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::collections::BTreeMap;
use crate::error::KernelError;

/// Package identifier
pub type PackageId = String;

/// Package version using semantic versioning
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }
}

/// Package metadata
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub name: String,
    pub version: Version,
    pub author: String,
    pub description: String,
    pub license: String,
    pub dependencies: Vec<Dependency>,
}

/// Package dependency
#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub version_req: String,
}

/// Package installation result
pub type PkgResult<T> = Result<T, KernelError>;

/// Package manager
pub struct PackageManager {
    /// Installed packages
    installed: BTreeMap<PackageId, PackageMetadata>,
    /// Dependency resolver
    resolver: resolver::DependencyResolver,
    /// Available repositories
    repositories: Vec<repository::Repository>,
}

impl PackageManager {
    pub fn new() -> Self {
        Self {
            installed: BTreeMap::new(),
            resolver: resolver::DependencyResolver::new(),
            repositories: Vec::new(),
        }
    }

    /// Add a repository
    pub fn add_repository(&mut self, repo: repository::Repository) {
        self.repositories.push(repo);
    }

    /// Install a package by name and version
    pub fn install(&mut self, name: String, version_req: String) -> PkgResult<()> {
        // Create dependency for the package
        let dep = Dependency { name: name.clone(), version_req: version_req.clone() };

        // Resolve dependencies
        let packages = self.resolver.resolve(&[dep])
            .map_err(|_e| KernelError::InvalidState {
                expected: "resolvable dependencies",
                actual: "dependency resolution failed",
            })?;

        // Install each package in order
        for (pkg_id, version) in packages {
            if self.installed.contains_key(&pkg_id) {
                // Already installed - skip or upgrade
                continue;
            }

            // Download package
            let package_data = self.download_package(&pkg_id, &version)?;

            // Verify package signature
            self.verify_package(&package_data)?;

            // Extract and install
            self.install_package(pkg_id.clone(), version.clone(), package_data)?;

            crate::println!("[PKG] Installed {} {}.{}.{}", pkg_id, version.major, version.minor, version.patch);
        }

        Ok(())
    }

    /// Remove an installed package
    pub fn remove(&mut self, package_id: &PackageId) -> PkgResult<()> {
        if !self.installed.contains_key(package_id) {
            return Err(KernelError::NotFound {
                resource: "package",
                id: 0,
            });
        }

        // Check reverse dependencies
        let dependents = self.find_dependents(package_id);
        if !dependents.is_empty() {
            return Err(KernelError::InvalidState {
                expected: "no reverse dependencies",
                actual: "package is required by other packages",
            });
        }

        // Remove package
        self.installed.remove(package_id);

        crate::println!("[PKG] Removed {}", package_id);

        Ok(())
    }

    /// List installed packages
    pub fn list_installed(&self) -> Vec<(PackageId, Version)> {
        self.installed
            .iter()
            .map(|(id, meta)| (id.clone(), meta.version.clone()))
            .collect()
    }

    /// Check if package is installed
    pub fn is_installed(&self, package_id: &PackageId) -> bool {
        self.installed.contains_key(package_id)
    }

    /// Get installed package metadata
    pub fn get_metadata(&self, package_id: &PackageId) -> Option<&PackageMetadata> {
        self.installed.get(package_id)
    }

    /// Update package lists from repositories
    pub fn update(&mut self) -> PkgResult<()> {
        for repo in &self.repositories {
            let packages = repo.fetch_package_list();

            for pkg in packages {
                self.resolver.register_package(
                    pkg.name.clone(),
                    pkg.version.clone(),
                    pkg.dependencies.clone(),
                    vec![], // TODO: Add conflicts field to metadata
                );
            }
        }

        Ok(())
    }

    /// Download package from repositories
    fn download_package(&self, package_id: &PackageId, _version: &Version) -> PkgResult<Vec<u8>> {
        for repo in &self.repositories {
            if let Some(data) = repo.download_package(package_id) {
                return Ok(data);
            }
        }

        Err(KernelError::NotFound {
            resource: "package in repositories",
            id: 0,
        })
    }

    /// Verify package signature (dual Ed25519 + Dilithium)
    fn verify_package(&self, _package_data: &[u8]) -> PkgResult<()> {
        // TODO: Implement package verification
        // 1. Parse package header
        // 2. Extract signatures
        // 3. Verify Ed25519 signature
        // 4. Verify Dilithium signature
        // 5. Check both pass
        Ok(())
    }

    /// Install package from data
    fn install_package(
        &mut self,
        package_id: PackageId,
        version: Version,
        _package_data: Vec<u8>,
    ) -> PkgResult<()> {
        // TODO: Implement package extraction
        // 1. Parse package format
        // 2. Decompress content
        // 3. Extract files
        // 4. Update installed registry

        // For now, just add to installed list
        let metadata = PackageMetadata {
            name: package_id.clone(),
            version: version.clone(),
            author: String::from("unknown"),
            description: String::from(""),
            license: String::from(""),
            dependencies: Vec::new(),
        };

        self.installed.insert(package_id, metadata);

        Ok(())
    }

    /// Find packages that depend on the given package
    fn find_dependents(&self, package_id: &PackageId) -> Vec<PackageId> {
        self.installed
            .iter()
            .filter(|(_, meta)| {
                meta.dependencies
                    .iter()
                    .any(|dep| &dep.name == package_id)
            })
            .map(|(id, _)| id.clone())
            .collect()
    }
}

impl Default for PackageManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global package manager instance
static mut PACKAGE_MANAGER: Option<PackageManager> = None;

/// Initialize package management system
pub fn init() {
    unsafe {
        PACKAGE_MANAGER = Some(PackageManager::new());
    }
    crate::println!("[PKG] Package management system initialized");
}

/// Get global package manager
pub fn get_package_manager() -> Option<&'static mut PackageManager> {
    unsafe { PACKAGE_MANAGER.as_mut() }
}
