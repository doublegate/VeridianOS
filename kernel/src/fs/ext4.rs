//! ext4 Read-Only Filesystem Implementation
//!
//! Supports reading files, directories, symlinks from ext4 formatted volumes.
//! Handles both extent-based and legacy block-map inodes.
//! Journal replay is not implemented (read-only mount).
//!
//! Struct fields and constants define the on-disk ext4 format per the kernel
//! documentation. Unused fields/constants are retained for format completeness
//! and future write support.
#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// ext4 superblock magic number
const EXT4_SUPER_MAGIC: u16 = 0xEF53;

/// Superblock offset from start of volume
const SUPERBLOCK_OFFSET: usize = 1024;

/// Inode flags
const EXT4_EXTENTS_FL: u32 = 0x0008_0000;

/// Inode file type flags (from i_mode)
const S_IFMT: u16 = 0xF000;
const S_IFREG: u16 = 0x8000;
const S_IFDIR: u16 = 0x4000;
const S_IFLNK: u16 = 0xA000;

/// Directory entry file types
const EXT4_FT_UNKNOWN: u8 = 0;
const EXT4_FT_REG_FILE: u8 = 1;
const EXT4_FT_DIR: u8 = 2;
const EXT4_FT_SYMLINK: u8 = 7;

/// Root directory inode number
const EXT4_ROOT_INO: u32 = 2;

/// Size of an ext4 extent header
const EXT4_EXTENT_HEADER_SIZE: usize = 12;
/// Size of an ext4 extent
const EXT4_EXTENT_SIZE: usize = 12;
/// Size of an ext4 extent index
const EXT4_EXTENT_IDX_SIZE: usize = 12;

/// Magic number for extent tree headers
const EXT4_EXT_MAGIC: u16 = 0xF30A;

static EXT4_NEXT_INODE: AtomicU64 = AtomicU64::new(1);

/// ext4 superblock (relevant fields only)
#[derive(Clone)]
struct Ext4Superblock {
    inodes_count: u32,
    blocks_count_lo: u32,
    free_blocks_count_lo: u32,
    free_inodes_count: u32,
    first_data_block: u32,
    log_block_size: u32,
    blocks_per_group: u32,
    inodes_per_group: u32,
    magic: u16,
    inode_size: u16,
    feature_incompat: u32,
    feature_ro_compat: u32,
    desc_size: u16,
    blocks_count_hi: u32,
}

impl Ext4Superblock {
    fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 256 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let magic = u16::from_le_bytes([data[0x38], data[0x39]]);
        if magic != EXT4_SUPER_MAGIC {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let desc_size_raw = u16::from_le_bytes([data[0xFE], data[0xFF]]);
        // desc_size is 0 for old ext2/ext3 (means 32 bytes)
        let desc_size = if desc_size_raw == 0 {
            32
        } else {
            desc_size_raw
        };

        Ok(Self {
            inodes_count: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            blocks_count_lo: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            free_blocks_count_lo: u32::from_le_bytes([
                data[0x0C], data[0x0D], data[0x0E], data[0x0F],
            ]),
            free_inodes_count: u32::from_le_bytes([data[0x10], data[0x11], data[0x12], data[0x13]]),
            first_data_block: u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]),
            log_block_size: u32::from_le_bytes([data[0x18], data[0x19], data[0x1A], data[0x1B]]),
            blocks_per_group: u32::from_le_bytes([data[0x20], data[0x21], data[0x22], data[0x23]]),
            inodes_per_group: u32::from_le_bytes([data[0x28], data[0x29], data[0x2A], data[0x2B]]),
            magic,
            inode_size: u16::from_le_bytes([data[0x58], data[0x59]]),
            feature_incompat: u32::from_le_bytes([data[0x60], data[0x61], data[0x62], data[0x63]]),
            feature_ro_compat: u32::from_le_bytes([data[0x64], data[0x65], data[0x66], data[0x67]]),
            desc_size,
            blocks_count_hi: u32::from_le_bytes([
                data[0x150],
                data[0x151],
                data[0x152],
                data[0x153],
            ]),
        })
    }

    fn block_size(&self) -> usize {
        1024 << self.log_block_size
    }

    fn num_block_groups(&self) -> u32 {
        self.blocks_count_lo.div_ceil(self.blocks_per_group)
    }
}

