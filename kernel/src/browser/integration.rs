//! Browser Integration
//!
//! End-to-end convenience functions and conformance tests that exercise
//! the full pipeline: HTML tokenization, DOM construction, CSS parsing,
//! style resolution, layout computation, and pixel painting.

#![allow(dead_code)]

use alloc::vec::Vec;

use super::{
    css_parser::CssParser,
    dom::Document,
    layout::{build_layout_tree, LayoutBox},
    paint::{build_display_list, DisplayList, Painter, PixelRect},
    style::StyleResolver,
    tree_builder::TreeBuilder,
};

/// Render an HTML string to a pixel buffer
///
/// Convenience function that runs the full pipeline:
/// tokenize -> build DOM -> parse CSS -> resolve styles -> layout -> paint
pub fn render_html(html: &str, width: i32, height: i32) -> Vec<u32> {
    render_html_with_css(html, "", width, height)
}

/// Render HTML with an external CSS string to a pixel buffer
pub fn render_html_with_css(html: &str, css: &str, width: i32, height: i32) -> Vec<u32> {
    let doc = TreeBuilder::build(html);
    let stylesheet = CssParser::parse(css);

    let mut resolver = StyleResolver::new();
    resolver.add_stylesheet(stylesheet);

    let layout = build_layout_tree(&doc, &resolver, width, height);
    let display_list = build_display_list(&layout, 0);

    let mut painter = Painter::new(width as usize, height as usize);
    painter.paint(&display_list);

    painter.pixels
}

/// Render HTML and return the layout tree (for inspection)
pub fn render_to_layout(html: &str, css: &str, width: i32, height: i32) -> LayoutBox {
    let doc = TreeBuilder::build(html);
    let stylesheet = CssParser::parse(css);

    let mut resolver = StyleResolver::new();
    resolver.add_stylesheet(stylesheet);

    build_layout_tree(&doc, &resolver, width, height)
}

/// Render HTML and return (layout, display_list) for inspection
pub fn render_to_display_list(
    html: &str,
    css: &str,
    width: i32,
    height: i32,
) -> (LayoutBox, DisplayList) {
    let doc = TreeBuilder::build(html);
    let stylesheet = CssParser::parse(css);

    let mut resolver = StyleResolver::new();
    resolver.add_stylesheet(stylesheet);

    let layout = build_layout_tree(&doc, &resolver, width, height);
    let display_list = build_display_list(&layout, 0);

    (layout, display_list)
}

/// Parse HTML and return the Document for inspection
pub fn parse_html(html: &str) -> Document {
    TreeBuilder::build(html)
}

