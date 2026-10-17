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
//! Rules that keep data safe (ADR 0003, ADR 0011):
//! - **Metadata blocks stay in memory until sync.** The superblock, bitmap and
//!   inode table are written only at sync and there is no journal, so a
//!   directory or indirect-pointer block written early would let the
//!   last-synced inodes point at blocks the last-synced bitmap calls free. Such
//!   blocks are *pinned*: dirty until the next sync, never evicted.
//! - **File data blocks may be written back before sync** (BM-FS-01), when more
//!   than the dirty limit of them accumulate: BlockFS never reuses a block
//!   freed since the last sync (it waits in quarantine until that sync has
//!   written the new bitmap), so a data block either belonged to the same file
//!   at the last sync (an in-place overwrite) or was free on the disk (nothing
//!   on the disk refers to it). A crash cannot corrupt another file. A block
//!   reused from quarantine, because the disk was otherwise full, is pinned
//!   like metadata.
//! - Only clean slots are evicted; when every slot is dirty the pool grows past
//!   its capacity. The capacity bounds clean data, the dirty limit unpinned
//!   dirty data; pinned data is bounded by what changes between syncs.
//! - A newly allocated block is inserted zeroed and dirty, so stale bytes on
//!   the disk can never be read back through it.
//! - A freed block is dropped without write-back.
//!
//! A miss does not hold the cache across the disk read (N-118 / BM-DRV-06):
//! [`BlockCache::lookup`] claims a slot, marks it *filling* and hands its
//! buffer out in a [`FillTicket`]; the caller drops the cache lock, reads
//! the block into the ticket and calls [`BlockCache::finish_fill`]. Readers
//! of other blocks carry on meanwhile; a reader of the same block sees
//! [`Lookup::Busy`] and waits. A filling slot is clean and never evicted.

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
    /// Its buffer is out in a [`FillTicket`]; `data` is empty until the
    /// fill finishes.
    filling: bool,
    /// Dirty and not writable before the next sync (see the module docs).
    pinned: bool,
    data: Box<[u8]>,
}

/// Result of [`BlockCache::lookup`].
pub(crate) enum Lookup {
    /// Resident in this slot ([`BlockCache::data`]).
    Hit(usize),
    /// Not allocated: reads as zeros.
    Zero,
    /// Another reader is filling it: wait, then look again.
    Busy,
    /// A miss: read the block into the ticket, then
    /// [`BlockCache::finish_fill`].
    Fill(FillTicket),
}

/// A claimed slot's buffer, out of the cache while the block is read into
/// it. It must go back through [`BlockCache::finish_fill`]; until then the
/// slot stays busy.
#[must_use]
pub(crate) struct FillTicket {
    slot: usize,
    block: u32,
    buf: Box<[u8]>,
}

impl FillTicket {
    pub(crate) fn block(&self) -> u32 {
        self.block
    }

    /// The buffer to read the block into.
    pub(crate) fn buf(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

/// Counters, reported by `blockfs` diagnostics and the tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Dirty blocks written by `flush` (sync).
    pub writebacks: u64,
    /// Unpinned data blocks written before sync (`writeback_unpinned`).
    pub early_writebacks: u64,
}

pub(crate) struct BlockCache {
    /// Block number -> slot index, or `NO_SLOT` when not resident.
    index: Vec<u32>,
    slots: Vec<Slot>,
    /// Slots holding no block; their buffers are kept for reuse.
    free: Vec<u32>,
    hand: usize,
    capacity: usize,
    /// Dirty slots that are not pinned.
    dirty_unpinned: usize,
    /// Above this many unpinned dirty blocks, `over_dirty_limit` asks the
    /// filesystem to write them back.
    dirty_limit: usize,
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
            dirty_unpinned: 0,
            dirty_limit: DEFAULT_CAPACITY_BLOCKS,
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

    /// Whether enough unpinned dirty blocks have accumulated that the
    /// filesystem should write them back (`writeback_unpinned`).
    pub(crate) fn over_dirty_limit(&self) -> bool {
        self.dirty_unpinned > self.dirty_limit
    }

    /// Change the dirty limit (tests and mount options).
    pub(crate) fn set_dirty_limit(&mut self, blocks: usize) {
        self.dirty_limit = blocks;
    }

