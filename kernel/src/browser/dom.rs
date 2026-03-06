//! DOM Tree
//!
//! Arena-based Document Object Model using `Vec<Node>` with `NodeId(usize)`
//! indices. Supports element creation, tree manipulation, traversal, and
//! querying by ID or tag name. No raw pointers.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

/// Index into the node arena
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NodeId(pub usize);

/// Type of DOM node
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NodeType {
    #[default]
    Document,
    Element,
    Text,
    Comment,
    DocumentType,
}

/// Data specific to Element nodes
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct ElementData {
    pub tag_name: String,
    pub attributes: BTreeMap<String, String>,
    pub namespace: Option<String>,
}

#[allow(dead_code)]
impl ElementData {
    pub fn new(tag_name: &str) -> Self {
        Self {
            tag_name: tag_name.to_string(),
            attributes: BTreeMap::new(),
            namespace: None,
        }
    }

    pub fn get_attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(|s| s.as_str())
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        self.attributes.insert(name.to_string(), value.to_string());
    }

    pub fn has_class(&self, class_name: &str) -> bool {
        self.attributes
            .get("class")
            .map(|c| c.split_whitespace().any(|w| w == class_name))
            .unwrap_or(false)
    }
}

/// A single node in the DOM tree
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub node_type: NodeType,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub element_data: Option<ElementData>,
    pub text_content: Option<String>,
}

/// Arena allocator for DOM nodes
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct NodeArena {
    nodes: Vec<Node>,
}

#[allow(dead_code)]
impl NodeArena {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Allocate a new node and return its ID
    pub fn alloc(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(node);
        id
    }

    /// Get a reference to a node by ID
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    /// Get a mutable reference to a node by ID
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0)
    }

    /// Number of nodes in the arena
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if arena is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The DOM Document
#[allow(dead_code)]
#[derive(Debug)]
pub struct Document {
    pub arena: NodeArena,
    pub root: NodeId,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl Document {
    /// Create a new empty document
    pub fn new() -> Self {
        let mut arena = NodeArena::new();
        let root = arena.alloc(Node {
            node_type: NodeType::Document,
            parent: None,
            children: Vec::new(),
            element_data: None,
            text_content: None,
        });
        Self { arena, root }
    }

    /// Create an element node
    pub fn create_element(&mut self, tag_name: &str) -> NodeId {
        self.arena.alloc(Node {
            node_type: NodeType::Element,
            parent: None,
            children: Vec::new(),
            element_data: Some(ElementData::new(tag_name)),
            text_content: None,
        })
    }

    /// Create an element node with attributes
    pub fn create_element_with_attrs(
        &mut self,
        tag_name: &str,
        attrs: BTreeMap<String, String>,
    ) -> NodeId {
        let mut ed = ElementData::new(tag_name);
        ed.attributes = attrs;
        self.arena.alloc(Node {
            node_type: NodeType::Element,
            parent: None,
            children: Vec::new(),
            element_data: Some(ed),
            text_content: None,
        })
    }

    /// Create a text node
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.arena.alloc(Node {
            node_type: NodeType::Text,
            parent: None,
            children: Vec::new(),
            element_data: None,
            text_content: Some(text.to_string()),
        })
    }

