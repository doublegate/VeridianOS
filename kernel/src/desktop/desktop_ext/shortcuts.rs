//! Global Keyboard Shortcuts
//!
//! Configurable key bindings with modifier masks.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

/// Modifier key bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModifierMask(pub u8);

impl ModifierMask {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const SUPER: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);

    /// Check if a modifier is set.
    pub fn has(self, modifier: Self) -> bool {
        (self.0 & modifier.0) == modifier.0
    }

    /// Combine two modifier masks.
    pub fn combine(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Remove a modifier.
    pub fn remove(self, modifier: Self) -> Self {
        Self(self.0 & !modifier.0)
    }

    /// Check if this mask is empty (no modifiers).
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Key code (PS/2 scancode or virtual key code).
pub type KeyCode = u8;

/// Actions that can be triggered by keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Launch the application launcher.
    LaunchLauncher,
    /// Launch terminal.
    LaunchTerminal,
    /// Launch file manager.
    LaunchFileManager,
    /// Close the focused window.
    CloseWindow,
    /// Minimize the focused window.
    MinimizeWindow,
    /// Maximize/restore the focused window.
    MaximizeWindow,
    /// Toggle fullscreen on focused window.
    ToggleFullscreen,
    /// Switch to workspace N (0-based).
    SwitchWorkspace(u8),
    /// Move window to workspace N (0-based).
    MoveToWorkspace(u8),
    /// Switch to next window (Alt+Tab).
    SwitchNextWindow,
    /// Switch to previous window (Alt+Shift+Tab).
    SwitchPrevWindow,
    /// Take a screenshot.
    Screenshot,
    /// Take a screenshot of the focused window.
    ScreenshotWindow,
    /// Lock the screen.
    LockScreen,
    /// Log out.
    Logout,
    /// Snap window left.
    SnapLeft,
    /// Snap window right.
    SnapRight,
    /// Copy (Ctrl+C).
    Copy,
    /// Paste (Ctrl+V).
    Paste,
    /// Cut (Ctrl+X).
    Cut,
    /// Undo (Ctrl+Z).
    Undo,
    /// Redo (Ctrl+Shift+Z or Ctrl+Y).
    Redo,
    /// Custom action identified by ID.
    Custom(u16),
}

/// Priority for shortcut matching (higher wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ShortcutPriority {
    /// System-level shortcuts (cannot be overridden).
    System = 3,
    /// Desktop environment shortcuts.
    Desktop = 2,
    /// Application shortcuts.
    Application = 1,
    /// User-defined shortcuts.
    #[default]
    User = 0,
}

/// A keyboard shortcut binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    /// Required modifier keys.
    pub modifiers: ModifierMask,
    /// The key code.
    pub key: KeyCode,
    /// Action to perform.
    pub action: ShortcutAction,
    /// Priority for conflict resolution.
    pub priority: ShortcutPriority,
    /// Whether this binding is currently enabled.
    pub enabled: bool,
}

impl KeyBinding {
    /// Create a new key binding.
    pub fn new(modifiers: ModifierMask, key: KeyCode, action: ShortcutAction) -> Self {
        Self {
            modifiers,
            key,
            action,
            priority: ShortcutPriority::User,
            enabled: true,
        }
    }

    /// Create a new key binding with priority.
    pub fn with_priority(mut self, priority: ShortcutPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Check if this binding matches the given modifiers and key.
    pub fn matches(&self, modifiers: ModifierMask, key: KeyCode) -> bool {
        self.enabled && self.modifiers == modifiers && self.key == key
    }
}

/// Maximum number of registered shortcuts.
const MAX_SHORTCUTS: usize = 128;

/// Keyboard shortcut manager.
#[derive(Debug)]
#[cfg(feature = "alloc")]
pub struct ShortcutManager {
    /// Registered bindings.
    bindings: Vec<KeyBinding>,
    /// Whether shortcut processing is globally enabled.
    enabled: bool,
    /// Binding IDs for removal (index into bindings).
    next_id: u32,
    /// Map from binding ID to index.
    id_map: BTreeMap<u32, usize>,
}

#[cfg(feature = "alloc")]
impl Default for ShortcutManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl ShortcutManager {
    /// Create a new shortcut manager with default system bindings.
    pub fn new() -> Self {
        let mut mgr = Self {
            bindings: Vec::new(),
            enabled: true,
            next_id: 0,
            id_map: BTreeMap::new(),
        };
        mgr.register_defaults();
        mgr
    }

