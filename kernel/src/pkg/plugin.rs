//! Package Plugin System for VeridianOS
//!
//! Defines the contract for package lifecycle hooks. Plugins can register
//! for specific lifecycle events (install, remove, update, etc.) and declare
//! the capabilities they require.
//!
//! NOTE: Actual ELF dynamic loading of plugin code is deferred to user-space.
//! This module defines the type-safe contract, state machine, and plugin
//! registry that the kernel uses to manage plugin metadata.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::error::{KernelError, KernelResult};

// ============================================================================
// PluginHook
// ============================================================================

/// Lifecycle hook points at which a plugin can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginHook {
    /// Invoked before a package is installed.
    PreInstall,
    /// Invoked after a package is successfully installed.
    PostInstall,
    /// Invoked before a package is removed.
    PreRemove,
    /// Invoked after a package is successfully removed.
    PostRemove,
    /// Invoked before a package is updated.
    PreUpdate,
    /// Invoked after a package is successfully updated.
    PostUpdate,
    /// Invoked when a package's configuration is being applied.
    Configure,
    /// Invoked to verify package integrity.
    Verify,
}

impl PluginHook {
    /// Return a short string identifier for this hook.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PreInstall => "pre-install",
            Self::PostInstall => "post-install",
            Self::PreRemove => "pre-remove",
            Self::PostRemove => "post-remove",
            Self::PreUpdate => "pre-update",
            Self::PostUpdate => "post-update",
            Self::Configure => "configure",
            Self::Verify => "verify",
        }
    }
}

// ============================================================================
// PluginCapability
// ============================================================================

/// Capabilities that a plugin may request in order to perform its work.
///
/// Each capability maps to a kernel-level permission check during plugin
/// invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginCapability {
    /// Read/write access to the filesystem.
    FileSystemAccess,
    /// Outbound network access (e.g. for downloading resources).
    NetworkAccess,
    /// Ability to spawn child processes.
    ProcessSpawn,
    /// Permission to modify package configuration files.
    ConfigModify,
    /// Control system services (start, stop, restart).
    ServiceControl,
}

impl PluginCapability {
    /// Return a short string identifier for this capability.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileSystemAccess => "fs-access",
            Self::NetworkAccess => "net-access",
            Self::ProcessSpawn => "process-spawn",
            Self::ConfigModify => "config-modify",
            Self::ServiceControl => "service-control",
        }
    }
}

// ============================================================================
// PluginMetadata
// ============================================================================

/// Metadata describing a plugin, including its identity, required capabilities,
/// and the hooks it supports.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    /// Unique plugin name.
    pub name: String,
    /// Plugin version string (semver).
    pub version: String,
    /// Human-readable description of the plugin.
    pub description: String,
    /// Capabilities this plugin requires.
    pub capabilities: Vec<PluginCapability>,
    /// Lifecycle hooks this plugin handles.
    pub hooks: Vec<PluginHook>,
}

#[cfg(feature = "alloc")]
impl PluginMetadata {
    /// Create a new plugin metadata with the given identity.
    pub fn new(name: &str, version: &str, description: &str) -> Self {
        Self {
            name: String::from(name),
            version: String::from(version),
            description: String::from(description),
            capabilities: Vec::new(),
            hooks: Vec::new(),
        }
    }

    /// Declare that this plugin requires the given capability.
    pub fn add_capability(&mut self, capability: PluginCapability) {
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
    }

    /// Register a lifecycle hook that this plugin handles.
    pub fn add_hook(&mut self, hook: PluginHook) {
        if !self.hooks.contains(&hook) {
            self.hooks.push(hook);
        }
    }

    /// Check whether this plugin requires the given capability.
    pub fn has_capability(&self, capability: PluginCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Check whether this plugin supports the given hook.
    pub fn supports_hook(&self, hook: PluginHook) -> bool {
        self.hooks.contains(&hook)
    }
}

// ============================================================================
// PluginState
// ============================================================================

/// Lifecycle state of a loaded plugin instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is registered but not yet loaded into memory.
    Unloaded,
    /// Plugin binary has been loaded.
    Loaded,
    /// Plugin has been initialized (init function called).
    Initialized,
    /// Plugin is actively handling hooks.
    Active,
    /// Plugin encountered an error and is disabled.
    Error,
}

// ============================================================================
// PluginInstance
// ============================================================================

