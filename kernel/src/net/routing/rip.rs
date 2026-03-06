//! RIP v2 routing daemon (RFC 2453)
//!
//! Distance-vector routing protocol with:
//! - Split horizon with poisoned reverse
//! - Triggered updates
//! - Route timeout and garbage collection
//! - Multicast group 224.0.0.9, port 520

use alloc::{collections::BTreeMap, vec::Vec};

use crate::net::Ipv4Address;

/// RIP infinity metric (unreachable)
pub const RIP_INFINITY: u32 = 16;

/// RIP v2 default port
pub const RIP_PORT: u16 = 520;

/// RIP v2 multicast address (224.0.0.9)
pub const RIP_MULTICAST: Ipv4Address = Ipv4Address([224, 0, 0, 9]);

/// Maximum entries per RIP message
pub const RIP_MAX_ENTRIES: usize = 25;

/// Default advertisement interval in ticks
pub const RIP_UPDATE_INTERVAL: u64 = 30;

/// Route timeout in ticks (no update received)
pub const RIP_TIMEOUT: u64 = 180;

/// Garbage collection timer in ticks (after timeout)
pub const RIP_GARBAGE_COLLECT: u64 = 120;

/// RIP command type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RipCommand {
    Request = 1,
    Response = 2,
}

impl RipCommand {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Request),
            2 => Some(Self::Response),
            _ => None,
        }
    }
}

/// RIP v2 route entry (wire format)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RipEntry {
    /// Address family identifier (2 = IP)
    pub address_family: u16,
    /// Route tag for EGP/IGP distinction
    pub route_tag: u16,
    /// Destination IP address
    pub ip_address: Ipv4Address,
    /// Subnet mask
    pub subnet_mask: Ipv4Address,
    /// Next hop address
    pub next_hop: Ipv4Address,
    /// Metric (1-16, 16 = infinity)
    pub metric: u32,
}

/// RIP v2 message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RipMessage {
    /// Command (Request or Response)
    pub command: RipCommand,
    /// Version (always 2)
    pub version: u8,
    /// Route entries (up to 25)
    pub entries: Vec<RipEntry>,
}

impl RipMessage {
    /// Create a new RIP request message
    pub fn new_request() -> Self {
        Self {
            command: RipCommand::Request,
            version: 2,
            entries: Vec::new(),
        }
    }

    /// Create a new RIP response message
    pub fn new_response() -> Self {
        Self {
            command: RipCommand::Response,
            version: 2,
            entries: Vec::new(),
        }
    }
}

/// Internal route entry stored in the daemon's table
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RipRoute {
    /// Destination network address
    pub destination: Ipv4Address,
    /// Prefix length (e.g. 24 for /24)
    pub prefix_len: u8,
    /// Next hop router
    pub next_hop: Ipv4Address,
    /// Metric (hop count, 1-16)
    pub metric: u32,
    /// Route tag
    pub route_tag: u16,
    /// Tick when this route was last updated
    pub last_update: u64,
    /// Whether route is in garbage collection state
    pub garbage: bool,
    /// Source router that advertised this route
    pub source: Ipv4Address,
}

/// Destination key for route table lookups
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DestinationKey {
    /// Network address as u32
    pub network: u32,
    /// Prefix length
    pub prefix_len: u8,
}

impl DestinationKey {
    pub fn new(addr: Ipv4Address, prefix_len: u8) -> Self {
        let mask = prefix_to_mask(prefix_len);
        Self {
            network: addr.to_u32() & mask,
            prefix_len,
        }
    }
}

/// RIP v2 routing daemon
pub struct RipDaemon {
    /// Route table indexed by destination
    routes: BTreeMap<DestinationKey, RipRoute>,
    /// Current tick counter
    current_tick: u64,
    /// Tick when last periodic update was sent
    last_update_tick: u64,
    /// Whether a triggered update is pending
    triggered_update_pending: bool,
    /// Local router address
    _router_address: Ipv4Address,
}

impl RipDaemon {
    /// Create a new RIP daemon
    pub fn new(router_address: Ipv4Address) -> Self {
        Self {
            routes: BTreeMap::new(),
            current_tick: 0,
            last_update_tick: 0,
            triggered_update_pending: false,
            _router_address: router_address,
        }
    }

