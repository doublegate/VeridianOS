//! Desktop Extension Modules
//!
//! Provides advanced desktop functionality for the VeridianOS desktop
//! environment:
//!
//! 1. **Clipboard Protocol** -- Wayland wl_data_device compatible clipboard
//!    with MIME type negotiation, primary selection, and history.
//! 2. **Drag-and-Drop** -- wl_data_offer protocol with enter/leave/drop/motion
//!    events.
//! 3. **Global Keyboard Shortcuts** -- Configurable key bindings with modifier
//!    masks.
//! 4. **Theme Engine** -- Color schemes (light/dark/solarized/nord/dracula)
//!    with runtime switching.
//! 5. **Font Rendering** -- TrueType parser with integer Bezier rasterization
//!    and glyph caching.
//! 6. **CJK Unicode** -- Wide character detection, double-width cell rendering,
//!    and IME framework.
//!
//! All math is integer-only (no floating point). Uses fixed-point 8.8 or 16.16
//! where fractional precision is needed.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

// ============================================================================
// Section 1: Clipboard Protocol (~500 lines)
// ============================================================================

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
const CLIPBOARD_HISTORY_MAX: usize = 8;

/// Maximum data size per clipboard entry (64 KB).
const CLIPBOARD_MAX_DATA_SIZE: usize = 65536;

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

// ============================================================================
// Section 2: Drag-and-Drop (~450 lines)
// ============================================================================

/// Errors that can occur during drag-and-drop operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndError {
    /// No drag operation is in progress.
    NotDragging,
    /// A drag is already in progress.
    AlreadyDragging,
    /// The drop target rejected the drop.
    DropRejected,
    /// No MIME type matches between source and target.
    NoMimeMatch,
    /// Invalid surface ID.
    InvalidSurface,
    /// The drag operation was cancelled.
    Cancelled,
}

impl core::fmt::Display for DndError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotDragging => write!(f, "not dragging"),
            Self::AlreadyDragging => write!(f, "already dragging"),
            Self::DropRejected => write!(f, "drop rejected"),
            Self::NoMimeMatch => write!(f, "no MIME match"),
            Self::InvalidSurface => write!(f, "invalid surface"),
            Self::Cancelled => write!(f, "drag cancelled"),
        }
    }
}

/// State machine for drag-and-drop operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DndState {
    /// No drag operation active.
    #[default]
    Idle,
    /// A drag is in progress (user holding mouse button).
    Dragging,
    /// Over a valid drop target, waiting for drop confirmation.
    DropPending,
    /// Drop was accepted and data transfer is happening.
    Transferring,
}

/// Events emitted by the DnD subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndEvent {
    /// Drag entered a surface.
    Enter { surface_id: u32, x: i32, y: i32 },
    /// Drag left a surface.
    Leave { surface_id: u32 },
    /// Drag moved within a surface.
    Motion { surface_id: u32, x: i32, y: i32 },
    /// Drop occurred on a surface.
    Drop { surface_id: u32, x: i32, y: i32 },
    /// Drag was cancelled.
    Cancelled,
}

/// Information about a drag source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct DragSource {
    /// Surface ID that initiated the drag.
    pub source_surface: u32,
    /// MIME types offered by the source.
    pub offered_mimes: Vec<ClipboardMime>,
    /// Position where drag started.
    pub origin_x: i32,
    pub origin_y: i32,
    /// Visual feedback: ghost image dimensions (width, height).
    pub ghost_width: u32,
    pub ghost_height: u32,
}

/// Information about a drop target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct DropTarget {
    /// Surface ID that can receive drops.
    pub surface_id: u32,
    /// MIME types accepted by this target.
    pub accepted_mimes: Vec<ClipboardMime>,
    /// Bounding box (x, y, width, height).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "alloc")]
impl DropTarget {
    /// Check if a point is within this drop target's bounds.
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width as i32)
            && py >= self.y
            && py < self.y.saturating_add(self.height as i32)
    }
}

/// Drag-and-drop manager.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct DndManager {
    /// Current DnD state.
    state: DndState,
    /// Active drag source (if dragging).
    source: Option<DragSource>,
    /// Currently hovered surface.
    hover_surface: Option<u32>,
    /// Current cursor position during drag.
    cursor_x: i32,
    cursor_y: i32,
    /// Registered drop targets.
    targets: Vec<DropTarget>,
    /// Pending events.
    events: Vec<DndEvent>,
}

#[cfg(feature = "alloc")]
impl Default for DndManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl DndManager {
    /// Create a new DnD manager.
    pub fn new() -> Self {
        Self {
            state: DndState::Idle,
            source: None,
            hover_surface: None,
            cursor_x: 0,
            cursor_y: 0,
            targets: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Begin a drag operation.
    pub fn start_drag(
        &mut self,
        source_surface: u32,
        offered_mimes: Vec<ClipboardMime>,
        origin_x: i32,
        origin_y: i32,
        ghost_width: u32,
        ghost_height: u32,
    ) -> Result<(), DndError> {
        if self.state != DndState::Idle {
            return Err(DndError::AlreadyDragging);
        }

        self.source = Some(DragSource {
            source_surface,
            offered_mimes,
            origin_x,
            origin_y,
            ghost_width,
            ghost_height,
        });
        self.state = DndState::Dragging;
        self.cursor_x = origin_x;
        self.cursor_y = origin_y;

        Ok(())
    }

    /// Update cursor position during drag. Performs hit testing and emits
    /// events.
    pub fn motion(&mut self, x: i32, y: i32) -> Result<(), DndError> {
        if self.state != DndState::Dragging && self.state != DndState::DropPending {
            return Err(DndError::NotDragging);
        }

        self.cursor_x = x;
        self.cursor_y = y;

        // Hit-test against registered drop targets.
        let hit = self.targets.iter().find(|t| t.contains(x, y));

        let new_surface = hit.map(|t| t.surface_id);
        let old_surface = self.hover_surface;

        // Emit leave/enter events on surface change.
        if new_surface != old_surface {
            if let Some(old_id) = old_surface {
                self.events.push(DndEvent::Leave { surface_id: old_id });
            }
            if let Some(new_id) = new_surface {
                self.events.push(DndEvent::Enter {
                    surface_id: new_id,
                    x,
                    y,
                });
                self.state = DndState::DropPending;
            } else {
                self.state = DndState::Dragging;
            }
            self.hover_surface = new_surface;
        } else if let Some(sid) = new_surface {
            self.events.push(DndEvent::Motion {
                surface_id: sid,
                x,
                y,
            });
        }

        Ok(())
    }

    /// Perform a drop at the current position.
    pub fn drop_action(&mut self) -> Result<DndEvent, DndError> {
        if self.state != DndState::DropPending {
            return Err(DndError::NotDragging);
        }

        let surface_id = self.hover_surface.ok_or(DndError::InvalidSurface)?;
        let source = self.source.as_ref().ok_or(DndError::NotDragging)?;

        // Check MIME compatibility.
        let target = self
            .targets
            .iter()
            .find(|t| t.surface_id == surface_id)
            .ok_or(DndError::InvalidSurface)?;

        let has_match = source
            .offered_mimes
            .iter()
            .any(|m| target.accepted_mimes.contains(m));

        if !has_match {
            self.cancel();
            return Err(DndError::NoMimeMatch);
        }

        let event = DndEvent::Drop {
            surface_id,
            x: self.cursor_x,
            y: self.cursor_y,
        };
        self.events.push(event);

        self.state = DndState::Transferring;

        Ok(event)
    }

    /// Cancel the current drag operation.
    pub fn cancel(&mut self) {
        if let Some(sid) = self.hover_surface.take() {
            self.events.push(DndEvent::Leave { surface_id: sid });
        }
        self.events.push(DndEvent::Cancelled);
        self.source = None;
        self.state = DndState::Idle;
    }

    /// Complete the data transfer (called after successful drop).
    pub fn finish_transfer(&mut self) {
        self.source = None;
        self.hover_surface = None;
        self.state = DndState::Idle;
    }

    /// Register a drop target.
    pub fn register_target(&mut self, target: DropTarget) {
        // Remove existing target with same surface ID.
        self.targets.retain(|t| t.surface_id != target.surface_id);
        self.targets.push(target);
    }

    /// Unregister a drop target.
    pub fn unregister_target(&mut self, surface_id: u32) {
        self.targets.retain(|t| t.surface_id != surface_id);
    }

    /// Get current DnD state.
    pub fn state(&self) -> DndState {
        self.state
    }

    /// Get cursor position during drag.
    pub fn cursor_position(&self) -> (i32, i32) {
        (self.cursor_x, self.cursor_y)
    }

    /// Get the active drag source info.
    pub fn source(&self) -> Option<&DragSource> {
        self.source.as_ref()
    }

    /// Get the ghost image position (centered on cursor).
    pub fn ghost_position(&self) -> Option<(i32, i32, u32, u32)> {
        self.source.as_ref().map(|s| {
            (
                self.cursor_x - (s.ghost_width as i32 / 2),
                self.cursor_y - (s.ghost_height as i32 / 2),
                s.ghost_width,
                s.ghost_height,
            )
        })
    }

    /// Drain pending events.
    pub fn drain_events(&mut self) -> Vec<DndEvent> {
        core::mem::take(&mut self.events)
    }

    /// Find the best matching MIME between source and a specific target.
    pub fn negotiate_mime(&self, surface_id: u32) -> Option<ClipboardMime> {
        let source = self.source.as_ref()?;
        let target = self.targets.iter().find(|t| t.surface_id == surface_id)?;
        source
            .offered_mimes
            .iter()
            .find(|m| target.accepted_mimes.contains(m))
            .copied()
    }
}

// ============================================================================
// Section 3: Global Keyboard Shortcuts (~400 lines)
// ============================================================================

/// Modifier key bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModifierMask(pub u8);

impl ModifierMask {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const SUPER: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);

    /// Check if a modifier is set.
    pub fn has(self, modifier: Self) -> bool {
        (self.0 & modifier.0) == modifier.0
    }

    /// Combine two modifier masks.
    pub fn combine(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Remove a modifier.
    pub fn remove(self, modifier: Self) -> Self {
        Self(self.0 & !modifier.0)
    }

    /// Check if this mask is empty (no modifiers).
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Key code (PS/2 scancode or virtual key code).
pub type KeyCode = u8;

/// Actions that can be triggered by keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Launch the application launcher.
    LaunchLauncher,
    /// Launch terminal.
    LaunchTerminal,
    /// Launch file manager.
    LaunchFileManager,
    /// Close the focused window.
    CloseWindow,
    /// Minimize the focused window.
    MinimizeWindow,
    /// Maximize/restore the focused window.
    MaximizeWindow,
    /// Toggle fullscreen on focused window.
    ToggleFullscreen,
    /// Switch to workspace N (0-based).
    SwitchWorkspace(u8),
    /// Move window to workspace N (0-based).
    MoveToWorkspace(u8),
    /// Switch to next window (Alt+Tab).
    SwitchNextWindow,
    /// Switch to previous window (Alt+Shift+Tab).
    SwitchPrevWindow,
    /// Take a screenshot.
    Screenshot,
    /// Take a screenshot of the focused window.
    ScreenshotWindow,
    /// Lock the screen.
    LockScreen,
    /// Log out.
    Logout,
    /// Snap window left.
    SnapLeft,
    /// Snap window right.
    SnapRight,
    /// Copy (Ctrl+C).
    Copy,
    /// Paste (Ctrl+V).
    Paste,
    /// Cut (Ctrl+X).
    Cut,
    /// Undo (Ctrl+Z).
    Undo,
    /// Redo (Ctrl+Shift+Z or Ctrl+Y).
    Redo,
    /// Custom action identified by ID.
    Custom(u16),
}

/// Priority for shortcut matching (higher wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ShortcutPriority {
    /// System-level shortcuts (cannot be overridden).
    System = 3,
    /// Desktop environment shortcuts.
    Desktop = 2,
    /// Application shortcuts.
    Application = 1,
    /// User-defined shortcuts.
    #[default]
    User = 0,
}

/// A keyboard shortcut binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    /// Required modifier keys.
    pub modifiers: ModifierMask,
    /// The key code.
    pub key: KeyCode,
    /// Action to perform.
    pub action: ShortcutAction,
    /// Priority for conflict resolution.
    pub priority: ShortcutPriority,
    /// Whether this binding is currently enabled.
    pub enabled: bool,
}

