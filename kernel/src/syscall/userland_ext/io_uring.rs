//! io_uring - Asynchronous I/O Interface
//!
//! Implements the io_uring submission/completion ring interface for
//! high-performance async I/O.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Types and Constants
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

// ============================================================================
// Ring Buffer
// ============================================================================

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

// ============================================================================
// IoUring Instance
// ============================================================================

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

// ============================================================================
// IoUring Manager
// ============================================================================

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
