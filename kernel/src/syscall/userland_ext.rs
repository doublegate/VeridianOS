//! Shell/Userland Extensions for VeridianOS (Phase 7.5 Wave 8)
//!
//! Implements six subsystems for advanced userland support:
//! 1. io_uring - Async I/O submission/completion ring interface
//! 2. ptrace - Process tracing and debugging
//! 3. Core dump - ELF core file generation
//! 4. User/Group management - /etc/passwd, shadow, group
//! 5. sudo/su privilege elevation - sudoers, authentication
//! 6. Crontab scheduler - Periodic job scheduling

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// 1. io_uring - Asynchronous I/O Interface
// ============================================================================

/// io_uring operation opcodes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoUringOpcode {
    /// No operation (used for testing/padding)
    Nop = 0,
    /// Vectored read
    Readv = 1,
    /// Vectored write
    Writev = 2,
    /// File sync
    Fsync = 3,
    /// Add poll monitor
    PollAdd = 4,
    /// Remove poll monitor
    PollRemove = 5,
    /// Send message on socket
    Sendmsg = 6,
    /// Receive message from socket
    Recvmsg = 7,
    /// Timeout operation
    Timeout = 8,
    /// Accept connection
    Accept = 9,
    /// Cancel pending operation
    AsyncCancel = 10,
    /// Open file
    Openat = 11,
    /// Close file descriptor
    Close = 12,
    /// Statx
    Statx = 13,
    /// Read fixed buffer
    ReadFixed = 14,
    /// Write fixed buffer
    WriteFixed = 15,
}

impl IoUringOpcode {
    /// Convert from raw u8 value
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Nop),
            1 => Some(Self::Readv),
            2 => Some(Self::Writev),
            3 => Some(Self::Fsync),
            4 => Some(Self::PollAdd),
            5 => Some(Self::PollRemove),
            6 => Some(Self::Sendmsg),
            7 => Some(Self::Recvmsg),
            8 => Some(Self::Timeout),
            9 => Some(Self::Accept),
            10 => Some(Self::AsyncCancel),
            11 => Some(Self::Openat),
            12 => Some(Self::Close),
            13 => Some(Self::Statx),
            14 => Some(Self::ReadFixed),
            15 => Some(Self::WriteFixed),
            _ => None,
        }
    }
}

/// Submission queue entry flags
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqeFlag {
    /// No flags
    None = 0,
    /// Use fixed file set
    FixedFile = 1 << 0,
    /// Issue after inflight IO completes
    IoDrain = 1 << 1,
    /// Link this SQE to the next
    IoLink = 1 << 2,
    /// Hard-link (fail chain on error)
    IoHardlink = 1 << 3,
    /// Use async worker thread
    Async = 1 << 4,
    /// Use registered buffer
    BufferSelect = 1 << 5,
}

/// Submission Queue Entry - describes an I/O operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SqEntry {
    /// Operation code
    pub opcode: u8,
    /// SQE flags (bitwise OR of SqeFlag values)
    pub flags: u8,
    /// I/O priority
    pub ioprio: u16,
    /// File descriptor
    pub fd: i32,
    /// Offset in file (or timeout spec)
    pub off: u64,
    /// Buffer address (or pointer to iovec array)
    pub addr: u64,
    /// Length (bytes or iovec count)
    pub len: u32,
    /// Operation-specific flags
    pub op_flags: u32,
    /// User data passed back in CQE
    pub user_data: u64,
    /// Buffer group ID for buffer selection
    pub buf_group: u16,
    /// Personality ID for credentials
    pub personality: u16,
    /// Splice-related fd
    pub splice_fd_in: i32,
    /// Padding for future use
    pub _pad: [u64; 2],
}

impl SqEntry {
    /// Create a new zeroed SQE
    pub const fn zeroed() -> Self {
        Self {
            opcode: 0,
            flags: 0,
            ioprio: 0,
            fd: -1,
            off: 0,
            addr: 0,
            len: 0,
            op_flags: 0,
            user_data: 0,
            buf_group: 0,
            personality: 0,
            splice_fd_in: -1,
            _pad: [0; 2],
        }
    }

    /// Create a NOP entry
    pub fn nop(user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Nop as u8;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a READV entry
    pub fn readv(fd: i32, iovec_addr: u64, iovec_count: u32, offset: u64, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Readv as u8;
        sqe.fd = fd;
        sqe.addr = iovec_addr;
        sqe.len = iovec_count;
        sqe.off = offset;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a WRITEV entry
    pub fn writev(fd: i32, iovec_addr: u64, iovec_count: u32, offset: u64, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Writev as u8;
        sqe.fd = fd;
        sqe.addr = iovec_addr;
        sqe.len = iovec_count;
        sqe.off = offset;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a FSYNC entry
    pub fn fsync(fd: i32, datasync: bool, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Fsync as u8;
        sqe.fd = fd;
        sqe.op_flags = if datasync { 1 } else { 0 };
        sqe.user_data = user_data;
        sqe
    }

    /// Create a POLL_ADD entry
    pub fn poll_add(fd: i32, poll_mask: u32, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::PollAdd as u8;
        sqe.fd = fd;
        sqe.op_flags = poll_mask;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a SENDMSG entry
    pub fn sendmsg(fd: i32, msg_addr: u64, flags: u32, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Sendmsg as u8;
        sqe.fd = fd;
        sqe.addr = msg_addr;
        sqe.op_flags = flags;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a RECVMSG entry
    pub fn recvmsg(fd: i32, msg_addr: u64, flags: u32, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Recvmsg as u8;
        sqe.fd = fd;
        sqe.addr = msg_addr;
        sqe.op_flags = flags;
        sqe.user_data = user_data;
        sqe
    }

    /// Create a TIMEOUT entry (timeout in nanoseconds)
    pub fn timeout(timeout_ns: u64, count: u32, user_data: u64) -> Self {
        let mut sqe = Self::zeroed();
        sqe.opcode = IoUringOpcode::Timeout as u8;
        sqe.addr = timeout_ns;
        sqe.len = count;
        sqe.user_data = user_data;
        sqe
    }
}

impl Default for SqEntry {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Completion Queue Entry - result of an I/O operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CqEntry {
    /// User data from the corresponding SQE
    pub user_data: u64,
    /// Result of the operation (negative for error)
    pub result: i32,
    /// Completion flags
    pub flags: u32,
}

impl CqEntry {
    /// Create a new CQE
    pub const fn new(user_data: u64, result: i32, flags: u32) -> Self {
        Self {
            user_data,
            result,
            flags,
        }
    }

    /// Check if the operation succeeded
    pub fn is_success(&self) -> bool {
        self.result >= 0
    }

    /// Check if more completions follow in a chain
    pub fn has_more(&self) -> bool {
        self.flags & CQE_F_MORE != 0
    }
}

impl Default for CqEntry {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

/// CQE flag: more completions coming for this SQE
const CQE_F_MORE: u32 = 1 << 0;
/// CQE flag: buffer selected from buffer group
const CQE_F_BUFFER: u32 = 1 << 1;

/// io_uring setup parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoUringParams {
    /// Number of SQ entries (power of 2)
    pub sq_entries: u32,
    /// Number of CQ entries (power of 2, typically 2x sq_entries)
    pub cq_entries: u32,
    /// Setup flags
    pub flags: u32,
    /// SQ thread CPU affinity
    pub sq_thread_cpu: u32,
    /// SQ thread idle timeout (ms)
    pub sq_thread_idle: u32,
}

impl Default for IoUringParams {
    fn default() -> Self {
        Self {
            sq_entries: 32,
            cq_entries: 64,
            flags: 0,
            sq_thread_cpu: 0,
            sq_thread_idle: 1000,
        }
    }
}

/// Setup flags for io_uring
pub const IORING_SETUP_IOPOLL: u32 = 1 << 0;
pub const IORING_SETUP_SQPOLL: u32 = 1 << 1;
pub const IORING_SETUP_SQ_AFF: u32 = 1 << 2;

/// io_uring instance identifier
pub type IoUringId = u64;

/// io_uring submission/completion state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoUringState {
    /// Ring is idle
    Idle,
    /// Ring has pending submissions
    Submitting,
    /// Ring is processing completions
    Completing,
    /// Ring is shut down
    Shutdown,
}

/// io_uring error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoUringError {
    /// Invalid number of entries (not power of 2)
    InvalidEntries,
    /// Ring is full, cannot submit more
    SubmissionQueueFull,
    /// No completions available
    CompletionQueueEmpty,
    /// Invalid opcode in SQE
    InvalidOpcode,
    /// Bad file descriptor
    BadFd,
    /// Ring not found by ID
    NotFound,
    /// Ring already shut down
    Shutdown,
    /// Out of memory for ring allocation
    OutOfMemory,
    /// Operation cancelled
    Cancelled,
    /// Operation timed out
    TimedOut,
    /// Invalid buffer address
    InvalidBuffer,
    /// Permission denied
    PermissionDenied,
}

/// Ring buffer for SQ/CQ entries (generic)
#[derive(Debug)]
pub struct RingBuffer<T: Copy + Default> {
    /// Backing storage
    entries: Vec<T>,
    /// Number of entries (power of 2)
    capacity: u32,
    /// Mask for wrapping (capacity - 1)
    mask: u32,
    /// Head index (consumer reads here)
    head: u32,
    /// Tail index (producer writes here)
    tail: u32,
}

impl<T: Copy + Default> RingBuffer<T> {
    /// Create a new ring buffer with given capacity (must be power of 2)
    pub fn new(capacity: u32) -> Result<Self, IoUringError> {
        if capacity == 0 || (capacity & (capacity - 1)) != 0 {
            return Err(IoUringError::InvalidEntries);
        }
        let entries = vec![T::default(); capacity as usize];
        Ok(Self {
            entries,
            capacity,
            mask: capacity - 1,
            head: 0,
            tail: 0,
        })
    }

    /// Number of entries currently in the ring
    pub fn len(&self) -> u32 {
        self.tail.wrapping_sub(self.head)
    }

    /// Check if ring is empty
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Check if ring is full
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    /// Available space for new entries
    pub fn available(&self) -> u32 {
        self.capacity - self.len()
    }

    /// Push an entry to the tail
    pub fn push(&mut self, entry: T) -> Result<(), IoUringError> {
        if self.is_full() {
            return Err(IoUringError::SubmissionQueueFull);
        }
        let idx = (self.tail & self.mask) as usize;
        self.entries[idx] = entry;
        self.tail = self.tail.wrapping_add(1);
        Ok(())
    }

    /// Pop an entry from the head
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.head & self.mask) as usize;
        let entry = self.entries[idx];
        self.head = self.head.wrapping_add(1);
        Some(entry)
    }

    /// Peek at the head entry without consuming it
    pub fn peek(&self) -> Option<&T> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.head & self.mask) as usize;
        Some(&self.entries[idx])
    }

    /// Get total capacity
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// Pending operation state for processing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingOp {
    /// The SQE being processed
    pub sqe: SqEntry,
    /// Monotonic sequence number
    pub seq: u64,
    /// Whether this op has been cancelled
    pub cancelled: bool,
}

/// io_uring instance
#[derive(Debug)]
pub struct IoUring {
    /// Unique ID for this ring
    pub id: IoUringId,
    /// Submission queue
    sq: RingBuffer<SqEntry>,
    /// Completion queue
    cq: RingBuffer<CqEntry>,
    /// Setup parameters
    params: IoUringParams,
    /// Current state
    state: IoUringState,
    /// Total submissions processed
    submissions_total: u64,
    /// Total completions generated
    completions_total: u64,
    /// Pending operations awaiting completion
    pending: Vec<PendingOp>,
    /// Monotonic sequence counter
    seq_counter: u64,
    /// Registered file descriptors (for FIXED_FILE)
    registered_fds: Vec<i32>,
    /// Registered buffers (address, length)
    registered_buffers: Vec<(u64, u32)>,
    /// Owner process ID
    owner_pid: u64,
}

#[allow(clippy::new_without_default)]
impl IoUring {
    /// Create a new io_uring instance
    pub fn new(id: IoUringId, params: IoUringParams, owner_pid: u64) -> Result<Self, IoUringError> {
        let sq = RingBuffer::new(params.sq_entries)?;
        let cq = RingBuffer::new(params.cq_entries)?;
        Ok(Self {
            id,
            sq,
            cq,
            params,
            state: IoUringState::Idle,
            submissions_total: 0,
            completions_total: 0,
            pending: Vec::new(),
            seq_counter: 0,
            registered_fds: Vec::new(),
            registered_buffers: Vec::new(),
            owner_pid,
        })
    }

    /// Submit an SQE to the submission queue
    pub fn submit(&mut self, sqe: SqEntry) -> Result<u64, IoUringError> {
        if matches!(self.state, IoUringState::Shutdown) {
            return Err(IoUringError::Shutdown);
        }
        // Validate opcode
        if IoUringOpcode::from_u8(sqe.opcode).is_none() {
            return Err(IoUringError::InvalidOpcode);
        }
        self.sq.push(sqe)?;
        let seq = self.seq_counter;
        self.seq_counter += 1;
        self.submissions_total += 1;
        self.state = IoUringState::Submitting;
        Ok(seq)
    }

