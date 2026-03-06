//! OSPF routing daemon (RFC 2328)
//!
//! Link-state routing protocol implementation (single area, Area 0):
//! - Hello protocol for neighbor discovery
//! - Database description exchange
//! - Link-state database with LSA flooding
//! - SPF (Dijkstra) shortest path calculation
//! - DR/BDR election

use alloc::{collections::BTreeMap, vec::Vec};

use crate::net::Ipv4Address;

/// OSPF protocol number (IP protocol 89)
pub const OSPF_PROTOCOL: u8 = 89;

/// OSPF version 2
pub const OSPF_VERSION: u8 = 2;

/// Default Hello interval in ticks
pub const HELLO_INTERVAL: u32 = 10;

/// Default dead interval in ticks (4x hello)
pub const DEAD_INTERVAL: u32 = 40;

/// Maximum LSA age in ticks
pub const MAX_LSA_AGE: u16 = 3600;

/// OSPF all-routers multicast (224.0.0.5)
pub const OSPF_ALL_ROUTERS: Ipv4Address = Ipv4Address([224, 0, 0, 5]);

/// OSPF all-DR multicast (224.0.0.6)
pub const OSPF_ALL_DR: Ipv4Address = Ipv4Address([224, 0, 0, 6]);

/// Cost representing infinity (unreachable)
pub const OSPF_INFINITY: u32 = 0xFFFF_FFFF;

/// OSPF packet types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OspfPacketType {
    Hello = 1,
    DatabaseDescription = 2,
    LinkStateRequest = 3,
    LinkStateUpdate = 4,
    LinkStateAck = 5,
}

impl OspfPacketType {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Hello),
            2 => Some(Self::DatabaseDescription),
            3 => Some(Self::LinkStateRequest),
            4 => Some(Self::LinkStateUpdate),
            5 => Some(Self::LinkStateAck),
            _ => None,
        }
    }
}

/// OSPF authentication type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum AuthType {
    None = 0,
    SimplePassword = 1,
    CryptographicMd5 = 2,
}

/// OSPF packet header (24 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OspfHeader {
    /// OSPF version (2)
    pub version: u8,
    /// Packet type
    pub packet_type: OspfPacketType,
    /// Packet length including header
    pub packet_length: u16,
    /// Router ID of the originating router
    pub router_id: u32,
    /// Area ID this packet belongs to
    pub area_id: u32,
    /// Checksum (IP-style)
    pub checksum: u16,
    /// Authentication type
    pub auth_type: AuthType,
}

impl OspfHeader {
    /// Create a new OSPF header
    pub fn new(packet_type: OspfPacketType, router_id: u32, area_id: u32) -> Self {
        Self {
            version: OSPF_VERSION,
            packet_type,
            packet_length: 0, // filled during serialization
            router_id,
            area_id,
            checksum: 0,
            auth_type: AuthType::None,
        }
    }
}

/// OSPF Hello packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloPacket {
    /// Network mask of the interface
    pub network_mask: u32,
    /// Hello interval in ticks
    pub hello_interval: u32,
    /// Router dead interval in ticks
    pub dead_interval: u32,
    /// Router priority for DR election
    pub priority: u8,
    /// Options field
    pub options: u8,
    /// Designated Router ID
    pub designated_router: u32,
    /// Backup Designated Router ID
    pub backup_dr: u32,
    /// List of neighbor router IDs seen on this interface
    pub neighbors: Vec<u32>,
}

impl HelloPacket {
    /// Create a new Hello packet with default intervals
    pub fn new(network_mask: u32, priority: u8) -> Self {
        Self {
            network_mask,
            hello_interval: HELLO_INTERVAL,
            dead_interval: DEAD_INTERVAL,
            priority,
            options: 0x02, // E-bit (external routing capability)
            designated_router: 0,
            backup_dr: 0,
            neighbors: Vec::new(),
        }
    }
}

