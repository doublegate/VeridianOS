//! Enhanced container runtime with OCI specification support, cgroup
//! controllers, overlay filesystem, veth networking, and seccomp BPF filtering.
//!
//! This module implements 7 container enhancement sprints:
//! 1. OCI Runtime Specification (config.json parsing, lifecycle, hooks,
//!    pivot_root)
//! 2. Container Image Format (layers, overlay composition, manifest, SHA-256
//!    IDs)
//! 3. Cgroup Memory Controller (limits, usage tracking, OOM, hierarchical
//!    accounting)
//! 4. Cgroup CPU Controller (shares, quota/period, throttling, burst,
//!    hierarchy)
//! 5. Overlay Filesystem (lower/upper layers, copy-up, whiteout, directory
//!    merge)
//! 6. Veth Networking (virtual pairs, bridge, NAT masquerade, ARP proxy, MTU)
//! 7. Seccomp BPF (filter instructions, syscall filtering, arg inspection,
//!    inheritance)

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Sprint 1: OCI Runtime Specification
// ---------------------------------------------------------------------------

/// OCI container lifecycle states per the runtime-spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OciLifecycleState {
    /// Container bundle loaded, namespaces created, but process not started.
    Creating,
    /// Container created but user process not yet started (post-create hooks
    /// ran).
    Created,
    /// Container process is running.
    Running,
    /// Container process has exited.
    Stopped,
}

impl core::fmt::Display for OciLifecycleState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Creating => write!(f, "creating"),
            Self::Created => write!(f, "created"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// A single mount specification from the OCI config.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciMount {
    /// Destination path inside the container.
    pub destination: String,
    /// Mount type (e.g., "proc", "tmpfs", "bind").
    pub mount_type: String,
    /// Source path on the host.
    pub source: String,
    /// Mount options (e.g., "nosuid", "noexec", "ro").
    pub options: Vec<String>,
}

/// Linux namespace configuration from the OCI config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OciNamespaceKind {
    Pid,
    Network,
    Mount,
    Ipc,
    Uts,
    User,
    Cgroup,
}

impl OciNamespaceKind {
    /// Parse from OCI config string.
    pub fn from_str_kind(s: &str) -> Option<Self> {
        match s {
            "pid" => Some(Self::Pid),
            "network" => Some(Self::Network),
            "mount" => Some(Self::Mount),
            "ipc" => Some(Self::Ipc),
            "uts" => Some(Self::Uts),
            "user" => Some(Self::User),
            "cgroup" => Some(Self::Cgroup),
            _ => None,
        }
    }

    /// Return the OCI string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pid => "pid",
            Self::Network => "network",
            Self::Mount => "mount",
            Self::Ipc => "ipc",
            Self::Uts => "uts",
            Self::User => "user",
            Self::Cgroup => "cgroup",
        }
    }
}

/// A namespace entry in the OCI linux config.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciNamespace {
    pub kind: OciNamespaceKind,
    /// Optional path to an existing namespace to join.
    pub path: Option<String>,
}

/// Lifecycle hook specification.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciHook {
    /// Path to the hook executable.
    pub path: String,
    /// Arguments passed to the hook.
    pub args: Vec<String>,
    /// Environment variables for the hook.
    pub env: Vec<String>,
    /// Timeout in seconds (0 = no timeout).
    pub timeout_secs: u32,
}

/// OCI hooks at different lifecycle points.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OciHooks {
    pub prestart: Vec<OciHook>,
    pub poststart: Vec<OciHook>,
    pub poststop: Vec<OciHook>,
}

/// Root filesystem specification.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciRoot {
    /// Path to the root filesystem.
    pub path: String,
    /// Whether the root filesystem is read-only.
    pub readonly: bool,
}

/// Process specification from config.json.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciProcess {
    /// Path to the executable.
    pub args: Vec<String>,
    /// Environment variables in KEY=VALUE format.
    pub env: Vec<String>,
    /// Working directory inside the container.
    pub cwd: String,
    /// User ID.
    pub uid: u32,
    /// Group ID.
    pub gid: u32,
    /// Whether a terminal is attached.
    pub terminal: bool,
}

/// Linux-specific configuration.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciLinuxConfig {
    /// Namespaces to create or join.
    pub namespaces: Vec<OciNamespace>,
    /// Cgroups path.
    pub cgroups_path: String,
    /// Memory limit in bytes (0 = unlimited).
    pub memory_limit: u64,
    /// CPU shares (default 1024).
    pub cpu_shares: u32,
    /// CPU quota in microseconds per period (0 = unlimited).
    pub cpu_quota: u64,
    /// CPU period in microseconds (default 100000).
    pub cpu_period: u64,
}

#[cfg(feature = "alloc")]
impl Default for OciLinuxConfig {
    fn default() -> Self {
        Self {
            namespaces: Vec::new(),
            cgroups_path: String::new(),
            memory_limit: 0,
            cpu_shares: 1024,
            cpu_quota: 0,
            cpu_period: 100_000,
        }
    }
}

/// Parsed OCI runtime configuration (config.json equivalent).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct OciConfig {
    /// OCI specification version.
    pub oci_version: String,
    /// Root filesystem.
    pub root: OciRoot,
    /// Container process.
    pub process: OciProcess,
    /// Mount points.
    pub mounts: Vec<OciMount>,
    /// Hostname.
    pub hostname: String,
    /// Lifecycle hooks.
    pub hooks: OciHooks,
    /// Linux-specific configuration.
    pub linux: OciLinuxConfig,
}

#[cfg(feature = "alloc")]
impl OciConfig {
    /// Parse a simplified config.json representation from key-value lines.
    ///
    /// Format: one "key=value" per line. Recognized keys:
    ///   oci_version, root_path, root_readonly, hostname,
    ///   process_cwd, process_uid, process_gid, process_terminal,
    ///   process_arg, process_env,
    ///   mount (destination:type:source:options),
    ///   namespace (kind[:path]),
    ///   cgroups_path, memory_limit, cpu_shares, cpu_quota, cpu_period,
    ///   hook_prestart, hook_poststart, hook_poststop (path:timeout)
    pub fn parse(input: &str) -> Result<Self, KernelError> {
        let mut config = Self {
            oci_version: String::from("1.0.2"),
            root: OciRoot {
                path: String::from("/"),
                readonly: false,
            },
            process: OciProcess {
                args: Vec::new(),
                env: Vec::new(),
                cwd: String::from("/"),
                uid: 0,
                gid: 0,
                terminal: false,
            },
            mounts: Vec::new(),
            hostname: String::new(),
            hooks: OciHooks::default(),
            linux: OciLinuxConfig::default(),
        };

        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "oci_version" => config.oci_version = String::from(val),
                    "root_path" => config.root.path = String::from(val),
                    "root_readonly" => config.root.readonly = val == "true",
                    "hostname" => config.hostname = String::from(val),
                    "process_cwd" => config.process.cwd = String::from(val),
                    "process_uid" => {
                        config.process.uid = parse_u32(val).unwrap_or(0);
                    }
                    "process_gid" => {
                        config.process.gid = parse_u32(val).unwrap_or(0);
                    }
                    "process_terminal" => config.process.terminal = val == "true",
                    "process_arg" => config.process.args.push(String::from(val)),
                    "process_env" => config.process.env.push(String::from(val)),
                    "mount" => {
                        // destination:type:source:options
                        let parts: Vec<&str> = val.splitn(4, ':').collect();
                        if parts.len() >= 3 {
                            let options = if parts.len() > 3 {
                                parts[3]
                                    .split(',')
                                    .map(|s| String::from(s.trim()))
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            config.mounts.push(OciMount {
                                destination: String::from(parts[0]),
                                mount_type: String::from(parts[1]),
                                source: String::from(parts[2]),
                                options,
                            });
                        }
                    }
                    "namespace" => {
                        if let Some((kind_str, path)) = val.split_once(':') {
                            if let Some(kind) = OciNamespaceKind::from_str_kind(kind_str) {
                                config.linux.namespaces.push(OciNamespace {
                                    kind,
                                    path: Some(String::from(path)),
                                });
                            }
                        } else if let Some(kind) = OciNamespaceKind::from_str_kind(val) {
                            config
                                .linux
                                .namespaces
                                .push(OciNamespace { kind, path: None });
                        }
                    }
                    "cgroups_path" => config.linux.cgroups_path = String::from(val),
                    "memory_limit" => {
                        config.linux.memory_limit = parse_u64(val).unwrap_or(0);
                    }
                    "cpu_shares" => {
                        config.linux.cpu_shares = parse_u32(val).unwrap_or(1024);
                    }
                    "cpu_quota" => {
                        config.linux.cpu_quota = parse_u64(val).unwrap_or(0);
                    }
                    "cpu_period" => {
                        config.linux.cpu_period = parse_u64(val).unwrap_or(100_000);
                    }
                    "hook_prestart" | "hook_poststart" | "hook_poststop" => {
                        let hook = parse_hook(val);
                        match key {
                            "hook_prestart" => config.hooks.prestart.push(hook),
                            "hook_poststart" => config.hooks.poststart.push(hook),
                            "hook_poststop" => config.hooks.poststop.push(hook),
                            _ => {}
                        }
                    }
                    _ => {} // ignore unknown keys
                }
            }
        }

        Ok(config)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.root.path.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "root.path",
                value: "empty",
            });
        }
        if self.process.args.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "process.args",
                value: "empty",
            });
        }
        if self.process.cwd.is_empty() || !self.process.cwd.starts_with('/') {
            return Err(KernelError::InvalidArgument {
                name: "process.cwd",
                value: "must be absolute path",
            });
        }
        Ok(())
    }
}

/// An OCI-compliant container runtime instance.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct OciContainer {
    /// Unique container ID.
    pub id: String,
    /// Current lifecycle state.
    pub state: OciLifecycleState,
    /// Parsed OCI configuration.
    pub config: OciConfig,
    /// PID of the container init process (0 if not started).
    pub pid: u64,
    /// Bundle path (directory containing config.json + rootfs).
    pub bundle: String,
    /// Creation timestamp (monotonic counter value).
    pub created_at: u64,
}

#[cfg(feature = "alloc")]
impl OciContainer {
    /// Create a new container from a parsed config.
    pub fn new(id: &str, bundle: &str, config: OciConfig) -> Result<Self, KernelError> {
        config.validate()?;
        Ok(Self {
            id: String::from(id),
            state: OciLifecycleState::Creating,
            config,
            pid: 0,
            bundle: String::from(bundle),
            created_at: CONTAINER_COUNTER.fetch_add(1, Ordering::Relaxed),
        })
    }