    /// Advance the tick counter and age routes
    pub fn tick(&mut self, ticks: u64) {
        self.current_tick += ticks;
    }

    /// Check if periodic update is due
    pub fn is_update_due(&self) -> bool {
        self.current_tick.saturating_sub(self.last_update_tick) >= RIP_UPDATE_INTERVAL
    }

    /// Check if a triggered update is pending
    pub fn has_triggered_update(&self) -> bool {
        self.triggered_update_pending
    }

    /// Clear triggered update flag and record update time
    pub fn mark_update_sent(&mut self) {
        self.triggered_update_pending = false;
        self.last_update_tick = self.current_tick;
    }

    /// Add or update a route
    pub fn add_route(&mut self, route: RipRoute) {
        let key = DestinationKey::new(route.destination, route.prefix_len);
        self.routes.insert(key, route);
    }

    /// Remove a route by destination
    pub fn remove_route(&mut self, destination: Ipv4Address, prefix_len: u8) -> Option<RipRoute> {
        let key = DestinationKey::new(destination, prefix_len);
        self.routes.remove(&key)
    }

    /// Get a route by destination
    pub fn get_route(&self, destination: Ipv4Address, prefix_len: u8) -> Option<&RipRoute> {
        let key = DestinationKey::new(destination, prefix_len);
        self.routes.get(&key)
    }

    /// Get total number of routes
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Age routes: mark timed-out routes for garbage collection, remove expired
    /// garbage
    pub fn age_routes(&mut self) {
        let current = self.current_tick;
        let mut to_remove = Vec::new();

        for (key, route) in self.routes.iter_mut() {
            let age = current.saturating_sub(route.last_update);

            if route.garbage {
                // In garbage collection: remove after RIP_GARBAGE_COLLECT ticks
                if age >= RIP_TIMEOUT + RIP_GARBAGE_COLLECT {
                    to_remove.push(*key);
                }
            } else if age >= RIP_TIMEOUT {
                // Route timed out: set metric to infinity, start garbage collection
                route.metric = RIP_INFINITY;
                route.garbage = true;
            }
        }

        for key in &to_remove {
            self.routes.remove(key);
        }

        if !to_remove.is_empty() {
            self.triggered_update_pending = true;
        }
    }

    /// Process an incoming RIP message from a neighbor
    pub fn process_message(&mut self, msg: &RipMessage, source: Ipv4Address) {
        match msg.command {
            RipCommand::Request => {
                // Requests are handled by generating a response (caller should
                // call generate_response)
            }
            RipCommand::Response => {
                for entry in &msg.entries {
                    self.process_response_entry(entry, source);
                }
            }
        }
    }

    /// Process a single RIP response entry
    fn process_response_entry(&mut self, entry: &RipEntry, source: Ipv4Address) {
        // Validate metric
        let new_metric = (entry.metric + 1).min(RIP_INFINITY);
        let prefix_len = mask_to_prefix(entry.subnet_mask.to_u32());
        let key = DestinationKey::new(entry.ip_address, prefix_len);

        let next_hop = if entry.next_hop == Ipv4Address::ANY {
            source
        } else {
            entry.next_hop
        };

        if let Some(existing) = self.routes.get(&key) {
            if existing.source == source {
                // Same source: always update (even if metric increased)
                let changed = existing.metric != new_metric;
                let route = RipRoute {
                    destination: entry.ip_address,
                    prefix_len,
                    next_hop,
                    metric: new_metric,
                    route_tag: entry.route_tag,
                    last_update: self.current_tick,
                    garbage: new_metric >= RIP_INFINITY,
                    source,
                };
                self.routes.insert(key, route);
                if changed {
                    self.triggered_update_pending = true;
                }
            } else if new_metric < existing.metric {
                // Better metric from different source: adopt
                let route = RipRoute {
                    destination: entry.ip_address,
                    prefix_len,
                    next_hop,
                    metric: new_metric,
                    route_tag: entry.route_tag,
                    last_update: self.current_tick,
                    garbage: false,
                    source,
                };
                self.routes.insert(key, route);
                self.triggered_update_pending = true;
            }
            // Worse metric from different source: ignore
        } else if new_metric < RIP_INFINITY {
            // New route
            let route = RipRoute {
                destination: entry.ip_address,
                prefix_len,
                next_hop,
                metric: new_metric,
                route_tag: entry.route_tag,
                last_update: self.current_tick,
                garbage: false,
                source,
            };
            self.routes.insert(key, route);
            self.triggered_update_pending = true;
        }
    }

