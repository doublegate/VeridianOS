//! Device Filesystem (/dev)
//!
//! Provides device nodes for hardware and virtual devices.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};

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

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str> {
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
                // TODO(phase4): Dispatch read to actual device driver via driver registry
                Err("Device not implemented")
            }
        }
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, &'static str> {
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
                // TODO(phase4): Dispatch write to actual device driver via driver registry
                Err("Device not implemented")
            }
        }
    }

    fn metadata(&self) -> Result<Metadata, &'static str> {
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

    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str> {
        Err("Not a directory")
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("Not a directory")
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("Cannot create files in device")
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("Cannot create directories in device")
    }

    fn unlink(&self, _name: &str) -> Result<(), &'static str> {
        Err("Cannot unlink device")
    }

    fn truncate(&self, _size: usize) -> Result<(), &'static str> {
        Err("Cannot truncate device")
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

    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, &'static str> {
        Err("Cannot read directory")
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, &'static str> {
        Err("Cannot write to directory")
    }

    fn metadata(&self) -> Result<Metadata, &'static str> {
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

    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str> {
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

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        let devices = self.devices.read();
        devices
            .get(name)
            .map(|node| node.clone() as Arc<dyn VfsNode>)
            .ok_or("Device not found")
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("Cannot create files in /dev")
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("Cannot create directories in /dev")
    }

    fn unlink(&self, _name: &str) -> Result<(), &'static str> {
        Err("Cannot unlink from /dev")
    }

    fn truncate(&self, _size: usize) -> Result<(), &'static str> {
        Err("Cannot truncate directory")
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

    fn sync(&self) -> Result<(), &'static str> {
        Ok(())
    }
}
