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
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use crate::{error::KernelError, process::ProcessId, sync::once_lock::GlobalState};

/// PTY buffer size
const PTY_BUFFER_SIZE: usize = 4096;

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
        }
    }

    /// Get PTY ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Read from slave output (what the slave wrote)
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let mut output = self.output_buffer.write();
        let bytes_to_read = buffer.len().min(output.len());

        for i in 0..bytes_to_read {
            // bytes_to_read <= output.len(), so pop_front cannot return None
            buffer[i] = output.pop_front().expect("output buffer underrun");
        }

        Ok(bytes_to_read)
    }

    /// Write to slave input (what the slave will read)
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        let mut input = self.input_buffer.write();
        let flags = self.flags.read();

        for &byte in data {
            // Handle special characters if signals are enabled
            if flags.isig {
                if byte == termios::VINTR_CHAR {
                    // Send SIGINT to foreground process group (^C)
                    self.send_signal_to_foreground_group(2);
                    continue;
                }

                if byte == termios::VSUSP_CHAR {
                    // Send SIGTSTP to foreground process group (^Z)
                    self.send_signal_to_foreground_group(20);
                    continue;
                }
            }

            if input.len() < PTY_BUFFER_SIZE {
                input.push_back(byte);
            } else {
                return Err(KernelError::ResourceExhausted {
                    resource: "pty_input_buffer",
                });
            }
        }

        Ok(data.len())
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
                    if let Err(_e) = proc.send_signal(signal as usize) {
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
            let mut input = master.input_buffer.write();
            let bytes_to_read = buffer.len().min(input.len());

            for i in 0..bytes_to_read {
                // bytes_to_read <= input.len(), so pop_front cannot return None
                buffer[i] = input.pop_front().expect("input buffer underrun");
            }

            Ok(bytes_to_read)
        } else {
            Err(KernelError::NotFound {
                resource: "pty_master",
                id: self.master_id as u64,
            })
        }
    }

    /// Write to master (output to terminal)
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        // Get master and write to output buffer
        if let Some(master) = get_pty_master(self.master_id) {
            let mut output = master.output_buffer.write();
            let flags = master.flags.read();

            for &byte in data {
                // Process output if opost is enabled
                if flags.opost && byte == b'\n' {
                    // Convert \n to \r\n
                    if output.len() < PTY_BUFFER_SIZE - 1 {
                        output.push_back(b'\r');
                        output.push_back(b'\n');
                    }
                } else if output.len() < PTY_BUFFER_SIZE {
                    output.push_back(byte);
                } else {
                    return Err(KernelError::ResourceExhausted {
                        resource: "pty_output_buffer",
                    });
                }
            }

            Ok(data.len())
        } else {
            Err(KernelError::NotFound {
                resource: "pty_master",
                id: self.master_id as u64,
            })
        }
    }
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

    /// Read bytes produced by the slave (the program's stdout/stderr).
    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        self.master.read(buffer)
    }

    /// Write bytes that the slave will read (simulate keyboard input).
    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        self.master.write(data)
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

impl VfsNode for PtySlaveNode {
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
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
    use super::*;

    #[test]
    fn test_pty_creation() {
        let mut manager = PtyManager::new();
        let result = manager.create_pty();
        assert!(result.is_ok());
    }

    #[test]
    fn test_pty_read_write() {
        let master = PtyMaster::new(0);
        let slave = PtySlave::new(0, 0);

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
}
