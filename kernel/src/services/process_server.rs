//! Process Server Implementation
//!
//! Manages process lifecycle, resource tracking, and process enumeration.

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::RwLock;

use crate::process::{ProcessId, ProcessPriority};

/// Resource limits for a process
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_memory: u64,   // Maximum memory in bytes
    pub max_cpu_time: u64, // Maximum CPU time in microseconds
    pub max_files: u32,    // Maximum open files
    pub max_threads: u32,  // Maximum threads
    pub nice_value: i8,    // Process nice value (-20 to 19)
    pub stack_size: u64,   // Stack size limit
    pub core_size: u64,    // Core dump size limit
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory: 256 * 1024 * 1024, // 256 MB
            max_cpu_time: u64::MAX,        // Unlimited
            max_files: 1024,               // 1024 files
            max_threads: 256,              // 256 threads
            nice_value: 0,                 // Normal priority
            stack_size: 8 * 1024 * 1024,   // 8 MB
            core_size: 0,                  // No core dumps
        }
    }
}

/// Process information
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: ProcessId,
    pub ppid: ProcessId,
    pub name: String,
    pub state: ProcessState,
    pub uid: u32,
    pub gid: u32,
    pub start_time: u64,
    pub cpu_time: u64,
    pub memory_usage: u64,
    pub thread_count: u32,
    pub priority: ProcessPriority,
    pub exit_code: Option<i32>,
    pub command_line: Vec<String>,
    pub environment: Vec<String>,
    pub working_directory: String,
    pub session_id: u32,
    pub terminal: Option<String>,
}

/// Process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Sleeping,
    Waiting,
    Stopped,
    Zombie,
    Dead,
}

/// Session information
#[derive(Debug, Clone)]
pub struct Session {
    pub sid: u32,
    pub leader: ProcessId,
    pub terminal: Option<String>,
    pub processes: Vec<ProcessId>,
}

/// Process group information
#[derive(Debug, Clone)]
pub struct ProcessGroup {
    pub pgid: u32,
    pub leader: ProcessId,
    pub session_id: u32,
    pub processes: Vec<ProcessId>,
}

/// Global process server instance
pub struct ProcessServer {
    /// All processes
    processes: RwLock<BTreeMap<u64, ProcessInfo>>,

    /// Resource limits per process
    resource_limits: RwLock<BTreeMap<u64, ResourceLimits>>,

    /// Sessions
    sessions: RwLock<BTreeMap<u32, Session>>,

    /// Process groups
    process_groups: RwLock<BTreeMap<u32, ProcessGroup>>,

    /// Next PID to allocate
    next_pid: AtomicU64,

    /// Next session ID
    next_sid: AtomicU64,

    /// Next process group ID
    next_pgid: AtomicU64,

    /// Orphaned processes waiting for reaping
    orphans: RwLock<Vec<ProcessId>>,

    /// Process accounting statistics
    total_processes_created: AtomicU64,
    total_processes_exited: AtomicU64,
}

impl ProcessServer {
    /// Create a new process server
    pub fn new() -> Self {
        crate::println!("[PROCESS_SERVER] Creating new ProcessServer...");

        crate::println!("[PROCESS_SERVER] Creating BTreeMaps...");
        let processes = RwLock::new(BTreeMap::new());
        let resource_limits = RwLock::new(BTreeMap::new());
        let sessions = RwLock::new(BTreeMap::new());
        let process_groups = RwLock::new(BTreeMap::new());

        crate::println!("[PROCESS_SERVER] Creating Vec...");
        let orphans = RwLock::new(Vec::new());

        crate::println!("[PROCESS_SERVER] Creating atomics...");
        let next_pid = AtomicU64::new(2); // Start at 2, PID 1 is init
        let next_sid = AtomicU64::new(1);
        let next_pgid = AtomicU64::new(1);
        let total_processes_created = AtomicU64::new(0);
        let total_processes_exited = AtomicU64::new(0);

        crate::println!("[PROCESS_SERVER] Constructing ProcessServer...");
        let server = Self {
            processes,
            resource_limits,
            sessions,
            process_groups,
            next_pid,
            next_sid,
            next_pgid,
            orphans,
            total_processes_created,
            total_processes_exited,
        };

        crate::println!("[PROCESS_SERVER] ProcessServer created successfully");
        server
    }
}

