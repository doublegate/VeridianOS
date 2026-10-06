//! Bounded write-back cache of BlockFS blocks (FS-PERF-01).
//!
//! Before this cache, mounting read every allocated block of the image into
//! its own heap `Vec`. A 7,000-block root image cost ~28 MB of heap and 3.5 s
//! of I/O at mount, and could not mount at all on the 8 MB non-x86 heaps.
//!
//! Now blocks are read on first use into a pool of reusable 4 KiB slots,
//! replaced by CLOCK (second chance), an LRU approximation with O(1) work per
//! access. Buffers are recycled rather than freed, so the pool never churns
//! the allocator; that matters on AArch64/RISC-V, whose bump allocator never
//! reclaims memory.
//!
//! Rules that keep data safe:
//! - **Dirty blocks stay in memory until sync.** Only clean slots are evicted;
//!   when every slot is dirty the pool grows past its capacity. The superblock,
//!   bitmap and inode table are written only at sync and there is no journal,
//!   so writing a data block earlier could overwrite a block that was freed and
//!   reused while the last-synced inodes still point at it. A crash would then
//!   corrupt the old file. Pinning keeps the disk exactly at its last-synced
//!   state between syncs, as before this cache. The capacity therefore bounds
//!   clean (read) data; dirty data is bounded by how much is written between
//!   syncs.
//! - A newly allocated block is inserted zeroed and dirty, so stale bytes on
//!   the disk can never be read back through it.
//! - A freed block is dropped without write-back.

use alloc::{boxed::Box, vec, vec::Vec};

use super::{DiskBackend, BLOCK_SIZE};
use crate::error::KernelError;

const NO_SLOT: u32 = u32::MAX;

/// Default cache size: 16 MiB on x86_64, 1 MiB elsewhere (the AArch64 and
/// RISC-V kernel heaps are 8 MiB and never free).
#[cfg(target_arch = "x86_64")]
pub(crate) const DEFAULT_CAPACITY_BLOCKS: usize = 16 * 1024 * 1024 / BLOCK_SIZE;
#[cfg(not(target_arch = "x86_64"))]
pub(crate) const DEFAULT_CAPACITY_BLOCKS: usize = 1024 * 1024 / BLOCK_SIZE;

/// The disk as the cache sees it. `device_blocks` and `writable` are sampled
/// once when the backend is attached; querying the device on every access
/// would take the driver lock each time.
#[derive(Clone, Copy)]
pub(crate) struct Disk<'a> {
    pub backend: &'a dyn DiskBackend,
    pub device_blocks: u64,
    pub writable: bool,
}

impl Disk<'_> {
    fn can_write(&self, block: u32) -> bool {
        self.writable && (block as u64) < self.device_blocks
    }
}

struct Slot {
    block: u32,
    dirty: bool,
    referenced: bool,
    data: Box<[u8]>,
}

/// Counters, reported by `blockfs` diagnostics and the tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Dirty blocks written by `flush` (sync).
    pub writebacks: u64,
}

pub(crate) struct BlockCache {
    /// Block number -> slot index, or `NO_SLOT` when not resident.
    index: Vec<u32>,
    slots: Vec<Slot>,
    /// Slots holding no block; their buffers are kept for reuse.
    free: Vec<u32>,
    hand: usize,
    capacity: usize,
    stats: CacheStats,
}

impl BlockCache {
    pub(crate) fn new(block_count: usize, capacity: usize) -> Self {
        Self {
            index: vec![NO_SLOT; block_count],
            slots: Vec::new(),
            free: Vec::new(),
            hand: 0,
            capacity: capacity.max(1),
            stats: CacheStats::default(),
        }
    }

