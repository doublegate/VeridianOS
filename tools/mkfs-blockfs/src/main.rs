//! mkfs-blockfs -- Create and populate VeridianOS BlockFS disk images
//!
//! This is a host-side tool (runs on Linux) that creates a raw disk image
//! containing a pre-formatted BlockFS filesystem, optionally populated with
//! files from a host directory.
//!
//! The on-disk layout matches what the kernel's BlockFS driver expects:
//!
//! ```text
//! Block 0:              Superblock (62 bytes serialized, padded to 4KB)
//! Blocks 1..1+B:        Block bitmap (B = ceil(total_blocks / 32768))
//! Blocks 1+B..1+B+I:    Inode table (I = ceil(inode_count * 96 / 4096))
//! Blocks 1+B+I..end:    Data blocks
//! ```
//!
//! Usage:
//!   mkfs-blockfs --output <path> --size <MB> [--populate <dir>]

use std::collections::VecDeque;
use std::env;
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const BLOCK_SIZE: usize = 4096;
const BLOCKFS_MAGIC: u32 = 0x424C4B46; // "BLKF"
const DISK_INODE_SIZE: usize = 96;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / DISK_INODE_SIZE; // 42
const DIRECT_BLOCKS: usize = 12;
const PTRS_PER_BLOCK: usize = BLOCK_SIZE / 4; // 1024
const MAX_FILENAME_LEN: usize = 255;
const DIR_ENTRY_HEADER_SIZE: usize = 8;

// File type constants for directory entries
const FT_REG_FILE: u8 = 1;
const FT_DIR: u8 = 2;

fn align4(val: usize) -> usize {
    (val + 3) & !3
}

