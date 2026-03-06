//! Browser Window
//!
//! Top-level browser window that ties together URL navigation,
//! HTML parsing, CSS styling, layout, and painting. Manages the
//! address bar, viewport, scroll position, and history stacks.

#![allow(dead_code)]

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::{
    css_parser::{CssParser, Stylesheet},
    dom::Document,
    layout::{build_layout_tree, LayoutBox},
    paint::{build_display_list, DisplayList, Painter},
    style::StyleResolver,
    tree_builder::TreeBuilder,
};

/// Browser window state
pub struct BrowserWindow {
    /// Text in the address bar
    pub address_bar: String,
    /// Currently loaded URL
    pub current_url: String,
    /// Parsed DOM document
    pub document: Option<Document>,
    /// Author stylesheet
    pub stylesheet: Option<Stylesheet>,
    /// Layout tree root
    pub layout_root: Option<LayoutBox>,
    /// Current display list
    pub display_list: Option<DisplayList>,
    /// Vertical scroll offset (pixels)
    pub scroll_y: i32,
    /// Viewport width (pixels)
    pub viewport_width: i32,
    /// Viewport height (pixels)
    pub viewport_height: i32,
    /// Back navigation history
    pub back_history: Vec<String>,
    /// Forward navigation history
    pub forward_history: Vec<String>,
    /// Style resolver
    resolver: StyleResolver,
    /// Painter for rendering
    painter: Option<Painter>,
}

impl Default for BrowserWindow {
    fn default() -> Self {
        Self::new(800, 600)
    }
}

