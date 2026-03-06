//! Container Runtime Service
//!
//! Provides pod sandbox and container lifecycle management following
//! the CRI RuntimeService specification.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Pod Sandbox
// ---------------------------------------------------------------------------

/// Network configuration for a pod sandbox.
#[derive(Debug, Clone, Default)]
pub struct NetworkConfig {
    /// Pod CIDR.
    pub pod_cidr: String,
    /// DNS server addresses.
    pub dns_servers: Vec<String>,
    /// DNS search domains.
    pub dns_searches: Vec<String>,
}

/// Pod sandbox state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PodSandboxState {
    /// Sandbox is ready to run containers.
    Ready,
    /// Sandbox is not ready (stopped or failed).
    #[default]
    NotReady,
}

/// A pod sandbox groups containers with shared namespaces and networking.
#[derive(Debug, Clone)]
pub struct PodSandbox {
    /// Unique sandbox identifier.
    pub id: u64,
    /// Human-readable name.
    pub name: String,
    /// Kubernetes namespace.
    pub namespace: String,
    /// Current state.
    pub state: PodSandboxState,
    /// Network configuration.
    pub network_config: NetworkConfig,
    /// Tick when the sandbox was created.
    pub created_tick: u64,
    /// Labels (key-value metadata).
    pub labels: BTreeMap<String, String>,
    /// Annotations.
    pub annotations: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// Container
// ---------------------------------------------------------------------------

/// Container state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContainerState {
    /// Container has been created but not started.
    #[default]
    Created,
    /// Container is actively running.
    Running,
    /// Container has exited.
    Exited,
    /// Container state is unknown.
    Unknown,
}

impl ContainerState {
    /// Check if a state transition is valid.
    pub fn can_transition_to(self, target: ContainerState) -> bool {
        matches!(
            (self, target),
            (ContainerState::Created, ContainerState::Running)
                | (ContainerState::Running, ContainerState::Exited)
                | (ContainerState::Created, ContainerState::Exited)
                | (_, ContainerState::Unknown)
        )
    }
}

/// A container running within a pod sandbox.
#[derive(Debug, Clone)]
pub struct Container {
    /// Unique container identifier.
    pub id: u64,
    /// Human-readable name.
    pub name: String,
    /// Container image reference.
    pub image: String,
    /// Current state.
    pub state: ContainerState,
    /// Pod sandbox this container belongs to.
    pub pod_sandbox_id: u64,
    /// Command to execute.
    pub command: Vec<String>,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables (key=value).
    pub env: Vec<String>,
    /// Exit code (set when state is Exited).
    pub exit_code: i32,
    /// Tick when started.
    pub started_tick: u64,
    /// Tick when finished.
    pub finished_tick: u64,
    /// Labels.
    pub labels: BTreeMap<String, String>,
}

/// Container status information.
#[derive(Debug, Clone)]
pub struct ContainerStatus {
    /// Container ID.
    pub id: u64,
    /// Container name.
    pub name: String,
    /// Current state.
    pub state: ContainerState,
    /// Image reference.
    pub image: String,
    /// Exit code.
    pub exit_code: i32,
    /// Start tick.
    pub started_tick: u64,
    /// Finish tick.
    pub finished_tick: u64,
}

// ---------------------------------------------------------------------------
// Runtime Service
// ---------------------------------------------------------------------------

/// CRI error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CriError {
    /// Sandbox not found.
    SandboxNotFound(u64),
    /// Container not found.
    ContainerNotFound(u64),
    /// Invalid state transition.
    InvalidStateTransition,
    /// Sandbox is not ready.
    SandboxNotReady(u64),
    /// Duplicate ID.
    AlreadyExists(u64),
}

/// Next unique ID generator.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn alloc_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// CRI RuntimeService implementation.
#[derive(Debug)]
pub struct RuntimeService {
    /// Active pod sandboxes.
    sandboxes: BTreeMap<u64, PodSandbox>,
    /// Active containers.
    containers: BTreeMap<u64, Container>,
}

impl Default for RuntimeService {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeService {
    /// Create a new runtime service.
    pub fn new() -> Self {
        RuntimeService {
            sandboxes: BTreeMap::new(),
            containers: BTreeMap::new(),
        }
    }

    /// Create and start a pod sandbox.
    pub fn run_pod_sandbox(
        &mut self,
        name: String,
        namespace: String,
        network_config: NetworkConfig,
        current_tick: u64,
    ) -> Result<u64, CriError> {
        let id = alloc_id();
        let sandbox = PodSandbox {
            id,
            name,
            namespace,
            state: PodSandboxState::Ready,
            network_config,
            created_tick: current_tick,
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        };
        self.sandboxes.insert(id, sandbox);
        Ok(id)
    }