    /// Submit a batch of SQEs
    pub fn submit_batch(&mut self, sqes: &[SqEntry]) -> Result<u32, IoUringError> {
        if matches!(self.state, IoUringState::Shutdown) {
            return Err(IoUringError::Shutdown);
        }
        let mut submitted = 0u32;
        for sqe in sqes {
            if IoUringOpcode::from_u8(sqe.opcode).is_none() {
                continue;
            }
            match self.sq.push(*sqe) {
                Ok(()) => {
                    self.seq_counter += 1;
                    self.submissions_total += 1;
                    submitted += 1;
                }
                Err(IoUringError::SubmissionQueueFull) => break,
                Err(e) => return Err(e),
            }
        }
        if submitted > 0 {
            self.state = IoUringState::Submitting;
        }
        Ok(submitted)
    }

    /// Process pending submissions (kernel-side processing)
    ///
    /// Drains the SQ, "processes" each entry (stub: generates immediate
    /// completion), and pushes CQEs.
    pub fn process_submissions(&mut self) -> u32 {
        let mut processed = 0u32;
        while let Some(sqe) = self.sq.pop() {
            let result = self.process_single_sqe(&sqe);
            let cqe = CqEntry::new(sqe.user_data, result, 0);
            // Best effort: if CQ is full, drop the completion
            let _ = self.cq.push(cqe);
            self.completions_total += 1;
            processed += 1;
        }
        if processed > 0 {
            self.state = IoUringState::Completing;
        } else if self.cq.is_empty() {
            self.state = IoUringState::Idle;
        }
        processed
    }

    /// Process a single SQE and return result code
    fn process_single_sqe(&self, sqe: &SqEntry) -> i32 {
        match IoUringOpcode::from_u8(sqe.opcode) {
            Some(IoUringOpcode::Nop) => 0,
            Some(IoUringOpcode::Readv) => {
                if sqe.fd < 0 {
                    return -9; // EBADF
                }
                // Stub: return bytes "read" equal to len
                sqe.len as i32
            }
            Some(IoUringOpcode::Writev) => {
                if sqe.fd < 0 {
                    return -9; // EBADF
                }
                sqe.len as i32
            }
            Some(IoUringOpcode::Fsync) => {
                if sqe.fd < 0 {
                    return -9;
                }
                0
            }
            Some(IoUringOpcode::PollAdd) => 0,
            Some(IoUringOpcode::PollRemove) => 0,
            Some(IoUringOpcode::Sendmsg) => {
                if sqe.fd < 0 {
                    return -9;
                }
                sqe.len as i32
            }
            Some(IoUringOpcode::Recvmsg) => {
                if sqe.fd < 0 {
                    return -9;
                }
                0
            }
            Some(IoUringOpcode::Timeout) => {
                // Stub: immediate completion
                -62 // ETIME
            }
            Some(IoUringOpcode::Accept) => {
                if sqe.fd < 0 {
                    return -9;
                }
                0
            }
            Some(IoUringOpcode::AsyncCancel) => 0,
            Some(IoUringOpcode::Openat) => 0,
            Some(IoUringOpcode::Close) => {
                if sqe.fd < 0 {
                    return -9;
                }
                0
            }
            Some(IoUringOpcode::Statx) => 0,
            Some(IoUringOpcode::ReadFixed) | Some(IoUringOpcode::WriteFixed) => {
                if sqe.fd < 0 {
                    return -9;
                }
                sqe.len as i32
            }
            None => -22, // EINVAL
        }
    }

    /// Reap a single completion from the CQ
    pub fn reap_completion(&mut self) -> Option<CqEntry> {
        self.cq.pop()
    }

    /// Reap up to `max` completions from the CQ
    pub fn reap_completions(&mut self, max: u32) -> Vec<CqEntry> {
        let mut results = Vec::new();
        for _ in 0..max {
            match self.cq.pop() {
                Some(cqe) => results.push(cqe),
                None => break,
            }
        }
        results
    }

    /// Submit all pending SQEs and wait for at least `min_complete` completions
    pub fn submit_and_wait(&mut self, min_complete: u32) -> Result<u32, IoUringError> {
        if matches!(self.state, IoUringState::Shutdown) {
            return Err(IoUringError::Shutdown);
        }
        // Process all submissions first
        self.process_submissions();
        // In a real implementation, this would block. Stub: return available
        // completions.
        let available = self.cq.len();
        if available < min_complete {
            // Would block here in real implementation
        }
        Ok(available)
    }

    /// Register file descriptors for FIXED_FILE operations
    pub fn register_files(&mut self, fds: &[i32]) -> Result<(), IoUringError> {
        if matches!(self.state, IoUringState::Shutdown) {
            return Err(IoUringError::Shutdown);
        }
        self.registered_fds = fds.to_vec();
        Ok(())
    }

    /// Register buffers for fixed buffer operations
    pub fn register_buffers(&mut self, buffers: &[(u64, u32)]) -> Result<(), IoUringError> {
        if matches!(self.state, IoUringState::Shutdown) {
            return Err(IoUringError::Shutdown);
        }
        self.registered_buffers = buffers.to_vec();
        Ok(())
    }

    /// Unregister all files
    pub fn unregister_files(&mut self) {
        self.registered_fds.clear();
    }

    /// Unregister all buffers
    pub fn unregister_buffers(&mut self) {
        self.registered_buffers.clear();
    }

    /// Get number of pending submissions in the SQ
    pub fn sq_pending(&self) -> u32 {
        self.sq.len()
    }

    /// Get number of ready completions in the CQ
    pub fn cq_ready(&self) -> u32 {
        self.cq.len()
    }

    /// Get total submissions ever processed
    pub fn total_submissions(&self) -> u64 {
        self.submissions_total
    }

    /// Get total completions ever generated
    pub fn total_completions(&self) -> u64 {
        self.completions_total
    }

    /// Get current ring state
    pub fn state(&self) -> IoUringState {
        self.state
    }

    /// Shutdown this ring (no more submissions accepted)
    pub fn shutdown(&mut self) {
        self.state = IoUringState::Shutdown;
        // Drain pending submissions as cancelled
        while let Some(sqe) = self.sq.pop() {
            let cqe = CqEntry::new(sqe.user_data, -125, 0); // ECANCELED
            let _ = self.cq.push(cqe);
        }
    }

    /// Get ring parameters
    pub fn params(&self) -> &IoUringParams {
        &self.params
    }

    /// Get owner PID
    pub fn owner_pid(&self) -> u64 {
        self.owner_pid
    }
}

/// Global io_uring manager
#[derive(Debug)]
pub struct IoUringManager {
    /// All active rings indexed by ID
    rings: BTreeMap<IoUringId, IoUring>,
    /// Next ring ID
    next_id: AtomicU64,
}

impl Default for IoUringManager {
    fn default() -> Self {
        Self::new()
    }
}

impl IoUringManager {
    /// Create a new io_uring manager
    pub fn new() -> Self {
        Self {
            rings: BTreeMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Set up a new io_uring instance
    pub fn setup(
        &mut self,
        params: IoUringParams,
        owner_pid: u64,
    ) -> Result<IoUringId, IoUringError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let ring = IoUring::new(id, params, owner_pid)?;
        self.rings.insert(id, ring);
        Ok(id)
    }

    /// Get a mutable reference to a ring by ID
    pub fn get_ring_mut(&mut self, id: IoUringId) -> Option<&mut IoUring> {
        self.rings.get_mut(&id)
    }

    /// Get an immutable reference to a ring by ID
    pub fn get_ring(&self, id: IoUringId) -> Option<&IoUring> {
        self.rings.get(&id)
    }

    /// Destroy a ring
    pub fn destroy(&mut self, id: IoUringId) -> Result<(), IoUringError> {
        match self.rings.remove(&id) {
            Some(mut ring) => {
                ring.shutdown();
                Ok(())
            }
            None => Err(IoUringError::NotFound),
        }
    }

    /// Get number of active rings
    pub fn active_rings(&self) -> usize {
        self.rings.len()
    }

    /// Process submissions for all rings
    pub fn process_all(&mut self) -> u32 {
        let mut total = 0u32;
        for ring in self.rings.values_mut() {
            total += ring.process_submissions();
        }
        total
    }
}

// ============================================================================
// 2. ptrace - Process Tracing and Debugging
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

// ============================================================================
// 3. Core Dump - ELF Core File Generation
// ============================================================================

/// ELF class (64-bit)
const ELFCLASS64: u8 = 2;
/// ELF data (little-endian)
const ELFDATA2LSB: u8 = 1;
/// ELF version (current)
const EV_CURRENT: u8 = 1;
/// ELF OS/ABI (System V)
const ELFOSABI_NONE: u8 = 0;
/// ELF type: core dump
const ET_CORE: u16 = 4;
/// ELF machine: x86_64
const EM_X86_64: u16 = 62;

/// Program header type: note segment
const PT_NOTE: u32 = 4;
/// Program header type: loadable segment
const PT_LOAD: u32 = 1;

/// Note type: prstatus (register state)
const NT_PRSTATUS: u32 = 1;
/// Note type: prpsinfo (process info)
const NT_PRPSINFO: u32 = 3;
/// Note type: auxv (auxiliary vector)
const NT_AUXV: u32 = 6;
/// Note type: file mappings
const NT_FILE: u32 = 0x46494C45;

/// Note name for core dumps
const CORE_NOTE_NAME: &[u8] = b"CORE\0\0\0\0"; // padded to 8 bytes

/// ELF file header size (64-bit)
const ELF64_EHDR_SIZE: usize = 64;
/// ELF program header size (64-bit)
const ELF64_PHDR_SIZE: usize = 56;

/// Core dump error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDumpError {
    /// Process not found
    ProcessNotFound,
    /// No memory mappings available
    NoMappings,
    /// Buffer allocation failure
    OutOfMemory,
    /// Permission denied (cannot dump another process)
    PermissionDenied,
    /// Core dumps disabled for this process
    Disabled,
    /// Internal error during generation
    InternalError,
}

/// Memory segment descriptor for a core dump
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreMemorySegment {
    /// Virtual address start
    pub vaddr: u64,
    /// Size in bytes
    pub size: u64,
    /// Flags (PF_R=4, PF_W=2, PF_X=1)
    pub flags: u32,
    /// Offset in the core file where data is stored
    pub file_offset: u64,
}

/// Process status information for NT_PRSTATUS note
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrStatus {
    /// Signal that caused the dump
    pub signal: i32,
    /// Process ID
    pub pid: u64,
    /// Parent process ID
    pub ppid: u64,
    /// Process group ID
    pub pgrp: u64,
    /// Session ID
    pub sid: u64,
    /// User time (microseconds, integer)
    pub user_time_us: u64,
    /// System time (microseconds, integer)
    pub sys_time_us: u64,
    /// Register state at time of dump
    pub registers: RegisterState,
}

/// Process info for NT_PRPSINFO note
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrPsInfo {
    /// Process state character ('R', 'S', 'D', 'T', 'Z', etc.)
    pub state: u8,
    /// Filename of the executable (up to 16 chars)
    pub fname: [u8; 16],
    /// Command line arguments (up to 80 chars)
    pub psargs: [u8; 80],
    /// Process ID
    pub pid: u64,
    /// Parent PID
    pub ppid: u64,
    /// Process group ID
    pub pgrp: u64,
    /// Session ID
    pub sid: u64,
    /// User ID
    pub uid: u32,
    /// Group ID
    pub gid: u32,
}

impl Default for PrPsInfo {
    fn default() -> Self {
        Self {
            state: b'R',
            fname: [0u8; 16],
            psargs: [0u8; 80],
            pid: 0,
            ppid: 0,
            pgrp: 0,
            sid: 0,
            uid: 0,
            gid: 0,
        }
    }
}

impl PrPsInfo {
    /// Set the filename from a string
    pub fn set_fname(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = core::cmp::min(bytes.len(), 15);
        self.fname[..len].copy_from_slice(&bytes[..len]);
        self.fname[len] = 0;
    }

    /// Set the command arguments from a string
    pub fn set_psargs(&mut self, args: &str) {
        let bytes = args.as_bytes();
        let len = core::cmp::min(bytes.len(), 79);
        self.psargs[..len].copy_from_slice(&bytes[..len]);
        self.psargs[len] = 0;
    }
}

/// Core dump writer
#[derive(Debug)]
pub struct CoreDumpWriter {
    /// Process status
    pub prstatus: PrStatus,
    /// Process info
    pub prpsinfo: PrPsInfo,
    /// Memory segments
    pub segments: Vec<CoreMemorySegment>,
    /// Memory content for each segment (indexed by segment index)
    pub segment_data: Vec<Vec<u8>>,
}

impl Default for CoreDumpWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreDumpWriter {
    /// Create a new core dump writer
    pub fn new() -> Self {
        Self {
            prstatus: PrStatus::default(),
            prpsinfo: PrPsInfo::default(),
            segments: Vec::new(),
            segment_data: Vec::new(),
        }
    }

    /// Add a memory segment to the core dump
    pub fn add_segment(&mut self, vaddr: u64, flags: u32, data: Vec<u8>) {
        let seg = CoreMemorySegment {
            vaddr,
            size: data.len() as u64,
            flags,
            file_offset: 0, // computed during write
        };
        self.segments.push(seg);
        self.segment_data.push(data);
    }

