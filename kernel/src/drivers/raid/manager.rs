//! Software RAID Implementation
//!
//! Supports RAID levels 0 (striping), 1 (mirroring), and 5 (striping with
//! distributed parity). Includes stripe mapping, XOR parity computation,
//! array health monitoring, hot-spare replacement, and rebuild.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// RAID Superblock
// ---------------------------------------------------------------------------

/// mdadm v1.2 compatible superblock magic.
const RAID_SUPER_MAGIC: u32 = 0xA92B_4EFC;

/// Superblock size in bytes.
const SUPERBLOCK_SIZE: usize = 256;

/// Default chunk size (512 KB).
const DEFAULT_CHUNK_SIZE: u64 = 512 * 1024;

// ---------------------------------------------------------------------------
// RAID Level
// ---------------------------------------------------------------------------

/// Supported RAID levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidLevel {
    /// Striping (no redundancy).
    Raid0,
    /// Mirroring (full redundancy).
    Raid1,
    /// Striping with distributed parity.
    Raid5,
}

impl RaidLevel {
    /// Minimum number of disks for this level.
    pub fn min_disks(&self) -> usize {
        match self {
            Self::Raid0 => 2,
            Self::Raid1 => 2,
            Self::Raid5 => 3,
        }
    }

    /// Number of data disks given total disk count.
    pub fn data_disk_count(&self, total: usize) -> usize {
        match self {
            Self::Raid0 => total,
            Self::Raid1 => 1,
            Self::Raid5 => total.saturating_sub(1),
        }
    }
}

// ---------------------------------------------------------------------------
// Disk State
// ---------------------------------------------------------------------------

/// State of an individual disk in a RAID array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    /// Disk is active and healthy.
    Active,
    /// Disk has some errors but is still functional.
    Degraded,
    /// Disk has failed and is offline.
    Failed,
    /// Disk is being rebuilt.
    Rebuilding,
    /// Disk is a hot spare.
    Spare,
}

// ---------------------------------------------------------------------------
// RAID Disk
// ---------------------------------------------------------------------------

/// A physical disk member of a RAID array.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub struct RaidDisk {
    /// Disk identifier.
    pub id: u32,
    /// Device path (e.g., "/dev/vda").
    pub path: String,
    /// Current state.
    pub state: DiskState,
    /// Size in blocks.
    pub size_blocks: u64,
    /// Tick count of last error (0 = no error).
    pub last_error_tick: u64,
}

#[cfg(feature = "alloc")]
impl RaidDisk {
    /// Create a new active disk.
    pub fn new(id: u32, path: &str, size_blocks: u64) -> Self {
        Self {
            id,
            path: String::from(path),
            state: DiskState::Active,
            size_blocks,
            last_error_tick: 0,
        }
    }

    /// Mark disk as failed.
    pub fn mark_failed(&mut self, tick: u64) {
        self.state = DiskState::Failed;
        self.last_error_tick = tick;
    }

    /// Mark disk as rebuilding.
    pub fn mark_rebuilding(&mut self) {
        self.state = DiskState::Rebuilding;
    }

    /// Mark disk as active.
    pub fn mark_active(&mut self) {
        self.state = DiskState::Active;
    }
}

// ---------------------------------------------------------------------------
// RAID Superblock
// ---------------------------------------------------------------------------

/// mdadm v1.2-compatible superblock.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq)]
pub struct Superblock {
    /// Magic number (0xA92B4EFC).
    pub magic: u32,
    /// Major version (1).
    pub major_version: u32,
    /// Minor version (2).
    pub minor_version: u32,
    /// Array UUID.
    pub set_uuid: [u8; 16],
    /// Array name.
    pub set_name: String,
    /// Creation time (tick count).
    pub ctime: u64,
    /// RAID level.
    pub level: RaidLevel,
    /// Layout (left-symmetric = 0 for RAID5).
    pub layout: u32,
    /// Usable size per device in blocks.
    pub size: u64,
    /// Number of RAID disks (not counting spares).
    pub raid_disks: u32,
    /// Device number within array.
    pub dev_number: u32,
    /// Event count (incremented on every state change).
    pub events: u64,
    /// Data offset in blocks from start of device.
    pub data_offset: u64,
    /// Data size in blocks.
    pub data_size: u64,
}

