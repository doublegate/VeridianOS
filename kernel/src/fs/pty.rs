//! Pseudo-Terminal (PTY) Support
//!
//! Provides pseudo-terminal devices for terminal emulation and shell
//! interaction.
//!
//! # Architecture
//!
//! A PTY pair consists of a master and a slave side. The master is used by a
//! terminal emulator (or the graphical desktop renderer); the slave is used by
//! the shell or application running inside the terminal.
//!
//! Data flow:
//! - Master writes to `input_buffer`  → slave reads from `input_buffer`
//! - Slave  writes to `output_buffer` → master reads from `output_buffer`
//!
//! The [`PtyMasterNode`] and [`PtySlaveNode`] VfsNode wrappers expose these
//! buffers as regular file descriptors so that standard `read`/`write`
//! syscalls work transparently.

// Allow dead code for PTY fields not yet used in current implementation
#![allow(dead_code, clippy::needless_range_loop)]

#[allow(unused_imports)]
use alloc::{collections::VecDeque, format, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use spin::RwLock;

use crate::{error::KernelError, process::ProcessId, sync::once_lock::GlobalState};

/// PTY buffer size
const PTY_BUFFER_SIZE: usize = 4096;

const POLLIN: u16 = 0x0001;
const POLLOUT: u16 = 0x0004;
const POLLERR: u16 = 0x0008;
const POLLHUP: u16 = 0x0010;

/// Terminal control characters
pub mod termios {
    pub const VINTR: u8 = 0; // ^C
    pub const VQUIT: u8 = 1; // ^\
    pub const VERASE: u8 = 2; // Backspace
    pub const VKILL: u8 = 3; // ^U
    pub const VEOF: u8 = 4; // ^D
    pub const VEOL: u8 = 5; // End of line
    pub const VSUSP: u8 = 6; // ^Z
    pub const VINTR_CHAR: u8 = 3; // ASCII ^C
    pub const VEOF_CHAR: u8 = 4; // ASCII ^D
    pub const VSUSP_CHAR: u8 = 26; // ASCII ^Z (0x1A)
}

/// PTY Terminal mode flags
#[derive(Debug, Clone, Copy)]
pub struct TermiosFlags {
    /// Echo input characters
    pub echo: bool,

    /// Canonical mode (line buffering)
    pub canonical: bool,

    /// Enable signals
    pub isig: bool,

    /// Process output (convert \n to \r\n)
    pub opost: bool,
}

impl Default for TermiosFlags {
    fn default() -> Self {
        Self {
            echo: true,
            canonical: true,
            isig: true,
            opost: true,
        }
    }
}

/// Window size information
#[derive(Debug, Clone, Copy)]
pub struct Winsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl Default for Winsize {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 80,
            xpixel: 0,
            ypixel: 0,
        }
    }
}

/// PTY Master side (controlled by terminal emulator)
pub struct PtyMaster {
    /// PTY ID
    id: u32,

    /// Input buffer (from master to slave)
    input_buffer: RwLock<VecDeque<u8>>,

    /// Output buffer (from slave to master)
    output_buffer: RwLock<VecDeque<u8>>,

    /// Window size
    winsize: RwLock<Winsize>,

    /// Terminal flags
    flags: RwLock<TermiosFlags>,

    /// Process ID of controlling process
    controller: RwLock<Option<ProcessId>>,

    /// Foreground process group ID for this terminal.
    /// Processes in this group receive keyboard-generated signals (SIGINT,
    /// SIGTSTP). Background processes accessing this terminal receive
    /// SIGTTIN/SIGTTOU.
    foreground_pgid: AtomicU64,

    /// Open slave nodes, and whether there ever was one: once the last
    /// slave closes, the master reads EIO when drained and polls POLLHUP,
    /// as on Linux (PR #24 review: it waited forever).
    slaves: AtomicUsize,
    had_slave: AtomicBool,
}

/// EIO, what a terminal whose other side is gone reads and writes.
fn hung_up() -> KernelError {
    KernelError::FsError(crate::error::FsError::IoError)
}

impl PtyMaster {
    /// Create a new PTY master
    pub fn new(id: u32) -> Self {
        Self {
            id,
            input_buffer: RwLock::new(VecDeque::with_capacity(PTY_BUFFER_SIZE)),
            output_buffer: RwLock::new(VecDeque::with_capacity(PTY_BUFFER_SIZE)),
            winsize: RwLock::new(Winsize::default()),
            flags: RwLock::new(TermiosFlags::default()),
            controller: RwLock::new(None),
            foreground_pgid: AtomicU64::new(0),
            slaves: AtomicUsize::new(0),
            had_slave: AtomicBool::new(false),
        }
    }

