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
    clippy::if_same_then_else,
    clippy::manual_div_ceil,
    clippy::slow_vector_initialization,
    clippy::manual_saturating_arithmetic,
    clippy::implicit_saturating_sub
)]

use alloc::{sync::Arc, vec, vec::Vec};
use core::mem::size_of;

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};

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
        } else if self.is_file() {
            NodeType::File
        } else {
            NodeType::File // Default
        }
    }
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

    fn read(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let fs = self.fs.read();
        fs.read_inode(self.inode_num, offset, buffer)
    }

    fn write(&self, offset: usize, data: &[u8]) -> Result<usize, &'static str> {
        let mut fs = self.fs.write();
        fs.write_inode(self.inode_num, offset, data)
    }

    fn metadata(&self) -> Result<Metadata, &'static str> {
        let fs = self.fs.read();
        fs.get_metadata(self.inode_num)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, &'static str> {
        let fs = self.fs.read();
        fs.readdir(self.inode_num)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn VfsNode>, &'static str> {
        let fs = self.fs.read();
        let child_inode = fs.lookup_in_dir(self.inode_num, name)?;
        Ok(Arc::new(BlockFsNode::new(child_inode, self.fs.clone())))
    }

    fn create(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        let mut fs = self.fs.write();
        let new_inode = fs.create_file(self.inode_num, name, permissions)?;
        Ok(Arc::new(BlockFsNode::new(new_inode, self.fs.clone())))
    }

    fn mkdir(
        &self,
        name: &str,
        permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, &'static str> {
        let mut fs = self.fs.write();
        let new_inode = fs.create_directory(self.inode_num, name, permissions)?;
        Ok(Arc::new(BlockFsNode::new(new_inode, self.fs.clone())))
    }

    fn unlink(&self, name: &str) -> Result<(), &'static str> {
        let mut fs = self.fs.write();
        fs.unlink_from_dir(self.inode_num, name)
    }

    fn truncate(&self, size: usize) -> Result<(), &'static str> {
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
        inode_table[0] = DiskInode::new(0x41ED, 0, 0); // Directory, rwxr-xr-x

        // Initialize block storage
        let mut block_data = Vec::new();
        for _ in 0..block_count {
            block_data.push(vec![0u8; BLOCK_SIZE]);
        }

        Self {
            superblock,
            block_bitmap,
            inode_table,
            block_data,
        }
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
    ) -> Result<usize, &'static str> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or("Invalid inode")?;

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
    ) -> Result<usize, &'static str> {
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
                let new_block = self.allocate_block().ok_or("No free blocks")?;
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

    fn get_metadata(&self, inode_num: u32) -> Result<Metadata, &'static str> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or("Invalid inode")?;

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

    fn readdir(&self, inode_num: u32) -> Result<Vec<DirEntry>, &'static str> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or("Invalid inode")?;

        if !inode.is_dir() {
            return Err("Not a directory");
        }

        // TODO(phase4): Parse directory entries from on-disk block data
        Ok(Vec::new())
    }

    fn lookup_in_dir(&self, _dir_inode: u32, _name: &str) -> Result<u32, &'static str> {
        // TODO(phase4): Implement directory entry lookup by name
        Err("Not found")
    }

    fn create_file(
        &mut self,
        _parent: u32,
        _name: &str,
        permissions: Permissions,
    ) -> Result<u32, &'static str> {
        let inode_num = self.allocate_inode().ok_or("No free inodes")?;

        let mode = permissions_to_mode(permissions, false);
        self.inode_table[inode_num as usize] = DiskInode::new(mode, 0, 0);

        // TODO(phase4): Add directory entry to parent inode

        Ok(inode_num)
    }

    fn create_directory(
        &mut self,
        _parent: u32,
        _name: &str,
        permissions: Permissions,
    ) -> Result<u32, &'static str> {
        let inode_num = self.allocate_inode().ok_or("No free inodes")?;

        let mode = permissions_to_mode(permissions, true);
        self.inode_table[inode_num as usize] = DiskInode::new(mode, 0, 0);

        // TODO(phase4): Add directory entry to parent and create . and .. entries

        Ok(inode_num)
    }

    fn unlink_from_dir(&mut self, _parent: u32, _name: &str) -> Result<(), &'static str> {
        // TODO(phase4): Implement file unlinking (remove dir entry, decrement link
        // count)
        Ok(())
    }

    fn truncate_inode(&mut self, inode_num: u32, size: usize) -> Result<(), &'static str> {
        let inode = self
            .inode_table
            .get_mut(inode_num as usize)
            .ok_or("Invalid inode")?;

        inode.size = size as u32;
        // TODO(phase4): Free data blocks beyond the new truncated size

        Ok(())
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

    pub fn format(block_count: u32, inode_count: u32) -> Result<Self, &'static str> {
        if block_count < 100 {
            return Err("Block count too small");
        }

        if inode_count < 10 {
            return Err("Inode count too small");
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

    fn sync(&self) -> Result<(), &'static str> {
        // TODO(phase4): Sync dirty blocks and inodes to underlying block device
        Ok(())
    }
}

/// Initialize BlockFS
pub fn init() -> Result<(), &'static str> {
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
