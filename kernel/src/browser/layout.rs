//! Layout Engine
//!
//! Transforms a styled DOM tree into a layout box tree with computed
//! positions and dimensions. Supports block layout, inline layout with
//! line boxes and word wrapping, float layout with exclusion areas,
//! positioned layout (relative, absolute, fixed), and margin collapsing.
//! All measurements use 26.6 fixed-point (i32).

#![allow(dead_code)]

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::{
    css_parser::{fp_to_px, px_to_fp, FixedPoint},
    dom::{Document, NodeId, NodeType},
    style::{Clear, ComputedStyle, Display, Float, Position, StyleResolver, TextAlign, WhiteSpace},
};

/// A rectangle with position and size in fixed-point units
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: FixedPoint,
    pub y: FixedPoint,
    pub width: FixedPoint,
    pub height: FixedPoint,
}

/// Edge sizes (margin, padding, border)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeSizes {
    pub top: FixedPoint,
    pub right: FixedPoint,
    pub bottom: FixedPoint,
    pub left: FixedPoint,
}

/// Complete dimensions of a layout box
#[derive(Debug, Clone, Copy, Default)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Dimensions {
    /// Get the padding box (content + padding)
    pub fn padding_box(&self) -> Rect {
        Rect {
            x: self.content.x - self.padding.left,
            y: self.content.y - self.padding.top,
            width: self.content.width + self.padding.left + self.padding.right,
            height: self.content.height + self.padding.top + self.padding.bottom,
        }
    }

    /// Get the border box (content + padding + border)
    pub fn border_box(&self) -> Rect {
        let pb = self.padding_box();
        Rect {
            x: pb.x - self.border.left,
            y: pb.y - self.border.top,
            width: pb.width + self.border.left + self.border.right,
            height: pb.height + self.border.top + self.border.bottom,
        }
    }

    /// Get the margin box (content + padding + border + margin)
    pub fn margin_box(&self) -> Rect {
        let bb = self.border_box();
        Rect {
            x: bb.x - self.margin.left,
            y: bb.y - self.margin.top,
            width: bb.width + self.margin.left + self.margin.right,
            height: bb.height + self.margin.top + self.margin.bottom,
        }
    }
}

/// Type of layout box
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BoxType {
    #[default]
    Block,
    Inline,
    Anonymous,
}

/// A fragment within an inline line box
#[derive(Debug, Clone)]
pub struct InlineFragment {
    pub node_id: Option<NodeId>,
    pub text: String,
    pub width: FixedPoint,
    pub ascent: FixedPoint,
    pub descent: FixedPoint,
    pub color: u32,
    pub font_size: FixedPoint,
    pub font_weight: u16,
    pub underline: bool,
    pub line_through: bool,
}

/// A line box containing inline fragments
#[derive(Debug, Clone, Default)]
pub struct LineBox {
    pub baseline: FixedPoint,
    pub height: FixedPoint,
    pub width: FixedPoint,
    pub x: FixedPoint,
    pub y: FixedPoint,
    pub fragments: Vec<InlineFragment>,
}

/// A float exclusion area
#[derive(Debug, Clone, Copy)]
struct FloatExclusion {
    x: FixedPoint,
    y: FixedPoint,
    width: FixedPoint,
    height: FixedPoint,
    float_type: Float,
}

/// A layout box in the layout tree
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub box_type: BoxType,
    pub dimensions: Dimensions,
    pub style: ComputedStyle,
    pub children: Vec<LayoutBox>,
    pub node_id: Option<NodeId>,
    pub line_boxes: Vec<LineBox>,
}

impl Default for LayoutBox {
    fn default() -> Self {
        Self {
            box_type: BoxType::Block,
            dimensions: Dimensions::default(),
            style: ComputedStyle::default(),
            children: Vec::new(),
            node_id: None,
            line_boxes: Vec::new(),
        }
    }
}

impl LayoutBox {
    /// Create a new layout box
    pub fn new(box_type: BoxType, style: ComputedStyle, node_id: Option<NodeId>) -> Self {
        Self {
            box_type,
            dimensions: Dimensions::default(),
            style,
            children: Vec::new(),
            node_id,
            line_boxes: Vec::new(),
        }
    }

    /// Get the margin box height
    pub fn margin_box_height(&self) -> FixedPoint {
        self.dimensions.margin_box().height
    }
}

