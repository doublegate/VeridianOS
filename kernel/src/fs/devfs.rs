//! Device Filesystem (/dev)
//!
//! Provides device nodes for hardware and virtual devices.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// Device node
struct DevNode {
    name: String,
    node_type: NodeType,
    _major: u32,
    _minor: u32,
    permissions: Permissions,
}

impl DevNode {
    fn new_char(name: String, major: u32, minor: u32) -> Self {
        Self {
            name,
            node_type: NodeType::CharDevice,
            _major: major,
            _minor: minor,
            permissions: Permissions::default(),
        }
    }

    #[allow(dead_code)] // Block device creation API -- used when block devices are registered
    fn new_block(name: String, major: u32, minor: u32) -> Self {
        Self {
            name,
            node_type: NodeType::BlockDevice,
            _major: major,
            _minor: minor,
            permissions: Permissions::default(),
        }
    }
}

impl VfsNode for DevNode {
    fn node_type(&self) -> NodeType {
        self.node_type
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        // Special handling for common devices
        match self.name.as_str() {
            "null" => {
                // /dev/null always returns EOF
                Ok(0)
            }
            "zero" => {
                // /dev/zero returns zeros
                buffer.fill(0);
                Ok(buffer.len())
            }
            "random" | "urandom" => {
                // Simple pseudo-random for now
                for byte in buffer.iter_mut() {
                    *byte = (crate::read_timestamp() & 0xFF) as u8;
                }
                Ok(buffer.len())
            }
            "console" | "tty0" => {
                // Read from serial console, respecting terminal state (ICANON, ECHO).
                //
                // Canonical mode (ICANON set, default): buffer until newline/EOF,
                // support VERASE (backspace), return full line.
                // Raw mode (ICANON cleared): return characters immediately per
                // VMIN/VTIME settings.
                #[cfg(target_arch = "x86_64")]
                {
                    use crate::drivers::terminal;

                    let canonical = terminal::is_canonical_mode();
                    let echo = terminal::is_echo_enabled();
                    let verase = terminal::get_verase();
                    let vmin = terminal::get_vmin() as usize;

                    if canonical {
                        // Canonical mode: line-buffered with editing support.
                        // Buffer characters until newline, CR, or EOF (Ctrl-D).
                        // Support backspace (VERASE) to erase last character.
                        //
                        // Block indefinitely waiting for input, just like a real
                        // terminal device. POSIX requires read() on a terminal to
                        // block until data is available. Without this, shells and
                        // other interactive programs see EOF and exit immediately.
                        let mut line_buf: [u8; 4096] = [0u8; 4096];
                        let mut line_len: usize = 0;

                        loop {
                            let byte = loop {
                                let lsr: u8;
                                // SAFETY: Reading COM1 LSR (port 0x3FD).
                                unsafe {
                                    core::arch::asm!(
                                        "in al, dx",
                                        out("al") lsr,
                                        in("dx") 0x3FDu16,
                                        options(nomem, nostack)
                                    );
                                }
                                if lsr & 1 != 0 {
                                    let data: u8;
                                    // SAFETY: Reading COM1 data register (port 0x3F8).
                                    unsafe {
                                        core::arch::asm!(
                                            "in al, dx",
                                            out("al") data,
                                            in("dx") 0x3F8u16,
                                            options(nomem, nostack)
                                        );
                                    }
                                    break data;
                                }
                                core::hint::spin_loop();
                            };

                            // Handle special characters in canonical mode
                            if byte == verase || byte == 8 {
                                // Backspace: erase last character
                                if line_len > 0 {
                                    line_len -= 1;
                                    if echo {
                                        // Echo backspace-space-backspace
                                        for &b in &[8u8, b' ', 8u8] {
                                            // SAFETY: Writing COM1 data register.
                                            unsafe {
                                                while {
                                                    let s: u8;
                                                    core::arch::asm!(
                                                        "in al, dx",
                                                        out("al") s,
                                                        in("dx") 0x3FDu16,
                                                        options(nomem, nostack)
                                                    );
                                                    s & 0x20 == 0
                                                } {
                                                    core::hint::spin_loop();
                                                }
                                                core::arch::asm!(
                                                    "out dx, al",
                                                    in("al") b,
                                                    in("dx") 0x3F8u16,
                                                    options(nomem, nostack)
                                                );
                                            }
                                        }
                                    }
                                }
                                continue;
                            }

                            // CR -> NL conversion (c_iflag ICRNL)
                            let byte = if byte == b'\r' { b'\n' } else { byte };

                            // Store in line buffer
                            if line_len < line_buf.len() {
                                line_buf[line_len] = byte;
                                line_len += 1;
                            }

                            // Echo the character
                            if echo {
                                let echo_byte = byte;
                                // SAFETY: Writing COM1 data register.
                                unsafe {
                                    while {
                                        let s: u8;
                                        core::arch::asm!(
                                            "in al, dx",
                                            out("al") s,
                                            in("dx") 0x3FDu16,
                                            options(nomem, nostack)
                                        );
                                        s & 0x20 == 0
                                    } {
                                        core::hint::spin_loop();
                                    }
                                    core::arch::asm!(
                                        "out dx, al",
                                        in("al") echo_byte,
                                        in("dx") 0x3F8u16,
                                        options(nomem, nostack)
                                    );
                                    // Also echo CR after NL for terminal display
                                    if echo_byte == b'\n' {
                                        while {
                                            let s: u8;
                                            core::arch::asm!(
                                                "in al, dx",
                                                out("al") s,
                                                in("dx") 0x3FDu16,
                                                options(nomem, nostack)
                                            );
                                            s & 0x20 == 0
                                        } {
                                            core::hint::spin_loop();
                                        }
                                        core::arch::asm!(
                                            "out dx, al",
                                            in("al") b'\r',
                                            in("dx") 0x3F8u16,
                                            options(nomem, nostack)
                                        );
                                    }
                                }
                            }

                            // Line complete on newline or EOF
                            if byte == b'\n' || byte == 4 {
                                // Ctrl-D (EOF)
                                let copy_len = line_len.min(buffer.len());
                                buffer[..copy_len].copy_from_slice(&line_buf[..copy_len]);
                                return Ok(copy_len);
                            }
                        }
                    } else {
                        // Raw mode (non-canonical): return characters immediately.
                        // VMIN controls minimum chars, VTIME controls timeout.
                        let target = if vmin == 0 { 1 } else { vmin.min(buffer.len()) };
                        let max_spins: u64 = if terminal::get_vtime() > 0 {
                            // VTIME is in tenths of a second; approximate with spin count
                            (terminal::get_vtime() as u64) * 10_000_000
                        } else if vmin == 0 {
                            // VMIN=0, VTIME=0: pure non-blocking
                            1
                        } else {
                            // VMIN>0, VTIME=0: block indefinitely
                            u64::MAX
                        };

                        let mut bytes_read = 0usize;
                        let mut total_spins: u64 = 0;

                        while bytes_read < target {
                            let lsr: u8;
                            // SAFETY: Reading COM1 LSR (port 0x3FD).
                            unsafe {
                                core::arch::asm!(
                                    "in al, dx",
                                    out("al") lsr,
                                    in("dx") 0x3FDu16,
                                    options(nomem, nostack)
                                );
                            }
                            if lsr & 1 != 0 {
                                let data: u8;
                                // SAFETY: Reading COM1 data register (port 0x3F8).
                                unsafe {
                                    core::arch::asm!(
                                        "in al, dx",
                                        out("al") data,
                                        in("dx") 0x3F8u16,
                                        options(nomem, nostack)
                                    );
                                }
                                buffer[bytes_read] = data;
                                bytes_read += 1;

                                // Echo if enabled (even in raw mode)
                                if echo {
                                    // SAFETY: Writing COM1 data register.
                                    unsafe {
                                        while {
                                            let s: u8;
                                            core::arch::asm!(
                                                "in al, dx",
                                                out("al") s,
                                                in("dx") 0x3FDu16,
                                                options(nomem, nostack)
                                            );
                                            s & 0x20 == 0
                                        } {
                                            core::hint::spin_loop();
                                        }
                                        core::arch::asm!(
                                            "out dx, al",
                                            in("al") data,
                                            in("dx") 0x3F8u16,
                                            options(nomem, nostack)
                                        );
                                    }
                                }

                                // Reset timeout after each character
                                total_spins = 0;
                            } else {
                                total_spins += 1;
                                if total_spins >= max_spins {
                                    break;
                                }
                                core::hint::spin_loop();
                            }
                        }

                        Ok(bytes_read)
                    }
                }
                // Non-x86_64: no serial port polling available, return EOF
                #[cfg(not(target_arch = "x86_64"))]
                {
                    let _ = buffer;
                    Ok(0)
                }
            }
            _ => {
                // Dispatch read to registered device driver via driver framework
                if let Some(fw) = crate::services::driver_framework::try_get_driver_framework() {
                    // Look up device by major/minor number
                    let devices = fw.list_devices();
                    for dev in &devices {
                        if let Some(ref driver_name) = dev.driver {
                            // Match device name to our device node name
                            if dev.name == self.name {
                                // Device has a bound driver -- attempt read via framework
                                return fw.read_device(dev.id, _offset as u64, buffer).map_err(
                                    |_| KernelError::OperationNotSupported {
                                        operation: "device read failed",
                                    },
                                );
                            }
                            let _ = driver_name; // suppress unused warning
                        }
                    }
                }
                Err(KernelError::OperationNotSupported {
                    operation: "read on unregistered device",
                })
            }
        }
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        match self.name.as_str() {
            "null" => {
                // /dev/null discards all data
                Ok(data.len())
            }
            "console" | "tty0" => {
                // Write to console
                for &_byte in data {
                    crate::print!("{}", _byte as char);
                }
                Ok(data.len())
            }
            _ => {
                // Dispatch write to registered device driver via driver framework
                if let Some(fw) = crate::services::driver_framework::try_get_driver_framework() {
                    let devices = fw.list_devices();
                    for dev in &devices {
                        if let Some(ref driver_name) = dev.driver {
                            if dev.name == self.name {
                                return fw.write_device(dev.id, _offset as u64, data).map_err(
                                    |_| KernelError::OperationNotSupported {
                                        operation: "device write failed",
                                    },
                                );
                            }
                            let _ = driver_name;
                        }
                    }
                }
                Err(KernelError::OperationNotSupported {
                    operation: "write on unregistered device",
                })
            }
        }
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            node_type: self.node_type,
            size: 0,
            permissions: self.permissions,
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "create in device node",
        })
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "mkdir in device node",
        })
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "unlink device node",
        })
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "truncate device node",
        })
    }
}

