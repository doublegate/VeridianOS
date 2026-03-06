//! Service Discovery
//!
//! Provides a service registry with DNS-based resolution following
//! Kubernetes-style naming (name.namespace.svc.cluster.local).

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Service Types
// ---------------------------------------------------------------------------

/// Protocol for a service endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
    /// HTTP.
    Http,
    /// gRPC.
    Grpc,
}

/// A single endpoint (instance) of a service.
#[derive(Debug, Clone)]
pub struct ServiceEndpoint {
    /// IP address (as a dotted string or integer).
    pub address: String,
    /// Port number.
    pub port: u16,
    /// Protocol.
    pub protocol: Protocol,
    /// Whether this endpoint is healthy.
    pub healthy: bool,
}

/// A registered service with its endpoints.
#[derive(Debug, Clone)]
pub struct ServiceEntry {
    /// Service name.
    pub name: String,
    /// Namespace.
    pub namespace: String,
    /// Active endpoints.
    pub endpoints: Vec<ServiceEndpoint>,
    /// Labels for selection.
    pub labels: BTreeMap<String, String>,
    /// Cluster IP (virtual IP).
    pub cluster_ip: Option<String>,
    /// Port that the service exposes.
    pub service_port: u16,
}

// ---------------------------------------------------------------------------
// Discovery Error
// ---------------------------------------------------------------------------

/// Service discovery error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// Service not found.
    NotFound(String),
    /// Service already registered.
    AlreadyRegistered(String),
    /// No healthy endpoints.
    NoHealthyEndpoints(String),
}

// ---------------------------------------------------------------------------
// Service Registry
// ---------------------------------------------------------------------------

/// Key for service lookup: (namespace, name).
type ServiceKey = (String, String);

/// Service registry and discovery.
#[derive(Debug)]
pub struct ServiceRegistry {
    /// Services keyed by (namespace, name).
    services: BTreeMap<ServiceKey, ServiceEntry>,
    /// DNS domain suffix.
    domain: String,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    /// Create a new service registry.
    pub fn new() -> Self {
        ServiceRegistry {
            services: BTreeMap::new(),
            domain: String::from("cluster.local"),
        }
    }

    /// Create with a custom domain.
    pub fn with_domain(domain: String) -> Self {
        ServiceRegistry {
            services: BTreeMap::new(),
            domain,
        }
    }

    /// Register a new service.
    pub fn register(&mut self, entry: ServiceEntry) -> Result<(), DiscoveryError> {
        let key = (entry.namespace.clone(), entry.name.clone());
        if self.services.contains_key(&key) {
            return Err(DiscoveryError::AlreadyRegistered(entry.name));
        }
        self.services.insert(key, entry);
        Ok(())
    }

    /// Deregister a service.
    pub fn deregister(
        &mut self,
        name: &str,
        namespace: &str,
    ) -> Result<ServiceEntry, DiscoveryError> {
        let key = (String::from(namespace), String::from(name));
        self.services
            .remove(&key)
            .ok_or_else(|| DiscoveryError::NotFound(String::from(name)))
    }

    /// Look up a service by name and namespace.
    pub fn lookup(&self, name: &str, namespace: &str) -> Result<&ServiceEntry, DiscoveryError> {
        let key = (String::from(namespace), String::from(name));
        self.services
            .get(&key)
            .ok_or_else(|| DiscoveryError::NotFound(String::from(name)))
    }

    /// List all services, optionally filtered by namespace.
    pub fn list(&self, namespace_filter: Option<&str>) -> Vec<&ServiceEntry> {
        self.services
            .values()
            .filter(|s| {
                namespace_filter.is_none() || namespace_filter == Some(s.namespace.as_str())
            })
            .collect()
    }

    /// Resolve a Kubernetes-style DNS name to service endpoints.
    ///
    /// Format: `name.namespace.svc.cluster.local`
    pub fn resolve_service_dns(&self, dns_name: &str) -> Result<&ServiceEntry, DiscoveryError> {
        // Parse: name.namespace.svc.domain
        let parts: Vec<&str> = dns_name.splitn(4, '.').collect();
        if parts.len() < 2 {
            return Err(DiscoveryError::NotFound(String::from(dns_name)));
        }

        let name = parts[0];
        let namespace = parts[1];
        self.lookup(name, namespace)
    }

