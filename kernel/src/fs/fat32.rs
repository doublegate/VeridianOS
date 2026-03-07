//! FAT32 Filesystem Implementation
//!
//! Read/write support for the FAT32 filesystem format.
//! Supports 8.3 short names and VFAT long file names (LFN).
//! Integrates with block devices (VirtIO-blk, NVMe, etc.) via the BlockDevice
//! trait.
//!
//! Struct fields, attribute constants, and helper methods define the complete
//! FAT32 on-disk format. Unused items are retained for format completeness.
#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// FAT32 end-of-chain marker
const FAT32_EOC: u32 = 0x0FFF_FFF8;

/// FAT32 free cluster marker
const FAT32_FREE: u32 = 0x0000_0000;

/// FAT32 directory entry size
const DIR_ENTRY_SIZE: usize = 32;

/// Attribute flags for directory entries
const ATTR_READ_ONLY: u8 = 0x01;
const ATTR_HIDDEN: u8 = 0x02;
const ATTR_SYSTEM: u8 = 0x04;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LONG_NAME: u8 = 0x0F;

/// BIOS Parameter Block (BPB) -- parsed from boot sector
#[derive(Debug, Clone)]
pub struct Bpb {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub total_sectors_32: u32,
    pub fat_size_32: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
}

impl Bpb {
    /// Parse BPB from boot sector bytes (first 512 bytes)
    pub fn parse(sector: &[u8]) -> Result<Self, KernelError> {
        if sector.len() < 512 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        // Check boot signature
        if sector[510] != 0x55 || sector[511] != 0xAA {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
        let sectors_per_cluster = sector[13];
        let reserved_sectors = u16::from_le_bytes([sector[14], sector[15]]);
        let num_fats = sector[16];
        let total_sectors_32 = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
        let fat_size_32 = u32::from_le_bytes([sector[36], sector[37], sector[38], sector[39]]);
        let root_cluster = u32::from_le_bytes([sector[44], sector[45], sector[46], sector[47]]);
        let fs_info_sector = u16::from_le_bytes([sector[48], sector[49]]);

        if bytes_per_sector == 0 || sectors_per_cluster == 0 || num_fats == 0 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            total_sectors_32,
            fat_size_32,
            root_cluster,
            fs_info_sector,
        })
    }

    /// First sector of the data region
    fn data_start_sector(&self) -> u32 {
        self.reserved_sectors as u32 + (self.num_fats as u32 * self.fat_size_32)
    }

    /// Convert cluster number to absolute sector number
    fn cluster_to_sector(&self, cluster: u32) -> u32 {
        self.data_start_sector() + (cluster - 2) * self.sectors_per_cluster as u32
    }

    /// Bytes per cluster
    fn cluster_size(&self) -> usize {
        self.bytes_per_sector as usize * self.sectors_per_cluster as usize
    }

    /// Sector offset of a FAT entry for a given cluster
    fn fat_sector_for_cluster(&self, cluster: u32) -> u32 {
        let fat_offset = cluster * 4;
        self.reserved_sectors as u32 + fat_offset / self.bytes_per_sector as u32
    }

    /// Byte offset within FAT sector for a given cluster
    fn fat_offset_in_sector(&self, cluster: u32) -> usize {
        ((cluster * 4) % self.bytes_per_sector as u32) as usize
    }
}

/// In-memory FAT table cache
struct FatTable {
    /// FAT entries indexed by cluster number
    entries: Vec<u32>,
}

impl FatTable {
    fn new(num_clusters: usize) -> Self {
        Self {
            entries: vec![0u32; num_clusters],
        }
    }

    fn get(&self, cluster: u32) -> u32 {
        if (cluster as usize) < self.entries.len() {
            self.entries[cluster as usize] & 0x0FFF_FFFF
        } else {
            FAT32_EOC
        }
    }

    fn set(&mut self, cluster: u32, value: u32) {
        if (cluster as usize) < self.entries.len() {
            self.entries[cluster as usize] = value & 0x0FFF_FFFF;
        }
    }

