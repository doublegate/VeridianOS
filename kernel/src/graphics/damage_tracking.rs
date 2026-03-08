//! Damage Tracking for Compositor Re-composition
//!
//! Per-surface dirty-rect list with merge algorithm. The compositor
//! re-composites only damaged regions instead of the full framebuffer,
//! dramatically reducing fill-rate pressure on software renderers
//! (llvmpipe / CPU blitter).
//!
//! All arithmetic uses integer math (no FPU required).

#![allow(dead_code)]

use alloc::vec::Vec;

use spin::Mutex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of damage rects per surface before we fall back to
/// full-surface damage. Keeps merge cost bounded.
const MAX_DAMAGE_RECTS: usize = 32;

/// Maximum number of tracked surfaces.
const MAX_SURFACES: usize = 256;

// ---------------------------------------------------------------------------
// DamageRect
// ---------------------------------------------------------------------------

/// Axis-aligned damage rectangle (integer coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DamageRect {
    /// Create a new damage rect.
    pub(crate) fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge (exclusive).
    #[inline]
    pub(crate) fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// Bottom edge (exclusive).
    #[inline]
    pub(crate) fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// Area in pixels.
    #[inline]
    pub(crate) fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// True when this rect has zero area.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Check if two rects overlap.
    #[inline]
    fn overlaps(&self, other: &DamageRect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Check if two rects touch (overlap or share an edge).
    #[inline]
    fn touches(&self, other: &DamageRect) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    /// Compute the bounding-box union of two rects.
    fn union(&self, other: &DamageRect) -> DamageRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        DamageRect {
            x,
            y,
            width: (r - x) as u32,
            height: (b - y) as u32,
        }
    }

    /// Compute the intersection of two rects (may be empty).
    fn intersect(&self, other: &DamageRect) -> DamageRect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        if r <= x || b <= y {
            DamageRect::new(0, 0, 0, 0)
        } else {
            DamageRect::new(x, y, (r - x) as u32, (b - y) as u32)
        }
    }

    /// Clamp this rect to fit within `bounds`.
    pub(crate) fn clamp_to(&self, bounds: &DamageRect) -> DamageRect {
        self.intersect(bounds)
    }
}

// ---------------------------------------------------------------------------
// Per-surface damage list
// ---------------------------------------------------------------------------

/// Damage state for one surface.
#[derive(Debug, Clone)]
struct SurfaceDamage {
    surface_id: u64,
    rects: Vec<DamageRect>,
    /// Surface dimensions (for full-surface fallback).
    surface_width: u32,
    surface_height: u32,
    /// Set when the rect count exceeds the threshold; means "entire surface".
    full_damage: bool,
}

impl SurfaceDamage {
    fn new(surface_id: u64, width: u32, height: u32) -> Self {
        Self {
            surface_id,
            rects: Vec::new(),
            surface_width: width,
            surface_height: height,
            full_damage: false,
        }
    }

    fn clear(&mut self) {
        self.rects.clear();
        self.full_damage = false;
    }

    fn has_damage(&self) -> bool {
        self.full_damage || !self.rects.is_empty()
    }

    /// Mark the entire surface damaged.
    fn mark_full(&mut self) {
        self.rects.clear();
        self.rects.push(DamageRect::new(
            0,
            0,
            self.surface_width,
            self.surface_height,
        ));
        self.full_damage = true;
    }

    fn add(&mut self, rect: DamageRect) {
        if self.full_damage {
            return; // already full
        }
        if rect.is_empty() {
            return;
        }
        self.rects.push(rect);
        if self.rects.len() > MAX_DAMAGE_RECTS {
            self.mark_full();
        }
    }
}

// ---------------------------------------------------------------------------
// DamageTracker
// ---------------------------------------------------------------------------

/// Global damage tracker managing per-surface dirty regions.
pub(crate) struct DamageTracker {
    surfaces: Vec<SurfaceDamage>,
    /// Accumulated compositor-level damage (union of all surface damage
    /// projected into screen coordinates).
    screen_damage: Vec<DamageRect>,
}

impl DamageTracker {
    /// Create a new empty tracker.
    pub(crate) fn new() -> Self {
        Self {
            surfaces: Vec::new(),
            screen_damage: Vec::new(),
        }
    }

    // -- Surface registration ------------------------------------------------