/// Block group descriptor (relevant fields)
#[derive(Clone)]
struct BlockGroupDesc {
    inode_table_lo: u32,
    inode_table_hi: u32,
}

impl BlockGroupDesc {
    fn parse(data: &[u8], desc_size: u16) -> Self {
        let inode_table_lo = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let inode_table_hi = if desc_size >= 64 && data.len() >= 44 {
            u32::from_le_bytes([data[40], data[41], data[42], data[43]])
        } else {
            0
        };
        Self {
            inode_table_lo,
            inode_table_hi,
        }
    }

    fn inode_table_block(&self) -> u64 {
        self.inode_table_lo as u64 | ((self.inode_table_hi as u64) << 32)
    }
}

/// On-disk inode (relevant fields)
struct Ext4Inode {
    mode: u16,
    size_lo: u32,
    size_hi: u32,
    atime: u32,
    mtime: u32,
    flags: u32,
    /// The 60-byte i_block area (direct/indirect blocks or extent tree)
    block_data: [u8; 60],
    links_count: u16,
}

impl Ext4Inode {
    fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 128 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let mut block_data = [0u8; 60];
        block_data.copy_from_slice(&data[0x28..0x64]);

        Ok(Self {
            mode: u16::from_le_bytes([data[0], data[1]]),
            size_lo: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            size_hi: u32::from_le_bytes([data[0x6C], data[0x6D], data[0x6E], data[0x6F]]),
            atime: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            mtime: u32::from_le_bytes([data[0x10], data[0x11], data[0x12], data[0x13]]),
            flags: u32::from_le_bytes([data[0x20], data[0x21], data[0x22], data[0x23]]),
            block_data,
            links_count: u16::from_le_bytes([data[0x1A], data[0x1B]]),
        })
    }

    fn size(&self) -> u64 {
        self.size_lo as u64 | ((self.size_hi as u64) << 32)
    }

    fn is_dir(&self) -> bool {
        (self.mode & S_IFMT) == S_IFDIR
    }

    fn is_file(&self) -> bool {
        (self.mode & S_IFMT) == S_IFREG
    }

    fn is_symlink(&self) -> bool {
        (self.mode & S_IFMT) == S_IFLNK
    }

    fn uses_extents(&self) -> bool {
        (self.flags & EXT4_EXTENTS_FL) != 0
    }

    fn node_type(&self) -> NodeType {
        if self.is_dir() {
            NodeType::Directory
        } else if self.is_symlink() {
            NodeType::Symlink
        } else {
            NodeType::File
        }
    }

    fn permissions(&self) -> Permissions {
        let mode = self.mode;
        Permissions {
            owner_read: (mode & 0o400) != 0,
            owner_write: (mode & 0o200) != 0,
            owner_exec: (mode & 0o100) != 0,
            group_read: (mode & 0o040) != 0,
            group_write: (mode & 0o020) != 0,
            group_exec: (mode & 0o010) != 0,
            other_read: (mode & 0o004) != 0,
            other_write: (mode & 0o002) != 0,
            other_exec: (mode & 0o001) != 0,
        }
    }
}

/// ext4 extent header (12 bytes)
struct ExtentHeader {
    magic: u16,
    entries: u16,
    _max: u16,
    depth: u16,
}

impl ExtentHeader {
    fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < EXT4_EXTENT_HEADER_SIZE {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }
        let magic = u16::from_le_bytes([data[0], data[1]]);
        if magic != EXT4_EXT_MAGIC {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }
        Ok(Self {
            magic,
            entries: u16::from_le_bytes([data[2], data[3]]),
            _max: u16::from_le_bytes([data[4], data[5]]),
            depth: u16::from_le_bytes([data[6], data[7]]),
        })
    }
}

/// ext4 extent leaf (12 bytes)
struct Extent {
    /// First file block covered by this extent
    block: u32,
    /// Number of blocks
    len: u16,
    /// High 16 bits of physical block
    start_hi: u16,
    /// Low 32 bits of physical block
    start_lo: u32,
}

impl Extent {
    fn parse(data: &[u8]) -> Self {
        Self {
            block: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            len: u16::from_le_bytes([data[4], data[5]]),
            start_hi: u16::from_le_bytes([data[6], data[7]]),
            start_lo: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        }
    }

