# Phase 4: Package Ecosystem and Self-Hosting (Months 22-27)

**Status**: COMPLETE (100%) - Completed across v0.3.4 through v0.4.1 (February 15, 2026)
**Last Updated**: February 15, 2026

### Completion Summary

Phase 4 was implemented across multiple releases:
- **v0.3.4** (P4-1 through P4-7): Package manager with transactions, DPLL SAT dependency resolver, shell/syscall integration, ports system with TOML parser, build environment, SDK types
- **v0.3.6** (Group 1 Core Gaps): Repository index/mirrors, delta updates, config tracking, orphan detection
- **v0.3.7 - v0.3.8** (Groups 2-4): Security signing, toolchain integration, testing framework, compliance, ecosystem
- **v0.4.0 - v0.4.1**: Final integration, userland bridge, remaining gaps closed

Key implementations: DPLL SAT-based dependency resolution with CNF encoding, unit propagation, and backtracking; VFS-backed flat file package database; dual signature verification (Ed25519 + Dilithium); ports system with TOML parser and build environment; SDK framework with syscall API wrappers.

## Overview

Phase 4 establishes a comprehensive package management ecosystem for VeridianOS, including source-based ports, binary packages, development tools, and a secure software distribution infrastructure. This phase enables third-party software development and creates a sustainable ecosystem, culminating in self-hosting capability.

## Objectives

1. **Package Manager**: Advanced package management with dependency resolution
2. **Ports System**: Source-based software building framework
3. **Binary Packages**: Pre-compiled package distribution
4. **Development Tools**: Compilers, debuggers, and build systems
5. **Package Repository**: Secure package hosting and distribution
6. **SDK and APIs**: Developer tools and documentation
7. **Self-Hosting**: Native compilation of VeridianOS on VeridianOS

## Self-Hosting Roadmap (15-Month Plan)

### Phase 4A: Cross-Compilation Foundation (Months 1-3)
- LLVM/GCC target implementation for VeridianOS
- Custom target triples: `{x86_64,aarch64,riscv64}-unknown-veridian`
- CMake toolchain files and build system support

### Phase 4B: Bootstrap Environment (Months 4-6)
- Port binutils (as, ld, ar, etc.)
- Minimal C compiler (GCC or Clang)
- Essential build tools (make, pkg-config)

### Phase 4C: Development Platform (Months 7-9)
- Full compiler suite (C, C++, Rust, Go via gccgo)
- Debuggers (GDB with ptrace support)
- Modern build systems (CMake, Meson, Cargo)

### Phase 4D: Full Self-Hosting (Months 10-15)
- Native VeridianOS compilation
- Package building on VeridianOS
- CI/CD running on VeridianOS
- SDK and developer documentation

## Compiler Toolchain Strategy

**Priority Order** (AI Recommendation):
1. **LLVM/Clang**: Unified backend for C/C++/Rust
2. **Rust**: Native target with std library support
3. **GCC**: Alternative C/C++ compiler
4. **Go**: Via gccgo initially, native runtime later
5. **Python**: CPython port with essential modules

## Architecture Components

### 1. Package Management System

#### 1.1 Package Manager Core

