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

mod cache;
use cache::{BlockCache, Disk};

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

        // One request per 4 KiB block (was eight 512-byte requests).
        device.read_sectors(block_num * SECTORS_PER_BLOCK as u64, &mut buf[..BLOCK_SIZE])?;

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

        device.write_sectors(block_num * SECTORS_PER_BLOCK as u64, &data[..BLOCK_SIZE])?;

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

    /// An unused inode-table slot. `new` starts at one link, which marks
    /// the slot as in use, so a table filled with `new(0, 0, 0)` had no
    /// allocatable inode at all.
    pub fn free() -> Self {
        Self {
            links_count: 0,
            ..Self::new(0, 0, 0)
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

    fn symlink(&self, name: &str, target: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        let mut fs = self.fs.write();
        let new_inode = fs.create_symlink(self.inode_num, name, target)?;
        Ok(Arc::new(BlockFsNode::new(new_inode, self.fs.clone())))
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

    fn chmod(&self, permissions: Permissions) -> Result<(), KernelError> {
        let mut fs = self.fs.write();
        fs.chmod_inode(self.inode_num, permissions)
    }

    fn chown(&self, uid: Option<u32>, gid: Option<u32>) -> Result<(), KernelError> {
        let mut fs = self.fs.write();
        fs.chown_inode(self.inode_num, uid, gid)
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn rename(
        &self,
        old_name: &str,
        new_parent: &Arc<dyn VfsNode>,
        new_name: &str,
    ) -> Result<(), KernelError> {
        let np = new_parent
            .as_any()
            .and_then(|a| a.downcast_ref::<BlockFsNode>())
            .filter(|np| Arc::ptr_eq(&np.fs, &self.fs))
            .ok_or(KernelError::FsError(FsError::CrossDevice))?;
        self.fs
            .write()
            .rename_entry(self.inode_num, old_name, np.inode_num, new_name)
    }

    fn link(&self, name: &str, target: Arc<dyn VfsNode>) -> Result<(), KernelError> {
        // A hard link can only name an inode of this same filesystem. The
        // inode number alone does not prove that (another filesystem can
        // use the same numbers), so require the same BlockFS instance (W-18).
        let same_fs = target
            .as_any()
            .and_then(|a| a.downcast_ref::<BlockFsNode>())
            .is_some_and(|t| Arc::ptr_eq(&t.fs, &self.fs));
        if !same_fs {
            return Err(KernelError::FsError(FsError::CrossDevice));
        }

        // Extract metadata BEFORE acquiring the write lock on BlockFsInner.
        // The target node may share the same Arc<RwLock<BlockFsInner>>, so
        // calling target.metadata() while holding the write lock would deadlock
        // (metadata() takes a read lock on the same RwLock).
        let target_meta = target.metadata()?;
        let target_node_type = target.node_type();

        let mut fs = self.fs.write();
        fs.link_in_dir_with_meta(self.inode_num, name, &target_meta, target_node_type)
    }
}

/// Internal BlockFS state
pub struct BlockFsInner {
    superblock: Superblock,
    block_bitmap: BlockBitmap,
    inode_table: Vec<DiskInode>,
    /// Data blocks, loaded on first use and written back on eviction or
    /// sync (FS-PERF-01). Locked separately so readers holding the
    /// filesystem's read lock can still fill it.
    cache: RwLock<BlockCache>,
    /// Optional disk backend for persistence. When `None`, BlockFS operates
    /// as a pure RAM filesystem (all data lost on reboot) and the cache
    /// holds every block.
    disk: Option<Arc<Mutex<dyn DiskBackend>>>,
    /// Device size in blocks, sampled when the backend is attached.
    device_blocks: u64,
    /// Whether the backend accepts writes, sampled when attached.
    disk_writable: bool,
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
        inode_table.resize(inode_count as usize, DiskInode::free());

        // Initialize root directory (inode 0)
        // links_count = 2: one for itself (".") and one from the parent (root is its
        // own parent)
        let mut root_inode = DiskInode::new(0x41ED, 0, 0); // Directory, rwxr-xr-x
        root_inode.links_count = 2;
        inode_table[0] = root_inode;

        let mut fs = Self {
            superblock,
            block_bitmap,
            inode_table,
            cache: RwLock::new(BlockCache::new(
                block_count as usize,
                cache::DEFAULT_CAPACITY_BLOCKS,
            )),
            disk: None,
            device_blocks: 0,
            disk_writable: false,
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

    /// Allocate a block. Its contents start as zeros and are dirty, so the
    /// old bytes on the disk can never be read back through it.
    fn allocate_block(&mut self) -> Option<u32> {
        let block = self.block_bitmap.allocate_block()?;
        if self.cache.get_mut().insert_zeroed(block).is_err() {
            self.block_bitmap.free_block(block);
            return None;
        }
        self.superblock.free_blocks -= 1;
        Some(block)
    }

    fn free_block(&mut self, block: u32) {
        self.block_bitmap.free_block(block);
        self.superblock.free_blocks += 1;
        // The data is logically gone: drop it without writing it back.
        self.cache.get_mut().discard(block);
    }

    fn disk_view<'a>(&self, backend: Option<&'a dyn DiskBackend>) -> Option<Disk<'a>> {
        backend.map(|backend| Disk {
            backend,
            device_blocks: self.device_blocks,
            writable: self.disk_writable,
        })
    }

    /// Run `f` on a block's contents, reading it from the disk on a miss.
    /// Unallocated blocks read as zeros.
    fn with_block<R>(&self, idx: u32, f: impl FnOnce(&[u8]) -> R) -> Result<R, KernelError> {
        let allocated = self.block_bitmap.is_allocated(idx);
        let guard = self.disk.as_ref().map(|d| d.lock());
        let view = self.disk_view(guard.as_deref());
        self.cache.write().read(idx, allocated, view, f)
    }

    /// Mutable access to a block, which is marked dirty.
    fn block_mut(&mut self, idx: u32) -> Result<&mut [u8], KernelError> {
        let allocated = self.block_bitmap.is_allocated(idx);
        let disk = self.disk.clone();
        let guard = disk.as_ref().map(|d| d.lock());
        let view = self.disk_view(guard.as_deref());
        self.cache.get_mut().write(idx, allocated, view)
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
        if !self.disk_writable {
            return Err(KernelError::FsError(FsError::ReadOnly));
        }
        let view = Disk {
            backend: &*backend,
            device_blocks: self.device_blocks,
            writable: true,
        };

        // A dirty block past the end of the device can never be written.
        // Persisting metadata that references it would make its contents
        // read back as zeros after a remount, so fail before writing
        // anything; the dirty blocks stay in memory (ADR 0003). Review of the
        // v0.26.0 stack, PR #12.
        let cache = self.cache.get_mut();
        if let Some(_block) = cache.unwritable_dirty(view).next() {
            crate::println!(
                "[BLOCKFS] sync refused: dirty block {} exceeds device capacity {}",
                _block,
                self.device_blocks
            );
            return Err(KernelError::ResourceExhausted {
                resource: "blockfs device capacity",
            });
        }

        // Write all dirty data blocks
        let mut synced = cache.flush(view)?;

        // Update superblock write time and mount count
        self.superblock.write_time = crate::arch::timer::read_hw_timestamp();

        // Write metadata (superblock, bitmap, inode table)
        self.serialize_superblock(&*backend)?;
        self.serialize_bitmap(&*backend)?;
        self.serialize_inode_table(&*backend)?;
        synced += 1; // Count metadata as one sync unit

        Ok(synced)
    }

    /// Make the disk backend the source of data blocks.
    ///
    /// Clean cached blocks are dropped so the next access rereads them from
    /// the disk; dirty ones are kept. Returns how many allocated blocks are
    /// now served from the disk (they are read lazily, not here).
    fn load_from_disk(&mut self) -> Result<usize, KernelError> {
        if self.disk.is_none() {
            return Ok(0);
        }
        self.cache.get_mut().invalidate_clean();
        let blocks = (self.superblock.block_count as u64).min(self.device_blocks) as u32;
        Ok((self.superblock.first_data_block..blocks)
            .filter(|&b| self.block_bitmap.is_allocated(b))
            .count())
    }

    /// Attach `backend`, sampling its size and writability once.
    ///
    /// A device with fewer blocks than the superblock's `block_count` is
    /// refused: the bitmap could hand out blocks that the device cannot
    /// store.
    fn attach_disk(&mut self, backend: Arc<Mutex<dyn DiskBackend>>) -> Result<(), KernelError> {
        {
            let bk = backend.lock();
            let device_blocks = bk.block_count();
            if device_blocks < u64::from(self.superblock.block_count) {
                crate::println!(
                    "[BLOCKFS] device has {} blocks, filesystem needs {}",
                    device_blocks,
                    self.superblock.block_count
                );
                return Err(KernelError::InvalidArgument {
                    name: "disk",
                    value: "device smaller than the filesystem",
                });
            }
            self.device_blocks = device_blocks;
            self.disk_writable = !bk.is_read_only();
        }
        self.disk = Some(backend);
        Ok(())
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
    /// Reads the superblock, bitmap and inode table; data blocks are read
    /// on first use.
    fn load_existing(
        backend: Arc<Mutex<dyn DiskBackend>>,
        cache_blocks: usize,
    ) -> Result<Self, KernelError> {
        let started_ms = crate::timer::get_uptime_ms();
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

        // Data blocks are read on first use (FS-PERF-01).
        let block_count = superblock.block_count as usize;
        let first_data = superblock.first_data_block as usize;
        let allocated = (first_data..block_count)
            .filter(|&b| block_bitmap.is_allocated(b as u32))
            .count();

        crate::println!(
            "[BLOCKFS] Mounted in {} ms: {} of {} data blocks in use, read on demand ({} KiB \
             cache)",
            crate::timer::get_uptime_ms().saturating_sub(started_ms),
            allocated,
            block_count.saturating_sub(first_data),
            cache_blocks * BLOCK_SIZE / 1024
        );

        drop(bk);

        let mut fs = Self {
            superblock,
            block_bitmap,
            inode_table,
            cache: RwLock::new(BlockCache::new(block_count, cache_blocks)),
            disk: None,
            device_blocks: 0,
            disk_writable: false,
        };
        fs.attach_disk(backend)?;

        // Update mount count and time
        fs.superblock.mount_count += 1;
        fs.superblock.mount_time = crate::arch::timer::read_hw_timestamp();

        Ok(fs)
    }

    // --- Indirect block helpers ---

    /// Read a u32 block pointer from position `index` within an indirect block.
    fn read_block_ptr(&self, indirect_block: u32, index: usize) -> Result<u32, KernelError> {
        let off = index * size_of::<u32>();
        self.with_block(indirect_block, |block| {
            u32::from_le_bytes([block[off], block[off + 1], block[off + 2], block[off + 3]])
        })
    }

    /// Write a u32 block pointer at position `index` within an indirect block.
    fn write_block_ptr(
        &mut self,
        indirect_block: u32,
        index: usize,
        value: u32,
    ) -> Result<(), KernelError> {
        let off = index * size_of::<u32>();
        self.block_mut(indirect_block)?[off..off + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Resolve a logical block index to a physical block number for reading.
    ///
    /// Returns `Some(physical_block)` if the block is allocated, `None` if it
    /// falls in a sparse hole or exceeds the addressing range.
    fn resolve_block(
        &self,
        inode: &DiskInode,
        logical_block: usize,
    ) -> Result<Option<u32>, KernelError> {
        if logical_block < DIRECT_BLOCKS {
            // Direct block
            let blk = inode.direct_blocks[logical_block];
            Ok((blk != 0).then_some(blk))
        } else if logical_block < SINGLE_INDIRECT_MAX_BLOCKS {
            // Single indirect
            let indirect = inode.indirect_block;
            if indirect == 0 {
                return Ok(None);
            }
            let idx = logical_block - DIRECT_BLOCKS;
            let blk = self.read_block_ptr(indirect, idx)?;
            Ok((blk != 0).then_some(blk))
        } else if logical_block < DOUBLE_INDIRECT_MAX_BLOCKS {
            // Double indirect
            let dbl_indirect = inode.double_indirect_block;
            if dbl_indirect == 0 {
                return Ok(None);
            }
            let rel = logical_block - SINGLE_INDIRECT_MAX_BLOCKS;
            let l1_idx = rel / PTRS_PER_BLOCK;
            let l2_idx = rel % PTRS_PER_BLOCK;
            let l1_block = self.read_block_ptr(dbl_indirect, l1_idx)?;
            if l1_block == 0 {
                return Ok(None);
            }
            let blk = self.read_block_ptr(l1_block, l2_idx)?;
            Ok((blk != 0).then_some(blk))
        } else {
            // Beyond double indirect range (triple indirect not implemented)
            Ok(None)
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
            Ok(new_blk)
        } else if logical_block < SINGLE_INDIRECT_MAX_BLOCKS {
            // Single indirect
            let mut indirect = self.inode_table[inode_num as usize].indirect_block;
            if indirect == 0 {
                indirect = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.inode_table[inode_num as usize].indirect_block = indirect;
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let idx = logical_block - DIRECT_BLOCKS;
            let blk = self.read_block_ptr(indirect, idx)?;
            if blk != 0 {
                return Ok(blk);
            }
            let new_blk = self
                .allocate_block()
                .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
            self.write_block_ptr(indirect, idx, new_blk)?;
            self.inode_table[inode_num as usize].blocks += 1;
            Ok(new_blk)
        } else if logical_block < DOUBLE_INDIRECT_MAX_BLOCKS {
            // Double indirect
            let mut dbl_indirect = self.inode_table[inode_num as usize].double_indirect_block;
            if dbl_indirect == 0 {
                dbl_indirect = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.inode_table[inode_num as usize].double_indirect_block = dbl_indirect;
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let rel = logical_block - SINGLE_INDIRECT_MAX_BLOCKS;
            let l1_idx = rel / PTRS_PER_BLOCK;
            let l2_idx = rel % PTRS_PER_BLOCK;
            let mut l1_block = self.read_block_ptr(dbl_indirect, l1_idx)?;
            if l1_block == 0 {
                l1_block = self
                    .allocate_block()
                    .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
                self.write_block_ptr(dbl_indirect, l1_idx, l1_block)?;
                self.inode_table[inode_num as usize].blocks += 1;
            }
            let blk = self.read_block_ptr(l1_block, l2_idx)?;
            if blk != 0 {
                return Ok(blk);
            }
            let new_blk = self
                .allocate_block()
                .ok_or(KernelError::ResourceExhausted { resource: "blocks" })?;
            self.write_block_ptr(l1_block, l2_idx, new_blk)?;
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

            match self.resolve_block(inode, logical_block)? {
                Some(block_num) => {
                    let copy_len = (BLOCK_SIZE - block_offset).min(to_read - bytes_read);
                    let dst = &mut buffer[bytes_read..bytes_read + copy_len];
                    self.with_block(block_num, |block| {
                        dst.copy_from_slice(&block[block_offset..block_offset + copy_len])
                    })?;
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
            self.block_mut(block_num)?[*block_offset..*block_offset + *copy_len]
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

            let block_end = BLOCK_SIZE.min(dir_size - block_start);
            self.with_block(block_num, |block| {
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
            })?;
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

        match self.find_dir_entry(dir_inode, name)? {
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
        if self.find_dir_entry(parent, name)?.is_some() {
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
        if self.find_dir_entry(parent, name)?.is_some() {
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
        if self.find_dir_entry(parent, name)?.is_some() {
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

    /// Point the directory entry at (`dir`, `block_idx`, `offset`) at
    /// `inode` (0 deletes it). Link counts are the caller's business.
    fn set_dir_entry_inode(
        &mut self,
        dir: u32,
        block_idx: usize,
        offset: usize,
        inode: u32,
    ) -> Result<(), KernelError> {
        let block_num = self.inode_table[dir as usize].direct_blocks[block_idx];
        self.block_mut(block_num)?[offset..offset + 4].copy_from_slice(&inode.to_le_bytes());
        Ok(())
    }

    /// Rename `old_dir/old_name` to `new_dir/new_name` (FS-PERF-03).
    ///
    /// Every check runs before anything changes, including the check that a
    /// directory is not moved into its own subtree. An existing target is
    /// replaced under POSIX rules by overwriting its directory slot in place
    /// (so this works in a full directory), and only then is the old
    /// target's link dropped (freeing it if that was the last one). With no
    /// target, the new entry is written before the old one is cleared, so a
    /// failure (directory full) leaves the source in place. A directory
    /// moving to a new parent gets its ".." repointed and the parents' link
    /// counts adjusted.
    fn rename_entry(
        &mut self,
        old_dir: u32,
        old_name: &str,
        new_dir: u32,
        new_name: &str,
    ) -> Result<(), KernelError> {
        if super::is_special_name(old_name) || super::is_special_name(new_name) {
            return Err(KernelError::FsError(FsError::InvalidPath));
        }
        let (src, src_block, src_off) = self
            .find_dir_entry(old_dir, old_name)?
            .ok_or(KernelError::FsError(FsError::NotFound))?;
        let src_is_dir = src.file_type == DiskDirEntry::FT_DIR;
        if !self
            .inode_table
            .get(new_dir as usize)
            .is_some_and(|i| i.is_dir())
        {
            return Err(KernelError::FsError(FsError::NotADirectory));
        }

        let target = self.find_dir_entry(new_dir, new_name)?;
        if let Some((dst, _, _)) = &target {
            if dst.inode == src.inode {
                return Ok(()); // same inode: POSIX says do nothing
            }
            let dst_is_dir = dst.file_type == DiskDirEntry::FT_DIR;
            match (src_is_dir, dst_is_dir) {
                (true, false) => return Err(KernelError::FsError(FsError::NotADirectory)),
                (false, true) => return Err(KernelError::FsError(FsError::IsADirectory)),
                _ => {}
            }
            if dst_is_dir && !self.dir_is_empty(dst.inode)? {
                return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
            }
        }

        if src_is_dir && old_dir != new_dir {
            // Not into its own subtree: walk from the destination up the
            // ".." chain under this lock (bounded, in case of corruption).
            let mut at = new_dir;
            for _ in 0..self.inode_table.len() {
                if at == src.inode {
                    return Err(KernelError::FsError(FsError::InvalidPath));
                }
                match self.find_dir_entry(at, "..")? {
                    Some((up, _, _)) if up.inode != at => at = up.inode,
                    _ => break, // reached the root
                }
            }
        }

        // All checks passed. Entries are changed in place and never moved,
        // so the positions found above are still valid.
        match target {
            Some((dst, dst_block, dst_off)) => {
                self.set_dir_entry_slot(new_dir, dst_block, dst_off, src.inode, src.file_type)?;
                self.set_dir_entry_inode(old_dir, src_block, src_off, 0)?;
                let dst_is_dir = dst.file_type == DiskDirEntry::FT_DIR;
                self.drop_inode_link(dst.inode, dst_is_dir, new_dir)?;
            }
            None => {
                self.write_dir_entry(new_dir, src.inode, new_name, src.file_type)?;
                self.set_dir_entry_inode(old_dir, src_block, src_off, 0)?;
            }
        }

        if src_is_dir && old_dir != new_dir {
            if let Some((_, b, o)) = self.find_dir_entry(src.inode, "..")? {
                self.set_dir_entry_inode(src.inode, b, o, new_dir)?;
            }
            let old_parent = &mut self.inode_table[old_dir as usize];
            old_parent.links_count = old_parent.links_count.saturating_sub(1);
            self.inode_table[new_dir as usize].links_count += 1;
        }
        Ok(())
    }

    /// Point the directory slot at (`dir`, `block_idx`, `offset`) at `inode`
    /// with `file_type`, keeping its name and record length.
    fn set_dir_entry_slot(
        &mut self,
        dir: u32,
        block_idx: usize,
        offset: usize,
        inode: u32,
        file_type: u8,
    ) -> Result<(), KernelError> {
        let block_num = self.inode_table[dir as usize].direct_blocks[block_idx];
        let block = self.block_mut(block_num)?;
        block[offset..offset + 4].copy_from_slice(&inode.to_le_bytes());
        block[offset + 7] = file_type;
        Ok(())
    }

    /// Whether directory `dir` holds nothing but "." and "..".
    fn dir_is_empty(&self, dir: u32) -> Result<bool, KernelError> {
        Ok(self
            .readdir(dir)?
            .iter()
            .all(|e| e.name == "." || e.name == ".."))
    }

    /// Drop one link to `inode` whose directory entry in `parent` has just
    /// been removed, and free its blocks if that was the last link. For a
    /// directory, the parent also loses the link from its "..".
    fn drop_inode_link(
        &mut self,
        inode: u32,
        is_dir: bool,
        parent: u32,
    ) -> Result<(), KernelError> {
        if let Some(target) = self.inode_table.get_mut(inode as usize) {
            if target.links_count > 0 {
                target.links_count -= 1;
            }
            // Callers only drop a directory once it is empty, so it also
            // loses its own "." link. Without this it stayed at one link
            // and its inode and blocks were never freed (N-44).
            if is_dir && target.links_count > 0 {
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

            // If links reach 0, free all data blocks and the inode itself
            // (allocate_inode takes any zero-link inode and decrements
            // free_inodes, so not counting it back here underflowed).
            if self.inode_table[inode as usize].links_count == 0 {
                self.free_inode_blocks(inode)?;
                self.superblock.free_inodes += 1;
            }
        }
        Ok(())
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
            .find_dir_entry(parent, name)?
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        let target_inode = entry.inode;
        let is_dir = entry.file_type == DiskDirEntry::FT_DIR;

        // If unlinking a directory, check that it is empty (only "." and ".." entries)
        if is_dir && !self.dir_is_empty(target_inode)? {
            return Err(KernelError::FsError(FsError::DirectoryNotEmpty));
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
        self.block_mut(block_num)?[offset..offset + 4].fill(0);

        self.drop_inode_link(target_inode, is_dir, parent)
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
            self.truncate_single_indirect(inode_num, first_free_block)?;

            // Free double-indirect blocks beyond the new size
            self.truncate_double_indirect(inode_num, first_free_block)?;

            // If truncating to non-zero size within a block, zero the tail
            if size > 0 {
                let tail_logical_block = (size - 1) / BLOCK_SIZE;
                // Resolve using the inode (borrow scoped to avoid conflicts)
                let phys_block = {
                    let inode = &self.inode_table[inode_num as usize];
                    self.resolve_block(inode, tail_logical_block)?
                };
                if let Some(block_num) = phys_block {
                    let zero_from = size % BLOCK_SIZE;
                    if zero_from > 0 {
                        self.block_mut(block_num)?[zero_from..BLOCK_SIZE].fill(0);
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

    /// Change the permission bits on an inode, preserving the type bits.
    fn chmod_inode(&mut self, inode_num: u32, permissions: Permissions) -> Result<(), KernelError> {
        let inode = self
            .inode_table
            .get_mut(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;

        // Preserve the file type bits (top 4 bits of mode), replace
        // the permission bits (bottom 12 bits).
        let type_bits = inode.mode & 0xF000;
        let perm_bits = permissions_to_mode(permissions, false) & 0x0FFF;
        inode.mode = type_bits | perm_bits;
        inode.ctime = crate::arch::timer::read_hw_timestamp() as u32;

        Ok(())
    }

    /// Change an inode's owner and/or group. On-disk ids are 16-bit, so a
    /// larger id is rejected rather than truncated.
    fn chown_inode(
        &mut self,
        inode_num: u32,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(), KernelError> {
        let to_u16 = |id: Option<u32>| -> Result<Option<u16>, KernelError> {
            id.map(|v| {
                u16::try_from(v).map_err(|_| KernelError::InvalidArgument {
                    name: "owner id",
                    value: "exceeds 16-bit BlockFS limit",
                })
            })
            .transpose()
        };
        let (uid, gid) = (to_u16(uid)?, to_u16(gid)?);
        let inode = self
            .inode_table
            .get_mut(inode_num as usize)
            .ok_or(KernelError::FsError(FsError::NotFound))?;
        if let Some(uid) = uid {
            inode.uid = uid;
        }
        if let Some(gid) = gid {
            inode.gid = gid;
        }
        inode.ctime = crate::arch::timer::read_hw_timestamp() as u32;
        Ok(())
    }

    /// Create a hard link in a directory using pre-extracted metadata.
    ///
    /// This variant accepts pre-extracted metadata instead of an
    /// `Arc<dyn VfsNode>` to avoid deadlocks: the caller extracts the
    /// target's metadata BEFORE acquiring the BlockFsInner write lock,
    /// since `target.metadata()` would need a read lock on the same
    /// `RwLock<BlockFsInner>` (deadlock with the held write lock).
    fn link_in_dir_with_meta(
        &mut self,
        dir_inode: u32,
        name: &str,
        target_meta: &Metadata,
        target_node_type: NodeType,
    ) -> Result<(), KernelError> {
        if name.is_empty() || name.len() > MAX_FILENAME_LEN {
            return Err(KernelError::InvalidArgument {
                name: "filename",
                value: "empty or exceeds maximum length",
            });
        }

        // Hard links to directories are not allowed (POSIX)
        if target_node_type == NodeType::Directory {
            return Err(KernelError::FsError(FsError::IsADirectory));
        }

        // Check that the name doesn't already exist
        if self.find_dir_entry(dir_inode, name)?.is_some() {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }

        // Get the target's inode number from the pre-extracted metadata
        let target_inode = target_meta.inode as u32;

        // Verify the target inode exists in our inode table (same filesystem)
        if target_inode as usize >= self.inode_table.len()
            || self.inode_table[target_inode as usize].links_count == 0
        {
            return Err(KernelError::FsError(FsError::NotSupported));
        }

        // Determine file type for directory entry
        let file_type = if self.inode_table[target_inode as usize].is_symlink() {
            DiskDirEntry::FT_SYMLINK
        } else {
            DiskDirEntry::FT_REG_FILE
        };

        // Add directory entry
        self.write_dir_entry(dir_inode, target_inode, name, file_type)?;

        // Increment link count on the target inode
        self.inode_table[target_inode as usize].links_count += 1;

        Ok(())
    }

    /// Free single-indirect data blocks at or beyond `first_free_block`.
    /// Also frees the indirect block itself if it becomes fully empty.
    fn truncate_single_indirect(
        &mut self,
        inode_num: u32,
        first_free_block: usize,
    ) -> Result<(), KernelError> {
        let indirect = self.inode_table[inode_num as usize].indirect_block;
        if indirect == 0 {
            return Ok(());
        }

        // If all indirect entries are being freed
        if first_free_block <= DIRECT_BLOCKS {
            // Free every data block referenced by the indirect block
            for idx in 0..PTRS_PER_BLOCK {
                let blk = self.read_block_ptr(indirect, idx)?;
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
                    let blk = self.read_block_ptr(indirect, idx)?;
                    if blk != 0 {
                        self.free_block(blk);
                        self.write_block_ptr(indirect, idx, 0)?;
                        if self.inode_table[inode_num as usize].blocks > 0 {
                            self.inode_table[inode_num as usize].blocks -= 1;
                        }
                    }
                } else {
                    let blk = self.read_block_ptr(indirect, idx)?;
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
        Ok(())
    }

    /// Free double-indirect data blocks at or beyond `first_free_block`.
    /// Also frees level-1 indirect blocks and the double-indirect block itself
    /// if they become fully empty.
    fn truncate_double_indirect(
        &mut self,
        inode_num: u32,
        first_free_block: usize,
    ) -> Result<(), KernelError> {
        let dbl_indirect = self.inode_table[inode_num as usize].double_indirect_block;
        if dbl_indirect == 0 {
            return Ok(());
        }

        // Logical block range covered by double indirect:
        //   [SINGLE_INDIRECT_MAX_BLOCKS .. DOUBLE_INDIRECT_MAX_BLOCKS)

        if first_free_block <= SINGLE_INDIRECT_MAX_BLOCKS {
            // Free everything in double-indirect range
            for l1_idx in 0..PTRS_PER_BLOCK {
                let l1_block = self.read_block_ptr(dbl_indirect, l1_idx)?;
                if l1_block != 0 {
                    // Free all data blocks in this L1 indirect block
                    for l2_idx in 0..PTRS_PER_BLOCK {
                        let data_blk = self.read_block_ptr(l1_block, l2_idx)?;
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
                let l1_block = self.read_block_ptr(dbl_indirect, l1_idx)?;
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
                        let data_blk = self.read_block_ptr(l1_block, l2_idx)?;
                        if data_blk != 0 {
                            self.free_block(data_blk);
                            self.write_block_ptr(l1_block, l2_idx, 0)?;
                            if self.inode_table[inode_num as usize].blocks > 0 {
                                self.inode_table[inode_num as usize].blocks -= 1;
                            }
                        }
                    } else {
                        let data_blk = self.read_block_ptr(l1_block, l2_idx)?;
                        if data_blk != 0 {
                            any_l2_remain = true;
                        }
                    }
                }

                if !any_l2_remain {
                    // Free the now-empty L1 indirect block
                    self.free_block(l1_block);
                    self.write_block_ptr(dbl_indirect, l1_idx, 0)?;
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
    fn find_dir_entry(
        &self,
        dir_inode: u32,
        name: &str,
    ) -> Result<Option<(DiskDirEntry, usize, usize)>, KernelError> {
        let Some(inode) = self.inode_table.get(dir_inode as usize) else {
            return Ok(None);
        };

        if !inode.is_dir() {
            return Ok(None);
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

            let block_end = BLOCK_SIZE.min(dir_size - block_start);
            let found = self.with_block(block_num, |block| {
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
                None
            })?;
            if found.is_some() {
                return Ok(found);
            }
        }

        Ok(None)
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

        let block = self.block_mut(block_num)?;

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

    /// Free all data blocks belonging to an inode (direct + indirect).
    fn free_inode_blocks(&mut self, inode_num: u32) -> Result<(), KernelError> {
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
        self.truncate_single_indirect(inode_num, 0)?;
        self.truncate_double_indirect(inode_num, 0)?;

        self.inode_table[inode_num as usize].blocks = 0;
        self.inode_table[inode_num as usize].size = 0;
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
    if perms.sticky {
        mode |= 0o1000;
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
    /// Reads the superblock, validates the magic number, and loads the bitmap
    /// and inode table. Data blocks are read on first use into a bounded
    /// cache (FS-PERF-01). The disk backend remains attached for subsequent
    /// sync operations.
    pub fn open_existing(backend: Arc<Mutex<dyn DiskBackend>>) -> Result<Self, KernelError> {
        Self::open_existing_with_cache(backend, cache::DEFAULT_CAPACITY_BLOCKS)
    }

    /// `open_existing` with a cache of `cache_blocks` 4 KiB blocks.
    pub(crate) fn open_existing_with_cache(
        backend: Arc<Mutex<dyn DiskBackend>>,
        cache_blocks: usize,
    ) -> Result<Self, KernelError> {
        let inner = BlockFsInner::load_existing(backend, cache_blocks)?;
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
    /// If `load` is true, the disk becomes the source of data blocks: clean
    /// cached blocks are dropped and reread on demand. Otherwise memory is
    /// treated as newer than the disk (every block written since creation
    /// is still dirty) and the next sync writes it out.
    ///
    /// Fails, leaving no backend attached, if the device has fewer blocks
    /// than the filesystem.
    pub fn set_disk_backend(
        &self,
        backend: Arc<Mutex<dyn DiskBackend>>,
        load: bool,
    ) -> Result<(), KernelError> {
        let mut inner = self.inner.write();
        inner.attach_disk(backend)?;

        if load {
            let loaded = inner.load_from_disk()?;
            crate::println!("[BLOCKFS] {} blocks now served from disk backend", loaded);
        }

        Ok(())
    }

    /// Get the number of dirty blocks pending sync.
    pub fn dirty_block_count(&self) -> usize {
        let inner = self.inner.read();
        let count = inner.cache.read().dirty_count();
        count
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
    #[test]
    fn test_link_rejects_other_filesystem() {
        let a = BlockFs::format(1000, 100).unwrap();
        let b = BlockFs::format(1000, 100).unwrap();
        let (root_a, root_b) = (a.root(), b.root());
        let file_a = root_a.create("f", Permissions::default()).unwrap();
        let file_b = root_b.create("g", Permissions::default()).unwrap();
        // Both filesystems hand out the same inode numbers, which the old
        // range check took as proof of "same filesystem" (W-18).
        assert_eq!(
            file_a.metadata().unwrap().inode,
            file_b.metadata().unwrap().inode
        );
        assert!(matches!(
            root_a.link("h", file_b),
            Err(KernelError::FsError(FsError::CrossDevice))
        ));
        root_a.link("h", file_a).unwrap();
        assert!(root_a.lookup("h").is_ok());
    }

    #[test]
    fn test_sticky_bit_persists_through_chmod() {
        let fs = BlockFs::format(1000, 100).unwrap();
        let dir = fs
            .root()
            .mkdir("tmp", Permissions::from_mode(0o1777))
            .unwrap();
        assert!(dir.metadata().unwrap().permissions.sticky);
        dir.chmod(Permissions::from_mode(0o777)).unwrap();
        assert!(!dir.metadata().unwrap().permissions.sticky);
        dir.chmod(Permissions::from_mode(0o1777)).unwrap();
        assert_eq!(dir.metadata().unwrap().permissions.to_mode(), 0o1777);
    }

    /// In-memory disk for persistence tests.
    struct MemDisk {
        blocks: Mutex<Vec<Vec<u8>>>,
        writes: core::sync::atomic::AtomicUsize,
    }

    impl MemDisk {
        fn new(n: usize) -> Arc<Self> {
            Arc::new(Self {
                blocks: Mutex::new(vec![vec![0u8; BLOCK_SIZE]; n]),
                writes: core::sync::atomic::AtomicUsize::new(0),
            })
        }
        fn writes(&self) -> usize {
            self.writes.load(core::sync::atomic::Ordering::Relaxed)
        }
    }

    impl DiskBackend for MemDisk {
        fn read_block(&self, n: u64, buf: &mut [u8]) -> Result<(), KernelError> {
            buf[..BLOCK_SIZE].copy_from_slice(&self.blocks.lock()[n as usize]);
            Ok(())
        }
        fn write_block(&self, n: u64, data: &[u8]) -> Result<(), KernelError> {
            self.writes
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            self.blocks.lock()[n as usize].copy_from_slice(&data[..BLOCK_SIZE]);
            Ok(())
        }
        fn block_count(&self) -> u64 {
            self.blocks.lock().len() as u64
        }
        fn is_read_only(&self) -> bool {
            false
        }
    }

    /// The filesystem owns its backend behind a mutex; this lets the test
    /// keep a handle on the same disk.
    struct Shared(Arc<MemDisk>);

    impl DiskBackend for Shared {
        fn read_block(&self, n: u64, buf: &mut [u8]) -> Result<(), KernelError> {
            self.0.read_block(n, buf)
        }
        fn write_block(&self, n: u64, data: &[u8]) -> Result<(), KernelError> {
            self.0.write_block(n, data)
        }
        fn block_count(&self) -> u64 {
            self.0.block_count()
        }
        fn is_read_only(&self) -> bool {
            false
        }
    }

    fn backend(disk: &Arc<MemDisk>) -> Arc<Mutex<dyn DiskBackend>> {
        Arc::new(Mutex::new(Shared(disk.clone())))
    }

    fn pattern(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add((i / 7) as u8)).collect()
    }

    fn read_all(node: &Arc<dyn VfsNode>) -> Vec<u8> {
        let mut buf = vec![0u8; node.metadata().unwrap().size];
        let n = node.read(0, &mut buf).unwrap();
        buf.truncate(n);
        buf
    }

    /// A rename that is rejected must not have destroyed the target first
    /// (review of the v0.26.0 stack, PR #11).
    #[test]
    fn rename_into_own_subtree_keeps_existing_target() {
        let fs = BlockFs::format(1000, 100).unwrap();
        let root = fs.root();
        let a = root.mkdir("a", Permissions::default()).unwrap();
        let b = a.mkdir("b", Permissions::default()).unwrap();
        b.mkdir("x", Permissions::default()).unwrap();
        assert_eq!(
            root.rename("a", &b, "x"),
            Err(KernelError::FsError(FsError::InvalidPath))
        );
        assert!(
            b.lookup("x").is_ok(),
            "target destroyed by a rejected rename"
        );
        assert!(root.lookup("a").is_ok());
    }

    /// Replacing an existing name reuses its slot, so it works even in a
    #[test]
    fn removed_directories_and_files_return_their_inodes() {
        // 16 inodes: creating and removing far more than that only works
        // if every removal frees its inode. An empty directory kept its "."
        // link and never reached zero links (N-44), and the final free
        // never returned the inode to free_inodes, which then underflowed.
        let fs = BlockFs::format(1000, 16).unwrap();
        let root = fs.root();
        for _ in 0..64 {
            root.mkdir("d", Permissions::default()).unwrap();
            root.unlink("d").unwrap();
            root.create("f", Permissions::default())
                .unwrap()
                .write(0, b"x")
                .unwrap();
            root.unlink("f").unwrap();
        }
        let inner = fs.inner.read();
        assert_eq!(inner.superblock.free_inodes, 15);
        assert_eq!(
            inner.superblock.free_blocks,
            BlockFs::format(1000, 16)
                .unwrap()
                .inner
                .read()
                .superblock
                .free_blocks
        );
    }

    /// directory that has no room for another entry.
    #[test]
    fn rename_over_existing_name_in_full_directory() {
        let fs = BlockFs::format(1000, 400).unwrap();
        let root = fs.root();
        let full = root.mkdir("full", Permissions::default()).unwrap();
        let name = |i: usize| alloc::format!("{:0>200}", i);
        let mut n = 0;
        loop {
            match full.create(&name(n), Permissions::default()) {
                Ok(_) => n += 1,
                Err(KernelError::ResourceExhausted { .. }) => break,
                Err(e) => panic!("unexpected {:?}", e),
            }
        }
        assert!(n > 0);
        // Top up the tail with minimum-size entries (names of 1-4 bytes).
        let mut k = 0;
        loop {
            match full.create(&alloc::format!("q{}", k), Permissions::default()) {
                Ok(_) => k += 1,
                Err(KernelError::ResourceExhausted { .. }) => break,
                Err(e) => panic!("unexpected {:?}", e),
            }
        }
        let src = root.mkdir("src", Permissions::default()).unwrap();
        src.create("s", Permissions::default())
            .unwrap()
            .write(0, b"payload")
            .unwrap();
        src.create("t", Permissions::default()).unwrap();

        // A new name does not fit: the rename fails and the source stays.
        assert!(src.rename("t", &full, "new").is_err());
        assert!(src.lookup("t").is_ok());

        // An existing name is overwritten in place.
        let entries = full.readdir().unwrap().len();
        src.rename("s", &full, &name(0)).unwrap();
        assert!(src.lookup("s").is_err());
        assert_eq!(read_all(&full.lookup(&name(0)).unwrap()), b"payload");
        assert_eq!(
            full.readdir().unwrap().len(),
            entries,
            "no entry added or lost"
        );
    }

    /// A device smaller than the filesystem cannot hold every block the
    /// bitmap may hand out: attaching it is refused (review of the v0.26.0
    /// stack, PR #12).
    #[test]
    fn attach_rejects_device_smaller_than_filesystem() {
        let fs = BlockFs::format(1000, 100).unwrap();
        assert!(fs
            .set_disk_backend(backend(&MemDisk::new(64)), false)
            .is_err());
        assert!(fs.inner.read().disk.is_none());

        // A valid image whose device was truncated must not mount either.
        let full = MemDisk::new(1000);
        fs.set_disk_backend(backend(&full), false).unwrap();
        fs.sync().unwrap();
        let small = MemDisk::new(64);
        small
            .blocks
            .lock()
            .clone_from_slice(&full.blocks.lock()[..64]);
        assert!(BlockFs::open_existing(backend(&small)).is_err());
    }

    /// If any dirty block lies past the end of the device, sync fails before
    /// it writes anything, so no metadata can point at unwritten data.
    #[test]
    fn sync_fails_without_writing_when_a_dirty_block_does_not_fit() {
        let fs = BlockFs::format(1000, 100).unwrap();
        let root = fs.root();
        // ~75 data blocks: the bitmap hands out blocks past index 64.
        root.create("big", Permissions::default())
            .unwrap()
            .write(0, &pattern(3, 300 * 1024))
            .unwrap();
        let disk = MemDisk::new(64);
        {
            // Bypass the attach-time size check to model a device that is
            // smaller than the filesystem.
            let mut inner = fs.inner.write();
            inner.disk = Some(backend(&disk));
            inner.device_blocks = 64;
            inner.disk_writable = true;
        }
        assert!(fs.sync().is_err());
        assert_eq!(disk.writes(), 0, "nothing, metadata included, is written");
        assert!(fs.dirty_block_count() > 0, "dirty blocks stay in memory");
    }

    /// FS-PERF-01: data larger than the cache survives sync and remount, and
    /// nothing reaches the disk between syncs.
    #[test]
    fn lazy_cache_persists_through_remount() {
        let disk = MemDisk::new(2048);
        let fs = BlockFs::format(2048, 128).unwrap();
        fs.set_disk_backend(backend(&disk), false).unwrap();
        let root = fs.root();
        let dir = root.mkdir("d", Permissions::default()).unwrap();
        // 600 KiB: past the 12 direct blocks, through the single indirect.
        let big = pattern(1, 600 * 1024);
        dir.create("big", Permissions::default())
            .unwrap()
            .write(0, &big)
            .unwrap();
        for i in 0..20u8 {
            let name = alloc::format!("f{}", i);
            root.create(&name, Permissions::default())
                .unwrap()
                .write(0, &pattern(i, 5000))
                .unwrap();
        }
        assert_eq!(disk.writes(), 0, "nothing is written before sync");
        fs.sync().unwrap();
        drop((root, dir, fs));

        // Remount with a 4-block cache: every read now cycles the pool.
        let fs = BlockFs::open_existing_with_cache(backend(&disk), 4).unwrap();
        let root = fs.root();
        let d = root.lookup("d").unwrap();
        assert_eq!(read_all(&d.lookup("big").unwrap()), big);
        for i in 0..20u8 {
            let f = root.lookup(&alloc::format!("f{}", i)).unwrap();
            assert_eq!(read_all(&f), pattern(i, 5000));
        }
        // 20 files plus "d"; readdir omits "." and "..".
        assert_eq!(root.readdir().unwrap().len(), 21);
        {
            let inner = fs.inner.read();
            let cache = inner.cache.read();
            assert!(cache.slot_count() <= 4, "clean data stays within the bound");
            assert!(cache.stats().evictions > 100);
        }

        // Changes after the last sync stay off the disk, then land on sync.
        let before = disk.writes();
        root.unlink("f3").unwrap();
        d.lookup("big").unwrap().truncate(10_000).unwrap();
        root.create("new", Permissions::default())
            .unwrap()
            .write(0, &pattern(9, 70_000))
            .unwrap();
        assert_eq!(disk.writes(), before, "dirty blocks are pinned until sync");
        assert!(fs.dirty_block_count() > 0);
        fs.sync().unwrap();
        assert_eq!(fs.dirty_block_count(), 0);
        drop((root, d, fs));

        let fs = BlockFs::open_existing_with_cache(backend(&disk), 4).unwrap();
        let root = fs.root();
        assert!(root.lookup("f3").is_err());
        assert_eq!(
            read_all(&root.lookup("d").unwrap().lookup("big").unwrap()),
            big[..10_000]
        );
        assert_eq!(read_all(&root.lookup("new").unwrap()), pattern(9, 70_000));
        assert_eq!(read_all(&root.lookup("f4").unwrap()), pattern(4, 5000));
    }
}
