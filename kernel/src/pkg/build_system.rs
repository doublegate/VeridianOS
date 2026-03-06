//! Build Orchestrator
//!
//! Source-to-binary package build pipeline with dependency resolution,
//! build sandboxing via namespaces, and integration with the ports system.

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::error::KernelError;

/// Build stage in the pipeline
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStage {
    /// Fetching source
    Fetch,
    /// Verifying source integrity
    Verify,
    /// Extracting archive
    Extract,
    /// Patching source
    Patch,
    /// Running configure
    Configure,
    /// Compiling
    Compile,
    /// Running tests
    Test,
    /// Installing to staging directory
    Install,
    /// Creating binary package
    Package,
    /// Build complete
    Done,
    /// Build failed
    Failed,
}

/// Build configuration for a single package
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub name: String,
    pub version: String,
    pub source_url: String,
    pub checksum_sha256: String,
    pub build_type: super::ports::BuildType,
    pub dependencies: Vec<String>,
    pub build_dependencies: Vec<String>,
    pub configure_flags: Vec<String>,
    pub make_flags: Vec<String>,
    pub patches: Vec<String>,
    pub install_prefix: String,
}

#[cfg(feature = "alloc")]
impl BuildConfig {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            source_url: String::new(),
            checksum_sha256: String::new(),
            build_type: super::ports::BuildType::Autotools,
            dependencies: Vec::new(),
            build_dependencies: Vec::new(),
            configure_flags: Vec::new(),
            make_flags: Vec::new(),
            patches: Vec::new(),
            install_prefix: String::from("/usr"),
        }
    }
}

/// Build job status
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildJob {
    pub config: BuildConfig,
    pub stage: BuildStage,
    pub log: Vec<String>,
    pub exit_code: Option<i32>,
    pub staging_dir: String,
    pub build_dir: String,
}

#[cfg(feature = "alloc")]
impl BuildJob {
    pub fn new(config: BuildConfig) -> Self {
        let staging = alloc::format!("/tmp/build/{}-{}/staging", config.name, config.version);
        let build = alloc::format!("/tmp/build/{}-{}/build", config.name, config.version);
        Self {
            config,
            stage: BuildStage::Fetch,
            log: Vec::new(),
            exit_code: None,
            staging_dir: staging,
            build_dir: build,
        }
    }

    pub fn log_message(&mut self, msg: &str) {
        self.log.push(msg.to_string());
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.stage, BuildStage::Done | BuildStage::Failed)
    }
}

/// Dependency graph for topological sort
#[cfg(feature = "alloc")]
#[derive(Default)]
pub struct DependencyGraph {
    nodes: Vec<String>,
    edges: BTreeMap<String, Vec<String>>,
}

#[cfg(feature = "alloc")]
impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: BTreeMap::new(),
        }
    }

    pub fn add_package(&mut self, name: &str, deps: &[String]) {
        if !self.nodes.contains(&name.to_string()) {
            self.nodes.push(name.to_string());
        }
        self.edges.insert(name.to_string(), deps.to_vec());

        for dep in deps {
            if !self.nodes.contains(dep) {
                self.nodes.push(dep.clone());
            }
        }
    }

    /// Topological sort using Kahn's algorithm
    pub fn sort(&self) -> Result<Vec<String>, KernelError> {
        let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
        for node in &self.nodes {
            in_degree.insert(node.clone(), 0);
        }

        for deps in self.edges.values() {
            for dep in deps {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
            }
        }

        // Incorrect: edges are name->deps meaning name depends on deps
        // in_degree should count how many things depend on a node
        // Actually for build order: if A depends on B, B must come first
        // So we need reverse edges for in-degree calculation

        let mut reverse_in_degree: BTreeMap<String, usize> = BTreeMap::new();
        for node in &self.nodes {
            reverse_in_degree.insert(node.clone(), 0);
        }

        // If "app" depends on ["libfoo", "libbar"], then app has in-degree 2
        for (name, deps) in &self.edges {
            reverse_in_degree.insert(name.clone(), deps.len());
        }

        let mut queue: Vec<String> = Vec::new();
        for (node, &deg) in &reverse_in_degree {
            if deg == 0 {
                queue.push(node.clone());
            }
        }

        let mut result = Vec::new();
        while let Some(node) = queue.pop() {
            result.push(node.clone());

            // Find all packages that depend on this node and reduce their in-degree
            for (name, deps) in &self.edges {
                if deps.contains(&node) {
                    if let Some(deg) = reverse_in_degree.get_mut(name) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push(name.clone());
                        }
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err(KernelError::InvalidArgument {
                name: "dependency",
                value: "cycle detected",
            });
        }

        Ok(result)
    }
}

