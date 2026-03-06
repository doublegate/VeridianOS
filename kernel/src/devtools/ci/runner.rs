//! CI Job Runner
//!
//! Executes CI jobs defined in TOML configuration files. Each job runs
//! in a namespace-isolated environment with artifact collection.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

/// Job status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
}

/// CI job step
#[derive(Debug, Clone)]
pub struct JobStep {
    pub name: String,
    pub command: String,
    pub status: JobStatus,
    pub output: String,
    pub exit_code: Option<i32>,
}

impl JobStep {
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            status: JobStatus::Pending,
            output: String::new(),
            exit_code: None,
        }
    }
}

/// CI job definition
#[derive(Debug, Clone)]
pub struct Job {
    pub name: String,
    pub steps: Vec<JobStep>,
    pub status: JobStatus,
    pub environment: BTreeMap<String, String>,
    pub artifacts: Vec<String>,
    pub dependencies: Vec<String>,
    pub allow_failure: bool,
}

impl Job {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            steps: Vec::new(),
            status: JobStatus::Pending,
            environment: BTreeMap::new(),
            artifacts: Vec::new(),
            dependencies: Vec::new(),
            allow_failure: false,
        }
    }

    pub fn add_step(&mut self, name: &str, command: &str) {
        self.steps.push(JobStep::new(name, command));
    }

    /// Execute all steps (simulated)
    pub fn execute(&mut self) -> bool {
        self.status = JobStatus::Running;

        for step in &mut self.steps {
            step.status = JobStatus::Running;
            // In real implementation, would fork+exec the command
            step.status = JobStatus::Passed;
            step.exit_code = Some(0);
            step.output = alloc::format!("$ {}\n[ok]", step.command);
        }

        let all_passed = self.steps.iter().all(|s| s.status == JobStatus::Passed);
        self.status = if all_passed || self.allow_failure {
            JobStatus::Passed
        } else {
            JobStatus::Failed
        };

        all_passed
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

/// CI pipeline (collection of jobs)
pub struct Pipeline {
    pub name: String,
    pub jobs: Vec<Job>,
    pub trigger: PipelineTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineTrigger {
    Push,
    PullRequest,
    Manual,
    Schedule(String),
}

impl Pipeline {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            jobs: Vec::new(),
            trigger: PipelineTrigger::Manual,
        }
    }

    pub fn add_job(&mut self, job: Job) {
        self.jobs.push(job);
    }

    /// Execute all jobs in order
    pub fn execute(&mut self) -> bool {
        let mut all_passed = true;
        let mut completed: Vec<String> = Vec::new();

        for job in &mut self.jobs {
            // Check dependencies
            let deps_met = job.dependencies.iter().all(|d| completed.contains(d));
            if !deps_met {
                job.status = JobStatus::Skipped;
                continue;
            }

            if !job.execute() && !job.allow_failure {
                all_passed = false;
            }
            completed.push(job.name.clone());
        }

        all_passed
    }

    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    pub fn passed_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.status == JobStatus::Passed)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.status == JobStatus::Failed)
            .count()
    }
}

/// Build artifact stored after job completion.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub name: String,
    pub size: u64,
    pub job_id: String,
    pub pipeline_id: String,
    pub created_tick: u64,
    pub data_hash: [u8; 32],
}

impl Artifact {
    pub fn new(name: &str, job_id: &str, pipeline_id: &str) -> Self {
        Self {
            name: name.to_string(),
            size: 0,
            job_id: job_id.to_string(),
            pipeline_id: pipeline_id.to_string(),
            created_tick: 0,
            data_hash: [0u8; 32],
        }
    }
}

/// Artifact storage with retention management.
pub struct ArtifactStore {
    artifacts: BTreeMap<String, Artifact>,
}

impl Default for ArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self {
            artifacts: BTreeMap::new(),
        }
    }

    /// Store a build artifact with metadata.
    pub fn store_artifact(
        &mut self,
        name: &str,
        data_hash: [u8; 32],
        size: u64,
        job_id: &str,
        pipeline_id: &str,
        tick: u64,
    ) {
        let mut artifact = Artifact::new(name, job_id, pipeline_id);
        artifact.data_hash = data_hash;
        artifact.size = size;
        artifact.created_tick = tick;
        self.artifacts.insert(name.to_string(), artifact);
    }

    /// Retrieve an artifact by name.
    pub fn get_artifact(&self, name: &str) -> Option<&Artifact> {
        self.artifacts.get(name)
    }

    /// List all stored artifacts.
    pub fn list_artifacts(&self) -> Vec<&Artifact> {
        self.artifacts.values().collect()
    }

    /// Remove artifacts older than the retention tick threshold.
    pub fn cleanup_old(&mut self, current_tick: u64, retention_ticks: u64) -> usize {
        let cutoff = current_tick.saturating_sub(retention_ticks);
        let before = self.artifacts.len();
        self.artifacts.retain(|_, a| a.created_tick >= cutoff);
        before - self.artifacts.len()
    }

    pub fn count(&self) -> usize {
        self.artifacts.len()
    }
}