impl BrowserWindow {
    /// Create a new browser window with given viewport dimensions
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            address_bar: String::new(),
            current_url: String::new(),
            document: None,
            stylesheet: None,
            layout_root: None,
            display_list: None,
            scroll_y: 0,
            viewport_width: width,
            viewport_height: height,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            resolver: StyleResolver::new(),
            painter: None,
        }
    }

    /// Navigate to a URL string
    pub fn navigate(&mut self, url: &str) {
        // Save current URL in history
        if !self.current_url.is_empty() {
            self.back_history.push(self.current_url.clone());
        }
        self.forward_history.clear();

        self.current_url = url.to_string();
        self.address_bar = url.to_string();
        self.scroll_y = 0;

        // Fetch content (placeholder - would use net::http in real implementation)
        let html = self.fetch_url(url);
        self.load_html(&html);
    }

    /// Load raw HTML content directly
    pub fn load_html(&mut self, html: &str) {
        // Parse HTML
        let doc = TreeBuilder::build(html);

        // Extract inline styles from <style> elements
        let css = Self::extract_styles(html);

        // Parse CSS
        let stylesheet = CssParser::parse(&css);

        // Set up resolver
        self.resolver = StyleResolver::new();
        self.resolver.add_stylesheet(stylesheet.clone());

        // Build layout tree
        let layout = build_layout_tree(
            &doc,
            &self.resolver,
            self.viewport_width,
            self.viewport_height,
        );

        // Build display list
        let display_list = build_display_list(&layout, self.scroll_y);

        self.document = Some(doc);
        self.stylesheet = Some(stylesheet);
        self.layout_root = Some(layout);
        self.display_list = Some(display_list);
    }

    /// Load HTML with a separate CSS string
    pub fn load_html_with_css(&mut self, html: &str, css: &str) {
        let doc = TreeBuilder::build(html);
        let stylesheet = CssParser::parse(css);

        self.resolver = StyleResolver::new();
        self.resolver.add_stylesheet(stylesheet.clone());

        let layout = build_layout_tree(
            &doc,
            &self.resolver,
            self.viewport_width,
            self.viewport_height,
        );

        let display_list = build_display_list(&layout, self.scroll_y);

        self.document = Some(doc);
        self.stylesheet = Some(stylesheet);
        self.layout_root = Some(layout);
        self.display_list = Some(display_list);
    }

    /// Render the current page to a pixel buffer
    pub fn render(&mut self) -> &[u32] {
        let mut painter = Painter::new(self.viewport_width as usize, self.viewport_height as usize);

        if let Some(ref dl) = self.display_list {
            painter.paint(dl);
        }

        self.painter = Some(painter);
        self.painter.as_ref().unwrap().as_bytes()
    }

    /// Handle scrolling
    pub fn handle_scroll(&mut self, delta_y: i32) {
        self.scroll_y += delta_y;
        if self.scroll_y < 0 {
            self.scroll_y = 0;
        }

        // Cap scroll at content height minus viewport
        if let Some(ref layout) = self.layout_root {
            let content_height = super::css_parser::fp_to_px(
                layout.dimensions.content.height
                    + layout.dimensions.padding.top
                    + layout.dimensions.padding.bottom,
            );
            let max_scroll = content_height - self.viewport_height;
            if max_scroll > 0 && self.scroll_y > max_scroll {
                self.scroll_y = max_scroll;
            }
        }

        // Rebuild display list with new scroll
        self.rebuild_display_list();
    }

    /// Handle viewport resize
    pub fn handle_resize(&mut self, width: i32, height: i32) {
        self.viewport_width = width;
        self.viewport_height = height;
        self.relayout();
    }

    /// Go back in history
    pub fn go_back(&mut self) -> bool {
        if let Some(url) = self.back_history.pop() {
            self.forward_history.push(self.current_url.clone());
            let url_clone = url.clone();
            self.current_url = url;
            self.address_bar = url_clone.clone();
            let html = self.fetch_url(&url_clone);
            self.load_html(&html);
            true
        } else {
            false
        }
    }

    /// Go forward in history
    pub fn go_forward(&mut self) -> bool {
        if let Some(url) = self.forward_history.pop() {
            self.back_history.push(self.current_url.clone());
            let url_clone = url.clone();
            self.current_url = url;
            self.address_bar = url_clone.clone();
            let html = self.fetch_url(&url_clone);
            self.load_html(&html);
            true
        } else {
            false
        }
    }

    /// Reload the current page
    pub fn reload(&mut self) {
        if !self.current_url.is_empty() {
            let url = self.current_url.clone();
            let html = self.fetch_url(&url);
            self.load_html(&html);
        }
    }

    /// Set the address bar text
    pub fn set_url(&mut self, url: &str) {
        self.address_bar = url.to_string();
    }

    /// Get the current URL
    pub fn get_url(&self) -> &str {
        &self.current_url
    }

    /// Get the address bar text
    pub fn get_address_bar(&self) -> &str {
        &self.address_bar
    }

    /// Handle text input in address bar
    pub fn address_bar_input(&mut self, ch: char) {
        self.address_bar.push(ch);
    }

    /// Handle backspace in address bar
    pub fn address_bar_backspace(&mut self) {
        self.address_bar.pop();
    }

    /// Submit the address bar (navigate to typed URL)
    pub fn address_bar_submit(&mut self) {
        let url = self.address_bar.clone();
        self.navigate(&url);
    }

    /// Check if we can go back
    pub fn can_go_back(&self) -> bool {
        !self.back_history.is_empty()
    }

    /// Check if we can go forward
    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.is_empty()
    }

    /// Get the page title (from <title> element)
    pub fn get_title(&self) -> String {
        if let Some(ref doc) = self.document {
            let titles = doc.get_elements_by_tag_name("title");
            if let Some(&title_id) = titles.first() {
                return doc.inner_text(title_id);
            }
        }
        String::from("Untitled")
    }

    /// Get the document node count
    pub fn node_count(&self) -> usize {
        self.document.as_ref().map(|d| d.node_count()).unwrap_or(0)
    }

    /// Fetch URL content (placeholder for network integration)
    fn fetch_url(&self, url: &str) -> String {
        // In a real implementation, this would use crate::net::http
        // For now, return a placeholder page
        if url.starts_with("about:blank") {
            return String::new();
        }

        if url.starts_with("about:") {
            return alloc::format!(
                "<!DOCTYPE html><html><head><title>{0}</title></head><body><h1>{0}</\
                 h1><p>Internal page</p></body></html>",
                url
            );
        }

        // Default: show a "page not found" since we can't actually fetch
        alloc::format!(
            "<!DOCTYPE html><html><head><title>Error</title></head><body><h1>Page Not \
             Found</h1><p>Could not load: {}</p></body></html>",
            url
        )
    }

    /// Extract CSS from <style> tags in HTML
    fn extract_styles(html: &str) -> String {
        let mut css = String::new();
        let mut pos = 0;
        let bytes = html.as_bytes();

        while pos < bytes.len() {
            // Find <style
            if let Some(start) = find_tag_start(bytes, pos, b"<style") {
                // Find >
                let mut tag_end = start + 6;
                while tag_end < bytes.len() && bytes[tag_end] != b'>' {
                    tag_end += 1;
                }
                tag_end += 1; // skip '>'

                // Find </style>
                if let Some(end) = find_tag_start(bytes, tag_end, b"</style") {
                    let style_content = &html[tag_end..end];
                    css.push_str(style_content);
                    css.push('\n');
                    pos = end + 8;
                } else {
                    pos = tag_end;
                }
            } else {
                break;
            }
        }

        css
    }

    /// Re-layout the current document
    fn relayout(&mut self) {
        if let Some(ref doc) = self.document {
            let layout = build_layout_tree(
                doc,
                &self.resolver,
                self.viewport_width,
                self.viewport_height,
            );
            let display_list = build_display_list(&layout, self.scroll_y);
            self.layout_root = Some(layout);
            self.display_list = Some(display_list);
        }
    }

    /// Rebuild the display list (e.g., after scroll)
    fn rebuild_display_list(&mut self) {
        if let Some(ref layout) = self.layout_root {
            let display_list = build_display_list(layout, self.scroll_y);
            self.display_list = Some(display_list);
        }
    }
}