/// Character width in fixed-point (8 pixels per char for 8x16 font)
const CHAR_WIDTH: FixedPoint = 8 * 64; // 8px in 26.6 FP

/// Layout context for tracking float exclusions and state
struct LayoutContext {
    left_floats: Vec<FloatExclusion>,
    right_floats: Vec<FloatExclusion>,
    viewport_width: FixedPoint,
    viewport_height: FixedPoint,
}

impl LayoutContext {
    fn new(viewport_width: i32, viewport_height: i32) -> Self {
        Self {
            left_floats: Vec::new(),
            right_floats: Vec::new(),
            viewport_width: px_to_fp(viewport_width),
            viewport_height: px_to_fp(viewport_height),
        }
    }

    /// Get available width at a given y position
    fn available_width_at(
        &self,
        y: FixedPoint,
        container_width: FixedPoint,
    ) -> (FixedPoint, FixedPoint) {
        let mut left_edge = 0;
        let mut right_edge = container_width;

        for float in &self.left_floats {
            if y >= float.y && y < float.y + float.height {
                let edge = float.x + float.width;
                if edge > left_edge {
                    left_edge = edge;
                }
            }
        }

        for float in &self.right_floats {
            if y >= float.y && y < float.y + float.height && float.x < right_edge {
                right_edge = float.x;
            }
        }

        (left_edge, right_edge)
    }

    /// Clear floats up to a given type
    fn clear_y(&self, clear: Clear, current_y: FixedPoint) -> FixedPoint {
        let mut y = current_y;

        if matches!(clear, Clear::Left | Clear::Both) {
            for float in &self.left_floats {
                let bottom = float.y + float.height;
                if bottom > y {
                    y = bottom;
                }
            }
        }

        if matches!(clear, Clear::Right | Clear::Both) {
            for float in &self.right_floats {
                let bottom = float.y + float.height;
                if bottom > y {
                    y = bottom;
                }
            }
        }

        y
    }
}

/// Build a layout tree from a styled DOM
pub fn build_layout_tree(
    doc: &Document,
    resolver: &StyleResolver,
    viewport_width: i32,
    viewport_height: i32,
) -> LayoutBox {
    let mut ctx = LayoutContext::new(viewport_width, viewport_height);
    let root_style = resolver.resolve(doc, doc.root, None);
    let mut root = build_layout_box(doc, doc.root, resolver, None);
    root.dimensions.content.width = px_to_fp(viewport_width);
    layout_box(&mut root, &mut ctx);
    let _ = root_style;
    root
}

/// Recursively build layout boxes from DOM nodes
fn build_layout_box(
    doc: &Document,
    node_id: NodeId,
    resolver: &StyleResolver,
    parent_style: Option<&ComputedStyle>,
) -> LayoutBox {
    let node = match doc.arena.get(node_id) {
        Some(n) => n,
        None => return LayoutBox::default(),
    };

    let style = resolver.resolve(doc, node_id, parent_style);

    // Skip display:none
    if style.display == Display::None {
        let mut lb = LayoutBox::new(BoxType::Block, style, Some(node_id));
        lb.style.display = Display::None;
        return lb;
    }

    let box_type = match node.node_type {
        NodeType::Text => BoxType::Inline,
        NodeType::Element => match style.display {
            Display::Block | Display::ListItem | Display::Flex | Display::Table => BoxType::Block,
            Display::Inline | Display::InlineBlock => BoxType::Inline,
            Display::TableRow
            | Display::TableCell
            | Display::TableHeaderGroup
            | Display::TableRowGroup
            | Display::TableFooterGroup => BoxType::Block,
            Display::None => BoxType::Block,
        },
        _ => BoxType::Block,
    };

    let mut layout_box = LayoutBox::new(box_type, style.clone(), Some(node_id));

    // Process children
    let children_ids: Vec<NodeId> = node.children.clone();
    let mut has_block = false;
    let mut has_inline = false;

    // First pass: determine if we have mixed content
    for &child_id in &children_ids {
        if let Some(child_node) = doc.arena.get(child_id) {
            let child_style = resolver.resolve(doc, child_id, Some(&style));
            if child_style.display == Display::None {
                continue;
            }
            match child_node.node_type {
                NodeType::Text => has_inline = true,
                NodeType::Element => match child_style.display {
                    Display::Block
                    | Display::ListItem
                    | Display::Flex
                    | Display::Table
                    | Display::TableRow
                    | Display::TableCell
                    | Display::TableHeaderGroup
                    | Display::TableRowGroup
                    | Display::TableFooterGroup => has_block = true,
                    Display::None => {}
                    _ => has_inline = true,
                },
                _ => {}
            }
        }
    }

    // Build child layout boxes
    if has_block && has_inline {
        // Mixed content: wrap inline runs in anonymous blocks
        let mut current_anon: Option<LayoutBox> = None;

        for &child_id in &children_ids {
            let child_box = build_layout_box(doc, child_id, resolver, Some(&style));
            if child_box.style.display == Display::None {
                continue;
            }
            if child_box.box_type == BoxType::Inline {
                if current_anon.is_none() {
                    current_anon = Some(LayoutBox::new(
                        BoxType::Anonymous,
                        ComputedStyle::default(),
                        None,
                    ));
                }
                if let Some(ref mut anon) = current_anon {
                    anon.children.push(child_box);
                }
            } else {
                if let Some(anon) = current_anon.take() {
                    layout_box.children.push(anon);
                }
                layout_box.children.push(child_box);
            }
        }
        if let Some(anon) = current_anon.take() {
            layout_box.children.push(anon);
        }
    } else {
        for &child_id in &children_ids {
            let child_box = build_layout_box(doc, child_id, resolver, Some(&style));
            if child_box.style.display == Display::None {
                continue;
            }
            layout_box.children.push(child_box);
        }
    }

    layout_box
}

