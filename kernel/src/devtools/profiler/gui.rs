//! Profiling GUI
//!
//! Flame graph visualization, CPU/memory timeline, and call tree view.
//! Reads from the perf::trace ring buffer system.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// Flame graph frame
#[derive(Debug, Clone)]
pub struct FlameFrame {
    pub name: String,
    pub samples: u64,
    pub children: Vec<FlameFrame>,
}

impl FlameFrame {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            samples: 0,
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, child: FlameFrame) {
        self.children.push(child);
    }

    /// Total samples including children
    pub fn total_samples(&self) -> u64 {
        self.samples + self.children.iter().map(|c| c.total_samples()).sum::<u64>()
    }

    /// Find or create a child with the given name
    pub fn get_or_create_child(&mut self, name: &str) -> &mut FlameFrame {
        let idx = self.children.iter().position(|c| c.name == name);
        if let Some(i) = idx {
            &mut self.children[i]
        } else {
            self.children.push(FlameFrame::new(name));
            self.children.last_mut().unwrap()
        }
    }
}

/// Call stack sample
#[derive(Debug, Clone)]
pub struct StackSample {
    pub timestamp: u64,
    pub frames: Vec<String>,
    pub cpu: u32,
}

/// CPU timeline data point
#[derive(Debug, Clone, Copy)]
pub struct CpuDataPoint {
    pub timestamp: u64,
    pub usage_percent: u8,
    pub cpu_id: u32,
}

/// Memory timeline data point
#[derive(Debug, Clone, Copy)]
pub struct MemDataPoint {
    pub timestamp: u64,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Profiler session
pub struct ProfilerSession {
    pub name: String,
    pub samples: Vec<StackSample>,
    pub cpu_timeline: Vec<CpuDataPoint>,
    pub mem_timeline: Vec<MemDataPoint>,
    pub start_time: u64,
    pub end_time: u64,
}

impl ProfilerSession {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            samples: Vec::new(),
            cpu_timeline: Vec::new(),
            mem_timeline: Vec::new(),
            start_time: 0,
            end_time: 0,
        }
    }

    pub fn add_sample(&mut self, sample: StackSample) {
        if self.samples.is_empty() {
            self.start_time = sample.timestamp;
        }
        self.end_time = sample.timestamp;
        self.samples.push(sample);
    }

    pub fn add_cpu_point(&mut self, point: CpuDataPoint) {
        self.cpu_timeline.push(point);
    }

    pub fn add_mem_point(&mut self, point: MemDataPoint) {
        self.mem_timeline.push(point);
    }

    /// Build flame graph from samples
    pub fn build_flame_graph(&self) -> FlameFrame {
        let mut root = FlameFrame::new("all");

        for sample in &self.samples {
            let mut current = &mut root;
            for frame in &sample.frames {
                current = current.get_or_create_child(frame);
                current.samples += 1;
            }
        }

        root
    }

    /// Duration in timestamp units
    pub fn duration(&self) -> u64 {
        self.end_time.saturating_sub(self.start_time)
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Average CPU usage
    pub fn avg_cpu_usage(&self) -> u8 {
        if self.cpu_timeline.is_empty() {
            return 0;
        }
        let sum: u64 = self
            .cpu_timeline
            .iter()
            .map(|p| p.usage_percent as u64)
            .sum();
        (sum / self.cpu_timeline.len() as u64) as u8
    }

    /// Peak memory usage
    pub fn peak_memory(&self) -> u64 {
        self.mem_timeline
            .iter()
            .map(|p| p.used_bytes)
            .max()
            .unwrap_or(0)
    }
}

