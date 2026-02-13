//! Pseudo-Terminal (PTY) Support
//!
//! Provides pseudo-terminal devices for terminal emulation and shell
//! interaction.

// Allow dead code for PTY fields not yet used in current implementation
#![allow(dead_code, clippy::needless_range_loop)]

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

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
            buffer[i] = output.pop_front().unwrap();
        }

        Ok(bytes_to_read)
    }

    /// Write to slave input (what the slave will read)
    pub fn write(&self, data: &[u8]) -> Result<usize, KernelError> {
        let mut input = self.input_buffer.write();
        let flags = self.flags.read();

        for &byte in data {
            // Handle special characters if needed
            if flags.isig && byte == termios::VINTR_CHAR {
                // TODO: Send SIGINT to foreground process group
                continue;
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

    /// Set window size
    pub fn set_winsize(&self, winsize: Winsize) {
        *self.winsize.write() = winsize;
        // TODO: Send SIGWINCH to foreground process group
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
                buffer[i] = input.pop_front().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_pty_creation() {
        let mut manager = PtyManager::new();
        let result = manager.create_pty();
        assert!(result.is_ok());
    }

    #[test_case]
    fn test_pty_read_write() {
        let master = PtyMaster::new(0);
        let slave = PtySlave::new(0, 0);

        // Write from master
        let data = b"Hello PTY!";
        assert!(master.write(data).is_ok());

        // Read would require accessing the manager
        // This is a simplified test
    }

    #[test_case]
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
