//! L4 (Transport Layer) Load Balancer
//!
//! Provides TCP/UDP load balancing with multiple algorithms including
//! round-robin, least connections, weighted round-robin, random, and
//! IP hash.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------

/// A backend server in a virtual IP pool.
#[derive(Debug, Clone)]
pub struct Backend {
    /// Backend address (IP string).
    pub address: String,
    /// Backend port.
    pub port: u16,
    /// Weight for weighted algorithms (1-100).
    pub weight: u32,
    /// Whether the backend is healthy.
    pub healthy: bool,
    /// Number of active connections.
    pub active_connections: u32,
    /// Total requests served.
    pub total_requests: u64,
}

impl Backend {
    /// Create a new backend.
    pub fn new(address: String, port: u16, weight: u32) -> Self {
        Backend {
            address,
            port,
            weight,
            healthy: true,
            active_connections: 0,
            total_requests: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Load Balancing Algorithm
// ---------------------------------------------------------------------------

/// Load balancing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LbAlgorithm {
    /// Simple round-robin.
    #[default]
    RoundRobin,
    /// Least active connections.
    LeastConnections,
    /// Weighted round-robin.
    WeightedRoundRobin,
    /// Pseudo-random selection.
    Random,
    /// Hash of client IP.
    IpHash,
}

// ---------------------------------------------------------------------------
// Virtual IP
// ---------------------------------------------------------------------------

/// A virtual IP (VIP) with its backend pool.
#[derive(Debug, Clone)]
pub struct VirtualIp {
    /// VIP address.
    pub vip_addr: String,
    /// VIP port.
    pub vip_port: u16,
    /// Backend servers.
    pub backends: Vec<Backend>,
    /// Load balancing algorithm.
    pub algorithm: LbAlgorithm,
}

impl VirtualIp {
    /// Get the number of healthy backends.
    pub fn healthy_count(&self) -> usize {
        self.backends.iter().filter(|b| b.healthy).count()
    }
}

// ---------------------------------------------------------------------------
// L4 Error
// ---------------------------------------------------------------------------

/// L4 load balancer error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L4Error {
    /// VIP not found.
    VipNotFound(String),
    /// VIP already exists.
    VipAlreadyExists(String),
    /// No healthy backends.
    NoHealthyBackend,
    /// Backend not found.
    BackendNotFound(String),
}

// ---------------------------------------------------------------------------
// L4 Load Balancer
// ---------------------------------------------------------------------------

/// L4 Load Balancer implementation.
#[derive(Debug)]
pub struct L4LoadBalancer {
    /// Virtual IPs keyed by "addr:port".
    vips: BTreeMap<String, VirtualIp>,
    /// Round-robin counter (per VIP is cleaner but global works for
    /// simplicity).
    rr_counter: u64,
    /// Pseudo-random state.
    random_state: u64,
}

impl Default for L4LoadBalancer {
    fn default() -> Self {
        Self::new()
    }
}

impl L4LoadBalancer {
    /// Create a new L4 load balancer.
    pub fn new() -> Self {
        L4LoadBalancer {
            vips: BTreeMap::new(),
            rr_counter: 0,
            random_state: 0x12345678,
        }
    }

    /// VIP key string.
    fn vip_key(addr: &str, port: u16) -> String {
        alloc::format!("{}:{}", addr, port)
    }

    /// Add a virtual IP.
    pub fn add_vip(
        &mut self,
        vip_addr: String,
        vip_port: u16,
        algorithm: LbAlgorithm,
    ) -> Result<(), L4Error> {
        let key = Self::vip_key(&vip_addr, vip_port);
        if self.vips.contains_key(&key) {
            return Err(L4Error::VipAlreadyExists(key));
        }
        self.vips.insert(
            key,
            VirtualIp {
                vip_addr,
                vip_port,
                backends: Vec::new(),
                algorithm,
            },
        );
        Ok(())
    }

    /// Remove a virtual IP.
    pub fn remove_vip(&mut self, vip_addr: &str, vip_port: u16) -> Result<(), L4Error> {
        let key = Self::vip_key(vip_addr, vip_port);
        self.vips
            .remove(&key)
            .map(|_| ())
            .ok_or(L4Error::VipNotFound(key))
    }

    /// Add a backend to a VIP.
    pub fn add_backend(
        &mut self,
        vip_addr: &str,
        vip_port: u16,
        backend: Backend,
    ) -> Result<(), L4Error> {
        let key = Self::vip_key(vip_addr, vip_port);
        let vip = self.vips.get_mut(&key).ok_or(L4Error::VipNotFound(key))?;
        vip.backends.push(backend);
        Ok(())
    }

