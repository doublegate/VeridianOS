//! Incremental Layout and Rendering
//!
//! Tracks dirty nodes and damage regions to avoid full re-layout and
//! re-paint on every DOM change. Uses per-node dirty flags and
//! rectangular damage regions.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, vec::Vec};

use super::events::NodeId;

// ---------------------------------------------------------------------------
// Dirty flags
// ---------------------------------------------------------------------------

/// Dirty state of a node, ordered by severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum DirtyFlag {
    /// No changes needed
    #[default]
    Clean = 0,
    /// Only painting is out of date (e.g., color changed)
    PaintDirty = 1,
    /// Layout needs recomputation (size/position changed)
    LayoutDirty = 2,
    /// Style needs resolution (class/attribute changed)
    StyleDirty = 3,
}

// ---------------------------------------------------------------------------
// Dirty tracker
// ---------------------------------------------------------------------------

/// Tracks per-node dirty state for incremental updates
pub struct DirtyTracker {
    /// Per-node dirty flags
    flags: BTreeMap<NodeId, DirtyFlag>,
    /// Parent map for propagating dirty state upward
    parents: BTreeMap<NodeId, NodeId>,
    /// Number of dirty nodes
    dirty_count: usize,
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            flags: BTreeMap::new(),
            parents: BTreeMap::new(),
            dirty_count: 0,
        }
    }

    /// Register a parent relationship
    pub fn set_parent(&mut self, child: NodeId, parent: NodeId) {
        self.parents.insert(child, parent);
    }

    /// Remove a node from tracking
    pub fn remove_node(&mut self, node: NodeId) {
        if let Some(flag) = self.flags.remove(&node) {
            if flag != DirtyFlag::Clean {
                self.dirty_count = self.dirty_count.saturating_sub(1);
            }
        }
        self.parents.remove(&node);
    }

    /// Mark a node as dirty with the given severity.
    /// Propagates LayoutDirty/StyleDirty upward to ancestors.
    pub fn mark_dirty(&mut self, node: NodeId, flag: DirtyFlag) {
        if flag == DirtyFlag::Clean {
            return;
        }

        // Set or upgrade the flag
        let current = self.flags.get(&node).copied().unwrap_or(DirtyFlag::Clean);
        if flag > current {
            if current == DirtyFlag::Clean {
                self.dirty_count += 1;
            }
            self.flags.insert(node, flag);
        }

        // Propagate upward: ancestors need at least LayoutDirty
        if flag >= DirtyFlag::LayoutDirty {
            let mut current_node = node;
            while let Some(&parent) = self.parents.get(&current_node) {
                let parent_flag = self.flags.get(&parent).copied().unwrap_or(DirtyFlag::Clean);
                if parent_flag >= DirtyFlag::LayoutDirty {
                    break; // Already dirty enough
                }
                if parent_flag == DirtyFlag::Clean {
                    self.dirty_count += 1;
                }
                self.flags.insert(parent, DirtyFlag::LayoutDirty);
                current_node = parent;
            }
        }
    }

    /// Mark a node as style-dirty (strongest)
    pub fn mark_style_dirty(&mut self, node: NodeId) {
        self.mark_dirty(node, DirtyFlag::StyleDirty);
    }

    /// Mark a node as layout-dirty
    pub fn mark_layout_dirty(&mut self, node: NodeId) {
        self.mark_dirty(node, DirtyFlag::LayoutDirty);
    }

    /// Mark a node as paint-dirty
    pub fn mark_paint_dirty(&mut self, node: NodeId) {
        self.mark_dirty(node, DirtyFlag::PaintDirty);
    }

    /// Get the dirty flag for a node
    pub fn get_flag(&self, node: NodeId) -> DirtyFlag {
        self.flags.get(&node).copied().unwrap_or(DirtyFlag::Clean)
    }

    /// Whether a node needs restyle
    pub fn needs_restyle(&self, node: NodeId) -> bool {
        self.get_flag(node) >= DirtyFlag::StyleDirty
    }

    /// Whether a node needs relayout
    pub fn needs_relayout(&self, node: NodeId) -> bool {
        self.get_flag(node) >= DirtyFlag::LayoutDirty
    }

    /// Whether a node needs repaint
    pub fn needs_repaint(&self, node: NodeId) -> bool {
        self.get_flag(node) >= DirtyFlag::PaintDirty
    }

    /// Clear a specific node's dirty flag
    pub fn clear(&mut self, node: NodeId) {
        if let Some(flag) = self.flags.get_mut(&node) {
            if *flag != DirtyFlag::Clean {
                *flag = DirtyFlag::Clean;
                self.dirty_count = self.dirty_count.saturating_sub(1);
            }
        }
    }

    /// Clear all dirty flags
    pub fn clear_all(&mut self) {
        for flag in self.flags.values_mut() {
            *flag = DirtyFlag::Clean;
        }
        self.dirty_count = 0;
    }

    /// Whether any nodes are dirty
    pub fn has_dirty_nodes(&self) -> bool {
        self.dirty_count > 0
    }

    /// Number of dirty nodes
    pub fn dirty_node_count(&self) -> usize {
        self.dirty_count
    }

    /// Get all nodes with at least the given dirty level, sorted by node ID
    pub fn dirty_nodes(&self, min_flag: DirtyFlag) -> Vec<NodeId> {
        self.flags
            .iter()
            .filter(|(_, &flag)| flag >= min_flag)
            .map(|(&node, _)| node)
            .collect()
    }

    /// Find the highest dirty ancestor of a node (the subtree root to
    /// re-process)
    pub fn highest_dirty_ancestor(&self, node: NodeId, min_flag: DirtyFlag) -> NodeId {
        let mut highest = node;
        let mut current = node;
        while let Some(&parent) = self.parents.get(&current) {
            if self.get_flag(parent) >= min_flag {
                highest = parent;
            }
            current = parent;
        }
        highest
    }
}