    pub(crate) fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Slots currently allocated (resident plus recycled).
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn dirty_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.dirty && s.block != NO_SLOT)
            .count()
    }

    fn resident(&mut self, block: u32) -> Option<usize> {
        let slot = *self.index.get(block as usize)?;
        if slot == NO_SLOT {
            return None;
        }
        let slot = slot as usize;
        self.slots[slot].referenced = true;
        self.stats.hits += 1;
        Some(slot)
    }

    /// Get a slot for `block`, evicting or growing as needed. The returned
    /// slot is mapped to `block`, clean and referenced; its contents are
    /// stale and must be filled by the caller.
    fn claim(&mut self, block: u32) -> usize {
        let slot = if let Some(slot) = self.free.pop() {
            slot as usize
        } else if self.slots.len() < self.capacity {
            self.grow()
        } else {
            match self.evict() {
                Some(slot) => slot,
                None => self.grow(),
            }
        };
        let s = &mut self.slots[slot];
        s.block = block;
        s.dirty = false;
        s.referenced = true;
        self.index[block as usize] = slot as u32;
        slot
    }

    fn grow(&mut self) -> usize {
        self.slots.push(Slot {
            block: NO_SLOT,
            dirty: false,
            referenced: false,
            data: vec![0u8; BLOCK_SIZE].into_boxed_slice(),
        });
        self.slots.len() - 1
    }

    /// CLOCK sweep: clear reference bits until an unreferenced clean slot
    /// comes round. Dirty slots are pinned until sync (see the module docs).
    /// Two full turns without a candidate means every slot is dirty.
    fn evict(&mut self) -> Option<usize> {
        let n = self.slots.len();
        for _ in 0..2 * n {
            let i = self.hand;
            self.hand = (self.hand + 1) % n;
            let s = &mut self.slots[i];
            if s.dirty {
                continue;
            }
            if s.referenced {
                s.referenced = false;
                continue;
            }
            self.index[s.block as usize] = NO_SLOT;
            s.block = NO_SLOT;
            self.stats.evictions += 1;
            return Some(i);
        }
        None
    }

    /// Read access. Blocks that are not allocated read as zeros without
    /// touching the disk or the pool.
    pub(crate) fn read<R>(
        &mut self,
        block: u32,
        allocated: bool,
        disk: Option<Disk<'_>>,
        f: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, KernelError> {
        if (block as usize) >= self.index.len() {
            return Err(KernelError::FsError(crate::error::FsError::IoError));
        }
        if let Some(slot) = self.resident(block) {
            return Ok(f(&self.slots[slot].data));
        }
        if !allocated {
            return Ok(f(&super::ZERO_BLOCK));
        }
        let slot = self.fill(block, disk)?;
        Ok(f(&self.slots[slot].data))
    }

    /// Write access to an existing block: its current contents are loaded
    /// first, and the slot is marked dirty.
    pub(crate) fn write(
        &mut self,
        block: u32,
        allocated: bool,
        disk: Option<Disk<'_>>,
    ) -> Result<&mut [u8], KernelError> {
        if (block as usize) >= self.index.len() {
            return Err(KernelError::FsError(crate::error::FsError::IoError));
        }
        let slot = match self.resident(block) {
            Some(slot) => slot,
            None if allocated => self.fill(block, disk)?,
            None => {
                let slot = self.claim(block);
                self.slots[slot].data.fill(0);
                slot
            }
        };
        let s = &mut self.slots[slot];
        s.dirty = true;
        Ok(&mut s.data)
    }

    /// Insert a newly allocated block: zeroed and dirty, no disk read.
    pub(crate) fn insert_zeroed(&mut self, block: u32) -> Result<(), KernelError> {
        if (block as usize) >= self.index.len() {
            return Err(KernelError::FsError(crate::error::FsError::IoError));
        }
        let slot = match self.resident(block) {
            Some(slot) => slot,
            None => self.claim(block),
        };
        let s = &mut self.slots[slot];
        s.data.fill(0);
        s.dirty = true;
        Ok(())
    }

    fn fill(&mut self, block: u32, disk: Option<Disk<'_>>) -> Result<usize, KernelError> {
        self.stats.misses += 1;
        let slot = self.claim(block);
        let result = match disk {
            Some(d) if (block as u64) < d.device_blocks => d
                .backend
                .read_block(block as u64, &mut self.slots[slot].data),
            _ => {
                self.slots[slot].data.fill(0);
                Ok(())
            }
        };
        if let Err(e) = result {
            self.release(slot);
            return Err(e);
        }
        Ok(slot)
    }

    fn release(&mut self, slot: usize) {
        let s = &mut self.slots[slot];
        if s.block != NO_SLOT {
            self.index[s.block as usize] = NO_SLOT;
        }
        s.block = NO_SLOT;
        s.dirty = false;
        s.referenced = false;
        self.free.push(slot as u32);
    }

    /// A freed block's contents are dead: drop it without write-back.
    pub(crate) fn discard(&mut self, block: u32) {
        if let Some(&slot) = self.index.get(block as usize) {
            if slot != NO_SLOT {
                self.release(slot as usize);
            }
        }
    }

    /// Write every dirty block the device can hold. Returns how many were
    /// written. Blocks past the end of the device stay dirty (and are
    /// reported by the caller).
    pub(crate) fn flush(&mut self, disk: Disk<'_>) -> Result<usize, KernelError> {
        let mut written = 0;
        for s in self.slots.iter_mut() {
            if s.block == NO_SLOT || !s.dirty || !disk.can_write(s.block) {
                continue;
            }
            disk.backend.write_block(s.block as u64, &s.data)?;
            s.dirty = false;
            written += 1;
        }
        self.stats.writebacks += written as u64;
        Ok(written)
    }

    /// Blocks that are dirty but cannot be written to `disk`.
    pub(crate) fn unwritable_dirty<'a>(&'a self, disk: Disk<'a>) -> impl Iterator<Item = u32> + 'a {
        self.slots
            .iter()
            .filter(move |s| s.block != NO_SLOT && s.dirty && !disk.can_write(s.block))
            .map(|s| s.block)
    }

    /// Drop every clean block, so the next access rereads the disk.
    pub(crate) fn invalidate_clean(&mut self) {
        for i in 0..self.slots.len() {
            if self.slots[i].block != NO_SLOT && !self.slots[i].dirty {
                self.release(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::cell::RefCell;

    use super::*;

    /// In-memory disk recording reads and writes.
    struct MemDisk {
        blocks: RefCell<Vec<Vec<u8>>>,
        reads: RefCell<usize>,
        writes: RefCell<usize>,
    }

    // SAFETY: test-only, single-threaded.
    unsafe impl Sync for MemDisk {}

    impl MemDisk {
        fn new(n: usize) -> Self {
            let blocks = (0..n).map(|i| vec![i as u8; BLOCK_SIZE]).collect();
            Self {
                blocks: RefCell::new(blocks),
                reads: RefCell::new(0),
                writes: RefCell::new(0),
            }
        }
        fn disk(&self) -> Disk<'_> {
            Disk {
                backend: self,
                device_blocks: self.blocks.borrow().len() as u64,
                writable: true,
            }
        }
    }

    impl DiskBackend for MemDisk {
        fn read_block(&self, n: u64, buf: &mut [u8]) -> Result<(), KernelError> {
            *self.reads.borrow_mut() += 1;
            buf[..BLOCK_SIZE].copy_from_slice(&self.blocks.borrow()[n as usize]);
            Ok(())
        }
        fn write_block(&self, n: u64, data: &[u8]) -> Result<(), KernelError> {
            *self.writes.borrow_mut() += 1;
            self.blocks.borrow_mut()[n as usize].copy_from_slice(&data[..BLOCK_SIZE]);
            Ok(())
        }
        fn block_count(&self) -> u64 {
            self.blocks.borrow().len() as u64
        }
        fn is_read_only(&self) -> bool {
            false
        }
    }

    fn first_byte(c: &mut BlockCache, b: u32, d: Disk<'_>) -> u8 {
        c.read(b, true, Some(d), |x| x[0]).unwrap()
    }

    #[test]
    fn reads_lazily_and_hits_after_first_use() {
        let m = MemDisk::new(16);
        let mut c = BlockCache::new(16, 4);
        assert_eq!(*m.reads.borrow(), 0);
        assert_eq!(first_byte(&mut c, 5, m.disk()), 5);
        assert_eq!(first_byte(&mut c, 5, m.disk()), 5);
        assert_eq!(*m.reads.borrow(), 1);
        assert_eq!(c.stats().misses, 1);
        assert_eq!(c.stats().hits, 1);
    }

    #[test]
    fn unallocated_blocks_read_zero_without_io() {
        let m = MemDisk::new(16);
        let mut c = BlockCache::new(16, 4);
        assert_eq!(c.read(3, false, Some(m.disk()), |x| x[0]).unwrap(), 0);
        assert_eq!(*m.reads.borrow(), 0);
        assert_eq!(c.slot_count(), 0);
    }

    #[test]
    fn pool_stays_bounded_and_recycles_buffers() {
        let m = MemDisk::new(64);
        let mut c = BlockCache::new(64, 4);
        for b in 0..64 {
            assert_eq!(first_byte(&mut c, b, m.disk()), b as u8);
        }
        assert_eq!(c.slot_count(), 4);
        assert_eq!(c.stats().evictions, 60);
    }

    #[test]
    fn dirty_blocks_stay_in_memory_until_flush() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 2);
        c.write(1, true, Some(m.disk())).unwrap()[0] = 0xAA;
        for b in 2..8 {
            first_byte(&mut c, b, m.disk());
        }
        // The disk keeps its last-synced contents however much is read.
        assert_eq!(*m.writes.borrow(), 0);
        assert_eq!(m.blocks.borrow()[1][0], 1);
        assert_eq!(first_byte(&mut c, 1, m.disk()), 0xAA);
        // Clean data was still recycled around the pinned block.
        assert_eq!(c.slot_count(), 2);
        assert_eq!(c.flush(m.disk()).unwrap(), 1);
        assert_eq!(m.blocks.borrow()[1][0], 0xAA);
        // Partial write kept the rest of the block.
        assert_eq!(m.blocks.borrow()[1][1], 1);
    }

    #[test]
    fn pool_grows_rather_than_dropping_dirty_data() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 2);
        for b in 0..4 {
            c.write(b, true, Some(m.disk())).unwrap()[0] = 0xF0 | b as u8;
        }
        assert_eq!(c.slot_count(), 4);
        assert_eq!(*m.writes.borrow(), 0);
        for b in 0..4 {
            assert_eq!(first_byte(&mut c, b, m.disk()), 0xF0 | b as u8);
        }
        // Once flushed, the slots are clean and evictable again.
        c.flush(m.disk()).unwrap();
        for b in 4..8 {
            first_byte(&mut c, b, m.disk());
        }
        assert_eq!(c.slot_count(), 4);
    }

    #[test]
    fn new_blocks_never_expose_stale_disk_bytes() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 1);
        c.insert_zeroed(6).unwrap();
        assert_eq!(first_byte(&mut c, 6, m.disk()), 0);
        // Pinned dirty: other reads cannot push it out and reread disk.
        first_byte(&mut c, 2, m.disk());
        assert_eq!(first_byte(&mut c, 6, m.disk()), 0);
        c.flush(m.disk()).unwrap();
        assert_eq!(m.blocks.borrow()[6][0], 0);
    }

    #[test]
    fn discard_drops_without_writeback_and_flush_writes_dirty() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 4);
        c.write(2, true, Some(m.disk())).unwrap()[0] = 0x11;
        c.write(3, true, Some(m.disk())).unwrap()[0] = 0x22;
        c.discard(2);
        assert_eq!(c.dirty_count(), 1);
        assert_eq!(c.flush(m.disk()).unwrap(), 1);
        assert_eq!(m.blocks.borrow()[2][0], 2);
        assert_eq!(m.blocks.borrow()[3][0], 0x22);
        assert_eq!(c.dirty_count(), 0);
    }

    #[test]
    fn blocks_past_device_end_stay_dirty() {
        let m = MemDisk::new(4);
        let mut c = BlockCache::new(8, 8);
        c.insert_zeroed(6).unwrap();
        assert_eq!(c.flush(m.disk()).unwrap(), 0);
        assert_eq!(c.unwritable_dirty(m.disk()).collect::<Vec<_>>(), vec![6]);
    }

    #[test]
    fn out_of_range_block_is_an_error() {
        let mut c = BlockCache::new(4, 4);
        assert!(c.read(4, true, None, |_| ()).is_err());
        assert!(c.write(9, true, None).is_err());
    }
}