    fn physical_block(&self) -> u64 {
        self.start_lo as u64 | ((self.start_hi as u64) << 32)
    }

    /// Actual length (high bit set means uninitialized)
    fn actual_len(&self) -> u32 {
        (self.len & 0x7FFF) as u32
    }
}

/// ext4 extent index (12 bytes) -- for internal tree nodes
struct ExtentIdx {
    /// File block covered by subtree
    block: u32,
    /// Physical block of child node
    leaf_lo: u32,
    leaf_hi: u16,
}

impl ExtentIdx {
    fn parse(data: &[u8]) -> Self {
        Self {
            block: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            leaf_lo: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            leaf_hi: u16::from_le_bytes([data[8], data[9]]),
        }
    }

    fn child_block(&self) -> u64 {
        self.leaf_lo as u64 | ((self.leaf_hi as u64) << 32)
    }
}

/// Shared ext4 state holding the image data and metadata
struct Ext4State {
    sb: Ext4Superblock,
    group_descs: Vec<BlockGroupDesc>,
    /// Full image data (read-only)
    data: Vec<u8>,
}

impl Ext4State {
    /// Read a block from the image
    fn read_block(&self, block: u64) -> Option<&[u8]> {
        let bs = self.sb.block_size();
        let offset = block as usize * bs;
        let end = offset + bs;
        if end <= self.data.len() {
            Some(&self.data[offset..end])
        } else {
            None
        }
    }