    /// Register a surface for tracking. Must be called before `add_damage`.
    pub(crate) fn register_surface(&mut self, surface_id: u64, width: u32, height: u32) {
        if self.find(surface_id).is_none() && self.surfaces.len() < MAX_SURFACES {
            self.surfaces
                .push(SurfaceDamage::new(surface_id, width, height));
        }
    }

    /// Unregister a surface.
    pub(crate) fn unregister_surface(&mut self, surface_id: u64) {
        self.surfaces.retain(|s| s.surface_id != surface_id);
    }

    /// Update surface dimensions (e.g. on resize).
    pub(crate) fn resize_surface(&mut self, surface_id: u64, width: u32, height: u32) {
        if let Some(s) = self.find_mut(surface_id) {
            s.surface_width = width;
            s.surface_height = height;
            // Resize implies full redraw
            s.mark_full();
        }
    }

    // -- Damage submission ---------------------------------------------------

    /// Add a damage rect for a specific surface.
    pub(crate) fn add_damage(&mut self, surface_id: u64, rect: DamageRect) {
        if let Some(s) = self.find_mut(surface_id) {
            s.add(rect);
        }
    }

    /// Mark an entire surface as damaged.
    pub(crate) fn add_full_damage(&mut self, surface_id: u64) {
        if let Some(s) = self.find_mut(surface_id) {
            s.mark_full();
        }
    }

    // -- Queries -------------------------------------------------------------

    /// Get the current damage rects for a surface.
    pub(crate) fn get_damage(&self, surface_id: u64) -> &[DamageRect] {
        match self.find(surface_id) {
            Some(s) => &s.rects,
            None => &[],
        }
    }

    /// Check whether a surface has any pending damage.
    pub(crate) fn has_damage(&self, surface_id: u64) -> bool {
        self.find(surface_id).is_some_and(|s| s.has_damage())
    }

    /// Check whether any surface has pending damage.
    pub(crate) fn has_any_damage(&self) -> bool {
        self.surfaces.iter().any(|s| s.has_damage())
    }

    // -- Merge algorithm -----------------------------------------------------

    /// Merge a list of rects by combining overlapping/touching rects.
    ///
    /// Uses a greedy merge: iterate pairs, merge touching rects, repeat
    /// until stable. Worst-case O(n^2) per pass but n <= MAX_DAMAGE_RECTS.
    pub(crate) fn merge_damage(rects: &[DamageRect]) -> Vec<DamageRect> {
        if rects.is_empty() {
            return Vec::new();
        }
        let mut merged: Vec<DamageRect> = rects.to_vec();

        loop {
            let mut changed = false;
            let mut i = 0;
            while i < merged.len() {
                let mut j = i + 1;
                while j < merged.len() {
                    if merged[i].touches(&merged[j]) {
                        // Check that merging doesn't waste too much area.
                        // If the union area is less than 2x the sum of
                        // individual areas, merge them.
                        let union = merged[i].union(&merged[j]);
                        let sum_area = merged[i].area().saturating_add(merged[j].area());
                        let union_area = union.area();
                        if union_area <= sum_area.saturating_mul(2) {
                            merged[i] = union;
                            merged.swap_remove(j);
                            changed = true;
                            continue; // re-check j (new element swapped in)
                        }
                    }
                    j += 1;
                }
                i += 1;
            }
            if !changed {
                break;
            }
        }

        merged
    }

    /// Get merged damage for a surface.
    pub(crate) fn get_merged_damage(&self, surface_id: u64) -> Vec<DamageRect> {
        let rects = self.get_damage(surface_id);
        Self::merge_damage(rects)
    }

    /// Compute screen-space damage from all surfaces.
    ///
    /// Each surface's damage rects are offset by `(sx, sy)` (the surface
    /// position on screen), then merged into a single list.
    pub(crate) fn compute_screen_damage(
        &mut self,
        surface_positions: &[(u64, i32, i32)], // (surface_id, x, y)
    ) -> Vec<DamageRect> {
        let mut all_rects: Vec<DamageRect> = Vec::new();

        for &(sid, sx, sy) in surface_positions {
            if let Some(s) = self.find(sid) {
                for r in &s.rects {
                    all_rects.push(DamageRect::new(
                        r.x.saturating_add(sx),
                        r.y.saturating_add(sy),
                        r.width,
                        r.height,
                    ));
                }
            }
        }

        Self::merge_damage(&all_rects)
    }

    // -- Clear ---------------------------------------------------------------

