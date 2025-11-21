//! Capability space implementation
//!
//! Provides per-process capability tables with O(1) lookup.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use super::{
    object::ObjectRef,
    token::{CapabilityToken, Rights},
    types::CapabilityId,
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
type L2Tables = BTreeMap<u16, Box<[RwLock<Option<CapabilityEntry>>; L2_SIZE]>>;

/// A single capability entry in the capability space
pub struct CapabilityEntry {
    /// The capability token
    pub capability: CapabilityToken,
    /// Object reference
    pub object: ObjectRef,
    /// Access rights
    pub rights: Rights,
    /// Usage count for statistics
    pub usage_count: AtomicU64,
    /// Inheritance flags
    pub inheritance_flags: u32,
}

impl Clone for CapabilityEntry {
    fn clone(&self) -> Self {
        Self {
            capability: self.capability,
            object: self.object.clone(),
            rights: self.rights,
            usage_count: AtomicU64::new(self.usage_count.load(Ordering::Relaxed)),
            inheritance_flags: self.inheritance_flags,
        }
    }
}

impl CapabilityEntry {
    pub fn new(capability: CapabilityToken, object: ObjectRef, rights: Rights) -> Self {
        use super::inheritance::InheritanceFlags;
        Self {
            capability,
            object,
            rights,
            usage_count: AtomicU64::new(0),
            inheritance_flags: InheritanceFlags::INHERITABLE,
        }
    }

    pub fn with_flags(mut self, flags: u32) -> Self {
        self.inheritance_flags = flags;
        self
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
    l1_table: Box<[RwLock<Option<CapabilityEntry>>; L1_SIZE]>,

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
            *entry = Some(CapabilityEntry::new(cap, object, rights));
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
            *entry = Some(CapabilityEntry::new(cap, object, rights));
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

    /// Clone capabilities from another capability space
    ///
    /// Copies all capabilities from the source space to this space.
    /// Used during fork() to give child process same capabilities as parent.
    pub fn clone_from(&self, other: &Self) -> Result<(), &'static str> {
        // Clear existing capabilities first
        self.clear();

        // Clone L1 table entries
        for i in 0..L1_SIZE {
            let source_entry = other.l1_table[i].read();
            if let Some(ref entry) = *source_entry {
                *self.l1_table[i].write() = Some(CapabilityEntry {
                    capability: entry.capability,
                    object: entry.object.clone(),
                    rights: entry.rights,
                    usage_count: AtomicU64::new(0),
                    inheritance_flags: entry.inheritance_flags,
                });
                self.stats.total_caps.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Clone L2 table entries
        #[cfg(feature = "alloc")]
        {
            let source_l2 = other.l2_tables.read();
            let mut dest_l2 = self.l2_tables.write();

            for (l1_index, source_table) in source_l2.iter() {
                let new_table: Box<[RwLock<Option<CapabilityEntry>>; L2_SIZE]> =
                    Box::new(core::array::from_fn(|_| RwLock::new(None)));

                for j in 0..L2_SIZE {
                    let source_entry = source_table[j].read();
                    if let Some(ref entry) = *source_entry {
                        *new_table[j].write() = Some(CapabilityEntry {
                            capability: entry.capability,
                            object: entry.object.clone(),
                            rights: entry.rights,
                            usage_count: AtomicU64::new(0),
                            inheritance_flags: entry.inheritance_flags,
                        });
                        self.stats.total_caps.fetch_add(1, Ordering::Relaxed);
                    }
                }

                dest_l2.insert(*l1_index, new_table);
            }
        }

        // Set generation to match source
        self.generation
            .store(other.generation.load(Ordering::SeqCst), Ordering::SeqCst);

        Ok(())
    }

    /// Revoke all capabilities (for process cleanup)
    pub fn revoke_all(&mut self) {
        self.clear();
        // Increment generation to invalidate any outstanding references
        self.increment_generation();
    }

    /// Revoke a specific capability by ID
    pub fn revoke(&mut self, cap_id: CapabilityId) -> Result<(), &'static str> {
        // Create a token with the given ID and current generation
        let token = CapabilityToken::from_id_and_generation(cap_id, self.generation());

        if self.remove(token).is_some() {
            Ok(())
        } else {
            Err("Capability not found")
        }
    }

    /// Create a capability (simplified for testing)
    pub fn create_capability(
        &mut self,
        rights: Rights,
        object: ObjectRef,
    ) -> Result<CapabilityId, &'static str> {
        // Simple ID allocation - in real implementation this would use the global
        // manager
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

        if id > 0xFFFF_FFFF_FFFF {
            return Err("Capability ID space exhausted");
        }

        let token = CapabilityToken::new(id, self.generation(), 0, 0);
        self.insert(token, object, rights)?;

        Ok(CapabilityId(id))
    }

    /// Get statistics
    pub fn stats(&self) -> &CapSpaceStats {
        &self.stats
    }

    /// Iterate over all capabilities (for inheritance)
    #[cfg(feature = "alloc")]
    pub fn iter_capabilities<F>(&self, mut f: F) -> Result<(), &'static str>
    where
        F: FnMut(&CapabilityEntry) -> bool,
    {
        // Iterate L1 table
        for i in 0..L1_SIZE {
            let entry_guard = self.l1_table[i].read();
            if let Some(ref entry) = *entry_guard {
                if !f(entry) {
                    return Ok(()); // Early exit if function returns false
                }
            }
        }

        // Iterate L2 tables
        let l2_tables = self.l2_tables.read();
        for (_, l2_table) in l2_tables.iter() {
            for i in 0..L2_SIZE {
                let entry_guard = l2_table[i].read();
                if let Some(ref entry) = *entry_guard {
                    if !f(entry) {
                        return Ok(()); // Early exit
                    }
                }
            }
        }

        Ok(())
    }

    /// Get capability entry by ID (for inheritance)
    pub fn get_entry(&self, cap_id: usize) -> Option<CapabilityEntry> {
        // Fast path: check L1 table
        if cap_id < L1_SIZE {
            let entry = self.l1_table[cap_id].read();
            return entry.clone();
        }

        // Slow path: check L2 tables
        #[cfg(feature = "alloc")]
        {
            let l1_index = (cap_id >> 8) as u16;
            let l2_index = cap_id & 0xFF;

            let l2_tables = self.l2_tables.read();
            if let Some(l2_table) = l2_tables.get(&l1_index) {
                let entry = l2_table[l2_index].read();
                return entry.clone();
            }
        }

        None
    }

    /// Lookup and get full capability entry
    pub fn lookup_entry(&self, cap: CapabilityToken) -> Option<(ObjectRef, Rights)> {
        let cap_id = cap.id() as usize;

        // Fast path: check L1 table
        if cap_id < L1_SIZE {
            let entry = self.l1_table[cap_id].read();
            if let Some(ref cap_entry) = *entry {
                if cap_entry.capability == cap {
                    self.stats.hits.fetch_add(1, Ordering::Relaxed);
                    cap_entry.usage_count.fetch_add(1, Ordering::Relaxed);
                    return Some((cap_entry.object.clone(), cap_entry.rights));
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
                        return Some((cap_entry.object.clone(), cap_entry.rights));
                    }
                }
            }
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
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