    /// Get the fully qualified DNS name for a service.
    pub fn fqdn(&self, name: &str, namespace: &str) -> String {
        alloc::format!("{}.{}.svc.{}", name, namespace, self.domain)
    }

    /// Get healthy endpoints for a service.
    pub fn healthy_endpoints(
        &self,
        name: &str,
        namespace: &str,
    ) -> Result<Vec<&ServiceEndpoint>, DiscoveryError> {
        let entry = self.lookup(name, namespace)?;
        let healthy: Vec<&ServiceEndpoint> = entry.endpoints.iter().filter(|e| e.healthy).collect();
        if healthy.is_empty() {
            return Err(DiscoveryError::NoHealthyEndpoints(String::from(name)));
        }
        Ok(healthy)
    }

    /// Update endpoint health for a service.
    pub fn update_endpoint_health(
        &mut self,
        name: &str,
        namespace: &str,
        address: &str,
        healthy: bool,
    ) -> Result<(), DiscoveryError> {
        let key = (String::from(namespace), String::from(name));
        let entry = self
            .services
            .get_mut(&key)
            .ok_or_else(|| DiscoveryError::NotFound(String::from(name)))?;

        for ep in &mut entry.endpoints {
            if ep.address == address {
                ep.healthy = healthy;
                return Ok(());
            }
        }
        Ok(())
    }

    /// Get the total number of registered services.
    pub fn service_count(&self) -> usize {
        self.services.len()
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

    fn make_entry(name: &str, namespace: &str) -> ServiceEntry {
        ServiceEntry {
            name: String::from(name),
            namespace: String::from(namespace),
            endpoints: vec![
                ServiceEndpoint {
                    address: String::from("10.0.0.1"),
                    port: 8080,
                    protocol: Protocol::Http,
                    healthy: true,
                },
                ServiceEndpoint {
                    address: String::from("10.0.0.2"),
                    port: 8080,
                    protocol: Protocol::Http,
                    healthy: true,
                },
            ],
            labels: BTreeMap::new(),
            cluster_ip: Some(String::from("10.96.0.10")),
            service_port: 80,
        }
    }

    #[test]
    fn test_register_and_lookup() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        let entry = registry.lookup("nginx", "default").unwrap();
        assert_eq!(entry.name, "nginx");
        assert_eq!(entry.endpoints.len(), 2);
    }

    #[test]
    fn test_register_duplicate() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        assert!(registry.register(make_entry("nginx", "default")).is_err());
    }

    #[test]
    fn test_deregister() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        let entry = registry.deregister("nginx", "default").unwrap();
        assert_eq!(entry.name, "nginx");
        assert_eq!(registry.service_count(), 0);
    }

    #[test]
    fn test_list_with_filter() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("svc1", "default")).unwrap();
        registry
            .register(make_entry("svc2", "kube-system"))
            .unwrap();

        let all = registry.list(None);
        assert_eq!(all.len(), 2);

        let filtered = registry.list(Some("default"));
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_dns_resolution() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        let entry = registry
            .resolve_service_dns("nginx.default.svc.cluster.local")
            .unwrap();
        assert_eq!(entry.name, "nginx");
    }

    #[test]
    fn test_fqdn() {
        let registry = ServiceRegistry::new();
        assert_eq!(
            registry.fqdn("nginx", "default"),
            "nginx.default.svc.cluster.local"
        );
    }

    #[test]
    fn test_healthy_endpoints() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        let healthy = registry.healthy_endpoints("nginx", "default").unwrap();
        assert_eq!(healthy.len(), 2);
    }

    #[test]
    fn test_update_endpoint_health() {
        let mut registry = ServiceRegistry::new();
        registry.register(make_entry("nginx", "default")).unwrap();
        registry
            .update_endpoint_health("nginx", "default", "10.0.0.1", false)
            .unwrap();
        let healthy = registry.healthy_endpoints("nginx", "default").unwrap();
        assert_eq!(healthy.len(), 1);
    }
}