impl Default for ProcessServer {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessServer {
    /// Create a new process
    pub fn create_process(
        &self,
        parent_pid: ProcessId,
        name: String,
        uid: u32,
        gid: u32,
        command_line: Vec<String>,
        environment: Vec<String>,
    ) -> Result<ProcessId, &'static str> {
        let pid = ProcessId(self.next_pid.fetch_add(1, Ordering::SeqCst));

        // Get parent's session and process group
        let (session_id, pgid) = {
            let processes = self.processes.read();
            if let Some(parent) = processes.get(&parent_pid.0) {
                (parent.session_id, parent.session_id) // Inherit from parent
            } else {
                (0, 0) // Init process
            }
        };

        let info = ProcessInfo {
            pid,
            ppid: parent_pid,
            name: name.clone(),
            state: ProcessState::Running,
            uid,
            gid,
            start_time: self.get_system_time(),
            cpu_time: 0,
            memory_usage: 0,
            thread_count: 1,
            priority: ProcessPriority::Normal,
            exit_code: None,
            command_line,
            environment,
            working_directory: String::from("/"),
            session_id,
            terminal: None,
        };

        // Set default resource limits
        let limits = ResourceLimits::default();

        // Add to process table
        self.processes.write().insert(pid.0, info);
        self.resource_limits.write().insert(pid.0, limits);

        // Add to session's process list
        if session_id > 0 {
            if let Some(session) = self.sessions.write().get_mut(&session_id) {
                session.processes.push(pid);
            }
        }

        // Add to process group
        if pgid > 0 {
            if let Some(pg) = self.process_groups.write().get_mut(&pgid) {
                pg.processes.push(pid);
            }
        }

        self.total_processes_created.fetch_add(1, Ordering::Relaxed);

        crate::println!("[PROCESS_SERVER] Created process {} ({})", pid.0, name);
        Ok(pid)
    }

    /// Terminate a process
    pub fn terminate_process(&self, pid: ProcessId, exit_code: i32) -> Result<(), &'static str> {
        let mut processes = self.processes.write();

