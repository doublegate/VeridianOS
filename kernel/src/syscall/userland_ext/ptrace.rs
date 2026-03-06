//! ptrace - Process Tracing and Debugging
//!
//! Implements the ptrace system call interface for process tracing,
//! debugging, and system call interception.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

// ============================================================================
// Types and Constants
// ============================================================================

/// ptrace request types
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtraceRequest {
    /// Attach to a process
    Attach = 0,
    /// Detach from a process
    Detach = 1,
    /// Read a word from the tracee's text (code) segment
    PeekText = 2,
    /// Read a word from the tracee's data segment
    PeekData = 3,
    /// Write a word to the tracee's text segment
    PokeText = 4,
    /// Write a word to the tracee's data segment
    PokeData = 5,
    /// Single-step the tracee
    SingleStep = 6,
    /// Continue the tracee
    Cont = 7,
    /// Get register state
    GetRegs = 8,
    /// Set register state
    SetRegs = 9,
    /// Get signal information
    GetSigInfo = 10,
    /// Trace system calls
    Syscall = 11,
    /// Kill the tracee
    Kill = 12,
    /// Set tracing options
    SetOptions = 13,
    /// Get event message
    GetEventMsg = 14,
    /// Peek user area
    PeekUser = 15,
    /// Poke user area
    PokeUser = 16,
}

impl PtraceRequest {
    /// Convert from raw u32
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(Self::Attach),
            1 => Some(Self::Detach),
            2 => Some(Self::PeekText),
            3 => Some(Self::PeekData),
            4 => Some(Self::PokeText),
            5 => Some(Self::PokeData),
            6 => Some(Self::SingleStep),
            7 => Some(Self::Cont),
            8 => Some(Self::GetRegs),
            9 => Some(Self::SetRegs),
            10 => Some(Self::GetSigInfo),
            11 => Some(Self::Syscall),
            12 => Some(Self::Kill),
            13 => Some(Self::SetOptions),
            14 => Some(Self::GetEventMsg),
            15 => Some(Self::PeekUser),
            16 => Some(Self::PokeUser),
            _ => None,
        }
    }
}

/// ptrace error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtraceError {
    /// Process not found
    ProcessNotFound,
    /// Already being traced
    AlreadyTraced,
    /// Not being traced by this process
    NotTraced,
    /// Invalid address for peek/poke
    InvalidAddress,
    /// Permission denied
    PermissionDenied,
    /// Tracee is not stopped
    NotStopped,
    /// Invalid request
    InvalidRequest,
    /// Invalid signal number
    InvalidSignal,
    /// Tracee is dead
    TraceeExited,
    /// Internal error
    InternalError,
}

/// Tracee state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceeState {
    /// Tracee is running normally
    Running,
    /// Tracee is stopped (by signal or ptrace)
    Stopped(u32),
    /// Tracee stopped at syscall entry/exit
    SyscallStop {
        /// true = entry, false = exit
        is_entry: bool,
        /// Syscall number
        syscall_nr: u64,
    },
    /// Tracee stopped for single-step
    SingleStep,
    /// Tracee has exited with status
    Exited(i32),
    /// Tracee killed by signal
    Signaled(u32),
}

/// x86_64 register state (matches Linux struct user_regs_struct layout)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct RegisterState {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub orig_rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
}

/// Signal information structure
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SigInfo {
    /// Signal number
    pub signo: i32,
    /// Error number
    pub errno: i32,
    /// Signal code
    pub code: i32,
    /// Sending process PID
    pub sender_pid: u64,
    /// Fault address (for SIGSEGV, SIGBUS, etc.)
    pub fault_addr: u64,
}

/// ptrace options (set via PTRACE_SETOPTIONS)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PtraceOptions {
    /// Trace fork events
    pub trace_fork: bool,
    /// Trace vfork events
    pub trace_vfork: bool,
    /// Trace clone events
    pub trace_clone: bool,
    /// Trace exec events
    pub trace_exec: bool,
    /// Trace exit events
    pub trace_exit: bool,
    /// Automatically kill tracee when tracer exits
    pub exit_kill: bool,
    /// Trace syscall entry/exit
    pub trace_syscall: bool,
}

// ============================================================================
// Trace Relationship (private)
// ============================================================================