    /// Mark `slot` dirty; `pin` keeps it until the next sync. A pinned slot
    /// stays pinned whatever later writes say.
    fn mark_dirty(&mut self, slot: usize, pin: bool) {
        let s = &mut self.slots[slot];
        let was_unpinned = s.dirty && !s.pinned;
        s.dirty = true;
        s.pinned |= pin;
        match (was_unpinned, !s.pinned) {
            (false, true) => self.dirty_unpinned += 1,
            (true, false) => self.dirty_unpinned -= 1,
            _ => {}
        }
    }

    /// Mark `slot` clean (written, or its contents dropped).
    fn mark_clean(&mut self, slot: usize) {
        let s = &mut self.slots[slot];
        if s.dirty && !s.pinned {
            self.dirty_unpinned -= 1;
        }
        s.dirty = false;
        s.pinned = false;
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
        if self.slots[slot].filling {
            return None;
        }
        self.slots[slot].referenced = true;
        self.stats.hits += 1;
        Some(slot)
    }

    /// Look `block` up for reading without doing any I/O (see the module
    /// docs): a hit, zeros, busy, or a ticket to fill it.
    pub(crate) fn lookup(&mut self, block: u32, allocated: bool) -> Result<Lookup, KernelError> {
        if (block as usize) >= self.index.len() {
            return Err(KernelError::FsError(crate::error::FsError::IoError));
        }
        if self.is_filling(block) {
            return Ok(Lookup::Busy);
        }
        if let Some(slot) = self.resident(block) {
            return Ok(Lookup::Hit(slot));
        }
        if !allocated {
            return Ok(Lookup::Zero);
        }
        self.stats.misses += 1;
        let slot = self.claim(block);
        let s = &mut self.slots[slot];
        s.filling = true;
        let buf = core::mem::take(&mut s.data);
        Ok(Lookup::Fill(FillTicket { slot, block, buf }))
    }

    /// Whether `block` is being filled by a [`FillTicket`].
    pub(crate) fn is_filling(&self, block: u32) -> bool {
        match self.index.get(block as usize) {
            Some(&slot) if slot != NO_SLOT => self.slots[slot as usize].filling,
            _ => false,
        }
    }

    /// A resident slot's contents.
    pub(crate) fn data(&self, slot: usize) -> &[u8] {
        &self.slots[slot].data
    }

    /// Return a ticket's buffer with the outcome of its read. On success
    /// the block is resident in the returned slot; on failure the slot is
    /// released and the next access reads the disk again.
    pub(crate) fn finish_fill(
        &mut self,
        ticket: FillTicket,
        read: Result<(), KernelError>,
    ) -> Result<usize, KernelError> {
        let FillTicket { slot, block, buf } = ticket;
        let s = &mut self.slots[slot];
        // The slot cannot change hands while filling: eviction skips it,
        // and discard and invalidation run with the filesystem locked
        // exclusively, so no ticket is outstanding then.
        debug_assert!(s.filling && s.block == block);
        s.data = buf;
        s.filling = false;
        if let Err(e) = read {
            self.release(slot);
            return Err(e);
        }
        Ok(slot)
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
        debug_assert!(!s.dirty, "claim takes only clean slots");
        s.block = block;
        s.dirty = false;
        s.pinned = false;
        s.referenced = true;
        self.index[block as usize] = slot as u32;
        slot
    }

    fn grow(&mut self) -> usize {
        self.slots.push(Slot {
            block: NO_SLOT,
            dirty: false,
            referenced: false,
            filling: false,
            pinned: false,
            data: vec![0u8; BLOCK_SIZE].into_boxed_slice(),
        });
        self.slots.len() - 1
    }

