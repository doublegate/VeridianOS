//! Process Filesystem (/proc)
//!
//! Provides information about running processes and system state.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::sync::Arc;
use alloc::format;
use super::{VfsNode, Filesystem, NodeType, Metadata, Permissions, DirEntry};

/// ProcFS node types
enum ProcNodeType {
    Root,
    ProcessDir(u64),
    ProcessFile(u64, String),
    SystemFile(String),
}

/// ProcFS node
struct ProcNode {
    node_type: ProcNodeType,
}

impl ProcNode {
    fn new_root() -> Self {
        Self {
            node_type: ProcNodeType::Root,
        }
    }
    
    fn new_process_dir(pid: u64) -> Self {
        Self {
            node_type: ProcNodeType::ProcessDir(pid),
        }
    }
    
    fn new_process_file(pid: u64, name: String) -> Self {
        Self {
            node_type: ProcNodeType::ProcessFile(pid, name),
        }
    }
    
    fn new_system_file(name: String) -> Self {
        Self {
            node_type: ProcNodeType::SystemFile(name),
        }
    }
}

impl VfsNode for ProcNode {
    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let content = match &self.node_type {
            ProcNodeType::SystemFile(name) => {
                match name.as_str() {
                    "version" => {
                        format!("VeridianOS 0.3.0-dev\n")
                    }
                    "uptime" => {
                        let ticks = crate::read_timestamp();
                        format!("{} seconds\n", ticks / 1_000_000)
                    }
                    "meminfo" => {
                        // Get actual memory statistics from the allocator
                        let stats = crate::mm::get_memory_stats();
                        let total_kb = stats.total_frames * 4; // 4KB per frame
                        let free_kb = stats.free_frames * 4;
                        let used_kb = total_kb - free_kb;
                        let available_kb = free_kb + (stats.cached_frames * 4); // Free + cached
                        
                        format!(
                            "MemTotal:       {} kB\n\
                             MemFree:        {} kB\n\
                             MemUsed:        {} kB\n\
                             MemAvailable:   {} kB\n\
                             Cached:         {} kB\n",
                            total_kb, free_kb, used_kb, available_kb, stats.cached_frames * 4
                        )
                    }
                    "cpuinfo" => {
                        #[cfg(target_arch = "x86_64")]
                        let arch = "x86_64";
                        #[cfg(target_arch = "aarch64")]
                        let arch = "aarch64";
                        #[cfg(target_arch = "riscv64")]
                        let arch = "riscv64";
                        
                        format!(
                            "processor\t: 0\n\
                             architecture\t: {}\n\
                             model name\t: VeridianOS Virtual CPU\n",
                            arch
                        )
                    }
                    _ => String::new(),
                }
            }
            ProcNodeType::ProcessFile(pid, name) => {
                match name.as_str() {
                    "status" => {
                        // Get actual process information
                        if let Some(process) = crate::process::get_process(crate::process::ProcessId(*pid)) {
                            let state = match process.get_state() {
                                crate::process::ProcessState::Creating => "N (new)",
                                crate::process::ProcessState::Ready => "R (running)",
                                crate::process::ProcessState::Running => "R (running)",
                                crate::process::ProcessState::Blocked => "S (sleeping)",
                                crate::process::ProcessState::Sleeping => "S (sleeping)",
                                crate::process::ProcessState::Zombie => "Z (zombie)",
                                crate::process::ProcessState::Dead => "X (dead)",
                            };
                            
                            #[cfg(feature = "alloc")]
                            let name = &process.name;
                            #[cfg(not(feature = "alloc"))]
                            let name = "process";
                            
                            let parent = process.parent.unwrap_or(crate::process::ProcessId(0));
                            
                            format!(
                                "Name:\t{}\n\
                                 Pid:\t{}\n\
                                 PPid:\t{}\n\
                                 State:\t{}\n",
                                name,
                                pid,
                                parent.0,
                                state
                            )
                        } else {
                            format!("Name:\tProcess\nPid:\t{}\nState:\tR (running)\n", pid)
                        }
                    }
                    "cmdline" => {
                        // Get actual command line
                        if let Some(process) = crate::process::get_process(crate::process::ProcessId(*pid)) {
                            #[cfg(feature = "alloc")]
                            let name = &process.name;
                            #[cfg(not(feature = "alloc"))]
                            let name = "process";
                            format!("{}\0", name)
                        } else {
                            format!("init\0")
                        }
                    }
                    _ => String::new(),
                }
            }
            _ => return Err("Cannot read this file"),
        };
        
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }
        
        let bytes_to_read = core::cmp::min(buffer.len(), bytes.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&bytes[offset..offset + bytes_to_read]);
        Ok(bytes_to_read)
    }
    
    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, &'static str> {
        Err("ProcFS is read-only")
    }
    
    fn metadata(&self) -> Result<Metadata, &'static str> {
        let node_type = match &self.node_type {
            ProcNodeType::Root | ProcNodeType::ProcessDir(_) => NodeType::Directory,
            _ => NodeType::File,
        };
        
        Ok(Metadata {
            node_type,
            size: 0,
            permissions: Permissions::read_only(),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
        })
    }
    
    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str> {
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
        
        match &self.node_type {
            ProcNodeType::Root => {
                // System files
                entries.push(DirEntry {
                    name: String::from("version"),
                    node_type: NodeType::File,
                    inode: 0,
                });
                
                entries.push(DirEntry {
                    name: String::from("uptime"),
                    node_type: NodeType::File,
                    inode: 0,
                });
                
                entries.push(DirEntry {
                    name: String::from("meminfo"),
                    node_type: NodeType::File,
                    inode: 0,
                });
                
                entries.push(DirEntry {
                    name: String::from("cpuinfo"),
                    node_type: NodeType::File,
                    inode: 0,
                });
                
                // Add process directories for all running processes
                if let Some(process_list) = crate::process::get_process_list() {
                    for pid in process_list {
                        entries.push(DirEntry {
                            name: format!("{}", pid),
                            node_type: NodeType::Directory,
                            inode: pid,
                        });
                    }
                } else {
                    // Fallback: just show init process
                    entries.push(DirEntry {
                        name: String::from("1"),
                        node_type: NodeType::Directory,
                        inode: 1,
                    });
                }
            }
            ProcNodeType::ProcessDir(_pid) => {
                entries.push(DirEntry {
                    name: String::from("status"),
                    node_type: NodeType::File,
                    inode: 0,
                });
                
                entries.push(DirEntry {
                    name: String::from("cmdline"),
                    node_type: NodeType::File,
                    inode: 0,
                });
            }
            _ => return Err("Not a directory"),
        }
        
        Ok(entries)
    }
    
    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        match &self.node_type {
            ProcNodeType::Root => {
                // Check for system files
                match name {
                    "version" | "uptime" | "meminfo" | "cpuinfo" => {
                        Ok(Arc::new(ProcNode::new_system_file(String::from(name))) as Arc<dyn VfsNode>)
                    }
                    _ => {
                        // Try to parse as PID
                        if let Ok(pid) = name.parse::<u64>() {
                            // TODO: Check if process exists
                            Ok(Arc::new(ProcNode::new_process_dir(pid)) as Arc<dyn VfsNode>)
                        } else {
                            Err("File not found")
                        }
                    }
                }
            }
            ProcNodeType::ProcessDir(pid) => {
                match name {
                    "status" | "cmdline" => {
                        Ok(Arc::new(ProcNode::new_process_file(*pid, String::from(name))) as Arc<dyn VfsNode>)
                    }
                    _ => Err("File not found")
                }
            }
            _ => Err("Not a directory")
        }
    }
    
    fn create(&self, _name: &str, _permissions: Permissions) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("ProcFS is read-only")
    }
    
    fn mkdir(&self, _name: &str, _permissions: Permissions) -> Result<Arc<dyn VfsNode>, &'static str> {
        Err("ProcFS is read-only")
    }
    
    fn unlink(&self, _name: &str) -> Result<(), &'static str> {
        Err("ProcFS is read-only")
    }
    
    fn truncate(&self, _size: usize) -> Result<(), &'static str> {
        Err("ProcFS is read-only")
    }
}

/// Process filesystem
pub struct ProcFs {
    root: Arc<ProcNode>,
}

impl ProcFs {
    pub fn new() -> Self {
        Self {
            root: Arc::new(ProcNode::new_root()),
        }
    }
}

impl Filesystem for ProcFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }
    
    fn name(&self) -> &str {
        "procfs"
    }
    
    fn is_readonly(&self) -> bool {
        true
    }
    
    fn sync(&self) -> Result<(), &'static str> {
        Ok(())
    }
}