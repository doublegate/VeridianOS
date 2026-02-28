//! Job control for vsh.
//!
//! Manages background jobs, process groups, and job state transitions.
//! Supports `fg`, `bg`, `jobs`, `wait`, and `disown` builtins.

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use crate::syscall;

/// Job status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// Currently running.
    Running,
    /// Stopped by a signal (e.g., SIGTSTP).
    Stopped,
    /// Exited with a status code.
    Done(i32),
    /// Killed by a signal.
    Killed(i32),
}

impl JobStatus {
    /// Human-readable status string.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Running => "Running",
            JobStatus::Stopped => "Stopped",
            JobStatus::Done(_) => "Done",
            JobStatus::Killed(_) => "Killed",
        }
    }

    /// Whether this job has terminated (done or killed).
    pub fn is_terminated(&self) -> bool {
        matches!(self, JobStatus::Done(_) | JobStatus::Killed(_))
    }
}

/// A single job (may consist of multiple processes in a pipeline).
#[derive(Debug, Clone)]
pub struct Job {
    /// Job number (1-based, as displayed by `jobs`).
    pub job_id: usize,
    /// Process group ID (pgid) of the job.
    pub pgid: i32,
    /// PIDs of all processes in this job.
    pub pids: Vec<i32>,
    /// Current status.
    pub status: JobStatus,
    /// The command string (for display).
    pub command: String,
    /// Whether this job is in the background.
    pub background: bool,
    /// Whether this job has been reported to the user since last status change.
    pub notified: bool,
}

/// The job table.
pub struct JobTable {
    /// Active jobs. Indexed by job_id - 1.
    jobs: Vec<Option<Job>>,
    /// The "current" job (most recent background/stopped job), %+ or %%.
    current_job: Option<usize>,
    /// The "previous" job, %-.
    previous_job: Option<usize>,
}

