//! Clipboard Protocol
//!
//! Wayland wl_data_device compatible clipboard with MIME type negotiation,
//! primary selection, and history.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

/// Errors that can occur during clipboard operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardError {
    /// The clipboard is empty.
    Empty,
    /// The requested MIME type is not available.
    MimeNotFound,
    /// History is full (should not happen as we evict oldest).
    HistoryFull,
    /// Invalid operation for the current selection type.
    InvalidSelection,
    /// Data too large for clipboard.
    DataTooLarge,
    /// Source has been destroyed.
    SourceDestroyed,
}

impl core::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "clipboard is empty"),
            Self::MimeNotFound => write!(f, "MIME type not found"),
            Self::HistoryFull => write!(f, "clipboard history full"),
            Self::InvalidSelection => write!(f, "invalid selection type"),
            Self::DataTooLarge => write!(f, "data too large"),
            Self::SourceDestroyed => write!(f, "source destroyed"),
        }
    }
}

/// MIME types supported by the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum ClipboardMime {
    /// Plain text (text/plain)
    TextPlain,
    /// UTF-8 plain text (text/plain;charset=utf-8)
    TextPlainUtf8,
    /// HTML (text/html)
    TextHtml,
    /// URI list (text/uri-list)
    TextUriList,
    /// PNG image (image/png)
    ImagePng,
    /// BMP image (image/bmp)
    ImageBmp,
    /// Custom/unknown MIME type (stored as hash)
    Custom(u32),
}

impl ClipboardMime {
    /// Return the MIME type string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TextPlain => "text/plain",
            Self::TextPlainUtf8 => "text/plain;charset=utf-8",
            Self::TextHtml => "text/html",
            Self::TextUriList => "text/uri-list",
            Self::ImagePng => "image/png",
            Self::ImageBmp => "image/bmp",
            Self::Custom(_) => "application/octet-stream",
        }
    }

    /// Check if this MIME type is a text type.
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            Self::TextPlain | Self::TextPlainUtf8 | Self::TextHtml | Self::TextUriList
        )
    }
}

/// Selection type for X11-style selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionType {
    /// Standard clipboard (Ctrl+C/V).
    #[default]
    Clipboard,
    /// Primary selection (mouse highlight).
    Primary,
}

/// A single clipboard entry with data and associated MIME types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEntry {
    /// Data for each MIME type offered.
    #[cfg(feature = "alloc")]
    pub mime_data: BTreeMap<ClipboardMime, Vec<u8>>,
    /// Timestamp (monotonic tick count when copied).
    pub timestamp: u64,
    /// Source surface ID (Wayland surface that set this data).
    pub source_surface: u32,
}

#[cfg(feature = "alloc")]
impl ClipboardEntry {
    /// Create a new clipboard entry.
    pub fn new(source_surface: u32, timestamp: u64) -> Self {
        Self {
            mime_data: BTreeMap::new(),
            timestamp,
            source_surface,
        }
    }

    /// Add data for a MIME type.
    pub fn set_data(&mut self, mime: ClipboardMime, data: Vec<u8>) {
        self.mime_data.insert(mime, data);
    }

    /// Get data for a specific MIME type.
    pub fn get_data(&self, mime: ClipboardMime) -> Option<&[u8]> {
        self.mime_data.get(&mime).map(|v| v.as_slice())
    }

    /// Get all offered MIME types.
    pub fn offered_mimes(&self) -> Vec<ClipboardMime> {
        self.mime_data.keys().copied().collect()
    }

    /// Check if this entry offers a specific MIME type.
    pub fn offers(&self, mime: ClipboardMime) -> bool {
        self.mime_data.contains_key(&mime)
    }

    /// Total size of all data in this entry.
    pub fn total_size(&self) -> usize {
        self.mime_data.values().map(|v| v.len()).sum()
    }
}

/// Maximum clipboard history entries.
pub(crate) const CLIPBOARD_HISTORY_MAX: usize = 8;

/// Maximum data size per clipboard entry (64 KB).
pub(crate) const CLIPBOARD_MAX_DATA_SIZE: usize = 65536;

/// Clipboard manager with history and primary selection support.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct ClipboardManager {
    /// Standard clipboard contents.
    clipboard: Option<ClipboardEntry>,
    /// Primary selection contents (mouse highlight).
    primary: Option<ClipboardEntry>,
    /// Clipboard history (most recent first).
    history: Vec<ClipboardEntry>,
    /// Whether clipboard history is enabled.
    history_enabled: bool,
    /// Monotonic timestamp counter.
    tick: u64,
}