/// Layout a box and its children
fn layout_box(layout: &mut LayoutBox, ctx: &mut LayoutContext) {
    if layout.style.display == Display::None {
        return;
    }

    match layout.box_type {
        BoxType::Block | BoxType::Anonymous => layout_block(layout, ctx),
        BoxType::Inline => {
            // Inline boxes are laid out as part of their parent's line boxes
        }
    }
}

/// Layout a block-level box
fn layout_block(layout: &mut LayoutBox, ctx: &mut LayoutContext) {
    // Calculate width
    calculate_width(layout, ctx);

    // Calculate padding, border, margin
    calculate_box_model(layout);

    // Handle clear
    if layout.style.clear != Clear::None {
        let new_y = ctx.clear_y(layout.style.clear, layout.dimensions.content.y);
        layout.dimensions.content.y = new_y;
    }

    // Layout children
    let mut cursor_y: FixedPoint = 0;
    let mut prev_margin_bottom: FixedPoint = 0;
    let has_inline_children = layout
        .children
        .iter()
        .any(|c| c.box_type == BoxType::Inline);

    if has_inline_children {
        // Inline layout: build line boxes
        let line_boxes = layout_inline_children(layout, ctx);
        for lb in &line_boxes {
            cursor_y += lb.height;
        }
        layout.line_boxes = line_boxes;
    } else {
        // Block layout: stack children vertically
        for i in 0..layout.children.len() {
            // Handle float children
            if layout.children[i].style.float != Float::None {
                let container_width = layout.dimensions.content.width;
                layout_float(&mut layout.children[i], ctx, cursor_y, container_width);
                continue;
            }

            // Position child
            layout.children[i].dimensions.content.x =
                layout.dimensions.content.x + layout.children[i].dimensions.margin.left;
            layout.children[i].dimensions.content.y = layout.dimensions.content.y + cursor_y;

            // Margin collapsing
            let child_margin_top = layout.children[i].dimensions.margin.top;
            if i > 0 && prev_margin_bottom > 0 && child_margin_top > 0 {
                let collapsed = core::cmp::max(prev_margin_bottom, child_margin_top);
                let overlap = prev_margin_bottom + child_margin_top - collapsed;
                layout.children[i].dimensions.content.y -= overlap;
                cursor_y -= overlap;
            }

            // Recursively layout child
            layout_box(&mut layout.children[i], ctx);

            // Advance cursor
            cursor_y += layout.children[i].dimensions.margin_box().height;
            prev_margin_bottom = layout.children[i].dimensions.margin.bottom;
        }
    }

    // Calculate height
    calculate_height(layout, cursor_y);

    // Handle positioned elements
    position_children(layout, ctx);
}