#[cfg(feature = "alloc")]
impl Superblock {
    /// Create a new superblock for an array.
    pub fn new(name: &str, level: RaidLevel, raid_disks: u32, size: u64) -> Self {
        Self {
            magic: RAID_SUPER_MAGIC,
            major_version: 1,
            minor_version: 2,
            set_uuid: [0u8; 16],
            set_name: String::from(name),
            ctime: 0,
            level,
            layout: 0,
            size,
            raid_disks,
            dev_number: 0,
            events: 0,
            data_offset: 2048, // Standard mdadm v1.2 offset
            data_size: size,
        }
    }

    /// Serialize superblock to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SUPERBLOCK_SIZE);

        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.major_version.to_le_bytes());
        buf.extend_from_slice(&self.minor_version.to_le_bytes());
        // Pad to 12 bytes
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&self.set_uuid);

        // Set name (32 bytes, null-padded)
        let name_bytes = self.set_name.as_bytes();
        let name_len = core::cmp::min(name_bytes.len(), 32);
        buf.extend_from_slice(&name_bytes[..name_len]);
        buf.resize(buf.len() + (32 - name_len), 0);

        buf.extend_from_slice(&self.ctime.to_le_bytes());

        let level_val: u32 = match self.level {
            RaidLevel::Raid0 => 0,
            RaidLevel::Raid1 => 1,
            RaidLevel::Raid5 => 5,
        };
        buf.extend_from_slice(&level_val.to_le_bytes());
        buf.extend_from_slice(&self.layout.to_le_bytes());
        buf.extend_from_slice(&self.size.to_le_bytes());
        buf.extend_from_slice(&self.raid_disks.to_le_bytes());
        buf.extend_from_slice(&self.dev_number.to_le_bytes());
        buf.extend_from_slice(&self.events.to_le_bytes());
        buf.extend_from_slice(&self.data_offset.to_le_bytes());
        buf.extend_from_slice(&self.data_size.to_le_bytes());

        // Pad to SUPERBLOCK_SIZE
        buf.resize(SUPERBLOCK_SIZE, 0);
        buf
    }

    /// Deserialize superblock from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < SUPERBLOCK_SIZE {
            return None;
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != RAID_SUPER_MAGIC {
            return None;
        }

        let major_version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let minor_version = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        let mut set_uuid = [0u8; 16];
        set_uuid.copy_from_slice(&data[16..32]);

        // Set name (32 bytes at offset 32)
        let name_end = data[32..64].iter().position(|&b| b == 0).unwrap_or(32);
        let set_name = String::from_utf8_lossy(&data[32..32 + name_end]).into_owned();

        let ctime = u64::from_le_bytes([
            data[64], data[65], data[66], data[67], data[68], data[69], data[70], data[71],
        ]);

        let level_val = u32::from_le_bytes([data[72], data[73], data[74], data[75]]);
        let level = match level_val {
            0 => RaidLevel::Raid0,
            1 => RaidLevel::Raid1,
            5 => RaidLevel::Raid5,
            _ => return None,
        };

        let layout = u32::from_le_bytes([data[76], data[77], data[78], data[79]]);
        let size = u64::from_le_bytes([
            data[80], data[81], data[82], data[83], data[84], data[85], data[86], data[87],
        ]);
        let raid_disks = u32::from_le_bytes([data[88], data[89], data[90], data[91]]);
        let dev_number = u32::from_le_bytes([data[92], data[93], data[94], data[95]]);
        let events = u64::from_le_bytes([
            data[96], data[97], data[98], data[99], data[100], data[101], data[102], data[103],
        ]);
        let data_offset = u64::from_le_bytes([
            data[104], data[105], data[106], data[107], data[108], data[109], data[110], data[111],
        ]);
        let data_size = u64::from_le_bytes([
            data[112], data[113], data[114], data[115], data[116], data[117], data[118], data[119],
        ]);

        Some(Self {
            magic,
            major_version,
            minor_version,
            set_uuid,
            set_name,
            ctime,
            level,
            layout,
            size,
            raid_disks,
            dev_number,
            events,
            data_offset,
            data_size,
        })
    }
}