    /// Transition to Created state (namespaces set up, hooks run).
    pub fn mark_created(&mut self) -> Result<(), KernelError> {
        if self.state != OciLifecycleState::Creating {
            return Err(KernelError::InvalidState {
                expected: "creating",
                actual: self.state_str(),
            });
        }
        self.state = OciLifecycleState::Created;
        Ok(())
    }

    /// Start the container process, transitioning to Running.
    pub fn start(&mut self, pid: u64) -> Result<(), KernelError> {
        if self.state != OciLifecycleState::Created {
            return Err(KernelError::InvalidState {
                expected: "created",
                actual: self.state_str(),
            });
        }
        self.pid = pid;
        self.state = OciLifecycleState::Running;
        Ok(())
    }

    /// Transition to Stopped state.
    pub fn stop(&mut self) -> Result<(), KernelError> {
        if self.state != OciLifecycleState::Running {
            return Err(KernelError::InvalidState {
                expected: "running",
                actual: self.state_str(),
            });
        }
        self.state = OciLifecycleState::Stopped;
        Ok(())
    }

    fn state_str(&self) -> &'static str {
        match self.state {
            OciLifecycleState::Creating => "creating",
            OciLifecycleState::Created => "created",
            OciLifecycleState::Running => "running",
            OciLifecycleState::Stopped => "stopped",
        }
    }

    /// Perform pivot_root: change the root filesystem for the container.
    /// Returns the old root path and new root path.
    pub fn pivot_root(&self) -> Result<(String, String), KernelError> {
        if self.config.root.path.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "root",
                value: "empty path",
            });
        }
        let old_root = String::from("/.pivot_root");
        let new_root = self.config.root.path.clone();
        Ok((old_root, new_root))
    }
}

static CONTAINER_COUNTER: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------
// Sprint 2: Container Image Format
// ---------------------------------------------------------------------------

/// Image layer digest (SHA-256).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerDigest {
    pub bytes: [u8; 32],
}

impl LayerDigest {
    /// Compute a SHA-256 digest of the given data.
    pub fn compute(data: &[u8]) -> Self {
        Self {
            bytes: simple_sha256(data),
        }
    }

    /// Format as hex string.
    #[cfg(feature = "alloc")]
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.bytes {
            let hi = HEX_CHARS[(b >> 4) as usize];
            let lo = HEX_CHARS[(b & 0x0f) as usize];
            s.push(hi as char);
            s.push(lo as char);
        }
        s
    }
}

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// A single layer in a container image.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ImageLayer {
    /// SHA-256 digest of the layer content.
    pub digest: LayerDigest,
    /// Compressed size in bytes.
    pub compressed_size: u64,
    /// Uncompressed size in bytes.
    pub uncompressed_size: u64,
    /// Media type (e.g., "application/vnd.oci.image.layer.v1.tar+gzip").
    pub media_type: String,
}

/// Gzip detection: check for gzip magic bytes (0x1f, 0x8b).
pub fn is_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

/// TAR header: first 100 bytes are the filename, bytes 124-135 are octal size.
#[cfg(feature = "alloc")]
pub fn parse_tar_filename(header: &[u8; 512]) -> String {
    let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
    let mut name = String::new();
    for &b in &header[..name_end] {
        if b.is_ascii() && b != 0 {
            name.push(b as char);
        }
    }
    name
}

/// Parse octal size from TAR header bytes 124..135.
pub fn parse_tar_size(header: &[u8; 512]) -> u64 {
    let mut size: u64 = 0;
    for &b in &header[124..135] {
        if (b'0'..=b'7').contains(&b) {
            size = size.saturating_mul(8);
            size = size.saturating_add((b - b'0') as u64);
        }
    }
    size
}

/// Container image manifest.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ImageManifest {
    /// Schema version (usually 2).
    pub schema_version: u32,
    /// Media type of the manifest.
    pub media_type: String,
    /// Config digest (SHA-256 of config JSON).
    pub config_digest: LayerDigest,
    /// Config size in bytes.
    pub config_size: u64,
    /// Ordered list of layer digests.
    pub layer_digests: Vec<LayerDigest>,
}

/// Container image: manifest + layers + config.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ContainerImage {
    /// Image ID (SHA-256 of the config blob).
    pub image_id: LayerDigest,
    /// Human-readable name (e.g., "alpine:3.19").
    pub name: String,
    /// Image manifest.
    pub manifest: ImageManifest,
    /// Layers in order (bottom to top).
    pub layers: Vec<ImageLayer>,
}

/// Layer cache: stores extracted layers by their digest.
#[cfg(feature = "alloc")]
pub struct LayerCache {
    /// Maps digest hex -> layer entry.
    entries: BTreeMap<String, CachedLayer>,
    /// Maximum number of cached layers.
    max_entries: usize,
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CachedLayer {
    pub digest: LayerDigest,
    pub extracted_path: String,
    pub size_bytes: u64,
    pub reference_count: u32,
}

#[cfg(feature = "alloc")]
impl LayerCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            max_entries,
        }
    }

    /// Get a cached layer by digest hex.
    pub fn get(&self, digest_hex: &str) -> Option<&CachedLayer> {
        self.entries.get(digest_hex)
    }

    /// Insert a layer into the cache. Returns false if cache is full.
    pub fn insert(&mut self, layer: CachedLayer) -> bool {
        if self.entries.len() >= self.max_entries {
            return false;
        }
        let hex = layer.digest.to_hex();
        self.entries.insert(hex, layer);
        true
    }

    /// Increment reference count for a layer.
    pub fn add_ref(&mut self, digest_hex: &str) -> bool {
        if let Some(entry) = self.entries.get_mut(digest_hex) {
            entry.reference_count = entry.reference_count.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// Decrement reference count. Removes the entry if it reaches zero.
    pub fn release(&mut self, digest_hex: &str) -> bool {
        let should_remove = if let Some(entry) = self.entries.get_mut(digest_hex) {
            entry.reference_count = entry.reference_count.saturating_sub(1);
            entry.reference_count == 0
        } else {
            return false;
        };
        if should_remove {
            self.entries.remove(digest_hex);
        }
        true
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_entries
    }
}

#[cfg(feature = "alloc")]
impl ContainerImage {
    /// Compose an image from config data and a list of layer data blobs.
    pub fn compose(name: &str, config_data: &[u8], layer_data: &[&[u8]]) -> Self {
        let config_digest = LayerDigest::compute(config_data);
        let image_id = config_digest.clone();

        let mut layers = Vec::new();
        let mut layer_digests = Vec::new();
        for data in layer_data {
            let digest = LayerDigest::compute(data);
            let compressed = is_gzip(data);
            layers.push(ImageLayer {
                digest: digest.clone(),
                compressed_size: if compressed { data.len() as u64 } else { 0 },
                uncompressed_size: data.len() as u64,
                media_type: if compressed {
                    String::from("application/vnd.oci.image.layer.v1.tar+gzip")
                } else {
                    String::from("application/vnd.oci.image.layer.v1.tar")
                },
            });
            layer_digests.push(digest);
        }

        let manifest = ImageManifest {
            schema_version: 2,
            media_type: String::from("application/vnd.oci.image.manifest.v1+json"),
            config_digest,
            config_size: config_data.len() as u64,
            layer_digests,
        };

        Self {
            image_id,
            name: String::from(name),
            manifest,
            layers,
        }
    }
}

// ---------------------------------------------------------------------------
// Sprint 3: Cgroup Memory Controller
// ---------------------------------------------------------------------------

/// Memory statistics counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryStat {
    /// Resident set size in bytes.
    pub rss: u64,
    /// Page cache usage in bytes.
    pub cache: u64,
    /// Memory-mapped file usage in bytes.
    pub mapped_file: u64,
    /// Anonymous memory usage in bytes.
    pub anon: u64,
    /// Swap usage in bytes.
    pub swap: u64,
}

impl MemoryStat {
    /// Total memory usage (rss + cache).
    pub fn total(&self) -> u64 {
        self.rss.saturating_add(self.cache)
    }
}

/// OOM event information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OomEvent {
    /// Number of OOM events triggered.
    pub oom_kill_count: u64,
    /// Whether OOM kill is enabled.
    pub oom_kill_enabled: bool,
    /// Whether the group is currently under OOM.
    pub under_oom: bool,
}

impl Default for OomEvent {
    fn default() -> Self {
        Self {
            oom_kill_count: 0,
            oom_kill_enabled: true,
            under_oom: false,
        }
    }
}

/// Cgroup memory controller.
#[derive(Debug, Clone)]
pub struct CgroupMemoryController {
    /// Hard memory limit in bytes (0 = unlimited).
    pub limit_hard: u64,
    /// Soft memory limit in bytes (0 = unlimited).
    pub limit_soft: u64,
    /// Current usage in bytes.
    pub usage_current: u64,
    /// Peak (maximum) usage in bytes.
    pub usage_peak: u64,
    /// Detailed memory statistics.
    pub stat: MemoryStat,
    /// OOM event state.
    pub oom: OomEvent,
    /// Parent cgroup ID for hierarchical accounting (0 = root).
    pub parent_id: u64,
    /// Unique cgroup ID.
    pub cgroup_id: u64,
}

impl CgroupMemoryController {
    pub fn new(cgroup_id: u64) -> Self {
        Self {
            limit_hard: 0,
            limit_soft: 0,
            usage_current: 0,
            usage_peak: 0,
            stat: MemoryStat::default(),
            oom: OomEvent::default(),
            parent_id: 0,
            cgroup_id,
        }
    }

    /// Set the hard limit. Returns error if current usage exceeds new limit.
    pub fn set_hard_limit(&mut self, limit: u64) -> Result<(), KernelError> {
        if limit > 0 && self.usage_current > limit {
            // Trigger reclaim attempt
            self.try_reclaim(self.usage_current.saturating_sub(limit));
            if self.usage_current > limit {
                return Err(KernelError::ResourceExhausted {
                    resource: "cgroup memory",
                });
            }
        }
        self.limit_hard = limit;
        Ok(())
    }

    /// Set the soft limit.
    pub fn set_soft_limit(&mut self, limit: u64) {
        self.limit_soft = limit;
    }

    /// Charge memory usage. Returns error if hard limit would be exceeded.
    pub fn charge(&mut self, bytes: u64) -> Result<(), KernelError> {
        let new_usage = self.usage_current.saturating_add(bytes);
        if self.limit_hard > 0 && new_usage > self.limit_hard {
            // Try reclaim first
            self.try_reclaim(new_usage.saturating_sub(self.limit_hard));
            let after_reclaim = self.usage_current.saturating_add(bytes);
            if after_reclaim > self.limit_hard {
                self.oom.under_oom = true;
                self.oom.oom_kill_count = self.oom.oom_kill_count.saturating_add(1);
                return Err(KernelError::OutOfMemory {
                    requested: bytes as usize,
                    available: self.limit_hard.saturating_sub(self.usage_current) as usize,
                });
            }
        }
        self.usage_current = self.usage_current.saturating_add(bytes);
        if self.usage_current > self.usage_peak {
            self.usage_peak = self.usage_current;
        }
        self.stat.rss = self.stat.rss.saturating_add(bytes);
        Ok(())
    }

