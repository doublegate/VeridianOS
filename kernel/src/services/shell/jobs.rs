//! Job control for the VeridianOS shell.
//!
//! Provides background job tracking, status management, and display
//! formatting. Each job represents a pipeline or command group that
//! can be suspended, resumed, or run in the background.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// JobStatus
// ---------------------------------------------------------------------------

/// Current state of a shell job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// The job is currently executing.
    Running,
    /// The job has been suspended (e.g. via Ctrl-Z / SIGTSTP).
    Stopped,
    /// The job has finished executing.
    Done,
}

impl JobStatus {
    /// Human-readable label for display.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Running => "Running",
            JobStatus::Stopped => "Stopped",
            JobStatus::Done => "Done",
        }
    }
}

impl core::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Job
// ---------------------------------------------------------------------------

/// A single shell job — one or more processes forming a pipeline.
#[derive(Debug, Clone)]
pub struct Job {
    /// Unique job identifier (1-based, as displayed to the user).
    pub job_id: u32,
    /// Process IDs belonging to this job (pipeline members).
    pub pids: Vec<u64>,
    /// Current status of the job.
    pub status: JobStatus,
    /// The original command line that created the job.
    pub command_line: String,
}

impl Job {
    /// Create a new job.
    pub fn new(job_id: u32, pids: Vec<u64>, command_line: String) -> Self {
        Self {
            job_id,
            pids,
            status: JobStatus::Running,
            command_line,
        }
    }

    /// Check whether the job has completed.
    pub fn is_done(&self) -> bool {
        self.status == JobStatus::Done
    }

    /// Check whether the job is currently stopped.
    pub fn is_stopped(&self) -> bool {
        self.status == JobStatus::Stopped
    }

    /// Check whether the job is currently running.
    pub fn is_running(&self) -> bool {
        self.status == JobStatus::Running
    }

    /// Return the leader PID (first process in the pipeline).
    pub fn leader_pid(&self) -> Option<u64> {
        self.pids.first().copied()
    }

    /// Return the number of processes in this job.
    pub fn process_count(&self) -> usize {
        self.pids.len()
    }
}

// ---------------------------------------------------------------------------
// Format helper
// ---------------------------------------------------------------------------

/// Format a job for display, matching traditional shell output:
///
/// ```text
/// [1]+  Running                 sleep 60 &
/// [2]-  Stopped                 vim file.txt
/// ```
pub fn format_job(job: &Job) -> String {
    format!(
        "[{}]  {:<20}  {}",
        job.job_id,
        job.status.as_str(),
        job.command_line,
    )
}

/// Format a job with a current/previous indicator (`+` / `-` / ` `).
pub fn format_job_with_indicator(job: &Job, indicator: char) -> String {
    format!(
        "[{}]{}  {:<20}  {}",
        job.job_id,
        indicator,
        job.status.as_str(),
        job.command_line,
    )
}

// ---------------------------------------------------------------------------
// JobTable
// ---------------------------------------------------------------------------

/// Manages the set of active shell jobs.
///
/// Job IDs are assigned sequentially starting from 1 and are reused once a
/// job is removed from the table.
pub struct JobTable {
    /// Active jobs keyed by job ID.
    jobs: BTreeMap<u32, Job>,
    /// The next job ID to assign.
    next_id: u32,
}

impl Default for JobTable {
    fn default() -> Self {
        Self::new()
    }
}

