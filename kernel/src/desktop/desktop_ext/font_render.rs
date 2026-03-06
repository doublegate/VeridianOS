//! TrueType Font Rendering
//!
//! TrueType parser with integer Bezier rasterization and glyph caching.
//! All math is integer-only (no floating point).

#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};

/// Errors during font parsing or rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// Invalid or unrecognized font data.
    InvalidFont,
    /// Required table not found.
    TableNotFound,
    /// Glyph index out of range.
    GlyphNotFound,
    /// Unsupported format version.
    UnsupportedFormat,
    /// Data truncated or corrupt.
    DataTruncated,
    /// Buffer too small for rendered glyph.
    BufferTooSmall,
}

impl core::fmt::Display for FontError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFont => write!(f, "invalid font"),
            Self::TableNotFound => write!(f, "table not found"),
            Self::GlyphNotFound => write!(f, "glyph not found"),
            Self::UnsupportedFormat => write!(f, "unsupported format"),
            Self::DataTruncated => write!(f, "data truncated"),
            Self::BufferTooSmall => write!(f, "buffer too small"),
        }
    }
}

/// Subpixel rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubpixelMode {
    /// No subpixel rendering (grayscale AA).
    #[default]
    None,
    /// RGB subpixel order (most common LCD).
    Rgb,
    /// BGR subpixel order.
    Bgr,
    /// Vertical RGB (rotated display).
    VerticalRgb,
    /// Vertical BGR.
    VerticalBgr,
}

/// A point in a glyph outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlinePoint {
    /// X coordinate in font units.
    pub x: i16,
    /// Y coordinate in font units.
    pub y: i16,
    /// Whether this is an on-curve control point.
    pub on_curve: bool,
}

/// A contour in a glyph outline (sequence of points).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct GlyphContour {
    /// Points forming this contour.
    pub points: Vec<OutlinePoint>,
}

/// A parsed glyph outline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct GlyphOutline {
    /// Contours forming this glyph.
    pub contours: Vec<GlyphContour>,
    /// Bounding box: min x.
    pub x_min: i16,
    /// Bounding box: min y.
    pub y_min: i16,
    /// Bounding box: max x.
    pub x_max: i16,
    /// Bounding box: max y.
    pub y_max: i16,
    /// Advance width in font units.
    pub advance_width: u16,
    /// Left side bearing.
    pub lsb: i16,
}

/// TrueType table tag (4-byte ASCII).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableTag(pub [u8; 4]);

impl TableTag {
    pub const CMAP: Self = Self(*b"cmap");
    pub const GLYF: Self = Self(*b"glyf");
    pub const HEAD: Self = Self(*b"head");
    pub const HHEA: Self = Self(*b"hhea");
    pub const HMTX: Self = Self(*b"hmtx");
    pub const LOCA: Self = Self(*b"loca");
    pub const MAXP: Self = Self(*b"maxp");
}

/// A table directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableEntry {
    pub tag: TableTag,
    pub offset: u32,
    pub length: u32,
}

/// Parsed `head` table fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadTable {
    /// Units per em (typically 1000 or 2048).
    pub units_per_em: u16,
    /// Index-to-loc format: 0=short, 1=long.
    pub index_to_loc_format: i16,
    /// Font bounding box.
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
}

/// Parsed `hhea` table fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HheaTable {
    /// Ascent.
    pub ascent: i16,
    /// Descent (negative).
    pub descent: i16,
    /// Line gap.
    pub line_gap: i16,
    /// Number of horizontal metrics in hmtx.
    pub num_h_metrics: u16,
}

/// Parsed `maxp` table fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxpTable {
    pub num_glyphs: u16,
}

/// Helper: read u16 big-endian from a byte slice.
pub(crate) fn read_u16_be(data: &[u8], offset: usize) -> Option<u16> {
    if offset + 2 > data.len() {
        return None;
    }
    Some(u16::from_be_bytes([data[offset], data[offset + 1]]))
}

/// Helper: read i16 big-endian.
pub(crate) fn read_i16_be(data: &[u8], offset: usize) -> Option<i16> {
    read_u16_be(data, offset).map(|v| v as i16)
}

/// Helper: read u32 big-endian.
pub(crate) fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }
    Some(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// TrueType font parser.
///
/// Parses the font directory and individual tables from raw TTF data.
/// Does not own the font data; operates on borrowed slices.
#[derive(Debug)]
pub struct TtfParser<'a> {
    /// Raw font file data.
    data: &'a [u8],
    /// Number of tables.
    num_tables: u16,
}

