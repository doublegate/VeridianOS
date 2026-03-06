//! Tab Process Isolation
//!
//! Provides per-tab sandboxing with separate DOM trees, JS virtual machines,
//! and GC heaps. Each tab runs as an isolated "process" with restricted
//! capabilities. Includes crash recovery, IPC between tabs, and resource
//! limits to prevent any single tab from monopolizing the system.

#![allow(dead_code)]

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use super::{dom_bindings::DomApi, js_gc::GcHeap, js_vm::JsVm, tabs::TabId};

// ---------------------------------------------------------------------------
// Tab Error Type
// ---------------------------------------------------------------------------

/// Errors from tab process isolation
#[derive(Debug, Clone)]
pub enum TabError {
    /// Maximum process limit reached
    ProcessLimitReached,
    /// Process already exists for this tab
    ProcessAlreadyExists,
    /// No process found for the given tab
    ProcessNotFound,
    /// Tab is not in the expected state
    InvalidState { expected: &'static str },
    /// A capability is not permitted
    CapabilityDenied { capability: &'static str },
    /// A resource limit was violated
    ResourceLimitViolation { message: String },
    /// JavaScript execution failed
    ScriptError { message: String },
}

impl core::fmt::Display for TabError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ProcessLimitReached => write!(f, "Maximum process limit reached"),
            Self::ProcessAlreadyExists => write!(f, "Process already exists for this tab"),
            Self::ProcessNotFound => write!(f, "No process for tab"),
            Self::InvalidState { expected } => {
                write!(f, "Tab is not in {} state", expected)
            }
            Self::CapabilityDenied { capability } => {
                write!(f, "{} not permitted", capability)
            }
            Self::ResourceLimitViolation { message } => write!(f, "{}", message),
            Self::ScriptError { message } => write!(f, "{}", message),
        }
    }
}

// ---------------------------------------------------------------------------
// Tab process state
// ---------------------------------------------------------------------------

/// State of a tab process
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabProcessState {
    /// Not yet started
    #[default]
    Created,
    /// Running normally
    Running,
    /// Suspended (e.g., background tab)
    Suspended,
    /// Crashed and awaiting recovery
    Crashed,
    /// Terminated / cleaned up
    Terminated,
}

// ---------------------------------------------------------------------------
// Resource limits
// ---------------------------------------------------------------------------

/// Resource limits for a tab process
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum JS heap size in bytes
    pub max_heap_bytes: usize,
    /// Maximum DOM nodes
    pub max_dom_nodes: usize,
    /// Maximum timers
    pub max_timers: usize,
    /// Maximum execution steps per tick
    pub max_steps_per_tick: usize,
    /// Maximum network connections (future)
    pub max_connections: usize,
    /// CPU time budget per tick (in microseconds)
    pub cpu_budget_us: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_heap_bytes: 64 * 1024 * 1024, // 64 MB
            max_dom_nodes: 100_000,
            max_timers: 1000,
            max_steps_per_tick: 1_000_000,
            max_connections: 16,
            cpu_budget_us: 50_000, // 50ms
        }
    }
}

// ---------------------------------------------------------------------------
// Resource usage tracking
// ---------------------------------------------------------------------------

/// Current resource usage for a tab
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Current heap bytes
    pub heap_bytes: usize,
    /// Current DOM node count
    pub dom_node_count: usize,
    /// Current timer count
    pub timer_count: usize,
    /// Total JS execution steps
    pub total_steps: u64,
    /// Steps this tick
    pub steps_this_tick: usize,
    /// Total ticks processed
    pub ticks_processed: u64,
    /// Number of GC collections
    pub gc_collections: usize,
    /// Number of crashes
    pub crash_count: usize,
}

// ---------------------------------------------------------------------------
// IPC message between tabs
// ---------------------------------------------------------------------------

