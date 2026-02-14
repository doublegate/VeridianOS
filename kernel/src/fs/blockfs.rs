//! Block-based persistent filesystem (BlockFS)
//!
//! A simple ext2-like filesystem with:
//! - Superblock with metadata
//! - Inode table for file/directory metadata
//! - Block allocation bitmap
//! - Data blocks for file content

// Allow dead code for filesystem methods not yet called from higher layers
#![allow(
    dead_code,
    clippy::manual_div_ceil,
    clippy::slow_vector_initialization,
    clippy::manual_saturating_arithmetic,
    clippy::implicit_saturating_sub
)]

use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::mem::size_of;

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// Block size (4KB)
pub const BLOCK_SIZE: usize = 4096;

/// Magic number for BlockFS
pub const BLOCKFS_MAGIC: u32 = 0x424C4B46; // "BLKF"

/// Maximum filename length
pub const MAX_FILENAME_LEN: usize = 255;

/// Superblock structure
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Superblock {
    pub magic: u32,
    pub block_count: u32,
    pub inode_count: u32,
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub first_data_block: u32,
    pub block_size: u32,
    pub inode_size: u16,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub mount_time: u64,
    pub write_time: u64,
    pub mount_count: u16,
    pub max_mount_count: u16,
    pub state: u16,
    pub errors: u16,
}

impl Superblock {
    pub fn new(block_count: u32, inode_count: u32) -> Self {
        Self {
            magic: BLOCKFS_MAGIC,
            block_count,
            inode_count,
            free_blocks: block_count - 10, // Reserve first 10 blocks
            free_inodes: inode_count - 1,  // Reserve root inode
            first_data_block: 10,
            block_size: BLOCK_SIZE as u32,
            inode_size: size_of::<DiskInode>() as u16,
            blocks_per_group: 8192,
            inodes_per_group: 2048,
            mount_time: 0,
            write_time: 0,
            mount_count: 0,
            max_mount_count: 100,
            state: 1, // Clean
            errors: 0,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == BLOCKFS_MAGIC
    }
}

/// On-disk inode structure
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DiskInode {
    pub mode: u16,
    pub uid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks: u32,
    pub flags: u32,
    pub direct_blocks: [u32; 12],
    pub indirect_block: u32,
    pub double_indirect_block: u32,
    pub triple_indirect_block: u32,
}

impl DiskInode {
    pub fn new(mode: u16, uid: u16, gid: u16) -> Self {
        Self {
            mode,
            uid,
            gid,
            size: 0,
            atime: 0,
            ctime: 0,
            mtime: 0,
            dtime: 0,
            links_count: 1,
            blocks: 0,
            flags: 0,
            direct_blocks: [0; 12],
            indirect_block: 0,
            double_indirect_block: 0,
            triple_indirect_block: 0,
        }
    }

    pub fn is_dir(&self) -> bool {
        (self.mode & 0x4000) != 0
    }

    pub fn is_file(&self) -> bool {
        (self.mode & 0x8000) != 0
    }

    pub fn node_type(&self) -> NodeType {
        if self.is_dir() {
            NodeType::Directory
        } else {
            // Regular file or default for unrecognized inode modes
            NodeType::File
        }
    }
}

/// Size of the fixed header in a DiskDirEntry (inode + rec_len + name_len +
/// file_type)
pub const DIR_ENTRY_HEADER_SIZE: usize = 8;

/// On-disk directory entry (ext2-style variable-length record)
///
/// Layout:
///   - inode:     4 bytes (inode number, 0 = deleted entry)
///   - rec_len:   2 bytes (total record length, always 4-byte aligned)
///   - name_len:  1 byte  (actual name length)
///   - file_type: 1 byte  (1=file, 2=directory)
///   - name:      up to 255 bytes
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DiskDirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
    pub name: [u8; 255],
}

impl DiskDirEntry {
    /// File type constant for regular files
    pub const FT_REG_FILE: u8 = 1;
    /// File type constant for directories
    pub const FT_DIR: u8 = 2;