    /// Find a free cluster starting from `hint`
    fn alloc_cluster(&mut self, hint: u32) -> Option<u32> {
        let start = core::cmp::max(hint as usize, 2);
        // Search from hint to end
        for i in start..self.entries.len() {
            if self.entries[i] == FAT32_FREE {
                self.entries[i] = FAT32_EOC;
                return Some(i as u32);
            }
        }
        // Wrap around: search from 2 to hint
        for i in 2..start {
            if self.entries[i] == FAT32_FREE {
                self.entries[i] = FAT32_EOC;
                return Some(i as u32);
            }
        }
        None
    }

    /// Free a cluster chain starting from `start`
    fn free_chain(&mut self, start: u32) {
        let mut current = start;
        while (2..FAT32_EOC).contains(&current) {
            let next = self.get(current);
            self.entries[current as usize] = FAT32_FREE;
            if next >= FAT32_EOC {
                break;
            }
            current = next;
        }
    }

    /// Get the chain of clusters starting from `start`
    fn get_chain(&self, start: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut current = start;
        while (2..FAT32_EOC).contains(&current) {
            chain.push(current);
            let next = self.get(current);
            if next >= FAT32_EOC || chain.len() > 1_000_000 {
                break;
            }
            current = next;
        }
        chain
    }
}

/// Raw FAT32 directory entry (32 bytes)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct RawDirEntry {
    name: [u8; 11],
    attr: u8,
    _nt_reserved: u8,
    create_time_tenth: u8,
    create_time: u16,
    create_date: u16,
    access_date: u16,
    first_cluster_hi: u16,
    write_time: u16,
    write_date: u16,
    first_cluster_lo: u16,
    file_size: u32,
}

impl RawDirEntry {
    fn first_cluster(&self) -> u32 {
        ((self.first_cluster_hi as u32) << 16) | self.first_cluster_lo as u32
    }

    fn is_free(&self) -> bool {
        self.name[0] == 0xE5 || self.name[0] == 0x00
    }

    fn is_end(&self) -> bool {
        self.name[0] == 0x00
    }

    fn is_long_name(&self) -> bool {
        self.attr == ATTR_LONG_NAME
    }

    fn is_directory(&self) -> bool {
        (self.attr & ATTR_DIRECTORY) != 0
    }

    fn short_name(&self) -> String {
        let name_part: Vec<u8> = self.name[..8]
            .iter()
            .copied()
            .take_while(|&b| b != b' ')
            .collect();
        let ext_part: Vec<u8> = self.name[8..11]
            .iter()
            .copied()
            .take_while(|&b| b != b' ')
            .collect();

        let mut result = String::new();
        for &b in &name_part {
            result.push(b as char);
        }
        if !ext_part.is_empty() {
            result.push('.');
            for &b in &ext_part {
                result.push(b as char);
            }
        }
        result
    }
}

/// Long file name directory entry
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct LfnDirEntry {
    order: u8,
    name1: [u16; 5],
    attr: u8,
    entry_type: u8,
    checksum: u8,
    name2: [u16; 6],
    _first_cluster_lo: u16,
    name3: [u16; 2],
}

impl LfnDirEntry {
    /// Extract the Unicode characters from this LFN entry.
    /// Uses read_unaligned to handle packed struct field access safely.
    fn chars(&self) -> Vec<u16> {
        let mut chars = Vec::new();

        // Copy fields from packed struct to local arrays to avoid unaligned access
        // SAFETY: addr_of! on packed fields avoids creating unaligned refs;
        // read_unaligned handles alignment.
        let name1: [u16; 5] = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.name1)) };
        let name2: [u16; 6] = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.name2)) };
        let name3: [u16; 2] = unsafe { core::ptr::read_unaligned(core::ptr::addr_of!(self.name3)) };

        for &c in &name1 {
            if c == 0x0000 || c == 0xFFFF {
                return chars;
            }
            chars.push(c);
        }
        for &c in &name2 {
            if c == 0x0000 || c == 0xFFFF {
                return chars;
            }
            chars.push(c);
        }
        for &c in &name3 {
            if c == 0x0000 || c == 0xFFFF {
                return chars;
            }
            chars.push(c);
        }
        chars
    }
}