// ---------------------------------------------------------------------------
// Array State
// ---------------------------------------------------------------------------

/// RAID array operational state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayState {
    /// Array is not started.
    Inactive,
    /// All disks are healthy, no writes pending.
    Clean,
    /// Array is operating normally.
    Active,
    /// One or more disks have failed but array is still operational.
    Degraded,
    /// A failed disk is being rebuilt.
    Rebuilding,
}

// ---------------------------------------------------------------------------
// RAID Array
// ---------------------------------------------------------------------------

/// RAID error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidError {
    /// Not enough disks for the RAID level.
    NotEnoughDisks,
    /// Array is degraded and cannot tolerate more failures.
    ArrayDegraded,
    /// Array has failed (too many disk failures).
    ArrayFailed,
    /// Disk not found.
    DiskNotFound,
    /// Invalid block address.
    InvalidAddress,
    /// Array already exists.
    AlreadyExists,
    /// Array not found.
    NotFound,
    /// I/O error on disk.
    IoError,
    /// Invalid configuration.
    InvalidConfig,
}

/// Stripe mapping result: which disk and offset to access.
#[derive(Debug, Clone, Copy)]
pub struct StripeMap {
    /// Index of the data disk within the array's disk list.
    pub disk_index: usize,
    /// Block offset on that disk.
    pub disk_offset: u64,
    /// For RAID5: index of the parity disk.
    pub parity_disk: Option<usize>,
}

/// A RAID array composed of multiple disks.
#[cfg(feature = "alloc")]
#[derive(Debug, PartialEq)]
pub struct RaidArray {
    /// Array name.
    pub name: String,
    /// Array UUID.
    pub uuid: [u8; 16],
    /// RAID level.
    pub level: RaidLevel,
    /// Chunk size in blocks.
    pub chunk_size: u64,
    /// Member disks.
    pub disks: Vec<RaidDisk>,
    /// Hot spare disks.
    pub spares: Vec<RaidDisk>,
    /// Array state.
    pub state: ArrayState,
    /// Superblock.
    pub superblock: Superblock,
    /// Rebuild progress (0-100).
    pub rebuild_progress: u8,
}

#[cfg(feature = "alloc")]
impl RaidArray {
    /// Create a new RAID array.
    pub fn new(name: &str, level: RaidLevel, disks: Vec<RaidDisk>) -> Result<Self, RaidError> {
        if disks.len() < level.min_disks() {
            return Err(RaidError::NotEnoughDisks);
        }

        // Use the smallest disk's size for uniform striping
        let min_size = disks.iter().map(|d| d.size_blocks).min().unwrap_or(0);

        let superblock = Superblock::new(name, level, disks.len() as u32, min_size);

        Ok(Self {
            name: String::from(name),
            uuid: [0u8; 16],
            level,
            chunk_size: DEFAULT_CHUNK_SIZE / 512, // Convert to blocks (assuming 512-byte blocks)
            disks,
            spares: Vec::new(),
            state: ArrayState::Active,
            superblock,
            rebuild_progress: 0,
        })
    }

    /// Map a logical block to a physical stripe location (RAID0).
    pub fn stripe_map(&self, logical_block: u64) -> Result<StripeMap, RaidError> {
        if self.disks.is_empty() {
            return Err(RaidError::NotEnoughDisks);
        }

        let num_disks = self.disks.len() as u64;
        let stripe = logical_block / self.chunk_size;
        let offset_in_chunk = logical_block % self.chunk_size;

        let disk_index = (stripe % num_disks) as usize;
        let disk_offset = (stripe / num_disks) * self.chunk_size + offset_in_chunk;

        Ok(StripeMap {
            disk_index,
            disk_offset,
            parity_disk: None,
        })
    }