/// Message types for inter-tab communication
#[derive(Debug, Clone)]
pub enum IpcMessage {
    /// postMessage-style string message
    PostMessage {
        source_tab: TabId,
        target_tab: TabId,
        origin: String,
        data: String,
    },
    /// Broadcast channel message
    BroadcastMessage {
        source_tab: TabId,
        channel: String,
        data: String,
    },
    /// Storage event (localStorage change)
    StorageEvent {
        key: String,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    /// Tab lifecycle notification
    TabEvent {
        tab_id: TabId,
        event: TabLifecycleEvent,
    },
}

/// Tab lifecycle events for IPC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabLifecycleEvent {
    /// Tab was created
    Created,
    /// Tab became active
    Activated,
    /// Tab was deactivated
    Deactivated,
    /// Tab is about to close
    BeforeUnload,
    /// Tab was closed
    Closed,
    /// Tab crashed
    Crashed,
    /// Tab recovered from crash
    Recovered,
}

// ---------------------------------------------------------------------------
// Tab process
// ---------------------------------------------------------------------------

/// An isolated tab process with its own JS VM, DOM, and GC
pub struct TabProcess {
    /// Associated tab ID
    pub tab_id: TabId,
    /// Process state
    pub state: TabProcessState,
    /// JavaScript virtual machine (isolated per tab)
    pub vm: JsVm,
    /// Garbage collector (isolated per tab)
    pub gc: GcHeap,
    /// DOM API (isolated per tab)
    pub dom_api: DomApi,
    /// Resource limits
    pub limits: ResourceLimits,
    /// Resource usage
    pub usage: ResourceUsage,
    /// Pending IPC messages to deliver
    pub inbox: Vec<IpcMessage>,
    /// Capabilities bitmap (what this tab is allowed to do)
    pub capabilities: TabCapabilities,
    /// Last error message
    pub last_error: Option<String>,
    /// Origin (scheme + host + port) for same-origin policy
    pub origin: String,
}

impl TabProcess {
    pub fn new(tab_id: TabId) -> Self {
        Self {
            tab_id,
            state: TabProcessState::Created,
            vm: JsVm::new(),
            gc: GcHeap::new(),
            dom_api: DomApi::new(),
            limits: ResourceLimits::default(),
            usage: ResourceUsage::default(),
            inbox: Vec::new(),
            capabilities: TabCapabilities::default_web(),
            last_error: None,
            origin: String::new(),
        }
    }

    /// Start the process
    pub fn start(&mut self) {
        self.state = TabProcessState::Running;
    }

    /// Suspend the process (background tab)
    pub fn suspend(&mut self) {
        if self.state == TabProcessState::Running {
            self.state = TabProcessState::Suspended;
        }
    }

    /// Resume a suspended process
    pub fn resume(&mut self) {
        if self.state == TabProcessState::Suspended {
            self.state = TabProcessState::Running;
        }
    }

    /// Mark as crashed
    pub fn crash(&mut self, error: &str) {
        self.state = TabProcessState::Crashed;
        self.last_error = Some(error.to_string());
        self.usage.crash_count += 1;
    }

    /// Terminate and clean up
    pub fn terminate(&mut self) {
        self.state = TabProcessState::Terminated;
        self.vm = JsVm::new(); // Reset VM
        self.gc = GcHeap::new(); // Reset GC
        self.dom_api = DomApi::new(); // Reset DOM
        self.inbox.clear();
    }

    /// Check if a resource limit would be exceeded
    pub fn check_limits(&self) -> Option<TabError> {
        if self.usage.heap_bytes > self.limits.max_heap_bytes {
            return Some(TabError::ResourceLimitViolation {
                message: String::from("Heap size limit exceeded"),
            });
        }
        if self.usage.dom_node_count > self.limits.max_dom_nodes {
            return Some(TabError::ResourceLimitViolation {
                message: String::from("DOM node limit exceeded"),
            });
        }
        if self.usage.timer_count > self.limits.max_timers {
            return Some(TabError::ResourceLimitViolation {
                message: String::from("Timer limit exceeded"),
            });
        }
        if self.usage.steps_this_tick > self.limits.max_steps_per_tick {
            return Some(TabError::ResourceLimitViolation {
                message: String::from("CPU budget exceeded for this tick"),
            });
        }
        None
    }

