//! DOM API Bindings for JavaScript
//!
//! Bridges the JS VM with the DOM tree, providing document.getElementById,
//! element manipulation, event listener registration, timers (setTimeout/
//! setInterval), and console output.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use super::events::{EventDispatcher, EventType, NodeId};

// ---------------------------------------------------------------------------
// Timer system
// ---------------------------------------------------------------------------

/// Timer entry for setTimeout/setInterval
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TimerEntry {
    /// Unique timer ID
    pub id: u32,
    /// JS callback function ID
    pub callback_id: usize,
    /// Tick at which this timer fires
    pub fire_at: u64,
    /// Repeat interval (0 = one-shot)
    pub interval: u64,
    /// Whether this timer has been cancelled
    pub cancelled: bool,
}

/// Timer queue managing setTimeout/setInterval
#[allow(dead_code)]
pub struct TimerQueue {
    /// All scheduled timers
    timers: Vec<TimerEntry>,
    /// Next timer ID
    next_id: u32,
    /// Current tick counter
    current_tick: u64,
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerQueue {
    pub fn new() -> Self {
        Self {
            timers: Vec::new(),
            next_id: 1,
            current_tick: 0,
        }
    }

    /// Schedule a one-shot timer (setTimeout).
    /// Returns the timer ID.
    pub fn set_timeout(&mut self, callback_id: usize, delay_ticks: u64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.timers.push(TimerEntry {
            id,
            callback_id,
            fire_at: self.current_tick + delay_ticks,
            interval: 0,
            cancelled: false,
        });
        id
    }

    /// Schedule a repeating timer (setInterval).
    /// Returns the timer ID.
    pub fn set_interval(&mut self, callback_id: usize, interval_ticks: u64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.timers.push(TimerEntry {
            id,
            callback_id,
            fire_at: self.current_tick + interval_ticks,
            interval: interval_ticks,
            cancelled: false,
        });
        id
    }

    /// Cancel a timer by ID
    pub fn clear_timer(&mut self, timer_id: u32) -> bool {
        for timer in &mut self.timers {
            if timer.id == timer_id {
                timer.cancelled = true;
                return true;
            }
        }
        false
    }

    /// Advance the tick counter and return expired callback IDs
    pub fn tick(&mut self) -> Vec<usize> {
        self.current_tick += 1;
        let mut expired = Vec::new();

        let mut i = 0;
        while i < self.timers.len() {
            if self.timers[i].cancelled {
                self.timers.swap_remove(i);
                continue;
            }
            if self.timers[i].fire_at <= self.current_tick {
                expired.push(self.timers[i].callback_id);
                if self.timers[i].interval > 0 {
                    // Reschedule repeating timer
                    self.timers[i].fire_at = self.current_tick + self.timers[i].interval;
                    i += 1;
                } else {
                    self.timers.swap_remove(i);
                }
            } else {
                i += 1;
            }
        }

        expired
    }

    /// Check how many timers are pending
    pub fn pending_count(&self) -> usize {
        self.timers.iter().filter(|t| !t.cancelled).count()
    }

    /// Current tick value
    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }
}

// ---------------------------------------------------------------------------
// DOM node reference (simplified)
// ---------------------------------------------------------------------------

/// Simplified DOM node for JS bindings
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct DomNode {
    /// Node ID
    pub id: NodeId,
    /// Tag name (e.g., "div", "p", "span")
    pub tag: String,
    /// Element ID attribute
    pub element_id: String,
    /// Class names
    pub class_list: Vec<String>,
    /// Text content
    pub text_content: String,
    /// Inner HTML (raw text representation)
    pub inner_html_content: String,
    /// Attributes
    pub attributes: BTreeMap<String, String>,
    /// Child node IDs
    pub children: Vec<NodeId>,
    /// Parent node ID
    pub parent: Option<NodeId>,
    /// Inline style properties
    pub style: BTreeMap<String, String>,
}