    /// Write a u16 in little-endian to a buffer
    fn write_u16(buf: &mut Vec<u8>, val: u16) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write a u32 in little-endian to a buffer
    fn write_u32(buf: &mut Vec<u8>, val: u32) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write a u64 in little-endian to a buffer
    fn write_u64(buf: &mut Vec<u8>, val: u64) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write an i32 in little-endian to a buffer
    fn write_i32(buf: &mut Vec<u8>, val: i32) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Build the ELF header
    fn build_elf_header(&self, phnum: u16, buf: &mut Vec<u8>) {
        // e_ident: magic, class, data, version, OS/ABI, padding
        buf.extend_from_slice(&[0x7F, b'E', b'L', b'F']); // magic
        buf.push(ELFCLASS64); // 64-bit
        buf.push(ELFDATA2LSB); // little-endian
        buf.push(EV_CURRENT); // ELF version
        buf.push(ELFOSABI_NONE); // OS/ABI
        buf.extend_from_slice(&[0u8; 8]); // padding
        Self::write_u16(buf, ET_CORE); // e_type
        Self::write_u16(buf, EM_X86_64); // e_machine
        Self::write_u32(buf, 1); // e_version
        Self::write_u64(buf, 0); // e_entry
        Self::write_u64(buf, ELF64_EHDR_SIZE as u64); // e_phoff (immediately after header)
        Self::write_u64(buf, 0); // e_shoff (no section headers)
        Self::write_u32(buf, 0); // e_flags
        Self::write_u16(buf, ELF64_EHDR_SIZE as u16); // e_ehsize
        Self::write_u16(buf, ELF64_PHDR_SIZE as u16); // e_phentsize
        Self::write_u16(buf, phnum); // e_phnum
        Self::write_u16(buf, 0); // e_shentsize
        Self::write_u16(buf, 0); // e_shnum
        Self::write_u16(buf, 0); // e_shstrndx
    }

    /// Build a program header
    #[allow(clippy::too_many_arguments)]
    fn build_phdr(
        buf: &mut Vec<u8>,
        p_type: u32,
        p_flags: u32,
        p_offset: u64,
        p_vaddr: u64,
        p_paddr: u64,
        p_filesz: u64,
        p_memsz: u64,
        p_align: u64,
    ) {
        Self::write_u32(buf, p_type);
        Self::write_u32(buf, p_flags);
        Self::write_u64(buf, p_offset);
        Self::write_u64(buf, p_vaddr);
        Self::write_u64(buf, p_paddr);
        Self::write_u64(buf, p_filesz);
        Self::write_u64(buf, p_memsz);
        Self::write_u64(buf, p_align);
    }

    /// Build a note entry
    fn build_note(buf: &mut Vec<u8>, name: &[u8], note_type: u32, desc: &[u8]) {
        let namesz = name.len() as u32;
        let descsz = desc.len() as u32;
        Self::write_u32(buf, namesz);
        Self::write_u32(buf, descsz);
        Self::write_u32(buf, note_type);
        // Name (padded to 4-byte boundary)
        buf.extend_from_slice(name);
        let name_pad = (4 - (namesz as usize % 4)) % 4;
        for _ in 0..name_pad {
            buf.push(0);
        }
        // Descriptor (padded to 4-byte boundary)
        buf.extend_from_slice(desc);
        let desc_pad = (4 - (descsz as usize % 4)) % 4;
        for _ in 0..desc_pad {
            buf.push(0);
        }
    }

    /// Build NT_PRSTATUS note descriptor
    fn build_prstatus_desc(&self) -> Vec<u8> {
        let mut desc = Vec::with_capacity(336);
        // si_signo, si_code, si_errno
        Self::write_i32(&mut desc, self.prstatus.signal);
        Self::write_i32(&mut desc, 0); // code
        Self::write_i32(&mut desc, 0); // errno
                                       // cursig, sigpend, sighold
        Self::write_u16(&mut desc, self.prstatus.signal as u16);
        desc.extend_from_slice(&[0u8; 6]); // padding
        Self::write_u64(&mut desc, 0); // sigpend
        Self::write_u64(&mut desc, 0); // sighold
                                       // pid, ppid, pgrp, sid
        Self::write_u32(&mut desc, self.prstatus.pid as u32);
        Self::write_u32(&mut desc, self.prstatus.ppid as u32);
        Self::write_u32(&mut desc, self.prstatus.pgrp as u32);
        Self::write_u32(&mut desc, self.prstatus.sid as u32);
        // user time, system time (timeval: sec + usec)
        Self::write_u64(&mut desc, self.prstatus.user_time_us / 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.user_time_us % 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.sys_time_us / 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.sys_time_us % 1_000_000);
        // Registers (all 27 u64 fields of RegisterState)
        let regs = &self.prstatus.registers;
        for &val in &[
            regs.r15,
            regs.r14,
            regs.r13,
            regs.r12,
            regs.rbp,
            regs.rbx,
            regs.r11,
            regs.r10,
            regs.r9,
            regs.r8,
            regs.rax,
            regs.rcx,
            regs.rdx,
            regs.rsi,
            regs.rdi,
            regs.orig_rax,
            regs.rip,
            regs.cs,
            regs.rflags,
            regs.rsp,
            regs.ss,
            regs.fs_base,
            regs.gs_base,
            regs.ds,
            regs.es,
            regs.fs,
            regs.gs,
        ] {
            Self::write_u64(&mut desc, val);
        }
        desc
    }

    /// Build NT_PRPSINFO note descriptor
    fn build_prpsinfo_desc(&self) -> Vec<u8> {
        let mut desc = Vec::with_capacity(136);
        desc.push(self.prpsinfo.state); // pr_state
        desc.extend_from_slice(&self.prpsinfo.fname); // pr_fname
        desc.extend_from_slice(&[0u8; 3]); // padding
        desc.extend_from_slice(&self.prpsinfo.psargs); // pr_psargs
        Self::write_u32(&mut desc, self.prpsinfo.pid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.ppid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.pgrp as u32);
        Self::write_u32(&mut desc, self.prpsinfo.sid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.uid);
        Self::write_u32(&mut desc, self.prpsinfo.gid);
        desc
    }

    /// Generate the complete core dump as a byte vector
    pub fn write_core_dump(&mut self) -> Result<Vec<u8>, CoreDumpError> {
        // Number of program headers: 1 for PT_NOTE + 1 per segment
        let num_segments = self.segments.len();
        let phnum = (1 + num_segments) as u16;

        // Build the notes section
        let mut notes = Vec::new();
        let prstatus_desc = self.build_prstatus_desc();
        Self::build_note(&mut notes, CORE_NOTE_NAME, NT_PRSTATUS, &prstatus_desc);
        let prpsinfo_desc = self.build_prpsinfo_desc();
        Self::build_note(&mut notes, CORE_NOTE_NAME, NT_PRPSINFO, &prpsinfo_desc);

        // Calculate offsets
        let headers_size = ELF64_EHDR_SIZE + (phnum as usize) * ELF64_PHDR_SIZE;
        let notes_offset = headers_size;
        let notes_size = notes.len();

        // Data starts after headers + notes
        let mut data_offset = notes_offset + notes_size;
        // Align to 4096 for segment data
        data_offset = (data_offset + 4095) & !4095;

        // Update segment file offsets
        let mut current_offset = data_offset as u64;
        for seg in &mut self.segments {
            seg.file_offset = current_offset;
            current_offset += seg.size;
        }

        // Build the complete file
        let mut output = Vec::new();

        // ELF header
        self.build_elf_header(phnum, &mut output);

        // PT_NOTE program header
        Self::build_phdr(
            &mut output,
            PT_NOTE,
            0,
            notes_offset as u64,
            0,
            0,
            notes_size as u64,
            notes_size as u64,
            4,
        );

        // PT_LOAD program headers for each segment
        for seg in &self.segments {
            Self::build_phdr(
                &mut output,
                PT_LOAD,
                seg.flags,
                seg.file_offset,
                seg.vaddr,
                0,
                seg.size,
                seg.size,
                4096,
            );
        }

        // Notes
        output.extend_from_slice(&notes);

        // Pad to data offset
        while output.len() < data_offset {
            output.push(0);
        }

        // Segment data
        for data in &self.segment_data {
            output.extend_from_slice(data);
        }

        Ok(output)
    }

    /// Get the total number of segments
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Estimated size of the core dump
    pub fn estimated_size(&self) -> usize {
        let headers = ELF64_EHDR_SIZE + (1 + self.segments.len()) * ELF64_PHDR_SIZE;
        let notes = 512; // estimated
        let data: u64 = self.segments.iter().map(|s| s.size).sum();
        headers + notes + data as usize
    }
}

// ============================================================================
// 4. User/Group Management
// ============================================================================

/// User/group management errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserGroupError {
    /// User not found
    UserNotFound,
    /// Group not found
    GroupNotFound,
    /// User already exists
    UserExists,
    /// Group already exists
    GroupExists,
    /// Invalid UID
    InvalidUid,
    /// Invalid GID
    InvalidGid,
    /// Invalid username (empty, too long, bad chars)
    InvalidUsername,
    /// Invalid group name
    InvalidGroupName,
    /// Authentication failure
    AuthFailure,
    /// Permission denied
    PermissionDenied,
    /// Parse error in config file
    ParseError,
    /// Database full (max users/groups reached)
    DatabaseFull,
    /// Password hash mismatch
    PasswordMismatch,
}

/// Maximum username length
const MAX_USERNAME_LEN: usize = 32;
/// Maximum group name length
const MAX_GROUPNAME_LEN: usize = 32;
/// Root UID
const ROOT_UID: u32 = 0;
/// Root GID
const ROOT_GID: u32 = 0;
/// Default shell path
const DEFAULT_SHELL: &str = "/bin/vsh";
/// Default home directory prefix
const DEFAULT_HOME_PREFIX: &str = "/home/";

/// User entry (equivalent to struct passwd)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEntry {
    /// Username
    pub username: String,
    /// User ID
    pub uid: u32,
    /// Primary group ID
    pub gid: u32,
    /// Full name / comment (GECOS field)
    pub gecos: String,
    /// Home directory
    pub home: String,
    /// Login shell
    pub shell: String,
}

impl UserEntry {
    /// Create a new user entry
    pub fn new(username: &str, uid: u32, gid: u32) -> Self {
        let home = if uid == ROOT_UID {
            String::from("/root")
        } else {
            let mut h = String::from(DEFAULT_HOME_PREFIX);
            h.push_str(username);
            h
        };
        Self {
            username: String::from(username),
            uid,
            gid,
            gecos: String::new(),
            home,
            shell: String::from(DEFAULT_SHELL),
        }
    }

    /// Serialize to /etc/passwd format
    pub fn to_passwd_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.username);
        line.push_str(":x:");
        push_u32_str(&mut line, self.uid);
        line.push(':');
        push_u32_str(&mut line, self.gid);
        line.push(':');
        line.push_str(&self.gecos);
        line.push(':');
        line.push_str(&self.home);
        line.push(':');
        line.push_str(&self.shell);
        line
    }

    /// Parse from /etc/passwd format line
    pub fn from_passwd_line(line: &str) -> Result<Self, UserGroupError> {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 7 {
            return Err(UserGroupError::ParseError);
        }
        let uid = parse_u32(parts[2]).ok_or(UserGroupError::ParseError)?;
        let gid = parse_u32(parts[3]).ok_or(UserGroupError::ParseError)?;
        Ok(Self {
            username: String::from(parts[0]),
            uid,
            gid,
            gecos: String::from(parts[4]),
            home: String::from(parts[5]),
            shell: String::from(parts[6]),
        })
    }
}

/// Shadow password entry (equivalent to struct spwd)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowEntry {
    /// Username
    pub username: String,
    /// Hashed password (or "!" for locked, "*" for no login)
    pub password_hash: String,
    /// Days since epoch of last password change
    pub last_change: u64,
    /// Minimum days between password changes
    pub min_days: u64,
    /// Maximum days between password changes
    pub max_days: u64,
    /// Days before expiry to warn user
    pub warn_days: u64,
    /// Days after expiry until account is disabled
    pub inactive_days: u64,
    /// Absolute expiry date (days since epoch, 0 = never)
    pub expire_date: u64,
}

impl ShadowEntry {
    /// Create a new shadow entry with a locked password
    pub fn new_locked(username: &str) -> Self {
        Self {
            username: String::from(username),
            password_hash: String::from("!"),
            last_change: 0,
            min_days: 0,
            max_days: 99999,
            warn_days: 7,
            inactive_days: 0,
            expire_date: 0,
        }
    }

    /// Create a shadow entry with a hashed password
    pub fn with_password(username: &str, hash: &str) -> Self {
        Self {
            username: String::from(username),
            password_hash: String::from(hash),
            last_change: 0,
            min_days: 0,
            max_days: 99999,
            warn_days: 7,
            inactive_days: 0,
            expire_date: 0,
        }
    }

    /// Serialize to /etc/shadow format
    pub fn to_shadow_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.username);
        line.push(':');
        line.push_str(&self.password_hash);
        line.push(':');
        push_u64_str(&mut line, self.last_change);
        line.push(':');
        push_u64_str(&mut line, self.min_days);
        line.push(':');
        push_u64_str(&mut line, self.max_days);
        line.push(':');
        push_u64_str(&mut line, self.warn_days);
        line.push(':');
        push_u64_str(&mut line, self.inactive_days);
        line.push(':');
        push_u64_str(&mut line, self.expire_date);
        line.push(':');
        line
    }

    /// Parse from /etc/shadow format line
    pub fn from_shadow_line(line: &str) -> Result<Self, UserGroupError> {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 8 {
            return Err(UserGroupError::ParseError);
        }
        Ok(Self {
            username: String::from(parts[0]),
            password_hash: String::from(parts[1]),
            last_change: parse_u64(parts[2]).unwrap_or(0),
            min_days: parse_u64(parts[3]).unwrap_or(0),
            max_days: parse_u64(parts[4]).unwrap_or(99999),
            warn_days: parse_u64(parts[5]).unwrap_or(7),
            inactive_days: parse_u64(parts[6]).unwrap_or(0),
            expire_date: parse_u64(parts[7]).unwrap_or(0),
        })
    }

    /// Check if account is locked
    pub fn is_locked(&self) -> bool {
        self.password_hash.starts_with('!') || self.password_hash.starts_with('*')
    }
}

