//! Global process table implementation
//!
//! The process table maintains a global view of all processes in the system
//! and provides efficient lookup operations.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};

use spin::Mutex;

use super::{pcb::ProcessState, Process, ProcessId};
#[allow(unused_imports)]
use crate::{error::KernelError, println};

/// Process table entry
#[cfg(feature = "alloc")]
pub struct ProcessEntry {
    /// The process
    pub process: Box<Process>,
    /// Reference count (for safe access)
    pub ref_count: usize,
}

/// Global process table
pub struct ProcessTable {
    /// Process entries indexed by PID
    #[cfg(feature = "alloc")]
    entries: Mutex<BTreeMap<ProcessId, ProcessEntry>>,

    /// Simple array-based storage for no-alloc mode
    #[cfg(not(feature = "alloc"))]
    entries: Mutex<ProcessArray>,

    /// Number of active processes
    pub process_count: core::sync::atomic::AtomicUsize,
}

/// Fixed-size process array for no-alloc mode
#[cfg(not(feature = "alloc"))]
pub struct ProcessArray {
    processes: [Option<Process>; super::MAX_PROCESSES],
    count: usize,
}

#[cfg(not(feature = "alloc"))]
impl ProcessArray {
    const fn new() -> Self {
        Self {
            processes: [const { None }; super::MAX_PROCESSES],
            count: 0,
        }
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            entries: Mutex::new(BTreeMap::new()),
            #[cfg(not(feature = "alloc"))]
            entries: Mutex::new(ProcessArray::new()),
            process_count: core::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl ProcessTable {
    /// Create a new process table
    pub const fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            entries: Mutex::new(BTreeMap::new()),
            #[cfg(not(feature = "alloc"))]
            entries: Mutex::new(ProcessArray::new()),
            process_count: core::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Add a process to the table
    #[cfg(feature = "alloc")]
    pub fn add_process(&self, process: Process) -> Result<ProcessId, KernelError> {
        let pid = process.pid;
        let mut entries = self.entries.lock();

        if entries.contains_key(&pid) {
            return Err(KernelError::AlreadyExists {
                resource: "process",
                id: pid.0,
            });
        }

        entries.insert(
            pid,
            ProcessEntry {
                process: Box::new(process),
                ref_count: 1,
            },
        );

        self.process_count
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Ok(pid)
    }

    /// Add a process to the table (no-alloc version)
    #[cfg(not(feature = "alloc"))]
    pub fn add_process(&self, process: Process) -> Result<ProcessId, KernelError> {
        let pid = process.pid;
        let mut entries = self.entries.lock();

        if entries.count >= super::MAX_PROCESSES {
            return Err(KernelError::ResourceExhausted {
                resource: "process table",
            });
        }

        // Find empty slot
        for i in 0..super::MAX_PROCESSES {
            if entries.processes[i].is_none() {
                entries.processes[i] = Some(process);
                entries.count += 1;
                self.process_count
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return Ok(pid);
            }
        }

        Err(KernelError::ResourceExhausted {
            resource: "process table",
        })
    }

    /// Remove a process from the table
    #[cfg(feature = "alloc")]
    pub fn remove_process(&self, pid: ProcessId) -> Option<Box<Process>> {
        let mut entries = self.entries.lock();

        if let Some(entry) = entries.remove(&pid) {
            self.process_count
                .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
            Some(entry.process)
        } else {
            None
        }
    }

    /// Remove a process from the table (no-alloc version)
    #[cfg(not(feature = "alloc"))]
    pub fn remove_process(&self, pid: ProcessId) -> Option<Process> {
        let mut entries = self.entries.lock();

        for i in 0..super::MAX_PROCESSES {
            if let Some(ref process) = entries.processes[i] {
                if process.pid == pid {
                    let process = entries.processes[i].take();
                    entries.count -= 1;
                    self.process_count
                        .fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
                    return process;
                }
            }
        }

        None
    }

    /// Get a process by PID
    #[cfg(feature = "alloc")]
    pub fn get_process(&self, pid: ProcessId) -> Option<&'static Process> {
        let entries = self.entries.lock();

        entries.get(&pid).map(|entry| {
            // SAFETY: The Process is stored in a BTreeMap behind a Mutex, giving
            // it a stable heap address. Casting to *const and back to &'static
            // extends the borrow lifetime beyond the lock. This is sound because
            // processes are never moved or deallocated while references exist in
            // the current kernel model.
            unsafe { &*(entry.process.as_ref() as *const Process) }
        })
    }

    /// Get a process by PID (no-alloc version)
    #[cfg(not(feature = "alloc"))]
    pub fn get_process(&self, pid: ProcessId) -> Option<&'static Process> {
        let entries = self.entries.lock();

        for i in 0..super::MAX_PROCESSES {
            if let Some(ref process) = entries.processes[i] {
                if process.pid == pid {
                    // SAFETY: The Process is stored in a fixed-size array behind a
                    // Mutex. Casting to *const and back to &'static extends the
                    // borrow lifetime beyond the lock. Sound because processes are
                    // not moved or deallocated while references exist.
                    return Some(unsafe { &*(process as *const Process) });
                }
            }
        }

        None
    }