    /// Clear damage for a specific surface (after compositing).
    pub(crate) fn clear_damage(&mut self, surface_id: u64) {
        if let Some(s) = self.find_mut(surface_id) {
            s.clear();
        }
    }

    /// Clear all surface damage (after a full composite pass).
    pub(crate) fn clear_all(&mut self) {
        for s in &mut self.surfaces {
            s.clear();
        }
        self.screen_damage.clear();
    }

    // -- Internals -----------------------------------------------------------

    fn find(&self, surface_id: u64) -> Option<&SurfaceDamage> {
        self.surfaces.iter().find(|s| s.surface_id == surface_id)
    }

    fn find_mut(&mut self, surface_id: u64) -> Option<&mut SurfaceDamage> {
        self.surfaces
            .iter_mut()
            .find(|s| s.surface_id == surface_id)
    }
}

// ---------------------------------------------------------------------------
// Global instance
// ---------------------------------------------------------------------------

static DAMAGE_TRACKER: Mutex<Option<DamageTracker>> = Mutex::new(None);

/// Initialize the global damage tracker.
pub(crate) fn init() {
    let mut guard = DAMAGE_TRACKER.lock();
    if guard.is_none() {
        *guard = Some(DamageTracker::new());
    }
}

/// Access the global damage tracker.
pub(crate) fn with_tracker<R, F: FnOnce(&mut DamageTracker) -> R>(f: F) -> Option<R> {
    DAMAGE_TRACKER.lock().as_mut().map(f)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_damage_rect_new() {
        let r = DamageRect::new(10, 20, 100, 200);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.width, 100);
        assert_eq!(r.height, 200);
    }

    #[test]
    fn test_damage_rect_edges() {
        let r = DamageRect::new(10, 20, 100, 50);
        assert_eq!(r.right(), 110);
        assert_eq!(r.bottom(), 70);
    }

    #[test]
    fn test_damage_rect_area() {
        let r = DamageRect::new(0, 0, 100, 200);
        assert_eq!(r.area(), 20_000);
    }

    #[test]
    fn test_damage_rect_empty() {
        assert!(DamageRect::new(0, 0, 0, 100).is_empty());
        assert!(DamageRect::new(0, 0, 100, 0).is_empty());
        assert!(!DamageRect::new(0, 0, 1, 1).is_empty());
    }

    #[test]
    fn test_damage_rect_overlap() {
        let a = DamageRect::new(0, 0, 100, 100);
        let b = DamageRect::new(50, 50, 100, 100);
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));

        let c = DamageRect::new(200, 200, 10, 10);
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn test_damage_rect_touches() {
        // Adjacent (sharing edge)
        let a = DamageRect::new(0, 0, 100, 100);
        let b = DamageRect::new(100, 0, 100, 100);
        assert!(a.touches(&b));
        assert!(!a.overlaps(&b)); // edge-sharing is not overlap

        // Gap between them
        let c = DamageRect::new(101, 0, 100, 100);
        assert!(!a.touches(&c));
    }

    #[test]
    fn test_damage_rect_union() {
        let a = DamageRect::new(10, 20, 100, 50);
        let b = DamageRect::new(50, 30, 200, 80);
        let u = a.union(&b);
        assert_eq!(u.x, 10);
        assert_eq!(u.y, 20);
        assert_eq!(u.right(), 250);
        assert_eq!(u.bottom(), 110);
        assert_eq!(u.width, 240);
        assert_eq!(u.height, 90);
    }

    #[test]
    fn test_damage_rect_intersect() {
        let a = DamageRect::new(0, 0, 100, 100);
        let b = DamageRect::new(50, 50, 100, 100);
        let i = a.intersect(&b);
        assert_eq!(i.x, 50);
        assert_eq!(i.y, 50);
        assert_eq!(i.width, 50);
        assert_eq!(i.height, 50);

        // Non-overlapping -> empty
        let c = DamageRect::new(200, 200, 10, 10);
        let empty = a.intersect(&c);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_damage_rect_clamp() {
        let r = DamageRect::new(-10, -10, 50, 50);
        let bounds = DamageRect::new(0, 0, 1920, 1080);
        let clamped = r.clamp_to(&bounds);
        assert_eq!(clamped.x, 0);
        assert_eq!(clamped.y, 0);
        assert_eq!(clamped.width, 40);
        assert_eq!(clamped.height, 40);
    }

    #[test]
    fn test_tracker_register_and_damage() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 800, 600);
        assert!(!tracker.has_damage(1));

        tracker.add_damage(1, DamageRect::new(0, 0, 100, 100));
        assert!(tracker.has_damage(1));
        assert_eq!(tracker.get_damage(1).len(), 1);
    }

    #[test]
    fn test_tracker_clear_damage() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 800, 600);
        tracker.add_damage(1, DamageRect::new(0, 0, 50, 50));
        tracker.clear_damage(1);
        assert!(!tracker.has_damage(1));
        assert_eq!(tracker.get_damage(1).len(), 0);
    }

    #[test]
    fn test_tracker_full_damage_fallback() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 800, 600);
        // Exceed threshold
        for i in 0..MAX_DAMAGE_RECTS + 5 {
            tracker.add_damage(1, DamageRect::new(i as i32 * 10, 0, 5, 5));
        }
        // Should have collapsed to one full-surface rect
        assert!(tracker.has_damage(1));
        let rects = tracker.get_damage(1);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 800);
        assert_eq!(rects[0].height, 600);
    }

    #[test]
    fn test_tracker_unregister() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(42, 640, 480);
        tracker.add_damage(42, DamageRect::new(0, 0, 10, 10));
        tracker.unregister_surface(42);
        assert!(!tracker.has_damage(42));
        assert_eq!(tracker.get_damage(42).len(), 0);
    }

    #[test]
    fn test_tracker_resize_marks_full() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 800, 600);
        tracker.resize_surface(1, 1920, 1080);
        assert!(tracker.has_damage(1));
        let rects = tracker.get_damage(1);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].width, 1920);
        assert_eq!(rects[0].height, 1080);
    }

    #[test]
    fn test_merge_overlapping() {
        let rects = vec![
            DamageRect::new(0, 0, 100, 100),
            DamageRect::new(50, 50, 100, 100),
        ];
        let merged = DamageTracker::merge_damage(&rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].x, 0);
        assert_eq!(merged[0].y, 0);
        assert_eq!(merged[0].width, 150);
        assert_eq!(merged[0].height, 150);
    }

    #[test]
    fn test_merge_non_overlapping() {
        let rects = vec![
            DamageRect::new(0, 0, 10, 10),
            DamageRect::new(500, 500, 10, 10),
        ];
        let merged = DamageTracker::merge_damage(&rects);
        // Far apart, should not merge (union area would be huge)
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merge_empty() {
        let merged = DamageTracker::merge_damage(&[]);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_adjacent() {
        let rects = vec![
            DamageRect::new(0, 0, 100, 100),
            DamageRect::new(100, 0, 100, 100),
        ];
        let merged = DamageTracker::merge_damage(&rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].width, 200);
    }

    #[test]
    fn test_has_any_damage() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 100, 100);
        tracker.register_surface(2, 100, 100);
        assert!(!tracker.has_any_damage());

        tracker.add_damage(2, DamageRect::new(0, 0, 10, 10));
        assert!(tracker.has_any_damage());
    }

    #[test]
    fn test_clear_all() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 100, 100);
        tracker.register_surface(2, 100, 100);
        tracker.add_damage(1, DamageRect::new(0, 0, 10, 10));
        tracker.add_damage(2, DamageRect::new(0, 0, 10, 10));
        tracker.clear_all();
        assert!(!tracker.has_any_damage());
    }

    #[test]
    fn test_empty_rect_not_added() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 800, 600);
        tracker.add_damage(1, DamageRect::new(0, 0, 0, 0));
        assert!(!tracker.has_damage(1));
    }

    #[test]
    fn test_compute_screen_damage() {
        let mut tracker = DamageTracker::new();
        tracker.register_surface(1, 200, 200);
        tracker.register_surface(2, 200, 200);

        tracker.add_damage(1, DamageRect::new(0, 0, 50, 50));
        tracker.add_damage(2, DamageRect::new(10, 10, 30, 30));

        let positions = vec![(1, 100, 100), (2, 300, 300)];
        let screen = tracker.compute_screen_damage(&positions);
        // Two rects far apart, should not merge
        assert_eq!(screen.len(), 2);
    }

    #[test]
    fn test_damage_to_unregistered_surface() {
        let mut tracker = DamageTracker::new();
        // Should silently ignore
        tracker.add_damage(999, DamageRect::new(0, 0, 10, 10));
        assert!(!tracker.has_damage(999));
    }
}