/// OSPF neighbor states (RFC 2328 Section 10.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum NeighborState {
    /// No information received from neighbor
    #[default]
    Down,
    /// Hello received, but bidirectionality not established
    Init,
    /// Bidirectional communication established
    TwoWay,
    /// Master/slave negotiation
    ExStart,
    /// Database description exchange
    Exchange,
    /// Link state requests being sent
    Loading,
    /// Fully adjacent
    Full,
}

impl NeighborState {
    /// Transition on receiving a Hello packet
    pub fn on_hello_received(self) -> Self {
        match self {
            Self::Down => Self::Init,
            other => other,
        }
    }

    /// Transition when bidirectionality is confirmed (our router_id seen in
    /// neighbor's Hello)
    pub fn on_two_way_received(self) -> Self {
        match self {
            Self::Init => Self::TwoWay,
            other => other,
        }
    }

    /// Transition to begin adjacency formation (DR/BDR or point-to-point)
    pub fn on_adjacency_ok(self) -> Self {
        match self {
            Self::TwoWay => Self::ExStart,
            other => other,
        }
    }

    /// Transition when negotiation completes
    pub fn on_negotiation_done(self) -> Self {
        match self {
            Self::ExStart => Self::Exchange,
            other => other,
        }
    }

    /// Transition when exchange completes
    pub fn on_exchange_done(self) -> Self {
        match self {
            Self::Exchange => Self::Loading,
            other => other,
        }
    }

    /// Transition when loading completes
    pub fn on_loading_done(self) -> Self {
        match self {
            Self::Loading => Self::Full,
            other => other,
        }
    }

    /// Transition on inactivity timeout
    pub fn on_kill(self) -> Self {
        Self::Down
    }

    /// Whether this state represents an established adjacency
    pub fn is_adjacent(&self) -> bool {
        matches!(self, Self::Full)
    }
}

/// OSPF neighbor entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OspfNeighbor {
    /// Neighbor's router ID
    pub router_id: u32,
    /// Current neighbor state
    pub state: NeighborState,
    /// Neighbor's IP address
    pub address: Ipv4Address,
    /// Neighbor's priority for DR election
    pub priority: u8,
    /// Tick when last Hello was received
    pub last_hello: u64,
    /// Neighbor's Designated Router value
    pub designated_router: u32,
    /// Neighbor's Backup DR value
    pub backup_dr: u32,
}

/// LSA types
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LsaType {
    /// Router LSA (type 1)
    RouterLsa = 1,
    /// Network LSA (type 2)
    NetworkLsa = 2,
}

impl LsaType {
    #[allow(dead_code)]
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::RouterLsa),
            2 => Some(Self::NetworkLsa),
            _ => None,
        }
    }
}

/// LSA header (20 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LsaHeader {
    /// Age of the LSA in ticks
    pub ls_age: u16,
    /// Options field
    pub options: u8,
    /// LSA type
    pub ls_type: LsaType,
    /// Link State ID
    pub link_state_id: u32,
    /// Advertising router ID
    pub advertising_router: u32,
    /// Sequence number (for versioning)
    pub seq_number: u32,
    /// Checksum
    pub checksum: u16,
    /// Total length of LSA including header
    pub length: u16,
}

impl LsaHeader {
    /// Create a new LSA header
    pub fn new(ls_type: LsaType, link_state_id: u32, advertising_router: u32) -> Self {
        Self {
            ls_age: 0,
            options: 0x02,
            ls_type,
            link_state_id,
            advertising_router,
            seq_number: 1,
            checksum: 0,
            length: 0,
        }
    }

    /// Check if this LSA is newer than another
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self.seq_number > other.seq_number
    }
}

/// Router LSA link types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RouterLinkType {
    /// Point-to-point connection to another router
    PointToPoint = 1,
    /// Connection to a transit network
    Transit = 2,
    /// Connection to a stub network
    Stub = 3,
    /// Virtual link
    Virtual = 4,
}

/// A single link in a Router LSA
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterLink {
    /// Link type
    pub link_type: RouterLinkType,
    /// Link ID (interpretation depends on link_type)
    pub link_id: u32,
    /// Link data (interpretation depends on link_type)
    pub link_data: u32,
    /// Cost metric
    pub metric: u32,
}