    /// Read an inode by number (1-indexed)
    fn read_inode(&self, ino: u32) -> Result<Ext4Inode, KernelError> {
        if ino == 0 || ino > self.sb.inodes_count {
            return Err(KernelError::FsError(FsError::NotFound));
        }

        let group = ((ino - 1) / self.sb.inodes_per_group) as usize;
        let index = ((ino - 1) % self.sb.inodes_per_group) as usize;

        if group >= self.group_descs.len() {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let table_block = self.group_descs[group].inode_table_block();
        let bs = self.sb.block_size();
        let inode_size = self.sb.inode_size as usize;
        let offset = table_block as usize * bs + index * inode_size;

        if offset + inode_size > self.data.len() {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        Ext4Inode::parse(&self.data[offset..offset + inode_size])
    }

    /// Read file data using the extent tree or block map
    fn read_inode_data(&self, inode: &Ext4Inode, max_size: u64) -> Vec<u8> {
        let size = core::cmp::min(inode.size(), max_size);
        if size == 0 {
            return Vec::new();
        }

        // For small symlinks, data is stored inline in i_block
        if inode.is_symlink() && size < 60 {
            return inode.block_data[..size as usize].to_vec();
        }

        if inode.uses_extents() {
            self.read_extent_data(&inode.block_data, size)
        } else {
            self.read_block_map_data(&inode.block_data, size)
        }
    }

    /// Read data via extent tree
    fn read_extent_data(&self, block_data: &[u8; 60], size: u64) -> Vec<u8> {
        let mut result = vec![0u8; size as usize];
        let bs = self.sb.block_size();

        // Parse extent tree from the inode's i_block area
        self.walk_extent_tree(block_data, &mut result, bs);

        result
    }

    /// Walk the extent tree recursively
    fn walk_extent_tree(&self, tree_data: &[u8], result: &mut [u8], bs: usize) {
        let header = match ExtentHeader::parse(tree_data) {
            Ok(h) => h,
            Err(_) => return,
        };

        if header.magic != EXT4_EXT_MAGIC {
            return;
        }

        let entries = header.entries as usize;

        if header.depth == 0 {
            // Leaf level: read extents directly
            for i in 0..entries {
                let off = EXT4_EXTENT_HEADER_SIZE + i * EXT4_EXTENT_SIZE;
                if off + EXT4_EXTENT_SIZE > tree_data.len() {
                    break;
                }
                let extent = Extent::parse(&tree_data[off..]);
                let file_offset = extent.block as usize * bs;
                let phys_block = extent.physical_block();
                let len = extent.actual_len() as usize;

                for b in 0..len {
                    let src_block = phys_block + b as u64;
                    let dst_offset = file_offset + b * bs;
                    if dst_offset >= result.len() {
                        break;
                    }

                    if let Some(block_data) = self.read_block(src_block) {
                        let copy_len = core::cmp::min(bs, result.len() - dst_offset);
                        result[dst_offset..dst_offset + copy_len]
                            .copy_from_slice(&block_data[..copy_len]);
                    }
                }
            }
        } else {
            // Internal node: follow index entries
            for i in 0..entries {
                let off = EXT4_EXTENT_HEADER_SIZE + i * EXT4_EXTENT_SIZE;
                if off + EXT4_EXTENT_SIZE > tree_data.len() {
                    break;
                }
                let idx = ExtentIdx::parse(&tree_data[off..]);
                if let Some(child_block) = self.read_block(idx.child_block()) {
                    // Recurse into child node (use up to block_size bytes)
                    let child_len = core::cmp::min(child_block.len(), bs);
                    self.walk_extent_tree(&child_block[..child_len], result, bs);
                }
            }
        }
    }

    /// Read data via legacy block map (direct + indirect blocks)
    fn read_block_map_data(&self, block_data: &[u8; 60], size: u64) -> Vec<u8> {
        let mut result = vec![0u8; size as usize];
        let bs = self.sb.block_size();
        let mut file_offset = 0usize;

        // 12 direct block pointers at offsets 0..48
        for i in 0..12 {
            if file_offset >= result.len() {
                return result;
            }
            let block_num = u32::from_le_bytes([
                block_data[i * 4],
                block_data[i * 4 + 1],
                block_data[i * 4 + 2],
                block_data[i * 4 + 3],
            ]);
            if block_num != 0 {
                if let Some(bdata) = self.read_block(block_num as u64) {
                    let copy_len = core::cmp::min(bs, result.len() - file_offset);
                    result[file_offset..file_offset + copy_len].copy_from_slice(&bdata[..copy_len]);
                }
            }
            file_offset += bs;
        }

        // Singly indirect block (offset 48)
        let indirect = u32::from_le_bytes([
            block_data[48],
            block_data[49],
            block_data[50],
            block_data[51],
        ]);
        if indirect != 0 && file_offset < result.len() {
            file_offset =
                self.read_indirect_blocks(indirect as u64, &mut result, file_offset, bs, 0);
        }

        // Doubly indirect (offset 52)
        let dindirect = u32::from_le_bytes([
            block_data[52],
            block_data[53],
            block_data[54],
            block_data[55],
        ]);
        if dindirect != 0 && file_offset < result.len() {
            file_offset =
                self.read_indirect_blocks(dindirect as u64, &mut result, file_offset, bs, 1);
        }

        // Triply indirect (offset 56)
        let tindirect = u32::from_le_bytes([
            block_data[56],
            block_data[57],
            block_data[58],
            block_data[59],
        ]);
        if tindirect != 0 && file_offset < result.len() {
            let _ = self.read_indirect_blocks(tindirect as u64, &mut result, file_offset, bs, 2);
        }

        result
    }

    /// Read through indirect block levels
    fn read_indirect_blocks(
        &self,
        block: u64,
        result: &mut [u8],
        mut offset: usize,
        bs: usize,
        level: u32,
    ) -> usize {
        let bdata = match self.read_block(block) {
            Some(d) => d,
            None => return offset,
        };

        let ptrs_per_block = bs / 4;
        for i in 0..ptrs_per_block {
            if offset >= result.len() {
                break;
            }
            let ptr = u32::from_le_bytes([
                bdata[i * 4],
                bdata[i * 4 + 1],
                bdata[i * 4 + 2],
                bdata[i * 4 + 3],
            ]);
            if ptr == 0 {
                if level == 0 {
                    offset += bs;
                }
                continue;
            }

            if level == 0 {
                // Direct data block
                if let Some(data_block) = self.read_block(ptr as u64) {
                    let copy_len = core::cmp::min(bs, result.len() - offset);
                    result[offset..offset + copy_len].copy_from_slice(&data_block[..copy_len]);
                }
                offset += bs;
            } else {
                // Recurse into next level
                offset = self.read_indirect_blocks(ptr as u64, result, offset, bs, level - 1);
            }
        }
        offset
    }

    /// Parse directory entries from raw data
    fn parse_directory(&self, data: &[u8]) -> Vec<(String, u32, u8)> {
        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + 8 <= data.len() {
            let inode =
                u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            let rec_len = u16::from_le_bytes([data[pos + 4], data[pos + 5]]) as usize;
            let name_len = data[pos + 6] as usize;
            let file_type = data[pos + 7];

            if rec_len == 0 {
                break;
            }

            if inode != 0 && name_len > 0 && pos + 8 + name_len <= data.len() {
                let name_bytes = &data[pos + 8..pos + 8 + name_len];
                if let Ok(name) = core::str::from_utf8(name_bytes) {
                    if name != "." && name != ".." {
                        entries.push((String::from(name), inode, file_type));
                    }
                }
            }

            pos += rec_len;
        }

        entries
    }
}

/// A cached ext4 directory/file/symlink node
pub struct Ext4Node {
    vfs_inode: u64,
    ext4_ino: u32,
    node_type: NodeType,
    metadata: RwLock<Metadata>,
    children: RwLock<BTreeMap<String, Arc<Ext4Node>>>,
    fs_state: Arc<RwLock<Ext4State>>,
    /// Symlink target (populated for symlinks)
    link_target: RwLock<Option<String>>,
}

impl Ext4Node {
    fn new(
        ext4_ino: u32,
        inode: &Ext4Inode,
        _parent_inode: u64,
        fs_state: Arc<RwLock<Ext4State>>,
    ) -> Self {
        let vfs_inode = EXT4_NEXT_INODE.fetch_add(1, Ordering::Relaxed);
        Self {
            vfs_inode,
            ext4_ino,
            node_type: inode.node_type(),
            metadata: RwLock::new(Metadata {
                inode: vfs_inode,
                size: inode.size() as usize,
                node_type: inode.node_type(),
                permissions: inode.permissions(),
                created: 0,
                modified: inode.mtime as u64,
                accessed: 0,
                uid: 0,
                gid: 0,
            }),
            children: RwLock::new(BTreeMap::new()),
            fs_state,
            link_target: RwLock::new(None),
        }
    }

    /// Load children from disk if directory and not yet cached
    fn ensure_children_loaded(&self) {
        if self.node_type != NodeType::Directory {
            return;
        }

        {
            let children = self.children.read();
            if !children.is_empty() {
                return;
            }
        }

        let entries = {
            let state = self.fs_state.read();
            let inode = match state.read_inode(self.ext4_ino) {
                Ok(i) => i,
                Err(_) => return,
            };
            let dir_data = state.read_inode_data(&inode, inode.size());
            let parsed = state.parse_directory(&dir_data);

            // Build child nodes while we still hold state
            let mut result = Vec::new();
            for (name, ino, ft) in parsed {
                if let Ok(child_inode) = state.read_inode(ino) {
                    result.push((name, ino, child_inode, ft));
                }
            }
            result
        };

        let mut children = self.children.write();
        for (name, ino, child_inode, _ft) in entries {
            let child = Arc::new(Ext4Node::new(
                ino,
                &child_inode,
                self.vfs_inode,
                self.fs_state.clone(),
            ));
            children.insert(name, child);
        }
    }

    /// Load symlink target
    fn ensure_link_loaded(&self) {
        if self.node_type != NodeType::Symlink {
            return;
        }

        {
            let link = self.link_target.read();
            if link.is_some() {
                return;
            }
        }

        let target = {
            let state = self.fs_state.read();
            match state.read_inode(self.ext4_ino) {
                Ok(inode) => {
                    let data = state.read_inode_data(&inode, inode.size());
                    String::from_utf8_lossy(&data).into_owned()
                }
                Err(_) => return,
            }
        };

        let mut link = self.link_target.write();
        *link = Some(target);
    }
}

impl VfsNode for Ext4Node {
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

        let file_data = {
            let state = self.fs_state.read();
            let inode = state.read_inode(self.ext4_ino)?;
            state.read_inode_data(&inode, inode.size())
        };

        let bytes_to_read = core::cmp::min(buffer.len(), file_data.len().saturating_sub(offset));
        buffer[..bytes_to_read].copy_from_slice(&file_data[offset..offset + bytes_to_read]);

        Ok(bytes_to_read)
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
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

        let entries = children
            .iter()
            .map(|(name, node)| DirEntry {
                name: name.clone(),
                inode: node.vfs_inode,
                node_type: node.node_type,
            })
            .collect();

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
            .map(|n| n.clone() as Arc<dyn VfsNode>)
            .ok_or(KernelError::FsError(FsError::NotFound))
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

    fn readlink(&self) -> Result<String, KernelError> {
        if self.node_type != NodeType::Symlink {
            return Err(KernelError::FsError(FsError::NotASymlink));
        }
        self.ensure_link_loaded();
        let link = self.link_target.read();
        link.clone().ok_or(KernelError::FsError(FsError::IoError))
    }

    fn chmod(&self, _permissions: Permissions) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::ReadOnly))
    }
}

