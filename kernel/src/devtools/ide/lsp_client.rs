//! Language Server Protocol (LSP) Client
//!
//! JSON-RPC 2.0 transport over stdin/stdout, with support for
//! initialization, completion, diagnostics, go-to-definition, and hover.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

/// LSP message ID
type MessageId = u64;

/// JSON value (minimal subset)
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Number(i64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            Self::Object(m) => Some(m),
            _ => None,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&JsonValue> {
        self.as_object()?.get(key)
    }

    /// Serialize to JSON string
    pub(crate) fn to_json(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            Self::Number(n) => alloc::format!("{}", n),
            Self::Str(s) => {
                let escaped = s
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                alloc::format!("\"{}\"", escaped)
            }
            Self::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.to_json()).collect();
                alloc::format!("[{}]", items.join(","))
            }
            Self::Object(map) => {
                let items: Vec<String> = map
                    .iter()
                    .map(|(k, v)| alloc::format!("\"{}\":{}", k, v.to_json()))
                    .collect();
                alloc::format!("{{{}}}", items.join(","))
            }
        }
    }
}

/// Minimal JSON parser
pub(crate) fn parse_json(input: &str) -> Option<JsonValue> {
    let trimmed = input.trim();
    let (val, _) = parse_value(trimmed.as_bytes(), 0)?;
    Some(val)
}

fn parse_value(data: &[u8], mut pos: usize) -> Option<(JsonValue, usize)> {
    pos = skip_whitespace(data, pos);
    if pos >= data.len() {
        return None;
    }

    match data[pos] {
        b'"' => parse_string(data, pos),
        b'{' => parse_object(data, pos),
        b'[' => parse_array(data, pos),
        b't' | b'f' => parse_bool(data, pos),
        b'n' => parse_null(data, pos),
        b'-' | b'0'..=b'9' => parse_number(data, pos),
        _ => None,
    }
}

fn skip_whitespace(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && matches!(data[pos], b' ' | b'\t' | b'\n' | b'\r') {
        pos += 1;
    }
    pos
}

fn parse_string(data: &[u8], mut pos: usize) -> Option<(JsonValue, usize)> {
    if data[pos] != b'"' {
        return None;
    }
    pos += 1;
    let mut s = String::new();
    while pos < data.len() && data[pos] != b'"' {
        if data[pos] == b'\\' && pos + 1 < data.len() {
            pos += 1;
            match data[pos] {
                b'"' => s.push('"'),
                b'\\' => s.push('\\'),
                b'n' => s.push('\n'),
                b'r' => s.push('\r'),
                b't' => s.push('\t'),
                _ => {
                    s.push('\\');
                    s.push(data[pos] as char);
                }
            }
        } else {
            s.push(data[pos] as char);
        }
        pos += 1;
    }
    if pos >= data.len() {
        return None;
    }
    pos += 1; // Skip closing quote
    Some((JsonValue::Str(s), pos))
}

fn parse_object(data: &[u8], mut pos: usize) -> Option<(JsonValue, usize)> {
    pos += 1; // Skip '{'
    let mut map = BTreeMap::new();
    pos = skip_whitespace(data, pos);

    if pos < data.len() && data[pos] == b'}' {
        return Some((JsonValue::Object(map), pos + 1));
    }

    loop {
        pos = skip_whitespace(data, pos);
        let (key_val, new_pos) = parse_string(data, pos)?;
        pos = new_pos;
        let key = match key_val {
            JsonValue::Str(s) => s,
            _ => return None,
        };

        pos = skip_whitespace(data, pos);
        if pos >= data.len() || data[pos] != b':' {
            return None;
        }
        pos += 1;

        let (val, new_pos) = parse_value(data, pos)?;
        pos = new_pos;
        map.insert(key, val);

        pos = skip_whitespace(data, pos);
        if pos >= data.len() {
            return None;
        }
        if data[pos] == b'}' {
            return Some((JsonValue::Object(map), pos + 1));
        }
        if data[pos] == b',' {
            pos += 1;
        }
    }
}

fn parse_array(data: &[u8], mut pos: usize) -> Option<(JsonValue, usize)> {
    pos += 1; // Skip '['
    let mut arr = Vec::new();
    pos = skip_whitespace(data, pos);

    if pos < data.len() && data[pos] == b']' {
        return Some((JsonValue::Array(arr), pos + 1));
    }

    loop {
        let (val, new_pos) = parse_value(data, pos)?;
        pos = new_pos;
        arr.push(val);

        pos = skip_whitespace(data, pos);
        if pos >= data.len() {
            return None;
        }
        if data[pos] == b']' {
            return Some((JsonValue::Array(arr), pos + 1));
        }
        if data[pos] == b',' {
            pos += 1;
        }
    }
}

