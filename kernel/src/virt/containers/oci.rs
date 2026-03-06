//! OCI Runtime Specification - container lifecycle, config parsing, hooks,
//! pivot_root.

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use super::{parse_u32, parse_u64};
use crate::error::KernelError;

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

/// Parse a hook specification: "path:timeout" or just "path".
#[cfg(feature = "alloc")]
pub(super) fn parse_hook(val: &str) -> OciHook {
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