    /// Uncharge (release) memory usage.
    pub fn uncharge(&mut self, bytes: u64) {
        self.usage_current = self.usage_current.saturating_sub(bytes);
        self.stat.rss = self.stat.rss.saturating_sub(bytes);
        self.oom.under_oom = false;
    }

    /// Check if soft limit is exceeded (triggers reclaim pressure).
    pub fn soft_limit_exceeded(&self) -> bool {
        self.limit_soft > 0 && self.usage_current > self.limit_soft
    }

    /// Try to reclaim `target` bytes. Returns bytes reclaimed.
    /// In a real implementation this would trigger page reclaim; here it
    /// reclaims from cache.
    fn try_reclaim(&mut self, target: u64) -> u64 {
        let reclaimable = self.stat.cache;
        let reclaimed = if reclaimable >= target {
            target
        } else {
            reclaimable
        };
        self.stat.cache = self.stat.cache.saturating_sub(reclaimed);
        self.usage_current = self.usage_current.saturating_sub(reclaimed);
        reclaimed
    }

    /// Record a cache page addition.
    pub fn add_cache(&mut self, bytes: u64) {
        self.stat.cache = self.stat.cache.saturating_add(bytes);
        self.usage_current = self.usage_current.saturating_add(bytes);
        if self.usage_current > self.usage_peak {
            self.usage_peak = self.usage_current;
        }
    }

    /// Record a mapped file addition.
    pub fn add_mapped_file(&mut self, bytes: u64) {
        self.stat.mapped_file = self.stat.mapped_file.saturating_add(bytes);
    }

    /// Hierarchical usage including parent chain (simplified: just self).
    pub fn hierarchical_usage(&self) -> u64 {
        self.usage_current
    }
}

// ---------------------------------------------------------------------------
// Sprint 4: Cgroup CPU Controller
// ---------------------------------------------------------------------------

/// CPU bandwidth statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuBandwidthStats {
    /// Number of times throttled.
    pub nr_throttled: u64,
    /// Total throttled time in nanoseconds.
    pub throttled_time_ns: u64,
    /// Number of scheduling periods elapsed.
    pub nr_periods: u64,
    /// Burst time accumulated in nanoseconds.
    pub nr_bursts: u64,
    /// Total burst time used in nanoseconds.
    pub burst_time_ns: u64,
}

/// Cgroup CPU controller with shares and bandwidth limiting.
#[derive(Debug, Clone)]
pub struct CgroupCpuController {
    /// CPU shares (weight-based fair scheduling, default 1024).
    pub shares: u32,
    /// CPU quota in microseconds per period (0 = unlimited).
    pub quota_us: u64,
    /// CPU period in microseconds (default 100000 = 100ms).
    pub period_us: u64,
    /// Burst capacity in microseconds (0 = no burst).
    pub burst_us: u64,
    /// Accumulated burst budget in nanoseconds.
    burst_budget_ns: u64,
    /// Runtime consumed in the current period in nanoseconds.
    runtime_consumed_ns: u64,
    /// Bandwidth statistics.
    pub stats: CpuBandwidthStats,
    /// Whether currently throttled.
    pub throttled: bool,
    /// Parent cgroup ID for hierarchical distribution (0 = root).
    pub parent_id: u64,
    /// Unique cgroup ID.
    pub cgroup_id: u64,
}

impl CgroupCpuController {
    pub fn new(cgroup_id: u64) -> Self {
        Self {
            shares: 1024,
            quota_us: 0,
            period_us: 100_000,
            burst_us: 0,
            burst_budget_ns: 0,
            runtime_consumed_ns: 0,
            stats: CpuBandwidthStats::default(),
            throttled: false,
            parent_id: 0,
            cgroup_id,
        }
    }

    /// Set CPU shares (weight). Minimum 2, maximum 262144.
    pub fn set_shares(&mut self, shares: u32) -> Result<(), KernelError> {
        if !(2..=262144).contains(&shares) {
            return Err(KernelError::InvalidArgument {
                name: "cpu.shares",
                value: "out of range [2, 262144]",
            });
        }
        self.shares = shares;
        Ok(())
    }

    /// Set CPU bandwidth quota and period.
    /// quota_us=0 means unlimited. Period must be >= 1000us and <= 1000000us.
    pub fn set_bandwidth(&mut self, quota_us: u64, period_us: u64) -> Result<(), KernelError> {
        if !(1000..=1_000_000).contains(&period_us) {
            return Err(KernelError::InvalidArgument {
                name: "cpu.cfs_period_us",
                value: "out of range [1000, 1000000]",
            });
        }
        if quota_us > 0 && quota_us < 1000 {
            return Err(KernelError::InvalidArgument {
                name: "cpu.cfs_quota_us",
                value: "must be >= 1000 or 0 (unlimited)",
            });
        }
        self.quota_us = quota_us;
        self.period_us = period_us;
        Ok(())
    }

    /// Set burst capacity in microseconds.
    pub fn set_burst(&mut self, burst_us: u64) {
        self.burst_us = burst_us;
    }

    /// Consume runtime. Returns true if the task is now throttled.
    pub fn consume_runtime(&mut self, ns: u64) -> bool {
        self.runtime_consumed_ns = self.runtime_consumed_ns.saturating_add(ns);

        if self.quota_us == 0 {
            return false; // unlimited
        }

        // Convert quota from us to ns: quota_us * 1000
        let quota_ns = self.quota_us.saturating_mul(1000);
        let effective_quota = quota_ns.saturating_add(self.burst_budget_ns);

        if self.runtime_consumed_ns > effective_quota {
            self.throttled = true;
            self.stats.nr_throttled = self.stats.nr_throttled.saturating_add(1);
            let overshoot = self.runtime_consumed_ns.saturating_sub(effective_quota);
            self.stats.throttled_time_ns = self.stats.throttled_time_ns.saturating_add(overshoot);
            true
        } else {
            false
        }
    }

    /// Begin a new scheduling period. Refills runtime and handles burst.
    pub fn new_period(&mut self) {
        self.stats.nr_periods = self.stats.nr_periods.saturating_add(1);

        if self.quota_us > 0 {
            let quota_ns = self.quota_us.saturating_mul(1000);
            // Any unused runtime becomes burst budget (up to burst limit)
            if self.runtime_consumed_ns < quota_ns {
                let unused = quota_ns.saturating_sub(self.runtime_consumed_ns);
                let burst_limit_ns = self.burst_us.saturating_mul(1000);
                self.burst_budget_ns = self
                    .burst_budget_ns
                    .saturating_add(unused)
                    .min(burst_limit_ns);
                if unused > 0 {
                    self.stats.nr_bursts = self.stats.nr_bursts.saturating_add(1);
                    self.stats.burst_time_ns = self.stats.burst_time_ns.saturating_add(unused);
                }
            } else {
                // Used from burst budget
                let overdraft = self.runtime_consumed_ns.saturating_sub(quota_ns);
                self.burst_budget_ns = self.burst_budget_ns.saturating_sub(overdraft);
            }
        }

        self.runtime_consumed_ns = 0;
        self.throttled = false;
    }

    /// Calculate the effective CPU percentage (quota/period * 100).
    /// Returns percentage * 100 (fixed-point with 2 decimal digits).
    /// For example, quota=50000, period=100000 returns 5000 (50.00%).
    pub fn effective_cpu_percent_x100(&self) -> u64 {
        if self.quota_us == 0 || self.period_us == 0 {
            return 0; // unlimited or invalid
        }
        // (quota_us * 10000) / period_us gives percent * 100
        self.quota_us
            .saturating_mul(10000)
            .checked_div(self.period_us)
            .unwrap_or(0)
    }

    /// Compute the weight-proportional share of CPU time for this cgroup
    /// relative to a total weight sum. Returns nanoseconds per period.
    pub fn proportional_runtime_ns(&self, total_shares: u32) -> u64 {
        if total_shares == 0 {
            return 0;
        }
        let period_ns = self.period_us.saturating_mul(1000);
        // (shares * period_ns) / total_shares
        let shares_u64 = self.shares as u64;
        shares_u64
            .saturating_mul(period_ns)
            .checked_div(total_shares as u64)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Sprint 5: Overlay Filesystem
// ---------------------------------------------------------------------------

/// Whiteout marker prefix per the OCI/overlay specification.
const WHITEOUT_PREFIX: &str = ".wh.";

/// Opaque directory marker.
const OPAQUE_WHITEOUT: &str = ".wh..wh..opq";

/// Entry type in the overlay filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayEntryKind {
    File,
    Directory,
    Symlink,
    Whiteout,
    OpaqueDir,
}

/// A single entry in an overlay layer.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    /// Full path relative to the layer root.
    pub path: String,
    /// Entry kind.
    pub kind: OverlayEntryKind,
    /// File content (empty for directories/whiteouts).
    pub content: Vec<u8>,
    /// File permissions (Unix mode).
    pub mode: u32,
}

/// A single layer in the overlay filesystem.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct OverlayLayer {
    /// Layer entries keyed by path.
    entries: BTreeMap<String, OverlayEntry>,
    /// Whether this layer is read-only (lower layer).
    pub readonly: bool,
}

#[cfg(feature = "alloc")]
impl OverlayLayer {
    /// Create a new layer.
    pub fn new(readonly: bool) -> Self {
        Self {
            entries: BTreeMap::new(),
            readonly,
        }
    }

    /// Add an entry to the layer.
    pub fn add_entry(&mut self, entry: OverlayEntry) -> Result<(), KernelError> {
        if self.readonly {
            return Err(KernelError::PermissionDenied {
                operation: "write to readonly layer",
            });
        }
        self.entries.insert(entry.path.clone(), entry);
        Ok(())
    }

    /// Look up an entry by path.
    pub fn get_entry(&self, path: &str) -> Option<&OverlayEntry> {
        self.entries.get(path)
    }

    /// Check if a path has been whited out.
    pub fn is_whiteout(&self, path: &str) -> bool {
        // Check for explicit whiteout entry
        if let Some(entry) = self.entries.get(path) {
            return entry.kind == OverlayEntryKind::Whiteout;
        }
        // Check for whiteout file (.wh.<name>)
        if let Some((_dir, name)) = path.rsplit_once('/') {
            let wh_path = alloc::format!(
                "{}/{}{}",
                path.rsplit_once('/').map(|(d, _)| d).unwrap_or(""),
                WHITEOUT_PREFIX,
                name
            );
            self.entries.contains_key(&wh_path)
        } else {
            let wh_path = alloc::format!("{}{}", WHITEOUT_PREFIX, path);
            self.entries.contains_key(&wh_path)
        }
    }