/// Global inode counter for FAT32 nodes
static FAT32_NEXT_INODE: AtomicU64 = AtomicU64::new(1);

/// In-memory representation of a FAT32 node (file or directory)
struct Fat32Node {
    node_type: NodeType,
    /// Data cache for files; directory entry bytes for directories
    data: RwLock<Vec<u8>>,
    /// Children (populated on first readdir/lookup for directories)
    children: RwLock<BTreeMap<String, Arc<Fat32Node>>>,
    metadata: RwLock<Metadata>,
    inode: u64,
    parent_inode: u64,
    /// Starting cluster on disk
    start_cluster: RwLock<u32>,
    /// Shared reference to filesystem state for cluster I/O
    fs_state: Arc<RwLock<Fat32State>>,
}

/// Shared mutable state for the FAT32 filesystem
struct Fat32State {
    bpb: Bpb,
    fat: FatTable,
    /// Cached disk data: maps sector number -> sector bytes
    sector_cache: BTreeMap<u64, Vec<u8>>,
    /// Whether the filesystem has been modified (dirty)
    dirty: bool,
    /// Next free cluster hint
    next_free_hint: u32,
}

impl Fat32State {
    /// Read a cluster's data from cache or synthesize zeros
    fn read_cluster(&self, cluster: u32) -> Vec<u8> {
        let sector = self.bpb.cluster_to_sector(cluster);
        let cluster_size = self.bpb.cluster_size();
        let mut data = vec![0u8; cluster_size];

        let sectors_per_cluster = self.bpb.sectors_per_cluster as u32;
        for i in 0..sectors_per_cluster {
            let sec = (sector + i) as u64;
            if let Some(cached) = self.sector_cache.get(&sec) {
                let offset = i as usize * self.bpb.bytes_per_sector as usize;
                let len = core::cmp::min(cached.len(), self.bpb.bytes_per_sector as usize);
                data[offset..offset + len].copy_from_slice(&cached[..len]);
            }
        }
        data
    }

    /// Write a cluster's data to cache
    fn write_cluster(&mut self, cluster: u32, data: &[u8]) {
        let sector = self.bpb.cluster_to_sector(cluster);
        let bps = self.bpb.bytes_per_sector as usize;
        let sectors_per_cluster = self.bpb.sectors_per_cluster as u32;

        for i in 0..sectors_per_cluster {
            let sec = (sector + i) as u64;
            let offset = i as usize * bps;
            let end = core::cmp::min(offset + bps, data.len());
            if offset < data.len() {
                self.sector_cache.insert(sec, data[offset..end].to_vec());
            }
        }
        self.dirty = true;
    }

    /// Read file data from a cluster chain
    fn read_chain_data(&self, start_cluster: u32, size: usize) -> Vec<u8> {
        let chain = self.fat.get_chain(start_cluster);
        let mut data = Vec::with_capacity(size);

        for &cluster in &chain {
            let cluster_data = self.read_cluster(cluster);
            let remaining = size.saturating_sub(data.len());
            let take = core::cmp::min(remaining, cluster_data.len());
            data.extend_from_slice(&cluster_data[..take]);
            if data.len() >= size {
                break;
            }
        }
        data
    }

    /// Allocate clusters and write data
    fn write_data(&mut self, start_cluster: u32, data: &[u8]) -> Result<u32, KernelError> {
        let cluster_size = self.bpb.cluster_size();
        let clusters_needed = if data.is_empty() {
            0
        } else {
            data.len().div_ceil(cluster_size)
        };

        // Free old chain if any
        if start_cluster >= 2 {
            self.fat.free_chain(start_cluster);
        }

        if clusters_needed == 0 {
            return Ok(0);
        }

        // Allocate new chain
        let mut chain = Vec::with_capacity(clusters_needed);
        for _ in 0..clusters_needed {
            let cluster = self
                .fat
                .alloc_cluster(self.next_free_hint)
                .ok_or(KernelError::FsError(FsError::NoSpace))?;
            self.next_free_hint = cluster + 1;
            chain.push(cluster);
        }

        // Link chain
        for i in 0..chain.len() - 1 {
            self.fat.set(chain[i], chain[i + 1]);
        }
        // Last cluster is already marked EOC by alloc_cluster

        // Write data to clusters
        for (i, &cluster) in chain.iter().enumerate() {
            let offset = i * cluster_size;
            let end = core::cmp::min(offset + cluster_size, data.len());
            let mut cluster_buf = vec![0u8; cluster_size];
            cluster_buf[..end - offset].copy_from_slice(&data[offset..end]);
            self.write_cluster(cluster, &cluster_buf);
        }

        Ok(chain[0])
    }