impl KeyBinding {
    /// Create a new key binding.
    pub fn new(modifiers: ModifierMask, key: KeyCode, action: ShortcutAction) -> Self {
        Self {
            modifiers,
            key,
            action,
            priority: ShortcutPriority::User,
            enabled: true,
        }
    }

    /// Create a new key binding with priority.
    pub fn with_priority(mut self, priority: ShortcutPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Check if this binding matches the given modifiers and key.
    pub fn matches(&self, modifiers: ModifierMask, key: KeyCode) -> bool {
        self.enabled && self.modifiers == modifiers && self.key == key
    }
}

/// Maximum number of registered shortcuts.
const MAX_SHORTCUTS: usize = 128;

/// Keyboard shortcut manager.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct ShortcutManager {
    /// Registered bindings.
    bindings: Vec<KeyBinding>,
    /// Whether shortcut processing is globally enabled.
    enabled: bool,
    /// Binding IDs for removal (index into bindings).
    next_id: u32,
    /// Map from binding ID to index.
    id_map: BTreeMap<u32, usize>,
}

#[cfg(feature = "alloc")]
impl Default for ShortcutManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl ShortcutManager {
    /// Create a new shortcut manager with default system bindings.
    pub fn new() -> Self {
        let mut mgr = Self {
            bindings: Vec::new(),
            enabled: true,
            next_id: 0,
            id_map: BTreeMap::new(),
        };
        mgr.register_defaults();
        mgr
    }

    /// Register default system shortcuts.
    fn register_defaults(&mut self) {
        // Alt+Tab: switch window
        self.register(
            KeyBinding::new(ModifierMask::ALT, 0x0F, ShortcutAction::SwitchNextWindow)
                .with_priority(ShortcutPriority::System),
        );
        // Ctrl+Alt+L: lock screen
        self.register(
            KeyBinding::new(
                ModifierMask(ModifierMask::CTRL.0 | ModifierMask::ALT.0),
                0x26,
                ShortcutAction::LockScreen,
            )
            .with_priority(ShortcutPriority::System),
        );
        // Super: launcher
        self.register(
            KeyBinding::new(ModifierMask::SUPER, 0xDB, ShortcutAction::LaunchLauncher)
                .with_priority(ShortcutPriority::Desktop),
        );
        // Ctrl+C: copy
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2E, ShortcutAction::Copy)
                .with_priority(ShortcutPriority::Application),
        );
        // Ctrl+V: paste
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2F, ShortcutAction::Paste)
                .with_priority(ShortcutPriority::Application),
        );
        // Ctrl+X: cut
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2D, ShortcutAction::Cut)
                .with_priority(ShortcutPriority::Application),
        );
        // Alt+F4: close window
        self.register(
            KeyBinding::new(ModifierMask::ALT, 0x3E, ShortcutAction::CloseWindow)
                .with_priority(ShortcutPriority::Desktop),
        );
        // Print Screen (scancode 0x37 with E0 prefix): screenshot
        self.register(
            KeyBinding::new(ModifierMask::NONE, 0xB7, ShortcutAction::Screenshot)
                .with_priority(ShortcutPriority::System),
        );
    }

    /// Register a new shortcut binding. Returns binding ID.
    pub fn register(&mut self, binding: KeyBinding) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        if self.bindings.len() < MAX_SHORTCUTS {
            self.id_map.insert(id, self.bindings.len());
            self.bindings.push(binding);
        }

        id
    }

    /// Remove a shortcut by ID.
    pub fn unregister(&mut self, id: u32) -> bool {
        if let Some(&index) = self.id_map.get(&id) {
            if index < self.bindings.len() {
                self.bindings.remove(index);
                self.id_map.remove(&id);
                // Rebuild ID map (indices shifted).
                let mut new_map = BTreeMap::new();
                for (&k, &v) in &self.id_map {
                    if v > index {
                        new_map.insert(k, v - 1);
                    } else {
                        new_map.insert(k, v);
                    }
                }
                self.id_map = new_map;
                return true;
            }
        }
        false
    }

    /// Process a key event and return the matching action (if any).
    /// Returns the highest-priority matching action.
    pub fn process_key(&self, modifiers: ModifierMask, key: KeyCode) -> Option<ShortcutAction> {
        if !self.enabled {
            return None;
        }

        self.bindings
            .iter()
            .filter(|b| b.matches(modifiers, key))
            .max_by_key(|b| b.priority)
            .map(|b| b.action)
    }

    /// Enable or disable all shortcut processing.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if shortcuts are enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the number of registered bindings.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Get all bindings for a given action.
    pub fn bindings_for_action(&self, action: ShortcutAction) -> Vec<&KeyBinding> {
        self.bindings
            .iter()
            .filter(|b| b.action == action)
            .collect()
    }

    /// Enable or disable a specific binding by ID.
    pub fn set_binding_enabled(&mut self, id: u32, enabled: bool) -> bool {
        if let Some(&index) = self.id_map.get(&id) {
            if let Some(binding) = self.bindings.get_mut(index) {
                binding.enabled = enabled;
                return true;
            }
        }
        false
    }
}