    /// A slave node was opened.
    pub fn slave_opened(&self) {
        self.slaves.fetch_add(1, Ordering::AcqRel);
        self.had_slave.store(true, Ordering::Release);
    }

    /// A slave node closed; the last one hangs the terminal up.
    pub fn slave_closed(&self) {
        if self.slaves.fetch_sub(1, Ordering::AcqRel) == 1 {
            crate::sched::dispatch::io_event();
        }
    }

    /// Every slave that was open has closed.
    fn hung(&self) -> bool {
        self.had_slave.load(Ordering::Acquire) && self.slaves.load(Ordering::Acquire) == 0
    }

    /// Get PTY ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Read from slave output (what the slave wrote). Nothing to read is
    /// `WouldBlock` (EAGAIN, or a sleep for a blocking descriptor): it used
    /// to return 0, which a reader takes as end of file.
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let bytes_to_read = {
            let mut output = self.output_buffer.write();
            let n = buffer.len().min(output.len());
            for slot in buffer.iter_mut().take(n) {
                // n <= output.len(), so pop_front cannot return None
                *slot = output.pop_front().expect("output buffer underrun");
            }
            n
        };
        if bytes_to_read == 0 && !buffer.is_empty() {
            return Err(if self.hung() {
                hung_up()
            } else {
                KernelError::WouldBlock
            });
        }
        // Space freed for the slave's writes.
        crate::sched::dispatch::io_event();
        Ok(bytes_to_read)
    }

    /// poll bits for the master side: POLLIN when the slave has written
    /// something, POLLOUT when the slave's input has room.
    pub fn master_readiness(&self) -> u16 {
        let mut ready = 0;
        if !self.output_buffer.read().is_empty() {
            ready |= POLLIN;
        }
        if self.input_buffer.read().len() < PTY_BUFFER_SIZE {
            ready |= POLLOUT;
        }
        if self.hung() {
            ready |= POLLHUP;
        }
        ready
    }

    /// poll bits for the slave side: POLLIN when the master has written
    /// input, POLLOUT when the output has room.
    pub fn slave_readiness(&self) -> u16 {
        let mut ready = 0;
        if !self.input_buffer.read().is_empty() {
            ready |= POLLIN;
        }
        if self.output_buffer.read().len() < PTY_BUFFER_SIZE {
            ready |= POLLOUT;
        }
        ready
    }

    /// Write to slave input (what the slave will read).
    ///
    /// Returns how many bytes were taken: fewer than `data.len()` when the
    /// input buffer fills, `WouldBlock` (EAGAIN) when none fit. Signals
    /// from ^C/^Z are sent after the buffer lock is released (N-128).
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        let isig = self.flags.read().isig;
        let mut signals = Vec::new();
        let taken = {
            let mut input = self.input_buffer.write();
            master_input(data, isig, &mut input, &mut signals)
        };
        for sig in signals {
            self.send_signal_to_foreground_group(sig);
        }
        if taken == 0 && !data.is_empty() {
            return Err(KernelError::WouldBlock);
        }
        // Input for the slave's readers.
        crate::sched::dispatch::io_event();
        Ok(taken)
    }

    /// Send a signal to all processes in the foreground process group.
    ///
    /// Falls back to the controlling process if no foreground group is set.
    fn send_signal_to_foreground_group(&self, signal: i32) {
        let fg_pgid = self.foreground_pgid.load(Ordering::Acquire);

        if fg_pgid != 0 {
            // Send signal to all processes in the foreground process group
            crate::process::table::PROCESS_TABLE.for_each(|proc| {
                let proc_pgid = proc.pgid.load(Ordering::Acquire);
                if proc_pgid == fg_pgid && proc.is_alive() {
                    if let Err(_e) = {
                        crate::process::signals::notify(proc, signal as usize);
                        Ok::<(), crate::error::KernelError>(())
                    } {
                        crate::println!(
                            "[PTY] Warning: failed to send signal {} to PID {}: {:?}",
                            signal,
                            proc.pid.0,
                            _e
                        );
                    }
                }
            });
        } else if let Some(pid) = *self.controller.read() {
            // Fallback: send to controlling process only
            let process_server = crate::services::process_server::get_process_server();
            if let Err(_e) = process_server.send_signal(pid, signal) {
                crate::println!(
                    "[PTY] Warning: failed to send signal {} to PID {}: {:?}",
                    signal,
                    pid.0,
                    _e
                );
            }
        }
    }

    /// Set window size
    pub fn set_winsize(&self, winsize: Winsize) {
        *self.winsize.write() = winsize;
        // Send SIGWINCH to controlling process
        if let Some(pid) = *self.controller.read() {
            let process_server = crate::services::process_server::get_process_server();
            if let Err(_e) = process_server.send_signal(pid, 28) {
                crate::println!(
                    "[PTY] Warning: failed to send SIGWINCH to PID {}: {:?}",
                    pid.0,
                    _e
                );
            }
        }
    }

    /// Get window size
    pub fn get_winsize(&self) -> Winsize {
        *self.winsize.read()
    }

    /// Set terminal flags
    pub fn set_flags(&self, flags: TermiosFlags) {
        *self.flags.write() = flags;
    }

    /// Get terminal flags
    pub fn get_flags(&self) -> TermiosFlags {
        *self.flags.read()
    }

    /// Set the controlling process for this PTY.
    ///
    /// The controlling process receives signals (SIGINT, SIGTSTP, SIGWINCH)
    /// generated by special terminal characters (^C, ^Z) and window size
    /// changes.
    pub fn set_controller(&self, pid: ProcessId) {
        *self.controller.write() = Some(pid);
    }

    /// Get the controlling process ID, if any.
    pub fn get_controller(&self) -> Option<ProcessId> {
        *self.controller.read()
    }

    /// Set the foreground process group ID for this terminal.
    ///
    /// The foreground process group receives keyboard-generated signals
    /// (SIGINT from ^C, SIGTSTP from ^Z). Processes in background groups
    /// that attempt to read from or write to this terminal receive
    /// SIGTTIN or SIGTTOU respectively.
    pub fn set_foreground_pgid(&self, pgid: u64) {
        self.foreground_pgid.store(pgid, Ordering::Release);
    }

    /// Get the foreground process group ID for this terminal.
    ///
    /// Returns 0 if no foreground group has been set.
    pub fn get_foreground_pgid(&self) -> u64 {
        self.foreground_pgid.load(Ordering::Acquire)
    }
}

