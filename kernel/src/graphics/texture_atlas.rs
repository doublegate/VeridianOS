//! Texture Atlas with Shelf-based Bin Packing
//!
//! Implements a shelf (skyline) allocator for packing rectangular texture
//! regions into a single atlas texture. Used by the GL compositor to batch
//! surface textures into a single GPU-side allocation.
//!
//! All arithmetic is integer-only (no FPU required).

#![allow(dead_code)]

use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Atlas region
// ---------------------------------------------------------------------------

/// A rectangular region within the texture atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRegion {
    /// X offset in the atlas (pixels).
    pub x: u32,
    /// Y offset in the atlas (pixels).
    pub y: u32,
    /// Width of the region (pixels).
    pub width: u32,
    /// Height of the region (pixels).
    pub height: u32,
}

impl AtlasRegion {
    /// Total pixel area of this region.
    pub fn area(&self) -> u32 {
        self.width * self.height
    }

    /// Whether a point lies inside this region.
    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

// ---------------------------------------------------------------------------
// Free span within a shelf
// ---------------------------------------------------------------------------

/// A contiguous horizontal free span on a shelf.
#[derive(Debug, Clone, Copy)]
struct FreeSpan {
    x: u32,
    width: u32,
}

// ---------------------------------------------------------------------------
// Shelf
// ---------------------------------------------------------------------------

/// A horizontal shelf in the atlas.
#[derive(Debug, Clone)]
struct Shelf {
    /// Y position of this shelf's top edge.
    y: u32,
    /// Height of this shelf (set by the first allocation).
    height: u32,
    /// Free horizontal spans remaining on this shelf.
    free_spans: Vec<FreeSpan>,
}

impl Shelf {
    /// Create a new shelf at `y` with `height` and full atlas width free.
    fn new(y: u32, height: u32, atlas_width: u32) -> Self {
        Self {
            y,
            height,
            free_spans: alloc::vec![FreeSpan {
                x: 0,
                width: atlas_width,
            }],
        }
    }

    /// Try to allocate a rectangle of `w x h` on this shelf.
    /// Returns the region if successful.
    fn allocate(&mut self, w: u32, h: u32) -> Option<AtlasRegion> {
        if h > self.height {
            return None;
        }

        for i in 0..self.free_spans.len() {
            let span = self.free_spans[i];
            if span.width >= w {
                let region = AtlasRegion {
                    x: span.x,
                    y: self.y,
                    width: w,
                    height: h,
                };

                // Shrink or remove the span
                if span.width == w {
                    self.free_spans.remove(i);
                } else {
                    self.free_spans[i] = FreeSpan {
                        x: span.x + w,
                        width: span.width - w,
                    };
                }

                return Some(region);
            }
        }

        None
    }

    /// Return a previously allocated region to this shelf.
    fn deallocate(&mut self, region: &AtlasRegion) {
        if region.y != self.y {
            return;
        }

        // Insert the freed span and merge with neighbours
        let new_span = FreeSpan {
            x: region.x,
            width: region.width,
        };

        // Find insertion position (keep sorted by x)
        let pos = self
            .free_spans
            .iter()
            .position(|s| s.x > new_span.x)
            .unwrap_or(self.free_spans.len());

        self.free_spans.insert(pos, new_span);

        // Merge adjacent spans
        self.merge_spans();
    }