// ============================================================================
// Section 4: Theme Engine (~500 lines)
// ============================================================================

/// Named color schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreset {
    /// Light theme with white backgrounds.
    Light,
    /// Dark theme with dark backgrounds.
    #[default]
    Dark,
    /// Solarized Dark.
    SolarizedDark,
    /// Solarized Light.
    SolarizedLight,
    /// Nord theme.
    Nord,
    /// Dracula theme.
    Dracula,
    /// Custom (user-defined).
    Custom,
}

/// ARGB color (alpha in high byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColor(pub u32);

impl ThemeColor {
    /// Create a color from ARGB components.
    pub const fn from_argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Create a fully opaque color from RGB.
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self::from_argb(0xFF, r, g, b)
    }

    /// Get the alpha component.
    pub const fn alpha(self) -> u8 {
        (self.0 >> 24) as u8
    }

    /// Get the red component.
    pub const fn red(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// Get the green component.
    pub const fn green(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// Get the blue component.
    pub const fn blue(self) -> u8 {
        self.0 as u8
    }

    /// Blend two colors using integer alpha blending.
    /// `alpha_256` is 0-256 (not 0-255) for shift-based division.
    pub fn blend(self, other: Self, alpha_256: u32) -> Self {
        let inv = 256 - alpha_256;
        let r = ((self.red() as u32 * inv) + (other.red() as u32 * alpha_256)) >> 8;
        let g = ((self.green() as u32 * inv) + (other.green() as u32 * alpha_256)) >> 8;
        let b = ((self.blue() as u32 * inv) + (other.blue() as u32 * alpha_256)) >> 8;
        Self::from_rgb(r as u8, g as u8, b as u8)
    }

    /// Darken a color by a percentage (0-100).
    pub fn darken(self, percent: u32) -> Self {
        let factor = 100u32.saturating_sub(percent);
        let r = (self.red() as u32 * factor) / 100;
        let g = (self.green() as u32 * factor) / 100;
        let b = (self.blue() as u32 * factor) / 100;
        Self::from_argb(self.alpha(), r as u8, g as u8, b as u8)
    }

    /// Lighten a color by a percentage (0-100).
    pub fn lighten(self, percent: u32) -> Self {
        let factor = percent;
        let r = self.red() as u32 + ((255 - self.red() as u32) * factor) / 100;
        let g = self.green() as u32 + ((255 - self.green() as u32) * factor) / 100;
        let b = self.blue() as u32 + ((255 - self.blue() as u32) * factor) / 100;
        Self::from_argb(
            self.alpha(),
            r.min(255) as u8,
            g.min(255) as u8,
            b.min(255) as u8,
        )
    }
}

/// Color slots in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    // Window
    pub window_background: ThemeColor,
    pub window_foreground: ThemeColor,
    pub window_border: ThemeColor,
    pub window_border_focused: ThemeColor,

    // Title bar
    pub titlebar_background: ThemeColor,
    pub titlebar_foreground: ThemeColor,
    pub titlebar_background_inactive: ThemeColor,
    pub titlebar_foreground_inactive: ThemeColor,

    // Buttons
    pub button_background: ThemeColor,
    pub button_foreground: ThemeColor,
    pub button_hover: ThemeColor,
    pub button_pressed: ThemeColor,

    // Accent / selection
    pub accent: ThemeColor,
    pub selection_background: ThemeColor,
    pub selection_foreground: ThemeColor,

    // Desktop
    pub desktop_background: ThemeColor,
    pub panel_background: ThemeColor,
    pub panel_foreground: ThemeColor,

    // Text
    pub text_primary: ThemeColor,
    pub text_secondary: ThemeColor,
    pub text_disabled: ThemeColor,

    // Status colors
    pub error: ThemeColor,
    pub warning: ThemeColor,
    pub success: ThemeColor,
    pub info: ThemeColor,

    // Scrollbar
    pub scrollbar_track: ThemeColor,
    pub scrollbar_thumb: ThemeColor,

    // Tooltip
    pub tooltip_background: ThemeColor,
    pub tooltip_foreground: ThemeColor,
}

