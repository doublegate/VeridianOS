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

pub mod cjk;
pub mod clipboard;
pub mod dnd;
pub mod font_render;
pub mod shortcuts;
pub mod theme;

// Re-export all public types and functions for API compatibility.

// Clipboard
// CJK
pub use cjk::{char_width, is_cjk_wide, CellContent, ImeState};
#[cfg(feature = "alloc")]
pub use cjk::{string_width, truncate_to_width, ImeCandidate, InputMethodEditor};
#[cfg(feature = "alloc")]
pub use clipboard::ClipboardManager;
pub use clipboard::{ClipboardEntry, ClipboardError, ClipboardMime, SelectionType};
// Drag-and-Drop
pub use dnd::{DndError, DndEvent, DndState};
#[cfg(feature = "alloc")]
pub use dnd::{DndManager, DragSource, DropTarget};
#[cfg(feature = "alloc")]
pub use font_render::{
    apply_hinting, rasterize_outline, render_glyph, GlyphBitmap, GlyphCache, GlyphContour,
    GlyphOutline,
};
// Font Rendering
pub use font_render::{
    FontError, HeadTable, HheaTable, HintingMode, MaxpTable, OutlinePoint, SubpixelMode,
    TableEntry, TableTag, TtfParser,
};
#[cfg(feature = "alloc")]
pub use shortcuts::ShortcutManager;
// Shortcuts
pub use shortcuts::{KeyBinding, KeyCode, ModifierMask, ShortcutAction, ShortcutPriority};
// Theme
pub use theme::{IconTheme, StyleProperty, ThemeColor, ThemeColors, ThemeManager, ThemePreset};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::{string::String, vec, vec::Vec};

    use super::{
        clipboard::{CLIPBOARD_HISTORY_MAX, CLIPBOARD_MAX_DATA_SIZE},
        font_render::{read_u16_be, read_u32_be, GLYPH_CACHE_SIZE},
        *,
    };

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
