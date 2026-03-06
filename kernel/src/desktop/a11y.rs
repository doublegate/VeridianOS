//! Accessibility (a11y) Framework
//!
//! Provides an accessibility tree, screen reader support, high contrast
//! themes, and keyboard-driven navigation. Implements a subset of WAI-ARIA
//! roles and properties for desktop widget accessibility.
//!
//! All coordinates and sizes use integer types.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Roles and actions
// ---------------------------------------------------------------------------

/// Accessibility role for a UI element (WAI-ARIA subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A11yRole {
    /// Top-level window.
    Window,
    /// Clickable button.
    Button,
    /// Static text label.
    Label,
    /// Text input field.
    TextInput,
    /// Pop-up or pull-down menu.
    Menu,
    /// An item within a menu.
    MenuItem,
    /// Toolbar container.
    Toolbar,
    /// Scroll bar control.
    Scrollbar,
    /// A list container.
    List,
    /// An item within a list.
    ListItem,
    /// Modal or non-modal dialog.
    Dialog,
    /// Alert or notification.
    Alert,
    /// Visual separator.
    Separator,
    /// Image or icon.
    Image,
}

/// Actions that can be performed on an accessible element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A11yAction {
    /// Activate (click/press).
    Click,
    /// Set keyboard focus.
    Focus,
    /// Expand a collapsed node.
    Expand,
    /// Collapse an expanded node.
    Collapse,
    /// Scroll content upward.
    ScrollUp,
    /// Scroll content downward.
    ScrollDown,
}

// ---------------------------------------------------------------------------
// Accessibility node
// ---------------------------------------------------------------------------

/// Unique identifier for an a11y node.
pub type A11yNodeId = u32;

/// A single node in the accessibility tree.
#[derive(Debug, Clone)]
pub struct A11yNode {
    /// Unique identifier.
    pub id: A11yNodeId,
    /// Role of this element.
    pub role: A11yRole,
    /// Human-readable name.
    pub name: String,
    /// Optional longer description.
    pub description: String,
    /// Bounding rectangle (x, y, width, height).
    pub bounds_x: i32,
    pub bounds_y: i32,
    pub bounds_w: u32,
    pub bounds_h: u32,
    /// Child node IDs.
    pub children: Vec<A11yNodeId>,
    /// Whether this node can receive keyboard focus.
    pub focusable: bool,
    /// Whether this node currently has focus.
    pub focused: bool,
    /// Available actions.
    pub actions: Vec<A11yAction>,
    /// Current value (for sliders, text fields, etc.).
    pub value: String,
    /// Whether this node is expanded (for tree items, menus).
    pub expanded: bool,
}

impl A11yNode {
    /// Create a new node with the given role and name.
    pub fn new(id: A11yNodeId, role: A11yRole, name: &str) -> Self {
        Self {
            id,
            role,
            name: String::from(name),
            description: String::new(),
            bounds_x: 0,
            bounds_y: 0,
            bounds_w: 0,
            bounds_h: 0,
            children: Vec::new(),
            focusable: matches!(
                role,
                A11yRole::Button | A11yRole::TextInput | A11yRole::MenuItem | A11yRole::ListItem
            ),
            focused: false,
            actions: Vec::new(),
            value: String::new(),
            expanded: false,
        }
    }

    /// Set the bounding rectangle.
    pub fn set_bounds(&mut self, x: i32, y: i32, w: u32, h: u32) {
        self.bounds_x = x;
        self.bounds_y = y;
        self.bounds_w = w;
        self.bounds_h = h;
    }

    /// Add a child node ID.
    pub fn add_child(&mut self, child_id: A11yNodeId) {
        self.children.push(child_id);
    }

    /// Add an available action.
    pub fn add_action(&mut self, action: A11yAction) {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
    }
}

// ---------------------------------------------------------------------------
// Accessibility tree
// ---------------------------------------------------------------------------

/// The complete accessibility tree for the desktop.
#[derive(Debug)]
pub struct A11yTree {
    /// All nodes, indexed by ID.
    nodes: Vec<A11yNode>,
    /// ID of the root node (if any).
    root_id: Option<A11yNodeId>,
    /// ID of the currently focused node.
    focused_node_id: Option<A11yNodeId>,
    /// Next ID to assign.
    next_id: A11yNodeId,
}

impl Default for A11yTree {
    fn default() -> Self {
        Self::new()
    }
}

