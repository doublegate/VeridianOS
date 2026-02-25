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

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::mem::size_of;

use spin::Mutex;
#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use super::{DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode};
use crate::error::{FsError, KernelError};

/// Block size (4KB)
pub const BLOCK_SIZE: usize = 4096;

/// Number of direct block pointers in a DiskInode
pub const DIRECT_BLOCKS: usize = 12;

/// Number of block pointers that fit in one indirect block (4096 / 4 = 1024)
pub const PTRS_PER_BLOCK: usize = BLOCK_SIZE / size_of::<u32>();

/// Maximum file size addressable via direct blocks only: 12 * 4KB = 48KB
pub const DIRECT_MAX_BLOCKS: usize = DIRECT_BLOCKS;

/// Maximum file size addressable via direct + single indirect:
/// 12 + 1024 = 1036 blocks = ~4MB
pub const SINGLE_INDIRECT_MAX_BLOCKS: usize = DIRECT_BLOCKS + PTRS_PER_BLOCK;

/// Maximum file size addressable via direct + single + double indirect:
/// 12 + 1024 + 1024*1024 = 1_049_612 blocks = ~4GB
pub const DOUBLE_INDIRECT_MAX_BLOCKS: usize =
    DIRECT_BLOCKS + PTRS_PER_BLOCK + PTRS_PER_BLOCK * PTRS_PER_BLOCK;

/// Magic number for BlockFS
pub const BLOCKFS_MAGIC: u32 = 0x424C4B46; // "BLKF"

/// Maximum filename length
pub const MAX_FILENAME_LEN: usize = 255;

/// Number of 512-byte virtio sectors per 4KB BlockFS block
const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / 512;

/// On-disk superblock lives in block 0
const SUPERBLOCK_BLOCK: u32 = 0;

/// Serialized superblock size in bytes (fixed layout, LE)
const SUPERBLOCK_SERIALIZED_SIZE: usize = 62;

/// On-disk DiskInode size (96 bytes, repr(C), no padding gaps)
const DISK_INODE_SIZE: usize = 96;

/// Number of DiskInodes that fit in one 4KB block
const INODES_PER_BLOCK: usize = BLOCK_SIZE / DISK_INODE_SIZE; // 42