    /// Update resource usage from current state
    pub fn update_usage(&mut self) {
        self.usage.heap_bytes = self.gc.arena.bytes_allocated();
        self.usage.dom_node_count = self.dom_api.node_count();
        self.usage.timer_count = self.dom_api.timer_queue.pending_count();
    }

    /// Process one tick of execution
    pub fn tick(&mut self) -> Result<(), TabError> {
        if self.state != TabProcessState::Running {
            return Ok(());
        }

        self.usage.steps_this_tick = 0;
        self.usage.ticks_processed += 1;

        // Process timers
        let expired = self.dom_api.timer_queue.tick();
        for _callback_id in expired {
            // Invoke callback (stub -- connected when VM has function
            // call-by-id)
        }

        // Process IPC inbox
        let messages = core::mem::take(&mut self.inbox);
        for _msg in messages {
            // Deliver message to JS context (stub)
        }

        // GC check
        if self.gc.should_collect() {
            self.gc.collect(&self.vm);
            self.usage.gc_collections += 1;
        }

        // Update usage
        self.update_usage();

        // Check limits
        if let Some(violation) = self.check_limits() {
            let msg = format!("{}", violation);
            self.crash(&msg);
            return Err(violation);
        }

        Ok(())
    }

    /// Execute JavaScript in this tab's context
    pub fn execute_script(&mut self, source: &str) -> Result<(), TabError> {
        if self.state != TabProcessState::Running {
            return Err(TabError::InvalidState {
                expected: "running",
            });
        }

        // Quick check: can we handle this?
        if !self.capabilities.can_execute_js {
            return Err(TabError::CapabilityDenied {
                capability: "JavaScript execution",
            });
        }

        let mut parser = super::js_parser::JsParser::from_source(source);
        let root = parser.parse();

        if !parser.errors.is_empty() {
            let err = parser.errors.join("; ");
            self.last_error = Some(err.clone());
            return Err(TabError::ScriptError { message: err });
        }

        let mut compiler = super::js_compiler::Compiler::new();
        let chunk = compiler.compile(&parser.arena, root);

        match self.vm.run_chunk(&chunk) {
            Ok(_) => {
                self.update_usage();
                if let Some(violation) = self.check_limits() {
                    let msg = format!("{}", violation);
                    self.crash(&msg);
                    return Err(violation);
                }
                Ok(())
            }
            Err(e) => {
                let msg = format!("{}", e);
                self.last_error = Some(msg.clone());
                Err(TabError::ScriptError { message: msg })
            }
        }
    }

    /// Set the origin for same-origin checks
    pub fn set_origin(&mut self, origin: &str) {
        self.origin = origin.to_string();
    }

    /// Check if a target origin matches this tab's origin
    pub fn is_same_origin(&self, target_origin: &str) -> bool {
        self.origin == target_origin
    }
}

// ---------------------------------------------------------------------------
// Tab capabilities
// ---------------------------------------------------------------------------

/// Capabilities bitmap for tab sandboxing
#[derive(Debug, Clone)]
pub struct TabCapabilities {
    /// Can execute JavaScript
    pub can_execute_js: bool,
    /// Can access localStorage
    pub can_local_storage: bool,
    /// Can use timers (setTimeout/setInterval)
    pub can_timers: bool,
    /// Can make network requests
    pub can_network: bool,
    /// Can use postMessage to other tabs
    pub can_post_message: bool,
    /// Can use geolocation (future)
    pub can_geolocation: bool,
    /// Can use clipboard
    pub can_clipboard: bool,
    /// Can use notifications
    pub can_notifications: bool,
    /// Can access camera/microphone (future)
    pub can_media_devices: bool,
    /// Can create popups / new windows
    pub can_popups: bool,
}

impl TabCapabilities {
    /// Default capabilities for a web page
    pub fn default_web() -> Self {
        Self {
            can_execute_js: true,
            can_local_storage: true,
            can_timers: true,
            can_network: true,
            can_post_message: true,
            can_geolocation: false,
            can_clipboard: false,
            can_notifications: false,
            can_media_devices: false,
            can_popups: false,
        }
    }