impl DomNode {
    pub fn new(id: NodeId, tag: &str) -> Self {
        Self {
            id,
            tag: tag.to_string(),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// DOM API
// ---------------------------------------------------------------------------

/// Bridge between JS VM and DOM tree
#[allow(dead_code)]
pub struct DomApi {
    /// All DOM nodes (arena)
    nodes: Vec<DomNode>,
    /// ID-to-NodeId mapping
    id_map: BTreeMap<String, NodeId>,
    /// Event dispatcher
    pub event_dispatcher: EventDispatcher,
    /// Timer queue
    pub timer_queue: TimerQueue,
    /// Console output
    pub console_output: Vec<String>,
    /// Navigation requests (URLs to load)
    pub navigation_requests: Vec<String>,
}

impl Default for DomApi {
    fn default() -> Self {
        Self::new()
    }
}

impl DomApi {
    pub fn new() -> Self {
        // Create document root
        let root = DomNode::new(0, "html");
        Self {
            nodes: alloc::vec![root],
            id_map: BTreeMap::new(),
            event_dispatcher: EventDispatcher::new(),
            timer_queue: TimerQueue::new(),
            console_output: Vec::new(),
            navigation_requests: Vec::new(),
        }
    }

    /// Get a node by ID
    pub fn get_node(&self, id: NodeId) -> Option<&DomNode> {
        self.nodes.get(id)
    }

    /// Get a mutable node by ID
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut DomNode> {
        self.nodes.get_mut(id)
    }

    /// Total number of nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // -- document methods --

    /// document.getElementById(id) -> NodeId or None
    pub fn get_element_by_id(&self, element_id: &str) -> Option<NodeId> {
        self.id_map.get(element_id).copied()
    }

    /// document.createElement(tag) -> NodeId
    pub fn create_element(&mut self, tag: &str) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(DomNode::new(id, tag));
        id
    }

    /// document.createTextNode(text) -> NodeId
    pub fn create_text_node(&mut self, text: &str) -> NodeId {
        let id = self.nodes.len();
        let mut node = DomNode::new(id, "#text");
        node.text_content = text.to_string();
        self.nodes.push(node);
        id
    }

    /// Simple querySelector by tag name (first match)
    pub fn query_selector(&self, selector: &str) -> Option<NodeId> {
        if let Some(id_part) = selector.strip_prefix('#') {
            return self.get_element_by_id(id_part);
        }
        for node in &self.nodes {
            if node.tag == selector {
                return Some(node.id);
            }
        }
        None
    }

    // -- element methods --

    /// element.getAttribute(name)
    pub fn get_attribute(&self, node_id: NodeId, name: &str) -> Option<String> {
        self.nodes
            .get(node_id)
            .and_then(|n| n.attributes.get(name).cloned())
    }

    /// element.setAttribute(name, value)
    pub fn set_attribute(&mut self, node_id: NodeId, name: &str, value: &str) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.attributes.insert(name.to_string(), value.to_string());
            if name == "id" {
                let old_id = node.element_id.clone();
                if !old_id.is_empty() {
                    self.id_map.remove(&old_id);
                }
                node.element_id = value.to_string();
                self.id_map.insert(value.to_string(), node_id);
            }
        }
    }