impl A11yTree {
    /// Create a new empty tree.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root_id: None,
            focused_node_id: None,
            next_id: 1,
        }
    }

    /// Add a node and return its ID.
    pub fn add_node(&mut self, mut node: A11yNode) -> A11yNodeId {
        let id = self.next_id;
        self.next_id += 1;
        node.id = id;
        if self.root_id.is_none() {
            self.root_id = Some(id);
        }
        self.nodes.push(node);
        id
    }

    /// Find a node by ID.
    pub fn find_by_id(&self, id: A11yNodeId) -> Option<&A11yNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find a mutable node by ID.
    pub fn find_by_id_mut(&mut self, id: A11yNodeId) -> Option<&mut A11yNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Get the currently focused node.
    pub fn focused_node(&self) -> Option<&A11yNode> {
        self.focused_node_id.and_then(|id| self.find_by_id(id))
    }

    /// Set focus to a specific node.
    pub fn set_focus(&mut self, id: A11yNodeId) -> bool {
        // Unfocus current
        if let Some(old_id) = self.focused_node_id {
            if let Some(node) = self.find_by_id_mut(old_id) {
                node.focused = false;
            }
        }

        // Focus new
        if let Some(node) = self.find_by_id_mut(id) {
            if node.focusable {
                node.focused = true;
                self.focused_node_id = Some(id);
                return true;
            }
        }
        false
    }

    /// Move focus to the next focusable node.
    pub fn next_focusable(&mut self) -> Option<A11yNodeId> {
        let focusable: Vec<A11yNodeId> = self
            .nodes
            .iter()
            .filter(|n| n.focusable)
            .map(|n| n.id)
            .collect();

        if focusable.is_empty() {
            return None;
        }

        let current_idx = self
            .focused_node_id
            .and_then(|id| focusable.iter().position(|&fid| fid == id))
            .unwrap_or(focusable.len().wrapping_sub(1));

        let next_idx = (current_idx + 1) % focusable.len();
        let next_id = focusable[next_idx];
        self.set_focus(next_id);
        Some(next_id)
    }

    /// Move focus to the previous focusable node.
    pub fn prev_focusable(&mut self) -> Option<A11yNodeId> {
        let focusable: Vec<A11yNodeId> = self
            .nodes
            .iter()
            .filter(|n| n.focusable)
            .map(|n| n.id)
            .collect();

        if focusable.is_empty() {
            return None;
        }

        let current_idx = self
            .focused_node_id
            .and_then(|id| focusable.iter().position(|&fid| fid == id))
            .unwrap_or(1);

        let prev_idx = if current_idx == 0 {
            focusable.len() - 1
        } else {
            current_idx - 1
        };
        let prev_id = focusable[prev_idx];
        self.set_focus(prev_id);
        Some(prev_id)
    }

    /// Number of nodes in the tree.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Build a tree from a list of window descriptions.
    pub fn build_from_windows(windows: &[(&str, i32, i32, u32, u32)]) -> Self {
        let mut tree = Self::new();

        for (name, x, y, w, h) in windows {
            let mut node = A11yNode::new(0, A11yRole::Window, name);
            node.set_bounds(*x, *y, *w, *h);
            node.focusable = true;
            node.add_action(A11yAction::Focus);
            tree.add_node(node);
        }

        tree
    }
}

// ---------------------------------------------------------------------------
// Screen reader
// ---------------------------------------------------------------------------

/// Screen reader that generates text announcements from the a11y tree.
#[derive(Debug)]
pub struct ScreenReader {
    /// Queue of pending announcements.
    announcements: Vec<String>,
    /// Whether the screen reader is enabled.
    pub enabled: bool,
    /// Maximum announcements to buffer.
    max_queue: usize,
}

impl Default for ScreenReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenReader {
    /// Create a new screen reader.
    pub fn new() -> Self {
        Self {
            announcements: Vec::new(),
            enabled: false,
            max_queue: 64,
        }
    }

    /// Announce a message.
    pub fn announce(&mut self, message: &str) {
        if !self.enabled {
            return;
        }
        if self.announcements.len() < self.max_queue {
            self.announcements.push(String::from(message));
        }
    }

    /// Read the focused element and announce its name and role.
    pub fn read_focused(&mut self, tree: &A11yTree) {
        if !self.enabled {
            return;
        }
        if let Some(node) = tree.focused_node() {
            let role_str = match node.role {
                A11yRole::Window => "window",
                A11yRole::Button => "button",
                A11yRole::Label => "label",
                A11yRole::TextInput => "text input",
                A11yRole::Menu => "menu",
                A11yRole::MenuItem => "menu item",
                A11yRole::Toolbar => "toolbar",
                A11yRole::Scrollbar => "scrollbar",
                A11yRole::List => "list",
                A11yRole::ListItem => "list item",
                A11yRole::Dialog => "dialog",
                A11yRole::Alert => "alert",
                A11yRole::Separator => "separator",
                A11yRole::Image => "image",
            };
            let mut msg = String::new();
            msg.push_str(&node.name);
            msg.push_str(", ");
            msg.push_str(role_str);
            if !node.value.is_empty() {
                msg.push_str(", value: ");
                msg.push_str(&node.value);
            }
            self.announce(&msg);
        }
    }