/// Check if a pixel region contains a specific color
pub fn region_contains_color(pixels: &[u32], width: usize, rect: &PixelRect, color: u32) -> bool {
    for y in rect.y..(rect.y + rect.height) {
        for x in rect.x..(rect.x + rect.width) {
            if x >= 0 && y >= 0 {
                let idx = y as usize * width + x as usize;
                if idx < pixels.len() && pixels[idx] == color {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a pixel region is entirely one color
pub fn region_is_solid_color(pixels: &[u32], width: usize, rect: &PixelRect, color: u32) -> bool {
    for y in rect.y..(rect.y + rect.height) {
        for x in rect.x..(rect.x + rect.width) {
            if x >= 0 && y >= 0 {
                let idx = y as usize * width + x as usize;
                if idx < pixels.len() && pixels[idx] != color {
                    return false;
                }
            }
        }
    }
    true
}

/// Count the number of unique colors in a pixel region
pub fn count_unique_colors(pixels: &[u32], width: usize, rect: &PixelRect) -> usize {
    let mut colors: Vec<u32> = Vec::new();
    for y in rect.y..(rect.y + rect.height) {
        for x in rect.x..(rect.x + rect.width) {
            if x >= 0 && y >= 0 {
                let idx = y as usize * width + x as usize;
                if idx < pixels.len() && !colors.contains(&pixels[idx]) {
                    colors.push(pixels[idx]);
                }
            }
        }
    }
    colors.len()
}

/// Find a layout box with a specific tag (for test inspection)
pub fn find_layout_box_by_tag<'a>(
    layout: &'a LayoutBox,
    doc: &Document,
    tag: &str,
) -> Option<&'a LayoutBox> {
    if let Some(node_id) = layout.node_id {
        if doc.tag_name(node_id) == Some(tag) {
            return Some(layout);
        }
    }
    for child in &layout.children {
        if let Some(found) = find_layout_box_by_tag(child, doc, tag) {
            return Some(found);
        }
    }
    None
}

/// Count layout boxes in the tree
pub fn count_layout_boxes(layout: &LayoutBox) -> usize {
    let mut count = 1;
    for child in &layout.children {
        count += count_layout_boxes(child);
    }
    count
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::{string::String, vec};

    use super::*;

    #[test]
    fn test_render_empty() {
        let pixels = render_html("", 100, 100);
        assert_eq!(pixels.len(), 10000);
    }

    #[test]
    fn test_render_simple_paragraph() {
        let pixels = render_html("<p>Hello</p>", 200, 200);
        assert_eq!(pixels.len(), 40000);
    }

    #[test]
    fn test_render_with_css() {
        let pixels = render_html_with_css(
            "<div>Hello</div>",
            "div { background-color: #ff0000; width: 100px; height: 50px; }",
            200,
            200,
        );
        assert_eq!(pixels.len(), 40000);
        // Should contain some red pixels
        let has_red = pixels.iter().any(|&p| p == 0xFFFF0000);
        assert!(has_red, "Expected red pixels from background-color");
    }

    #[test]
    fn test_render_headings() {
        let pixels = render_html("<h1>Title</h1><h2>Subtitle</h2><p>Text</p>", 400, 300);
        assert_eq!(pixels.len(), 120000);
    }

    #[test]
    fn test_render_colored_text() {
        let pixels = render_html_with_css("<p>Red text</p>", "p { color: #ff0000; }", 200, 200);
        assert_eq!(pixels.len(), 40000);
    }

    #[test]
    fn test_render_nested_elements() {
        let pixels = render_html("<div><p>Nested <b>bold</b></p></div>", 200, 200);
        assert_eq!(pixels.len(), 40000);
    }

    #[test]
    fn test_render_links() {
        let doc = parse_html("<a href=\"/page\">Click me</a>");
        let anchors = doc.get_elements_by_tag_name("a");
        assert_eq!(anchors.len(), 1);
        assert_eq!(doc.inner_text(anchors[0]), "Click me");
    }

    #[test]
    fn test_render_to_layout() {
        let layout = render_to_layout("<div><p>Hello</p></div>", "", 800, 600);
        assert!(layout.dimensions.content.width > 0);
    }

    #[test]
    fn test_render_to_display_list() {
        let (layout, display_list) = render_to_display_list(
            "<div>Hello</div>",
            "div { background-color: red; height: 50px; }",
            800,
            600,
        );
        assert!(layout.dimensions.content.width > 0);
        assert!(!display_list.commands.is_empty());
    }

    #[test]
    fn test_parse_html_returns_document() {
        let doc = parse_html("<html><body><p>Test</p></body></html>");
        assert!(doc.node_count() > 1);
    }

    #[test]
    fn test_region_contains_color() {
        let mut pixels = alloc::vec![0xFFFFFFFF; 100];
        pixels[55] = 0xFFFF0000; // Set one pixel red
        let rect = PixelRect::new(0, 0, 10, 10);
        assert!(region_contains_color(&pixels, 10, &rect, 0xFFFF0000));
    }

    #[test]
    fn test_region_solid_color() {
        let pixels = alloc::vec![0xFFFFFFFF; 100];
        let rect = PixelRect::new(0, 0, 10, 10);
        assert!(region_is_solid_color(&pixels, 10, &rect, 0xFFFFFFFF));
    }

    #[test]
    fn test_region_not_solid() {
        let mut pixels = alloc::vec![0xFFFFFFFF; 100];
        pixels[0] = 0xFF000000;
        let rect = PixelRect::new(0, 0, 10, 10);
        assert!(!region_is_solid_color(&pixels, 10, &rect, 0xFFFFFFFF));
    }

    #[test]
    fn test_count_unique_colors() {
        let mut pixels = alloc::vec![0xFFFFFFFF; 100];
        pixels[0] = 0xFF000000;
        pixels[1] = 0xFFFF0000;
        let rect = PixelRect::new(0, 0, 10, 10);
        let count = count_unique_colors(&pixels, 10, &rect);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_count_layout_boxes() {
        let layout = render_to_layout("<div><p>A</p><p>B</p></div>", "", 800, 600);
        let count = count_layout_boxes(&layout);
        assert!(count >= 3, "Expected at least 3 boxes, got {}", count);
    }

    #[test]
    fn test_render_background_visible() {
        let pixels = render_html_with_css(
            "<div>Content</div>",
            "div { background-color: #0000ff; width: 200px; height: 100px; }",
            400,
            300,
        );
        let has_blue = pixels.iter().any(|&p| p == 0xFF0000FF);
        assert!(has_blue, "Expected blue background pixels");
    }

    #[test]
    fn test_render_multiple_backgrounds() {
        let pixels = render_html_with_css(
            "<div id='a'>A</div><div id='b'>B</div>",
            "#a { background-color: #ff0000; height: 50px; } #b { background-color: #0000ff; \
             height: 50px; }",
            200,
            200,
        );
        let has_red = pixels.iter().any(|&p| p == 0xFFFF0000);
        let has_blue = pixels.iter().any(|&p| p == 0xFF0000FF);
        assert!(has_red, "Expected red background");
        assert!(has_blue, "Expected blue background");
    }

    #[test]
    fn test_render_large_document() {
        let mut html = String::from("<html><body>");
        for i in 0..20 {
            html.push_str(&alloc::format!("<p>Paragraph {}</p>", i));
        }
        html.push_str("</body></html>");
        let pixels = render_html(&html, 800, 600);
        assert_eq!(pixels.len(), 480000);
    }

    #[test]
    fn test_full_pipeline_with_styles() {
        let html = "<!DOCTYPE html><html><head><style>body { background-color: #eee; } h1 { \
                    color: #333; } p { margin: 10px; \
                    }</style></head><body><h1>Welcome</h1><p>This is a test \
                    page.</p></body></html>";
        let pixels = render_html(html, 800, 600);
        assert_eq!(pixels.len(), 480000);
    }
}
