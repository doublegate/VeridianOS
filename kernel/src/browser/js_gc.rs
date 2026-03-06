//! JavaScript Garbage Collector
//!
//! Conservative mark-sweep GC for the JS object arena. Uses tri-color
//! marking with root scanning from the VM stack, call frames, and globals.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::js_vm::{JsObject, JsValue, JsVm, ObjectId};

// ---------------------------------------------------------------------------
// GC cell wrapper
// ---------------------------------------------------------------------------

/// A GC-managed cell wrapping a JsObject
#[derive(Debug, Clone)]
pub struct GcCell {
    /// The object data
    pub data: JsObject,
    /// Whether this cell is marked (reachable)
    pub marked: bool,
    /// Approximate size in bytes
    pub size: usize,
}

impl GcCell {
    pub fn new(data: JsObject) -> Self {
        let size = estimate_object_size(&data);
        Self {
            data,
            marked: false,
            size,
        }
    }
}

// ---------------------------------------------------------------------------
// GC Arena
// ---------------------------------------------------------------------------

/// Arena for garbage-collected objects
pub struct GcArena {
    /// Object slots (None = free)
    objects: Vec<Option<GcCell>>,
    /// Free list (indices of None slots)
    free_list: Vec<usize>,
    /// Total bytes allocated
    bytes_allocated: usize,
    /// GC threshold in bytes (trigger collection when exceeded)
    threshold: usize,
    /// Number of live objects
    live_count: usize,
    /// Number of collections performed
    collection_count: usize,
}

impl Default for GcArena {
    fn default() -> Self {
        Self::new()
    }
}

impl GcArena {
    /// Default threshold: 8MB equivalent in bytes
    const DEFAULT_THRESHOLD: usize = 8 * 1024 * 1024;

    pub fn new() -> Self {
        Self {
            objects: Vec::with_capacity(256),
            free_list: Vec::new(),
            bytes_allocated: 0,
            threshold: Self::DEFAULT_THRESHOLD,
            live_count: 0,
            collection_count: 0,
        }
    }

    /// Allocate a new object, returning its ID
    pub fn allocate(&mut self, obj: JsObject) -> ObjectId {
        let cell = GcCell::new(obj);
        self.bytes_allocated += cell.size;
        self.live_count += 1;

        if let Some(idx) = self.free_list.pop() {
            self.objects[idx] = Some(cell);
            idx
        } else {
            let idx = self.objects.len();
            self.objects.push(Some(cell));
            idx
        }
    }

    /// Get an object by ID
    pub fn get(&self, id: ObjectId) -> Option<&JsObject> {
        self.objects
            .get(id)
            .and_then(|slot| slot.as_ref().map(|c| &c.data))
    }

    /// Get a mutable object by ID
    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut JsObject> {
        self.objects
            .get_mut(id)
            .and_then(|slot| slot.as_mut().map(|c| &mut c.data))
    }

    /// Whether the threshold has been exceeded
    pub fn should_collect(&self) -> bool {
        self.bytes_allocated >= self.threshold
    }

    /// Number of live objects
    pub fn live_count(&self) -> usize {
        self.live_count
    }

    /// Total bytes allocated
    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated
    }

    /// Number of collections performed
    pub fn collection_count(&self) -> usize {
        self.collection_count
    }

    /// Total capacity (slots)
    pub fn capacity(&self) -> usize {
        self.objects.len()
    }
}

// ---------------------------------------------------------------------------
// GC Heap (mark-sweep collector)
// ---------------------------------------------------------------------------