    /// Stop a running pod sandbox.
    pub fn stop_pod_sandbox(&mut self, sandbox_id: u64) -> Result<(), CriError> {
        let sandbox = self
            .sandboxes
            .get_mut(&sandbox_id)
            .ok_or(CriError::SandboxNotFound(sandbox_id))?;
        sandbox.state = PodSandboxState::NotReady;

        // Stop all containers in this sandbox
        for container in self.containers.values_mut() {
            if container.pod_sandbox_id == sandbox_id && container.state == ContainerState::Running
            {
                container.state = ContainerState::Exited;
                container.exit_code = -1;
            }
        }
        Ok(())
    }

    /// Remove a stopped pod sandbox.
    pub fn remove_pod_sandbox(&mut self, sandbox_id: u64) -> Result<(), CriError> {
        if !self.sandboxes.contains_key(&sandbox_id) {
            return Err(CriError::SandboxNotFound(sandbox_id));
        }

        // Remove all containers in this sandbox
        self.containers
            .retain(|_, c| c.pod_sandbox_id != sandbox_id);
        self.sandboxes.remove(&sandbox_id);
        Ok(())
    }

    /// Get pod sandbox status.
    pub fn pod_sandbox_status(&self, sandbox_id: u64) -> Result<&PodSandbox, CriError> {
        self.sandboxes
            .get(&sandbox_id)
            .ok_or(CriError::SandboxNotFound(sandbox_id))
    }

    /// List pod sandboxes, optionally filtered by state.
    pub fn list_pod_sandboxes(&self, state_filter: Option<PodSandboxState>) -> Vec<&PodSandbox> {
        self.sandboxes
            .values()
            .filter(|s| state_filter.is_none() || Some(s.state) == state_filter)
            .collect()
    }

    /// Create a container within a pod sandbox.
    pub fn create_container(
        &mut self,
        pod_sandbox_id: u64,
        name: String,
        image: String,
        command: Vec<String>,
        args: Vec<String>,
        env: Vec<String>,
    ) -> Result<u64, CriError> {
        // Verify sandbox exists and is ready
        let sandbox = self
            .sandboxes
            .get(&pod_sandbox_id)
            .ok_or(CriError::SandboxNotFound(pod_sandbox_id))?;
        if sandbox.state != PodSandboxState::Ready {
            return Err(CriError::SandboxNotReady(pod_sandbox_id));
        }

        let id = alloc_id();
        let container = Container {
            id,
            name,
            image,
            state: ContainerState::Created,
            pod_sandbox_id,
            command,
            args,
            env,
            exit_code: 0,
            started_tick: 0,
            finished_tick: 0,
            labels: BTreeMap::new(),
        };
        self.containers.insert(id, container);
        Ok(id)
    }

    /// Start a created container.
    pub fn start_container(
        &mut self,
        container_id: u64,
        current_tick: u64,
    ) -> Result<(), CriError> {
        let container = self
            .containers
            .get_mut(&container_id)
            .ok_or(CriError::ContainerNotFound(container_id))?;

        if !container.state.can_transition_to(ContainerState::Running) {
            return Err(CriError::InvalidStateTransition);
        }

        container.state = ContainerState::Running;
        container.started_tick = current_tick;
        Ok(())
    }

    /// Stop a running container.
    pub fn stop_container(&mut self, container_id: u64, current_tick: u64) -> Result<(), CriError> {
        let container = self
            .containers
            .get_mut(&container_id)
            .ok_or(CriError::ContainerNotFound(container_id))?;

        if !container.state.can_transition_to(ContainerState::Exited) {
            return Err(CriError::InvalidStateTransition);
        }

        container.state = ContainerState::Exited;
        container.exit_code = 0;
        container.finished_tick = current_tick;
        Ok(())
    }

    /// Remove a stopped container.
    pub fn remove_container(&mut self, container_id: u64) -> Result<(), CriError> {
        if !self.containers.contains_key(&container_id) {
            return Err(CriError::ContainerNotFound(container_id));
        }
        self.containers.remove(&container_id);
        Ok(())
    }

    /// Get container status.
    pub fn container_status(&self, container_id: u64) -> Result<ContainerStatus, CriError> {
        let c = self
            .containers
            .get(&container_id)
            .ok_or(CriError::ContainerNotFound(container_id))?;

        Ok(ContainerStatus {
            id: c.id,
            name: c.name.clone(),
            state: c.state,
            image: c.image.clone(),
            exit_code: c.exit_code,
            started_tick: c.started_tick,
            finished_tick: c.finished_tick,
        })
    }

    /// List containers, optionally filtered by state and/or sandbox.
    pub fn list_containers(
        &self,
        state_filter: Option<ContainerState>,
        sandbox_filter: Option<u64>,
    ) -> Vec<&Container> {
        self.containers
            .values()
            .filter(|c| state_filter.is_none() || Some(c.state) == state_filter)
            .filter(|c| sandbox_filter.is_none() || Some(c.pod_sandbox_id) == sandbox_filter)
            .collect()
    }