    /// Merge adjacent free spans.
    fn merge_spans(&mut self) {
        let mut i = 0;
        while i + 1 < self.free_spans.len() {
            let a_end = self.free_spans[i].x + self.free_spans[i].width;
            if a_end == self.free_spans[i + 1].x {
                self.free_spans[i].width += self.free_spans[i + 1].width;
                self.free_spans.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shelf allocator
// ---------------------------------------------------------------------------

/// Bin-packing shelf allocator for a texture atlas.
///
/// Allocates rectangular regions from a fixed-size atlas using the shelf
/// (skyline) algorithm. Shelves are created lazily as needed.
#[derive(Debug)]
pub struct ShelfAllocator {
    /// Total atlas width in pixels.
    atlas_width: u32,
    /// Total atlas height in pixels.
    atlas_height: u32,
    /// Shelves allocated so far.
    shelves: Vec<Shelf>,
    /// Y coordinate of the next shelf to create.
    next_shelf_y: u32,
    /// Number of live allocations.
    allocation_count: u32,
}

impl ShelfAllocator {
    /// Create a new shelf allocator for an atlas of the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            atlas_width: width,
            atlas_height: height,
            shelves: Vec::new(),
            next_shelf_y: 0,
            allocation_count: 0,
        }
    }

    /// Atlas width.
    pub fn width(&self) -> u32 {
        self.atlas_width
    }

    /// Atlas height.
    pub fn height(&self) -> u32 {
        self.atlas_height
    }

    /// Number of live allocations.
    pub fn allocation_count(&self) -> u32 {
        self.allocation_count
    }

    /// Allocate a region of `width x height` pixels.
    ///
    /// Returns `None` if the atlas is full.
    pub fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasRegion> {
        if width == 0 || height == 0 || width > self.atlas_width {
            return None;
        }

        // Try existing shelves (best-fit by shelf height)
        let mut best_idx: Option<usize> = None;
        let mut best_waste = u32::MAX;

        for (i, shelf) in self.shelves.iter().enumerate() {
            if shelf.height >= height {
                let waste = shelf.height - height;
                // Check if the shelf has space
                let has_space = shelf.free_spans.iter().any(|s| s.width >= width);
                if has_space && waste < best_waste {
                    best_waste = waste;
                    best_idx = Some(i);
                }
            }
        }

        if let Some(idx) = best_idx {
            let region = self.shelves[idx].allocate(width, height);
            if region.is_some() {
                self.allocation_count += 1;
            }
            return region;
        }

        // Create a new shelf
        if self.next_shelf_y + height > self.atlas_height {
            return None; // Atlas full
        }

        let mut shelf = Shelf::new(self.next_shelf_y, height, self.atlas_width);
        let region = shelf.allocate(width, height);
        self.next_shelf_y += height;
        self.shelves.push(shelf);

        if region.is_some() {
            self.allocation_count += 1;
        }
        region
    }

    /// Deallocate a previously allocated region.
    ///
    /// The region is returned to its shelf for reuse.
    pub fn deallocate(&mut self, region: &AtlasRegion) {
        for shelf in &mut self.shelves {
            if shelf.y == region.y {
                shelf.deallocate(region);
                if self.allocation_count > 0 {
                    self.allocation_count -= 1;
                }
                return;
            }
        }
    }

    /// Total number of shelves created.
    pub fn shelf_count(&self) -> usize {
        self.shelves.len()
    }

    /// Fraction of atlas height used, as a percentage (0-100).
    pub fn utilisation_percent(&self) -> u32 {
        if self.atlas_height == 0 {
            return 0;
        }
        (self.next_shelf_y * 100) / self.atlas_height
    }

    /// Reset the allocator, freeing all regions.
    pub fn reset(&mut self) {
        self.shelves.clear();
        self.next_shelf_y = 0;
        self.allocation_count = 0;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atlas_region_basics() {
        let r = AtlasRegion {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        assert_eq!(r.area(), 1200);
        assert!(r.contains(10, 20));
        assert!(r.contains(39, 59));
        assert!(!r.contains(40, 20));
    }

    #[test]
    fn test_allocate_single() {
        let mut alloc = ShelfAllocator::new(256, 256);
        let r = alloc.allocate(64, 32);
        assert!(r.is_some());
        let r = r.unwrap();
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 64);
        assert_eq!(r.height, 32);
        assert_eq!(alloc.allocation_count(), 1);
    }

    #[test]
    fn test_allocate_multiple_same_height() {
        let mut alloc = ShelfAllocator::new(256, 256);
        let r1 = alloc.allocate(64, 32).unwrap();
        let r2 = alloc.allocate(64, 32).unwrap();
        // Both should be on the same shelf
        assert_eq!(r1.y, r2.y);
        assert_eq!(r2.x, 64);
        assert_eq!(alloc.shelf_count(), 1);
    }

    #[test]
    fn test_allocate_different_heights() {
        let mut alloc = ShelfAllocator::new(256, 256);
        let r1 = alloc.allocate(64, 32).unwrap();
        let r2 = alloc.allocate(64, 64).unwrap();
        // Should be on different shelves
        assert_eq!(r1.y, 0);
        assert_eq!(r2.y, 32);
        assert_eq!(alloc.shelf_count(), 2);
    }

    #[test]
    fn test_allocate_full() {
        let mut alloc = ShelfAllocator::new(64, 64);
        let _ = alloc.allocate(64, 64).unwrap();
        let r = alloc.allocate(1, 1);
        assert!(r.is_none());
    }

    #[test]
    fn test_allocate_too_wide() {
        let mut alloc = ShelfAllocator::new(64, 64);
        let r = alloc.allocate(128, 32);
        assert!(r.is_none());
    }

    #[test]
    fn test_deallocate_reuse() {
        let mut alloc = ShelfAllocator::new(128, 128);
        let r1 = alloc.allocate(64, 32).unwrap();
        let _ = alloc.allocate(64, 32).unwrap();
        alloc.deallocate(&r1);
        // Should be able to allocate in the freed space
        let r3 = alloc.allocate(64, 32).unwrap();
        assert_eq!(r3.x, 0);
        assert_eq!(r3.y, 0);
    }

    #[test]
    fn test_zero_size() {
        let mut alloc = ShelfAllocator::new(256, 256);
        assert!(alloc.allocate(0, 32).is_none());
        assert!(alloc.allocate(32, 0).is_none());
    }

    #[test]
    fn test_utilisation() {
        let mut alloc = ShelfAllocator::new(256, 100);
        assert_eq!(alloc.utilisation_percent(), 0);
        let _ = alloc.allocate(128, 50).unwrap();
        assert_eq!(alloc.utilisation_percent(), 50);
    }
}