/// Group entry (equivalent to struct group)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    /// Group name
    pub name: String,
    /// Group ID
    pub gid: u32,
    /// Member usernames
    pub members: Vec<String>,
}

impl GroupEntry {
    /// Create a new group entry
    pub fn new(name: &str, gid: u32) -> Self {
        Self {
            name: String::from(name),
            gid,
            members: Vec::new(),
        }
    }

    /// Add a member to this group
    pub fn add_member(&mut self, username: &str) {
        let name = String::from(username);
        if !self.members.contains(&name) {
            self.members.push(name);
        }
    }

    /// Remove a member from this group
    pub fn remove_member(&mut self, username: &str) {
        self.members.retain(|m| m.as_str() != username);
    }

    /// Serialize to /etc/group format
    pub fn to_group_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.name);
        line.push_str(":x:");
        push_u32_str(&mut line, self.gid);
        line.push(':');
        for (i, member) in self.members.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            line.push_str(member);
        }
        line
    }

    /// Parse from /etc/group format line
    pub fn from_group_line(line: &str) -> Result<Self, UserGroupError> {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 3 {
            return Err(UserGroupError::ParseError);
        }
        let gid = parse_u32(if parts.len() > 2 { parts[2] } else { "0" })
            .ok_or(UserGroupError::ParseError)?;
        let members = if parts.len() > 3 && !parts[3].is_empty() {
            parts[3].split(',').map(String::from).collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            name: String::from(parts[0]),
            gid,
            members,
        })
    }
}

/// User database (manages /etc/passwd + /etc/shadow)
#[derive(Debug)]
pub struct UserDatabase {
    /// User entries indexed by UID
    users: BTreeMap<u32, UserEntry>,
    /// Shadow entries indexed by username
    shadows: BTreeMap<String, ShadowEntry>,
    /// Username to UID mapping
    name_to_uid: BTreeMap<String, u32>,
    /// Next available UID
    next_uid: u32,
}

impl Default for UserDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl UserDatabase {
    /// Create a new user database with root user
    pub fn new() -> Self {
        let mut db = Self {
            users: BTreeMap::new(),
            shadows: BTreeMap::new(),
            name_to_uid: BTreeMap::new(),
            next_uid: 1000,
        };
        // Always create root user
        let root = UserEntry {
            username: String::from("root"),
            uid: ROOT_UID,
            gid: ROOT_GID,
            gecos: String::from("root"),
            home: String::from("/root"),
            shell: String::from(DEFAULT_SHELL),
        };
        db.users.insert(ROOT_UID, root);
        db.name_to_uid.insert(String::from("root"), ROOT_UID);
        db.shadows.insert(
            String::from("root"),
            ShadowEntry::with_password("root", "$6$veridian$rootpasswordhash"),
        );
        db
    }

    /// Validate a username
    fn validate_username(name: &str) -> Result<(), UserGroupError> {
        if name.is_empty() || name.len() > MAX_USERNAME_LEN {
            return Err(UserGroupError::InvalidUsername);
        }
        // Must start with a letter or underscore
        let first = name.as_bytes()[0];
        if !first.is_ascii_lowercase() && first != b'_' {
            return Err(UserGroupError::InvalidUsername);
        }
        // Only alphanumeric, underscore, hyphen, dot
        for &b in name.as_bytes() {
            if !b.is_ascii_alphanumeric() && b != b'_' && b != b'-' && b != b'.' {
                return Err(UserGroupError::InvalidUsername);
            }
        }
        Ok(())
    }

    /// Add a new user (useradd)
    pub fn add_user(
        &mut self,
        username: &str,
        gid: u32,
        uid: Option<u32>,
    ) -> Result<u32, UserGroupError> {
        Self::validate_username(username)?;
        if self.name_to_uid.contains_key(username) {
            return Err(UserGroupError::UserExists);
        }
        let uid = uid.unwrap_or_else(|| {
            let u = self.next_uid;
            self.next_uid += 1;
            u
        });
        if self.users.contains_key(&uid) {
            return Err(UserGroupError::InvalidUid);
        }
        let entry = UserEntry::new(username, uid, gid);
        self.users.insert(uid, entry);
        self.name_to_uid.insert(String::from(username), uid);
        self.shadows
            .insert(String::from(username), ShadowEntry::new_locked(username));
        if uid >= self.next_uid {
            self.next_uid = uid + 1;
        }
        Ok(uid)
    }

    /// Remove a user (userdel)
    pub fn remove_user(&mut self, username: &str) -> Result<(), UserGroupError> {
        let uid = self
            .name_to_uid
            .remove(username)
            .ok_or(UserGroupError::UserNotFound)?;
        self.users.remove(&uid);
        self.shadows.remove(username);
        Ok(())
    }

    /// Look up user by UID
    pub fn get_user_by_uid(&self, uid: u32) -> Option<&UserEntry> {
        self.users.get(&uid)
    }

    /// Look up user by username (getpwnam)
    pub fn get_user_by_name(&self, name: &str) -> Option<&UserEntry> {
        self.name_to_uid
            .get(name)
            .and_then(|uid| self.users.get(uid))
    }

    /// Set password for a user
    pub fn set_password(&mut self, username: &str, hash: &str) -> Result<(), UserGroupError> {
        let shadow = self
            .shadows
            .get_mut(username)
            .ok_or(UserGroupError::UserNotFound)?;
        shadow.password_hash = String::from(hash);
        Ok(())
    }

    /// Verify password hash for a user
    pub fn verify_password(&self, username: &str, hash: &str) -> Result<bool, UserGroupError> {
        let shadow = self
            .shadows
            .get(username)
            .ok_or(UserGroupError::UserNotFound)?;
        Ok(shadow.password_hash == hash)
    }

    /// Get total number of users
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Serialize to /etc/passwd format
    pub fn to_passwd_file(&self) -> String {
        let mut output = String::new();
        for user in self.users.values() {
            output.push_str(&user.to_passwd_line());
            output.push('\n');
        }
        output
    }

    /// Serialize to /etc/shadow format
    pub fn to_shadow_file(&self) -> String {
        let mut output = String::new();
        for shadow in self.shadows.values() {
            output.push_str(&shadow.to_shadow_line());
            output.push('\n');
        }
        output
    }

    /// Parse /etc/passwd file content
    pub fn load_passwd(&mut self, content: &str) -> Result<usize, UserGroupError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let entry = UserEntry::from_passwd_line(trimmed)?;
            self.name_to_uid.insert(entry.username.clone(), entry.uid);
            if entry.uid >= self.next_uid {
                self.next_uid = entry.uid + 1;
            }
            self.users.insert(entry.uid, entry);
            count += 1;
        }
        Ok(count)
    }

    /// Parse /etc/shadow file content
    pub fn load_shadow(&mut self, content: &str) -> Result<usize, UserGroupError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let entry = ShadowEntry::from_shadow_line(trimmed)?;
            self.shadows.insert(entry.username.clone(), entry);
            count += 1;
        }
        Ok(count)
    }

    /// Get the shadow entry for a user
    pub fn get_shadow(&self, username: &str) -> Option<&ShadowEntry> {
        self.shadows.get(username)
    }

    /// List all usernames
    pub fn list_usernames(&self) -> Vec<&str> {
        self.users.values().map(|u| u.username.as_str()).collect()
    }
}

/// Group database (manages /etc/group)
#[derive(Debug)]
pub struct GroupDatabase {
    /// Groups indexed by GID
    groups: BTreeMap<u32, GroupEntry>,
    /// Group name to GID mapping
    name_to_gid: BTreeMap<String, u32>,
    /// Next available GID
    next_gid: u32,
}

impl Default for GroupDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupDatabase {
    /// Create a new group database with root group
    pub fn new() -> Self {
        let mut db = Self {
            groups: BTreeMap::new(),
            name_to_gid: BTreeMap::new(),
            next_gid: 1000,
        };
        let root_group = GroupEntry::new("root", ROOT_GID);
        db.groups.insert(ROOT_GID, root_group);
        db.name_to_gid.insert(String::from("root"), ROOT_GID);
        db
    }

    /// Add a new group (groupadd)
    pub fn add_group(&mut self, name: &str, gid: Option<u32>) -> Result<u32, UserGroupError> {
        if name.is_empty() || name.len() > MAX_GROUPNAME_LEN {
            return Err(UserGroupError::InvalidGroupName);
        }
        if self.name_to_gid.contains_key(name) {
            return Err(UserGroupError::GroupExists);
        }
        let gid = gid.unwrap_or_else(|| {
            let g = self.next_gid;
            self.next_gid += 1;
            g
        });
        if self.groups.contains_key(&gid) {
            return Err(UserGroupError::InvalidGid);
        }
        let entry = GroupEntry::new(name, gid);
        self.groups.insert(gid, entry);
        self.name_to_gid.insert(String::from(name), gid);
        if gid >= self.next_gid {
            self.next_gid = gid + 1;
        }
        Ok(gid)
    }

    /// Remove a group (groupdel)
    pub fn remove_group(&mut self, name: &str) -> Result<(), UserGroupError> {
        let gid = self
            .name_to_gid
            .remove(name)
            .ok_or(UserGroupError::GroupNotFound)?;
        self.groups.remove(&gid);
        Ok(())
    }

    /// Look up group by GID
    pub fn get_group_by_gid(&self, gid: u32) -> Option<&GroupEntry> {
        self.groups.get(&gid)
    }

    /// Look up group by name (getgrnam)
    pub fn get_group_by_name(&self, name: &str) -> Option<&GroupEntry> {
        self.name_to_gid
            .get(name)
            .and_then(|gid| self.groups.get(gid))
    }

    /// Add a user to a group
    pub fn add_user_to_group(
        &mut self,
        username: &str,
        group_name: &str,
    ) -> Result<(), UserGroupError> {
        let gid = *self
            .name_to_gid
            .get(group_name)
            .ok_or(UserGroupError::GroupNotFound)?;
        let group = self
            .groups
            .get_mut(&gid)
            .ok_or(UserGroupError::GroupNotFound)?;
        group.add_member(username);
        Ok(())
    }

    /// Remove a user from a group
    pub fn remove_user_from_group(
        &mut self,
        username: &str,
        group_name: &str,
    ) -> Result<(), UserGroupError> {
        let gid = *self
            .name_to_gid
            .get(group_name)
            .ok_or(UserGroupError::GroupNotFound)?;
        let group = self
            .groups
            .get_mut(&gid)
            .ok_or(UserGroupError::GroupNotFound)?;
        group.remove_member(username);
        Ok(())
    }

    /// Get all groups a user belongs to
    pub fn get_user_groups(&self, username: &str) -> Vec<u32> {
        self.groups
            .values()
            .filter(|g| g.members.iter().any(|m| m.as_str() == username))
            .map(|g| g.gid)
            .collect()
    }

    /// Get total number of groups
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Serialize to /etc/group format
    pub fn to_group_file(&self) -> String {
        let mut output = String::new();
        for group in self.groups.values() {
            output.push_str(&group.to_group_line());
            output.push('\n');
        }
        output
    }

    /// Parse /etc/group file content
    pub fn load_group_file(&mut self, content: &str) -> Result<usize, UserGroupError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let entry = GroupEntry::from_group_line(trimmed)?;
            self.name_to_gid.insert(entry.name.clone(), entry.gid);
            if entry.gid >= self.next_gid {
                self.next_gid = entry.gid + 1;
            }
            self.groups.insert(entry.gid, entry);
            count += 1;
        }
        Ok(count)
    }
}

// ============================================================================
// 5. sudo/su Privilege Elevation
// ============================================================================

/// Privilege elevation errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeError {
    /// User not found
    UserNotFound,
    /// Permission denied by sudoers
    PermissionDenied,
    /// Authentication failed (bad password)
    AuthFailed,
    /// sudoers parse error
    ParseError,
    /// Session expired
    SessionExpired,
    /// Target user not found
    TargetUserNotFound,
    /// Operation not permitted
    NotPermitted,
    /// Internal error
    InternalError,
}

/// Sudoers rule specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoersRule {
    /// User or group specification (user name, or %group)
    pub user_spec: String,
    /// Host specification (ALL or hostname)
    pub host_spec: String,
    /// Runas specification (user to run as, ALL = any)
    pub runas_spec: String,
    /// Command specification (ALL or specific path)
    pub command_spec: String,
    /// Whether NOPASSWD is set
    pub nopasswd: bool,
    /// Whether SETENV is allowed
    pub setenv: bool,
}

impl SudoersRule {
    /// Create a standard rule
    pub fn new(user_spec: &str, host_spec: &str, runas_spec: &str, command_spec: &str) -> Self {
        Self {
            user_spec: String::from(user_spec),
            host_spec: String::from(host_spec),
            runas_spec: String::from(runas_spec),
            command_spec: String::from(command_spec),
            nopasswd: false,
            setenv: false,
        }
    }