**pkgmgr/src/core/mod.rs**
```rust
use serde::{Deserialize, Serialize};
use blake3::Hasher;
use ed25519_dalek::{Signature, Verifier};

/// Main package manager
pub struct PackageManager {
    /// Package database
    database: PackageDatabase,
    /// Repository configuration
    repositories: Vec<Repository>,
    /// Installed packages
    installed: InstalledPackages,
    /// Transaction system
    transactions: TransactionManager,
    /// Download manager
    downloader: DownloadManager,
    /// Signature verification
    verifier: SignatureVerifier,
    /// Configuration
    config: PkgConfig,
}

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name
    pub name: String,
    /// Version
    pub version: Version,
    /// Description
    pub description: String,
    /// Dependencies
    pub dependencies: Vec<Dependency>,
    /// Build dependencies
    pub build_dependencies: Vec<Dependency>,
    /// Provides
    pub provides: Vec<String>,
    /// Conflicts
    pub conflicts: Vec<String>,
    /// Files
    pub files: Vec<FileEntry>,
    /// Install scripts
    pub scripts: Scripts,
    /// Metadata
    pub metadata: PackageMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version_requirement: VersionReq,
    pub optional: bool,
    pub features: Vec<String>,
}

impl PackageManager {
    /// Install packages
    pub async fn install(&mut self, packages: &[&str]) -> Result<(), Error> {
        // Resolve dependencies
        let resolution = self.resolve_dependencies(packages).await?;
        
        // Check conflicts
        self.check_conflicts(&resolution)?;
        
        // Calculate download size
        let download_size = self.calculate_download_size(&resolution)?;
        let install_size = self.calculate_install_size(&resolution)?;
        
        // Confirm with user
        if !self.confirm_transaction(&resolution, download_size, install_size)? {
            return Ok(());
        }
        
        // Start transaction
        let transaction = self.transactions.begin()?;
        
        // Download packages
        let downloaded = self.download_packages(&resolution).await?;
        
        // Verify signatures
        for (pkg, path) in &downloaded {
            self.verify_package_signature(pkg, path)?;
        }
        
        // Install in dependency order
        for pkg in resolution.install_order() {
            self.install_package(pkg, &transaction)?;
        }
        
        // Commit transaction
        transaction.commit()?;
        
        // Update database
        self.database.mark_installed(&resolution)?;
        
        Ok(())
    }
    
    /// Dependency resolver
    async fn resolve_dependencies(
        &self,
        requested: &[&str],
    ) -> Result<Resolution, Error> {
        let mut resolver = DependencyResolver::new(&self.database);
        
        // Add requested packages
        for pkg_spec in requested {
            let requirement = self.parse_package_spec(pkg_spec)?;
            resolver.add_requirement(requirement)?;
        }
        
        // Add already installed packages as constraints
        for installed in self.installed.all() {
            resolver.add_constraint(Constraint::Installed(installed))?;
        }
        
        // Resolve
        let resolution = resolver.resolve().await?;
        
        Ok(resolution)
    }
    
    /// Install single package
    fn install_package(
        &mut self,
        package: &Package,
        transaction: &Transaction,
    ) -> Result<(), Error> {
        let pkg_path = self.get_package_path(package)?;
        
        // Run pre-install script
        if let Some(script) = &package.scripts.pre_install {
            self.run_script(script, package, Phase::PreInstall)?;
        }
        
        // Extract files
        let extractor = PackageExtractor::new(&pkg_path)?;
        for file in &package.files {
            let dest = self.config.root.join(&file.path);
            
            // Check for conflicts
            if dest.exists() && !self.can_overwrite(&dest, package)? {
                return Err(Error::FileConflict(dest));
            }
            
            // Extract file
            extractor.extract_file(file, &dest)?;
            
            // Set permissions
            fs::set_permissions(&dest, file.permissions)?;
            
            // Record in transaction
            transaction.add_file(&dest, package)?;
        }
        
        // Run post-install script
        if let Some(script) = &package.scripts.post_install {
            self.run_script(script, package, Phase::PostInstall)?;
        }
        
        // Register package
        self.installed.add(package)?;
        
        Ok(())
    }
    
    /// Remove packages
    pub async fn remove(&mut self, packages: &[&str]) -> Result<(), Error> {
        // Check dependencies
        let removal_set = self.check_removal_dependencies(packages)?;
        
        // Confirm
        if !self.confirm_removal(&removal_set)? {
            return Ok(());
        }
        
        // Start transaction
        let transaction = self.transactions.begin()?;
        
        // Remove in reverse dependency order
        for pkg in removal_set.iter().rev() {
            self.remove_package(pkg, &transaction)?;
        }
        
        // Commit
        transaction.commit()?;
        
        Ok(())
    }
    
    /// Update packages
    pub async fn update(&mut self, packages: Vec<String>) -> Result<(), Error> {
        // Update repository metadata
        self.sync_repositories().await?;
        
        // Get updatable packages
        let updates = if packages.is_empty() {
            self.find_all_updates()?
        } else {
            self.find_updates(&packages)?
        };
        
        if updates.is_empty() {
            println!("All packages are up to date");
            return Ok(());
        }
        
        // Show updates
        self.display_updates(&updates)?;
        
        // Install updates
        let update_specs: Vec<String> = updates.iter()
            .map(|u| format!("{}={}", u.name, u.new_version))
            .collect();
            
        self.install(&update_specs.iter().map(|s| s.as_str()).collect::<Vec<_>>()).await
    }
}

/// Dependency resolver
struct DependencyResolver {
    /// Available packages
    available: BTreeMap<String, Vec<Package>>,
    /// Requirements
    requirements: Vec<Requirement>,
    /// Constraints
    constraints: Vec<Constraint>,
    /// SAT solver
    solver: SatSolver,
}

impl DependencyResolver {
    /// Resolve dependencies using SAT solver
    async fn resolve(&mut self) -> Result<Resolution, Error> {
        // Convert to SAT problem
        let problem = self.build_sat_problem()?;
        
        // Solve
        let solution = self.solver.solve(problem)?;
        
        // Convert solution back to packages
        let packages = self.solution_to_packages(solution)?;
        
        // Build installation order
        let order = self.topological_sort(&packages)?;
        
        Ok(Resolution {
            packages,
            order,
            conflicts: Vec::new(),
        })
    }
    
    /// Build SAT problem from dependencies
    fn build_sat_problem(&self) -> Result<SatProblem, Error> {
        let mut problem = SatProblem::new();
        let mut var_map = BTreeMap::new();
        let mut next_var = 1;
        
        // Create variables for each package version
        for (name, versions) in &self.available {
            for version in versions {
                let var = next_var;
                next_var += 1;
                var_map.insert((name.clone(), version.version.clone()), var);
                problem.add_variable(var);
            }
        }
        
        // At most one version of each package
        for (name, versions) in &self.available {
            if versions.len() > 1 {
                let vars: Vec<i32> = versions.iter()
                    .map(|v| var_map[&(name.clone(), v.version.clone())] as i32)
                    .collect();
                problem.add_at_most_one_constraint(vars);
            }
        }
        
        // Dependencies
        for (name, versions) in &self.available {
            for version in versions {
                let pkg_var = var_map[&(name.clone(), version.version.clone())];
                
                for dep in &version.dependencies {
                    if dep.optional {
                        continue;
                    }
                    
                    // Find satisfying versions
                    let satisfying: Vec<i32> = self.available
                        .get(&dep.name)
                        .map(|versions| {
                            versions.iter()
                                .filter(|v| dep.version_requirement.matches(&v.version))
                                .map(|v| {
                                    var_map[&(dep.name.clone(), v.version.clone())] as i32
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    
                    if satisfying.is_empty() {
                        return Err(Error::UnsatisfiableDependency(dep.clone()));
                    }
                    
                    // If package is installed, at least one dependency must be
                    problem.add_implication(pkg_var as i32, satisfying);
                }
            }
        }
        
        // Requirements
        for req in &self.requirements {
            let satisfying: Vec<i32> = self.available
                .get(&req.name)
                .map(|versions| {
                    versions.iter()
                        .filter(|v| req.version_req.matches(&v.version))
                        .map(|v| var_map[&(req.name.clone(), v.version.clone())] as i32)
                        .collect()
                })
                .unwrap_or_default();
                
            if satisfying.is_empty() {
                return Err(Error::NoMatchingPackage(req.name.clone()));
            }
            
            // At least one must be installed
            problem.add_clause(satisfying);
        }
        
        Ok(problem)
    }
}
```