/// Profiler GUI renderer (renders to a pixel buffer)
pub struct ProfilerGui {
    pub width: u32,
    pub height: u32,
    pub view: ProfilerView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilerView {
    FlameGraph,
    CpuTimeline,
    MemoryTimeline,
    CallTree,
}

impl ProfilerGui {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            view: ProfilerView::FlameGraph,
        }
    }

    pub fn switch_view(&mut self, view: ProfilerView) {
        self.view = view;
    }

    /// Render the flame graph to a color buffer
    pub fn render_flame_graph(&self, root: &FlameFrame, buf: &mut [u32]) {
        let total = root.total_samples();
        if total == 0 || buf.len() < (self.width * self.height) as usize {
            return;
        }

        // Clear buffer
        for pixel in buf.iter_mut() {
            *pixel = 0xFF1A1A2E; // Dark background
        }

        // Render root frame
        self.render_flame_frame(root, buf, 0, 0, self.width, total, 0);
    }

    fn render_flame_frame(
        &self,
        frame: &FlameFrame,
        buf: &mut [u32],
        x: u32,
        _y: u32,
        width: u32,
        total_samples: u64,
        depth: u32,
    ) {
        if width < 2 || depth > 50 {
            return;
        }

        let bar_height = 20u32;
        let bar_y = self.height.saturating_sub((depth + 1) * bar_height);

        // Color based on depth (warm gradient)
        let color = match depth % 6 {
            0 => 0xFFE74C3C, // Red
            1 => 0xFFE67E22, // Orange
            2 => 0xFFF39C12, // Yellow
            3 => 0xFF2ECC71, // Green
            4 => 0xFF3498DB, // Blue
            _ => 0xFF9B59B6, // Purple
        };

        // Draw bar
        for dy in 0..bar_height.saturating_sub(1) {
            let py = bar_y + dy;
            if py >= self.height {
                continue;
            }
            for dx in 1..width.saturating_sub(1) {
                let px = x + dx;
                if px < self.width {
                    buf[(py * self.width + px) as usize] = color;
                }
            }
        }

        // Render children
        let mut child_x = x;
        for child in &frame.children {
            let child_total = child.total_samples();
            let child_width = ((child_total * width as u64) / total_samples.max(1)) as u32;
            if child_width > 0 {
                self.render_flame_frame(
                    child,
                    buf,
                    child_x,
                    _y,
                    child_width,
                    total_samples,
                    depth + 1,
                );
                child_x += child_width;
            }
        }
    }
}

/// Node in an aggregated call tree.
#[derive(Debug, Clone)]
pub struct CallTreeNode {
    pub function_name: String,
    /// Samples where this function is the leaf (self time).
    pub self_time: u64,
    /// Total samples including all descendants.
    pub total_time: u64,
    pub children: Vec<CallTreeNode>,
    pub call_count: u64,
}

impl CallTreeNode {
    pub fn new(name: &str) -> Self {
        Self {
            function_name: name.to_string(),
            self_time: 0,
            total_time: 0,
            children: Vec::new(),
            call_count: 0,
        }
    }

    /// Find a child by name, returning its index.
    fn find_child(&self, name: &str) -> Option<usize> {
        self.children.iter().position(|c| c.function_name == name)
    }

    /// Get or create a child node with the given name.
    fn get_or_create_child(&mut self, name: &str) -> &mut CallTreeNode {
        let idx = self.find_child(name);
        if let Some(i) = idx {
            &mut self.children[i]
        } else {
            self.children.push(CallTreeNode::new(name));
            self.children.last_mut().unwrap()
        }
    }
}

/// Flattened call tree entry for rendering.
#[derive(Debug, Clone)]
pub struct FlatCallEntry {
    pub function_name: String,
    pub self_time: u64,
    pub total_time: u64,
    pub call_count: u64,
    pub depth: u32,
}

/// Aggregated call tree built from stack samples.
pub struct CallTree {
    pub roots: Vec<CallTreeNode>,
    pub total_samples: u64,
}

impl Default for CallTree {
    fn default() -> Self {
        Self::new()
    }
}

