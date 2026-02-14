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

    #[allow(dead_code)]
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
