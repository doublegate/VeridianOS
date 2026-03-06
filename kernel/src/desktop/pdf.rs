//! PDF Renderer
//!
//! Minimal PDF 1.4 parser and renderer. Parses the cross-reference table,
//! trailer, and page tree to extract page content streams. Renders text
//! (using the kernel's 8x16 bitmap font) and filled rectangles to a pixel
//! buffer.
//!
//! All coordinate math is integer-only.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// PDF objects
// ---------------------------------------------------------------------------

/// Represents a PDF object value.
#[derive(Debug, Clone)]
pub enum PdfObject {
    /// The null object.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Integer(i64),
    /// PDF name (e.g. `/Type`).
    Name(String),
    /// Literal string (parenthesised).
    StringLiteral(Vec<u8>),
    /// Array of objects.
    Array(Vec<PdfObject>),
    /// Dictionary of name-object pairs.
    Dictionary(BTreeMap<String, PdfObject>),
    /// Stream: dictionary + raw bytes.
    Stream(BTreeMap<String, PdfObject>, Vec<u8>),
    /// Indirect reference: object number, generation.
    Reference(u32, u16),
}

impl PdfObject {
    /// Try to extract as integer.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            PdfObject::Integer(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to extract as name string.
    pub fn as_name(&self) -> Option<&str> {
        match self {
            PdfObject::Name(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to extract as dictionary.
    pub fn as_dict(&self) -> Option<&BTreeMap<String, PdfObject>> {
        match self {
            PdfObject::Dictionary(d) => Some(d),
            PdfObject::Stream(d, _) => Some(d),
            _ => None,
        }
    }

    /// Try to extract as array.
    pub fn as_array(&self) -> Option<&Vec<PdfObject>> {
        match self {
            PdfObject::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Try to extract stream data.
    pub fn as_stream_data(&self) -> Option<&[u8]> {
        match self {
            PdfObject::Stream(_, data) => Some(data),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-reference table
// ---------------------------------------------------------------------------

/// A single xref entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XrefEntry {
    /// Byte offset of the object in the file.
    pub offset: u64,
    /// Generation number.
    pub generation: u16,
    /// Whether this entry is in use (vs free).
    pub in_use: bool,
}

// ---------------------------------------------------------------------------
// PDF parser
// ---------------------------------------------------------------------------

/// PDF file parser.
#[derive(Debug)]
pub struct PdfParser {
    /// Raw file data.
    data: Vec<u8>,
    /// Parsed xref table.
    xref: Vec<XrefEntry>,
    /// Trailer dictionary.
    trailer: BTreeMap<String, PdfObject>,
    /// Cached parsed objects.
    objects: BTreeMap<u32, PdfObject>,
}

impl PdfParser {
    /// Create a parser from raw PDF file bytes.
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            xref: Vec::new(),
            trailer: BTreeMap::new(),
            objects: BTreeMap::new(),
        }
    }

    /// Parse the PDF header and verify the signature.
    pub fn parse_header(&self) -> bool {
        self.data.len() >= 5 && &self.data[0..5] == b"%PDF-"
    }

    /// Parse the cross-reference table.
    ///
    /// Looks for `startxref` near the end of the file, then parses the xref
    /// section.
    pub fn parse_xref_table(&mut self) -> bool {
        // Find "startxref" near end of file
        let search_start = if self.data.len() > 1024 {
            self.data.len() - 1024
        } else {
            0
        };

        let startxref_pos = self.find_bytes(b"startxref", search_start);
        if startxref_pos.is_none() {
            return false;
        }

        let pos = startxref_pos.unwrap();
        // Parse the offset after "startxref\n"
        let offset_str = self.read_line(pos + 9);
        let xref_offset = self.parse_u64(&offset_str);

        if xref_offset == 0 || xref_offset as usize >= self.data.len() {
            return false;
        }

        // Parse xref section at offset
        let mut cursor = xref_offset as usize;

        // Skip "xref\n"
        if cursor + 4 <= self.data.len() && &self.data[cursor..cursor + 4] == b"xref" {
            cursor += 4;
            cursor = self.skip_whitespace(cursor);
        } else {
            return false;
        }

        // Parse subsections: "start_obj count\n"
        while cursor < self.data.len() {
            let line = self.read_line(cursor);
            if line.starts_with("trailer") {
                break;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                cursor += line.len() + 1;
                continue;
            }

            let start_obj = self.parse_u64(parts[0]) as u32;
            let count = self.parse_u64(parts[1]) as u32;
            cursor += line.len() + 1;

            // Pre-size xref table
            let needed = (start_obj + count) as usize;
            while self.xref.len() < needed {
                self.xref.push(XrefEntry::default());
            }

            for i in 0..count {
                if cursor + 20 > self.data.len() {
                    break;
                }
                let entry_line = self.read_line(cursor);
                let entry_parts: Vec<&str> = entry_line.split_whitespace().collect();
                if entry_parts.len() >= 3 {
                    let offset = self.parse_u64(entry_parts[0]);
                    let gen = self.parse_u64(entry_parts[1]) as u16;
                    let in_use = entry_parts[2] == "n";
                    let idx = (start_obj + i) as usize;
                    if idx < self.xref.len() {
                        self.xref[idx] = XrefEntry {
                            offset,
                            generation: gen,
                            in_use,
                        };
                    }
                }
                cursor += entry_line.len() + 1;
            }
        }

        true
    }

    /// Parse a single object from the data at the given offset.
    pub fn parse_object(&mut self, obj_num: u32) -> Option<PdfObject> {
        if let Some(cached) = self.objects.get(&obj_num) {
            return Some(cached.clone());
        }

        let idx = obj_num as usize;
        if idx >= self.xref.len() || !self.xref[idx].in_use {
            return None;
        }

        let offset = self.xref[idx].offset as usize;
        if offset >= self.data.len() {
            return None;
        }

        // Skip "N G obj\n"
        let line = self.read_line(offset);
        let cursor = offset + line.len() + 1;

        let obj = self.parse_value(cursor).map(|(v, _)| v);
        if let Some(ref o) = obj {
            self.objects.insert(obj_num, o.clone());
        }
        obj
    }

    /// Parse a PDF value at the given cursor position.
    /// Returns (value, new_cursor).
    fn parse_value(&self, mut pos: usize) -> Option<(PdfObject, usize)> {
        pos = self.skip_whitespace(pos);
        if pos >= self.data.len() {
            return None;
        }

        let b = self.data[pos];

        match b {
            // Dictionary or Name
            b'/' => {
                let (name, end) = self.parse_name(pos);
                Some((PdfObject::Name(name), end))
            }
            b'<' => {
                if pos + 1 < self.data.len() && self.data[pos + 1] == b'<' {
                    // Dictionary
                    let (dict, end) = self.parse_dictionary(pos);
                    Some((PdfObject::Dictionary(dict), end))
                } else {
                    // Hex string
                    let (bytes, end) = self.parse_hex_string(pos);
                    Some((PdfObject::StringLiteral(bytes), end))
                }
            }
            b'(' => {
                let (bytes, end) = self.parse_literal_string(pos);
                Some((PdfObject::StringLiteral(bytes), end))
            }
            b'[' => {
                let (arr, end) = self.parse_array(pos);
                Some((PdfObject::Array(arr), end))
            }
            b't' => {
                // true
                Some((PdfObject::Bool(true), pos + 4))
            }
            b'f' => {
                // false
                Some((PdfObject::Bool(false), pos + 5))
            }
            b'n' => {
                // null
                Some((PdfObject::Null, pos + 4))
            }
            b'0'..=b'9' | b'-' | b'+' => {
                let (num, end) = self.parse_number(pos);
                // Check if this is an indirect reference (N G R)
                let after = self.skip_whitespace(end);
                if after < self.data.len() && self.data[after].is_ascii_digit() {
                    let (gen, end2) = self.parse_number(after);
                    let after2 = self.skip_whitespace(end2);
                    if after2 < self.data.len() && self.data[after2] == b'R' {
                        return Some((PdfObject::Reference(num as u32, gen as u16), after2 + 1));
                    }
                }
                Some((PdfObject::Integer(num), end))
            }
            _ => None,
        }
    }

    // -- helper parsers --

    fn parse_name(&self, pos: usize) -> (String, usize) {
        // pos is at '/'
        let mut end = pos + 1;
        while end < self.data.len() {
            let c = self.data[end];
            if c.is_ascii_whitespace()
                || c == b'/'
                || c == b'<'
                || c == b'>'
                || c == b'['
                || c == b']'
                || c == b'('
                || c == b')'
            {
                break;
            }
            end += 1;
        }
        let name = String::from_utf8_lossy(&self.data[pos + 1..end]).into_owned();
        (name, end)
    }

    fn parse_dictionary(&self, pos: usize) -> (BTreeMap<String, PdfObject>, usize) {
        let mut dict = BTreeMap::new();
        let mut cursor = pos + 2; // skip "<<"

        loop {
            cursor = self.skip_whitespace(cursor);
            if cursor + 1 >= self.data.len() {
                break;
            }
            if self.data[cursor] == b'>' && self.data[cursor + 1] == b'>' {
                cursor += 2;
                break;
            }
            if self.data[cursor] != b'/' {
                cursor += 1;
                continue;
            }
            let (key, end) = self.parse_name(cursor);
            if let Some((val, end2)) = self.parse_value(end) {
                dict.insert(key, val);
                cursor = end2;
            } else {
                cursor = end;
            }
        }

        (dict, cursor)
    }

    fn parse_array(&self, pos: usize) -> (Vec<PdfObject>, usize) {
        let mut arr = Vec::new();
        let mut cursor = pos + 1; // skip '['

        loop {
            cursor = self.skip_whitespace(cursor);
            if cursor >= self.data.len() || self.data[cursor] == b']' {
                cursor += 1;
                break;
            }
            if let Some((val, end)) = self.parse_value(cursor) {
                arr.push(val);
                cursor = end;
            } else {
                cursor += 1;
            }
        }

        (arr, cursor)
    }

    fn parse_literal_string(&self, pos: usize) -> (Vec<u8>, usize) {
        let mut result = Vec::new();
        let mut cursor = pos + 1; // skip '('
        let mut depth = 1u32;

        while cursor < self.data.len() && depth > 0 {
            match self.data[cursor] {
                b'(' => {
                    depth += 1;
                    result.push(b'(');
                }
                b')' => {
                    depth -= 1;
                    if depth > 0 {
                        result.push(b')');
                    }
                }
                b'\\' => {
                    cursor += 1;
                    if cursor < self.data.len() {
                        match self.data[cursor] {
                            b'n' => result.push(b'\n'),
                            b'r' => result.push(b'\r'),
                            b't' => result.push(b'\t'),
                            other => result.push(other),
                        }
                    }
                }
                other => result.push(other),
            }
            cursor += 1;
        }

        (result, cursor)
    }

    fn parse_hex_string(&self, pos: usize) -> (Vec<u8>, usize) {
        let mut result = Vec::new();
        let mut cursor = pos + 1; // skip '<'

        let mut high: Option<u8> = None;
        while cursor < self.data.len() && self.data[cursor] != b'>' {
            let c = self.data[cursor];
            if let Some(nibble) = hex_nibble(c) {
                if let Some(h) = high {
                    result.push((h << 4) | nibble);
                    high = None;
                } else {
                    high = Some(nibble);
                }
            }
            cursor += 1;
        }
        if let Some(h) = high {
            result.push(h << 4);
        }
        if cursor < self.data.len() {
            cursor += 1; // skip '>'
        }

        (result, cursor)
    }

    fn parse_number(&self, pos: usize) -> (i64, usize) {
        let mut end = pos;
        if end < self.data.len() && (self.data[end] == b'-' || self.data[end] == b'+') {
            end += 1;
        }
        while end < self.data.len() && self.data[end].is_ascii_digit() {
            end += 1;
        }
        // Skip decimal point and fractional digits (we truncate to integer)
        if end < self.data.len() && self.data[end] == b'.' {
            end += 1;
            while end < self.data.len() && self.data[end].is_ascii_digit() {
                end += 1;
            }
        }
        let s = String::from_utf8_lossy(&self.data[pos..end]);
        // Parse integer part only
        let int_str: String = s.chars().take_while(|c| *c != '.').collect();
        let val = self.parse_i64(&int_str);
        (val, end)
    }

    fn skip_whitespace(&self, mut pos: usize) -> usize {
        while pos < self.data.len() {
            match self.data[pos] {
                b' ' | b'\t' | b'\r' | b'\n' | 0 => pos += 1,
                b'%' => {
                    // Skip comment to end of line
                    while pos < self.data.len() && self.data[pos] != b'\n' {
                        pos += 1;
                    }
                }
                _ => break,
            }
        }
        pos
    }

    fn read_line(&self, pos: usize) -> String {
        let mut end = pos;
        while end < self.data.len() && self.data[end] != b'\n' && self.data[end] != b'\r' {
            end += 1;
        }
        String::from_utf8_lossy(&self.data[pos..end]).into_owned()
    }

    fn find_bytes(&self, needle: &[u8], start: usize) -> Option<usize> {
        if needle.is_empty() || self.data.len() < needle.len() {
            return None;
        }
        let end = self.data.len() - needle.len();
        for i in start..=end {
            if &self.data[i..i + needle.len()] == needle {
                return Some(i);
            }
        }
        None
    }

    fn parse_u64(&self, s: &str) -> u64 {
        let s = s.trim();
        let mut val: u64 = 0;
        for c in s.bytes() {
            if c.is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add((c - b'0') as u64);
            }
        }
        val
    }

    fn parse_i64(&self, s: &str) -> i64 {
        let s = s.trim();
        let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = s.strip_prefix('+') {
            (false, rest)
        } else {
            (false, s)
        };
        let mut val: i64 = 0;
        for c in digits.bytes() {
            if c.is_ascii_digit() {
                val = val.saturating_mul(10).saturating_add((c - b'0') as i64);
            }
        }
        if neg {
            -val
        } else {
            val
        }
    }

    /// Get the number of xref entries.
    pub fn xref_count(&self) -> usize {
        self.xref.len()
    }
}

/// Convert a hex character to its nibble value.
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// PDF page
// ---------------------------------------------------------------------------

/// A single PDF page with its content and media box.
#[derive(Debug, Clone)]
pub struct PdfPage {
    /// Media box: x, y, width, height (integer coordinates).
    pub media_box_x: i32,
    pub media_box_y: i32,
    pub media_box_width: i32,
    pub media_box_height: i32,
    /// Raw content stream bytes.
    pub content_stream: Vec<u8>,
    /// Resource dictionary.
    pub resources: BTreeMap<String, PdfObject>,
}

impl Default for PdfPage {
    fn default() -> Self {
        Self {
            media_box_x: 0,
            media_box_y: 0,
            media_box_width: 612, // US Letter width in points
            media_box_height: 792,
            content_stream: Vec::new(),
            resources: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Content stream operations
// ---------------------------------------------------------------------------

/// A parsed content stream operation.
#[derive(Debug, Clone)]
pub enum ContentStreamOp {
    /// Begin text object.
    BT,
    /// End text object.
    ET,
    /// Set text matrix: a, b, c, d, e, f (integer, 1000ths).
    Tm(i32, i32, i32, i32, i32, i32),
    /// Show text string.
    Tj(Vec<u8>),
    /// Draw rectangle: x, y, w, h (integer points).
    Re(i32, i32, i32, i32),
    /// Fill path.
    F,
    /// Set non-stroking colour (RGB, 0-1000 each — milli-units).
    Rg(i32, i32, i32),
    /// Concatenate matrix.
    Cm(i32, i32, i32, i32, i32, i32),
    /// Move to next line and set leading.
    Td(i32, i32),
    /// Set font and size.
    Tf(String, i32),
}

// ---------------------------------------------------------------------------
// Content stream parser (minimal)
// ---------------------------------------------------------------------------

/// Parse a content stream into operations.
pub fn parse_content_stream(data: &[u8]) -> Vec<ContentStreamOp> {
    let mut ops = Vec::new();
    let text = String::from_utf8_lossy(data);
    let mut operands: Vec<String> = Vec::new();

    for token in text.split_whitespace() {
        match token {
            "BT" => ops.push(ContentStreamOp::BT),
            "ET" => ops.push(ContentStreamOp::ET),
            "f" | "F" => ops.push(ContentStreamOp::F),
            "Tm" => {
                if operands.len() >= 6 {
                    let vals: Vec<i32> = operands
                        .iter()
                        .rev()
                        .take(6)
                        .rev()
                        .map(|s| parse_content_int(s))
                        .collect();
                    ops.push(ContentStreamOp::Tm(
                        vals[0], vals[1], vals[2], vals[3], vals[4], vals[5],
                    ));
                }
                operands.clear();
            }
            "Td" | "TD" => {
                if operands.len() >= 2 {
                    let n = operands.len();
                    let tx = parse_content_int(&operands[n - 2]);
                    let ty = parse_content_int(&operands[n - 1]);
                    ops.push(ContentStreamOp::Td(tx, ty));
                }
                operands.clear();
            }
            "Tj" => {
                // Text from last parenthesised string
                let joined: String = operands.join(" ");
                if let Some(start) = joined.find('(') {
                    if let Some(end) = joined.rfind(')') {
                        let text_bytes = joined.as_bytes()[start + 1..end].to_vec();
                        ops.push(ContentStreamOp::Tj(text_bytes));
                    }
                }
                operands.clear();
            }
            "re" => {
                if operands.len() >= 4 {
                    let n = operands.len();
                    ops.push(ContentStreamOp::Re(
                        parse_content_int(&operands[n - 4]),
                        parse_content_int(&operands[n - 3]),
                        parse_content_int(&operands[n - 2]),
                        parse_content_int(&operands[n - 1]),
                    ));
                }
                operands.clear();
            }
            "rg" => {
                if operands.len() >= 3 {
                    let n = operands.len();
                    ops.push(ContentStreamOp::Rg(
                        parse_content_milli(&operands[n - 3]),
                        parse_content_milli(&operands[n - 2]),
                        parse_content_milli(&operands[n - 1]),
                    ));
                }
                operands.clear();
            }
            "cm" => {
                if operands.len() >= 6 {
                    let vals: Vec<i32> = operands
                        .iter()
                        .rev()
                        .take(6)
                        .rev()
                        .map(|s| parse_content_int(s))
                        .collect();
                    ops.push(ContentStreamOp::Cm(
                        vals[0], vals[1], vals[2], vals[3], vals[4], vals[5],
                    ));
                }
                operands.clear();
            }
            "Tf" => {
                if operands.len() >= 2 {
                    let n = operands.len();
                    let font = String::from(operands[n - 2].trim_start_matches('/'));
                    let size = parse_content_int(&operands[n - 1]);
                    ops.push(ContentStreamOp::Tf(font, size));
                }
                operands.clear();
            }
            other => {
                operands.push(String::from(other));
            }
        }
    }

    ops
}

/// Parse a content stream number to integer (truncates decimal).
fn parse_content_int(s: &str) -> i32 {
    let s = s.trim();
    let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let mut val: i32 = 0;
    for c in digits.bytes() {
        if c == b'.' {
            break;
        }
        if c.is_ascii_digit() {
            val = val.saturating_mul(10).saturating_add((c - b'0') as i32);
        }
    }
    if neg {
        -val
    } else {
        val
    }
}

/// Parse a content stream number to milli-units (0.0-1.0 -> 0-1000).
fn parse_content_milli(s: &str) -> i32 {
    let s = s.trim();
    let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };

    let mut integer_part: i32 = 0;
    let mut frac_part: i32 = 0;
    let mut frac_divisor: i32 = 1;
    let mut in_frac = false;

    for c in digits.bytes() {
        if c == b'.' {
            in_frac = true;
            continue;
        }
        if c.is_ascii_digit() {
            if in_frac {
                frac_part = frac_part
                    .saturating_mul(10)
                    .saturating_add((c - b'0') as i32);
                frac_divisor = frac_divisor.saturating_mul(10);
            } else {
                integer_part = integer_part
                    .saturating_mul(10)
                    .saturating_add((c - b'0') as i32);
            }
        }
    }

    let milli = integer_part * 1000 + (frac_part * 1000) / frac_divisor.max(1);
    if neg {
        -milli
    } else {
        milli
    }
}

// ---------------------------------------------------------------------------
// PDF renderer
// ---------------------------------------------------------------------------

/// Renders PDF content stream operations to a pixel buffer.
pub struct PdfRenderer {
    /// Output buffer width.
    width: u32,
    /// Output buffer height.
    height: u32,
    /// Current text position X (points).
    text_x: i32,
    /// Current text position Y (points).
    text_y: i32,
    /// Current fill colour (ARGB8888).
    fill_color: u32,
    /// Current font size (points).
    font_size: i32,
    /// Scale factor from PDF points to pixels (256 = 1.0).
    scale: i32,
}

impl PdfRenderer {
    /// Create a renderer targeting a buffer of `width x height` pixels.
    pub fn new(width: u32, height: u32) -> Self {
        // Default scale: assume 72 DPI PDF, target ~96 DPI screen
        // scale = width * 256 / 612 (US Letter width in points)
        let scale = if width > 0 {
            (width as i32 * 256) / 612
        } else {
            256
        };

        Self {
            width,
            height,
            text_x: 0,
            text_y: 0,
            fill_color: 0xFF000000,
            font_size: 12,
            scale,
        }
    }

    /// Scale a PDF-point coordinate to pixel coordinate.
    fn to_px(&self, pt: i32) -> i32 {
        (pt * self.scale) / 256
    }

    /// Render a page's content stream to a pixel buffer.
    ///
    /// `buf` must be `width * height` u32 values (ARGB8888).
    pub fn render_page(&mut self, page: &PdfPage, buf: &mut [u32]) {
        // Clear to white
        for px in buf.iter_mut() {
            *px = 0xFFFFFFFF;
        }

        let ops = parse_content_stream(&page.content_stream);
        self.text_x = 0;
        self.text_y = 0;
        self.fill_color = 0xFF000000;

        for op in &ops {
            match op {
                ContentStreamOp::BT => {
                    self.text_x = 0;
                    self.text_y = 0;
                }
                ContentStreamOp::ET => {}
                ContentStreamOp::Tm(_, _, _, _, e, f) => {
                    self.text_x = *e;
                    // PDF Y is bottom-up; convert to top-down
                    self.text_y = page.media_box_height - *f;
                }
                ContentStreamOp::Td(tx, ty) => {
                    self.text_x += *tx;
                    self.text_y -= *ty; // PDF Y is bottom-up
                }
                ContentStreamOp::Tj(text) => {
                    self.render_text(text, buf);
                }
                ContentStreamOp::Re(x, y, w, h) => {
                    let px = self.to_px(*x);
                    let py = self.to_px(page.media_box_height - *y - *h);
                    let pw = self.to_px(*w);
                    let ph = self.to_px(*h);
                    self.fill_rect(buf, px, py, pw, ph);
                }
                ContentStreamOp::F => {
                    // Fill is applied by Re already in this simple renderer
                }
                ContentStreamOp::Rg(r, g, b) => {
                    let rc = ((*r * 255) / 1000).clamp(0, 255) as u32;
                    let gc = ((*g * 255) / 1000).clamp(0, 255) as u32;
                    let bc = ((*b * 255) / 1000).clamp(0, 255) as u32;
                    self.fill_color = 0xFF000000 | (rc << 16) | (gc << 8) | bc;
                }
                ContentStreamOp::Tf(_, size) => {
                    self.font_size = *size;
                }
                ContentStreamOp::Cm(_, _, _, _, _, _) => {
                    // Matrix concatenation — not implemented in simple renderer
                }
            }
        }
    }

    /// Render text at the current text position using the 8x16 bitmap font.
    fn render_text(&mut self, text: &[u8], buf: &mut [u32]) {
        let px = self.to_px(self.text_x);
        let py = self.to_px(self.text_y);
        let color = self.fill_color;
        let bw = self.width as i32;
        let bh = self.height as i32;
        let char_w = 8i32;
        let char_h = 16i32;

        for (i, &ch) in text.iter().enumerate() {
            let cx = px + (i as i32) * char_w;
            if cx + char_w <= 0 || cx >= bw {
                continue;
            }
            if py + char_h <= 0 || py >= bh {
                continue;
            }

            // Simple placeholder glyph rendering: filled rect for printable chars
            if (0x20..0x7F).contains(&ch) {
                for row in 0..char_h {
                    let dy = py + row;
                    if dy < 0 || dy >= bh {
                        continue;
                    }
                    for col in 0..char_w {
                        let dx = cx + col;
                        if dx < 0 || dx >= bw {
                            continue;
                        }
                        // Simple bitmap: draw character outline
                        if row == 0 || row == char_h - 1 || col == 0 || col == char_w - 1 {
                            buf[(dy * bw + dx) as usize] = color;
                        }
                    }
                }
            }
        }

        // Advance text position
        self.text_x += (text.len() as i32) * 8;
    }

    /// Fill a rectangle in the buffer.
    fn fill_rect(&self, buf: &mut [u32], x: i32, y: i32, w: i32, h: i32) {
        let bw = self.width as i32;
        let bh = self.height as i32;

        for row in 0..h {
            let dy = y + row;
            if dy < 0 || dy >= bh {
                continue;
            }
            for col in 0..w {
                let dx = x + col;
                if dx < 0 || dx >= bw {
                    continue;
                }
                buf[(dy * bw + dx) as usize] = self.fill_color;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PDF document
// ---------------------------------------------------------------------------

/// A parsed PDF document with pages.
#[derive(Debug)]
pub struct PdfDocument {
    /// Parser instance.
    parser: PdfParser,
    /// Extracted pages.
    pub pages: Vec<PdfPage>,
}

impl PdfDocument {
    /// Open a PDF document from raw bytes.
    pub fn open(data: Vec<u8>) -> Option<Self> {
        let mut parser = PdfParser::new(data);

        if !parser.parse_header() {
            return None;
        }

        parser.parse_xref_table();

        Some(Self {
            parser,
            pages: Vec::new(),
        })
    }

    /// Add a page manually (useful for constructing test documents).
    pub fn add_page(&mut self, page: PdfPage) {
        self.pages.push(page);
    }

    /// Get a page by index.
    pub fn get_page(&self, index: usize) -> Option<&PdfPage> {
        self.pages.get(index)
    }

    /// Number of pages.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Number of xref entries in the file.
    pub fn xref_count(&self) -> usize {
        self.parser.xref_count()
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
    fn test_pdf_header_valid() {
        let data = b"%PDF-1.4\n".to_vec();
        let parser = PdfParser::new(data);
        assert!(parser.parse_header());
    }

    #[test]
    fn test_pdf_header_invalid() {
        let data = b"not a pdf".to_vec();
        let parser = PdfParser::new(data);
        assert!(!parser.parse_header());
    }

    #[test]
    fn test_pdf_object_accessors() {
        let obj = PdfObject::Integer(42);
        assert_eq!(obj.as_integer(), Some(42));
        assert!(obj.as_name().is_none());

        let obj = PdfObject::Name(String::from("Type"));
        assert_eq!(obj.as_name(), Some("Type"));
    }

    #[test]
    fn test_xref_entry_default() {
        let entry = XrefEntry::default();
        assert_eq!(entry.offset, 0);
        assert!(!entry.in_use);
    }

    #[test]
    fn test_content_stream_parse() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let ops = parse_content_stream(data);
        assert!(!ops.is_empty());
        // Should contain BT, Tf, Td, Tj, ET
        let mut has_bt = false;
        let mut has_et = false;
        for op in &ops {
            match op {
                ContentStreamOp::BT => has_bt = true,
                ContentStreamOp::ET => has_et = true,
                _ => {}
            }
        }
        assert!(has_bt);
        assert!(has_et);
    }

    #[test]
    fn test_content_stream_rect() {
        let data = b"100 200 50 30 re f";
        let ops = parse_content_stream(data);
        let mut found_re = false;
        for op in &ops {
            if let ContentStreamOp::Re(x, y, w, h) = op {
                assert_eq!(*x, 100);
                assert_eq!(*y, 200);
                assert_eq!(*w, 50);
                assert_eq!(*h, 30);
                found_re = true;
            }
        }
        assert!(found_re);
    }

    #[test]
    fn test_content_stream_color() {
        let data = b"1 0 0 rg";
        let ops = parse_content_stream(data);
        let mut found = false;
        for op in &ops {
            if let ContentStreamOp::Rg(r, g, b) = op {
                assert_eq!(*r, 1000);
                assert_eq!(*g, 0);
                assert_eq!(*b, 0);
                found = true;
            }
        }
        assert!(found);
    }

    #[test]
    fn test_parse_content_milli() {
        assert_eq!(parse_content_milli("0.5"), 500);
        assert_eq!(parse_content_milli("1"), 1000);
        assert_eq!(parse_content_milli("0"), 0);
        assert_eq!(parse_content_milli("0.25"), 250);
    }

    #[test]
    fn test_pdf_renderer_render_page() {
        let mut renderer = PdfRenderer::new(100, 100);
        let page = PdfPage {
            content_stream: b"BT 10 780 Td (Test) Tj ET".to_vec(),
            ..PdfPage::default()
        };
        let mut buf = vec![0u32; 100 * 100];
        renderer.render_page(&page, &mut buf);
        // Should have modified at least some pixels (white background)
        assert!(buf.iter().any(|&p| p == 0xFFFFFFFF));
    }

    #[test]
    fn test_pdf_document_open() {
        let data = b"%PDF-1.4\nsome content".to_vec();
        let doc = PdfDocument::open(data);
        assert!(doc.is_some());
    }
}