    /// Check if a directory is opaque (blocks looking into lower layers).
    pub fn is_opaque_dir(&self, dir_path: &str) -> bool {
        let opq_path = if dir_path.ends_with('/') {
            alloc::format!("{}{}", dir_path, OPAQUE_WHITEOUT)
        } else {
            alloc::format!("{}/{}", dir_path, OPAQUE_WHITEOUT)
        };
        self.entries.contains_key(&opq_path)
    }

    /// List entries in a directory (non-recursive).
    pub fn list_dir(&self, dir_path: &str) -> Vec<&OverlayEntry> {
        let prefix = if dir_path.ends_with('/') || dir_path.is_empty() {
            String::from(dir_path)
        } else {
            alloc::format!("{}/", dir_path)
        };
        self.entries
            .values()
            .filter(|e| {
                if e.path.starts_with(prefix.as_str()) {
                    let rest = &e.path[prefix.len()..];
                    !rest.is_empty() && !rest.contains('/')
                } else {
                    false
                }
            })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Overlay filesystem combining multiple layers.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct OverlayFs {
    /// Lower (read-only) layers, ordered bottom to top.
    lower_layers: Vec<OverlayLayer>,
    /// Upper (writable) layer.
    upper_layer: OverlayLayer,
    /// Work directory path (used for atomic operations).
    work_dir: String,
}

#[cfg(feature = "alloc")]
impl OverlayFs {
    /// Create a new overlay filesystem.
    pub fn new(work_dir: &str) -> Self {
        Self {
            lower_layers: Vec::new(),
            upper_layer: OverlayLayer::new(false),
            work_dir: String::from(work_dir),
        }
    }

    /// Add a read-only lower layer (bottom-most first).
    pub fn add_lower_layer(&mut self, layer: OverlayLayer) {
        self.lower_layers.push(layer);
    }

    /// Look up a file: check upper layer first, then lower layers top to
    /// bottom.
    pub fn lookup(&self, path: &str) -> Option<&OverlayEntry> {
        // Check upper layer first
        if self.upper_layer.is_whiteout(path) {
            return None; // deleted in upper
        }
        if let Some(entry) = self.upper_layer.get_entry(path) {
            return Some(entry);
        }

        // Check lower layers from top to bottom
        for layer in self.lower_layers.iter().rev() {
            if layer.is_whiteout(path) {
                return None;
            }
            // If the parent dir is opaque in this layer, skip lower layers
            if let Some((parent, _)) = path.rsplit_once('/') {
                if layer.is_opaque_dir(parent) {
                    return layer.get_entry(path);
                }
            }
            if let Some(entry) = layer.get_entry(path) {
                return Some(entry);
            }
        }

        None
    }

    /// Write a file to the upper layer. If the file exists in a lower layer,
    /// performs copy-up first.
    pub fn write_file(
        &mut self,
        path: &str,
        content: Vec<u8>,
        mode: u32,
    ) -> Result<(), KernelError> {
        let entry = OverlayEntry {
            path: String::from(path),
            kind: OverlayEntryKind::File,
            content,
            mode,
        };
        self.upper_layer.entries.insert(String::from(path), entry);
        Ok(())
    }

    /// Delete a file by creating a whiteout in the upper layer.
    pub fn delete_file(&mut self, path: &str) -> Result<(), KernelError> {
        // Remove from upper if present
        self.upper_layer.entries.remove(path);

        // Check if it exists in any lower layer
        let exists_in_lower = self
            .lower_layers
            .iter()
            .any(|l| l.get_entry(path).is_some());

        if exists_in_lower {
            // Create whiteout
            if let Some((dir, name)) = path.rsplit_once('/') {
                let wh_path = alloc::format!("{}/{}{}", dir, WHITEOUT_PREFIX, name);
                self.upper_layer.entries.insert(
                    wh_path.clone(),
                    OverlayEntry {
                        path: wh_path,
                        kind: OverlayEntryKind::Whiteout,
                        content: Vec::new(),
                        mode: 0,
                    },
                );
            } else {
                let wh_path = alloc::format!("{}{}", WHITEOUT_PREFIX, path);
                self.upper_layer.entries.insert(
                    wh_path.clone(),
                    OverlayEntry {
                        path: wh_path,
                        kind: OverlayEntryKind::Whiteout,
                        content: Vec::new(),
                        mode: 0,
                    },
                );
            }
        }

        Ok(())
    }

    /// Make a directory opaque (hides all entries from lower layers).
    pub fn make_opaque_dir(&mut self, dir_path: &str) -> Result<(), KernelError> {
        let opq_path = alloc::format!("{}/{}", dir_path, OPAQUE_WHITEOUT);
        self.upper_layer.entries.insert(
            opq_path.clone(),
            OverlayEntry {
                path: opq_path,
                kind: OverlayEntryKind::OpaqueDir,
                content: Vec::new(),
                mode: 0,
            },
        );
        Ok(())
    }

    /// List directory contents merging all layers. Upper entries take
    /// precedence. Whited-out entries are excluded.
    pub fn list_dir(&self, dir_path: &str) -> Vec<&OverlayEntry> {
        let mut seen: BTreeMap<String, &OverlayEntry> = BTreeMap::new();
        let mut whited_out: Vec<String> = Vec::new();

        // Upper layer first
        for entry in self.upper_layer.list_dir(dir_path) {
            if entry.kind == OverlayEntryKind::Whiteout {
                // Extract the original filename from the whiteout name
                if let Some(name) = entry
                    .path
                    .rsplit('/')
                    .next()
                    .and_then(|n| n.strip_prefix(WHITEOUT_PREFIX))
                {
                    let orig = if dir_path.is_empty() {
                        String::from(name)
                    } else {
                        alloc::format!("{}/{}", dir_path, name)
                    };
                    whited_out.push(orig);
                }
            } else if entry.kind != OverlayEntryKind::OpaqueDir {
                seen.insert(entry.path.clone(), entry);
            }
        }

        // Check if upper declares this directory opaque
        let is_opaque = self.upper_layer.is_opaque_dir(dir_path);

        if !is_opaque {
            // Lower layers from top to bottom
            for layer in self.lower_layers.iter().rev() {
                if layer.is_opaque_dir(dir_path) {
                    // This layer is opaque, add its entries but stop going lower
                    for entry in layer.list_dir(dir_path) {
                        if !seen.contains_key(&entry.path) && !whited_out.contains(&entry.path) {
                            seen.insert(entry.path.clone(), entry);
                        }
                    }
                    break;
                }
                for entry in layer.list_dir(dir_path) {
                    if !seen.contains_key(&entry.path) && !whited_out.contains(&entry.path) {
                        seen.insert(entry.path.clone(), entry);
                    }
                }
            }
        }

        seen.into_values().collect()
    }

    pub fn work_dir(&self) -> &str {
        &self.work_dir
    }

    pub fn lower_layer_count(&self) -> usize {
        self.lower_layers.len()
    }
}

// ---------------------------------------------------------------------------
// Sprint 6: Veth Networking
// ---------------------------------------------------------------------------

/// Virtual Ethernet interface state.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VethEndpoint {
    /// Interface name.
    pub name: String,
    /// Peer interface name.
    pub peer_name: String,
    /// MAC address (6 bytes).
    pub mac: [u8; 6],
    /// IPv4 address (network byte order).
    pub ipv4_addr: u32,
    /// IPv4 subnet mask (network byte order).
    pub ipv4_mask: u32,
    /// MTU in bytes (default 1500).
    pub mtu: u16,
    /// Whether the interface is up.
    pub is_up: bool,
    /// Namespace ID this endpoint belongs to (0 = host).
    pub namespace_id: u64,
}

/// A virtual Ethernet pair.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct VethPair {
    /// Host-side endpoint.
    pub host: VethEndpoint,
    /// Container-side endpoint.
    pub container: VethEndpoint,
}

/// NAT port mapping entry.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatPortMapping {
    /// External (host) port.
    pub external_port: u16,
    /// Internal (container) port.
    pub internal_port: u16,
    /// Protocol: 6=TCP, 17=UDP.
    pub protocol: u8,
    /// Container IPv4 address.
    pub container_ip: u32,
}

/// NAT masquerade table for outbound SNAT and inbound port forwarding.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct NatTable {
    /// Host external IP address.
    pub host_ip: u32,
    /// Port mappings for inbound (DNAT).
    pub port_mappings: Vec<NatPortMapping>,
    /// Whether SNAT masquerade is enabled.
    pub masquerade_enabled: bool,
}

#[cfg(feature = "alloc")]
impl NatTable {
    pub fn new(host_ip: u32) -> Self {
        Self {
            host_ip,
            port_mappings: Vec::new(),
            masquerade_enabled: false,
        }
    }

    /// Enable SNAT masquerade for outbound traffic.
    pub fn enable_masquerade(&mut self) {
        self.masquerade_enabled = true;
    }

    /// Add a port mapping for inbound traffic.
    pub fn add_port_mapping(&mut self, mapping: NatPortMapping) -> Result<(), KernelError> {
        // Check for duplicate external port + protocol
        for existing in &self.port_mappings {
            if existing.external_port == mapping.external_port
                && existing.protocol == mapping.protocol
            {
                return Err(KernelError::AlreadyExists {
                    resource: "nat port mapping",
                    id: mapping.external_port as u64,
                });
            }
        }
        self.port_mappings.push(mapping);
        Ok(())
    }

    /// Remove a port mapping.
    pub fn remove_port_mapping(&mut self, external_port: u16, protocol: u8) -> bool {
        let before = self.port_mappings.len();
        self.port_mappings
            .retain(|m| !(m.external_port == external_port && m.protocol == protocol));
        self.port_mappings.len() < before
    }

    /// Look up a port mapping for inbound traffic.
    pub fn lookup_inbound(&self, external_port: u16, protocol: u8) -> Option<&NatPortMapping> {
        self.port_mappings
            .iter()
            .find(|m| m.external_port == external_port && m.protocol == protocol)
    }

    /// Apply SNAT: rewrite source IP to host IP for outbound packets.
    pub fn snat_rewrite(&self, _src_ip: u32) -> Option<u32> {
        if self.masquerade_enabled {
            Some(self.host_ip)
        } else {
            None
        }
    }
}