fn bitmap_blocks(total_blocks: u32) -> u32 {
    let bits_needed = total_blocks as usize;
    let bytes_needed = (bits_needed + 7) / 8;
    ((bytes_needed + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32
}

fn inode_table_blocks(inode_count: u32) -> u32 {
    ((inode_count as usize * DISK_INODE_SIZE + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32
}

fn computed_first_data_block(total_blocks: u32, inode_count: u32) -> u32 {
    1 + bitmap_blocks(total_blocks) + inode_table_blocks(inode_count)
}

/// On-disk superblock (62 bytes serialized)
struct Superblock {
    magic: u32,
    block_count: u32,
    inode_count: u32,
    free_blocks: u32,
    free_inodes: u32,
    first_data_block: u32,
    block_size: u32,
    inode_size: u16,
    blocks_per_group: u32,
    inodes_per_group: u32,
    mount_time: u64,
    write_time: u64,
    mount_count: u16,
    max_mount_count: u16,
    state: u16,
    errors: u16,
}

impl Superblock {
    fn serialize(&self) -> [u8; BLOCK_SIZE] {
        let mut buf = [0u8; BLOCK_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.block_count.to_le_bytes());
        buf[8..12].copy_from_slice(&self.inode_count.to_le_bytes());
        buf[12..16].copy_from_slice(&self.free_blocks.to_le_bytes());
        buf[16..20].copy_from_slice(&self.free_inodes.to_le_bytes());
        buf[20..24].copy_from_slice(&self.first_data_block.to_le_bytes());
        buf[24..28].copy_from_slice(&self.block_size.to_le_bytes());
        buf[28..30].copy_from_slice(&self.inode_size.to_le_bytes());
        buf[30..34].copy_from_slice(&self.blocks_per_group.to_le_bytes());
        buf[34..38].copy_from_slice(&self.inodes_per_group.to_le_bytes());
        buf[38..46].copy_from_slice(&self.mount_time.to_le_bytes());
        buf[46..54].copy_from_slice(&self.write_time.to_le_bytes());
        buf[54..56].copy_from_slice(&self.mount_count.to_le_bytes());
        buf[56..58].copy_from_slice(&self.max_mount_count.to_le_bytes());
        buf[58..60].copy_from_slice(&self.state.to_le_bytes());
        buf[60..62].copy_from_slice(&self.errors.to_le_bytes());
        buf
    }
}

/// On-disk inode (96 bytes)
#[derive(Clone)]
struct DiskInode {
    mode: u16,
    uid: u16,
    size: u32,
    atime: u32,
    ctime: u32,
    mtime: u32,
    dtime: u32,
    gid: u16,
    links_count: u16,
    blocks: u32,
    flags: u32,
    direct_blocks: [u32; 12],
    indirect_block: u32,
    double_indirect_block: u32,
    triple_indirect_block: u32,
}

impl DiskInode {
    fn new(mode: u16) -> Self {
        Self {
            mode,
            uid: 0,
            size: 0,
            atime: 0,
            ctime: 0,
            mtime: 0,
            dtime: 0,
            gid: 0,
            links_count: 1,
            blocks: 0,
            flags: 0,
            direct_blocks: [0; 12],
            indirect_block: 0,
            double_indirect_block: 0,
            triple_indirect_block: 0,
        }
    }

    fn serialize(&self, buf: &mut [u8]) {
        buf[0..2].copy_from_slice(&self.mode.to_le_bytes());
        buf[2..4].copy_from_slice(&self.uid.to_le_bytes());
        buf[4..8].copy_from_slice(&self.size.to_le_bytes());
        buf[8..12].copy_from_slice(&self.atime.to_le_bytes());
        buf[12..16].copy_from_slice(&self.ctime.to_le_bytes());
        buf[16..20].copy_from_slice(&self.mtime.to_le_bytes());
        buf[20..24].copy_from_slice(&self.dtime.to_le_bytes());
        buf[24..26].copy_from_slice(&self.gid.to_le_bytes());
        buf[26..28].copy_from_slice(&self.links_count.to_le_bytes());
        buf[28..32].copy_from_slice(&self.blocks.to_le_bytes());
        buf[32..36].copy_from_slice(&self.flags.to_le_bytes());
        for (j, &blk) in self.direct_blocks.iter().enumerate() {
            let off = 36 + j * 4;
            buf[off..off + 4].copy_from_slice(&blk.to_le_bytes());
        }
        buf[84..88].copy_from_slice(&self.indirect_block.to_le_bytes());
        buf[88..92].copy_from_slice(&self.double_indirect_block.to_le_bytes());
        buf[92..96].copy_from_slice(&self.triple_indirect_block.to_le_bytes());
    }
}

/// BlockFS image builder
struct BlockFsBuilder {
    block_count: u32,
    inode_count: u32,
    first_data_block: u32,
    bitmap: Vec<u8>,
    inodes: Vec<DiskInode>,
    /// Data blocks (only used ones are stored; written by block index)
    blocks: Vec<Vec<u8>>,
    next_free_inode: u32,
}

impl BlockFsBuilder {
    fn new(block_count: u32, inode_count: u32) -> Self {
        let first_data = computed_first_data_block(block_count, inode_count);
        let bitmap_size = (block_count as usize + 7) / 8;
        let mut bitmap = vec![0u8; bitmap_size];

        // Mark metadata blocks as allocated
        for b in 0..first_data {
            let byte_idx = (b / 8) as usize;
            let bit = (b % 8) as usize;
            if byte_idx < bitmap.len() {
                bitmap[byte_idx] |= 1 << bit;
            }
        }

        let mut inodes = vec![DiskInode::new(0); inode_count as usize];

        // Root inode (inode 0): directory, rwxr-xr-x
        inodes[0].mode = 0x41ED;
        inodes[0].links_count = 2;

        let mut blocks = Vec::with_capacity(block_count as usize);
        for _ in 0..block_count {
            blocks.push(vec![0u8; BLOCK_SIZE]);
        }

        let mut builder = Self {
            block_count,
            inode_count,
            first_data_block: first_data,
            bitmap,
            inodes,
            blocks,
            next_free_inode: 1,
        };

        // Create "." and ".." entries in root
        builder.write_dir_entry(0, 0, ".", FT_DIR);
        builder.write_dir_entry(0, 0, "..", FT_DIR);

        builder
    }

    fn allocate_block(&mut self) -> Option<u32> {
        for byte_idx in 0..self.bitmap.len() {
            if self.bitmap[byte_idx] != 0xFF {
                for bit in 0..8 {
                    if (self.bitmap[byte_idx] & (1 << bit)) == 0 {
                        self.bitmap[byte_idx] |= 1 << bit;
                        let block_num = (byte_idx * 8 + bit) as u32;
                        if block_num < self.block_count {
                            return Some(block_num);
                        }
                    }
                }
            }
        }
        None
    }

    fn allocate_inode(&mut self) -> Option<u32> {
        if self.next_free_inode >= self.inode_count {
            return None;
        }
        let idx = self.next_free_inode;
        self.next_free_inode += 1;
        Some(idx)
    }

    fn free_blocks_count(&self) -> u32 {
        let mut count = 0u32;
        for byte_idx in 0..self.bitmap.len() {
            for bit in 0..8 {
                let block_num = byte_idx * 8 + bit;
                if block_num < self.block_count as usize
                    && (self.bitmap[byte_idx] & (1 << bit)) == 0
                {
                    count += 1;
                }
            }
        }
        count
    }

    /// Ensure a logical block for an inode is allocated, creating indirect
    /// blocks as needed.
    fn ensure_block(&mut self, inode_idx: u32, logical_block: usize) -> u32 {
        if logical_block < DIRECT_BLOCKS {
            let existing = self.inodes[inode_idx as usize].direct_blocks[logical_block];
            if existing != 0 {
                return existing;
            }
            let blk = self.allocate_block().expect("out of blocks");
            self.inodes[inode_idx as usize].direct_blocks[logical_block] = blk;
            self.inodes[inode_idx as usize].blocks += 1;
            blk
        } else if logical_block < DIRECT_BLOCKS + PTRS_PER_BLOCK {
            // Single indirect
            let mut indirect = self.inodes[inode_idx as usize].indirect_block;
            if indirect == 0 {
                indirect = self.allocate_block().expect("out of blocks");
                self.blocks[indirect as usize] = vec![0u8; BLOCK_SIZE];
                self.inodes[inode_idx as usize].indirect_block = indirect;
                self.inodes[inode_idx as usize].blocks += 1;
            }
            let idx = logical_block - DIRECT_BLOCKS;
            let off = idx * 4;
            let existing = u32::from_le_bytes([
                self.blocks[indirect as usize][off],
                self.blocks[indirect as usize][off + 1],
                self.blocks[indirect as usize][off + 2],
                self.blocks[indirect as usize][off + 3],
            ]);
            if existing != 0 {
                return existing;
            }
            let blk = self.allocate_block().expect("out of blocks");
            self.blocks[indirect as usize][off..off + 4].copy_from_slice(&blk.to_le_bytes());
            self.inodes[inode_idx as usize].blocks += 1;
            blk
        } else {
            panic!(
                "file too large: logical block {} exceeds single indirect range",
                logical_block
            );
        }
    }

    /// Write data to an inode's data blocks
    fn write_inode_data(&mut self, inode_idx: u32, offset: usize, data: &[u8]) {
        let mut current_offset = offset;
        let mut bytes_written = 0;

        while bytes_written < data.len() {
            let logical_block = current_offset / BLOCK_SIZE;
            let block_offset = current_offset % BLOCK_SIZE;
            let copy_len = (BLOCK_SIZE - block_offset).min(data.len() - bytes_written);

            let phys_block = self.ensure_block(inode_idx, logical_block);
            self.blocks[phys_block as usize][block_offset..block_offset + copy_len]
                .copy_from_slice(&data[bytes_written..bytes_written + copy_len]);

            bytes_written += copy_len;
            current_offset += copy_len;
        }

        // Update inode size
        let new_end = offset + data.len();
        if new_end > self.inodes[inode_idx as usize].size as usize {
            self.inodes[inode_idx as usize].size = new_end as u32;
        }
    }

    /// Add a directory entry to a directory inode
    fn write_dir_entry(&mut self, dir_inode: u32, child_inode: u32, name: &str, file_type: u8) {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(MAX_FILENAME_LEN);
        let rec_len = align4(DIR_ENTRY_HEADER_SIZE + name_len);

        let mut entry = vec![0u8; rec_len];
        entry[0..4].copy_from_slice(&child_inode.to_le_bytes());
        entry[4..6].copy_from_slice(&(rec_len as u16).to_le_bytes());
        entry[6] = name_len as u8;
        entry[7] = file_type;
        entry[8..8 + name_len].copy_from_slice(&name_bytes[..name_len]);

        let current_size = self.inodes[dir_inode as usize].size as usize;
        self.write_inode_data(dir_inode, current_size, &entry);
    }

    /// Create a file inode and add it to a parent directory
    fn create_file(&mut self, parent_inode: u32, name: &str, data: &[u8], mode: u16) -> u32 {
        let inode_idx = self.allocate_inode().expect("out of inodes");
        self.inodes[inode_idx as usize].mode = mode;
        self.inodes[inode_idx as usize].links_count = 1;

        // Write file data
        if !data.is_empty() {
            self.write_inode_data(inode_idx, 0, data);
        }

        // Add to parent directory
        self.write_dir_entry(parent_inode, inode_idx, name, FT_REG_FILE);

        inode_idx
    }

    /// Create a directory inode and add it to a parent directory
    fn create_directory(&mut self, parent_inode: u32, name: &str) -> u32 {
        let inode_idx = self.allocate_inode().expect("out of inodes");
        self.inodes[inode_idx as usize].mode = 0x41ED; // directory, rwxr-xr-x
        self.inodes[inode_idx as usize].links_count = 2;

        // Create "." and ".." entries
        self.write_dir_entry(inode_idx, inode_idx, ".", FT_DIR);
        self.write_dir_entry(inode_idx, parent_inode, "..", FT_DIR);

        // Add to parent directory
        self.write_dir_entry(parent_inode, inode_idx, name, FT_DIR);

        // Increment parent's link count (subdirectory ".." points to parent)
        self.inodes[parent_inode as usize].links_count += 1;

        inode_idx
    }

    /// Populate from a host directory tree
    fn populate_from_dir(&mut self, host_dir: &Path, fs_inode: u32) {
        let mut queue: VecDeque<(PathBuf, u32)> = VecDeque::new();
        queue.push_back((host_dir.to_path_buf(), fs_inode));

        while let Some((dir_path, parent_inode)) = queue.pop_front() {
            let entries = match fs::read_dir(&dir_path) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Warning: cannot read {}: {}", dir_path.display(), e);
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.is_dir() {
                    let child_inode = self.create_directory(parent_inode, &name_str);
                    queue.push_back((path, child_inode));
                } else if metadata.is_file() {
                    let data = match fs::read(&path) {
                        Ok(d) => d,
                        Err(e) => {
                            eprintln!("Warning: cannot read {}: {}", path.display(), e);
                            continue;
                        }
                    };

                    // Determine mode from host permissions
                    let mode = if is_executable(&path) {
                        0x81ED // file, rwxr-xr-x
                    } else {
                        0x81A4 // file, rw-r--r--
                    };

                    self.create_file(parent_inode, &name_str, &data, mode);
                }
                // Skip symlinks, special files for now
            }
        }
    }

    /// Write the complete image to a file
    fn write_image(&self, output: &Path) -> std::io::Result<()> {
        let total_size = self.block_count as u64 * BLOCK_SIZE as u64;
        let mut file = File::create(output)?;

        // Pre-allocate the file
        file.set_len(total_size)?;

        // Write superblock (block 0)
        let sb = Superblock {
            magic: BLOCKFS_MAGIC,
            block_count: self.block_count,
            inode_count: self.inode_count,
            free_blocks: self.free_blocks_count(),
            free_inodes: self.inode_count - self.next_free_inode,
            first_data_block: self.first_data_block,
            block_size: BLOCK_SIZE as u32,
            inode_size: DISK_INODE_SIZE as u16,
            blocks_per_group: 8192,
            inodes_per_group: 2048,
            mount_time: 0,
            write_time: 0,
            mount_count: 0,
            max_mount_count: 100,
            state: 1, // Clean
            errors: 0,
        };

        file.seek(SeekFrom::Start(0))?;
        file.write_all(&sb.serialize())?;

        // Write bitmap (blocks 1..1+B)
        let bm_blocks = bitmap_blocks(self.block_count);
        let bitmap_start = 1u32;
        for i in 0..bm_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            let byte_offset = i as usize * BLOCK_SIZE;
            let bytes_remaining = self.bitmap.len().saturating_sub(byte_offset);
            let copy_len = bytes_remaining.min(BLOCK_SIZE);
            if copy_len > 0 {
                buf[..copy_len].copy_from_slice(&self.bitmap[byte_offset..byte_offset + copy_len]);
            }
            file.seek(SeekFrom::Start((bitmap_start + i) as u64 * BLOCK_SIZE as u64))?;
            file.write_all(&buf)?;
        }

        // Write inode table
        let inode_start = 1 + bm_blocks;
        let it_blocks = inode_table_blocks(self.inode_count);
        for blk_idx in 0..it_blocks {
            let mut buf = [0u8; BLOCK_SIZE];
            let base_inode = blk_idx as usize * INODES_PER_BLOCK;

            for slot in 0..INODES_PER_BLOCK {
                let inode_idx = base_inode + slot;
                if inode_idx >= self.inodes.len() {
                    break;
                }
                let off = slot * DISK_INODE_SIZE;
                self.inodes[inode_idx].serialize(&mut buf[off..off + DISK_INODE_SIZE]);
            }

            file.seek(SeekFrom::Start((inode_start + blk_idx) as u64 * BLOCK_SIZE as u64))?;
            file.write_all(&buf)?;
        }

        // Write data blocks
        for (block_idx, block_data) in self.blocks.iter().enumerate() {
            let idx = block_idx as u32;
            if idx < self.first_data_block {
                continue; // Skip metadata blocks
            }
            // Only write non-zero blocks
            if block_data.iter().any(|&b| b != 0) {
                file.seek(SeekFrom::Start(block_idx as u64 * BLOCK_SIZE as u64))?;
                file.write_all(block_data)?;
            }
        }

        file.sync_all()?;
        Ok(())
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    false
}

fn print_usage() {
    eprintln!("Usage: mkfs-blockfs --output <path> --size <MB> [--populate <dir>]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --output <path>    Output image file path");
    eprintln!("  --size <MB>        Image size in megabytes (e.g., 128)");
    eprintln!("  --populate <dir>   Populate filesystem from host directory");
    eprintln!("  --inodes <count>   Number of inodes (default: auto-calculated)");
    eprintln!();
    eprintln!("Example:");
    eprintln!("  mkfs-blockfs --output rootfs.img --size 128 --populate target/rootfs-busybox/");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut output: Option<String> = None;
    let mut size_mb: Option<u32> = None;
    let mut populate_dir: Option<String> = None;
    let mut inode_count_override: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                output = Some(args[i].clone());
            }
            "--size" | "-s" => {
                i += 1;
                size_mb = Some(args[i].parse().expect("invalid size"));
            }
            "--populate" | "-p" => {
                i += 1;
                populate_dir = Some(args[i].clone());
            }
            "--inodes" => {
                i += 1;
                inode_count_override = Some(args[i].parse().expect("invalid inode count"));
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                print_usage();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let output = match output {
        Some(o) => o,
        None => {
            eprintln!("Error: --output is required");
            print_usage();
            std::process::exit(1);
        }
    };

    let size_mb = match size_mb {
        Some(s) => s,
        None => {
            eprintln!("Error: --size is required");
            print_usage();
            std::process::exit(1);
        }
    };

    let block_count = size_mb * (1024 * 1024 / BLOCK_SIZE as u32);
    let inode_count = inode_count_override.unwrap_or_else(|| {
        // Default: 1 inode per 16KB (generous for small files)
        let auto = block_count / 4;
        auto.max(672).min(65536)
    });

    let first_data = computed_first_data_block(block_count, inode_count);

    println!("mkfs-blockfs: Creating BlockFS image");
    println!("  Output:           {}", output);
    println!("  Size:             {} MB ({} blocks)", size_mb, block_count);
    println!("  Inodes:           {}", inode_count);
    println!("  Bitmap blocks:    {}", bitmap_blocks(block_count));
    println!("  Inode table:      {} blocks", inode_table_blocks(inode_count));
    println!("  First data block: {}", first_data);
    println!(
        "  Data blocks:      {}",
        block_count.saturating_sub(first_data)
    );

    let mut builder = BlockFsBuilder::new(block_count, inode_count);

    if let Some(ref dir) = populate_dir {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            eprintln!("Error: {} is not a directory", dir);
            std::process::exit(1);
        }

        println!("  Populating from:  {}", dir);
        builder.populate_from_dir(dir_path, 0);
        println!(
            "  Inodes used:      {}/{}",
            builder.next_free_inode, inode_count
        );
        println!("  Free blocks:      {}", builder.free_blocks_count());
    }

    match builder.write_image(Path::new(&output)) {
        Ok(()) => {
            println!(
                "mkfs-blockfs: Image created successfully ({} MB)",
                size_mb
            );
        }
        Err(e) => {
            eprintln!("Error writing image: {}", e);
            std::process::exit(1);
        }
    }
}
