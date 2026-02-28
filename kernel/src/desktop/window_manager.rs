//! Window Manager with Event Loop
//!
//! Manages windows, input events, and coordinates desktop applications.
//! Provides window placement heuristics, snap/tile support, and virtual
//! workspaces.

use alloc::{collections::BTreeMap, vec::Vec};

use spin::RwLock;

use crate::{error::KernelError, sync::once_lock::GlobalState};

/// Window ID type
pub type WindowId = u32;

/// Workspace identifier
pub type WorkspaceId = u8;

/// Maximum number of workspaces
pub const MAX_WORKSPACES: usize = 4;

/// Window state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
    Hidden,
}

/// Window placement heuristic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementHeuristic {
    /// Cascade from top-left corner
    Cascade,
    /// Center on screen
    Center,
    /// Smart placement avoiding overlap
    Smart,
    /// User-specified position
    Manual { x: i32, y: i32 },
}

/// Snap zone for window tiling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Maximize,
}

/// Tile layout mode for arranging all visible windows
#[derive(Debug, Clone, Copy)]
pub enum TileLayout {
    /// Equal horizontal split (side by side columns)
    EqualColumns,
    /// Master-stack: largest window on left (60%), remainder stacked right
    /// (40%)
    MasterStack,
    /// Grid arrangement (auto rows x cols)
    Grid,
}

/// Window structure
#[derive(Debug, Clone)]
pub struct Window {
    pub id: WindowId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: [u8; 64],
    pub title_len: usize,
    pub state: WindowState,
    pub visible: bool,
    pub focused: bool,
    pub owner_pid: u64,
    /// Window opacity (0 = transparent, 255 = fully opaque)
    pub opacity: u8,
    /// Saved geometry before snap/maximize (x, y, w, h)
    pub saved_geometry: Option<(i32, i32, u32, u32)>,
    /// Current snap zone
    pub snap_zone: SnapZone,
    /// Workspace this window belongs to
    pub workspace: WorkspaceId,
}

impl Window {
    /// Create a new window
    pub fn new(id: WindowId, x: i32, y: i32, width: u32, height: u32, owner_pid: u64) -> Self {
        Self {
            id,
            x,
            y,
            width,
            height,
            title: [0; 64],
            title_len: 0,
            state: WindowState::Normal,
            visible: true,
            focused: false,
            owner_pid,
            opacity: 255,
            saved_geometry: None,
            snap_zone: SnapZone::None,
            workspace: 0,
        }
    }

    /// Set window title
    pub fn set_title(&mut self, title: &str) {
        let bytes = title.as_bytes();
        let len = bytes.len().min(64);
        self.title[..len].copy_from_slice(&bytes[..len]);
        self.title_len = len;
    }

    /// Get window title as string slice
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }
}

/// Virtual workspace containing a set of windows
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: [u8; 32],
    pub name_len: usize,
    pub windows: Vec<WindowId>,
}

impl Workspace {
    /// Create a new workspace with a numeric name
    fn new(id: WorkspaceId) -> Self {
        let mut name = [0u8; 32];
        let digit = b'1' + id;
        name[0] = digit;
        Self {
            id,
            name,
            name_len: 1,
            windows: Vec::new(),
        }
    }

    /// Get workspace name as string slice
    #[allow(dead_code)]
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

/// Input event types
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    KeyPress {
        scancode: u8,
        character: char,
    },
    KeyRelease {
        scancode: u8,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u8,
        pressed: bool,
        x: i32,
        y: i32,
    },
    MouseScroll {
        delta_x: i16,
        delta_y: i16,
    },
}

/// Window event
#[derive(Debug, Clone)]
pub struct WindowEvent {
    pub window_id: WindowId,
    pub event: InputEvent,
}

/// Window Manager
pub struct WindowManager {
    /// All windows indexed by ID
    windows: RwLock<BTreeMap<WindowId, Window>>,

    /// Window Z-order (bottom to top)
    z_order: RwLock<Vec<WindowId>>,

