//! File Associations
//!
//! Manages the mapping from file extensions and MIME types to default
//! applications. Supports registering multiple alternative applications
//! per type and selecting a default.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// Association entry
// ---------------------------------------------------------------------------

/// A single file association entry.
#[derive(Debug, Clone)]
pub struct FileAssociation {
    /// File extension (without leading dot), e.g. "txt".
    pub extension: String,
    /// MIME type string, e.g. "text/plain".
    pub mime_type: String,
    /// Default application command.
    pub default_app: String,
    /// Alternative application commands.
    pub alternatives: Vec<String>,
}

impl FileAssociation {
    /// Create a new association.
    pub fn new(extension: &str, mime_type: &str, default_app: &str) -> Self {
        Self {
            extension: String::from(extension),
            mime_type: String::from(mime_type),
            default_app: String::from(default_app),
            alternatives: Vec::new(),
        }
    }

    /// Add an alternative application.
    pub fn add_alternative(&mut self, app: &str) {
        let s = String::from(app);
        if !self.alternatives.contains(&s) && s != self.default_app {
            self.alternatives.push(s);
        }
    }

    /// All available applications (default first, then alternatives).
    pub fn all_apps(&self) -> Vec<String> {
        let mut apps = vec![self.default_app.clone()];
        for alt in &self.alternatives {
            apps.push(alt.clone());
        }
        apps
    }
}

// ---------------------------------------------------------------------------
// Association registry
// ---------------------------------------------------------------------------

/// Registry of file associations, keyed by extension and MIME type.
#[derive(Debug)]
pub struct AssociationRegistry {
    /// Associations indexed by file extension (lowercase).
    by_extension: BTreeMap<String, FileAssociation>,
    /// Associations indexed by MIME type.
    by_mime: BTreeMap<String, FileAssociation>,
}

impl Default for AssociationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AssociationRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            by_extension: BTreeMap::new(),
            by_mime: BTreeMap::new(),
        }
    }

    /// Create a registry pre-populated with built-in defaults.
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();

        // Text files
        reg.register(FileAssociation::new("txt", "text/plain", "text_editor"));
        reg.register(FileAssociation::new("md", "text/markdown", "text_editor"));
        reg.register(FileAssociation::new("rs", "text/x-rust", "text_editor"));
        reg.register(FileAssociation::new("c", "text/x-c", "text_editor"));
        reg.register(FileAssociation::new("h", "text/x-c", "text_editor"));
        reg.register(FileAssociation::new("cpp", "text/x-c++", "text_editor"));
        reg.register(FileAssociation::new("py", "text/x-python", "text_editor"));
        reg.register(FileAssociation::new(
            "sh",
            "text/x-shellscript",
            "text_editor",
        ));
        reg.register(FileAssociation::new("toml", "text/x-toml", "text_editor"));
        reg.register(FileAssociation::new(
            "json",
            "application/json",
            "text_editor",
        ));
        reg.register(FileAssociation::new(
            "xml",
            "application/xml",
            "text_editor",
        ));
        reg.register(FileAssociation::new("html", "text/html", "text_editor"));
        reg.register(FileAssociation::new("css", "text/css", "text_editor"));

        // Images
        reg.register(FileAssociation::new("png", "image/png", "image_viewer"));
        reg.register(FileAssociation::new("jpg", "image/jpeg", "image_viewer"));
        reg.register(FileAssociation::new("jpeg", "image/jpeg", "image_viewer"));
        reg.register(FileAssociation::new("bmp", "image/bmp", "image_viewer"));
        reg.register(FileAssociation::new("tga", "image/x-tga", "image_viewer"));
        reg.register(FileAssociation::new("qoi", "image/x-qoi", "image_viewer"));
        reg.register(FileAssociation::new(
            "ppm",
            "image/x-portable-pixmap",
            "image_viewer",
        ));

        // Documents
        reg.register(FileAssociation::new("pdf", "application/pdf", "pdf_viewer"));

        // Audio
        reg.register(FileAssociation::new("wav", "audio/wav", "media_player"));
        reg.register(FileAssociation::new("mp3", "audio/mpeg", "media_player"));
        reg.register(FileAssociation::new("ogg", "audio/ogg", "media_player"));

        // Video
        reg.register(FileAssociation::new(
            "avi",
            "video/x-msvideo",
            "media_player",
        ));
        reg.register(FileAssociation::new("mp4", "video/mp4", "media_player"));

        reg
    }

    /// Register a file association.
    ///
    /// If an association for the same extension or MIME type already exists,
    /// the new one replaces it.
    pub fn register(&mut self, assoc: FileAssociation) {
        self.by_extension
            .insert(assoc.extension.clone(), assoc.clone());
        self.by_mime.insert(assoc.mime_type.clone(), assoc);
    }

    /// Look up an association by file extension (case-insensitive).
    pub fn lookup_by_ext(&self, ext: &str) -> Option<&FileAssociation> {
        // Convert to lowercase for lookup
        let mut lower = String::new();
        for c in ext.chars() {
            for lc in c.to_lowercase() {
                lower.push(lc);
            }
        }
        self.by_extension.get(&lower)
    }

    /// Look up an association by MIME type.
    pub fn lookup_by_mime(&self, mime: &str) -> Option<&FileAssociation> {
        self.by_mime.get(mime)
    }

    /// Set the default application for a given extension.
    pub fn set_default(&mut self, ext: &str, app: &str) -> bool {
        let mut lower = String::new();
        for c in ext.chars() {
            for lc in c.to_lowercase() {
                lower.push(lc);
            }
        }

        if let Some(assoc) = self.by_extension.get_mut(&lower) {
            let old = assoc.default_app.clone();
            assoc.default_app = String::from(app);
            // Move old default to alternatives if not already there
            if !old.is_empty() && old != app {
                assoc.add_alternative(&old);
            }
            // Update MIME entry too
            let mime = assoc.mime_type.clone();
            if let Some(mime_assoc) = self.by_mime.get_mut(&mime) {
                mime_assoc.default_app = String::from(app);
            }
            true
        } else {
            false
        }
    }

    /// Get the default application for a given extension.
    pub fn get_default(&self, ext: &str) -> Option<&str> {
        self.lookup_by_ext(ext).map(|a| a.default_app.as_str())
    }

    /// Number of registered extensions.
    pub fn extension_count(&self) -> usize {
        self.by_extension.len()
    }

    /// Number of registered MIME types.
    pub fn mime_count(&self) -> usize {
        self.by_mime.len()
    }

    /// Get the default app for a filename (extracts extension).
    pub fn get_app_for_file(&self, filename: &str) -> Option<&str> {
        let ext = filename.rsplit('.').next()?;
        self.get_default(ext)
    }
}