/// PTY Slave side (used by shell/application)
pub struct PtySlave {
    /// PTY ID
    id: u32,

    /// Reference to master (for accessing buffers)
    master_id: u32,
}

impl PtySlave {
    /// Create a new PTY slave
    pub fn new(id: u32, master_id: u32) -> Self {
        Self { id, master_id }
    }

    /// Get PTY ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Get master ID
    pub fn master_id(&self) -> u32 {
        self.master_id
    }

    /// Read from master (what user typed)
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        // Get master and read from input buffer
        if let Some(master) = get_pty_master(self.master_id) {
            let bytes_to_read = {
                let mut input = master.input_buffer.write();
                let n = buffer.len().min(input.len());
                for slot in buffer.iter_mut().take(n) {
                    // n <= input.len(), so pop_front cannot return None
                    *slot = input.pop_front().expect("input buffer underrun");
                }
                n
            };
            // Nothing typed yet: wait (or EAGAIN), not end of file.
            if bytes_to_read == 0 && !buffer.is_empty() {
                return Err(KernelError::WouldBlock);
            }
            crate::sched::dispatch::io_event();
            Ok(bytes_to_read)
        } else {
            // The master is gone: EIO, as Linux's hung-up terminal (it was
            // ENOENT).
            Err(hung_up())
        }
    }

    /// Write to master (output to terminal)
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        // Get master and write to output buffer
        if let Some(master) = get_pty_master(self.master_id) {
            let opost = master.flags.read().opost;
            let taken = slave_output(data, opost, &mut master.output_buffer.write());
            if taken == 0 && !data.is_empty() {
                return Err(KernelError::WouldBlock);
            }
            // Output for the master's readers.
            crate::sched::dispatch::io_event();
            Ok(taken)
        } else {
            Err(hung_up())
        }
    }
}