    /// Currently focused window
    focused_window: RwLock<Option<WindowId>>,

    /// Event queue
    event_queue: RwLock<Vec<WindowEvent>>,

    /// Next window ID
    next_window_id: RwLock<WindowId>,

    /// Mouse cursor position
    mouse_x: RwLock<i32>,
    mouse_y: RwLock<i32>,

    // --- WM-1: Placement heuristics ---
    /// Current placement heuristic for new windows
    placement_heuristic: RwLock<PlacementHeuristic>,

    /// Next cascade offset position (x, y)
    cascade_offset: RwLock<(i32, i32)>,

    /// Screen dimensions
    screen_width: RwLock<u32>,
    screen_height: RwLock<u32>,

    // --- WM-4: Virtual workspaces ---
    /// Virtual workspaces
    workspaces: RwLock<Vec<Workspace>>,

    /// Currently active workspace
    active_workspace: RwLock<WorkspaceId>,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new() -> Self {
        let mut workspaces = Vec::with_capacity(MAX_WORKSPACES);
        for i in 0..MAX_WORKSPACES {
            workspaces.push(Workspace::new(i as WorkspaceId));
        }

        Self {
            windows: RwLock::new(BTreeMap::new()),
            z_order: RwLock::new(Vec::new()),
            focused_window: RwLock::new(None),
            event_queue: RwLock::new(Vec::new()),
            next_window_id: RwLock::new(1),
            mouse_x: RwLock::new(0),
            mouse_y: RwLock::new(0),
            placement_heuristic: RwLock::new(PlacementHeuristic::Cascade),
            cascade_offset: RwLock::new((32, 32)),
            screen_width: RwLock::new(1280),
            screen_height: RwLock::new(800),
            workspaces: RwLock::new(workspaces),
            active_workspace: RwLock::new(0),
        }
    }