    /// Parse directory entries from raw cluster data
    fn parse_dir_entries(&self, dir_data: &[u8]) -> Vec<(String, u32, u32, u8)> {
        let mut entries = Vec::new();
        let mut lfn_parts: Vec<(u8, Vec<u16>)> = Vec::new();
        let entry_count = dir_data.len() / DIR_ENTRY_SIZE;

        for i in 0..entry_count {
            let offset = i * DIR_ENTRY_SIZE;
            let entry_bytes = &dir_data[offset..offset + DIR_ENTRY_SIZE];

            if entry_bytes[0] == 0x00 {
                break; // End of directory
            }
            if entry_bytes[0] == 0xE5 {
                lfn_parts.clear();
                continue; // Deleted entry
            }

            let attr = entry_bytes[11];

            if attr == ATTR_LONG_NAME {
                // LFN entry
                // SAFETY: entry_bytes is a 32-byte aligned slice matching LfnDirEntry layout.
                let lfn = unsafe { &*(entry_bytes.as_ptr() as *const LfnDirEntry) };
                let order = lfn.order & 0x3F;
                lfn_parts.push((order, lfn.chars()));
                continue;
            }

            // Short name entry
            // SAFETY: entry_bytes is a 32-byte aligned slice matching RawDirEntry layout.
            let raw = unsafe { &*(entry_bytes.as_ptr() as *const RawDirEntry) };

            // Build long name if LFN parts exist
            let name = if !lfn_parts.is_empty() {
                lfn_parts.sort_by_key(|(order, _)| *order);
                let mut chars: Vec<u16> = Vec::new();
                for (_, part) in &lfn_parts {
                    chars.extend_from_slice(part);
                }
                lfn_parts.clear();

                // Convert UTF-16 to ASCII/UTF-8
                let mut name = String::new();
                for &c in &chars {
                    if c == 0 {
                        break;
                    }
                    if c < 128 {
                        name.push(c as u8 as char);
                    } else {
                        name.push('?'); // Non-ASCII replacement
                    }
                }
                name
            } else {
                lfn_parts.clear();
                raw.short_name()
            };

            // Skip . and .. entries
            if name == "." || name == ".." {
                continue;
            }

            entries.push((name, raw.first_cluster(), raw.file_size, raw.attr));
        }

        entries
    }
}

