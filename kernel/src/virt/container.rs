//! Container management using namespace isolation

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};

use super::namespace::NamespaceSet;
use crate::{error::KernelError, process::ProcessId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Created,
    Running,
    Stopped,
}

impl core::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Created => write!(f, "Created"),
            Self::Running => write!(f, "Running"),
            Self::Stopped => write!(f, "Stopped"),
        }
    }
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: u64,
    pub name: String,
    pub state: ContainerState,
    pub root_pid: Option<ProcessId>,
    pub process_count: usize,
    pub hostname: String,
}

#[cfg(feature = "alloc")]
pub struct Container {
    pub id: u64,
    pub name: String,
    pub namespaces: NamespaceSet,
    pub root_process: Option<ProcessId>,
    pub state: ContainerState,
    pub init_program: String,
}

#[cfg(feature = "alloc")]
impl Container {
    pub fn info(&self) -> ContainerInfo {
        ContainerInfo {
            id: self.id,
            name: self.name.clone(),
            state: self.state,
            root_pid: self.root_process,
            process_count: self.namespaces.pid.process_count(),
            hostname: self.namespaces.uts.hostname().to_string(),
        }
    }
}

static NEXT_CONTAINER_ID: AtomicU64 = AtomicU64::new(1);
pub static CONTAINER_MGR: spin::Mutex<Option<ContainerManager>> = spin::Mutex::new(None);

#[cfg(feature = "alloc")]
pub struct ContainerManager {
    containers: BTreeMap<u64, Container>,
}

#[cfg(feature = "alloc")]
impl ContainerManager {
    pub fn new() -> Self {
        Self {
            containers: BTreeMap::new(),
        }
    }

    pub fn create(&mut self, name: &str) -> Result<u64, KernelError> {
        let id = NEXT_CONTAINER_ID.fetch_add(1, Ordering::Relaxed);
        self.containers.insert(
            id,
            Container {
                id,
                name: String::from(name),
                namespaces: NamespaceSet::new(name),
                root_process: None,
                state: ContainerState::Created,
                init_program: String::new(),
            },
        );
        crate::println!("  [container] Created container {} (id={})", name, id);
        Ok(id)
    }

    pub fn start(&mut self, id: u64, program: &str) -> Result<(), KernelError> {
        let container = self.containers.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "container",
            id,
        })?;
        if container.state != ContainerState::Created {
            return Err(KernelError::InvalidState {
                expected: "Created",
                actual: match container.state {
                    ContainerState::Running => "Running",
                    ContainerState::Stopped => "Stopped",
                    ContainerState::Created => "Created",
                },
            });
        }
        container.init_program = String::from(program);
        let placeholder_pid = ProcessId(1000 + id);
        let cpid = container.namespaces.pid.add_process(placeholder_pid);
        container.root_process = Some(placeholder_pid);
        container.state = ContainerState::Running;
        crate::println!(
            "  [container] Started {} (id={}, root_pid={}, container_pid={})",
            container.name,
            id,
            placeholder_pid.0,
            cpid
        );
        Ok(())
    }

    pub fn stop(&mut self, id: u64) -> Result<(), KernelError> {
        let container = self.containers.get_mut(&id).ok_or(KernelError::NotFound {
            resource: "container",
            id,
        })?;
        if container.state != ContainerState::Running {
            return Err(KernelError::InvalidState {
                expected: "Running",
                actual: match container.state {
                    ContainerState::Created => "Created",
                    ContainerState::Stopped => "Stopped",
                    ContainerState::Running => "Running",
                },
            });
        }
        if let Some(root_pid) = container.root_process.take() {
            container.namespaces.pid.remove_process(root_pid);
        }
        container.state = ContainerState::Stopped;
        crate::println!("  [container] Stopped {} (id={})", container.name, id);
        Ok(())
    }

    pub fn destroy(&mut self, id: u64) -> Result<(), KernelError> {
        let container = self.containers.get(&id).ok_or(KernelError::NotFound {
            resource: "container",
            id,
        })?;
        if container.state == ContainerState::Running {
            return Err(KernelError::InvalidState {
                expected: "Created or Stopped",
                actual: "Running",
            });
        }
        let name = container.name.clone();
        self.containers.remove(&id);
        crate::println!("  [container] Destroyed {} (id={})", name, id);
        Ok(())
    }

    pub fn list(&self) -> Vec<ContainerInfo> {
        self.containers.values().map(|c| c.info()).collect()
    }
    pub fn container_count(&self) -> usize {
        self.containers.len()
    }
    pub fn get(&self, id: u64) -> Option<&Container> {
        self.containers.get(&id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Container> {
        self.containers.get_mut(&id)
    }
}

#[cfg(feature = "alloc")]
impl Default for ContainerManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init() {
    #[cfg(feature = "alloc")]
    {
        let mut mgr = CONTAINER_MGR.lock();
        *mgr = Some(ContainerManager::new());
    }
    crate::println!("  [container] Container manager initialized");
}

#[cfg(feature = "alloc")]
pub fn with_container_manager<R, F: FnOnce(&mut ContainerManager) -> R>(
    f: F,
) -> Result<R, KernelError> {
    let mut mgr = CONTAINER_MGR.lock();
    match mgr.as_mut() {
        Some(m) => Ok(f(m)),
        None => Err(KernelError::NotInitialized {
            subsystem: "container manager",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_lifecycle() {
        let mut mgr = ContainerManager::new();
        let id = mgr.create("test").unwrap();
        assert_eq!(mgr.container_count(), 1);
        assert!(mgr.start(id, "/bin/init").is_ok());
        assert_eq!(mgr.get(id).unwrap().state, ContainerState::Running);
        assert!(mgr.stop(id).is_ok());
        assert_eq!(mgr.get(id).unwrap().state, ContainerState::Stopped);
        assert!(mgr.destroy(id).is_ok());
        assert_eq!(mgr.container_count(), 0);
    }

    #[test]
    fn test_container_destroy_running_fails() {
        let mut mgr = ContainerManager::new();
        let id = mgr.create("test").unwrap();
        mgr.start(id, "/bin/init").unwrap();
        assert!(mgr.destroy(id).is_err());
    }

    #[test]
    fn test_container_list() {
        let mut mgr = ContainerManager::new();
        mgr.create("web").unwrap();
        mgr.create("db").unwrap();
        assert_eq!(mgr.list().len(), 2);
    }

    #[test]
    fn test_container_state_display() {
        assert_eq!(alloc::format!("{}", ContainerState::Created), "Created");
        assert_eq!(alloc::format!("{}", ContainerState::Running), "Running");
        assert_eq!(alloc::format!("{}", ContainerState::Stopped), "Stopped");
    }
}