    /// Create a NOPASSWD rule
    pub fn new_nopasswd(
        user_spec: &str,
        host_spec: &str,
        runas_spec: &str,
        command_spec: &str,
    ) -> Self {
        let mut rule = Self::new(user_spec, host_spec, runas_spec, command_spec);
        rule.nopasswd = true;
        rule
    }

    /// Check if this rule matches a user (direct or group membership)
    pub fn matches_user(&self, username: &str, groups: &[String]) -> bool {
        if self.user_spec == "ALL" || self.user_spec == username {
            return true;
        }
        // Check group match (%group)
        if let Some(group) = self.user_spec.strip_prefix('%') {
            return groups.iter().any(|g| g.as_str() == group);
        }
        false
    }

    /// Check if this rule matches a runas target
    pub fn matches_runas(&self, target_user: &str) -> bool {
        self.runas_spec == "ALL" || self.runas_spec == target_user
    }

    /// Check if this rule matches a command
    pub fn matches_command(&self, command: &str) -> bool {
        if self.command_spec == "ALL" {
            return true;
        }
        // Exact match or prefix match (for paths)
        command == self.command_spec || command.starts_with(&self.command_spec)
    }

    /// Serialize to sudoers format
    pub fn to_sudoers_line(&self) -> String {
        let mut line = String::new();
        line.push_str(&self.user_spec);
        line.push(' ');
        line.push_str(&self.host_spec);
        line.push_str("=(");
        line.push_str(&self.runas_spec);
        line.push_str(") ");
        if self.nopasswd {
            line.push_str("NOPASSWD: ");
        }
        if self.setenv {
            line.push_str("SETENV: ");
        }
        line.push_str(&self.command_spec);
        line
    }
}

/// Sudoers parser and validator
#[derive(Debug)]
pub struct SudoersParser {
    /// Parsed rules
    pub rules: Vec<SudoersRule>,
    /// Host aliases (name -> hostnames)
    pub host_aliases: BTreeMap<String, Vec<String>>,
    /// User aliases (name -> usernames)
    pub user_aliases: BTreeMap<String, Vec<String>>,
    /// Command aliases (name -> commands)
    pub cmnd_aliases: BTreeMap<String, Vec<String>>,
    /// Defaults settings
    pub defaults: Vec<String>,
}

impl Default for SudoersParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SudoersParser {
    /// Create a new empty sudoers parser
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            host_aliases: BTreeMap::new(),
            user_aliases: BTreeMap::new(),
            cmnd_aliases: BTreeMap::new(),
            defaults: Vec::new(),
        }
    }

    /// Parse a sudoers file content
    pub fn parse(&mut self, content: &str) -> Result<usize, PrivilegeError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with("Defaults") {
                self.defaults.push(String::from(trimmed));
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Host_Alias") {
                let mut aliases = self.host_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.host_aliases = aliases;
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("User_Alias") {
                let mut aliases = self.user_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.user_aliases = aliases;
                count += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Cmnd_Alias") {
                let mut aliases = self.cmnd_aliases.clone();
                self.parse_alias(rest.trim(), &mut aliases)?;
                self.cmnd_aliases = aliases;
                count += 1;
                continue;
            }
            // Parse user rule
            if let Some(rule) = self.parse_rule(trimmed) {
                self.rules.push(rule);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Parse an alias definition (NAME = val1, val2, ...)
    fn parse_alias(
        &self,
        spec: &str,
        aliases: &mut BTreeMap<String, Vec<String>>,
    ) -> Result<(), PrivilegeError> {
        let parts: Vec<&str> = spec.splitn(2, '=').collect();
        if parts.len() != 2 {
            return Err(PrivilegeError::ParseError);
        }
        let name = parts[0].trim();
        let values: Vec<String> = parts[1]
            .split(',')
            .map(|s| String::from(s.trim()))
            .collect();
        aliases.insert(String::from(name), values);
        Ok(())
    }

    /// Parse a single sudoers rule line
    fn parse_rule(&self, line: &str) -> Option<SudoersRule> {
        // Format: user host=(runas) [NOPASSWD:] command
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() < 2 {
            return None;
        }
        let user_spec = parts[0];
        let rest = parts[1];

        // Find host=(runas) pattern
        let eq_pos = rest.find('=')?;
        let host_spec = rest[..eq_pos].trim();

        let after_eq = rest[eq_pos + 1..].trim();
        let paren_close = after_eq.find(')')?;
        let runas_spec = after_eq[1..paren_close].trim(); // skip opening '('

        let mut cmd_part = after_eq[paren_close + 1..].trim();
        let mut nopasswd = false;
        let mut setenv = false;

        if let Some(rest_after) = cmd_part.strip_prefix("NOPASSWD:") {
            nopasswd = true;
            cmd_part = rest_after.trim();
        }
        if let Some(rest_after) = cmd_part.strip_prefix("SETENV:") {
            setenv = true;
            cmd_part = rest_after.trim();
        }

        let mut rule = SudoersRule::new(user_spec, host_spec, runas_spec, cmd_part);
        rule.nopasswd = nopasswd;
        rule.setenv = setenv;
        Some(rule)
    }

    /// Add a rule programmatically
    pub fn add_rule(&mut self, rule: SudoersRule) {
        self.rules.push(rule);
    }

    /// Get the number of rules
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

/// Sudo session tracking (timestamp-based)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SudoSession {
    /// User UID
    pub uid: u32,
    /// Terminal/TTY identifier
    pub tty: u64,
    /// Timestamp of last successful auth (monotonic, in seconds)
    pub auth_time: u64,
    /// Session timeout in seconds (default: 300 = 5 minutes)
    pub timeout_secs: u64,
}

impl SudoSession {
    /// Create a new session
    pub fn new(uid: u32, tty: u64, auth_time: u64) -> Self {
        Self {
            uid,
            tty,
            auth_time,
            timeout_secs: 300,
        }
    }

    /// Check if the session is still valid at the given time
    pub fn is_valid(&self, current_time: u64) -> bool {
        current_time.saturating_sub(self.auth_time) < self.timeout_secs
    }

    /// Refresh the session timestamp
    pub fn refresh(&mut self, current_time: u64) {
        self.auth_time = current_time;
    }
}

/// Privilege manager for sudo/su operations
#[derive(Debug)]
pub struct PrivilegeManager {
    /// Sudoers configuration
    pub sudoers: SudoersParser,
    /// Active sudo sessions (uid -> session)
    sessions: BTreeMap<u32, SudoSession>,
    /// Sanitized environment variable names to keep
    env_keep: Vec<String>,
    /// Environment variables to reset
    env_reset: Vec<(String, String)>,
}

impl Default for PrivilegeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivilegeManager {
    /// Create a new privilege manager
    pub fn new() -> Self {
        let env_keep = vec![
            String::from("TERM"),
            String::from("LANG"),
            String::from("LC_ALL"),
        ];
        let env_reset = vec![
            (
                String::from("PATH"),
                String::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"),
            ),
            (String::from("HOME"), String::new()),
            (String::from("USER"), String::new()),
            (String::from("LOGNAME"), String::new()),
            (String::from("SHELL"), String::new()),
        ];
        Self {
            sudoers: SudoersParser::new(),
            sessions: BTreeMap::new(),
            env_keep,
            env_reset,
        }
    }

    /// Check if a user has sudo permission for a command
    pub fn check_sudo_permission(
        &self,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
    ) -> Result<bool, PrivilegeError> {
        for rule in &self.sudoers.rules {
            if rule.matches_user(username, groups)
                && rule.matches_runas(target_user)
                && rule.matches_command(command)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check if NOPASSWD is set for a matching rule
    pub fn is_nopasswd(
        &self,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
    ) -> bool {
        for rule in &self.sudoers.rules {
            if rule.matches_user(username, groups)
                && rule.matches_runas(target_user)
                && rule.matches_command(command)
            {
                return rule.nopasswd;
            }
        }
        false
    }

    /// Execute sudo: validate permissions, optionally authenticate, switch
    /// context
    #[allow(clippy::too_many_arguments)]
    pub fn sudo_exec(
        &mut self,
        uid: u32,
        username: &str,
        groups: &[String],
        target_user: &str,
        command: &str,
        password_hash: Option<&str>,
        current_time: u64,
        tty: u64,
    ) -> Result<SudoExecResult, PrivilegeError> {
        // Check permission
        let has_perm = self.check_sudo_permission(username, groups, target_user, command)?;
        if !has_perm {
            return Err(PrivilegeError::PermissionDenied);
        }

        // Check if NOPASSWD or session is still valid
        let nopasswd = self.is_nopasswd(username, groups, target_user, command);
        let session_valid = self
            .sessions
            .get(&uid)
            .map(|s| s.tty == tty && s.is_valid(current_time))
            .unwrap_or(false);

        if !nopasswd && !session_valid {
            // Need password authentication
            match password_hash {
                Some(_hash) => {
                    // Stub: in real implementation, verify against shadow
                    // database For now, accept any provided
                    // hash
                }
                None => {
                    return Err(PrivilegeError::AuthFailed);
                }
            }
            // Create/refresh session
            let session = SudoSession::new(uid, tty, current_time);
            self.sessions.insert(uid, session);
        } else if session_valid {
            // Refresh existing session
            if let Some(session) = self.sessions.get_mut(&uid) {
                session.refresh(current_time);
            }
        }

        // Build sanitized environment
        let env = self.build_sanitized_env(target_user);

        Ok(SudoExecResult {
            target_uid: 0, // caller resolves via user database
            target_gid: 0,
            command: String::from(command),
            environment: env,
        })
    }

    /// Switch user (su) - simpler than sudo, always requires auth for non-root
    pub fn su_switch(
        &self,
        caller_uid: u32,
        target_user: &str,
        password_hash: Option<&str>,
    ) -> Result<SuSwitchResult, PrivilegeError> {
        // Root can su without password
        if caller_uid != ROOT_UID {
            match password_hash {
                Some(_hash) => {
                    // Stub: verify against shadow database
                }
                None => {
                    return Err(PrivilegeError::AuthFailed);
                }
            }
        }

        let env = self.build_sanitized_env(target_user);

        Ok(SuSwitchResult {
            target_user: String::from(target_user),
            environment: env,
        })
    }

    /// Build sanitized environment for privilege elevation
    fn build_sanitized_env(&self, target_user: &str) -> Vec<(String, String)> {
        let mut env = Vec::new();
        for (key, value) in &self.env_reset {
            let val = if key == "USER" || key == "LOGNAME" {
                String::from(target_user)
            } else if key == "HOME" {
                let mut home = String::from("/home/");
                home.push_str(target_user);
                home
            } else if key == "SHELL" {
                String::from(DEFAULT_SHELL)
            } else {
                value.clone()
            };
            env.push((key.clone(), val));
        }
        env
    }

    /// Invalidate a sudo session
    pub fn invalidate_session(&mut self, uid: u32) {
        self.sessions.remove(&uid);
    }

    /// Invalidate all sessions
    pub fn invalidate_all_sessions(&mut self) {
        self.sessions.clear();
    }

    /// Get number of active sessions
    pub fn active_sessions(&self) -> usize {
        self.sessions.len()
    }

    /// PBKDF2-like stub for password hashing (integer-only)
    ///
    /// In production, use a proper PBKDF2/argon2 implementation.
    /// This is a simplified hash for development purposes.
    pub fn hash_password_stub(password: &[u8], salt: &[u8], iterations: u32) -> u64 {
        let mut hash: u64 = 0x517cc1b727220a95;
        for _ in 0..iterations {
            for &b in password {
                hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            for &b in salt {
                hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
            }
            hash ^= hash >> 33;
            hash = hash.wrapping_mul(0xff51afd7ed558ccd);
            hash ^= hash >> 33;
        }
        hash
    }
}

/// Result of a successful sudo exec
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoExecResult {
    /// Target UID
    pub target_uid: u32,
    /// Target GID
    pub target_gid: u32,
    /// Command to execute
    pub command: String,
    /// Sanitized environment
    pub environment: Vec<(String, String)>,
}

/// Result of a successful su switch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuSwitchResult {
    /// Target user
    pub target_user: String,
    /// Sanitized environment
    pub environment: Vec<(String, String)>,
}

// ============================================================================
// 6. Crontab Scheduler
// ============================================================================

/// Crontab errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronError {
    /// Invalid cron expression
    InvalidExpression,
    /// Invalid field value
    InvalidField,
    /// Value out of range
    OutOfRange,
    /// Too many entries
    TooManyEntries,
    /// Entry not found
    NotFound,
    /// User not permitted
    PermissionDenied,
    /// Parse error
    ParseError,
}

/// Maximum crontab entries per user
const MAX_CRON_ENTRIES: usize = 256;

/// Cron field specification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronField {
    /// Match any value (*)
    Any,
    /// Match a specific value
    Value(u8),
    /// Match a range (start-end inclusive)
    Range(u8, u8),
    /// Match with step (*/step or start-end/step)
    Step {
        /// Start value (0 for *)
        start: u8,
        /// End value (max for *)
        end: u8,
        /// Step interval
        step: u8,
    },
    /// Match a list of values
    List(Vec<CronFieldItem>),
}

/// A single item in a cron field list
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronFieldItem {
    /// Single value
    Value(u8),
    /// Range
    Range(u8, u8),
    /// Step
    Step { start: u8, end: u8, step: u8 },
}

impl CronField {
    /// Parse a cron field string with min/max bounds
    pub fn parse(field: &str, min: u8, max: u8) -> Result<Self, CronError> {
        // Handle list (comma-separated)
        if field.contains(',') {
            let items: Result<Vec<CronFieldItem>, CronError> = field
                .split(',')
                .map(|part| Self::parse_item(part.trim(), min, max))
                .collect();
            return Ok(CronField::List(items?));
        }

        // Handle step (*/N or start-end/N)
        if field.contains('/') {
            let parts: Vec<&str> = field.splitn(2, '/').collect();
            if parts.len() != 2 {
                return Err(CronError::InvalidField);
            }
            let step = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if step == 0 {
                return Err(CronError::InvalidField);
            }
            if parts[0] == "*" {
                return Ok(CronField::Step {
                    start: min,
                    end: max,
                    step,
                });
            }
            if parts[0].contains('-') {
                let range_parts: Vec<&str> = parts[0].splitn(2, '-').collect();
                let start = parse_u8(range_parts[0]).ok_or(CronError::InvalidField)?;
                let end = parse_u8(range_parts[1]).ok_or(CronError::InvalidField)?;
                if start > end || start < min || end > max {
                    return Err(CronError::OutOfRange);
                }
                return Ok(CronField::Step { start, end, step });
            }
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            return Ok(CronField::Step {
                start,
                end: max,
                step,
            });
        }

        // Handle wildcard
        if field == "*" {
            return Ok(CronField::Any);
        }

        // Handle range (start-end)
        if field.contains('-') {
            let parts: Vec<&str> = field.splitn(2, '-').collect();
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            let end = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if start > end || start < min || end > max {
                return Err(CronError::OutOfRange);
            }
            return Ok(CronField::Range(start, end));
        }

        // Single value
        let val = parse_u8(field).ok_or(CronError::InvalidField)?;
        if val < min || val > max {
            return Err(CronError::OutOfRange);
        }
        Ok(CronField::Value(val))
    }

    /// Parse a single item (for list parsing)
    fn parse_item(item: &str, min: u8, max: u8) -> Result<CronFieldItem, CronError> {
        if item.contains('/') {
            let parts: Vec<&str> = item.splitn(2, '/').collect();
            let step = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if parts[0].contains('-') {
                let rp: Vec<&str> = parts[0].splitn(2, '-').collect();
                let start = parse_u8(rp[0]).ok_or(CronError::InvalidField)?;
                let end = parse_u8(rp[1]).ok_or(CronError::InvalidField)?;
                if start < min || end > max {
                    return Err(CronError::OutOfRange);
                }
                return Ok(CronFieldItem::Step { start, end, step });
            }
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            return Ok(CronFieldItem::Step {
                start,
                end: max,
                step,
            });
        }
        if item.contains('-') {
            let parts: Vec<&str> = item.splitn(2, '-').collect();
            let start = parse_u8(parts[0]).ok_or(CronError::InvalidField)?;
            let end = parse_u8(parts[1]).ok_or(CronError::InvalidField)?;
            if start < min || end > max {
                return Err(CronError::OutOfRange);
            }
            return Ok(CronFieldItem::Range(start, end));
        }
        let val = parse_u8(item).ok_or(CronError::InvalidField)?;
        if val < min || val > max {
            return Err(CronError::OutOfRange);
        }
        Ok(CronFieldItem::Value(val))
    }

    /// Check if a given value matches this field
    pub fn matches(&self, value: u8) -> bool {
        match self {
            CronField::Any => true,
            CronField::Value(v) => value == *v,
            CronField::Range(start, end) => ((*start)..=(*end)).contains(&value),
            CronField::Step { start, end, step } => {
                if value < *start || value > *end {
                    return false;
                }
                let offset = value - *start;
                offset.is_multiple_of(*step)
            }
            CronField::List(items) => items.iter().any(|item| match item {
                CronFieldItem::Value(v) => value == *v,
                CronFieldItem::Range(s, e) => ((*s)..=(*e)).contains(&value),
                CronFieldItem::Step { start, end, step } => {
                    if value < *start || value > *end {
                        return false;
                    }
                    let offset = value - *start;
                    offset.is_multiple_of(*step)
                }
            }),
        }
    }
}

/// Special cron schedule strings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronSpecial {
    /// Run once at startup
    Reboot,
    /// Run once per year (0 0 1 1 *)
    Yearly,
    /// Run once per month (0 0 1 * *)
    Monthly,
    /// Run once per week (0 0 * * 0)
    Weekly,
    /// Run once per day (0 0 * * *)
    Daily,
    /// Run once per hour (0 * * * *)
    Hourly,
}

