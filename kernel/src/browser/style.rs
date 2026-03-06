//! Style Resolution
//!
//! Resolves computed styles for DOM nodes by cascading CSS rules,
//! sorting by specificity, and inheriting inheritable properties
//! from parent nodes. Includes user-agent default styles.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

use super::{
    css_parser::{
        named_color, px_to_fp, CssValue, Declaration, FixedPoint, Selector, SimpleSelector,
        Specificity, Stylesheet,
    },
    dom::{Document, NodeId, NodeType},
};

/// CSS display property
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Display {
    Block,
    #[default]
    Inline,
    InlineBlock,
    None,
    Flex,
    Table,
    TableRow,
    TableCell,
    TableHeaderGroup,
    TableRowGroup,
    TableFooterGroup,
    ListItem,
}

/// CSS position property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
}

/// CSS float property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

/// CSS clear property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

/// CSS text-align property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// CSS overflow property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
}

/// CSS visibility property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

/// CSS border style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

/// CSS white-space property
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
}

/// Computed style for a DOM node (all values resolved to final values)
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub position: Position,
    pub width: Option<FixedPoint>,
    pub height: Option<FixedPoint>,
    pub min_width: FixedPoint,
    pub min_height: FixedPoint,
    pub max_width: Option<FixedPoint>,
    pub max_height: Option<FixedPoint>,
    pub margin_top: FixedPoint,
    pub margin_right: FixedPoint,
    pub margin_bottom: FixedPoint,
    pub margin_left: FixedPoint,
    pub padding_top: FixedPoint,
    pub padding_right: FixedPoint,
    pub padding_bottom: FixedPoint,
    pub padding_left: FixedPoint,
    pub color: u32,
    pub background_color: u32,
    pub font_size: FixedPoint,
    pub font_weight: u16,
    pub text_align: TextAlign,
    pub line_height: FixedPoint,
    pub border_top_width: FixedPoint,
    pub border_right_width: FixedPoint,
    pub border_bottom_width: FixedPoint,
    pub border_left_width: FixedPoint,
    pub border_top_color: u32,
    pub border_right_color: u32,
    pub border_bottom_color: u32,
    pub border_left_color: u32,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub overflow: Overflow,
    pub visibility: Visibility,
    pub opacity: u8,
    pub float: Float,
    pub clear: Clear,
    pub z_index: i32,
    pub text_decoration_underline: bool,
    pub text_decoration_line_through: bool,
    pub white_space: WhiteSpace,
    pub list_style_type: Option<String>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            width: None,
            height: None,
            min_width: 0,
            min_height: 0,
            max_width: None,
            max_height: None,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            margin_left: 0,
            padding_top: 0,
            padding_right: 0,
            padding_bottom: 0,
            padding_left: 0,
            color: 0xFF000000,            // black
            background_color: 0x00000000, // transparent
            font_size: px_to_fp(16),      // 16px default
            font_weight: 400,             // normal
            text_align: TextAlign::Left,
            line_height: px_to_fp(20), // ~1.25 * 16px
            border_top_width: 0,
            border_right_width: 0,
            border_bottom_width: 0,
            border_left_width: 0,
            border_top_color: 0xFF000000,
            border_right_color: 0xFF000000,
            border_bottom_color: 0xFF000000,
            border_left_color: 0xFF000000,
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,
            overflow: Overflow::Visible,
            visibility: Visibility::Visible,
            opacity: 255,
            float: Float::None,
            clear: Clear::None,
            z_index: 0,
            text_decoration_underline: false,
            text_decoration_line_through: false,
            white_space: WhiteSpace::Normal,
            list_style_type: None,
        }
    }
}

/// A matched rule with its specificity for sorting
#[derive(Debug)]
struct MatchedRule {
    specificity: Specificity,
    declarations: Vec<Declaration>,
    important: bool,
}

/// Style resolver that applies CSS rules to DOM nodes
pub struct StyleResolver {
    pub stylesheets: Vec<Stylesheet>,
    pub ua_stylesheet: Stylesheet,
}