    /// Restricted capabilities (e.g., sandboxed iframe)
    pub fn sandboxed() -> Self {
        Self {
            can_execute_js: false,
            can_local_storage: false,
            can_timers: false,
            can_network: false,
            can_post_message: false,
            can_geolocation: false,
            can_clipboard: false,
            can_notifications: false,
            can_media_devices: false,
            can_popups: false,
        }
    }

    /// Full capabilities (trusted content like veridian:// pages)
    pub fn trusted() -> Self {
        Self {
            can_execute_js: true,
            can_local_storage: true,
            can_timers: true,
            can_network: true,
            can_post_message: true,
            can_geolocation: true,
            can_clipboard: true,
            can_notifications: true,
            can_media_devices: true,
            can_popups: true,
        }
    }

    /// Apply sandbox flags (like HTML sandbox attribute)
    pub fn apply_sandbox_flags(&mut self, flags: &str) {
        // Reset all to sandboxed first
        *self = Self::sandboxed();

        // Then allow specific things based on flags
        for flag in flags.split_whitespace() {
            match flag {
                "allow-scripts" => self.can_execute_js = true,
                "allow-same-origin" => self.can_local_storage = true,
                "allow-popups" => self.can_popups = true,
                "allow-forms" => {}          // forms always allowed in our model
                "allow-top-navigation" => {} // handled elsewhere
                _ => {}                      // unknown flag, ignore
            }
        }
    }

    /// Count of enabled capabilities
    pub fn enabled_count(&self) -> usize {
        let bools = [
            self.can_execute_js,
            self.can_local_storage,
            self.can_timers,
            self.can_network,
            self.can_post_message,
            self.can_geolocation,
            self.can_clipboard,
            self.can_notifications,
            self.can_media_devices,
            self.can_popups,
        ];
        bools.iter().filter(|&&b| b).count()
    }
}

// ---------------------------------------------------------------------------
// Process isolation manager
// ---------------------------------------------------------------------------

/// Manages all tab processes and their isolation
pub struct ProcessIsolation {
    /// Tab processes keyed by TabId
    processes: BTreeMap<TabId, TabProcess>,
    /// IPC message queue (pending delivery)
    message_queue: Vec<IpcMessage>,
    /// Shared storage (simulates localStorage shared by same-origin tabs)
    shared_storage: BTreeMap<String, BTreeMap<String, String>>,
    /// Maximum concurrent processes
    max_processes: usize,
    /// Broadcast channel subscriptions: channel_name -> [tab_ids]
    broadcast_channels: BTreeMap<String, Vec<TabId>>,
    /// Total crash count across all tabs
    total_crashes: usize,
}

impl Default for ProcessIsolation {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessIsolation {
    pub fn new() -> Self {
        Self {
            processes: BTreeMap::new(),
            message_queue: Vec::new(),
            shared_storage: BTreeMap::new(),
            max_processes: 32,
            broadcast_channels: BTreeMap::new(),
            total_crashes: 0,
        }
    }

    /// Spawn a new tab process
    pub fn spawn_tab_process(&mut self, tab_id: TabId) -> Result<(), TabError> {
        if self.processes.len() >= self.max_processes {
            return Err(TabError::ProcessLimitReached);
        }
        if self.processes.contains_key(&tab_id) {
            return Err(TabError::ProcessAlreadyExists);
        }

        let mut process = TabProcess::new(tab_id);
        process.start();
        self.processes.insert(tab_id, process);
        Ok(())
    }

    /// Spawn with custom capabilities
    pub fn spawn_with_capabilities(
        &mut self,
        tab_id: TabId,
        capabilities: TabCapabilities,
    ) -> Result<(), TabError> {
        self.spawn_tab_process(tab_id)?;
        if let Some(proc) = self.processes.get_mut(&tab_id) {
            proc.capabilities = capabilities;
        }
        Ok(())
    }