#### 1.2 Package Repository

**pkgmgr/src/repository/mod.rs**
```rust
/// Package repository
pub struct Repository {
    /// Repository URL
    pub url: Url,
    /// Repository name
    pub name: String,
    /// Priority
    pub priority: u32,
    /// Enabled
    pub enabled: bool,
    /// GPG key
    pub gpg_key: Option<PublicKey>,
    /// Mirror list
    pub mirrors: Vec<Url>,
    /// Metadata cache
    cache: RepositoryCache,
}

impl Repository {
    /// Sync repository metadata
    pub async fn sync(&mut self) -> Result<(), Error> {
        // Download metadata
        let metadata_url = self.url.join("metadata.json.gz")?;
        let metadata_sig_url = self.url.join("metadata.json.gz.sig")?;
        
        // Try primary URL first, then mirrors
        let (metadata, signature) = self.download_with_fallback(
            &metadata_url,
            &metadata_sig_url,
        ).await?;
        
        // Verify signature
        if let Some(key) = &self.gpg_key {
            key.verify(&metadata, &signature)?;
        }
        
        // Decompress and parse
        let metadata = decompress_gzip(&metadata)?;
        let repo_metadata: RepositoryMetadata = serde_json::from_slice(&metadata)?;
        
        // Update cache
        self.cache.update(repo_metadata)?;
        
        Ok(())
    }
    
    /// Download package
    pub async fn download_package(
        &self,
        package: &Package,
    ) -> Result<PathBuf, Error> {
        let filename = format!("{}-{}.vpkg", package.name, package.version);
        let url = self.url.join(&format!("packages/{}", filename))?;
        let sig_url = self.url.join(&format!("packages/{}.sig", filename))?;
        
        // Check cache first
        if let Some(cached) = self.cache.get_package(&package.name, &package.version) {
            if self.verify_cached_package(cached, package)? {
                return Ok(cached.path.clone());
            }
        }
        
        // Download
        let (data, signature) = self.download_with_fallback(&url, &sig_url).await?;
        
        // Verify
        if let Some(key) = &self.gpg_key {
            key.verify(&data, &signature)?;
        }
        
        // Verify hash
        let hash = blake3::hash(&data);
        if hash.as_bytes() != package.metadata.hash.as_slice() {
            return Err(Error::HashMismatch);
        }
        
        // Save to cache
        let path = self.cache.store_package(&package.name, &package.version, &data)?;
        
        Ok(path)
    }
}

/// Repository metadata format
#[derive(Debug, Serialize, Deserialize)]
pub struct RepositoryMetadata {
    /// Repository version
    pub version: u32,
    /// Generation timestamp
    pub timestamp: u64,
    /// Packages
    pub packages: Vec<PackageEntry>,
    /// Provides mapping
    pub provides: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub dependencies: Vec<String>,
    pub size: u64,
    pub hash: String,
    pub signature: String,
}
```

### 2. Ports System

#### 2.1 Port Framework