impl ThemeColors {
    /// Create the default dark theme.
    pub const fn dark() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0x2D, 0x2D, 0x2D),
            window_foreground: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
            window_border: ThemeColor::from_rgb(0x44, 0x44, 0x44),
            window_border_focused: ThemeColor::from_rgb(0x5A, 0x9F, 0xD4),
            titlebar_background: ThemeColor::from_rgb(0x38, 0x38, 0x38),
            titlebar_foreground: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
            titlebar_background_inactive: ThemeColor::from_rgb(0x30, 0x30, 0x30),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x80, 0x80, 0x80),
            button_background: ThemeColor::from_rgb(0x45, 0x45, 0x45),
            button_foreground: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
            button_hover: ThemeColor::from_rgb(0x55, 0x55, 0x55),
            button_pressed: ThemeColor::from_rgb(0x35, 0x35, 0x35),
            accent: ThemeColor::from_rgb(0x5A, 0x9F, 0xD4),
            selection_background: ThemeColor::from_rgb(0x26, 0x4F, 0x78),
            selection_foreground: ThemeColor::from_rgb(0xFF, 0xFF, 0xFF),
            desktop_background: ThemeColor::from_rgb(0x1A, 0x1A, 0x2E),
            panel_background: ThemeColor::from_rgb(0x20, 0x20, 0x20),
            panel_foreground: ThemeColor::from_rgb(0xD0, 0xD0, 0xD0),
            text_primary: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
            text_secondary: ThemeColor::from_rgb(0xA0, 0xA0, 0xA0),
            text_disabled: ThemeColor::from_rgb(0x60, 0x60, 0x60),
            error: ThemeColor::from_rgb(0xE0, 0x50, 0x50),
            warning: ThemeColor::from_rgb(0xE0, 0xA0, 0x30),
            success: ThemeColor::from_rgb(0x50, 0xC8, 0x78),
            info: ThemeColor::from_rgb(0x5A, 0x9F, 0xD4),
            scrollbar_track: ThemeColor::from_rgb(0x30, 0x30, 0x30),
            scrollbar_thumb: ThemeColor::from_rgb(0x55, 0x55, 0x55),
            tooltip_background: ThemeColor::from_rgb(0x40, 0x40, 0x40),
            tooltip_foreground: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
        }
    }

    /// Create the light theme.
    pub const fn light() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0xF5, 0xF5, 0xF5),
            window_foreground: ThemeColor::from_rgb(0x20, 0x20, 0x20),
            window_border: ThemeColor::from_rgb(0xCC, 0xCC, 0xCC),
            window_border_focused: ThemeColor::from_rgb(0x33, 0x7A, 0xB7),
            titlebar_background: ThemeColor::from_rgb(0xE8, 0xE8, 0xE8),
            titlebar_foreground: ThemeColor::from_rgb(0x20, 0x20, 0x20),
            titlebar_background_inactive: ThemeColor::from_rgb(0xF0, 0xF0, 0xF0),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x80, 0x80, 0x80),
            button_background: ThemeColor::from_rgb(0xE0, 0xE0, 0xE0),
            button_foreground: ThemeColor::from_rgb(0x20, 0x20, 0x20),
            button_hover: ThemeColor::from_rgb(0xD0, 0xD0, 0xD0),
            button_pressed: ThemeColor::from_rgb(0xC0, 0xC0, 0xC0),
            accent: ThemeColor::from_rgb(0x33, 0x7A, 0xB7),
            selection_background: ThemeColor::from_rgb(0xB3, 0xD4, 0xFC),
            selection_foreground: ThemeColor::from_rgb(0x00, 0x00, 0x00),
            desktop_background: ThemeColor::from_rgb(0xDE, 0xDE, 0xE8),
            panel_background: ThemeColor::from_rgb(0xF0, 0xF0, 0xF0),
            panel_foreground: ThemeColor::from_rgb(0x30, 0x30, 0x30),
            text_primary: ThemeColor::from_rgb(0x20, 0x20, 0x20),
            text_secondary: ThemeColor::from_rgb(0x60, 0x60, 0x60),
            text_disabled: ThemeColor::from_rgb(0xA0, 0xA0, 0xA0),
            error: ThemeColor::from_rgb(0xD3, 0x2F, 0x2F),
            warning: ThemeColor::from_rgb(0xF5, 0x7C, 0x00),
            success: ThemeColor::from_rgb(0x38, 0x8E, 0x3C),
            info: ThemeColor::from_rgb(0x19, 0x76, 0xD2),
            scrollbar_track: ThemeColor::from_rgb(0xE8, 0xE8, 0xE8),
            scrollbar_thumb: ThemeColor::from_rgb(0xB0, 0xB0, 0xB0),
            tooltip_background: ThemeColor::from_rgb(0x30, 0x30, 0x30),
            tooltip_foreground: ThemeColor::from_rgb(0xF0, 0xF0, 0xF0),
        }
    }

    /// Create the Solarized Dark theme.
    pub const fn solarized_dark() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0x00, 0x2B, 0x36),
            window_foreground: ThemeColor::from_rgb(0x83, 0x94, 0x96),
            window_border: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            window_border_focused: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            titlebar_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            titlebar_foreground: ThemeColor::from_rgb(0x93, 0xA1, 0xA1),
            titlebar_background_inactive: ThemeColor::from_rgb(0x00, 0x2B, 0x36),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x58, 0x6E, 0x75),
            button_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            button_foreground: ThemeColor::from_rgb(0x93, 0xA1, 0xA1),
            button_hover: ThemeColor::from_rgb(0x0A, 0x43, 0x50),
            button_pressed: ThemeColor::from_rgb(0x05, 0x2A, 0x33),
            accent: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            selection_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            selection_foreground: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
            desktop_background: ThemeColor::from_rgb(0x00, 0x2B, 0x36),
            panel_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            panel_foreground: ThemeColor::from_rgb(0x83, 0x94, 0x96),
            text_primary: ThemeColor::from_rgb(0x83, 0x94, 0x96),
            text_secondary: ThemeColor::from_rgb(0x58, 0x6E, 0x75),
            text_disabled: ThemeColor::from_rgb(0x3B, 0x51, 0x50),
            error: ThemeColor::from_rgb(0xDC, 0x32, 0x2F),
            warning: ThemeColor::from_rgb(0xCB, 0x4B, 0x16),
            success: ThemeColor::from_rgb(0x85, 0x99, 0x00),
            info: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            scrollbar_track: ThemeColor::from_rgb(0x00, 0x2B, 0x36),
            scrollbar_thumb: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            tooltip_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            tooltip_foreground: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
        }
    }

    /// Create the Solarized Light theme.
    pub const fn solarized_light() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
            window_foreground: ThemeColor::from_rgb(0x65, 0x7B, 0x83),
            window_border: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            window_border_focused: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            titlebar_background: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            titlebar_foreground: ThemeColor::from_rgb(0x58, 0x6E, 0x75),
            titlebar_background_inactive: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x93, 0xA1, 0xA1),
            button_background: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            button_foreground: ThemeColor::from_rgb(0x58, 0x6E, 0x75),
            button_hover: ThemeColor::from_rgb(0xE0, 0xDA, 0xC7),
            button_pressed: ThemeColor::from_rgb(0xD3, 0xCD, 0xBB),
            accent: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            selection_background: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            selection_foreground: ThemeColor::from_rgb(0x00, 0x2B, 0x36),
            desktop_background: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
            panel_background: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            panel_foreground: ThemeColor::from_rgb(0x65, 0x7B, 0x83),
            text_primary: ThemeColor::from_rgb(0x65, 0x7B, 0x83),
            text_secondary: ThemeColor::from_rgb(0x93, 0xA1, 0xA1),
            text_disabled: ThemeColor::from_rgb(0xC0, 0xBB, 0xAA),
            error: ThemeColor::from_rgb(0xDC, 0x32, 0x2F),
            warning: ThemeColor::from_rgb(0xCB, 0x4B, 0x16),
            success: ThemeColor::from_rgb(0x85, 0x99, 0x00),
            info: ThemeColor::from_rgb(0x26, 0x8B, 0xD2),
            scrollbar_track: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
            scrollbar_thumb: ThemeColor::from_rgb(0xEE, 0xE8, 0xD5),
            tooltip_background: ThemeColor::from_rgb(0x07, 0x36, 0x42),
            tooltip_foreground: ThemeColor::from_rgb(0xFD, 0xF6, 0xE3),
        }
    }

    /// Create the Nord theme.
    pub const fn nord() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0x2E, 0x34, 0x40),
            window_foreground: ThemeColor::from_rgb(0xD8, 0xDE, 0xE9),
            window_border: ThemeColor::from_rgb(0x3B, 0x42, 0x52),
            window_border_focused: ThemeColor::from_rgb(0x88, 0xC0, 0xD0),
            titlebar_background: ThemeColor::from_rgb(0x3B, 0x42, 0x52),
            titlebar_foreground: ThemeColor::from_rgb(0xEC, 0xEF, 0xF4),
            titlebar_background_inactive: ThemeColor::from_rgb(0x2E, 0x34, 0x40),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x4C, 0x56, 0x6A),
            button_background: ThemeColor::from_rgb(0x43, 0x4C, 0x5E),
            button_foreground: ThemeColor::from_rgb(0xEC, 0xEF, 0xF4),
            button_hover: ThemeColor::from_rgb(0x4C, 0x56, 0x6A),
            button_pressed: ThemeColor::from_rgb(0x3B, 0x42, 0x52),
            accent: ThemeColor::from_rgb(0x88, 0xC0, 0xD0),
            selection_background: ThemeColor::from_rgb(0x43, 0x4C, 0x5E),
            selection_foreground: ThemeColor::from_rgb(0xEC, 0xEF, 0xF4),
            desktop_background: ThemeColor::from_rgb(0x2E, 0x34, 0x40),
            panel_background: ThemeColor::from_rgb(0x3B, 0x42, 0x52),
            panel_foreground: ThemeColor::from_rgb(0xD8, 0xDE, 0xE9),
            text_primary: ThemeColor::from_rgb(0xD8, 0xDE, 0xE9),
            text_secondary: ThemeColor::from_rgb(0x81, 0xA1, 0xC1),
            text_disabled: ThemeColor::from_rgb(0x4C, 0x56, 0x6A),
            error: ThemeColor::from_rgb(0xBF, 0x61, 0x6A),
            warning: ThemeColor::from_rgb(0xEB, 0xCB, 0x8B),
            success: ThemeColor::from_rgb(0xA3, 0xBE, 0x8C),
            info: ThemeColor::from_rgb(0x88, 0xC0, 0xD0),
            scrollbar_track: ThemeColor::from_rgb(0x2E, 0x34, 0x40),
            scrollbar_thumb: ThemeColor::from_rgb(0x4C, 0x56, 0x6A),
            tooltip_background: ThemeColor::from_rgb(0x3B, 0x42, 0x52),
            tooltip_foreground: ThemeColor::from_rgb(0xEC, 0xEF, 0xF4),
        }
    }

    /// Create the Dracula theme.
    pub const fn dracula() -> Self {
        Self {
            window_background: ThemeColor::from_rgb(0x28, 0x2A, 0x36),
            window_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            window_border: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            window_border_focused: ThemeColor::from_rgb(0xBD, 0x93, 0xF9),
            titlebar_background: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            titlebar_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            titlebar_background_inactive: ThemeColor::from_rgb(0x28, 0x2A, 0x36),
            titlebar_foreground_inactive: ThemeColor::from_rgb(0x62, 0x72, 0xA4),
            button_background: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            button_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            button_hover: ThemeColor::from_rgb(0x55, 0x58, 0x6E),
            button_pressed: ThemeColor::from_rgb(0x38, 0x3A, 0x4A),
            accent: ThemeColor::from_rgb(0xBD, 0x93, 0xF9),
            selection_background: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            selection_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            desktop_background: ThemeColor::from_rgb(0x28, 0x2A, 0x36),
            panel_background: ThemeColor::from_rgb(0x21, 0x22, 0x2C),
            panel_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            text_primary: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
            text_secondary: ThemeColor::from_rgb(0x62, 0x72, 0xA4),
            text_disabled: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            error: ThemeColor::from_rgb(0xFF, 0x55, 0x55),
            warning: ThemeColor::from_rgb(0xFF, 0xB8, 0x6C),
            success: ThemeColor::from_rgb(0x50, 0xFA, 0x7B),
            info: ThemeColor::from_rgb(0x8B, 0xE9, 0xFD),
            scrollbar_track: ThemeColor::from_rgb(0x28, 0x2A, 0x36),
            scrollbar_thumb: ThemeColor::from_rgb(0x44, 0x47, 0x5A),
            tooltip_background: ThemeColor::from_rgb(0x28, 0x2A, 0x36),
            tooltip_foreground: ThemeColor::from_rgb(0xF8, 0xF8, 0xF2),
        }
    }
}