    /// Remove a backend from a VIP by address.
    pub fn remove_backend(
        &mut self,
        vip_addr: &str,
        vip_port: u16,
        backend_addr: &str,
    ) -> Result<(), L4Error> {
        let key = Self::vip_key(vip_addr, vip_port);
        let vip = self
            .vips
            .get_mut(&key)
            .ok_or_else(|| L4Error::VipNotFound(key.clone()))?;

        let before = vip.backends.len();
        vip.backends.retain(|b| b.address != backend_addr);
        if vip.backends.len() == before {
            return Err(L4Error::BackendNotFound(String::from(backend_addr)));
        }
        Ok(())
    }

    /// Select a backend using the VIP's configured algorithm.
    pub fn select_backend(
        &mut self,
        vip_addr: &str,
        vip_port: u16,
        client_ip: u32,
    ) -> Result<(String, u16), L4Error> {
        let key = Self::vip_key(vip_addr, vip_port);
        let vip = self
            .vips
            .get_mut(&key)
            .ok_or_else(|| L4Error::VipNotFound(key.clone()))?;

        let healthy: Vec<usize> = vip
            .backends
            .iter()
            .enumerate()
            .filter(|(_, b)| b.healthy)
            .map(|(i, _)| i)
            .collect();

        if healthy.is_empty() {
            return Err(L4Error::NoHealthyBackend);
        }

        let idx = match vip.algorithm {
            LbAlgorithm::RoundRobin => {
                let i = (self.rr_counter as usize) % healthy.len();
                self.rr_counter += 1;
                healthy[i]
            }
            LbAlgorithm::LeastConnections => {
                let mut min_idx = healthy[0];
                let mut min_conns = vip.backends[healthy[0]].active_connections;
                for &h in &healthy[1..] {
                    if vip.backends[h].active_connections < min_conns {
                        min_conns = vip.backends[h].active_connections;
                        min_idx = h;
                    }
                }
                min_idx
            }
            LbAlgorithm::WeightedRoundRobin => {
                // Weighted selection: pick highest weight among healthy
                let total_weight: u32 = healthy.iter().map(|&i| vip.backends[i].weight).sum();
                if total_weight == 0 {
                    healthy[0]
                } else {
                    let target = (self.rr_counter % total_weight as u64) as u32;
                    self.rr_counter += 1;
                    let mut cumulative = 0u32;
                    let mut selected = healthy[0];
                    for &h in &healthy {
                        cumulative += vip.backends[h].weight;
                        if target < cumulative {
                            selected = h;
                            break;
                        }
                    }
                    selected
                }
            }
            LbAlgorithm::Random => {
                // LCG pseudo-random
                self.random_state = self
                    .random_state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let i = (self.random_state >> 33) as usize % healthy.len();
                healthy[i]
            }
            LbAlgorithm::IpHash => {
                let hash = client_ip.wrapping_mul(2654435761);
                let i = (hash as usize) % healthy.len();
                healthy[i]
            }
        };

        vip.backends[idx].active_connections += 1;
        vip.backends[idx].total_requests += 1;

        Ok((vip.backends[idx].address.clone(), vip.backends[idx].port))
    }

    /// Run health checks on all backends in all VIPs.
    pub fn health_check(&mut self) {
        // Simulated: just mark backends based on active connections
        // In real code this would send probes.
        for vip in self.vips.values_mut() {
            for backend in &mut vip.backends {
                // If too many connections, consider unhealthy
                if backend.active_connections > 10000 {
                    backend.healthy = false;
                }
            }
        }
    }

    /// Mark a specific backend as healthy/unhealthy.
    pub fn set_backend_health(
        &mut self,
        vip_addr: &str,
        vip_port: u16,
        backend_addr: &str,
        healthy: bool,
    ) -> Result<(), L4Error> {
        let key = Self::vip_key(vip_addr, vip_port);
        let vip = self
            .vips
            .get_mut(&key)
            .ok_or_else(|| L4Error::VipNotFound(key.clone()))?;

        for backend in &mut vip.backends {
            if backend.address == backend_addr {
                backend.healthy = healthy;
                return Ok(());
            }
        }
        Err(L4Error::BackendNotFound(String::from(backend_addr)))
    }