impl CronSpecial {
    /// Parse a special string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "@reboot" => Some(Self::Reboot),
            "@yearly" | "@annually" => Some(Self::Yearly),
            "@monthly" => Some(Self::Monthly),
            "@weekly" => Some(Self::Weekly),
            "@daily" | "@midnight" => Some(Self::Daily),
            "@hourly" => Some(Self::Hourly),
            _ => None,
        }
    }

    /// Convert to cron schedule fields
    pub fn to_schedule(self) -> CronSchedule {
        match self {
            Self::Reboot => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: true,
            },
            Self::Yearly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Value(1),
                month: CronField::Value(1),
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Monthly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Value(1),
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Weekly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Value(0),
                is_reboot: false,
            },
            Self::Daily => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Value(0),
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
            Self::Hourly => CronSchedule {
                minute: CronField::Value(0),
                hour: CronField::Any,
                day_of_month: CronField::Any,
                month: CronField::Any,
                day_of_week: CronField::Any,
                is_reboot: false,
            },
        }
    }
}

/// Cron schedule (5-field specification)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    /// Minute (0-59)
    pub minute: CronField,
    /// Hour (0-23)
    pub hour: CronField,
    /// Day of month (1-31)
    pub day_of_month: CronField,
    /// Month (1-12)
    pub month: CronField,
    /// Day of week (0-6, 0=Sunday)
    pub day_of_week: CronField,
    /// Is this a @reboot schedule
    pub is_reboot: bool,
}

impl CronSchedule {
    /// Parse a 5-field cron expression or special string
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let trimmed = expr.trim();

        // Check for special strings
        if trimmed.starts_with('@') {
            return CronSpecial::from_str(trimmed)
                .map(|s| s.to_schedule())
                .ok_or(CronError::InvalidExpression);
        }

        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 5 {
            return Err(CronError::InvalidExpression);
        }

        Ok(Self {
            minute: CronField::parse(fields[0], 0, 59)?,
            hour: CronField::parse(fields[1], 0, 23)?,
            day_of_month: CronField::parse(fields[2], 1, 31)?,
            month: CronField::parse(fields[3], 1, 12)?,
            day_of_week: CronField::parse(fields[4], 0, 6)?,
            is_reboot: false,
        })
    }

    /// Check if this schedule matches a given date/time
    ///
    /// Parameters are plain integers (no floating point):
    /// - minute: 0-59
    /// - hour: 0-23
    /// - day: 1-31
    /// - month: 1-12
    /// - dow: 0-6 (0=Sunday)
    pub fn matches(&self, minute: u8, hour: u8, day: u8, month: u8, dow: u8) -> bool {
        if self.is_reboot {
            return false; // @reboot only runs at boot
        }
        self.minute.matches(minute)
            && self.hour.matches(hour)
            && self.day_of_month.matches(day)
            && self.month.matches(month)
            && self.day_of_week.matches(dow)
    }

    /// Calculate the next matching time from the given start
    ///
    /// Returns (minute, hour, day, month, year) or None if not found
    /// within a reasonable search window (1 year).
    ///
    /// All arithmetic is integer-only.
    pub fn next_run(
        &self,
        start_minute: u8,
        start_hour: u8,
        start_day: u8,
        start_month: u8,
        start_year: u16,
    ) -> Option<(u8, u8, u8, u8, u16)> {
        if self.is_reboot {
            return None;
        }

        let mut minute = start_minute;
        let mut hour = start_hour;
        let mut day = start_day;
        let mut month = start_month;
        let mut year = start_year;

        // Advance minute by 1 to avoid matching current time
        minute += 1;
        if minute > 59 {
            minute = 0;
            hour += 1;
        }
        if hour > 23 {
            hour = 0;
            day += 1;
        }

        // Search up to ~366 days * 24 hours * 60 minutes = 527040 iterations max
        // But we optimize by skipping non-matching months/days
        let max_iterations = 527_040u32;
        let mut iterations = 0u32;

        loop {
            if iterations >= max_iterations {
                return None;
            }
            iterations += 1;

            // Fix day overflow
            let max_day = days_in_month(month, year);
            if day > max_day {
                day = 1;
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                minute = 0;
                hour = 0;
                continue;
            }

            // Check month
            if !self.month.matches(month) {
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                day = 1;
                hour = 0;
                minute = 0;
                continue;
            }

            // Check day of month and day of week
            let dow = day_of_week(year, month, day);
            if !self.day_of_month.matches(day) || !self.day_of_week.matches(dow) {
                day += 1;
                hour = 0;
                minute = 0;
                continue;
            }

            // Check hour
            if !self.hour.matches(hour) {
                hour += 1;
                if hour > 23 {
                    hour = 0;
                    day += 1;
                }
                minute = 0;
                continue;
            }

            // Check minute
            if !self.minute.matches(minute) {
                minute += 1;
                if minute > 59 {
                    minute = 0;
                    hour += 1;
                    if hour > 23 {
                        hour = 0;
                        day += 1;
                    }
                }
                continue;
            }

            return Some((minute, hour, day, month, year));
        }
    }
}

/// Cron job entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEntry {
    /// Unique job ID
    pub id: u64,
    /// Schedule specification
    pub schedule: CronSchedule,
    /// Command to execute
    pub command: String,
    /// Owner username
    pub owner: String,
    /// Whether this job is enabled
    pub enabled: bool,
    /// Last run timestamp (epoch seconds)
    pub last_run: u64,
    /// Run count
    pub run_count: u64,
    /// Last exit status
    pub last_exit_status: i32,
}

impl CronEntry {
    /// Create a new cron entry
    pub fn new(id: u64, schedule: CronSchedule, command: &str, owner: &str) -> Self {
        Self {
            id,
            schedule,
            command: String::from(command),
            owner: String::from(owner),
            enabled: true,
            last_run: 0,
            run_count: 0,
            last_exit_status: 0,
        }
    }

    /// Parse a crontab line (schedule + command)
    pub fn parse(id: u64, line: &str, owner: &str) -> Result<Self, CronError> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Err(CronError::ParseError);
        }

        // Check for special strings
        if trimmed.starts_with('@') {
            let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
            if parts.len() < 2 {
                return Err(CronError::InvalidExpression);
            }
            let schedule = CronSchedule::parse(parts[0])?;
            return Ok(Self::new(id, schedule, parts[1].trim(), owner));
        }

        // Standard 5-field format: min hour dom mon dow command
        let fields: Vec<&str> = trimmed.splitn(6, char::is_whitespace).collect();
        if fields.len() < 6 {
            return Err(CronError::InvalidExpression);
        }

        let schedule_str = &[fields[0], fields[1], fields[2], fields[3], fields[4]].join(" ");
        let schedule = CronSchedule::parse(schedule_str)?;
        let command = fields[5].trim();

        Ok(Self::new(id, schedule, command, owner))
    }

    /// Serialize to crontab format
    pub fn to_crontab_line(&self) -> String {
        if self.schedule.is_reboot {
            let mut line = String::from("@reboot ");
            line.push_str(&self.command);
            return line;
        }
        let mut line = String::new();
        line.push_str(&format_cron_field(&self.schedule.minute));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.hour));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.day_of_month));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.month));
        line.push(' ');
        line.push_str(&format_cron_field(&self.schedule.day_of_week));
        line.push(' ');
        line.push_str(&self.command);
        line
    }
}

/// Per-user crontab
#[derive(Debug)]
pub struct CronTab {
    /// Owner username
    pub owner: String,
    /// Job entries
    pub entries: Vec<CronEntry>,
    /// Next entry ID
    next_id: u64,
}

impl CronTab {
    /// Create a new crontab for a user
    pub fn new(owner: &str) -> Self {
        Self {
            owner: String::from(owner),
            entries: Vec::new(),
            next_id: 1,
        }
    }

    /// Add an entry from a crontab line
    pub fn add_line(&mut self, line: &str) -> Result<u64, CronError> {
        if self.entries.len() >= MAX_CRON_ENTRIES {
            return Err(CronError::TooManyEntries);
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = CronEntry::parse(id, line, &self.owner)?;
        self.entries.push(entry);
        Ok(id)
    }

    /// Add an entry with a parsed schedule
    pub fn add_entry(&mut self, schedule: CronSchedule, command: &str) -> Result<u64, CronError> {
        if self.entries.len() >= MAX_CRON_ENTRIES {
            return Err(CronError::TooManyEntries);
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = CronEntry::new(id, schedule, command, &self.owner);
        self.entries.push(entry);
        Ok(id)
    }

    /// Remove an entry by ID
    pub fn remove_entry(&mut self, id: u64) -> Result<(), CronError> {
        let initial = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == initial {
            return Err(CronError::NotFound);
        }
        Ok(())
    }

    /// Enable/disable an entry
    pub fn set_enabled(&mut self, id: u64, enabled: bool) -> Result<(), CronError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(CronError::NotFound)?;
        entry.enabled = enabled;
        Ok(())
    }

    /// Get entries that match the current time
    pub fn get_due_entries(
        &self,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        dow: u8,
    ) -> Vec<&CronEntry> {
        self.entries
            .iter()
            .filter(|e| e.enabled && e.schedule.matches(minute, hour, day, month, dow))
            .collect()
    }

    /// Parse a complete crontab file
    pub fn load(&mut self, content: &str) -> Result<usize, CronError> {
        let mut count = 0usize;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Skip environment variable assignments
            if trimmed.contains('=') && !trimmed.starts_with('*') && !trimmed.starts_with('@') {
                // Could be VAR=value
                let first_char = trimmed.as_bytes()[0];
                if first_char.is_ascii_alphabetic() {
                    continue;
                }
            }
            self.add_line(trimmed)?;
            count += 1;
        }
        Ok(count)
    }

