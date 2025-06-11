//! Capability space implementation
//!
//! Provides per-process capability tables with O(1) lookup.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use super::{
    object::ObjectRef,
    token::{CapabilityToken, Rights},
};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, collections::BTreeMap};

use spin::RwLock;

/// Size of L1 capability table (direct lookup)
const L1_SIZE: usize = 256;

/// Size of L2 capability tables
const L2_SIZE: usize = 256;

/// Type alias for L2 tables to reduce complexity
#[cfg(feature = "alloc")]
type L2Tables = BTreeMap<u16, Box<[RwLock<Option<CapEntry>>; L2_SIZE]>>;

/// A single capability entry in the capability space
pub struct CapEntry {
    /// The capability token
    pub capability: CapabilityToken,
    /// Object reference
    pub object: ObjectRef,
    /// Access rights
    pub rights: Rights,
    /// Usage count for statistics
    pub usage_count: AtomicU64,
}

impl CapEntry {
    pub fn new(capability: CapabilityToken, object: ObjectRef, rights: Rights) -> Self {
        Self {
            capability,
            object,
            rights,
            usage_count: AtomicU64::new(0),
        }
    }
}

/// Statistics for capability space
#[derive(Default)]
pub struct CapSpaceStats {
    pub total_caps: AtomicU64,
    pub lookups: AtomicU64,
    pub hits: AtomicU64,
    pub misses: AtomicU64,
}

/// Per-process capability space
pub struct CapabilitySpace {
    /// Fast lookup table (L1) - for first 256 capabilities
    l1_table: Box<[RwLock<Option<CapEntry>>; L1_SIZE]>,

    /// Second level tables (L2) - for capabilities beyond 256
    #[cfg(feature = "alloc")]
    l2_tables: RwLock<L2Tables>,

    /// Generation counter for this space
    generation: AtomicU8,

    /// Statistics
    stats: CapSpaceStats,
}

impl CapabilitySpace {
    /// Create a new capability space
    pub fn new() -> Self {
        // Initialize L1 table with None values
        let l1_table = Box::new(core::array::from_fn(|_| RwLock::new(None)));

        Self {
            l1_table,
            #[cfg(feature = "alloc")]
            l2_tables: RwLock::new(BTreeMap::new()),
            generation: AtomicU8::new(0),
            stats: CapSpaceStats::default(),
        }
    }