fn parse_bool(data: &[u8], pos: usize) -> Option<(JsonValue, usize)> {
    if data[pos..].starts_with(b"true") {
        Some((JsonValue::Bool(true), pos + 4))
    } else if data[pos..].starts_with(b"false") {
        Some((JsonValue::Bool(false), pos + 5))
    } else {
        None
    }
}

fn parse_null(data: &[u8], pos: usize) -> Option<(JsonValue, usize)> {
    if data[pos..].starts_with(b"null") {
        Some((JsonValue::Null, pos + 4))
    } else {
        None
    }
}

fn parse_number(data: &[u8], mut pos: usize) -> Option<(JsonValue, usize)> {
    let start = pos;
    if pos < data.len() && data[pos] == b'-' {
        pos += 1;
    }
    while pos < data.len() && data[pos].is_ascii_digit() {
        pos += 1;
    }
    let num_str = core::str::from_utf8(&data[start..pos]).ok()?;
    let num: i64 = num_str.parse().ok()?;
    Some((JsonValue::Number(num), pos))
}

// ---------------------------------------------------------------------------
// LSP Protocol Types
// ---------------------------------------------------------------------------

/// LSP completion item
#[derive(Debug, Clone)]
pub(crate) struct CompletionItem {
    pub(crate) label: String,
    pub(crate) kind: CompletionKind,
    pub(crate) detail: Option<String>,
    pub(crate) insert_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionKind {
    Text = 1,
    Method = 2,
    Function = 3,
    Constructor = 4,
    Field = 5,
    Variable = 6,
    Class = 7,
    Interface = 8,
    Module = 9,
    Property = 10,
    Keyword = 14,
    Snippet = 15,
}

/// LSP diagnostic
#[derive(Debug, Clone)]
pub(crate) struct Diagnostic {
    pub(crate) range_start_line: u32,
    pub(crate) range_start_col: u32,
    pub(crate) range_end_line: u32,
    pub(crate) range_end_col: u32,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) message: String,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

/// LSP location (for go-to-definition)
#[derive(Debug, Clone)]
pub(crate) struct Location {
    pub(crate) uri: String,
    pub(crate) line: u32,
    pub(crate) col: u32,
}

/// LSP hover result
#[derive(Debug, Clone)]
pub(crate) struct HoverResult {
    pub(crate) contents: String,
    pub(crate) range: Option<(u32, u32, u32, u32)>,
}

/// LSP client state
pub(crate) struct LspClient {
    next_id: MessageId,
    pub(crate) server_capabilities: Option<JsonValue>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    initialized: bool,
}

impl Default for LspClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LspClient {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            server_capabilities: None,
            diagnostics: Vec::new(),
            initialized: false,
        }
    }

    fn next_id(&mut self) -> MessageId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Build JSON-RPC request
    pub(crate) fn build_request(&mut self, method: &str, params: JsonValue) -> String {
        let id = self.next_id();
        let mut obj = BTreeMap::new();
        obj.insert("jsonrpc".to_string(), JsonValue::Str("2.0".to_string()));
        obj.insert("id".to_string(), JsonValue::Number(id as i64));
        obj.insert("method".to_string(), JsonValue::Str(method.to_string()));
        obj.insert("params".to_string(), params);

        let json = JsonValue::Object(obj).to_json();
        alloc::format!("Content-Length: {}\r\n\r\n{}", json.len(), json)
    }

    /// Build JSON-RPC notification (no id)
    pub(crate) fn build_notification(&self, method: &str, params: JsonValue) -> String {
        let mut obj = BTreeMap::new();
        obj.insert("jsonrpc".to_string(), JsonValue::Str("2.0".to_string()));
        obj.insert("method".to_string(), JsonValue::Str(method.to_string()));
        obj.insert("params".to_string(), params);

        let json = JsonValue::Object(obj).to_json();
        alloc::format!("Content-Length: {}\r\n\r\n{}", json.len(), json)
    }

    /// Build initialize request
    pub(crate) fn build_initialize(&mut self, root_uri: &str) -> String {
        let mut capabilities = BTreeMap::new();
        let mut text_doc = BTreeMap::new();
        let mut completion = BTreeMap::new();
        completion.insert("dynamicRegistration".to_string(), JsonValue::Bool(false));
        text_doc.insert("completion".to_string(), JsonValue::Object(completion));
        capabilities.insert("textDocument".to_string(), JsonValue::Object(text_doc));

        let mut params = BTreeMap::new();
        params.insert("processId".to_string(), JsonValue::Number(1));
        params.insert("rootUri".to_string(), JsonValue::Str(root_uri.to_string()));
        params.insert("capabilities".to_string(), JsonValue::Object(capabilities));

        self.build_request("initialize", JsonValue::Object(params))
    }

    /// Parse an LSP response
    pub(crate) fn parse_response(&mut self, data: &str) -> Option<JsonValue> {
        // Skip Content-Length header
        let body_start = data.find("\r\n\r\n")?;
        let body = &data[body_start + 4..];
        let val = parse_json(body)?;

        // Check for server capabilities in initialize response
        if let Some(result) = val.get("result") {
            if let Some(caps) = result.get("capabilities") {
                self.server_capabilities = Some(caps.clone());
                self.initialized = true;
            }
        }

        Some(val)
    }

    /// Parse completion items from response
    pub(crate) fn parse_completions(&self, result: &JsonValue) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        let arr = match result {
            JsonValue::Array(a) => a,
            JsonValue::Object(obj) => match obj.get("items") {
                Some(JsonValue::Array(a)) => a,
                _ => return items,
            },
            _ => return items,
        };

        for item in arr {
            if let Some(obj) = item.as_object() {
                let label = obj
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let kind_num = obj.get("kind").and_then(|v| v.as_i64()).unwrap_or(1);

                let kind = match kind_num {
                    2 => CompletionKind::Method,
                    3 => CompletionKind::Function,
                    4 => CompletionKind::Constructor,
                    5 => CompletionKind::Field,
                    6 => CompletionKind::Variable,
                    7 => CompletionKind::Class,
                    8 => CompletionKind::Interface,
                    9 => CompletionKind::Module,
                    10 => CompletionKind::Property,
                    14 => CompletionKind::Keyword,
                    15 => CompletionKind::Snippet,
                    _ => CompletionKind::Text,
                };

                let detail = obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let insert_text = obj
                    .get("insertText")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                items.push(CompletionItem {
                    label,
                    kind,
                    detail,
                    insert_text,
                });
            }
        }

        items
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
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
    fn test_parse_json_null() {
        assert_eq!(parse_json("null"), Some(JsonValue::Null));
    }

    #[test]
    fn test_parse_json_bool() {
        assert_eq!(parse_json("true"), Some(JsonValue::Bool(true)));
        assert_eq!(parse_json("false"), Some(JsonValue::Bool(false)));
    }

    #[test]
    fn test_parse_json_number() {
        assert_eq!(parse_json("42"), Some(JsonValue::Number(42)));
        assert_eq!(parse_json("-1"), Some(JsonValue::Number(-1)));
    }

    #[test]
    fn test_parse_json_string() {
        assert_eq!(
            parse_json("\"hello\""),
            Some(JsonValue::Str("hello".to_string()))
        );
    }

    #[test]
    fn test_parse_json_array() {
        let val = parse_json("[1, 2, 3]").unwrap();
        match val {
            JsonValue::Array(arr) => assert_eq!(arr.len(), 3),
            _ => panic!("Expected array"),
        }
    }

    #[test]
    fn test_parse_json_object() {
        let val = parse_json("{\"key\": \"value\"}").unwrap();
        assert_eq!(val.get("key"), Some(&JsonValue::Str("value".to_string())));
    }

    #[test]
    fn test_json_serialize() {
        let val = JsonValue::Str("hello".to_string());
        assert_eq!(val.to_json(), "\"hello\"");

        let val = JsonValue::Number(42);
        assert_eq!(val.to_json(), "42");
    }

    #[test]
    fn test_lsp_client_build_request() {
        let mut client = LspClient::new();
        let req = client.build_request("test/method", JsonValue::Null);
        assert!(req.contains("Content-Length:"));
        assert!(req.contains("test/method"));
        assert!(req.contains("\"id\":1"));
    }

    #[test]
    fn test_lsp_client_initialize() {
        let mut client = LspClient::new();
        let req = client.build_initialize("file:///workspace");
        assert!(req.contains("initialize"));
        assert!(req.contains("file:///workspace"));
    }

    #[test]
    fn test_lsp_client_not_initialized() {
        let client = LspClient::new();
        assert!(!client.is_initialized());
    }

    #[test]
    fn test_parse_completions_empty() {
        let client = LspClient::new();
        let result = JsonValue::Array(Vec::new());
        let items = client.parse_completions(&result);
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_completions_items() {
        let client = LspClient::new();
        let mut item = BTreeMap::new();
        item.insert("label".to_string(), JsonValue::Str("println!".to_string()));
        item.insert("kind".to_string(), JsonValue::Number(3)); // Function

        let result = JsonValue::Array(vec![JsonValue::Object(item)]);
        let items = client.parse_completions(&result);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "println!");
        assert_eq!(items[0].kind, CompletionKind::Function);
    }

    #[test]
    fn test_diagnostic_severity() {
        assert_eq!(DiagnosticSeverity::Error as u8, 1);
        assert_eq!(DiagnosticSeverity::Warning as u8, 2);
    }

    #[test]
    fn test_json_escape() {
        let val = JsonValue::Str("line1\nline2".to_string());
        let json = val.to_json();
        assert!(json.contains("\\n"));
    }
}