    /// Map a logical block for RAID5 (left-symmetric parity rotation).
    pub fn raid5_map(&self, logical_block: u64) -> Result<StripeMap, RaidError> {
        if self.disks.len() < 3 {
            return Err(RaidError::NotEnoughDisks);
        }

        let num_disks = self.disks.len() as u64;
        let data_disks = num_disks - 1;
        let stripe = logical_block / self.chunk_size;
        let offset_in_chunk = logical_block % self.chunk_size;

        // Which full stripe (row of chunks across all data disks)
        let stripe_row = stripe / data_disks;
        // Position within the stripe row
        let data_index = stripe % data_disks;

        // Left-symmetric: parity rotates backward
        let parity_disk = (num_disks - 1 - (stripe_row % num_disks)) as usize;

        // Map data disk: skip over the parity position
        let mut physical_disk = data_index as usize;
        if physical_disk >= parity_disk {
            physical_disk += 1;
        }

        let disk_offset = stripe_row * self.chunk_size + offset_in_chunk;

        Ok(StripeMap {
            disk_index: physical_disk,
            disk_offset,
            parity_disk: Some(parity_disk),
        })
    }

    /// Read a stripe (dispatch by RAID level).
    pub fn read_stripe(&self, logical_block: u64) -> Result<StripeMap, RaidError> {
        match self.level {
            RaidLevel::Raid0 => self.stripe_map(logical_block),
            RaidLevel::Raid1 => self.mirror_read(logical_block),
            RaidLevel::Raid5 => self.raid5_map(logical_block),
        }
    }

    /// Write a stripe (dispatch by RAID level, returns all disks to write).
    pub fn write_stripe(&self, logical_block: u64) -> Result<Vec<StripeMap>, RaidError> {
        match self.level {
            RaidLevel::Raid0 => {
                let map = self.stripe_map(logical_block)?;
                Ok(vec![map])
            }
            RaidLevel::Raid1 => self.mirror_write(logical_block),
            RaidLevel::Raid5 => {
                let map = self.raid5_map(logical_block)?;
                // For RAID5 write: need to update data disk + parity disk
                let mut writes = vec![map];
                if let Some(parity_idx) = map.parity_disk {
                    writes.push(StripeMap {
                        disk_index: parity_idx,
                        disk_offset: map.disk_offset,
                        parity_disk: None,
                    });
                }
                Ok(writes)
            }
        }
    }

    /// RAID1: read from any active mirror.
    fn mirror_read(&self, logical_block: u64) -> Result<StripeMap, RaidError> {
        // Read from first active disk
        for (i, disk) in self.disks.iter().enumerate() {
            if disk.state == DiskState::Active {
                return Ok(StripeMap {
                    disk_index: i,
                    disk_offset: logical_block,
                    parity_disk: None,
                });
            }
        }
        Err(RaidError::ArrayFailed)
    }

    /// RAID1: write to all active mirrors.
    fn mirror_write(&self, logical_block: u64) -> Result<Vec<StripeMap>, RaidError> {
        let mut writes = Vec::new();
        for (i, disk) in self.disks.iter().enumerate() {
            if disk.state == DiskState::Active || disk.state == DiskState::Rebuilding {
                writes.push(StripeMap {
                    disk_index: i,
                    disk_offset: logical_block,
                    parity_disk: None,
                });
            }
        }
        if writes.is_empty() {
            return Err(RaidError::ArrayFailed);
        }
        Ok(writes)
    }

    /// Compute XOR parity across data blocks.
    pub fn compute_parity(blocks: &[&[u8]]) -> Vec<u8> {
        if blocks.is_empty() {
            return Vec::new();
        }
        let len = blocks[0].len();
        let mut parity = vec![0u8; len];
        for block in blocks {
            xor_blocks(&mut parity, &block[..core::cmp::min(block.len(), len)]);
        }
        parity
    }