**ports/framework/port.rs**
```rust
use std::process::Command;
use toml::Value;

/// Port definition
pub struct Port {
    /// Port name
    pub name: String,
    /// Version
    pub version: String,
    /// Port metadata
    pub metadata: PortMetadata,
    /// Build instructions
    pub build: BuildInstructions,
    /// Dependencies
    pub dependencies: Dependencies,
    /// Patches
    pub patches: Vec<Patch>,
}

#[derive(Debug, Deserialize)]
pub struct PortMetadata {
    pub description: String,
    pub homepage: String,
    pub license: Vec<String>,
    pub maintainer: String,
    pub categories: Vec<String>,
}

#[derive(Debug)]
pub struct BuildInstructions {
    /// Source URL
    pub source: Source,
    /// Build type
    pub build_type: BuildType,
    /// Configure args
    pub configure_args: Vec<String>,
    /// Build environment
    pub build_env: BTreeMap<String, String>,
    /// Install prefix
    pub prefix: PathBuf,
}

#[derive(Debug)]
pub enum BuildType {
    Autotools,
    CMake { generator: String },
    Meson,
    Cargo,
    Make,
    Custom { script: String },
}

impl Port {
    /// Load port from directory
    pub fn load(path: &Path) -> Result<Self, Error> {
        let portfile = path.join("Portfile.toml");
        let content = fs::read_to_string(&portfile)?;
        let port_def: PortDefinition = toml::from_str(&content)?;
        
        Ok(Self::from_definition(port_def)?)
    }
    
    /// Build port
    pub fn build(&self, options: &BuildOptions) -> Result<(), Error> {
        let work_dir = options.work_dir.join(&self.name);
        fs::create_dir_all(&work_dir)?;
        
        // Fetch source
        self.fetch_source(&work_dir)?;
        
        // Extract
        self.extract_source(&work_dir)?;
        
        // Apply patches
        self.apply_patches(&work_dir)?;
        
        // Configure
        self.configure(&work_dir, options)?;
        
        // Build
        self.compile(&work_dir, options)?;
        
        // Install
        self.install(&work_dir, options)?;
        
        // Package
        if options.create_package {
            self.create_package(&work_dir, options)?;
        }
        
        Ok(())
    }
    
    /// Configure build
    fn configure(&self, work_dir: &Path, options: &BuildOptions) -> Result<(), Error> {
        let build_dir = work_dir.join("build");
        fs::create_dir_all(&build_dir)?;
        
        match &self.build.build_type {
            BuildType::Autotools => {
                let configure = work_dir.join("src").join("configure");
                
                // Run autoreconf if needed
                if !configure.exists() {
                    Command::new("autoreconf")
                        .arg("-fiv")
                        .current_dir(work_dir.join("src"))
                        .check_status()?;
                }
                
                // Run configure
                let mut cmd = Command::new(&configure);
                cmd.current_dir(&build_dir)
                    .arg(format!("--prefix={}", self.build.prefix.display()));
                    
                for arg in &self.build.configure_args {
                    cmd.arg(arg);
                }
                
                cmd.envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::CMake { generator } => {
                let mut cmd = Command::new("cmake");
                cmd.current_dir(&build_dir)
                    .arg("-G").arg(generator)
                    .arg(format!("-DCMAKE_INSTALL_PREFIX={}", self.build.prefix.display()))
                    .arg(format!("-DCMAKE_BUILD_TYPE={}", options.build_type))
                    .arg("../src");
                    
                for arg in &self.build.configure_args {
                    cmd.arg(arg);
                }
                
                cmd.envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::Meson => {
                Command::new("meson")
                    .current_dir(work_dir)
                    .arg("setup")
                    .arg("build")
                    .arg("src")
                    .arg(format!("--prefix={}", self.build.prefix.display()))
                    .args(&self.build.configure_args)
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
            
            _ => {}
        }
        
        Ok(())
    }
    
    /// Compile
    fn compile(&self, work_dir: &Path, options: &BuildOptions) -> Result<(), Error> {
        let build_dir = work_dir.join("build");
        
        match &self.build.build_type {
            BuildType::Autotools | BuildType::Make => {
                Command::new("make")
                    .current_dir(&build_dir)
                    .arg(format!("-j{}", options.jobs))
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::CMake { .. } => {
                Command::new("cmake")
                    .current_dir(&build_dir)
                    .arg("--build")
                    .arg(".")
                    .arg("--parallel")
                    .arg(options.jobs.to_string())
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::Meson => {
                Command::new("meson")
                    .current_dir(&build_dir)
                    .arg("compile")
                    .arg("-j").arg(options.jobs.to_string())
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::Cargo => {
                Command::new("cargo")
                    .current_dir(work_dir.join("src"))
                    .arg("build")
                    .arg("--release")
                    .arg("--jobs").arg(options.jobs.to_string())
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
            
            BuildType::Custom { script } => {
                Command::new("sh")
                    .current_dir(&build_dir)
                    .arg("-c")
                    .arg(script)
                    .envs(&self.build.build_env)
                    .check_status()?;
            }
        }
        
        Ok(())
    }
}

/// Port collection manager
pub struct PortCollection {
    /// Collection path
    pub path: PathBuf,
    /// Port index
    pub index: PortIndex,
    /// Build options
    pub options: BuildOptions,
}

impl PortCollection {
    /// Update ports tree
    pub fn update(&mut self) -> Result<(), Error> {
        // Update via git
        Command::new("git")
            .current_dir(&self.path)
            .args(&["pull", "--ff-only"])
            .check_status()?;
            
        // Rebuild index
        self.rebuild_index()?;
        
        Ok(())
    }
    
    /// Search ports
    pub fn search(&self, query: &str) -> Vec<&PortInfo> {
        self.index.search(query)
    }
    
    /// Build port and dependencies
    pub fn build_port(&self, name: &str) -> Result<(), Error> {
        // Load port
        let port_path = self.path.join(name);
        let port = Port::load(&port_path)?;
        
        // Check dependencies
        let deps = self.resolve_dependencies(&port)?;
        
        // Build dependencies first
        for dep in deps {
            if !self.is_installed(&dep)? {
                self.build_port(&dep)?;
            }
        }
        
        // Build port
        port.build(&self.options)?;
        
        Ok(())
    }
}
```