    /// Read all children of a node.
    pub fn read_all_children(&mut self, tree: &A11yTree, parent_id: A11yNodeId) {
        if !self.enabled {
            return;
        }
        if let Some(parent) = tree.find_by_id(parent_id) {
            let children = parent.children.clone();
            for child_id in &children {
                if let Some(child) = tree.find_by_id(*child_id) {
                    self.announce(&child.name);
                }
            }
        }
    }

    /// Drain all pending announcements.
    pub fn drain_announcements(&mut self) -> Vec<String> {
        let mut drained = Vec::new();
        core::mem::swap(&mut self.announcements, &mut drained);
        drained
    }

    /// Number of pending announcements.
    pub fn pending_count(&self) -> usize {
        self.announcements.len()
    }
}

// ---------------------------------------------------------------------------
// High contrast theme
// ---------------------------------------------------------------------------

/// High contrast colour theme for accessibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighContrastTheme {
    /// Background colour (ARGB8888).
    pub bg_color: u32,
    /// Foreground / text colour.
    pub fg_color: u32,
    /// Accent colour (focused items, links).
    pub accent_color: u32,
    /// Border colour.
    pub border_color: u32,
}

impl HighContrastTheme {
    /// Black background, white text (classic high contrast).
    pub const DARK: Self = Self {
        bg_color: 0xFF000000,
        fg_color: 0xFFFFFFFF,
        accent_color: 0xFF00FFFF,
        border_color: 0xFFFFFF00,
    };

    /// White background, black text.
    pub const LIGHT: Self = Self {
        bg_color: 0xFFFFFFFF,
        fg_color: 0xFF000000,
        accent_color: 0xFF0000FF,
        border_color: 0xFF000000,
    };

    /// Apply the theme colours to a pixel buffer region (fills with bg_color).
    pub fn apply_background(&self, buf: &mut [u32], start: usize, count: usize) {
        let end = (start + count).min(buf.len());
        for px in &mut buf[start..end] {
            *px = self.bg_color;
        }
    }
}

// ---------------------------------------------------------------------------
// Accessibility settings
// ---------------------------------------------------------------------------

/// Global accessibility settings.
#[derive(Debug, Clone)]
pub struct AccessibilitySettings {
    /// Whether the screen reader is enabled.
    pub screen_reader_enabled: bool,
    /// Whether high contrast mode is active.
    pub high_contrast: bool,
    /// High contrast theme to use.
    pub contrast_theme: HighContrastTheme,
    /// Whether to use larger text (2x glyph scaling).
    pub large_text: bool,
    /// Whether to reduce motion/animations.
    pub reduce_motion: bool,
    /// Whether sticky keys are enabled.
    pub sticky_keys: bool,
    /// Keyboard repeat delay in ticks.
    pub key_repeat_delay: u32,
    /// Keyboard repeat rate in ticks per character.
    pub key_repeat_rate: u32,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self {
            screen_reader_enabled: false,
            high_contrast: false,
            contrast_theme: HighContrastTheme::DARK,
            large_text: false,
            reduce_motion: false,
            sticky_keys: false,
            key_repeat_delay: 500,
            key_repeat_rate: 50,
        }
    }
}

impl AccessibilitySettings {
    /// Whether any accessibility feature is active.
    pub fn any_active(&self) -> bool {
        self.screen_reader_enabled
            || self.high_contrast
            || self.large_text
            || self.reduce_motion
            || self.sticky_keys
    }
}

// ---------------------------------------------------------------------------
// Keyboard navigator
// ---------------------------------------------------------------------------

/// Navigation area (a group of focusable elements).
#[derive(Debug, Clone)]
pub struct NavigationArea {
    /// Area name (for screen reader).
    pub name: String,
    /// Node IDs belonging to this area.
    pub node_ids: Vec<A11yNodeId>,
    /// Currently focused index within this area.
    pub focus_index: usize,
}

impl NavigationArea {
    /// Create a new area.
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            node_ids: Vec::new(),
            focus_index: 0,
        }
    }

    /// Add a node to this area.
    pub fn add_node(&mut self, id: A11yNodeId) {
        self.node_ids.push(id);
    }
}

