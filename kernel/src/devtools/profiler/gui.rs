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
            for frame in sample.frames.iter().rev() {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
