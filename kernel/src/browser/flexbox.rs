//! CSS Flexbox Layout
//!
//! Implements the CSS Flexible Box Layout algorithm. All dimensions
//! use 26.6 fixed-point arithmetic (i32) consistent with Phase A.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::events::NodeId;

// ---------------------------------------------------------------------------
// 26.6 fixed-point helpers
// ---------------------------------------------------------------------------

/// 26.6 fixed-point type
pub type FixedPoint = i32;

/// Shift for 26.6 fixed-point
pub const FP_SHIFT: u32 = 6;

/// Convert integer to 26.6 fixed-point
#[inline]
pub const fn fp(v: i32) -> FixedPoint {
    v << FP_SHIFT
}

/// Convert 26.6 fixed-point to integer (truncate)
#[inline]
pub const fn fp_int(v: FixedPoint) -> i32 {
    v >> FP_SHIFT
}

/// Multiply two 26.6 fixed-point values
#[inline]
pub const fn fp_mul(a: FixedPoint, b: FixedPoint) -> FixedPoint {
    ((a as i64 * b as i64) >> FP_SHIFT) as i32
}

/// Divide two 26.6 fixed-point values
#[inline]
pub fn fp_div(a: FixedPoint, b: FixedPoint) -> FixedPoint {
    if b == 0 {
        return 0;
    }
    (((a as i64) << FP_SHIFT) / (b as i64)) as i32
}

/// Zero in 26.6
pub const FP_ZERO: FixedPoint = 0;

/// One in 26.6
pub const FP_ONE: FixedPoint = 1 << FP_SHIFT;

// ---------------------------------------------------------------------------
// Flex properties
// ---------------------------------------------------------------------------

/// Flex direction (main axis orientation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    /// Whether the main axis is horizontal
    pub fn is_row(self) -> bool {
        matches!(self, Self::Row | Self::RowReverse)
    }

    /// Whether the direction is reversed
    pub fn is_reverse(self) -> bool {
        matches!(self, Self::RowReverse | Self::ColumnReverse)
    }
}

/// Flex wrapping behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// Cross-axis alignment for flex items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    #[default]
    Stretch,
    Baseline,
}

/// Main-axis content distribution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// Alignment for multiple flex lines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    #[default]
    Stretch,
    SpaceBetween,
    SpaceAround,
}

/// Per-item alignment override
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

// ---------------------------------------------------------------------------
// Flex item
// ---------------------------------------------------------------------------

/// A flex item with its computed properties
#[derive(Debug, Clone)]
pub struct FlexItem {
    /// DOM node ID
    pub node_id: NodeId,
    /// CSS order property
    pub order: i32,
    /// flex-grow factor (26.6 fixed-point, e.g., fp(1) = 1.0)
    pub flex_grow: FixedPoint,
    /// flex-shrink factor
    pub flex_shrink: FixedPoint,
    /// flex-basis (26.6 fixed-point pixels)
    pub flex_basis: FixedPoint,
    /// Minimum main-axis size
    pub min_main: FixedPoint,
    /// Maximum main-axis size (0 = none)
    pub max_main: FixedPoint,
    /// Minimum cross-axis size
    pub min_cross: FixedPoint,
    /// Maximum cross-axis size (0 = none)
    pub max_cross: FixedPoint,
    /// Hypothetical main size (from content or flex-basis)
    pub hyp_main: FixedPoint,
    /// Hypothetical cross size
    pub hyp_cross: FixedPoint,
    /// Item self-alignment override
    pub align_self: AlignSelf,

    // -- Output (computed layout position) --
    /// Main-axis offset from container start
    pub main_offset: FixedPoint,
    /// Cross-axis offset from line start
    pub cross_offset: FixedPoint,
    /// Final main-axis size
    pub main_size: FixedPoint,
    /// Final cross-axis size
    pub cross_size: FixedPoint,
    /// Whether this item was frozen during flexible length resolution
    frozen: bool,
    /// Target main size during resolution
    target_main: FixedPoint,
}