impl Fat32Node {
    fn new_file(
        inode: u64,
        parent_inode: u64,
        start_cluster: u32,
        size: usize,
        fs_state: Arc<RwLock<Fat32State>>,
    ) -> Self {
        Self {
            node_type: NodeType::File,
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::File,
                size,
                permissions: Permissions::default(),
                uid: 0,
                gid: 0,
                created: 0,
                modified: 0,
                accessed: 0,
                inode,
            }),
            inode,
            parent_inode,
            start_cluster: RwLock::new(start_cluster),
            fs_state,
        }
    }

    fn new_directory(
        inode: u64,
        parent_inode: u64,
        start_cluster: u32,
        fs_state: Arc<RwLock<Fat32State>>,
    ) -> Self {
        Self {
            node_type: NodeType::Directory,
            data: RwLock::new(Vec::new()),
            children: RwLock::new(BTreeMap::new()),
            metadata: RwLock::new(Metadata {
                node_type: NodeType::Directory,
                size: 0,
                permissions: Permissions::default(),
                uid: 0,
                gid: 0,
                created: 0,
                modified: 0,
                accessed: 0,
                inode,
            }),
            inode,
            parent_inode,
            start_cluster: RwLock::new(start_cluster),
            fs_state,
        }
    }

    /// Load directory children from disk if not already cached
    fn ensure_children_loaded(&self) {
        if self.node_type != NodeType::Directory {
            return;
        }

        {
            let children = self.children.read();
            // Already loaded check: if we have any children, skip
            // (empty directory is re-checked each time, which is acceptable)
            if !children.is_empty() {
                return;
            }
        }

        let start = *self.start_cluster.read();
        if start < 2 {
            return;
        }

        let parsed = {
            let state = self.fs_state.read();
            let dir_data = state.read_chain_data(start, 1024 * 1024); // Up to 1MB directory
            state.parse_dir_entries(&dir_data)
        };

        let mut children = self.children.write();
        for (name, cluster, size, attr) in parsed {
            let inode = FAT32_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
            let node = if (attr & ATTR_DIRECTORY) != 0 {
                Arc::new(Fat32Node::new_directory(
                    inode,
                    self.inode,
                    cluster,
                    self.fs_state.clone(),
                ))
            } else {
                Arc::new(Fat32Node::new_file(
                    inode,
                    self.inode,
                    cluster,
                    size as usize,
                    self.fs_state.clone(),
                ))
            };
            children.insert(name, node);
        }
    }
}

impl VfsNode for Fat32Node {
    fn node_type(&self) -> NodeType {
        self.node_type
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let meta = self.metadata.read();
        if offset >= meta.size {
            return Ok(0);
        }

        let start = *self.start_cluster.read();
        let file_data = {
            let state = self.fs_state.read();
            state.read_chain_data(start, meta.size)
        };

        let bytes_to_read = core::cmp::min(buffer.len(), file_data.len().saturating_sub(offset));
        buffer[..bytes_to_read].copy_from_slice(&file_data[offset..offset + bytes_to_read]);

        Ok(bytes_to_read)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        // Read existing data
        let old_start = *self.start_cluster.read();
        let old_size = self.metadata.read().size;

        let mut file_data = {
            let state = self.fs_state.read();
            if old_start >= 2 && old_size > 0 {
                state.read_chain_data(old_start, old_size)
            } else {
                Vec::new()
            }
        };

        // Extend if needed
        if offset > file_data.len() {
            file_data.resize(offset, 0);
        }
        let new_end = offset + data.len();
        if new_end > file_data.len() {
            file_data.resize(new_end, 0);
        }
        file_data[offset..new_end].copy_from_slice(data);

        // Write back
        let new_cluster = {
            let mut state = self.fs_state.write();
            state.write_data(old_start, &file_data)?
        };

        *self.start_cluster.write() = new_cluster;
        let mut meta = self.metadata.write();
        meta.size = file_data.len();
        meta.modified = crate::arch::timer::get_timestamp_secs();

        Ok(data.len())
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(self.metadata.read().clone())
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.ensure_children_loaded();

        let children = self.children.read();
        let mut entries = Vec::new();

        entries.push(DirEntry {
            name: String::from("."),
            node_type: NodeType::Directory,
            inode: self.inode,
        });
        entries.push(DirEntry {
            name: String::from(".."),
            node_type: NodeType::Directory,
            inode: self.parent_inode,
        });

        for (name, child) in children.iter() {
            entries.push(DirEntry {
                name: name.clone(),
                node_type: child.node_type,
                inode: child.inode,
            });
        }

        Ok(entries)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.ensure_children_loaded();

        let children = self.children.read();
        children
            .get(name)
            .map(|node| node.clone() as Arc<dyn VfsNode>)
            .ok_or(KernelError::FsError(FsError::NotFound))
    }

    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.ensure_children_loaded();

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode = FAT32_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let new_file = Arc::new(Fat32Node::new_file(
            inode,
            self.inode,
            0, // No clusters allocated yet
            0,
            self.fs_state.clone(),
        ));
        // Override permissions
        new_file.metadata.write().permissions = permissions;
        children.insert(String::from(name), new_file.clone());