// ---------------------------------------------------------------------------
// Open-with dialog model
// ---------------------------------------------------------------------------

/// Model for an "Open With" dialog, presenting available apps for a file type.
#[derive(Debug)]
pub struct OpenWithDialog {
    /// File extension being opened.
    pub extension: String,
    /// MIME type of the file.
    pub mime_type: String,
    /// Available applications.
    pub apps: Vec<String>,
    /// Currently highlighted index.
    pub selected_index: usize,
    /// Whether the dialog is visible.
    pub visible: bool,
}

impl OpenWithDialog {
    /// Create a dialog from an association registry and file extension.
    pub fn from_registry(registry: &AssociationRegistry, ext: &str) -> Self {
        let (mime, apps) = if let Some(assoc) = registry.lookup_by_ext(ext) {
            (assoc.mime_type.clone(), assoc.all_apps())
        } else {
            (String::from("application/octet-stream"), Vec::new())
        };

        Self {
            extension: String::from(ext),
            mime_type: mime,
            apps,
            selected_index: 0,
            visible: true,
        }
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if !self.apps.is_empty() && self.selected_index + 1 < self.apps.len() {
            self.selected_index += 1;
        }
    }

    /// Get the currently selected application.
    pub fn selected_app(&self) -> Option<&str> {
        self.apps.get(self.selected_index).map(|s| s.as_str())
    }

    /// Dismiss the dialog.
    pub fn dismiss(&mut self) {
        self.visible = false;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_association() {
        let assoc = FileAssociation::new("txt", "text/plain", "editor");
        assert_eq!(assoc.extension, "txt");
        assert_eq!(assoc.default_app, "editor");
        assert!(assoc.alternatives.is_empty());
    }

    #[test]
    fn test_add_alternative() {
        let mut assoc = FileAssociation::new("txt", "text/plain", "editor");
        assoc.add_alternative("notepad");
        assoc.add_alternative("notepad"); // duplicate
        assert_eq!(assoc.alternatives.len(), 1);
        assoc.add_alternative("editor"); // same as default
        assert_eq!(assoc.alternatives.len(), 1);
    }

    #[test]
    fn test_registry_defaults() {
        let reg = AssociationRegistry::with_defaults();
        assert!(reg.extension_count() > 10);
        let txt = reg.lookup_by_ext("txt").unwrap();
        assert_eq!(txt.default_app, "text_editor");
    }

    #[test]
    fn test_lookup_by_mime() {
        let reg = AssociationRegistry::with_defaults();
        let assoc = reg.lookup_by_mime("image/png").unwrap();
        assert_eq!(assoc.default_app, "image_viewer");
    }

    #[test]
    fn test_set_default() {
        let mut reg = AssociationRegistry::with_defaults();
        assert!(reg.set_default("txt", "vscode"));
        let assoc = reg.lookup_by_ext("txt").unwrap();
        assert_eq!(assoc.default_app, "vscode");
        assert!(assoc.alternatives.contains(&String::from("text_editor")));
    }

    #[test]
    fn test_get_app_for_file() {
        let reg = AssociationRegistry::with_defaults();
        assert_eq!(reg.get_app_for_file("hello.rs"), Some("text_editor"));
        assert_eq!(reg.get_app_for_file("photo.png"), Some("image_viewer"));
    }

    #[test]
    fn test_open_with_dialog() {
        let reg = AssociationRegistry::with_defaults();
        let dialog = OpenWithDialog::from_registry(&reg, "txt");
        assert!(dialog.visible);
        assert!(!dialog.apps.is_empty());
        assert_eq!(dialog.selected_app(), Some("text_editor"));
    }

    #[test]
    fn test_open_with_navigation() {
        let mut reg = AssociationRegistry::with_defaults();
        let mut assoc = FileAssociation::new("txt", "text/plain", "editor");
        assoc.add_alternative("notepad");
        reg.register(assoc);
        let mut dialog = OpenWithDialog::from_registry(&reg, "txt");
        assert_eq!(dialog.selected_index, 0);
        dialog.select_next();
        assert_eq!(dialog.selected_index, 1);
        dialog.select_prev();
        assert_eq!(dialog.selected_index, 0);
    }
}
