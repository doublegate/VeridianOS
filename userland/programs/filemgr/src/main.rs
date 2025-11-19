//! VeridianOS File Manager
//!
//! A graphical file manager for browsing and managing files.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::format;
use core::panic::PanicInfo;

/// File type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    CharDevice,
    BlockDevice,
    Pipe,
    Socket,
}

/// File entry
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
    pub permissions: u32,
    pub modified: u64,
}

impl FileEntry {
    pub fn new(name: String, file_type: FileType) -> Self {
        Self {
            name,
            file_type,
            size: 0,
            permissions: 0o644,
            modified: 0,
        }
    }

    /// Get icon character for file type
    pub fn icon(&self) -> char {
        match self.file_type {
            FileType::Directory => '📁',
            FileType::File => '📄',
            FileType::Symlink => '🔗',
            FileType::CharDevice => '🔌',
            FileType::BlockDevice => '💾',
            FileType::Pipe => '🔧',
            FileType::Socket => '🔌',
        }
    }
}

/// View mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
    Details,
}

/// Sort order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Name,
    Size,
    Modified,
    Type,
}

/// File manager state
pub struct FileManager {
    current_path: String,
    entries: Vec<FileEntry>,
    selected_index: Option<usize>,
    scroll_offset: usize,
    view_mode: ViewMode,
    sort_order: SortOrder,
    show_hidden: bool,

    // UI dimensions
    width: usize,
    height: usize,

    // Color scheme (Nord theme)
    bg_color: u32,
    fg_color: u32,
    selection_color: u32,
    directory_color: u32,
    file_color: u32,
}

