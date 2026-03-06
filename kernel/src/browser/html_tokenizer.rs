//! HTML Tokenizer
//!
//! Implements a state-machine HTML tokenizer that converts raw HTML bytes
//! into a stream of tokens (start tags, end tags, text, comments, doctype).
//! Handles entity references (&amp;, &lt;, &gt;, &quot;) and void elements.

#![allow(dead_code)]

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// An HTML attribute (name=value pair)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

/// Tokens produced by the HTML tokenizer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// <!DOCTYPE ...>
    Doctype(String),
    /// <tag attr="val">
    StartTag(String, Vec<Attribute>, bool),
    /// </tag>
    EndTag(String),
    /// A single character of text
    Character(char),
    /// <!-- comment -->
    Comment(String),
    /// End of input
    Eof,
}

/// States for the tokenizer state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenizerState {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueDoubleQuoted,
    AttrValueSingleQuoted,
    AttrValueUnquoted,
    AfterAttrValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    InComment,
    CommentEndDash,
    CommentEnd,
}

/// Void elements that are self-closing
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// HTML tokenizer that converts input bytes into tokens
pub struct HtmlTokenizer {
    input: Vec<u8>,
    pos: usize,
    state: TokenizerState,
    current_tag_name: String,
    current_tag_attrs: Vec<Attribute>,
    current_attr_name: String,
    current_attr_value: String,
    self_closing: bool,
    is_end_tag: bool,
    comment_data: String,
    reconsume: bool,
    current_char: u8,
}

impl HtmlTokenizer {
    /// Create a new tokenizer from input bytes
    pub fn new(input: &[u8]) -> Self {
        Self {
            input: input.to_vec(),
            pos: 0,
            state: TokenizerState::Data,
            current_tag_name: String::new(),
            current_tag_attrs: Vec::new(),
            current_attr_name: String::new(),
            current_attr_value: String::new(),
            self_closing: false,
            is_end_tag: false,
            comment_data: String::new(),
            reconsume: false,
            current_char: 0,
        }
    }

    /// Create a tokenizer from a string slice
    pub fn from_text(input: &str) -> Self {
        Self::new(input.as_bytes())
    }