impl Default for FlexItem {
    fn default() -> Self {
        Self {
            node_id: 0,
            order: 0,
            flex_grow: FP_ZERO,
            flex_shrink: FP_ONE,
            flex_basis: FP_ZERO,
            min_main: FP_ZERO,
            max_main: 0,
            min_cross: FP_ZERO,
            max_cross: 0,
            hyp_main: FP_ZERO,
            hyp_cross: FP_ZERO,
            align_self: AlignSelf::Auto,
            main_offset: FP_ZERO,
            cross_offset: FP_ZERO,
            main_size: FP_ZERO,
            cross_size: FP_ZERO,
            frozen: false,
            target_main: FP_ZERO,
        }
    }
}

impl FlexItem {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            ..Default::default()
        }
    }

    /// Clamp the target main size to min/max constraints
    fn clamp_main(&self, size: FixedPoint) -> FixedPoint {
        let clamped = if self.min_main > 0 {
            size.max(self.min_main)
        } else {
            size
        };
        if self.max_main > 0 {
            clamped.min(self.max_main)
        } else {
            clamped
        }
    }
}

// ---------------------------------------------------------------------------
// Flex line (for wrapping)
// ---------------------------------------------------------------------------

/// A single line of flex items (created during wrapping)
#[derive(Debug, Clone, Default)]
struct FlexLine {
    /// Indices into the items vec
    item_indices: Vec<usize>,
    /// Cross-axis size of this line
    cross_size: FixedPoint,
    /// Cross-axis offset of this line
    cross_offset: FixedPoint,
}

// ---------------------------------------------------------------------------
// Flex container / layout engine
// ---------------------------------------------------------------------------

/// Flex container properties
#[derive(Debug, Clone, Default)]
pub struct FlexContainerStyle {
    pub direction: FlexDirection,
    pub wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_content: AlignContent,
}

/// Flex layout engine
pub struct FlexLayout {
    /// Container style
    pub style: FlexContainerStyle,
    /// Available main-axis size (26.6 fixed-point)
    pub available_main: FixedPoint,
    /// Available cross-axis size
    pub available_cross: FixedPoint,
}

impl FlexLayout {
    pub fn new(
        style: FlexContainerStyle,
        available_main: FixedPoint,
        available_cross: FixedPoint,
    ) -> Self {
        Self {
            style,
            available_main,
            available_cross,
        }
    }

    /// Run the full flexbox layout algorithm on the given items.
    /// Modifies items in-place with computed positions and sizes.
    pub fn layout(&self, items: &mut [FlexItem]) {
        if items.is_empty() {
            return;
        }

        // Sort by CSS order
        items.sort_by_key(|item| item.order);

        // If direction is reversed, reverse the item order
        if self.style.direction.is_reverse() {
            items.reverse();
        }

        // Collect items into flex lines
        let lines = self.collect_into_lines(items);

        // Resolve flexible lengths per line
        for line in &lines {
            self.resolve_flexible_lengths(items, &line.item_indices);
        }

        // Compute cross sizes
        let mut computed_lines = self.compute_cross_sizes(items, &lines);

        // Distribute cross-axis space
        self.distribute_cross_space(&mut computed_lines, items);

        // Position items on main axis per line
        for line in &computed_lines {
            self.position_main_axis(items, &line.item_indices);
        }

        // Position items on cross axis
        for line in &computed_lines {
            self.position_cross_axis(items, line);
        }
    }

    /// Break items into flex lines (wrapping)
    fn collect_into_lines(&self, items: &[FlexItem]) -> Vec<FlexLine> {
        let mut lines = Vec::new();
        let mut current_line = FlexLine::default();
        let mut line_main_size: FixedPoint = FP_ZERO;

        for (i, item) in items.iter().enumerate() {
            let item_main = if item.flex_basis > 0 {
                item.flex_basis
            } else {
                item.hyp_main
            };

            if self.style.wrap != FlexWrap::NoWrap
                && !current_line.item_indices.is_empty()
                && line_main_size + item_main > self.available_main
            {
                lines.push(current_line);
                current_line = FlexLine::default();
                line_main_size = FP_ZERO;
            }

            current_line.item_indices.push(i);
            line_main_size += item_main;
        }

        if !current_line.item_indices.is_empty() {
            lines.push(current_line);
        }

        if self.style.wrap == FlexWrap::WrapReverse {
            lines.reverse();
        }

        lines
    }