impl FileManager {
    /// Create a new file manager
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            current_path: String::from("/"),
            entries: Vec::new(),
            selected_index: None,
            scroll_offset: 0,
            view_mode: ViewMode::List,
            sort_order: SortOrder::Name,
            show_hidden: false,
            width,
            height,
            bg_color: 0xFF2E3440,
            fg_color: 0xFFECEFF4,
            selection_color: 0xFF5E81AC,
            directory_color: 0xFF88C0D0,
            file_color: 0xFFD8DEE9,
        }
    }

    /// Change directory
    pub fn change_directory(&mut self, path: &str) {
        self.current_path = path.to_string();
        self.selected_index = None;
        self.scroll_offset = 0;
        self.load_directory();
    }

    /// Load current directory entries
    fn load_directory(&mut self) {
        // In a real implementation, this would use VFS syscalls
        // For now, create some example entries
        self.entries.clear();

        // Parent directory
        if self.current_path != "/" {
            self.entries.push(FileEntry::new("..".to_string(), FileType::Directory));
        }

        // Example entries
        self.entries.push(FileEntry::new("bin".to_string(), FileType::Directory));
        self.entries.push(FileEntry::new("etc".to_string(), FileType::Directory));
        self.entries.push(FileEntry::new("home".to_string(), FileType::Directory));
        self.entries.push(FileEntry::new("usr".to_string(), FileType::Directory));
        self.entries.push(FileEntry::new("var".to_string(), FileType::Directory));

        let mut readme = FileEntry::new("README.txt".to_string(), FileType::File);
        readme.size = 1024;
        self.entries.push(readme);

        let mut config = FileEntry::new("config.toml".to_string(), FileType::File);
        config.size = 512;
        self.entries.push(config);

        // Sort entries
        self.sort_entries();
    }

    /// Sort entries by current sort order
    fn sort_entries(&mut self) {
        match self.sort_order {
            SortOrder::Name => {
                self.entries.sort_by(|a, b| a.name.cmp(&b.name));
            }
            SortOrder::Size => {
                self.entries.sort_by(|a, b| b.size.cmp(&a.size));
            }
            SortOrder::Modified => {
                self.entries.sort_by(|a, b| b.modified.cmp(&a.modified));
            }
            SortOrder::Type => {
                self.entries.sort_by(|a, b| {
                    (a.file_type as u8).cmp(&(b.file_type as u8))
                });
            }
        }

        // Directories first
        self.entries.sort_by(|a, b| {
            match (a.file_type, b.file_type) {
                (FileType::Directory, FileType::Directory) => core::cmp::Ordering::Equal,
                (FileType::Directory, _) => core::cmp::Ordering::Less,
                (_, FileType::Directory) => core::cmp::Ordering::Greater,
                _ => core::cmp::Ordering::Equal,
            }
        });
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        match self.selected_index {
            None => {
                self.selected_index = Some(0);
            }
            Some(0) => {
                // Wrap to end
                self.selected_index = Some(self.entries.len() - 1);
            }
            Some(i) => {
                self.selected_index = Some(i - 1);
            }
        }

        self.ensure_visible();
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        match self.selected_index {
            None => {
                self.selected_index = Some(0);
            }
            Some(i) if i >= self.entries.len() - 1 => {
                // Wrap to start
                self.selected_index = Some(0);
            }
            Some(i) => {
                self.selected_index = Some(i + 1);
            }
        }

        self.ensure_visible();
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        if let Some(selected) = self.selected_index {
            let visible_count = (self.height - 50) / 20; // Approximate visible items

            if selected < self.scroll_offset {
                self.scroll_offset = selected;
            } else if selected >= self.scroll_offset + visible_count {
                self.scroll_offset = selected - visible_count + 1;
            }
        }
    }

    /// Open selected item
    pub fn open_selected(&mut self) {
        if let Some(index) = self.selected_index {
            if let Some(entry) = self.entries.get(index) {
                match entry.file_type {
                    FileType::Directory => {
                        // Navigate to directory
                        if entry.name == ".." {
                            // Go to parent
                            if let Some(pos) = self.current_path.rfind('/') {
                                if pos == 0 {
                                    self.change_directory("/");
                                } else {
                                    self.change_directory(&self.current_path[..pos]);
                                }
                            }
                        } else {
                            // Go to subdirectory
                            let new_path = if self.current_path == "/" {
                                format!("/{}", entry.name)
                            } else {
                                format!("{}/{}", self.current_path, entry.name)
                            };
                            self.change_directory(&new_path);
                        }
                    }
                    FileType::File => {
                        // Open file (would launch associated application)
                        // TODO: Implement file opening
                    }
                    _ => {
                        // Other file types
                    }
                }
            }
        }
    }

    /// Render file manager to framebuffer
    pub fn render(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        // Clear background
        for pixel in fb.iter_mut() {
            *pixel = self.bg_color;
        }

        // Draw title bar
        self.draw_title_bar(fb, fb_width);

        // Draw toolbar
        self.draw_toolbar(fb, fb_width);

        // Draw file list
        self.draw_file_list(fb, fb_width, fb_height);

        // Draw status bar
        self.draw_status_bar(fb, fb_width, fb_height);
    }

    /// Draw title bar
    fn draw_title_bar(&self, fb: &mut [u32], fb_width: usize) {
        // Draw title bar background (30px high)
        for y in 0..30 {
            for x in 0..fb_width {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = 0xFF3B4252; // Darker background
                }
            }
        }

        // Title text would be rendered here (requires font rendering)
        // For now, just draw a colored rectangle to indicate the title
        for y in 8..22 {
            for x in 10..150 {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = self.fg_color;
                }
            }
        }
    }

    /// Draw toolbar
    fn draw_toolbar(&self, fb: &mut [u32], fb_width: usize) {
        // Draw toolbar background (40px high, starting at y=30)
        for y in 30..70 {
            for x in 0..fb_width {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = 0xFF434C5E;
                }
            }
        }

        // Draw toolbar buttons (placeholders)
        let button_positions = [10, 60, 110, 160, 210];
        for &x_start in &button_positions {
            for y in 35..65 {
                for x in x_start..(x_start + 40) {
                    let offset = y * fb_width + x;
                    if offset < fb.len() {
                        fb[offset] = 0xFF4C566A;
                    }
                }
            }
        }
    }

    /// Draw file list
    fn draw_file_list(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        let list_top = 70;
        let item_height = 24;
        let visible_count = (fb_height - list_top - 30) / item_height;

        for (i, entry) in self.entries.iter().enumerate().skip(self.scroll_offset).take(visible_count) {
            let y_offset = list_top + (i - self.scroll_offset) * item_height;
            let is_selected = Some(i) == self.selected_index;

            // Draw selection background
            if is_selected {
                for y in y_offset..(y_offset + item_height) {
                    for x in 0..fb_width {
                        let offset = y * fb_width + x;
                        if offset < fb.len() {
                            fb[offset] = self.selection_color;
                        }
                    }
                }
            }

            // Draw icon (simplified - just a colored square)
            let icon_color = match entry.file_type {
                FileType::Directory => self.directory_color,
                _ => self.file_color,
            };

            for y in (y_offset + 4)..(y_offset + 20) {
                for x in 10..26 {
                    let offset = y * fb_width + x;
                    if offset < fb.len() {
                        fb[offset] = icon_color;
                    }
                }
            }

            // Draw filename (simplified - colored rectangle)
            for y in (y_offset + 8)..(y_offset + 16) {
                for x in 35..(35 + entry.name.len() * 8).min(fb_width - 10) {
                    let offset = y * fb_width + x;
                    if offset < fb.len() {
                        fb[offset] = if is_selected { 0xFFECEFF4 } else { self.fg_color };
                    }
                }
            }
        }
    }

    /// Draw status bar
    fn draw_status_bar(&self, fb: &mut [u32], fb_width: usize, fb_height: usize) {
        let status_height = 25;
        let status_top = fb_height - status_height;

        // Draw status bar background
        for y in status_top..fb_height {
            for x in 0..fb_width {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = 0xFF3B4252;
                }
            }
        }

        // Draw status text (simplified)
        for y in (status_top + 8)..(status_top + 17) {
            for x in 10..200 {
                let offset = y * fb_width + x;
                if offset < fb.len() {
                    fb[offset] = self.fg_color;
                }
            }
        }
    }
}

/// Main entry point
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Create file manager
    let mut fm = FileManager::new(800, 600);

    // Load initial directory
    fm.load_directory();

    // In a real implementation, this would:
    // 1. Connect to window manager to get a window
    // 2. Enter event loop:
    //    - Receive keyboard/mouse input
    //    - Process navigation commands
    //    - Update file list
    //    - Render to framebuffer
    //    - Send update to window manager

    // Render initial view
    let mut framebuffer = alloc::vec![0u32; 800 * 600];
    fm.render(&mut framebuffer, 800, 600);

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
