//! JavaScript Integration
//!
//! Ties together the JS lexer, parser, compiler, VM, GC, DOM bindings,
//! and event system into a cohesive script engine. Handles `<script>`
//! tag processing, event loop ticking, and callback dispatch.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::{
    dom_bindings::DomApi, events::EventType, js_compiler::Compiler, js_gc::GcHeap,
    js_parser::JsParser, js_vm::JsVm,
};

// ---------------------------------------------------------------------------
// Script engine
// ---------------------------------------------------------------------------

/// The script engine integrating all JS components
#[allow(dead_code)]
pub struct ScriptEngine {
    /// JavaScript virtual machine
    pub vm: JsVm,
    /// Garbage collector heap
    pub gc: GcHeap,
    /// DOM API bridge
    pub dom_api: DomApi,
    /// Total scripts executed
    scripts_executed: usize,
    /// Total ticks processed
    ticks_processed: u64,
    /// Pending microtasks (callback IDs)
    microtasks: Vec<usize>,
    /// Last error message
    pub last_error: Option<String>,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    pub fn new() -> Self {
        Self {
            vm: JsVm::new(),
            gc: GcHeap::new(),
            dom_api: DomApi::new(),
            scripts_executed: 0,
            ticks_processed: 0,
            microtasks: Vec::new(),
            last_error: None,
        }
    }

    /// Execute a JavaScript source string.
    /// Returns Ok(()) on success, Err(message) on failure.
    pub fn execute_script(&mut self, source: &str) -> Result<(), String> {
        if source.trim().is_empty() {
            return Ok(());
        }

        let mut parser = JsParser::from_source(source);
        let root = parser.parse();

        if !parser.errors.is_empty() {
            let err = parser.errors.join("; ");
            self.last_error = Some(err.clone());
            return Err(err);
        }

        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&parser.arena, root);