    /// Tokenize the entire input into a Vec of tokens
    pub fn tokenize_all(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token();
            if token == Token::Eof {
                tokens.push(Token::Eof);
                break;
            }
            tokens.push(token);
        }
        tokens
    }

    /// Consume the next character from input
    fn consume_next(&mut self) -> Option<u8> {
        if self.reconsume {
            self.reconsume = false;
            return Some(self.current_char);
        }
        if self.pos < self.input.len() {
            let ch = self.input[self.pos];
            self.pos += 1;
            self.current_char = ch;
            Some(ch)
        } else {
            None
        }
    }

    /// Peek at the next character without consuming
    fn peek(&self) -> Option<u8> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    /// Check if the next bytes match a string (case-insensitive)
    fn starts_with_ci(&self, s: &str) -> bool {
        let bytes = s.as_bytes();
        if self.pos + bytes.len() > self.input.len() {
            return false;
        }
        for (i, &b) in bytes.iter().enumerate() {
            let input_byte = self.input[self.pos + i];
            if !input_byte.eq_ignore_ascii_case(&b) {
                return false;
            }
        }
        true
    }

    /// Emit the current tag as a token
    fn emit_tag(&mut self) -> Token {
        let name = core::mem::take(&mut self.current_tag_name);
        let attrs = core::mem::take(&mut self.current_tag_attrs);
        let sc = self.self_closing;
        self.self_closing = false;

        if self.is_end_tag {
            self.is_end_tag = false;
            Token::EndTag(name)
        } else {
            // Check if it's a void element
            let is_void = VOID_ELEMENTS.contains(&name.as_str());
            Token::StartTag(name, attrs, sc || is_void)
        }
    }

    /// Finalize the current attribute and add it to the list
    fn finish_attr(&mut self) {
        if !self.current_attr_name.is_empty() {
            let attr = Attribute {
                name: core::mem::take(&mut self.current_attr_name),
                value: core::mem::take(&mut self.current_attr_value),
            };
            self.current_tag_attrs.push(attr);
        } else {
            self.current_attr_name.clear();
            self.current_attr_value.clear();
        }
    }

    /// Try to decode an entity reference starting at current position.
    /// Returns the decoded character or None.
    fn try_decode_entity(&mut self) -> Option<char> {
        // We've already consumed '&'
        let start = self.pos;
        let mut entity = String::new();

        // Read up to 10 chars until ';' or non-alpha
        for _ in 0..10 {
            match self.peek() {
                Some(b';') => {
                    self.pos += 1; // consume ';'
                    break;
                }
                Some(ch) if ch.is_ascii_alphanumeric() || ch == b'#' => {
                    self.pos += 1;
                    entity.push(ch as char);
                }
                _ => break,
            }
        }

        match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{00A0}'),
            s if s.starts_with('#') => {
                let num_str = &s[1..];
                let code = if let Some(hex) = num_str.strip_prefix('x') {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    num_str.parse::<u32>().ok()
                };
                code.and_then(char::from_u32)
            }
            _ => {
                // Unknown entity, rewind and emit '&' literally
                self.pos = start;
                Some('&')
            }
        }
    }

    /// Get the next token from the input
    pub fn next_token(&mut self) -> Token {
        loop {
            match self.state {
                TokenizerState::Data => match self.consume_next() {
                    Some(b'<') => {
                        self.state = TokenizerState::TagOpen;
                    }
                    Some(b'&') => {
                        if let Some(ch) = self.try_decode_entity() {
                            return Token::Character(ch);
                        }
                        return Token::Character('&');
                    }
                    Some(ch) => {
                        return Token::Character(ch as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::TagOpen => {
                    match self.consume_next() {
                        Some(b'!') => {
                            self.state = TokenizerState::MarkupDeclarationOpen;
                        }
                        Some(b'/') => {
                            self.state = TokenizerState::EndTagOpen;
                        }
                        Some(ch) if ch.is_ascii_alphabetic() => {
                            self.current_tag_name.clear();
                            self.current_tag_attrs.clear();
                            self.self_closing = false;
                            self.is_end_tag = false;
                            self.current_tag_name.push(ch.to_ascii_lowercase() as char);
                            self.state = TokenizerState::TagName;
                        }
                        Some(b'?') => {
                            // Processing instruction, treat as bogus comment
                            self.comment_data.clear();
                            self.state = TokenizerState::BogusComment;
                        }
                        _ => {
                            // Not a tag, emit '<' as character
                            self.reconsume = true;
                            self.state = TokenizerState::Data;
                            return Token::Character('<');
                        }
                    }
                }

                TokenizerState::EndTagOpen => {
                    match self.consume_next() {
                        Some(ch) if ch.is_ascii_alphabetic() => {
                            self.current_tag_name.clear();
                            self.current_tag_attrs.clear();
                            self.self_closing = false;
                            self.is_end_tag = true;
                            self.current_tag_name.push(ch.to_ascii_lowercase() as char);
                            self.state = TokenizerState::TagName;
                        }
                        Some(b'>') => {
                            // </> is invalid, ignore
                            self.state = TokenizerState::Data;
                        }
                        _ => {
                            self.comment_data.clear();
                            self.reconsume = true;
                            self.state = TokenizerState::BogusComment;
                        }
                    }
                }

                TokenizerState::TagName => match self.consume_next() {
                    Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                        self.state = TokenizerState::BeforeAttrName;
                    }
                    Some(b'/') => {
                        self.state = TokenizerState::SelfClosingStartTag;
                    }
                    Some(b'>') => {
                        self.state = TokenizerState::Data;
                        return self.emit_tag();
                    }
                    Some(ch) => {
                        self.current_tag_name.push(ch.to_ascii_lowercase() as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::BeforeAttrName => {
                    match self.consume_next() {
                        Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                            // skip whitespace
                        }
                        Some(b'/') => {
                            self.state = TokenizerState::SelfClosingStartTag;
                        }
                        Some(b'>') => {
                            self.state = TokenizerState::Data;
                            return self.emit_tag();
                        }
                        Some(ch) => {
                            self.current_attr_name.clear();
                            self.current_attr_value.clear();
                            self.current_attr_name.push(ch.to_ascii_lowercase() as char);
                            self.state = TokenizerState::AttrName;
                        }
                        None => return Token::Eof,
                    }
                }

                TokenizerState::AttrName => match self.consume_next() {
                    Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                        self.state = TokenizerState::AfterAttrName;
                    }
                    Some(b'/') => {
                        self.finish_attr();
                        self.state = TokenizerState::SelfClosingStartTag;
                    }
                    Some(b'=') => {
                        self.state = TokenizerState::BeforeAttrValue;
                    }
                    Some(b'>') => {
                        self.finish_attr();
                        self.state = TokenizerState::Data;
                        return self.emit_tag();
                    }
                    Some(ch) => {
                        self.current_attr_name.push(ch.to_ascii_lowercase() as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::AfterAttrName => {
                    match self.consume_next() {
                        Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                            // skip whitespace
                        }
                        Some(b'/') => {
                            self.finish_attr();
                            self.state = TokenizerState::SelfClosingStartTag;
                        }
                        Some(b'=') => {
                            self.state = TokenizerState::BeforeAttrValue;
                        }
                        Some(b'>') => {
                            self.finish_attr();
                            self.state = TokenizerState::Data;
                            return self.emit_tag();
                        }
                        Some(ch) => {
                            // New attribute without value
                            self.finish_attr();
                            self.current_attr_name.clear();
                            self.current_attr_value.clear();
                            self.current_attr_name.push(ch.to_ascii_lowercase() as char);
                            self.state = TokenizerState::AttrName;
                        }
                        None => return Token::Eof,
                    }
                }

                TokenizerState::BeforeAttrValue => {
                    match self.consume_next() {
                        Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                            // skip whitespace
                        }
                        Some(b'"') => {
                            self.state = TokenizerState::AttrValueDoubleQuoted;
                        }
                        Some(b'\'') => {
                            self.state = TokenizerState::AttrValueSingleQuoted;
                        }
                        Some(b'>') => {
                            self.finish_attr();
                            self.state = TokenizerState::Data;
                            return self.emit_tag();
                        }
                        Some(ch) => {
                            self.current_attr_value.push(ch as char);
                            self.state = TokenizerState::AttrValueUnquoted;
                        }
                        None => return Token::Eof,
                    }
                }

                TokenizerState::AttrValueDoubleQuoted => match self.consume_next() {
                    Some(b'"') => {
                        self.finish_attr();
                        self.state = TokenizerState::AfterAttrValueQuoted;
                    }
                    Some(b'&') => {
                        if let Some(ch) = self.try_decode_entity() {
                            self.current_attr_value.push(ch);
                        } else {
                            self.current_attr_value.push('&');
                        }
                    }
                    Some(ch) => {
                        self.current_attr_value.push(ch as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::AttrValueSingleQuoted => match self.consume_next() {
                    Some(b'\'') => {
                        self.finish_attr();
                        self.state = TokenizerState::AfterAttrValueQuoted;
                    }
                    Some(b'&') => {
                        if let Some(ch) = self.try_decode_entity() {
                            self.current_attr_value.push(ch);
                        } else {
                            self.current_attr_value.push('&');
                        }
                    }
                    Some(ch) => {
                        self.current_attr_value.push(ch as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::AttrValueUnquoted => match self.consume_next() {
                    Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                        self.finish_attr();
                        self.state = TokenizerState::BeforeAttrName;
                    }
                    Some(b'&') => {
                        if let Some(ch) = self.try_decode_entity() {
                            self.current_attr_value.push(ch);
                        } else {
                            self.current_attr_value.push('&');
                        }
                    }
                    Some(b'>') => {
                        self.finish_attr();
                        self.state = TokenizerState::Data;
                        return self.emit_tag();
                    }
                    Some(ch) => {
                        self.current_attr_value.push(ch as char);
                    }
                    None => return Token::Eof,
                },

                TokenizerState::AfterAttrValueQuoted => match self.consume_next() {
                    Some(b'\t') | Some(b'\n') | Some(b'\x0C') | Some(b' ') => {
                        self.state = TokenizerState::BeforeAttrName;
                    }
                    Some(b'/') => {
                        self.state = TokenizerState::SelfClosingStartTag;
                    }
                    Some(b'>') => {
                        self.state = TokenizerState::Data;
                        return self.emit_tag();
                    }
                    _ => {
                        self.reconsume = true;
                        self.state = TokenizerState::BeforeAttrName;
                    }
                },

                TokenizerState::SelfClosingStartTag => match self.consume_next() {
                    Some(b'>') => {
                        self.self_closing = true;
                        self.state = TokenizerState::Data;
                        return self.emit_tag();
                    }
                    _ => {
                        self.reconsume = true;
                        self.state = TokenizerState::BeforeAttrName;
                    }
                },

                TokenizerState::BogusComment => match self.consume_next() {
                    Some(b'>') => {
                        let data = core::mem::take(&mut self.comment_data);
                        self.state = TokenizerState::Data;
                        return Token::Comment(data);
                    }
                    Some(ch) => {
                        self.comment_data.push(ch as char);
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        self.state = TokenizerState::Data;
                        return Token::Comment(data);
                    }
                },

                TokenizerState::MarkupDeclarationOpen => {
                    if self.starts_with_ci("--") {
                        self.pos += 2;
                        self.comment_data.clear();
                        self.state = TokenizerState::CommentStart;
                    } else if self.starts_with_ci("doctype") {
                        self.pos += 7;
                        // Read doctype name
                        let mut name = String::new();
                        // Skip whitespace
                        while let Some(&ch) = self.input.get(self.pos) {
                            if ch == b' ' || ch == b'\t' || ch == b'\n' {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                        // Read name until '>'
                        while let Some(&ch) = self.input.get(self.pos) {
                            if ch == b'>' {
                                self.pos += 1;
                                break;
                            }
                            name.push(ch.to_ascii_lowercase() as char);
                            self.pos += 1;
                        }
                        self.state = TokenizerState::Data;
                        return Token::Doctype(name.trim().to_string());
                    } else {
                        // Bogus comment
                        self.comment_data.clear();
                        self.state = TokenizerState::BogusComment;
                    }
                }

                TokenizerState::CommentStart => match self.consume_next() {
                    Some(b'-') => {
                        self.state = TokenizerState::CommentStartDash;
                    }
                    Some(b'>') => {
                        let data = core::mem::take(&mut self.comment_data);
                        self.state = TokenizerState::Data;
                        return Token::Comment(data);
                    }
                    Some(ch) => {
                        self.comment_data.push(ch as char);
                        self.state = TokenizerState::InComment;
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        return Token::Comment(data);
                    }
                },

                TokenizerState::CommentStartDash => match self.consume_next() {
                    Some(b'-') => {
                        self.state = TokenizerState::CommentEnd;
                    }
                    Some(b'>') => {
                        let data = core::mem::take(&mut self.comment_data);
                        self.state = TokenizerState::Data;
                        return Token::Comment(data);
                    }
                    Some(ch) => {
                        self.comment_data.push('-');
                        self.comment_data.push(ch as char);
                        self.state = TokenizerState::InComment;
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        return Token::Comment(data);
                    }
                },

                TokenizerState::InComment => match self.consume_next() {
                    Some(b'-') => {
                        self.state = TokenizerState::CommentEndDash;
                    }
                    Some(ch) => {
                        self.comment_data.push(ch as char);
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        return Token::Comment(data);
                    }
                },

                TokenizerState::CommentEndDash => match self.consume_next() {
                    Some(b'-') => {
                        self.state = TokenizerState::CommentEnd;
                    }
                    Some(ch) => {
                        self.comment_data.push('-');
                        self.comment_data.push(ch as char);
                        self.state = TokenizerState::InComment;
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        return Token::Comment(data);
                    }
                },

                TokenizerState::CommentEnd => match self.consume_next() {
                    Some(b'>') => {
                        let data = core::mem::take(&mut self.comment_data);
                        self.state = TokenizerState::Data;
                        return Token::Comment(data);
                    }
                    Some(b'-') => {
                        self.comment_data.push('-');
                    }
                    Some(ch) => {
                        self.comment_data.push('-');
                        self.comment_data.push('-');
                        self.comment_data.push(ch as char);
                        self.state = TokenizerState::InComment;
                    }
                    None => {
                        let data = core::mem::take(&mut self.comment_data);
                        return Token::Comment(data);
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_empty_input() {
        let mut t = HtmlTokenizer::from_text("");
        assert_eq!(t.next_token(), Token::Eof);
    }

    #[test]
    fn test_plain_text() {
        let mut t = HtmlTokenizer::from_text("hello");
        assert_eq!(t.next_token(), Token::Character('h'));
        assert_eq!(t.next_token(), Token::Character('e'));
        assert_eq!(t.next_token(), Token::Character('l'));
        assert_eq!(t.next_token(), Token::Character('l'));
        assert_eq!(t.next_token(), Token::Character('o'));
        assert_eq!(t.next_token(), Token::Eof);
    }

    #[test]
    fn test_simple_tag() {
        let mut t = HtmlTokenizer::from_text("<p>");
        let token = t.next_token();
        assert_eq!(token, Token::StartTag("p".into(), vec![], false));
    }

    #[test]
    fn test_end_tag() {
        let mut t = HtmlTokenizer::from_text("</p>");
        let token = t.next_token();
        assert_eq!(token, Token::EndTag("p".into()));
    }

    #[test]
    fn test_self_closing_tag() {
        let mut t = HtmlTokenizer::from_text("<br/>");
        let token = t.next_token();
        assert_eq!(token, Token::StartTag("br".into(), vec![], true));
    }

    #[test]
    fn test_void_element_auto_self_closing() {
        let mut t = HtmlTokenizer::from_text("<br>");
        let token = t.next_token();
        assert_eq!(token, Token::StartTag("br".into(), vec![], true));
    }

    #[test]
    fn test_tag_with_attribute() {
        let mut t = HtmlTokenizer::from_text("<div class=\"main\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "div".into(),
                vec![Attribute {
                    name: "class".into(),
                    value: "main".into()
                }],
                false
            )
        );
    }

    #[test]
    fn test_tag_with_multiple_attrs() {
        let mut t = HtmlTokenizer::from_text("<a href=\"/\" id=\"home\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "a".into(),
                vec![
                    Attribute {
                        name: "href".into(),
                        value: "/".into()
                    },
                    Attribute {
                        name: "id".into(),
                        value: "home".into()
                    },
                ],
                false
            )
        );
    }

    #[test]
    fn test_single_quoted_attr() {
        let mut t = HtmlTokenizer::from_text("<div class='main'>");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "div".into(),
                vec![Attribute {
                    name: "class".into(),
                    value: "main".into()
                }],
                false
            )
        );
    }

    #[test]
    fn test_unquoted_attr() {
        let mut t = HtmlTokenizer::from_text("<div class=main>");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "div".into(),
                vec![Attribute {
                    name: "class".into(),
                    value: "main".into()
                }],
                false
            )
        );
    }

    #[test]
    fn test_entity_amp() {
        let mut t = HtmlTokenizer::from_text("&amp;");
        assert_eq!(t.next_token(), Token::Character('&'));
    }

    #[test]
    fn test_entity_lt() {
        let mut t = HtmlTokenizer::from_text("&lt;");
        assert_eq!(t.next_token(), Token::Character('<'));
    }

    #[test]
    fn test_entity_gt() {
        let mut t = HtmlTokenizer::from_text("&gt;");
        assert_eq!(t.next_token(), Token::Character('>'));
    }

    #[test]
    fn test_entity_quot() {
        let mut t = HtmlTokenizer::from_text("&quot;");
        assert_eq!(t.next_token(), Token::Character('"'));
    }

    #[test]
    fn test_entity_numeric() {
        let mut t = HtmlTokenizer::from_text("&#65;");
        assert_eq!(t.next_token(), Token::Character('A'));
    }

    #[test]
    fn test_entity_hex() {
        let mut t = HtmlTokenizer::from_text("&#x41;");
        assert_eq!(t.next_token(), Token::Character('A'));
    }

    #[test]
    fn test_entity_in_attr() {
        let mut t = HtmlTokenizer::from_text("<a href=\"a&amp;b\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "a".into(),
                vec![Attribute {
                    name: "href".into(),
                    value: "a&b".into()
                }],
                false
            )
        );
    }

    #[test]
    fn test_comment() {
        let mut t = HtmlTokenizer::from_text("<!-- hello -->");
        let token = t.next_token();
        assert_eq!(token, Token::Comment(" hello ".into()));
    }

    #[test]
    fn test_empty_comment() {
        let mut t = HtmlTokenizer::from_text("<!---->");
        let token = t.next_token();
        assert_eq!(token, Token::Comment(String::new()));
    }

    #[test]
    fn test_doctype() {
        let mut t = HtmlTokenizer::from_text("<!DOCTYPE html>");
        let token = t.next_token();
        assert_eq!(token, Token::Doctype("html".into()));
    }

    #[test]
    fn test_case_insensitive_tags() {
        let mut t = HtmlTokenizer::from_text("<DIV>");
        let token = t.next_token();
        assert_eq!(token, Token::StartTag("div".into(), vec![], false));
    }

    #[test]
    fn test_img_void() {
        let mut t = HtmlTokenizer::from_text("<img src=\"a.png\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "img".into(),
                vec![Attribute {
                    name: "src".into(),
                    value: "a.png".into()
                }],
                true
            )
        );
    }

    #[test]
    fn test_input_void() {
        let mut t = HtmlTokenizer::from_text("<input type=\"text\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "input".into(),
                vec![Attribute {
                    name: "type".into(),
                    value: "text".into()
                }],
                true
            )
        );
    }

    #[test]
    fn test_hr_void() {
        let mut t = HtmlTokenizer::from_text("<hr>");
        let token = t.next_token();
        assert_eq!(token, Token::StartTag("hr".into(), vec![], true));
    }

    #[test]
    fn test_meta_void() {
        let mut t = HtmlTokenizer::from_text("<meta charset=\"utf-8\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "meta".into(),
                vec![Attribute {
                    name: "charset".into(),
                    value: "utf-8".into()
                }],
                true
            )
        );
    }

    #[test]
    fn test_link_void() {
        let mut t = HtmlTokenizer::from_text("<link rel=\"stylesheet\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "link".into(),
                vec![Attribute {
                    name: "rel".into(),
                    value: "stylesheet".into()
                }],
                true
            )
        );
    }

    #[test]
    fn test_boolean_attribute() {
        let mut t = HtmlTokenizer::from_text("<input disabled>");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "input".into(),
                vec![Attribute {
                    name: "disabled".into(),
                    value: String::new(),
                }],
                true
            )
        );
    }

    #[test]
    fn test_full_document() {
        let html =
            "<!DOCTYPE html><html><head><title>Hi</title></head><body><p>Hello</p></body></html>";
        let mut t = HtmlTokenizer::from_text(html);
        let tokens = t.tokenize_all();
        // Should start with Doctype, then StartTag html, etc.
        assert_eq!(tokens[0], Token::Doctype("html".into()));
        assert_eq!(tokens[1], Token::StartTag("html".into(), vec![], false));
        // Last should be Eof
        assert_eq!(*tokens.last().unwrap(), Token::Eof);
    }

    #[test]
    fn test_tokenize_all() {
        let mut t = HtmlTokenizer::from_text("<b>hi</b>");
        let tokens = t.tokenize_all();
        assert_eq!(tokens.len(), 5); // StartTag, 'h', 'i', EndTag, Eof
    }

    #[test]
    fn test_text_between_tags() {
        let mut t = HtmlTokenizer::from_text("<p>ab</p>");
        assert_eq!(t.next_token(), Token::StartTag("p".into(), vec![], false));
        assert_eq!(t.next_token(), Token::Character('a'));
        assert_eq!(t.next_token(), Token::Character('b'));
        assert_eq!(t.next_token(), Token::EndTag("p".into()));
        assert_eq!(t.next_token(), Token::Eof);
    }

    #[test]
    fn test_nested_tags() {
        let mut t = HtmlTokenizer::from_text("<div><span></span></div>");
        let tokens = t.tokenize_all();
        assert_eq!(tokens[0], Token::StartTag("div".into(), vec![], false));
        assert_eq!(tokens[1], Token::StartTag("span".into(), vec![], false));
        assert_eq!(tokens[2], Token::EndTag("span".into()));
        assert_eq!(tokens[3], Token::EndTag("div".into()));
        assert_eq!(tokens[4], Token::Eof);
    }

    #[test]
    fn test_attribute_no_value_before_another() {
        let mut t = HtmlTokenizer::from_text("<input disabled type=\"text\">");
        let token = t.next_token();
        assert_eq!(
            token,
            Token::StartTag(
                "input".into(),
                vec![
                    Attribute {
                        name: "disabled".into(),
                        value: String::new()
                    },
                    Attribute {
                        name: "type".into(),
                        value: "text".into()
                    },
                ],
                true
            )
        );
    }

    #[test]
    fn test_entity_nbsp() {
        let mut t = HtmlTokenizer::from_text("&nbsp;");
        assert_eq!(t.next_token(), Token::Character('\u{00A0}'));
    }

    #[test]
    fn test_entity_apos() {
        let mut t = HtmlTokenizer::from_text("&apos;");
        assert_eq!(t.next_token(), Token::Character('\''));
    }

    #[test]
    fn test_processing_instruction() {
        let mut t = HtmlTokenizer::from_text("<?xml version=\"1.0\"?>");
        let token = t.next_token();
        // Treated as bogus comment
        if let Token::Comment(_) = token {
            // ok
        } else {
            panic!("Expected Comment for PI");
        }
    }

    #[test]
    fn test_mixed_content() {
        let mut t = HtmlTokenizer::from_text("a<b>c</b>d");
        assert_eq!(t.next_token(), Token::Character('a'));
        assert_eq!(t.next_token(), Token::StartTag("b".into(), vec![], false));
        assert_eq!(t.next_token(), Token::Character('c'));
        assert_eq!(t.next_token(), Token::EndTag("b".into()));
        assert_eq!(t.next_token(), Token::Character('d'));
        assert_eq!(t.next_token(), Token::Eof);
    }

    #[test]
    fn test_whitespace_in_tags() {
        let mut t = HtmlTokenizer::from_text("<  p  >");
        // '<' followed by space is not a tag
        assert_eq!(t.next_token(), Token::Character('<'));
    }

    #[test]
    fn test_comment_with_dashes() {
        let mut t = HtmlTokenizer::from_text("<!-- a-b -->");
        let token = t.next_token();
        assert_eq!(token, Token::Comment(" a-b ".into()));
    }

    #[test]
    fn test_self_closing_nonvoid() {
        let mut t = HtmlTokenizer::from_text("<div/>");
        let token = t.next_token();
        // Self-closing on non-void is allowed in tokenizer
        assert_eq!(token, Token::StartTag("div".into(), vec![], true));
    }
}
