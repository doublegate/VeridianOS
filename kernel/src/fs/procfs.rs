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
    /// Virtual subdirectory (e.g., /proc/sys, /proc/sys/kernel, /proc/self)
    SubDir(String),
    /// /proc/self/exe: symlink to the calling process's executable
    SelfExe,
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
    /// procfs: `simple_statfs`, no counts.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, crate::error::KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::PROC_SUPER_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        match &self.node_type {
            ProcNodeType::Root | ProcNodeType::ProcessDir(_) | ProcNodeType::SubDir(_) => {
                NodeType::Directory
            }
            ProcNodeType::ProcessFile(_, _) | ProcNodeType::SystemFile(_) => NodeType::File,
            ProcNodeType::SelfExe => NodeType::Symlink,
        }
    }

    fn readlink(&self) -> Result<String, KernelError> {
        match &self.node_type {
            // It used to be an empty regular file: exec("/proc/self/exe"),
            // which BusyBox uses to run its own applets, read zero bytes.
            ProcNodeType::SelfExe => crate::process::current_process()
                .map(|p| p.exe_path.lock().clone())
                .filter(|path| !path.is_empty())
                .ok_or(KernelError::FsError(FsError::NotFound)),
            _ => Err(KernelError::FsError(FsError::NotASymlink)),
        }
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let content = match &self.node_type {
            ProcNodeType::SystemFile(name) => {
                match name.as_str() {
                    // The system's identity (ADR 0021).
                    // The system name is the reader's (Linux uname mode).
                    "version" => {
                        crate::utils::uname::proc_version(crate::utils::uname::current_sysname())
                    }
                    "ostype" => format!("{}\n", crate::utils::uname::current_sysname()),
                    "osrelease" => format!("{}\n", crate::utils::uname::RELEASE),
                    "sys/kernel/version" => format!("{}\n", crate::utils::uname::VERSION),
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
                        // Cached: the file page cache (ADR 0010). Frames on
                        // the allocator's per-CPU lists are free memory too.
                        let cached_kb = crate::mm::page_cache::cached_pages() * 4;
                        let available_kb = free_kb + stats.cached_frames * 4;
                        let buffers_kb = 0usize;
                        let slab_kb = 0usize;

                        format!(
                            "MemTotal:       {:>8} kB\n\
                             MemFree:        {:>8} kB\n\
                             MemAvailable:   {:>8} kB\n\
                             Buffers:        {:>8} kB\n\
                             Cached:         {:>8} kB\n\
                             Slab:           {:>8} kB\n\
                             MemUsed:        {:>8} kB\n\
                             CowShared:      {:>8} kB\n",
                            total_kb,
                            free_kb,
                            available_kb,
                            buffers_kb,
                            cached_kb,
                            slab_kb,
                            used_kb,
                            // Frames with more than one owner (each counted
                            // once): shared copy-on-write after fork, or a
                            // cached file page that processes map.
                            crate::mm::frame_refs::shared_frames() * 4,
                        )
                    }
                    "cpuinfo" => generate_cpuinfo(),
                    "loadavg" => generate_loadavg(),
                    // Qt/KDE compatibility files
                    "core_pattern" => String::from("core\n"),
                    "pid_max" => format!("{}\n", crate::process::PID_MAX),
                    "cap_last_cap" => {
                        format!("{}\n", crate::process::linux_caps::CAP_LAST_CAP)
                    }
                    "boot_id" => String::from("00000000-0000-0000-0000-000000000001\n"),
                    "self/maps" => crate::process::current_process()
                        .map(|p| generate_maps(p.pid.0))
                        .unwrap_or_default(),
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

                            let parent = process.parent().unwrap_or(crate::process::ProcessId(0));

                            // The sets are per thread; the process's are its
                            // main thread's, as on Linux.
                            let caps = process
                                .get_main_thread_id()
                                .and_then(|tid| process.get_thread(tid))
                                .map(|t| status_caps(&t.caps.lock()))
                                .unwrap_or_default();

                            format!(
                                "Name:\t{}\nPid:\t{}\nPPid:\t{}\nState:\t{}\n{}{}",
                                name,
                                pid,
                                parent.0,
                                state,
                                status_ids(&process.credentials()),
                                caps
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
        // EACCES, as Linux's read-only /proc files give (EROFS would say
        // the filesystem is mounted read-only).
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        let node_type = match &self.node_type {
            ProcNodeType::Root | ProcNodeType::ProcessDir(_) | ProcNodeType::SubDir(_) => {
                NodeType::Directory
            }
            ProcNodeType::SelfExe => NodeType::Symlink,
            _ => NodeType::File,
        };

        // As on Linux: everyone may walk the directories and read the
        // files; links (self/exe) are rwxrwxrwx.
        let permissions = Permissions::from_mode(match node_type {
            NodeType::Directory => 0o555,
            NodeType::Symlink => 0o777,
            _ => 0o444,
        });
        Ok(Metadata {
            node_type,
            size: 0,
            permissions,
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

                entries.push(DirEntry {
                    name: String::from("sys"),
                    node_type: NodeType::Directory,
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
            // What lookup finds in /proc/sys and below.
            ProcNodeType::SubDir(dir) if dir.starts_with("sys") => {
                let listed: &[(&str, NodeType)] = match dir.as_str() {
                    "sys" => &[("kernel", NodeType::Directory)],
                    "sys/kernel" => &[
                        ("cap_last_cap", NodeType::File),
                        ("core_pattern", NodeType::File),
                        ("osrelease", NodeType::File),
                        ("ostype", NodeType::File),
                        ("pid_max", NodeType::File),
                        ("random", NodeType::Directory),
                        ("version", NodeType::File),
                    ],
                    "sys/kernel/random" => &[("boot_id", NodeType::File)],
                    _ => return Err(KernelError::FsError(FsError::NotADirectory)),
                };
                for (name, node_type) in listed {
                    entries.push(DirEntry {
                        name: String::from(*name),
                        node_type: *node_type,
                        inode: 0,
                    });
                }
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
                    // Virtual subdirectories for Qt/KDE compatibility
                    "sys" => Ok(Arc::new(ProcNode {
                        node_type: ProcNodeType::SubDir(String::from("sys")),
                    }) as Arc<dyn VfsNode>),
                    "self" => Ok(Arc::new(ProcNode {
                        node_type: ProcNodeType::SubDir(String::from("self")),
                    }) as Arc<dyn VfsNode>),
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
            ProcNodeType::SubDir(dir_name) => {
                // Handle /proc/sys/*, /proc/self/*
                match dir_name.as_str() {
                    "sys" => match name {
                        "kernel" => Ok(Arc::new(ProcNode {
                            node_type: ProcNodeType::SubDir(String::from("sys/kernel")),
                        }) as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/kernel" => match name {
                        "core_pattern" => Ok(Arc::new(ProcNode::new_system_file(String::from(
                            "core_pattern",
                        ))) as Arc<dyn VfsNode>),
                        "cap_last_cap" | "pid_max" | "ostype" | "osrelease" => {
                            Ok(Arc::new(ProcNode::new_system_file(String::from(name)))
                                as Arc<dyn VfsNode>)
                        }
                        // Not /proc/version, which has the same file name.
                        "version" => Ok(Arc::new(ProcNode::new_system_file(String::from(
                            "sys/kernel/version",
                        ))) as Arc<dyn VfsNode>),
                        "random" => Ok(Arc::new(ProcNode {
                            node_type: ProcNodeType::SubDir(String::from("sys/kernel/random")),
                        }) as Arc<dyn VfsNode>),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "sys/kernel/random" => match name {
                        "boot_id" => {
                            Ok(Arc::new(ProcNode::new_system_file(String::from("boot_id")))
                                as Arc<dyn VfsNode>)
                        }
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    "self" => match name {
                        "exe" => Ok(Arc::new(ProcNode {
                            node_type: ProcNodeType::SelfExe,
                        }) as Arc<dyn VfsNode>),
                        // The caller's own files, as /proc/<its pid>/...
                        // (Linux's /proc/self is a link to that directory).
                        "status" | "cmdline" => crate::process::current_process()
                            .map(|p| {
                                Arc::new(ProcNode::new_process_file(p.pid.0, String::from(name)))
                                    as Arc<dyn VfsNode>
                            })
                            .ok_or(KernelError::FsError(FsError::NotFound)),
                        "maps" => Ok(
                            Arc::new(ProcNode::new_system_file(format!("self/{}", name)))
                                as Arc<dyn VfsNode>,
                        ),
                        _ => Err(KernelError::FsError(FsError::NotFound)),
                    },
                    _ => Err(KernelError::FsError(FsError::NotFound)),
                }
            }
            _ => Err(KernelError::FsError(FsError::NotADirectory)),
        }
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::PermissionDenied))
    }
}

/// Format one `/proc/<pid>/maps` line in the Linux layout:
/// `start-end perms offset dev inode [label]`.
///
/// Mappings are anonymous (offset 0, device 00:00, inode 0). Shared
/// mappings (`MAP_SHARED`, IPC shared regions, device memory) are marked `s`
/// and the rest `p`, as Linux does.
fn format_maps_line(
    start: u64,
    size: usize,
    flags: crate::mm::PageFlags,
    mapping_type: crate::mm::vas::MappingType,
) -> String {
    use crate::mm::{vas::MappingType, PageFlags};

    let end = start.saturating_add(size as u64);
    let write = if flags.contains(PageFlags::WRITABLE) {
        'w'
    } else {
        '-'
    };
    let exec = if flags.contains(PageFlags::NO_EXECUTE) {
        '-'
    } else {
        'x'
    };
    // Shared mappings used to be reported as private (review of the v0.26.0
    // stack, PR #7).
    let share = match mapping_type {
        MappingType::Shared | MappingType::SharedRegion | MappingType::Device => 's',
        _ => 'p',
    };
    let label = match mapping_type {
        MappingType::Stack => " [stack]",
        MappingType::Heap => " [heap]",
        _ => "",
    };
    format!(
        "{:08x}-{:08x} r{}{}{} 00000000 00:00 0{}\n",
        start, end, write, exec, share, label
    )
}

/// Generate `/proc/<pid>/maps` from the process's virtual address space.
///
/// Returns an empty string if the process does not exist or its address
/// space lock is contended (the lock is never waited on from procfs).
fn generate_maps(pid: u64) -> String {
    let Some(process) = crate::process::get_process(crate::process::ProcessId(pid)) else {
        return String::new();
    };
    let Some(space) = process.memory_space.try_lock() else {
        return String::new();
    };
    let Some(mappings) = space.mappings_ref().try_lock() else {
        return String::new();
    };
    let mut out = String::new();
    for mapping in mappings.values() {
        out.push_str(&format_maps_line(
            mapping.start.as_u64(),
            mapping.size,
            mapping.flags,
            mapping.mapping_type,
        ));
    }
    out
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
        format!(
            "processor\t: 0\narchitecture\t: riscv64\nmodel name\t: RISC-V Processor\nmmu\t\t: \
             {}\n",
            riscv_mmu_name(crate::mm::page_table::paging_levels())
        )
    }
}

/// The `Uid:`, `Gid:` and `Groups:` lines of /proc/[pid]/status, as Linux
/// prints them: real, effective, saved and filesystem IDs, tab-separated,
/// then the supplementary groups separated by spaces.
fn status_ids(c: &crate::process::creds::Credentials) -> String {
    let mut groups = String::new();
    for (i, g) in c.groups().iter().enumerate() {
        if i > 0 {
            groups.push(' ');
        }
        groups.push_str(&format!("{}", g));
    }
    format!(
        "Uid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nGroups:\t{}\n",
        c.ruid, c.euid, c.suid, c.fsuid, c.rgid, c.egid, c.sgid, c.fsgid, groups
    )
}

/// The capability lines of /proc/[pid]/status, as Linux prints them (ADR
/// 0020): each set as 16 hexadecimal digits.
fn status_caps(c: &crate::process::linux_caps::CapSets) -> String {
    format!(
        concat!(
            "CapInh:\t{:016x}\n",
            "CapPrm:\t{:016x}\n",
            "CapEff:\t{:016x}\n",
            "CapBnd:\t{:016x}\n",
            "CapAmb:\t{:016x}\n",
        ),
        c.inheritable.bits(),
        c.permitted.bits(),
        c.effective.bits(),
        c.bounding.bits(),
        c.ambient.bits()
    )
}

/// Linux's `mmu` field of a RISC-V /proc/cpuinfo: the paging mode in use
/// (ADR 0018), which user programs read to learn their address-space limit.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), allow(dead_code))]
fn riscv_mmu_name(levels: u8) -> &'static str {
    if levels == 3 {
        "sv39"
    } else {
        "sv48"
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
    let [one, five, fifteen] =
        crate::sched::loadavg::averages().map(crate::sched::loadavg::format_parts);
    format!(
        "{}.{:02} {}.{:02} {}.{:02} {}/{} {}\n",
        one.0, one.1, five.0, five.1, fifteen.0, fifteen.1, running, total, last_pid
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Everyone may walk /proc and read its files, as on Linux: directories
    /// r-xr-xr-x, files r--r--r--. (Directories had no execute bit, so a
    /// non-root process could open nothing below /proc.)
    #[test]
    fn proc_nodes_are_world_readable() {
        let dir = ProcNode::new_root().metadata().unwrap();
        assert_eq!(dir.permissions, Permissions::from_mode(0o555));
        let pid_dir = ProcNode::new_process_dir(1).metadata().unwrap();
        assert_eq!(pid_dir.permissions, Permissions::from_mode(0o555));
        let file = ProcNode::new_system_file(String::from("uptime"))
            .metadata()
            .unwrap();
        assert_eq!(file.permissions, Permissions::from_mode(0o444));
    }

    /// Status shows all four user and group IDs, filesystem ones last.
    #[test]
    fn status_shows_every_id() {
        let mut c = crate::process::creds::Credentials::new(0, 0);
        c.setgroups(&[10, 20], true).unwrap();
        c.setresgid(5, 6, 7, true).unwrap();
        c.setfsgid(8, true);
        c.setresuid(1, 2, 3, true).unwrap();
        assert_eq!(
            status_ids(&c),
            "Uid:\t1\t2\t3\t2\nGid:\t5\t6\t7\t8\nGroups:\t10 20\n"
        );
        let mut c = crate::process::creds::Credentials::new(0, 0);
        c.setfsuid(1000, true);
        assert_eq!(
            status_ids(&c),
            "Uid:\t0\t0\t0\t1000\nGid:\t0\t0\t0\t0\nGroups:\t\n"
        );
    }

    /// Status shows the five capability sets as Linux does: 16 hex digits,
    /// in the order inheritable, permitted, effective, bounding, ambient.
    #[test]
    fn status_shows_the_capability_sets() {
        use crate::process::linux_caps::{CapSets, LinuxCaps, CAP_KILL, CAP_NET_BIND_SERVICE};
        assert_eq!(
            status_caps(&CapSets::ROOT),
            concat!(
                "CapInh:\t0000000000000000\n",
                "CapPrm:\t000001ffffffffff\n",
                "CapEff:\t000001ffffffffff\n",
                "CapBnd:\t000001ffffffffff\n",
                "CapAmb:\t0000000000000000\n",
            )
        );
        let mut caps = CapSets::NONE;
        caps.inheritable = LinuxCaps::of(CAP_KILL);
        caps.ambient = LinuxCaps::of(CAP_NET_BIND_SERVICE);
        assert_eq!(
            status_caps(&caps),
            concat!(
                "CapInh:\t0000000000000020\n",
                "CapPrm:\t0000000000000000\n",
                "CapEff:\t0000000000000000\n",
                "CapBnd:\t000001ffffffffff\n",
                "CapAmb:\t0000000000000400\n",
            )
        );
    }

    /// /proc/sys/kernel/cap_last_cap is the highest capability number, which
    /// libcap and LTP read to size their sets.
    #[test]
    fn cap_last_cap_names_the_highest_capability() {
        let dir = ProcNode {
            node_type: ProcNodeType::SubDir(String::from("sys/kernel")),
        };
        let file = dir.lookup("cap_last_cap").unwrap();
        let mut buf = [0u8; 16];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"40\n");
    }

    /// /proc/sys/kernel/{ostype,osrelease,version} are uname's sysname,
    /// release and version, and /proc/version names all three (ADR 0021).
    #[test]
    fn kernel_identity_files_match_uname() {
        let dir = ProcNode {
            node_type: ProcNodeType::SubDir(String::from("sys/kernel")),
        };
        let read = |node: Arc<dyn VfsNode>| {
            let mut buf = [0u8; 160];
            let n = node.read(0, &mut buf).unwrap();
            String::from(core::str::from_utf8(&buf[..n]).unwrap())
        };
        assert_eq!(read(dir.lookup("ostype").unwrap()), "VeridianOS\n");
        assert_eq!(read(dir.lookup("osrelease").unwrap()), "7.3.0-veridian\n");
        assert_eq!(
            read(dir.lookup("version").unwrap()),
            concat!("#1 SMP VeridianOS ", env!("CARGO_PKG_VERSION"), "\n")
        );
        let proc_version = read(Arc::new(ProcNode::new_system_file(String::from("version"))));
        assert!(proc_version.starts_with("VeridianOS version 7.3.0-veridian ("));
    }

    /// /proc/sys and the directories below it list what lookup finds there,
    /// so a program reading the directory (getdents64) sees the files; it
    /// was ENOTDIR (PR #34 review). /proc lists sys.
    #[test]
    fn sys_directories_list_their_entries() {
        let names = |n: &ProcNode| -> Vec<String> {
            n.readdir()
                .unwrap()
                .into_iter()
                .filter(|e| e.name != "." && e.name != "..")
                .map(|e| e.name)
                .collect()
        };
        assert!(names(&ProcNode::new_root()).iter().any(|n| n == "sys"));
        for (dir, want) in [
            ("sys", &["kernel"][..]),
            (
                "sys/kernel",
                &[
                    "cap_last_cap",
                    "core_pattern",
                    "osrelease",
                    "ostype",
                    "pid_max",
                    "random",
                    "version",
                ][..],
            ),
            ("sys/kernel/random", &["boot_id"][..]),
        ] {
            let node = ProcNode {
                node_type: ProcNodeType::SubDir(String::from(dir)),
            };
            let mut got = names(&node);
            got.sort();
            assert_eq!(got, want, "/proc/{}", dir);
            for e in node
                .readdir()
                .unwrap()
                .iter()
                .filter(|e| !e.name.starts_with('.'))
            {
                let found = node.lookup(&e.name).unwrap();
                assert_eq!(found.node_type(), e.node_type, "/proc/{}/{}", dir, e.name);
            }
        }
    }

    /// /proc/sys/kernel/pid_max is Linux's 64-bit limit (PID_MAX_LIMIT);
    /// VeridianOS ids do not wrap at it (docs/KNOWN-LIMITATIONS.md).
    #[test]
    fn pid_max_is_the_linux_64_bit_limit() {
        let dir = ProcNode {
            node_type: ProcNodeType::SubDir(String::from("sys/kernel")),
        };
        let file = dir.lookup("pid_max").unwrap();
        let mut buf = [0u8; 16];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"4194304\n");
    }

    /// The RISC-V `mmu` field names the mode, as Linux prints it.
    #[test]
    fn riscv_mmu_field_names_the_paging_mode() {
        assert_eq!(riscv_mmu_name(3), "sv39");
        assert_eq!(riscv_mmu_name(4), "sv48");
    }
    use crate::mm::{vas::MappingType, PageFlags};

    #[test]
    fn maps_line_matches_linux_format() {
        let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::NO_EXECUTE;
        assert_eq!(
            format_maps_line(0x40_0000, 0x2000, flags, MappingType::Data),
            "00400000-00402000 r--p 00000000 00:00 0\n"
        );
    }

    #[test]
    fn maps_line_reports_write_exec_and_labels() {
        let rw = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITABLE | PageFlags::NO_EXECUTE;
        assert_eq!(
            format_maps_line(0x7fff_f000_0000, 0x1000, rw, MappingType::Stack),
            "7ffff0000000-7ffff0001000 rw-p 00000000 00:00 0 [stack]\n"
        );
        assert_eq!(
            format_maps_line(0x1000, 0x1000, rw, MappingType::Heap),
            "00001000-00002000 rw-p 00000000 00:00 0 [heap]\n"
        );
        let rx = PageFlags::PRESENT | PageFlags::USER;
        assert_eq!(
            format_maps_line(0x40_1000, 0x1000, rx, MappingType::Code),
            "00401000-00402000 r-xp 00000000 00:00 0\n"
        );
        // Shared mappings are `s`, not `p`.
        for ty in [MappingType::Shared, MappingType::SharedRegion] {
            assert_eq!(
                format_maps_line(0x4000_0000, 0x1000, rw, ty),
                "40000000-40001000 rw-s 00000000 00:00 0\n"
            );
        }
    }
}