    /// Kill a tab process
    pub fn kill_tab_process(&mut self, tab_id: TabId) -> bool {
        if let Some(mut proc) = self.processes.remove(&tab_id) {
            proc.terminate();

            // Remove from broadcast channels
            for subscribers in self.broadcast_channels.values_mut() {
                subscribers.retain(|&id| id != tab_id);
            }

            // Queue lifecycle event
            self.message_queue.push(IpcMessage::TabEvent {
                tab_id,
                event: TabLifecycleEvent::Closed,
            });

            true
        } else {
            false
        }
    }

    /// Recover a crashed tab process (recreate its context)
    pub fn recover_from_crash(&mut self, tab_id: TabId) -> Result<(), TabError> {
        let proc = self
            .processes
            .get_mut(&tab_id)
            .ok_or(TabError::ProcessNotFound)?;

        if proc.state != TabProcessState::Crashed {
            return Err(TabError::InvalidState {
                expected: "crashed",
            });
        }

        // Reset VM and GC, keep limits and capabilities
        let limits = proc.limits.clone();
        let capabilities = proc.capabilities.clone();
        let origin = proc.origin.clone();
        let crash_count = proc.usage.crash_count;

        proc.vm = JsVm::new();
        proc.gc = GcHeap::new();
        proc.dom_api = DomApi::new();
        proc.inbox.clear();
        proc.state = TabProcessState::Running;
        proc.limits = limits;
        proc.capabilities = capabilities;
        proc.origin = origin;
        proc.usage = ResourceUsage::default();
        proc.usage.crash_count = crash_count;
        proc.last_error = None;

        self.total_crashes += 1;

        // Notify other tabs
        self.message_queue.push(IpcMessage::TabEvent {
            tab_id,
            event: TabLifecycleEvent::Recovered,
        });

        Ok(())
    }

    /// Restrict capabilities for a tab
    pub fn restrict_capabilities(&mut self, tab_id: TabId, capabilities: TabCapabilities) -> bool {
        if let Some(proc) = self.processes.get_mut(&tab_id) {
            proc.capabilities = capabilities;
            true
        } else {
            false
        }
    }

    /// Get a tab process
    pub fn get_process(&self, tab_id: TabId) -> Option<&TabProcess> {
        self.processes.get(&tab_id)
    }

    /// Get a tab process mutably
    pub fn get_process_mut(&mut self, tab_id: TabId) -> Option<&mut TabProcess> {
        self.processes.get_mut(&tab_id)
    }

    /// Send postMessage from one tab to another
    pub fn post_message(
        &mut self,
        source: TabId,
        target: TabId,
        data: &str,
    ) -> Result<(), TabError> {
        // Check source can send
        let source_proc = self
            .processes
            .get(&source)
            .ok_or(TabError::ProcessNotFound)?;
        if !source_proc.capabilities.can_post_message {
            return Err(TabError::CapabilityDenied {
                capability: "postMessage",
            });
        }
        let origin = source_proc.origin.clone();

        // Deliver to target
        let target_proc = self
            .processes
            .get_mut(&target)
            .ok_or(TabError::ProcessNotFound)?;

        target_proc.inbox.push(IpcMessage::PostMessage {
            source_tab: source,
            target_tab: target,
            origin,
            data: data.to_string(),
        });

        Ok(())
    }

    /// Subscribe a tab to a broadcast channel
    pub fn subscribe_broadcast(&mut self, tab_id: TabId, channel: &str) -> bool {
        if !self.processes.contains_key(&tab_id) {
            return false;
        }
        let subscribers = self
            .broadcast_channels
            .entry(channel.to_string())
            .or_default();
        if !subscribers.contains(&tab_id) {
            subscribers.push(tab_id);
        }
        true
    }

    /// Unsubscribe a tab from a broadcast channel
    pub fn unsubscribe_broadcast(&mut self, tab_id: TabId, channel: &str) {
        if let Some(subscribers) = self.broadcast_channels.get_mut(channel) {
            subscribers.retain(|&id| id != tab_id);
        }
    }

