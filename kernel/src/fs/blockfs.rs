//! Block-based persistent filesystem (BlockFS)
//!
//! A simple ext2-like filesystem with:
//! - Superblock with metadata
//! - Inode table for file/directory metadata
//! - Block allocation bitmap
//! - Data blocks for file content

use super::{FileSystem, Inode, InodeType};
use crate::error::KernelError;
use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

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
}

/// Directory entry
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
    pub name: [u8; MAX_FILENAME_LEN],
}

impl DirEntry {
    pub fn new(inode: u32, name: &str, file_type: u8) -> Self {
        let mut name_bytes = [0u8; MAX_FILENAME_LEN];
        let name_len = name.len().min(MAX_FILENAME_LEN);
        name_bytes[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);

        let rec_len = (8 + name_len + 3) & !3; // Align to 4 bytes

        Self {
            inode,
            rec_len: rec_len as u16,
            name_len: name_len as u8,
            file_type,
            name: name_bytes,
        }
    }

    pub fn name_str(&self) -> &str {
        let name_slice = &self.name[..self.name_len as usize];
        core::str::from_utf8(name_slice).unwrap_or("")
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

/// BlockFS filesystem implementation
pub struct BlockFileSystem {
    superblock: Superblock,
    block_bitmap: BlockBitmap,
    inode_table: Vec<DiskInode>,
    mounted: bool,
}

impl BlockFileSystem {
    pub fn new(block_count: u32, inode_count: u32) -> Self {
        let superblock = Superblock::new(block_count, inode_count);
        let block_bitmap = BlockBitmap::new(block_count as usize);
        let mut inode_table = Vec::new();
        inode_table.resize(inode_count as usize, DiskInode::new(0, 0, 0));

        // Initialize root directory (inode 0)
        inode_table[0] = DiskInode::new(0x41ED, 0, 0); // Directory, rwxr-xr-x

        Self {
            superblock,
            block_bitmap,
            inode_table,
            mounted: false,
        }
    }

    pub fn format(block_count: u32, inode_count: u32) -> Result<Self, KernelError> {
        if block_count < 100 {
            return Err(KernelError::InvalidArgument {
                name: "block_count",
                value: "too_small",
            });
        }

        if inode_count < 10 {
            return Err(KernelError::InvalidArgument {
                name: "inode_count",
                value: "too_small",
            });
        }

        Ok(Self::new(block_count, inode_count))
    }

    pub fn allocate_inode(&mut self) -> Option<u32> {
        for (idx, inode) in self.inode_table.iter().enumerate() {
            if inode.links_count == 0 {
                self.superblock.free_inodes -= 1;
                return Some(idx as u32);
            }
        }
        None
    }

    pub fn free_inode(&mut self, inode_num: u32) {
        if (inode_num as usize) < self.inode_table.len() {
            self.inode_table[inode_num as usize].links_count = 0;
            self.inode_table[inode_num as usize].dtime = 1; // Mark as deleted
            self.superblock.free_inodes += 1;
        }
    }

    pub fn get_inode(&self, inode_num: u32) -> Option<&DiskInode> {
        self.inode_table.get(inode_num as usize)
    }

    pub fn get_inode_mut(&mut self, inode_num: u32) -> Option<&mut DiskInode> {
        self.inode_table.get_mut(inode_num as usize)
    }

    pub fn allocate_block(&mut self) -> Option<u32> {
        let block = self.block_bitmap.allocate_block()?;
        self.superblock.free_blocks -= 1;
        Some(block)
    }

    pub fn free_block(&mut self, block: u32) {
        self.block_bitmap.free_block(block);
        self.superblock.free_blocks += 1;
    }
}

impl FileSystem for BlockFileSystem {
    fn mount(&mut self, _source: &str, _target: &str, _flags: u32) -> Result<(), KernelError> {
        if self.mounted {
            return Err(KernelError::InvalidState {
                expected: "unmounted",
                actual: "mounted",
            });
        }

        if !self.superblock.is_valid() {
            return Err(KernelError::InvalidArgument {
                name: "superblock",
                value: "invalid_magic",
            });
        }

        self.mounted = true;
        self.superblock.mount_count += 1;
        Ok(())
    }

    fn unmount(&mut self, _target: &str) -> Result<(), KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        self.mounted = false;
        Ok(())
    }

    fn create(&mut self, path: &str, inode_type: InodeType) -> Result<Inode, KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        let inode_num = self.allocate_inode().ok_or(KernelError::ResourceExhausted {
            resource: "inodes",
        })?;

        let mode = match inode_type {
            InodeType::Directory => 0x41ED, // Directory, rwxr-xr-x
            InodeType::File => 0x81A4,      // Regular file, rw-r--r--
            _ => 0x81A4,
        };

        let disk_inode = DiskInode::new(mode, 0, 0);
        self.inode_table[inode_num as usize] = disk_inode;

        Ok(Inode {
            ino: inode_num as usize,
            inode_type,
            size: 0,
            mode: mode as u32,
        })
    }

    fn lookup(&self, path: &str) -> Result<Inode, KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        if path == "/" {
            return Ok(Inode {
                ino: 0,
                inode_type: InodeType::Directory,
                size: 0,
                mode: 0x41ED,
            });
        }

        Err(KernelError::NotFound {
            resource: "inode",
            id: 0,
        })
    }

    fn read(&self, _inode: &Inode, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        // TODO: Implement block reading
        Ok(0)
    }

    fn write(&mut self, _inode: &Inode, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        // TODO: Implement block writing
        Ok(0)
    }

    fn mkdir(&mut self, path: &str) -> Result<Inode, KernelError> {
        self.create(path, InodeType::Directory)
    }

    fn rmdir(&mut self, _path: &str) -> Result<(), KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        // TODO: Implement directory removal
        Ok(())
    }

    fn unlink(&mut self, _path: &str) -> Result<(), KernelError> {
        if !self.mounted {
            return Err(KernelError::InvalidState {
                expected: "mounted",
                actual: "unmounted",
            });
        }

        // TODO: Implement file removal
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
        let fs = BlockFileSystem::format(1000, 100).unwrap();
        assert_eq!(fs.superblock.block_count, 1000);
        assert_eq!(fs.superblock.inode_count, 100);
    }

    #[test_case]
    fn test_inode_allocation() {
        let mut fs = BlockFileSystem::format(1000, 100).unwrap();

        let inode1 = fs.allocate_inode().unwrap();
        assert!(inode1 > 0); // 0 is reserved for root

        fs.free_inode(inode1);
        let inode2 = fs.allocate_inode().unwrap();
        assert_eq!(inode1, inode2); // Should reuse freed inode
    }
}
