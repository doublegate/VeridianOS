//! Process Filesystem (/proc)
//!
//! Provides information about running processes and system state.
//! Populated with real data from CPUID, frame allocator, timer, and
//! process table.

#![allow(clippy::useless_format)]

use alloc::{format, string::String, sync::Arc, vec::Vec};

use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

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
    fn node_type(&self) -> NodeType {
        match &self.node_type {
            ProcNodeType::Root | ProcNodeType::ProcessDir(_) => NodeType::Directory,
            ProcNodeType::ProcessFile(_, _) | ProcNodeType::SystemFile(_) => NodeType::File,
        }
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let content = match &self.node_type {
            ProcNodeType::SystemFile(name) => {
                match name.as_str() {
                    "version" => {
                        format!(
                            "VeridianOS version 0.5.0 (gcc 14.2.0) #1 SMP {}\n",
                            env!("CARGO_PKG_VERSION")
                        )
                    }
                    "uptime" => {
                        let secs = crate::arch::timer::get_timestamp_secs();
                        let frac_ms = crate::arch::timer::get_timestamp_ms() % 1000;
                        // Linux format: uptime_secs idle_secs
                        format!("{}.{:02} 0.00\n", secs, frac_ms / 10)
                    }
                    "meminfo" => {
                        let stats = crate::mm::get_memory_stats();
                        let total_kb = stats.total_frames * 4; // 4KB per frame
                        let free_kb = stats.free_frames * 4;
                        let used_kb = total_kb.saturating_sub(free_kb);
                        let cached_kb = stats.cached_frames * 4;
                        let available_kb = free_kb + cached_kb;
                        let buffers_kb = 0usize;
                        let slab_kb = 0usize;

                        format!(
                            "MemTotal:       {:>8} kB\n\
                             MemFree:        {:>8} kB\n\
                             MemAvailable:   {:>8} kB\n\
                             Buffers:        {:>8} kB\n\
                             Cached:         {:>8} kB\n\
                             Slab:           {:>8} kB\n\
                             MemUsed:        {:>8} kB\n",
                            total_kb,
                            free_kb,
                            available_kb,
                            buffers_kb,
                            cached_kb,
                            slab_kb,
                            used_kb,
                        )
                    }
                    "cpuinfo" => generate_cpuinfo(),
                    "loadavg" => generate_loadavg(),
                    _ => String::new(),
                }
            }
            ProcNodeType::ProcessFile(pid, name) => {
                match name.as_str() {
                    "status" => {
                        // Get actual process information
                        if let Some(process) =
                            crate::process::get_process(crate::process::ProcessId(*pid))
                        {
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
                                "Name:\t{}\nPid:\t{}\nPPid:\t{}\nState:\t{}\n",
                                name, pid, parent.0, state
                            )
                        } else {
                            format!("Name:\tProcess\nPid:\t{}\nState:\tR (running)\n", pid)
                        }
                    }
                    "cmdline" => {
                        // Get actual command line
                        if let Some(process) =
                            crate::process::get_process(crate::process::ProcessId(*pid))
                        {
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
            _ => return Err(KernelError::FsError(FsError::NotAFile)),
        };

        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }

        let bytes_to_read = core::cmp::min(buffer.len(), bytes.len() - offset);
        buffer[..bytes_to_read].copy_from_slice(&bytes[offset..offset + bytes_to_read]);
        Ok(bytes_to_read)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
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
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
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

                entries.push(DirEntry {
                    name: String::from("loadavg"),
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
            _ => return Err(KernelError::FsError(FsError::NotADirectory)),
        }

        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        match &self.node_type {
            ProcNodeType::Root => {
                // Check for system files
                match name {
                    "version" | "uptime" | "meminfo" | "cpuinfo" | "loadavg" => {
                        Ok(Arc::new(ProcNode::new_system_file(String::from(name)))
                            as Arc<dyn VfsNode>)
                    }
                    _ => {
                        // Try to parse as PID
                        if let Ok(pid) = name.parse::<u64>() {
                            // Validate PID exists in process table
                            if crate::process::get_process(crate::process::ProcessId(pid)).is_some()
                            {
                                Ok(Arc::new(ProcNode::new_process_dir(pid)) as Arc<dyn VfsNode>)
                            } else {
                                Err(KernelError::FsError(FsError::NotFound))
                            }
                        } else {
                            Err(KernelError::FsError(FsError::NotFound))
                        }
                    }
                }
            }
            ProcNodeType::ProcessDir(pid) => match name {
                "status" | "cmdline" => Ok(Arc::new(ProcNode::new_process_file(
                    *pid,
                    String::from(name),
                )) as Arc<dyn VfsNode>),
                _ => Err(KernelError::FsError(FsError::NotFound)),
            },
            _ => Err(KernelError::FsError(FsError::NotADirectory)),
        }
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }
}

/// Generate /proc/cpuinfo content with real CPUID data on x86_64.
fn generate_cpuinfo() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        let brand = cpuid_brand_string();
        let freq_mhz = crate::arch::timer::hw_ticks_per_second() / 1_000_000;

        format!(
            "processor\t: 0\nvendor_id\t: {}\nmodel name\t: {}\ncpu MHz\t\t: {}\ncache size\t: 0 \
             KB\nbogomips\t: {}\n",
            cpuid_vendor_id(),
            brand,
            freq_mhz,
            freq_mhz * 2,
        )
    }

    #[cfg(target_arch = "aarch64")]
    {
        let freq_mhz = crate::arch::timer::hw_ticks_per_second() / 1_000_000;
        format!(
            "processor\t: 0\narchitecture\t: aarch64\nmodel name\t: ARMv8 Processor\nBogoMIPS\t: \
             {}\nFeatures\t: fp asimd\n",
            freq_mhz * 2,
        )
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        format!("processor\t: 0\narchitecture\t: riscv64\nmodel name\t: RISC-V Processor\n")
    }
}