    /// Get VIP info.
    pub fn get_vip(&self, vip_addr: &str, vip_port: u16) -> Option<&VirtualIp> {
        let key = Self::vip_key(vip_addr, vip_port);
        self.vips.get(&key)
    }

    /// List all VIPs.
    pub fn list_vips(&self) -> Vec<&VirtualIp> {
        self.vips.values().collect()
    }

    /// Get total number of VIPs.
    pub fn vip_count(&self) -> usize {
        self.vips.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::string::ToString;

    use super::*;

    fn make_lb() -> L4LoadBalancer {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::RoundRobin)
            .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.1"), 8080, 1),
        )
        .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.2"), 8080, 1),
        )
        .unwrap();
        lb
    }

    #[test]
    fn test_add_vip() {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::RoundRobin)
            .unwrap();
        assert_eq!(lb.vip_count(), 1);
    }

    #[test]
    fn test_add_duplicate_vip() {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::RoundRobin)
            .unwrap();
        assert!(lb
            .add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::RoundRobin)
            .is_err());
    }

    #[test]
    fn test_remove_vip() {
        let mut lb = make_lb();
        lb.remove_vip("10.96.0.1", 80).unwrap();
        assert_eq!(lb.vip_count(), 0);
    }

    #[test]
    fn test_round_robin() {
        let mut lb = make_lb();
        let (addr1, _) = lb.select_backend("10.96.0.1", 80, 0).unwrap();
        let (addr2, _) = lb.select_backend("10.96.0.1", 80, 0).unwrap();
        assert_ne!(addr1, addr2);
    }

    #[test]
    fn test_least_connections() {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::LeastConnections)
            .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.1"), 8080, 1),
        )
        .unwrap();
        let mut b2 = Backend::new(String::from("10.0.0.2"), 8080, 1);
        b2.active_connections = 5;
        lb.add_backend("10.96.0.1", 80, b2).unwrap();

        let (addr, _) = lb.select_backend("10.96.0.1", 80, 0).unwrap();
        assert_eq!(addr, "10.0.0.1"); // fewer connections
    }

    #[test]
    fn test_ip_hash_deterministic() {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(String::from("10.96.0.1"), 80, LbAlgorithm::IpHash)
            .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.1"), 8080, 1),
        )
        .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.2"), 8080, 1),
        )
        .unwrap();

        let client_ip: u32 = 0xC0A80001; // 192.168.0.1
        let (a1, _) = lb.select_backend("10.96.0.1", 80, client_ip).unwrap();
        let (a2, _) = lb.select_backend("10.96.0.1", 80, client_ip).unwrap();
        assert_eq!(a1, a2); // Same client IP -> same backend
    }

    #[test]
    fn test_no_healthy_backend() {
        let mut lb = make_lb();
        lb.set_backend_health("10.96.0.1", 80, "10.0.0.1", false)
            .unwrap();
        lb.set_backend_health("10.96.0.1", 80, "10.0.0.2", false)
            .unwrap();
        assert_eq!(
            lb.select_backend("10.96.0.1", 80, 0),
            Err(L4Error::NoHealthyBackend)
        );
    }

    #[test]
    fn test_remove_backend() {
        let mut lb = make_lb();
        lb.remove_backend("10.96.0.1", 80, "10.0.0.1").unwrap();
        let vip = lb.get_vip("10.96.0.1", 80).unwrap();
        assert_eq!(vip.backends.len(), 1);
    }

    #[test]
    fn test_weighted_round_robin() {
        let mut lb = L4LoadBalancer::new();
        lb.add_vip(
            String::from("10.96.0.1"),
            80,
            LbAlgorithm::WeightedRoundRobin,
        )
        .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.1"), 8080, 3),
        )
        .unwrap();
        lb.add_backend(
            "10.96.0.1",
            80,
            Backend::new(String::from("10.0.0.2"), 8080, 1),
        )
        .unwrap();

        // With weights 3:1, in 4 selections ~3 should go to first
        let mut count_first = 0;
        for _ in 0..4 {
            let (addr, _) = lb.select_backend("10.96.0.1", 80, 0).unwrap();
            if addr == "10.0.0.1" {
                count_first += 1;
            }
        }
        assert!(count_first >= 2); // Should favor first backend
    }

    #[test]
    fn test_vip_not_found() {
        let mut lb = L4LoadBalancer::new();
        assert_eq!(
            lb.select_backend("10.96.0.1", 80, 0),
            Err(L4Error::VipNotFound(String::from("10.96.0.1:80")))
        );
    }
}