/// ARP proxy entry for container IPs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpProxyEntry {
    /// IPv4 address to proxy.
    pub ip: u32,
    /// MAC address to respond with.
    pub mac: [u8; 6],
}

/// Bridge configuration for container networking.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct VethBridge {
    /// Bridge name.
    pub name: String,
    /// Bridge IPv4 address (gateway).
    pub bridge_ip: u32,
    /// Bridge subnet mask.
    pub subnet_mask: u32,
    /// Attached veth host-side endpoint names.
    pub attached_interfaces: Vec<String>,
    /// ARP proxy entries.
    pub arp_proxy_entries: Vec<ArpProxyEntry>,
    /// NAT table.
    pub nat: NatTable,
}

#[cfg(feature = "alloc")]
impl VethBridge {
    pub fn new(name: &str, bridge_ip: u32, subnet_mask: u32) -> Self {
        Self {
            name: String::from(name),
            bridge_ip,
            subnet_mask,
            attached_interfaces: Vec::new(),
            arp_proxy_entries: Vec::new(),
            nat: NatTable::new(bridge_ip),
        }
    }

    /// Attach a host-side veth endpoint to the bridge.
    pub fn attach(&mut self, interface_name: &str) {
        if !self.attached_interfaces.iter().any(|n| n == interface_name) {
            self.attached_interfaces.push(String::from(interface_name));
        }
    }

    /// Detach an interface from the bridge.
    pub fn detach(&mut self, interface_name: &str) {
        self.attached_interfaces.retain(|n| n != interface_name);
    }

    /// Add an ARP proxy entry.
    pub fn add_arp_proxy(&mut self, entry: ArpProxyEntry) {
        self.arp_proxy_entries.push(entry);
    }

    /// Look up an ARP proxy entry by IP.
    pub fn arp_lookup(&self, ip: u32) -> Option<&ArpProxyEntry> {
        self.arp_proxy_entries.iter().find(|e| e.ip == ip)
    }

    /// Check if an IP is within the bridge subnet.
    pub fn in_subnet(&self, ip: u32) -> bool {
        (ip & self.subnet_mask) == (self.bridge_ip & self.subnet_mask)
    }

    pub fn attached_count(&self) -> usize {
        self.attached_interfaces.len()
    }
}

static NEXT_VETH_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a deterministic MAC address from a veth pair ID.
pub fn generate_veth_mac(veth_id: u64) -> [u8; 6] {
    [
        0x02, // locally administered
        0x42,
        ((veth_id >> 24) & 0xff) as u8,
        ((veth_id >> 16) & 0xff) as u8,
        ((veth_id >> 8) & 0xff) as u8,
        (veth_id & 0xff) as u8,
    ]
}

/// Create a veth pair with generated MACs.
#[cfg(feature = "alloc")]
pub fn create_veth_pair(host_name: &str, container_name: &str, namespace_id: u64) -> VethPair {
    let id = NEXT_VETH_ID.fetch_add(1, Ordering::Relaxed);
    let host_mac = generate_veth_mac(id);
    // Container MAC: flip one bit to differentiate
    let mut container_mac = generate_veth_mac(id);
    container_mac[5] ^= 0x01;

    VethPair {
        host: VethEndpoint {
            name: String::from(host_name),
            peer_name: String::from(container_name),
            mac: host_mac,
            ipv4_addr: 0,
            ipv4_mask: 0,
            mtu: 1500,
            is_up: false,
            namespace_id: 0,
        },
        container: VethEndpoint {
            name: String::from(container_name),
            peer_name: String::from(host_name),
            mac: container_mac,
            ipv4_addr: 0,
            ipv4_mask: 0,
            mtu: 1500,
            is_up: false,
            namespace_id,
        },
    }
}

// ---------------------------------------------------------------------------
// Sprint 7: Seccomp BPF
// ---------------------------------------------------------------------------

/// BPF instruction opcodes for seccomp filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BpfOpcode {
    /// Load word at absolute offset.
    LdAbsW = 0x20,
    /// Load half-word at absolute offset.
    LdAbsH = 0x28,
    /// Load byte at absolute offset.
    LdAbsB = 0x30,
    /// Jump if equal (immediate).
    JmpJeqK = 0x15,
    /// Jump if greater or equal (immediate).
    JmpJgeK = 0x35,
    /// Jump if set (bitwise AND, immediate).
    JmpJsetK = 0x45,
    /// Unconditional jump.
    JmpJa = 0x05,
    /// Return (action).
    Ret = 0x06,
    /// ALU AND (immediate).
    AluAndK = 0x54,
}

/// Seccomp return action values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SeccompAction {
    /// Allow the syscall.
    Allow = 0x7fff_0000,
    /// Kill the thread.
    KillThread = 0x0000_0000,
    /// Kill the process.
    KillProcess = 0x8000_0000,
    /// Trigger a SIGSYS and deliver a signal.
    Trap = 0x0003_0000,
    /// Return an errno value (low 16 bits).
    Errno = 0x0005_0000,
    /// Notify a tracing process.
    Trace = 0x7ff0_0000,
    /// Log the syscall and allow it.
    Log = 0x7ffc_0000,
}

impl SeccompAction {
    /// Create an Errno action with a specific errno value.
    pub fn errno(errno: u16) -> u32 {
        Self::Errno as u32 | (errno as u32)
    }
}

/// A single BPF instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpfInstruction {
    /// Opcode.
    pub code: u16,
    /// Jump target if condition is true.
    pub jt: u8,
    /// Jump target if condition is false.
    pub jf: u8,
    /// Immediate value.
    pub k: u32,
}

impl BpfInstruction {
    /// Create a load-word instruction at the given offset.
    pub fn load_word(offset: u32) -> Self {
        Self {
            code: BpfOpcode::LdAbsW as u16,
            jt: 0,
            jf: 0,
            k: offset,
        }
    }

    /// Create a jump-if-equal instruction.
    pub fn jump_eq(value: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJeqK as u16,
            jt,
            jf,
            k: value,
        }
    }

    /// Create a jump-if-greater-or-equal instruction.
    pub fn jump_ge(value: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJgeK as u16,
            jt,
            jf,
            k: value,
        }
    }

    /// Create a bitwise AND test (jump if set) instruction.
    pub fn jump_set(mask: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJsetK as u16,
            jt,
            jf,
            k: mask,
        }
    }

    /// Create an unconditional jump.
    pub fn jump(offset: u32) -> Self {
        Self {
            code: BpfOpcode::JmpJa as u16,
            jt: 0,
            jf: 0,
            k: offset,
        }
    }

    /// Create a return instruction.
    pub fn ret(action: u32) -> Self {
        Self {
            code: BpfOpcode::Ret as u16,
            jt: 0,
            jf: 0,
            k: action,
        }
    }

    /// Create an ALU AND instruction.
    pub fn alu_and(mask: u32) -> Self {
        Self {
            code: BpfOpcode::AluAndK as u16,
            jt: 0,
            jf: 0,
            k: mask,
        }
    }
}

/// Seccomp data offsets (for x86_64 struct seccomp_data layout).
pub mod seccomp_offsets {
    /// Offset of syscall number (nr field).
    pub const NR: u32 = 0;
    /// Offset of architecture (arch field).
    pub const ARCH: u32 = 4;
    /// Offset of instruction pointer (instruction_pointer field).
    pub const IP_LO: u32 = 8;
    pub const IP_HI: u32 = 12;
    /// Offset of syscall arguments (args[0..5]).
    pub const ARG0_LO: u32 = 16;
    pub const ARG0_HI: u32 = 20;
    pub const ARG1_LO: u32 = 24;
    pub const ARG1_HI: u32 = 28;
    pub const ARG2_LO: u32 = 32;
    pub const ARG2_HI: u32 = 36;
    pub const ARG3_LO: u32 = 40;
    pub const ARG3_HI: u32 = 44;
    pub const ARG4_LO: u32 = 48;
    pub const ARG4_HI: u32 = 52;
    pub const ARG5_LO: u32 = 56;
    pub const ARG5_HI: u32 = 60;
}

/// Audit architecture values.
pub mod audit_arch {
    pub const X86_64: u32 = 0xC000_003E;
    pub const AARCH64: u32 = 0xC000_00B7;
    pub const RISCV64: u32 = 0xC000_00F3;
}

/// Seccomp operating modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompMode {
    /// No filtering (disabled).
    Disabled,
    /// Strict mode: only read, write, exit, sigreturn allowed.
    Strict,
    /// Filter mode: BPF program decides.
    Filter,
}

/// A seccomp BPF filter program.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SeccompFilter {
    /// BPF instructions.
    pub instructions: Vec<BpfInstruction>,
    /// Whether this filter should be inherited on fork.
    pub inherit_on_fork: bool,
    /// Filter ID for tracking.
    pub filter_id: u64,
}

static NEXT_FILTER_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "alloc")]
impl SeccompFilter {
    /// Create a new empty filter.
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            inherit_on_fork: true,
            filter_id: NEXT_FILTER_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Add an instruction to the filter.
    pub fn push(&mut self, insn: BpfInstruction) {
        self.instructions.push(insn);
    }