    /// Send a broadcast message to all subscribers of a channel
    pub fn broadcast_message(&mut self, source: TabId, channel: &str, data: &str) -> usize {
        let subscribers: Vec<TabId> = self
            .broadcast_channels
            .get(channel)
            .cloned()
            .unwrap_or_default();

        let mut delivered = 0;
        for &sub_id in &subscribers {
            if sub_id == source {
                continue; // Don't deliver to sender
            }
            if let Some(proc) = self.processes.get_mut(&sub_id) {
                proc.inbox.push(IpcMessage::BroadcastMessage {
                    source_tab: source,
                    channel: channel.to_string(),
                    data: data.to_string(),
                });
                delivered += 1;
            }
        }
        delivered
    }

    /// Set a value in shared storage for an origin
    pub fn storage_set(&mut self, origin: &str, key: &str, value: &str) {
        let old_value = self
            .shared_storage
            .entry(origin.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());

        // Generate storage events for all same-origin tabs
        let msg = IpcMessage::StorageEvent {
            key: key.to_string(),
            old_value,
            new_value: Some(value.to_string()),
        };

        let tab_ids: Vec<TabId> = self
            .processes
            .iter()
            .filter(|(_, proc)| proc.origin == origin)
            .map(|(&id, _)| id)
            .collect();

        for tab_id in tab_ids {
            if let Some(proc) = self.processes.get_mut(&tab_id) {
                proc.inbox.push(msg.clone());
            }
        }
    }

    /// Get a value from shared storage
    pub fn storage_get(&self, origin: &str, key: &str) -> Option<&String> {
        self.shared_storage.get(origin)?.get(key)
    }

    /// Remove a value from shared storage
    pub fn storage_remove(&mut self, origin: &str, key: &str) -> Option<String> {
        self.shared_storage.get_mut(origin)?.remove(key)
    }

    /// Tick all running processes
    pub fn tick_all(&mut self) {
        let tab_ids: Vec<TabId> = self.processes.keys().copied().collect();
        for tab_id in tab_ids {
            if let Some(proc) = self.processes.get_mut(&tab_id) {
                if proc.state == TabProcessState::Running {
                    if let Err(_e) = proc.tick() {
                        // Process crashed during tick, already marked
                    }
                }
            }
        }
    }

    /// Suspend all background tab processes
    pub fn suspend_background_tabs(&mut self, active_tab: TabId) {
        for (&id, proc) in self.processes.iter_mut() {
            if id != active_tab && proc.state == TabProcessState::Running {
                proc.suspend();
            }
        }
    }

    /// Resume a specific tab process
    pub fn resume_tab(&mut self, tab_id: TabId) -> bool {
        if let Some(proc) = self.processes.get_mut(&tab_id) {
            proc.resume();
            true
        } else {
            false
        }
    }

    /// Number of active processes
    pub fn active_count(&self) -> usize {
        self.processes
            .values()
            .filter(|p| p.state == TabProcessState::Running)
            .count()
    }

    /// Total number of processes
    pub fn total_count(&self) -> usize {
        self.processes.len()
    }

    /// Total crashes across all tabs
    pub fn total_crashes(&self) -> usize {
        self.total_crashes
    }

    /// Get resource usage for a tab
    pub fn resource_usage(&self, tab_id: TabId) -> Option<&ResourceUsage> {
        self.processes.get(&tab_id).map(|p| &p.usage)
    }