    /// Resolve flexible lengths using the grow/shrink algorithm
    fn resolve_flexible_lengths(&self, items: &mut [FlexItem], indices: &[usize]) {
        // Initialize: set each item's target to its flex basis
        for &idx in indices {
            items[idx].frozen = false;
            items[idx].target_main = if items[idx].flex_basis > 0 {
                items[idx].flex_basis
            } else {
                items[idx].hyp_main
            };
        }

        // Calculate used space
        let used: FixedPoint = indices.iter().map(|&i| items[i].target_main).sum();

        let free_space = self.available_main - used;
        let growing = free_space > 0;

        // Freeze items that cannot flex
        for &idx in indices {
            let item = &mut items[idx];
            if (growing && item.flex_grow == 0) || (!growing && item.flex_shrink == 0) {
                item.frozen = true;
                item.main_size = item.target_main;
            }
        }

        // Iterative resolution (up to 10 iterations to prevent infinite loops)
        for _ in 0..10 {
            let unfrozen: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|&i| !items[i].frozen)
                .collect();

            if unfrozen.is_empty() {
                break;
            }

            // Recalculate free space considering frozen items
            let frozen_size: FixedPoint = indices
                .iter()
                .filter(|&&i| items[i].frozen)
                .map(|&i| items[i].main_size)
                .sum();
            let unfrozen_basis: FixedPoint = unfrozen.iter().map(|&i| items[i].target_main).sum();
            let remaining = self.available_main - frozen_size - unfrozen_basis;

            if growing {
                let total_grow: FixedPoint = unfrozen.iter().map(|&i| items[i].flex_grow).sum();
                if total_grow > 0 {
                    for &idx in &unfrozen {
                        // Single-step division avoids intermediate fp_div truncation
                        let addition = ((remaining as i64 * items[idx].flex_grow as i64)
                            / total_grow as i64) as i32;
                        items[idx].target_main += addition;
                    }
                }
            } else {
                let total_shrink: FixedPoint = unfrozen
                    .iter()
                    .map(|&i| fp_mul(items[i].flex_shrink, items[i].target_main))
                    .sum();
                if total_shrink > 0 {
                    for &idx in &unfrozen {
                        let scaled = fp_mul(items[idx].flex_shrink, items[idx].target_main);
                        let ratio = fp_div(scaled, total_shrink);
                        let reduction = fp_mul(remaining.abs(), ratio);
                        items[idx].target_main -= reduction;
                    }
                }
            }

            // Clamp and freeze items that hit min/max
            let mut all_ok = true;
            for &idx in &unfrozen {
                let clamped = items[idx].clamp_main(items[idx].target_main);
                if clamped != items[idx].target_main {
                    items[idx].target_main = clamped;
                    items[idx].frozen = true;
                    all_ok = false;
                }
            }

            // Freeze all remaining if nothing was clamped
            if all_ok {
                for &idx in &unfrozen {
                    items[idx].frozen = true;
                }
                break;
            }
        }

