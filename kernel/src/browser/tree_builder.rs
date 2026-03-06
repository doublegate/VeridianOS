//! HTML Tree Builder
//!
//! Consumes tokens from the HTML tokenizer and builds a DOM tree.
//! Implements a simplified version of the HTML5 tree construction
//! algorithm with insertion modes, auto-closing, and formatting
//! element reconstruction.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use super::{
    dom::{Document, NodeId, NodeType},
    html_tokenizer::{Attribute, HtmlTokenizer, Token},
};

/// Insertion modes for the tree builder
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionMode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    InTable,
    InTableBody,
    InRow,
    InCell,
    AfterBody,
    AfterAfterBody,
}

/// Elements that auto-close a <p> tag
#[allow(dead_code)]
const P_CLOSING_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

/// Elements that auto-close themselves
#[allow(dead_code)]
const SELF_CLOSING_TAGS: &[(&str, &[&str])] = &[
    ("li", &["li"]),
    ("dt", &["dt", "dd"]),
    ("dd", &["dt", "dd"]),
    ("td", &["td", "th"]),
    ("th", &["td", "th"]),
    ("tr", &["tr"]),
    ("thead", &["tbody", "tfoot"]),
    ("tbody", &["tbody", "tfoot"]),
    ("tfoot", &["tbody"]),
    ("option", &["option"]),
    ("optgroup", &["optgroup"]),
];

/// Formatting element names
#[allow(dead_code)]
const FORMATTING_ELEMENTS: &[&str] = &[
    "b", "big", "code", "em", "font", "i", "s", "small", "strike", "strong", "tt", "u",
];

/// Active formatting entry
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FormattingEntry {
    tag_name: String,
    node_id: NodeId,
    attrs: BTreeMap<String, String>,
}

/// HTML tree builder that constructs a DOM from tokens
#[allow(dead_code)]
pub struct TreeBuilder {
    document: Document,
    insertion_mode: InsertionMode,
    open_elements: Vec<NodeId>,
    head_element: Option<NodeId>,
    body_element: Option<NodeId>,
    foster_parenting: bool,
    active_formatting: Vec<Option<FormattingEntry>>,
    frameset_ok: bool,
}