// ---------------------------------------------------------------------------
// Damage region
// ---------------------------------------------------------------------------

/// A rectangular damage region in pixel coordinates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DamageRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DamageRegion {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        }
    }

    /// Whether this region is empty (zero area)
    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Right edge
    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    /// Bottom edge
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// Area in pixels
    pub fn area(&self) -> i64 {
        self.width as i64 * self.height as i64
    }

    /// Union with another region (bounding box)
    pub fn union(&self, other: &DamageRegion) -> DamageRegion {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        DamageRegion::new(x, y, r - x, b - y)
    }

    /// Intersection with another region
    pub fn intersect(&self, other: &DamageRegion) -> DamageRegion {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        if r <= x || b <= y {
            DamageRegion::default()
        } else {
            DamageRegion::new(x, y, r - x, b - y)
        }
    }

    /// Whether this region fully contains another
    pub fn contains(&self, other: &DamageRegion) -> bool {
        self.x <= other.x
            && self.y <= other.y
            && self.right() >= other.right()
            && self.bottom() >= other.bottom()
    }

    /// Whether this region contains a point
    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

// ---------------------------------------------------------------------------
// Damage list (accumulator of damage regions)
// ---------------------------------------------------------------------------

/// Accumulates damage regions and merges them
pub struct DamageList {
    /// Individual damage regions
    regions: Vec<DamageRegion>,
    /// Bounding box of all damage
    bounds: DamageRegion,
    /// Maximum number of regions before merging all into bounding box
    max_regions: usize,
}

impl Default for DamageList {
    fn default() -> Self {
        Self::new()
    }
}