impl<'a> TtfParser<'a> {
    /// Create a new parser from raw TTF data.
    pub fn new(data: &'a [u8]) -> Result<Self, FontError> {
        if data.len() < 12 {
            return Err(FontError::InvalidFont);
        }

        // Check sfVersion (0x00010000 for TrueType, 'OTTO' for CFF).
        let version = read_u32_be(data, 0).ok_or(FontError::DataTruncated)?;
        if version != 0x00010000 && version != 0x4F54544F {
            return Err(FontError::InvalidFont);
        }

        let num_tables = read_u16_be(data, 4).ok_or(FontError::DataTruncated)?;

        Ok(Self { data, num_tables })
    }

    /// Find a table by tag.
    pub fn find_table(&self, tag: TableTag) -> Option<TableEntry> {
        let header_size = 12;
        let entry_size = 16;

        for i in 0..self.num_tables as usize {
            let offset = header_size + i * entry_size;
            if offset + entry_size > self.data.len() {
                break;
            }

            let t = [
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ];

            if t == tag.0 {
                let table_offset = read_u32_be(self.data, offset + 8)?;
                let length = read_u32_be(self.data, offset + 12)?;
                return Some(TableEntry {
                    tag,
                    offset: table_offset,
                    length,
                });
            }
        }
        None
    }