/// Router LSA (type 1)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterLsa {
    /// LSA header
    pub header: LsaHeader,
    /// Router flags (V=virtual link, E=ASBR, B=ABR)
    pub flags: u8,
    /// List of router links
    pub links: Vec<RouterLink>,
}

/// Network LSA (type 2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkLsa {
    /// LSA header
    pub header: LsaHeader,
    /// Network mask
    pub network_mask: u32,
    /// List of attached router IDs
    pub attached_routers: Vec<u32>,
}

/// Stored LSA in the database
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lsa {
    Router(RouterLsa),
    Network(NetworkLsa),
}

impl Lsa {
    /// Get the LSA header
    pub fn header(&self) -> &LsaHeader {
        match self {
            Lsa::Router(r) => &r.header,
            Lsa::Network(n) => &n.header,
        }
    }

    /// Get the LSA header mutably
    pub fn header_mut(&mut self) -> &mut LsaHeader {
        match self {
            Lsa::Router(r) => &mut r.header,
            Lsa::Network(n) => &mut n.header,
        }
    }
}

/// Key for LSA database lookup: (type, link_state_id, advertising_router)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LsaKey {
    pub ls_type: u8,
    pub link_state_id: u32,
    pub advertising_router: u32,
}

/// Link-State Database
#[derive(Debug, Default)]
pub struct LsDatabase {
    /// LSAs indexed by (type, id, router)
    lsas: BTreeMap<LsaKey, Lsa>,
}

impl LsDatabase {
    /// Create a new empty database
    pub fn new() -> Self {
        Self {
            lsas: BTreeMap::new(),
        }
    }

    /// Insert or update an LSA. Returns true if the LSA was newer and was
    /// installed.
    pub fn install(&mut self, lsa: Lsa) -> bool {
        let key = LsaKey {
            ls_type: match lsa.header().ls_type {
                LsaType::RouterLsa => 1,
                LsaType::NetworkLsa => 2,
            },
            link_state_id: lsa.header().link_state_id,
            advertising_router: lsa.header().advertising_router,
        };

        if let Some(existing) = self.lsas.get(&key) {
            if !lsa.header().is_newer_than(existing.header()) {
                return false;
            }
        }

        self.lsas.insert(key, lsa);
        true
    }

    /// Lookup an LSA
    pub fn lookup(&self, ls_type: LsaType, link_state_id: u32, router: u32) -> Option<&Lsa> {
        let key = LsaKey {
            ls_type: ls_type as u8,
            link_state_id,
            advertising_router: router,
        };
        self.lsas.get(&key)
    }

    /// Get all LSAs
    pub fn all_lsas(&self) -> Vec<&Lsa> {
        self.lsas.values().collect()
    }