/// Generate /proc/loadavg content.
fn generate_loadavg() -> String {
    // Count running/total tasks from process table
    let (running, total) = if let Some(pids) = crate::process::get_process_list() {
        let total = pids.len();
        let mut running = 0usize;
        for pid in &pids {
            if let Some(p) = crate::process::get_process(crate::process::ProcessId(*pid)) {
                match p.get_state() {
                    crate::process::ProcessState::Running | crate::process::ProcessState::Ready => {
                        running += 1;
                    }
                    _ => {}
                }
            }
        }
        (running, total)
    } else {
        (1, 1)
    };

    // Find highest PID for last field
    let last_pid = crate::process::get_process_list()
        .and_then(|pids| pids.iter().copied().max())
        .unwrap_or(1);

    // Linux format: 1min 5min 15min running/total last_pid
    format!("0.00 0.00 0.00 {}/{} {}\n", running, total, last_pid)
}

/// Read CPUID vendor ID string (x86_64 only).
#[cfg(target_arch = "x86_64")]
fn cpuid_vendor_id() -> String {
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    // SAFETY: CPUID leaf 0 returns vendor string in EBX:EDX:ECX.
    // push/pop RBX required because CPUID clobbers it.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "xor eax, eax",
            "cpuid",
            "mov {0:e}, ebx",
            "mov {1:e}, ecx",
            "mov {2:e}, edx",
            "pop rbx",
            out(reg) ebx,
            out(reg) ecx,
            out(reg) edx,
            out("eax") _,
        );
    }
    let mut buf = [0u8; 12];
    buf[0..4].copy_from_slice(&ebx.to_le_bytes());
    buf[4..8].copy_from_slice(&edx.to_le_bytes());
    buf[8..12].copy_from_slice(&ecx.to_le_bytes());
    String::from_utf8_lossy(&buf).trim_end_matches('\0').into()
}

/// Read CPUID brand string (x86_64 only, leaves 0x80000002-0x80000004).
#[cfg(target_arch = "x86_64")]
fn cpuid_brand_string() -> String {
    // Check if extended CPUID is supported
    let max_ext: u32;
    // SAFETY: CPUID leaf 0x80000000 returns max extended leaf in EAX.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 0x80000000",
            "cpuid",
            "mov {0:e}, eax",
            "pop rbx",
            out(reg) max_ext,
            out("eax") _,
        );
    }

    if max_ext < 0x80000004 {
        return String::from("Unknown CPU");
    }

    let mut buf = [0u8; 48];

    for (i, leaf) in [0x80000002u32, 0x80000003, 0x80000004].iter().enumerate() {
        let eax: u32;
        let ebx: u32;
        let ecx: u32;
        let edx: u32;
        // SAFETY: CPUID leaves 0x80000002-4 return the CPU brand string.
        unsafe {
            core::arch::asm!(
                "push rbx",
                "mov eax, {leaf:e}",
                "cpuid",
                "mov {0:e}, eax",
                "mov {1:e}, ebx",
                "mov {2:e}, ecx",
                "mov {3:e}, edx",
                "pop rbx",
                out(reg) eax,
                out(reg) ebx,
                out(reg) ecx,
                out(reg) edx,
                leaf = in(reg) *leaf,
                out("eax") _,
            );
        }
        let off = i * 16;
        buf[off..off + 4].copy_from_slice(&eax.to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&ebx.to_le_bytes());
        buf[off + 8..off + 12].copy_from_slice(&ecx.to_le_bytes());
        buf[off + 12..off + 16].copy_from_slice(&edx.to_le_bytes());
    }

    String::from_utf8_lossy(&buf)
        .trim_end_matches('\0')
        .trim()
        .into()
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

impl Default for ProcFs {
    fn default() -> Self {
        Self::new()
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

    fn sync(&self) -> Result<(), KernelError> {
        Ok(())
    }
}