#### 2.2 Port Examples

**ports/lang/rust/Portfile.toml**
```toml
[metadata]
name = "rust"
version = "1.75.0"
description = "Systems programming language"
homepage = "https://rust-lang.org"
license = ["MIT", "Apache-2.0"]
maintainer = "ports@veridian-os.org"
categories = ["lang", "devel"]

[source]
url = "https://static.rust-lang.org/dist/rustc-${version}-src.tar.gz"
hash = "sha256:abcdef..."

[dependencies]
build = ["cmake", "python3", "ninja"]
runtime = ["llvm@17"]

[build]
type = "custom"
script = """
./configure \
    --prefix=${PREFIX} \
    --enable-extended \
    --tools=cargo,rustfmt,clippy \
    --set llvm.link-shared=true
    
make -j${JOBS}
"""

[build.env]
RUST_BACKTRACE = "1"

[patches]
# VeridianOS-specific patches
"veridian-target.patch" = """
--- a/src/librustc_target/spec/mod.rs
+++ b/src/librustc_target/spec/mod.rs
@@ -1000,6 +1000,7 @@
     ("x86_64-unknown-veridian", x86_64_unknown_veridian),
+    ("aarch64-unknown-veridian", aarch64_unknown_veridian),
"""
```

### 3. Development Tools

#### 3.1 Toolchain Manager

**tools/toolchain/src/main.rs**
```rust
/// VeridianOS toolchain manager
pub struct ToolchainManager {
    /// Installed toolchains
    toolchains: BTreeMap<String, Toolchain>,
    /// Active toolchain
    active: Option<String>,
    /// Configuration
    config: ToolchainConfig,
}

pub struct Toolchain {
    pub name: String,
    pub version: String,
    pub target: Target,
    pub components: Vec<Component>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Component {
    Compiler { name: String, version: String },
    Debugger { name: String, version: String },
    Profiler { name: String, version: String },
    Analyzer { name: String, version: String },
    Formatter { name: String, version: String },
}

impl ToolchainManager {
    /// Install toolchain
    pub async fn install_toolchain(
        &mut self,
        spec: &str,
    ) -> Result<(), Error> {
        let toolchain_def = self.fetch_toolchain_definition(spec).await?;
        
        println!("Installing toolchain: {} {}", 
                 toolchain_def.name, toolchain_def.version);
        
        // Download components
        for component in &toolchain_def.components {
            self.download_component(component).await?;
        }
        
        // Install
        let install_path = self.config.toolchains_dir
            .join(&toolchain_def.name)
            .join(&toolchain_def.version);
            
        self.install_components(&toolchain_def, &install_path)?;
        
        // Register toolchain
        let toolchain = Toolchain {
            name: toolchain_def.name.clone(),
            version: toolchain_def.version.clone(),
            target: toolchain_def.target.clone(),
            components: toolchain_def.components.clone(),
            path: install_path,
        };
        
        self.toolchains.insert(toolchain.name.clone(), toolchain);
        
        Ok(())
    }
    
    /// Set active toolchain
    pub fn set_active(&mut self, name: &str) -> Result<(), Error> {
        if !self.toolchains.contains_key(name) {
            return Err(Error::ToolchainNotFound(name.to_string()));
        }
        
        self.active = Some(name.to_string());
        
        // Update environment
        self.update_environment()?;
        
        Ok(())
    }
    
    /// Update PATH and environment
    fn update_environment(&self) -> Result<(), Error> {
        if let Some(active) = &self.active {
            let toolchain = &self.toolchains[active];
            
            // Create symlinks in ~/.veridian/bin
            let bin_dir = self.config.bin_dir();
            fs::create_dir_all(&bin_dir)?;
            
            // Link compiler
            for component in &toolchain.components {
                match component {
                    Component::Compiler { name, .. } => {
                        let src = toolchain.path.join("bin").join(name);
                        let dst = bin_dir.join(name);
                        
                        if dst.exists() {
                            fs::remove_file(&dst)?;
                        }
                        
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(&src, &dst)?;
                    }
                    _ => {}
                }
            }
            
            // Update shell profile
            self.update_shell_profile()?;
        }
        
        Ok(())
    }
}

/// Cross-compilation support
pub struct CrossCompiler {
    /// Host target
    host: Target,
    /// Build targets
    targets: Vec<Target>,
    /// Sysroots
    sysroots: BTreeMap<Target, Sysroot>,
}

impl CrossCompiler {
    /// Build for target
    pub fn build(
        &self,
        project: &Project,
        target: &Target,
    ) -> Result<(), Error> {
        // Get sysroot
        let sysroot = self.sysroots.get(target)
            .ok_or(Error::NoSysroot(target.clone()))?;
        
        // Set up environment
        let mut env = BTreeMap::new();
        env.insert("CC", self.get_compiler(target, "c"));
        env.insert("CXX", self.get_compiler(target, "c++"));
        env.insert("AR", self.get_tool(target, "ar"));
        env.insert("RANLIB", self.get_tool(target, "ranlib"));
        env.insert("SYSROOT", sysroot.path.to_str().unwrap());
        
        // Configure build
        match &project.build_system {
            BuildSystem::Cargo => {
                Command::new("cargo")
                    .arg("build")
                    .arg("--target").arg(&target.triple)
                    .arg("--release")
                    .envs(&env)
                    .check_status()?;
            }
            
            BuildSystem::CMake => {
                // Use toolchain file
                let toolchain_file = self.generate_cmake_toolchain(target, sysroot)?;
                
                Command::new("cmake")
                    .arg("-DCMAKE_TOOLCHAIN_FILE").arg(&toolchain_file)
                    .arg("-DCMAKE_BUILD_TYPE=Release")
                    .arg(".")
                    .envs(&env)
                    .check_status()?;
                    
                Command::new("cmake")
                    .arg("--build")
                    .arg(".")
                    .check_status()?;
            }
            
            _ => return Err(Error::UnsupportedBuildSystem),
        }
        
        Ok(())
    }
}
```