impl JobTable {
    /// Create an empty job table.
    pub fn new() -> Self {
        Self {
            jobs: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Add a new job to the table.
    ///
    /// Returns the assigned job ID.
    pub fn add_job(&mut self, pids: Vec<u64>, command_line: String) -> u32 {
        let id = self.next_id;
        let job = Job::new(id, pids, command_line);
        self.jobs.insert(id, job);
        self.next_id = id + 1;
        id
    }

    /// Remove a job from the table by ID.
    ///
    /// Returns the removed job, or `None` if the ID was not found.
    pub fn remove_job(&mut self, job_id: u32) -> Option<Job> {
        self.jobs.remove(&job_id)
    }

    /// Get an immutable reference to a job by ID.
    pub fn get_job(&self, job_id: u32) -> Option<&Job> {
        self.jobs.get(&job_id)
    }

    /// Get a mutable reference to a job by ID.
    pub fn get_job_mut(&mut self, job_id: u32) -> Option<&mut Job> {
        self.jobs.get_mut(&job_id)
    }

    /// List all jobs ordered by job ID.
    pub fn list_jobs(&self) -> Vec<&Job> {
        self.jobs.values().collect()
    }

    /// Update the status of a specific job.
    ///
    /// Returns `true` if the job was found and updated, `false` otherwise.
    pub fn update_status(&mut self, job_id: u32, status: JobStatus) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.status = status;
            true
        } else {
            false
        }
    }

    /// Find a job by any of its PIDs.
    pub fn find_by_pid(&self, pid: u64) -> Option<&Job> {
        self.jobs.values().find(|j| j.pids.contains(&pid))
    }

    /// Find a job by any of its PIDs (mutable).
    pub fn find_by_pid_mut(&mut self, pid: u64) -> Option<&mut Job> {
        self.jobs.values_mut().find(|j| j.pids.contains(&pid))
    }

    /// Return the number of active jobs.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Check whether the job table is empty.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Remove all jobs that have completed (status == Done).
    ///
    /// Returns the list of removed jobs so the caller can report them.
    pub fn reap_done(&mut self) -> Vec<Job> {
        let done_ids: Vec<u32> = self
            .jobs
            .iter()
            .filter(|(_, j)| j.status == JobStatus::Done)
            .map(|(&id, _)| id)
            .collect();

        let mut reaped = Vec::new();
        for id in done_ids {
            if let Some(job) = self.jobs.remove(&id) {
                reaped.push(job);
            }
        }
        reaped
    }

    /// Return the most recently added job (highest ID), if any.
    pub fn current_job(&self) -> Option<&Job> {
        self.jobs.values().next_back()
    }

    /// Return the second most recently added job, if any.
    pub fn previous_job(&self) -> Option<&Job> {
        let mut iter = self.jobs.values().rev();
        let _current = iter.next(); // skip most recent
        iter.next()
    }

    /// Format all jobs for display.
    pub fn format_all(&self) -> Vec<String> {
        let jobs: Vec<&Job> = self.list_jobs();
        let len = jobs.len();
        let mut output = Vec::with_capacity(len);

        for (i, job) in jobs.iter().enumerate() {
            let indicator = if i == len - 1 {
                '+' // current
            } else if i == len.wrapping_sub(2) && len >= 2 {
                '-' // previous
            } else {
                ' '
            };
            output.push(format_job_with_indicator(job, indicator));
        }
        output
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_creation() {
        let job = Job::new(1, vec![100, 101], String::from("ls | grep foo"));
        assert_eq!(job.job_id, 1);
        assert_eq!(job.pids, vec![100, 101]);
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.command_line, "ls | grep foo");
        assert!(job.is_running());
        assert!(!job.is_done());
        assert!(!job.is_stopped());
    }

    #[test]
    fn test_job_leader_pid() {
        let job = Job::new(1, vec![42, 43], String::from("cmd"));
        assert_eq!(job.leader_pid(), Some(42));

        let empty_job = Job::new(2, vec![], String::from("empty"));
        assert_eq!(empty_job.leader_pid(), None);
    }

    #[test]
    fn test_job_table_add_remove() {
        let mut table = JobTable::new();
        assert!(table.is_empty());

        let id1 = table.add_job(vec![10], String::from("sleep 60"));
        assert_eq!(id1, 1);
        assert_eq!(table.len(), 1);

        let id2 = table.add_job(vec![20, 21], String::from("ls | wc"));
        assert_eq!(id2, 2);
        assert_eq!(table.len(), 2);

        let removed = table.remove_job(1);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().command_line, "sleep 60");
        assert_eq!(table.len(), 1);