    /// Get the raw bytes for a table.
    pub fn table_data(&self, entry: &TableEntry) -> Result<&'a [u8], FontError> {
        let start = entry.offset as usize;
        let end = start + entry.length as usize;
        if end > self.data.len() {
            return Err(FontError::DataTruncated);
        }
        Ok(&self.data[start..end])
    }

    /// Parse the `head` table.
    pub fn parse_head(&self) -> Result<HeadTable, FontError> {
        let entry = self
            .find_table(TableTag::HEAD)
            .ok_or(FontError::TableNotFound)?;
        let d = self.table_data(&entry)?;
        if d.len() < 54 {
            return Err(FontError::DataTruncated);
        }

        Ok(HeadTable {
            units_per_em: read_u16_be(d, 18).ok_or(FontError::DataTruncated)?,
            x_min: read_i16_be(d, 36).ok_or(FontError::DataTruncated)?,
            y_min: read_i16_be(d, 38).ok_or(FontError::DataTruncated)?,
            x_max: read_i16_be(d, 40).ok_or(FontError::DataTruncated)?,
            y_max: read_i16_be(d, 42).ok_or(FontError::DataTruncated)?,
            index_to_loc_format: read_i16_be(d, 50).ok_or(FontError::DataTruncated)?,
        })
    }

    /// Parse the `hhea` table.
    pub fn parse_hhea(&self) -> Result<HheaTable, FontError> {
        let entry = self
            .find_table(TableTag::HHEA)
            .ok_or(FontError::TableNotFound)?;
        let d = self.table_data(&entry)?;
        if d.len() < 36 {
            return Err(FontError::DataTruncated);
        }

        Ok(HheaTable {
            ascent: read_i16_be(d, 4).ok_or(FontError::DataTruncated)?,
            descent: read_i16_be(d, 6).ok_or(FontError::DataTruncated)?,
            line_gap: read_i16_be(d, 8).ok_or(FontError::DataTruncated)?,
            num_h_metrics: read_u16_be(d, 34).ok_or(FontError::DataTruncated)?,
        })
    }

    /// Parse the `maxp` table.
    pub fn parse_maxp(&self) -> Result<MaxpTable, FontError> {
        let entry = self
            .find_table(TableTag::MAXP)
            .ok_or(FontError::TableNotFound)?;
        let d = self.table_data(&entry)?;
        if d.len() < 6 {
            return Err(FontError::DataTruncated);
        }

        Ok(MaxpTable {
            num_glyphs: read_u16_be(d, 4).ok_or(FontError::DataTruncated)?,
        })
    }

    /// Look up a glyph index from a character code using `cmap` table.
    /// Supports format 4 (BMP) cmap subtable.
    pub fn char_to_glyph(&self, ch: u32) -> Result<u16, FontError> {
        let entry = self
            .find_table(TableTag::CMAP)
            .ok_or(FontError::TableNotFound)?;
        let d = self.table_data(&entry)?;
        if d.len() < 4 {
            return Err(FontError::DataTruncated);
        }

        let num_subtables = read_u16_be(d, 2).ok_or(FontError::DataTruncated)?;

        // Find a Unicode (platform 0 or 3) subtable.
        for i in 0..num_subtables as usize {
            let rec_off = 4 + i * 8;
            if rec_off + 8 > d.len() {
                break;
            }
            let platform = read_u16_be(d, rec_off).ok_or(FontError::DataTruncated)?;
            let sub_offset = read_u32_be(d, rec_off + 4).ok_or(FontError::DataTruncated)? as usize;

            if platform != 0 && platform != 3 {
                continue;
            }

            if sub_offset + 6 > d.len() {
                continue;
            }

            let format = read_u16_be(d, sub_offset).ok_or(FontError::DataTruncated)?;

            if format == 4 {
                return self.cmap_format4_lookup(d, sub_offset, ch);
            }
        }

        Err(FontError::GlyphNotFound)
    }

    /// Format 4 cmap lookup (segmented mapping for BMP).
    fn cmap_format4_lookup(
        &self,
        cmap_data: &[u8],
        offset: usize,
        ch: u32,
    ) -> Result<u16, FontError> {
        if ch > 0xFFFF {
            return Err(FontError::GlyphNotFound);
        }
        let ch = ch as u16;

        let seg_count_x2 =
            read_u16_be(cmap_data, offset + 6).ok_or(FontError::DataTruncated)? as usize;
        let seg_count = seg_count_x2 / 2;

        let end_codes_off = offset + 14;
        // +2 for reserved pad
        let start_codes_off = end_codes_off + seg_count_x2 + 2;
        let id_delta_off = start_codes_off + seg_count_x2;
        let id_range_off = id_delta_off + seg_count_x2;

        for seg in 0..seg_count {
            let end_code =
                read_u16_be(cmap_data, end_codes_off + seg * 2).ok_or(FontError::DataTruncated)?;

            if ch > end_code {
                continue;
            }

            let start_code = read_u16_be(cmap_data, start_codes_off + seg * 2)
                .ok_or(FontError::DataTruncated)?;

            if ch < start_code {
                return Err(FontError::GlyphNotFound);
            }

            let id_delta =
                read_i16_be(cmap_data, id_delta_off + seg * 2).ok_or(FontError::DataTruncated)?;
            let id_range =
                read_u16_be(cmap_data, id_range_off + seg * 2).ok_or(FontError::DataTruncated)?;

            if id_range == 0 {
                return Ok((ch as i16).wrapping_add(id_delta) as u16);
            }

            let glyph_offset =
                id_range_off + seg * 2 + id_range as usize + (ch - start_code) as usize * 2;
            let glyph_id = read_u16_be(cmap_data, glyph_offset).ok_or(FontError::DataTruncated)?;

            if glyph_id == 0 {
                return Err(FontError::GlyphNotFound);
            }

            return Ok((glyph_id as i16).wrapping_add(id_delta) as u16);
        }

        Err(FontError::GlyphNotFound)
    }

    /// Get glyph offset from `loca` table.
    pub fn glyph_offset(&self, glyph_id: u16, head: &HeadTable) -> Result<(u32, u32), FontError> {
        let entry = self
            .find_table(TableTag::LOCA)
            .ok_or(FontError::TableNotFound)?;
        let d = self.table_data(&entry)?;

        if head.index_to_loc_format == 0 {
            // Short format: offset/2 stored as u16.
            let idx = glyph_id as usize * 2;
            let off1 = read_u16_be(d, idx).ok_or(FontError::DataTruncated)? as u32 * 2;
            let off2 = read_u16_be(d, idx + 2).ok_or(FontError::DataTruncated)? as u32 * 2;
            Ok((off1, off2))
        } else {
            // Long format: offsets stored as u32.
            let idx = glyph_id as usize * 4;
            let off1 = read_u32_be(d, idx).ok_or(FontError::DataTruncated)?;
            let off2 = read_u32_be(d, idx + 4).ok_or(FontError::DataTruncated)?;
            Ok((off1, off2))
        }
    }

    /// Parse a simple glyph outline from the `glyf` table.
    #[cfg(feature = "alloc")]
    pub fn parse_glyph(&self, glyph_id: u16) -> Result<GlyphOutline, FontError> {
        let head = self.parse_head()?;
        let (off1, off2) = self.glyph_offset(glyph_id, &head)?;

        if off1 == off2 {
            // Empty glyph (e.g., space).
            return Ok(GlyphOutline {
                contours: Vec::new(),
                x_min: 0,
                y_min: 0,
                x_max: 0,
                y_max: 0,
                advance_width: 0,
                lsb: 0,
            });
        }

        let glyf_entry = self
            .find_table(TableTag::GLYF)
            .ok_or(FontError::TableNotFound)?;
        let glyf_data = self.table_data(&glyf_entry)?;

        let glyph_start = off1 as usize;
        if glyph_start + 10 > glyf_data.len() {
            return Err(FontError::DataTruncated);
        }

        let num_contours = read_i16_be(glyf_data, glyph_start).ok_or(FontError::DataTruncated)?;
        let x_min = read_i16_be(glyf_data, glyph_start + 2).ok_or(FontError::DataTruncated)?;
        let y_min = read_i16_be(glyf_data, glyph_start + 4).ok_or(FontError::DataTruncated)?;
        let x_max = read_i16_be(glyf_data, glyph_start + 6).ok_or(FontError::DataTruncated)?;
        let y_max = read_i16_be(glyf_data, glyph_start + 8).ok_or(FontError::DataTruncated)?;

        if num_contours < 0 {
            // Compound glyph -- not parsed, return bounding box only.
            return Ok(GlyphOutline {
                contours: Vec::new(),
                x_min,
                y_min,
                x_max,
                y_max,
                advance_width: 0,
                lsb: 0,
            });
        }

        let num_contours = num_contours as usize;
        let mut cursor = glyph_start + 10;

        // Read end-points of each contour.
        let mut end_pts = Vec::with_capacity(num_contours);
        for _ in 0..num_contours {
            let ep = read_u16_be(glyf_data, cursor).ok_or(FontError::DataTruncated)?;
            end_pts.push(ep);
            cursor += 2;
        }

        let num_points = if let Some(&last) = end_pts.last() {
            last as usize + 1
        } else {
            return Ok(GlyphOutline {
                contours: Vec::new(),
                x_min,
                y_min,
                x_max,
                y_max,
                advance_width: 0,
                lsb: 0,
            });
        };

        // Skip instructions.
        let instruction_length =
            read_u16_be(glyf_data, cursor).ok_or(FontError::DataTruncated)? as usize;
        cursor += 2 + instruction_length;

        // Parse flags.
        let mut flags = Vec::with_capacity(num_points);
        while flags.len() < num_points {
            if cursor >= glyf_data.len() {
                return Err(FontError::DataTruncated);
            }
            let flag = glyf_data[cursor];
            cursor += 1;
            flags.push(flag);

            // Bit 3: repeat.
            if flag & 0x08 != 0 {
                if cursor >= glyf_data.len() {
                    return Err(FontError::DataTruncated);
                }
                let repeat = glyf_data[cursor] as usize;
                cursor += 1;
                for _ in 0..repeat {
                    if flags.len() < num_points {
                        flags.push(flag);
                    }
                }
            }
        }

        // Parse X coordinates.
        let mut x_coords = Vec::with_capacity(num_points);
        let mut x: i16 = 0;
        for flag in &flags {
            let short = flag & 0x02 != 0;
            let same_or_positive = flag & 0x10 != 0;

            if short {
                if cursor >= glyf_data.len() {
                    return Err(FontError::DataTruncated);
                }
                let dx = glyf_data[cursor] as i16;
                cursor += 1;
                x += if same_or_positive { dx } else { -dx };
            } else if !same_or_positive {
                let dx = read_i16_be(glyf_data, cursor).ok_or(FontError::DataTruncated)?;
                cursor += 2;
                x += dx;
            }
            // else: same_or_positive && !short => x unchanged.
            x_coords.push(x);
        }

        // Parse Y coordinates.
        let mut y_coords = Vec::with_capacity(num_points);
        let mut y: i16 = 0;
        for flag in &flags {
            let short = flag & 0x04 != 0;
            let same_or_positive = flag & 0x20 != 0;

            if short {
                if cursor >= glyf_data.len() {
                    return Err(FontError::DataTruncated);
                }
                let dy = glyf_data[cursor] as i16;
                cursor += 1;
                y += if same_or_positive { dy } else { -dy };
            } else if !same_or_positive {
                let dy = read_i16_be(glyf_data, cursor).ok_or(FontError::DataTruncated)?;
                cursor += 2;
                y += dy;
            }
            y_coords.push(y);
        }

        // Build contours.
        let mut contours = Vec::with_capacity(num_contours);
        let mut start = 0usize;
        for &end in &end_pts {
            let end = end as usize;
            let mut points = Vec::new();
            for idx in start..=end {
                if idx < num_points {
                    points.push(OutlinePoint {
                        x: x_coords[idx],
                        y: y_coords[idx],
                        on_curve: flags[idx] & 0x01 != 0,
                    });
                }
            }
            contours.push(GlyphContour { points });
            start = end + 1;
        }

        Ok(GlyphOutline {
            contours,
            x_min,
            y_min,
            x_max,
            y_max,
            advance_width: 0,
            lsb: 0,
        })
    }
}

