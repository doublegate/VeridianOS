//! Web Browser Engine for VeridianOS
//!
//! A minimal web browser engine providing HTML parsing, CSS styling,
//! layout computation, pixel rendering, JavaScript execution, and
//! tabbed browsing with process isolation. All layout math uses 26.6
//! fixed-point arithmetic; JS numbers use 32.32 fixed-point (no floats).

// Phase A: Static HTML Renderer
pub mod css_parser;
pub mod dom;
pub mod html_tokenizer;
pub mod integration;
pub mod layout;
pub mod paint;
pub mod style;
pub mod tree_builder;
pub mod window;

// Phase B: DOM Interactivity
pub mod events;
pub mod flexbox;
pub mod forms;
pub mod incremental;

// Phase C: JavaScript VM with GC
pub mod dom_bindings;
pub mod js_compiler;
pub mod js_gc;
pub mod js_integration;
pub mod js_lexer;
pub mod js_parser;
pub mod js_vm;

// Phase D: Tabbed Browsing
pub mod browser_main;
pub mod tab_isolation;
pub mod tabs;