/// Move master-side input into `input` (at most [`PTY_BUFFER_SIZE`]
/// bytes queued). With `isig`, ^C and ^Z are consumed and their signals
/// (SIGINT 2, SIGTSTP 20) appended to `signals` for the caller to send
/// once the lock is dropped. Returns the number of bytes of `data` taken.
fn master_input(
    data: &[u8],
    isig: bool,
    input: &mut VecDeque<u8>,
    signals: &mut Vec<i32>,
) -> usize {
    for (i, &byte) in data.iter().enumerate() {
        if isig && byte == termios::VINTR_CHAR {
            signals.push(2);
            continue;
        }
        if isig && byte == termios::VSUSP_CHAR {
            signals.push(20);
            continue;
        }
        if input.len() >= PTY_BUFFER_SIZE {
            return i;
        }
        input.push_back(byte);
    }
    data.len()
}

/// Move slave output into `output` (at most [`PTY_BUFFER_SIZE`] bytes
/// queued), turning `\n` into `\r\n` with `opost`. A byte is taken only if
/// all of its output fits, so the count returned is exact: the old code
/// dropped a `\n` that did not fit and still reported success (N-128).
fn slave_output(data: &[u8], opost: bool, output: &mut VecDeque<u8>) -> usize {
    for (i, &byte) in data.iter().enumerate() {
        let expanded = opost && byte == b'\n';
        let need = if expanded { 2 } else { 1 };
        if output.len() + need > PTY_BUFFER_SIZE {
            return i;
        }
        if expanded {
            output.push_back(b'\r');
        }
        output.push_back(byte);
    }
    data.len()
}

/// PTY Manager for creating and managing PTY pairs
pub struct PtyManager {
    /// All PTY masters (with interior mutability and Arc for sharing)
    masters: RwLock<Vec<Arc<PtyMaster>>>,

    /// Next PTY ID (atomic for thread-safety)
    next_id: AtomicU32,
}

impl PtyManager {
    /// Create a new PTY manager
    pub fn new() -> Self {
        Self {
            masters: RwLock::new(Vec::new()),
            next_id: AtomicU32::new(0),
        }
    }

    /// Create a new PTY pair
    pub fn create_pty(&self) -> Result<(u32, u32), KernelError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let master = Arc::new(PtyMaster::new(id));
        let master_id = master.id();

        self.masters.write().push(master);

        println!(
            "[PTY] Created PTY pair: master={}, slave={}",
            master_id, master_id
        );

        Ok((master_id, master_id))
    }

    /// Get PTY master by ID
    pub fn get_master(&self, id: u32) -> Option<Arc<PtyMaster>> {
        self.masters.read().iter().find(|m| m.id == id).cloned()
    }

    /// Check if PTY exists
    pub fn has_pty(&self, id: u32) -> bool {
        self.masters.read().iter().any(|m| m.id == id)
    }

    /// Close a PTY
    pub fn close_pty(&self, id: u32) -> Result<(), KernelError> {
        let mut masters = self.masters.write();
        if let Some(pos) = masters.iter().position(|m| m.id == id) {
            masters.remove(pos);
            println!("[PTY] Closed PTY {}", id);
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "pty",
                id: id as u64,
            })
        }
    }

    /// Get number of active PTYs
    pub fn count(&self) -> usize {
        self.masters.read().len()
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global PTY manager
static PTY_MANAGER: GlobalState<PtyManager> = GlobalState::new();

/// Initialize PTY system
pub fn init() -> Result<(), KernelError> {
    let manager = PtyManager::new();
    PTY_MANAGER
        .init(manager)
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    println!("[PTY] Pseudo-terminal system initialized");
    Ok(())
}

/// Execute a function with the PTY manager
pub fn with_pty_manager<R, F: FnOnce(&PtyManager) -> R>(f: F) -> Option<R> {
    PTY_MANAGER.with(f)
}

/// Helper function to get PTY master
fn get_pty_master(id: u32) -> Option<Arc<PtyMaster>> {
    PTY_MANAGER.with(|manager| manager.get_master(id)).flatten()
}

// ============================================================================
// VfsNode wrappers for PTY file descriptors
// ============================================================================

use super::{DirEntry, Metadata, NodeType, Permissions, VfsNode};

/// VfsNode adapter for the master side of a PTY.
///
/// Reading from this node returns bytes that the slave has written (i.e. the
/// program's output). Writing to this node delivers bytes to the slave's input
/// buffer (i.e. simulates keyboard input).
pub struct PtyMasterNode {
    /// The underlying PTY master, shared with the slave view.
    master: Arc<PtyMaster>,
}

/// The last reference to the master side closes the terminal: the PTY
/// leaves the manager (nothing ever removed it, so every openpty leaked
/// one; PR #23 review), and its slaves hang up.
impl Drop for PtyMasterNode {
    fn drop(&mut self) {
        let id = self.master.id();
        let _ = with_pty_manager(|mgr| mgr.close_pty(id));
        crate::sched::dispatch::io_event();
    }
}

impl PtyMasterNode {
    /// Wrap an existing [`PtyMaster`] as a VfsNode.
    pub fn new(master: Arc<PtyMaster>) -> Self {
        Self { master }
    }

    /// Return the PTY ID owned by this master node.
    pub fn pty_id(&self) -> u32 {
        self.master.id()
    }
}

impl VfsNode for PtyMasterNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    /// Read bytes produced by the slave (the program's stdout/stderr).
    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        self.master.read(buffer)
    }

    /// Write bytes that the slave will read (simulate keyboard input).
    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.master.write(data)
    }

    fn poll_readiness(&self) -> u16 {
        self.master.master_readiness()
    }

    /// Every buffer change calls `io_event`, so poll, epoll and blocking
    /// reads sleep instead of re-checking every 10 ms.
    fn wakes_io_waiters(&self) -> bool {
        true
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            node_type: NodeType::CharDevice,
            size: 0,
            permissions: Permissions::from_mode(0o620),
            uid: 0,
            gid: 5, // tty group
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0x9000_0000 | self.master.id() as u64,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate PTY master",
        })
    }
}

