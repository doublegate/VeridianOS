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

/// Major number of DRM (Direct Rendering Manager) character devices.
pub(crate) const DRM_MAJOR: u32 = 226;

/// PID of the calling process (0 when there is none, e.g. during boot).
fn current_pid() -> u64 {
    crate::process::current_process().map_or(0, |p| p.pid.0)
}

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

    /// Terminals are streams; null, zero and random take any offset.
    fn is_stream(&self) -> bool {
        matches!(self.name.as_str(), "console" | "tty0")
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
            // The console's line discipline (canonical or raw, echo) is
            // shared with the other console readers; with nothing to return
            // yet this is WouldBlock and read(2) waits (N-118).
            "console" | "tty0" => crate::drivers::console_input::read_node(buffer),
            // evdev input device nodes: /dev/input/event*
            "input/event0" | "input/event1" => {
                let minor = self._minor;
                let bytes_read = crate::drivers::evdev::read_device(minor, buffer);
                Ok(bytes_read)
            }
            // DRI device nodes: /dev/dri/card0, /dev/dri/renderD128
            "dri/card0" | "dri/renderD128" => {
                // DRM read(): return queued page-flip completion events.
                // kwin's render loop reads drm_event_vblank structs (32 bytes
                // each) from the DRM fd after epoll_wait signals readability.
                let bytes = crate::graphics::gpu_accel::read_drm_events(current_pid(), buffer);
                Ok(bytes)
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

    fn poll_readiness(&self) -> u16 {
        match self.name.as_str() {
            // DRM device nodes: only report POLLIN when page-flip events
            // are pending. Always report POLLOUT (writes are ioctl-based).
            // This prevents kwin from reading the DRM fd when no events
            // are queued (which would return 0 bytes = EOF).
            "dri/card0" | "dri/renderD128" => {
                let mut events = 0x0004u16; // POLLOUT
                if crate::graphics::gpu_accel::has_pending_drm_events(current_pid()) {
                    events |= 0x0001; // POLLIN
                }
                events
            }
            "console" | "tty0" => crate::drivers::console_input::tty_readiness(),
            // All other devices: default (always readable + writable)
            _ => 0x0001 | 0x0004, // POLLIN | POLLOUT
        }
    }

    fn device_id(&self) -> Option<(u32, u32)> {
        Some((self._major, self._minor))
    }

    /// Console input wakes waiters from the x86_64 timer tick; elsewhere
    /// (no tick hook yet) readers re-check.
    fn wakes_io_waiters(&self) -> bool {
        cfg!(target_arch = "x86_64") && matches!(self.name.as_str(), "console" | "tty0")
    }

    fn ready_at_ns(&self) -> Option<u64> {
        match self.name.as_str() {
            "console" | "tty0" => crate::drivers::console_input::tty_ready_at(),
            _ => None,
        }
    }
}

/// A device subdirectory (e.g., `/dev/dri/`, `/dev/input/`).
///
/// Contains its own set of device nodes, supports lookup and readdir.
struct DevSubDir {
    #[allow(dead_code)] // Retained for debugging/identification
    name: String,
    devices: RwLock<BTreeMap<String, Arc<DevNode>>>,
}

impl DevSubDir {
    fn new(name: String) -> Self {
        Self {
            name,
            devices: RwLock::new(BTreeMap::new()),
        }
    }

    fn add_device(&self, dev_name: String, node: Arc<DevNode>) {
        self.devices.write().insert(dev_name, node);
    }
}

impl VfsNode for DevSubDir {
    fn node_type(&self) -> NodeType {
        NodeType::Directory
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        // Dispatch reads on child device nodes.
        // This is called when a file under the subdir is read directly.
        // For evdev nodes, read input events; for DRI nodes, not supported.
        let _ = buffer;
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
            operation: "create in device subdir",
        })
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "mkdir in device subdir",
        })
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "unlink device subdir node",
        })
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::IsADirectory))
    }
}

/// Device filesystem root directory
struct DevRoot {
    devices: RwLock<BTreeMap<String, Arc<DevNode>>>,
    /// Subdirectories (dri, input, etc.)
    subdirs: RwLock<BTreeMap<String, Arc<DevSubDir>>>,
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

        // Create /dev/dri/ subdirectory with DRM device nodes
        let dri_dir = Arc::new(DevSubDir::new(String::from("dri")));
        dri_dir.add_device(
            String::from("card0"),
            Arc::new(DevNode::new_char(String::from("dri/card0"), DRM_MAJOR, 0)),
        );
        dri_dir.add_device(
            String::from("renderD128"),
            Arc::new(DevNode::new_char(
                String::from("dri/renderD128"),
                DRM_MAJOR,
                128,
            )),
        );

        // Create /dev/input/ subdirectory with evdev device nodes
        let input_dir = Arc::new(DevSubDir::new(String::from("input")));
        input_dir.add_device(
            String::from("event0"),
            Arc::new(DevNode::new_char(String::from("input/event0"), 13, 64)),
        );
        input_dir.add_device(
            String::from("event1"),
            Arc::new(DevNode::new_char(String::from("input/event1"), 13, 65)),
        );

        let mut subdirs = BTreeMap::new();
        subdirs.insert(String::from("dri"), dri_dir);
        subdirs.insert(String::from("input"), input_dir);

        Self {
            devices: RwLock::new(devices),
            subdirs: RwLock::new(subdirs),
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
        let subdirs = self.subdirs.read();
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

        // List subdirectories (dri, input)
        for (name, _) in subdirs.iter() {
            entries.push(DirEntry {
                name: name.clone(),
                node_type: NodeType::Directory,
                inode: 0,
            });
        }

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
        // Check subdirectories first (dri, input)
        {
            let subdirs = self.subdirs.read();
            if let Some(subdir) = subdirs.get(name) {
                return Ok(subdir.clone() as Arc<dyn VfsNode>);
            }
        }

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