/// ext4 filesystem (read-only)
pub struct Ext4Fs {
    root: Arc<Ext4Node>,
    state: Arc<RwLock<Ext4State>>,
}

impl Ext4Fs {
    /// Create an ext4 filesystem from raw image data (read-only).
    pub fn from_image(image_data: &[u8]) -> Result<Self, KernelError> {
        if image_data.len() < SUPERBLOCK_OFFSET + 256 {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        let sb = Ext4Superblock::parse(&image_data[SUPERBLOCK_OFFSET..])?;
        let bs = sb.block_size();

        // Parse block group descriptors
        // They start at the block after the superblock
        let gd_block = if bs == 1024 { 2 } else { 1 };
        let gd_offset = gd_block * bs;
        let num_groups = sb.num_block_groups() as usize;
        let desc_size = sb.desc_size as usize;

        let mut group_descs = Vec::with_capacity(num_groups);
        for i in 0..num_groups {
            let off = gd_offset + i * desc_size;
            if off + desc_size > image_data.len() {
                break;
            }
            group_descs.push(BlockGroupDesc::parse(
                &image_data[off..off + desc_size],
                sb.desc_size,
            ));
        }

        let state = Arc::new(RwLock::new(Ext4State {
            sb: sb.clone(),
            group_descs,
            data: image_data.to_vec(),
        }));

        // Read root inode (inode 2)
        let root_inode = {
            let st = state.read();
            st.read_inode(EXT4_ROOT_INO)?
        };

        let root = Arc::new(Ext4Node::new(EXT4_ROOT_INO, &root_inode, 0, state.clone()));

        Ok(Self { root, state })
    }
}

impl Filesystem for Ext4Fs {
    fn root(&self) -> Arc<dyn VfsNode> {
        self.root.clone() as Arc<dyn VfsNode>
    }

