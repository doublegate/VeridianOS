//! Capability space implementation
//!
//! Per-process capability tables (CAP-PERF-01): one open-addressed hash
//! table keyed by capability ID, under a single lock. Lookup is O(1) on
//! average and memory is proportional to the capabilities a process holds;
//! an empty space allocates nothing.
//!
//! The previous layout was a 256-slot L1 array plus 256-slot L2 arrays keyed
//! by `id >> 8`. Capability IDs come from one global allocator, so nearly
//! every capability landed in an L2 array of its own: about 14 KiB of
//! mostly empty slots per capability, behind a map lock taken on every
//! lookup, and the inheritance code that walked "the L1 range" missed most
//! capabilities entirely.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

use spin::RwLock;

use super::{
    object::ObjectRef,
    token::{CapabilityToken, Rights},
    types::CapabilityId,
};
use crate::error::KernelError;

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

    /// A copy for another space: same capability, fresh usage count.
    fn copy_for_new_space(&self) -> Self {
        Self {
            usage_count: AtomicU64::new(0),
            ..self.clone()
        }
    }
}

/// Statistics for capability space
#[derive(Default)]
pub struct CapSpaceStats {
    pub total_caps: AtomicU64,
    /// Lookups that found the capability (lookups = hits + misses; not
    /// counted separately to keep a shared counter off the lookup path).
    pub hits: AtomicU64,
    pub misses: AtomicU64,
}

/// Default capability quota per process
pub const DEFAULT_CAP_QUOTA: usize = 256;

enum Slot {
    Empty,
    /// A removed entry; probes continue past it.
    Tombstone,
    Full(CapabilityEntry),
}

/// Open-addressed (linear probing) table keyed by capability ID. The
/// capacity is zero or a power of two, and at most 3/4 of it is in use
/// (live entries plus tombstones), so every probe meets an empty slot.
#[derive(Default)]
struct CapTable {
    slots: Vec<Slot>,
    full: usize,
    tombstones: usize,
}

impl CapTable {
    const MIN_CAPACITY: usize = 8;

    fn home(&self, id: u64) -> usize {
        // Fibonacci hashing: IDs are sequential, so spread them.
        (id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & (self.slots.len() - 1)
    }

    /// Slot index of the entry with `id`.
    fn find(&self, id: u64) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut i = self.home(id);
        loop {
            match &self.slots[i] {
                Slot::Empty => return None,
                Slot::Full(e) if e.capability.id() == id => return Some(i),
                _ => i = (i + 1) & mask,
            }
        }
    }

    fn get(&self, id: u64) -> Option<&CapabilityEntry> {
        match &self.slots[self.find(id)?] {
            Slot::Full(e) => Some(e),
            _ => None,
        }
    }

    /// Insert an entry whose ID is not present.
    fn insert_new(&mut self, entry: CapabilityEntry) {
        if (self.full + self.tombstones + 1) * 4 > self.slots.len() * 3 {
            // Rehash; doubles only when live entries need the room.
            let needed = (self.full + 1) * 2;
            self.rehash(needed.next_power_of_two().max(Self::MIN_CAPACITY));
        }
        let mask = self.slots.len() - 1;
        let mut i = self.home(entry.capability.id());
        loop {
            match self.slots[i] {
                Slot::Empty => break,
                Slot::Tombstone => {
                    self.tombstones -= 1;
                    break;
                }
                Slot::Full(_) => i = (i + 1) & mask,
            }
        }
        self.slots[i] = Slot::Full(entry);
        self.full += 1;
    }

    fn rehash(&mut self, capacity: usize) {
        let old = core::mem::take(&mut self.slots);
        self.slots = (0..capacity).map(|_| Slot::Empty).collect();
        self.full = 0;
        self.tombstones = 0;
        for slot in old {
            if let Slot::Full(e) = slot {
                self.insert_new(e);
            }
        }
    }

    fn remove_at(&mut self, i: usize) -> Option<CapabilityEntry> {
        let Slot::Full(e) = core::mem::replace(&mut self.slots[i], Slot::Tombstone) else {
            return None;
        };
        self.full -= 1;
        self.tombstones += 1;
        if self.full == 0 {
            // Give the memory back once the space is empty.
            *self = Self::default();
        }
        Some(e)
    }

    fn entries(&self) -> impl Iterator<Item = &CapabilityEntry> {
        self.slots.iter().filter_map(|s| match s {
            Slot::Full(e) => Some(e),
            _ => None,
        })
    }
}

/// Per-process capability space
pub struct CapabilitySpace {
    table: RwLock<CapTable>,

    /// Generation counter for this space
    generation: AtomicU8,

    /// Maximum number of capabilities allowed in this space
    quota: usize,

    /// Number of capabilities currently in this space (mirrors the table;
    /// readable without the lock)
    used: AtomicUsize,

    /// Statistics
    stats: CapSpaceStats,
}

impl CapabilitySpace {
    /// Create a new capability space with default quota
    pub fn new() -> Self {
        Self::with_quota(DEFAULT_CAP_QUOTA)
    }

    /// Create a new capability space with a specific quota
    pub fn with_quota(quota: usize) -> Self {
        Self {
            table: RwLock::new(CapTable::default()),
            generation: AtomicU8::new(0),
            quota,
            used: AtomicUsize::new(0),
            stats: CapSpaceStats::default(),
        }
    }

    /// Get the current number of capabilities in this space
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Get the quota for this space
    pub fn quota(&self) -> usize {
        self.quota
    }

    /// Slots allocated by the table (for tests and diagnostics).
    pub(crate) fn table_capacity(&self) -> usize {
        self.table.read().slots.len()
    }