        match self.vm.run_chunk(&chunk) {
            Ok(_) => {
                self.scripts_executed += 1;
                // Collect garbage if needed
                if self.gc.should_collect() {
                    self.gc.collect(&self.vm);
                }
                Ok(())
            }
            Err(e) => {
                self.last_error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Extract inline JavaScript from `<script>` tags and execute each.
    /// Returns the number of scripts successfully executed.
    pub fn process_script_tags(&mut self, html: &str) -> usize {
        let mut count = 0;
        let mut search_from = 0;

        while let Some(open_tag) = find_ci(html, "<script", search_from) {
            // Find end of open tag
            let tag_end = match html[open_tag..].find('>') {
                Some(pos) => open_tag + pos + 1,
                None => break,
            };

            // Find </script>
            let close_tag = match find_ci(html, "</script>", tag_end) {
                Some(pos) => pos,
                None => break,
            };

            let script_src = &html[tag_end..close_tag];
            if self.execute_script(script_src).is_ok() {
                count += 1;
            }

            search_from = close_tag + 9; // len("</script>")
        }

        count
    }

    /// Process a single tick of the event loop:
    /// 1. Process expired timers
    /// 2. Execute microtasks
    /// 3. Run GC if needed
    pub fn tick(&mut self) {
        self.ticks_processed += 1;

        // Process timers
        let expired = self.dom_api.timer_queue.tick();
        for callback_id in expired {
            self.invoke_callback(callback_id);
        }

        // Process microtasks
        let tasks = core::mem::take(&mut self.microtasks);
        for callback_id in tasks {
            self.invoke_callback(callback_id);
        }

        // GC check
        if self.gc.should_collect() {
            self.gc.collect(&self.vm);
        }
    }

    /// Process a DOM event: dispatch it through the event system,
    /// then invoke any JS callbacks that were triggered.
    pub fn process_event(&mut self, event_type: EventType, target_node: super::events::NodeId) {
        let mut event = super::events::Event::new(event_type, target_node);
        self.dom_api.event_dispatcher.dispatch(&mut event);

        let invoked = self.dom_api.event_dispatcher.take_invoked();
        for (callback_id, _event_type) in invoked {
            self.invoke_callback(callback_id);
        }
    }

    /// Process a click at pixel coordinates
    pub fn process_click(&mut self, x: i32, y: i32) {
        if let Some((_target, _prevented)) = self.dom_api.event_dispatcher.dispatch_click(x, y, 0) {
            let invoked = self.dom_api.event_dispatcher.take_invoked();
            for (callback_id, _) in invoked {
                self.invoke_callback(callback_id);
            }
        }
    }

    /// Invoke a JS callback by function ID.
    /// In a full implementation this would look up the function in the VM
    /// and execute it. Here we use a simplified approach.
    fn invoke_callback(&mut self, _callback_id: usize) {
        // Callbacks would be stored in a table mapping callback_id -> JS
        // function. For now this is a stub that will be connected when
        // the JS VM has a function call-by-id mechanism.
    }

    /// Schedule a microtask
    pub fn queue_microtask(&mut self, callback_id: usize) {
        self.microtasks.push(callback_id);
    }

    /// Get the console output from both JS VM and DOM API
    pub fn console_output(&self) -> Vec<String> {
        let mut output = self.vm.output.clone();
        output.extend(self.dom_api.console_output.iter().cloned());
        output
    }

    /// Clear console output
    pub fn clear_console(&mut self) {
        self.vm.output.clear();
        self.dom_api.console_output.clear();
    }

    /// Number of scripts executed
    pub fn scripts_executed(&self) -> usize {
        self.scripts_executed
    }

    /// Number of ticks processed
    pub fn ticks_processed(&self) -> u64 {
        self.ticks_processed
    }

    /// Set a global variable in the JS VM
    pub fn set_global(&mut self, name: &str, value: super::js_vm::JsValue) {
        self.vm.globals.insert(name.to_string(), value);
    }

    /// Get a global variable from the JS VM
    pub fn get_global(&self, name: &str) -> Option<&super::js_vm::JsValue> {
        self.vm.globals.get(name)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Case-insensitive find in a string
fn find_ci(haystack: &str, needle: &str, start: usize) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || start + n.len() > h.len() {
        return None;
    }
    'outer: for i in start..=(h.len() - n.len()) {
        for j in 0..n.len() {
            if !h[i + j].eq_ignore_ascii_case(&n[j]) {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{js_lexer::js_int, js_vm::JsValue};

    #[test]
    fn test_engine_new() {
        let engine = ScriptEngine::new();
        assert_eq!(engine.scripts_executed(), 0);
        assert_eq!(engine.ticks_processed(), 0);
    }

    #[test]
    fn test_execute_empty() {
        let mut engine = ScriptEngine::new();
        assert!(engine.execute_script("").is_ok());
        assert!(engine.execute_script("   ").is_ok());
    }

    #[test]
    fn test_execute_simple() {
        let mut engine = ScriptEngine::new();
        assert!(engine.execute_script("let x = 42;").is_ok());
        assert_eq!(engine.scripts_executed(), 1);
        let val = engine.get_global("x");
        assert!(matches!(val, Some(JsValue::Number(n)) if *n == js_int(42)));
    }

    #[test]
    fn test_execute_multiple() {
        let mut engine = ScriptEngine::new();
        engine.execute_script("let a = 1;").unwrap();
        engine.execute_script("let b = 2;").unwrap();
        assert_eq!(engine.scripts_executed(), 2);
    }

    #[test]
    fn test_execute_error() {
        let mut engine = ScriptEngine::new();
        // Infinite loop should hit execution limit
        let result = engine.execute_script("while(true){}");
        assert!(result.is_err());
        assert!(engine.last_error.is_some());
    }

    #[test]
    fn test_set_get_global() {
        let mut engine = ScriptEngine::new();
        engine.set_global("myVar", JsValue::String("hello".to_string()));
        let val = engine.get_global("myVar");
        assert!(matches!(val, Some(JsValue::String(s)) if s == "hello"));
    }

    #[test]
    fn test_process_script_tags() {
        let mut engine = ScriptEngine::new();
        let html = "<html><body><script>let x = 1;</script><p>Hello</p><script>let y = \
                    2;</script></body></html>";
        let count = engine.process_script_tags(html);
        assert_eq!(count, 2);
        assert!(engine.get_global("x").is_some());
        assert!(engine.get_global("y").is_some());
    }

    #[test]
    fn test_process_script_tags_case_insensitive() {
        let mut engine = ScriptEngine::new();
        let html = "<SCRIPT>let z = 3;</SCRIPT>";
        let count = engine.process_script_tags(html);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_process_script_tags_empty() {
        let mut engine = ScriptEngine::new();
        let html = "<p>No scripts here</p>";
        let count = engine.process_script_tags(html);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_tick() {
        let mut engine = ScriptEngine::new();
        engine.tick();
        assert_eq!(engine.ticks_processed(), 1);
        engine.tick();
        assert_eq!(engine.ticks_processed(), 2);
    }

    #[test]
    fn test_timer_via_engine() {
        let mut engine = ScriptEngine::new();
        engine.dom_api.timer_queue.set_timeout(42, 3);
        engine.tick(); // 1
        engine.tick(); // 2
        engine.tick(); // 3 -- timer fires
        assert_eq!(engine.ticks_processed(), 3);
    }

    #[test]
    fn test_console_output() {
        let mut engine = ScriptEngine::new();
        engine.dom_api.console_log("from dom");
        let output = engine.console_output();
        assert!(output.contains(&"from dom".to_string()));
    }

    #[test]
    fn test_clear_console() {
        let mut engine = ScriptEngine::new();
        engine.dom_api.console_log("test");
        engine.clear_console();
        assert!(engine.console_output().is_empty());
    }

    #[test]
    fn test_queue_microtask() {
        let mut engine = ScriptEngine::new();
        engine.queue_microtask(99);
        assert_eq!(engine.microtasks.len(), 1);
        engine.tick(); // processes microtasks
        assert!(engine.microtasks.is_empty());
    }

    #[test]
    fn test_process_event() {
        let mut engine = ScriptEngine::new();
        let div = engine.dom_api.create_element("div");
        engine.dom_api.add_event_listener(div, EventType::Click, 77);
        engine.process_event(EventType::Click, div);
        // Callback was invoked (stub), no crash
    }

    #[test]
    fn test_process_click() {
        let mut engine = ScriptEngine::new();
        let div = engine.dom_api.create_element("div");
        engine
            .dom_api
            .event_dispatcher
            .add_hit_box(super::super::events::HitRect::new(0, 0, 100, 100, div));
        engine.dom_api.add_event_listener(div, EventType::Click, 50);
        engine.process_click(50, 50);
        // No crash, event dispatched
    }

    #[test]
    fn test_find_ci() {
        assert_eq!(find_ci("Hello World", "hello", 0), Some(0));
        assert_eq!(find_ci("Hello World", "WORLD", 0), Some(6));
        assert_eq!(find_ci("Hello World", "xyz", 0), None);
        assert_eq!(find_ci("aabb", "bb", 0), Some(2));
    }

    #[test]
    fn test_find_ci_empty() {
        assert_eq!(find_ci("test", "", 0), None);
    }

    #[test]
    fn test_engine_default() {
        let engine = ScriptEngine::default();
        assert_eq!(engine.scripts_executed(), 0);
    }

    #[test]
    fn test_script_with_function() {
        let mut engine = ScriptEngine::new();
        let result = engine.execute_script("function add(a,b) { return a+b; } let r = add(3,4);");
        assert!(result.is_ok());
        let val = engine.get_global("r");
        assert!(matches!(val, Some(JsValue::Number(n)) if *n == js_int(7)));
    }

    #[test]
    fn test_script_variables_persist() {
        let mut engine = ScriptEngine::new();
        engine.execute_script("let counter = 0;").unwrap();
        engine.execute_script("counter = counter + 1;").unwrap();
        let val = engine.get_global("counter");
        assert!(matches!(val, Some(JsValue::Number(n)) if *n == js_int(1)));
    }

    #[test]
    fn test_dom_create_via_script() {
        let mut engine = ScriptEngine::new();
        // Direct DOM API usage (script bridge not fully wired)
        let div = engine.dom_api.create_element("div");
        engine.dom_api.set_attribute(div, "id", "test");
        assert_eq!(engine.dom_api.get_element_by_id("test"), Some(div));
    }

    #[test]
    fn test_nested_script_tags() {
        let mut engine = ScriptEngine::new();
        let html = "<script>let a = 1;</script><script>let b = a + 1;</script>";
        let count = engine.process_script_tags(html);
        assert_eq!(count, 2);
        // b depends on a from first script
        let val = engine.get_global("b");
        assert!(matches!(val, Some(JsValue::Number(n)) if *n == js_int(2)));
    }

    #[test]
    fn test_gc_integration() {
        let mut engine = ScriptEngine::new();
        // Allocate objects and trigger GC
        for i in 0..100 {
            engine.gc.allocate(super::super::js_vm::JsObject::new());
        }
        engine.gc.collect(&engine.vm);
        // All unreferenced objects should be collected
        assert_eq!(engine.gc.arena.live_count(), 0);
    }

    #[test]
    fn test_multiple_ticks_with_timer() {
        let mut engine = ScriptEngine::new();
        engine.dom_api.timer_queue.set_interval(10, 2);
        for _ in 0..6 {
            engine.tick();
        }
        assert_eq!(engine.ticks_processed(), 6);
    }
}