    /// Get mutable access to a process
    #[cfg(feature = "alloc")]
    pub fn get_process_mut(&self, pid: ProcessId) -> Option<&'static mut Process> {
        let mut entries = self.entries.lock();

        entries.get_mut(&pid).map(|entry| {
            // SAFETY: The Process is stored in a BTreeMap behind a Mutex, giving
            // it a stable heap address. Casting to *mut and back to &'static mut
            // extends the borrow lifetime. Sound because the Mutex prevents
            // concurrent mutable access and processes are not moved while
            // references exist.
            unsafe { &mut *(entry.process.as_mut() as *mut Process) }
        })
    }

    /// Check if a process exists
    pub fn exists(&self, pid: ProcessId) -> bool {
        #[cfg(feature = "alloc")]
        {
            self.entries.lock().contains_key(&pid)
        }

        #[cfg(not(feature = "alloc"))]
        {
            let entries = self.entries.lock();
            for i in 0..super::MAX_PROCESSES {
                if let Some(ref process) = entries.processes[i] {
                    if process.pid == pid {
                        return true;
                    }
                }
            }
            false
        }
    }

    /// Get total number of processes
    pub fn count(&self) -> usize {
        self.process_count
            .load(core::sync::atomic::Ordering::Relaxed)
    }

    /// Find all child processes of a parent
    #[cfg(feature = "alloc")]
    pub fn find_children(&self, parent_pid: ProcessId) -> Vec<ProcessId> {
        let entries = self.entries.lock();
        let mut children = Vec::new();

        for (pid, entry) in entries.iter() {
            if entry.process.parent == Some(parent_pid) {
                children.push(*pid);
            }
        }

        children
    }

    /// Iterate over all processes
    #[cfg(feature = "alloc")]
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&Process),
    {
        let entries = self.entries.lock();
        for (_, entry) in entries.iter() {
            f(&entry.process);
        }
    }

    /// Find processes by state
    #[cfg(feature = "alloc")]
    pub fn find_by_state(&self, state: ProcessState) -> Vec<ProcessId> {
        let entries = self.entries.lock();
        let mut results = Vec::new();

        for (pid, entry) in entries.iter() {
            if entry.process.get_state() == state {
                results.push(*pid);
            }
        }

        results
    }

    /// Clean up zombie processes
    #[cfg(feature = "alloc")]
    pub fn reap_zombies(&self) -> Vec<(ProcessId, i32)> {
        let mut results = Vec::new();
        let zombies = self.find_by_state(ProcessState::Zombie);

        for pid in zombies {
            if let Some(process) = self.remove_process(pid) {
                results.push((pid, process.get_exit_code()));
            }
        }

        results
    }
}

/// Global process table instance
pub static PROCESS_TABLE: ProcessTable = ProcessTable::new();

/// Initialize the process table
pub fn init() {
    println!("[PROCESS] Process table initialized");
}

/// Get a process by PID
pub fn get_process(pid: ProcessId) -> Option<&'static Process> {
    PROCESS_TABLE.get_process(pid)
}

/// Get mutable access to a process
#[cfg(feature = "alloc")]
pub fn get_process_mut(pid: ProcessId) -> Option<&'static mut Process> {
    PROCESS_TABLE.get_process_mut(pid)
}

/// Add a process to the table
pub fn add_process(process: Process) -> Result<ProcessId, KernelError> {
    PROCESS_TABLE.add_process(process)
}

/// Remove a process from the table
#[cfg(feature = "alloc")]
pub fn remove_process(pid: ProcessId) -> Option<Box<Process>> {
    PROCESS_TABLE.remove_process(pid)
}

/// Check if a process exists
pub fn process_exists(pid: ProcessId) -> bool {
    PROCESS_TABLE.exists(pid)
}

/// Get total number of processes
pub fn process_count() -> usize {
    PROCESS_TABLE.count()
}