    /// Get aggregate resource usage across all tabs
    pub fn aggregate_usage(&self) -> ResourceUsage {
        let mut total = ResourceUsage::default();
        for proc in self.processes.values() {
            total.heap_bytes += proc.usage.heap_bytes;
            total.dom_node_count += proc.usage.dom_node_count;
            total.timer_count += proc.usage.timer_count;
            total.total_steps += proc.usage.total_steps;
            total.gc_collections += proc.usage.gc_collections;
            total.crash_count += proc.usage.crash_count;
        }
        total
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_process_new() {
        let proc = TabProcess::new(1);
        assert_eq!(proc.tab_id, 1);
        assert_eq!(proc.state, TabProcessState::Created);
    }

    #[test]
    fn test_tab_process_lifecycle() {
        let mut proc = TabProcess::new(1);
        proc.start();
        assert_eq!(proc.state, TabProcessState::Running);
        proc.suspend();
        assert_eq!(proc.state, TabProcessState::Suspended);
        proc.resume();
        assert_eq!(proc.state, TabProcessState::Running);
        proc.terminate();
        assert_eq!(proc.state, TabProcessState::Terminated);
    }

    #[test]
    fn test_tab_process_crash() {
        let mut proc = TabProcess::new(1);
        proc.start();
        proc.crash("out of memory");
        assert_eq!(proc.state, TabProcessState::Crashed);
        assert_eq!(proc.last_error.as_deref(), Some("out of memory"));
        assert_eq!(proc.usage.crash_count, 1);
    }

    #[test]
    fn test_tab_process_tick() {
        let mut proc = TabProcess::new(1);
        proc.start();
        assert!(proc.tick().is_ok());
        assert_eq!(proc.usage.ticks_processed, 1);
    }

    #[test]
    fn test_tab_process_tick_not_running() {
        let mut proc = TabProcess::new(1);
        // Still in Created state
        assert!(proc.tick().is_ok());
        assert_eq!(proc.usage.ticks_processed, 0);
    }

    #[test]
    fn test_tab_process_execute_script() {
        let mut proc = TabProcess::new(1);
        proc.start();
        assert!(proc.execute_script("let x = 42;").is_ok());
    }

    #[test]
    fn test_tab_process_execute_script_not_running() {
        let mut proc = TabProcess::new(1);
        assert!(proc.execute_script("let x = 1;").is_err());
    }

    #[test]
    fn test_tab_process_execute_js_disabled() {
        let mut proc = TabProcess::new(1);
        proc.start();
        proc.capabilities.can_execute_js = false;
        assert!(proc.execute_script("let x = 1;").is_err());
    }

    #[test]
    fn test_capabilities_default_web() {
        let caps = TabCapabilities::default_web();
        assert!(caps.can_execute_js);
        assert!(caps.can_local_storage);
        assert!(caps.can_timers);
        assert!(caps.can_network);
        assert!(!caps.can_geolocation);
        assert!(!caps.can_clipboard);
    }

    #[test]
    fn test_capabilities_sandboxed() {
        let caps = TabCapabilities::sandboxed();
        assert_eq!(caps.enabled_count(), 0);
    }

    #[test]
    fn test_capabilities_trusted() {
        let caps = TabCapabilities::trusted();
        assert_eq!(caps.enabled_count(), 10);
    }

    #[test]
    fn test_capabilities_sandbox_flags() {
        let mut caps = TabCapabilities::default_web();
        caps.apply_sandbox_flags("allow-scripts allow-popups");
        assert!(caps.can_execute_js);
        assert!(caps.can_popups);
        assert!(!caps.can_network);
        assert!(!caps.can_local_storage);
    }

    #[test]
    fn test_process_isolation_spawn() {
        let mut iso = ProcessIsolation::new();
        assert!(iso.spawn_tab_process(1).is_ok());
        assert_eq!(iso.total_count(), 1);
        assert_eq!(iso.active_count(), 1);
    }

    #[test]
    fn test_process_isolation_duplicate_spawn() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        assert!(iso.spawn_tab_process(1).is_err());
    }

    #[test]
    fn test_process_isolation_kill() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        assert!(iso.kill_tab_process(1));
        assert_eq!(iso.total_count(), 0);
        assert!(!iso.kill_tab_process(1)); // already gone
    }

