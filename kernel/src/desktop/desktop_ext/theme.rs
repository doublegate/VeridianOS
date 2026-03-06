//! Theme Engine
//!
//! Color schemes (light/dark/solarized/nord/dracula) with runtime switching.

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

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
