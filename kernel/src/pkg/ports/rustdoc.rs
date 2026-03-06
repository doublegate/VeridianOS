//! Rustdoc Generation on Target
//!
//! Native documentation generation for VeridianOS packages. Manages
//! `cargo doc` invocation, search index construction, theme configuration,
//! and HTML output path management with conceptual VFS integration.
//!
//! Supports single-package, workspace-wide, and cross-referenced
//! documentation builds with configurable themes and a simple HTTP
//! serving configuration for local doc browsing.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default output directory for generated documentation
pub const DEFAULT_DOC_OUTPUT: &str = "/usr/share/doc/rust";

/// Default HTTP port for documentation server
pub const DEFAULT_DOC_SERVER_PORT: u16 = 8080;

/// Maximum number of items in a search index before compaction
pub const MAX_SEARCH_INDEX_ITEMS: usize = 100_000;

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// Documentation theme variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocTheme {
    /// Rustdoc default light theme
    Default,
    /// Dark theme (ayu)
    Dark,
    /// High-contrast for accessibility
    HighContrast,
}

impl DocTheme {
    /// Return the rustdoc CLI flag value for this theme.
    pub fn flag_value(self) -> &'static str {
        match self {
            Self::Default => "light",
            Self::Dark => "ayu",
            Self::HighContrast => "high-contrast",
        }
    }
}

// ---------------------------------------------------------------------------
// DocItem — a single documented entity
// ---------------------------------------------------------------------------

/// Kind of a documented item (function, struct, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocItemKind {
    Function,
    Struct,
    Enum,
    Trait,
    Module,
    Macro,
    Const,
    Type,
}

impl DocItemKind {
    /// Short label used in search results.
    pub fn label(self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Module => "mod",
            Self::Macro => "macro",
            Self::Const => "const",
            Self::Type => "type",
        }
    }
}

/// A single documented item stored in the search index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocItem {
    /// Simple name (e.g. `HashMap`)
    pub name: String,
    /// Item kind
    pub kind: DocItemKind,
    /// Fully-qualified path (e.g. `std::collections::HashMap`)
    pub path: String,
    /// One-line description extracted from `///` doc comment
    pub description: String,
    /// Whether the item is `pub`
    pub is_public: bool,
}

impl DocItem {
    /// Create a new documented item.
    pub fn new(
        name: &str,
        kind: DocItemKind,
        path: &str,
        description: &str,
        is_public: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            kind,
            path: path.to_string(),
            description: description.to_string(),
            is_public,
        }
    }

    /// Return the relative HTML file path for this item.
    ///
    /// E.g. `std/collections/struct.HashMap.html`
    pub fn html_path(&self) -> String {
        let module_path = self.path.replace("::", "/");
        alloc::format!("{}/{}.{}.html", module_path, self.kind.label(), self.name)
    }
}

// ---------------------------------------------------------------------------
// DocIndex — searchable index of documented items
// ---------------------------------------------------------------------------

/// Searchable index of all documented items across one or more crates.
#[derive(Debug, Clone)]
pub struct DocIndex {
    items: Vec<DocItem>,
}

impl Default for DocIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DocIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Add an item to the index.
    pub fn add(&mut self, item: DocItem) -> Result<(), KernelError> {
        if self.items.len() >= MAX_SEARCH_INDEX_ITEMS {
            return Err(KernelError::ResourceExhausted {
                resource: "doc_search_index",
            });
        }
        self.items.push(item);
        Ok(())
    }

    /// Number of indexed items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Search by name substring (case-insensitive).
    pub fn search_by_name(&self, query: &str) -> Vec<&DocItem> {
        let query_lower = query.to_ascii_lowercase();
        self.items
            .iter()
            .filter(|item| {
                let name_lower = item.name.to_ascii_lowercase();
                name_lower.contains(&query_lower)
            })
            .collect()
    }

    /// Search by fully-qualified path prefix.
    pub fn search_by_path(&self, prefix: &str) -> Vec<&DocItem> {
        self.items
            .iter()
            .filter(|item| item.path.starts_with(prefix))
            .collect()
    }

    /// Return counts per item kind.
    pub fn statistics(&self) -> DocIndexStats {
        let mut stats = DocIndexStats::default();
        for item in &self.items {
            match item.kind {
                DocItemKind::Function => stats.functions += 1,
                DocItemKind::Struct => stats.structs += 1,
                DocItemKind::Enum => stats.enums += 1,
                DocItemKind::Trait => stats.traits += 1,
                DocItemKind::Module => stats.modules += 1,
                DocItemKind::Macro => stats.macros += 1,
                DocItemKind::Const => stats.consts += 1,
                DocItemKind::Type => stats.types += 1,
            }
        }
        stats.total = self.items.len();
        stats
    }
}

/// Per-kind counts for a [`DocIndex`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocIndexStats {
    pub total: usize,
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub modules: usize,
    pub macros: usize,
    pub consts: usize,
    pub types: usize,
}