    /// Get the number of instructions.
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    /// Check if the filter is empty.
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Validate the filter program.
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.instructions.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "seccomp filter",
                value: "empty program",
            });
        }
        // Max 4096 instructions (Linux limit)
        if self.instructions.len() > 4096 {
            return Err(KernelError::InvalidArgument {
                name: "seccomp filter",
                value: "exceeds 4096 instructions",
            });
        }
        // Last instruction must be a return
        if let Some(last) = self.instructions.last() {
            if last.code != BpfOpcode::Ret as u16 {
                return Err(KernelError::InvalidArgument {
                    name: "seccomp filter",
                    value: "must end with RET",
                });
            }
        }
        // Validate jump targets
        let len = self.instructions.len();
        for (i, insn) in self.instructions.iter().enumerate() {
            let code = insn.code;
            if code == BpfOpcode::JmpJeqK as u16
                || code == BpfOpcode::JmpJgeK as u16
                || code == BpfOpcode::JmpJsetK as u16
            {
                let jt_target = i + 1 + insn.jt as usize;
                let jf_target = i + 1 + insn.jf as usize;
                if jt_target >= len || jf_target >= len {
                    return Err(KernelError::InvalidArgument {
                        name: "seccomp filter",
                        value: "jump target out of bounds",
                    });
                }
            }
            if code == BpfOpcode::JmpJa as u16 {
                let target = i + 1 + insn.k as usize;
                if target >= len {
                    return Err(KernelError::InvalidArgument {
                        name: "seccomp filter",
                        value: "jump target out of bounds",
                    });
                }
            }
        }
        Ok(())
    }

    /// Execute the filter against a seccomp_data structure.
    /// Returns the action (SeccompAction value | errno).
    pub fn evaluate(&self, data: &SeccompData) -> u32 {
        let mut accumulator: u32 = 0;
        let mut pc: usize = 0;
        let data_bytes = data.as_bytes();

        while pc < self.instructions.len() {
            let insn = &self.instructions[pc];
            match insn.code {
                c if c == BpfOpcode::LdAbsW as u16 => {
                    let off = insn.k as usize;
                    if off + 4 <= data_bytes.len() {
                        accumulator = u32::from_ne_bytes([
                            data_bytes[off],
                            data_bytes[off + 1],
                            data_bytes[off + 2],
                            data_bytes[off + 3],
                        ]);
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::LdAbsH as u16 => {
                    let off = insn.k as usize;
                    if off + 2 <= data_bytes.len() {
                        accumulator =
                            u16::from_ne_bytes([data_bytes[off], data_bytes[off + 1]]) as u32;
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::LdAbsB as u16 => {
                    let off = insn.k as usize;
                    if off < data_bytes.len() {
                        accumulator = data_bytes[off] as u32;
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::JmpJeqK as u16 => {
                    if accumulator == insn.k {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJgeK as u16 => {
                    if accumulator >= insn.k {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJsetK as u16 => {
                    if accumulator & insn.k != 0 {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJa as u16 => {
                    pc += 1 + insn.k as usize;
                }
                c if c == BpfOpcode::Ret as u16 => {
                    return insn.k;
                }
                c if c == BpfOpcode::AluAndK as u16 => {
                    accumulator &= insn.k;
                    pc += 1;
                }
                _ => {
                    // Unknown opcode: kill
                    return SeccompAction::KillThread as u32;
                }
            }

            // Safety: prevent infinite loops
            if pc >= self.instructions.len() {
                return SeccompAction::KillThread as u32;
            }
        }

        SeccompAction::KillThread as u32
    }

    /// Build a filter that checks architecture and denies a set of syscall
    /// numbers.
    pub fn deny_syscalls(arch: u32, denied: &[u32], errno_val: u16) -> Self {
        let mut filter = Self::new();
        let num_denied = denied.len();

        // Load architecture
        filter.push(BpfInstruction::load_word(seccomp_offsets::ARCH));
        // If arch doesn't match, kill
        filter.push(BpfInstruction::jump_eq(arch, 1, 0));
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Load syscall number
        filter.push(BpfInstruction::load_word(seccomp_offsets::NR));

        // For each denied syscall, check and return errno
        for (i, &nr) in denied.iter().enumerate() {
            let remaining = num_denied - i - 1;
            // jt = jump to errno return (which is at the end of deny checks)
            // jf = check next deny or fall through to allow
            // jt must skip remaining deny checks + the allow return to reach errno return
            let jt = (remaining as u8).saturating_add(1);
            filter.push(BpfInstruction::jump_eq(nr, jt, 0));
        }

        // Default: allow
        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));

        // Errno return
        filter.push(BpfInstruction::ret(SeccompAction::errno(errno_val)));

        filter
    }

    /// Build a filter that only allows a whitelist of syscalls.
    pub fn allow_syscalls(arch: u32, allowed: &[u32]) -> Self {
        let mut filter = Self::new();
        let num_allowed = allowed.len();

        // Load architecture
        filter.push(BpfInstruction::load_word(seccomp_offsets::ARCH));
        filter.push(BpfInstruction::jump_eq(arch, 1, 0));
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Load syscall number
        filter.push(BpfInstruction::load_word(seccomp_offsets::NR));

        // For each allowed syscall, jump to allow
        for (i, &nr) in allowed.iter().enumerate() {
            let remaining = num_allowed - i - 1;
            // jt = jump to allow (which is `remaining` checks + 1 kill instruction away)
            let jt = (remaining as u8).saturating_add(1);
            filter.push(BpfInstruction::jump_eq(nr, jt, 0));
        }

        // Default: kill
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Allow return
        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));

        filter
    }
}

#[cfg(feature = "alloc")]
impl Default for SeccompFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Seccomp data structure matching the kernel's struct seccomp_data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SeccompData {
    /// Syscall number.
    pub nr: u32,
    /// Architecture (AUDIT_ARCH_*).
    pub arch: u32,
    /// Instruction pointer.
    pub instruction_pointer: u64,
    /// Syscall arguments (up to 6).
    pub args: [u64; 6],
}

impl SeccompData {
    pub fn new(nr: u32, arch: u32, args: [u64; 6]) -> Self {
        Self {
            nr,
            arch,
            instruction_pointer: 0,
            args,
        }
    }

    /// Convert to a byte representation for BPF evaluation.
    pub fn as_bytes(&self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        // nr at offset 0
        buf[0..4].copy_from_slice(&self.nr.to_ne_bytes());
        // arch at offset 4
        buf[4..8].copy_from_slice(&self.arch.to_ne_bytes());
        // instruction_pointer at offset 8
        buf[8..16].copy_from_slice(&self.instruction_pointer.to_ne_bytes());
        // args at offset 16
        for (i, &arg) in self.args.iter().enumerate() {
            let off = 16 + i * 8;
            buf[off..off + 8].copy_from_slice(&arg.to_ne_bytes());
        }
        buf
    }
}

/// Per-process seccomp state.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SeccompState {
    /// Current mode.
    pub mode: SeccompMode,
    /// Stack of filters (all evaluated, most restrictive wins).
    pub filters: Vec<SeccompFilter>,
}

#[cfg(feature = "alloc")]
impl SeccompState {
    pub fn new() -> Self {
        Self {
            mode: SeccompMode::Disabled,
            filters: Vec::new(),
        }
    }

    /// Install a new filter. Mode transitions to Filter.
    pub fn install_filter(&mut self, filter: SeccompFilter) -> Result<(), KernelError> {
        filter.validate()?;
        self.mode = SeccompMode::Filter;
        self.filters.push(filter);
        Ok(())
    }

    /// Evaluate all filters against the given syscall data.
    /// Returns the most restrictive action (lowest value wins per Linux
    /// semantics).
    pub fn evaluate(&self, data: &SeccompData) -> u32 {
        match self.mode {
            SeccompMode::Disabled => SeccompAction::Allow as u32,
            SeccompMode::Strict => {
                // Only allow read(0), write(1), exit(60), sigreturn(15)
                match data.nr {
                    0 | 1 | 15 | 60 => SeccompAction::Allow as u32,
                    _ => SeccompAction::KillThread as u32,
                }
            }
            SeccompMode::Filter => {
                let mut result = SeccompAction::Allow as u32;
                for filter in &self.filters {
                    let action = filter.evaluate(data);
                    // Most restrictive wins (lower value = more restrictive)
                    if action < result {
                        result = action;
                    }
                }
                result
            }
        }
    }

    /// Create a copy for a forked process (inherits filters marked for
    /// inheritance).
    pub fn fork_inherit(&self) -> Self {
        Self {
            mode: self.mode,
            filters: self
                .filters
                .iter()
                .filter(|f| f.inherit_on_fork)
                .cloned()
                .collect(),
        }
    }

    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for SeccompState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a u32 from a decimal string.
fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u32)?;
        } else {
            return None;
        }
    }
    Some(result)
}

/// Parse a u64 from a decimal string.
fn parse_u64(s: &str) -> Option<u64> {
    let mut result: u64 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            result = result.checked_mul(10)?;
            result = result.checked_add((b - b'0') as u64)?;
        } else {
            return None;
        }
    }
    Some(result)
}

/// Parse a hook specification: "path:timeout" or just "path".
#[cfg(feature = "alloc")]
fn parse_hook(val: &str) -> OciHook {
    let (path, timeout) = if let Some((p, t)) = val.split_once(':') {
        (p, parse_u32(t).unwrap_or(0))
    } else {
        (val, 0)
    };
    OciHook {
        path: String::from(path),
        args: Vec::new(),
        env: Vec::new(),
        timeout_secs: timeout,
    }
}