/// VfsNode adapter for the slave side of a PTY.
///
/// Reading from this node returns bytes that the master has written (i.e.
/// keyboard input forwarded to the shell). Writing to this node sends bytes
/// to the master's output buffer (the program's output visible in the
/// terminal emulator).
pub struct PtySlaveNode {
    /// The slave descriptor, which back-references the master by ID.
    slave: PtySlave,
}

impl PtySlaveNode {
    /// Wrap a [`PtySlave`] as a VfsNode.
    pub fn new(slave: PtySlave) -> Self {
        if let Some(master) = get_pty_master(slave.master_id()) {
            master.slave_opened();
        }
        Self { slave }
    }

    /// Return the PTY ID of this slave.
    pub fn pty_id(&self) -> u32 {
        self.slave.id()
    }

    /// Return the path that would be exposed as the slave device name.
    pub fn pts_path(&self) -> alloc::string::String {
        format!("/dev/pts/{}", self.slave.id())
    }
}

impl Drop for PtySlaveNode {
    fn drop(&mut self) {
        if let Some(master) = get_pty_master(self.slave.master_id()) {
            master.slave_closed();
        }
    }
}

impl VfsNode for PtySlaveNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    fn is_stream(&self) -> bool {
        true
    }

    /// Read bytes that the master wrote (keyboard input / program stdin).
    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        self.slave.read(buffer)
    }

    /// Write bytes that the master will read (program output / terminal
    /// output).
    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.slave.write(data)
    }

    fn poll_readiness(&self) -> u16 {
        // A slave whose master is gone is hung up: reads and writes give
        // EIO, and poll says so (POLLIN promised data a read did not give).
        get_pty_master(self.slave.master_id()).map_or(POLLHUP | POLLERR, |m| m.slave_readiness())
    }

    fn wakes_io_waiters(&self) -> bool {
        true
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            node_type: NodeType::CharDevice,
            size: 0,
            permissions: Permissions::from_mode(0o620),
            uid: 0,
            gid: 5, // tty group
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0x9100_0000 | self.slave.id() as u64,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::PermissionDenied {
            operation: "truncate PTY slave",
        })
    }
}

#[cfg(test)]
mod tests {
    /// Readiness follows the buffers, and an empty read is WouldBlock (a
    /// sleep for a blocking reader), not 0, which reads as end of file.
    /// Once the last slave closes, the drained master reads EIO and polls
    /// POLLHUP, as on Linux; before any slave, or while one is open, an
    /// empty read waits (PR #24 review).
    #[test]
    fn the_master_hangs_up_after_its_last_slave() {
        let m = PtyMaster::new(9);
        let mut buf = [0u8; 4];
        assert!(matches!(m.read(&mut buf), Err(KernelError::WouldBlock)));
        m.slave_opened();
        m.slave_opened();
        m.slave_closed();
        assert!(matches!(m.read(&mut buf), Err(KernelError::WouldBlock)));
        assert_eq!(m.master_readiness() & POLLHUP, 0);
        m.slave_closed();
        assert!(matches!(
            m.read(&mut buf),
            Err(KernelError::FsError(crate::error::FsError::IoError))
        ));
        assert_ne!(m.master_readiness() & POLLHUP, 0);
    }