/// Compute number of blocks needed for the block bitmap.
/// Each byte covers 8 blocks, each block is 4096 bytes = 32768 bits.
fn bitmap_blocks(total_blocks: u32) -> u32 {
    let bits_needed = total_blocks as usize;
    let bytes_needed = (bits_needed + 7) / 8;
    ((bytes_needed + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32
}

/// Compute number of blocks needed for the inode table.
fn inode_table_blocks(inode_count: u32) -> u32 {
    ((inode_count as usize * DISK_INODE_SIZE + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32
}

/// Compute first_data_block from total_blocks and inode_count.
/// Layout: [superblock(1)] [bitmap(N)] [inode_table(M)] [data...]
fn computed_first_data_block(total_blocks: u32, inode_count: u32) -> u32 {
    1 + bitmap_blocks(total_blocks) + inode_table_blocks(inode_count)
}

// ---------------------------------------------------------------------------
// Disk backend trait -- abstracts block-level I/O for persistence
// ---------------------------------------------------------------------------

/// Trait for a block-level disk backend that BlockFS can sync to.
///
/// Operates on BlockFS-sized blocks (4KB). Implementations are responsible for
/// translating to the underlying device's sector size (typically 512 bytes).
pub trait DiskBackend: Send + Sync {
    /// Read a single 4KB block from the disk.
    ///
    /// `block_num` is the 0-based BlockFS block index.
    /// `buf` must be at least `BLOCK_SIZE` (4096) bytes.
    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> Result<(), KernelError>;

    /// Write a single 4KB block to the disk.
    ///
    /// `block_num` is the 0-based BlockFS block index.
    /// `data` must be at least `BLOCK_SIZE` (4096) bytes.
    fn write_block(&self, block_num: u64, data: &[u8]) -> Result<(), KernelError>;

    /// Total capacity in BlockFS-sized blocks (4KB each).
    fn block_count(&self) -> u64;

    /// Whether the device is read-only.
    fn is_read_only(&self) -> bool;
}

/// Adapter that wraps the global virtio-blk device as a `DiskBackend`.
///
/// Translates 4KB BlockFS blocks into 512-byte virtio sector reads/writes.
pub struct VirtioBlockBackend;

impl DiskBackend for VirtioBlockBackend {
    fn read_block(&self, block_num: u64, buf: &mut [u8]) -> Result<(), KernelError> {
        if buf.len() < BLOCK_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "buf",
                value: "buffer must be at least 4096 bytes",
            });
        }

        let device_lock =
            crate::drivers::virtio::blk::get_device().ok_or(KernelError::NotInitialized {
                subsystem: "virtio-blk",
            })?;
        let mut device = device_lock.lock();

        let base_sector = block_num * SECTORS_PER_BLOCK as u64;
        for i in 0..SECTORS_PER_BLOCK {
            let sector = base_sector + i as u64;
            let offset = i * 512;
            device.read_block(sector, &mut buf[offset..offset + 512])?;
        }

        Ok(())
    }

    fn write_block(&self, block_num: u64, data: &[u8]) -> Result<(), KernelError> {
        if data.len() < BLOCK_SIZE {
            return Err(KernelError::InvalidArgument {
                name: "data",
                value: "data must be at least 4096 bytes",
            });
        }

        let device_lock =
            crate::drivers::virtio::blk::get_device().ok_or(KernelError::NotInitialized {
                subsystem: "virtio-blk",
            })?;
        let mut device = device_lock.lock();

        let base_sector = block_num * SECTORS_PER_BLOCK as u64;
        for i in 0..SECTORS_PER_BLOCK {
            let sector = base_sector + i as u64;
            let offset = i * 512;
            device.write_block(sector, &data[offset..offset + 512])?;
        }

        Ok(())
    }

    fn block_count(&self) -> u64 {
        match crate::drivers::virtio::blk::get_device() {
            Some(lock) => {
                let device = lock.lock();
                device.capacity_sectors() / SECTORS_PER_BLOCK as u64
            }
            None => 0,
        }
    }

    fn is_read_only(&self) -> bool {
        match crate::drivers::virtio::blk::get_device() {
            Some(lock) => {
                let device = lock.lock();
                device.is_read_only()
            }
            None => true, // No device = effectively read-only
        }
    }
}

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
        let first_data = computed_first_data_block(block_count, inode_count);
        Self {
            magic: BLOCKFS_MAGIC,
            block_count,
            inode_count,
            free_blocks: block_count.saturating_sub(first_data),
            free_inodes: inode_count - 1, // Reserve root inode
            first_data_block: first_data,
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

    pub fn is_symlink(&self) -> bool {
        (self.mode & 0xA000) == 0xA000
    }

    pub fn node_type(&self) -> NodeType {
        if self.is_dir() {
            NodeType::Directory
        } else if self.is_symlink() {
            NodeType::Symlink
        } else {
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
    /// File type constant for symlinks
    pub const FT_SYMLINK: u8 = 7;

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
            Self::FT_SYMLINK => NodeType::Symlink,
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

    /// Create a symbolic link in this BlockFS directory.
    ///
    /// Currently returns `NotImplemented`. BlockFS has full symlink
    /// infrastructure in `BlockFsInner::create_symlink` (which allocates
    /// an inode with mode 0o120777 and stores the target as file data),
    /// but the VfsNode interface is not yet wired up. This will be
    /// connected in a future sprint.
    fn symlink(&self, _name: &str, _target: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::NotImplemented {
            feature: "blockfs symlink",
        })
    }

    /// Read the target of a symbolic link in BlockFS.
    ///
    /// Delegates to `BlockFsInner::read_symlink` which checks whether
    /// this inode has the symlink mode bit set (0xA000) and, if so, reads
    /// the target path from the inode's data blocks.
    ///
    /// Returns `FsError::NotASymlink` if this node is not a symlink.
    fn readlink(&self) -> Result<String, KernelError> {
        let fs = self.fs.read();
        fs.read_symlink(self.inode_num)
    }
}

/// Internal BlockFS state
pub struct BlockFsInner {
    superblock: Superblock,
    block_bitmap: BlockBitmap,
    inode_table: Vec<DiskInode>,
    block_data: Vec<Vec<u8>>, // In-memory block storage (RAM cache)
    /// Set of block indices that have been modified since the last sync.
    /// Used to track which blocks need to be written to the disk backend.
    dirty_blocks: BTreeSet<usize>,
    /// Optional disk backend for persistence. When `Some`, `sync()` writes
    /// dirty blocks to this device. When `None`, BlockFS operates as a pure
    /// RAM filesystem (all data lost on reboot).
    disk: Option<Arc<Mutex<dyn DiskBackend>>>,
}

impl BlockFsInner {
    pub fn new(block_count: u32, inode_count: u32) -> Self {
        let first_data = computed_first_data_block(block_count, inode_count);
        let mut superblock = Superblock::new(block_count, inode_count);
        superblock.first_data_block = first_data;
        superblock.free_blocks = block_count.saturating_sub(first_data);

        let mut block_bitmap = BlockBitmap::new(block_count as usize);

        // Mark metadata blocks (0..first_data_block) as allocated
        for _b in 0..first_data {
            block_bitmap.allocate_block();
        }

        let mut inode_table = Vec::new();
        inode_table.resize(inode_count as usize, DiskInode::new(0, 0, 0));

        // Initialize root directory (inode 0)
        // links_count = 2: one for itself (".") and one from the parent (root is its
        // own parent)
        let mut root_inode = DiskInode::new(0x41ED, 0, 0); // Directory, rwxr-xr-x
        root_inode.links_count = 2;
        inode_table[0] = root_inode;

        // Initialize block storage (sparse -- blocks materialized on first write)
        let mut block_data = Vec::with_capacity(block_count as usize);
        for _ in 0..block_count {
            block_data.push(Vec::new());
        }

        let mut fs = Self {
            superblock,
            block_bitmap,
            inode_table,
            block_data,
            dirty_blocks: BTreeSet::new(),
            disk: None,
        };

        // Create "." and ".." entries in the root directory (both point to inode 0)
        if let Err(_e) = fs.write_dir_entry(0, 0, ".", DiskDirEntry::FT_DIR) {
            crate::println!(
                "[BLOCKFS] Warning: failed to create '.' root dir entry: {:?}",
                _e
            );
        }
        if let Err(_e) = fs.write_dir_entry(0, 0, "..", DiskDirEntry::FT_DIR) {
            crate::println!(
                "[BLOCKFS] Warning: failed to create '..' root dir entry: {:?}",
                _e
            );
        }

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
        self.materialize_block(block as usize);
        Some(block)
    }

    fn free_block(&mut self, block: u32) {
        self.block_bitmap.free_block(block);
        self.superblock.free_blocks += 1;
        // Freed blocks no longer need syncing (data is logically gone)
        self.dirty_blocks.remove(&(block as usize));
    }

    /// Mark a block as dirty so it will be written to the disk backend on sync.
    fn mark_dirty(&mut self, block_num: u32) {
        self.dirty_blocks.insert(block_num as usize);
    }

    /// Ensure the block at `idx` is materialized (has 4KB allocated).
    /// Called before any write to a block. No-op if already materialized.
    fn materialize_block(&mut self, idx: usize) {
        if idx < self.block_data.len() && self.block_data[idx].is_empty() {
            self.block_data[idx] = vec![0u8; BLOCK_SIZE];
        }
    }

    /// Get a read-only reference to a block's data.
    /// Returns a reference to the shared zero block for unmaterialized entries,
    /// avoiding the need to allocate memory for blocks that have never been
    /// written.
    fn block_ref(&self, idx: usize) -> &[u8] {
        if idx < self.block_data.len() && !self.block_data[idx].is_empty() {
            &self.block_data[idx]
        } else {
            &ZERO_BLOCK
        }
    }

    /// Sync all dirty blocks and metadata to the disk backend.
    ///
    /// If no disk backend is configured, this is a no-op. Returns the number
    /// of blocks synced on success.
    fn sync_to_disk(&mut self) -> Result<usize, KernelError> {
        let disk = match self.disk {
            Some(ref d) => d.clone(),
            None => return Ok(0), // No backend -- pure RAM mode
        };

        let backend = disk.lock();
        if backend.is_read_only() {
            return Err(KernelError::FsError(FsError::ReadOnly));
        }

        let device_blocks = backend.block_count();
        let mut synced = 0usize;

        // Write all dirty data blocks
        let dirty: Vec<usize> = self.dirty_blocks.iter().copied().collect();
        for block_idx in &dirty {
            if *block_idx >= self.block_data.len() {
                continue; // Skip invalid indices
            }
            if (*block_idx as u64) >= device_blocks {
                // Block beyond device capacity -- skip but warn
                crate::println!(
                    "[BLOCKFS] Warning: dirty block {} exceeds device capacity {}",
                    block_idx,
                    device_blocks
                );
                continue;
            }
            // Skip unmaterialized (sparse) blocks -- they contain only zeros
            if self.block_data[*block_idx].is_empty() {
                continue;
            }
            backend.write_block(*block_idx as u64, &self.block_data[*block_idx])?;
            synced += 1;
        }

        // Clear dirty set after successful write
        self.dirty_blocks.clear();

        // Update superblock write time and mount count
        self.superblock.write_time = crate::arch::timer::read_hw_timestamp();

        // Write metadata (superblock, bitmap, inode table)
        self.serialize_superblock(&*backend)?;
        self.serialize_bitmap(&*backend)?;
        self.serialize_inode_table(&*backend)?;
        synced += 1; // Count metadata as one sync unit

        Ok(synced)
    }

    /// Load filesystem data from the disk backend into memory.
    ///
    /// Reads all data blocks from disk into the in-memory `block_data` array.
    /// This should be called after attaching a disk backend to populate the
    /// in-memory cache with persisted data.
    fn load_from_disk(&mut self) -> Result<usize, KernelError> {
        let disk = match self.disk {
            Some(ref d) => d.clone(),
            None => return Ok(0),
        };

        let backend = disk.lock();
        let device_blocks = backend.block_count();
        let fs_blocks = self.block_data.len() as u64;
        let blocks_to_read = fs_blocks.min(device_blocks) as usize;
        let mut loaded = 0usize;

        for block_idx in 0..blocks_to_read {
            if self.block_bitmap.is_allocated(block_idx as u32) {
                self.materialize_block(block_idx);
                backend.read_block(block_idx as u64, &mut self.block_data[block_idx])?;
                loaded += 1;
            }
        }

        Ok(loaded)
    }

    // --- On-disk metadata serialization ---

    /// Serialize the superblock to block 0 on disk (62 bytes, LE).
    fn serialize_superblock(&self, backend: &dyn DiskBackend) -> Result<(), KernelError> {
        let mut buf = [0u8; BLOCK_SIZE];
        let sb = &self.superblock;

        buf[0..4].copy_from_slice(&sb.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&sb.block_count.to_le_bytes());
        buf[8..12].copy_from_slice(&sb.inode_count.to_le_bytes());
        buf[12..16].copy_from_slice(&sb.free_blocks.to_le_bytes());
        buf[16..20].copy_from_slice(&sb.free_inodes.to_le_bytes());
        buf[20..24].copy_from_slice(&sb.first_data_block.to_le_bytes());
        buf[24..28].copy_from_slice(&sb.block_size.to_le_bytes());
        buf[28..30].copy_from_slice(&sb.inode_size.to_le_bytes());
        buf[30..34].copy_from_slice(&sb.blocks_per_group.to_le_bytes());
        buf[34..38].copy_from_slice(&sb.inodes_per_group.to_le_bytes());
        buf[38..46].copy_from_slice(&sb.mount_time.to_le_bytes());
        buf[46..54].copy_from_slice(&sb.write_time.to_le_bytes());
        buf[54..56].copy_from_slice(&sb.mount_count.to_le_bytes());
        buf[56..58].copy_from_slice(&sb.max_mount_count.to_le_bytes());
        buf[58..60].copy_from_slice(&sb.state.to_le_bytes());
        buf[60..62].copy_from_slice(&sb.errors.to_le_bytes());

        backend.write_block(SUPERBLOCK_BLOCK as u64, &buf)
    }

    /// Deserialize the superblock from block 0. Returns the parsed superblock.
    fn deserialize_superblock(backend: &dyn DiskBackend) -> Result<Superblock, KernelError> {
        let mut buf = [0u8; BLOCK_SIZE];
        backend.read_block(SUPERBLOCK_BLOCK as u64, &mut buf)?;

        let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != BLOCKFS_MAGIC {
            return Err(KernelError::FsError(FsError::CorruptedData));
        }

        Ok(Superblock {
            magic,
            block_count: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            inode_count: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            free_blocks: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            free_inodes: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            first_data_block: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
            block_size: u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]),
            inode_size: u16::from_le_bytes([buf[28], buf[29]]),
            blocks_per_group: u32::from_le_bytes([buf[30], buf[31], buf[32], buf[33]]),
            inodes_per_group: u32::from_le_bytes([buf[34], buf[35], buf[36], buf[37]]),
            mount_time: u64::from_le_bytes([
                buf[38], buf[39], buf[40], buf[41], buf[42], buf[43], buf[44], buf[45],
            ]),
            write_time: u64::from_le_bytes([
                buf[46], buf[47], buf[48], buf[49], buf[50], buf[51], buf[52], buf[53],
            ]),
            mount_count: u16::from_le_bytes([buf[54], buf[55]]),
            max_mount_count: u16::from_le_bytes([buf[56], buf[57]]),
            state: u16::from_le_bytes([buf[58], buf[59]]),
            errors: u16::from_le_bytes([buf[60], buf[61]]),
        })
    }

    /// Serialize the block bitmap to disk (blocks 1..1+bitmap_blocks).
    fn serialize_bitmap(&self, backend: &dyn DiskBackend) -> Result<(), KernelError> {
        let bm_blocks = bitmap_blocks(self.superblock.block_count);
        let bitmap_start = SUPERBLOCK_BLOCK + 1;

        for i in 0..bm_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            let byte_offset = i as usize * BLOCK_SIZE;
            let bytes_remaining = self.block_bitmap.bitmap.len().saturating_sub(byte_offset);
            let copy_len = bytes_remaining.min(BLOCK_SIZE);
            if copy_len > 0 {
                buf[..copy_len].copy_from_slice(
                    &self.block_bitmap.bitmap[byte_offset..byte_offset + copy_len],
                );
            }
            backend.write_block((bitmap_start + i) as u64, &buf)?;
        }

        Ok(())
    }

    /// Deserialize the block bitmap from disk.
    fn deserialize_bitmap(
        backend: &dyn DiskBackend,
        total_blocks: u32,
    ) -> Result<BlockBitmap, KernelError> {
        let bm_blocks = bitmap_blocks(total_blocks);
        let bitmap_start = SUPERBLOCK_BLOCK + 1;
        let bitmap_size = (total_blocks as usize + 7) / 8;
        let mut bitmap_data = vec![0u8; bitmap_size];

        for i in 0..bm_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            backend.read_block((bitmap_start + i) as u64, &mut buf)?;

            let byte_offset = i as usize * BLOCK_SIZE;
            let bytes_remaining = bitmap_size.saturating_sub(byte_offset);
            let copy_len = bytes_remaining.min(BLOCK_SIZE);
            if copy_len > 0 {
                bitmap_data[byte_offset..byte_offset + copy_len].copy_from_slice(&buf[..copy_len]);
            }
        }

        Ok(BlockBitmap {
            bitmap: bitmap_data,
            total_blocks: total_blocks as usize,
        })
    }

    /// Serialize a single DiskInode to 96 bytes (LE).
    fn serialize_disk_inode(inode: &DiskInode, buf: &mut [u8]) {
        buf[0..2].copy_from_slice(&inode.mode.to_le_bytes());
        buf[2..4].copy_from_slice(&inode.uid.to_le_bytes());
        buf[4..8].copy_from_slice(&inode.size.to_le_bytes());
        buf[8..12].copy_from_slice(&inode.atime.to_le_bytes());
        buf[12..16].copy_from_slice(&inode.ctime.to_le_bytes());
        buf[16..20].copy_from_slice(&inode.mtime.to_le_bytes());
        buf[20..24].copy_from_slice(&inode.dtime.to_le_bytes());
        buf[24..26].copy_from_slice(&inode.gid.to_le_bytes());
        buf[26..28].copy_from_slice(&inode.links_count.to_le_bytes());
        buf[28..32].copy_from_slice(&inode.blocks.to_le_bytes());
        buf[32..36].copy_from_slice(&inode.flags.to_le_bytes());
        for (j, &blk) in inode.direct_blocks.iter().enumerate() {
            let off = 36 + j * 4;
            buf[off..off + 4].copy_from_slice(&blk.to_le_bytes());
        }
        buf[84..88].copy_from_slice(&inode.indirect_block.to_le_bytes());
        buf[88..92].copy_from_slice(&inode.double_indirect_block.to_le_bytes());
        buf[92..96].copy_from_slice(&inode.triple_indirect_block.to_le_bytes());
    }

    /// Deserialize a single DiskInode from 96 bytes (LE).
    fn deserialize_disk_inode(buf: &[u8]) -> DiskInode {
        let mut direct_blocks = [0u32; 12];
        for (j, block) in direct_blocks.iter_mut().enumerate() {
            let off = 36 + j * 4;
            *block = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        }
        DiskInode {
            mode: u16::from_le_bytes([buf[0], buf[1]]),
            uid: u16::from_le_bytes([buf[2], buf[3]]),
            size: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            atime: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            ctime: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            mtime: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
            dtime: u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
            gid: u16::from_le_bytes([buf[24], buf[25]]),
            links_count: u16::from_le_bytes([buf[26], buf[27]]),
            blocks: u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]),
            flags: u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]),
            direct_blocks,
            indirect_block: u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]),
            double_indirect_block: u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]),
            triple_indirect_block: u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]),
        }
    }

    /// Serialize the entire inode table to disk.
    fn serialize_inode_table(&self, backend: &dyn DiskBackend) -> Result<(), KernelError> {
        let bm_blocks = bitmap_blocks(self.superblock.block_count);
        let inode_start = SUPERBLOCK_BLOCK + 1 + bm_blocks;
        let it_blocks = inode_table_blocks(self.superblock.inode_count);

        for blk_idx in 0..it_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            let base_inode = blk_idx as usize * INODES_PER_BLOCK;

            for slot in 0..INODES_PER_BLOCK {
                let inode_idx = base_inode + slot;
                if inode_idx >= self.inode_table.len() {
                    break;
                }
                let off = slot * DISK_INODE_SIZE;
                Self::serialize_disk_inode(
                    &self.inode_table[inode_idx],
                    &mut buf[off..off + DISK_INODE_SIZE],
                );
            }

            backend.write_block((inode_start + blk_idx) as u64, &buf)?;
        }

        Ok(())
    }

    /// Deserialize the entire inode table from disk.
    fn deserialize_inode_table(
        backend: &dyn DiskBackend,
        inode_count: u32,
        total_blocks: u32,
    ) -> Result<Vec<DiskInode>, KernelError> {
        let bm_blocks = bitmap_blocks(total_blocks);
        let inode_start = SUPERBLOCK_BLOCK + 1 + bm_blocks;
        let it_blocks = inode_table_blocks(inode_count);
        let mut inode_table = Vec::with_capacity(inode_count as usize);

        for blk_idx in 0..it_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            backend.read_block((inode_start + blk_idx) as u64, &mut buf)?;

            for slot in 0..INODES_PER_BLOCK {
                let inode_idx = blk_idx as usize * INODES_PER_BLOCK + slot;
                if inode_idx >= inode_count as usize {
                    break;
                }
                let off = slot * DISK_INODE_SIZE;
                inode_table.push(Self::deserialize_disk_inode(
                    &buf[off..off + DISK_INODE_SIZE],
                ));
            }
        }

        Ok(inode_table)
    }

    /// Load an existing BlockFS from a disk backend.
    ///
    /// Reads superblock, bitmap, inode table, and all data blocks.
    fn load_existing(backend: Arc<Mutex<dyn DiskBackend>>) -> Result<Self, KernelError> {
        let bk = backend.lock();

        // Read and validate superblock
        let superblock = Self::deserialize_superblock(&*bk)?;
        crate::println!(
            "[BLOCKFS] Found existing filesystem: {} blocks, {} inodes, first_data={}",
            superblock.block_count,
            superblock.inode_count,
            superblock.first_data_block
        );

        // Read bitmap
        let block_bitmap = Self::deserialize_bitmap(&*bk, superblock.block_count)?;

        // Read inode table
        let inode_table =
            Self::deserialize_inode_table(&*bk, superblock.inode_count, superblock.block_count)?;

        // Allocate sparse in-memory block storage and read only allocated blocks
        let block_count = superblock.block_count as usize;
        let mut block_data = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            block_data.push(Vec::new()); // Empty -- sparse
        }

        // Only load blocks that are marked as allocated in the bitmap
        let device_blocks = bk.block_count();
        let first_data = superblock.first_data_block as usize;
        let mut loaded = 0usize;
        for (i, block) in block_data
            .iter_mut()
            .enumerate()
            .take(block_count)
            .skip(first_data)
        {
            if (i as u64) >= device_blocks {
                break;
            }
            if block_bitmap.is_allocated(i as u32) {
                *block = vec![0u8; BLOCK_SIZE];
                bk.read_block(i as u64, block)?;
                loaded += 1;
            }
        }

        crate::println!(
            "[BLOCKFS] Loaded {} allocated data blocks from disk (sparse, {} total)",
            loaded,
            block_count.saturating_sub(first_data)
        );

        drop(bk);

        let mut fs = Self {
            superblock,
            block_bitmap,
            inode_table,
            block_data,
            dirty_blocks: BTreeSet::new(),
            disk: Some(backend),
        };

        // Update mount count and time
        fs.superblock.mount_count += 1;
        fs.superblock.mount_time = crate::arch::timer::read_hw_timestamp();

        Ok(fs)
    }

    // --- Indirect block helpers ---

    /// Read a u32 block pointer from position `index` within an indirect block.
    fn read_block_ptr(&self, indirect_block: u32, index: usize) -> u32 {
        let block = self.block_ref(indirect_block as usize);
        let off = index * size_of::<u32>();
        u32::from_le_bytes([block[off], block[off + 1], block[off + 2], block[off + 3]])
    }

    /// Write a u32 block pointer at position `index` within an indirect block.
    fn write_block_ptr(&mut self, indirect_block: u32, index: usize, value: u32) {
        let off = index * size_of::<u32>();
        let bytes = value.to_le_bytes();
        self.materialize_block(indirect_block as usize);
        self.block_data[indirect_block as usize][off..off + 4].copy_from_slice(&bytes);
        self.mark_dirty(indirect_block);
    }

    /// Resolve a logical block index to a physical block number for reading.
    ///
    /// Returns `Some(physical_block)` if the block is allocated, `None` if it
    /// falls in a sparse hole or exceeds the addressing range.
    fn resolve_block(&self, inode: &DiskInode, logical_block: usize) -> Option<u32> {
        if logical_block < DIRECT_BLOCKS {
            // Direct block
            let blk = inode.direct_blocks[logical_block];
            if blk == 0 {
                None
            } else {
                Some(blk)
            }
        } else if logical_block < SINGLE_INDIRECT_MAX_BLOCKS {
            // Single indirect
            let indirect = inode.indirect_block;
            if indirect == 0 {
                return None;
            }
            let idx = logical_block - DIRECT_BLOCKS;
            let blk = self.read_block_ptr(indirect, idx);
            if blk == 0 {
                None
            } else {
                Some(blk)
            }
        } else if logical_block < DOUBLE_INDIRECT_MAX_BLOCKS {
            // Double indirect
            let dbl_indirect = inode.double_indirect_block;
            if dbl_indirect == 0 {
                return None;
            }
            let rel = logical_block - SINGLE_INDIRECT_MAX_BLOCKS;
            let l1_idx = rel / PTRS_PER_BLOCK;
            let l2_idx = rel % PTRS_PER_BLOCK;
            let l1_block = self.read_block_ptr(dbl_indirect, l1_idx);
            if l1_block == 0 {
                return None;
            }
            let blk = self.read_block_ptr(l1_block, l2_idx);
            if blk == 0 {
                None
            } else {
                Some(blk)
            }
        } else {
            // Beyond double indirect range (triple indirect not implemented)
            None
        }
    }

    /// Ensure a logical block index has a physical block allocated, creating
    /// indirect blocks as needed. Returns the physical block number.
    fn ensure_block(&mut self, inode_num: u32, logical_block: usize) -> Result<u32, KernelError> {
        if logical_block < DIRECT_BLOCKS {
            // Direct block
            let blk = self.inode_table[inode_num as usize].direct_blocks[logical_block];
            if blk != 0 {
                return Ok(blk);
            }
            let new_blk = self
                .allocate_block()
                .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
            self.inode_table[inode_num as usize].direct_blocks[logical_block] = new_blk;
            self.inode_table[inode_num as usize].blocks += 1;
            self.mark_dirty(new_blk);
            Ok(new_blk)
        } else if logical_block < SINGLE_INDIRECT_MAX_BLOCKS {
            // Single indirect
            let mut indirect = self.inode_table[inode_num as usize].indirect_block;
            if indirect == 0 {
                indirect = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                // Zero the new indirect block (already zeroed by block_data init,
                // but be explicit for safety after reuse)
                for byte in &mut self.block_data[indirect as usize] {
                    *byte = 0;
                }
                self.mark_dirty(indirect);
                self.inode_table[inode_num as usize].indirect_block = indirect;
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let idx = logical_block - DIRECT_BLOCKS;
            let blk = self.read_block_ptr(indirect, idx);
            if blk != 0 {
                return Ok(blk);
            }
            let new_blk = self
                .allocate_block()
                .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
            self.write_block_ptr(indirect, idx, new_blk);
            self.mark_dirty(new_blk);
            self.inode_table[inode_num as usize].blocks += 1;
            Ok(new_blk)
        } else if logical_block < DOUBLE_INDIRECT_MAX_BLOCKS {
            // Double indirect
            let mut dbl_indirect = self.inode_table[inode_num as usize].double_indirect_block;
            if dbl_indirect == 0 {
                dbl_indirect = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                for byte in &mut self.block_data[dbl_indirect as usize] {
                    *byte = 0;
                }
                self.mark_dirty(dbl_indirect);
                self.inode_table[inode_num as usize].double_indirect_block = dbl_indirect;
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let rel = logical_block - SINGLE_INDIRECT_MAX_BLOCKS;
            let l1_idx = rel / PTRS_PER_BLOCK;
            let l2_idx = rel % PTRS_PER_BLOCK;
            let mut l1_block = self.read_block_ptr(dbl_indirect, l1_idx);
            if l1_block == 0 {
                l1_block = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                for byte in &mut self.block_data[l1_block as usize] {
                    *byte = 0;
                }
                self.mark_dirty(l1_block);
                self.write_block_ptr(dbl_indirect, l1_idx, l1_block);
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let blk = self.read_block_ptr(l1_block, l2_idx);
            if blk != 0 {
                return Ok(blk);
            }
            let new_blk = self
                .allocate_block()
                .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
            self.write_block_ptr(l1_block, l2_idx, new_blk);
            self.mark_dirty(new_blk);
            self.inode_table[inode_num as usize].blocks += 1;
            Ok(new_blk)
        } else {
            Err(KernelError::FsError(FsError::FileTooLarge))
        }
    }

    // --- Inode I/O ---

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
        let mut current_offset = offset;

        while bytes_read < to_read {
            let logical_block = current_offset / BLOCK_SIZE;
            let block_offset = current_offset % BLOCK_SIZE;

            match self.resolve_block(inode, logical_block) {
                Some(block_num) => {
                    let block = self.block_ref(block_num as usize);
                    let copy_len = (BLOCK_SIZE - block_offset).min(to_read - bytes_read);
                    buffer[bytes_read..bytes_read + copy_len]
                        .copy_from_slice(&block[block_offset..block_offset + copy_len]);
                    bytes_read += copy_len;
                    current_offset += copy_len;
                }
                None => {
                    // Sparse hole or beyond addressing range -- fill with zeros
                    let copy_len = (BLOCK_SIZE - block_offset).min(to_read - bytes_read);
                    for byte in &mut buffer[bytes_read..bytes_read + copy_len] {
                        *byte = 0;
                    }
                    bytes_read += copy_len;
                    current_offset += copy_len;
                }
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
        let mut blocks_needed: Vec<(usize, usize, usize)> = Vec::new();
        let mut current_offset = offset;
        let mut bytes_remaining = data.len();

        // Determine which blocks we need (up to double-indirect limit)
        while bytes_remaining > 0 {
            let logical_block = current_offset / BLOCK_SIZE;
            if logical_block >= DOUBLE_INDIRECT_MAX_BLOCKS {
                break; // Beyond addressable range
            }

            let block_offset = current_offset % BLOCK_SIZE;
            let copy_len = (BLOCK_SIZE - block_offset).min(bytes_remaining);

            blocks_needed.push((logical_block, block_offset, copy_len));

            bytes_remaining -= copy_len;
            current_offset += copy_len;
        }

        // Ensure all required blocks are allocated and collect physical block numbers
        let mut block_numbers: Vec<u32> = Vec::new();
        for (logical_block, _, _) in &blocks_needed {
            let phys_block = self.ensure_block(inode_num, *logical_block)?;
            block_numbers.push(phys_block);
        }

        // Write data to blocks
        let mut bytes_written = 0;
        for (i, (_, block_offset, copy_len)) in blocks_needed.iter().enumerate() {
            let block_num = block_numbers[i];
            self.block_data[block_num as usize][*block_offset..*block_offset + *copy_len]
                .copy_from_slice(&data[bytes_written..bytes_written + *copy_len]);
            self.mark_dirty(block_num);
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
            inode: inode_num as u64,
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

            let block = self.block_ref(block_num as usize);
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

    /// Create a symbolic link inode in the given parent directory.
    ///
    /// Allocates a new inode with symlink mode (0o120777), stores `target`
    /// as the inode's file data (the symlink target path), and adds a
    /// directory entry of type `FT_SYMLINK` in the parent.
    ///
    /// # Arguments
    /// - `parent`: Inode number of the parent directory.
    /// - `name`: Name of the symlink entry in the parent directory.
    /// - `target`: The target path that the symlink points to.
    ///
    /// # Returns
    /// The inode number of the newly created symlink.
    fn create_symlink(
        &mut self,
        parent: u32,
        name: &str,
        target: &str,
    ) -> Result<u32, KernelError> {
        if name.is_empty() || name.len() > MAX_FILENAME_LEN {
            return Err(KernelError::InvalidArgument {
                name: "symlink",
                value: "empty or too long",
            });
        }
        if self.find_dir_entry(parent, name).is_some() {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        let inode_num = self
            .allocate_inode()
            .ok_or(KernelError::ResourceExhausted { resource: "inodes" })?;

        // Symlink inode: mode 0o120000 | 0o777 (rwx for all)
        let mut inode = DiskInode::new(0o120000 | 0o777, 0, 0);
        inode.links_count = 1;
        inode.size = target.len() as u32;
        self.inode_table[inode_num as usize] = inode;

        // Store target contents as file data
        self.write_inode(inode_num, 0, target.as_bytes())?;

        // Add dir entry in parent
        if let Err(e) = self.write_dir_entry(parent, inode_num, name, DiskDirEntry::FT_SYMLINK) {
            self.inode_table[inode_num as usize].links_count = 0;
            self.superblock.free_inodes += 1;
            return Err(e);
        }

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
        self.materialize_block(block_num as usize);
        let block = &mut self.block_data[block_num as usize];
        block[offset] = 0;
        block[offset + 1] = 0;
        block[offset + 2] = 0;
        block[offset + 3] = 0;
        self.mark_dirty(block_num);

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
            // First logical block index that is no longer needed
            let first_free_block = if size == 0 {
                0
            } else {
                (size + BLOCK_SIZE - 1) / BLOCK_SIZE
            };

            // Free direct blocks beyond the new size
            let direct_start = first_free_block.min(DIRECT_BLOCKS);
            for i in direct_start..DIRECT_BLOCKS {
                let block_num = self.inode_table[inode_num as usize].direct_blocks[i];
                if block_num != 0 {
                    self.free_block(block_num);
                    self.inode_table[inode_num as usize].direct_blocks[i] = 0;
                    if self.inode_table[inode_num as usize].blocks > 0 {
                        self.inode_table[inode_num as usize].blocks -= 1;
                    }
                }
            }

            // Free single-indirect blocks beyond the new size
            self.truncate_single_indirect(inode_num, first_free_block);

            // Free double-indirect blocks beyond the new size
            self.truncate_double_indirect(inode_num, first_free_block);

            // If truncating to non-zero size within a block, zero the tail
            if size > 0 {
                let tail_logical_block = (size - 1) / BLOCK_SIZE;
                // Resolve using the inode (borrow scoped to avoid conflicts)
                let phys_block = {
                    let inode = &self.inode_table[inode_num as usize];
                    self.resolve_block(inode, tail_logical_block)
                };
                if let Some(block_num) = phys_block {
                    let zero_from = size % BLOCK_SIZE;
                    if zero_from > 0 {
                        self.materialize_block(block_num as usize);
                        let block = &mut self.block_data[block_num as usize];
                        for byte in &mut block[zero_from..BLOCK_SIZE] {
                            *byte = 0;
                        }
                        self.mark_dirty(block_num);
                    }
                }
            }
        }

        Ok(())
    }

    /// Read the target of a symlink inode.
    ///
    /// Reads the inode's data content (which contains the symlink target
    /// path stored at creation time) and returns it as a `String`.
    ///
    /// # Returns
    /// - `Ok(String)`: The symlink target path.
    /// - `Err(FsError::NotASymlink)`: The inode is not a symlink.
    /// - `Err(FsError::NotFound)`: The inode does not exist.
    /// - `Err(FsError::InvalidPath)`: The stored target is not valid UTF-8.
    fn read_symlink(&self, inode_num: u32) -> Result<String, KernelError> {
        let inode = self
            .inode_table
            .get(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;
        if !inode.is_symlink() {
            return Err(KernelError::FsError(FsError::NotASymlink));
        }

        let mut buf = vec![0u8; inode.size as usize];
        let read = self.read_inode(inode_num, 0, &mut buf)?;
        buf.truncate(read);
        let s =
            core::str::from_utf8(&buf).map_err(|_| KernelError::FsError(FsError::InvalidPath))?;
        Ok(s.to_string())
    }

    /// Free single-indirect data blocks at or beyond `first_free_block`.
    /// Also frees the indirect block itself if it becomes fully empty.
    fn truncate_single_indirect(&mut self, inode_num: u32, first_free_block: usize) {
        let indirect = self.inode_table[inode_num as usize].indirect_block;
        if indirect == 0 {
            return;
        }

        // If all indirect entries are being freed
        if first_free_block <= DIRECT_BLOCKS {
            // Free every data block referenced by the indirect block
            for idx in 0..PTRS_PER_BLOCK {
                let blk = self.read_block_ptr(indirect, idx);
                if blk != 0 {
                    self.free_block(blk);
                    if self.inode_table[inode_num as usize].blocks > 0 {
                        self.inode_table[inode_num as usize].blocks -= 1;
                    }
                }
            }
            // Free the indirect block itself
            self.free_block(indirect);
            self.inode_table[inode_num as usize].indirect_block = 0;
            if self.inode_table[inode_num as usize].blocks > 0 {
                self.inode_table[inode_num as usize].blocks -= 1;
            }
        } else if first_free_block < SINGLE_INDIRECT_MAX_BLOCKS {
            // Partial free within the single-indirect range
            let start_idx = first_free_block - DIRECT_BLOCKS;
            let mut any_remain = false;
            for idx in 0..PTRS_PER_BLOCK {
                if idx >= start_idx {
                    let blk = self.read_block_ptr(indirect, idx);
                    if blk != 0 {
                        self.free_block(blk);
                        self.write_block_ptr(indirect, idx, 0);
                        if self.inode_table[inode_num as usize].blocks > 0 {
                            self.inode_table[inode_num as usize].blocks -= 1;
                        }
                    }
                } else {
                    let blk = self.read_block_ptr(indirect, idx);
                    if blk != 0 {
                        any_remain = true;
                    }
                }
            }
            // If no entries remain, free the indirect block itself
            if !any_remain {
                self.free_block(indirect);
                self.inode_table[inode_num as usize].indirect_block = 0;
                if self.inode_table[inode_num as usize].blocks > 0 {
                    self.inode_table[inode_num as usize].blocks -= 1;
                }
            }
        }
        // If first_free_block >= SINGLE_INDIRECT_MAX_BLOCKS, nothing in the
        // single-indirect range needs freeing.
    }

    /// Free double-indirect data blocks at or beyond `first_free_block`.
    /// Also frees level-1 indirect blocks and the double-indirect block itself
    /// if they become fully empty.
    fn truncate_double_indirect(&mut self, inode_num: u32, first_free_block: usize) {
        let dbl_indirect = self.inode_table[inode_num as usize].double_indirect_block;
        if dbl_indirect == 0 {
            return;
        }

        // Logical block range covered by double indirect:
        //   [SINGLE_INDIRECT_MAX_BLOCKS .. DOUBLE_INDIRECT_MAX_BLOCKS)

        if first_free_block <= SINGLE_INDIRECT_MAX_BLOCKS {
            // Free everything in double-indirect range
            for l1_idx in 0..PTRS_PER_BLOCK {
                let l1_block = self.read_block_ptr(dbl_indirect, l1_idx);
                if l1_block != 0 {
                    // Free all data blocks in this L1 indirect block
                    for l2_idx in 0..PTRS_PER_BLOCK {
                        let data_blk = self.read_block_ptr(l1_block, l2_idx);
                        if data_blk != 0 {
                            self.free_block(data_blk);
                            if self.inode_table[inode_num as usize].blocks > 0 {
                                self.inode_table[inode_num as usize].blocks -= 1;
                            }
                        }
                    }
                    // Free the L1 indirect block itself
                    self.free_block(l1_block);
                    if self.inode_table[inode_num as usize].blocks > 0 {
                        self.inode_table[inode_num as usize].blocks -= 1;
                    }
                }
            }
            // Free the double-indirect block itself
            self.free_block(dbl_indirect);
            self.inode_table[inode_num as usize].double_indirect_block = 0;
            if self.inode_table[inode_num as usize].blocks > 0 {
                self.inode_table[inode_num as usize].blocks -= 1;
            }
        } else if first_free_block < DOUBLE_INDIRECT_MAX_BLOCKS {
            // Partial free within the double-indirect range
            let rel = first_free_block - SINGLE_INDIRECT_MAX_BLOCKS;
            let first_l1 = rel / PTRS_PER_BLOCK;
            let first_l2 = rel % PTRS_PER_BLOCK;
            let mut any_l1_remain = false;

            for l1_idx in 0..PTRS_PER_BLOCK {
                let l1_block = self.read_block_ptr(dbl_indirect, l1_idx);
                if l1_block == 0 {
                    continue;
                }

                if l1_idx < first_l1 {
                    // Entirely before the truncation point -- keep
                    any_l1_remain = true;
                    continue;
                }

                let l2_start = if l1_idx == first_l1 { first_l2 } else { 0 };

                let mut any_l2_remain = false;
                for l2_idx in 0..PTRS_PER_BLOCK {
                    if l2_idx >= l2_start {
                        let data_blk = self.read_block_ptr(l1_block, l2_idx);
                        if data_blk != 0 {
                            self.free_block(data_blk);
                            self.write_block_ptr(l1_block, l2_idx, 0);
                            if self.inode_table[inode_num as usize].blocks > 0 {
                                self.inode_table[inode_num as usize].blocks -= 1;
                            }
                        }
                    } else {
                        let data_blk = self.read_block_ptr(l1_block, l2_idx);
                        if data_blk != 0 {
                            any_l2_remain = true;
                        }
                    }
                }

                if !any_l2_remain {
                    // Free the now-empty L1 indirect block
                    self.free_block(l1_block);
                    self.write_block_ptr(dbl_indirect, l1_idx, 0);
                    if self.inode_table[inode_num as usize].blocks > 0 {
                        self.inode_table[inode_num as usize].blocks -= 1;
                    }
                } else {
                    any_l1_remain = true;
                }
            }

            if !any_l1_remain {
                self.free_block(dbl_indirect);
                self.inode_table[inode_num as usize].double_indirect_block = 0;
                if self.inode_table[inode_num as usize].blocks > 0 {
                    self.inode_table[inode_num as usize].blocks -= 1;
                }
            }
        }
        // If first_free_block >= DOUBLE_INDIRECT_MAX_BLOCKS, nothing in the
        // double-indirect range needs freeing.
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

            let block = self.block_ref(block_num as usize);
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

        self.materialize_block(block_num as usize);
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

        self.mark_dirty(block_num);

        Ok(())
    }

    /// Free all data blocks belonging to an inode (direct + indirect).
    fn free_inode_blocks(&mut self, inode_num: u32) {
        // Free direct blocks
        for i in 0..DIRECT_BLOCKS {
            let block_num = self.inode_table[inode_num as usize].direct_blocks[i];
            if block_num != 0 {
                self.free_block(block_num);
                self.inode_table[inode_num as usize].direct_blocks[i] = 0;
            }
        }

        // Free single-indirect and double-indirect blocks (first_free_block=0 frees
        // all)
        self.truncate_single_indirect(inode_num, 0);
        self.truncate_double_indirect(inode_num, 0);

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

/// A shared zero block for reads of unmaterialized (sparse) blocks.
/// Avoids allocating 4KB for every unoccupied block index.
static ZERO_BLOCK: [u8; BLOCK_SIZE] = [0u8; BLOCK_SIZE];

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

    /// Open an existing BlockFS from a disk backend.
    ///
    /// Reads the superblock, validates the magic number, and loads the bitmap,
    /// inode table, and all data blocks into memory. The disk backend remains
    /// attached for subsequent sync operations.
    pub fn open_existing(backend: Arc<Mutex<dyn DiskBackend>>) -> Result<Self, KernelError> {
        let inner = BlockFsInner::load_existing(backend)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
        })
    }

    /// Attach a disk backend for persistent storage.
    ///
    /// When a disk backend is attached, `sync()` will write all dirty blocks
    /// to the device. Without a backend, BlockFS operates as a pure RAM
    /// filesystem.
    ///
    /// If `load` is true, existing data is read from the disk into memory.
    pub fn set_disk_backend(
        &self,
        backend: Arc<Mutex<dyn DiskBackend>>,
        load: bool,
    ) -> Result<(), KernelError> {
        let mut inner = self.inner.write();
        inner.disk = Some(backend);

        if load {
            let loaded = inner.load_from_disk()?;
            crate::println!("[BLOCKFS] Loaded {} blocks from disk backend", loaded);
        }

        Ok(())
    }

    /// Detach the disk backend. Outstanding dirty blocks will NOT be flushed;
    /// call `sync()` first if persistence is needed.
    pub fn detach_disk_backend(&self) {
        let mut inner = self.inner.write();
        inner.disk = None;
    }

    /// Get the number of dirty blocks pending sync.
    pub fn dirty_block_count(&self) -> usize {
        let inner = self.inner.read();
        inner.dirty_blocks.len()
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
        let mut inner = self.inner.write();
        let synced = inner.sync_to_disk()?;
        if synced > 0 {
            crate::println!("[BLOCKFS] Synced {} dirty blocks to disk", synced);
        }
        Ok(())
    }
}

/// Initialize BlockFS
pub fn init() -> Result<(), KernelError> {
    crate::println!("[BLOCKFS] Initializing block-based filesystem...");
    crate::println!("[BLOCKFS] Block size: {} bytes", BLOCK_SIZE);
    crate::println!("[BLOCKFS] Inode size: {} bytes", size_of::<DiskInode>());
    crate::println!("[BLOCKFS] BlockFS initialized");
    Ok(())
}

/// Try to attach the virtio-blk device as a disk backend for the given
/// BlockFS instance. Returns `true` if a device was found and attached.
///
/// This should be called after both the BlockFS and virtio-blk driver have
/// been initialized.
pub fn attach_virtio_backend(fs: &BlockFs, load_from_disk: bool) -> bool {
    if !crate::drivers::virtio::blk::is_initialized() {
        crate::println!("[BLOCKFS] No virtio-blk device available; operating in RAM-only mode");
        return false;
    }

    let backend = Arc::new(Mutex::new(VirtioBlockBackend));
    match fs.set_disk_backend(backend, load_from_disk) {
        Ok(()) => {
            crate::println!("[BLOCKFS] Attached virtio-blk disk backend for persistence");
            true
        }
        Err(e) => {
            crate::println!("[BLOCKFS] Failed to attach virtio-blk backend: {:?}", e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_superblock_creation() {
        let sb = Superblock::new(10000, 1000);
        assert_eq!(sb.magic, BLOCKFS_MAGIC);
        assert!(sb.is_valid());
        assert_eq!(sb.block_count, 10000);
        assert_eq!(sb.inode_count, 1000);
    }

    #[test]
    fn test_block_bitmap() {
        let mut bitmap = BlockBitmap::new(100);

        let block1 = bitmap.allocate_block().unwrap();
        assert!(bitmap.is_allocated(block1));

        bitmap.free_block(block1);
        assert!(!bitmap.is_allocated(block1));
    }

    #[test]
    fn test_blockfs_format() {
        let fs = BlockFs::format(1000, 100).unwrap();
        assert_eq!(fs.name(), "blockfs");
        assert!(!fs.is_readonly());
    }
}