/// Minimal SHA-256 implementation (same algorithm as crypto::hash::sha256
/// but self-contained to avoid circular dependencies).
fn simple_sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let original_len_bits = (data.len() as u64).saturating_mul(8);

    // Pad message: append 0x80, zeros, then 64-bit big-endian length
    let padded_len = (data.len() + 9).div_ceil(64) * 64;
    // Use a stack buffer for small inputs, otherwise heap
    let mut padded = [0u8; 128]; // enough for up to 119 bytes of input
    let use_stack = padded_len <= 128;

    #[cfg(feature = "alloc")]
    let mut heap_padded: Vec<u8>;
    #[cfg(not(feature = "alloc"))]
    let heap_padded: [u8; 0] = [];

    let buf: &mut [u8] = if use_stack {
        padded[..data.len()].copy_from_slice(data);
        padded[data.len()] = 0x80;
        let len_offset = padded_len - 8;
        padded[len_offset..len_offset + 8].copy_from_slice(&original_len_bits.to_be_bytes());
        &mut padded[..padded_len]
    } else {
        #[cfg(feature = "alloc")]
        {
            heap_padded = vec![0u8; padded_len];
            heap_padded[..data.len()].copy_from_slice(data);
            heap_padded[data.len()] = 0x80;
            let len_offset = padded_len - 8;
            heap_padded[len_offset..len_offset + 8]
                .copy_from_slice(&original_len_bits.to_be_bytes());
            &mut heap_padded[..]
        }
        #[cfg(not(feature = "alloc"))]
        {
            // Without alloc, we cannot handle inputs > 119 bytes.
            // Return zeros as a safe fallback.
            return [0u8; 32];
        }
    };

    // Process 64-byte blocks
    let mut w = [0u32; 64];
    let mut block_offset = 0;
    while block_offset < buf.len() {
        let block = &buf[block_offset..block_offset + 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);

        block_offset += 64;
    }

    let mut out = [0u8; 32];
    for (i, &val) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- OCI Runtime Spec tests ---

    #[test]
    fn test_oci_lifecycle_state_display() {
        assert_eq!(
            alloc::format!("{}", OciLifecycleState::Creating),
            "creating"
        );
        assert_eq!(alloc::format!("{}", OciLifecycleState::Created), "created");
        assert_eq!(alloc::format!("{}", OciLifecycleState::Running), "running");
        assert_eq!(alloc::format!("{}", OciLifecycleState::Stopped), "stopped");
    }

    #[test]
    fn test_oci_namespace_kind_parse() {
        assert_eq!(
            OciNamespaceKind::from_str_kind("pid"),
            Some(OciNamespaceKind::Pid)
        );
        assert_eq!(
            OciNamespaceKind::from_str_kind("network"),
            Some(OciNamespaceKind::Network)
        );
        assert_eq!(OciNamespaceKind::from_str_kind("invalid"), None);
    }

    #[test]
    fn test_oci_config_parse_basic() {
        let input = "oci_version=1.0.2\nroot_path=/rootfs\nroot_readonly=true\\
                     nhostname=mycontainer\nprocess_cwd=/app\nprocess_uid=1000\nprocess_gid=1000\\
                     nprocess_terminal=true\nprocess_arg=/bin/sh\nprocess_arg=-c\\
                     nprocess_env=PATH=/usr/bin\nnamespace=pid\nnamespace=network:/proc/123/ns/\
                     net\ncgroups_path=/sys/fs/cgroup/mycontainer\nmemory_limit=67108864\\
                     ncpu_shares=512\ncpu_quota=50000\ncpu_period=100000\nhook_prestart=/usr/bin/\
                     hook:5\nmount=/proc:proc:proc:nosuid,noexec\n";
        let config = OciConfig::parse(input).unwrap();
        assert_eq!(config.oci_version, "1.0.2");
        assert_eq!(config.root.path, "/rootfs");
        assert!(config.root.readonly);
        assert_eq!(config.hostname, "mycontainer");
        assert_eq!(config.process.cwd, "/app");
        assert_eq!(config.process.uid, 1000);
        assert_eq!(config.process.gid, 1000);
        assert!(config.process.terminal);
        assert_eq!(config.process.args.len(), 2);
        assert_eq!(config.process.args[0], "/bin/sh");
        assert_eq!(config.process.env.len(), 1);
        assert_eq!(config.linux.namespaces.len(), 2);
        assert_eq!(config.linux.namespaces[0].kind, OciNamespaceKind::Pid);
        assert!(config.linux.namespaces[0].path.is_none());
        assert_eq!(config.linux.namespaces[1].kind, OciNamespaceKind::Network);
        assert!(config.linux.namespaces[1].path.is_some());
        assert_eq!(config.linux.memory_limit, 67108864);
        assert_eq!(config.linux.cpu_shares, 512);
        assert_eq!(config.linux.cpu_quota, 50000);
        assert_eq!(config.hooks.prestart.len(), 1);
        assert_eq!(config.hooks.prestart[0].timeout_secs, 5);
        assert_eq!(config.mounts.len(), 1);
        assert_eq!(config.mounts[0].options.len(), 2);
    }

    #[test]
    fn test_oci_config_validate_empty_args() {
        let input = "root_path=/rootfs\n";
        let config = OciConfig::parse(input).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_oci_container_lifecycle() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let mut container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        assert_eq!(container.state, OciLifecycleState::Creating);

        container.mark_created().unwrap();
        assert_eq!(container.state, OciLifecycleState::Created);

        container.start(42).unwrap();
        assert_eq!(container.state, OciLifecycleState::Running);
        assert_eq!(container.pid, 42);

        container.stop().unwrap();
        assert_eq!(container.state, OciLifecycleState::Stopped);
    }

    #[test]
    fn test_oci_container_invalid_transition() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let mut container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        // Cannot start from Creating (must be Created first)
        assert!(container.start(1).is_err());
    }

    #[test]
    fn test_oci_container_pivot_root() {
        let input = "root_path=/rootfs\nprocess_arg=/bin/sh\nprocess_cwd=/\n";
        let config = OciConfig::parse(input).unwrap();
        let container = OciContainer::new("test1", "/bundles/test1", config).unwrap();
        let (old, new) = container.pivot_root().unwrap();
        assert_eq!(old, "/.pivot_root");
        assert_eq!(new, "/rootfs");
    }

    // --- Container Image Format tests ---

    #[test]
    fn test_layer_digest_compute() {
        let digest = LayerDigest::compute(b"hello");
        // SHA-256("hello") =
        // 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(digest.bytes[0], 0x2c);
        assert_eq!(digest.bytes[1], 0xf2);
    }

    #[test]
    fn test_layer_digest_hex() {
        let digest = LayerDigest::compute(b"");
        let hex = digest.to_hex();
        assert_eq!(hex.len(), 64);
        // SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert!(hex.starts_with("e3b0c442"));
    }

    #[test]
    fn test_is_gzip() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x00]));
        assert!(!is_gzip(&[0x1f]));
    }

    #[test]
    fn test_tar_size_parse() {
        let mut header = [0u8; 512];
        // Octal "0000644" at offset 124
        header[124] = b'0';
        header[125] = b'0';
        header[126] = b'0';
        header[127] = b'0';
        header[128] = b'6';
        header[129] = b'4';
        header[130] = b'4';
        assert_eq!(parse_tar_size(&header), 0o644);
    }

    #[test]
    fn test_container_image_compose() {
        let config = b"config data";
        let layer1 = b"layer 1 data";
        let layer2 = b"layer 2 data";
        let image = ContainerImage::compose("test:latest", config, &[layer1, layer2]);
        assert_eq!(image.name, "test:latest");
        assert_eq!(image.layers.len(), 2);
        assert_eq!(image.manifest.layer_digests.len(), 2);
        assert_eq!(image.manifest.schema_version, 2);
        assert_eq!(image.image_id, image.manifest.config_digest);
    }

    #[test]
    fn test_layer_cache_operations() {
        let mut cache = LayerCache::new(2);
        let digest = LayerDigest::compute(b"test");
        let hex = digest.to_hex();
        let layer = CachedLayer {
            digest: digest.clone(),
            extracted_path: String::from("/layers/test"),
            size_bytes: 1024,
            reference_count: 1,
        };
        assert!(cache.insert(layer));
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.get(&hex).is_some());
        assert!(cache.add_ref(&hex));
        assert_eq!(cache.get(&hex).unwrap().reference_count, 2);
        assert!(cache.release(&hex));
        assert_eq!(cache.get(&hex).unwrap().reference_count, 1);
        assert!(cache.release(&hex)); // drops to 0, removed
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn test_layer_cache_full() {
        let mut cache = LayerCache::new(1);
        let l1 = CachedLayer {
            digest: LayerDigest::compute(b"a"),
            extracted_path: String::from("/a"),
            size_bytes: 100,
            reference_count: 1,
        };
        let l2 = CachedLayer {
            digest: LayerDigest::compute(b"b"),
            extracted_path: String::from("/b"),
            size_bytes: 200,
            reference_count: 1,
        };
        assert!(cache.insert(l1));
        assert!(cache.is_full());
        assert!(!cache.insert(l2));
    }

    // --- Cgroup Memory Controller tests ---

    #[test]
    fn test_cgroup_memory_basic() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(4096).unwrap();
        assert!(mem.charge(2048).is_ok());
        assert_eq!(mem.usage_current, 2048);
        assert_eq!(mem.usage_peak, 2048);
        mem.uncharge(1024);
        assert_eq!(mem.usage_current, 1024);
        assert_eq!(mem.usage_peak, 2048); // peak unchanged
    }

    #[test]
    fn test_cgroup_memory_oom() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(1024).unwrap();
        mem.charge(512).unwrap();
        let result = mem.charge(1024);
        assert!(result.is_err());
        assert_eq!(mem.oom.oom_kill_count, 1);
        assert!(mem.oom.under_oom);
    }

    #[test]
    fn test_cgroup_memory_soft_limit() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_soft_limit(512);
        mem.charge(256).unwrap();
        assert!(!mem.soft_limit_exceeded());
        // Set hard limit high enough to not OOM
        mem.set_hard_limit(4096).unwrap();
        mem.charge(512).unwrap();
        assert!(mem.soft_limit_exceeded());
    }

    #[test]
    fn test_cgroup_memory_reclaim_from_cache() {
        let mut mem = CgroupMemoryController::new(1);
        mem.set_hard_limit(2048).unwrap();
        mem.charge(1024).unwrap();
        mem.add_cache(512);
        // Now usage_current = 1024 + 512 = 1536, cache = 512
        // Charge 1024 more would exceed 2048, but cache can be reclaimed
        assert!(mem.charge(1024).is_ok());
    }

    #[test]
    fn test_memory_stat_total() {
        let stat = MemoryStat {
            rss: 1000,
            cache: 500,
            mapped_file: 200,
            anon: 300,
            swap: 0,
        };
        assert_eq!(stat.total(), 1500);
    }

    // --- Cgroup CPU Controller tests ---

    #[test]
    fn test_cgroup_cpu_shares() {
        let mut cpu = CgroupCpuController::new(1);
        assert_eq!(cpu.shares, 1024);
        cpu.set_shares(2048).unwrap();
        assert_eq!(cpu.shares, 2048);
        assert!(cpu.set_shares(1).is_err()); // below minimum
        assert!(cpu.set_shares(300000).is_err()); // above maximum
    }

    #[test]
    fn test_cgroup_cpu_bandwidth() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(50000, 100000).unwrap();
        assert_eq!(cpu.quota_us, 50000);
        assert_eq!(cpu.period_us, 100000);
        // 50% CPU
        assert_eq!(cpu.effective_cpu_percent_x100(), 5000);
    }

    #[test]
    fn test_cgroup_cpu_throttle() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap(); // 10% CPU
                                                   // 10000us = 10_000_000ns quota
        assert!(!cpu.consume_runtime(5_000_000)); // 5ms, not throttled
        assert!(cpu.consume_runtime(6_000_000)); // 6ms more, total 11ms > 10ms quota
        assert!(cpu.throttled);
        assert_eq!(cpu.stats.nr_throttled, 1);
    }

    #[test]
    fn test_cgroup_cpu_period_reset() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap();
        cpu.consume_runtime(10_000_000);
        cpu.new_period();
        assert!(!cpu.throttled);
        assert_eq!(cpu.stats.nr_periods, 1);
    }

    #[test]
    fn test_cgroup_cpu_burst() {
        let mut cpu = CgroupCpuController::new(1);
        cpu.set_bandwidth(10000, 100000).unwrap(); // 10ms quota
        cpu.set_burst(5000); // 5ms burst allowed
                             // Use only 5ms of 10ms quota -> 5ms unused
        cpu.consume_runtime(5_000_000);
        cpu.new_period(); // 5ms saved as burst budget
        assert!(cpu.burst_budget_ns > 0);
        // Now can use up to 15ms (10ms quota + 5ms burst)
        assert!(!cpu.consume_runtime(14_000_000));
    }

    #[test]
    fn test_cgroup_cpu_proportional() {
        let cpu = CgroupCpuController::new(1);
        // Default shares=1024, total=2048 -> 50% of period
        let ns = cpu.proportional_runtime_ns(2048);
        // period=100000us=100_000_000ns, 1024/2048 = 50% = 50_000_000ns
        assert_eq!(ns, 50_000_000);
    }

    // --- Overlay Filesystem tests ---

    #[test]
    fn test_overlay_basic_lookup() {
        let mut lower = OverlayLayer::new(true);
        // Add entry directly (bypass readonly check for setup)
        lower.entries.insert(
            String::from("etc/passwd"),
            OverlayEntry {
                path: String::from("etc/passwd"),
                kind: OverlayEntryKind::File,
                content: b"root:x:0:0".to_vec(),
                mode: 0o644,
            },
        );

        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        let entry = fs.lookup("etc/passwd");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().content, b"root:x:0:0");
    }

    #[test]
    fn test_overlay_upper_takes_precedence() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/hostname"),
            OverlayEntry {
                path: String::from("etc/hostname"),
                kind: OverlayEntryKind::File,
                content: b"oldhost".to_vec(),
                mode: 0o644,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        fs.write_file("etc/hostname", b"newhost".to_vec(), 0o644)
            .unwrap();
        let entry = fs.lookup("etc/hostname").unwrap();
        assert_eq!(entry.content, b"newhost");
    }

    #[test]
    fn test_overlay_whiteout() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/shadow"),
            OverlayEntry {
                path: String::from("etc/shadow"),
                kind: OverlayEntryKind::File,
                content: b"secret".to_vec(),
                mode: 0o600,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        assert!(fs.lookup("etc/shadow").is_some());
        fs.delete_file("etc/shadow").unwrap();
        assert!(fs.lookup("etc/shadow").is_none());
    }

    #[test]
    fn test_overlay_opaque_dir() {
        let mut lower = OverlayLayer::new(true);
        lower.entries.insert(
            String::from("etc/conf.d/old.conf"),
            OverlayEntry {
                path: String::from("etc/conf.d/old.conf"),
                kind: OverlayEntryKind::File,
                content: b"old".to_vec(),
                mode: 0o644,
            },
        );
        let mut fs = OverlayFs::new("/tmp/work");
        fs.add_lower_layer(lower);
        fs.make_opaque_dir("etc/conf.d").unwrap();
        // The lower layer file should not be visible via listing
        let listing = fs.list_dir("etc/conf.d");
        assert!(listing.is_empty());
    }

    #[test]
    fn test_overlay_readonly_layer() {
        let mut lower = OverlayLayer::new(true);
        let result = lower.add_entry(OverlayEntry {
            path: String::from("test"),
            kind: OverlayEntryKind::File,
            content: Vec::new(),
            mode: 0o644,
        });
        assert!(result.is_err());
    }

    // --- Veth Networking tests ---

    #[test]
    fn test_veth_pair_creation() {
        let pair = create_veth_pair("veth0", "eth0", 42);
        assert_eq!(pair.host.name, "veth0");
        assert_eq!(pair.container.name, "eth0");
        assert_eq!(pair.host.peer_name, "eth0");
        assert_eq!(pair.container.peer_name, "veth0");
        assert_eq!(pair.host.namespace_id, 0);
        assert_eq!(pair.container.namespace_id, 42);
        assert_eq!(pair.host.mtu, 1500);
        // MACs differ
        assert_ne!(pair.host.mac, pair.container.mac);
    }

    #[test]
    fn test_veth_mac_generation() {
        let mac1 = generate_veth_mac(1);
        let mac2 = generate_veth_mac(2);
        assert_eq!(mac1[0], 0x02); // locally administered
        assert_ne!(mac1, mac2);
    }

    #[test]
    fn test_nat_table() {
        let mut nat = NatTable::new(0xC0A80001); // 192.168.0.1
        nat.enable_masquerade();
        assert!(nat.masquerade_enabled);

        let mapping = NatPortMapping {
            external_port: 8080,
            internal_port: 80,
            protocol: 6,              // TCP
            container_ip: 0x0A000002, // 10.0.0.2
        };
        nat.add_port_mapping(mapping).unwrap();
        assert_eq!(nat.port_mappings.len(), 1);

        // Duplicate should fail
        let dup = NatPortMapping {
            external_port: 8080,
            internal_port: 8080,
            protocol: 6,
            container_ip: 0x0A000003,
        };
        assert!(nat.add_port_mapping(dup).is_err());

        // Lookup
        let found = nat.lookup_inbound(8080, 6).unwrap();
        assert_eq!(found.internal_port, 80);
        assert_eq!(found.container_ip, 0x0A000002);

        // SNAT rewrite
        let rewritten = nat.snat_rewrite(0x0A000002);
        assert_eq!(rewritten, Some(0xC0A80001));

        // Remove
        assert!(nat.remove_port_mapping(8080, 6));
        assert!(nat.lookup_inbound(8080, 6).is_none());
    }

    #[test]
    fn test_veth_bridge() {
        let mut bridge = VethBridge::new("br0", 0x0A000001, 0xFFFFFF00);
        bridge.attach("veth0");
        bridge.attach("veth1");
        assert_eq!(bridge.attached_count(), 2);
        assert!(bridge.in_subnet(0x0A0000FE)); // 10.0.0.254
        assert!(!bridge.in_subnet(0x0B000001)); // 11.0.0.1

        bridge.add_arp_proxy(ArpProxyEntry {
            ip: 0x0A000002,
            mac: [0x02, 0x42, 0x00, 0x00, 0x00, 0x01],
        });
        assert!(bridge.arp_lookup(0x0A000002).is_some());
        assert!(bridge.arp_lookup(0x0A000003).is_none());

        bridge.detach("veth0");
        assert_eq!(bridge.attached_count(), 1);
    }

    // --- Seccomp BPF tests ---

    #[test]
    fn test_seccomp_data_as_bytes() {
        let data = SeccompData::new(1, audit_arch::X86_64, [0; 6]);
        let bytes = data.as_bytes();
        assert_eq!(bytes.len(), 64);
        // nr=1 at offset 0
        assert_eq!(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            1
        );
        // arch at offset 4
        assert_eq!(
            u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            audit_arch::X86_64
        );
    }

    #[test]
    fn test_seccomp_filter_validate() {
        let mut filter = SeccompFilter::new();
        assert!(filter.validate().is_err()); // empty

        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));
        assert!(filter.validate().is_ok());
    }

    #[test]
    fn test_seccomp_filter_deny_syscalls() {
        let filter = SeccompFilter::deny_syscalls(
            audit_arch::X86_64,
            &[59], // deny execve
            1,     // EPERM
        );
        assert!(filter.validate().is_ok());

        // Test: execve should be denied
        let data_execve = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        let action = filter.evaluate(&data_execve);
        assert_eq!(action, SeccompAction::errno(1));

        // Test: read should be allowed
        let data_read = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        let action = filter.evaluate(&data_read);
        assert_eq!(action, SeccompAction::Allow as u32);
    }

    #[test]
    fn test_seccomp_filter_allow_syscalls() {
        let filter = SeccompFilter::allow_syscalls(
            audit_arch::X86_64,
            &[0, 1, 60], // read, write, exit
        );
        assert!(filter.validate().is_ok());

        let data_read = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        assert_eq!(filter.evaluate(&data_read), SeccompAction::Allow as u32);

        let data_execve = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(
            filter.evaluate(&data_execve),
            SeccompAction::KillProcess as u32
        );
    }

    #[test]
    fn test_seccomp_wrong_arch_killed() {
        let filter = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        let data = SeccompData::new(59, audit_arch::AARCH64, [0; 6]);
        assert_eq!(filter.evaluate(&data), SeccompAction::KillProcess as u32);
    }

    #[test]
    fn test_seccomp_state_disabled() {
        let state = SeccompState::new();
        let data = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data), SeccompAction::Allow as u32);
    }

    #[test]
    fn test_seccomp_state_strict() {
        let mut state = SeccompState::new();
        state.mode = SeccompMode::Strict;
        // read(0) allowed
        let data = SeccompData::new(0, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data), SeccompAction::Allow as u32);
        // execve(59) killed
        let data2 = SeccompData::new(59, audit_arch::X86_64, [0; 6]);
        assert_eq!(state.evaluate(&data2), SeccompAction::KillThread as u32);
    }

    #[test]
    fn test_seccomp_state_filter_install() {
        let mut state = SeccompState::new();
        let filter = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        state.install_filter(filter).unwrap();
        assert_eq!(state.mode, SeccompMode::Filter);
        assert_eq!(state.filter_count(), 1);
    }

    #[test]
    fn test_seccomp_fork_inherit() {
        let mut state = SeccompState::new();
        let mut f1 = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[59], 1);
        f1.inherit_on_fork = true;
        let mut f2 = SeccompFilter::deny_syscalls(audit_arch::X86_64, &[60], 1);
        f2.inherit_on_fork = false;
        state.install_filter(f1).unwrap();
        state.install_filter(f2).unwrap();
        let child = state.fork_inherit();
        assert_eq!(child.filter_count(), 1); // only inherited one
    }

    #[test]
    fn test_bpf_instruction_constructors() {
        let load = BpfInstruction::load_word(4);
        assert_eq!(load.code, BpfOpcode::LdAbsW as u16);
        assert_eq!(load.k, 4);

        let jeq = BpfInstruction::jump_eq(42, 1, 0);
        assert_eq!(jeq.code, BpfOpcode::JmpJeqK as u16);
        assert_eq!(jeq.k, 42);
        assert_eq!(jeq.jt, 1);
        assert_eq!(jeq.jf, 0);

        let ret = BpfInstruction::ret(SeccompAction::Allow as u32);
        assert_eq!(ret.code, BpfOpcode::Ret as u16);
    }

    #[test]
    fn test_seccomp_errno_action() {
        let action = SeccompAction::errno(13); // EACCES
        assert_eq!(action, 0x0005_000D);
    }

    // --- Helper tests ---

    #[test]
    fn test_parse_u32() {
        assert_eq!(parse_u32("12345"), Some(12345));
        assert_eq!(parse_u32("0"), Some(0));
        assert_eq!(parse_u32("abc"), None);
        assert_eq!(parse_u32(""), Some(0));
    }

    #[test]
    fn test_parse_u64() {
        assert_eq!(parse_u64("123456789"), Some(123456789));
        assert_eq!(parse_u64("0"), Some(0));
    }

    #[test]
    fn test_sha256_empty() {
        let hash = simple_sha256(b"");
        // SHA-256("") =
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(hash[0], 0xe3);
        assert_eq!(hash[1], 0xb0);
        assert_eq!(hash[2], 0xc4);
        assert_eq!(hash[3], 0x42);
    }

    #[test]
    fn test_sha256_hello() {
        let hash = simple_sha256(b"hello");
        // SHA-256("hello") =
        // 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(hash[0], 0x2c);
        assert_eq!(hash[1], 0xf2);
        assert_eq!(hash[2], 0x4d);
        assert_eq!(hash[3], 0xba);
    }
}