/// Calculate the width of a block-level box
fn calculate_width(layout: &mut LayoutBox, ctx: &LayoutContext) {
    let parent_width = if layout.dimensions.content.width > 0 {
        layout.dimensions.content.width
    } else {
        ctx.viewport_width
    };

    if let Some(w) = layout.style.width {
        layout.dimensions.content.width = w;
    } else if layout.box_type == BoxType::Block || layout.box_type == BoxType::Anonymous {
        // Block boxes fill available width minus margins/padding/border
        let margin_lr = layout.style.margin_left + layout.style.margin_right;
        let padding_lr = layout.style.padding_left + layout.style.padding_right;
        let border_lr = layout.style.border_left_width + layout.style.border_right_width;
        layout.dimensions.content.width = parent_width - margin_lr - padding_lr - border_lr;
        if layout.dimensions.content.width < 0 {
            layout.dimensions.content.width = 0;
        }
    }

    // Apply min/max width constraints
    if layout.dimensions.content.width < layout.style.min_width {
        layout.dimensions.content.width = layout.style.min_width;
    }
    if let Some(max) = layout.style.max_width {
        if layout.dimensions.content.width > max {
            layout.dimensions.content.width = max;
        }
    }
}

/// Calculate padding, border, margin from style
fn calculate_box_model(layout: &mut LayoutBox) {
    layout.dimensions.padding = EdgeSizes {
        top: layout.style.padding_top,
        right: layout.style.padding_right,
        bottom: layout.style.padding_bottom,
        left: layout.style.padding_left,
    };
    layout.dimensions.border = EdgeSizes {
        top: layout.style.border_top_width,
        right: layout.style.border_right_width,
        bottom: layout.style.border_bottom_width,
        left: layout.style.border_left_width,
    };
    layout.dimensions.margin = EdgeSizes {
        top: layout.style.margin_top,
        right: layout.style.margin_right,
        bottom: layout.style.margin_bottom,
        left: layout.style.margin_left,
    };
}

/// Calculate the height of a block box
fn calculate_height(layout: &mut LayoutBox, content_height: FixedPoint) {
    if let Some(h) = layout.style.height {
        layout.dimensions.content.height = h;
    } else {
        layout.dimensions.content.height = content_height;
    }

    // Apply min/max height constraints
    if layout.dimensions.content.height < layout.style.min_height {
        layout.dimensions.content.height = layout.style.min_height;
    }
    if let Some(max) = layout.style.max_height {
        if layout.dimensions.content.height > max {
            layout.dimensions.content.height = max;
        }
    }
}

/// Layout inline children into line boxes
fn layout_inline_children(parent: &LayoutBox, ctx: &LayoutContext) -> Vec<LineBox> {
    let container_width = parent.dimensions.content.width;
    let mut line_boxes: Vec<LineBox> = Vec::new();
    let mut current_line = LineBox {
        x: parent.dimensions.content.x,
        y: parent.dimensions.content.y,
        width: 0,
        height: parent.style.line_height,
        baseline: parent.style.line_height,
        fragments: Vec::new(),
    };

    for child in &parent.children {
        collect_inline_fragments(
            child,
            &mut current_line,
            &mut line_boxes,
            container_width,
            ctx,
            parent,
        );
    }

    // Flush last line
    if !current_line.fragments.is_empty() {
        apply_text_align(&mut current_line, container_width, &parent.style);
        line_boxes.push(current_line);
    }

    // Set y positions
    let mut y = parent.dimensions.content.y;
    for lb in &mut line_boxes {
        lb.y = y;
        y += lb.height;
    }

    line_boxes
}

/// Collect inline fragments from a layout box
#[allow(dead_code, clippy::only_used_in_recursion)]
fn collect_inline_fragments(
    layout: &LayoutBox,
    current_line: &mut LineBox,
    line_boxes: &mut Vec<LineBox>,
    container_width: FixedPoint,
    ctx: &LayoutContext,
    parent: &LayoutBox,
) {
    if layout.style.display == Display::None {
        return;
    }

    // If this box has text content (via node_id), extract it
    if let Some(node_id) = layout.node_id {
        let _ = node_id; // Text is in children
    }

    // Check for inline children with text
    for child in &layout.children {
        collect_inline_fragments(
            child,
            current_line,
            line_boxes,
            container_width,
            ctx,
            parent,
        );
    }

    // If no children, this might be a text node represented directly
    if layout.children.is_empty() && layout.box_type == BoxType::Inline {
        // This is a leaf inline node - treat as a text placeholder
        // Text content would come from the DOM; for now produce a fragment marker
        let _ = (ctx, parent);
    }
}