impl JobTable {
    /// Create an empty job table.
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            current_job: None,
            previous_job: None,
        }
    }

    /// Add a new job to the table. Returns the job ID.
    pub fn add_job(
        &mut self,
        pgid: i32,
        pids: Vec<i32>,
        command: String,
        background: bool,
    ) -> usize {
        let status = JobStatus::Running;

        // Find an empty slot or append
        let job_id = self.find_free_slot();
        let job = Job {
            job_id,
            pgid,
            pids,
            status,
            command,
            background,
            notified: false,
        };

        if job_id > self.jobs.len() {
            while self.jobs.len() < job_id {
                self.jobs.push(None);
            }
        }

        if job_id <= self.jobs.len() {
            self.jobs[job_id - 1] = Some(job);
        }

        // Update current/previous
        self.previous_job = self.current_job;
        self.current_job = Some(job_id);

        job_id
    }

    /// Find the next available job ID.
    fn find_free_slot(&self) -> usize {
        for (i, slot) in self.jobs.iter().enumerate() {
            if slot.is_none() {
                return i + 1;
            }
        }
        self.jobs.len() + 1
    }

    /// Get a job by ID.
    pub fn get(&self, job_id: usize) -> Option<&Job> {
        if job_id == 0 || job_id > self.jobs.len() {
            return None;
        }
        self.jobs[job_id - 1].as_ref()
    }

    /// Get a mutable reference to a job by ID.
    pub fn get_mut(&mut self, job_id: usize) -> Option<&mut Job> {
        if job_id == 0 || job_id > self.jobs.len() {
            return None;
        }
        self.jobs[job_id - 1].as_mut()
    }

    /// Remove a job from the table.
    pub fn remove(&mut self, job_id: usize) {
        if job_id > 0 && job_id <= self.jobs.len() {
            self.jobs[job_id - 1] = None;
            if self.current_job == Some(job_id) {
                self.current_job = self.previous_job;
                self.previous_job = None;
            } else if self.previous_job == Some(job_id) {
                self.previous_job = None;
            }
        }
    }

    /// Get the current job ID (`%+` or `%%`).
    pub fn current(&self) -> Option<usize> {
        self.current_job
    }

    /// Get the previous job ID (`%-`).
    pub fn previous(&self) -> Option<usize> {
        self.previous_job
    }

    /// Iterate over all active jobs.
    pub fn iter(&self) -> impl Iterator<Item = &Job> {
        self.jobs.iter().filter_map(|j| j.as_ref())
    }

    /// Update job status by polling for child process changes.
    ///
    /// Uses `waitpid(-1, WNOHANG)` to check for any finished or stopped
    /// child processes.
    pub fn update_status(&mut self) {
        loop {
            let (pid, status) = syscall::sys_waitpid(-1, syscall::WNOHANG);
            if pid <= 0 {
                break;
            }

            let pid = pid as i32;
            let new_status = decode_wait_status(status);

            // Find which job owns this PID
            for job in self.jobs.iter_mut().flatten() {
                if job.pids.contains(&pid) {
                    job.status = new_status;
                    job.notified = false;
                    break;
                }
            }
        }
    }

    /// Report completed/stopped jobs to the user and clean up terminated ones.
    pub fn report_and_clean(&mut self) -> Vec<String> {
        let mut messages = Vec::new();

        let mut to_remove = Vec::new();
        for job in self.jobs.iter_mut().flatten() {
            if !job.notified && job.status != JobStatus::Running {
                let marker = if self.current_job == Some(job.job_id) {
                    "+"
                } else if self.previous_job == Some(job.job_id) {
                    "-"
                } else {
                    " "
                };

                messages.push(format!(
                    "[{}]{} {}                    {}",
                    job.job_id,
                    marker,
                    job.status.as_str(),
                    job.command,
                ));
                job.notified = true;

                if job.status.is_terminated() {
                    to_remove.push(job.job_id);
                }
            }
        }

        for id in to_remove {
            self.remove(id);
        }

        messages
    }

    /// Format the output for the `jobs` builtin.
    pub fn format_jobs(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for job in self.jobs.iter().flatten() {
            let marker = if self.current_job == Some(job.job_id) {
                "+"
            } else if self.previous_job == Some(job.job_id) {
                "-"
            } else {
                " "
            };

            lines.push(format!(
                "[{}]{} {}                    {}",
                job.job_id,
                marker,
                job.status.as_str(),
                job.command,
            ));
        }
        lines
    }

    /// Wait for a specific job to finish.
    pub fn wait_for_job(&mut self, job_id: usize) -> i32 {
        let pids = match self.get(job_id) {
            Some(job) => job.pids.clone(),
            None => return 127,
        };

        let mut last_status = 0;
        for pid in &pids {
            let (ret, status) = syscall::sys_waitpid(*pid, 0);
            if ret > 0 {
                last_status = decode_exit_code(status);
            }
        }

        if let Some(job) = self.get_mut(job_id) {
            job.status = JobStatus::Done(last_status);
            job.notified = false;
        }

        last_status
    }

    /// Find a job by its PID.
    pub fn find_by_pid(&self, pid: i32) -> Option<usize> {
        for job in self.jobs.iter().flatten() {
            if job.pids.contains(&pid) || job.pgid == pid {
                return Some(job.job_id);
            }
        }
        None
    }

    /// Parse a job specifier string: `%N`, `%+`, `%-`, `%%`, `%string`.
    pub fn resolve_job_spec(&self, spec: &str) -> Option<usize> {
        if !spec.starts_with('%') {
            // Could be a bare number (PID)
            if let Some(pid) = parse_i32(spec) {
                return self.find_by_pid(pid);
            }
            return None;
        }

        let rest = &spec[1..];
        match rest {
            "" | "+" | "%" => self.current_job,
            "-" => self.previous_job,
            _ => {
                // Try numeric job ID
                if let Some(n) = parse_usize(rest) {
                    if self.get(n).is_some() {
                        return Some(n);
                    }
                }
                // Try matching by command prefix
                for job in self.jobs.iter().flatten() {
                    if job.command.starts_with(rest) {
                        return Some(job.job_id);
                    }
                }
                None
            }
        }
    }
}

/// Decode a raw wait status value into a JobStatus.
fn decode_wait_status(status: i32) -> JobStatus {
    // POSIX-style status decoding
    if status & 0x7f == 0 {
        // Exited normally
        JobStatus::Done((status >> 8) & 0xff)
    } else if status & 0xff == 0x7f {
        // Stopped
        JobStatus::Stopped
    } else {
        // Killed by signal
        JobStatus::Killed(status & 0x7f)
    }
}

/// Extract exit code from raw wait status.
fn decode_exit_code(status: i32) -> i32 {
    if status & 0x7f == 0 {
        (status >> 8) & 0xff
    } else {
        128 + (status & 0x7f)
    }
}

fn parse_usize(s: &str) -> Option<usize> {
    let mut n: usize = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

fn parse_i32(s: &str) -> Option<i32> {
    let mut n: i32 = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    let neg = if !bytes.is_empty() && bytes[0] == b'-' {
        i = 1;
        true
    } else {
        false
    };
    if i >= bytes.len() {
        return None;
    }
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((bytes[i] - b'0') as i32)?;
        i += 1;
    }
    Some(if neg { -n } else { n })
}