#[cfg(feature = "alloc")]
impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl ClipboardManager {
    /// Create a new clipboard manager.
    pub fn new() -> Self {
        Self {
            clipboard: None,
            primary: None,
            history: Vec::new(),
            history_enabled: true,
            tick: 0,
        }
    }

    /// Copy data to the clipboard or primary selection.
    pub fn copy(
        &mut self,
        selection: SelectionType,
        source_surface: u32,
        mime: ClipboardMime,
        data: Vec<u8>,
    ) -> Result<(), ClipboardError> {
        if data.len() > CLIPBOARD_MAX_DATA_SIZE {
            return Err(ClipboardError::DataTooLarge);
        }

        self.tick += 1;
        let mut entry = ClipboardEntry::new(source_surface, self.tick);
        entry.set_data(mime, data);

        match selection {
            SelectionType::Clipboard => {
                // Push old clipboard to history if enabled.
                if self.history_enabled {
                    if let Some(old) = self.clipboard.take() {
                        self.push_history(old);
                    }
                }
                self.clipboard = Some(entry);
            }
            SelectionType::Primary => {
                self.primary = Some(entry);
            }
        }

        Ok(())
    }

    /// Copy data with multiple MIME representations.
    pub fn copy_multi(
        &mut self,
        selection: SelectionType,
        source_surface: u32,
        data: &[(ClipboardMime, Vec<u8>)],
    ) -> Result<(), ClipboardError> {
        let total_size: usize = data.iter().map(|(_, d)| d.len()).sum();
        if total_size > CLIPBOARD_MAX_DATA_SIZE {
            return Err(ClipboardError::DataTooLarge);
        }

        self.tick += 1;
        let mut entry = ClipboardEntry::new(source_surface, self.tick);
        for (mime, d) in data {
            entry.set_data(*mime, d.clone());
        }

        match selection {
            SelectionType::Clipboard => {
                if self.history_enabled {
                    if let Some(old) = self.clipboard.take() {
                        self.push_history(old);
                    }
                }
                self.clipboard = Some(entry);
            }
            SelectionType::Primary => {
                self.primary = Some(entry);
            }
        }

        Ok(())
    }

    /// Paste data from the clipboard or primary selection.
    pub fn paste(
        &self,
        selection: SelectionType,
        mime: ClipboardMime,
    ) -> Result<&[u8], ClipboardError> {
        let entry = match selection {
            SelectionType::Clipboard => self.clipboard.as_ref(),
            SelectionType::Primary => self.primary.as_ref(),
        };

        let entry = entry.ok_or(ClipboardError::Empty)?;
        entry.get_data(mime).ok_or(ClipboardError::MimeNotFound)
    }

    /// Get the list of MIME types available for pasting.
    pub fn available_mimes(&self, selection: SelectionType) -> Vec<ClipboardMime> {
        match selection {
            SelectionType::Clipboard => self
                .clipboard
                .as_ref()
                .map(|e| e.offered_mimes())
                .unwrap_or_default(),
            SelectionType::Primary => self
                .primary
                .as_ref()
                .map(|e| e.offered_mimes())
                .unwrap_or_default(),
        }
    }

    /// Clear the clipboard or primary selection.
    pub fn clear(&mut self, selection: SelectionType) {
        match selection {
            SelectionType::Clipboard => {
                self.clipboard = None;
            }
            SelectionType::Primary => {
                self.primary = None;
            }
        }
    }

    /// Get clipboard history entries.
    pub fn history(&self) -> &[ClipboardEntry] {
        &self.history
    }

    /// Restore a history entry to the current clipboard.
    pub fn restore_from_history(&mut self, index: usize) -> Result<(), ClipboardError> {
        if index >= self.history.len() {
            return Err(ClipboardError::Empty);
        }
        let entry = self.history.remove(index);
        if let Some(old) = self.clipboard.take() {
            self.push_history(old);
        }
        self.clipboard = Some(entry);
        Ok(())
    }

    /// Clear all history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Enable or disable clipboard history.
    pub fn set_history_enabled(&mut self, enabled: bool) {
        self.history_enabled = enabled;
        if !enabled {
            self.history.clear();
        }
    }

    /// Check if clipboard has data.
    pub fn has_data(&self, selection: SelectionType) -> bool {
        match selection {
            SelectionType::Clipboard => self.clipboard.is_some(),
            SelectionType::Primary => self.primary.is_some(),
        }
    }

    /// Negotiate the best MIME type between offered types and requested types.
    pub fn negotiate_mime(
        &self,
        selection: SelectionType,
        requested: &[ClipboardMime],
    ) -> Option<ClipboardMime> {
        let available = self.available_mimes(selection);
        // Return the first requested type that is available.
        requested.iter().find(|r| available.contains(r)).copied()
    }

    /// Push an entry to history, evicting oldest if at capacity.
    fn push_history(&mut self, entry: ClipboardEntry) {
        if self.history.len() >= CLIPBOARD_HISTORY_MAX {
            self.history.pop();
        }
        self.history.insert(0, entry);
    }

    /// Get current tick count.
    pub fn current_tick(&self) -> u64 {
        self.tick
    }
}