/// Tracer-tracee relationship
#[derive(Debug)]
struct TraceRelation {
    /// Tracer PID
    tracer_pid: u64,
    /// Tracee PID
    tracee_pid: u64,
    /// Current tracee state
    state: TraceeState,
    /// Saved register state (when stopped)
    registers: RegisterState,
    /// Signal info for the stop
    sig_info: SigInfo,
    /// Tracing options
    options: PtraceOptions,
    /// Pending signal to deliver on continue (0 = none)
    pending_signal: u32,
    /// Memory snapshot for peek/poke (address -> value)
    memory_cache: BTreeMap<u64, u64>,
}

// ============================================================================
// PtraceManager
// ============================================================================

/// ptrace manager
#[derive(Debug)]
pub struct PtraceManager {
    /// Active trace relationships (tracee_pid -> TraceRelation)
    relations: BTreeMap<u64, TraceRelation>,
    /// Reverse map (tracer_pid -> list of tracee_pids)
    tracer_to_tracees: BTreeMap<u64, Vec<u64>>,
}

impl Default for PtraceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtraceManager {
    /// Create a new ptrace manager
    pub fn new() -> Self {
        Self {
            relations: BTreeMap::new(),
            tracer_to_tracees: BTreeMap::new(),
        }
    }

    /// Attach to a process for tracing
    pub fn attach(&mut self, tracer_pid: u64, tracee_pid: u64) -> Result<(), PtraceError> {
        // Cannot trace yourself
        if tracer_pid == tracee_pid {
            return Err(PtraceError::PermissionDenied);
        }
        // Cannot attach twice
        if self.relations.contains_key(&tracee_pid) {
            return Err(PtraceError::AlreadyTraced);
        }
        let relation = TraceRelation {
            tracer_pid,
            tracee_pid,
            state: TraceeState::Stopped(19), // SIGSTOP
            registers: RegisterState::default(),
            sig_info: SigInfo {
                signo: 19,
                errno: 0,
                code: 0,
                sender_pid: tracer_pid,
                fault_addr: 0,
            },
            options: PtraceOptions::default(),
            pending_signal: 0,
            memory_cache: BTreeMap::new(),
        };
        self.relations.insert(tracee_pid, relation);
        self.tracer_to_tracees
            .entry(tracer_pid)
            .or_default()
            .push(tracee_pid);
        Ok(())
    }

