//! DOM Event System
//!
//! Implements W3C-style DOM events with capture/bubble propagation,
//! hit testing against layout boxes, and event listener management.
//! All coordinates use 26.6 fixed-point arithmetic (i32).

use alloc::{collections::BTreeMap, vec::Vec};

// ---------------------------------------------------------------------------
// Type aliases for Phase A interop
// ---------------------------------------------------------------------------

/// Node identifier (arena index into DOM tree)
pub type NodeId = usize;

/// 26.6 fixed-point coordinate (from Phase A layout module)
pub type FixedPoint = i32;

/// Shift amount for 26.6 fixed-point
pub const FP_SHIFT: i32 = 6;

/// Convert integer to 26.6 fixed-point
#[inline]
pub const fn fp_from_int(v: i32) -> FixedPoint {
    v << FP_SHIFT
}

/// Convert 26.6 fixed-point to integer (truncate)
#[inline]
pub const fn fp_to_int(v: FixedPoint) -> i32 {
    v >> FP_SHIFT
}

// ---------------------------------------------------------------------------
// Event types and phases
// ---------------------------------------------------------------------------

/// DOM event type enumeration
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    Click,
    MouseDown,
    MouseUp,
    MouseMove,
    MouseOver,
    MouseOut,
    KeyDown,
    KeyUp,
    KeyPress,
    Focus,
    Blur,
    Submit,
    Input,
    Change,
    Scroll,
    Load,
    Unload,
    Resize,
}

impl EventType {
    /// Whether this event type bubbles by default
    pub fn bubbles_default(self) -> bool {
        match self {
            Self::Click
            | Self::MouseDown
            | Self::MouseUp
            | Self::MouseMove
            | Self::MouseOver
            | Self::MouseOut
            | Self::KeyDown
            | Self::KeyUp
            | Self::KeyPress
            | Self::Input
            | Self::Change
            | Self::Scroll
            | Self::Submit => true,
            Self::Focus | Self::Blur | Self::Load | Self::Unload | Self::Resize => false,
        }
    }

    /// Whether this event type is cancelable by default
    pub fn cancelable_default(self) -> bool {
        matches!(
            self,
            Self::Click
                | Self::MouseDown
                | Self::MouseUp
                | Self::KeyDown
                | Self::KeyPress
                | Self::Submit
        )
    }
}

/// Event propagation phase
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventPhase {
    /// Not dispatched yet
    #[default]
    None,
    /// Capture phase: root to target
    Capture,
    /// At the target element
    Target,
    /// Bubble phase: target to root
    Bubble,
}

// ---------------------------------------------------------------------------
// Event struct
// ---------------------------------------------------------------------------

/// A DOM event
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Event {
    /// Type of the event
    pub event_type: EventType,
    /// Target node (the node the event was dispatched on)
    pub target: NodeId,
    /// Current target during propagation
    pub current_target: NodeId,
    /// Current propagation phase
    pub phase: EventPhase,
    /// Whether the event bubbles
    pub bubbles: bool,
    /// Whether the event is cancelable
    pub cancelable: bool,
    /// Whether preventDefault() was called
    pub default_prevented: bool,
    /// Whether stopPropagation() was called
    pub propagation_stopped: bool,
    /// Whether stopImmediatePropagation() was called
    pub immediate_propagation_stopped: bool,
    /// Mouse X position (pixel coordinates)
    pub mouse_x: i32,
    /// Mouse Y position (pixel coordinates)
    pub mouse_y: i32,
    /// Mouse button (0=left, 1=middle, 2=right)
    pub button: u8,
    /// Keyboard scancode
    pub key_code: u32,
    /// Character value for KeyPress
    pub char_code: u32,
    /// Modifier keys bitmask (1=shift, 2=ctrl, 4=alt, 8=meta)
    pub modifiers: u8,
}

impl Event {
    /// Create a new event with defaults from its type
    pub fn new(event_type: EventType, target: NodeId) -> Self {
        Self {
            event_type,
            target,
            current_target: target,
            phase: EventPhase::None,
            bubbles: event_type.bubbles_default(),
            cancelable: event_type.cancelable_default(),
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
            mouse_x: 0,
            mouse_y: 0,
            button: 0,
            key_code: 0,
            char_code: 0,
            modifiers: 0,
        }
    }