// ---------------------------------------------------------------------------
// RustdocConfig — what to document
// ---------------------------------------------------------------------------

/// Configuration for a rustdoc generation run.
#[derive(Debug, Clone)]
pub struct RustdocConfig {
    /// Root path of the source crate or workspace
    pub source_path: String,
    /// Output directory for generated HTML
    pub output_dir: String,
    /// Documentation theme
    pub theme: DocTheme,
    /// Enable cross-reference linking between crates
    pub cross_references: bool,
    /// Extra features to pass via `--features`
    pub features: Vec<String>,
    /// Extra `--cfg` flags
    pub cfg_flags: Vec<String>,
    /// Whether to include private items (`--document-private-items`)
    pub document_private: bool,
}

impl Default for RustdocConfig {
    fn default() -> Self {
        Self {
            source_path: String::from("/src"),
            output_dir: DEFAULT_DOC_OUTPUT.to_string(),
            theme: DocTheme::Default,
            cross_references: true,
            features: Vec::new(),
            cfg_flags: Vec::new(),
            document_private: false,
        }
    }
}

impl RustdocConfig {
    /// Create a config pointing at a specific crate directory.
    pub fn for_crate(crate_path: &str, output_dir: &str) -> Self {
        Self {
            source_path: crate_path.to_string(),
            output_dir: output_dir.to_string(),
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// DocServerConfig — simple HTTP serving
// ---------------------------------------------------------------------------

/// Minimal configuration for serving docs over HTTP.
#[derive(Debug, Clone)]
pub struct DocServerConfig {
    /// Directory containing generated HTML
    pub doc_root: String,
    /// Port to listen on
    pub port: u16,
    /// Bind address (e.g. `0.0.0.0` or `127.0.0.1`)
    pub bind_address: String,
}

impl Default for DocServerConfig {
    fn default() -> Self {
        Self {
            doc_root: DEFAULT_DOC_OUTPUT.to_string(),
            port: DEFAULT_DOC_SERVER_PORT,
            bind_address: "127.0.0.1".to_string(),
        }
    }
}

impl DocServerConfig {
    /// Generate the URL where documentation will be served.
    pub fn url(&self) -> String {
        alloc::format!("http://{}:{}", self.bind_address, self.port)
    }
}

// ---------------------------------------------------------------------------
// RustdocBuilder — orchestrates doc generation
// ---------------------------------------------------------------------------

/// Orchestrates `cargo doc` invocations for packages and workspaces.
pub struct RustdocBuilder {
    config: RustdocConfig,
    index: DocIndex,
}

impl RustdocBuilder {
    /// Create a new builder from a configuration.
    pub fn new(config: RustdocConfig) -> Self {
        Self {
            config,
            index: DocIndex::new(),
        }
    }

    /// Access the current configuration.
    pub fn config(&self) -> &RustdocConfig {
        &self.config
    }

    /// Access the documentation index built so far.
    pub fn index(&self) -> &DocIndex {
        &self.index
    }

    /// Set the documentation theme.
    pub fn configure_theme(&mut self, theme: DocTheme) {
        self.config.theme = theme;
    }

    // -- Command generation -------------------------------------------------

    /// Generate the base `cargo doc` command line for the current config.
    fn base_command(&self) -> String {
        let mut cmd = String::from("cargo doc --no-deps");

        if self.config.document_private {
            cmd.push_str(" --document-private-items");
        }

        if !self.config.features.is_empty() {
            cmd.push_str(&alloc::format!(
                " --features {}",
                self.config.features.join(",")
            ));
        }

        for cfg in &self.config.cfg_flags {
            cmd.push_str(&alloc::format!(" --cfg {}", cfg));
        }

        cmd.push_str(&alloc::format!(" --target-dir {}", self.config.output_dir));

        cmd
    }

    /// Generate the command to build documentation for a single package.
    pub fn generate_docs(&self) -> String {
        self.base_command()
    }

    /// Generate the command to build documentation for a specific package
    /// within a workspace.
    pub fn generate_package_docs(&self, package_name: &str) -> String {
        let base = self.base_command();
        alloc::format!("{} -p {}", base, package_name)
    }

    /// Generate the command to build documentation for the entire workspace.
    pub fn generate_workspace_docs(&self) -> String {
        let base = self.base_command();
        alloc::format!("{} --workspace", base)
    }

    // -- Search index -------------------------------------------------------

    /// Build the search index from a list of discovered items.
    ///
    /// In a real implementation this would parse the generated
    /// `search-index.js` or walk the source AST; here we accept a
    /// pre-collected list for testability.
    pub fn generate_search_index(&mut self, items: Vec<DocItem>) -> Result<&DocIndex, KernelError> {
        self.index = DocIndex::new();
        for item in items {
            self.index.add(item)?;
        }
        Ok(&self.index)
    }

    // -- Output path helpers ------------------------------------------------

    /// Return the HTML output root for a given crate.
    pub fn crate_doc_path(&self, crate_name: &str) -> String {
        alloc::format!("{}/doc/{}", self.config.output_dir, crate_name)
    }

    /// Return the full path to the top-level `index.html`.
    pub fn index_html_path(&self) -> String {
        alloc::format!("{}/doc/index.html", self.config.output_dir)
    }

    /// Create a [`DocServerConfig`] for serving the generated docs.
    pub fn server_config(&self, port: u16) -> DocServerConfig {
        DocServerConfig {
            doc_root: alloc::format!("{}/doc", self.config.output_dir),
            port,
            bind_address: "127.0.0.1".to_string(),
        }
    }

    /// Generate cross-reference extern flags for linking between crates
    /// in a workspace.
    pub fn cross_reference_flags(&self, crate_names: &[&str]) -> Vec<String> {
        if !self.config.cross_references {
            return Vec::new();
        }
        crate_names
            .iter()
            .map(|name| {
                alloc::format!(
                    "--extern-html-root-url {}={}/doc/{}",
                    name,
                    self.config.output_dir,
                    name
                )
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Helper: lowercase conversion without std
// ---------------------------------------------------------------------------

/// Ascii-lowercase a `&str` into a new `String`.
#[allow(dead_code)]
trait AsciiLowerExt {
    fn to_ascii_lowercase(&self) -> String;
}

impl AsciiLowerExt for str {
    fn to_ascii_lowercase(&self) -> String {
        let mut s = String::with_capacity(self.len());
        for c in self.chars() {
            if c.is_ascii_uppercase() {
                s.push((c as u8 + 32) as char);
            } else {
                s.push(c);
            }
        }
        s
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
    fn test_rustdoc_config_default() {
        let config = RustdocConfig::default();
        assert_eq!(config.output_dir, DEFAULT_DOC_OUTPUT);
        assert_eq!(config.theme, DocTheme::Default);
        assert!(config.cross_references);
        assert!(!config.document_private);
        assert!(config.features.is_empty());
    }

    #[test]
    fn test_generate_docs_command() {
        let builder = RustdocBuilder::new(RustdocConfig::default());
        let cmd = builder.generate_docs();
        assert!(cmd.contains("cargo doc --no-deps"));
        assert!(cmd.contains("--target-dir"));
        assert!(cmd.contains(DEFAULT_DOC_OUTPUT));
    }

    #[test]
    fn test_generate_package_docs() {
        let builder = RustdocBuilder::new(RustdocConfig::default());
        let cmd = builder.generate_package_docs("veridian-kernel");
        assert!(cmd.contains("-p veridian-kernel"));
        assert!(cmd.contains("cargo doc"));
    }

    #[test]
    fn test_generate_workspace_docs() {
        let builder = RustdocBuilder::new(RustdocConfig::default());
        let cmd = builder.generate_workspace_docs();
        assert!(cmd.contains("--workspace"));
    }

    #[test]
    fn test_search_index_build_and_query() {
        let mut builder = RustdocBuilder::new(RustdocConfig::default());
        let items = vec![
            DocItem::new(
                "HashMap",
                DocItemKind::Struct,
                "std::collections",
                "A hash map",
                true,
            ),
            DocItem::new(
                "hash_map",
                DocItemKind::Module,
                "std::collections",
                "Hash map module",
                true,
            ),
            DocItem::new(
                "Vec",
                DocItemKind::Struct,
                "alloc::vec",
                "A growable array",
                true,
            ),
        ];
        let index = builder.generate_search_index(items).unwrap();
        assert_eq!(index.len(), 3);

        let results = index.search_by_name("hash");
        assert_eq!(results.len(), 2);

        let results = index.search_by_path("std::collections");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_doc_index_statistics() {
        let mut index = DocIndex::new();
        index
            .add(DocItem::new(
                "foo",
                DocItemKind::Function,
                "crate",
                "desc",
                true,
            ))
            .unwrap();
        index
            .add(DocItem::new(
                "Bar",
                DocItemKind::Struct,
                "crate",
                "desc",
                true,
            ))
            .unwrap();
        index
            .add(DocItem::new(
                "Baz",
                DocItemKind::Enum,
                "crate",
                "desc",
                true,
            ))
            .unwrap();
        let stats = index.statistics();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.functions, 1);
        assert_eq!(stats.structs, 1);
        assert_eq!(stats.enums, 1);
    }

    #[test]
    fn test_configure_theme_and_flags() {
        let mut builder = RustdocBuilder::new(RustdocConfig::default());
        builder.configure_theme(DocTheme::Dark);
        assert_eq!(builder.config().theme, DocTheme::Dark);
        assert_eq!(DocTheme::Dark.flag_value(), "ayu");
        assert_eq!(DocTheme::HighContrast.flag_value(), "high-contrast");
    }

    #[test]
    fn test_doc_server_config() {
        let builder = RustdocBuilder::new(RustdocConfig::default());
        let server = builder.server_config(9090);
        assert_eq!(server.port, 9090);
        assert_eq!(server.url(), "http://127.0.0.1:9090");
        assert!(server.doc_root.contains("doc"));
    }
}