    /// Create a new directory entry
    pub fn new(inode: u32, name: &str, file_type: u8) -> Self {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(MAX_FILENAME_LEN) as u8;
        let rec_len = align4(DIR_ENTRY_HEADER_SIZE + name_len as usize) as u16;

        let mut entry = Self {
            inode,
            rec_len,
            name_len,
            file_type,
            name: [0u8; 255],
        };

        let copy_len = name_len as usize;
        entry.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        entry
    }

    /// Get the name as a string slice
    pub fn name_str(&self) -> &str {
        let slice = &self.name[..self.name_len as usize];
        core::str::from_utf8(slice).unwrap_or("")
    }

    /// Convert file_type to NodeType
    pub fn node_type(&self) -> NodeType {
        match self.file_type {
            Self::FT_DIR => NodeType::Directory,
            _ => NodeType::File,
        }
    }
}

/// Align a value up to the next 4-byte boundary
fn align4(val: usize) -> usize {
    (val + 3) & !3
}

/// Block allocation bitmap
pub struct BlockBitmap {
    bitmap: Vec<u8>,
    total_blocks: usize,
}

impl BlockBitmap {
    pub fn new(total_blocks: usize) -> Self {
        let bitmap_size = (total_blocks + 7) / 8;
        let mut bitmap = Vec::new();
        bitmap.resize(bitmap_size, 0);

        Self {
            bitmap,
            total_blocks,
        }
    }

    pub fn allocate_block(&mut self) -> Option<u32> {
        for (byte_idx, byte) in self.bitmap.iter_mut().enumerate() {
            if *byte != 0xFF {
                for bit in 0..8 {
                    if (*byte & (1 << bit)) == 0 {
                        *byte |= 1 << bit;
                        let block_num = (byte_idx * 8 + bit) as u32;
                        if (block_num as usize) < self.total_blocks {
                            return Some(block_num);
                        }
                    }
                }
            }
        }
        None
    }

    pub fn free_block(&mut self, block: u32) {
        let byte_idx = (block / 8) as usize;
        let bit = (block % 8) as usize;
        if byte_idx < self.bitmap.len() {
            self.bitmap[byte_idx] &= !(1 << bit);
        }
    }

    pub fn is_allocated(&self, block: u32) -> bool {
        let byte_idx = (block / 8) as usize;
        let bit = (block % 8) as usize;
        if byte_idx < self.bitmap.len() {
            (self.bitmap[byte_idx] & (1 << bit)) != 0
        } else {
            false
        }
    }
}

/// BlockFS node implementation
pub struct BlockFsNode {
    inode_num: u32,
    fs: Arc<RwLock<BlockFsInner>>,
}

impl BlockFsNode {
    pub fn new(inode_num: u32, fs: Arc<RwLock<BlockFsInner>>) -> Self {
        Self { inode_num, fs }
    }
}

impl VfsNode for BlockFsNode {
    fn node_type(&self) -> NodeType {
        self.metadata()
            .map(|m| m.node_type)
            .unwrap_or(NodeType::File)
    }

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let fs = self.fs.read();
        fs.read_inode(self.inode_num, offset, buffer)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        let mut fs = self.fs.write();
        fs.write_inode(self.inode_num, offset, data)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        let fs = self.fs.read();
        fs.get_metadata(self.inode_num)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        let fs = self.fs.read();
        fs.readdir(self.inode_num)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let fs = self.fs.read();
        let child_inode = fs.lookup_in_dir(self.inode_num, name)?;
        Ok(Arc::new(BlockFsNode::new(child_inode, self.fs.clone())))
    }

    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        let mut fs = self.fs.write();
        let new_inode = fs.create_file(self.inode_num, name, permissions)?;
        Ok(Arc::new(BlockFsNode::new(new_inode, self.fs.clone())))
    }

    fn mkdir(&self, name: &str, permissions: Permissions) -> Result<Arc<dyn VfsNode>, KernelError> {
        let mut fs = self.fs.write();
        let new_inode = fs.create_directory(self.inode_num, name, permissions)?;
        Ok(Arc::new(BlockFsNode::new(new_inode, self.fs.clone())))
    }

    fn unlink(&self, name: &str) -> Result<(), KernelError> {
        let mut fs = self.fs.write();
        fs.unlink_from_dir(self.inode_num, name)
    }

    fn truncate(&self, size: usize) -> Result<(), KernelError> {
        let mut fs = self.fs.write();
        fs.truncate_inode(self.inode_num, size)
    }
}