    /// Generate a RIP response message for a neighbor (split horizon with
    /// poisoned reverse)
    pub fn generate_response(&self, neighbor: Ipv4Address) -> Vec<RipMessage> {
        let mut messages = Vec::new();
        let mut current_msg = RipMessage::new_response();

        for route in self.routes.values() {
            // Split horizon with poisoned reverse:
            // Routes learned from this neighbor are advertised back with infinity metric
            let metric = if route.source == neighbor {
                RIP_INFINITY
            } else {
                route.metric
            };

            let entry = RipEntry {
                address_family: 2,
                route_tag: route.route_tag,
                ip_address: route.destination,
                subnet_mask: Ipv4Address::from_u32(prefix_to_mask(route.prefix_len)),
                next_hop: Ipv4Address::ANY,
                metric,
            };

            current_msg.entries.push(entry);

            if current_msg.entries.len() >= RIP_MAX_ENTRIES {
                messages.push(current_msg);
                current_msg = RipMessage::new_response();
            }
        }

        if !current_msg.entries.is_empty() {
            messages.push(current_msg);
        }

        messages
    }
}

/// Serialize a RIP message to wire format bytes
pub fn serialize_message(msg: &RipMessage) -> Vec<u8> {
    // Header: command(1) + version(1) + zero(2) = 4 bytes
    // Each entry: 20 bytes
    let size = 4 + msg.entries.len() * 20;
    let mut buf = Vec::with_capacity(size);

    buf.push(msg.command as u8);
    buf.push(msg.version);
    buf.push(0); // must be zero
    buf.push(0);

    for entry in &msg.entries {
        // Address family (2 bytes, big-endian)
        buf.extend_from_slice(&entry.address_family.to_be_bytes());
        // Route tag (2 bytes)
        buf.extend_from_slice(&entry.route_tag.to_be_bytes());
        // IP address (4 bytes)
        buf.extend_from_slice(&entry.ip_address.0);
        // Subnet mask (4 bytes)
        buf.extend_from_slice(&entry.subnet_mask.0);
        // Next hop (4 bytes)
        buf.extend_from_slice(&entry.next_hop.0);
        // Metric (4 bytes, big-endian)
        buf.extend_from_slice(&entry.metric.to_be_bytes());
    }

    buf
}

/// Deserialize a RIP message from wire format bytes
pub fn deserialize_message(data: &[u8]) -> Option<RipMessage> {
    if data.len() < 4 {
        return None;
    }

    let command = RipCommand::from_u8(data[0])?;
    let version = data[1];

    if version != 2 {
        return None;
    }

    let entry_data = &data[4..];
    if !entry_data.len().is_multiple_of(20) {
        return None;
    }

    let num_entries = entry_data.len() / 20;
    if num_entries > RIP_MAX_ENTRIES {
        return None;
    }

    let mut entries = Vec::with_capacity(num_entries);
    for i in 0..num_entries {
        let offset = i * 20;
        let e = &entry_data[offset..offset + 20];

        let address_family = u16::from_be_bytes([e[0], e[1]]);
        let route_tag = u16::from_be_bytes([e[2], e[3]]);
        let ip_address = Ipv4Address([e[4], e[5], e[6], e[7]]);
        let subnet_mask = Ipv4Address([e[8], e[9], e[10], e[11]]);
        let next_hop = Ipv4Address([e[12], e[13], e[14], e[15]]);
        let metric = u32::from_be_bytes([e[16], e[17], e[18], e[19]]);

        if metric > RIP_INFINITY {
            return None;
        }

        entries.push(RipEntry {
            address_family,
            route_tag,
            ip_address,
            subnet_mask,
            next_hop,
            metric,
        });
    }

    Some(RipMessage {
        command,
        version,
        entries,
    })
}