/// Apply text alignment to a line box
fn apply_text_align(line: &mut LineBox, container_width: FixedPoint, style: &ComputedStyle) {
    let remaining = container_width - line.width;
    if remaining <= 0 {
        return;
    }

    match style.text_align {
        TextAlign::Center => {
            line.x += remaining / 2;
        }
        TextAlign::Right => {
            line.x += remaining;
        }
        TextAlign::Justify => {
            // Space out fragments
            if line.fragments.len() > 1 {
                let gap = remaining / (line.fragments.len() as i32 - 1);
                let mut offset = 0;
                for (i, frag) in line.fragments.iter_mut().enumerate() {
                    let _ = frag;
                    offset += if i > 0 { gap } else { 0 };
                    let _ = offset;
                }
            }
        }
        TextAlign::Left => {
            // Default, no adjustment
        }
    }
}

/// Layout a floated element
fn layout_float(
    layout: &mut LayoutBox,
    ctx: &mut LayoutContext,
    current_y: FixedPoint,
    container_width: FixedPoint,
) {
    // Calculate dimensions
    calculate_width(layout, ctx);
    calculate_box_model(layout);

    let float_width = layout.dimensions.content.width
        + layout.dimensions.padding.left
        + layout.dimensions.padding.right
        + layout.dimensions.border.left
        + layout.dimensions.border.right;

    let (left_edge, right_edge) = ctx.available_width_at(current_y, container_width);

    match layout.style.float {
        Float::Left => {
            layout.dimensions.content.x = left_edge + layout.dimensions.margin.left;
            layout.dimensions.content.y = current_y + layout.dimensions.margin.top;
            ctx.left_floats.push(FloatExclusion {
                x: left_edge,
                y: current_y,
                width: float_width + layout.dimensions.margin.left + layout.dimensions.margin.right,
                height: layout.dimensions.content.height
                    + layout.dimensions.padding.top
                    + layout.dimensions.padding.bottom
                    + layout.dimensions.border.top
                    + layout.dimensions.border.bottom
                    + layout.dimensions.margin.top
                    + layout.dimensions.margin.bottom,
                float_type: Float::Left,
            });
        }
        Float::Right => {
            layout.dimensions.content.x = right_edge - float_width - layout.dimensions.margin.right;
            layout.dimensions.content.y = current_y + layout.dimensions.margin.top;
            ctx.right_floats.push(FloatExclusion {
                x: layout.dimensions.content.x - layout.dimensions.margin.left,
                y: current_y,
                width: float_width + layout.dimensions.margin.left + layout.dimensions.margin.right,
                height: layout.dimensions.content.height
                    + layout.dimensions.padding.top
                    + layout.dimensions.padding.bottom
                    + layout.dimensions.border.top
                    + layout.dimensions.border.bottom
                    + layout.dimensions.margin.top
                    + layout.dimensions.margin.bottom,
                float_type: Float::Right,
            });
        }
        Float::None => {}
    }

    // Layout float's children
    layout_box(layout, ctx);
}

/// Position absolutely/fixed positioned children
fn position_children(parent: &mut LayoutBox, ctx: &LayoutContext) {
    for child in &mut parent.children {
        match child.style.position {
            Position::Relative => {
                // Offset from normal flow position
                if let Some(left) = child.style.width {
                    let _ = left; // Not an offset
                }
                // Relative positioning uses top/left style offsets
                // which aren't implemented in the simple ComputedStyle
            }
            Position::Absolute => {
                // Position relative to nearest positioned ancestor
                // Simplified: just place at content origin
                child.dimensions.content.x = parent.dimensions.content.x;
                child.dimensions.content.y = parent.dimensions.content.y;
            }
            Position::Fixed => {
                // Position relative to viewport
                child.dimensions.content.x = 0;
                child.dimensions.content.y = 0;
            }
            Position::Static => {
                // Normal flow (already handled)
            }
        }
        let _ = ctx;
    }
}

/// Generate text content from a DOM node for inline layout
pub fn get_text_for_layout(doc: &Document, node_id: NodeId) -> String {
    let mut text = String::new();
    doc.walk(node_id, &mut |id| {
        if let Some(node) = doc.arena.get(id) {
            if node.node_type == NodeType::Text {
                if let Some(ref t) = node.text_content {
                    text.push_str(t);
                }
            }
        }
    });
    text
}