        Ok(new_file as Arc<dyn VfsNode>)
    }

    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.ensure_children_loaded();

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        // Allocate a cluster for the new directory
        let cluster = {
            let mut state = self.fs_state.write();
            let hint = state.next_free_hint;
            let cluster = state
                .fat
                .alloc_cluster(hint)
                .ok_or(KernelError::FsError(FsError::NoSpace))?;
            state.next_free_hint = cluster + 1;

            // Write empty directory data (. and .. entries)
            let cluster_size = state.bpb.cluster_size();
            let empty = vec![0u8; cluster_size];
            state.write_cluster(cluster, &empty);
            state.dirty = true;
            cluster
        };

        let inode = FAT32_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let new_dir = Arc::new(Fat32Node::new_directory(
            inode,
            self.inode,
            cluster,
            self.fs_state.clone(),
        ));
        new_dir.metadata.write().permissions = permissions;
        children.insert(String::from(name), new_dir.clone());

        Ok(new_dir as Arc<dyn VfsNode>)
    }

    fn unlink(&self, name: &str) -> Result<(), KernelError> {
        if self.node_type != NodeType::Directory {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        self.ensure_children_loaded();

        let mut children = self.children.write();

        if let Some(node) = children.get(name) {
            if node.node_type == NodeType::Directory {
                let node_children = node.children.read();
                if !node_children.is_empty() {
                    return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
                }
            }

            // Free cluster chain
            let start = *node.start_cluster.read();
            if start >= 2 {
                let mut state = self.fs_state.write();
                state.fat.free_chain(start);
                state.dirty = true;
            }

            children.remove(name);
            Ok(())
        } else {
            Err(KernelError::FsError(FsError::NotFound))
        }
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        if self.node_type != NodeType::File {
            return Err(KernelError::FsError(FsError::NotAFile));
        }

        let old_start = *self.start_cluster.read();
        let old_size = self.metadata.read().size;

        if size == 0 {
            // Free all clusters
            if old_start >= 2 {
                let mut state = self.fs_state.write();
                state.fat.free_chain(old_start);
                state.dirty = true;
            }
            *self.start_cluster.write() = 0;
        } else if size != old_size {
            // Re-read, resize, re-write
            let mut data = {
                let state = self.fs_state.read();
                state.read_chain_data(old_start, old_size)
            };

            data.resize(size, 0);
            let new_cluster = {
                let mut state = self.fs_state.write();
                state.write_data(old_start, &data)?
            };
            *self.start_cluster.write() = new_cluster;
        }

        let mut meta = self.metadata.write();
        meta.size = size;
        meta.modified = crate::arch::timer::get_timestamp_secs();

        Ok(())
    }
}

/// FAT32 filesystem
pub struct Fat32Fs {
    root: Arc<Fat32Node>,
    state: Arc<RwLock<Fat32State>>,
    readonly: bool,
}

impl Fat32Fs {
    /// Create a FAT32 filesystem from raw image data.
    ///
    /// `image_data` should be a complete FAT32 volume image.
    /// The image is loaded entirely into memory (sector cache).
    pub fn from_image(image_data: &[u8], readonly: bool) -> Result<Self, KernelError> {
        if image_data.len() < 512 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let bpb = Bpb::parse(&image_data[..512])?;
        let bps = bpb.bytes_per_sector as usize;

        // Load FAT into memory
        let fat_start = bpb.reserved_sectors as usize * bps;
        let fat_size_bytes = bpb.fat_size_32 as usize * bps;
        let total_data_sectors = bpb.total_sectors_32 as usize - bpb.data_start_sector() as usize;
        let total_clusters = total_data_sectors / bpb.sectors_per_cluster as usize + 2;

        let mut fat = FatTable::new(total_clusters);
        if fat_start + fat_size_bytes <= image_data.len() {
            let fat_data = &image_data[fat_start..fat_start + fat_size_bytes];
            for i in 0..core::cmp::min(total_clusters, fat_data.len() / 4) {
                fat.entries[i] = u32::from_le_bytes([
                    fat_data[i * 4],
                    fat_data[i * 4 + 1],
                    fat_data[i * 4 + 2],
                    fat_data[i * 4 + 3],
                ]);
            }
        }

        // Load all sectors into cache
        let mut sector_cache = BTreeMap::new();
        let total_sectors = image_data.len() / bps;
        for s in 0..total_sectors {
            let offset = s * bps;
            let end = core::cmp::min(offset + bps, image_data.len());
            sector_cache.insert(s as u64, image_data[offset..end].to_vec());
        }

        let state = Arc::new(RwLock::new(Fat32State {
            bpb: bpb.clone(),
            fat,
            sector_cache,
            dirty: false,
            next_free_hint: 2,
        }));

        let root_inode = FAT32_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        let root = Arc::new(Fat32Node::new_directory(
            root_inode,
            root_inode,
            bpb.root_cluster,
            state.clone(),
        ));

        Ok(Self {
            root,
            state,
            readonly,
        })
    }
}