    /// Create a mouse event
    pub fn mouse(event_type: EventType, target: NodeId, x: i32, y: i32, button: u8) -> Self {
        let mut ev = Self::new(event_type, target);
        ev.mouse_x = x;
        ev.mouse_y = y;
        ev.button = button;
        ev
    }

    /// Create a keyboard event
    pub fn keyboard(event_type: EventType, target: NodeId, key_code: u32, char_code: u32) -> Self {
        let mut ev = Self::new(event_type, target);
        ev.key_code = key_code;
        ev.char_code = char_code;
        ev
    }

    /// Call preventDefault()
    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.default_prevented = true;
        }
    }

    /// Call stopPropagation()
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    /// Call stopImmediatePropagation()
    pub fn stop_immediate_propagation(&mut self) {
        self.propagation_stopped = true;
        self.immediate_propagation_stopped = true;
    }

    /// Check if shift key is held
    pub fn shift_key(&self) -> bool {
        self.modifiers & 1 != 0
    }

    /// Check if ctrl key is held
    pub fn ctrl_key(&self) -> bool {
        self.modifiers & 2 != 0
    }

    /// Check if alt key is held
    pub fn alt_key(&self) -> bool {
        self.modifiers & 4 != 0
    }

    /// Check if meta key is held
    pub fn meta_key(&self) -> bool {
        self.modifiers & 8 != 0
    }
}

// ---------------------------------------------------------------------------
// Event Listener
// ---------------------------------------------------------------------------

/// An event listener registration
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventListener {
    /// Type of event to listen for
    pub event_type: EventType,
    /// Callback identifier (index into JS function table)
    pub callback_id: usize,
    /// Whether to listen during capture phase
    pub use_capture: bool,
}

impl EventListener {
    pub fn new(event_type: EventType, callback_id: usize, use_capture: bool) -> Self {
        Self {
            event_type,
            callback_id,
            use_capture,
        }
    }
}

// ---------------------------------------------------------------------------
// Layout box reference for hit testing
// ---------------------------------------------------------------------------

/// Axis-aligned bounding box for a layout element (pixel coordinates)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HitRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub node_id: NodeId,
}

impl HitRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32, node_id: NodeId) -> Self {
        Self {
            x,
            y,
            width,
            height,
            node_id,
        }
    }

    /// Check if point (px, py) is inside this rect
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

// ---------------------------------------------------------------------------
// Node ancestry (for propagation path)
// ---------------------------------------------------------------------------

/// Simple tree structure for tracking parent-child relationships
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct NodeTree {
    /// Parent of each node (None for root)
    parents: BTreeMap<NodeId, NodeId>,
}

impl NodeTree {
    pub fn new() -> Self {
        Self {
            parents: BTreeMap::new(),
        }
    }

    /// Set the parent of a node
    pub fn set_parent(&mut self, child: NodeId, parent: NodeId) {
        self.parents.insert(child, parent);
    }

    /// Get the parent of a node
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.parents.get(&node).copied()
    }

    /// Get the ancestor path from root to node (inclusive)
    pub fn ancestor_path(&self, node: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut current = node;
        path.push(current);
        while let Some(parent) = self.parents.get(&current) {
            path.push(*parent);
            current = *parent;
        }
        path.reverse();
        path
    }

    /// Remove a node from the tree
    pub fn remove(&mut self, node: NodeId) {
        self.parents.remove(&node);
    }
}

// ---------------------------------------------------------------------------
// Event Dispatcher
// ---------------------------------------------------------------------------