        if let Some(process) = processes.get_mut(&pid.0) {
            process.state = ProcessState::Zombie;
            process.exit_code = Some(exit_code);

            // Reparent children to init
            let children: Vec<ProcessId> = processes
                .values()
                .filter(|p| p.ppid == pid)
                .map(|p| p.pid)
                .collect();

            for child_pid in children {
                if let Some(child) = processes.get_mut(&child_pid.0) {
                    child.ppid = ProcessId(1); // Reparent to init

                    // Add to orphan list if child is zombie
                    if child.state == ProcessState::Zombie {
                        self.orphans.write().push(child_pid);
                    }
                }
            }

            self.total_processes_exited.fetch_add(1, Ordering::Relaxed);

            crate::println!(
                "[PROCESS_SERVER] Process {} terminated with code {}",
                pid.0,
                exit_code
            );
            Ok(())
        } else {
            Err("Process not found")
        }
    }

    /// Wait for a child process
    pub fn wait_for_child(
        &self,
        parent_pid: ProcessId,
        specific_pid: Option<ProcessId>,
    ) -> Result<(ProcessId, i32), &'static str> {
        let mut processes = self.processes.write();

        // Find zombie children
        let zombie_child = processes
            .values()
            .find(|p| {
                p.ppid == parent_pid
                    && p.state == ProcessState::Zombie
                    && (specific_pid.is_none() || specific_pid == Some(p.pid))
            })
            .map(|p| (p.pid, p.exit_code.unwrap_or(0)));

        if let Some((child_pid, exit_code)) = zombie_child {
            // Reap the zombie
            processes.remove(&child_pid.0);
            self.resource_limits.write().remove(&child_pid.0);

            // Remove from session and process group
            self.remove_from_session_and_group(child_pid);

            crate::println!("[PROCESS_SERVER] Reaped zombie process {}", child_pid.0);
            Ok((child_pid, exit_code))
        } else {
            Err("No zombie children found")
        }
    }

    /// Get process information
    pub fn get_process_info(&self, pid: ProcessId) -> Option<ProcessInfo> {
        self.processes.read().get(&pid.0).cloned()
    }

    /// List all processes
    pub fn list_processes(&self) -> Vec<ProcessInfo> {
        self.processes.read().values().cloned().collect()
    }

    /// Set resource limits for a process
    pub fn set_resource_limits(
        &self,
        pid: ProcessId,
        limits: ResourceLimits,
    ) -> Result<(), &'static str> {
        if self.processes.read().contains_key(&pid.0) {
            self.resource_limits.write().insert(pid.0, limits);
            Ok(())
        } else {
            Err("Process not found")
        }
    }

    /// Get resource limits for a process
    pub fn get_resource_limits(&self, pid: ProcessId) -> Option<ResourceLimits> {
        self.resource_limits.read().get(&pid.0).cloned()
    }

    /// Create a new session
    pub fn create_session(&self, leader_pid: ProcessId) -> Result<u32, &'static str> {
        let sid = self.next_sid.fetch_add(1, Ordering::SeqCst) as u32;

        let session = Session {
            sid,
            leader: leader_pid,
            terminal: None,
            processes: vec![leader_pid],
        };

        self.sessions.write().insert(sid, session);

        // Update process's session ID
        if let Some(process) = self.processes.write().get_mut(&leader_pid.0) {
            process.session_id = sid;
        }

        Ok(sid)
    }

    /// Create a new process group
    pub fn create_process_group(
        &self,
        leader_pid: ProcessId,
        session_id: u32,
    ) -> Result<u32, &'static str> {
        let pgid = self.next_pgid.fetch_add(1, Ordering::SeqCst) as u32;

        let pg = ProcessGroup {
            pgid,
            leader: leader_pid,
            session_id,
            processes: vec![leader_pid],
        };

        self.process_groups.write().insert(pgid, pg);
        Ok(pgid)
    }

    /// Send signal to process
    pub fn send_signal(&self, pid: ProcessId, signal: i32) -> Result<(), &'static str> {
        if let Some(_process) = self.processes.read().get(&pid.0) {
            match signal {
                9 => {
                    // SIGKILL
                    self.terminate_process(pid, -signal)?;
                }
                15 => {
                    // SIGTERM
                    self.terminate_process(pid, 0)?;
                }
                19 => {
                    // SIGSTOP
                    if let Some(process) = self.processes.write().get_mut(&pid.0) {
                        process.state = ProcessState::Stopped;
                    }
                }
                18 => {
                    // SIGCONT
                    if let Some(process) = self.processes.write().get_mut(&pid.0) {
                        if process.state == ProcessState::Stopped {
                            process.state = ProcessState::Running;
                        }
                    }
                }
                _ => {
                    // Handle other signals
                    crate::println!(
                        "[PROCESS_SERVER] Signal {} sent to process {}",
                        signal,
                        pid.0
                    );
                }
            }
            Ok(())
        } else {
            Err("Process not found")
        }
    }

    /// Update process statistics
    pub fn update_process_stats(&self, pid: ProcessId, cpu_time: u64, memory_usage: u64) {
        if let Some(process) = self.processes.write().get_mut(&pid.0) {
            process.cpu_time = cpu_time;
            process.memory_usage = memory_usage;
        }
    }

    /// Clean up zombie processes
    pub fn reap_zombies(&self) -> usize {
        let mut reaped = 0;
        let mut processes = self.processes.write();
        let mut to_remove = Vec::new();

        // Find zombies whose parents are init or dead
        for (pid, process) in processes.iter() {
            if process.state == ProcessState::Zombie
                && (process.ppid.0 == 1 || !processes.contains_key(&process.ppid.0))
            {
                to_remove.push(*pid);
            }
        }

        // Remove zombies
        for pid in to_remove {
            processes.remove(&pid);
            self.resource_limits.write().remove(&pid);
            self.remove_from_session_and_group(ProcessId(pid));
            reaped += 1;
        }

        if reaped > 0 {
            crate::println!("[PROCESS_SERVER] Reaped {} zombie processes", reaped);
        }

        reaped
    }

    /// Get system statistics
    pub fn get_statistics(&self) -> ProcessServerStats {
        let processes = self.processes.read();
        let mut running = 0;
        let mut sleeping = 0;
        let mut stopped = 0;
        let mut zombies = 0;

        for process in processes.values() {
            match process.state {
                ProcessState::Running => running += 1,
                ProcessState::Sleeping | ProcessState::Waiting => sleeping += 1,
                ProcessState::Stopped => stopped += 1,
                ProcessState::Zombie => zombies += 1,
                ProcessState::Dead => {}
            }
        }

        ProcessServerStats {
            total_processes: processes.len(),
            running,
            sleeping,
            stopped,
            zombies,
            total_created: self.total_processes_created.load(Ordering::Relaxed),
            total_exited: self.total_processes_exited.load(Ordering::Relaxed),
            sessions: self.sessions.read().len(),
            process_groups: self.process_groups.read().len(),
        }
    }

    /// List all active process IDs
    pub fn list_process_ids(&self) -> Vec<ProcessId> {
        self.processes
            .read()
            .keys()
            .map(|&pid| ProcessId(pid))
            .collect()
    }

    /// Notify a process that a capability has been revoked
    pub fn notify_capability_revoked(&self, _pid: ProcessId, _cap_id: u64) {
        // Mark the capability as revoked in the process's capability space.
        // In the current design, revocation is tracked globally in the
        // RevocationList, so per-process notification is a best-effort signal
        // that the process should re-validate its cached capabilities.
        crate::security::audit::log_capability_op(_pid.0, _cap_id, 2); // 2 = revoke
    }

    // Helper functions

    fn get_system_time(&self) -> u64 {
        // Get current system time in microseconds
        // For now, return a placeholder
        0
    }

    fn remove_from_session_and_group(&self, pid: ProcessId) {
        // Remove from session
        for session in self.sessions.write().values_mut() {
            session.processes.retain(|&p| p != pid);
        }

        // Remove from process group
        for pg in self.process_groups.write().values_mut() {
            pg.processes.retain(|&p| p != pid);
        }
    }
}