/// GTK/Qt-style property key for theme mapping stubs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleProperty {
    BackgroundColor,
    ForegroundColor,
    BorderColor,
    BorderWidth,
    BorderRadius,
    FontSize,
    FontWeight,
    Padding,
    Margin,
    Opacity,
}

/// Icon theme name stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconTheme {
    #[default]
    Adwaita,
    Breeze,
    Papirus,
    Custom,
}

/// Theme manager with runtime switching.
#[derive(Debug)]
pub struct ThemeManager {
    /// Current active theme preset.
    current_preset: ThemePreset,
    /// Resolved colors for the current theme.
    colors: ThemeColors,
    /// Current icon theme.
    icon_theme: IconTheme,
    /// Whether animations should follow theme (affects durations).
    animate_transitions: bool,
    /// Custom color overrides (slot index -> color).
    #[cfg(feature = "alloc")]
    custom_overrides: BTreeMap<u8, ThemeColor>,
}

impl Default for ThemeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeManager {
    /// Create a new theme manager with the default dark theme.
    pub fn new() -> Self {
        Self {
            current_preset: ThemePreset::Dark,
            colors: ThemeColors::dark(),
            icon_theme: IconTheme::Adwaita,
            animate_transitions: true,
            #[cfg(feature = "alloc")]
            custom_overrides: BTreeMap::new(),
        }
    }

    /// Switch to a named theme preset.
    pub fn set_theme(&mut self, preset: ThemePreset) {
        self.current_preset = preset;
        self.colors = match preset {
            ThemePreset::Light => ThemeColors::light(),
            ThemePreset::Dark => ThemeColors::dark(),
            ThemePreset::SolarizedDark => ThemeColors::solarized_dark(),
            ThemePreset::SolarizedLight => ThemeColors::solarized_light(),
            ThemePreset::Nord => ThemeColors::nord(),
            ThemePreset::Dracula => ThemeColors::dracula(),
            ThemePreset::Custom => self.colors, // Keep current
        };
    }

    /// Get current theme colors.
    pub fn colors(&self) -> &ThemeColors {
        &self.colors
    }

    /// Get current theme preset.
    pub fn current_preset(&self) -> ThemePreset {
        self.current_preset
    }

    /// Set custom colors directly.
    pub fn set_colors(&mut self, colors: ThemeColors) {
        self.current_preset = ThemePreset::Custom;
        self.colors = colors;
    }

    /// Set icon theme.
    pub fn set_icon_theme(&mut self, theme: IconTheme) {
        self.icon_theme = theme;
    }

    /// Get icon theme.
    pub fn icon_theme(&self) -> IconTheme {
        self.icon_theme
    }

    /// Set whether to animate theme transitions.
    pub fn set_animate_transitions(&mut self, animate: bool) {
        self.animate_transitions = animate;
    }

    /// Check if theme transitions should be animated.
    pub fn animate_transitions(&self) -> bool {
        self.animate_transitions
    }

    /// Map a GTK/Qt-style property to the current theme (stub).
    /// Returns the u32 color or size value for the property.
    pub fn map_style_property(&self, property: StyleProperty) -> u32 {
        match property {
            StyleProperty::BackgroundColor => self.colors.window_background.0,
            StyleProperty::ForegroundColor => self.colors.window_foreground.0,
            StyleProperty::BorderColor => self.colors.window_border.0,
            StyleProperty::BorderWidth => 1,
            StyleProperty::BorderRadius => 4,
            StyleProperty::FontSize => 14,
            StyleProperty::FontWeight => 400,
            StyleProperty::Padding => 8,
            StyleProperty::Margin => 4,
            StyleProperty::Opacity => 255,
        }
    }

    /// Get the GTK theme name string for this preset (stub for GTK
    /// integration).
    pub fn gtk_theme_name(&self) -> &'static str {
        match self.current_preset {
            ThemePreset::Light => "Adwaita",
            ThemePreset::Dark => "Adwaita-dark",
            ThemePreset::SolarizedDark | ThemePreset::SolarizedLight => "Solarized",
            ThemePreset::Nord => "Nordic",
            ThemePreset::Dracula => "Dracula",
            ThemePreset::Custom => "Custom",
        }
    }

    /// Get the Qt theme variant for this preset (stub for Qt integration).
    pub fn qt_style_hint(&self) -> u32 {
        match self.current_preset {
            ThemePreset::Light | ThemePreset::SolarizedLight => 0, // Light
            _ => 1,                                                // Dark
        }
    }
}

// ============================================================================
// Section 5: Font Rendering / TrueType Parser (~800 lines)
// ============================================================================

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
fn read_u16_be(data: &[u8], offset: usize) -> Option<u16> {
    if offset + 2 > data.len() {
        return None;
    }
    Some(u16::from_be_bytes([data[offset], data[offset + 1]]))
}

/// Helper: read i16 big-endian.
fn read_i16_be(data: &[u8], offset: usize) -> Option<i16> {
    read_u16_be(data, offset).map(|v| v as i16)
}

/// Helper: read u32 big-endian.
fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
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
const GLYPH_CACHE_SIZE: usize = 256;

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

// ============================================================================
// Section 6: CJK Unicode / Wide Character Support (~350 lines)
// ============================================================================

/// Check if a character is a CJK wide character (occupies 2 cells).
///
/// Based on Unicode East Asian Width property and common CJK ranges:
/// - CJK Unified Ideographs (U+4E00-U+9FFF)
/// - CJK Unified Ideographs Extension A (U+3400-U+4DBF)
/// - CJK Compatibility Ideographs (U+F900-U+FAFF)
/// - Hangul Syllables (U+AC00-U+D7AF)
/// - Katakana (U+30A0-U+30FF)
/// - Hiragana (U+3040-U+309F)
/// - CJK Symbols and Punctuation (U+3000-U+303F)
/// - Fullwidth Forms (U+FF01-U+FF60, U+FFE0-U+FFE6)
/// - Bopomofo (U+3100-U+312F)
/// - Enclosed CJK (U+3200-U+32FF)
/// - CJK Compatibility (U+3300-U+33FF)
/// - CJK Unified Ideographs Extension B+ (U+20000-U+2A6DF)
pub fn is_cjk_wide(ch: char) -> bool {
    let cp = ch as u32;

    // Check the most common ranges first for performance.
    if (0x4E00..=0x9FFF).contains(&cp) {
        return true;
    }
    if (0xAC00..=0xD7AF).contains(&cp) {
        return true;
    }
    if (0x3040..=0x30FF).contains(&cp) {
        return true;
    }
    if (0xFF01..=0xFF60).contains(&cp) {
        return true;
    }
    if (0xFFE0..=0xFFE6).contains(&cp) {
        return true;
    }
    if (0x3400..=0x4DBF).contains(&cp) {
        return true;
    }
    if (0x3000..=0x303F).contains(&cp) {
        return true;
    }
    if (0x3100..=0x312F).contains(&cp) {
        return true;
    }
    if (0x3200..=0x33FF).contains(&cp) {
        return true;
    }
    if (0xF900..=0xFAFF).contains(&cp) {
        return true;
    }
    if (0x20000..=0x2A6DF).contains(&cp) {
        return true;
    }

    false
}

/// Get the display width of a character in terminal cells.
///
/// Returns 2 for wide (CJK) characters, 0 for zero-width characters
/// (combining marks, control chars), and 1 for everything else.
pub fn char_width(ch: char) -> u8 {
    let cp = ch as u32;

    // Control characters and zero-width.
    if cp == 0 || (0x01..=0x1F).contains(&cp) || cp == 0x7F {
        return 0;
    }

    // Combining marks (general category Mn/Mc/Me).
    if (0x0300..=0x036F).contains(&cp) {
        return 0; // Combining Diacritical Marks
    }
    if (0x1AB0..=0x1AFF).contains(&cp) {
        return 0; // Combining Diacritical Marks Extended
    }
    if (0x1DC0..=0x1DFF).contains(&cp) {
        return 0; // Combining Diacritical Marks Supplement
    }
    if (0x20D0..=0x20FF).contains(&cp) {
        return 0; // Combining Diacritical Marks for Symbols
    }
    if (0xFE20..=0xFE2F).contains(&cp) {
        return 0; // Combining Half Marks
    }

    // Soft hyphen.
    if cp == 0x00AD {
        return 1;
    }

    // Zero-width joiner / non-joiner / space.
    if cp == 0x200B || cp == 0x200C || cp == 0x200D || cp == 0xFEFF {
        return 0;
    }

    if is_cjk_wide(ch) {
        return 2;
    }

    1
}

/// Calculate the display width of a string in terminal cells.
#[cfg(feature = "alloc")]
pub fn string_width(s: &str) -> usize {
    s.chars().map(|c| char_width(c) as usize).sum()
}