    /// Detach from a traced process
    pub fn detach(&mut self, tracer_pid: u64, tracee_pid: u64) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        self.relations.remove(&tracee_pid);
        if let Some(tracees) = self.tracer_to_tracees.get_mut(&tracer_pid) {
            tracees.retain(|&pid| pid != tracee_pid);
            if tracees.is_empty() {
                self.tracer_to_tracees.remove(&tracer_pid);
            }
        }
        Ok(())
    }

    /// Continue a stopped tracee, optionally delivering a signal
    pub fn cont(
        &mut self,
        tracer_pid: u64,
        tracee_pid: u64,
        signal: u32,
    ) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        if matches!(relation.state, TraceeState::Running) {
            return Err(PtraceError::NotStopped);
        }
        if matches!(
            relation.state,
            TraceeState::Exited(_) | TraceeState::Signaled(_)
        ) {
            return Err(PtraceError::TraceeExited);
        }
        relation.pending_signal = signal;
        relation.state = TraceeState::Running;
        Ok(())
    }

    /// Single-step the tracee
    pub fn single_step(&mut self, tracer_pid: u64, tracee_pid: u64) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        if matches!(relation.state, TraceeState::Running) {
            return Err(PtraceError::NotStopped);
        }
        // Set RFLAGS.TF for hardware single-step
        relation.registers.rflags |= 1 << 8; // TF bit
        relation.state = TraceeState::SingleStep;
        Ok(())
    }

    /// Trace syscalls (stop at entry and exit)
    pub fn trace_syscall(&mut self, tracer_pid: u64, tracee_pid: u64) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        relation.options.trace_syscall = true;
        if !matches!(relation.state, TraceeState::Running) {
            relation.state = TraceeState::Running;
        }
        Ok(())
    }

    /// Read a word from tracee memory
    pub fn peek_data(
        &self,
        tracer_pid: u64,
        tracee_pid: u64,
        addr: u64,
    ) -> Result<u64, PtraceError> {
        let relation = self
            .relations
            .get(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        // In a real kernel, this reads from the tracee's address space.
        // Stub: return from memory cache
        Ok(*relation.memory_cache.get(&addr).unwrap_or(&0))
    }

    /// Write a word to tracee memory
    pub fn poke_data(
        &mut self,
        tracer_pid: u64,
        tracee_pid: u64,
        addr: u64,
        data: u64,
    ) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        relation.memory_cache.insert(addr, data);
        Ok(())
    }

    /// Get register state of a stopped tracee
    pub fn get_regs(&self, tracer_pid: u64, tracee_pid: u64) -> Result<RegisterState, PtraceError> {
        let relation = self
            .relations
            .get(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        if matches!(relation.state, TraceeState::Running) {
            return Err(PtraceError::NotStopped);
        }
        Ok(relation.registers)
    }

    /// Set register state of a stopped tracee
    pub fn set_regs(
        &mut self,
        tracer_pid: u64,
        tracee_pid: u64,
        regs: RegisterState,
    ) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        if matches!(relation.state, TraceeState::Running) {
            return Err(PtraceError::NotStopped);
        }
        relation.registers = regs;
        Ok(())
    }

    /// Get signal info for the current stop
    pub fn get_sig_info(&self, tracer_pid: u64, tracee_pid: u64) -> Result<SigInfo, PtraceError> {
        let relation = self
            .relations
            .get(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        Ok(relation.sig_info)
    }

    /// Set ptrace options
    pub fn set_options(
        &mut self,
        tracer_pid: u64,
        tracee_pid: u64,
        options: PtraceOptions,
    ) -> Result<(), PtraceError> {
        let relation = self
            .relations
            .get_mut(&tracee_pid)
            .ok_or(PtraceError::NotTraced)?;
        if relation.tracer_pid != tracer_pid {
            return Err(PtraceError::NotTraced);
        }
        relation.options = options;
        Ok(())
    }

    /// Notify the manager that a tracee received a signal
    pub fn on_signal(&mut self, tracee_pid: u64, signal: u32, fault_addr: u64) {
        if let Some(relation) = self.relations.get_mut(&tracee_pid) {
            relation.state = TraceeState::Stopped(signal);
            relation.sig_info = SigInfo {
                signo: signal as i32,
                errno: 0,
                code: 0,
                sender_pid: 0,
                fault_addr,
            };
        }
    }

    /// Notify the manager that a tracee hit a syscall entry/exit
    pub fn on_syscall(&mut self, tracee_pid: u64, is_entry: bool, syscall_nr: u64) {
        if let Some(relation) = self.relations.get_mut(&tracee_pid) {
            if relation.options.trace_syscall {
                relation.state = TraceeState::SyscallStop {
                    is_entry,
                    syscall_nr,
                };
            }
        }
    }

    /// Notify the manager that a tracee exited
    pub fn on_exit(&mut self, tracee_pid: u64, exit_code: i32) {
        if let Some(relation) = self.relations.get_mut(&tracee_pid) {
            relation.state = TraceeState::Exited(exit_code);
        }
    }

    /// Get tracee state
    pub fn get_tracee_state(&self, tracee_pid: u64) -> Option<TraceeState> {
        self.relations.get(&tracee_pid).map(|r| r.state)
    }

    /// Check if a process is being traced
    pub fn is_traced(&self, pid: u64) -> bool {
        self.relations.contains_key(&pid)
    }

    /// Get the tracer of a given tracee
    pub fn get_tracer(&self, tracee_pid: u64) -> Option<u64> {
        self.relations.get(&tracee_pid).map(|r| r.tracer_pid)
    }

    /// Get all tracees of a tracer
    pub fn get_tracees(&self, tracer_pid: u64) -> Vec<u64> {
        self.tracer_to_tracees
            .get(&tracer_pid)
            .cloned()
            .unwrap_or_default()
    }

    /// Detach all tracees when a tracer exits
    pub fn on_tracer_exit(&mut self, tracer_pid: u64) {
        if let Some(tracees) = self.tracer_to_tracees.remove(&tracer_pid) {
            for tracee_pid in tracees {
                self.relations.remove(&tracee_pid);
            }
        }
    }

    /// Number of active trace relationships
    pub fn active_traces(&self) -> usize {
        self.relations.len()
    }
}