    /// Register default system shortcuts.
    fn register_defaults(&mut self) {
        // Alt+Tab: switch window
        self.register(
            KeyBinding::new(ModifierMask::ALT, 0x0F, ShortcutAction::SwitchNextWindow)
                .with_priority(ShortcutPriority::System),
        );
        // Ctrl+Alt+L: lock screen
        self.register(
            KeyBinding::new(
                ModifierMask(ModifierMask::CTRL.0 | ModifierMask::ALT.0),
                0x26,
                ShortcutAction::LockScreen,
            )
            .with_priority(ShortcutPriority::System),
        );
        // Super: launcher
        self.register(
            KeyBinding::new(ModifierMask::SUPER, 0xDB, ShortcutAction::LaunchLauncher)
                .with_priority(ShortcutPriority::Desktop),
        );
        // Ctrl+C: copy
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2E, ShortcutAction::Copy)
                .with_priority(ShortcutPriority::Application),
        );
        // Ctrl+V: paste
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2F, ShortcutAction::Paste)
                .with_priority(ShortcutPriority::Application),
        );
        // Ctrl+X: cut
        self.register(
            KeyBinding::new(ModifierMask::CTRL, 0x2D, ShortcutAction::Cut)
                .with_priority(ShortcutPriority::Application),
        );
        // Alt+F4: close window
        self.register(
            KeyBinding::new(ModifierMask::ALT, 0x3E, ShortcutAction::CloseWindow)
                .with_priority(ShortcutPriority::Desktop),
        );
        // Print Screen (scancode 0x37 with E0 prefix): screenshot
        self.register(
            KeyBinding::new(ModifierMask::NONE, 0xB7, ShortcutAction::Screenshot)
                .with_priority(ShortcutPriority::System),
        );
    }

    /// Register a new shortcut binding. Returns binding ID.
    pub fn register(&mut self, binding: KeyBinding) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        if self.bindings.len() < MAX_SHORTCUTS {
            self.id_map.insert(id, self.bindings.len());
            self.bindings.push(binding);
        }

        id
    }

    /// Remove a shortcut by ID.
    pub fn unregister(&mut self, id: u32) -> bool {
        if let Some(&index) = self.id_map.get(&id) {
            if index < self.bindings.len() {
                self.bindings.remove(index);
                self.id_map.remove(&id);
                // Rebuild ID map (indices shifted).
                let mut new_map = BTreeMap::new();
                for (&k, &v) in &self.id_map {
                    if v > index {
                        new_map.insert(k, v - 1);
                    } else {
                        new_map.insert(k, v);
                    }
                }
                self.id_map = new_map;
                return true;
            }
        }
        false
    }

    /// Process a key event and return the matching action (if any).
    /// Returns the highest-priority matching action.
    pub fn process_key(&self, modifiers: ModifierMask, key: KeyCode) -> Option<ShortcutAction> {
        if !self.enabled {
            return None;
        }

        self.bindings
            .iter()
            .filter(|b| b.matches(modifiers, key))
            .max_by_key(|b| b.priority)
            .map(|b| b.action)
    }

    /// Enable or disable all shortcut processing.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if shortcuts are enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the number of registered bindings.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Get all bindings for a given action.
    pub fn bindings_for_action(&self, action: ShortcutAction) -> Vec<&KeyBinding> {
        self.bindings
            .iter()
            .filter(|b| b.action == action)
            .collect()
    }

    /// Enable or disable a specific binding by ID.
    pub fn set_binding_enabled(&mut self, id: u32, enabled: bool) -> bool {
        if let Some(&index) = self.id_map.get(&id) {
            if let Some(binding) = self.bindings.get_mut(index) {
                binding.enabled = enabled;
                return true;
            }
        }
        false
    }
}