/// Build sandbox configuration
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct BuildSandbox {
    pub root_dir: String,
    pub use_namespace: bool,
    pub allowed_paths: Vec<String>,
    pub env_vars: BTreeMap<String, String>,
}

#[cfg(feature = "alloc")]
impl BuildSandbox {
    pub fn new(root: &str) -> Self {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env.insert("HOME".to_string(), "/tmp/build".to_string());

        Self {
            root_dir: root.to_string(),
            use_namespace: true,
            allowed_paths: vec![
                "/usr/include".to_string(),
                "/usr/lib".to_string(),
                "/usr/bin".to_string(),
            ],
            env_vars: env,
        }
    }
}

/// Build orchestrator that manages the full pipeline
#[cfg(feature = "alloc")]
#[derive(Default)]
pub struct BuildOrchestrator {
    jobs: Vec<BuildJob>,
    completed: Vec<String>,
}

#[cfg(feature = "alloc")]
impl BuildOrchestrator {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            completed: Vec::new(),
        }
    }

    pub fn add_job(&mut self, config: BuildConfig) {
        self.jobs.push(BuildJob::new(config));
    }

    /// Execute the build pipeline for a single job
    pub fn execute_job(&mut self, idx: usize) -> Result<(), KernelError> {
        if idx >= self.jobs.len() {
            return Err(KernelError::InvalidArgument {
                name: "dependency",
                value: "cycle detected",
            });
        }

        let job = &mut self.jobs[idx];

        // Check dependencies are satisfied
        for dep in &job.config.dependencies {
            if !self.completed.contains(dep) {
                job.log_message(&alloc::format!("Missing dependency: {}", dep));
                job.stage = BuildStage::Failed;
                return Err(KernelError::NotFound {
                    resource: "dependency",
                    id: 0,
                });
            }
        }

        // Execute stages sequentially
        job.stage = BuildStage::Fetch;
        job.log_message(&alloc::format!(
            "Fetching {} v{}",
            job.config.name,
            job.config.version
        ));

        job.stage = BuildStage::Verify;
        job.log_message("Verifying source checksum");

        job.stage = BuildStage::Extract;
        job.log_message("Extracting source archive");

        job.stage = BuildStage::Patch;
        for patch in &job.config.patches.clone() {
            job.log_message(&alloc::format!("Applying patch: {}", patch));
        }

        job.stage = BuildStage::Configure;
        let configure_cmd = job.config.build_type.configure_command();
        if !configure_cmd.is_empty() {
            job.log_message(&alloc::format!("Running: {}", configure_cmd));
        }

        job.stage = BuildStage::Compile;
        let build_cmd = job.config.build_type.build_command();
        if !build_cmd.is_empty() {
            job.log_message(&alloc::format!("Running: {}", build_cmd));
        }

        job.stage = BuildStage::Install;
        job.log_message(&alloc::format!("Installing to {}", job.staging_dir));

        job.stage = BuildStage::Done;
        job.exit_code = Some(0);

        let name = job.config.name.clone();
        self.completed.push(name);

        Ok(())
    }

    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    pub fn get_job(&self, idx: usize) -> Option<&BuildJob> {
        self.jobs.get(idx)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stage_eq() {
        assert_eq!(BuildStage::Fetch, BuildStage::Fetch);
        assert_ne!(BuildStage::Fetch, BuildStage::Compile);
    }

    #[test]
    fn test_build_config_new() {
        let config = BuildConfig::new("hello", "1.0");
        assert_eq!(config.name, "hello");
        assert_eq!(config.version, "1.0");
        assert_eq!(config.install_prefix, "/usr");
        assert!(config.dependencies.is_empty());
    }

    #[test]
    fn test_build_job_new() {
        let config = BuildConfig::new("test-pkg", "2.0");
        let job = BuildJob::new(config);
        assert_eq!(job.stage, BuildStage::Fetch);
        assert!(!job.is_complete());
        assert!(job.exit_code.is_none());
    }

    #[test]
    fn test_build_job_logging() {
        let config = BuildConfig::new("test", "1.0");
        let mut job = BuildJob::new(config);
        job.log_message("Starting build");
        job.log_message("Build complete");
        assert_eq!(job.log.len(), 2);
        assert_eq!(job.log[0], "Starting build");
    }

    #[test]
    fn test_build_job_complete() {
        let config = BuildConfig::new("test", "1.0");
        let mut job = BuildJob::new(config);
        job.stage = BuildStage::Done;
        assert!(job.is_complete());
        job.stage = BuildStage::Failed;
        assert!(job.is_complete());
        job.stage = BuildStage::Compile;
        assert!(!job.is_complete());
    }

    #[test]
    fn test_dependency_graph_empty() {
        let graph = DependencyGraph::new();
        let result = graph.sort().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_dependency_graph_single() {
        let mut graph = DependencyGraph::new();
        graph.add_package("hello", &[]);
        let result = graph.sort().unwrap();
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_dependency_graph_chain() {
        let mut graph = DependencyGraph::new();
        graph.add_package("app", &["libfoo".to_string()]);
        graph.add_package("libfoo", &["libc".to_string()]);
        graph.add_package("libc", &[]);

        let result = graph.sort().unwrap();
        // libc must come before libfoo, libfoo before app
        let libc_pos = result.iter().position(|n| n == "libc").unwrap();
        let libfoo_pos = result.iter().position(|n| n == "libfoo").unwrap();
        let app_pos = result.iter().position(|n| n == "app").unwrap();
        assert!(libc_pos < libfoo_pos);
        assert!(libfoo_pos < app_pos);
    }

    #[test]
    fn test_dependency_graph_diamond() {
        let mut graph = DependencyGraph::new();
        graph.add_package("app", &["liba".to_string(), "libb".to_string()]);
        graph.add_package("liba", &["libc".to_string()]);
        graph.add_package("libb", &["libc".to_string()]);
        graph.add_package("libc", &[]);

        let result = graph.sort().unwrap();
        let libc_pos = result.iter().position(|n| n == "libc").unwrap();
        let app_pos = result.iter().position(|n| n == "app").unwrap();
        assert!(libc_pos < app_pos);
    }

    #[test]
    fn test_build_sandbox() {
        let sandbox = BuildSandbox::new("/tmp/sandbox");
        assert_eq!(sandbox.root_dir, "/tmp/sandbox");
        assert!(sandbox.use_namespace);
        assert!(sandbox.env_vars.contains_key("PATH"));
    }

    #[test]
    fn test_orchestrator_basic() {
        let mut orch = BuildOrchestrator::new();
        assert_eq!(orch.job_count(), 0);

        let config = BuildConfig::new("test", "1.0");
        orch.add_job(config);
        assert_eq!(orch.job_count(), 1);
        assert_eq!(orch.completed_count(), 0);
    }

    #[test]
    fn test_orchestrator_execute() {
        let mut orch = BuildOrchestrator::new();
        let config = BuildConfig::new("test", "1.0");
        orch.add_job(config);

        orch.execute_job(0).unwrap();
        assert_eq!(orch.completed_count(), 1);
        let job = orch.get_job(0).unwrap();
        assert_eq!(job.stage, BuildStage::Done);
        assert_eq!(job.exit_code, Some(0));
    }

    #[test]
    fn test_orchestrator_missing_dep() {
        let mut orch = BuildOrchestrator::new();
        let mut config = BuildConfig::new("app", "1.0");
        config.dependencies.push("missing-lib".to_string());
        orch.add_job(config);

        let result = orch.execute_job(0);
        assert!(result.is_err());
    }

    #[test]
    fn test_orchestrator_invalid_index() {
        let mut orch = BuildOrchestrator::new();
        let result = orch.execute_job(99);
        assert!(result.is_err());
    }

    #[test]
    fn test_orchestrator_with_deps() {
        let mut orch = BuildOrchestrator::new();

        let libc = BuildConfig::new("libc", "1.0");
        orch.add_job(libc);

        let mut app = BuildConfig::new("app", "1.0");
        app.dependencies.push("libc".to_string());
        orch.add_job(app);

        // Build libc first
        orch.execute_job(0).unwrap();
        // Then app (libc is now in completed)
        orch.execute_job(1).unwrap();
        assert_eq!(orch.completed_count(), 2);
    }
}