    fn name(&self) -> &str {
        "ext4"
    }

    fn is_readonly(&self) -> bool {
        true
    }

    fn sync(&self) -> Result<(), KernelError> {
        Ok(()) // Read-only, nothing to sync
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal ext4 image for testing.
    /// This creates a tiny ext4 volume with:
    /// - 1KB block size (log_block_size = 0)
    /// - 1 block group
    /// - Root directory with one file "hello.txt" containing "Hello, ext4!\n"
    fn make_test_ext4_image() -> Vec<u8> {
        let block_size = 1024usize;
        let total_blocks = 64u32;
        let total_size = total_blocks as usize * block_size;
        let mut img = vec![0u8; total_size];

        // === Superblock at offset 1024 (block 1) ===
        let sb_off = SUPERBLOCK_OFFSET;
        let inodes_count = 16u32;
        let blocks_per_group = total_blocks;
        let inodes_per_group = inodes_count;
        let inode_size = 128u16;

        // s_inodes_count (0x00)
        img[sb_off..sb_off + 4].copy_from_slice(&inodes_count.to_le_bytes());
        // s_blocks_count_lo (0x04)
        img[sb_off + 0x04..sb_off + 0x08].copy_from_slice(&total_blocks.to_le_bytes());
        // s_first_data_block (0x14) - 1 for 1KB blocks
        img[sb_off + 0x14..sb_off + 0x18].copy_from_slice(&1u32.to_le_bytes());
        // s_log_block_size (0x18) - 0 means 1024
        img[sb_off + 0x18..sb_off + 0x1C].copy_from_slice(&0u32.to_le_bytes());
        // s_blocks_per_group (0x20)
        img[sb_off + 0x20..sb_off + 0x24].copy_from_slice(&blocks_per_group.to_le_bytes());
        // s_inodes_per_group (0x28)
        img[sb_off + 0x28..sb_off + 0x2C].copy_from_slice(&inodes_per_group.to_le_bytes());
        // s_magic (0x38)
        img[sb_off + 0x38..sb_off + 0x3A].copy_from_slice(&EXT4_SUPER_MAGIC.to_le_bytes());
        // s_inode_size (0x58)
        img[sb_off + 0x58..sb_off + 0x5A].copy_from_slice(&inode_size.to_le_bytes());
        // s_desc_size (0xFE) - 0 means 32 bytes (old format)
        img[sb_off + 0xFE..sb_off + 0x100].copy_from_slice(&0u16.to_le_bytes());

        // === Block Group Descriptor at block 2 (offset 2048) ===
        let gd_off = 2 * block_size;
        // bg_inode_table_lo (offset 8) - inode table at block 4
        let inode_table_block = 4u32;
        img[gd_off + 8..gd_off + 12].copy_from_slice(&inode_table_block.to_le_bytes());

        // === Inode Table at block 4 (offset 4096) ===
        let it_off = inode_table_block as usize * block_size;

        // --- Root inode (inode 2, index 1) at it_off + 128 ---
        let root_off = it_off + 1 * inode_size as usize;
        // i_mode: directory (0x4000) + rwxr-xr-x (0o755) = 0x41ED
        img[root_off..root_off + 2].copy_from_slice(&0x41EDu16.to_le_bytes());
        // i_size_lo: directory data size (1 block)
        img[root_off + 4..root_off + 8].copy_from_slice(&(block_size as u32).to_le_bytes());
        // i_flags: no extents, use legacy block map
        img[root_off + 0x20..root_off + 0x24].copy_from_slice(&0u32.to_le_bytes());
        // i_block[0]: direct block 10 (directory data)
        let root_data_block = 10u32;
        img[root_off + 0x28..root_off + 0x2C].copy_from_slice(&root_data_block.to_le_bytes());

        // --- File inode (inode 3, index 2) at it_off + 256 ---
        let file_content = b"Hello, ext4!\n";
        let file_off = it_off + 2 * inode_size as usize;
        // i_mode: regular file (0x8000) + rw-r--r-- (0o644) = 0x81A4
        img[file_off..file_off + 2].copy_from_slice(&0x81A4u16.to_le_bytes());
        // i_size_lo
        img[file_off + 4..file_off + 8].copy_from_slice(&(file_content.len() as u32).to_le_bytes());
        // i_flags: no extents
        img[file_off + 0x20..file_off + 0x24].copy_from_slice(&0u32.to_le_bytes());
        // i_block[0]: direct block 11
        let file_data_block = 11u32;
        img[file_off + 0x28..file_off + 0x2C].copy_from_slice(&file_data_block.to_le_bytes());

        // --- Symlink inode (inode 4, index 3) at it_off + 384 ---
        let link_target = b"hello.txt";
        let link_off = it_off + 3 * inode_size as usize;
        // i_mode: symlink (0xA000) + rwxrwxrwx (0o777) = 0xA1FF
        img[link_off..link_off + 2].copy_from_slice(&0xA1FFu16.to_le_bytes());
        // i_size_lo
        img[link_off + 4..link_off + 8].copy_from_slice(&(link_target.len() as u32).to_le_bytes());
        // Inline symlink: target stored in i_block (< 60 bytes)
        img[link_off + 0x28..link_off + 0x28 + link_target.len()].copy_from_slice(link_target);

        // === Root directory data at block 10 ===
        let dir_off = root_data_block as usize * block_size;

        // Entry 1: "." -> inode 2
        let mut pos = dir_off;
        img[pos..pos + 4].copy_from_slice(&2u32.to_le_bytes()); // inode
        img[pos + 4..pos + 6].copy_from_slice(&12u16.to_le_bytes()); // rec_len
        img[pos + 6] = 1; // name_len
        img[pos + 7] = EXT4_FT_DIR; // file_type
        img[pos + 8] = b'.';
        pos += 12;

        // Entry 2: ".." -> inode 2
        img[pos..pos + 4].copy_from_slice(&2u32.to_le_bytes());
        img[pos + 4..pos + 6].copy_from_slice(&12u16.to_le_bytes());
        img[pos + 6] = 2;
        img[pos + 7] = EXT4_FT_DIR;
        img[pos + 8] = b'.';
        img[pos + 9] = b'.';
        pos += 12;

        // Entry 3: "hello.txt" -> inode 3
        let name = b"hello.txt";
        img[pos..pos + 4].copy_from_slice(&3u32.to_le_bytes());
        img[pos + 4..pos + 6].copy_from_slice(&20u16.to_le_bytes()); // rec_len (aligned to 4)
        img[pos + 6] = name.len() as u8;
        img[pos + 7] = EXT4_FT_REG_FILE;
        img[pos + 8..pos + 8 + name.len()].copy_from_slice(name);
        pos += 20;

        // Entry 4: "link" -> inode 4
        let link_name = b"link";
        // rec_len = rest of block
        let rec_len = (block_size - (pos - dir_off)) as u16;
        img[pos..pos + 4].copy_from_slice(&4u32.to_le_bytes());
        img[pos + 4..pos + 6].copy_from_slice(&rec_len.to_le_bytes());
        img[pos + 6] = link_name.len() as u8;
        img[pos + 7] = EXT4_FT_SYMLINK;
        img[pos + 8..pos + 8 + link_name.len()].copy_from_slice(link_name);

        // === File data at block 11 ===
        let fdata_off = file_data_block as usize * block_size;
        img[fdata_off..fdata_off + file_content.len()].copy_from_slice(file_content);

        img
    }

    #[test]
    fn test_superblock_parse() {
        let img = make_test_ext4_image();
        let sb = Ext4Superblock::parse(&img[SUPERBLOCK_OFFSET..]).unwrap();
        assert_eq!(sb.magic, EXT4_SUPER_MAGIC);
        assert_eq!(sb.block_size(), 1024);
        assert_eq!(sb.inodes_count, 16);
        assert_eq!(sb.inode_size, 128);
    }

    #[test]
    fn test_mount_and_list_root() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        assert_eq!(fs.name(), "ext4");
        assert!(fs.is_readonly());

        let root = fs.root();
        assert_eq!(root.node_type(), NodeType::Directory);

        let entries = root.readdir().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hello.txt"));
        assert!(names.contains(&"link"));
    }