        // Apply computed sizes
        for &idx in indices {
            items[idx].main_size = items[idx].clamp_main(items[idx].target_main);
        }
    }

    /// Compute cross sizes for each line
    fn compute_cross_sizes(&self, items: &[FlexItem], lines: &[FlexLine]) -> Vec<FlexLine> {
        let mut result = Vec::with_capacity(lines.len());
        for line in lines {
            let mut max_cross = FP_ZERO;
            for &idx in &line.item_indices {
                let cross = if items[idx].hyp_cross > 0 {
                    items[idx].hyp_cross
                } else {
                    items[idx].main_size // fallback: square
                };
                if cross > max_cross {
                    max_cross = cross;
                }
            }
            result.push(FlexLine {
                item_indices: line.item_indices.clone(),
                cross_size: max_cross,
                cross_offset: FP_ZERO,
            });
        }
        result
    }

    /// Distribute cross-axis space among lines
    fn distribute_cross_space(&self, lines: &mut [FlexLine], _items: &[FlexItem]) {
        let total_cross: FixedPoint = lines.iter().map(|l| l.cross_size).sum();
        let free_cross = self.available_cross - total_cross;

        match self.style.align_content {
            AlignContent::FlexStart => {
                let mut offset = FP_ZERO;
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    offset += line.cross_size;
                }
            }
            AlignContent::FlexEnd => {
                let mut offset = free_cross.max(FP_ZERO);
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    offset += line.cross_size;
                }
            }
            AlignContent::Center => {
                let mut offset = (free_cross / 2).max(FP_ZERO);
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    offset += line.cross_size;
                }
            }
            AlignContent::Stretch => {
                let extra_per_line = if !lines.is_empty() && free_cross > 0 {
                    free_cross / lines.len() as i32
                } else {
                    FP_ZERO
                };
                let mut offset = FP_ZERO;
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    line.cross_size += extra_per_line;
                    offset += line.cross_size;
                }
            }
            AlignContent::SpaceBetween => {
                let gap = if lines.len() > 1 && free_cross > 0 {
                    free_cross / (lines.len() as i32 - 1)
                } else {
                    FP_ZERO
                };
                let mut offset = FP_ZERO;
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    offset += line.cross_size + gap;
                }
            }
            AlignContent::SpaceAround => {
                let gap = if !lines.is_empty() && free_cross > 0 {
                    free_cross / lines.len() as i32
                } else {
                    FP_ZERO
                };
                let mut offset = gap / 2;
                for line in lines.iter_mut() {
                    line.cross_offset = offset;
                    offset += line.cross_size + gap;
                }
            }
        }
    }

    /// Position items along the main axis within a line
    fn position_main_axis(&self, items: &mut [FlexItem], indices: &[usize]) {
        let total_main: FixedPoint = indices.iter().map(|&i| items[i].main_size).sum();
        let free = self.available_main - total_main;
        let count = indices.len() as i32;

        let (mut offset, gap) = match self.style.justify_content {
            JustifyContent::FlexStart => (FP_ZERO, FP_ZERO),
            JustifyContent::FlexEnd => (free.max(FP_ZERO), FP_ZERO),
            JustifyContent::Center => ((free / 2).max(FP_ZERO), FP_ZERO),
            JustifyContent::SpaceBetween => {
                let g = if count > 1 && free > 0 {
                    free / (count - 1)
                } else {
                    FP_ZERO
                };
                (FP_ZERO, g)
            }
            JustifyContent::SpaceAround => {
                let g = if count > 0 && free > 0 {
                    free / count
                } else {
                    FP_ZERO
                };
                (g / 2, g)
            }
            JustifyContent::SpaceEvenly => {
                let g = if count > 0 && free > 0 {
                    free / (count + 1)
                } else {
                    FP_ZERO
                };
                (g, g)
            }
        };

        for &idx in indices {
            items[idx].main_offset = offset;
            offset += items[idx].main_size + gap;
        }
    }

    /// Position items along the cross axis within a line
    fn position_cross_axis(&self, items: &mut [FlexItem], line: &FlexLine) {
        for &idx in &line.item_indices {
            let align = match items[idx].align_self {
                AlignSelf::Auto => self.style.align_items,
                AlignSelf::FlexStart => AlignItems::FlexStart,
                AlignSelf::FlexEnd => AlignItems::FlexEnd,
                AlignSelf::Center => AlignItems::Center,
                AlignSelf::Stretch => AlignItems::Stretch,
                AlignSelf::Baseline => AlignItems::Baseline,
            };

            let item_cross = if items[idx].hyp_cross > 0 {
                items[idx].hyp_cross
            } else {
                line.cross_size
            };

            match align {
                AlignItems::FlexStart => {
                    items[idx].cross_offset = line.cross_offset;
                    items[idx].cross_size = item_cross;
                }
                AlignItems::FlexEnd => {
                    items[idx].cross_offset = line.cross_offset + line.cross_size - item_cross;
                    items[idx].cross_size = item_cross;
                }
                AlignItems::Center => {
                    items[idx].cross_offset =
                        line.cross_offset + (line.cross_size - item_cross) / 2;
                    items[idx].cross_size = item_cross;
                }
                AlignItems::Stretch => {
                    items[idx].cross_offset = line.cross_offset;
                    items[idx].cross_size = line.cross_size;
                }
                AlignItems::Baseline => {
                    // Baseline alignment falls back to flex-start
                    items[idx].cross_offset = line.cross_offset;
                    items[idx].cross_size = item_cross;
                }
            }
        }
    }

    /// Get the final bounding rectangle for a flex item.
    /// Returns (x, y, width, height) in 26.6 fixed-point.
    pub fn item_rect(
        item: &FlexItem,
        direction: FlexDirection,
    ) -> (FixedPoint, FixedPoint, FixedPoint, FixedPoint) {
        if direction.is_row() {
            (
                item.main_offset,
                item.cross_offset,
                item.main_size,
                item.cross_size,
            )
        } else {
            (
                item.cross_offset,
                item.main_offset,
                item.cross_size,
                item.main_size,
            )
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

    fn make_item(node_id: NodeId, basis: i32, grow: i32, shrink: i32) -> FlexItem {
        FlexItem {
            node_id,
            flex_basis: fp(basis),
            flex_grow: fp(grow),
            flex_shrink: fp(shrink),
            hyp_main: fp(basis),
            hyp_cross: fp(30),
            ..Default::default()
        }
    }

    #[test]
    fn test_fp_helpers() {
        assert_eq!(fp(10), 640);
        assert_eq!(fp_int(fp(10)), 10);
        assert_eq!(fp_int(fp_mul(fp(3), fp(4))), 12);
    }

    #[test]
    fn test_fp_div() {
        assert_eq!(fp_int(fp_div(fp(10), fp(2))), 5);
        assert_eq!(fp_div(fp(1), fp(0)), 0);
    }

    #[test]
    fn test_direction_is_row() {
        assert!(FlexDirection::Row.is_row());
        assert!(FlexDirection::RowReverse.is_row());
        assert!(!FlexDirection::Column.is_row());
    }

    #[test]
    fn test_direction_is_reverse() {
        assert!(!FlexDirection::Row.is_reverse());
        assert!(FlexDirection::RowReverse.is_reverse());
        assert!(FlexDirection::ColumnReverse.is_reverse());
    }

    #[test]
    fn test_single_item_fills_container() {
        let style = FlexContainerStyle {
            direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 1, 0)];
        layout.layout(&mut items);
        // Single item with grow=1 should fill 300
        assert_eq!(fp_int(items[0].main_size), 300);
    }

    #[test]
    fn test_two_items_equal_grow() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(200), fp(100));
        let mut items = vec![make_item(0, 50, 1, 0), make_item(1, 50, 1, 0)];
        layout.layout(&mut items);
        // Free space 100, split equally
        assert_eq!(fp_int(items[0].main_size), 100);
        assert_eq!(fp_int(items[1].main_size), 100);
    }

    #[test]
    fn test_unequal_grow() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 0, 1, 0), make_item(1, 0, 2, 0)];
        layout.layout(&mut items);
        // grow 1:2 ratio on 300px
        assert_eq!(fp_int(items[0].main_size), 100);
        assert_eq!(fp_int(items[1].main_size), 200);
    }

    #[test]
    fn test_no_grow() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 80, 0, 0), make_item(1, 60, 0, 0)];
        layout.layout(&mut items);
        assert_eq!(fp_int(items[0].main_size), 80);
        assert_eq!(fp_int(items[1].main_size), 60);
    }

    #[test]
    fn test_justify_center() {
        let style = FlexContainerStyle {
            justify_content: JustifyContent::Center,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 0, 0)];
        layout.layout(&mut items);
        // 100px item centered in 300px => offset = 100
        assert_eq!(fp_int(items[0].main_offset), 100);
    }

    #[test]
    fn test_justify_flex_end() {
        let style = FlexContainerStyle {
            justify_content: JustifyContent::FlexEnd,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 0, 0)];
        layout.layout(&mut items);
        assert_eq!(fp_int(items[0].main_offset), 200);
    }

    #[test]
    fn test_justify_space_between() {
        let style = FlexContainerStyle {
            justify_content: JustifyContent::SpaceBetween,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![
            make_item(0, 50, 0, 0),
            make_item(1, 50, 0, 0),
            make_item(2, 50, 0, 0),
        ];
        layout.layout(&mut items);
        // 300 - 150 = 150 free, 2 gaps = 75 each
        assert_eq!(fp_int(items[0].main_offset), 0);
        assert_eq!(fp_int(items[1].main_offset), 125); // 50 + 75
        assert_eq!(fp_int(items[2].main_offset), 250); // 50 + 75 + 50 + 75
    }

    #[test]
    fn test_justify_space_evenly() {
        let style = FlexContainerStyle {
            justify_content: JustifyContent::SpaceEvenly,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(400), fp(100));
        let mut items = vec![make_item(0, 50, 0, 0), make_item(1, 50, 0, 0)];
        layout.layout(&mut items);
        // 400 - 100 = 300 free, 3 gaps = 100 each
        assert_eq!(fp_int(items[0].main_offset), 100);
        assert_eq!(fp_int(items[1].main_offset), 250);
    }

    #[test]
    fn test_align_items_center() {
        let style = FlexContainerStyle {
            align_items: AlignItems::Center,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 0, 0)];
        items[0].hyp_cross = fp(40);
        layout.layout(&mut items);
        // Cross size 40, line cross 40, centered in 40 => 0
        // But the line cross_size = max(hyp_cross) = 40
        // Then distribute_cross_space stretches to available_cross=100
        // With Stretch align_content (default), line gets 100px cross
        // Center: offset = (100 - 40)/2 = 30
        assert_eq!(fp_int(items[0].cross_offset), 30);
    }

    #[test]
    fn test_align_items_stretch() {
        let style = FlexContainerStyle {
            align_items: AlignItems::Stretch,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 0, 0)];
        items[0].hyp_cross = fp(40);
        layout.layout(&mut items);
        // Stretch: item cross = line cross = 100 (after stretch)
        assert_eq!(fp_int(items[0].cross_size), 100);
    }

    #[test]
    fn test_wrap_basic() {
        let style = FlexContainerStyle {
            wrap: FlexWrap::Wrap,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(200), fp(200));
        let mut items = vec![make_item(0, 120, 0, 0), make_item(1, 120, 0, 0)];
        layout.layout(&mut items);
        // 120 + 120 > 200, so wrap to 2 lines
        // Item 0 on first line, item 1 on second line
        assert_eq!(fp_int(items[0].main_offset), 0);
        assert_eq!(fp_int(items[1].main_offset), 0);
        // Item 1 should be on a different cross line
        assert!(items[1].cross_offset > items[0].cross_offset);
    }

    #[test]
    fn test_nowrap() {
        let style = FlexContainerStyle {
            wrap: FlexWrap::NoWrap,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(200), fp(100));
        let mut items = vec![make_item(0, 120, 0, 1), make_item(1, 120, 0, 1)];
        layout.layout(&mut items);
        // No wrap: both on same line, may overflow or shrink
        // With shrink=1, items shrink proportionally
    }

    #[test]
    fn test_order() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![
            {
                let mut item = make_item(0, 50, 0, 0);
                item.order = 2;
                item
            },
            {
                let mut item = make_item(1, 50, 0, 0);
                item.order = 1;
                item
            },
        ];
        layout.layout(&mut items);
        // Order 1 comes first
        assert!(items[0].order <= items[1].order);
    }

    #[test]
    fn test_reverse() {
        let style = FlexContainerStyle {
            direction: FlexDirection::RowReverse,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 100, 0, 0), make_item(1, 100, 0, 0)];
        layout.layout(&mut items);
        // Items reversed: original 1 is now at offset 0
    }

    #[test]
    fn test_column_direction() {
        let style = FlexContainerStyle {
            direction: FlexDirection::Column,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(400), fp(300));
        let mut items = vec![make_item(0, 100, 1, 0), make_item(1, 100, 1, 0)];
        layout.layout(&mut items);
        // Column: main = vertical, so items split 400 height
        assert_eq!(fp_int(items[0].main_size), 200);
        assert_eq!(fp_int(items[1].main_size), 200);
    }

    #[test]
    fn test_item_rect_row() {
        let mut item = FlexItem::new(0);
        item.main_offset = fp(10);
        item.cross_offset = fp(20);
        item.main_size = fp(100);
        item.cross_size = fp(50);
        let (x, y, w, h) = FlexLayout::item_rect(&item, FlexDirection::Row);
        assert_eq!(fp_int(x), 10);
        assert_eq!(fp_int(y), 20);
        assert_eq!(fp_int(w), 100);
        assert_eq!(fp_int(h), 50);
    }

    #[test]
    fn test_item_rect_column() {
        let mut item = FlexItem::new(0);
        item.main_offset = fp(10);
        item.cross_offset = fp(20);
        item.main_size = fp(100);
        item.cross_size = fp(50);
        let (x, y, w, h) = FlexLayout::item_rect(&item, FlexDirection::Column);
        // Column: x=cross_offset, y=main_offset
        assert_eq!(fp_int(x), 20);
        assert_eq!(fp_int(y), 10);
        assert_eq!(fp_int(w), 50);
        assert_eq!(fp_int(h), 100);
    }

    #[test]
    fn test_shrink() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(100), fp(50));
        let mut items = vec![make_item(0, 80, 0, 1), make_item(1, 80, 0, 1)];
        layout.layout(&mut items);
        // 160 > 100, shrink by 60 total
        // Equal shrink factors, equal basis => 50 each
        assert_eq!(fp_int(items[0].main_size), 50);
        assert_eq!(fp_int(items[1].main_size), 50);
    }

    #[test]
    fn test_min_max_clamp() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![{
            let mut item = make_item(0, 50, 1, 0);
            item.max_main = fp(200);
            item
        }];
        layout.layout(&mut items);
        // Would grow to 300, but clamped to 200
        assert_eq!(fp_int(items[0].main_size), 200);
    }

    #[test]
    fn test_align_self_override() {
        let style = FlexContainerStyle {
            align_items: AlignItems::FlexStart,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![{
            let mut item = make_item(0, 100, 0, 0);
            item.align_self = AlignSelf::FlexEnd;
            item.hyp_cross = fp(30);
            item
        }];
        layout.layout(&mut items);
        // AlignSelf::FlexEnd overrides FlexStart
        // Line cross = 100 (stretched), item cross = 30
        // FlexEnd: offset = 100 - 30 = 70
        assert_eq!(fp_int(items[0].cross_offset), 70);
    }

    #[test]
    fn test_empty_items() {
        let style = FlexContainerStyle::default();
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items: Vec<FlexItem> = Vec::new();
        layout.layout(&mut items);
        // Should not panic
    }

    #[test]
    fn test_flex_defaults() {
        let item = FlexItem::default();
        assert_eq!(item.flex_grow, FP_ZERO);
        assert_eq!(item.flex_shrink, FP_ONE);
        assert_eq!(item.order, 0);
    }

    #[test]
    fn test_justify_space_around() {
        let style = FlexContainerStyle {
            justify_content: JustifyContent::SpaceAround,
            ..Default::default()
        };
        let layout = FlexLayout::new(style, fp(300), fp(100));
        let mut items = vec![make_item(0, 50, 0, 0), make_item(1, 50, 0, 0)];
        layout.layout(&mut items);
        // Free = 200, 2 items, gap = 100, start = 50
        assert_eq!(fp_int(items[0].main_offset), 50);
        assert_eq!(fp_int(items[1].main_offset), 200);
    }
}