impl CallTree {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            total_samples: 0,
        }
    }

    /// Build a call tree from a slice of stack samples.
    ///
    /// Each sample's frames are walked bottom-up (callers first). The leaf
    /// frame receives self-time credit.
    pub fn build_from_stacks(samples: &[StackSample]) -> Self {
        let mut tree = CallTree::new();
        tree.total_samples = samples.len() as u64;

        for sample in samples {
            if sample.frames.is_empty() {
                continue;
            }

            // frames[0] is the bottom (caller), last is the leaf
            let bottom_name = &sample.frames[0];

            // Find or create root
            let root_idx = tree
                .roots
                .iter()
                .position(|r| r.function_name == *bottom_name);
            let root = if let Some(i) = root_idx {
                &mut tree.roots[i]
            } else {
                tree.roots.push(CallTreeNode::new(bottom_name));
                tree.roots.last_mut().unwrap()
            };

            root.call_count += 1;
            root.total_time += 1;

            // Walk down the stack
            let mut current = root as *mut CallTreeNode;
            for (i, frame_name) in sample.frames.iter().enumerate().skip(1) {
                // SAFETY: We only hold one mutable reference at a time,
                // descending through a tree we own.
                let node = unsafe { &mut *current };
                let child = node.get_or_create_child(frame_name);
                child.call_count += 1;
                child.total_time += 1;

                // Leaf frame gets self-time
                if i == sample.frames.len() - 1 {
                    child.self_time += 1;
                }

                current = child as *mut CallTreeNode;
            }

            // If single-frame sample, root is also the leaf
            if sample.frames.len() == 1 {
                root.self_time += 1;
            }
        }

        tree
    }

    /// Flatten the tree into a list with depth information for rendering.
    pub fn flatten(&self) -> Vec<FlatCallEntry> {
        let mut result = Vec::new();
        for root in &self.roots {
            Self::flatten_node(root, 0, &mut result);
        }
        result
    }

    fn flatten_node(node: &CallTreeNode, depth: u32, result: &mut Vec<FlatCallEntry>) {
        result.push(FlatCallEntry {
            function_name: node.function_name.clone(),
            self_time: node.self_time,
            total_time: node.total_time,
            call_count: node.call_count,
            depth,
        });
        for child in &node.children {
            Self::flatten_node(child, depth + 1, result);
        }
    }

    /// Find the top-N hottest paths by total_time across all root entries.
    pub fn hottest_paths(&self, top_n: usize) -> Vec<FlatCallEntry> {
        let mut sorted = self.flatten();
        // Sort descending by total_time
        sorted.sort_by(|a, b| b.total_time.cmp(&a.total_time));
        sorted.truncate(top_n);
        sorted
    }
}

/// Profiler data export utilities.
pub struct ProfilerExport;

impl ProfilerExport {
    /// Generate an SVG-like text representation of a flame graph.
    ///
    /// Produces a simplified folded-stack format suitable for text display:
    /// each line is `stack;path samples\n`.
    pub fn export_flamegraph_svg(session: &ProfilerSession) -> String {
        let mut out = String::new();
        // Header
        out.push_str("<!-- VeridianOS Profiler Flame Graph -->\n");
        out.push_str("<!-- Format: stack;path sample_count -->\n");

        for sample in &session.samples {
            if sample.frames.is_empty() {
                continue;
            }
            // Build folded stack line
            for (i, frame) in sample.frames.iter().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                out.push_str(frame);
            }
            out.push_str(" 1\n");
        }
        out
    }

    /// Generate a text summary of a profiling session.
    pub fn export_summary(session: &ProfilerSession) -> String {
        let mut out = String::from("=== Profiler Summary ===\n");
        out.push_str("Session: ");
        out.push_str(&session.name);
        out.push('\n');
        out.push_str("Samples: ");
        push_u64_str(&mut out, session.sample_count() as u64);
        out.push('\n');
        out.push_str("Duration: ");
        push_u64_str(&mut out, session.duration());
        out.push_str(" ticks\n");
        out.push_str("Avg CPU: ");
        push_u64_str(&mut out, session.avg_cpu_usage() as u64);
        out.push_str("%\n");
        out.push_str("Peak Mem: ");
        push_u64_str(&mut out, session.peak_memory());
        out.push_str(" bytes\n");

        // Top functions from call tree
        let tree = CallTree::build_from_stacks(&session.samples);
        let hot = tree.hottest_paths(5);
        if !hot.is_empty() {
            out.push_str("\nTop functions by total time:\n");
            for entry in &hot {
                out.push_str("  ");
                out.push_str(&entry.function_name);
                out.push_str(" (total=");
                push_u64_str(&mut out, entry.total_time);
                out.push_str(", self=");
                push_u64_str(&mut out, entry.self_time);
                out.push_str(")\n");
            }
        }

        out
    }
}

impl ProfilerGui {
    /// Render a call tree view into a text buffer (indented with percentages).
    pub fn render_call_tree(&self, tree: &CallTree) -> String {
        let mut out = String::from("Call Tree View\n");
        out.push_str("==============\n");
        let total = tree.total_samples.max(1);
        let flat = tree.flatten();
        for entry in &flat {
            // Indent
            for _ in 0..entry.depth {
                out.push_str("  ");
            }
            out.push_str(&entry.function_name);
            out.push_str(" [");
            // Percentage of total: (total_time * 100) / total_samples
            let pct = (entry.total_time * 100) / total;
            push_u64_str(&mut out, pct);
            out.push_str("%, self=");
            let self_pct = (entry.self_time * 100) / total;
            push_u64_str(&mut out, self_pct);
            out.push_str("%, calls=");
            push_u64_str(&mut out, entry.call_count);
            out.push_str("]\n");
        }
        out
    }