    /// Rebuild a failed disk from parity and remaining data.
    pub fn rebuild(&mut self, failed_disk_idx: usize) -> Result<(), RaidError> {
        if failed_disk_idx >= self.disks.len() {
            return Err(RaidError::DiskNotFound);
        }

        match self.level {
            RaidLevel::Raid0 => {
                // RAID0 cannot rebuild
                Err(RaidError::ArrayFailed)
            }
            RaidLevel::Raid1 => {
                // Find an active source disk
                let has_active = self.disks.iter().any(|d| d.state == DiskState::Active);

                if !has_active {
                    return Err(RaidError::ArrayFailed);
                }

                self.disks[failed_disk_idx].mark_rebuilding();
                self.state = ArrayState::Rebuilding;
                // In production: copy all data from active mirror to rebuilding disk
                self.rebuild_progress = 100;
                self.disks[failed_disk_idx].mark_active();
                self.update_state();
                Ok(())
            }
            RaidLevel::Raid5 => {
                // Need all other disks to be active
                let failed_count = self
                    .disks
                    .iter()
                    .filter(|d| d.state == DiskState::Failed)
                    .count();

                if failed_count > 1 {
                    return Err(RaidError::ArrayFailed);
                }

                self.disks[failed_disk_idx].mark_rebuilding();
                self.state = ArrayState::Rebuilding;
                // In production: XOR all other disks to reconstruct failed disk
                self.rebuild_progress = 100;
                self.disks[failed_disk_idx].mark_active();
                self.update_state();
                Ok(())
            }
        }
    }

    /// Check array health and update state.
    pub fn check_health(&mut self) -> ArrayState {
        self.update_state();
        self.state
    }

    /// Replace a failed disk with a spare.
    pub fn replace_disk(&mut self, failed_disk_idx: usize) -> Result<(), RaidError> {
        if failed_disk_idx >= self.disks.len() {
            return Err(RaidError::DiskNotFound);
        }
        if self.disks[failed_disk_idx].state != DiskState::Failed {
            return Err(RaidError::InvalidConfig);
        }

        // Find a spare
        let spare_idx = self.spares.iter().position(|s| s.state == DiskState::Spare);

        if let Some(idx) = spare_idx {
            let mut spare = self.spares.remove(idx);
            spare.state = DiskState::Rebuilding;
            spare.id = self.disks[failed_disk_idx].id;
            self.disks[failed_disk_idx] = spare;
            self.rebuild(failed_disk_idx)
        } else {
            Err(RaidError::DiskNotFound)
        }
    }

    /// Add a hot spare disk.
    pub fn add_spare(&mut self, disk: RaidDisk) {
        let mut spare = disk;
        spare.state = DiskState::Spare;
        self.spares.push(spare);
    }

    /// Get the number of active disks.
    pub fn active_disk_count(&self) -> usize {
        self.disks
            .iter()
            .filter(|d| d.state == DiskState::Active)
            .count()
    }

    /// Get the number of failed disks.
    pub fn failed_disk_count(&self) -> usize {
        self.disks
            .iter()
            .filter(|d| d.state == DiskState::Failed)
            .count()
    }

    /// Get usable capacity in blocks.
    pub fn capacity_blocks(&self) -> u64 {
        let min_size = self.disks.iter().map(|d| d.size_blocks).min().unwrap_or(0);

        match self.level {
            RaidLevel::Raid0 => min_size * self.disks.len() as u64,
            RaidLevel::Raid1 => min_size,
            RaidLevel::Raid5 => min_size * (self.disks.len() as u64 - 1),
        }
    }

    /// Update array state based on disk states.
    fn update_state(&mut self) {
        let failed = self.failed_disk_count();
        let rebuilding = self
            .disks
            .iter()
            .filter(|d| d.state == DiskState::Rebuilding)
            .count();

        self.state = match self.level {
            RaidLevel::Raid0 => {
                if failed > 0 {
                    ArrayState::Inactive
                } else {
                    ArrayState::Active
                }
            }
            RaidLevel::Raid1 => {
                if failed >= self.disks.len() {
                    ArrayState::Inactive
                } else if rebuilding > 0 {
                    ArrayState::Rebuilding
                } else if failed > 0 {
                    ArrayState::Degraded
                } else {
                    ArrayState::Active
                }
            }
            RaidLevel::Raid5 => {
                if failed > 1 {
                    ArrayState::Inactive
                } else if rebuilding > 0 {
                    ArrayState::Rebuilding
                } else if failed == 1 {
                    ArrayState::Degraded
                } else {
                    ArrayState::Active
                }
            }
        };
    }
}

/// XOR a source block into a destination block.
pub fn xor_blocks(dest: &mut [u8], src: &[u8]) {
    let len = core::cmp::min(dest.len(), src.len());
    for i in 0..len {
        dest[i] ^= src[i];
    }
}

