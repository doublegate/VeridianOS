//! System Settings Application
//!
//! Provides a settings interface for configuring display, network,
//! users, and appearance preferences.

#![allow(dead_code)]

use alloc::{format, string::String, vec, vec::Vec};

use super::renderer::draw_string_into_buffer;

// ---------------------------------------------------------------------------
// Settings panel categories
// ---------------------------------------------------------------------------

/// Which panel is active in the settings sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPanel {
    Display,
    Network,
    Users,
    Appearance,
    About,
}

impl SettingsPanel {
    /// Label shown in the sidebar for this panel.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::Network => "Network",
            Self::Users => "Users",
            Self::Appearance => "Appearance",
            Self::About => "About",
        }
    }

    /// Ordered list of all panels.
    pub fn all() -> &'static [SettingsPanel] {
        &[
            Self::Display,
            Self::Network,
            Self::Users,
            Self::Appearance,
            Self::About,
        ]
    }

    /// Index of this panel in the ordered list.
    fn index(&self) -> usize {
        match self {
            Self::Display => 0,
            Self::Network => 1,
            Self::Users => 2,
            Self::Appearance => 3,
            Self::About => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-panel settings structs
// ---------------------------------------------------------------------------

/// Display-related settings.
#[derive(Debug, Clone)]
pub struct DisplaySettings {
    pub resolution_index: usize,
    pub brightness: u8,
    pub available_resolutions: Vec<(usize, usize)>,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            resolution_index: 0,
            brightness: 80,
            available_resolutions: vec![(1280, 800), (1024, 768), (800, 600), (1920, 1080)],
        }
    }
}

impl DisplaySettings {
    /// Number of configurable items in this panel.
    fn item_count(&self) -> usize {
        2 // resolution, brightness
    }
}

/// Network-related settings.
#[derive(Debug, Clone)]
pub struct NetworkSettings {
    pub hostname: String,
    pub dhcp_enabled: bool,
    pub ip_address: String,
    pub gateway: String,
    pub dns: String,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            hostname: String::from("veridian"),
            dhcp_enabled: true,
            ip_address: String::from("10.0.2.15"),
            gateway: String::from("10.0.2.2"),
            dns: String::from("10.0.2.3"),
        }
    }
}

impl NetworkSettings {
    fn item_count(&self) -> usize {
        5 // hostname, dhcp, ip, gateway, dns
    }
}

/// User account settings.
#[derive(Debug, Clone)]
pub struct UserSettings {
    pub username: String,
    pub shell: String,
    pub home_dir: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            username: String::from("root"),
            shell: String::from("/bin/vsh"),
            home_dir: String::from("/root"),
        }
    }
}

impl UserSettings {
    fn item_count(&self) -> usize {
        3 // username, shell, home_dir
    }
}

/// Panel position on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPosition {
    Top,
    Bottom,
}

/// Appearance settings.
#[derive(Debug, Clone)]
pub struct AppearanceSettings {
    pub theme_index: usize,
    pub font_size: u8,
    pub show_desktop_icons: bool,
    pub panel_position: PanelPosition,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme_index: 0,
            font_size: 16,
            show_desktop_icons: true,
            panel_position: PanelPosition::Bottom,
        }
    }
}

impl AppearanceSettings {
    fn item_count(&self) -> usize {
        4 // theme, font_size, show_icons, panel_position
    }
}

/// Static system information displayed on the About panel.
#[derive(Debug, Clone)]
pub struct AboutInfo {
    pub os_name: &'static str,
    pub version: &'static str,
    pub kernel_version: &'static str,
    pub arch: &'static str,
    pub hostname: String,
}

impl Default for AboutInfo {
    fn default() -> Self {
        Self {
            os_name: "VeridianOS",
            version: "0.10.0",
            kernel_version: "0.10.0-phase7",
            arch: core::env!("CARGO_PKG_NAME"), // will be "veridian-kernel"
            hostname: String::from("veridian"),
        }
    }
}

// ---------------------------------------------------------------------------
// Actions returned by input handlers
// ---------------------------------------------------------------------------

/// Action produced by settings interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    /// No-op.
    None,
    /// Close the settings window.
    Close,
    /// Apply current settings.
    Apply,
    /// Switch to a different panel.
    SwitchPanel(SettingsPanel),
}

// ---------------------------------------------------------------------------
// Main application struct
// ---------------------------------------------------------------------------

/// System Settings application state.
pub struct SettingsApp {
    /// Currently active panel.
    pub active_panel: SettingsPanel,

    /// Per-panel state.
    pub display: DisplaySettings,
    pub network: NetworkSettings,
    pub user: UserSettings,
    pub appearance: AppearanceSettings,
    pub about: AboutInfo,

    /// Index of the selected item within the active panel's content area.
    pub selected_item: usize,

