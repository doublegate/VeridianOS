//! MIME Type Database
//!
//! Provides MIME type detection via file extension and magic byte analysis,
//! with associated application dispatch. Used by the file manager to open
//! files with the appropriate application.
//!
//! Detection strategy:
//! 1. Magic byte signatures (most reliable, checks file header bytes)
//! 2. File extension mapping (50+ extensions supported)
//! 3. Fallback to `Unknown`
//!
//! The `TODO(phase7): Open file in appropriate application via MIME dispatch`
//! in `file_manager.rs` line 229 is resolved by this module -- the file manager
//! can now call `MimeDatabase::detect_mime()` and `MimeDatabase::open_with()`
//! to determine what application should handle a given file.

#![allow(dead_code)]

use alloc::{string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// MIME type enumeration
// ---------------------------------------------------------------------------

/// Supported MIME types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeType {
    // Text types
    TextPlain,
    TextHtml,
    TextCss,
    TextJavascript,
    TextXml,
    TextMarkdown,
    TextRust,
    TextC,
    TextCpp,
    TextPython,
    TextShell,

    // Image types
    ImagePng,
    ImageJpeg,
    ImageGif,
    ImageBmp,
    ImageSvg,
    ImagePpm,

    // Audio types
    AudioWav,
    AudioMp3,
    AudioOgg,

    // Video types
    VideoMp4,
    VideoAvi,

    // Application types
    ApplicationPdf,
    ApplicationZip,
    ApplicationTar,
    ApplicationGzip,
    ApplicationElf,
    ApplicationDesktop,

    // Special types
    DirectoryType,
    Unknown,
}

// ---------------------------------------------------------------------------
// MIME category
// ---------------------------------------------------------------------------

/// Broad category for a MIME type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeCategory {
    Text,
    Image,
    Audio,
    Video,
    Application,
    Directory,
}

// ---------------------------------------------------------------------------
// Application association
// ---------------------------------------------------------------------------

/// Maps a MIME type to the application that should open it
#[derive(Debug, Clone)]
pub struct MimeAssociation {
    /// The MIME type this association applies to
    pub mime_type: MimeType,
    /// Human-readable application name (e.g. "Text Editor")
    pub app_name: String,
    /// Executable path (e.g. "/bin/text-editor")
    pub app_exec: String,
}

// ---------------------------------------------------------------------------
// MIME database
// ---------------------------------------------------------------------------

/// Central database for MIME type detection and application dispatch.
///
/// Contains a set of built-in default associations (populated in `new()`) plus
/// user-registered custom associations that take priority.
pub struct MimeDatabase {
    /// Built-in default associations (populated by `new()`)
    associations: Vec<MimeAssociation>,
    /// User-registered associations (checked first, override defaults)
    custom_associations: Vec<MimeAssociation>,
}