impl DamageList {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            bounds: DamageRegion::default(),
            max_regions: 32,
        }
    }

    /// Add a damage region
    pub fn add(&mut self, region: DamageRegion) {
        if region.is_empty() {
            return;
        }
        self.bounds = self.bounds.union(&region);
        self.regions.push(region);

        // Merge if too many regions
        if self.regions.len() > self.max_regions {
            self.merge_all();
        }
    }

    /// Merge all regions into a single bounding box
    pub fn merge_all(&mut self) {
        if !self.regions.is_empty() {
            self.regions.clear();
            self.regions.push(self.bounds);
        }
    }

    /// Get the bounding box of all damage
    pub fn bounding_box(&self) -> DamageRegion {
        self.bounds
    }

    /// Get individual damage regions
    pub fn regions(&self) -> &[DamageRegion] {
        &self.regions
    }

    /// Whether there is any damage
    pub fn has_damage(&self) -> bool {
        !self.regions.is_empty()
    }

    /// Clear all damage
    pub fn clear(&mut self) {
        self.regions.clear();
        self.bounds = DamageRegion::default();
    }

    /// Check if a rectangle overlaps with any damage region
    pub fn overlaps(&self, region: &DamageRegion) -> bool {
        // Quick check against bounding box first
        if self.bounds.intersect(region).is_empty() {
            return false;
        }
        self.regions.iter().any(|r| !r.intersect(region).is_empty())
    }
}

// ---------------------------------------------------------------------------
// Incremental layout engine
// ---------------------------------------------------------------------------

/// Coordinates incremental restyle, relayout, and repaint
pub struct IncrementalLayout {
    /// Dirty tracker
    pub tracker: DirtyTracker,
    /// Accumulated damage regions
    pub damage: DamageList,
    /// Node-to-rect mapping (filled during layout)
    node_rects: BTreeMap<NodeId, DamageRegion>,
}

impl Default for IncrementalLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl IncrementalLayout {
    pub fn new() -> Self {
        Self {
            tracker: DirtyTracker::new(),
            damage: DamageList::new(),
            node_rects: BTreeMap::new(),
        }
    }

    /// Register a node's layout rectangle (called after layout pass)
    pub fn set_node_rect(&mut self, node: NodeId, x: i32, y: i32, w: i32, h: i32) {
        self.node_rects.insert(node, DamageRegion::new(x, y, w, h));
    }

    /// Get a node's layout rectangle
    pub fn node_rect(&self, node: NodeId) -> Option<DamageRegion> {
        self.node_rects.get(&node).copied()
    }

    /// Remove a node's rect
    pub fn remove_node_rect(&mut self, node: NodeId) {
        self.node_rects.remove(&node);
    }