/// Keyboard-driven navigation controller.
///
/// Supports F6 for area cycling, Tab/Shift-Tab for intra-area focus,
/// Arrow keys for directional movement, Enter for activation, Escape for
/// cancel.
#[derive(Debug)]
pub struct KeyboardNavigator {
    /// Navigation areas.
    pub areas: Vec<NavigationArea>,
    /// Index of the currently active area.
    pub current_area: usize,
}

impl Default for KeyboardNavigator {
    fn default() -> Self {
        Self::new()
    }
}

/// Key events understood by the keyboard navigator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    /// F6: cycle between areas.
    CycleArea,
    /// Tab: next focusable in area.
    Tab,
    /// Shift+Tab: previous focusable in area.
    ShiftTab,
    /// Arrow up within area.
    Up,
    /// Arrow down within area.
    Down,
    /// Enter: activate focused element.
    Enter,
    /// Escape: cancel or close.
    Escape,
}

/// Result of handling a navigation key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavResult {
    /// Focus moved to a node.
    FocusChanged(A11yNodeId),
    /// An action was triggered on a node.
    Activated(A11yNodeId),
    /// Escape was pressed (dismiss/cancel).
    Cancelled,
    /// No change.
    NoOp,
}

impl KeyboardNavigator {
    /// Create a new navigator.
    pub fn new() -> Self {
        Self {
            areas: Vec::new(),
            current_area: 0,
        }
    }

    /// Add a navigation area.
    pub fn add_area(&mut self, area: NavigationArea) {
        self.areas.push(area);
    }

    /// Cycle to the next area (F6).
    pub fn cycle_area(&mut self) -> NavResult {
        if self.areas.is_empty() {
            return NavResult::NoOp;
        }
        self.current_area = (self.current_area + 1) % self.areas.len();
        self.current_focused_node()
            .map(NavResult::FocusChanged)
            .unwrap_or(NavResult::NoOp)
    }

    /// Move focus to the next node in the current area.
    pub fn focus_next(&mut self) -> NavResult {
        if let Some(area) = self.areas.get_mut(self.current_area) {
            if area.node_ids.is_empty() {
                return NavResult::NoOp;
            }
            area.focus_index = (area.focus_index + 1) % area.node_ids.len();
            NavResult::FocusChanged(area.node_ids[area.focus_index])
        } else {
            NavResult::NoOp
        }
    }

    /// Move focus to the previous node in the current area.
    pub fn focus_prev(&mut self) -> NavResult {
        if let Some(area) = self.areas.get_mut(self.current_area) {
            if area.node_ids.is_empty() {
                return NavResult::NoOp;
            }
            area.focus_index = if area.focus_index == 0 {
                area.node_ids.len() - 1
            } else {
                area.focus_index - 1
            };
            NavResult::FocusChanged(area.node_ids[area.focus_index])
        } else {
            NavResult::NoOp
        }
    }

    /// Handle a navigation key event.
    pub fn handle_key(&mut self, key: NavKey) -> NavResult {
        match key {
            NavKey::CycleArea => self.cycle_area(),
            NavKey::Tab | NavKey::Down => self.focus_next(),
            NavKey::ShiftTab | NavKey::Up => self.focus_prev(),
            NavKey::Enter => self
                .current_focused_node()
                .map(NavResult::Activated)
                .unwrap_or(NavResult::NoOp),
            NavKey::Escape => NavResult::Cancelled,
        }
    }