/// Find a tag in a byte slice (case-insensitive)
fn find_tag_start(bytes: &[u8], start: usize, tag: &[u8]) -> Option<usize> {
    if tag.is_empty() || start + tag.len() > bytes.len() {
        return None;
    }

    for i in start..=(bytes.len() - tag.len()) {
        let mut matched = true;
        for (j, &t) in tag.iter().enumerate() {
            if !bytes[i + j].eq_ignore_ascii_case(&t) {
                matched = false;
                break;
            }
        }
        if matched {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_new_window() {
        let w = BrowserWindow::new(800, 600);
        assert_eq!(w.viewport_width, 800);
        assert_eq!(w.viewport_height, 600);
        assert_eq!(w.scroll_y, 0);
    }

    #[test]
    fn test_default_window() {
        let w = BrowserWindow::default();
        assert_eq!(w.viewport_width, 800);
    }

    #[test]
    fn test_load_html() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html("<p>Hello</p>");
        assert!(w.document.is_some());
        assert!(w.layout_root.is_some());
    }

    #[test]
    fn test_load_html_with_css() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html_with_css("<p>Hello</p>", "p { color: red; }");
        assert!(w.document.is_some());
        assert!(w.stylesheet.is_some());
    }

    #[test]
    fn test_navigate() {
        let mut w = BrowserWindow::new(800, 600);
        w.navigate("about:test");
        assert_eq!(w.current_url, "about:test");
        assert!(w.document.is_some());
    }

    #[test]
    fn test_navigate_about_blank() {
        let mut w = BrowserWindow::new(800, 600);
        w.navigate("about:blank");
        assert_eq!(w.current_url, "about:blank");
    }

    #[test]
    fn test_history_back_forward() {
        let mut w = BrowserWindow::new(800, 600);
        w.navigate("about:page1");
        w.navigate("about:page2");
        assert!(w.can_go_back());
        assert!(!w.can_go_forward());

        w.go_back();
        assert_eq!(w.current_url, "about:page1");
        assert!(w.can_go_forward());

        w.go_forward();
        assert_eq!(w.current_url, "about:page2");
    }

    #[test]
    fn test_no_back_when_empty() {
        let mut w = BrowserWindow::new(800, 600);
        assert!(!w.can_go_back());
        assert!(!w.go_back());
    }

    #[test]
    fn test_reload() {
        let mut w = BrowserWindow::new(800, 600);
        w.navigate("about:test");
        w.reload();
        assert_eq!(w.current_url, "about:test");
    }

    #[test]
    fn test_scroll() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html("<p>Hello</p>");
        w.handle_scroll(100);
        assert!(w.scroll_y >= 0);
    }

    #[test]
    fn test_scroll_negative_clamped() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html("<p>Hello</p>");
        w.handle_scroll(-100);
        assert_eq!(w.scroll_y, 0);
    }

    #[test]
    fn test_resize() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html("<p>Hello</p>");
        w.handle_resize(1024, 768);
        assert_eq!(w.viewport_width, 1024);
        assert_eq!(w.viewport_height, 768);
    }

    #[test]
    fn test_get_title() {
        let mut w = BrowserWindow::new(800, 600);
        w.load_html("<html><head><title>My Page</title></head><body></body></html>");
        assert_eq!(w.get_title(), "My Page");
    }

    #[test]
    fn test_get_title_default() {
        let w = BrowserWindow::new(800, 600);
        assert_eq!(w.get_title(), "Untitled");
    }

    #[test]
    fn test_address_bar() {
        let mut w = BrowserWindow::new(800, 600);
        w.set_url("https://example.com");
        assert_eq!(w.get_address_bar(), "https://example.com");
    }

    #[test]
    fn test_address_bar_input() {
        let mut w = BrowserWindow::new(800, 600);
        w.address_bar_input('h');
        w.address_bar_input('i');
        assert_eq!(w.get_address_bar(), "hi");
    }

    #[test]
    fn test_address_bar_backspace() {
        let mut w = BrowserWindow::new(800, 600);
        w.set_url("abc");
        w.address_bar_backspace();
        assert_eq!(w.get_address_bar(), "ab");
    }

    #[test]
    fn test_node_count() {
        let mut w = BrowserWindow::new(800, 600);
        assert_eq!(w.node_count(), 0);
        w.load_html("<p>Hello</p>");
        assert!(w.node_count() > 0);
    }

    #[test]
    fn test_extract_styles() {
        let css = BrowserWindow::extract_styles(
            "<html><head><style>p { color: red; }</style></head><body></body></html>",
        );
        assert!(css.contains("color: red"));
    }

    #[test]
    fn test_extract_no_styles() {
        let css = BrowserWindow::extract_styles("<html><body></body></html>");
        assert!(css.is_empty());
    }

    #[test]
    fn test_find_tag_start() {
        let bytes = b"hello <style>content</style>";
        let result = find_tag_start(bytes, 0, b"<style");
        assert_eq!(result, Some(6));
    }

    #[test]
    fn test_find_tag_not_found() {
        let bytes = b"hello world";
        assert!(find_tag_start(bytes, 0, b"<style").is_none());
    }
}