    /// Get nodes that need restyle, returning the highest dirty ancestors
    /// to minimize redundant work
    pub fn restyle_roots(&self) -> Vec<NodeId> {
        let dirty = self.tracker.dirty_nodes(DirtyFlag::StyleDirty);
        let mut roots = Vec::new();
        for &node in &dirty {
            let root = self
                .tracker
                .highest_dirty_ancestor(node, DirtyFlag::StyleDirty);
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    }

    /// Get subtree roots that need relayout
    pub fn relayout_roots(&self) -> Vec<NodeId> {
        let dirty = self.tracker.dirty_nodes(DirtyFlag::LayoutDirty);
        let mut roots = Vec::new();
        for &node in &dirty {
            let root = self
                .tracker
                .highest_dirty_ancestor(node, DirtyFlag::LayoutDirty);
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
    }

    /// Mark damage for all nodes that need repaint
    pub fn compute_damage(&mut self) {
        let paint_dirty = self.tracker.dirty_nodes(DirtyFlag::PaintDirty);
        for &node in &paint_dirty {
            if let Some(rect) = self.node_rects.get(&node) {
                self.damage.add(*rect);
            }
        }
    }

    /// After processing: clear dirty flags for processed nodes and damage
    pub fn finish_update(&mut self) {
        self.tracker.clear_all();
        self.damage.clear();
    }

    /// Full update cycle: compute damage, return regions, clear state.
    /// Returns the bounding box of all damage (if any).
    pub fn flush(&mut self) -> Option<DamageRegion> {
        self.compute_damage();
        if self.damage.has_damage() {
            let bbox = self.damage.bounding_box();
            self.finish_update();
            Some(bbox)
        } else {
            self.tracker.clear_all();
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_dirty_flag_ordering() {
        assert!(DirtyFlag::Clean < DirtyFlag::PaintDirty);
        assert!(DirtyFlag::PaintDirty < DirtyFlag::LayoutDirty);
        assert!(DirtyFlag::LayoutDirty < DirtyFlag::StyleDirty);
    }

    #[test]
    fn test_dirty_tracker_mark_and_get() {
        let mut t = DirtyTracker::new();
        assert_eq!(t.get_flag(1), DirtyFlag::Clean);
        t.mark_paint_dirty(1);
        assert_eq!(t.get_flag(1), DirtyFlag::PaintDirty);
        assert!(t.needs_repaint(1));
        assert!(!t.needs_relayout(1));
    }

    #[test]
    fn test_dirty_upgrade() {
        let mut t = DirtyTracker::new();
        t.mark_paint_dirty(1);
        t.mark_layout_dirty(1);
        assert_eq!(t.get_flag(1), DirtyFlag::LayoutDirty);
        // Cannot downgrade
        t.mark_paint_dirty(1);
        assert_eq!(t.get_flag(1), DirtyFlag::LayoutDirty);
    }

    #[test]
    fn test_dirty_propagation() {
        let mut t = DirtyTracker::new();
        t.set_parent(1, 0);
        t.set_parent(2, 1);
        t.mark_layout_dirty(2);
        // Should propagate to ancestors
        assert!(t.needs_relayout(1));
        assert!(t.needs_relayout(0));
    }

    #[test]
    fn test_dirty_no_propagation_for_paint() {
        let mut t = DirtyTracker::new();
        t.set_parent(1, 0);
        t.mark_paint_dirty(1);
        assert!(!t.needs_repaint(0));
    }

    #[test]
    fn test_dirty_clear() {
        let mut t = DirtyTracker::new();
        t.mark_style_dirty(1);
        assert_eq!(t.dirty_node_count(), 1);
        t.clear(1);
        assert_eq!(t.get_flag(1), DirtyFlag::Clean);
        assert_eq!(t.dirty_node_count(), 0);
    }

    #[test]
    fn test_dirty_clear_all() {
        let mut t = DirtyTracker::new();
        t.mark_paint_dirty(1);
        t.mark_layout_dirty(2);
        t.clear_all();
        assert!(!t.has_dirty_nodes());
    }

    #[test]
    fn test_dirty_nodes_filter() {
        let mut t = DirtyTracker::new();
        t.mark_paint_dirty(1);
        t.mark_layout_dirty(2);
        t.mark_style_dirty(3);
        let layout_dirty = t.dirty_nodes(DirtyFlag::LayoutDirty);
        assert_eq!(layout_dirty.len(), 2); // 2 and 3
        assert!(layout_dirty.contains(&2));
        assert!(layout_dirty.contains(&3));
    }

    #[test]
    fn test_highest_dirty_ancestor() {
        let mut t = DirtyTracker::new();
        t.set_parent(1, 0);
        t.set_parent(2, 1);
        t.mark_layout_dirty(2);
        let root = t.highest_dirty_ancestor(2, DirtyFlag::LayoutDirty);
        assert_eq!(root, 0);
    }

    #[test]
    fn test_damage_region_basic() {
        let r = DamageRegion::new(10, 20, 100, 50);
        assert_eq!(r.right(), 110);
        assert_eq!(r.bottom(), 70);
        assert!(!r.is_empty());
        assert_eq!(r.area(), 5000);
    }

    #[test]
    fn test_damage_region_empty() {
        let r = DamageRegion::new(0, 0, 0, 0);
        assert!(r.is_empty());
    }

    #[test]
    fn test_damage_union() {
        let a = DamageRegion::new(0, 0, 50, 50);
        let b = DamageRegion::new(30, 30, 50, 50);
        let u = a.union(&b);
        assert_eq!(u.x, 0);
        assert_eq!(u.y, 0);
        assert_eq!(u.width, 80);
        assert_eq!(u.height, 80);
    }

    #[test]
    fn test_damage_intersect() {
        let a = DamageRegion::new(0, 0, 50, 50);
        let b = DamageRegion::new(30, 30, 50, 50);
        let i = a.intersect(&b);
        assert_eq!(i.x, 30);
        assert_eq!(i.y, 30);
        assert_eq!(i.width, 20);
        assert_eq!(i.height, 20);
    }

    #[test]
    fn test_damage_intersect_disjoint() {
        let a = DamageRegion::new(0, 0, 10, 10);
        let b = DamageRegion::new(20, 20, 10, 10);
        let i = a.intersect(&b);
        assert!(i.is_empty());
    }

    #[test]
    fn test_damage_contains() {
        let a = DamageRegion::new(0, 0, 100, 100);
        let b = DamageRegion::new(10, 10, 50, 50);
        assert!(a.contains(&b));
        assert!(!b.contains(&a));
    }

    #[test]
    fn test_damage_contains_point() {
        let r = DamageRegion::new(10, 10, 20, 20);
        assert!(r.contains_point(15, 15));
        assert!(!r.contains_point(5, 5));
        assert!(!r.contains_point(30, 30));
    }

    #[test]
    fn test_damage_list() {
        let mut dl = DamageList::new();
        assert!(!dl.has_damage());
        dl.add(DamageRegion::new(0, 0, 10, 10));
        dl.add(DamageRegion::new(50, 50, 20, 20));
        assert!(dl.has_damage());
        assert_eq!(dl.regions().len(), 2);
        let bbox = dl.bounding_box();
        assert_eq!(bbox.x, 0);
        assert_eq!(bbox.width, 70);
    }

    #[test]
    fn test_damage_list_clear() {
        let mut dl = DamageList::new();
        dl.add(DamageRegion::new(0, 0, 10, 10));
        dl.clear();
        assert!(!dl.has_damage());
    }

    #[test]
    fn test_damage_list_overlaps() {
        let mut dl = DamageList::new();
        dl.add(DamageRegion::new(10, 10, 20, 20));
        let query = DamageRegion::new(15, 15, 5, 5);
        assert!(dl.overlaps(&query));
        let query2 = DamageRegion::new(100, 100, 5, 5);
        assert!(!dl.overlaps(&query2));
    }

    #[test]
    fn test_incremental_layout_restyle_roots() {
        let mut il = IncrementalLayout::new();
        il.tracker.set_parent(1, 0);
        il.tracker.set_parent(2, 1);
        il.tracker.mark_style_dirty(2);
        let roots = il.restyle_roots();
        // Node 2 style-dirty, propagated to 1 and 0 as layout-dirty
        // Highest style-dirty ancestor of 2 is 2 itself
        assert_eq!(roots, vec![2]);
    }

    #[test]
    fn test_incremental_layout_flush() {
        let mut il = IncrementalLayout::new();
        il.tracker.mark_paint_dirty(1);
        il.set_node_rect(1, 10, 10, 50, 50);
        let bbox = il.flush();
        assert!(bbox.is_some());
        let bbox = bbox.unwrap();
        assert_eq!(bbox.x, 10);
        assert_eq!(bbox.width, 50);
        assert!(!il.tracker.has_dirty_nodes());
    }

    #[test]
    fn test_incremental_layout_no_damage() {
        let mut il = IncrementalLayout::new();
        let bbox = il.flush();
        assert!(bbox.is_none());
    }

    #[test]
    fn test_remove_node() {
        let mut t = DirtyTracker::new();
        t.mark_layout_dirty(5);
        assert_eq!(t.dirty_node_count(), 1);
        t.remove_node(5);
        assert_eq!(t.dirty_node_count(), 0);
    }
}