        assert!(table.remove_job(99).is_none());
    }

    #[test]
    fn test_job_table_get() {
        let mut table = JobTable::new();
        table.add_job(vec![10], String::from("cmd1"));
        table.add_job(vec![20], String::from("cmd2"));

        assert!(table.get_job(1).is_some());
        assert_eq!(table.get_job(1).unwrap().command_line, "cmd1");
        assert!(table.get_job(3).is_none());
    }

    #[test]
    fn test_job_table_update_status() {
        let mut table = JobTable::new();
        table.add_job(vec![10], String::from("vim"));

        assert!(table.update_status(1, JobStatus::Stopped));
        assert_eq!(table.get_job(1).unwrap().status, JobStatus::Stopped);

        assert!(table.update_status(1, JobStatus::Done));
        assert_eq!(table.get_job(1).unwrap().status, JobStatus::Done);

        assert!(!table.update_status(99, JobStatus::Running));
    }

    #[test]
    fn test_job_table_find_by_pid() {
        let mut table = JobTable::new();
        table.add_job(vec![10, 11], String::from("pipeline"));
        table.add_job(vec![20], String::from("single"));

        assert!(table.find_by_pid(10).is_some());
        assert_eq!(table.find_by_pid(10).unwrap().job_id, 1);
        assert!(table.find_by_pid(11).is_some());
        assert_eq!(table.find_by_pid(11).unwrap().job_id, 1);
        assert!(table.find_by_pid(20).is_some());
        assert_eq!(table.find_by_pid(20).unwrap().job_id, 2);
        assert!(table.find_by_pid(99).is_none());
    }

    #[test]
    fn test_job_table_list_jobs() {
        let mut table = JobTable::new();
        table.add_job(vec![10], String::from("a"));
        table.add_job(vec![20], String::from("b"));
        table.add_job(vec![30], String::from("c"));

        let jobs = table.list_jobs();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].job_id, 1);
        assert_eq!(jobs[1].job_id, 2);
        assert_eq!(jobs[2].job_id, 3);
    }

    #[test]
    fn test_job_table_reap_done() {
        let mut table = JobTable::new();
        table.add_job(vec![10], String::from("done1"));
        table.add_job(vec![20], String::from("running"));
        table.add_job(vec![30], String::from("done2"));

        table.update_status(1, JobStatus::Done);
        table.update_status(3, JobStatus::Done);

        let reaped = table.reap_done();
        assert_eq!(reaped.len(), 2);
        assert_eq!(table.len(), 1);
        assert!(table.get_job(2).is_some());
    }

    #[test]
    fn test_job_table_current_previous() {
        let mut table = JobTable::new();
        assert!(table.current_job().is_none());
        assert!(table.previous_job().is_none());

        table.add_job(vec![10], String::from("first"));
        assert_eq!(table.current_job().unwrap().job_id, 1);
        assert!(table.previous_job().is_none());

        table.add_job(vec![20], String::from("second"));
        assert_eq!(table.current_job().unwrap().job_id, 2);
        assert_eq!(table.previous_job().unwrap().job_id, 1);
    }

    #[test]
    fn test_format_job() {
        let job = Job::new(1, vec![42], String::from("sleep 60 &"));
        let formatted = format_job(&job);
        assert!(formatted.contains("[1]"));
        assert!(formatted.contains("Running"));
        assert!(formatted.contains("sleep 60 &"));
    }

    #[test]
    fn test_format_job_with_indicator() {
        let job = Job::new(3, vec![42], String::from("top"));
        let formatted = format_job_with_indicator(&job, '+');
        assert!(formatted.contains("[3]+"));
        assert!(formatted.contains("Running"));
        assert!(formatted.contains("top"));
    }

    #[test]
    fn test_job_status_display() {
        assert_eq!(JobStatus::Running.as_str(), "Running");
        assert_eq!(JobStatus::Stopped.as_str(), "Stopped");
        assert_eq!(JobStatus::Done.as_str(), "Done");
    }
}
