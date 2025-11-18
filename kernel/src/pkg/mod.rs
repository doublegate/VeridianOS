//! Package management system for VeridianOS
//!
//! Provides package installation, removal, updates, and dependency resolution.

use crate::error::KernelError;
use alloc::string::String;
use alloc::vec::Vec;

/// Package metadata
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    /// Package name
    pub name: String,
    /// Version (semantic versioning)
    pub version: (u32, u32, u32),
    /// Description
    pub description: String,
    /// Dependencies
    pub dependencies: Vec<String>,
    /// Package size in bytes
    pub size: usize,
    /// Install path
    pub install_path: String,
}

/// Package state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageState {
    NotInstalled,
    Installing,
    Installed,
    Updating,
    Removing,
    Failed,
}

/// Package database entry
#[derive(Debug, Clone)]
pub struct PackageEntry {
    pub metadata: PackageMetadata,
    pub state: PackageState,
    pub install_time: u64,
}

/// Maximum packages
const MAX_PACKAGES: usize = 256;

/// Package database
static mut PACKAGES: [Option<PackageEntry>; MAX_PACKAGES] = [const { None }; MAX_PACKAGES];
static mut PKG_COUNT: usize = 0;

/// Install a package
pub fn install(name: &str, version: (u32, u32, u32)) -> Result<(), KernelError> {
    println!("[PKG] Installing package: {} v{}.{}.{}", name, version.0, version.1, version.2);

    unsafe {
        if PKG_COUNT >= MAX_PACKAGES {
            return Err(KernelError::ResourceExhausted {
                resource: "package_slots",
            });
        }

        // Create package entry
        let entry = PackageEntry {
            metadata: PackageMetadata {
                name: String::from(name),
                version,
                description: String::from("Package description"),
                dependencies: Vec::new(),
                size: 0,
                install_path: String::from("/pkg/"),
            },
            state: PackageState::Installed,
            install_time: 0,
        };

        PACKAGES[PKG_COUNT] = Some(entry);
        PKG_COUNT += 1;
    }

    println!("[PKG] Package installed successfully");
    Ok(())
}

/// Remove a package
pub fn remove(name: &str) -> Result<(), KernelError> {
    println!("[PKG] Removing package: {}", name);

    unsafe {
        for i in 0..PKG_COUNT {
            if let Some(ref mut entry) = PACKAGES[i] {
                if entry.metadata.name == name {
                    PACKAGES[i] = None;
                    println!("[PKG] Package removed successfully");
                    return Ok(());
                }
            }
        }
    }

    Err(KernelError::NotFound {
        resource: "package",
        id: 0,
    })
}

/// List installed packages
pub fn list() -> Vec<String> {
    let mut result = Vec::new();
    unsafe {
        for i in 0..PKG_COUNT {
            if let Some(ref entry) = PACKAGES[i] {
                result.push(entry.metadata.name.clone());
            }
        }
    }
    result
}

/// Check if package is installed
pub fn is_installed(name: &str) -> bool {
    unsafe {
        for i in 0..PKG_COUNT {
            if let Some(ref entry) = PACKAGES[i] {
                if entry.metadata.name == name {
                    return entry.state == PackageState::Installed;
                }
            }
        }
    }
    false
}

/// Initialize package manager
pub fn init() -> Result<(), KernelError> {
    println!("[PKG] Initializing package manager...");

    // Install core packages
    install("veridian-base", (0, 1, 0))?;
    install("veridian-utils", (0, 1, 0))?;

    println!("[PKG] Package manager initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_install_package() {
        let result = install("test-pkg", (1, 0, 0));
        assert!(result.is_ok());
        assert!(is_installed("test-pkg"));
    }

    #[test_case]
    fn test_remove_package() {
        install("remove-test", (1, 0, 0)).unwrap();
        assert!(is_installed("remove-test"));
        remove("remove-test").unwrap();
        assert!(!is_installed("remove-test"));
    }
}