/// Internal BlockFS state
pub struct BlockFsInner {
    superblock: Superblock,
    block_bitmap: BlockBitmap,
    inode_table: Vec<DiskInode>,
    block_data: Vec<Vec<u8>>, // Simulated block storage
}

impl BlockFsInner {
    pub fn new(block_count: u32, inode_count: u32) -> Self {
        let superblock = Superblock::new(block_count, inode_count);
        let block_bitmap = BlockBitmap::new(block_count as usize);
        let mut inode_table = Vec::new();
        inode_table.resize(inode_count as usize, DiskInode::new(0, 0, 0));

        // Initialize root directory (inode 0)
        // links_count = 2: one for itself (".") and one from the parent (root is its
        // own parent)
        let mut root_inode = DiskInode::new(0x41ED, 0, 0); // Directory, rwxr-xr-x
        root_inode.links_count = 2;
        inode_table[0] = root_inode;

        // Initialize block storage
        let mut block_data = Vec::new();
        for _ in 0..block_count {
            block_data.push(vec![0u8; BLOCK_SIZE]);
        }

        let mut fs = Self {
            superblock,
            block_bitmap,
            inode_table,
            block_data,
        };

        // Create "." and ".." entries in the root directory (both point to inode 0)
        let _ = fs.write_dir_entry(0, 0, ".", DiskDirEntry::FT_DIR);
        let _ = fs.write_dir_entry(0, 0, "..", DiskDirEntry::FT_DIR);

        fs
    }

    fn allocate_inode(&mut self) -> Option<u32> {
        for (idx, inode) in self.inode_table.iter().enumerate() {
            if inode.links_count == 0 && idx > 0 {
                // Don't allocate root
                self.superblock.free_inodes -= 1;
                return Some(idx as u32);
            }
        }
        None
    }

    fn allocate_block(&mut self) -> Option<u32> {
        let block = self.block_bitmap.allocate_block()?;
        self.superblock.free_blocks -= 1;
        Some(block)
    }

    fn free_block(&mut self, block: u32) {
        self.block_bitmap.free_block(block);
        self.superblock.free_blocks += 1;
    }

    fn read_inode(
        &self,
        inode_num: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<usize, KernelError> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        if offset >= inode.size as usize {
            return Ok(0);
        }

        let to_read = buffer.len().min(inode.size as usize - offset);
        let mut bytes_read = 0;

        // Read from direct blocks
        for i in 0..12 {
            if bytes_read >= to_read {
                break;
            }

            let block_num = inode.direct_blocks[i];
            if block_num == 0 {
                break;
            }

            let block_offset = if offset > i * BLOCK_SIZE {
                offset - i * BLOCK_SIZE
            } else {
                0
            };

            if block_offset < BLOCK_SIZE {
                let block = &self.block_data[block_num as usize];
                let copy_len = (BLOCK_SIZE - block_offset).min(to_read - bytes_read);
                buffer[bytes_read..bytes_read + copy_len]
                    .copy_from_slice(&block[block_offset..block_offset + copy_len]);
                bytes_read += copy_len;
            }
        }

        Ok(bytes_read)
    }