/// Convert prefix length to 32-bit subnet mask
fn prefix_to_mask(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        0
    } else if prefix_len >= 32 {
        0xFFFF_FFFF
    } else {
        !((1u32 << (32 - prefix_len)) - 1)
    }
}

/// Convert 32-bit subnet mask to prefix length
fn mask_to_prefix(mask: u32) -> u8 {
    mask.leading_ones() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_to_mask() {
        assert_eq!(prefix_to_mask(0), 0x0000_0000);
        assert_eq!(prefix_to_mask(8), 0xFF00_0000);
        assert_eq!(prefix_to_mask(16), 0xFFFF_0000);
        assert_eq!(prefix_to_mask(24), 0xFFFF_FF00);
        assert_eq!(prefix_to_mask(32), 0xFFFF_FFFF);
    }

    #[test]
    fn test_mask_to_prefix() {
        assert_eq!(mask_to_prefix(0x0000_0000), 0);
        assert_eq!(mask_to_prefix(0xFF00_0000), 8);
        assert_eq!(mask_to_prefix(0xFFFF_FF00), 24);
        assert_eq!(mask_to_prefix(0xFFFF_FFFF), 32);
    }

    #[test]
    fn test_rip_message_serialize_deserialize() {
        let msg = RipMessage {
            command: RipCommand::Response,
            version: 2,
            entries: alloc::vec![RipEntry {
                address_family: 2,
                route_tag: 0,
                ip_address: Ipv4Address::new(10, 0, 0, 0),
                subnet_mask: Ipv4Address::from_u32(prefix_to_mask(8)),
                next_hop: Ipv4Address::ANY,
                metric: 1,
            }],
        };

        let bytes = serialize_message(&msg);
        assert_eq!(bytes.len(), 24); // 4 header + 20 entry

        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(decoded.command, RipCommand::Response);
        assert_eq!(decoded.version, 2);
        assert_eq!(decoded.entries.len(), 1);
        assert_eq!(decoded.entries[0].metric, 1);
        assert_eq!(decoded.entries[0].ip_address, Ipv4Address::new(10, 0, 0, 0));
    }

    #[test]
    fn test_deserialize_invalid() {
        // Too short
        assert!(deserialize_message(&[1, 2]).is_none());
        // Wrong version
        assert!(deserialize_message(&[1, 1, 0, 0]).is_none());
        // Invalid entry length (not multiple of 20)
        assert!(deserialize_message(&[2, 2, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn test_rip_daemon_add_remove() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        let route = RipRoute {
            destination: Ipv4Address::new(10, 0, 0, 0),
            prefix_len: 8,
            next_hop: Ipv4Address::new(192, 168, 1, 254),
            metric: 1,
            route_tag: 0,
            last_update: 0,
            garbage: false,
            source: Ipv4Address::new(192, 168, 1, 254),
        };

        daemon.add_route(route);
        assert_eq!(daemon.route_count(), 1);
        assert!(daemon.get_route(Ipv4Address::new(10, 0, 0, 0), 8).is_some());

        daemon.remove_route(Ipv4Address::new(10, 0, 0, 0), 8);
        assert_eq!(daemon.route_count(), 0);
    }

    #[test]
    fn test_split_horizon_poisoned_reverse() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        let neighbor = Ipv4Address::new(192, 168, 1, 2);

        daemon.add_route(RipRoute {
            destination: Ipv4Address::new(10, 0, 0, 0),
            prefix_len: 8,
            next_hop: neighbor,
            metric: 2,
            route_tag: 0,
            last_update: 0,
            garbage: false,
            source: neighbor,
        });

        // Response to the same neighbor should poison the route
        let responses = daemon.generate_response(neighbor);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].entries[0].metric, RIP_INFINITY);

        // Response to a different neighbor should show real metric
        let other = Ipv4Address::new(192, 168, 1, 3);
        let responses = daemon.generate_response(other);
        assert_eq!(responses[0].entries[0].metric, 2);
    }

    #[test]
    fn test_process_response_new_route() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        let source = Ipv4Address::new(192, 168, 1, 2);

        let msg = RipMessage {
            command: RipCommand::Response,
            version: 2,
            entries: alloc::vec![RipEntry {
                address_family: 2,
                route_tag: 0,
                ip_address: Ipv4Address::new(10, 0, 0, 0),
                subnet_mask: Ipv4Address::from_u32(prefix_to_mask(8)),
                next_hop: Ipv4Address::ANY,
                metric: 1,
            }],
        };

        daemon.process_message(&msg, source);
        assert_eq!(daemon.route_count(), 1);

        let route = daemon.get_route(Ipv4Address::new(10, 0, 0, 0), 8).unwrap();
        assert_eq!(route.metric, 2); // original 1 + 1 hop
        assert_eq!(route.next_hop, source); // next_hop was ANY, so use source
    }

    #[test]
    fn test_process_response_better_metric() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        let source1 = Ipv4Address::new(192, 168, 1, 2);
        let source2 = Ipv4Address::new(192, 168, 1, 3);

        // First route with metric 5
        daemon.add_route(RipRoute {
            destination: Ipv4Address::new(10, 0, 0, 0),
            prefix_len: 8,
            next_hop: source1,
            metric: 5,
            route_tag: 0,
            last_update: 0,
            garbage: false,
            source: source1,
        });

        // Better route with metric 1 from different source
        let msg = RipMessage {
            command: RipCommand::Response,
            version: 2,
            entries: alloc::vec![RipEntry {
                address_family: 2,
                route_tag: 0,
                ip_address: Ipv4Address::new(10, 0, 0, 0),
                subnet_mask: Ipv4Address::from_u32(prefix_to_mask(8)),
                next_hop: Ipv4Address::ANY,
                metric: 1,
            }],
        };

        daemon.process_message(&msg, source2);
        let route = daemon.get_route(Ipv4Address::new(10, 0, 0, 0), 8).unwrap();
        assert_eq!(route.metric, 2); // adopted better metric
        assert_eq!(route.source, source2);
    }

    #[test]
    fn test_route_timeout_and_garbage() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));

        daemon.add_route(RipRoute {
            destination: Ipv4Address::new(10, 0, 0, 0),
            prefix_len: 8,
            next_hop: Ipv4Address::new(192, 168, 1, 2),
            metric: 1,
            route_tag: 0,
            last_update: 0,
            garbage: false,
            source: Ipv4Address::new(192, 168, 1, 2),
        });

        // Before timeout
        daemon.tick(179);
        daemon.age_routes();
        let route = daemon.get_route(Ipv4Address::new(10, 0, 0, 0), 8).unwrap();
        assert_eq!(route.metric, 1);
        assert!(!route.garbage);

        // After timeout
        daemon.tick(1);
        daemon.age_routes();
        let route = daemon.get_route(Ipv4Address::new(10, 0, 0, 0), 8).unwrap();
        assert_eq!(route.metric, RIP_INFINITY);
        assert!(route.garbage);

        // After garbage collection
        daemon.tick(RIP_GARBAGE_COLLECT);
        daemon.age_routes();
        assert_eq!(daemon.route_count(), 0);
    }

    #[test]
    fn test_update_timer() {
        let daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        assert!(!daemon.is_update_due());
    }

    #[test]
    fn test_update_timer_due() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));
        daemon.tick(30);
        assert!(daemon.is_update_due());

        daemon.mark_update_sent();
        assert!(!daemon.is_update_due());
        assert!(!daemon.has_triggered_update());
    }

    #[test]
    fn test_message_splitting() {
        let mut daemon = RipDaemon::new(Ipv4Address::new(192, 168, 1, 1));

        // Add 30 routes (should split into 2 messages: 25 + 5)
        for i in 0..30u8 {
            daemon.add_route(RipRoute {
                destination: Ipv4Address::new(10, i, 0, 0),
                prefix_len: 16,
                next_hop: Ipv4Address::new(192, 168, 1, 2),
                metric: 1,
                route_tag: 0,
                last_update: 0,
                garbage: false,
                source: Ipv4Address::new(192, 168, 1, 3),
            });
        }

        let responses = daemon.generate_response(Ipv4Address::new(192, 168, 1, 4));
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].entries.len(), 25);
        assert_eq!(responses[1].entries.len(), 5);
    }
}