    /// Get the currently focused node ID.
    pub fn current_focused_node(&self) -> Option<A11yNodeId> {
        let area = self.areas.get(self.current_area)?;
        area.node_ids.get(area.focus_index).copied()
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
    fn test_a11y_node_new() {
        let node = A11yNode::new(1, A11yRole::Button, "OK");
        assert_eq!(node.name, "OK");
        assert!(node.focusable);
        assert!(!node.focused);
    }

    #[test]
    fn test_a11y_node_label_not_focusable() {
        let node = A11yNode::new(1, A11yRole::Label, "Status");
        assert!(!node.focusable);
    }

    #[test]
    fn test_a11y_tree_add_and_find() {
        let mut tree = A11yTree::new();
        let node = A11yNode::new(0, A11yRole::Window, "Main");
        let id = tree.add_node(node);
        assert!(tree.find_by_id(id).is_some());
        assert_eq!(tree.node_count(), 1);
    }

    #[test]
    fn test_a11y_tree_focus() {
        let mut tree = A11yTree::new();
        let mut btn = A11yNode::new(0, A11yRole::Button, "Click Me");
        btn.focusable = true;
        let id = tree.add_node(btn);
        assert!(tree.set_focus(id));
        assert!(tree.focused_node().is_some());
        assert_eq!(tree.focused_node().unwrap().name, "Click Me");
    }

    #[test]
    fn test_a11y_tree_next_focusable() {
        let mut tree = A11yTree::new();
        let btn1 = A11yNode::new(0, A11yRole::Button, "A");
        let btn2 = A11yNode::new(0, A11yRole::Button, "B");
        tree.add_node(btn1);
        tree.add_node(btn2);
        let next = tree.next_focusable();
        assert!(next.is_some());
    }

    #[test]
    fn test_a11y_tree_prev_focusable() {
        let mut tree = A11yTree::new();
        let btn1 = A11yNode::new(0, A11yRole::Button, "A");
        let btn2 = A11yNode::new(0, A11yRole::Button, "B");
        let id1 = tree.add_node(btn1);
        let _id2 = tree.add_node(btn2);
        tree.set_focus(id1);
        let prev = tree.prev_focusable();
        assert!(prev.is_some());
    }

    #[test]
    fn test_build_from_windows() {
        let tree = A11yTree::build_from_windows(&[("Terminal", 0, 0, 800, 600)]);
        assert_eq!(tree.node_count(), 1);
    }

    #[test]
    fn test_screen_reader_announce() {
        let mut reader = ScreenReader::new();
        reader.enabled = true;
        reader.announce("Hello");
        assert_eq!(reader.pending_count(), 1);
        let msgs = reader.drain_announcements();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "Hello");
    }

    #[test]
    fn test_screen_reader_disabled() {
        let mut reader = ScreenReader::new();
        reader.announce("Should not be added");
        assert_eq!(reader.pending_count(), 0);
    }

    #[test]
    fn test_screen_reader_read_focused() {
        let mut tree = A11yTree::new();
        let btn = A11yNode::new(0, A11yRole::Button, "Save");
        let id = tree.add_node(btn);
        tree.set_focus(id);
        let mut reader = ScreenReader::new();
        reader.enabled = true;
        reader.read_focused(&tree);
        assert_eq!(reader.pending_count(), 1);
    }

    #[test]
    fn test_high_contrast_theme() {
        let theme = HighContrastTheme::DARK;
        assert_eq!(theme.bg_color, 0xFF000000);
        assert_eq!(theme.fg_color, 0xFFFFFFFF);
        let mut buf = vec![0xFFFFFFFF_u32; 10];
        theme.apply_background(&mut buf, 0, 5);
        assert_eq!(buf[0], 0xFF000000);
        assert_eq!(buf[5], 0xFFFFFFFF);
    }

    #[test]
    fn test_accessibility_settings_default() {
        let settings = AccessibilitySettings::default();
        assert!(!settings.any_active());
    }

    #[test]
    fn test_keyboard_navigator_cycle() {
        let mut nav = KeyboardNavigator::new();
        let mut area1 = NavigationArea::new("Menu");
        area1.add_node(10);
        area1.add_node(11);
        let mut area2 = NavigationArea::new("Content");
        area2.add_node(20);
        nav.add_area(area1);
        nav.add_area(area2);
        assert_eq!(nav.current_area, 0);
        nav.cycle_area();
        assert_eq!(nav.current_area, 1);
        nav.cycle_area();
        assert_eq!(nav.current_area, 0);
    }

    #[test]
    fn test_keyboard_navigator_focus_next_prev() {
        let mut nav = KeyboardNavigator::new();
        let mut area = NavigationArea::new("Toolbar");
        area.add_node(1);
        area.add_node(2);
        area.add_node(3);
        nav.add_area(area);
        let result = nav.focus_next();
        assert_eq!(result, NavResult::FocusChanged(2));
        let result = nav.focus_prev();
        assert_eq!(result, NavResult::FocusChanged(1));
    }

    #[test]
    fn test_keyboard_navigator_handle_key() {
        let mut nav = KeyboardNavigator::new();
        let mut area = NavigationArea::new("Test");
        area.add_node(5);
        nav.add_area(area);
        let result = nav.handle_key(NavKey::Enter);
        assert_eq!(result, NavResult::Activated(5));
        let result = nav.handle_key(NavKey::Escape);
        assert_eq!(result, NavResult::Cancelled);
    }

    #[test]
    fn test_nav_result_no_op_empty() {
        let mut nav = KeyboardNavigator::new();
        assert_eq!(nav.handle_key(NavKey::Tab), NavResult::NoOp);
    }
}