/// Git repository poller for triggering CI pipelines on new commits.
#[derive(Debug, Clone)]
pub struct GitPoller {
    pub repo_url: String,
    pub branch: String,
    pub last_commit_hash: String,
    pub poll_interval_ticks: u64,
    last_poll_tick: u64,
}

impl GitPoller {
    pub fn new(repo_url: &str, branch: &str, poll_interval: u64) -> Self {
        Self {
            repo_url: repo_url.to_string(),
            branch: branch.to_string(),
            last_commit_hash: String::new(),
            poll_interval_ticks: poll_interval,
            last_poll_tick: 0,
        }
    }

    /// Check whether enough ticks have elapsed for a new poll.
    pub fn should_poll(&self, current_tick: u64) -> bool {
        current_tick.saturating_sub(self.last_poll_tick) >= self.poll_interval_ticks
    }

    /// Check for updates by comparing current HEAD with last known.
    ///
    /// Returns `true` if the commit hash changed (simulated: any non-empty
    /// `current_head` that differs from last known triggers an update).
    pub fn check_for_updates(&mut self, current_head: &str, current_tick: u64) -> bool {
        self.last_poll_tick = current_tick;
        if !current_head.is_empty() && current_head != self.last_commit_hash {
            self.last_commit_hash = current_head.to_string();
            true
        } else {
            false
        }
    }

    /// Trigger a pipeline on the new commit (creates a Pipeline with Push
    /// trigger).
    pub fn trigger_pipeline(&self, name: &str) -> Pipeline {
        let mut pipeline = Pipeline::new(name);
        pipeline.trigger = PipelineTrigger::Push;
        pipeline
    }
}

/// Namespace isolation for CI job sandboxing.
pub struct NamespaceIsolation {
    pub sandbox_id: u64,
    pub active: bool,
}

impl Default for NamespaceIsolation {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceIsolation {
    pub fn new() -> Self {
        Self {
            sandbox_id: 0,
            active: false,
        }
    }

    /// Create a conceptual sandbox for job isolation.
    pub fn create_sandbox(&mut self, job_name: &str) -> u64 {
        // In real implementation, this would create PID/mount namespaces
        let mut hash: u64 = 5381;
        for byte in job_name.bytes() {
            hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
        }
        self.sandbox_id = hash;
        self.active = true;
        self.sandbox_id
    }

    /// Tear down the sandbox.
    pub fn cleanup_sandbox(&mut self) {
        self.active = false;
        self.sandbox_id = 0;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// Pipeline execution report.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    pub pipeline_name: String,
    pub total_jobs: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub artifacts_collected: usize,
}

impl Pipeline {
    /// Execute the pipeline with git polling support.
    ///
    /// Checks the poller, runs if there are new commits, and collects
    /// artifacts into the store.
    pub fn run_with_polling(
        &mut self,
        poller: &mut GitPoller,
        current_head: &str,
        current_tick: u64,
        store: &mut ArtifactStore,
    ) -> Option<PipelineReport> {
        if !poller.should_poll(current_tick) {
            return None;
        }
        if !poller.check_for_updates(current_head, current_tick) {
            return None;
        }

        self.execute();
        let artifacts = self.collect_artifacts(store, current_tick);
        Some(self.generate_report(artifacts))
    }

    /// Collect declared artifacts from completed jobs into the artifact store.
    pub fn collect_artifacts(&self, store: &mut ArtifactStore, tick: u64) -> usize {
        let mut count = 0;
        for job in &self.jobs {
            if job.status != JobStatus::Passed {
                continue;
            }
            for artifact_name in &job.artifacts {
                store.store_artifact(
                    artifact_name,
                    [0u8; 32], // Placeholder hash
                    0,
                    &job.name,
                    &self.name,
                    tick,
                );
                count += 1;
            }
        }
        count
    }