// ---------------------------------------------------------------------------
// RAID Manager
// ---------------------------------------------------------------------------

/// RAID manager: manages multiple RAID arrays.
#[cfg(feature = "alloc")]
pub struct RaidManager {
    /// Arrays indexed by name.
    arrays: BTreeMap<String, RaidArray>,
}

#[cfg(feature = "alloc")]
impl Default for RaidManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl RaidManager {
    /// Create a new RAID manager.
    pub fn new() -> Self {
        Self {
            arrays: BTreeMap::new(),
        }
    }

    /// Create a new RAID array.
    pub fn create_array(
        &mut self,
        name: &str,
        level: RaidLevel,
        disks: Vec<RaidDisk>,
    ) -> Result<(), RaidError> {
        if self.arrays.contains_key(name) {
            return Err(RaidError::AlreadyExists);
        }
        let array = RaidArray::new(name, level, disks)?;
        self.arrays.insert(String::from(name), array);
        Ok(())
    }

    /// Destroy an array.
    pub fn destroy_array(&mut self, name: &str) -> Result<(), RaidError> {
        self.arrays
            .remove(name)
            .map(|_| ())
            .ok_or(RaidError::NotFound)
    }

    /// Add a hot spare to an array.
    pub fn add_spare(&mut self, array_name: &str, disk: RaidDisk) -> Result<(), RaidError> {
        let array = self.arrays.get_mut(array_name).ok_or(RaidError::NotFound)?;
        array.add_spare(disk);
        Ok(())
    }

    /// Get status summary of all arrays.
    pub fn get_status(&self) -> Vec<(&str, ArrayState, usize, usize)> {
        self.arrays
            .iter()
            .map(|(name, array)| {
                (
                    name.as_str(),
                    array.state,
                    array.active_disk_count(),
                    array.disks.len(),
                )
            })
            .collect()
    }

    /// Get an array by name.
    pub fn get_array(&self, name: &str) -> Option<&RaidArray> {
        self.arrays.get(name)
    }

    /// Get a mutable reference to an array by name.
    pub fn get_array_mut(&mut self, name: &str) -> Option<&mut RaidArray> {
        self.arrays.get_mut(name)
    }