    /// CLOCK sweep: clear reference bits until an unreferenced clean slot
    /// comes round. Dirty slots are pinned until sync (see the module docs),
    /// and slots being filled hold no data yet.
    /// Two full turns without a candidate means every slot is dirty.
    fn evict(&mut self) -> Option<usize> {
        let n = self.slots.len();
        for _ in 0..2 * n {
            let i = self.hand;
            self.hand = (self.hand + 1) % n;
            let s = &mut self.slots[i];
            if s.dirty || s.filling {
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
    #[cfg(test)]
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
    /// first, and the slot is marked dirty; `pin` (metadata, or a block
    /// reused from quarantine) keeps it until the next sync.
    pub(crate) fn write(
        &mut self,
        block: u32,
        allocated: bool,
        disk: Option<Disk<'_>>,
        pin: bool,
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
        self.mark_dirty(slot, pin);
        Ok(&mut self.slots[slot].data)
    }

    /// Insert a newly allocated block: zeroed and dirty, no disk read.
    pub(crate) fn insert_zeroed(&mut self, block: u32, pin: bool) -> Result<(), KernelError> {
        if (block as usize) >= self.index.len() {
            return Err(KernelError::FsError(crate::error::FsError::IoError));
        }
        let slot = match self.resident(block) {
            Some(slot) => slot,
            None => self.claim(block),
        };
        self.slots[slot].data.fill(0);
        self.mark_dirty(slot, pin);
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
        self.mark_clean(slot);
        let s = &mut self.slots[slot];
        if s.block != NO_SLOT {
            self.index[s.block as usize] = NO_SLOT;
        }
        s.block = NO_SLOT;
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
        for i in 0..self.slots.len() {
            let s = &self.slots[i];
            if s.block == NO_SLOT || !s.dirty || !disk.can_write(s.block) {
                continue;
            }
            disk.backend.write_block(s.block as u64, &s.data)?;
            self.mark_clean(i);
            written += 1;
        }
        self.stats.writebacks += written as u64;
        Ok(written)
    }

    /// Write back every dirty block that is not pinned, so it becomes
    /// clean and evictable (BM-FS-01; safe before sync, see the module
    /// docs). Returns how many were written.
    pub(crate) fn writeback_unpinned(&mut self, disk: Disk<'_>) -> Result<usize, KernelError> {
        let mut written = 0;
        for i in 0..self.slots.len() {
            let s = &self.slots[i];
            if s.block == NO_SLOT || !s.dirty || s.pinned || !disk.can_write(s.block) {
                continue;
            }
            disk.backend.write_block(s.block as u64, &s.data)?;
            self.mark_clean(i);
            written += 1;
        }
        self.stats.early_writebacks += written as u64;
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
            let s = &self.slots[i];
            if s.block != NO_SLOT && !s.dirty && !s.filling {
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
        c.write(1, true, Some(m.disk()), false).unwrap()[0] = 0xAA;
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
            c.write(b, true, Some(m.disk()), false).unwrap()[0] = 0xF0 | b as u8;
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
        c.insert_zeroed(6, false).unwrap();
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
        c.write(2, true, Some(m.disk()), false).unwrap()[0] = 0x11;
        c.write(3, true, Some(m.disk()), false).unwrap()[0] = 0x22;
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
        c.insert_zeroed(6, false).unwrap();
        assert_eq!(c.flush(m.disk()).unwrap(), 0);
        assert_eq!(c.unwritable_dirty(m.disk()).collect::<Vec<_>>(), vec![6]);
    }

    #[test]
    fn out_of_range_block_is_an_error() {
        let mut c = BlockCache::new(4, 4);
        assert!(c.read(4, true, None, |_| ()).is_err());
        assert!(c.write(9, true, None, false).is_err());
        assert!(c.lookup(4, true).is_err());
    }

    /// Run a fill the way `BlockFsInner::with_block` does: the disk read
    /// happens with no cache borrow held.
    fn fill_now(c: &mut BlockCache, mut t: FillTicket, m: &MemDisk) -> usize {
        let r = m.read_block(t.block() as u64, t.buf());
        c.finish_fill(t, r).unwrap()
    }

    /// N-118 / BM-DRV-06: a miss hands out a ticket and the cache stays
    /// usable while the read is in flight: other blocks hit and fill, the
    /// block being filled reports busy, and it hits once the fill is done.
    #[test]
    fn fill_ticket_leaves_the_cache_usable_during_io() {
        let m = MemDisk::new(16);
        let mut c = BlockCache::new(16, 4);
        let Ok(Lookup::Fill(t5)) = c.lookup(5, true) else {
            panic!("a miss hands out a fill ticket");
        };
        assert!(matches!(c.lookup(5, true), Ok(Lookup::Busy)));
        assert!(c.is_filling(5));
        // Another block fills and hits meanwhile.
        let Ok(Lookup::Fill(t3)) = c.lookup(3, true) else {
            panic!("block 3 misses");
        };
        let s3 = fill_now(&mut c, t3, &m);
        assert_eq!(c.data(s3)[0], 3);
        assert!(matches!(c.lookup(3, true), Ok(Lookup::Hit(_))));
        // Unallocated blocks still read as zeros.
        assert!(matches!(c.lookup(9, false), Ok(Lookup::Zero)));
        let s5 = fill_now(&mut c, t5, &m);
        assert!(!c.is_filling(5));
        let Ok(Lookup::Hit(hit)) = c.lookup(5, true) else {
            panic!("block 5 is resident after its fill");
        };
        assert_eq!(hit, s5);
        assert_eq!(c.data(s5)[0], 5);
        assert_eq!(c.stats().misses, 2);
        assert_eq!(*m.reads.borrow(), 2);
    }

    /// A slot being filled has no valid data, so it is never evicted: the
    /// pool grows instead.
    #[test]
    fn filling_slot_is_never_evicted() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 1);
        let Ok(Lookup::Fill(t1)) = c.lookup(1, true) else {
            panic!("miss");
        };
        let Ok(Lookup::Fill(t2)) = c.lookup(2, true) else {
            panic!("miss");
        };
        assert_eq!(
            c.slot_count(),
            2,
            "grew past capacity rather than evict a filling slot"
        );
        let s2 = fill_now(&mut c, t2, &m);
        let s1 = fill_now(&mut c, t1, &m);
        assert_eq!((c.data(s1)[0], c.data(s2)[0]), (1, 2));
    }

    /// A failed read releases its slot: nothing is left busy, and the next
    /// access tries the disk again.
    #[test]
    fn failed_fill_releases_the_slot() {
        let mut c = BlockCache::new(8, 2);
        let Ok(Lookup::Fill(t)) = c.lookup(4, true) else {
            panic!("miss");
        };
        let err = KernelError::FsError(crate::error::FsError::IoError);
        assert!(c.finish_fill(t, Err(err)).is_err());
        assert!(!c.is_filling(4));
        assert!(matches!(c.lookup(4, true), Ok(Lookup::Fill(_))));
    }

    /// BM-FS-01: unpinned dirty data past the limit is written back and
    /// becomes evictable; pinned (metadata) blocks stay dirty until flush.
    #[test]
    fn writeback_unpinned_leaves_pinned_blocks_dirty() {
        let m = MemDisk::new(16);
        let mut c = BlockCache::new(16, 2);
        c.set_dirty_limit(2);
        for b in 0..4 {
            c.write(b, true, Some(m.disk()), false).unwrap()[0] = 0xD0 | b as u8;
        }
        c.write(9, true, Some(m.disk()), true).unwrap()[0] = 0x99;
        // A later unpinned write does not unpin it.
        c.write(9, true, Some(m.disk()), false).unwrap()[1] = 0x98;
        assert!(c.over_dirty_limit());
        assert_eq!(c.writeback_unpinned(m.disk()).unwrap(), 4);
        assert!(!c.over_dirty_limit());
        assert_eq!(c.dirty_count(), 1, "the pinned block is still dirty");
        assert_eq!(m.blocks.borrow()[2][0], 0xD2);
        assert_eq!(m.blocks.borrow()[9][0], 9, "pinned block not written");
        assert_eq!(c.stats().early_writebacks, 4);
        // The written slots are clean again, so the pool can shrink back
        // around the pinned block.
        for b in 10..16 {
            first_byte(&mut c, b, m.disk());
        }
        assert!(c.slot_count() <= 5);
        // Sync writes the pinned block and clears the pin.
        assert_eq!(c.flush(m.disk()).unwrap(), 1);
        assert_eq!(m.blocks.borrow()[9][..2], [0x99, 0x98]);
        assert_eq!(c.dirty_count(), 0);
    }

    /// The unpinned-dirty count follows every transition, including
    /// discard of a dirty block and a pinned block being re-dirtied.
    #[test]
    fn dirty_limit_counts_only_unpinned_dirty_blocks() {
        let m = MemDisk::new(8);
        let mut c = BlockCache::new(8, 8);
        c.set_dirty_limit(1);
        c.insert_zeroed(1, false).unwrap();
        c.insert_zeroed(2, true).unwrap();
        assert!(!c.over_dirty_limit(), "one unpinned, one pinned");
        c.write(3, true, Some(m.disk()), false).unwrap();
        assert!(c.over_dirty_limit());
        c.discard(3);
        assert!(!c.over_dirty_limit());
        c.write(1, true, Some(m.disk()), true).unwrap();
        assert_eq!(
            c.writeback_unpinned(m.disk()).unwrap(),
            0,
            "1 is pinned now"
        );
        c.flush(m.disk()).unwrap();
        c.write(4, true, Some(m.disk()), false).unwrap();
        c.write(5, true, Some(m.disk()), false).unwrap();
        assert!(
            c.over_dirty_limit(),
            "flush reset the count, new writes count"
        );
    }
}