    /// O(1) lookup of capability
    pub fn lookup(&self, cap: CapabilityToken) -> Option<Rights> {
        self.stats.lookups.fetch_add(1, Ordering::Relaxed);

        let cap_id = cap.id() as usize;

        // Fast path: check L1 table
        if cap_id < L1_SIZE {
            let entry = self.l1_table[cap_id].read();
            if let Some(ref cap_entry) = *entry {
                if cap_entry.capability == cap {
                    self.stats.hits.fetch_add(1, Ordering::Relaxed);
                    cap_entry.usage_count.fetch_add(1, Ordering::Relaxed);
                    return Some(cap_entry.rights);
                }
            }
        }

        // Slow path: check L2 tables
        #[cfg(feature = "alloc")]
        {
            let l1_index = (cap_id >> 8) as u16;
            let l2_index = cap_id & 0xFF;

            let l2_tables = self.l2_tables.read();
            if let Some(l2_table) = l2_tables.get(&l1_index) {
                let entry = l2_table[l2_index].read();
                if let Some(ref cap_entry) = *entry {
                    if cap_entry.capability == cap {
                        self.stats.hits.fetch_add(1, Ordering::Relaxed);
                        cap_entry.usage_count.fetch_add(1, Ordering::Relaxed);
                        return Some(cap_entry.rights);
                    }
                }
            }
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Insert a capability into the space
    pub fn insert(
        &self,
        cap: CapabilityToken,
        object: ObjectRef,
        rights: Rights,
    ) -> Result<(), &'static str> {
        let cap_id = cap.id() as usize;

        // Fast path: insert into L1 table
        if cap_id < L1_SIZE {
            let mut entry = self.l1_table[cap_id].write();
            if entry.is_some() {
                return Err("Capability slot already occupied");
            }
            *entry = Some(CapEntry::new(cap, object, rights));
            self.stats.total_caps.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }

        // Slow path: insert into L2 table
        #[cfg(feature = "alloc")]
        {
            let l1_index = (cap_id >> 8) as u16;
            let l2_index = cap_id & 0xFF;

            let mut l2_tables = self.l2_tables.write();

            // Create L2 table if it doesn't exist
            let l2_table = l2_tables
                .entry(l1_index)
                .or_insert_with(|| Box::new(core::array::from_fn(|_| RwLock::new(None))));

            let mut entry = l2_table[l2_index].write();
            if entry.is_some() {
                return Err("Capability slot already occupied");
            }
            *entry = Some(CapEntry::new(cap, object, rights));
            self.stats.total_caps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        #[cfg(not(feature = "alloc"))]
        Err("Capability ID exceeds L1 table size")
    }

    /// Remove a capability from the space
    pub fn remove(&self, cap: CapabilityToken) -> Option<ObjectRef> {
        let cap_id = cap.id() as usize;

        // Fast path: remove from L1 table
        if cap_id < L1_SIZE {
            let mut entry = self.l1_table[cap_id].write();
            if let Some(cap_entry) = entry.take() {
                if cap_entry.capability == cap {
                    self.stats.total_caps.fetch_sub(1, Ordering::Relaxed);
                    return Some(cap_entry.object);
                }
            }
            return None;
        }

        // Slow path: remove from L2 table
        #[cfg(feature = "alloc")]
        {
            let l1_index = (cap_id >> 8) as u16;
            let l2_index = cap_id & 0xFF;

            let l2_tables = self.l2_tables.read();
            if let Some(l2_table) = l2_tables.get(&l1_index) {
                let mut entry = l2_table[l2_index].write();
                if let Some(cap_entry) = entry.take() {
                    if cap_entry.capability == cap {
                        self.stats.total_caps.fetch_sub(1, Ordering::Relaxed);
                        return Some(cap_entry.object);
                    }
                }
            }
        }

        None
    }

    /// Check if process has capability with specific rights
    pub fn check_rights(&self, cap: CapabilityToken, required: Rights) -> bool {
        if let Some(rights) = self.lookup(cap) {
            rights.contains(required)
        } else {
            false
        }
    }

    /// Increment generation counter (for revocation)
    pub fn increment_generation(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Get current generation
    pub fn generation(&self) -> u8 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Clear all capabilities
    pub fn clear(&self) {
        // Clear L1 table
        for i in 0..L1_SIZE {
            *self.l1_table[i].write() = None;
        }

        // Clear L2 tables
        #[cfg(feature = "alloc")]
        {
            self.l2_tables.write().clear();
        }

        self.stats.total_caps.store(0, Ordering::Relaxed);
    }

    /// Get statistics
    pub fn stats(&self) -> &CapSpaceStats {
        &self.stats
    }
}

impl Default for CapabilitySpace {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-CPU capability cache for fast repeated lookups
pub struct CapabilityCache {
    /// Cache entries (power of 2 for fast modulo)
    cache: [Option<CachedCap>; 16],
    /// Cache statistics
    hits: AtomicU64,
    misses: AtomicU64,
}

#[repr(align(64))] // Cache line aligned
pub struct CachedCap {
    pub capability: CapabilityToken,
    pub rights: Rights,
    pub last_used: u64, // Timestamp counter
}

impl CapabilityCache {
    pub const fn new() -> Self {
        const NONE: Option<CachedCap> = None;
        Self {
            cache: [NONE; 16],
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn lookup(&self, cap: CapabilityToken) -> Option<Rights> {
        let hash = (cap.id() as usize) & 0xF; // Fast modulo 16

        if let Some(ref cached) = self.cache[hash] {
            if cached.capability == cap {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(cached.rights);
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    #[inline]
    pub fn insert(&mut self, cap: CapabilityToken, rights: Rights) {
        let hash = (cap.id() as usize) & 0xF;

        self.cache[hash] = Some(CachedCap {
            capability: cap,
            rights,
            last_used: 0, // TODO: Use actual timestamp counter
        });
    }
}