    /// Get the total number of sandboxes.
    pub fn sandbox_count(&self) -> usize {
        self.sandboxes.len()
    }

    /// Get the total number of containers.
    pub fn container_count(&self) -> usize {
        self.containers.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn make_service() -> RuntimeService {
        RuntimeService::new()
    }

    #[test]
    fn test_run_pod_sandbox() {
        let mut svc = make_service();
        let id = svc
            .run_pod_sandbox(
                String::from("test-pod"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        assert!(id > 0);
        let sandbox = svc.pod_sandbox_status(id).unwrap();
        assert_eq!(sandbox.state, PodSandboxState::Ready);
        assert_eq!(sandbox.name, "test-pod");
    }

    #[test]
    fn test_stop_pod_sandbox() {
        let mut svc = make_service();
        let id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        svc.stop_pod_sandbox(id).unwrap();
        let sandbox = svc.pod_sandbox_status(id).unwrap();
        assert_eq!(sandbox.state, PodSandboxState::NotReady);
    }

    #[test]
    fn test_remove_pod_sandbox() {
        let mut svc = make_service();
        let id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        svc.stop_pod_sandbox(id).unwrap();
        svc.remove_pod_sandbox(id).unwrap();
        assert_eq!(svc.sandbox_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent_sandbox() {
        let mut svc = make_service();
        assert_eq!(
            svc.remove_pod_sandbox(999),
            Err(CriError::SandboxNotFound(999))
        );
    }

    #[test]
    fn test_create_container() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let cid = svc
            .create_container(
                pod_id,
                String::from("nginx"),
                String::from("nginx:latest"),
                vec![String::from("/usr/sbin/nginx")],
                vec![String::from("-g"), String::from("daemon off;")],
                vec![String::from("PORT=80")],
            )
            .unwrap();
        let status = svc.container_status(cid).unwrap();
        assert_eq!(status.state, ContainerState::Created);
    }

    #[test]
    fn test_start_and_stop_container() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let cid = svc
            .create_container(
                pod_id,
                String::from("app"),
                String::from("app:v1"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();

        svc.start_container(cid, 200).unwrap();
        assert_eq!(
            svc.container_status(cid).unwrap().state,
            ContainerState::Running
        );

        svc.stop_container(cid, 300).unwrap();
        let status = svc.container_status(cid).unwrap();
        assert_eq!(status.state, ContainerState::Exited);
        assert_eq!(status.exit_code, 0);
    }

    #[test]
    fn test_container_invalid_transition() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let cid = svc
            .create_container(
                pod_id,
                String::from("app"),
                String::from("app:v1"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();

        svc.start_container(cid, 200).unwrap();
        svc.stop_container(cid, 300).unwrap();
        // Cannot start an exited container
        assert_eq!(
            svc.start_container(cid, 400),
            Err(CriError::InvalidStateTransition)
        );
    }

    #[test]
    fn test_list_containers_filter() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let c1 = svc
            .create_container(
                pod_id,
                String::from("a"),
                String::from("img:v1"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        let _c2 = svc
            .create_container(
                pod_id,
                String::from("b"),
                String::from("img:v1"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();

        svc.start_container(c1, 200).unwrap();

        let running = svc.list_containers(Some(ContainerState::Running), None);
        assert_eq!(running.len(), 1);
        let created = svc.list_containers(Some(ContainerState::Created), None);
        assert_eq!(created.len(), 1);
        let all = svc.list_containers(None, None);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_stop_sandbox_stops_containers() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let cid = svc
            .create_container(
                pod_id,
                String::from("app"),
                String::from("img:v1"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        svc.start_container(cid, 200).unwrap();
        svc.stop_pod_sandbox(pod_id).unwrap();

        let status = svc.container_status(cid).unwrap();
        assert_eq!(status.state, ContainerState::Exited);
    }

    #[test]
    fn test_create_container_not_ready_sandbox() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        svc.stop_pod_sandbox(pod_id).unwrap();
        assert_eq!(
            svc.create_container(
                pod_id,
                String::from("a"),
                String::from("img"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            Err(CriError::SandboxNotReady(pod_id))
        );
    }

    #[test]
    fn test_remove_container() {
        let mut svc = make_service();
        let pod_id = svc
            .run_pod_sandbox(
                String::from("pod1"),
                String::from("default"),
                NetworkConfig::default(),
                100,
            )
            .unwrap();
        let cid = svc
            .create_container(
                pod_id,
                String::from("app"),
                String::from("img"),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        svc.remove_container(cid).unwrap();
        assert_eq!(svc.container_count(), 0);
    }
}