    fn write_inode(
        &mut self,
        inode_num: u32,
        offset: usize,
        data: &[u8],
    ) -> Result<usize, KernelError> {
        // Collect block information in multiple passes to avoid borrow conflicts
        let mut blocks_needed = Vec::new();
        let mut current_offset = offset;
        let mut bytes_remaining = data.len();

        // Determine which blocks we need
        while bytes_remaining > 0 {
            let block_idx = current_offset / BLOCK_SIZE;
            if block_idx >= 12 {
                break; // Beyond direct blocks
            }

            let block_offset = current_offset % BLOCK_SIZE;
            let copy_len = (BLOCK_SIZE - block_offset).min(bytes_remaining);

            blocks_needed.push((block_idx, block_offset, copy_len));

            bytes_remaining -= copy_len;
            current_offset += copy_len;
        }

        // Allocate any missing blocks and collect block numbers
        let mut block_numbers = Vec::new();
        for (block_idx, _, _) in &blocks_needed {
            let inode = &self.inode_table[inode_num as usize];
            let block_num = if inode.direct_blocks[*block_idx] == 0 {
                let new_block = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.inode_table[inode_num as usize].direct_blocks[*block_idx] = new_block;
                self.inode_table[inode_num as usize].blocks += 1;
                new_block
            } else {
                inode.direct_blocks[*block_idx]
            };
            block_numbers.push(block_num);
        }

        // Write data to blocks
        let mut bytes_written = 0;
        for (i, (_, block_offset, copy_len)) in blocks_needed.iter().enumerate() {
            let block_num = block_numbers[i];
            self.block_data[block_num as usize][*block_offset..*block_offset + *copy_len]
                .copy_from_slice(&data[bytes_written..bytes_written + *copy_len]);
            bytes_written += *copy_len;
        }

        // Update inode size
        if (offset + bytes_written) > self.inode_table[inode_num as usize].size as usize {
            self.inode_table[inode_num as usize].size = (offset + bytes_written) as u32;
        }

        Ok(bytes_written)
    }