/// Rendered glyph bitmap.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct GlyphBitmap {
    /// Grayscale pixel data (0 = transparent, 255 = opaque).
    pub data: Vec<u8>,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Left bearing in pixels.
    pub bearing_x: i32,
    /// Top bearing in pixels.
    pub bearing_y: i32,
    /// Advance width in pixels.
    pub advance: u32,
}

/// Glyph cache entry.
#[derive(Debug, Clone)]
#[cfg(feature = "alloc")]
struct GlyphCacheEntry {
    /// Character code.
    ch: u32,
    /// Rendered size in pixels.
    size_px: u16,
    /// Cached bitmap.
    bitmap: GlyphBitmap,
    /// Access count for LRU eviction.
    access_count: u32,
}

/// Maximum glyph cache entries.
pub(crate) const GLYPH_CACHE_SIZE: usize = 256;

/// Glyph cache with LRU eviction.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct GlyphCache {
    entries: Vec<GlyphCacheEntry>,
    total_lookups: u64,
    cache_hits: u64,
}

#[cfg(feature = "alloc")]
impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl GlyphCache {
    /// Create a new empty glyph cache.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            total_lookups: 0,
            cache_hits: 0,
        }
    }

    /// Look up a cached glyph.
    pub fn get(&mut self, ch: u32, size_px: u16) -> Option<&GlyphBitmap> {
        self.total_lookups += 1;

        let idx = self
            .entries
            .iter()
            .position(|e| e.ch == ch && e.size_px == size_px);

        if let Some(i) = idx {
            self.cache_hits += 1;
            self.entries[i].access_count += 1;
            Some(&self.entries[i].bitmap)
        } else {
            None
        }
    }

    /// Insert a glyph bitmap into the cache.
    pub fn insert(&mut self, ch: u32, size_px: u16, bitmap: GlyphBitmap) {
        // Evict LRU if at capacity.
        if self.entries.len() >= GLYPH_CACHE_SIZE {
            // Find the entry with the lowest access count.
            let min_idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.access_count)
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.entries.swap_remove(min_idx);
        }

        self.entries.push(GlyphCacheEntry {
            ch,
            size_px,
            bitmap,
            access_count: 1,
        });
    }

    /// Get cache hit rate as a percentage (0-100).
    pub fn hit_rate_percent(&self) -> u32 {
        if self.total_lookups == 0 {
            return 0;
        }
        ((self.cache_hits * 100) / self.total_lookups) as u32
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total_lookups = 0;
        self.cache_hits = 0;
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Rasterize a glyph outline to a grayscale bitmap using integer math.
///
/// `scale_num` / `scale_den` is the scaling factor (e.g., pixel_size /
/// units_per_em). Uses midpoint line drawing for on-curve segments and
/// quadratic Bezier subdivision for off-curve control points.
#[cfg(feature = "alloc")]
pub fn rasterize_outline(outline: &GlyphOutline, scale_num: u32, scale_den: u32) -> GlyphBitmap {
    if outline.contours.is_empty() || scale_den == 0 {
        return GlyphBitmap {
            data: Vec::new(),
            width: 0,
            height: 0,
            bearing_x: 0,
            bearing_y: 0,
            advance: 0,
        };
    }

    // Compute scaled bounding box.
    let scale = |v: i16| -> i32 { (v as i32 * scale_num as i32) / scale_den as i32 };

    let x_min = scale(outline.x_min);
    let y_min = scale(outline.y_min);
    let x_max = scale(outline.x_max);
    let y_max = scale(outline.y_max);

    let width = (x_max - x_min + 1).max(1) as u32;
    let height = (y_max - y_min + 1).max(1) as u32;

    // Clamp to reasonable size.
    let width = width.min(512);
    let height = height.min(512);

    let mut data = vec![0u8; (width * height) as usize];

    // Rasterize each contour using scanline edge tracking.
    for contour in &outline.contours {
        let points = &contour.points;
        if points.len() < 2 {
            continue;
        }

        let num = points.len();
        for i in 0..num {
            let p0 = &points[i];
            let p1 = &points[(i + 1) % num];

            let x0 = scale(p0.x) - x_min;
            let y0 = y_max - scale(p0.y);
            let x1 = scale(p1.x) - x_min;
            let y1 = y_max - scale(p1.y);

            if p0.on_curve && p1.on_curve {
                // Straight line segment.
                draw_line(&mut data, width, height, x0, y0, x1, y1);
            } else if !p1.on_curve && (i + 2) <= num {
                // Quadratic bezier: p0 on-curve, p1 off-curve, p2 on-curve.
                let p2 = &points[(i + 2) % num];
                let x2 = scale(p2.x) - x_min;
                let y2 = y_max - scale(p2.y);
                draw_quadratic_bezier(&mut data, width, height, x0, y0, x1, y1, x2, y2);
            }
        }
    }

    GlyphBitmap {
        data,
        width,
        height,
        bearing_x: x_min,
        bearing_y: y_max,
        advance: width,
    }
}

/// Draw a line using Bresenham's midpoint algorithm (integer only).
#[cfg(feature = "alloc")]
fn draw_line(buf: &mut [u8], w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) {
    let mut x0 = x0;
    let mut y0 = y0;

    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        // Plot pixel.
        if x0 >= 0 && (x0 as u32) < w && y0 >= 0 && (y0 as u32) < h {
            let idx = y0 as u32 * w + x0 as u32;
            if (idx as usize) < buf.len() {
                buf[idx as usize] = 255;
            }
        }

        if x0 == x1 && y0 == y1 {
            break;
        }

        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Draw a quadratic Bezier curve using recursive subdivision (integer only).
#[cfg(feature = "alloc")]
#[allow(clippy::too_many_arguments)]
fn draw_quadratic_bezier(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    cx: i32,
    cy: i32,
    x2: i32,
    y2: i32,
) {
    // Subdivision: if the control point is close to the midpoint of the
    // line p0-p2, just draw a line.
    let mx = (x0 + x2) / 2;
    let my = (y0 + y2) / 2;
    let dist = (cx - mx).abs() + (cy - my).abs();

    if dist <= 1 {
        draw_line(buf, w, h, x0, y0, x2, y2);
        return;
    }

    // Subdivide at midpoint.
    let ax = (x0 + cx) / 2;
    let ay = (y0 + cy) / 2;
    let bx = (cx + x2) / 2;
    let by = (cy + y2) / 2;
    let midx = (ax + bx) / 2;
    let midy = (ay + by) / 2;

    draw_quadratic_bezier(buf, w, h, x0, y0, ax, ay, midx, midy);
    draw_quadratic_bezier(buf, w, h, midx, midy, bx, by, x2, y2);
}

/// Hinting mode stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HintingMode {
    /// No hinting.
    #[default]
    None,
    /// Light hinting (vertical only).
    Light,
    /// Full hinting.
    Full,
    /// Auto-hinting (algorithmic).
    Auto,
}

/// Apply hinting to a glyph outline (stub -- returns outline unchanged).
#[cfg(feature = "alloc")]
pub fn apply_hinting(outline: &GlyphOutline, _mode: HintingMode) -> GlyphOutline {
    // Hinting is complex and typically requires bytecode interpretation.
    // This is a stub that returns the outline unchanged.
    outline.clone()
}

/// Render a character to a grayscale bitmap at the given pixel size.
///
/// This is the main entry point for glyph rendering. It:
/// 1. Looks up the glyph ID from the character code via `cmap`.
/// 2. Parses the glyph outline from `glyf`.
/// 3. Rasterizes the outline to a bitmap.
#[cfg(feature = "alloc")]
pub fn render_glyph(
    parser: &TtfParser<'_>,
    ch: char,
    pixel_size: u16,
) -> Result<GlyphBitmap, FontError> {
    let head = parser.parse_head()?;
    let glyph_id = parser.char_to_glyph(ch as u32)?;
    let outline = parser.parse_glyph(glyph_id)?;
    let bitmap = rasterize_outline(&outline, pixel_size as u32, head.units_per_em as u32);
    Ok(bitmap)
}