/// Truncate a string to fit within `max_width` terminal cells.
/// Appends "..." if truncated.
#[cfg(feature = "alloc")]
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width < 3 {
        return String::new();
    }

    let mut width = 0usize;
    let mut result = String::new();

    for ch in s.chars() {
        let cw = char_width(ch) as usize;
        if width + cw > max_width - 3 {
            result.push_str("...");
            return result;
        }
        result.push(ch);
        width += cw;
    }

    result
}

/// Double-width cell renderer helper.
///
/// When rendering a wide character at cell (col, row), it occupies
/// cells (col, row) and (col+1, row). The second cell should be marked
/// as a continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellContent {
    /// Normal single-width character.
    Narrow(char),
    /// First cell of a wide character.
    WideStart(char),
    /// Continuation of a wide character (second cell).
    WideContinuation,
    /// Empty cell.
    #[default]
    Empty,
}

/// Input Method Editor (IME) state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeState {
    /// IME is inactive (direct input).
    #[default]
    Inactive,
    /// Composing: user is typing a sequence that will be converted.
    Composing,
    /// Committed: the composed text has been finalized.
    Committed,
}

/// A candidate in the IME candidate list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "alloc")]
pub struct ImeCandidate {
    /// Display label (e.g., "1", "2").
    pub label: String,
    /// The candidate text.
    pub text: String,
}

/// Input Method Editor framework.
///
/// Provides the state machine and data structures for input composition.
/// Actual input method dictionaries would be loaded from user space.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct InputMethodEditor {
    /// Current IME state.
    state: ImeState,
    /// Preedit (composing) string.
    preedit: String,
    /// Cursor position within preedit.
    preedit_cursor: usize,
    /// Candidate list.
    candidates: Vec<ImeCandidate>,
    /// Selected candidate index.
    selected_candidate: usize,
    /// Committed text (ready for insertion).
    committed: String,
    /// Whether the IME is enabled.
    enabled: bool,
    /// Pinyin lookup table (stub).
    pinyin_table: BTreeMap<String, Vec<String>>,
}

#[cfg(feature = "alloc")]
impl Default for InputMethodEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl InputMethodEditor {
    /// Create a new IME with basic Pinyin stub entries.
    pub fn new() -> Self {
        let mut pinyin_table = BTreeMap::new();

        // Basic Pinyin stub entries for common characters.
        pinyin_table.insert(
            String::from("ni"),
            vec![String::from("\u{4F60}"), String::from("\u{5C3C}")],
        );
        pinyin_table.insert(
            String::from("hao"),
            vec![String::from("\u{597D}"), String::from("\u{53F7}")],
        );
        pinyin_table.insert(
            String::from("shi"),
            vec![
                String::from("\u{662F}"),
                String::from("\u{4E16}"),
                String::from("\u{4E8B}"),
            ],
        );
        pinyin_table.insert(
            String::from("de"),
            vec![String::from("\u{7684}"), String::from("\u{5F97}")],
        );
        pinyin_table.insert(String::from("wo"), vec![String::from("\u{6211}")]);
        pinyin_table.insert(
            String::from("ren"),
            vec![String::from("\u{4EBA}"), String::from("\u{8BA4}")],
        );
        pinyin_table.insert(
            String::from("da"),
            vec![String::from("\u{5927}"), String::from("\u{6253}")],
        );
        pinyin_table.insert(
            String::from("zhong"),
            vec![String::from("\u{4E2D}"), String::from("\u{91CD}")],
        );
        pinyin_table.insert(
            String::from("guo"),
            vec![String::from("\u{56FD}"), String::from("\u{8FC7}")],
        );
        pinyin_table.insert(
            String::from("yi"),
            vec![
                String::from("\u{4E00}"),
                String::from("\u{4E49}"),
                String::from("\u{5DF2}"),
            ],
        );

        Self {
            state: ImeState::Inactive,
            preedit: String::new(),
            preedit_cursor: 0,
            candidates: Vec::new(),
            selected_candidate: 0,
            committed: String::new(),
            enabled: false,
            pinyin_table,
        }
    }