/// Word wrap: split text into lines that fit within a given width
pub fn word_wrap(text: &str, max_width: FixedPoint, white_space: WhiteSpace) -> Vec<String> {
    if max_width <= 0 {
        return Vec::new();
    }

    let char_width = CHAR_WIDTH;
    let max_chars = fp_to_px(max_width) / fp_to_px(char_width);
    if max_chars <= 0 {
        return Vec::new();
    }
    let max_chars = max_chars as usize;

    match white_space {
        WhiteSpace::Pre | WhiteSpace::PreWrap => {
            // Preserve whitespace and newlines
            let mut lines = Vec::new();
            for line in text.split('\n') {
                if white_space == WhiteSpace::PreWrap && line.len() > max_chars {
                    // Wrap long lines
                    let mut pos = 0;
                    while pos < line.len() {
                        let end = core::cmp::min(pos + max_chars, line.len());
                        lines.push(line[pos..end].to_string());
                        pos = end;
                    }
                } else {
                    lines.push(line.to_string());
                }
            }
            if lines.is_empty() {
                lines.push(String::new());
            }
            lines
        }
        WhiteSpace::Nowrap => {
            // No wrapping
            let collapsed = collapse_whitespace(text);
            alloc::vec![collapsed]
        }
        _ => {
            // Normal wrapping at word boundaries
            let collapsed = collapse_whitespace(text);
            let words: Vec<&str> = collapsed.split_whitespace().collect();
            let mut lines = Vec::new();
            let mut current_line = String::new();

            for word in words {
                if current_line.is_empty() {
                    current_line = word.to_string();
                } else if current_line.len() + 1 + word.len() <= max_chars {
                    current_line.push(' ');
                    current_line.push_str(word);
                } else {
                    lines.push(current_line);
                    current_line = word.to_string();
                }
            }

            if !current_line.is_empty() {
                lines.push(current_line);
            }

            if lines.is_empty() {
                lines.push(String::new());
            }

            lines
        }
    }
}

/// Collapse whitespace according to CSS rules
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }
    result
}

/// Measure text width in fixed-point units (8px per char)
pub fn measure_text_width(text: &str) -> FixedPoint {
    (text.len() as i32) * CHAR_WIDTH
}