impl Default for StyleResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl StyleResolver {
    /// Create a new style resolver with user-agent defaults
    pub fn new() -> Self {
        Self {
            stylesheets: Vec::new(),
            ua_stylesheet: Self::default_ua_stylesheet(),
        }
    }

    /// Add a stylesheet
    pub fn add_stylesheet(&mut self, stylesheet: Stylesheet) {
        self.stylesheets.push(stylesheet);
    }

    /// Generate user-agent default stylesheet
    fn default_ua_stylesheet() -> Stylesheet {
        use super::css_parser::CssParser;
        CssParser::parse(
            "html, body, div, section, article, aside, nav, header, footer, main, address, \
             blockquote, figure, figcaption, details, summary, ul, ol, dl, pre, fieldset, form, \
             hgroup, p, h1, h2, h3, h4, h5, h6 { display: block; }
             li { display: list-item; }
             table { display: table; }
             thead { display: table-header-group; }
             tbody { display: table-row-group; }
             tfoot { display: table-footer-group; }
             tr { display: table-row; }
             td, th { display: table-cell; }
             head, script, style, link, meta, title { display: none; }
             b, strong { font-weight: bold; }
             i, em { font-style: italic; }
             h1 { font-size: 32px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
             h2 { font-size: 24px; font-weight: bold; margin-top: 19px; margin-bottom: 19px; }
             h3 { font-size: 18px; font-weight: bold; margin-top: 18px; margin-bottom: 18px; }
             h4 { font-size: 16px; font-weight: bold; margin-top: 21px; margin-bottom: 21px; }
             h5 { font-size: 13px; font-weight: bold; margin-top: 22px; margin-bottom: 22px; }
             h6 { font-size: 10px; font-weight: bold; margin-top: 24px; margin-bottom: 24px; }
             p { margin-top: 16px; margin-bottom: 16px; }
             body { margin-top: 8px; margin-right: 8px; margin-bottom: 8px; margin-left: 8px; }
             ul, ol { margin-top: 16px; margin-bottom: 16px; padding-left: 40px; }
             a { color: #0000ee; }
             u { color: inherit; }
             pre, code { font-family: monospace; }
            ",
        )
    }

    /// Resolve the computed style for a node
    pub fn resolve(
        &self,
        doc: &Document,
        node_id: NodeId,
        parent_style: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let node = match doc.arena.get(node_id) {
            Some(n) => n,
            None => return ComputedStyle::default(),
        };

        // Non-element nodes get default or inherited style
        if node.node_type != NodeType::Element {
            let mut style = ComputedStyle::default();
            if let Some(ps) = parent_style {
                Self::inherit(&mut style, ps);
            }
            return style;
        }

        // Start with defaults
        let mut style = ComputedStyle::default();

        // Cascade: collect matching rules sorted by specificity
        let matched = self.cascade(doc, node_id);

        // Apply declarations in order of specificity
        for decl in &matched {
            Self::apply_declaration(&mut style, decl);
        }

        // Inherit from parent
        if let Some(ps) = parent_style {
            Self::inherit_if_not_set(&mut style, ps);
        }

        style
    }

    /// Collect all matching declarations, sorted by specificity
    fn cascade(&self, doc: &Document, node_id: NodeId) -> Vec<Declaration> {
        let mut matched_rules: Vec<MatchedRule> = Vec::new();

        // UA stylesheet
        self.collect_matching_rules(&self.ua_stylesheet, doc, node_id, &mut matched_rules);

        // Author stylesheets
        for ss in &self.stylesheets {
            self.collect_matching_rules(ss, doc, node_id, &mut matched_rules);
        }

        // Sort by specificity (stable sort preserves source order)
        matched_rules.sort_by(|a, b| a.specificity.cmp(&b.specificity));

        // Flatten declarations, !important last
        let mut normal = Vec::new();
        let mut important = Vec::new();
        for rule in matched_rules {
            for decl in rule.declarations {
                if decl.important {
                    important.push(decl);
                } else {
                    normal.push(decl);
                }
            }
        }
        normal.extend(important);
        normal
    }

    fn collect_matching_rules(
        &self,
        stylesheet: &Stylesheet,
        doc: &Document,
        node_id: NodeId,
        matched: &mut Vec<MatchedRule>,
    ) {
        for rule in &stylesheet.rules {
            for selector in &rule.selectors {
                if Self::selector_matches(selector, doc, node_id) {
                    matched.push(MatchedRule {
                        specificity: selector.specificity(),
                        declarations: rule.declarations.clone(),
                        important: false,
                    });
                    break; // Only match first selector in the group
                }
            }
        }
    }

    /// Check if a selector matches a node
    fn selector_matches(selector: &Selector, doc: &Document, node_id: NodeId) -> bool {
        match selector {
            Selector::Universal => true,
            Selector::Tag(tag) => doc.tag_name(node_id) == Some(tag.as_str()),
            Selector::Id(id) => doc.get_attribute(node_id, "id").as_deref() == Some(id.as_str()),
            Selector::Class(class) => doc
                .arena
                .get(node_id)
                .and_then(|n| n.element_data.as_ref())
                .map(|ed| ed.has_class(class))
                .unwrap_or(false),
            Selector::Simple(simple) => Self::simple_selector_matches(simple, doc, node_id),
            Selector::Descendant(parts) => Self::descendant_matches(parts, doc, node_id),
            Selector::Child(parts) => Self::child_matches(parts, doc, node_id),
            Selector::Compound(parts) => parts
                .iter()
                .all(|p| Self::selector_matches(p, doc, node_id)),
        }
    }

    fn simple_selector_matches(sel: &SimpleSelector, doc: &Document, node_id: NodeId) -> bool {
        if let Some(ref tag) = sel.tag_name {
            if doc.tag_name(node_id) != Some(tag.as_str()) {
                return false;
            }
        }
        if let Some(ref id) = sel.id {
            if doc.get_attribute(node_id, "id").as_deref() != Some(id.as_str()) {
                return false;
            }
        }
        for class in &sel.classes {
            let has = doc
                .arena
                .get(node_id)
                .and_then(|n| n.element_data.as_ref())
                .map(|ed| ed.has_class(class))
                .unwrap_or(false);
            if !has {
                return false;
            }
        }
        true
    }

    fn descendant_matches(parts: &[Selector], doc: &Document, node_id: NodeId) -> bool {
        if parts.is_empty() {
            return false;
        }

        // Last part must match current node
        if !Self::selector_matches(&parts[parts.len() - 1], doc, node_id) {
            return false;
        }

        if parts.len() == 1 {
            return true;
        }

        // Walk ancestors to match remaining parts
        let remaining = &parts[..parts.len() - 1];
        let ancestors = doc.ancestors(node_id);
        for &ancestor_id in &ancestors {
            if Self::descendant_matches(remaining, doc, ancestor_id) {
                return true;
            }
        }
        false
    }

    fn child_matches(parts: &[Selector], doc: &Document, node_id: NodeId) -> bool {
        if parts.is_empty() {
            return false;
        }

        if !Self::selector_matches(&parts[parts.len() - 1], doc, node_id) {
            return false;
        }

        if parts.len() == 1 {
            return true;
        }

        // Direct parent must match
        if let Some(node) = doc.arena.get(node_id) {
            if let Some(parent_id) = node.parent {
                return Self::child_matches(&parts[..parts.len() - 1], doc, parent_id);
            }
        }
        false
    }

    /// Apply a CSS declaration to a computed style
    fn apply_declaration(style: &mut ComputedStyle, decl: &Declaration) {
        match decl.property.as_str() {
            "display" => {
                style.display = match &decl.value {
                    CssValue::Keyword(k) => match k.as_str() {
                        "block" => Display::Block,
                        "inline" => Display::Inline,
                        "inline-block" => Display::InlineBlock,
                        "flex" => Display::Flex,
                        "table" => Display::Table,
                        "table-row" => Display::TableRow,
                        "table-cell" => Display::TableCell,
                        "table-header-group" => Display::TableHeaderGroup,
                        "table-row-group" => Display::TableRowGroup,
                        "table-footer-group" => Display::TableFooterGroup,
                        "list-item" => Display::ListItem,
                        _ => Display::Block,
                    },
                    CssValue::None => Display::None,
                    _ => style.display,
                };
            }
            "position" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.position = match k.as_str() {
                        "static" => Position::Static,
                        "relative" => Position::Relative,
                        "absolute" => Position::Absolute,
                        "fixed" => Position::Fixed,
                        _ => style.position,
                    };
                }
            }
            "width" => {
                style.width = Self::resolve_length(&decl.value);
            }
            "height" => {
                style.height = Self::resolve_length(&decl.value);
            }
            "min-width" => {
                if let Some(v) = Self::resolve_length(&decl.value) {
                    style.min_width = v;
                }
            }
            "min-height" => {
                if let Some(v) = Self::resolve_length(&decl.value) {
                    style.min_height = v;
                }
            }
            "max-width" => {
                style.max_width = Self::resolve_length(&decl.value);
            }
            "max-height" => {
                style.max_height = Self::resolve_length(&decl.value);
            }
            "margin" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.margin_top = v;
                    style.margin_right = v;
                    style.margin_bottom = v;
                    style.margin_left = v;
                }
            }
            "margin-top" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.margin_top = v;
                }
            }
            "margin-right" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.margin_right = v;
                }
            }
            "margin-bottom" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.margin_bottom = v;
                }
            }
            "margin-left" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.margin_left = v;
                }
            }
            "padding" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.padding_top = v;
                    style.padding_right = v;
                    style.padding_bottom = v;
                    style.padding_left = v;
                }
            }
            "padding-top" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.padding_top = v;
                }
            }
            "padding-right" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.padding_right = v;
                }
            }
            "padding-bottom" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.padding_bottom = v;
                }
            }
            "padding-left" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.padding_left = v;
                }
            }
            "color" => {
                if let Some(c) = Self::resolve_color(&decl.value) {
                    style.color = c;
                }
            }
            "background-color" | "background" => {
                if let Some(c) = Self::resolve_color(&decl.value) {
                    style.background_color = c;
                }
            }
            "font-size" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.font_size = v;
                } else if let CssValue::Keyword(k) = &decl.value {
                    style.font_size = match k.as_str() {
                        "xx-small" => px_to_fp(9),
                        "x-small" => px_to_fp(10),
                        "small" => px_to_fp(13),
                        "medium" => px_to_fp(16),
                        "large" => px_to_fp(18),
                        "x-large" => px_to_fp(24),
                        "xx-large" => px_to_fp(32),
                        _ => style.font_size,
                    };
                }
            }
            "font-weight" => match &decl.value {
                CssValue::Keyword(k) => {
                    style.font_weight = match k.as_str() {
                        "normal" => 400,
                        "bold" => 700,
                        "lighter" => 100,
                        "bolder" => 900,
                        _ => style.font_weight,
                    };
                }
                CssValue::Number(n) => {
                    style.font_weight = (*n as u16).clamp(100, 900);
                }
                _ => {}
            },
            "text-align" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.text_align = match k.as_str() {
                        "left" => TextAlign::Left,
                        "center" => TextAlign::Center,
                        "right" => TextAlign::Right,
                        "justify" => TextAlign::Justify,
                        _ => style.text_align,
                    };
                }
            }
            "line-height" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.line_height = v;
                }
            }
            "border-width" => {
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.border_top_width = v;
                    style.border_right_width = v;
                    style.border_bottom_width = v;
                    style.border_left_width = v;
                }
            }
            "border-color" => {
                if let Some(c) = Self::resolve_color(&decl.value) {
                    style.border_top_color = c;
                    style.border_right_color = c;
                    style.border_bottom_color = c;
                    style.border_left_color = c;
                }
            }
            "border-style" => {
                if let CssValue::Keyword(k) = &decl.value {
                    let bs = Self::parse_border_style(k);
                    style.border_top_style = bs;
                    style.border_right_style = bs;
                    style.border_bottom_style = bs;
                    style.border_left_style = bs;
                }
            }
            "border" => {
                // Shorthand: width style color
                // Simplified: just handle color or width
                if let Some(c) = Self::resolve_color(&decl.value) {
                    style.border_top_color = c;
                    style.border_right_color = c;
                    style.border_bottom_color = c;
                    style.border_left_color = c;
                }
                if let Some(v) = Self::resolve_length_or_zero(&decl.value) {
                    style.border_top_width = v;
                    style.border_right_width = v;
                    style.border_bottom_width = v;
                    style.border_left_width = v;
                }
            }
            "overflow" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.overflow = match k.as_str() {
                        "hidden" => Overflow::Hidden,
                        "scroll" => Overflow::Scroll,
                        "auto" => Overflow::Auto,
                        _ => Overflow::Visible,
                    };
                }
            }
            "visibility" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.visibility = match k.as_str() {
                        "hidden" => Visibility::Hidden,
                        "collapse" => Visibility::Collapse,
                        _ => Visibility::Visible,
                    };
                }
            }
            "opacity" => {
                if let CssValue::Number(n) = &decl.value {
                    style.opacity = *n as u8;
                }
            }
            "float" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.float = match k.as_str() {
                        "left" => Float::Left,
                        "right" => Float::Right,
                        _ => Float::None,
                    };
                }
            }
            "clear" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.clear = match k.as_str() {
                        "left" => Clear::Left,
                        "right" => Clear::Right,
                        "both" => Clear::Both,
                        _ => Clear::None,
                    };
                }
            }
            "z-index" => {
                if let CssValue::Number(n) = &decl.value {
                    style.z_index = *n;
                }
            }
            "text-decoration" => {
                if let CssValue::Keyword(k) = &decl.value {
                    match k.as_str() {
                        "underline" => style.text_decoration_underline = true,
                        "line-through" => style.text_decoration_line_through = true,
                        "none" => {
                            style.text_decoration_underline = false;
                            style.text_decoration_line_through = false;
                        }
                        _ => {}
                    }
                }
            }
            "white-space" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.white_space = match k.as_str() {
                        "nowrap" => WhiteSpace::Nowrap,
                        "pre" => WhiteSpace::Pre,
                        "pre-wrap" => WhiteSpace::PreWrap,
                        "pre-line" => WhiteSpace::PreLine,
                        _ => WhiteSpace::Normal,
                    };
                }
            }
            "list-style-type" => {
                if let CssValue::Keyword(k) = &decl.value {
                    style.list_style_type = Some(k.clone());
                }
            }
            // font-style handled as keyword passthrough (italic is a text property)
            "font-style" | "font-family" => {
                // Ignored for layout purposes
            }
            _ => {
                // Unknown property, ignore
            }
        }
    }

    fn resolve_length(value: &CssValue) -> Option<FixedPoint> {
        match value {
            CssValue::Length(v, _) => Some(*v),
            CssValue::Number(0) => Some(0),
            CssValue::Auto => None,
            CssValue::None => None,
            CssValue::Percentage(v) => Some(*v), // Percentage stored as fixed-point
            _ => None,
        }
    }

    fn resolve_length_or_zero(value: &CssValue) -> Option<FixedPoint> {
        match value {
            CssValue::Length(v, _) => Some(*v),
            CssValue::Number(n) => Some(px_to_fp(*n)),
            CssValue::Percentage(v) => Some(*v),
            _ => None,
        }
    }

    fn resolve_color(value: &CssValue) -> Option<u32> {
        match value {
            CssValue::Color(c) => Some(*c),
            CssValue::Keyword(k) => named_color(k),
            _ => None,
        }
    }

    fn parse_border_style(s: &str) -> BorderStyle {
        match s {
            "solid" => BorderStyle::Solid,
            "dashed" => BorderStyle::Dashed,
            "dotted" => BorderStyle::Dotted,
            "double" => BorderStyle::Double,
            "groove" => BorderStyle::Groove,
            "ridge" => BorderStyle::Ridge,
            "inset" => BorderStyle::Inset,
            "outset" => BorderStyle::Outset,
            "none" => BorderStyle::None,
            _ => BorderStyle::None,
        }
    }

    /// Inherit inheritable properties from parent (always applied)
    fn inherit(style: &mut ComputedStyle, parent: &ComputedStyle) {
        style.color = parent.color;
        style.font_size = parent.font_size;
        style.font_weight = parent.font_weight;
        style.line_height = parent.line_height;
        style.text_align = parent.text_align;
        style.visibility = parent.visibility;
        style.white_space = parent.white_space;
        style.list_style_type = parent.list_style_type.clone();
    }

    /// Inherit only if the property was not explicitly set (for cascade)
    fn inherit_if_not_set(style: &mut ComputedStyle, parent: &ComputedStyle) {
        // Color inherits unless explicitly set
        // We treat 0xFF000000 (black) as the default
        if style.color == 0xFF000000 && parent.color != 0xFF000000 {
            style.color = parent.color;
        }
        // font-size: only inherit if still at default
        if style.font_size == px_to_fp(16) && parent.font_size != px_to_fp(16) {
            style.font_size = parent.font_size;
        }
        // line-height
        if style.line_height == px_to_fp(20) && parent.line_height != px_to_fp(20) {
            style.line_height = parent.line_height;
        }
        // text-align
        if style.text_align == TextAlign::Left && parent.text_align != TextAlign::Left {
            style.text_align = parent.text_align;
        }
        // visibility
        if style.visibility == Visibility::Visible && parent.visibility != Visibility::Visible {
            style.visibility = parent.visibility;
        }
        // white-space
        if style.white_space == WhiteSpace::Normal && parent.white_space != WhiteSpace::Normal {
            style.white_space = parent.white_space;
        }
        // list-style-type
        if style.list_style_type.is_none() {
            style.list_style_type = parent.list_style_type.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::{
        super::{css_parser::CssParser, tree_builder::TreeBuilder},
        *,
    };

    fn resolve_first_p(html: &str, css: &str) -> ComputedStyle {
        let doc = TreeBuilder::build(html);
        let stylesheet = CssParser::parse(css);
        let mut resolver = StyleResolver::new();
        resolver.add_stylesheet(stylesheet);
        let ps = doc.get_elements_by_tag_name("p");
        assert!(!ps.is_empty(), "No <p> found");
        resolver.resolve(&doc, ps[0], None)
    }

    #[test]
    fn test_default_style() {
        let style = ComputedStyle::default();
        assert_eq!(style.display, Display::Inline);
        assert_eq!(style.color, 0xFF000000);
        assert_eq!(style.font_size, px_to_fp(16));
    }

    #[test]
    fn test_ua_block_display() {
        let doc = TreeBuilder::build("<div>hello</div>");
        let resolver = StyleResolver::new();
        let divs = doc.get_elements_by_tag_name("div");
        let style = resolver.resolve(&doc, divs[0], None);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn test_ua_heading_font_size() {
        let doc = TreeBuilder::build("<h1>Title</h1>");
        let resolver = StyleResolver::new();
        let h1s = doc.get_elements_by_tag_name("h1");
        let style = resolver.resolve(&doc, h1s[0], None);
        assert_eq!(style.font_size, px_to_fp(32));
        assert_eq!(style.font_weight, 700);
    }

    #[test]
    fn test_ua_body_margin() {
        let doc = TreeBuilder::build("<body></body>");
        let resolver = StyleResolver::new();
        let bodies = doc.get_elements_by_tag_name("body");
        let style = resolver.resolve(&doc, bodies[0], None);
        assert_eq!(style.margin_top, px_to_fp(8));
    }

    #[test]
    fn test_color_override() {
        let style = resolve_first_p("<p>text</p>", "p { color: #ff0000; }");
        assert_eq!(style.color, 0xFFFF0000);
    }

    #[test]
    fn test_display_none() {
        let doc = TreeBuilder::build("<head><title>T</title></head>");
        let resolver = StyleResolver::new();
        let titles = doc.get_elements_by_tag_name("title");
        if !titles.is_empty() {
            let style = resolver.resolve(&doc, titles[0], None);
            assert_eq!(style.display, Display::None);
        }
    }

    #[test]
    fn test_background_color() {
        let style = resolve_first_p("<p>text</p>", "p { background-color: #00ff00; }");
        assert_eq!(style.background_color, 0xFF00FF00);
    }

    #[test]
    fn test_padding() {
        let style = resolve_first_p("<p>text</p>", "p { padding: 10px; }");
        assert_eq!(style.padding_top, px_to_fp(10));
        assert_eq!(style.padding_right, px_to_fp(10));
        assert_eq!(style.padding_bottom, px_to_fp(10));
        assert_eq!(style.padding_left, px_to_fp(10));
    }

    #[test]
    fn test_margin_individual() {
        let style = resolve_first_p("<p>text</p>", "p { margin-left: 20px; }");
        assert_eq!(style.margin_left, px_to_fp(20));
    }

    #[test]
    fn test_font_weight_bold() {
        let doc = TreeBuilder::build("<b>bold</b>");
        let resolver = StyleResolver::new();
        let bs = doc.get_elements_by_tag_name("b");
        let style = resolver.resolve(&doc, bs[0], None);
        assert_eq!(style.font_weight, 700);
    }

    #[test]
    fn test_width_height() {
        let style = resolve_first_p("<p>text</p>", "p { width: 200px; height: 100px; }");
        assert_eq!(style.width, Some(px_to_fp(200)));
        assert_eq!(style.height, Some(px_to_fp(100)));
    }

    #[test]
    fn test_text_align() {
        let style = resolve_first_p("<p>text</p>", "p { text-align: center; }");
        assert_eq!(style.text_align, TextAlign::Center);
    }

    #[test]
    fn test_float() {
        let style = resolve_first_p("<p>text</p>", "p { float: left; }");
        assert_eq!(style.float, Float::Left);
    }

    #[test]
    fn test_position() {
        let style = resolve_first_p("<p>text</p>", "p { position: absolute; }");
        assert_eq!(style.position, Position::Absolute);
    }

    #[test]
    fn test_border_style() {
        let style = resolve_first_p("<p>text</p>", "p { border-style: solid; }");
        assert_eq!(style.border_top_style, BorderStyle::Solid);
    }

    #[test]
    fn test_overflow() {
        let style = resolve_first_p("<p>text</p>", "p { overflow: hidden; }");
        assert_eq!(style.overflow, Overflow::Hidden);
    }

    #[test]
    fn test_z_index() {
        let style = resolve_first_p("<p>text</p>", "p { z-index: 10; }");
        assert_eq!(style.z_index, 10);
    }

    #[test]
    fn test_specificity_ordering() {
        let style = resolve_first_p(
            "<p id=\"x\">text</p>",
            "p { color: #ff0000; } #x { color: #0000ff; }",
        );
        // ID selector has higher specificity
        assert_eq!(style.color, 0xFF0000FF);
    }

    #[test]
    fn test_named_color_resolution() {
        let style = resolve_first_p("<p>text</p>", "p { background-color: red; }");
        assert_eq!(style.background_color, 0xFFFF0000);
    }

    #[test]
    fn test_class_selector_match() {
        let doc = TreeBuilder::build("<p class=\"highlight\">text</p>");
        let stylesheet = CssParser::parse(".highlight { color: #00ff00; }");
        let mut resolver = StyleResolver::new();
        resolver.add_stylesheet(stylesheet);
        let ps = doc.get_elements_by_tag_name("p");
        let style = resolver.resolve(&doc, ps[0], None);
        assert_eq!(style.color, 0xFF00FF00);
    }

    #[test]
    fn test_visibility_hidden() {
        let style = resolve_first_p("<p>text</p>", "p { visibility: hidden; }");
        assert_eq!(style.visibility, Visibility::Hidden);
    }

    #[test]
    fn test_white_space() {
        let style = resolve_first_p("<p>text</p>", "p { white-space: pre; }");
        assert_eq!(style.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn test_text_decoration() {
        let style = resolve_first_p("<p>text</p>", "p { text-decoration: underline; }");
        assert!(style.text_decoration_underline);
    }

    #[test]
    fn test_default_resolver() {
        let _r = StyleResolver::default();
    }
}