    fn get_metadata(&self, inode_num: u32) -> Result<Metadata, KernelError> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        Ok(Metadata {
            node_type: inode.node_type(),
            size: inode.size as usize,
            permissions: Permissions::from_mode(inode.mode as u32),
            uid: inode.uid as u32,
            gid: inode.gid as u32,
            created: inode.ctime as u64,
            modified: inode.mtime as u64,
            accessed: inode.atime as u64,
        })
    }

    fn readdir(&self, inode_num: u32) -> Result<Vec<DirEntry>, KernelError> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        if !inode.is_dir() {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let mut entries = Vec::new();
        let dir_size = inode.size as usize;

        // Iterate through direct blocks that contain directory entries
        for i in 0..12 {
            let block_num = inode.direct_blocks[i];
            if block_num == 0 {
                break;
            }

            let block_start = i * BLOCK_SIZE;
            if block_start >= dir_size {
                break;
            }

            let block = &self.block_data[block_num as usize];
            let block_end = BLOCK_SIZE.min(dir_size - block_start);
            let mut offset = 0;

            while offset + DIR_ENTRY_HEADER_SIZE <= block_end {
                let entry = self.read_dir_entry(block, offset);
                let rec_len = entry.rec_len as usize;

                // rec_len must be at least the header size and 4-byte aligned
                if rec_len < DIR_ENTRY_HEADER_SIZE || !rec_len.is_multiple_of(4) {
                    break;
                }

                // Skip deleted entries (inode == 0) but still advance
                if entry.inode != 0 && entry.name_len > 0 {
                    entries.push(DirEntry {
                        name: String::from(entry.name_str()),
                        node_type: entry.node_type(),
                        inode: entry.inode as u64,
                    });
                }

                offset += rec_len;
            }
        }

        Ok(entries)
    }

    fn lookup_in_dir(&self, dir_inode: u32, name: &str) -> Result<u32, KernelError> {
        // Validate inode exists and is a directory (scoped borrow)
        {
            let inode = self
                .inode_table
                .get(dir_inode as usize)
                .ok_or(KernelError::FsError(FsError::NotFound))?;

            if !inode.is_dir() {
                return Err(KernelError::FsError(FsError::NotADirectory));
            }
        }

        match self.find_dir_entry(dir_inode, name) {
            Some((entry, _, _)) => Ok(entry.inode),
            None => Err(KernelError::FsError(FsError::NotFound)),
        }
    }

    fn create_file(
        &mut self,
        parent: u32,
        name: &str,
        permissions: Permissions,
    ) -> Result<u32, KernelError> {
        // Check name length
        if name.is_empty() || name.len() > MAX_FILENAME_LEN {
            return Err(KernelError::InvalidArgument {
                name: "filename",
                value: "empty or exceeds maximum length",
            });
        }

        // Check if the name already exists in the parent directory
        if self.find_dir_entry(parent, name).is_some() {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode_num = self
            .allocate_inode()
            .ok_or(KernelError::ResourceExhausted { resource: "inodes" })?;

        let mode = permissions_to_mode(permissions, false);
        self.inode_table[inode_num as usize] = DiskInode::new(mode, 0, 0);

        // Add directory entry to parent
        if let Err(e) = self.write_dir_entry(parent, inode_num, name, DiskDirEntry::FT_REG_FILE) {
            // Roll back inode allocation on failure
            self.inode_table[inode_num as usize].links_count = 0;
            self.superblock.free_inodes += 1;
            return Err(e);
        }

        Ok(inode_num)
    }

    fn create_directory(
        &mut self,
        parent: u32,
        name: &str,
        permissions: Permissions,
    ) -> Result<u32, KernelError> {
        // Check name length
        if name.is_empty() || name.len() > MAX_FILENAME_LEN {
            return Err(KernelError::InvalidArgument {
                name: "dirname",
                value: "empty or exceeds maximum length",
            });
        }

        // Check if the name already exists in the parent directory
        if self.find_dir_entry(parent, name).is_some() {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode_num = self
            .allocate_inode()
            .ok_or(KernelError::ResourceExhausted { resource: "inodes" })?;

        let mode = permissions_to_mode(permissions, true);
        let mut new_inode = DiskInode::new(mode, 0, 0);
        // Directories start with link count 2 (parent's entry + self ".")
        new_inode.links_count = 2;
        self.inode_table[inode_num as usize] = new_inode;

        // Create "." entry (self-reference) in the new directory
        if let Err(e) = self.write_dir_entry(inode_num, inode_num, ".", DiskDirEntry::FT_DIR) {
            self.inode_table[inode_num as usize].links_count = 0;
            self.superblock.free_inodes += 1;
            return Err(e);
        }

        // Create ".." entry (parent reference) in the new directory
        if let Err(e) = self.write_dir_entry(inode_num, parent, "..", DiskDirEntry::FT_DIR) {
            self.inode_table[inode_num as usize].links_count = 0;
            self.superblock.free_inodes += 1;
            return Err(e);
        }

        // Add entry for the new directory in the parent directory
        if let Err(e) = self.write_dir_entry(parent, inode_num, name, DiskDirEntry::FT_DIR) {
            self.inode_table[inode_num as usize].links_count = 0;
            self.superblock.free_inodes += 1;
            return Err(e);
        }

        // Increment parent's link count (for the ".." entry pointing back)
        self.inode_table[parent as usize].links_count += 1;

        Ok(inode_num)
    }

    fn unlink_from_dir(&mut self, parent: u32, name: &str) -> Result<(), KernelError> {
        // Cannot unlink "." or ".."
        if name == "." || name == ".." {
            return Err(KernelError::InvalidArgument {
                name: "filename",
                value: "cannot unlink . or ..",
            });
        }

        // Find the entry in the parent directory
        let (entry, block_idx, offset) = self
            .find_dir_entry(parent, name)
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        let target_inode = entry.inode;
        let is_dir = entry.file_type == DiskDirEntry::FT_DIR;

        // If unlinking a directory, check that it is empty (only "." and ".." entries)
        if is_dir {
            let child_entries = self.readdir(target_inode)?;
            let non_dot_count = child_entries
                .iter()
                .filter(|e| e.name != "." && e.name != "..")
                .count();
            if non_dot_count > 0 {
                return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
            }
        }

        // Get the block number from the parent inode (scoped borrow)
        let block_num = {
            let parent_inode = self
                .inode_table
                .get(parent as usize)
                .ok_or(KernelError::FsError(FsError::NotFound))?;
            let bn = parent_inode.direct_blocks[block_idx];
            if bn == 0 {
                return Err(KernelError::FsError(FsError::IoError));
            }
            bn
        };

        // Zero out the inode field in the on-disk entry to mark it deleted
        let block = &mut self.block_data[block_num as usize];
        block[offset] = 0;
        block[offset + 1] = 0;
        block[offset + 2] = 0;
        block[offset + 3] = 0;

        // Decrement link count on the target inode
        if let Some(target) = self.inode_table.get_mut(target_inode as usize) {
            if target.links_count > 0 {
                target.links_count -= 1;
            }

            // If unlinking a directory, also decrement parent link count (for "..")
            if is_dir {
                if let Some(p) = self.inode_table.get_mut(parent as usize) {
                    if p.links_count > 0 {
                        p.links_count -= 1;
                    }
                }
            }

            // If links reach 0, free all data blocks
            if self.inode_table[target_inode as usize].links_count == 0 {
                self.free_inode_blocks(target_inode);
            }
        }

        Ok(())
    }

    fn truncate_inode(&mut self, inode_num: u32, size: usize) -> Result<(), KernelError> {
        let old_size = {
            let inode = self
                .inode_table
                .get(inode_num as usize)
                .ok_or(KernelError::FsError(FsError::NotFound))?;
            inode.size as usize
        };

        // Set the new size
        self.inode_table[inode_num as usize].size = size as u32;

        // Free data blocks that are fully beyond the new size
        if size < old_size {
            // First block index that is no longer needed
            let first_free_block = if size == 0 {
                0
            } else {
                (size + BLOCK_SIZE - 1) / BLOCK_SIZE
            };

            // Collect block numbers to free (to avoid borrow conflicts)
            let mut blocks_to_free = Vec::new();
            for i in first_free_block..12 {
                let block_num = self.inode_table[inode_num as usize].direct_blocks[i];
                if block_num != 0 {
                    blocks_to_free.push((i, block_num));
                }
            }

            // Free the blocks
            for (idx, block_num) in blocks_to_free {
                self.free_block(block_num);
                self.inode_table[inode_num as usize].direct_blocks[idx] = 0;
                if self.inode_table[inode_num as usize].blocks > 0 {
                    self.inode_table[inode_num as usize].blocks -= 1;
                }
            }

            // If truncating to non-zero size within a block, zero the tail of that block
            if size > 0 {
                let tail_block_idx = (size - 1) / BLOCK_SIZE;
                let block_num = self.inode_table[inode_num as usize].direct_blocks[tail_block_idx];
                if block_num != 0 {
                    let zero_from = size % BLOCK_SIZE;
                    if zero_from > 0 {
                        let block = &mut self.block_data[block_num as usize];
                        for byte in &mut block[zero_from..BLOCK_SIZE] {
                            *byte = 0;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    // --- Helper methods for directory entry operations ---

    /// Read a DiskDirEntry from a block at the given byte offset.
    ///
    /// Parses the fixed header fields and name bytes from raw block data.
    fn read_dir_entry(&self, block: &[u8], offset: usize) -> DiskDirEntry {
        let inode = u32::from_le_bytes([
            block[offset],
            block[offset + 1],
            block[offset + 2],
            block[offset + 3],
        ]);
        let rec_len = u16::from_le_bytes([block[offset + 4], block[offset + 5]]);
        let name_len = block[offset + 6];
        let file_type = block[offset + 7];

        let mut name = [0u8; 255];
        let actual_name_len = (name_len as usize).min(MAX_FILENAME_LEN);
        let available = block.len() - (offset + DIR_ENTRY_HEADER_SIZE);
        let copy_len = actual_name_len.min(available);
        name[..copy_len].copy_from_slice(
            &block[offset + DIR_ENTRY_HEADER_SIZE..offset + DIR_ENTRY_HEADER_SIZE + copy_len],
        );

        DiskDirEntry {
            inode,
            rec_len,
            name_len,
            file_type,
            name,
        }
    }

    /// Find a directory entry by name within a directory inode.
    ///
    /// Returns the entry, the direct block index, and the byte offset within
    /// that block where the entry starts. Returns None if not found.
    fn find_dir_entry(&self, dir_inode: u32, name: &str) -> Option<(DiskDirEntry, usize, usize)> {
        let inode = self.inode_table.get(dir_inode as usize)?;

        if !inode.is_dir() {
            return None;
        }

        let dir_size = inode.size as usize;

        for i in 0..12 {
            let block_num = inode.direct_blocks[i];
            if block_num == 0 {
                break;
            }

            let block_start = i * BLOCK_SIZE;
            if block_start >= dir_size {
                break;
            }

            let block = &self.block_data[block_num as usize];
            let block_end = BLOCK_SIZE.min(dir_size - block_start);
            let mut offset = 0;

            while offset + DIR_ENTRY_HEADER_SIZE <= block_end {
                let entry = self.read_dir_entry(block, offset);
                let rec_len = entry.rec_len as usize;

                if rec_len < DIR_ENTRY_HEADER_SIZE || !rec_len.is_multiple_of(4) {
                    break;
                }

                if entry.inode != 0 && entry.name_len > 0 && entry.name_str() == name {
                    return Some((entry, i, offset));
                }

                offset += rec_len;
            }
        }

        None
    }

    /// Write a new directory entry into a directory inode's data blocks.
    ///
    /// Appends the entry at the end of the directory's current content.
    /// Allocates a new data block if needed.
    fn write_dir_entry(
        &mut self,
        dir_inode: u32,
        target_inode: u32,
        name: &str,
        file_type: u8,
    ) -> Result<(), KernelError> {
        let entry = DiskDirEntry::new(target_inode, name, file_type);
        let entry_size = align4(DIR_ENTRY_HEADER_SIZE + entry.name_len as usize);

        let dir_size = self.inode_table[dir_inode as usize].size as usize;

        // Determine which block to write into and at what offset
        let block_idx = dir_size / BLOCK_SIZE;
        let offset_in_block = dir_size % BLOCK_SIZE;

        if block_idx >= 12 {
            return Err(KernelError::ResourceExhausted {
                resource: "directory direct blocks",
            });
        }

        // Check if the entry fits in the current block
        if offset_in_block + entry_size > BLOCK_SIZE {
            // Need a new block; current block cannot fit this entry
            let next_block_idx = block_idx + 1;
            if next_block_idx >= 12 {
                return Err(KernelError::ResourceExhausted {
                    resource: "directory direct blocks",
                });
            }

            // Allocate a new block if not already present
            if self.inode_table[dir_inode as usize].direct_blocks[next_block_idx] == 0 {
                let new_block = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.inode_table[dir_inode as usize].direct_blocks[next_block_idx] = new_block;
                self.inode_table[dir_inode as usize].blocks += 1;
            }

            // Write at the start of the new block
            self.serialize_dir_entry(dir_inode, next_block_idx, 0, &entry, entry_size)?;

            // Update directory size to include any padding in the old block plus the new
            // entry
            let new_size = (next_block_idx * BLOCK_SIZE) + entry_size;
            self.inode_table[dir_inode as usize].size = new_size as u32;
        } else {
            // Allocate the first block if needed (empty directory)
            if self.inode_table[dir_inode as usize].direct_blocks[block_idx] == 0 {
                let new_block = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.inode_table[dir_inode as usize].direct_blocks[block_idx] = new_block;
                self.inode_table[dir_inode as usize].blocks += 1;
            }

            self.serialize_dir_entry(dir_inode, block_idx, offset_in_block, &entry, entry_size)?;

            // Update directory size
            let new_size = dir_size + entry_size;
            self.inode_table[dir_inode as usize].size = new_size as u32;
        }

        Ok(())
    }

    /// Serialize a DiskDirEntry into a specific block at a given offset.
    fn serialize_dir_entry(
        &mut self,
        dir_inode: u32,
        block_idx: usize,
        offset: usize,
        entry: &DiskDirEntry,
        entry_size: usize,
    ) -> Result<(), KernelError> {
        let block_num = self.inode_table[dir_inode as usize].direct_blocks[block_idx];
        if block_num == 0 {
            return Err(KernelError::FsError(FsError::IoError));
        }

        let block = &mut self.block_data[block_num as usize];

        // Write inode (4 bytes, little-endian)
        let inode_bytes = entry.inode.to_le_bytes();
        block[offset..offset + 4].copy_from_slice(&inode_bytes);

        // Write rec_len (2 bytes, little-endian) - use the padded entry_size
        let rec_len_bytes = (entry_size as u16).to_le_bytes();
        block[offset + 4..offset + 6].copy_from_slice(&rec_len_bytes);

        // Write name_len (1 byte)
        block[offset + 6] = entry.name_len;

        // Write file_type (1 byte)
        block[offset + 7] = entry.file_type;

        // Write name bytes
        let name_len = entry.name_len as usize;
        block[offset + DIR_ENTRY_HEADER_SIZE..offset + DIR_ENTRY_HEADER_SIZE + name_len]
            .copy_from_slice(&entry.name[..name_len]);

        // Zero-fill any padding bytes between name end and rec_len boundary
        let name_end = offset + DIR_ENTRY_HEADER_SIZE + name_len;
        let rec_end = offset + entry_size;
        for byte in &mut block[name_end..rec_end] {
            *byte = 0;
        }

        Ok(())
    }

    /// Free all data blocks belonging to an inode.
    fn free_inode_blocks(&mut self, inode_num: u32) {
        // Collect blocks to free
        let mut blocks_to_free = Vec::new();
        for i in 0..12 {
            let block_num = self.inode_table[inode_num as usize].direct_blocks[i];
            if block_num != 0 {
                blocks_to_free.push(i);
            }
        }

        for i in blocks_to_free {
            let block_num = self.inode_table[inode_num as usize].direct_blocks[i];
            self.free_block(block_num);
            self.inode_table[inode_num as usize].direct_blocks[i] = 0;
        }

        self.inode_table[inode_num as usize].blocks = 0;
        self.inode_table[inode_num as usize].size = 0;
    }
}

fn permissions_to_mode(perms: Permissions, is_dir: bool) -> u16 {
    let mut mode = 0u16;

    if is_dir {
        mode |= 0x4000;
    } else {
        mode |= 0x8000;
    }

    if perms.owner_read {
        mode |= 0o400;
    }
    if perms.owner_write {
        mode |= 0o200;
    }
    if perms.owner_exec {
        mode |= 0o100;
    }
    if perms.group_read {
        mode |= 0o040;
    }
    if perms.group_write {
        mode |= 0o020;
    }
    if perms.group_exec {
        mode |= 0o010;
    }
    if perms.other_read {
        mode |= 0o004;
    }
    if perms.other_write {
        mode |= 0o002;
    }
    if perms.other_exec {
        mode |= 0o001;
    }

    mode
}

/// BlockFS filesystem
pub struct BlockFs {
    inner: Arc<RwLock<BlockFsInner>>,
}

impl BlockFs {
    pub fn new(block_count: u32, inode_count: u32) -> Self {
        Self {
            inner: Arc::new(RwLock::new(BlockFsInner::new(block_count, inode_count))),
        }
    }

    pub fn format(block_count: u32, inode_count: u32) -> Result<Self, KernelError> {
        if block_count < 100 {
            return Err(KernelError::InvalidArgument {
                name: "block_count",
                value: "too small (minimum 100)",
            });
        }

        if inode_count < 10 {
            return Err(KernelError::InvalidArgument {
                name: "inode_count",
                value: "too small (minimum 10)",
            });
        }

        Ok(Self::new(block_count, inode_count))
    }
}

impl Filesystem for BlockFs {
    fn root(&self) -> Arc<dyn VfsNode> {
        Arc::new(BlockFsNode::new(0, self.inner.clone()))
    }

    fn name(&self) -> &str {
        "blockfs"
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn sync(&self) -> Result<(), KernelError> {
        // TODO(phase4): Sync dirty blocks and inodes to underlying block device
        Ok(())
    }
}

/// Initialize BlockFS
pub fn init() -> Result<(), KernelError> {
    println!("[BLOCKFS] Initializing block-based filesystem...");
    println!("[BLOCKFS] Block size: {} bytes", BLOCK_SIZE);
    println!("[BLOCKFS] Inode size: {} bytes", size_of::<DiskInode>());
    println!("[BLOCKFS] BlockFS initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_superblock_creation() {
        let sb = Superblock::new(10000, 1000);
        assert_eq!(sb.magic, BLOCKFS_MAGIC);
        assert!(sb.is_valid());
        assert_eq!(sb.block_count, 10000);
        assert_eq!(sb.inode_count, 1000);
    }

    #[test_case]
    fn test_block_bitmap() {
        let mut bitmap = BlockBitmap::new(100);

        let block1 = bitmap.allocate_block().unwrap();
        assert!(bitmap.is_allocated(block1));

        bitmap.free_block(block1);
        assert!(!bitmap.is_allocated(block1));
    }

    #[test_case]
    fn test_blockfs_format() {
        let fs = BlockFs::format(1000, 100).unwrap();
        assert_eq!(fs.name(), "blockfs");
        assert!(!fs.is_readonly());
    }
}