    /// Compositor surface ID (set when wired to the desktop).
    pub surface_id: Option<u32>,

    /// Window dimensions.
    pub width: usize,
    pub height: usize,
}

// Layout constants
const SIDEBAR_WIDTH: usize = 140;
const SIDEBAR_ITEM_HEIGHT: usize = 24;
const CONTENT_X: usize = SIDEBAR_WIDTH + 12;
const CONTENT_Y: usize = 12;
const LINE_HEIGHT: usize = 22;
const CHAR_W: usize = 8;

impl SettingsApp {
    /// Create a new settings application with default values.
    pub fn new() -> Self {
        Self {
            active_panel: SettingsPanel::Display,
            display: DisplaySettings::default(),
            network: NetworkSettings::default(),
            user: UserSettings::default(),
            appearance: AppearanceSettings::default(),
            about: AboutInfo::default(),
            selected_item: 0,
            surface_id: None,
            width: 600,
            height: 400,
        }
    }

    /// Switch the active panel, resetting the item selection.
    pub fn switch_panel(&mut self, panel: SettingsPanel) {
        self.active_panel = panel;
        self.selected_item = 0;
    }

    /// Number of selectable items in the current panel.
    fn current_item_count(&self) -> usize {
        match self.active_panel {
            SettingsPanel::Display => self.display.item_count(),
            SettingsPanel::Network => self.network.item_count(),
            SettingsPanel::Users => self.user.item_count(),
            SettingsPanel::Appearance => self.appearance.item_count(),
            SettingsPanel::About => 0, // read-only
        }
    }

    /// Handle a keyboard event and return the resulting action.
    pub fn handle_key(&mut self, key: u8) -> SettingsAction {
        match key {
            // Tab -- cycle to next panel
            b'\t' => {
                let panels = SettingsPanel::all();
                let next = (self.active_panel.index() + 1) % panels.len();
                let panel = panels[next];
                self.switch_panel(panel);
                SettingsAction::SwitchPanel(panel)
            }
            // Up arrow (scancode translated to 'A' by ANSI CSI in our shell, or raw 72)
            b'k' | b'K' => {
                if self.selected_item > 0 {
                    self.selected_item -= 1;
                }
                SettingsAction::None
            }
            // Down arrow
            b'j' | b'J' => {
                let max = self.current_item_count();
                if max > 0 && self.selected_item < max - 1 {
                    self.selected_item += 1;
                }
                SettingsAction::None
            }
            // Enter -- toggle / activate selected item
            b'\r' | b'\n' => {
                self.activate_selected();
                SettingsAction::Apply
            }
            // Escape -- close
            0x1B => SettingsAction::Close,
            _ => SettingsAction::None,
        }
    }

    /// Handle a mouse click and return the resulting action.
    pub fn handle_click(&mut self, x: usize, y: usize) -> SettingsAction {
        // Sidebar click?
        if x < SIDEBAR_WIDTH {
            let panels = SettingsPanel::all();
            let index = y / SIDEBAR_ITEM_HEIGHT;
            if index < panels.len() {
                let panel = panels[index];
                self.switch_panel(panel);
                return SettingsAction::SwitchPanel(panel);
            }
            return SettingsAction::None;
        }

        // Content area click
        if x >= CONTENT_X && y >= CONTENT_Y {
            let item = (y - CONTENT_Y) / LINE_HEIGHT;
            let max = self.current_item_count();
            if item < max {
                self.selected_item = item;
                self.activate_selected();
                return SettingsAction::Apply;
            }
        }

        SettingsAction::None
    }