    /// Render a hotspot list: top functions sorted by self time.
    pub fn render_hotspots(&self, tree: &CallTree, top_n: usize) -> String {
        let mut out = String::from("Hotspot Analysis\n");
        out.push_str("================\n");
        let total = tree.total_samples.max(1);

        // Collect all nodes, sort by self_time descending
        let flat = tree.flatten();
        let mut by_self: Vec<&FlatCallEntry> = flat.iter().filter(|e| e.self_time > 0).collect();
        by_self.sort_by(|a, b| b.self_time.cmp(&a.self_time));
        by_self.truncate(top_n);

        for (i, entry) in by_self.iter().enumerate() {
            push_u64_str(&mut out, (i + 1) as u64);
            out.push_str(". ");
            out.push_str(&entry.function_name);
            out.push_str("  self=");
            let pct = (entry.self_time * 100) / total;
            push_u64_str(&mut out, pct);
            out.push_str("% (");
            push_u64_str(&mut out, entry.self_time);
            out.push_str(" samples)\n");
        }
        out
    }
}

/// Append a u64 as decimal text (no std formatting needed).
fn push_u64_str(out: &mut String, mut val: u64) {
    if val == 0 {
        out.push('0');
        return;
    }
    let start = out.len();
    while val > 0 {
        let digit = (val % 10) as u8 + b'0';
        out.push(digit as char);
        val /= 10;
    }
    let bytes = unsafe { out.as_bytes_mut() };
    bytes[start..].reverse();
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
    fn test_flame_frame_new() {
        let frame = FlameFrame::new("main");
        assert_eq!(frame.name, "main");
        assert_eq!(frame.samples, 0);
        assert!(frame.children.is_empty());
    }

    #[test]
    fn test_flame_frame_total_samples() {
        let mut root = FlameFrame::new("root");
        root.samples = 5;
        let mut child = FlameFrame::new("child");
        child.samples = 3;
        root.add_child(child);
        assert_eq!(root.total_samples(), 8);
    }

    #[test]
    fn test_profiler_session() {
        let mut session = ProfilerSession::new("test");
        session.add_sample(StackSample {
            timestamp: 100,
            frames: vec!["main".to_string(), "foo".to_string()],
            cpu: 0,
        });
        session.add_sample(StackSample {
            timestamp: 200,
            frames: vec!["main".to_string(), "bar".to_string()],
            cpu: 0,
        });

        assert_eq!(session.sample_count(), 2);
        assert_eq!(session.duration(), 100);
    }

    #[test]
    fn test_build_flame_graph() {
        let mut session = ProfilerSession::new("test");
        session.add_sample(StackSample {
            timestamp: 0,
            frames: vec!["main".to_string(), "foo".to_string()],
            cpu: 0,
        });
        session.add_sample(StackSample {
            timestamp: 1,
            frames: vec!["main".to_string(), "foo".to_string()],
            cpu: 0,
        });
        session.add_sample(StackSample {
            timestamp: 2,
            frames: vec!["main".to_string(), "bar".to_string()],
            cpu: 0,
        });

        let flame = session.build_flame_graph();
        assert_eq!(flame.children.len(), 1); // "main"
        let main_frame = &flame.children[0];
        assert_eq!(main_frame.name, "main");
        assert_eq!(main_frame.children.len(), 2); // "foo" and "bar"
    }

    #[test]
    fn test_cpu_timeline() {
        let mut session = ProfilerSession::new("test");
        session.add_cpu_point(CpuDataPoint {
            timestamp: 0,
            usage_percent: 50,
            cpu_id: 0,
        });
        session.add_cpu_point(CpuDataPoint {
            timestamp: 1,
            usage_percent: 80,
            cpu_id: 0,
        });
        assert_eq!(session.avg_cpu_usage(), 65);
    }

    #[test]
    fn test_mem_timeline() {
        let mut session = ProfilerSession::new("test");
        session.add_mem_point(MemDataPoint {
            timestamp: 0,
            used_bytes: 1000,
            total_bytes: 4096,
        });
        session.add_mem_point(MemDataPoint {
            timestamp: 1,
            used_bytes: 3000,
            total_bytes: 4096,
        });
        assert_eq!(session.peak_memory(), 3000);
    }

    #[test]
    fn test_profiler_gui_new() {
        let gui = ProfilerGui::new(800, 600);
        assert_eq!(gui.view, ProfilerView::FlameGraph);
    }

    #[test]
    fn test_profiler_gui_switch_view() {
        let mut gui = ProfilerGui::new(800, 600);
        gui.switch_view(ProfilerView::CpuTimeline);
        assert_eq!(gui.view, ProfilerView::CpuTimeline);
    }

    #[test]
    fn test_profiler_view_eq() {
        assert_eq!(ProfilerView::FlameGraph, ProfilerView::FlameGraph);
        assert_ne!(ProfilerView::FlameGraph, ProfilerView::CallTree);
    }

    #[test]
    fn test_empty_session_stats() {
        let session = ProfilerSession::new("empty");
        assert_eq!(session.avg_cpu_usage(), 0);
        assert_eq!(session.peak_memory(), 0);
        assert_eq!(session.duration(), 0);
    }

    #[test]
    fn test_call_tree_build() {
        let samples = vec![
            StackSample {
                timestamp: 0,
                frames: vec!["main".to_string(), "foo".to_string(), "bar".to_string()],
                cpu: 0,
            },
            StackSample {
                timestamp: 1,
                frames: vec!["main".to_string(), "foo".to_string()],
                cpu: 0,
            },
        ];
        let tree = CallTree::build_from_stacks(&samples);
        assert_eq!(tree.total_samples, 2);
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.roots[0].function_name, "main");
        assert_eq!(tree.roots[0].call_count, 2);
    }

    #[test]
    fn test_call_tree_flatten() {
        let samples = vec![StackSample {
            timestamp: 0,
            frames: vec!["main".to_string(), "compute".to_string()],
            cpu: 0,
        }];
        let tree = CallTree::build_from_stacks(&samples);
        let flat = tree.flatten();
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].depth, 0);
        assert_eq!(flat[1].depth, 1);
        assert_eq!(flat[1].function_name, "compute");
    }

    #[test]
    fn test_call_tree_hottest_paths() {
        let samples = vec![
            StackSample {
                timestamp: 0,
                frames: vec!["main".to_string(), "hot".to_string()],
                cpu: 0,
            },
            StackSample {
                timestamp: 1,
                frames: vec!["main".to_string(), "hot".to_string()],
                cpu: 0,
            },
            StackSample {
                timestamp: 2,
                frames: vec!["main".to_string(), "cold".to_string()],
                cpu: 0,
            },
        ];
        let tree = CallTree::build_from_stacks(&samples);
        let hot = tree.hottest_paths(2);
        assert!(!hot.is_empty());
        // "main" has highest total_time (3), then "hot" (2)
        assert_eq!(hot[0].function_name, "main");
    }

    #[test]
    fn test_export_flamegraph_svg() {
        let mut session = ProfilerSession::new("test");
        session.add_sample(StackSample {
            timestamp: 0,
            frames: vec!["main".to_string(), "foo".to_string()],
            cpu: 0,
        });
        let svg = ProfilerExport::export_flamegraph_svg(&session);
        assert!(svg.contains("main;foo 1"));
        assert!(svg.contains("VeridianOS"));
    }

    #[test]
    fn test_export_summary() {
        let mut session = ProfilerSession::new("bench");
        session.add_sample(StackSample {
            timestamp: 100,
            frames: vec!["main".to_string()],
            cpu: 0,
        });
        session.add_sample(StackSample {
            timestamp: 200,
            frames: vec!["main".to_string()],
            cpu: 0,
        });
        let summary = ProfilerExport::export_summary(&session);
        assert!(summary.contains("Session: bench"));
        assert!(summary.contains("Samples: 2"));
        assert!(summary.contains("Duration: 100"));
    }

    #[test]
    fn test_render_call_tree_view() {
        let samples = vec![
            StackSample {
                timestamp: 0,
                frames: vec!["main".to_string(), "work".to_string()],
                cpu: 0,
            },
            StackSample {
                timestamp: 1,
                frames: vec!["main".to_string(), "work".to_string()],
                cpu: 0,
            },
        ];
        let tree = CallTree::build_from_stacks(&samples);
        let gui = ProfilerGui::new(800, 600);
        let text = gui.render_call_tree(&tree);
        assert!(text.contains("main"));
        assert!(text.contains("work"));
        assert!(text.contains("100%")); // main has 100% total
    }
}