#### 3.2 SDK Generator

**tools/sdk-gen/src/main.rs**
```rust
/// SDK generator for VeridianOS
pub struct SdkGenerator {
    /// Target version
    version: Version,
    /// Components to include
    components: Vec<SdkComponent>,
    /// Output configuration
    output: SdkOutput,
}

#[derive(Debug, Clone)]
pub enum SdkComponent {
    Headers,
    Libraries { static_libs: bool, shared_libs: bool },
    Tools { debugger: bool, profiler: bool },
    Documentation,
    Examples,
    Templates,
}

impl SdkGenerator {
    /// Generate SDK
    pub fn generate(&self) -> Result<(), Error> {
        let sdk_dir = self.output.path.join(format!("veridian-sdk-{}", self.version));
        fs::create_dir_all(&sdk_dir)?;
        
        // Copy headers
        if self.components.contains(&SdkComponent::Headers) {
            self.copy_headers(&sdk_dir)?;
        }
        
        // Copy libraries
        for component in &self.components {
            if let SdkComponent::Libraries { static_libs, shared_libs } = component {
                self.copy_libraries(&sdk_dir, *static_libs, *shared_libs)?;
            }
        }
        
        // Generate pkg-config files
        self.generate_pkg_config(&sdk_dir)?;
        
        // Create CMake package files
        self.generate_cmake_config(&sdk_dir)?;
        
        // Copy tools
        self.copy_tools(&sdk_dir)?;
        
        // Generate documentation
        if self.components.contains(&SdkComponent::Documentation) {
            self.generate_documentation(&sdk_dir)?;
        }
        
        // Create examples
        if self.components.contains(&SdkComponent::Examples) {
            self.create_examples(&sdk_dir)?;
        }
        
        // Package SDK
        self.package_sdk(&sdk_dir)?;
        
        Ok(())
    }
    
    /// Generate CMake configuration
    fn generate_cmake_config(&self, sdk_dir: &Path) -> Result<(), Error> {
        let cmake_dir = sdk_dir.join("lib/cmake/veridian");
        fs::create_dir_all(&cmake_dir)?;
        
        // VeridianConfig.cmake
        let config = format!(r#"
# VeridianOS SDK Configuration
set(VERIDIAN_VERSION "{}")
set(VERIDIAN_INCLUDE_DIRS "${{CMAKE_CURRENT_LIST_DIR}}/../../../include")
set(VERIDIAN_LIBRARY_DIRS "${{CMAKE_CURRENT_LIST_DIR}}/../../../lib")

# Find components
set(VERIDIAN_LIBRARIES)

# Core library
find_library(VERIDIAN_CORE_LIBRARY
    NAMES veridian_core
    HINTS ${{VERIDIAN_LIBRARY_DIRS}}
)
list(APPEND VERIDIAN_LIBRARIES ${{VERIDIAN_CORE_LIBRARY}})

# System library
find_library(VERIDIAN_SYSTEM_LIBRARY
    NAMES veridian_system
    HINTS ${{VERIDIAN_LIBRARY_DIRS}}
)
list(APPEND VERIDIAN_LIBRARIES ${{VERIDIAN_SYSTEM_LIBRARY}})

# Create imported targets
add_library(Veridian::Core SHARED IMPORTED)
set_target_properties(Veridian::Core PROPERTIES
    IMPORTED_LOCATION ${{VERIDIAN_CORE_LIBRARY}}
    INTERFACE_INCLUDE_DIRECTORIES ${{VERIDIAN_INCLUDE_DIRS}}
)

add_library(Veridian::System SHARED IMPORTED)
set_target_properties(Veridian::System PROPERTIES
    IMPORTED_LOCATION ${{VERIDIAN_SYSTEM_LIBRARY}}
    INTERFACE_INCLUDE_DIRECTORIES ${{VERIDIAN_INCLUDE_DIRS}}
)
"#, self.version);
        
        fs::write(cmake_dir.join("VeridianConfig.cmake"), config)?;
        
        Ok(())
    }
    
    /// Create example projects
    fn create_examples(&self, sdk_dir: &Path) -> Result<(), Error> {
        let examples_dir = sdk_dir.join("examples");
        fs::create_dir_all(&examples_dir)?;
        
        // Hello World example
        let hello_dir = examples_dir.join("hello");
        fs::create_dir_all(&hello_dir)?;
        
        fs::write(hello_dir.join("main.c"), r#"
#include <veridian/system.h>
#include <veridian/io.h>

int main(int argc, char *argv[]) {
    veridian_init(argc, argv);
    
    veridian_print("Hello from VeridianOS!\n");
    veridian_printf("SDK Version: %s\n", VERIDIAN_VERSION);
    
    return 0;
}
"#)?;
        
        fs::write(hello_dir.join("CMakeLists.txt"), r#"
cmake_minimum_required(VERSION 3.16)
project(hello_veridian)

find_package(Veridian REQUIRED)

add_executable(hello main.c)
target_link_libraries(hello Veridian::System)
"#)?;
        
        // IPC example
        self.create_ipc_example(&examples_dir)?;
        
        // Driver example
        self.create_driver_example(&examples_dir)?;
        
        Ok(())
    }
}
```

