//! Linux-compatible namespace isolation for containers

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, string::String};

use crate::{error::KernelError, process::ProcessId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NamespaceType {
    Pid,
    Mount,
    Network,
    User,
    Ipc,
    Uts,
}

impl core::fmt::Display for NamespaceType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Pid => write!(f, "pid"),
            Self::Mount => write!(f, "mnt"),
            Self::Network => write!(f, "net"),
            Self::User => write!(f, "user"),
            Self::Ipc => write!(f, "ipc"),
            Self::Uts => write!(f, "uts"),
        }
    }
}

#[cfg(feature = "alloc")]
pub struct PidNamespace {
    container_to_global: BTreeMap<u32, ProcessId>,
    global_to_container: BTreeMap<u64, u32>,
    next_container_pid: u32,
}

#[cfg(feature = "alloc")]
impl PidNamespace {
    pub fn new() -> Self {
        Self {
            container_to_global: BTreeMap::new(),
            global_to_container: BTreeMap::new(),
            next_container_pid: 1,
        }
    }
    pub fn add_process(&mut self, global_pid: ProcessId) -> u32 {
        let cpid = self.next_container_pid;
        self.next_container_pid += 1;
        self.container_to_global.insert(cpid, global_pid);
        self.global_to_container.insert(global_pid.0, cpid);
        cpid
    }
    pub fn remove_process(&mut self, global_pid: ProcessId) {
        if let Some(cpid) = self.global_to_container.remove(&global_pid.0) {
            self.container_to_global.remove(&cpid);
        }
    }
    pub fn translate_pid(&self, container_pid: u32) -> Option<ProcessId> {
        self.container_to_global.get(&container_pid).copied()
    }
    pub fn container_pid(&self, global_pid: ProcessId) -> Option<u32> {
        self.global_to_container.get(&global_pid.0).copied()
    }
    pub fn process_count(&self) -> usize {
        self.container_to_global.len()
    }
    pub fn contains(&self, global_pid: ProcessId) -> bool {
        self.global_to_container.contains_key(&global_pid.0)
    }
}

#[cfg(feature = "alloc")]
impl Default for PidNamespace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
pub struct MountNamespace {
    root_path: String,
    mounts: BTreeMap<String, String>,
}