/// A registered plugin instance combining metadata and runtime state.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PluginInstance {
    /// Descriptive metadata for this plugin.
    pub metadata: PluginMetadata,
    /// Current lifecycle state.
    pub state: PluginState,
}

#[cfg(feature = "alloc")]
impl PluginInstance {
    /// Create a new plugin instance in the `Unloaded` state.
    pub fn new(metadata: PluginMetadata) -> Self {
        Self {
            metadata,
            state: PluginState::Unloaded,
        }
    }

    /// Attempt to transition to the given state.
    ///
    /// Valid transitions:
    /// - Unloaded -> Loaded
    /// - Loaded -> Initialized
    /// - Initialized -> Active
    /// - Any state -> Error
    ///
    /// Returns an error for invalid transitions.
    pub fn transition_to(&mut self, new_state: PluginState) -> KernelResult<()> {
        // Any state can transition to Error
        if new_state == PluginState::Error {
            self.state = new_state;
            return Ok(());
        }

        let valid = matches!(
            (self.state, new_state),
            (PluginState::Unloaded, PluginState::Loaded)
                | (PluginState::Loaded, PluginState::Initialized)
                | (PluginState::Initialized, PluginState::Active)
        );

        if valid {
            self.state = new_state;
            Ok(())
        } else {
            Err(KernelError::InvalidState {
                expected: match self.state {
                    PluginState::Unloaded => "Loaded",
                    PluginState::Loaded => "Initialized",
                    PluginState::Initialized => "Active",
                    PluginState::Active => "Error (terminal state)",
                    PluginState::Error => "Error (terminal state)",
                },
                actual: match new_state {
                    PluginState::Unloaded => "Unloaded",
                    PluginState::Loaded => "Loaded",
                    PluginState::Initialized => "Initialized",
                    PluginState::Active => "Active",
                    PluginState::Error => "Error",
                },
            })
        }
    }
}

// ============================================================================
// PluginManager
// ============================================================================

/// Registry and manager for package lifecycle plugins.
///
/// Tracks registered plugins and dispatches lifecycle hooks to all plugins
/// that declare support for a given hook point.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct PluginManager {
    /// Registered plugins, keyed by plugin name.
    plugins: BTreeMap<String, PluginInstance>,
}

#[cfg(feature = "alloc")]
impl PluginManager {
    /// Create a new empty plugin manager.
    pub fn new() -> Self {
        Self {
            plugins: BTreeMap::new(),
        }
    }

    /// Register a new plugin. Returns an error if a plugin with the same name
    /// is already registered.
    pub fn register(&mut self, metadata: PluginMetadata) -> KernelResult<()> {
        if self.plugins.contains_key(&metadata.name) {
            return Err(KernelError::AlreadyExists {
                resource: "plugin",
                id: 0,
            });
        }
        let name = metadata.name.clone();
        self.plugins.insert(name, PluginInstance::new(metadata));
        Ok(())
    }

    /// Unregister a plugin by name. Returns an error if the plugin is not
    /// found.
    pub fn unregister(&mut self, name: &str) -> KernelResult<()> {
        if self.plugins.remove(name).is_none() {
            return Err(KernelError::NotFound {
                resource: "plugin",
                id: 0,
            });
        }
        Ok(())
    }

    /// List all registered plugin names.
    pub fn list(&self) -> Vec<&str> {
        self.plugins.keys().map(|k| k.as_str()).collect()
    }

    /// Look up a plugin instance by name.
    pub fn get(&self, name: &str) -> Option<&PluginInstance> {
        self.plugins.get(name)
    }

    /// Invoke a lifecycle hook on all plugins that support it.
    ///
    /// Iterates through every registered plugin and, for those that declare
    /// support for `hook`, logs the invocation.
    ///
    /// TODO(user-space): Actual plugin execution requires ELF dynamic loading
    /// and user-space process spawning. Currently this validates and logs
    /// which plugins would be invoked.
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
    pub fn invoke_hook(&self, hook: PluginHook, package_name: &str) -> KernelResult<()> {
        for (plugin_name, instance) in &self.plugins {
            if instance.metadata.supports_hook(hook) {
                // TODO(user-space): Load plugin ELF and call hook entry point
                crate::println!(
                    "[PLUGIN] Would invoke {} on plugin '{}' for package '{}'",
                    hook.as_str(),
                    plugin_name,
                    package_name
                );
            }
        }
        Ok(())
    }

    /// Return the number of registered plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