    /// Toggle or cycle the currently selected item.
    fn activate_selected(&mut self) {
        match self.active_panel {
            SettingsPanel::Display => match self.selected_item {
                0 => {
                    // Cycle resolution
                    let n = self.display.available_resolutions.len();
                    if n > 0 {
                        self.display.resolution_index = (self.display.resolution_index + 1) % n;
                    }
                }
                1 => {
                    // Increase brightness by 10, wrap at 100
                    self.display.brightness = if self.display.brightness >= 100 {
                        10
                    } else {
                        self.display.brightness + 10
                    };
                }
                _ => {}
            },
            SettingsPanel::Network => {
                if self.selected_item == 1 {
                    // Toggle DHCP
                    self.network.dhcp_enabled = !self.network.dhcp_enabled;
                }
                // Other items would open an edit dialog (future).
            }
            SettingsPanel::Users => {
                // Read-only for now.
            }
            SettingsPanel::Appearance => match self.selected_item {
                0 => {
                    // Cycle theme (0=dark, 1=light)
                    self.appearance.theme_index = (self.appearance.theme_index + 1) % 2;
                }
                1 => {
                    // Cycle font size 12..20
                    self.appearance.font_size = if self.appearance.font_size >= 20 {
                        12
                    } else {
                        self.appearance.font_size + 2
                    };
                }
                2 => {
                    self.appearance.show_desktop_icons = !self.appearance.show_desktop_icons;
                }
                3 => {
                    self.appearance.panel_position = match self.appearance.panel_position {
                        PanelPosition::Top => PanelPosition::Bottom,
                        PanelPosition::Bottom => PanelPosition::Top,
                    };
                }
                _ => {}
            },
            SettingsPanel::About => {} // read-only
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    /// Render the settings window into a `u32` BGRA pixel buffer.
    ///
    /// `buffer` must be at least `buf_width * buf_height` elements.
    pub fn render_to_buffer(&self, buffer: &mut [u32], buf_width: usize, buf_height: usize) {
        // We render via an intermediate byte buffer (BGRA) since that is what
        // draw_char_into_buffer / draw_string_into_buffer expect.
        let byte_len = buf_width * buf_height * 4;
        let mut byte_buf = vec![0u8; byte_len];

        // -- background (dark gray) --
        for chunk in byte_buf.chunks_exact_mut(4) {
            chunk[0] = 0x28; // B
            chunk[1] = 0x28; // G
            chunk[2] = 0x28; // R
            chunk[3] = 0xFF; // A
        }

        // -- sidebar background (slightly lighter) --
        for y in 0..buf_height {
            for x in 0..SIDEBAR_WIDTH.min(buf_width) {
                let off = (y * buf_width + x) * 4;
                if off + 3 < byte_buf.len() {
                    byte_buf[off] = 0x32;
                    byte_buf[off + 1] = 0x32;
                    byte_buf[off + 2] = 0x32;
                    byte_buf[off + 3] = 0xFF;
                }
            }
        }

        // -- sidebar panel items --
        let panels = SettingsPanel::all();
        for (i, panel) in panels.iter().enumerate() {
            let item_y = i * SIDEBAR_ITEM_HEIGHT;

            // Highlight active panel
            if *panel == self.active_panel {
                for dy in 0..SIDEBAR_ITEM_HEIGHT {
                    for x in 0..SIDEBAR_WIDTH.min(buf_width) {
                        let off = ((item_y + dy) * buf_width + x) * 4;
                        if off + 3 < byte_buf.len() {
                            byte_buf[off] = 0x55;
                            byte_buf[off + 1] = 0x44;
                            byte_buf[off + 2] = 0x33;
                            byte_buf[off + 3] = 0xFF;
                        }
                    }
                }
            }

            let color = if *panel == self.active_panel {
                0xFFFFFF
            } else {
                0xAAAAAA
            };
            draw_string_into_buffer(
                &mut byte_buf,
                buf_width,
                panel.label().as_bytes(),
                10,
                item_y + 4,
                color,
            );
        }

        // -- sidebar / content divider --
        for y in 0..buf_height {
            let off = (y * buf_width + SIDEBAR_WIDTH) * 4;
            if off + 3 < byte_buf.len() {
                byte_buf[off] = 0x55;
                byte_buf[off + 1] = 0x55;
                byte_buf[off + 2] = 0x55;
                byte_buf[off + 3] = 0xFF;
            }
        }

        // -- content area --
        match self.active_panel {
            SettingsPanel::Display => {
                self.render_display_panel(&mut byte_buf, buf_width);
            }
            SettingsPanel::Network => {
                self.render_network_panel(&mut byte_buf, buf_width);
            }
            SettingsPanel::Users => {
                self.render_users_panel(&mut byte_buf, buf_width);
            }
            SettingsPanel::Appearance => {
                self.render_appearance_panel(&mut byte_buf, buf_width);
            }
            SettingsPanel::About => {
                self.render_about_panel(&mut byte_buf, buf_width);
            }
        }

        // Convert byte buffer (BGRA u8) into u32 buffer
        for (i, chunk) in byte_buf.chunks_exact(4).enumerate() {
            if i < buffer.len() {
                buffer[i] = (chunk[3] as u32) << 24
                    | (chunk[2] as u32) << 16
                    | (chunk[1] as u32) << 8
                    | (chunk[0] as u32);
            }
        }
    }

    // -- per-panel rendering helpers ----------------------------------------

    fn render_label_value(
        buf: &mut [u8],
        buf_width: usize,
        row: usize,
        label: &[u8],
        value: &[u8],
        selected: bool,
    ) {
        let y = CONTENT_Y + row * LINE_HEIGHT;

        // Selection highlight
        if selected {
            for dy in 0..LINE_HEIGHT {
                for x in CONTENT_X..buf_width {
                    let off = ((y + dy) * buf_width + x) * 4;
                    if off + 3 < buf.len() {
                        buf[off] = 0x44;
                        buf[off + 1] = 0x3A;
                        buf[off + 2] = 0x30;
                        buf[off + 3] = 0xFF;
                    }
                }
            }
        }

        let label_color: u32 = 0x999999;
        let value_color: u32 = if selected { 0xFFCC66 } else { 0xDDDDDD };

        draw_string_into_buffer(buf, buf_width, label, CONTENT_X, y + 3, label_color);
        let val_x = CONTENT_X + (label.len() + 1) * CHAR_W;
        draw_string_into_buffer(buf, buf_width, value, val_x, y + 3, value_color);
    }

    fn render_display_panel(&self, buf: &mut [u8], buf_width: usize) {
        let res = if self.display.resolution_index < self.display.available_resolutions.len() {
            let (w, h) = self.display.available_resolutions[self.display.resolution_index];
            format!("{}x{}", w, h)
        } else {
            String::from("unknown")
        };

        Self::render_label_value(
            buf,
            buf_width,
            0,
            b"Resolution:",
            res.as_bytes(),
            self.selected_item == 0,
        );

        let bright = format!("{}%", self.display.brightness);
        Self::render_label_value(
            buf,
            buf_width,
            1,
            b"Brightness:",
            bright.as_bytes(),
            self.selected_item == 1,
        );
    }

    fn render_network_panel(&self, buf: &mut [u8], buf_width: usize) {
        Self::render_label_value(
            buf,
            buf_width,
            0,
            b"Hostname:",
            self.network.hostname.as_bytes(),
            self.selected_item == 0,
        );

        let dhcp_str = if self.network.dhcp_enabled {
            "ON"
        } else {
            "OFF"
        };
        Self::render_label_value(
            buf,
            buf_width,
            1,
            b"DHCP:",
            dhcp_str.as_bytes(),
            self.selected_item == 1,
        );

        Self::render_label_value(
            buf,
            buf_width,
            2,
            b"IP Address:",
            self.network.ip_address.as_bytes(),
            self.selected_item == 2,
        );

        Self::render_label_value(
            buf,
            buf_width,
            3,
            b"Gateway:",
            self.network.gateway.as_bytes(),
            self.selected_item == 3,
        );

        Self::render_label_value(
            buf,
            buf_width,
            4,
            b"DNS:",
            self.network.dns.as_bytes(),
            self.selected_item == 4,
        );
    }

    fn render_users_panel(&self, buf: &mut [u8], buf_width: usize) {
        Self::render_label_value(
            buf,
            buf_width,
            0,
            b"Username:",
            self.user.username.as_bytes(),
            self.selected_item == 0,
        );

        Self::render_label_value(
            buf,
            buf_width,
            1,
            b"Shell:",
            self.user.shell.as_bytes(),
            self.selected_item == 1,
        );

        Self::render_label_value(
            buf,
            buf_width,
            2,
            b"Home:",
            self.user.home_dir.as_bytes(),
            self.selected_item == 2,
        );
    }

    fn render_appearance_panel(&self, buf: &mut [u8], buf_width: usize) {
        let theme = match self.appearance.theme_index {
            0 => "Dark",
            1 => "Light",
            _ => "Custom",
        };
        Self::render_label_value(
            buf,
            buf_width,
            0,
            b"Theme:",
            theme.as_bytes(),
            self.selected_item == 0,
        );

        let fsz = format!("{}px", self.appearance.font_size);
        Self::render_label_value(
            buf,
            buf_width,
            1,
            b"Font Size:",
            fsz.as_bytes(),
            self.selected_item == 1,
        );

        let icons = if self.appearance.show_desktop_icons {
            "ON"
        } else {
            "OFF"
        };
        Self::render_label_value(
            buf,
            buf_width,
            2,
            b"Desktop Icons:",
            icons.as_bytes(),
            self.selected_item == 2,
        );

        let pos = match self.appearance.panel_position {
            PanelPosition::Top => "Top",
            PanelPosition::Bottom => "Bottom",
        };
        Self::render_label_value(
            buf,
            buf_width,
            3,
            b"Panel Position:",
            pos.as_bytes(),
            self.selected_item == 3,
        );
    }

    fn render_about_panel(&self, buf: &mut [u8], buf_width: usize) {
        let rows: [(&[u8], &[u8]); 5] = [
            (b"OS:", self.about.os_name.as_bytes()),
            (b"Version:", self.about.version.as_bytes()),
            (b"Kernel:", self.about.kernel_version.as_bytes()),
            (b"Package:", self.about.arch.as_bytes()),
            (b"Hostname:", self.about.hostname.as_bytes()),
        ];

        for (i, (label, value)) in rows.iter().enumerate() {
            // About panel is read-only; never highlight.
            Self::render_label_value(buf, buf_width, i, label, value, false);
        }
    }
}

impl Default for SettingsApp {
    fn default() -> Self {
        Self::new()
    }
}