#[cfg(feature = "alloc")]
impl MountNamespace {
    pub fn new(root: &str) -> Self {
        Self {
            root_path: String::from(root),
            mounts: BTreeMap::new(),
        }
    }
    pub fn set_root(&mut self, path: &str) {
        self.root_path = String::from(path);
    }
    pub fn root(&self) -> &str {
        &self.root_path
    }
    pub fn add_mount(&mut self, mountpoint: &str, source: &str) {
        self.mounts
            .insert(String::from(mountpoint), String::from(source));
    }
    pub fn remove_mount(&mut self, mountpoint: &str) {
        self.mounts.remove(mountpoint);
    }
    pub fn mount_count(&self) -> usize {
        self.mounts.len()
    }
    pub fn resolve_path(&self, path: &str) -> String {
        if path.starts_with('/') {
            alloc::format!("{}{}", self.root_path, path)
        } else {
            alloc::format!("{}/{}", self.root_path, path)
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for MountNamespace {
    fn default() -> Self {
        Self::new("/")
    }
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct VethInterface {
    pub name: String,
    pub peer_name: String,
    pub ipv4_addr: Option<u32>,
    pub is_up: bool,
}

#[cfg(feature = "alloc")]
pub struct NetworkNamespace {
    interfaces: BTreeMap<String, VethInterface>,
    has_loopback: bool,
}

#[cfg(feature = "alloc")]
impl NetworkNamespace {
    pub fn new() -> Self {
        Self {
            interfaces: BTreeMap::new(),
            has_loopback: true,
        }
    }
    pub fn create_veth(&mut self, name: &str, peer_name: &str) -> Result<(), KernelError> {
        self.interfaces.insert(
            String::from(name),
            VethInterface {
                name: String::from(name),
                peer_name: String::from(peer_name),
                ipv4_addr: None,
                is_up: false,
            },
        );
        Ok(())
    }
    pub fn interface_up(&mut self, name: &str) -> Result<(), KernelError> {
        match self.interfaces.get_mut(name) {
            Some(i) => {
                i.is_up = true;
                Ok(())
            }
            None => Err(KernelError::NotFound {
                resource: "network interface",
                id: 0,
            }),
        }
    }
    pub fn assign_ipv4(&mut self, name: &str, addr: u32) -> Result<(), KernelError> {
        match self.interfaces.get_mut(name) {
            Some(i) => {
                i.ipv4_addr = Some(addr);
                Ok(())
            }
            None => Err(KernelError::NotFound {
                resource: "network interface",
                id: 0,
            }),
        }
    }
    pub fn interface_count(&self) -> usize {
        self.interfaces.len()
    }
    pub fn has_loopback(&self) -> bool {
        self.has_loopback
    }
}

#[cfg(feature = "alloc")]
impl Default for NetworkNamespace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
pub struct UtsNamespace {
    hostname: String,
    domainname: String,
}

#[cfg(feature = "alloc")]
impl UtsNamespace {
    pub fn new(hostname: &str) -> Self {
        Self {
            hostname: String::from(hostname),
            domainname: String::new(),
        }
    }
    pub fn hostname(&self) -> &str {
        &self.hostname
    }
    pub fn set_hostname(&mut self, name: &str) {
        self.hostname = String::from(name);
    }
    pub fn domainname(&self) -> &str {
        &self.domainname
    }
    pub fn set_domainname(&mut self, name: &str) {
        self.domainname = String::from(name);
    }
}

#[cfg(feature = "alloc")]
impl Default for UtsNamespace {
    fn default() -> Self {
        Self::new("localhost")
    }
}

#[cfg(feature = "alloc")]
pub struct NamespaceSet {
    pub pid: PidNamespace,
    pub mount: MountNamespace,
    pub network: NetworkNamespace,
    pub uts: UtsNamespace,
}

#[cfg(feature = "alloc")]
impl NamespaceSet {
    pub fn new(hostname: &str) -> Self {
        Self {
            pid: PidNamespace::new(),
            mount: MountNamespace::new("/"),
            network: NetworkNamespace::new(),
            uts: UtsNamespace::new(hostname),
        }
    }
}

#[cfg(feature = "alloc")]
impl Default for NamespaceSet {
    fn default() -> Self {
        Self::new("container")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_namespace() {
        let mut ns = PidNamespace::new();
        let cpid = ns.add_process(ProcessId(42));
        assert_eq!(cpid, 1);
        assert_eq!(ns.translate_pid(1), Some(ProcessId(42)));
        assert_eq!(ns.container_pid(ProcessId(42)), Some(1));
        ns.remove_process(ProcessId(42));
        assert_eq!(ns.process_count(), 0);
    }

    #[test]
    fn test_mount_namespace() {
        let ns = MountNamespace::new("/containers/test");
        assert_eq!(ns.resolve_path("/bin/sh"), "/containers/test/bin/sh");
    }

    #[test]
    fn test_network_namespace() {
        let mut ns = NetworkNamespace::new();
        assert!(ns.create_veth("veth0", "veth0-host").is_ok());
        assert_eq!(ns.interface_count(), 1);
        assert!(ns.interface_up("veth0").is_ok());
        assert!(ns.interface_up("nonexistent").is_err());
    }

    #[test]
    fn test_uts_namespace() {
        let mut ns = UtsNamespace::new("test");
        assert_eq!(ns.hostname(), "test");
        ns.set_hostname("new");
        assert_eq!(ns.hostname(), "new");
    }

    #[test]
    fn test_namespace_type_display() {
        assert_eq!(alloc::format!("{}", NamespaceType::Pid), "pid");
        assert_eq!(alloc::format!("{}", NamespaceType::Mount), "mnt");
    }
}
