//! Hazard Pointer Registry
//!
//! Hazard pointers provide safe memory reclamation for lock-free data
//! structures. When a thread reads a shared pointer, it publishes the
//! address in a per-CPU hazard pointer slot. Before freeing memory,
//! the reclaimer checks all hazard pointers to ensure no thread is
//! actively referencing the object.
//!
//! This is simpler than RCU for single-object protection and is used
//! alongside RCU for different access patterns:
//! - RCU: read-heavy data structures with infrequent updates
//! - Hazard pointers: fine-grained per-object protection in lock-free
//!   containers (queues, stacks, hash maps)

use core::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum number of CPUs (matches smp::MAX_CPUS).
const MAX_CPUS: usize = 16;

/// Number of hazard pointer slots per CPU.
const SLOTS_PER_CPU: usize = 4;

/// Total number of hazard pointer slots.
const TOTAL_SLOTS: usize = MAX_CPUS * SLOTS_PER_CPU;

/// Sentinel value indicating an unused hazard pointer slot.
const HP_EMPTY: usize = 0;

// ---------------------------------------------------------------------------
// Global Hazard Pointer Array
// ---------------------------------------------------------------------------

/// Global hazard pointer slots. Each slot holds the address of a memory
/// object that a thread is actively referencing. Set to HP_EMPTY when
/// not in use.
#[allow(clippy::declare_interior_mutable_const)]
static HAZARD_POINTERS: [AtomicUsize; TOTAL_SLOTS] = {
    const INIT: AtomicUsize = AtomicUsize::new(HP_EMPTY);
    [INIT; TOTAL_SLOTS]
};

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

/// A guard that holds a hazard pointer slot. When dropped, the slot is
/// cleared (set to HP_EMPTY).
pub struct HazardGuard {
    slot_index: usize,
}

impl Drop for HazardGuard {
    fn drop(&mut self) {
        HAZARD_POINTERS[self.slot_index].store(HP_EMPTY, Ordering::Release);
    }
}

/// Protect a pointer by publishing it in a hazard pointer slot.
///
/// Returns a `HazardGuard` that clears the slot when dropped. The caller
/// must keep the guard alive as long as the pointer is being accessed.
///
/// `slot` must be 0..SLOTS_PER_CPU-1 (per-CPU local slot index).
pub fn protect(ptr: usize, slot: usize) -> HazardGuard {
    debug_assert!(slot < SLOTS_PER_CPU, "slot index out of range");
    let cpu = crate::sched::smp::current_cpu_id() as usize;
    let global_slot = cpu * SLOTS_PER_CPU + slot;

    HAZARD_POINTERS[global_slot].store(ptr, Ordering::Release);

    // Memory barrier ensures the hazard pointer is visible to reclaimers
    // before we proceed to access the protected object.
    core::sync::atomic::fence(Ordering::SeqCst);

    HazardGuard {
        slot_index: global_slot,
    }
}

/// Check whether a given address is currently protected by any hazard pointer.
///
/// Used by reclaimers before freeing memory to ensure no thread is actively
/// referencing the object.
pub fn is_protected(addr: usize) -> bool {
    for slot in &HAZARD_POINTERS {
        if slot.load(Ordering::Acquire) == addr {
            return true;
        }
    }
    false
}

/// Scan all hazard pointer slots and return a list of currently protected
/// addresses. Used for batch reclamation: collect a retire list, then filter
/// out any addresses that appear in the hazard set.
pub fn collect_protected() -> [usize; TOTAL_SLOTS] {
    let mut result = [HP_EMPTY; TOTAL_SLOTS];
    for (i, slot) in HAZARD_POINTERS.iter().enumerate() {
        result[i] = slot.load(Ordering::Acquire);
    }
    result
}

/// Clear all hazard pointer slots for a given CPU.
///
/// Called during CPU shutdown or thread exit to release all protections.
pub fn clear_cpu_slots(cpu_id: u8) {
    let base = (cpu_id as usize) * SLOTS_PER_CPU;
    for i in 0..SLOTS_PER_CPU {
        HAZARD_POINTERS[base + i].store(HP_EMPTY, Ordering::Release);
    }
}

/// Get the number of active (non-empty) hazard pointer slots.
pub fn active_count() -> usize {
    HAZARD_POINTERS
        .iter()
        .filter(|slot| slot.load(Ordering::Relaxed) != HP_EMPTY)
        .count()
}