/// Device filesystem root directory
struct DevRoot {
    devices: RwLock<BTreeMap<String, Arc<DevNode>>>,
}

impl DevRoot {
    fn new() -> Self {
        let mut devices = BTreeMap::new();

        // Create standard device nodes
        devices.insert(
            String::from("null"),
            Arc::new(DevNode::new_char(String::from("null"), 1, 3)),
        );

        devices.insert(
            String::from("zero"),
            Arc::new(DevNode::new_char(String::from("zero"), 1, 5)),
        );

        devices.insert(
            String::from("random"),
            Arc::new(DevNode::new_char(String::from("random"), 1, 8)),
        );

        devices.insert(
            String::from("urandom"),
            Arc::new(DevNode::new_char(String::from("urandom"), 1, 9)),
        );

        devices.insert(
            String::from("console"),
            Arc::new(DevNode::new_char(String::from("console"), 5, 1)),
        );

        devices.insert(
            String::from("tty0"),
            Arc::new(DevNode::new_char(String::from("tty0"), 4, 0)),
        );

        Self {
            devices: RwLock::new(devices),
        }
    }
}

impl VfsNode for DevRoot {
    fn node_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(FsError::IsADirectory))
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(FsError::IsADirectory))
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            node_type: NodeType::Directory,
            size: 0,
            permissions: Permissions::default(),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        let devices = self.devices.read();
        let mut entries = Vec::new();

        entries.push(DirEntry {
            name: String::from("."),
            node_type: NodeType::Directory,
            inode: 0,
        });

        entries.push(DirEntry {
            name: String::from(".."),
            node_type: NodeType::Directory,
            inode: 0,
        });

        for (name, device) in devices.iter() {
            entries.push(DirEntry {
                name: name.clone(),
                node_type: device.node_type,
                inode: 0,
            });
        }

        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let devices = self.devices.read();
        devices
            .get(name)
            .map(|node| node.clone() as Arc<dyn VfsNode>)
            .ok_or(KernelError::FsError(FsError::NotFound))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "create in /dev",
        })
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "mkdir in /dev",
        })
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "unlink in /dev",
        })
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::IsADirectory))
    }
}

/// Device filesystem
pub struct DevFs {
    root: Arc<DevRoot>,
}

impl DevFs {
    pub fn new() -> Self {
        Self {
            root: Arc::new(DevRoot::new()),
        }
    }
}

impl Default for DevFs {
    fn default() -> Self {
        Self::new()
    }
}

impl Filesystem for DevFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "devfs"
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), KernelError> {
        Ok(())
    }
}