/// Process server statistics
#[derive(Debug)]
pub struct ProcessServerStats {
    pub total_processes: usize,
    pub running: usize,
    pub sleeping: usize,
    pub stopped: usize,
    pub zombies: usize,
    pub total_created: u64,
    pub total_exited: u64,
    pub sessions: usize,
    pub process_groups: usize,
}

/// Global process server instance using OnceLock for safe initialization.
static PROCESS_SERVER: crate::sync::once_lock::OnceLock<ProcessServer> =
    crate::sync::once_lock::OnceLock::new();

/// Initialize the process server
#[allow(clippy::if_same_then_else)]
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    println!("[PROCESS_SERVER] Creating ProcessServer...");
    if PROCESS_SERVER.set(ProcessServer::new()).is_err() {
        println!("[PROCESS_SERVER] WARNING: Already initialized! Skipping re-initialization.");
    } else {
        println!("[PROCESS_SERVER] Process server initialized");
    }
}

/// Try to get the global process server without panicking.
///
/// Returns `None` if the process server has not been initialized via [`init`].
pub fn try_get_process_server() -> Option<&'static ProcessServer> {
    PROCESS_SERVER.get()
}

/// Get the global process server.
///
/// Panics if the process server has not been initialized via [`init`].
/// Prefer [`try_get_process_server`] in contexts where a panic is unacceptable.
pub fn get_process_server() -> &'static ProcessServer {
    PROCESS_SERVER
        .get()
        .expect("Process server not initialized: init() was not called")
}