/// Garbage collector coordinating mark and sweep phases
pub struct GcHeap {
    pub arena: GcArena,
}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl GcHeap {
    pub fn new() -> Self {
        Self {
            arena: GcArena::new(),
        }
    }

    /// Allocate an object. May trigger GC if threshold exceeded.
    pub fn allocate(&mut self, obj: JsObject) -> ObjectId {
        self.arena.allocate(obj)
    }

    /// Run a full garbage collection cycle using roots from the VM
    pub fn collect(&mut self, vm: &JsVm) {
        self.mark_phase(vm);
        self.sweep_phase();
        self.arena.collection_count += 1;

        // Grow threshold after collection (adaptive)
        if self.arena.bytes_allocated > self.arena.threshold / 2 {
            self.arena.threshold = self.arena.threshold.saturating_mul(2);
        }
    }

    /// Mark phase: scan roots, mark all reachable objects
    fn mark_phase(&mut self, vm: &JsVm) {
        // Unmark all
        for cell in self.arena.objects.iter_mut().flatten() {
            cell.marked = false;
        }

        // Scan roots
        let mut worklist = Vec::new();

        // Root 1: operand stack
        for val in &vm.stack {
            if let Some(oid) = extract_object_id(val) {
                worklist.push(oid);
            }
        }

        // Root 2: call frame locals
        for frame in &vm.call_stack {
            for val in &frame.locals {
                if let Some(oid) = extract_object_id(val) {
                    worklist.push(oid);
                }
            }
        }

        // Root 3: global variables
        for val in vm.globals.values() {
            if let Some(oid) = extract_object_id(val) {
                worklist.push(oid);
            }
        }

        // Trace object graph (BFS)
        while let Some(oid) = worklist.pop() {
            if oid >= self.arena.objects.len() {
                continue;
            }
            if let Some(cell) = &self.arena.objects[oid] {
                if cell.marked {
                    continue;
                }
            } else {
                continue;
            }

            // Mark this object
            if let Some(cell) = &mut self.arena.objects[oid] {
                cell.marked = true;

                // Trace properties
                let props: Vec<JsValue> = cell.data.properties.values().cloned().collect();
                for val in &props {
                    if let Some(child_oid) = extract_object_id(val) {
                        worklist.push(child_oid);
                    }
                }

                // Trace prototype chain
                if let Some(proto) = cell.data.prototype {
                    worklist.push(proto);
                }
            }
        }
    }

    /// Sweep phase: free unmarked objects
    fn sweep_phase(&mut self) {
        for i in 0..self.arena.objects.len() {
            let should_free = if let Some(cell) = &self.arena.objects[i] {
                !cell.marked
            } else {
                false
            };

            if should_free {
                if let Some(cell) = self.arena.objects[i].take() {
                    self.arena.bytes_allocated =
                        self.arena.bytes_allocated.saturating_sub(cell.size);
                    self.arena.live_count = self.arena.live_count.saturating_sub(1);
                    self.arena.free_list.push(i);
                }
            }
        }
    }

    /// Check if a collection should be triggered
    pub fn should_collect(&self) -> bool {
        self.arena.should_collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract ObjectId from a JsValue if it refers to an object
fn extract_object_id(val: &JsValue) -> Option<ObjectId> {
    match val {
        JsValue::Object(oid) => Some(*oid),
        _ => None,
    }
}

/// Estimate the memory size of a JsObject in bytes
fn estimate_object_size(obj: &JsObject) -> usize {
    let base = 64; // struct overhead
    let props: usize = obj.properties.keys().map(|k| k.len() + 32).sum();
    base + props
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_vm() -> JsVm {
        JsVm::new()
    }

    #[test]
    fn test_gc_arena_allocate() {
        let mut arena = GcArena::new();
        let id = arena.allocate(JsObject::new());
        assert_eq!(id, 0);
        assert_eq!(arena.live_count(), 1);
        assert!(arena.bytes_allocated() > 0);
    }

    #[test]
    fn test_gc_arena_get() {
        let mut arena = GcArena::new();
        let mut obj = JsObject::new();
        obj.set("x", JsValue::Number(42));
        let id = arena.allocate(obj);

        let retrieved = arena.get(id).unwrap();
        assert!(retrieved.properties.contains_key("x"));
    }

    #[test]
    fn test_gc_arena_get_mut() {
        let mut arena = GcArena::new();
        let id = arena.allocate(JsObject::new());
        arena.get_mut(id).unwrap().set("y", JsValue::Boolean(true));
        assert!(arena.get(id).unwrap().properties.contains_key("y"));
    }

    #[test]
    fn test_gc_arena_free_list() {
        let mut arena = GcArena::new();
        let _id0 = arena.allocate(JsObject::new());
        let _id1 = arena.allocate(JsObject::new());
        assert_eq!(arena.live_count(), 2);
        assert_eq!(arena.capacity(), 2);
    }

    #[test]
    fn test_gc_heap_allocate() {
        let mut heap = GcHeap::new();
        let id = heap.allocate(JsObject::new());
        assert_eq!(id, 0);
        assert_eq!(heap.arena.live_count(), 1);
    }

    #[test]
    fn test_gc_collect_empty() {
        let mut heap = GcHeap::new();
        let vm = make_vm();
        heap.collect(&vm);
        assert_eq!(heap.arena.collection_count(), 1);
    }

    #[test]
    fn test_gc_collect_unreachable() {
        let mut heap = GcHeap::new();
        heap.allocate(JsObject::new());
        heap.allocate(JsObject::new());
        assert_eq!(heap.arena.live_count(), 2);

        // No roots referencing these objects
        let vm = make_vm();
        heap.collect(&vm);

        // Both should be swept
        assert_eq!(heap.arena.live_count(), 0);
        assert_eq!(heap.arena.free_list.len(), 2);
    }

    #[test]
    fn test_gc_collect_reachable_from_stack() {
        let mut heap = GcHeap::new();
        let oid = heap.allocate(JsObject::new());
        let _unreachable = heap.allocate(JsObject::new());

        let mut vm = make_vm();
        vm.stack.push(JsValue::Object(oid));

        heap.collect(&vm);

        // Only the reachable one survives
        assert_eq!(heap.arena.live_count(), 1);
        assert!(heap.arena.get(oid).is_some());
    }

    #[test]
    fn test_gc_collect_reachable_from_globals() {
        let mut heap = GcHeap::new();
        let oid = heap.allocate(JsObject::new());
        let _dead = heap.allocate(JsObject::new());

        let mut vm = make_vm();
        vm.globals.insert("alive".into(), JsValue::Object(oid));

        heap.collect(&vm);
        assert_eq!(heap.arena.live_count(), 1);
    }

    #[test]
    fn test_gc_collect_transitive() {
        let mut heap = GcHeap::new();
        let child = heap.allocate(JsObject::new());
        let mut parent = JsObject::new();
        parent.set("child", JsValue::Object(child));
        let parent_id = heap.allocate(parent);

        let mut vm = make_vm();
        vm.stack.push(JsValue::Object(parent_id));

        heap.collect(&vm);

        // Both parent and child survive (transitive reachability)
        assert_eq!(heap.arena.live_count(), 2);
    }

    #[test]
    fn test_gc_reuse_freed_slot() {
        let mut heap = GcHeap::new();
        let _id0 = heap.allocate(JsObject::new());
        let _id1 = heap.allocate(JsObject::new());

        let vm = make_vm();
        heap.collect(&vm);

        // Free list should have 2 entries
        assert_eq!(heap.arena.free_list.len(), 2);

        // Next allocation reuses a freed slot
        let id2 = heap.allocate(JsObject::new());
        assert!(id2 <= 1); // should reuse 0 or 1
    }

    #[test]
    fn test_gc_should_collect() {
        let heap = GcHeap::new();
        assert!(!heap.should_collect());
    }

    #[test]
    fn test_gc_arena_default() {
        let arena = GcArena::default();
        assert_eq!(arena.live_count(), 0);
        assert_eq!(arena.bytes_allocated(), 0);
        assert_eq!(arena.collection_count(), 0);
    }

    #[test]
    fn test_gc_heap_default() {
        let heap = GcHeap::default();
        assert_eq!(heap.arena.live_count(), 0);
    }

    #[test]
    fn test_gc_cell_new() {
        let obj = JsObject::new();
        let cell = GcCell::new(obj);
        assert!(!cell.marked);
        assert!(cell.size > 0);
    }

    #[test]
    fn test_estimate_object_size() {
        let mut obj = JsObject::new();
        let base_size = estimate_object_size(&obj);
        obj.set("key", JsValue::Number(0));
        let with_prop = estimate_object_size(&obj);
        assert!(with_prop > base_size);
    }

    #[test]
    fn test_extract_object_id() {
        assert_eq!(extract_object_id(&JsValue::Object(5)), Some(5));
        assert_eq!(extract_object_id(&JsValue::Number(42)), None);
        assert_eq!(extract_object_id(&JsValue::Null), None);
        assert_eq!(extract_object_id(&JsValue::Function(3)), None);
    }

    #[test]
    fn test_gc_collect_prototype_chain() {
        let mut heap = GcHeap::new();
        let proto = heap.allocate(JsObject::new());
        let mut child = JsObject::new();
        child.prototype = Some(proto);
        let child_id = heap.allocate(child);

        let mut vm = make_vm();
        vm.stack.push(JsValue::Object(child_id));

        heap.collect(&vm);
        // Both child and prototype survive
        assert_eq!(heap.arena.live_count(), 2);
    }

    #[test]
    fn test_gc_multiple_collections() {
        let mut heap = GcHeap::new();
        let vm = make_vm();

        heap.allocate(JsObject::new());
        heap.collect(&vm);
        assert_eq!(heap.arena.collection_count(), 1);

        heap.allocate(JsObject::new());
        heap.collect(&vm);
        assert_eq!(heap.arena.collection_count(), 2);
        assert_eq!(heap.arena.live_count(), 0);
    }

    #[test]
    fn test_gc_collect_with_call_frame_locals() {
        use super::super::{js_compiler::FunctionTemplate, js_vm::CallFrame};

        let mut heap = GcHeap::new();
        let oid = heap.allocate(JsObject::new());

        let mut vm = make_vm();
        vm.call_stack.push(CallFrame {
            function_id: 0,
            ip: 0,
            base_slot: 0,
            locals: vec![JsValue::Object(oid)],
            bytecode: Vec::new(),
            constants: Vec::new(),
        });

        heap.collect(&vm);
        assert_eq!(heap.arena.live_count(), 1);
    }

    #[test]
    fn test_gc_threshold_grows() {
        let mut heap = GcHeap::new();
        let initial_threshold = heap.arena.threshold;

        // Allocate enough to exceed half threshold
        for _ in 0..100_000 {
            heap.allocate(JsObject::new());
        }

        let mut vm = make_vm();
        // Keep all alive via globals
        for i in 0..heap.arena.objects.len() {
            if heap.arena.objects[i].is_some() {
                vm.globals
                    .insert(alloc::format!("o{}", i), JsValue::Object(i));
            }
        }

        heap.collect(&vm);

        if heap.arena.bytes_allocated > initial_threshold / 2 {
            assert!(heap.arena.threshold > initial_threshold);
        }
    }

    #[test]
    fn test_gc_arena_invalid_get() {
        let arena = GcArena::new();
        assert!(arena.get(999).is_none());
    }
}