/// Dispatches DOM events with capture/bubble propagation
#[allow(dead_code)]
pub struct EventDispatcher {
    /// Event listeners keyed by node ID
    listeners: BTreeMap<NodeId, Vec<EventListener>>,
    /// Node tree for propagation paths
    node_tree: NodeTree,
    /// Hit-test boxes for finding event targets
    hit_boxes: Vec<HitRect>,
    /// Callbacks that were invoked (callback_id, event snapshot)
    invoked: Vec<(usize, EventType)>,
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            listeners: BTreeMap::new(),
            node_tree: NodeTree::new(),
            hit_boxes: Vec::new(),
            invoked: Vec::new(),
        }
    }

    /// Access the node tree
    pub fn node_tree(&self) -> &NodeTree {
        &self.node_tree
    }

    /// Access the node tree mutably
    pub fn node_tree_mut(&mut self) -> &mut NodeTree {
        &mut self.node_tree
    }

    // -- Listener management --

    /// Add an event listener to a node
    pub fn add_event_listener(
        &mut self,
        node: NodeId,
        event_type: EventType,
        callback_id: usize,
        use_capture: bool,
    ) {
        let listener = EventListener::new(event_type, callback_id, use_capture);
        self.listeners.entry(node).or_default().push(listener);
    }

    /// Remove an event listener from a node
    pub fn remove_event_listener(
        &mut self,
        node: NodeId,
        event_type: EventType,
        callback_id: usize,
        use_capture: bool,
    ) -> bool {
        if let Some(list) = self.listeners.get_mut(&node) {
            let before = list.len();
            list.retain(|l| {
                !(l.event_type == event_type
                    && l.callback_id == callback_id
                    && l.use_capture == use_capture)
            });
            list.len() < before
        } else {
            false
        }
    }

    /// Get listeners for a node
    pub fn listeners_for(&self, node: NodeId) -> &[EventListener] {
        self.listeners
            .get(&node)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Clear all listeners for a node
    pub fn clear_listeners(&mut self, node: NodeId) {
        self.listeners.remove(&node);
    }

    // -- Hit-test boxes --

    /// Set the hit-test boxes (rebuilt after layout)
    pub fn set_hit_boxes(&mut self, boxes: Vec<HitRect>) {
        self.hit_boxes = boxes;
    }

    /// Add a single hit-test box
    pub fn add_hit_box(&mut self, rect: HitRect) {
        self.hit_boxes.push(rect);
    }

    /// Clear hit-test boxes
    pub fn clear_hit_boxes(&mut self) {
        self.hit_boxes.clear();
    }

    /// Hit test: find the frontmost (last-drawn) node at pixel coordinates.
    /// Returns None if no node at that position.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<NodeId> {
        // Later boxes are drawn on top, so iterate in reverse
        for rect in self.hit_boxes.iter().rev() {
            if rect.contains(x, y) {
                return Some(rect.node_id);
            }
        }
        None
    }

    // -- Dispatch --

    /// Get the list of invoked callbacks (callback_id, event_type) from last
    /// dispatch
    pub fn take_invoked(&mut self) -> Vec<(usize, EventType)> {
        core::mem::take(&mut self.invoked)
    }

    /// Dispatch an event through the capture → target → bubble phases.
    /// Returns true if the default action should be prevented.
    pub fn dispatch(&mut self, event: &mut Event) -> bool {
        // Build propagation path (root → target)
        let path = self.node_tree.ancestor_path(event.target);
        if path.is_empty() {
            return event.default_prevented;
        }

        let target_idx = path.len() - 1;

        // -- Capture phase (root → target, exclusive of target) --
        event.phase = EventPhase::Capture;
        for &node in &path[..target_idx] {
            if event.propagation_stopped {
                break;
            }
            event.current_target = node;
            self.invoke_listeners(node, event, true);
        }

        // -- Target phase --
        if !event.propagation_stopped {
            event.phase = EventPhase::Target;
            event.current_target = event.target;
            // At target, invoke both capture and bubble listeners
            self.invoke_listeners(event.target, event, true);
            if !event.immediate_propagation_stopped {
                self.invoke_listeners(event.target, event, false);
            }
        }

        // -- Bubble phase (target → root, exclusive of target) --
        if event.bubbles && !event.propagation_stopped {
            event.phase = EventPhase::Bubble;
            for &node in path[..target_idx].iter().rev() {
                if event.propagation_stopped {
                    break;
                }
                event.current_target = node;
                self.invoke_listeners(node, event, false);
            }
        }

        event.default_prevented
    }

    /// Invoke matching listeners on a node.
    /// In a real browser, this would call into the JS VM. Here we record
    /// the invocations for later processing by the ScriptEngine.
    fn invoke_listeners(&mut self, node: NodeId, event: &Event, capture: bool) {
        // Clone listener list to avoid borrow conflicts
        let listeners: Vec<EventListener> = self.listeners.get(&node).cloned().unwrap_or_default();

        for listener in &listeners {
            if event.immediate_propagation_stopped {
                break;
            }
            if listener.event_type != event.event_type {
                continue;
            }
            // During capture phase, only invoke capture listeners
            // During bubble phase, only invoke bubble listeners
            // During target phase, we call this twice (once for each)
            if event.phase != EventPhase::Target && listener.use_capture != capture {
                continue;
            }
            if event.phase == EventPhase::Target && listener.use_capture != capture {
                continue;
            }
            self.invoked.push((listener.callback_id, event.event_type));
        }
    }

    /// Convenience: dispatch a mouse click at pixel coordinates.
    /// Performs hit-test, then dispatches Click event.
    /// Returns (target_node, default_prevented) or None if miss.
    pub fn dispatch_click(&mut self, x: i32, y: i32, button: u8) -> Option<(NodeId, bool)> {
        let target = self.hit_test(x, y)?;
        let mut event = Event::mouse(EventType::Click, target, x, y, button);
        let prevented = self.dispatch(&mut event);
        Some((target, prevented))
    }

    /// Convenience: dispatch a mouse move event
    pub fn dispatch_mouse_move(&mut self, x: i32, y: i32) -> Option<(NodeId, bool)> {
        let target = self.hit_test(x, y)?;
        let mut event = Event::mouse(EventType::MouseMove, target, x, y, 0);
        let prevented = self.dispatch(&mut event);
        Some((target, prevented))
    }

    /// Convenience: dispatch a keyboard event to a focused node
    pub fn dispatch_key(
        &mut self,
        target: NodeId,
        event_type: EventType,
        key_code: u32,
        char_code: u32,
        modifiers: u8,
    ) -> bool {
        let mut event = Event::keyboard(event_type, target, key_code, char_code);
        event.modifiers = modifiers;
        self.dispatch(&mut event)
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

    fn setup_tree() -> EventDispatcher {
        // Build a simple tree: 0 (root) -> 1 -> 2 (leaf)
        let mut d = EventDispatcher::new();
        d.node_tree_mut().set_parent(1, 0);
        d.node_tree_mut().set_parent(2, 1);
        d
    }

    #[test]
    fn test_event_type_bubbles() {
        assert!(EventType::Click.bubbles_default());
        assert!(!EventType::Focus.bubbles_default());
        assert!(!EventType::Load.bubbles_default());
        assert!(EventType::KeyDown.bubbles_default());
    }

    #[test]
    fn test_event_type_cancelable() {
        assert!(EventType::Click.cancelable_default());
        assert!(!EventType::MouseMove.cancelable_default());
        assert!(EventType::Submit.cancelable_default());
    }

    #[test]
    fn test_event_creation() {
        let ev = Event::new(EventType::Click, 5);
        assert_eq!(ev.event_type, EventType::Click);
        assert_eq!(ev.target, 5);
        assert!(ev.bubbles);
        assert!(ev.cancelable);
        assert!(!ev.default_prevented);
    }

    #[test]
    fn test_event_mouse() {
        let ev = Event::mouse(EventType::MouseDown, 3, 100, 200, 1);
        assert_eq!(ev.mouse_x, 100);
        assert_eq!(ev.mouse_y, 200);
        assert_eq!(ev.button, 1);
        assert_eq!(ev.target, 3);
    }

    #[test]
    fn test_event_keyboard() {
        let ev = Event::keyboard(EventType::KeyDown, 2, 13, 0);
        assert_eq!(ev.key_code, 13);
        assert_eq!(ev.char_code, 0);
    }

    #[test]
    fn test_prevent_default() {
        let mut ev = Event::new(EventType::Click, 0);
        assert!(!ev.default_prevented);
        ev.prevent_default();
        assert!(ev.default_prevented);
    }

    #[test]
    fn test_prevent_default_non_cancelable() {
        let mut ev = Event::new(EventType::MouseMove, 0);
        assert!(!ev.cancelable);
        ev.prevent_default();
        assert!(!ev.default_prevented);
    }

    #[test]
    fn test_stop_propagation() {
        let mut ev = Event::new(EventType::Click, 0);
        ev.stop_propagation();
        assert!(ev.propagation_stopped);
        assert!(!ev.immediate_propagation_stopped);
    }

    #[test]
    fn test_stop_immediate_propagation() {
        let mut ev = Event::new(EventType::Click, 0);
        ev.stop_immediate_propagation();
        assert!(ev.propagation_stopped);
        assert!(ev.immediate_propagation_stopped);
    }

    #[test]
    fn test_modifier_keys() {
        let mut ev = Event::new(EventType::KeyDown, 0);
        ev.modifiers = 0b1111;
        assert!(ev.shift_key());
        assert!(ev.ctrl_key());
        assert!(ev.alt_key());
        assert!(ev.meta_key());

        ev.modifiers = 0;
        assert!(!ev.shift_key());
        assert!(!ev.ctrl_key());
    }

    #[test]
    fn test_hit_rect_contains() {
        let r = HitRect::new(10, 20, 100, 50, 0);
        assert!(r.contains(10, 20));
        assert!(r.contains(50, 40));
        assert!(r.contains(109, 69));
        assert!(!r.contains(110, 20));
        assert!(!r.contains(10, 70));
        assert!(!r.contains(9, 20));
    }

    #[test]
    fn test_node_tree_ancestor_path() {
        let mut tree = NodeTree::new();
        tree.set_parent(1, 0);
        tree.set_parent(2, 1);
        tree.set_parent(3, 1);

        let path = tree.ancestor_path(2);
        assert_eq!(path, vec![0, 1, 2]);

        let path = tree.ancestor_path(0);
        assert_eq!(path, vec![0]);
    }

    #[test]
    fn test_add_remove_listener() {
        let mut d = EventDispatcher::new();
        d.add_event_listener(1, EventType::Click, 42, false);
        assert_eq!(d.listeners_for(1).len(), 1);

        let removed = d.remove_event_listener(1, EventType::Click, 42, false);
        assert!(removed);
        assert_eq!(d.listeners_for(1).len(), 0);
    }

    #[test]
    fn test_remove_nonexistent_listener() {
        let mut d = EventDispatcher::new();
        let removed = d.remove_event_listener(1, EventType::Click, 99, false);
        assert!(!removed);
    }

    #[test]
    fn test_hit_test() {
        let mut d = EventDispatcher::new();
        d.add_hit_box(HitRect::new(0, 0, 800, 600, 0));
        d.add_hit_box(HitRect::new(10, 10, 100, 50, 1));
        d.add_hit_box(HitRect::new(20, 20, 30, 30, 2));

        // Frontmost box at (25, 25) is node 2
        assert_eq!(d.hit_test(25, 25), Some(2));
        // At (5, 5) only root box
        assert_eq!(d.hit_test(5, 5), Some(0));
        // Outside all boxes
        assert_eq!(d.hit_test(900, 900), None);
    }

    #[test]
    fn test_dispatch_capture_bubble() {
        let mut d = setup_tree();
        // Listeners: capture on root(0), bubble on middle(1), bubble on leaf(2)
        d.add_event_listener(0, EventType::Click, 100, true);
        d.add_event_listener(1, EventType::Click, 101, false);
        d.add_event_listener(2, EventType::Click, 102, false);

        let mut ev = Event::new(EventType::Click, 2);
        d.dispatch(&mut ev);

        let invoked = d.take_invoked();
        // Capture on 0, then target on 2, then bubble on 1
        assert_eq!(invoked.len(), 3);
        assert_eq!(invoked[0].0, 100); // capture on root
        assert_eq!(invoked[1].0, 102); // target on leaf
        assert_eq!(invoked[2].0, 101); // bubble on middle
    }

    #[test]
    fn test_dispatch_stop_propagation() {
        let mut d = setup_tree();
        d.add_event_listener(0, EventType::Click, 100, true);
        d.add_event_listener(1, EventType::Click, 101, true);
        d.add_event_listener(2, EventType::Click, 102, false);

        // Stop at root capture
        let mut ev = Event::new(EventType::Click, 2);
        ev.propagation_stopped = false;
        d.dispatch(&mut ev);
        let invoked = d.take_invoked();
        // Root capture fires, then middle capture, then target
        assert_eq!(invoked.len(), 3);
    }

    #[test]
    fn test_dispatch_no_bubble() {
        let mut d = setup_tree();
        d.add_event_listener(0, EventType::Focus, 100, false);
        d.add_event_listener(2, EventType::Focus, 102, false);

        let mut ev = Event::new(EventType::Focus, 2);
        assert!(!ev.bubbles);
        d.dispatch(&mut ev);

        let invoked = d.take_invoked();
        // Only target fires (no bubble to root)
        assert_eq!(invoked.len(), 1);
        assert_eq!(invoked[0].0, 102);
    }

    #[test]
    fn test_dispatch_click_convenience() {
        let mut d = EventDispatcher::new();
        d.node_tree_mut().set_parent(1, 0);
        d.add_hit_box(HitRect::new(0, 0, 100, 100, 1));
        d.add_event_listener(1, EventType::Click, 50, false);

        let result = d.dispatch_click(50, 50, 0);
        assert!(result.is_some());
        let (target, prevented) = result.unwrap();
        assert_eq!(target, 1);
        assert!(!prevented);
    }

    #[test]
    fn test_dispatch_click_miss() {
        let d = &mut EventDispatcher::new();
        d.add_hit_box(HitRect::new(0, 0, 10, 10, 0));
        let result = d.dispatch_click(100, 100, 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_dispatch_key() {
        let mut d = EventDispatcher::new();
        d.add_event_listener(5, EventType::KeyDown, 77, false);

        let prevented = d.dispatch_key(5, EventType::KeyDown, 65, 0, 0);
        assert!(!prevented);
        let invoked = d.take_invoked();
        assert_eq!(invoked.len(), 1);
        assert_eq!(invoked[0].0, 77);
    }

    #[test]
    fn test_clear_listeners() {
        let mut d = EventDispatcher::new();
        d.add_event_listener(1, EventType::Click, 10, false);
        d.add_event_listener(1, EventType::KeyDown, 11, false);
        assert_eq!(d.listeners_for(1).len(), 2);
        d.clear_listeners(1);
        assert_eq!(d.listeners_for(1).len(), 0);
    }

    #[test]
    fn test_multiple_listeners_same_node() {
        let mut d = EventDispatcher::new();
        d.add_event_listener(1, EventType::Click, 10, false);
        d.add_event_listener(1, EventType::Click, 11, false);
        d.add_event_listener(1, EventType::KeyDown, 12, false);

        let mut ev = Event::new(EventType::Click, 1);
        d.dispatch(&mut ev);
        let invoked = d.take_invoked();
        // Two Click listeners, not the KeyDown one
        assert_eq!(invoked.len(), 2);
    }

    #[test]
    fn test_fp_conversion() {
        assert_eq!(fp_from_int(10), 640);
        assert_eq!(fp_to_int(640), 10);
        assert_eq!(fp_to_int(fp_from_int(42)), 42);
    }

    #[test]
    fn test_event_phase_default() {
        let phase = EventPhase::default();
        assert_eq!(phase, EventPhase::None);
    }

    #[test]
    fn test_node_tree_remove() {
        let mut tree = NodeTree::new();
        tree.set_parent(1, 0);
        assert!(tree.parent(1).is_some());
        tree.remove(1);
        assert!(tree.parent(1).is_none());
    }
}