### 4. Package Repository Infrastructure

#### 4.1 Repository Server

**reposerver/src/main.rs**
```rust
use axum::{Router, routing::{get, post}, extract::{State, Path, Query}};
use tower_http::services::ServeDir;

/// Package repository server
struct RepositoryServer {
    /// Package storage
    storage: PackageStorage,
    /// Metadata database
    database: Database,
    /// Security configuration
    security: SecurityConfig,
    /// CDN configuration
    cdn: Option<CdnConfig>,
}

impl RepositoryServer {
    /// Start server
    pub async fn run(config: ServerConfig) -> Result<(), Error> {
        let server = Arc::new(Self {
            storage: PackageStorage::new(&config.storage_path)?,
            database: Database::connect(&config.database_url).await?,
            security: config.security,
            cdn: config.cdn,
        });
        
        // Build router
        let app = Router::new()
            // Package download
            .route("/packages/:name/:version", get(Self::download_package))
            // Metadata
            .route("/metadata.json", get(Self::get_metadata))
            .route("/metadata/:arch.json", get(Self::get_arch_metadata))
            // Search
            .route("/search", get(Self::search_packages))
            // Upload (authenticated)
            .route("/upload", post(Self::upload_package))
            // Statistics
            .route("/stats", get(Self::get_statistics))
            // Static files
            .nest_service("/static", ServeDir::new("static"))
            .with_state(server);
        
        // Start server
        let addr = config.listen_addr;
        println!("Repository server listening on {}", addr);
        
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await?;
            
        Ok(())
    }
    
    /// Handle package download
    async fn download_package(
        State(server): State<Arc<Self>>,
        Path((name, version)): Path<(String, String)>,
        Query(params): Query<DownloadParams>,
    ) -> Result<Response, Error> {
        // Check if package exists
        let package = server.database
            .get_package(&name, &version)
            .await?
            .ok_or(Error::PackageNotFound)?;
        
        // Update download statistics
        server.database.increment_downloads(&name, &version).await?;
        
        // Get file path
        let file_path = server.storage.get_package_path(&package)?;
        
        // If CDN is configured, redirect
        if let Some(cdn) = &server.cdn {
            if let Some(cdn_url) = cdn.get_package_url(&package) {
                return Ok(Response::redirect(&cdn_url));
            }
        }
        
        // Serve file directly
        Ok(Response::file(file_path))
    }
    
    /// Upload new package
    async fn upload_package(
        State(server): State<Arc<Self>>,
        auth: AuthToken,
        multipart: Multipart,
    ) -> Result<Response, Error> {
        // Verify authentication
        let uploader = server.security.verify_token(&auth)?;
        
        // Check permissions
        if !uploader.can_upload() {
            return Err(Error::Unauthorized);
        }
        
        // Parse multipart upload
        let (package_data, signature, metadata) = parse_upload(multipart).await?;
        
        // Verify package
        server.verify_package(&package_data, &signature, &metadata)?;
        
        // Check if version already exists
        if server.database.package_exists(&metadata.name, &metadata.version).await? {
            return Err(Error::VersionExists);
        }
        
        // Store package
        let stored_path = server.storage.store_package(
            &metadata.name,
            &metadata.version,
            &package_data,
        )?;
        
        // Add to database
        server.database.add_package(PackageRecord {
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            description: metadata.description,
            uploader: uploader.id,
            upload_time: Utc::now(),
            size: package_data.len() as u64,
            hash: blake3::hash(&package_data).to_hex(),
            signature: base64::encode(&signature),
            downloads: 0,
        }).await?;
        
        // Trigger CDN sync if configured
        if let Some(cdn) = &server.cdn {
            cdn.sync_package(&stored_path).await?;
        }
        
        // Regenerate metadata
        server.regenerate_metadata().await?;
        
        Ok(Response::json(&json!({
            "status": "success",
            "package": metadata.name,
            "version": metadata.version,
        })))
    }
    
    /// Generate repository metadata
    async fn regenerate_metadata(&self) -> Result<(), Error> {
        // Get all packages
        let packages = self.database.all_packages().await?;
        
        // Group by architecture
        let mut by_arch: BTreeMap<String, Vec<PackageEntry>> = BTreeMap::new();
        
        for package in packages {
            let entry = PackageEntry {
                name: package.name.clone(),
                version: package.version.clone(),
                description: package.description.clone(),
                dependencies: self.database.get_dependencies(&package.id).await?,
                size: package.size,
                hash: package.hash.clone(),
                signature: package.signature.clone(),
            };
            
            by_arch.entry(package.arch.clone())
                .or_default()
                .push(entry);
        }
        
        // Generate metadata files
        for (arch, packages) in by_arch {
            let metadata = RepositoryMetadata {
                version: METADATA_VERSION,
                timestamp: Utc::now().timestamp() as u64,
                packages,
                provides: self.generate_provides_map(&packages)?,
            };
            
            let json = serde_json::to_string_pretty(&metadata)?;
            let compressed = compress_gzip(json.as_bytes())?;
            
            // Sign metadata
            let signature = self.security.sign(&compressed)?;
            
            // Store
            self.storage.store_metadata(&arch, &compressed, &signature)?;
        }
        
        Ok(())
    }
}

/// Mirror synchronization
pub struct MirrorSync {
    /// Primary repository
    primary: Url,
    /// Mirror configuration
    config: MirrorConfig,
    /// Sync state
    state: SyncState,
}

impl MirrorSync {
    /// Synchronize with primary
    pub async fn sync(&mut self) -> Result<(), Error> {
        // Get primary metadata
        let primary_metadata = self.fetch_primary_metadata().await?;
        
        // Get local metadata
        let local_metadata = self.get_local_metadata()?;
        
        // Calculate differences
        let diff = self.calculate_diff(&primary_metadata, &local_metadata)?;
        
        println!("Sync required: {} new, {} updated, {} removed",
                 diff.new.len(), diff.updated.len(), diff.removed.len());
        
        // Download new and updated packages
        for package in diff.new.iter().chain(diff.updated.iter()) {
            self.sync_package(package).await?;
        }
        
        // Remove old packages
        for package in diff.removed {
            self.remove_package(&package)?;
        }
        
        // Update metadata
        self.update_metadata(primary_metadata)?;
        
        Ok(())
    }
}
```