    /// Get all Router LSAs
    pub fn router_lsas(&self) -> Vec<&RouterLsa> {
        self.lsas
            .values()
            .filter_map(|lsa| match lsa {
                Lsa::Router(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// Number of LSAs in the database
    pub fn len(&self) -> usize {
        self.lsas.len()
    }

    /// Whether the database is empty
    pub fn is_empty(&self) -> bool {
        self.lsas.is_empty()
    }

    /// Age all LSAs by the given number of ticks. Remove LSAs that exceed
    /// MAX_LSA_AGE.
    pub fn age_lsas(&mut self, ticks: u16) {
        let mut to_remove = Vec::new();

        for (key, lsa) in &mut self.lsas {
            let header = lsa.header_mut();
            header.ls_age = header.ls_age.saturating_add(ticks);
            if header.ls_age >= MAX_LSA_AGE {
                to_remove.push(*key);
            }
        }

        for key in &to_remove {
            self.lsas.remove(key);
        }
    }
}

/// Node in the SPF calculation
#[derive(Debug, Clone, PartialEq, Eq)]
struct SpfNode {
    /// Router ID
    router_id: u32,
    /// Accumulated cost from root
    cost: u32,
    /// Next hop router for this destination (0 = directly connected)
    next_hop: u32,
}

impl SpfNode {
    fn new(router_id: u32, cost: u32, next_hop: u32) -> Self {
        Self {
            router_id,
            cost,
            next_hop,
        }
    }
}

/// SPF result entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpfEntry {
    /// Destination router ID
    pub router_id: u32,
    /// Total cost
    pub cost: u32,
    /// Next hop router ID (0 = directly connected)
    pub next_hop: u32,
}

/// OSPF Router
pub struct OspfRouter {
    /// This router's ID
    pub router_id: u32,
    /// Area ID (0 for backbone)
    pub area_id: u32,
    /// Neighbor table
    neighbors: BTreeMap<u32, OspfNeighbor>,
    /// Link-state database
    lsdb: LsDatabase,
    /// Current tick
    current_tick: u64,
    /// Router priority for DR election
    priority: u8,
    /// Interface network mask
    network_mask: u32,
}

impl OspfRouter {
    /// Create a new OSPF router
    pub fn new(router_id: u32, area_id: u32, priority: u8, network_mask: u32) -> Self {
        Self {
            router_id,
            area_id,
            neighbors: BTreeMap::new(),
            lsdb: LsDatabase::new(),
            current_tick: 0,
            priority,
            network_mask,
        }
    }

    /// Advance the tick counter
    pub fn tick(&mut self, ticks: u64) {
        self.current_tick += ticks;
    }

    /// Get the link-state database
    pub fn lsdb(&self) -> &LsDatabase {
        &self.lsdb
    }

    /// Get the link-state database mutably
    pub fn lsdb_mut(&mut self) -> &mut LsDatabase {
        &mut self.lsdb
    }

    /// Get number of neighbors
    pub fn neighbor_count(&self) -> usize {
        self.neighbors.len()
    }

    /// Get a neighbor by router ID
    pub fn get_neighbor(&self, router_id: u32) -> Option<&OspfNeighbor> {
        self.neighbors.get(&router_id)
    }

    /// Get all neighbors
    pub fn neighbors(&self) -> Vec<&OspfNeighbor> {
        self.neighbors.values().collect()
    }

    /// Process an incoming Hello packet from a neighbor
    pub fn process_hello(&mut self, header: &OspfHeader, hello: &HelloPacket, source: Ipv4Address) {
        let neighbor_id = header.router_id;

        // Check for compatible parameters
        if hello.hello_interval != HELLO_INTERVAL || hello.dead_interval != DEAD_INTERVAL {
            return; // Parameter mismatch, ignore
        }

        if let Some(neighbor) = self.neighbors.get_mut(&neighbor_id) {
            // Existing neighbor: update and transition
            neighbor.last_hello = self.current_tick;
            neighbor.designated_router = hello.designated_router;
            neighbor.backup_dr = hello.backup_dr;

            // Check if our router_id appears in the neighbor's list (bidirectional)
            if hello.neighbors.contains(&self.router_id) {
                neighbor.state = neighbor.state.on_two_way_received();
            } else {
                neighbor.state = neighbor.state.on_hello_received();
            }
        } else {
            // New neighbor discovered
            let mut state = NeighborState::Down;
            state = state.on_hello_received();

            if hello.neighbors.contains(&self.router_id) {
                state = state.on_two_way_received();
            }

            let neighbor = OspfNeighbor {
                router_id: neighbor_id,
                state,
                address: source,
                priority: hello.priority,
                last_hello: self.current_tick,
                designated_router: hello.designated_router,
                backup_dr: hello.backup_dr,
            };
            self.neighbors.insert(neighbor_id, neighbor);
        }
    }

    /// Process a Database Description packet (simplified: just advance state)
    pub fn process_dbd(&mut self, header: &OspfHeader, _lsa_headers: &[LsaHeader]) {
        let neighbor_id = header.router_id;

        if let Some(neighbor) = self.neighbors.get_mut(&neighbor_id) {
            match neighbor.state {
                NeighborState::ExStart => {
                    neighbor.state = neighbor.state.on_negotiation_done();
                }
                NeighborState::Exchange => {
                    neighbor.state = neighbor.state.on_exchange_done();
                }
                NeighborState::Loading => {
                    neighbor.state = neighbor.state.on_loading_done();
                }
                _ => {}
            }
        }
    }

    /// Install an LSA into the database and return whether SPF recalculation is
    /// needed
    pub fn install_lsa(&mut self, lsa: Lsa) -> bool {
        self.lsdb.install(lsa)
    }

    /// Check for dead neighbors and remove them
    pub fn check_dead_neighbors(&mut self) {
        let dead_interval = u64::from(DEAD_INTERVAL);
        let current = self.current_tick;
        let mut dead_ids = Vec::new();

        for (id, neighbor) in &self.neighbors {
            if current.saturating_sub(neighbor.last_hello) >= dead_interval {
                dead_ids.push(*id);
            }
        }

        for id in &dead_ids {
            self.neighbors.remove(id);
        }
    }

    /// Generate a Hello packet for this interface
    pub fn generate_hello(&self) -> HelloPacket {
        let mut hello = HelloPacket::new(self.network_mask, self.priority);

        // Elect DR/BDR from current neighbor knowledge
        let (dr, bdr) = self.elect_dr_bdr();
        hello.designated_router = dr;
        hello.backup_dr = bdr;

        // Include all known neighbor router IDs
        for neighbor_id in self.neighbors.keys() {
            hello.neighbors.push(*neighbor_id);
        }

        hello
    }

    /// Simple DR/BDR election based on priority and router ID
    fn elect_dr_bdr(&self) -> (u32, u32) {
        // Collect candidates: (priority, router_id) for all neighbors + self
        // Only routers with priority > 0 are eligible
        let mut candidates: Vec<(u8, u32)> = Vec::new();

        if self.priority > 0 {
            candidates.push((self.priority, self.router_id));
        }

        for neighbor in self.neighbors.values() {
            if neighbor.priority > 0
                && (neighbor.state == NeighborState::TwoWay
                    || neighbor.state == NeighborState::Full)
            {
                candidates.push((neighbor.priority, neighbor.router_id));
            }
        }

        if candidates.is_empty() {
            return (0, 0);
        }

        // Sort by priority (descending), then router_id (descending) as tiebreaker
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

        let dr = candidates[0].1;
        let bdr = if candidates.len() > 1 {
            candidates[1].1
        } else {
            0
        };

        (dr, bdr)
    }

    /// Run SPF (Dijkstra) algorithm on the LSDB and return routing table
    /// entries
    pub fn run_spf(&self) -> Vec<SpfEntry> {
        let mut result = Vec::new();
        let mut visited: BTreeMap<u32, SpfEntry> = BTreeMap::new();

        // Priority queue: Vec-based min-heap (cost, node)
        // We use a simple Vec and find minimum each iteration for correctness
        let mut candidates: Vec<SpfNode> = Vec::new();

        // Start with self at cost 0
        candidates.push(SpfNode::new(self.router_id, 0, 0));

        while !candidates.is_empty() {
            // Find the candidate with minimum cost
            let min_idx = find_min_cost(&candidates);
            let current = candidates.swap_remove(min_idx);

            // Skip if already visited
            if visited.contains_key(&current.router_id) {
                continue;
            }

            // Mark visited
            let entry = SpfEntry {
                router_id: current.router_id,
                cost: current.cost,
                next_hop: current.next_hop,
            };
            visited.insert(current.router_id, entry.clone());

            if current.router_id != self.router_id {
                result.push(entry);
            }

            // Find the Router LSA for this node and explore its links
            let router_lsas = self.lsdb.router_lsas();
            for rlsa in &router_lsas {
                if rlsa.header.advertising_router != current.router_id {
                    continue;
                }

                for link in &rlsa.links {
                    match link.link_type {
                        RouterLinkType::PointToPoint | RouterLinkType::Transit => {
                            let neighbor_id = link.link_id;
                            if visited.contains_key(&neighbor_id) {
                                continue;
                            }

                            let new_cost = current.cost.saturating_add(link.metric);

                            // Determine next_hop: if current is root, next_hop is the neighbor
                            let next_hop = if current.router_id == self.router_id {
                                neighbor_id
                            } else {
                                current.next_hop
                            };

                            candidates.push(SpfNode::new(neighbor_id, new_cost, next_hop));
                        }
                        RouterLinkType::Stub | RouterLinkType::Virtual => {
                            // Stub networks don't participate in SPF graph
                            // traversal
                            // Virtual links not supported in single-area
                        }
                    }
                }
            }
        }

        result
    }
}

/// Find index of minimum cost node in the candidate list
fn find_min_cost(candidates: &[SpfNode]) -> usize {
    let mut min_idx = 0;
    let mut min_cost = candidates[0].cost;

    for (i, node) in candidates.iter().enumerate().skip(1) {
        if node.cost < min_cost {
            min_cost = node.cost;
            min_idx = i;
        }
    }

    min_idx
}

/// Serialize an OSPF header to bytes
pub fn serialize_header(header: &OspfHeader) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.push(header.version);
    buf.push(header.packet_type as u8);
    buf.extend_from_slice(&header.packet_length.to_be_bytes());
    buf.extend_from_slice(&header.router_id.to_be_bytes());
    buf.extend_from_slice(&header.area_id.to_be_bytes());
    buf.extend_from_slice(&header.checksum.to_be_bytes());
    buf.extend_from_slice(&(header.auth_type as u16).to_be_bytes());
    // 8 bytes of auth data (zeroed for no auth)
    buf.extend_from_slice(&[0u8; 8]);
    buf
}

/// Deserialize an OSPF header from bytes
pub fn deserialize_header(data: &[u8]) -> Option<OspfHeader> {
    if data.len() < 24 {
        return None;
    }

    let version = data[0];
    if version != OSPF_VERSION {
        return None;
    }

    let packet_type = OspfPacketType::from_u8(data[1])?;
    let packet_length = u16::from_be_bytes([data[2], data[3]]);
    let router_id = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let area_id = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let checksum = u16::from_be_bytes([data[12], data[13]]);
    let auth_type_val = u16::from_be_bytes([data[14], data[15]]);

    let auth_type = match auth_type_val {
        0 => AuthType::None,
        1 => AuthType::SimplePassword,
        2 => AuthType::CryptographicMd5,
        _ => return None,
    };

    Some(OspfHeader {
        version,
        packet_type,
        packet_length,
        router_id,
        area_id,
        checksum,
        auth_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neighbor_state_transitions() {
        let state = NeighborState::Down;
        assert_eq!(state.on_hello_received(), NeighborState::Init);
        assert_eq!(
            state.on_hello_received().on_two_way_received(),
            NeighborState::TwoWay
        );
        assert_eq!(
            NeighborState::TwoWay.on_adjacency_ok(),
            NeighborState::ExStart
        );
        assert_eq!(
            NeighborState::ExStart.on_negotiation_done(),
            NeighborState::Exchange
        );
        assert_eq!(
            NeighborState::Exchange.on_exchange_done(),
            NeighborState::Loading
        );
        assert_eq!(
            NeighborState::Loading.on_loading_done(),
            NeighborState::Full
        );
    }

    #[test]
    fn test_neighbor_state_kill() {
        assert_eq!(NeighborState::Full.on_kill(), NeighborState::Down);
        assert_eq!(NeighborState::Exchange.on_kill(), NeighborState::Down);
    }

    #[test]
    fn test_neighbor_state_is_adjacent() {
        assert!(!NeighborState::Down.is_adjacent());
        assert!(!NeighborState::Init.is_adjacent());
        assert!(!NeighborState::TwoWay.is_adjacent());
        assert!(NeighborState::Full.is_adjacent());
    }

    #[test]
    fn test_lsa_header_newer() {
        let h1 = LsaHeader {
            ls_age: 0,
            options: 0,
            ls_type: LsaType::RouterLsa,
            link_state_id: 1,
            advertising_router: 1,
            seq_number: 5,
            checksum: 0,
            length: 0,
        };

        let h2 = LsaHeader {
            seq_number: 10,
            ..h1
        };

        assert!(h2.is_newer_than(&h1));
        assert!(!h1.is_newer_than(&h2));
        assert!(!h1.is_newer_than(&h1));
    }

    #[test]
    fn test_lsdb_install_and_lookup() {
        let mut db = LsDatabase::new();

        let lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 1, 1),
            flags: 0,
            links: Vec::new(),
        });

        assert!(db.install(lsa));
        assert_eq!(db.len(), 1);
        assert!(db.lookup(LsaType::RouterLsa, 1, 1).is_some());
        assert!(db.lookup(LsaType::RouterLsa, 2, 1).is_none());
    }

    #[test]
    fn test_lsdb_reject_older() {
        let mut db = LsDatabase::new();

        let mut header = LsaHeader::new(LsaType::RouterLsa, 1, 1);
        header.seq_number = 10;

        let lsa = Lsa::Router(RouterLsa {
            header,
            flags: 0,
            links: Vec::new(),
        });
        assert!(db.install(lsa));

        // Try to install older version
        let mut older_header = LsaHeader::new(LsaType::RouterLsa, 1, 1);
        older_header.seq_number = 5;

        let older_lsa = Lsa::Router(RouterLsa {
            header: older_header,
            flags: 0,
            links: Vec::new(),
        });
        assert!(!db.install(older_lsa));
        assert_eq!(db.len(), 1);
    }

    #[test]
    fn test_lsdb_age() {
        let mut db = LsDatabase::new();

        let lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 1, 1),
            flags: 0,
            links: Vec::new(),
        });
        db.install(lsa);

        db.age_lsas(MAX_LSA_AGE);
        assert!(db.is_empty());
    }

    #[test]
    fn test_ospf_header_serialize_deserialize() {
        let header = OspfHeader::new(OspfPacketType::Hello, 0x01020304, 0);
        let bytes = serialize_header(&header);
        assert_eq!(bytes.len(), 24);

        let decoded = deserialize_header(&bytes).unwrap();
        assert_eq!(decoded.version, OSPF_VERSION);
        assert_eq!(decoded.packet_type, OspfPacketType::Hello);
        assert_eq!(decoded.router_id, 0x01020304);
        assert_eq!(decoded.area_id, 0);
        assert_eq!(decoded.auth_type, AuthType::None);
    }

    #[test]
    fn test_hello_generation() {
        let router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);
        let hello = router.generate_hello();

        assert_eq!(hello.network_mask, 0xFFFFFF00);
        assert_eq!(hello.hello_interval, HELLO_INTERVAL);
        assert_eq!(hello.dead_interval, DEAD_INTERVAL);
        assert!(hello.neighbors.is_empty());
    }

    #[test]
    fn test_process_hello_new_neighbor() {
        let mut router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        let header = OspfHeader::new(OspfPacketType::Hello, 2, 0);
        let mut hello = HelloPacket::new(0xFFFFFF00, 1);
        hello.neighbors.push(1); // Neighbor sees us

        router.process_hello(&header, &hello, Ipv4Address::new(10, 0, 0, 2));
        assert_eq!(router.neighbor_count(), 1);

        let neighbor = router.get_neighbor(2).unwrap();
        assert_eq!(neighbor.state, NeighborState::TwoWay);
    }

    #[test]
    fn test_process_hello_one_way() {
        let mut router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        let header = OspfHeader::new(OspfPacketType::Hello, 2, 0);
        let hello = HelloPacket::new(0xFFFFFF00, 1);
        // Neighbor does NOT list our router_id

        router.process_hello(&header, &hello, Ipv4Address::new(10, 0, 0, 2));
        let neighbor = router.get_neighbor(2).unwrap();
        assert_eq!(neighbor.state, NeighborState::Init);
    }

    #[test]
    fn test_dead_neighbor_removal() {
        let mut router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        let header = OspfHeader::new(OspfPacketType::Hello, 2, 0);
        let hello = HelloPacket::new(0xFFFFFF00, 1);
        router.process_hello(&header, &hello, Ipv4Address::new(10, 0, 0, 2));
        assert_eq!(router.neighbor_count(), 1);

        router.tick(u64::from(DEAD_INTERVAL));
        router.check_dead_neighbors();
        assert_eq!(router.neighbor_count(), 0);
    }

    #[test]
    fn test_dr_election() {
        let mut router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        // Add a neighbor with higher priority
        let header = OspfHeader::new(OspfPacketType::Hello, 2, 0);
        let mut hello = HelloPacket::new(0xFFFFFF00, 2); // priority 2
        hello.neighbors.push(1);
        router.process_hello(&header, &hello, Ipv4Address::new(10, 0, 0, 2));

        let generated_hello = router.generate_hello();
        // Neighbor 2 has higher priority, should be DR
        assert_eq!(generated_hello.designated_router, 2);
        assert_eq!(generated_hello.backup_dr, 1);
    }

    #[test]
    fn test_spf_simple_topology() {
        // Topology: R1 --10-- R2 --5-- R3
        let router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        // R1's Router LSA: link to R2 with cost 10
        let r1_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 1, 1),
            flags: 0,
            links: alloc::vec![RouterLink {
                link_type: RouterLinkType::PointToPoint,
                link_id: 2,
                link_data: 0x0A000001,
                metric: 10,
            }],
        });

        // R2's Router LSA: links to R1 (cost 10) and R3 (cost 5)
        let r2_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 2, 2),
            flags: 0,
            links: alloc::vec![
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 1,
                    link_data: 0x0A000002,
                    metric: 10,
                },
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 3,
                    link_data: 0x0A000002,
                    metric: 5,
                },
            ],
        });

        // R3's Router LSA: link to R2 with cost 5
        let r3_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 3, 3),
            flags: 0,
            links: alloc::vec![RouterLink {
                link_type: RouterLinkType::PointToPoint,
                link_id: 2,
                link_data: 0x0A000003,
                metric: 5,
            }],
        });

        // Build router with populated LSDB
        let mut test_router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);
        test_router.install_lsa(r1_lsa);
        test_router.install_lsa(r2_lsa);
        test_router.install_lsa(r3_lsa);

        let spf_result = test_router.run_spf();

        // Should have entries for R2 and R3
        assert_eq!(spf_result.len(), 2);

        let r2_entry = spf_result.iter().find(|e| e.router_id == 2).unwrap();
        assert_eq!(r2_entry.cost, 10);
        assert_eq!(r2_entry.next_hop, 2); // Direct neighbor

        let r3_entry = spf_result.iter().find(|e| e.router_id == 3).unwrap();
        assert_eq!(r3_entry.cost, 15); // 10 + 5
        assert_eq!(r3_entry.next_hop, 2); // Via R2
    }

    #[test]
    fn test_spf_shortest_path_preferred() {
        // Topology: R1 --1-- R2 --1-- R3
        //           R1 --10---------  R3
        // Shortest path to R3 should be via R2 (cost 2), not direct (cost 10)
        let mut router = OspfRouter::new(1, 0, 1, 0xFFFFFF00);

        let r1_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 1, 1),
            flags: 0,
            links: alloc::vec![
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 2,
                    link_data: 0,
                    metric: 1,
                },
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 3,
                    link_data: 0,
                    metric: 10,
                },
            ],
        });

        let r2_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 2, 2),
            flags: 0,
            links: alloc::vec![
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 1,
                    link_data: 0,
                    metric: 1,
                },
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 3,
                    link_data: 0,
                    metric: 1,
                },
            ],
        });

        let r3_lsa = Lsa::Router(RouterLsa {
            header: LsaHeader::new(LsaType::RouterLsa, 3, 3),
            flags: 0,
            links: alloc::vec![
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 2,
                    link_data: 0,
                    metric: 1,
                },
                RouterLink {
                    link_type: RouterLinkType::PointToPoint,
                    link_id: 1,
                    link_data: 0,
                    metric: 10,
                },
            ],
        });

        router.install_lsa(r1_lsa);
        router.install_lsa(r2_lsa);
        router.install_lsa(r3_lsa);

        let spf_result = router.run_spf();
        let r3_entry = spf_result.iter().find(|e| e.router_id == 3).unwrap();
        assert_eq!(r3_entry.cost, 2); // via R2: 1 + 1
        assert_eq!(r3_entry.next_hop, 2);
    }
}