    /// Generate an execution summary report.
    pub fn generate_report(&self, artifacts_collected: usize) -> PipelineReport {
        PipelineReport {
            pipeline_name: self.name.clone(),
            total_jobs: self.jobs.len(),
            passed: self.passed_count(),
            failed: self.failed_count(),
            skipped: self
                .jobs
                .iter()
                .filter(|j| j.status == JobStatus::Skipped)
                .count(),
            artifacts_collected,
        }
    }
}

/// Parse a minimal CI config (key=value style)
pub fn parse_ci_config(config: &str) -> Vec<Job> {
    let mut jobs = Vec::new();
    let mut current_job: Option<Job> = None;

    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(name) = line.strip_prefix("[job.") {
            let name = name.trim_end_matches(']');
            if let Some(job) = current_job.take() {
                jobs.push(job);
            }
            current_job = Some(Job::new(name));
        } else if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            if let Some(ref mut job) = current_job {
                match key {
                    "step" => {
                        let parts: Vec<&str> = value.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            job.add_step(parts[0].trim(), parts[1].trim());
                        } else {
                            job.add_step(value, value);
                        }
                    }
                    "artifact" => job.artifacts.push(value.to_string()),
                    "depends" => job.dependencies.push(value.to_string()),
                    "allow_failure" => job.allow_failure = value == "true",
                    _ => {}
                }
            }
        }
    }

    if let Some(job) = current_job {
        jobs.push(job);
    }

    jobs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_step_new() {
        let step = JobStep::new("build", "cargo build");
        assert_eq!(step.name, "build");
        assert_eq!(step.status, JobStatus::Pending);
    }

    #[test]
    fn test_job_execute() {
        let mut job = Job::new("test");
        job.add_step("build", "cargo build");
        job.add_step("test", "cargo test");
        assert!(job.execute());
        assert_eq!(job.status, JobStatus::Passed);
    }

    #[test]
    fn test_pipeline_execute() {
        let mut pipeline = Pipeline::new("ci");
        let mut build = Job::new("build");
        build.add_step("compile", "cargo build");
        pipeline.add_job(build);

        let mut test = Job::new("test");
        test.add_step("run", "cargo test");
        test.dependencies.push("build".to_string());
        pipeline.add_job(test);

        assert!(pipeline.execute());
        assert_eq!(pipeline.passed_count(), 2);
    }

    #[test]
    fn test_pipeline_skipped_deps() {
        let mut pipeline = Pipeline::new("ci");
        let mut job = Job::new("deploy");
        job.dependencies.push("missing".to_string());
        pipeline.add_job(job);

        pipeline.execute();
        assert_eq!(pipeline.jobs[0].status, JobStatus::Skipped);
    }

    #[test]
    fn test_parse_ci_config() {
        let config = r#"
[job.build]
step = "compile: cargo build --release"
step = "test: cargo test"
artifact = "target/release/app"

[job.deploy]
depends = "build"
step = "deploy: ./deploy.sh"
"#;

        let jobs = parse_ci_config(config);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].step_count(), 2);
        assert_eq!(jobs[0].artifacts.len(), 1);
        assert_eq!(jobs[1].dependencies.len(), 1);
    }

    #[test]
    fn test_job_status_eq() {
        assert_eq!(JobStatus::Pending, JobStatus::Pending);
        assert_ne!(JobStatus::Passed, JobStatus::Failed);
    }

    #[test]
    fn test_pipeline_trigger() {
        assert_eq!(PipelineTrigger::Push, PipelineTrigger::Push);
        assert_ne!(PipelineTrigger::Push, PipelineTrigger::Manual);
    }

    #[test]
    fn test_job_allow_failure() {
        let mut job = Job::new("flaky");
        job.allow_failure = true;
        job.add_step("test", "flaky-test");
        job.execute();
        assert_eq!(job.status, JobStatus::Passed);
    }

    #[test]
    fn test_empty_pipeline() {
        let mut pipeline = Pipeline::new("empty");
        assert!(pipeline.execute());
        assert_eq!(pipeline.job_count(), 0);
    }

    #[test]
    fn test_parse_empty_config() {
        let jobs = parse_ci_config("");
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_parse_config_comments() {
        let config = "# comment\n[job.test]\nstep = \"run: echo ok\"\n";
        let jobs = parse_ci_config(config);
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn test_pipeline_counts() {
        let mut pipeline = Pipeline::new("test");
        let mut j1 = Job::new("a");
        j1.add_step("s", "cmd");
        let mut j2 = Job::new("b");
        j2.add_step("s", "cmd");
        pipeline.add_job(j1);
        pipeline.add_job(j2);
        pipeline.execute();
        assert_eq!(pipeline.passed_count(), 2);
        assert_eq!(pipeline.failed_count(), 0);
    }

    #[test]
    fn test_artifact_store_basic() {
        let mut store = ArtifactStore::new();
        store.store_artifact("app.bin", [0u8; 32], 1024, "build", "ci-1", 100);
        assert_eq!(store.count(), 1);
        let art = store.get_artifact("app.bin").unwrap();
        assert_eq!(art.size, 1024);
        assert_eq!(art.job_id, "build");
    }

    #[test]
    fn test_artifact_store_list() {
        let mut store = ArtifactStore::new();
        store.store_artifact("a.bin", [0u8; 32], 10, "j1", "p1", 1);
        store.store_artifact("b.bin", [0u8; 32], 20, "j2", "p1", 2);
        assert_eq!(store.list_artifacts().len(), 2);
    }

    #[test]
    fn test_artifact_cleanup() {
        let mut store = ArtifactStore::new();
        store.store_artifact("old", [0u8; 32], 10, "j1", "p1", 10);
        store.store_artifact("new", [0u8; 32], 20, "j2", "p1", 100);
        let removed = store.cleanup_old(110, 50);
        assert_eq!(removed, 1);
        assert_eq!(store.count(), 1);
        assert!(store.get_artifact("new").is_some());
    }

    #[test]
    fn test_git_poller_check_updates() {
        let mut poller = GitPoller::new("https://example.com/repo.git", "main", 100);
        assert!(poller.check_for_updates("abc123", 100));
        assert!(!poller.check_for_updates("abc123", 200)); // same hash
        assert!(poller.check_for_updates("def456", 300)); // different hash
    }

    #[test]
    fn test_git_poller_should_poll() {
        let poller = GitPoller::new("https://example.com/repo.git", "main", 100);
        assert!(poller.should_poll(100));
        assert!(!poller.should_poll(50));
    }

    #[test]
    fn test_git_poller_trigger_pipeline() {
        let poller = GitPoller::new("https://example.com/repo.git", "main", 100);
        let pipeline = poller.trigger_pipeline("auto-ci");
        assert_eq!(pipeline.name, "auto-ci");
        assert_eq!(pipeline.trigger, PipelineTrigger::Push);
    }

    #[test]
    fn test_namespace_isolation() {
        let mut ns = NamespaceIsolation::new();
        assert!(!ns.is_active());
        let id = ns.create_sandbox("build-job");
        assert!(id > 0);
        assert!(ns.is_active());
        ns.cleanup_sandbox();
        assert!(!ns.is_active());
    }

    #[test]
    fn test_pipeline_collect_artifacts() {
        let mut pipeline = Pipeline::new("ci");
        let mut job = Job::new("build");
        job.add_step("compile", "cargo build");
        job.artifacts.push("target/app".to_string());
        pipeline.add_job(job);
        pipeline.execute();

        let mut store = ArtifactStore::new();
        let count = pipeline.collect_artifacts(&mut store, 500);
        assert_eq!(count, 1);
        assert!(store.get_artifact("target/app").is_some());
    }

    #[test]
    fn test_pipeline_generate_report() {
        let mut pipeline = Pipeline::new("ci");
        let mut job = Job::new("build");
        job.add_step("compile", "cargo build");
        pipeline.add_job(job);
        pipeline.execute();
        let report = pipeline.generate_report(1);
        assert_eq!(report.pipeline_name, "ci");
        assert_eq!(report.total_jobs, 1);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.artifacts_collected, 1);
    }

    #[test]
    fn test_run_with_polling_no_update() {
        let mut pipeline = Pipeline::new("ci");
        let mut job = Job::new("build");
        job.add_step("compile", "cargo build");
        pipeline.add_job(job);

        let mut poller = GitPoller::new("repo", "main", 100);
        let mut store = ArtifactStore::new();

        // Too early to poll
        let result = pipeline.run_with_polling(&mut poller, "abc", 50, &mut store);
        assert!(result.is_none());

        // Now poll with a commit
        let result = pipeline.run_with_polling(&mut poller, "abc", 100, &mut store);
        assert!(result.is_some());
        assert_eq!(result.unwrap().passed, 1);
    }
}