    /// Number of managed arrays.
    pub fn array_count(&self) -> usize {
        self.arrays.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_disks(count: usize, size: u64) -> Vec<RaidDisk> {
        (0..count)
            .map(|i| {
                RaidDisk::new(
                    i as u32,
                    &alloc::format!("/dev/vd{}", (b'a' + i as u8) as char),
                    size,
                )
            })
            .collect()
    }

    #[test]
    fn test_raid_level_min_disks() {
        assert_eq!(RaidLevel::Raid0.min_disks(), 2);
        assert_eq!(RaidLevel::Raid1.min_disks(), 2);
        assert_eq!(RaidLevel::Raid5.min_disks(), 3);
    }

    #[test]
    fn test_raid_level_data_disks() {
        assert_eq!(RaidLevel::Raid0.data_disk_count(4), 4);
        assert_eq!(RaidLevel::Raid1.data_disk_count(2), 1);
        assert_eq!(RaidLevel::Raid5.data_disk_count(4), 3);
    }

    #[test]
    fn test_raid0_not_enough_disks() {
        let disks = make_disks(1, 1000);
        assert_eq!(
            RaidArray::new("md0", RaidLevel::Raid0, disks),
            Err(RaidError::NotEnoughDisks)
        );
    }

    #[test]
    fn test_raid0_stripe_map() {
        let disks = make_disks(3, 10000);
        let array = RaidArray::new("md0", RaidLevel::Raid0, disks).unwrap();

        let map = array.stripe_map(0).unwrap();
        assert_eq!(map.disk_index, 0);
        assert_eq!(map.disk_offset, 0);

        // Second chunk goes to disk 1
        let map2 = array.stripe_map(array.chunk_size).unwrap();
        assert_eq!(map2.disk_index, 1);
    }

    #[test]
    fn test_raid1_mirror_read() {
        let disks = make_disks(2, 10000);
        let array = RaidArray::new("md1", RaidLevel::Raid1, disks).unwrap();
        let map = array.read_stripe(500).unwrap();
        assert_eq!(map.disk_offset, 500);
    }

    #[test]
    fn test_raid1_mirror_write() {
        let disks = make_disks(2, 10000);
        let array = RaidArray::new("md1", RaidLevel::Raid1, disks).unwrap();
        let writes = array.write_stripe(100).unwrap();
        // Should write to both mirrors
        assert_eq!(writes.len(), 2);
    }

    #[test]
    fn test_raid5_map() {
        let disks = make_disks(4, 10000);
        let array = RaidArray::new("md5", RaidLevel::Raid5, disks).unwrap();
        let map = array.raid5_map(0).unwrap();
        assert!(map.parity_disk.is_some());
        // Data disk should not be the parity disk
        assert_ne!(map.disk_index, map.parity_disk.unwrap());
    }

    #[test]
    fn test_xor_blocks() {
        let mut a = [0xAA, 0xBB, 0xCC, 0xDD];
        let b = [0x55, 0x44, 0x33, 0x22];
        xor_blocks(&mut a, &b);
        assert_eq!(a, [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_compute_parity() {
        let block1: &[u8] = &[0xFF, 0x00, 0xAA];
        let block2: &[u8] = &[0x00, 0xFF, 0x55];
        let parity = RaidArray::compute_parity(&[block1, block2]);
        assert_eq!(parity, &[0xFF, 0xFF, 0xFF]);

        // XOR parity with one block should give the other
        let mut recovered = parity.clone();
        xor_blocks(&mut recovered, block1);
        assert_eq!(recovered, block2);
    }

    #[test]
    fn test_superblock_serialize_deserialize() {
        let sb = Superblock::new("md0", RaidLevel::Raid5, 4, 100000);
        let bytes = sb.serialize();
        assert_eq!(bytes.len(), SUPERBLOCK_SIZE);

        let parsed = Superblock::deserialize(&bytes).unwrap();
        assert_eq!(parsed.magic, RAID_SUPER_MAGIC);
        assert_eq!(parsed.level, RaidLevel::Raid5);
        assert_eq!(parsed.raid_disks, 4);
        assert_eq!(parsed.set_name, "md0");
    }

    #[test]
    fn test_superblock_bad_magic() {
        let mut bytes = vec![0u8; SUPERBLOCK_SIZE];
        bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        assert!(Superblock::deserialize(&bytes).is_none());
    }

    #[test]
    fn test_capacity_blocks() {
        let disks = make_disks(4, 10000);
        let r0 = RaidArray::new("r0", RaidLevel::Raid0, disks.clone()).unwrap();
        assert_eq!(r0.capacity_blocks(), 40000);

        let r1 = RaidArray::new("r1", RaidLevel::Raid1, make_disks(2, 10000)).unwrap();
        assert_eq!(r1.capacity_blocks(), 10000);

        let r5 = RaidArray::new("r5", RaidLevel::Raid5, disks).unwrap();
        assert_eq!(r5.capacity_blocks(), 30000);
    }

    #[test]
    fn test_raid_manager_create_destroy() {
        let mut mgr = RaidManager::new();
        let disks = make_disks(3, 10000);
        mgr.create_array("md0", RaidLevel::Raid5, disks).unwrap();
        assert_eq!(mgr.array_count(), 1);

        // Duplicate name should fail
        let disks2 = make_disks(2, 10000);
        assert_eq!(
            mgr.create_array("md0", RaidLevel::Raid1, disks2),
            Err(RaidError::AlreadyExists)
        );

        mgr.destroy_array("md0").unwrap();
        assert_eq!(mgr.array_count(), 0);
    }

    #[test]
    fn test_raid_manager_status() {
        let mut mgr = RaidManager::new();
        let disks = make_disks(2, 5000);
        mgr.create_array("md0", RaidLevel::Raid1, disks).unwrap();

        let status = mgr.get_status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].0, "md0");
        assert_eq!(status[0].1, ArrayState::Active);
        assert_eq!(status[0].2, 2); // active disks
        assert_eq!(status[0].3, 2); // total disks
    }
}