## Implementation Timeline

### Month 22-23: Package Manager Core
- Week 1-2: Package database and dependency resolver
- Week 3-4: Repository client and download manager
- Week 5-6: Installation and removal logic
- Week 7-8: Transaction system and rollback

### Month 24: Ports System
- Week 1-2: Port framework implementation
- Week 3-4: Build system integrations

### Month 25: Repository Infrastructure
- Week 1-2: Repository server
- Week 3-4: Mirror synchronization

### Month 26: Development Tools
- Week 1-2: Toolchain manager
- Week 3-4: SDK generator

### Month 27: Integration and Polish
- Week 1-2: Package building automation
- Week 3-4: Documentation and testing

## Testing Strategy

### Unit Tests
- Dependency resolver edge cases
- Package verification
- Transaction rollback
- Port build system

### Integration Tests
- Full package installation/removal
- Repository synchronization
- Cross-compilation
- SDK validation

### System Tests
- Large-scale package operations
- Mirror failover
- Concurrent operations
- Build farm simulation

## Success Criteria

1. **Package Manager**: Reliable dependency resolution < 1s for 10k packages
2. **Ports System**: Support for major build systems
3. **Repository**: Scalable to 100k+ packages
4. **Development Tools**: Complete SDK for all targets
5. **Security**: Signed packages with secure distribution
6. **Performance**: CDN-ready with mirror support

## Dependencies for Phase 5

- Stable package management system
- Repository infrastructure
- Development toolchain
- Build automation
- Distribution network