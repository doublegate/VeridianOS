//! Web Browser Engine for VeridianOS
//!
//! A minimal web browser engine providing HTML parsing, CSS styling,
//! layout computation, pixel rendering, JavaScript execution, and
//! tabbed browsing with process isolation. All layout math uses 26.6
//! fixed-point arithmetic; JS numbers use 32.32 fixed-point (no floats).

// Phase A: Static HTML Renderer
#[allow(dead_code)]
pub mod css_parser;
#[allow(dead_code)]
pub mod dom;
#[allow(dead_code)]
pub mod html_tokenizer;
#[allow(dead_code)]
pub mod integration;
#[allow(dead_code)]
pub mod layout;
#[allow(dead_code)]
pub mod paint;
#[allow(dead_code)]
pub mod style;
#[allow(dead_code)]
pub mod tree_builder;
#[allow(dead_code)]
pub mod window;

// Phase B: DOM Interactivity
#[allow(dead_code)]
pub mod events;
#[allow(dead_code)]
pub mod flexbox;
#[allow(dead_code)]
pub mod forms;
#[allow(dead_code)]
pub mod incremental;

// Phase C: JavaScript VM with GC
#[allow(dead_code)]
pub mod dom_bindings;
#[allow(dead_code)]
pub mod js_compiler;
#[allow(dead_code)]
pub mod js_gc;
#[allow(dead_code)]
pub mod js_integration;
#[allow(dead_code)]
pub mod js_lexer;
#[allow(dead_code)]
pub mod js_parser;
#[allow(dead_code)]
pub mod js_vm;

// Phase D: Tabbed Browsing
#[allow(dead_code)]
pub mod browser_main;
#[allow(dead_code)]
pub mod tab_isolation;
#[allow(dead_code)]
pub mod tabs;