impl MimeDatabase {
    /// Create a new MIME database populated with default associations.
    ///
    /// Default dispatch table:
    /// - Text types -> Text Editor (`/bin/text-editor`)
    /// - Image types -> Image Viewer (`/bin/image-viewer`)
    /// - ELF binaries -> Terminal (`/bin/terminal`)
    /// - Directories -> File Manager (`/bin/file-manager`)
    /// - Everything else -> Text Editor (fallback)
    pub fn new() -> Self {
        let text_editor_name = String::from("Text Editor");
        let text_editor_exec = String::from("/bin/text-editor");
        let image_viewer_name = String::from("Image Viewer");
        let image_viewer_exec = String::from("/bin/image-viewer");

        let associations = vec![
            // ---- Text types -> Text Editor ----
            MimeAssociation {
                mime_type: MimeType::TextPlain,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextHtml,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextCss,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextJavascript,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextXml,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextMarkdown,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextRust,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextC,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextCpp,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextPython,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::TextShell,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            // ---- Image types -> Image Viewer ----
            MimeAssociation {
                mime_type: MimeType::ImagePng,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ImageJpeg,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ImageGif,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ImageBmp,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ImageSvg,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ImagePpm,
                app_name: image_viewer_name.clone(),
                app_exec: image_viewer_exec.clone(),
            },
            // ---- Audio types -> Text Editor (placeholder, no audio player yet) ----
            MimeAssociation {
                mime_type: MimeType::AudioWav,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::AudioMp3,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::AudioOgg,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            // ---- Video types -> Text Editor (placeholder, no video player yet) ----
            MimeAssociation {
                mime_type: MimeType::VideoMp4,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::VideoAvi,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            // ---- Application types ----
            MimeAssociation {
                mime_type: MimeType::ApplicationPdf,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ApplicationZip,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ApplicationTar,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ApplicationGzip,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            MimeAssociation {
                mime_type: MimeType::ApplicationElf,
                app_name: String::from("Terminal"),
                app_exec: String::from("/bin/terminal"),
            },
            MimeAssociation {
                mime_type: MimeType::ApplicationDesktop,
                app_name: text_editor_name.clone(),
                app_exec: text_editor_exec.clone(),
            },
            // ---- Special types ----
            MimeAssociation {
                mime_type: MimeType::DirectoryType,
                app_name: String::from("File Manager"),
                app_exec: String::from("/bin/file-manager"),
            },
            // ---- Fallback for Unknown ----
            MimeAssociation {
                mime_type: MimeType::Unknown,
                app_name: text_editor_name,
                app_exec: text_editor_exec,
            },
        ];

        Self {
            associations,
            custom_associations: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Detection
    // -----------------------------------------------------------------------

    /// Detect the MIME type of a file.
    ///
    /// Detection order:
    /// 1. Magic byte signatures (if `header_bytes` is `Some`)
    /// 2. File extension
    /// 3. `MimeType::Unknown` fallback
    pub fn detect_mime(filename: &str, header_bytes: Option<&[u8]>) -> MimeType {
        // Step 1: Try magic bytes first (most reliable)
        if let Some(bytes) = header_bytes {
            if let Some(mime) = Self::detect_from_magic(bytes) {
                return mime;
            }
        }

        // Step 2: Try extension
        if let Some(ext) = get_extension(filename) {
            return detect_mime_from_extension(ext);
        }

        // Step 3: Fallback
        MimeType::Unknown
    }

    /// Attempt to detect MIME type from magic byte signatures.
    ///
    /// Checks the first few bytes of a file against well-known magic numbers.
    /// Returns `None` if no match is found.
    fn detect_from_magic(bytes: &[u8]) -> Option<MimeType> {
        if bytes.len() < 2 {
            return None;
        }

        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if bytes.len() >= 4
            && bytes[0] == 0x89
            && bytes[1] == 0x50
            && bytes[2] == 0x4E
            && bytes[3] == 0x47
        {
            return Some(MimeType::ImagePng);
        }

        // JPEG: FF D8 FF
        if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
            return Some(MimeType::ImageJpeg);
        }

        // GIF: 47 49 46 ("GIF")
        if bytes.len() >= 3 && bytes[0] == 0x47 && bytes[1] == 0x49 && bytes[2] == 0x46 {
            return Some(MimeType::ImageGif);
        }

        // BMP: 42 4D ("BM")
        if bytes[0] == 0x42 && bytes[1] == 0x4D {
            return Some(MimeType::ImageBmp);
        }

        // PDF: 25 50 44 46 ("%PDF")
        if bytes.len() >= 4
            && bytes[0] == 0x25
            && bytes[1] == 0x50
            && bytes[2] == 0x44
            && bytes[3] == 0x46
        {
            return Some(MimeType::ApplicationPdf);
        }

        // ZIP / PK archives: 50 4B 03 04
        if bytes.len() >= 4
            && bytes[0] == 0x50
            && bytes[1] == 0x4B
            && bytes[2] == 0x03
            && bytes[3] == 0x04
        {
            return Some(MimeType::ApplicationZip);
        }

        // GZIP: 1F 8B
        if bytes[0] == 0x1F && bytes[1] == 0x8B {
            return Some(MimeType::ApplicationGzip);
        }

        // ELF: 7F 45 4C 46
        if bytes.len() >= 4
            && bytes[0] == 0x7F
            && bytes[1] == 0x45
            && bytes[2] == 0x4C
            && bytes[3] == 0x46
        {
            return Some(MimeType::ApplicationElf);
        }

        // WAV: RIFF....WAVE
        if bytes.len() >= 12
            && bytes[0] == b'R'
            && bytes[1] == b'I'
            && bytes[2] == b'F'
            && bytes[3] == b'F'
            && bytes[8] == b'W'
            && bytes[9] == b'A'
            && bytes[10] == b'V'
            && bytes[11] == b'E'
        {
            return Some(MimeType::AudioWav);
        }

        // OGG: 4F 67 67 53 ("OggS")
        if bytes.len() >= 4
            && bytes[0] == b'O'
            && bytes[1] == b'g'
            && bytes[2] == b'g'
            && bytes[3] == b'S'
        {
            return Some(MimeType::AudioOgg);
        }

        // MP3: FF FB, FF F3, FF F2, or ID3 tag
        if bytes.len() >= 3
            && ((bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
                || (bytes[0] == b'I' && bytes[1] == b'D' && bytes[2] == b'3'))
        {
            return Some(MimeType::AudioMp3);
        }

        // PPM: "P6" (binary) or "P3" (ASCII)
        if bytes.len() >= 2 && bytes[0] == b'P' && (bytes[1] == b'6' || bytes[1] == b'3') {
            return Some(MimeType::ImagePpm);
        }

        // SVG: check for "<?xml" or "<svg" at the start (heuristic)
        if bytes.len() >= 5
            && ((bytes[0] == b'<'
                && bytes[1] == b'?'
                && bytes[2] == b'x'
                && bytes[3] == b'm'
                && bytes[4] == b'l')
                || (bytes.len() >= 4
                    && bytes[0] == b'<'
                    && bytes[1] == b's'
                    && bytes[2] == b'v'
                    && bytes[3] == b'g'))
        {
            // Could be SVG if it contains <svg, but the xml check alone is
            // insufficient. We rely on extension for SVG primarily; this is
            // a best-effort heuristic.
            // For pure XML, return TextXml; extension-based detection will
            // refine to SVG if the extension says so.
            return Some(MimeType::TextXml);
        }

        // TAR: USTAR magic at offset 257
        if bytes.len() >= 263
            && bytes[257] == b'u'
            && bytes[258] == b's'
            && bytes[259] == b't'
            && bytes[260] == b'a'
            && bytes[261] == b'r'
        {
            return Some(MimeType::ApplicationTar);
        }

        None
    }

    // -----------------------------------------------------------------------
    // Application dispatch
    // -----------------------------------------------------------------------

    /// Get the default application for a MIME type.
    ///
    /// Checks custom associations first, then falls back to the built-in
    /// default table. Returns `None` only if both tables are empty (should
    /// not happen with default construction).
    pub fn open_with(&self, mime: &MimeType) -> Option<&MimeAssociation> {
        // Custom associations take priority
        for assoc in &self.custom_associations {
            if assoc.mime_type == *mime {
                return Some(assoc);
            }
        }

        // Fall back to defaults
        for assoc in &self.associations {
            if assoc.mime_type == *mime {
                return Some(assoc);
            }
        }

        // Final fallback: try Unknown entry
        if *mime != MimeType::Unknown {
            for assoc in &self.associations {
                if assoc.mime_type == MimeType::Unknown {
                    return Some(assoc);
                }
            }
        }

        None
    }

    /// Register a custom MIME association.
    ///
    /// Custom associations take priority over built-in defaults. If an
    /// association for the same MIME type already exists in the custom list,
    /// it is replaced.
    pub fn register_association(
        &mut self,
        mime_type: MimeType,
        app_name: String,
        app_exec: String,
    ) {
        // Remove any existing custom association for this MIME type
        self.custom_associations
            .retain(|a| a.mime_type != mime_type);

        self.custom_associations.push(MimeAssociation {
            mime_type,
            app_name,
            app_exec,
        });
    }

    // -----------------------------------------------------------------------
    // Classification helpers
    // -----------------------------------------------------------------------

    /// Return the broad category for a MIME type.
    pub fn category(mime: &MimeType) -> MimeCategory {
        match mime {
            MimeType::TextPlain
            | MimeType::TextHtml
            | MimeType::TextCss
            | MimeType::TextJavascript
            | MimeType::TextXml
            | MimeType::TextMarkdown
            | MimeType::TextRust
            | MimeType::TextC
            | MimeType::TextCpp
            | MimeType::TextPython
            | MimeType::TextShell => MimeCategory::Text,

            MimeType::ImagePng
            | MimeType::ImageJpeg
            | MimeType::ImageGif
            | MimeType::ImageBmp
            | MimeType::ImageSvg
            | MimeType::ImagePpm => MimeCategory::Image,

            MimeType::AudioWav | MimeType::AudioMp3 | MimeType::AudioOgg => MimeCategory::Audio,

            MimeType::VideoMp4 | MimeType::VideoAvi => MimeCategory::Video,

            MimeType::ApplicationPdf
            | MimeType::ApplicationZip
            | MimeType::ApplicationTar
            | MimeType::ApplicationGzip
            | MimeType::ApplicationElf
            | MimeType::ApplicationDesktop => MimeCategory::Application,

            MimeType::DirectoryType => MimeCategory::Directory,

            MimeType::Unknown => MimeCategory::Application,
        }
    }

    /// Return the standard MIME type string (e.g. `"text/plain"`).
    pub fn mime_to_str(mime: &MimeType) -> &'static str {
        match mime {
            MimeType::TextPlain => "text/plain",
            MimeType::TextHtml => "text/html",
            MimeType::TextCss => "text/css",
            MimeType::TextJavascript => "text/javascript",
            MimeType::TextXml => "text/xml",
            MimeType::TextMarkdown => "text/markdown",
            MimeType::TextRust => "text/x-rust",
            MimeType::TextC => "text/x-csrc",
            MimeType::TextCpp => "text/x-c++src",
            MimeType::TextPython => "text/x-python",
            MimeType::TextShell => "text/x-shellscript",

            MimeType::ImagePng => "image/png",
            MimeType::ImageJpeg => "image/jpeg",
            MimeType::ImageGif => "image/gif",
            MimeType::ImageBmp => "image/bmp",
            MimeType::ImageSvg => "image/svg+xml",
            MimeType::ImagePpm => "image/x-portable-pixmap",

            MimeType::AudioWav => "audio/wav",
            MimeType::AudioMp3 => "audio/mpeg",
            MimeType::AudioOgg => "audio/ogg",

            MimeType::VideoMp4 => "video/mp4",
            MimeType::VideoAvi => "video/x-msvideo",

            MimeType::ApplicationPdf => "application/pdf",
            MimeType::ApplicationZip => "application/zip",
            MimeType::ApplicationTar => "application/x-tar",
            MimeType::ApplicationGzip => "application/gzip",
            MimeType::ApplicationElf => "application/x-elf",
            MimeType::ApplicationDesktop => "application/x-desktop",

            MimeType::DirectoryType => "inode/directory",

            MimeType::Unknown => "application/octet-stream",
        }
    }

    /// Return a BGRA color for the file type icon in the file manager.
    ///
    /// Colors are chosen for quick visual identification:
    /// - Text/code files: muted shades
    /// - Images: bright green
    /// - Audio: orange
    /// - Video: magenta
    /// - Archives: yellow
    /// - Executables: red
    /// - Directories: blue (matches file_manager.rs existing `0x55AAFF`)
    ///
    /// The returned value is packed BGRA (B in bits 0-7, G in 8-15, R in
    /// 16-23, A in 24-31) matching the framebuffer byte order.
    pub fn icon_color(mime: &MimeType) -> u32 {
        match mime {
            // Text / code -- light gray
            MimeType::TextPlain => 0xFFCCCCCC,

            // Source code -- specific accent colors
            MimeType::TextRust => 0xFFDE8A56, // Rust orange-brown (BGRA)
            MimeType::TextC => 0xFFD19A55,    // C blue (looks brownish in BGRA)
            MimeType::TextCpp => 0xFFCB6D9F,  // C++ rose
            MimeType::TextPython => 0xFF55B4D1, // Python teal
            MimeType::TextShell => 0xFF66CC66, // Shell green

            MimeType::TextHtml => 0xFFE06633,       // HTML orange
            MimeType::TextCss => 0xFFCC6699,        // CSS pink
            MimeType::TextJavascript => 0xFF33CCDD, // JS cyan
            MimeType::TextXml => 0xFFAA8866,        // XML tan
            MimeType::TextMarkdown => 0xFFBBBBDD,   // Markdown lavender

            // Images -- bright green
            MimeType::ImagePng
            | MimeType::ImageJpeg
            | MimeType::ImageGif
            | MimeType::ImageBmp
            | MimeType::ImageSvg
            | MimeType::ImagePpm => 0xFF44DD44,

            // Audio -- orange
            MimeType::AudioWav | MimeType::AudioMp3 | MimeType::AudioOgg => 0xFF44AAEE,

            // Video -- magenta / purple
            MimeType::VideoMp4 | MimeType::VideoAvi => 0xFFDD44DD,

            // PDF -- dark red
            MimeType::ApplicationPdf => 0xFF3333CC,

            // Archives -- yellow
            MimeType::ApplicationZip | MimeType::ApplicationTar | MimeType::ApplicationGzip => {
                0xFF33DDDD
            }

            // ELF executable -- red
            MimeType::ApplicationElf => 0xFF4444EE,

            // Desktop entry -- cyan
            MimeType::ApplicationDesktop => 0xFFDDBB33,

            // Directory -- blue (matches file_manager.rs existing 0x55AAFF)
            MimeType::DirectoryType => 0xFFFFAA55,

            // Unknown -- dim gray
            MimeType::Unknown => 0xFF888888,
        }
    }
}

impl Default for MimeDatabase {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Free-standing helpers
// ---------------------------------------------------------------------------

/// Extract the file extension from a filename, lowercased for comparison.
///
/// Returns the extension without the leading dot, or `None` if there is no
/// extension. The returned slice borrows from `filename`.
///
/// # Examples (conceptual, no_std)
/// - `"readme.txt"` -> `Some("txt")`
/// - `"Makefile"` -> `None`
/// - `"archive.tar.gz"` -> `Some("gz")`
/// - `".hidden"` -> `None` (dot-files with no further extension)
pub fn get_extension(filename: &str) -> Option<&str> {
    // Find the last dot that is not the first character
    let bytes = filename.as_bytes();
    let mut dot_pos: Option<usize> = None;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'.' {
            // Dot at position 0 is a hidden file prefix, not an extension
            if i == 0 {
                return None;
            }
            // Dot right after '/' is also not an extension (e.g. "path/.hidden")
            if i > 0 && bytes[i - 1] == b'/' {
                return None;
            }
            dot_pos = Some(i);
            break;
        }
        // Stop searching if we hit a path separator
        if bytes[i] == b'/' {
            return None;
        }
    }

    dot_pos.map(|pos| &filename[pos + 1..])
}

/// Detect MIME type purely from file extension.
///
/// Performs case-insensitive comparison by checking both the original
/// extension and an ASCII-lowercased copy.
pub fn detect_mime_from_extension(ext: &str) -> MimeType {
    // We need case-insensitive matching. Since extensions are short ASCII
    // strings, we lowercase into a small stack buffer. Extensions longer
    // than 15 bytes are unsupported and fall through to Unknown.
    let mut lower_buf = [0u8; 16];
    let ext_bytes = ext.as_bytes();
    if ext_bytes.len() > 15 {
        return MimeType::Unknown;
    }
    for (i, &b) in ext_bytes.iter().enumerate() {
        lower_buf[i] = if b.is_ascii_uppercase() { b + 32 } else { b };
    }
    let lower = core::str::from_utf8(&lower_buf[..ext_bytes.len()]).unwrap_or("");

    match lower {
        // Plain text
        "txt" | "text" | "log" | "cfg" | "conf" | "ini" => MimeType::TextPlain,

        // Markup / web
        "html" | "htm" | "xhtml" => MimeType::TextHtml,
        "css" => MimeType::TextCss,
        "js" | "mjs" | "cjs" => MimeType::TextJavascript,
        "xml" | "xsl" | "xslt" => MimeType::TextXml,
        "md" | "markdown" | "mkd" => MimeType::TextMarkdown,

        // Programming languages
        "rs" => MimeType::TextRust,
        "c" | "h" => MimeType::TextC,
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => MimeType::TextCpp,
        "py" | "pyw" | "pyi" => MimeType::TextPython,
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" => MimeType::TextShell,

        // Additional text formats (treated as plain text)
        "json" | "yaml" | "yml" | "toml" | "csv" | "tsv" => MimeType::TextPlain,
        "diff" | "patch" => MimeType::TextPlain,
        "makefile" => MimeType::TextPlain,

        // Images
        "png" => MimeType::ImagePng,
        "jpg" | "jpeg" | "jpe" => MimeType::ImageJpeg,
        "gif" => MimeType::ImageGif,
        "bmp" | "dib" => MimeType::ImageBmp,
        "svg" | "svgz" => MimeType::ImageSvg,
        "ppm" | "pgm" | "pbm" | "pnm" => MimeType::ImagePpm,

        // Audio
        "wav" | "wave" => MimeType::AudioWav,
        "mp3" => MimeType::AudioMp3,
        "ogg" | "oga" | "opus" => MimeType::AudioOgg,

        // Video
        "mp4" | "m4v" => MimeType::VideoMp4,
        "avi" => MimeType::VideoAvi,

        // Application / archives
        "pdf" => MimeType::ApplicationPdf,
        "zip" | "jar" => MimeType::ApplicationZip,
        "tar" => MimeType::ApplicationTar,
        "gz" | "gzip" | "tgz" => MimeType::ApplicationGzip,
        "elf" | "bin" | "out" => MimeType::ApplicationElf,
        "desktop" => MimeType::ApplicationDesktop,

        _ => MimeType::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Display / Debug helpers for MimeType
// ---------------------------------------------------------------------------

impl MimeType {
    /// Return the standard MIME string. Convenience wrapper around
    /// `MimeDatabase::mime_to_str`.
    pub fn as_str(&self) -> &'static str {
        MimeDatabase::mime_to_str(self)
    }

    /// Return the broad category.
    pub fn category(&self) -> MimeCategory {
        MimeDatabase::category(self)
    }

    /// Return a BGRA icon color for this type.
    pub fn icon_color(&self) -> u32 {
        MimeDatabase::icon_color(self)
    }

    /// Return `true` if this is any text/source code type.
    pub fn is_text(&self) -> bool {
        matches!(self.category(), MimeCategory::Text)
    }

    /// Return `true` if this is any image type.
    pub fn is_image(&self) -> bool {
        matches!(self.category(), MimeCategory::Image)
    }

    /// Return `true` if this is any audio type.
    pub fn is_audio(&self) -> bool {
        matches!(self.category(), MimeCategory::Audio)
    }

    /// Return `true` if this is any video type.
    pub fn is_video(&self) -> bool {
        matches!(self.category(), MimeCategory::Video)
    }

    /// Return `true` if this is the directory pseudo-type.
    pub fn is_directory(&self) -> bool {
        *self == MimeType::DirectoryType
    }

    /// Return `true` if this is an executable binary.
    pub fn is_executable(&self) -> bool {
        *self == MimeType::ApplicationElf
    }

    /// Return `true` if this is an archive format.
    pub fn is_archive(&self) -> bool {
        matches!(
            self,
            MimeType::ApplicationZip | MimeType::ApplicationTar | MimeType::ApplicationGzip
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Extension extraction -----------------------------------------------

    #[test]
    fn test_get_extension_basic() {
        assert_eq!(get_extension("readme.txt"), Some("txt"));
        assert_eq!(get_extension("archive.tar.gz"), Some("gz"));
        assert_eq!(get_extension("Makefile"), None);
        assert_eq!(get_extension(".hidden"), None);
        assert_eq!(get_extension("path/.hidden"), None);
        assert_eq!(get_extension("no_ext"), None);
        assert_eq!(get_extension("photo.JPEG"), Some("JPEG"));
    }

    #[test]
    fn test_get_extension_empty() {
        assert_eq!(get_extension(""), None);
        assert_eq!(get_extension("."), None);
    }

    // -- Extension-based detection ------------------------------------------

    #[test]
    fn test_detect_extension_text() {
        assert_eq!(detect_mime_from_extension("txt"), MimeType::TextPlain);
        assert_eq!(detect_mime_from_extension("TXT"), MimeType::TextPlain);
        assert_eq!(detect_mime_from_extension("rs"), MimeType::TextRust);
        assert_eq!(detect_mime_from_extension("c"), MimeType::TextC);
        assert_eq!(detect_mime_from_extension("cpp"), MimeType::TextCpp);
        assert_eq!(detect_mime_from_extension("py"), MimeType::TextPython);
        assert_eq!(detect_mime_from_extension("sh"), MimeType::TextShell);
        assert_eq!(detect_mime_from_extension("md"), MimeType::TextMarkdown);
    }

    #[test]
    fn test_detect_extension_images() {
        assert_eq!(detect_mime_from_extension("png"), MimeType::ImagePng);
        assert_eq!(detect_mime_from_extension("jpg"), MimeType::ImageJpeg);
        assert_eq!(detect_mime_from_extension("jpeg"), MimeType::ImageJpeg);
        assert_eq!(detect_mime_from_extension("gif"), MimeType::ImageGif);
        assert_eq!(detect_mime_from_extension("bmp"), MimeType::ImageBmp);
        assert_eq!(detect_mime_from_extension("svg"), MimeType::ImageSvg);
        assert_eq!(detect_mime_from_extension("ppm"), MimeType::ImagePpm);
    }

    #[test]
    fn test_detect_extension_archives() {
        assert_eq!(detect_mime_from_extension("zip"), MimeType::ApplicationZip);
        assert_eq!(detect_mime_from_extension("tar"), MimeType::ApplicationTar);
        assert_eq!(detect_mime_from_extension("gz"), MimeType::ApplicationGzip);
    }

    #[test]
    fn test_detect_extension_unknown() {
        assert_eq!(detect_mime_from_extension("xyz"), MimeType::Unknown);
        assert_eq!(detect_mime_from_extension(""), MimeType::Unknown);
    }

    // -- Magic byte detection -----------------------------------------------

    #[test]
    fn test_magic_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ImagePng
        );
    }

    #[test]
    fn test_magic_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ImageJpeg
        );
    }

    #[test]
    fn test_magic_gif() {
        let bytes = b"GIF89a";
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(bytes)),
            MimeType::ImageGif
        );
    }

    #[test]
    fn test_magic_elf() {
        let bytes = [0x7F, 0x45, 0x4C, 0x46, 0x02, 0x01];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ApplicationElf
        );
    }

    #[test]
    fn test_magic_pdf() {
        let bytes = b"%PDF-1.7";
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(bytes)),
            MimeType::ApplicationPdf
        );
    }

    #[test]
    fn test_magic_gzip() {
        let bytes = [0x1F, 0x8B, 0x08, 0x00];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ApplicationGzip
        );
    }

    #[test]
    fn test_magic_zip() {
        let bytes = [0x50, 0x4B, 0x03, 0x04];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ApplicationZip
        );
    }

    #[test]
    fn test_magic_bmp() {
        let bytes = [0x42, 0x4D, 0x00, 0x00];
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::ImageBmp
        );
    }

    #[test]
    fn test_magic_wav() {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(b"RIFF");
        bytes[8..12].copy_from_slice(b"WAVE");
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(&bytes)),
            MimeType::AudioWav
        );
    }

    #[test]
    fn test_magic_ogg() {
        let bytes = b"OggS\x00\x02";
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(bytes)),
            MimeType::AudioOgg
        );
    }

    #[test]
    fn test_magic_ppm() {
        let bytes = b"P6\n640 480\n255\n";
        assert_eq!(
            MimeDatabase::detect_mime("unknown", Some(bytes)),
            MimeType::ImagePpm
        );
    }

    #[test]
    fn test_magic_priority_over_extension() {
        // ELF binary with a .txt extension -- magic bytes should win
        let elf_bytes = [0x7F, 0x45, 0x4C, 0x46, 0x02, 0x01];
        assert_eq!(
            MimeDatabase::detect_mime("sneaky.txt", Some(&elf_bytes)),
            MimeType::ApplicationElf
        );
    }

    #[test]
    fn test_extension_fallback_when_no_magic() {
        assert_eq!(
            MimeDatabase::detect_mime("script.py", None),
            MimeType::TextPython
        );
    }

    // -- Application dispatch -----------------------------------------------

    #[test]
    fn test_open_with_defaults() {
        let db = MimeDatabase::new();

        let assoc = db.open_with(&MimeType::TextPlain).unwrap();
        assert_eq!(assoc.app_name, "Text Editor");

        let assoc = db.open_with(&MimeType::ImagePng).unwrap();
        assert_eq!(assoc.app_name, "Image Viewer");

        let assoc = db.open_with(&MimeType::ApplicationElf).unwrap();
        assert_eq!(assoc.app_name, "Terminal");

        let assoc = db.open_with(&MimeType::DirectoryType).unwrap();
        assert_eq!(assoc.app_name, "File Manager");
    }

    #[test]
    fn test_custom_association_overrides_default() {
        let mut db = MimeDatabase::new();

        // Override TextPlain to use a custom editor
        db.register_association(
            MimeType::TextPlain,
            String::from("Custom Editor"),
            String::from("/bin/custom-editor"),
        );

        let assoc = db.open_with(&MimeType::TextPlain).unwrap();
        assert_eq!(assoc.app_name, "Custom Editor");
        assert_eq!(assoc.app_exec, "/bin/custom-editor");
    }

    #[test]
    fn test_custom_association_replaces_existing() {
        let mut db = MimeDatabase::new();

        db.register_association(
            MimeType::ImagePng,
            String::from("Viewer A"),
            String::from("/bin/a"),
        );
        db.register_association(
            MimeType::ImagePng,
            String::from("Viewer B"),
            String::from("/bin/b"),
        );

        let assoc = db.open_with(&MimeType::ImagePng).unwrap();
        assert_eq!(assoc.app_name, "Viewer B");
    }

    // -- Category / classification ------------------------------------------

    #[test]
    fn test_category() {
        assert_eq!(
            MimeDatabase::category(&MimeType::TextRust),
            MimeCategory::Text
        );
        assert_eq!(
            MimeDatabase::category(&MimeType::ImagePng),
            MimeCategory::Image
        );
        assert_eq!(
            MimeDatabase::category(&MimeType::AudioWav),
            MimeCategory::Audio
        );
        assert_eq!(
            MimeDatabase::category(&MimeType::VideoMp4),
            MimeCategory::Video
        );
        assert_eq!(
            MimeDatabase::category(&MimeType::ApplicationElf),
            MimeCategory::Application,
        );
        assert_eq!(
            MimeDatabase::category(&MimeType::DirectoryType),
            MimeCategory::Directory,
        );
    }

    #[test]
    fn test_mime_to_str() {
        assert_eq!(
            MimeDatabase::mime_to_str(&MimeType::TextPlain),
            "text/plain"
        );
        assert_eq!(MimeDatabase::mime_to_str(&MimeType::ImagePng), "image/png");
        assert_eq!(
            MimeDatabase::mime_to_str(&MimeType::ApplicationElf),
            "application/x-elf"
        );
        assert_eq!(
            MimeDatabase::mime_to_str(&MimeType::Unknown),
            "application/octet-stream"
        );
    }

    // -- MimeType convenience methods ---------------------------------------

    #[test]
    fn test_mimetype_helpers() {
        assert!(MimeType::TextRust.is_text());
        assert!(!MimeType::TextRust.is_image());
        assert!(MimeType::ImagePng.is_image());
        assert!(MimeType::AudioMp3.is_audio());
        assert!(MimeType::VideoMp4.is_video());
        assert!(MimeType::DirectoryType.is_directory());
        assert!(MimeType::ApplicationElf.is_executable());
        assert!(MimeType::ApplicationZip.is_archive());
        assert!(MimeType::ApplicationTar.is_archive());
        assert!(MimeType::ApplicationGzip.is_archive());
    }

    // -- Icon color ---------------------------------------------------------

    #[test]
    fn test_icon_color_nonzero() {
        // Every MIME type should have a non-zero icon color
        let types = [
            MimeType::TextPlain,
            MimeType::TextRust,
            MimeType::ImagePng,
            MimeType::AudioWav,
            MimeType::VideoMp4,
            MimeType::ApplicationElf,
            MimeType::DirectoryType,
            MimeType::Unknown,
        ];
        for t in &types {
            assert_ne!(MimeDatabase::icon_color(t), 0);
        }
    }
}