    #[test]
    fn test_process_isolation_recover() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.get_process_mut(1).unwrap().crash("boom");
        assert!(iso.recover_from_crash(1).is_ok());
        assert_eq!(iso.get_process(1).unwrap().state, TabProcessState::Running);
        assert_eq!(iso.total_crashes(), 1);
    }

    #[test]
    fn test_process_isolation_recover_not_crashed() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        assert!(iso.recover_from_crash(1).is_err());
    }

    #[test]
    fn test_post_message() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        iso.get_process_mut(1)
            .unwrap()
            .set_origin("https://example.com");
        assert!(iso.post_message(1, 2, "hello").is_ok());
        assert_eq!(iso.get_process(2).unwrap().inbox.len(), 1);
    }

    #[test]
    fn test_broadcast_channel() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        iso.spawn_tab_process(3).unwrap();
        iso.subscribe_broadcast(1, "updates");
        iso.subscribe_broadcast(2, "updates");
        iso.subscribe_broadcast(3, "updates");

        let delivered = iso.broadcast_message(1, "updates", "data");
        assert_eq!(delivered, 2); // 2 and 3, not 1 (sender excluded)
    }

    #[test]
    fn test_shared_storage() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.get_process_mut(1)
            .unwrap()
            .set_origin("https://example.com");

        iso.storage_set("https://example.com", "key1", "value1");
        assert_eq!(
            iso.storage_get("https://example.com", "key1"),
            Some(&"value1".to_string())
        );

        iso.storage_remove("https://example.com", "key1");
        assert!(iso.storage_get("https://example.com", "key1").is_none());
    }

    #[test]
    fn test_suspend_background_tabs() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        iso.spawn_tab_process(3).unwrap();

        iso.suspend_background_tabs(2);
        assert_eq!(
            iso.get_process(1).unwrap().state,
            TabProcessState::Suspended
        );
        assert_eq!(iso.get_process(2).unwrap().state, TabProcessState::Running);
        assert_eq!(
            iso.get_process(3).unwrap().state,
            TabProcessState::Suspended
        );
    }

    #[test]
    fn test_resume_tab() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.get_process_mut(1).unwrap().suspend();
        assert!(iso.resume_tab(1));
        assert_eq!(iso.get_process(1).unwrap().state, TabProcessState::Running);
    }

    #[test]
    fn test_restrict_capabilities() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        let caps = TabCapabilities::sandboxed();
        assert!(iso.restrict_capabilities(1, caps));
        assert_eq!(iso.get_process(1).unwrap().capabilities.enabled_count(), 0);
    }

    #[test]
    fn test_aggregate_usage() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        let agg = iso.aggregate_usage();
        assert_eq!(agg.crash_count, 0);
    }

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_heap_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_dom_nodes, 100_000);
    }

    #[test]
    fn test_same_origin() {
        let mut proc = TabProcess::new(1);
        proc.set_origin("https://example.com");
        assert!(proc.is_same_origin("https://example.com"));
        assert!(!proc.is_same_origin("https://other.com"));
    }

    #[test]
    fn test_tick_all() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        iso.tick_all();
        assert_eq!(iso.get_process(1).unwrap().usage.ticks_processed, 1);
        assert_eq!(iso.get_process(2).unwrap().usage.ticks_processed, 1);
    }

    #[test]
    fn test_max_processes() {
        let mut iso = ProcessIsolation::new();
        iso.max_processes = 2;
        iso.spawn_tab_process(1).unwrap();
        iso.spawn_tab_process(2).unwrap();
        assert!(iso.spawn_tab_process(3).is_err());
    }

    #[test]
    fn test_spawn_with_capabilities() {
        let mut iso = ProcessIsolation::new();
        let caps = TabCapabilities::trusted();
        iso.spawn_with_capabilities(1, caps).unwrap();
        assert_eq!(iso.get_process(1).unwrap().capabilities.enabled_count(), 10);
    }

    #[test]
    fn test_unsubscribe_broadcast() {
        let mut iso = ProcessIsolation::new();
        iso.spawn_tab_process(1).unwrap();
        iso.subscribe_broadcast(1, "ch");
        iso.unsubscribe_broadcast(1, "ch");
        let delivered = iso.broadcast_message(2, "ch", "data");
        assert_eq!(delivered, 0);
    }
}