    /// Run `f` on the entry for exactly `cap` (same ID and generation),
    /// counting the lookup.
    fn with_exact<R>(
        &self,
        cap: CapabilityToken,
        f: impl FnOnce(&CapabilityEntry) -> R,
    ) -> Option<R> {
        let table = self.table.read();
        match table.get(cap.id()) {
            Some(entry) if entry.capability == cap => {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                entry.usage_count.fetch_add(1, Ordering::Relaxed);
                Some(f(entry))
            }
            _ => {
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// O(1) lookup of capability
    pub fn lookup(&self, cap: CapabilityToken) -> Option<Rights> {
        self.with_exact(cap, |e| e.rights)
    }

    /// Insert a capability into the space
    pub fn insert(
        &self,
        cap: CapabilityToken,
        object: ObjectRef,
        rights: Rights,
    ) -> Result<(), KernelError> {
        self.insert_entry(CapabilityEntry::new(cap, object, rights))
    }

    /// Insert a complete entry (keeps its inheritance flags).
    pub(crate) fn insert_entry(&self, entry: CapabilityEntry) -> Result<(), KernelError> {
        let mut table = self.table.write();
        // Checked under the lock, so concurrent inserts cannot overshoot.
        if table.full >= self.quota {
            return Err(KernelError::ResourceExhausted {
                resource: "capability quota",
            });
        }
        let id = entry.capability.id();
        if table.find(id).is_some() {
            return Err(KernelError::AlreadyExists {
                resource: "capability slot",
                id,
            });
        }
        table.insert_new(entry);
        self.used.store(table.full, Ordering::Relaxed);
        self.stats.total_caps.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Remove a capability from the space
    pub fn remove(&self, cap: CapabilityToken) -> Option<ObjectRef> {
        let mut table = self.table.write();
        let i = table.find(cap.id())?;
        // Only the exact token (same generation) removes the entry; a
        // stale token must leave the live capability alone (N-04).
        match &table.slots[i] {
            Slot::Full(e) if e.capability == cap => {}
            _ => return None,
        }
        let entry = table.remove_at(i)?;
        self.used.store(table.full, Ordering::Relaxed);
        self.stats.total_caps.fetch_sub(1, Ordering::Relaxed);
        Some(entry.object)
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
        *self.table.write() = CapTable::default();
        self.used.store(0, Ordering::Relaxed);
        self.stats.total_caps.store(0, Ordering::Relaxed);
    }

    /// Copies of every entry, taken under the lock and returned after it is
    /// released, so callers may insert into any space (even this one).
    pub fn entries(&self) -> Vec<CapabilityEntry> {
        self.table.read().entries().cloned().collect()
    }

    /// Clone capabilities from another capability space
    ///
    /// Copies all capabilities from the source space to this space.
    /// Used during fork() to give child process same capabilities as parent.
    pub fn clone_from(&self, other: &Self) -> Result<(), KernelError> {
        // Copy first, then publish: the two locks are never held together.
        let copies: Vec<CapabilityEntry> = other
            .table
            .read()
            .entries()
            .map(CapabilityEntry::copy_for_new_space)
            .collect();
        let mut table = CapTable::default();
        for entry in copies {
            table.insert_new(entry);
        }
        let count = table.full;
        *self.table.write() = table;
        self.used.store(count, Ordering::Relaxed);
        self.stats.total_caps.store(count as u64, Ordering::Relaxed);

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
    pub fn revoke(&mut self, cap_id: CapabilityId) -> Result<(), KernelError> {
        // Create a token with the given ID and current generation
        let token = CapabilityToken::from_id_and_generation(cap_id, self.generation());

        if self.remove(token).is_some() {
            Ok(())
        } else {
            Err(KernelError::InvalidCapability {
                cap_id: cap_id.0,
                reason: crate::error::CapError::NotFound,
            })
        }
    }

    /// Create a capability (simplified for testing)
    pub fn create_capability(
        &mut self,
        rights: Rights,
        object: ObjectRef,
    ) -> Result<CapabilityId, KernelError> {
        let id = super::manager::cap_manager().allocate_id().map_err(|_| {
            KernelError::ResourceExhausted {
                resource: "capability IDs",
            }
        })?;

        if id > 0xFFFF_FFFF_FFFF {
            return Err(KernelError::ResourceExhausted {
                resource: "capability IDs",
            });
        }

        let token = CapabilityToken::new(id, self.generation(), 0, 0);
        self.insert(token, object, rights)?;

        Ok(CapabilityId(id))
    }

    /// Get statistics
    pub fn stats(&self) -> &CapSpaceStats {
        &self.stats
    }

    /// Iterate over all capabilities until `f` returns false. `f` runs
    /// under the space's read lock and must not modify this space; use
    /// [`entries`](Self::entries) for that.
    pub fn iter_capabilities<F>(&self, mut f: F) -> Result<(), KernelError>
    where
        F: FnMut(&CapabilityEntry) -> bool,
    {
        let table = self.table.read();
        for entry in table.entries() {
            if !f(entry) {
                break;
            }
        }
        Ok(())
    }

    /// Get capability entry by ID (any generation)
    pub fn get_entry(&self, cap_id: usize) -> Option<CapabilityEntry> {
        self.table.read().get(cap_id as u64).cloned()
    }

    /// Lookup and get full capability entry
    pub fn lookup_entry(&self, cap: CapabilityToken) -> Option<(ObjectRef, Rights)> {
        self.with_exact(cap, |e| (e.object.clone(), e.rights))
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
            last_used: crate::arch::entropy::read_timestamp(),
        });
    }
}