    #[test]
    fn readiness_and_empty_reads_follow_the_buffers() {
        let master = PtyMaster::new(91);
        let mut buf = [0u8; 8];
        assert_eq!(master.master_readiness(), POLLOUT);
        assert_eq!(master.slave_readiness(), POLLOUT);
        assert!(matches!(
            master.read(&mut buf),
            Err(KernelError::WouldBlock)
        ));
        assert_eq!(master.write(b"ls").unwrap(), 2);
        assert_eq!(master.slave_readiness(), POLLIN | POLLOUT);
        assert!(master.master_readiness() & POLLIN == 0);
        master.output_buffer.write().push_back(b'x');
        assert_eq!(master.master_readiness(), POLLIN | POLLOUT);
        assert_eq!(master.read(&mut buf).unwrap(), 1);
        assert_eq!(master.master_readiness(), POLLOUT);
        // A full input buffer is not writable.
        master.input_buffer.write().resize(PTY_BUFFER_SIZE, b'a');
        assert_eq!(master.master_readiness() & POLLOUT, 0);
    }

    use super::*;

    #[test]
    fn test_pty_creation() {
        let manager = PtyManager::new();
        let result = manager.create_pty();
        assert!(result.is_ok());
    }

    #[test]
    fn test_pty_read_write() {
        let master = PtyMaster::new(0);
        let _slave = PtySlave::new(0, 0);

        // Write from master
        let data = b"Hello PTY!";
        assert!(master.write(data).is_ok());

        // Read would require accessing the manager
        // This is a simplified test
    }

    #[test]
    fn test_winsize() {
        let master = PtyMaster::new(0);
        let winsize = Winsize {
            rows: 30,
            cols: 100,
            xpixel: 800,
            ypixel: 600,
        };

        master.set_winsize(winsize);
        let retrieved = master.get_winsize();

        assert_eq!(retrieved.rows, 30);
        assert_eq!(retrieved.cols, 100);
    }

    /// N-128: ^C/^Z become signals without being queued, and a full input
    /// buffer gives a short count instead of an error after the fact.
    #[test]
    fn master_input_reports_short_writes_and_signals() {
        let mut input = VecDeque::new();
        let mut sigs = Vec::new();
        let data = [b'a', termios::VINTR_CHAR, b'b', termios::VSUSP_CHAR];
        assert_eq!(master_input(&data, true, &mut input, &mut sigs), 4);
        assert_eq!(input, VecDeque::from(alloc::vec![b'a', b'b']));
        assert_eq!(sigs, alloc::vec![2, 20]);
        // Without ISIG the control characters are data.
        input.clear();
        sigs.clear();
        assert_eq!(master_input(&data, false, &mut input, &mut sigs), 4);
        assert_eq!(input.len(), 4);
        assert!(sigs.is_empty());
        // Room for two more bytes: three offered, two taken.
        input = VecDeque::from(alloc::vec![0u8; PTY_BUFFER_SIZE - 2]);
        assert_eq!(master_input(b"xyz", false, &mut input, &mut sigs), 2);
        assert_eq!(input.len(), PTY_BUFFER_SIZE);
    }

    /// N-128: an OPOST newline needs two slots; it is taken whole or not
    /// at all, and the count says which.
    #[test]
    fn slave_output_counts_opost_newlines_exactly() {
        let mut out = VecDeque::new();
        assert_eq!(slave_output(b"a\nb", true, &mut out), 3);
        assert_eq!(out, VecDeque::from(alloc::vec![b'a', b'\r', b'\n', b'b']));
        // One slot left: "x" fits, the newline does not.
        let mut out = VecDeque::from(alloc::vec![0u8; PTY_BUFFER_SIZE - 2]);
        assert_eq!(slave_output(b"x\n", true, &mut out), 1);
        assert_eq!(out.len(), PTY_BUFFER_SIZE - 1);
        // Without OPOST the newline takes one slot.
        assert_eq!(slave_output(b"\n", false, &mut out), 1);
        assert_eq!(out.len(), PTY_BUFFER_SIZE);
        assert_eq!(slave_output(b"z", false, &mut out), 0);
    }
}