    /// element.appendChild(child)
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId) -> bool {
        if parent_id >= self.nodes.len() || child_id >= self.nodes.len() {
            return false;
        }
        if parent_id == child_id {
            return false;
        }

        // Remove from old parent
        if let Some(old_parent) = self.nodes[child_id].parent {
            if let Some(parent_node) = self.nodes.get_mut(old_parent) {
                parent_node.children.retain(|&c| c != child_id);
            }
        }

        self.nodes[parent_id].children.push(child_id);
        self.nodes[child_id].parent = Some(parent_id);

        self.event_dispatcher
            .node_tree_mut()
            .set_parent(child_id, parent_id);

        true
    }

    /// element.removeChild(child)
    pub fn remove_child(&mut self, parent_id: NodeId, child_id: NodeId) -> bool {
        if let Some(parent) = self.nodes.get_mut(parent_id) {
            let before = parent.children.len();
            parent.children.retain(|&c| c != child_id);
            if parent.children.len() < before {
                if let Some(child) = self.nodes.get_mut(child_id) {
                    child.parent = None;
                }
                return true;
            }
        }
        false
    }

    /// element.textContent (getter)
    pub fn get_text_content(&self, node_id: NodeId) -> String {
        self.nodes
            .get(node_id)
            .map(|n| n.text_content.clone())
            .unwrap_or_default()
    }

    /// element.textContent = value (setter)
    pub fn set_text_content(&mut self, node_id: NodeId, text: &str) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.text_content = text.to_string();
            node.children.clear();
        }
    }

    /// element.innerHTML (getter)
    pub fn get_inner_html(&self, node_id: NodeId) -> String {
        self.nodes
            .get(node_id)
            .map(|n| n.inner_html_content.clone())
            .unwrap_or_default()
    }

    /// element.innerHTML = value (setter)
    pub fn set_inner_html(&mut self, node_id: NodeId, html: &str) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.inner_html_content = html.to_string();
            node.children.clear();
        }
    }

    /// element.style.setProperty(name, value)
    pub fn set_style_property(&mut self, node_id: NodeId, name: &str, value: &str) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.style.insert(name.to_string(), value.to_string());
        }
    }

    /// element.style.getPropertyValue(name)
    pub fn get_style_property(&self, node_id: NodeId, name: &str) -> Option<String> {
        self.nodes
            .get(node_id)
            .and_then(|n| n.style.get(name).cloned())
    }

    // -- Event listener bridge --

    /// addEventListener(node, type, callback_id)
    pub fn add_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: EventType,
        callback_id: usize,
    ) {
        self.event_dispatcher
            .add_event_listener(node_id, event_type, callback_id, false);
    }

    /// removeEventListener(node, type, callback_id)
    pub fn remove_event_listener(
        &mut self,
        node_id: NodeId,
        event_type: EventType,
        callback_id: usize,
    ) {
        self.event_dispatcher
            .remove_event_listener(node_id, event_type, callback_id, false);
    }

    // -- Console --

    /// console.log(args...)
    pub fn console_log(&mut self, message: &str) {
        self.console_output.push(message.to_string());
    }

    /// console.error(args...)
    pub fn console_error(&mut self, message: &str) {
        self.console_output
            .push(alloc::format!("[ERROR] {}", message));
    }

    // -- window stubs --

    /// window.alert(message) -- stub
    pub fn window_alert(&mut self, message: &str) {
        self.console_output
            .push(alloc::format!("[ALERT] {}", message));
    }

    /// Register a node's element ID in the lookup map
    pub fn register_element_id(&mut self, node_id: NodeId, element_id: &str) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.element_id = element_id.to_string();
        }
        self.id_map.insert(element_id.to_string(), node_id);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;

    #[test]
    fn test_dom_api_new() {
        let api = DomApi::new();
        assert_eq!(api.node_count(), 1);
        assert_eq!(api.get_node(0).unwrap().tag, "html");
    }

    #[test]
    fn test_create_element() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        assert_eq!(div, 1);
        assert_eq!(api.get_node(div).unwrap().tag, "div");
    }

    #[test]
    fn test_create_text_node() {
        let mut api = DomApi::new();
        let text = api.create_text_node("Hello");
        assert_eq!(api.get_node(text).unwrap().text_content, "Hello");
    }

    #[test]
    fn test_get_element_by_id() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.register_element_id(div, "main");
        assert_eq!(api.get_element_by_id("main"), Some(div));
        assert_eq!(api.get_element_by_id("nonexistent"), None);
    }

    #[test]
    fn test_set_attribute() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.set_attribute(div, "class", "container");
        assert_eq!(
            api.get_attribute(div, "class"),
            Some("container".to_string())
        );
    }

    #[test]
    fn test_set_attribute_id() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.set_attribute(div, "id", "my-div");
        assert_eq!(api.get_element_by_id("my-div"), Some(div));
    }

    #[test]
    fn test_append_child() {
        let mut api = DomApi::new();
        let parent = api.create_element("div");
        let child = api.create_element("span");
        assert!(api.append_child(parent, child));
        assert_eq!(api.get_node(parent).unwrap().children, vec![child]);
        assert_eq!(api.get_node(child).unwrap().parent, Some(parent));
    }

    #[test]
    fn test_remove_child() {
        let mut api = DomApi::new();
        let parent = api.create_element("div");
        let child = api.create_element("span");
        api.append_child(parent, child);
        assert!(api.remove_child(parent, child));
        assert!(api.get_node(parent).unwrap().children.is_empty());
        assert_eq!(api.get_node(child).unwrap().parent, None);
    }

    #[test]
    fn test_text_content() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.set_text_content(div, "Hello World");
        assert_eq!(api.get_text_content(div), "Hello World");
    }

    #[test]
    fn test_inner_html() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.set_inner_html(div, "<p>Test</p>");
        assert_eq!(api.get_inner_html(div), "<p>Test</p>");
    }

    #[test]
    fn test_style_property() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.set_style_property(div, "color", "red");
        assert_eq!(
            api.get_style_property(div, "color"),
            Some("red".to_string())
        );
        assert_eq!(api.get_style_property(div, "margin"), None);
    }

    #[test]
    fn test_query_selector_tag() {
        let mut api = DomApi::new();
        let _div = api.create_element("div");
        let p = api.create_element("p");
        assert_eq!(api.query_selector("p"), Some(p));
    }

    #[test]
    fn test_query_selector_id() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.register_element_id(div, "main");
        assert_eq!(api.query_selector("#main"), Some(div));
    }

    #[test]
    fn test_console_log() {
        let mut api = DomApi::new();
        api.console_log("test message");
        assert_eq!(api.console_output.len(), 1);
        assert_eq!(api.console_output[0], "test message");
    }

    #[test]
    fn test_console_error() {
        let mut api = DomApi::new();
        api.console_error("oh no");
        assert_eq!(api.console_output[0], "[ERROR] oh no");
    }

    #[test]
    fn test_window_alert() {
        let mut api = DomApi::new();
        api.window_alert("hello");
        assert_eq!(api.console_output[0], "[ALERT] hello");
    }

    #[test]
    fn test_timer_set_timeout() {
        let mut tq = TimerQueue::new();
        let id = tq.set_timeout(42, 5);
        assert_eq!(id, 1);
        assert_eq!(tq.pending_count(), 1);
    }

    #[test]
    fn test_timer_fires() {
        let mut tq = TimerQueue::new();
        tq.set_timeout(42, 3);
        assert!(tq.tick().is_empty());
        assert!(tq.tick().is_empty());
        let expired = tq.tick();
        assert_eq!(expired, vec![42]);
        assert_eq!(tq.pending_count(), 0);
    }

    #[test]
    fn test_timer_interval() {
        let mut tq = TimerQueue::new();
        tq.set_interval(99, 2);
        assert!(tq.tick().is_empty());
        let e2 = tq.tick();
        assert_eq!(e2, vec![99]);
        assert!(tq.tick().is_empty());
        let e4 = tq.tick();
        assert_eq!(e4, vec![99]);
    }

    #[test]
    fn test_timer_cancel() {
        let mut tq = TimerQueue::new();
        let id = tq.set_timeout(42, 5);
        assert!(tq.clear_timer(id));
        for _ in 0..10 {
            assert!(tq.tick().is_empty());
        }
    }

    #[test]
    fn test_timer_cancel_nonexistent() {
        let mut tq = TimerQueue::new();
        assert!(!tq.clear_timer(999));
    }

    #[test]
    fn test_timer_queue_default() {
        let tq = TimerQueue::default();
        assert_eq!(tq.pending_count(), 0);
        assert_eq!(tq.current_tick(), 0);
    }

    #[test]
    fn test_append_child_self() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        assert!(!api.append_child(div, div));
    }

    #[test]
    fn test_reparent_child() {
        let mut api = DomApi::new();
        let p1 = api.create_element("div");
        let p2 = api.create_element("div");
        let child = api.create_element("span");
        api.append_child(p1, child);
        api.append_child(p2, child);
        assert!(api.get_node(p1).unwrap().children.is_empty());
        assert_eq!(api.get_node(p2).unwrap().children, vec![child]);
    }

    #[test]
    fn test_add_event_listener_via_api() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.add_event_listener(div, EventType::Click, 100);
        assert_eq!(api.event_dispatcher.listeners_for(div).len(), 1);
    }

    #[test]
    fn test_remove_event_listener_via_api() {
        let mut api = DomApi::new();
        let div = api.create_element("div");
        api.add_event_listener(div, EventType::Click, 100);
        api.remove_event_listener(div, EventType::Click, 100);
        assert_eq!(api.event_dispatcher.listeners_for(div).len(), 0);
    }

    #[test]
    fn test_dom_node_default() {
        let node = DomNode::default();
        assert!(node.tag.is_empty());
        assert!(node.children.is_empty());
        assert!(node.parent.is_none());
    }
}