    /// Create a new window
    pub fn create_window(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        owner_pid: u64,
    ) -> Result<WindowId, KernelError> {
        let id = {
            let mut next_id = self.next_window_id.write();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let window = Window::new(id, x, y, width, height, owner_pid);

        self.windows.write().insert(id, window);
        self.z_order.write().push(id);

        // Add to active workspace
        let active = *self.active_workspace.read();
        {
            let mut workspaces = self.workspaces.write();
            if let Some(ws) = workspaces.get_mut(active as usize) {
                ws.windows.push(id);
            }
        }

        println!("[WM] Created window {} for PID {}", id, owner_pid);

        Ok(id)
    }

    /// Destroy a window
    pub fn destroy_window(&self, window_id: WindowId) -> Result<(), KernelError> {
        self.windows.write().remove(&window_id);
        self.z_order.write().retain(|&id| id != window_id);

        // Remove from all workspaces
        {
            let mut workspaces = self.workspaces.write();
            for ws in workspaces.iter_mut() {
                ws.windows.retain(|&id| id != window_id);
            }
        }

        if *self.focused_window.read() == Some(window_id) {
            *self.focused_window.write() = None;
        }

        println!("[WM] Destroyed window {}", window_id);

        Ok(())
    }

    /// Move a window
    pub fn move_window(&self, window_id: WindowId, x: i32, y: i32) -> Result<(), KernelError> {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.x = x;
            window.y = y;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Resize a window
    pub fn resize_window(
        &self,
        window_id: WindowId,
        width: u32,
        height: u32,
    ) -> Result<(), KernelError> {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.width = width;
            window.height = height;
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Focus a window
    pub fn focus_window(&self, window_id: WindowId) -> Result<(), KernelError> {
        // Unfocus previous window
        if let Some(prev_id) = *self.focused_window.read() {
            if let Some(prev_window) = self.windows.write().get_mut(&prev_id) {
                prev_window.focused = false;
            }
        }

        // Focus new window
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.focused = true;
            *self.focused_window.write() = Some(window_id);

            // Bring to front
            let mut z_order = self.z_order.write();
            z_order.retain(|&id| id != window_id);
            z_order.push(window_id);

            println!("[WM] Focused window {}", window_id);
            Ok(())
        } else {
            Err(KernelError::NotFound {
                resource: "window",
                id: window_id as u64,
            })
        }
    }

    /// Get window at position
    pub fn window_at_position(&self, x: i32, y: i32) -> Option<WindowId> {
        let windows = self.windows.read();
        let z_order = self.z_order.read();

        // Search from top to bottom
        for &window_id in z_order.iter().rev() {
            if let Some(window) = windows.get(&window_id) {
                if window.visible
                    && x >= window.x
                    && x < window.x + window.width as i32
                    && y >= window.y
                    && y < window.y + window.height as i32
                {
                    return Some(window_id);
                }
            }
        }

        None
    }

    /// Process input event
    pub fn process_input(&self, event: InputEvent) {
        match event {
            InputEvent::MouseMove { x, y } => {
                *self.mouse_x.write() = x;
                *self.mouse_y.write() = y;

                // Send to focused window
                if let Some(window_id) = *self.focused_window.read() {
                    self.queue_event(WindowEvent { window_id, event });
                }
            }
            InputEvent::MouseButton {
                button: _button,
                pressed,
                x,
                y,
            } => {
                if pressed {
                    // Click - focus window at position
                    if let Some(window_id) = self.window_at_position(x, y) {
                        if let Err(_e) = self.focus_window(window_id) {
                            crate::println!(
                                "[WM] Warning: failed to focus window {}: {:?}",
                                window_id,
                                _e
                            );
                        }

                        // Send click event to window
                        self.queue_event(WindowEvent { window_id, event });
                    }
                } else {
                    // Release - send to focused window
                    if let Some(window_id) = *self.focused_window.read() {
                        self.queue_event(WindowEvent { window_id, event });
                    }
                }
            }
            InputEvent::KeyPress { .. }
            | InputEvent::KeyRelease { .. }
            | InputEvent::MouseScroll { .. } => {
                // Send keyboard events to focused window
                if let Some(window_id) = *self.focused_window.read() {
                    self.queue_event(WindowEvent { window_id, event });
                }
            }
        }
    }

    /// Queue an event for delivery
    pub fn queue_event(&self, event: WindowEvent) {
        self.event_queue.write().push(event);
    }

    /// Get pending events for a window
    pub fn get_events(&self, window_id: WindowId) -> Vec<InputEvent> {
        let mut queue = self.event_queue.write();
        let mut events = Vec::new();

        // Extract events for this window
        let mut i = 0;
        while i < queue.len() {
            if queue[i].window_id == window_id {
                events.push(queue.remove(i).event);
            } else {
                i += 1;
            }
        }

        events
    }

    /// Set a window's title.
    pub fn set_window_title(&self, window_id: WindowId, title: &str) {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.set_title(title);
        }
    }

    /// Get a clone of a window by ID.
    pub fn get_window(&self, window_id: WindowId) -> Option<Window> {
        self.windows.read().get(&window_id).cloned()
    }

    /// Get the currently focused window ID.
    pub fn get_focused_window_id(&self) -> Option<WindowId> {
        *self.focused_window.read()
    }

    /// Get all windows
    pub fn get_all_windows(&self) -> Vec<Window> {
        self.windows.read().values().cloned().collect()
    }

    /// Get all visible windows on the active workspace
    #[allow(dead_code)]
    pub fn get_visible_windows(&self) -> Vec<Window> {
        let active = *self.active_workspace.read();
        self.windows
            .read()
            .values()
            .filter(|w| w.visible && w.workspace == active && w.state != WindowState::Minimized)
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // WM-1: Placement heuristics and snap/tile
    // -----------------------------------------------------------------------

    /// Set the screen dimensions (called when display is configured)
    #[allow(dead_code)]
    pub fn set_screen_size(&self, width: u32, height: u32) {
        *self.screen_width.write() = width;
        *self.screen_height.write() = height;
    }

    /// Set the window placement heuristic
    #[allow(dead_code)]
    pub fn set_placement_heuristic(&self, heuristic: PlacementHeuristic) {
        *self.placement_heuristic.write() = heuristic;
    }

    /// Compute the placement position for a window based on the current
    /// heuristic.
    ///
    /// The window must already be inserted into `self.windows` so its
    /// dimensions can be read. Returns `(x, y)` for the top-left corner.
    #[allow(dead_code)]
    pub fn place_window(&self, window_id: WindowId) -> (i32, i32) {
        let (win_w, win_h) = {
            let windows = self.windows.read();
            match windows.get(&window_id) {
                Some(w) => (w.width, w.height),
                None => return (0, 0),
            }
        };

        let scr_w = *self.screen_width.read();
        let scr_h = *self.screen_height.read();
        let heuristic = *self.placement_heuristic.read();

        match heuristic {
            PlacementHeuristic::Cascade => {
                let mut offset = self.cascade_offset.write();
                let x = offset.0;
                let y = offset.1;

                // Advance cascade position
                offset.0 += 32;
                offset.1 += 32;

                // Wrap around if we go off-screen
                if offset.0 + win_w as i32 > scr_w as i32 || offset.1 + win_h as i32 > scr_h as i32
                {
                    offset.0 = 32;
                    offset.1 = 32;
                }

                (x, y)
            }
            PlacementHeuristic::Center => {
                let x = (scr_w as i32 - win_w as i32) / 2;
                let y = (scr_h as i32 - win_h as i32) / 2;
                (x.max(0), y.max(0))
            }
            PlacementHeuristic::Smart => self.smart_place(win_w, win_h, scr_w, scr_h),
            PlacementHeuristic::Manual { x, y } => (x, y),
        }
    }

    /// Smart placement: find position with minimal overlap with existing
    /// windows
    fn smart_place(&self, win_w: u32, win_h: u32, scr_w: u32, scr_h: u32) -> (i32, i32) {
        let windows = self.windows.read();
        let visible: Vec<&Window> = windows
            .values()
            .filter(|w| w.visible && w.state != WindowState::Minimized)
            .collect();

        if visible.is_empty() {
            let x = (scr_w as i32 - win_w as i32) / 2;
            let y = (scr_h as i32 - win_h as i32) / 2;
            return (x.max(0), y.max(0));
        }

        let step = 64i32;
        let mut best_x = 0i32;
        let mut best_y = 0i32;
        let mut best_overlap = i64::MAX;

        let max_x = (scr_w as i32 - win_w as i32).max(0);
        let max_y = (scr_h as i32 - win_h as i32).max(0);

        let mut cy = 0i32;
        while cy <= max_y {
            let mut cx = 0i32;
            while cx <= max_x {
                let mut total_overlap: i64 = 0;

                for w in &visible {
                    let ox1 = cx.max(w.x);
                    let oy1 = cy.max(w.y);
                    let ox2 = (cx + win_w as i32).min(w.x + w.width as i32);
                    let oy2 = (cy + win_h as i32).min(w.y + w.height as i32);

                    if ox1 < ox2 && oy1 < oy2 {
                        total_overlap += (ox2 - ox1) as i64 * (oy2 - oy1) as i64;
                    }
                }

                if total_overlap < best_overlap {
                    best_overlap = total_overlap;
                    best_x = cx;
                    best_y = cy;

                    if total_overlap == 0 {
                        return (best_x, best_y);
                    }
                }

                cx += step;
            }
            cy += step;
        }

        (best_x, best_y)
    }

    /// Snap a window to a screen zone (half, quarter, or maximize).
    ///
    /// Saves the window's current geometry so it can be restored later.
    #[allow(dead_code)]
    pub fn snap_window(&self, window_id: WindowId, zone: SnapZone) {
        let scr_w = *self.screen_width.read();
        let scr_h = *self.screen_height.read();
        let panel_h: u32 = 32;
        let usable_h = scr_h.saturating_sub(panel_h);

        let half_w = scr_w / 2;
        let half_h = usable_h / 2;

        let mut windows = self.windows.write();
        let window = match windows.get_mut(&window_id) {
            Some(w) => w,
            None => return,
        };

        // Save geometry before snapping (only if not already snapped)
        if window.snap_zone == SnapZone::None {
            window.saved_geometry = Some((window.x, window.y, window.width, window.height));
        }

        match zone {
            SnapZone::None => {
                if let Some((sx, sy, sw, sh)) = window.saved_geometry.take() {
                    window.x = sx;
                    window.y = sy;
                    window.width = sw;
                    window.height = sh;
                }
                window.state = WindowState::Normal;
            }
            SnapZone::Left => {
                window.x = 0;
                window.y = 0;
                window.width = half_w;
                window.height = usable_h;
            }
            SnapZone::Right => {
                window.x = half_w as i32;
                window.y = 0;
                window.width = scr_w - half_w;
                window.height = usable_h;
            }
            SnapZone::Top => {
                window.x = 0;
                window.y = 0;
                window.width = scr_w;
                window.height = half_h;
            }
            SnapZone::Bottom => {
                window.x = 0;
                window.y = half_h as i32;
                window.width = scr_w;
                window.height = usable_h - half_h;
            }
            SnapZone::TopLeft => {
                window.x = 0;
                window.y = 0;
                window.width = half_w;
                window.height = half_h;
            }
            SnapZone::TopRight => {
                window.x = half_w as i32;
                window.y = 0;
                window.width = scr_w - half_w;
                window.height = half_h;
            }
            SnapZone::BottomLeft => {
                window.x = 0;
                window.y = half_h as i32;
                window.width = half_w;
                window.height = usable_h - half_h;
            }
            SnapZone::BottomRight => {
                window.x = half_w as i32;
                window.y = half_h as i32;
                window.width = scr_w - half_w;
                window.height = usable_h - half_h;
            }
            SnapZone::Maximize => {
                window.x = 0;
                window.y = 0;
                window.width = scr_w;
                window.height = usable_h;
                window.state = WindowState::Maximized;
            }
        }

        window.snap_zone = zone;
    }

    /// Detect which snap zone a screen coordinate falls in.
    ///
    /// Returns `SnapZone::None` if the position is not within the edge
    /// threshold (8 pixels).
    #[allow(dead_code)]
    pub fn detect_snap_zone(x: i32, y: i32, screen_w: u32, screen_h: u32) -> SnapZone {
        const EDGE_THRESHOLD: i32 = 8;
        let sw = screen_w as i32;
        let sh = screen_h as i32;

        let near_left = x < EDGE_THRESHOLD;
        let near_right = x >= sw - EDGE_THRESHOLD;
        let near_top = y < EDGE_THRESHOLD;
        let near_bottom = y >= sh - EDGE_THRESHOLD;

        match (near_left, near_right, near_top, near_bottom) {
            (true, false, true, false) => SnapZone::TopLeft,
            (true, false, false, true) => SnapZone::BottomLeft,
            (false, true, true, false) => SnapZone::TopRight,
            (false, true, false, true) => SnapZone::BottomRight,
            (true, false, false, false) => SnapZone::Left,
            (false, true, false, false) => SnapZone::Right,
            (false, false, true, false) => SnapZone::Top,
            (false, false, false, true) => SnapZone::Bottom,
            _ => SnapZone::None,
        }
    }

    /// Tile all visible windows on the active workspace using the given layout.
    #[allow(dead_code)]
    pub fn tile_windows(&self, layout: TileLayout) {
        let scr_w = *self.screen_width.read();
        let scr_h = *self.screen_height.read();
        let panel_h: u32 = 32;
        let usable_h = scr_h.saturating_sub(panel_h);

        let active_ws = *self.active_workspace.read();
        let mut windows = self.windows.write();

        let visible_ids: Vec<WindowId> = windows
            .values()
            .filter(|w| {
                w.visible
                    && w.workspace == active_ws
                    && w.state != WindowState::Minimized
                    && w.state != WindowState::Hidden
            })
            .map(|w| w.id)
            .collect();

        let count = visible_ids.len();
        if count == 0 {
            return;
        }

        match layout {
            TileLayout::EqualColumns => {
                let col_width = scr_w / count as u32;
                for (i, &wid) in visible_ids.iter().enumerate() {
                    if let Some(w) = windows.get_mut(&wid) {
                        w.x = (i as u32 * col_width) as i32;
                        w.y = 0;
                        w.width = col_width;
                        w.height = usable_h;
                        w.snap_zone = SnapZone::None;
                    }
                }
            }
            TileLayout::MasterStack => {
                if count == 1 {
                    if let Some(w) = windows.get_mut(&visible_ids[0]) {
                        w.x = 0;
                        w.y = 0;
                        w.width = scr_w;
                        w.height = usable_h;
                        w.snap_zone = SnapZone::None;
                    }
                } else {
                    let master_w = (scr_w * 60) / 100;
                    let stack_w = scr_w - master_w;
                    let stack_count = (count - 1) as u32;
                    let stack_h = usable_h / stack_count;

                    if let Some(w) = windows.get_mut(&visible_ids[0]) {
                        w.x = 0;
                        w.y = 0;
                        w.width = master_w;
                        w.height = usable_h;
                        w.snap_zone = SnapZone::None;
                    }

                    for (i, &wid) in visible_ids.iter().skip(1).enumerate() {
                        if let Some(w) = windows.get_mut(&wid) {
                            w.x = master_w as i32;
                            w.y = (i as u32 * stack_h) as i32;
                            w.width = stack_w;
                            w.height = stack_h;
                            w.snap_zone = SnapZone::None;
                        }
                    }
                }
            }
            TileLayout::Grid => {
                let cols = {
                    let mut c = 1u32;
                    while c * c < count as u32 {
                        c += 1;
                    }
                    c
                };
                let rows = (count as u32).div_ceil(cols);
                let cell_w = scr_w / cols;
                let cell_h = usable_h / rows;

                for (i, &wid) in visible_ids.iter().enumerate() {
                    let col = (i as u32) % cols;
                    let row = (i as u32) / cols;
                    if let Some(w) = windows.get_mut(&wid) {
                        w.x = (col * cell_w) as i32;
                        w.y = (row * cell_h) as i32;
                        w.width = cell_w;
                        w.height = cell_h;
                        w.snap_zone = SnapZone::None;
                    }
                }
            }
        }
    }

    /// Set window opacity (0 = transparent, 255 = opaque)
    #[allow(dead_code)]
    pub fn set_window_opacity(&self, window_id: WindowId, opacity: u8) {
        if let Some(window) = self.windows.write().get_mut(&window_id) {
            window.opacity = opacity;
        }
    }

    // -----------------------------------------------------------------------
    // WM-4: Virtual workspaces
    // -----------------------------------------------------------------------

    /// Switch to a different workspace.
    #[allow(dead_code)]
    pub fn switch_workspace(&self, workspace_id: WorkspaceId) {
        if workspace_id as usize >= MAX_WORKSPACES {
            return;
        }

        let current = *self.active_workspace.read();
        if current == workspace_id {
            return;
        }

        let mut windows = self.windows.write();

        for window in windows.values_mut() {
            if window.workspace == current {
                window.visible = false;
            }
        }

        for window in windows.values_mut() {
            if window.workspace == workspace_id && window.state != WindowState::Minimized {
                window.visible = true;
            }
        }

        *self.active_workspace.write() = workspace_id;
        *self.focused_window.write() = None;

        crate::println!("[WM] Switched to workspace {}", workspace_id + 1);
    }

    /// Move a window to a different workspace.
    #[allow(dead_code)]
    pub fn move_window_to_workspace(&self, window_id: WindowId, workspace_id: WorkspaceId) {
        if workspace_id as usize >= MAX_WORKSPACES {
            return;
        }

        let active = *self.active_workspace.read();

        {
            let mut workspaces = self.workspaces.write();
            for ws in workspaces.iter_mut() {
                ws.windows.retain(|&id| id != window_id);
            }
            if let Some(ws) = workspaces.get_mut(workspace_id as usize) {
                ws.windows.push(window_id);
            }
        }

        let mut windows = self.windows.write();
        if let Some(window) = windows.get_mut(&window_id) {
            window.workspace = workspace_id;

            if workspace_id != active {
                window.visible = false;
                if *self.focused_window.read() == Some(window_id) {
                    *self.focused_window.write() = None;
                }
            } else {
                window.visible = true;
            }
        }

        crate::println!(
            "[WM] Moved window {} to workspace {}",
            window_id,
            workspace_id + 1
        );
    }

    /// Get the currently active workspace ID.
    #[allow(dead_code)]
    pub fn get_active_workspace(&self) -> WorkspaceId {
        *self.active_workspace.read()
    }

    /// Get the list of window IDs on a given workspace.
    #[allow(dead_code)]
    pub fn get_workspace_windows(&self, workspace_id: WorkspaceId) -> Vec<WindowId> {
        if workspace_id as usize >= MAX_WORKSPACES {
            return Vec::new();
        }
        let workspaces = self.workspaces.read();
        match workspaces.get(workspace_id as usize) {
            Some(ws) => ws.windows.clone(),
            None => Vec::new(),
        }
    }

    /// Event loop iteration
    pub fn event_loop_iteration(&self) {
        // Process any pending hardware events
        // This would integrate with keyboard/mouse drivers

        // For now, this is a stub showing the structure
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global window manager
static WINDOW_MANAGER: GlobalState<WindowManager> = GlobalState::new();

/// Initialize window manager
pub fn init() -> Result<(), KernelError> {
    let wm = WindowManager::new();
    WINDOW_MANAGER
        .init(wm)
        .map_err(|_| KernelError::InvalidState {
            expected: "uninitialized",
            actual: "initialized",
        })?;

    println!("[WM] Window manager initialized");
    Ok(())
}

/// Execute a function with the window manager
pub fn with_window_manager<R, F: FnOnce(&WindowManager) -> R>(f: F) -> Option<R> {
    WINDOW_MANAGER.with(f)
}

/// Get the global window manager (deprecated - use with_window_manager instead)
pub fn get_window_manager() -> Result<(), KernelError> {
    WINDOW_MANAGER
        .with(|_| ())
        .ok_or(KernelError::InvalidState {
            expected: "initialized",
            actual: "uninitialized",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_creation() {
        let wm = WindowManager::new();
        let id = wm.create_window(0, 0, 640, 480, 1).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_window_focus() {
        let wm = WindowManager::new();
        let id1 = wm.create_window(0, 0, 640, 480, 1).unwrap();
        let id2 = wm.create_window(100, 100, 640, 480, 1).unwrap();

        wm.focus_window(id1).unwrap();
        assert_eq!(*wm.focused_window.read(), Some(id1));

        wm.focus_window(id2).unwrap();
        assert_eq!(*wm.focused_window.read(), Some(id2));
    }

    #[test]
    fn test_window_at_position() {
        let wm = WindowManager::new();
        let id = wm.create_window(100, 100, 200, 150, 1).unwrap();

        assert_eq!(wm.window_at_position(150, 150), Some(id));
        assert_eq!(wm.window_at_position(50, 50), None);
    }

    #[test]
    fn test_snap_zone_detection() {
        assert_eq!(
            WindowManager::detect_snap_zone(2, 400, 1280, 800),
            SnapZone::Left
        );
        assert_eq!(
            WindowManager::detect_snap_zone(1275, 400, 1280, 800),
            SnapZone::Right
        );
        assert_eq!(
            WindowManager::detect_snap_zone(2, 2, 1280, 800),
            SnapZone::TopLeft
        );
        assert_eq!(
            WindowManager::detect_snap_zone(640, 400, 1280, 800),
            SnapZone::None
        );
    }
}