    #[test]
    fn test_read_file() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        let file = root.lookup("hello.txt").unwrap();
        assert_eq!(file.node_type(), NodeType::File);

        let mut buf = [0u8; 64];
        let n = file.read(0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"Hello, ext4!\n");
    }

    #[test]
    fn test_read_symlink() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        let link = root.lookup("link").unwrap();
        assert_eq!(link.node_type(), NodeType::Symlink);

        let target = link.readlink().unwrap();
        assert_eq!(target, "hello.txt");
    }

    #[test]
    fn test_read_only_enforcement() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        assert!(root.write(0, b"test").is_err());
        assert!(root.create("new", Permissions::default()).is_err());
        assert!(root.mkdir("dir", Permissions::default()).is_err());
        assert!(root.unlink("hello.txt").is_err());
        assert!(root.truncate(0).is_err());
    }

    #[test]
    fn test_metadata() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        let file = root.lookup("hello.txt").unwrap();
        let meta = file.metadata().unwrap();
        assert_eq!(meta.size, 13); // "Hello, ext4!\n"
        assert_eq!(meta.node_type, NodeType::File);
        assert!(meta.permissions.owner_read);
        assert!(meta.permissions.owner_write);
    }

    #[test]
    fn test_invalid_magic() {
        let mut img = vec![0u8; 2048];
        // No valid superblock
        assert!(Ext4Fs::from_image(&img).is_err());

        // Set wrong magic
        img[SUPERBLOCK_OFFSET + 0x38] = 0xFF;
        img[SUPERBLOCK_OFFSET + 0x39] = 0xFF;
        assert!(Ext4Fs::from_image(&img).is_err());
    }

    #[test]
    fn test_file_not_found() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        assert!(root.lookup("nonexistent").is_err());
    }

    #[test]
    fn test_read_at_offset() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        let file = root.lookup("hello.txt").unwrap();
        let mut buf = [0u8; 64];
        let n = file.read(7, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"ext4!\n");
    }

    #[test]
    fn test_read_past_end() {
        let img = make_test_ext4_image();
        let fs = Ext4Fs::from_image(&img).unwrap();
        let root = fs.root();

        let file = root.lookup("hello.txt").unwrap();
        let mut buf = [0u8; 64];
        let n = file.read(100, &mut buf).unwrap();
        assert_eq!(n, 0);
    }
}