    /// Serialize to crontab file format
    pub fn to_crontab_file(&self) -> String {
        let mut output = String::new();
        output.push_str("# Crontab for ");
        output.push_str(&self.owner);
        output.push('\n');
        for entry in &self.entries {
            if !entry.enabled {
                output.push_str("# ");
            }
            output.push_str(&entry.to_crontab_line());
            output.push('\n');
        }
        output
    }

    /// Number of entries
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Cron job execution record
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobExecution {
    /// Job ID
    pub job_id: u64,
    /// Owner
    pub owner: String,
    /// Command
    pub command: String,
    /// Scheduled time (epoch seconds)
    pub scheduled_time: u64,
}

/// Cron daemon
#[derive(Debug)]
pub struct CronDaemon {
    /// Per-user crontabs
    pub crontabs: BTreeMap<String, CronTab>,
    /// System crontab (/etc/crontab)
    pub system_crontab: CronTab,
    /// Pending execution queue
    pub execution_queue: Vec<CronJobExecution>,
    /// Total jobs executed
    pub total_executed: u64,
    /// Last tick time (for deduplication)
    pub last_tick_minute: u8,
    /// Whether the daemon is running
    pub running: bool,
    /// Whether reboot jobs have been fired
    pub reboot_fired: bool,
}

impl Default for CronDaemon {
    fn default() -> Self {
        Self::new()
    }
}

impl CronDaemon {
    /// Create a new cron daemon
    pub fn new() -> Self {
        Self {
            crontabs: BTreeMap::new(),
            system_crontab: CronTab::new("root"),
            execution_queue: Vec::new(),
            total_executed: 0,
            last_tick_minute: 255, // invalid, forces first tick
            running: false,
            reboot_fired: false,
        }
    }

    /// Start the daemon
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Stop the daemon
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Get or create a user's crontab
    pub fn get_or_create_crontab(&mut self, username: &str) -> &mut CronTab {
        if !self.crontabs.contains_key(username) {
            self.crontabs
                .insert(String::from(username), CronTab::new(username));
        }
        self.crontabs.get_mut(username).unwrap()
    }

    /// Remove a user's crontab
    pub fn remove_crontab(&mut self, username: &str) -> bool {
        self.crontabs.remove(username).is_some()
    }

    /// Fire @reboot jobs (called once at boot)
    pub fn fire_reboot_jobs(&mut self, current_time: u64) {
        if self.reboot_fired {
            return;
        }
        self.reboot_fired = true;

        // System crontab
        for entry in &self.system_crontab.entries {
            if entry.enabled && entry.schedule.is_reboot {
                self.execution_queue.push(CronJobExecution {
                    job_id: entry.id,
                    owner: entry.owner.clone(),
                    command: entry.command.clone(),
                    scheduled_time: current_time,
                });
            }
        }

        // User crontabs
        for crontab in self.crontabs.values() {
            for entry in &crontab.entries {
                if entry.enabled && entry.schedule.is_reboot {
                    self.execution_queue.push(CronJobExecution {
                        job_id: entry.id,
                        owner: entry.owner.clone(),
                        command: entry.command.clone(),
                        scheduled_time: current_time,
                    });
                }
            }
        }
    }

    /// Tick the daemon with current time components
    ///
    /// Should be called once per minute. Enqueues any matching jobs.
    pub fn tick(
        &mut self,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        dow: u8,
        current_time: u64,
    ) -> usize {
        if !self.running {
            return 0;
        }

        // Deduplicate: only process each minute once
        if minute == self.last_tick_minute {
            return 0;
        }
        self.last_tick_minute = minute;

        let mut queued = 0usize;

        // Check system crontab
        let due_system: Vec<(u64, String, String)> = self
            .system_crontab
            .get_due_entries(minute, hour, day, month, dow)
            .iter()
            .map(|e| (e.id, e.owner.clone(), e.command.clone()))
            .collect();

        for (id, owner, cmd) in due_system {
            self.execution_queue.push(CronJobExecution {
                job_id: id,
                owner,
                command: cmd,
                scheduled_time: current_time,
            });
            queued += 1;
        }

        // Check user crontabs
        let user_due: Vec<(u64, String, String)> = self
            .crontabs
            .values()
            .flat_map(|tab| {
                tab.get_due_entries(minute, hour, day, month, dow)
                    .into_iter()
                    .map(|e| (e.id, e.owner.clone(), e.command.clone()))
            })
            .collect();

        for (id, owner, cmd) in user_due {
            self.execution_queue.push(CronJobExecution {
                job_id: id,
                owner,
                command: cmd,
                scheduled_time: current_time,
            });
            queued += 1;
        }

        self.total_executed += queued as u64;
        queued
    }

    /// Drain the execution queue
    pub fn drain_queue(&mut self) -> Vec<CronJobExecution> {
        core::mem::take(&mut self.execution_queue)
    }

    /// Get the total number of crontab entries across all users
    pub fn total_entries(&self) -> usize {
        let user_entries: usize = self.crontabs.values().map(|t| t.entry_count()).sum();
        user_entries + self.system_crontab.entry_count()
    }

    /// Check if the daemon is running
    pub fn is_running(&self) -> bool {
        self.running
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse a u8 from a string (no_std compatible)
fn parse_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u16 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u16)?;
        if result > 255 {
            return None;
        }
    }
    Some(result as u8)
}

/// Parse a u32 from a string (no_std compatible)
fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        if result > u32::MAX as u64 {
            return None;
        }
    }
    Some(result as u32)
}

/// Parse a u64 from a string (no_std compatible)
fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(result)
}

/// Push a u32 as decimal string to a String (no_std compatible)
fn push_u32_str(s: &mut String, val: u32) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[pos..] {
        s.push(b as char);
    }
}

/// Push a u64 as decimal string to a String (no_std compatible)
fn push_u64_str(s: &mut String, val: u64) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[pos..] {
        s.push(b as char);
    }
}

/// Days in a given month (integer arithmetic, no floating point)
fn days_in_month(month: u8, year: u16) -> u8 {
    match month {
        1 => 31,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        3 => 31,
        4 => 30,
        5 => 31,
        6 => 30,
        7 => 31,
        8 => 31,
        9 => 30,
        10 => 31,
        11 => 30,
        12 => 31,
        _ => 30,
    }
}

/// Check if a year is a leap year (integer-only)
fn is_leap_year(year: u16) -> bool {
    if year.is_multiple_of(400) {
        true
    } else if year.is_multiple_of(100) {
        false
    } else {
        year.is_multiple_of(4)
    }
}

/// Calculate day of week (0=Sunday) using Tomohiko Sakamoto's algorithm
/// (integer-only)
fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if month < 3 {
        y -= 1;
    }
    let m = month as u16;
    let d = day as u16;
    ((y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + d) % 7) as u8
}