#[allow(dead_code)]
impl TreeBuilder {
    /// Create a new tree builder
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            insertion_mode: InsertionMode::Initial,
            open_elements: Vec::new(),
            head_element: None,
            body_element: None,
            foster_parenting: false,
            active_formatting: Vec::new(),
            frameset_ok: true,
        }
    }

    /// Build a DOM tree from an HTML string
    pub fn build(html: &str) -> Document {
        let mut tokenizer = HtmlTokenizer::from_text(html);
        let mut builder = Self::new();
        loop {
            let token = tokenizer.next_token();
            let is_eof = token == Token::Eof;
            builder.process_token(token);
            if is_eof {
                break;
            }
        }
        builder.document
    }

    /// Get the current insertion point (last open element or document root)
    fn current_node(&self) -> NodeId {
        self.open_elements
            .last()
            .copied()
            .unwrap_or(self.document.root)
    }

    /// Get the tag name of the current node
    fn current_tag_name(&self) -> Option<String> {
        let id = self.current_node();
        self.document.tag_name(id).map(|s| s.to_string())
    }

    /// Convert tokenizer attributes to a BTreeMap
    fn attrs_to_map(attrs: &[Attribute]) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for attr in attrs {
            map.insert(attr.name.clone(), attr.value.clone());
        }
        map
    }

    /// Insert an element and push to open elements
    fn insert_element(&mut self, tag_name: &str, attrs: &[Attribute]) -> NodeId {
        let attr_map = Self::attrs_to_map(attrs);
        let node_id = self.document.create_element_with_attrs(tag_name, attr_map);
        let parent = self.current_node();
        self.document.append_child(parent, node_id);
        self.open_elements.push(node_id);
        node_id
    }

    /// Insert a void element (no push to open elements)
    fn insert_void_element(&mut self, tag_name: &str, attrs: &[Attribute]) -> NodeId {
        let attr_map = Self::attrs_to_map(attrs);
        let node_id = self.document.create_element_with_attrs(tag_name, attr_map);
        let parent = self.current_node();
        self.document.append_child(parent, node_id);
        node_id
    }

    /// Insert text at the current insertion point
    fn insert_text(&mut self, ch: char) {
        let parent = self.current_node();
        // Try to append to existing text node
        if let Some(node) = self.document.arena.get(parent) {
            if let Some(&last_child_id) = node.children.last() {
                if let Some(last_child) = self.document.arena.get_mut(last_child_id) {
                    if last_child.node_type == NodeType::Text {
                        if let Some(ref mut text) = last_child.text_content {
                            text.push(ch);
                            return;
                        }
                    }
                }
            }
        }
        // Create new text node
        let text_id = self.document.create_text(&String::from(ch));
        self.document.append_child(parent, text_id);
    }

    /// Auto-close elements when a new tag implies closing
    fn auto_close_for_tag(&mut self, tag_name: &str) {
        // Close <p> if needed
        if P_CLOSING_TAGS.contains(&tag_name) {
            self.close_p_if_in_scope();
        }

        // Check self-closing patterns (li, dt, dd, etc.)
        for &(new_tag, closers) in SELF_CLOSING_TAGS {
            if new_tag == tag_name {
                for &closer in closers {
                    if self.current_tag_name().as_deref() == Some(closer) {
                        self.open_elements.pop();
                        break;
                    }
                }
                break;
            }
        }
    }

    /// Close <p> if one is in button scope
    fn close_p_if_in_scope(&mut self) {
        if self.has_in_scope("p") {
            self.pop_until("p");
        }
    }

    /// Check if a tag is in scope
    fn has_in_scope(&self, tag: &str) -> bool {
        for &id in self.open_elements.iter().rev() {
            if self.document.tag_name(id) == Some(tag) {
                return true;
            }
            // Scope-breaking elements
            let name = self.document.tag_name(id).unwrap_or("");
            if matches!(
                name,
                "applet"
                    | "caption"
                    | "html"
                    | "table"
                    | "td"
                    | "th"
                    | "marquee"
                    | "object"
                    | "template"
            ) {
                return false;
            }
        }
        false
    }

    /// Pop elements until we find the given tag
    fn pop_until(&mut self, tag: &str) {
        while let Some(&id) = self.open_elements.last() {
            self.open_elements.pop();
            if self.document.tag_name(id) == Some(tag) {
                break;
            }
        }
    }

    /// Push a formatting marker
    fn push_formatting_marker(&mut self) {
        self.active_formatting.push(None);
    }

    /// Add a formatting element
    fn add_formatting_element(&mut self, tag_name: &str, node_id: NodeId, attrs: &[Attribute]) {
        let entry = FormattingEntry {
            tag_name: tag_name.to_string(),
            node_id,
            attrs: Self::attrs_to_map(attrs),
        };
        self.active_formatting.push(Some(entry));
    }

    /// Reconstruct active formatting elements
    fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }

        // Find the last marker or start
        let last = self.active_formatting.len() - 1;
        if self.active_formatting[last].is_none() {
            return;
        }

        // Check if the last entry's element is on the stack
        if let Some(ref entry) = self.active_formatting[last] {
            if self.open_elements.contains(&entry.node_id) {
                return;
            }
        }

        // Reconstruct: re-insert formatting elements
        let entries_to_reopen: Vec<_> = self
            .active_formatting
            .iter()
            .rev()
            .take_while(|e| e.is_some())
            .filter_map(|e| e.clone())
            .collect();

        for entry in entries_to_reopen.into_iter().rev() {
            if !self.open_elements.contains(&entry.node_id) {
                let attrs: Vec<Attribute> = entry
                    .attrs
                    .iter()
                    .map(|(k, v)| Attribute {
                        name: k.clone(),
                        value: v.clone(),
                    })
                    .collect();
                let new_id = self.insert_element(&entry.tag_name, &attrs);
                // Update the formatting list
                for e in self.active_formatting.iter_mut().flatten() {
                    if e.node_id == entry.node_id {
                        e.node_id = new_id;
                    }
                }
            }
        }
    }

    /// Is the given tag a formatting element
    fn is_formatting_element(tag: &str) -> bool {
        FORMATTING_ELEMENTS.contains(&tag)
    }

    /// Process a single token
    pub fn process_token(&mut self, token: Token) {
        match self.insertion_mode {
            InsertionMode::Initial => self.process_initial(token),
            InsertionMode::BeforeHtml => self.process_before_html(token),
            InsertionMode::BeforeHead => self.process_before_head(token),
            InsertionMode::InHead => self.process_in_head(token),
            InsertionMode::AfterHead => self.process_after_head(token),
            InsertionMode::InBody => self.process_in_body(token),
            InsertionMode::InTable => self.process_in_table(token),
            InsertionMode::InTableBody => self.process_in_table_body(token),
            InsertionMode::InRow => self.process_in_row(token),
            InsertionMode::InCell => self.process_in_cell(token),
            InsertionMode::AfterBody => self.process_after_body(token),
            InsertionMode::AfterAfterBody => self.process_after_after_body(token),
        }
    }

    fn process_initial(&mut self, token: Token) {
        match token {
            Token::Doctype(name) => {
                let dt = self.document.create_doctype(&name);
                self.document.append_child(self.document.root, dt);
                self.insertion_mode = InsertionMode::BeforeHtml;
            }
            Token::Character(c) if c.is_whitespace() => {
                // Ignore whitespace in initial mode
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                self.document.append_child(self.document.root, c);
            }
            _ => {
                // No doctype, switch to before html and reprocess
                self.insertion_mode = InsertionMode::BeforeHtml;
                self.process_token(token);
            }
        }
    }

    fn process_before_html(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) if name == "html" => {
                let id = self.insert_element(name, attrs);
                let _ = id;
                self.insertion_mode = InsertionMode::BeforeHead;
            }
            Token::Character(c) if c.is_whitespace() => {
                // ignore
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                self.document.append_child(self.document.root, c);
            }
            _ => {
                // Implied <html>
                let html = self.document.create_element("html");
                self.document.append_child(self.document.root, html);
                self.open_elements.push(html);
                self.insertion_mode = InsertionMode::BeforeHead;
                self.process_token(token);
            }
        }
    }

    fn process_before_head(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) if name == "head" => {
                let id = self.insert_element(name, attrs);
                self.head_element = Some(id);
                self.insertion_mode = InsertionMode::InHead;
            }
            Token::Character(c) if c.is_whitespace() => {
                // ignore
            }
            _ => {
                // Implied <head>
                let head = self.document.create_element("head");
                let parent = self.current_node();
                self.document.append_child(parent, head);
                self.open_elements.push(head);
                self.head_element = Some(head);
                self.insertion_mode = InsertionMode::InHead;
                self.process_token(token);
            }
        }
    }

    fn process_in_head(&mut self, token: Token) {
        match token {
            Token::Character(c) if c.is_whitespace() => {
                self.insert_text(c);
            }
            Token::Character(c) => {
                // Non-whitespace text inside <title>/<style>/<script>
                let current = self.current_node();
                let is_raw_text = self
                    .document
                    .arena
                    .get(current)
                    .and_then(|n| n.element_data.as_ref())
                    .map(|ed| {
                        matches!(
                            ed.tag_name.as_str(),
                            "title" | "style" | "script" | "noscript"
                        )
                    })
                    .unwrap_or(false);
                if is_raw_text {
                    self.insert_text(c);
                } else {
                    self.open_elements.pop();
                    self.insertion_mode = InsertionMode::AfterHead;
                    self.process_token(Token::Character(c));
                }
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                let parent = self.current_node();
                self.document.append_child(parent, c);
            }
            Token::StartTag(ref name, ref attrs, self_closing) => {
                match name.as_str() {
                    "title" | "style" | "script" | "noscript" => {
                        if self_closing {
                            self.insert_void_element(name, attrs);
                        } else {
                            self.insert_element(name, attrs);
                        }
                    }
                    "meta" | "link" | "base" => {
                        self.insert_void_element(name, attrs);
                    }
                    "head" => {
                        // Ignore duplicate head
                    }
                    _ => {
                        // Implied </head>
                        self.open_elements.pop(); // pop head
                        self.insertion_mode = InsertionMode::AfterHead;
                        self.process_token(Token::StartTag(
                            name.clone(),
                            attrs.clone(),
                            self_closing,
                        ));
                    }
                }
            }
            Token::EndTag(ref name) => {
                match name.as_str() {
                    "head" => {
                        self.open_elements.pop();
                        self.insertion_mode = InsertionMode::AfterHead;
                    }
                    "title" | "style" | "script" | "noscript" => {
                        self.open_elements.pop();
                    }
                    _ => {
                        // Implied </head>
                        self.open_elements.pop();
                        self.insertion_mode = InsertionMode::AfterHead;
                    }
                }
            }
            Token::Eof => {
                self.open_elements.pop();
                self.insertion_mode = InsertionMode::AfterHead;
                self.process_token(Token::Eof);
            }
            _ => {
                self.open_elements.pop();
                self.insertion_mode = InsertionMode::AfterHead;
                self.process_token(token);
            }
        }
    }

    fn process_after_head(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) if name == "body" => {
                let id = self.insert_element(name, attrs);
                self.body_element = Some(id);
                self.insertion_mode = InsertionMode::InBody;
            }
            Token::Character(c) if c.is_whitespace() => {
                self.insert_text(c);
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                let parent = self.current_node();
                self.document.append_child(parent, c);
            }
            _ => {
                // Implied <body>
                let body = self.document.create_element("body");
                let parent = self.current_node();
                self.document.append_child(parent, body);
                self.open_elements.push(body);
                self.body_element = Some(body);
                self.insertion_mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }

    fn process_in_body(&mut self, token: Token) {
        match token {
            Token::Character(c) => {
                self.reconstruct_formatting();
                self.insert_text(c);
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                let parent = self.current_node();
                self.document.append_child(parent, c);
            }
            Token::StartTag(ref name, ref attrs, self_closing) => {
                self.process_in_body_start_tag(name.clone(), attrs.clone(), self_closing);
            }
            Token::EndTag(ref name) => {
                self.process_in_body_end_tag(name.clone());
            }
            Token::Eof => {
                // Done
            }
            Token::Doctype(_) => {
                // Ignore in body
            }
        }
    }

    fn process_in_body_start_tag(
        &mut self,
        name: String,
        attrs: Vec<Attribute>,
        self_closing: bool,
    ) {
        match name.as_str() {
            "html" => {
                // Merge attributes onto existing html element
            }
            "body" => {
                // Merge attributes onto existing body element
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.auto_close_for_tag(&name);
                // Auto-close other heading if open
                if let Some(tag) = self.current_tag_name() {
                    if matches!(tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                        self.open_elements.pop();
                    }
                }
                self.insert_element(&name, &attrs);
            }
            "p" | "div" | "section" | "article" | "aside" | "nav" | "header" | "footer"
            | "main" | "address" | "blockquote" | "figure" | "figcaption" | "details"
            | "summary" | "ul" | "ol" | "dl" | "pre" | "fieldset" | "form" | "hgroup" => {
                self.auto_close_for_tag(&name);
                self.insert_element(&name, &attrs);
            }
            "li" => {
                self.auto_close_for_tag("li");
                self.insert_element(&name, &attrs);
            }
            "dt" | "dd" => {
                self.auto_close_for_tag(&name);
                self.insert_element(&name, &attrs);
            }
            "table" => {
                self.close_p_if_in_scope();
                self.insert_element(&name, &attrs);
                self.insertion_mode = InsertionMode::InTable;
            }
            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong"
            | "tt" | "u" => {
                self.reconstruct_formatting();
                let id = self.insert_element(&name, &attrs);
                self.add_formatting_element(&name, id, &attrs);
            }
            "a" => {
                self.reconstruct_formatting();
                let id = self.insert_element(&name, &attrs);
                self.add_formatting_element(&name, id, &attrs);
            }
            "br" | "hr" | "img" | "input" | "meta" | "link" | "embed" | "source" | "track"
            | "wbr" | "area" | "col" | "param" | "base" => {
                self.reconstruct_formatting();
                self.insert_void_element(&name, &attrs);
            }
            "span" | "label" | "abbr" | "cite" | "dfn" | "kbd" | "mark" | "q" | "sub" | "sup"
            | "time" | "var" | "data" | "ruby" | "rt" | "rp" | "bdi" | "bdo" | "output"
            | "progress" | "meter" | "slot" | "canvas" | "dialog" | "picture" | "video"
            | "audio" | "map" | "object" | "iframe" | "button" | "select" | "textarea" => {
                self.reconstruct_formatting();
                if self_closing {
                    self.insert_void_element(&name, &attrs);
                } else {
                    self.insert_element(&name, &attrs);
                }
            }
            _ => {
                self.reconstruct_formatting();
                if self_closing {
                    self.insert_void_element(&name, &attrs);
                } else {
                    self.insert_element(&name, &attrs);
                }
            }
        }
    }

    fn process_in_body_end_tag(&mut self, name: String) {
        match name.as_str() {
            "body" => {
                self.insertion_mode = InsertionMode::AfterBody;
            }
            "html" => {
                self.insertion_mode = InsertionMode::AfterBody;
                self.process_token(Token::EndTag(name));
            }
            "p" => {
                if !self.has_in_scope("p") {
                    // Create empty <p>
                    let p = self.document.create_element("p");
                    let parent = self.current_node();
                    self.document.append_child(parent, p);
                } else {
                    self.pop_until("p");
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                // Pop until any heading
                let mut found = false;
                for &id in self.open_elements.iter().rev() {
                    if let Some(tag) = self.document.tag_name(id) {
                        if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                            found = true;
                            break;
                        }
                    }
                }
                if found {
                    while let Some(&id) = self.open_elements.last() {
                        self.open_elements.pop();
                        if let Some(tag) = self.document.tag_name(id) {
                            if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                                break;
                            }
                        }
                    }
                }
            }
            "div" | "section" | "article" | "aside" | "nav" | "header" | "footer" | "main"
            | "address" | "blockquote" | "figure" | "figcaption" | "details" | "summary" | "ul"
            | "ol" | "dl" | "pre" | "fieldset" | "form" | "hgroup" | "li" | "dd" | "dt" => {
                if self.has_in_scope(&name) {
                    self.pop_until(&name);
                }
            }
            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong"
            | "tt" | "u" | "a" => {
                // Adoption agency algorithm (simplified)
                if self.has_in_scope(&name) {
                    self.pop_until(&name);
                    // Remove from active formatting
                    self.active_formatting.retain(|e| {
                        if let Some(ref entry) = e {
                            entry.tag_name != name
                        } else {
                            true
                        }
                    });
                }
            }
            _ => {
                // Any other end tag
                self.pop_matching_end_tag(&name);
            }
        }
    }

    /// Pop elements to close a matching start tag
    fn pop_matching_end_tag(&mut self, tag: &str) {
        for i in (0..self.open_elements.len()).rev() {
            let id = self.open_elements[i];
            if self.document.tag_name(id) == Some(tag) {
                self.open_elements.truncate(i);
                return;
            }
            // Stop at special elements
            if let Some(name) = self.document.tag_name(id) {
                if matches!(
                    name,
                    "address"
                        | "applet"
                        | "area"
                        | "article"
                        | "aside"
                        | "base"
                        | "basefont"
                        | "bgsound"
                        | "blockquote"
                        | "body"
                        | "br"
                        | "button"
                        | "caption"
                        | "center"
                        | "col"
                        | "colgroup"
                        | "dd"
                        | "details"
                        | "dir"
                        | "div"
                        | "dl"
                        | "dt"
                        | "embed"
                        | "fieldset"
                        | "figcaption"
                        | "figure"
                        | "footer"
                        | "form"
                        | "frame"
                        | "frameset"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "head"
                        | "header"
                        | "hgroup"
                        | "hr"
                        | "html"
                        | "iframe"
                        | "img"
                        | "input"
                        | "li"
                        | "link"
                        | "listing"
                        | "main"
                        | "marquee"
                        | "menu"
                        | "meta"
                        | "nav"
                        | "noembed"
                        | "noframes"
                        | "noscript"
                        | "object"
                        | "ol"
                        | "p"
                        | "param"
                        | "plaintext"
                        | "pre"
                        | "script"
                        | "section"
                        | "select"
                        | "source"
                        | "style"
                        | "summary"
                        | "table"
                        | "tbody"
                        | "td"
                        | "template"
                        | "textarea"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "title"
                        | "tr"
                        | "track"
                        | "ul"
                        | "wbr"
                ) {
                    return;
                }
            }
        }
    }

    fn process_in_table(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) => {
                match name.as_str() {
                    "caption" | "colgroup" | "col" => {
                        self.insert_element(name, attrs);
                    }
                    "thead" | "tbody" | "tfoot" => {
                        self.insert_element(name, attrs);
                        self.insertion_mode = InsertionMode::InTableBody;
                    }
                    "tr" => {
                        // Implied <tbody>
                        let tbody = self.document.create_element("tbody");
                        let parent = self.current_node();
                        self.document.append_child(parent, tbody);
                        self.open_elements.push(tbody);
                        self.insert_element(name, attrs);
                        self.insertion_mode = InsertionMode::InRow;
                    }
                    "td" | "th" => {
                        // Implied <tbody><tr>
                        let tbody = self.document.create_element("tbody");
                        let parent = self.current_node();
                        self.document.append_child(parent, tbody);
                        self.open_elements.push(tbody);
                        let tr = self.document.create_element("tr");
                        self.document.append_child(tbody, tr);
                        self.open_elements.push(tr);
                        self.insert_element(name, attrs);
                        self.insertion_mode = InsertionMode::InCell;
                    }
                    _ => {
                        // Foster parenting: process as in body
                        self.process_in_body(Token::StartTag(name.clone(), attrs.clone(), false));
                    }
                }
            }
            Token::EndTag(ref name) if name == "table" => {
                self.pop_until("table");
                self.insertion_mode = InsertionMode::InBody;
            }
            Token::Character(_) | Token::Comment(_) => {
                self.process_in_body(token);
            }
            Token::Eof => {}
            _ => {
                self.process_in_body(token);
            }
        }
    }

    fn process_in_table_body(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) => match name.as_str() {
                "tr" => {
                    self.insert_element(name, attrs);
                    self.insertion_mode = InsertionMode::InRow;
                }
                "td" | "th" => {
                    let tr = self.document.create_element("tr");
                    let parent = self.current_node();
                    self.document.append_child(parent, tr);
                    self.open_elements.push(tr);
                    self.insert_element(name, attrs);
                    self.insertion_mode = InsertionMode::InCell;
                }
                _ => {
                    self.process_in_table(token);
                }
            },
            Token::EndTag(ref name) => {
                match name.as_str() {
                    "thead" | "tbody" | "tfoot" => {
                        self.pop_until(name);
                        self.insertion_mode = InsertionMode::InTable;
                    }
                    "table" => {
                        // Close table body first
                        if let Some(tag) = self.current_tag_name() {
                            if matches!(tag.as_str(), "thead" | "tbody" | "tfoot") {
                                self.open_elements.pop();
                            }
                        }
                        self.insertion_mode = InsertionMode::InTable;
                        self.process_token(token);
                    }
                    _ => {
                        self.process_in_table(token);
                    }
                }
            }
            _ => {
                self.process_in_table(token);
            }
        }
    }

    fn process_in_row(&mut self, token: Token) {
        match token {
            Token::StartTag(ref name, ref attrs, _) => match name.as_str() {
                "td" | "th" => {
                    self.insert_element(name, attrs);
                    self.insertion_mode = InsertionMode::InCell;
                    self.push_formatting_marker();
                }
                _ => {
                    self.process_in_table(token);
                }
            },
            Token::EndTag(ref name) => match name.as_str() {
                "tr" => {
                    self.pop_until("tr");
                    self.insertion_mode = InsertionMode::InTableBody;
                }
                "table" => {
                    self.pop_until("tr");
                    self.insertion_mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
                _ => {
                    self.process_in_table(token);
                }
            },
            _ => {
                self.process_in_table(token);
            }
        }
    }

    fn process_in_cell(&mut self, token: Token) {
        match token {
            Token::EndTag(ref name) if name == "td" || name == "th" => {
                self.pop_until(name);
                self.insertion_mode = InsertionMode::InRow;
            }
            Token::StartTag(ref name, _, _) if matches!(name.as_str(), "td" | "th" | "tr") => {
                // Close current cell
                if let Some(tag) = self.current_tag_name() {
                    if tag == "td" || tag == "th" {
                        self.open_elements.pop();
                    }
                }
                self.insertion_mode = InsertionMode::InRow;
                self.process_token(token);
            }
            Token::EndTag(ref name) if name == "table" => {
                // Close cell, close row
                if let Some(tag) = self.current_tag_name() {
                    if tag == "td" || tag == "th" {
                        self.open_elements.pop();
                    }
                }
                self.insertion_mode = InsertionMode::InRow;
                self.process_token(token);
            }
            _ => {
                self.process_in_body(token);
            }
        }
    }

    fn process_after_body(&mut self, token: Token) {
        match token {
            Token::Character(c) if c.is_whitespace() => {
                self.insert_text(c);
            }
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                self.document.append_child(self.document.root, c);
            }
            Token::EndTag(ref name) if name == "html" => {
                self.insertion_mode = InsertionMode::AfterAfterBody;
            }
            Token::Eof => {}
            _ => {
                self.insertion_mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }

    fn process_after_after_body(&mut self, token: Token) {
        match token {
            Token::Character(c) if c.is_whitespace() => {}
            Token::Comment(text) => {
                let c = self.document.create_comment(&text);
                self.document.append_child(self.document.root, c);
            }
            Token::Eof => {}
            _ => {
                self.insertion_mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_empty_document() {
        let doc = TreeBuilder::build("");
        assert!(doc.node_count() >= 1);
    }

    #[test]
    fn test_simple_paragraph() {
        let doc = TreeBuilder::build("<p>Hello</p>");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 1);
        assert_eq!(doc.inner_text(ps[0]), "Hello");
    }

    #[test]
    fn test_full_html_document() {
        let doc = TreeBuilder::build(
            "<!DOCTYPE html><html><head><title>Test</title></head><body><p>Hello</p></body></html>",
        );
        let titles = doc.get_elements_by_tag_name("title");
        assert_eq!(titles.len(), 1);
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 1);
    }

    #[test]
    fn test_implied_html_head_body() {
        let doc = TreeBuilder::build("<p>Text</p>");
        // Should have implied html, head, body
        let htmls = doc.get_elements_by_tag_name("html");
        assert!(!htmls.is_empty());
        let bodies = doc.get_elements_by_tag_name("body");
        assert!(!bodies.is_empty());
    }

    #[test]
    fn test_nested_elements() {
        let doc = TreeBuilder::build("<div><span>A</span><span>B</span></div>");
        let spans = doc.get_elements_by_tag_name("span");
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn test_auto_close_p() {
        let doc = TreeBuilder::build("<p>one<p>two");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 2);
    }

    #[test]
    fn test_auto_close_li() {
        let doc = TreeBuilder::build("<ul><li>a<li>b<li>c</ul>");
        let lis = doc.get_elements_by_tag_name("li");
        assert_eq!(lis.len(), 3);
    }

    #[test]
    fn test_headings() {
        let doc = TreeBuilder::build("<h1>Title</h1><h2>Sub</h2>");
        let h1s = doc.get_elements_by_tag_name("h1");
        let h2s = doc.get_elements_by_tag_name("h2");
        assert_eq!(h1s.len(), 1);
        assert_eq!(h2s.len(), 1);
    }

    #[test]
    fn test_heading_auto_close() {
        let doc = TreeBuilder::build("<h1>A<h2>B");
        let h1s = doc.get_elements_by_tag_name("h1");
        let h2s = doc.get_elements_by_tag_name("h2");
        assert_eq!(h1s.len(), 1);
        assert_eq!(h2s.len(), 1);
    }

    #[test]
    fn test_void_elements() {
        let doc = TreeBuilder::build("<p>before<br>after</p>");
        let brs = doc.get_elements_by_tag_name("br");
        assert_eq!(brs.len(), 1);
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(doc.inner_text(ps[0]), "beforeafter");
    }

    #[test]
    fn test_formatting_elements() {
        let doc = TreeBuilder::build("<p><b>bold</b> normal</p>");
        let bs = doc.get_elements_by_tag_name("b");
        assert_eq!(bs.len(), 1);
        assert_eq!(doc.inner_text(bs[0]), "bold");
    }

    #[test]
    fn test_comment_in_body() {
        let doc = TreeBuilder::build("<body><!-- comment --><p>text</p></body>");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 1);
    }

    #[test]
    fn test_doctype() {
        let doc = TreeBuilder::build("<!DOCTYPE html><html><body></body></html>");
        // Should have doctype node
        let root_node = doc.arena.get(doc.root).unwrap();
        let first_child = root_node.children[0];
        let dt = doc.arena.get(first_child).unwrap();
        assert_eq!(dt.node_type, NodeType::DocumentType);
    }

    #[test]
    fn test_text_concatenation() {
        let doc = TreeBuilder::build("<p>abc</p>");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(doc.inner_text(ps[0]), "abc");
    }

    #[test]
    fn test_mixed_text_and_elements() {
        let doc = TreeBuilder::build("<div>Hello <b>world</b>!</div>");
        let divs = doc.get_elements_by_tag_name("div");
        assert_eq!(doc.inner_text(divs[0]), "Hello world!");
    }

    #[test]
    fn test_attributes_preserved() {
        let doc = TreeBuilder::build("<div id=\"main\" class=\"container\">text</div>");
        let el = doc.get_element_by_id("main");
        assert!(el.is_some());
    }

    #[test]
    fn test_table_basic() {
        let doc = TreeBuilder::build("<table><tr><td>cell</td></tr></table>");
        let tds = doc.get_elements_by_tag_name("td");
        assert_eq!(tds.len(), 1);
        assert_eq!(doc.inner_text(tds[0]), "cell");
    }

    #[test]
    fn test_table_implied_tbody() {
        let doc = TreeBuilder::build("<table><tr><td>A</td></tr></table>");
        let tbodies = doc.get_elements_by_tag_name("tbody");
        assert!(!tbodies.is_empty());
    }

    #[test]
    fn test_table_with_thead() {
        let doc = TreeBuilder::build(
            "<table><thead><tr><th>H</th></tr></thead><tbody><tr><td>D</td></tr></tbody></table>",
        );
        let ths = doc.get_elements_by_tag_name("th");
        let tds = doc.get_elements_by_tag_name("td");
        assert_eq!(ths.len(), 1);
        assert_eq!(tds.len(), 1);
    }

    #[test]
    fn test_dt_dd_auto_close() {
        let doc = TreeBuilder::build("<dl><dt>term<dd>def<dt>term2<dd>def2</dl>");
        let dts = doc.get_elements_by_tag_name("dt");
        let dds = doc.get_elements_by_tag_name("dd");
        assert_eq!(dts.len(), 2);
        assert_eq!(dds.len(), 2);
    }

    #[test]
    fn test_no_body_text() {
        let doc = TreeBuilder::build("just text");
        // Should still produce a document with implied body
        let bodies = doc.get_elements_by_tag_name("body");
        assert!(!bodies.is_empty());
    }

    #[test]
    fn test_anchor_tag() {
        let doc = TreeBuilder::build("<a href=\"/page\">link</a>");
        let anchors = doc.get_elements_by_tag_name("a");
        assert_eq!(anchors.len(), 1);
        assert_eq!(
            doc.get_attribute(anchors[0], "href"),
            Some("/page".to_string())
        );
    }

    #[test]
    fn test_img_in_body() {
        let doc = TreeBuilder::build("<body><img src=\"pic.png\"></body>");
        let imgs = doc.get_elements_by_tag_name("img");
        assert_eq!(imgs.len(), 1);
    }

    #[test]
    fn test_multiple_text_runs() {
        let doc = TreeBuilder::build("<p>Hello</p><p>World</p>");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(ps.len(), 2);
        assert_eq!(doc.inner_text(ps[0]), "Hello");
        assert_eq!(doc.inner_text(ps[1]), "World");
    }

    #[test]
    fn test_deeply_nested() {
        let doc = TreeBuilder::build("<div><div><div><span>deep</span></div></div></div>");
        let spans = doc.get_elements_by_tag_name("span");
        assert_eq!(spans.len(), 1);
        assert_eq!(doc.inner_text(spans[0]), "deep");
    }

    #[test]
    fn test_form_in_body() {
        let doc = TreeBuilder::build("<form><input type=\"text\"><button>Submit</button></form>");
        let forms = doc.get_elements_by_tag_name("form");
        assert_eq!(forms.len(), 1);
        let inputs = doc.get_elements_by_tag_name("input");
        assert_eq!(inputs.len(), 1);
    }

    #[test]
    fn test_style_in_head() {
        let doc = TreeBuilder::build(
            "<html><head><style>body{color:red}</style></head><body>text</body></html>",
        );
        let styles = doc.get_elements_by_tag_name("style");
        assert_eq!(styles.len(), 1);
    }

    #[test]
    fn test_meta_in_head() {
        let doc =
            TreeBuilder::build("<html><head><meta charset=\"utf-8\"></head><body></body></html>");
        let metas = doc.get_elements_by_tag_name("meta");
        assert_eq!(metas.len(), 1);
    }

    #[test]
    fn test_link_in_head() {
        let doc = TreeBuilder::build(
            "<html><head><link rel=\"stylesheet\" href=\"s.css\"></head><body></body></html>",
        );
        let links = doc.get_elements_by_tag_name("link");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_strong_em() {
        let doc = TreeBuilder::build("<p><strong>bold</strong> and <em>italic</em></p>");
        let strong = doc.get_elements_by_tag_name("strong");
        let em = doc.get_elements_by_tag_name("em");
        assert_eq!(strong.len(), 1);
        assert_eq!(em.len(), 1);
    }

    #[test]
    fn test_default_tree_builder() {
        let _builder = TreeBuilder::default();
    }

    #[test]
    fn test_ul_ol_nesting() {
        let doc = TreeBuilder::build("<ul><li>a</li><li>b<ul><li>c</li></ul></li></ul>");
        let lis = doc.get_elements_by_tag_name("li");
        assert_eq!(lis.len(), 3);
    }

    #[test]
    fn test_entity_in_text() {
        let doc = TreeBuilder::build("<p>&amp;</p>");
        let ps = doc.get_elements_by_tag_name("p");
        assert_eq!(doc.inner_text(ps[0]), "&");
    }
}
