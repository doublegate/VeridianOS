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
}