/// Format a cron field as a string
fn format_cron_field(field: &CronField) -> String {
    match field {
        CronField::Any => String::from("*"),
        CronField::Value(v) => {
            let mut s = String::new();
            push_u32_str(&mut s, *v as u32);
            s
        }
        CronField::Range(start, end) => {
            let mut s = String::new();
            push_u32_str(&mut s, *start as u32);
            s.push('-');
            push_u32_str(&mut s, *end as u32);
            s
        }
        CronField::Step { start, end, step } => {
            let mut s = String::new();
            if *start == 0 {
                s.push('*');
            } else {
                push_u32_str(&mut s, *start as u32);
                s.push('-');
                push_u32_str(&mut s, *end as u32);
            }
            s.push('/');
            push_u32_str(&mut s, *step as u32);
            s
        }
        CronField::List(items) => {
            let mut s = String::new();
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                match item {
                    CronFieldItem::Value(v) => push_u32_str(&mut s, *v as u32),
                    CronFieldItem::Range(start, end) => {
                        push_u32_str(&mut s, *start as u32);
                        s.push('-');
                        push_u32_str(&mut s, *end as u32);
                    }
                    CronFieldItem::Step { start, end, step } => {
                        push_u32_str(&mut s, *start as u32);
                        s.push('-');
                        push_u32_str(&mut s, *end as u32);
                        s.push('/');
                        push_u32_str(&mut s, *step as u32);
                    }
                }
            }
            s
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- io_uring tests ---

    #[test]
    fn test_ring_buffer_basic() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(4).unwrap();
        assert!(rb.is_empty());
        assert_eq!(rb.capacity(), 4);
        assert_eq!(rb.available(), 4);
        rb.push(1).unwrap();
        rb.push(2).unwrap();
        assert_eq!(rb.len(), 2);
        assert_eq!(rb.pop(), Some(1));
        assert_eq!(rb.pop(), Some(2));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_ring_buffer_full() {
        let mut rb: RingBuffer<u32> = RingBuffer::new(2).unwrap();
        rb.push(10).unwrap();
        rb.push(20).unwrap();
        assert!(rb.is_full());
        assert!(matches!(
            rb.push(30),
            Err(IoUringError::SubmissionQueueFull)
        ));
    }

    #[test]
    fn test_ring_buffer_invalid_capacity() {
        let result: Result<RingBuffer<u32>, _> = RingBuffer::new(3);
        assert!(matches!(result, Err(IoUringError::InvalidEntries)));
        let result2: Result<RingBuffer<u32>, _> = RingBuffer::new(0);
        assert!(matches!(result2, Err(IoUringError::InvalidEntries)));
    }

    #[test]
    fn test_io_uring_create_and_submit() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();
        assert_eq!(ring.state(), IoUringState::Idle);

        let sqe = SqEntry::nop(42);
        ring.submit(sqe).unwrap();
        assert_eq!(ring.sq_pending(), 1);
        assert_eq!(ring.state(), IoUringState::Submitting);
    }

    #[test]
    fn test_io_uring_process_and_reap() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();

        ring.submit(SqEntry::nop(1)).unwrap();
        ring.submit(SqEntry::readv(5, 0x1000, 4, 0, 2)).unwrap();

        let processed = ring.process_submissions();
        assert_eq!(processed, 2);
        assert_eq!(ring.cq_ready(), 2);

        let cqe1 = ring.reap_completion().unwrap();
        assert_eq!(cqe1.user_data, 1);
        assert_eq!(cqe1.result, 0); // NOP returns 0

        let cqe2 = ring.reap_completion().unwrap();
        assert_eq!(cqe2.user_data, 2);
        assert_eq!(cqe2.result, 4); // READV returns len
    }

    #[test]
    fn test_io_uring_batch_submit() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();

        let sqes = vec![SqEntry::nop(1), SqEntry::nop(2), SqEntry::nop(3)];
        let submitted = ring.submit_batch(&sqes).unwrap();
        assert_eq!(submitted, 3);
        assert_eq!(ring.total_submissions(), 3);
    }

    #[test]
    fn test_io_uring_shutdown() {
        let params = IoUringParams::default();
        let mut ring = IoUring::new(1, params, 100).unwrap();
        ring.submit(SqEntry::nop(1)).unwrap();
        ring.shutdown();
        assert_eq!(ring.state(), IoUringState::Shutdown);
        assert!(matches!(
            ring.submit(SqEntry::nop(2)),
            Err(IoUringError::Shutdown)
        ));
    }

    #[test]
    fn test_io_uring_manager() {
        let mut mgr = IoUringManager::new();
        let id = mgr.setup(IoUringParams::default(), 100).unwrap();
        assert_eq!(mgr.active_rings(), 1);
        assert!(mgr.get_ring(id).is_some());
        mgr.destroy(id).unwrap();
        assert_eq!(mgr.active_rings(), 0);
    }

    #[test]
    fn test_io_uring_register_files() {
        let mut ring = IoUring::new(1, IoUringParams::default(), 100).unwrap();
        ring.register_files(&[1, 2, 3]).unwrap();
        ring.unregister_files();
    }

    #[test]
    fn test_sqe_constructors() {
        let fsync = SqEntry::fsync(10, true, 99);
        assert_eq!(fsync.opcode, IoUringOpcode::Fsync as u8);
        assert_eq!(fsync.fd, 10);
        assert_eq!(fsync.op_flags, 1);
        assert_eq!(fsync.user_data, 99);

        let poll = SqEntry::poll_add(5, 0x01, 50);
        assert_eq!(poll.opcode, IoUringOpcode::PollAdd as u8);
    }

    // --- ptrace tests ---

    #[test]
    fn test_ptrace_attach_detach() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(mgr.is_traced(2));
        assert_eq!(mgr.get_tracer(2), Some(1));
        mgr.detach(1, 2).unwrap();
        assert!(!mgr.is_traced(2));
    }

    #[test]
    fn test_ptrace_cannot_trace_self() {
        let mut mgr = PtraceManager::new();
        assert!(matches!(
            mgr.attach(1, 1),
            Err(PtraceError::PermissionDenied)
        ));
    }

    #[test]
    fn test_ptrace_double_attach() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(matches!(mgr.attach(3, 2), Err(PtraceError::AlreadyTraced)));
    }

    #[test]
    fn test_ptrace_cont_and_state() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Stopped(19))
        ));
        mgr.cont(1, 2, 0).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Running)
        ));
    }

    #[test]
    fn test_ptrace_single_step() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.single_step(1, 2).unwrap();
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::SingleStep)
        ));
    }

    #[test]
    fn test_ptrace_get_set_regs() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        let mut regs = mgr.get_regs(1, 2).unwrap();
        regs.rip = 0x4000;
        regs.rax = 42;
        mgr.set_regs(1, 2, regs).unwrap();
        let updated = mgr.get_regs(1, 2).unwrap();
        assert_eq!(updated.rip, 0x4000);
        assert_eq!(updated.rax, 42);
    }

    #[test]
    fn test_ptrace_peek_poke() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.poke_data(1, 2, 0x1000, 0xDEAD).unwrap();
        let val = mgr.peek_data(1, 2, 0x1000).unwrap();
        assert_eq!(val, 0xDEAD);
    }

    #[test]
    fn test_ptrace_on_signal() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.cont(1, 2, 0).unwrap();
        mgr.on_signal(2, 11, 0xBAD); // SIGSEGV
        assert!(matches!(
            mgr.get_tracee_state(2),
            Some(TraceeState::Stopped(11))
        ));
        let info = mgr.get_sig_info(1, 2).unwrap();
        assert_eq!(info.signo, 11);
        assert_eq!(info.fault_addr, 0xBAD);
    }

    #[test]
    fn test_ptrace_tracer_exit_cleanup() {
        let mut mgr = PtraceManager::new();
        mgr.attach(1, 2).unwrap();
        mgr.attach(1, 3).unwrap();
        assert_eq!(mgr.active_traces(), 2);
        mgr.on_tracer_exit(1);
        assert_eq!(mgr.active_traces(), 0);
    }

    // --- Core Dump tests ---

    #[test]
    fn test_core_dump_basic() {
        let mut writer = CoreDumpWriter::new();
        writer.prstatus.pid = 42;
        writer.prstatus.signal = 11;
        writer.prpsinfo.set_fname("test_prog");
        writer.add_segment(0x400000, 5, vec![0xCC; 64]); // R+X
        writer.add_segment(0x600000, 6, vec![0; 128]); // R+W

        let dump = writer.write_core_dump().unwrap();
        // Check ELF magic
        assert_eq!(&dump[0..4], &[0x7F, b'E', b'L', b'F']);
        // Check it's 64-bit
        assert_eq!(dump[4], ELFCLASS64);
        // Check it's a core file
        assert_eq!(dump[16], ET_CORE as u8);
        assert_eq!(dump[17], 0);
        assert!(dump.len() > ELF64_EHDR_SIZE);
    }

    #[test]
    fn test_core_dump_empty_segments() {
        let mut writer = CoreDumpWriter::new();
        writer.prstatus.pid = 1;
        let dump = writer.write_core_dump().unwrap();
        assert_eq!(&dump[0..4], &[0x7F, b'E', b'L', b'F']);
    }

    #[test]
    fn test_prpsinfo_fname() {
        let mut info = PrPsInfo::default();
        info.set_fname("hello_world");
        assert_eq!(&info.fname[..11], b"hello_world");
        assert_eq!(info.fname[11], 0);
    }

    // --- User/Group tests ---

    #[test]
    fn test_user_database_creation() {
        let db = UserDatabase::new();
        assert_eq!(db.user_count(), 1); // root
        let root = db.get_user_by_uid(0).unwrap();
        assert_eq!(root.username, "root");
    }

    #[test]
    fn test_user_add_remove() {
        let mut db = UserDatabase::new();
        let uid = db.add_user("alice", 1000, None).unwrap();
        assert_eq!(uid, 1000);
        assert_eq!(db.user_count(), 2);
        assert!(db.get_user_by_name("alice").is_some());
        db.remove_user("alice").unwrap();
        assert_eq!(db.user_count(), 1);
    }

    #[test]
    fn test_user_duplicate() {
        let mut db = UserDatabase::new();
        db.add_user("bob", 1000, None).unwrap();
        assert!(matches!(
            db.add_user("bob", 1000, None),
            Err(UserGroupError::UserExists)
        ));
    }

    #[test]
    fn test_passwd_serialization() {
        let user = UserEntry::new("alice", 1000, 1000);
        let line = user.to_passwd_line();
        assert!(line.contains("alice:x:1000:1000:"));
        let parsed = UserEntry::from_passwd_line(&line).unwrap();
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.uid, 1000);
    }

    #[test]
    fn test_shadow_entry() {
        let shadow = ShadowEntry::new_locked("alice");
        assert!(shadow.is_locked());
        let shadow2 = ShadowEntry::with_password("alice", "$6$salt$hash");
        assert!(!shadow2.is_locked());
    }

    #[test]
    fn test_group_database() {
        let mut db = GroupDatabase::new();
        let gid = db.add_group("developers", None).unwrap();
        assert_eq!(gid, 1000);
        db.add_user_to_group("alice", "developers").unwrap();
        let groups = db.get_user_groups("alice");
        assert_eq!(groups, vec![1000]);
    }

    #[test]
    fn test_group_serialization() {
        let mut group = GroupEntry::new("staff", 100);
        group.add_member("alice");
        group.add_member("bob");
        let line = group.to_group_line();
        assert!(line.contains("staff:x:100:alice,bob"));
        let parsed = GroupEntry::from_group_line(&line).unwrap();
        assert_eq!(parsed.members.len(), 2);
    }

    #[test]
    fn test_username_validation() {
        let mut db = UserDatabase::new();
        assert!(matches!(
            db.add_user("", 1000, None),
            Err(UserGroupError::InvalidUsername)
        ));
        assert!(matches!(
            db.add_user("1bad", 1000, None),
            Err(UserGroupError::InvalidUsername)
        ));
        assert!(db.add_user("_valid-name.1", 1000, None).is_ok());
    }

    // --- sudo/su tests ---

    #[test]
    fn test_sudoers_rule_match() {
        let rule = SudoersRule::new("alice", "ALL", "ALL", "ALL");
        assert!(rule.matches_user("alice", &[]));
        assert!(!rule.matches_user("bob", &[]));
        assert!(rule.matches_runas("root"));
        assert!(rule.matches_command("/bin/ls"));
    }

    #[test]
    fn test_sudoers_group_match() {
        let rule = SudoersRule::new("%wheel", "ALL", "ALL", "ALL");
        let groups = vec![String::from("wheel")];
        assert!(rule.matches_user("anyone", &groups));
        assert!(!rule.matches_user("anyone", &[]));
    }

    #[test]
    fn test_privilege_manager_check() {
        let mut mgr = PrivilegeManager::new();
        mgr.sudoers
            .add_rule(SudoersRule::new("alice", "ALL", "ALL", "ALL"));
        let result = mgr
            .check_sudo_permission("alice", &[], "root", "/bin/ls")
            .unwrap();
        assert!(result);
        let result2 = mgr
            .check_sudo_permission("bob", &[], "root", "/bin/ls")
            .unwrap();
        assert!(!result2);
    }

    #[test]
    fn test_sudo_nopasswd() {
        let mut mgr = PrivilegeManager::new();
        mgr.sudoers
            .add_rule(SudoersRule::new_nopasswd("alice", "ALL", "ALL", "ALL"));
        assert!(mgr.is_nopasswd("alice", &[], "root", "/bin/ls"));
    }

    #[test]
    fn test_sudo_session_timeout() {
        let session = SudoSession::new(1000, 1, 100);
        assert!(session.is_valid(200)); // within 300s
        assert!(session.is_valid(399)); // edge
        assert!(!session.is_valid(400)); // expired
    }

    #[test]
    fn test_su_switch_root_no_password() {
        let mgr = PrivilegeManager::new();
        let result = mgr.su_switch(0, "alice", None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_su_switch_non_root_needs_password() {
        let mgr = PrivilegeManager::new();
        let result = mgr.su_switch(1000, "root", None);
        assert!(matches!(result, Err(PrivilegeError::AuthFailed)));
    }

    #[test]
    fn test_password_hash_stub() {
        let hash1 = PrivilegeManager::hash_password_stub(b"password", b"salt", 10);
        let hash2 = PrivilegeManager::hash_password_stub(b"password", b"salt", 10);
        assert_eq!(hash1, hash2); // deterministic
        let hash3 = PrivilegeManager::hash_password_stub(b"different", b"salt", 10);
        assert_ne!(hash1, hash3);
    }

    // --- Crontab tests ---

    #[test]
    fn test_cron_field_any() {
        let field = CronField::parse("*", 0, 59).unwrap();
        assert!(matches!(field, CronField::Any));
        assert!(field.matches(0));
        assert!(field.matches(59));
    }

    #[test]
    fn test_cron_field_value() {
        let field = CronField::parse("15", 0, 59).unwrap();
        assert!(field.matches(15));
        assert!(!field.matches(16));
    }

    #[test]
    fn test_cron_field_range() {
        let field = CronField::parse("10-20", 0, 59).unwrap();
        assert!(field.matches(10));
        assert!(field.matches(15));
        assert!(field.matches(20));
        assert!(!field.matches(9));
        assert!(!field.matches(21));
    }

    #[test]
    fn test_cron_field_step() {
        let field = CronField::parse("*/15", 0, 59).unwrap();
        assert!(field.matches(0));
        assert!(field.matches(15));
        assert!(field.matches(30));
        assert!(field.matches(45));
        assert!(!field.matches(10));
    }

    #[test]
    fn test_cron_field_list() {
        let field = CronField::parse("1,5,10", 0, 59).unwrap();
        assert!(field.matches(1));
        assert!(field.matches(5));
        assert!(field.matches(10));
        assert!(!field.matches(2));
    }

    #[test]
    fn test_cron_schedule_parse() {
        let sched = CronSchedule::parse("30 2 * * *").unwrap();
        assert!(sched.matches(30, 2, 15, 6, 3));
        assert!(!sched.matches(0, 2, 15, 6, 3));
        assert!(!sched.matches(30, 3, 15, 6, 3));
    }

    #[test]
    fn test_cron_special_strings() {
        let daily = CronSchedule::parse("@daily").unwrap();
        assert!(daily.matches(0, 0, 1, 1, 0));
        assert!(!daily.matches(1, 0, 1, 1, 0));

        let reboot = CronSchedule::parse("@reboot").unwrap();
        assert!(reboot.is_reboot);
        assert!(!reboot.matches(0, 0, 1, 1, 0));
    }

    #[test]
    fn test_cron_entry_parse() {
        let entry = CronEntry::parse(1, "0 3 * * 1 /usr/bin/backup", "root").unwrap();
        assert_eq!(entry.command, "/usr/bin/backup");
        assert_eq!(entry.owner, "root");
    }

    #[test]
    fn test_crontab_add_remove() {
        let mut tab = CronTab::new("alice");
        let id = tab.add_line("0 * * * * /bin/echo hello").unwrap();
        assert_eq!(tab.entry_count(), 1);
        tab.remove_entry(id).unwrap();
        assert_eq!(tab.entry_count(), 0);
    }

    #[test]
    fn test_cron_daemon_tick() {
        let mut daemon = CronDaemon::new();
        daemon.start();
        let tab = daemon.get_or_create_crontab("alice");
        tab.add_line("30 * * * * /bin/echo tick").unwrap();

        let queued = daemon.tick(30, 12, 15, 6, 3, 1000);
        assert_eq!(queued, 1);

        let jobs = daemon.drain_queue();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].command, "/bin/echo tick");
    }

    #[test]
    fn test_cron_daemon_reboot_jobs() {
        let mut daemon = CronDaemon::new();
        daemon.start();
        let tab = daemon.get_or_create_crontab("root");
        tab.add_line("@reboot /etc/init.d/startup").unwrap();

        daemon.fire_reboot_jobs(0);
        let jobs = daemon.drain_queue();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].command, "/etc/init.d/startup");

        // Should not fire again
        daemon.fire_reboot_jobs(100);
        assert!(daemon.drain_queue().is_empty());
    }

    #[test]
    fn test_cron_next_run() {
        let sched = CronSchedule::parse("0 12 * * *").unwrap();
        // From 10:30, next should be 12:00 same day
        let next = sched.next_run(30, 10, 15, 6, 2026);
        assert!(next.is_some());
        let (min, hour, _day, _month, _year) = next.unwrap();
        assert_eq!(min, 0);
        assert_eq!(hour, 12);
    }

    #[test]
    fn test_day_of_week_calculation() {
        // 2026-03-05 is a Thursday (4)
        assert_eq!(day_of_week(2026, 3, 5), 4);
        // 2026-01-01 is a Thursday (4)
        assert_eq!(day_of_week(2026, 1, 1), 4);
    }

    #[test]
    fn test_days_in_month() {
        assert_eq!(days_in_month(2, 2024), 29); // leap year
        assert_eq!(days_in_month(2, 2025), 28); // not leap
        assert_eq!(days_in_month(1, 2025), 31);
        assert_eq!(days_in_month(4, 2025), 30);
    }

    #[test]
    fn test_helper_parse_functions() {
        assert_eq!(parse_u8("0"), Some(0));
        assert_eq!(parse_u8("255"), Some(255));
        assert_eq!(parse_u8("256"), None);
        assert_eq!(parse_u32("4294967295"), Some(u32::MAX));
        assert_eq!(parse_u64("0"), Some(0));
        assert_eq!(parse_u64("abc"), None);
    }
}