    /// Create a comment node
    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.arena.alloc(Node {
            node_type: NodeType::Comment,
            parent: None,
            children: Vec::new(),
            element_data: None,
            text_content: Some(text.to_string()),
        })
    }

    /// Create a document type node
    pub fn create_doctype(&mut self, name: &str) -> NodeId {
        self.arena.alloc(Node {
            node_type: NodeType::DocumentType,
            parent: None,
            children: Vec::new(),
            element_data: Some(ElementData::new(name)),
            text_content: None,
        })
    }

    /// Append a child node to a parent
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        // Set parent on child
        if let Some(child) = self.arena.get_mut(child_id) {
            child.parent = Some(parent_id);
        }
        // Add child to parent's children
        if let Some(parent) = self.arena.get_mut(parent_id) {
            parent.children.push(child_id);
        }
    }

    /// Remove a child from its parent
    pub fn remove_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        if let Some(parent) = self.arena.get_mut(parent_id) {
            parent.children.retain(|&id| id != child_id);
        }
        if let Some(child) = self.arena.get_mut(child_id) {
            child.parent = None;
        }
    }

    /// Insert a child before a reference node
    pub fn insert_before(&mut self, parent_id: NodeId, new_child_id: NodeId, ref_child_id: NodeId) {
        // Set parent
        if let Some(child) = self.arena.get_mut(new_child_id) {
            child.parent = Some(parent_id);
        }
        // Insert before reference
        if let Some(parent) = self.arena.get_mut(parent_id) {
            if let Some(pos) = parent.children.iter().position(|&id| id == ref_child_id) {
                parent.children.insert(pos, new_child_id);
            } else {
                parent.children.push(new_child_id);
            }
        }
    }

    /// Find an element by its "id" attribute
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        let mut result = None;
        self.walk(self.root, &mut |node_id| {
            if let Some(node) = self.arena.get(node_id) {
                if let Some(ref ed) = node.element_data {
                    if ed.get_attr("id") == Some(id) {
                        result = Some(node_id);
                    }
                }
            }
        });
        result
    }

    /// Find all elements with a given tag name
    pub fn get_elements_by_tag_name(&self, tag: &str) -> Vec<NodeId> {
        let mut results = Vec::new();
        self.walk(self.root, &mut |node_id| {
            if let Some(node) = self.arena.get(node_id) {
                if let Some(ref ed) = node.element_data {
                    if ed.tag_name == tag {
                        results.push(node_id);
                    }
                }
            }
        });
        results
    }

    /// Walk the tree depth-first, calling the callback on each node
    pub fn walk<F: FnMut(NodeId)>(&self, start: NodeId, callback: &mut F) {
        callback(start);
        if let Some(node) = self.arena.get(start) {
            let children = node.children.clone();
            for child_id in children {
                self.walk(child_id, callback);
            }
        }
    }

    /// Get all descendant NodeIds (depth-first)
    pub fn descendants(&self, start: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        self.walk(start, &mut |id| {
            if id != start {
                result.push(id);
            }
        });
        result
    }

    /// Get all ancestor NodeIds (from parent up to root)
    pub fn ancestors(&self, start: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut current = start;
        while let Some(node) = self.arena.get(current) {
            if let Some(parent_id) = node.parent {
                result.push(parent_id);
                current = parent_id;
            } else {
                break;
            }
        }
        result
    }

    /// Get the inner text of a node (concatenation of all text descendants)
    pub fn inner_text(&self, node_id: NodeId) -> String {
        let mut text = String::new();
        self.walk(node_id, &mut |id| {
            if let Some(node) = self.arena.get(id) {
                if node.node_type == NodeType::Text {
                    if let Some(ref t) = node.text_content {
                        text.push_str(t);
                    }
                }
            }
        });
        text
    }

    /// Generate outer HTML for debugging
    pub fn outer_html(&self, node_id: NodeId) -> String {
        let node = match self.arena.get(node_id) {
            Some(n) => n,
            None => return String::new(),
        };

        match node.node_type {
            NodeType::Document => {
                let mut s = String::new();
                let children = node.children.clone();
                for child in children {
                    s.push_str(&self.outer_html(child));
                }
                s
            }
            NodeType::Element => {
                let ed = node.element_data.as_ref().unwrap();
                let mut s = String::from("<");
                s.push_str(&ed.tag_name);
                for (k, v) in &ed.attributes {
                    s.push(' ');
                    s.push_str(k);
                    s.push_str("=\"");
                    s.push_str(v);
                    s.push('"');
                }
                s.push('>');
                let children = node.children.clone();
                for child in children {
                    s.push_str(&self.outer_html(child));
                }
                s.push_str("</");
                s.push_str(&ed.tag_name);
                s.push('>');
                s
            }
            NodeType::Text => node.text_content.clone().unwrap_or_default(),
            NodeType::Comment => {
                let mut s = String::from("<!--");
                if let Some(ref t) = node.text_content {
                    s.push_str(t);
                }
                s.push_str("-->");
                s
            }
            NodeType::DocumentType => {
                let name = node
                    .element_data
                    .as_ref()
                    .map(|e| e.tag_name.as_str())
                    .unwrap_or("html");
                let mut s = String::from("<!DOCTYPE ");
                s.push_str(name);
                s.push('>');
                s
            }
        }
    }

    /// Get the tag name of an element node
    pub fn tag_name(&self, node_id: NodeId) -> Option<&str> {
        self.arena
            .get(node_id)
            .and_then(|n| n.element_data.as_ref())
            .map(|ed| ed.tag_name.as_str())
    }

    /// Get an attribute value from an element node
    pub fn get_attribute(&self, node_id: NodeId, attr: &str) -> Option<String> {
        self.arena
            .get(node_id)
            .and_then(|n| n.element_data.as_ref())
            .and_then(|ed| ed.attributes.get(attr))
            .cloned()
    }

    /// Count total nodes in the arena
    pub fn node_count(&self) -> usize {
        self.arena.len()
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_new_document() {
        let doc = Document::new();
        assert_eq!(doc.root, NodeId(0));
        assert_eq!(doc.arena.len(), 1);
    }

    #[test]
    fn test_create_element() {
        let mut doc = Document::new();
        let div = doc.create_element("div");
        assert_eq!(div, NodeId(1));
        assert_eq!(doc.tag_name(div), Some("div"));
    }

    #[test]
    fn test_create_text() {
        let mut doc = Document::new();
        let text = doc.create_text("hello");
        let node = doc.arena.get(text).unwrap();
        assert_eq!(node.node_type, NodeType::Text);
        assert_eq!(node.text_content.as_deref(), Some("hello"));
    }

    #[test]
    fn test_create_comment() {
        let mut doc = Document::new();
        let c = doc.create_comment("a comment");
        let node = doc.arena.get(c).unwrap();
        assert_eq!(node.node_type, NodeType::Comment);
        assert_eq!(node.text_content.as_deref(), Some("a comment"));
    }

    #[test]
    fn test_append_child() {
        let mut doc = Document::new();
        let div = doc.create_element("div");
        doc.append_child(doc.root, div);
        let root = doc.arena.get(doc.root).unwrap();
        assert_eq!(root.children, vec![div]);
        let div_node = doc.arena.get(div).unwrap();
        assert_eq!(div_node.parent, Some(doc.root));
    }

    #[test]
    fn test_remove_child() {
        let mut doc = Document::new();
        let div = doc.create_element("div");
        doc.append_child(doc.root, div);
        doc.remove_child(doc.root, div);
        let root = doc.arena.get(doc.root).unwrap();
        assert!(root.children.is_empty());
    }

    #[test]
    fn test_insert_before() {
        let mut doc = Document::new();
        let a = doc.create_element("a");
        let b = doc.create_element("b");
        let c = doc.create_element("c");
        doc.append_child(doc.root, a);
        doc.append_child(doc.root, c);
        doc.insert_before(doc.root, b, c);
        let root = doc.arena.get(doc.root).unwrap();
        assert_eq!(root.children, vec![a, b, c]);
    }

    #[test]
    fn test_get_element_by_id() {
        let mut doc = Document::new();
        let mut attrs = BTreeMap::new();
        attrs.insert("id".to_string(), "main".to_string());
        let div = doc.create_element_with_attrs("div", attrs);
        doc.append_child(doc.root, div);
        assert_eq!(doc.get_element_by_id("main"), Some(div));
        assert_eq!(doc.get_element_by_id("none"), None);
    }

    #[test]
    fn test_get_elements_by_tag_name() {
        let mut doc = Document::new();
        let p1 = doc.create_element("p");
        let p2 = doc.create_element("p");
        let div = doc.create_element("div");
        doc.append_child(doc.root, p1);
        doc.append_child(doc.root, div);
        doc.append_child(div, p2);
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 2);
    }

    #[test]
    fn test_inner_text() {
        let mut doc = Document::new();
        let p = doc.create_element("p");
        let t = doc.create_text("hello world");
        doc.append_child(doc.root, p);
        doc.append_child(p, t);
        assert_eq!(doc.inner_text(p), "hello world");
    }

    #[test]
    fn test_inner_text_nested() {
        let mut doc = Document::new();
        let div = doc.create_element("div");
        let span = doc.create_element("span");
        let t1 = doc.create_text("hello ");
        let t2 = doc.create_text("world");
        doc.append_child(doc.root, div);
        doc.append_child(div, t1);
        doc.append_child(div, span);
        doc.append_child(span, t2);
        assert_eq!(doc.inner_text(div), "hello world");
    }

    #[test]
    fn test_outer_html_element() {
        let mut doc = Document::new();
        let p = doc.create_element("p");
        let t = doc.create_text("hi");
        doc.append_child(p, t);
        assert_eq!(doc.outer_html(p), "<p>hi</p>");
    }

    #[test]
    fn test_outer_html_with_attrs() {
        let mut doc = Document::new();
        let mut attrs = BTreeMap::new();
        attrs.insert("class".to_string(), "big".to_string());
        let div = doc.create_element_with_attrs("div", attrs);
        assert_eq!(doc.outer_html(div), "<div class=\"big\"></div>");
    }

    #[test]
    fn test_descendants() {
        let mut doc = Document::new();
        let a = doc.create_element("a");
        let b = doc.create_element("b");
        let c = doc.create_element("c");
        doc.append_child(doc.root, a);
        doc.append_child(a, b);
        doc.append_child(a, c);
        let desc = doc.descendants(a);
        assert_eq!(desc, vec![b, c]);
    }

    #[test]
    fn test_ancestors() {
        let mut doc = Document::new();
        let a = doc.create_element("a");
        let b = doc.create_element("b");
        doc.append_child(doc.root, a);
        doc.append_child(a, b);
        let anc = doc.ancestors(b);
        assert_eq!(anc, vec![a, doc.root]);
    }

    #[test]
    fn test_walk() {
        let mut doc = Document::new();
        let a = doc.create_element("a");
        let b = doc.create_element("b");
        doc.append_child(doc.root, a);
        doc.append_child(a, b);
        let mut visited = Vec::new();
        doc.walk(doc.root, &mut |id| visited.push(id));
        assert_eq!(visited, vec![doc.root, a, b]);
    }

    #[test]
    fn test_node_count() {
        let mut doc = Document::new();
        assert_eq!(doc.node_count(), 1);
        let _a = doc.create_element("a");
        assert_eq!(doc.node_count(), 2);
    }

    #[test]
    fn test_doctype_node() {
        let mut doc = Document::new();
        let dt = doc.create_doctype("html");
        let node = doc.arena.get(dt).unwrap();
        assert_eq!(node.node_type, NodeType::DocumentType);
    }

    #[test]
    fn test_element_data_has_class() {
        let mut ed = ElementData::new("div");
        ed.set_attr("class", "foo bar baz");
        assert!(ed.has_class("foo"));
        assert!(ed.has_class("bar"));
        assert!(!ed.has_class("qux"));
    }

    #[test]
    fn test_element_data_get_attr() {
        let mut ed = ElementData::new("a");
        ed.set_attr("href", "/home");
        assert_eq!(ed.get_attr("href"), Some("/home"));
        assert_eq!(ed.get_attr("none"), None);
    }

    #[test]
    fn test_get_attribute() {
        let mut doc = Document::new();
        let mut attrs = BTreeMap::new();
        attrs.insert("src".to_string(), "img.png".to_string());
        let img = doc.create_element_with_attrs("img", attrs);
        assert_eq!(doc.get_attribute(img, "src"), Some("img.png".to_string()));
    }

    #[test]
    fn test_empty_arena() {
        let arena = NodeArena::new();
        assert!(arena.is_empty());
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn test_multiple_children() {
        let mut doc = Document::new();
        let a = doc.create_element("a");
        let b = doc.create_element("b");
        let c = doc.create_element("c");
        let d = doc.create_element("d");
        doc.append_child(doc.root, a);
        doc.append_child(doc.root, b);
        doc.append_child(doc.root, c);
        doc.append_child(doc.root, d);
        let root = doc.arena.get(doc.root).unwrap();
        assert_eq!(root.children.len(), 4);
    }

    #[test]
    fn test_outer_html_comment() {
        let mut doc = Document::new();
        let c = doc.create_comment("test");
        assert_eq!(doc.outer_html(c), "<!--test-->");
    }

    #[test]
    fn test_outer_html_doctype() {
        let mut doc = Document::new();
        let dt = doc.create_doctype("html");
        assert_eq!(doc.outer_html(dt), "<!DOCTYPE html>");
    }

    #[test]
    fn test_default_document() {
        let doc = Document::default();
        assert_eq!(doc.node_count(), 1);
    }

    #[test]
    fn test_deep_nesting() {
        let mut doc = Document::new();
        let mut parent = doc.root;
        for i in 0..10 {
            let child = doc.create_element("div");
            doc.append_child(parent, child);
            // Create a text child for inner_text testing
            if i == 9 {
                let t = doc.create_text("deep");
                doc.append_child(child, t);
            }
            parent = child;
        }
        assert_eq!(doc.inner_text(doc.root), "deep");
    }
}