    /// Enable or disable the IME.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.reset();
        }
    }

    /// Check if IME is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get current IME state.
    pub fn state(&self) -> ImeState {
        self.state
    }

    /// Get the preedit string (what the user is typing).
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// Get the preedit cursor position.
    pub fn preedit_cursor(&self) -> usize {
        self.preedit_cursor
    }

    /// Get the candidate list.
    pub fn candidates(&self) -> &[ImeCandidate] {
        &self.candidates
    }

    /// Get the selected candidate index.
    pub fn selected_candidate(&self) -> usize {
        self.selected_candidate
    }

    /// Get and clear the committed text.
    pub fn take_committed(&mut self) -> String {
        let result = core::mem::take(&mut self.committed);
        if self.state == ImeState::Committed {
            self.state = ImeState::Inactive;
        }
        result
    }

    /// Feed a character into the IME.
    pub fn feed_char(&mut self, ch: char) {
        if !self.enabled {
            self.committed.push(ch);
            self.state = ImeState::Committed;
            return;
        }

        if ch.is_ascii_alphabetic() {
            self.preedit.push(ch.to_ascii_lowercase());
            self.preedit_cursor = self.preedit.len();
            self.state = ImeState::Composing;
            self.update_candidates();
        } else if ch.is_ascii_digit() && self.state == ImeState::Composing {
            // Select candidate by number.
            let idx = (ch as u8 - b'1') as usize;
            self.select_candidate(idx);
        } else if ch == ' ' && self.state == ImeState::Composing {
            // Commit first candidate.
            self.select_candidate(0);
        } else {
            // Non-alphabetic input while not composing: pass through.
            if self.state == ImeState::Composing {
                self.commit_preedit();
            }
            self.committed.push(ch);
            self.state = ImeState::Committed;
        }
    }

    /// Feed a backspace into the IME.
    pub fn feed_backspace(&mut self) {
        if self.state == ImeState::Composing && !self.preedit.is_empty() {
            self.preedit.pop();
            self.preedit_cursor = self.preedit.len();
            if self.preedit.is_empty() {
                self.state = ImeState::Inactive;
                self.candidates.clear();
            } else {
                self.update_candidates();
            }
        }
    }

    /// Feed an Enter key: commit preedit as-is.
    pub fn feed_enter(&mut self) {
        if self.state == ImeState::Composing {
            self.commit_preedit();
        }
    }

    /// Feed Escape: cancel composition.
    pub fn feed_escape(&mut self) {
        self.reset();
    }

    /// Move candidate selection up.
    pub fn candidate_prev(&mut self) {
        if !self.candidates.is_empty() && self.selected_candidate > 0 {
            self.selected_candidate -= 1;
        }
    }

    /// Move candidate selection down.
    pub fn candidate_next(&mut self) {
        if !self.candidates.is_empty() && self.selected_candidate + 1 < self.candidates.len() {
            self.selected_candidate += 1;
        }
    }

    /// Update the candidate list based on current preedit.
    fn update_candidates(&mut self) {
        self.candidates.clear();
        self.selected_candidate = 0;

        if let Some(chars) = self.pinyin_table.get(&self.preedit) {
            for (i, text) in chars.iter().enumerate() {
                self.candidates.push(ImeCandidate {
                    label: String::from(match i {
                        0 => "1",
                        1 => "2",
                        2 => "3",
                        3 => "4",
                        4 => "5",
                        5 => "6",
                        6 => "7",
                        7 => "8",
                        8 => "9",
                        _ => "?",
                    }),
                    text: text.clone(),
                });
            }
        }
    }

    /// Select and commit a candidate by index.
    fn select_candidate(&mut self, idx: usize) {
        if idx < self.candidates.len() {
            self.committed = self.candidates[idx].text.clone();
        } else if !self.preedit.is_empty() {
            // No matching candidate: commit preedit as-is.
            self.committed = core::mem::take(&mut self.preedit);
        }
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.state = ImeState::Committed;
    }

    /// Commit the raw preedit string.
    fn commit_preedit(&mut self) {
        self.committed = core::mem::take(&mut self.preedit);
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.state = ImeState::Committed;
    }

    /// Reset the IME to inactive state.
    pub fn reset(&mut self) {
        self.preedit.clear();
        self.preedit_cursor = 0;
        self.candidates.clear();
        self.selected_candidate = 0;
        self.committed.clear();
        self.state = ImeState::Inactive;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- Clipboard Tests ---

    #[test]
    fn test_clipboard_copy_paste() {
        let mut mgr = ClipboardManager::new();
        let data = vec![0x48, 0x65, 0x6C, 0x6C, 0x6F]; // "Hello"
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextPlain,
            data.clone(),
        )
        .unwrap();
        let result = mgr
            .paste(SelectionType::Clipboard, ClipboardMime::TextPlain)
            .unwrap();
        assert_eq!(result, &data[..]);
    }

    #[test]
    fn test_clipboard_paste_empty() {
        let mgr = ClipboardManager::new();
        assert_eq!(
            mgr.paste(SelectionType::Clipboard, ClipboardMime::TextPlain),
            Err(ClipboardError::Empty)
        );
    }

    #[test]
    fn test_clipboard_paste_wrong_mime() {
        let mut mgr = ClipboardManager::new();
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextPlain,
            vec![1, 2, 3],
        )
        .unwrap();
        assert_eq!(
            mgr.paste(SelectionType::Clipboard, ClipboardMime::ImagePng),
            Err(ClipboardError::MimeNotFound)
        );
    }

    #[test]
    fn test_clipboard_primary_selection() {
        let mut mgr = ClipboardManager::new();
        mgr.copy(SelectionType::Primary, 1, ClipboardMime::TextPlain, vec![1])
            .unwrap();
        assert!(mgr.has_data(SelectionType::Primary));
        assert!(!mgr.has_data(SelectionType::Clipboard));
    }

    #[test]
    fn test_clipboard_history() {
        let mut mgr = ClipboardManager::new();
        for i in 0..10u8 {
            mgr.copy(
                SelectionType::Clipboard,
                1,
                ClipboardMime::TextPlain,
                vec![i],
            )
            .unwrap();
        }
        // History should have at most CLIPBOARD_HISTORY_MAX entries.
        assert!(mgr.history().len() <= CLIPBOARD_HISTORY_MAX);
    }

    #[test]
    fn test_clipboard_data_too_large() {
        let mut mgr = ClipboardManager::new();
        let big = vec![0u8; CLIPBOARD_MAX_DATA_SIZE + 1];
        assert_eq!(
            mgr.copy(SelectionType::Clipboard, 1, ClipboardMime::TextPlain, big),
            Err(ClipboardError::DataTooLarge)
        );
    }

    #[test]
    fn test_clipboard_negotiate_mime() {
        let mut mgr = ClipboardManager::new();
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextHtml,
            vec![1],
        )
        .unwrap();
        let result = mgr.negotiate_mime(
            SelectionType::Clipboard,
            &[ClipboardMime::TextPlain, ClipboardMime::TextHtml],
        );
        assert_eq!(result, Some(ClipboardMime::TextHtml));
    }

    #[test]
    fn test_clipboard_clear() {
        let mut mgr = ClipboardManager::new();
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextPlain,
            vec![1],
        )
        .unwrap();
        mgr.clear(SelectionType::Clipboard);
        assert!(!mgr.has_data(SelectionType::Clipboard));
    }

    #[test]
    fn test_clipboard_restore_history() {
        let mut mgr = ClipboardManager::new();
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextPlain,
            vec![1],
        )
        .unwrap();
        mgr.copy(
            SelectionType::Clipboard,
            1,
            ClipboardMime::TextPlain,
            vec![2],
        )
        .unwrap();
        // History has the first entry (vec![1]).
        mgr.restore_from_history(0).unwrap();
        let result = mgr
            .paste(SelectionType::Clipboard, ClipboardMime::TextPlain)
            .unwrap();
        assert_eq!(result, &[1]);
    }

    // --- Drag-and-Drop Tests ---

    #[test]
    fn test_dnd_start_drag() {
        let mut dnd = DndManager::new();
        dnd.start_drag(1, vec![ClipboardMime::TextPlain], 10, 20, 32, 32)
            .unwrap();
        assert_eq!(dnd.state(), DndState::Dragging);
    }

    #[test]
    fn test_dnd_double_drag_error() {
        let mut dnd = DndManager::new();
        dnd.start_drag(1, vec![ClipboardMime::TextPlain], 0, 0, 32, 32)
            .unwrap();
        assert_eq!(
            dnd.start_drag(2, vec![], 0, 0, 32, 32),
            Err(DndError::AlreadyDragging)
        );
    }

    #[test]
    fn test_dnd_motion_no_drag() {
        let mut dnd = DndManager::new();
        assert_eq!(dnd.motion(10, 10), Err(DndError::NotDragging));
    }

    #[test]
    fn test_dnd_enter_leave_events() {
        let mut dnd = DndManager::new();
        dnd.register_target(DropTarget {
            surface_id: 42,
            accepted_mimes: vec![ClipboardMime::TextPlain],
            x: 100,
            y: 100,
            width: 200,
            height: 200,
        });
        dnd.start_drag(1, vec![ClipboardMime::TextPlain], 0, 0, 32, 32)
            .unwrap();
        dnd.drain_events(); // Clear start events.

        // Move into target.
        dnd.motion(150, 150).unwrap();
        let events = dnd.drain_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, DndEvent::Enter { surface_id: 42, .. })));

        // Move out of target.
        dnd.motion(0, 0).unwrap();
        let events = dnd.drain_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, DndEvent::Leave { surface_id: 42 })));
    }

    #[test]
    fn test_dnd_drop_action() {
        let mut dnd = DndManager::new();
        dnd.register_target(DropTarget {
            surface_id: 5,
            accepted_mimes: vec![ClipboardMime::TextPlain],
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        dnd.start_drag(1, vec![ClipboardMime::TextPlain], 50, 50, 16, 16)
            .unwrap();
        dnd.motion(50, 50).unwrap();
        let result = dnd.drop_action();
        assert!(result.is_ok());
    }

    #[test]
    fn test_dnd_cancel() {
        let mut dnd = DndManager::new();
        dnd.start_drag(1, vec![], 0, 0, 10, 10).unwrap();
        dnd.cancel();
        assert_eq!(dnd.state(), DndState::Idle);
    }

    #[test]
    fn test_drop_target_contains() {
        let t = DropTarget {
            surface_id: 1,
            accepted_mimes: vec![],
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert!(t.contains(10, 20));
        assert!(t.contains(109, 69));
        assert!(!t.contains(110, 20));
        assert!(!t.contains(5, 20));
    }

    // --- Shortcut Tests ---

    #[test]
    fn test_shortcut_manager_defaults() {
        let mgr = ShortcutManager::new();
        // Should have default bindings registered.
        assert!(mgr.binding_count() > 0);
    }

    #[test]
    fn test_shortcut_process_key() {
        let mgr = ShortcutManager::new();
        // Alt+Tab should match SwitchNextWindow.
        let result = mgr.process_key(ModifierMask::ALT, 0x0F);
        assert_eq!(result, Some(ShortcutAction::SwitchNextWindow));
    }

    #[test]
    fn test_shortcut_no_match() {
        let mgr = ShortcutManager::new();
        let result = mgr.process_key(ModifierMask::NONE, 0x99);
        assert_eq!(result, None);
    }

    #[test]
    fn test_shortcut_register_unregister() {
        let mut mgr = ShortcutManager::new();
        let count = mgr.binding_count();
        let id = mgr.register(KeyBinding::new(
            ModifierMask::CTRL,
            0x1E,
            ShortcutAction::Custom(42),
        ));
        assert_eq!(mgr.binding_count(), count + 1);
        mgr.unregister(id);
        assert_eq!(mgr.binding_count(), count);
    }

    #[test]
    fn test_shortcut_disabled() {
        let mut mgr = ShortcutManager::new();
        mgr.set_enabled(false);
        let result = mgr.process_key(ModifierMask::ALT, 0x0F);
        assert_eq!(result, None);
    }

    #[test]
    fn test_modifier_mask_combine() {
        let m = ModifierMask::CTRL.combine(ModifierMask::ALT);
        assert!(m.has(ModifierMask::CTRL));
        assert!(m.has(ModifierMask::ALT));
        assert!(!m.has(ModifierMask::SHIFT));
    }

    // --- Theme Tests ---

    #[test]
    fn test_theme_default_dark() {
        let mgr = ThemeManager::new();
        assert_eq!(mgr.current_preset(), ThemePreset::Dark);
    }

    #[test]
    fn test_theme_switch() {
        let mut mgr = ThemeManager::new();
        mgr.set_theme(ThemePreset::Nord);
        assert_eq!(mgr.current_preset(), ThemePreset::Nord);
        // Verify a characteristic Nord color.
        assert_eq!(mgr.colors().accent, ThemeColor::from_rgb(0x88, 0xC0, 0xD0));
    }

    #[test]
    fn test_theme_all_presets_load() {
        let mut mgr = ThemeManager::new();
        let presets = [
            ThemePreset::Light,
            ThemePreset::Dark,
            ThemePreset::SolarizedDark,
            ThemePreset::SolarizedLight,
            ThemePreset::Nord,
            ThemePreset::Dracula,
        ];
        for preset in &presets {
            mgr.set_theme(*preset);
            assert_eq!(mgr.current_preset(), *preset);
        }
    }

    #[test]
    fn test_theme_color_components() {
        let c = ThemeColor::from_argb(0x80, 0xFF, 0x00, 0xAA);
        assert_eq!(c.alpha(), 0x80);
        assert_eq!(c.red(), 0xFF);
        assert_eq!(c.green(), 0x00);
        assert_eq!(c.blue(), 0xAA);
    }

    #[test]
    fn test_theme_color_darken() {
        let c = ThemeColor::from_rgb(100, 200, 50);
        let d = c.darken(50);
        assert_eq!(d.red(), 50);
        assert_eq!(d.green(), 100);
        assert_eq!(d.blue(), 25);
    }

    #[test]
    fn test_theme_style_property() {
        let mgr = ThemeManager::new();
        assert_eq!(mgr.map_style_property(StyleProperty::FontSize), 14);
        assert_eq!(mgr.map_style_property(StyleProperty::BorderWidth), 1);
    }

    #[test]
    fn test_theme_gtk_name() {
        let mut mgr = ThemeManager::new();
        mgr.set_theme(ThemePreset::Dracula);
        assert_eq!(mgr.gtk_theme_name(), "Dracula");
    }

    // --- Font Rendering Tests ---

    #[test]
    fn test_ttf_parser_invalid_data() {
        let result = TtfParser::new(&[0, 1, 2, 3]);
        assert!(result.is_err());
    }

    #[test]
    fn test_ttf_parser_empty() {
        let result = TtfParser::new(&[]);
        assert!(matches!(result, Err(FontError::InvalidFont)));
    }

    #[test]
    fn test_read_u16_be() {
        assert_eq!(read_u16_be(&[0x01, 0x02], 0), Some(0x0102));
        assert_eq!(read_u16_be(&[0xFF, 0x00], 0), Some(0xFF00));
        assert_eq!(read_u16_be(&[0x01], 0), None);
    }

    #[test]
    fn test_read_u32_be() {
        assert_eq!(read_u32_be(&[0x00, 0x01, 0x00, 0x00], 0), Some(0x00010000));
        assert_eq!(read_u32_be(&[0x01, 0x02], 0), None);
    }

    #[test]
    fn test_glyph_cache_insert_lookup() {
        let mut cache = GlyphCache::new();
        let bmp = GlyphBitmap {
            data: vec![128; 16],
            width: 4,
            height: 4,
            bearing_x: 0,
            bearing_y: 4,
            advance: 5,
        };
        cache.insert(65, 16, bmp.clone());
        assert_eq!(cache.len(), 1);
        let result = cache.get(65, 16);
        assert!(result.is_some());
        assert_eq!(result.unwrap().width, 4);
    }

    #[test]
    fn test_glyph_cache_miss() {
        let mut cache = GlyphCache::new();
        assert!(cache.get(65, 16).is_none());
    }

    #[test]
    fn test_glyph_cache_hit_rate() {
        let mut cache = GlyphCache::new();
        cache.insert(
            65,
            16,
            GlyphBitmap {
                data: vec![0; 4],
                width: 2,
                height: 2,
                bearing_x: 0,
                bearing_y: 2,
                advance: 3,
            },
        );
        cache.get(65, 16); // hit
        cache.get(66, 16); // miss
        assert_eq!(cache.hit_rate_percent(), 50);
    }

    #[test]
    fn test_glyph_cache_eviction() {
        let mut cache = GlyphCache::new();
        for i in 0..GLYPH_CACHE_SIZE + 10 {
            cache.insert(
                i as u32,
                12,
                GlyphBitmap {
                    data: vec![0; 1],
                    width: 1,
                    height: 1,
                    bearing_x: 0,
                    bearing_y: 1,
                    advance: 1,
                },
            );
        }
        assert!(cache.len() <= GLYPH_CACHE_SIZE);
    }

    #[test]
    fn test_rasterize_empty_outline() {
        let outline = GlyphOutline {
            contours: Vec::new(),
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
            advance_width: 0,
            lsb: 0,
        };
        let bmp = rasterize_outline(&outline, 16, 2048);
        assert_eq!(bmp.width, 0);
        assert_eq!(bmp.height, 0);
    }

    #[test]
    fn test_table_tag_constants() {
        assert_eq!(TableTag::CMAP.0, *b"cmap");
        assert_eq!(TableTag::HEAD.0, *b"head");
        assert_eq!(TableTag::GLYF.0, *b"glyf");
    }

    // --- CJK / Unicode Tests ---

    #[test]
    fn test_is_cjk_wide_basic() {
        assert!(is_cjk_wide('\u{4E00}')); // CJK Unified start
        assert!(is_cjk_wide('\u{9FFF}')); // CJK Unified end
        assert!(is_cjk_wide('\u{AC00}')); // Hangul start
        assert!(is_cjk_wide('\u{3042}')); // Hiragana 'a'
        assert!(is_cjk_wide('\u{30A2}')); // Katakana 'a'
        assert!(is_cjk_wide('\u{FF01}')); // Fullwidth '!'
    }

    #[test]
    fn test_is_cjk_wide_false() {
        assert!(!is_cjk_wide('A'));
        assert!(!is_cjk_wide('z'));
        assert!(!is_cjk_wide(' '));
        assert!(!is_cjk_wide('\u{00E9}')); // e-acute
    }

    #[test]
    fn test_char_width() {
        assert_eq!(char_width('A'), 1);
        assert_eq!(char_width('\u{4E00}'), 2);
        assert_eq!(char_width('\0'), 0);
        assert_eq!(char_width('\u{0300}'), 0); // Combining
        assert_eq!(char_width('\u{200B}'), 0); // Zero-width space
    }

    #[test]
    fn test_string_width() {
        assert_eq!(string_width("Hello"), 5);
        assert_eq!(string_width("\u{4F60}\u{597D}"), 4); // Two CJK chars
        assert_eq!(string_width("A\u{4E00}B"), 4); // Mixed
    }

    #[test]
    fn test_truncate_to_width() {
        let s = "Hello, World!";
        let truncated = truncate_to_width(s, 10);
        assert!(string_width(&truncated) <= 10);
    }

    #[test]
    fn test_cell_content_default() {
        assert_eq!(CellContent::default(), CellContent::Empty);
    }

    // --- IME Tests ---

    #[test]
    fn test_ime_disabled_passthrough() {
        let mut ime = InputMethodEditor::new();
        // IME is disabled by default.
        ime.feed_char('a');
        assert_eq!(ime.state(), ImeState::Committed);
        assert_eq!(ime.take_committed(), "a");
    }

    #[test]
    fn test_ime_composing() {
        let mut ime = InputMethodEditor::new();
        ime.set_enabled(true);
        ime.feed_char('n');
        ime.feed_char('i');
        assert_eq!(ime.state(), ImeState::Composing);
        assert_eq!(ime.preedit(), "ni");
        assert!(!ime.candidates().is_empty());
    }

    #[test]
    fn test_ime_select_candidate() {
        let mut ime = InputMethodEditor::new();
        ime.set_enabled(true);
        ime.feed_char('n');
        ime.feed_char('i');
        ime.feed_char('1'); // Select first candidate.
        assert_eq!(ime.state(), ImeState::Committed);
        let committed = ime.take_committed();
        assert_eq!(committed, "\u{4F60}"); // ni -> U+4F60
    }

    #[test]
    fn test_ime_backspace() {
        let mut ime = InputMethodEditor::new();
        ime.set_enabled(true);
        ime.feed_char('h');
        ime.feed_char('a');
        ime.feed_backspace();
        assert_eq!(ime.preedit(), "h");
        ime.feed_backspace();
        assert_eq!(ime.state(), ImeState::Inactive);
    }

    #[test]
    fn test_ime_escape_cancels() {
        let mut ime = InputMethodEditor::new();
        ime.set_enabled(true);
        ime.feed_char('s');
        ime.feed_char('h');
        ime.feed_char('i');
        ime.feed_escape();
        assert_eq!(ime.state(), ImeState::Inactive);
        assert!(ime.preedit().is_empty());
    }

    #[test]
    fn test_ime_space_commits_first() {
        let mut ime = InputMethodEditor::new();
        ime.set_enabled(true);
        ime.feed_char('w');
        ime.feed_char('o');
        ime.feed_char(' '); // Commit first candidate.
        assert_eq!(ime.state(), ImeState::Committed);
        let committed = ime.take_committed();
        assert_eq!(committed, "\u{6211}"); // wo -> U+6211
    }
}