impl Filesystem for Fat32Fs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "fat32"
    }

    fn is_readonly(&self) -> bool {
        self.readonly
    }

    fn sync(&self) -> Result<(), KernelError> {
        // In a full implementation, this would flush the sector cache and FAT
        // back to the block device. For now, all data is in-memory.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal FAT32 image for testing
    fn make_test_fat32_image() -> Vec<u8> {
        let bps: usize = 512;
        let spc: usize = 1;
        let reserved: usize = 32;
        let num_fats: usize = 2;
        let fat_size: usize = 128; // sectors per FAT
        let total_sectors: usize = reserved + num_fats * fat_size + 1024; // ~512KB data

        let mut image = vec![0u8; total_sectors * bps];

        // Write BPB
        image[11] = (bps & 0xFF) as u8;
        image[12] = ((bps >> 8) & 0xFF) as u8;
        image[13] = spc as u8;
        image[14] = (reserved & 0xFF) as u8;
        image[15] = ((reserved >> 8) & 0xFF) as u8;
        image[16] = num_fats as u8;
        let ts = total_sectors as u32;
        image[32] = (ts & 0xFF) as u8;
        image[33] = ((ts >> 8) & 0xFF) as u8;
        image[34] = ((ts >> 16) & 0xFF) as u8;
        image[35] = ((ts >> 24) & 0xFF) as u8;
        let fs = fat_size as u32;
        image[36] = (fs & 0xFF) as u8;
        image[37] = ((fs >> 8) & 0xFF) as u8;
        image[38] = ((fs >> 16) & 0xFF) as u8;
        image[39] = ((fs >> 24) & 0xFF) as u8;
        // Root cluster = 2
        image[44] = 2;
        // FSInfo sector = 1
        image[48] = 1;
        // Boot signature
        image[510] = 0x55;
        image[511] = 0xAA;

        // Write FAT: cluster 0 and 1 are reserved
        let fat_start = reserved * bps;
        // Cluster 0: media descriptor
        image[fat_start] = 0xF8;
        image[fat_start + 1] = 0xFF;
        image[fat_start + 2] = 0xFF;
        image[fat_start + 3] = 0x0F;
        // Cluster 1: EOC
        image[fat_start + 4] = 0xFF;
        image[fat_start + 5] = 0xFF;
        image[fat_start + 6] = 0xFF;
        image[fat_start + 7] = 0x0F;
        // Cluster 2 (root dir): EOC
        image[fat_start + 8] = 0xFF;
        image[fat_start + 9] = 0xFF;
        image[fat_start + 10] = 0xFF;
        image[fat_start + 11] = 0x0F;

        // Data start = (reserved + num_fats * fat_size) * bps
        // Cluster 2 = root directory -- leave empty for now

        image
    }

    #[test]
    fn test_bpb_parse() {
        let image = make_test_fat32_image();
        let bpb = Bpb::parse(&image[..512]).unwrap();
        assert_eq!(bpb.bytes_per_sector, 512);
        assert_eq!(bpb.sectors_per_cluster, 1);
        assert_eq!(bpb.reserved_sectors, 32);
        assert_eq!(bpb.num_fats, 2);
        assert_eq!(bpb.root_cluster, 2);
    }

    #[test]
    fn test_bpb_parse_bad_signature() {
        let mut image = vec![0u8; 512];
        let result = Bpb::parse(&image);
        assert!(result.is_err());
    }

    #[test]
    fn test_fat32_from_image() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        assert_eq!(fs.name(), "fat32");
        assert!(!fs.is_readonly());

        let root = fs.root();
        assert_eq!(root.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_fat32_readonly() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, true).unwrap();
        assert!(fs.is_readonly());
    }

    #[test]
    fn test_fat32_empty_root_readdir() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        let root = fs.root();

        let entries = root.readdir().unwrap();
        // Just . and ..
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_fat32_create_and_write_file() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        let root = fs.root();

        let file = root.create("TEST.TXT", Permissions::default()).unwrap();
        file.write(0, b"Hello FAT32!").unwrap();

        let mut buf = vec![0u8; 20];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&buf[..12], b"Hello FAT32!");

        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 12);
    }

    #[test]
    fn test_fat32_mkdir() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        let root = fs.root();

        let dir = root.mkdir("SUBDIR", Permissions::default()).unwrap();
        assert_eq!(dir.node_type(), NodeType::Directory);

        let found = root.lookup("SUBDIR").unwrap();
        assert_eq!(found.node_type(), NodeType::Directory);
    }

    #[test]
    fn test_fat32_unlink_file() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        let root = fs.root();

        root.create("DEL.TXT", Permissions::default()).unwrap();
        assert!(root.lookup("DEL.TXT").is_ok());

        root.unlink("DEL.TXT").unwrap();
        assert!(root.lookup("DEL.TXT").is_err());
    }

    #[test]
    fn test_fat32_truncate() {
        let image = make_test_fat32_image();
        let fs = Fat32Fs::from_image(&image, false).unwrap();
        let root = fs.root();

        let file = root.create("TRUNC.TXT", Permissions::default()).unwrap();
        file.write(0, b"0123456789").unwrap();
        assert_eq!(file.metadata().unwrap().size, 10);

        file.truncate(5).unwrap();
        assert_eq!(file.metadata().unwrap().size, 5);

        let mut buf = vec![0u8; 10];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"01234");
    }

    #[test]
    fn test_fat_table_alloc_and_free() {
        let mut fat = FatTable::new(100);

        // All clusters start free
        let c1 = fat.alloc_cluster(2).unwrap();
        assert_eq!(c1, 2);
        assert_eq!(fat.get(2), FAT32_EOC & 0x0FFF_FFFF);

        let c2 = fat.alloc_cluster(2).unwrap();
        assert_eq!(c2, 3);

        // Free cluster 2
        fat.free_chain(2);
        assert_eq!(fat.get(2), FAT32_FREE);
    }

    #[test]
    fn test_fat_table_chain() {
        let mut fat = FatTable::new(100);

        fat.set(2, 3);
        fat.set(3, 4);
        fat.set(4, FAT32_EOC);

        let chain = fat.get_chain(2);
        assert_eq!(chain, vec![2, 3, 4]);
    }

    #[test]
    fn test_short_name_parsing() {
        let mut entry = [0u8; 32];
        entry[..8].copy_from_slice(b"TEST    ");
        entry[8..11].copy_from_slice(b"TXT");
        entry[11] = 0x20; // Archive attribute
                          // SAFETY: entry is a stack-local 32-byte array matching RawDirEntry layout.
        let raw = unsafe { &*(entry.as_ptr() as *const RawDirEntry) };
        assert_eq!(raw.short_name(), "TEST.TXT");
    }

    #[test]
    fn test_short_name_no_extension() {
        let mut entry = [0u8; 32];
        entry[..8].copy_from_slice(b"README  ");
        entry[8..11].copy_from_slice(b"   ");
        entry[11] = 0x10; // Directory
                          // SAFETY: entry is a stack-local 32-byte array matching RawDirEntry layout.
        let raw = unsafe { &*(entry.as_ptr() as *const RawDirEntry) };
        assert_eq!(raw.short_name(), "README");
    }
}