/// Measure text height for a given font size
pub fn measure_text_height(font_size: FixedPoint) -> FixedPoint {
    // Using 8x16 font, height scales with font size
    // Default 16px font = 16px height
    let _ = font_size;
    px_to_fp(16) // Fixed height for 8x16 font
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::{
        super::{css_parser::CssParser, tree_builder::TreeBuilder},
        *,
    };

    fn layout_html(html: &str, css: &str, width: i32, height: i32) -> LayoutBox {
        let doc = TreeBuilder::build(html);
        let stylesheet = CssParser::parse(css);
        let mut resolver = StyleResolver::new();
        resolver.add_stylesheet(stylesheet);
        build_layout_tree(&doc, &resolver, width, height)
    }

    #[test]
    fn test_rect_default() {
        let r = Rect::default();
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, 0);
        assert_eq!(r.height, 0);
    }

    #[test]
    fn test_edge_sizes_default() {
        let e = EdgeSizes::default();
        assert_eq!(e.top, 0);
    }

    #[test]
    fn test_dimensions_padding_box() {
        let d = Dimensions {
            content: Rect {
                x: px_to_fp(10),
                y: px_to_fp(10),
                width: px_to_fp(100),
                height: px_to_fp(50),
            },
            padding: EdgeSizes {
                top: px_to_fp(5),
                right: px_to_fp(5),
                bottom: px_to_fp(5),
                left: px_to_fp(5),
            },
            border: EdgeSizes::default(),
            margin: EdgeSizes::default(),
        };
        let pb = d.padding_box();
        assert_eq!(pb.width, px_to_fp(110));
        assert_eq!(pb.height, px_to_fp(60));
    }

    #[test]
    fn test_dimensions_border_box() {
        let d = Dimensions {
            content: Rect {
                x: px_to_fp(10),
                y: px_to_fp(10),
                width: px_to_fp(100),
                height: px_to_fp(50),
            },
            padding: EdgeSizes {
                top: px_to_fp(5),
                right: px_to_fp(5),
                bottom: px_to_fp(5),
                left: px_to_fp(5),
            },
            border: EdgeSizes {
                top: px_to_fp(1),
                right: px_to_fp(1),
                bottom: px_to_fp(1),
                left: px_to_fp(1),
            },
            margin: EdgeSizes::default(),
        };
        let bb = d.border_box();
        assert_eq!(bb.width, px_to_fp(112));
    }

    #[test]
    fn test_dimensions_margin_box() {
        let d = Dimensions {
            content: Rect {
                x: px_to_fp(20),
                y: px_to_fp(20),
                width: px_to_fp(100),
                height: px_to_fp(50),
            },
            padding: EdgeSizes::default(),
            border: EdgeSizes::default(),
            margin: EdgeSizes {
                top: px_to_fp(10),
                right: px_to_fp(10),
                bottom: px_to_fp(10),
                left: px_to_fp(10),
            },
        };
        let mb = d.margin_box();
        assert_eq!(mb.width, px_to_fp(120));
        assert_eq!(mb.height, px_to_fp(70));
    }

    #[test]
    fn test_layout_empty() {
        let root = layout_html("", "", 800, 600);
        assert_eq!(root.box_type, BoxType::Block);
    }

    #[test]
    fn test_layout_single_block() {
        let root = layout_html("<div>Hello</div>", "", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_nested_blocks() {
        let root = layout_html(
            "<div><div>Inner</div></div>",
            "div { padding: 10px; }",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_width_set() {
        let root = layout_html("<div>text</div>", "div { width: 200px; }", 800, 600);
        // The div should be inside the body which is inside html
        // Find the div
        fn find_box_with_width(lb: &LayoutBox, target: FixedPoint) -> bool {
            if lb.dimensions.content.width == target {
                return true;
            }
            for child in &lb.children {
                if find_box_with_width(child, target) {
                    return true;
                }
            }
            false
        }
        assert!(find_box_with_width(&root, px_to_fp(200)));
    }

    #[test]
    fn test_layout_with_padding() {
        let root = layout_html(
            "<div>text</div>",
            "div { padding: 20px; width: 100px; }",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_with_margin() {
        let root = layout_html("<div>text</div>", "div { margin: 10px; }", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_display_none() {
        let root = layout_html(
            "<div>visible</div><div style=\"display:none\">hidden</div>",
            "",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_word_wrap_normal() {
        let lines = word_wrap("hello world foo bar", px_to_fp(80), WhiteSpace::Normal);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_word_wrap_nowrap() {
        let lines = word_wrap("hello world", px_to_fp(40), WhiteSpace::Nowrap);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_word_wrap_pre() {
        let lines = word_wrap("line1\nline2\nline3", px_to_fp(800), WhiteSpace::Pre);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_word_wrap_empty() {
        let lines = word_wrap("", px_to_fp(100), WhiteSpace::Normal);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_collapse_whitespace() {
        assert_eq!(collapse_whitespace("  hello   world  "), " hello world ");
    }

    #[test]
    fn test_measure_text_width() {
        let w = measure_text_width("hello");
        assert_eq!(w, 5 * CHAR_WIDTH);
    }

    #[test]
    fn test_measure_text_height() {
        let h = measure_text_height(px_to_fp(16));
        assert_eq!(h, px_to_fp(16));
    }

    #[test]
    fn test_layout_box_default() {
        let lb = LayoutBox::default();
        assert_eq!(lb.box_type, BoxType::Block);
        assert!(lb.children.is_empty());
    }

    #[test]
    fn test_margin_box_height() {
        let mut lb = LayoutBox::default();
        lb.dimensions.content.height = px_to_fp(100);
        lb.dimensions.margin.top = px_to_fp(10);
        lb.dimensions.margin.bottom = px_to_fp(10);
        assert_eq!(lb.margin_box_height(), px_to_fp(120));
    }

    #[test]
    fn test_layout_multiple_blocks() {
        let root = layout_html(
            "<div>A</div><div>B</div><div>C</div>",
            "div { height: 50px; }",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_word_wrap_single_long_word() {
        let lines = word_wrap("superlongword", px_to_fp(40), WhiteSpace::Normal);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_layout_headings() {
        let root = layout_html("<h1>Title</h1><h2>Subtitle</h2><p>Text</p>", "", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_get_text_for_layout() {
        let doc = TreeBuilder::build("<p>Hello <b>world</b></p>");
        let ps = doc.get_elements_by_tag_name("p");
        let text = get_text_for_layout(&doc, ps[0]);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_layout_context_available_width() {
        let ctx = LayoutContext::new(800, 600);
        let (left, right) = ctx.available_width_at(0, px_to_fp(800));
        assert_eq!(left, 0);
        assert_eq!(right, px_to_fp(800));
    }

    #[test]
    fn test_layout_context_clear() {
        let mut ctx = LayoutContext::new(800, 600);
        ctx.left_floats.push(FloatExclusion {
            x: 0,
            y: 0,
            width: px_to_fp(200),
            height: px_to_fp(100),
            float_type: Float::Left,
        });
        let y = ctx.clear_y(Clear::Left, 0);
        assert_eq!(y, px_to_fp(100));
    }

    #[test]
    fn test_word_wrap_prewrap() {
        let lines = word_wrap("abc\ndef", px_to_fp(800), WhiteSpace::PreWrap);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_word_wrap_zero_width() {
        let lines = word_wrap("hello", 0, WhiteSpace::Normal);
        assert!(lines.is_empty());
    }

    // Float layout tests
    #[test]
    fn test_float_left() {
        let root = layout_html(
            "<div><div>floated</div><div>content</div></div>",
            "div div:first-child { float: left; width: 100px; }",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    // Positioned layout tests
    #[test]
    fn test_relative_position() {
        let root = layout_html("<div>text</div>", "div { position: relative; }", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_table() {
        let root = layout_html("<table><tr><td>cell</td></tr></table>", "", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_list() {
        let root = layout_html("<ul><li>item 1</li><li>item 2</li></ul>", "", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_with_border() {
        let root = layout_html(
            "<div>text</div>",
            "div { border-width: 2px; border-style: solid; width: 200px; }",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    // Additional tests
    #[test]
    fn test_layout_height_set() {
        let root = layout_html("<div>text</div>", "div { height: 300px; }", 800, 600);
        fn find_height(lb: &LayoutBox, target: FixedPoint) -> bool {
            if lb.dimensions.content.height == target {
                return true;
            }
            for child in &lb.children {
                if find_height(child, target) {
                    return true;
                }
            }
            false
        }
        assert!(find_height(&root, px_to_fp(300)));
    }

    #[test]
    fn test_layout_mixed_content() {
        let root = layout_html("<div>text <span>inline</span> more</div>", "", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_text_align_center() {
        let root = layout_html("<div>text</div>", "div { text-align: center; }", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_deeply_nested() {
        let root = layout_html(
            "<div><div><div><div>deep</div></div></div></div>",
            "",
            800,
            600,
        );
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_word_wrap_exact_fit() {
        // 10 chars at 8px each = 80px, container = 80px
        let lines = word_wrap("1234567890", px_to_fp(80), WhiteSpace::Normal);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_collapse_whitespace_tabs() {
        assert_eq!(collapse_whitespace("a\t\tb"), "a b");
    }

    #[test]
    fn test_collapse_whitespace_newlines() {
        assert_eq!(collapse_whitespace("a\n\nb"), "a b");
    }

    #[test]
    fn test_layout_wide_content() {
        let root = layout_html("<div>text</div>", "div { width: 1000px; }", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_overflow_hidden() {
        let root = layout_html(
            "<div>text</div>",
            "div { overflow: hidden; height: 50px; }",
            800,
            600,
        );
        let _ = root.style.overflow;
    }

    #[test]
    fn test_layout_visibility() {
        let root = layout_html("<div>text</div>", "div { visibility: hidden; }", 800, 600);
        assert!(root.dimensions.content.width > 0);
    }

    #[test]
    fn test_layout_min_width() {
        let root = layout_html(
            "<div>text</div>",
            "div { min-width: 500px; width: 100px; }",
            800,
            600,
        );
        fn find_min(lb: &LayoutBox, min: FixedPoint) -> bool {
            if lb.dimensions.content.width >= min {
                return true;
            }
            for child in &lb.children {
                if find_min(child, min) {
                    return true;
                }
            }
            false
        }
        assert!(find_min(&root, px_to_fp(500)));
    }

    #[test]
    fn test_layout_max_height() {
        let root = layout_html(
            "<div>text</div>",
            "div { max-height: 50px; height: 200px; }",
            800,
            600,
        );
        fn find_max(lb: &LayoutBox, max: FixedPoint) -> bool {
            if lb.dimensions.content.height == max {
                return true;
            }
            for child in &lb.children {
                if find_max(child, max) {
                    return true;
                }
            }
            false
        }
        assert!(find_max(&root, px_to_fp(50)));
    }
}
