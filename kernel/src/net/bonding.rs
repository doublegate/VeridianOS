//! NIC Bonding / Link Aggregation
//!
//! Provides bond interfaces that aggregate multiple physical NICs for
//! redundancy (active-backup) or load distribution (round-robin).
//!
//! Supports ARP monitoring for link health detection and automatic failover.

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use crate::{error::KernelError, sync::once_lock::GlobalState};

// ============================================================================
// Bond Mode
// ============================================================================

/// Bond operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BondMode {
    /// Mode 0: distribute packets across slaves in round-robin order
    RoundRobin,
    /// Mode 1: only one slave active at a time, failover on link down
    ActiveBackup,
}

// ============================================================================
// Error Types
// ============================================================================

/// Errors returned by bonding operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BondError {
    /// Bond with the given name already exists
    BondAlreadyExists,
    /// Bond with the given name was not found
    BondNotFound,
    /// Slave with the given name already exists in the bond
    SlaveAlreadyExists,
    /// Slave with the given name was not found in the bond
    SlaveNotFound,
    /// No slaves available for transmission
    NoSlavesAvailable,
    /// Bond manager not initialized
    NotInitialized,
}

impl From<BondError> for KernelError {
    fn from(_e: BondError) -> Self {
        KernelError::InvalidArgument {
            name: "bonding",
            value: "operation failed",
        }
    }
}

// ============================================================================
// Bond Slave
// ============================================================================

/// A slave (member) interface within a bond
#[derive(Debug, Clone)]
pub struct BondSlave {
    /// Interface name (e.g., "eth0")
    pub name: String,
    /// Hardware MAC address
    pub mac_address: [u8; 6],
    /// Whether the physical link is up
    pub link_up: bool,
    /// Whether this slave is the active transmitter (ActiveBackup mode)
    pub is_active: bool,
    /// Transmitted packet count
    pub tx_packets: u64,
    /// Received packet count
    pub rx_packets: u64,
    /// Last successful ARP response timestamp (ms)
    pub last_arp_reply_ms: u64,
}

impl BondSlave {
    /// Create a new slave with link initially up
    pub fn new(name: &str, mac_address: [u8; 6]) -> Self {
        Self {
            name: String::from(name),
            mac_address,
            link_up: true,
            is_active: false,
            tx_packets: 0,
            rx_packets: 0,
            last_arp_reply_ms: 0,
        }
    }
}

// ============================================================================
// ARP Monitor
// ============================================================================

/// ARP-based link health monitor
#[derive(Debug, Clone)]
pub struct ArpMonitor {
    /// Monitoring interval in milliseconds
    pub interval_ms: u64,
    /// ARP probe target IP addresses
    pub targets: Vec<[u8; 4]>,
    /// Timestamp of the last ARP probe (ms)
    pub last_check: u64,
}

impl ArpMonitor {
    /// Create a new ARP monitor with the given interval
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            targets: Vec::new(),
            last_check: 0,
        }
    }

    /// Add an ARP probe target IP address
    pub fn add_target(&mut self, ip: [u8; 4]) {
        if !self.targets.contains(&ip) {
            self.targets.push(ip);
        }
    }

    /// Check whether the monitoring interval has elapsed
    ///
    /// Returns `true` if it is time to send ARP probes, and updates
    /// the internal timestamp.
    pub fn tick(&mut self, now: u64) -> bool {
        if self.interval_ms == 0 {
            return false;
        }
        if now.saturating_sub(self.last_check) >= self.interval_ms {
            self.last_check = now;
            true
        } else {
            false
        }
    }
}

// ============================================================================
// Bond Interface
// ============================================================================

/// A bond (link aggregation) interface
#[derive(Debug, Clone)]
pub struct BondInterface {
    /// Bond interface name (e.g., "bond0")
    pub name: String,
    /// Operating mode
    pub mode: BondMode,
    /// Member interfaces
    pub slaves: Vec<BondSlave>,
    /// Bond-level MAC address (set from first slave added)
    pub mac_address: [u8; 6],
    /// Index of the currently active slave (ActiveBackup mode)
    pub active_slave_index: usize,
    /// Round-robin counter for TX distribution
    pub rr_counter: usize,
    /// ARP health monitor
    pub arp_monitor: ArpMonitor,
}

impl BondInterface {
    /// Create a new bond interface
    pub fn new(name: &str, mode: BondMode) -> Self {
        Self {
            name: String::from(name),
            mode,
            slaves: Vec::new(),
            mac_address: [0u8; 6],
            active_slave_index: 0,
            rr_counter: 0,
            arp_monitor: ArpMonitor::new(0),
        }
    }

    /// Add a slave interface to this bond
    pub fn add_slave(&mut self, slave_name: &str, mac: [u8; 6]) -> Result<(), BondError> {
        // Check for duplicate
        if self.slaves.iter().any(|s| s.name == slave_name) {
            return Err(BondError::SlaveAlreadyExists);
        }

        let mut slave = BondSlave::new(slave_name, mac);

        // First slave provides the bond MAC address
        if self.slaves.is_empty() {
            self.mac_address = mac;
        }

        // In ActiveBackup mode, activate first link-up slave if none active
        if self.mode == BondMode::ActiveBackup && !self.has_active_slave() && slave.link_up {
            slave.is_active = true;
            self.active_slave_index = self.slaves.len();
        }

        // In RoundRobin mode, all slaves are effectively active
        if self.mode == BondMode::RoundRobin {
            slave.is_active = true;
        }

        self.slaves.push(slave);
        Ok(())
    }

    /// Remove a slave interface from this bond
    pub fn remove_slave(&mut self, slave_name: &str) -> Result<(), BondError> {
        let idx = self
            .slaves
            .iter()
            .position(|s| s.name == slave_name)
            .ok_or(BondError::SlaveNotFound)?;

        let was_active = self.slaves[idx].is_active;
        self.slaves.remove(idx);

        // Fix active_slave_index after removal
        if self.active_slave_index >= self.slaves.len() && !self.slaves.is_empty() {
            self.active_slave_index = 0;
        }

        // If we removed the active slave in ActiveBackup, promote another
        if was_active && self.mode == BondMode::ActiveBackup {
            self.promote_next_slave();
        }

        Ok(())
    }

    /// Select the slave index to use for the next TX packet
    pub fn select_tx_slave(&mut self) -> Option<usize> {
        if self.slaves.is_empty() {
            return None;
        }

        match self.mode {
            BondMode::ActiveBackup => {
                // Use the active slave if it is link-up
                if self.active_slave_index < self.slaves.len()
                    && self.slaves[self.active_slave_index].link_up
                {
                    Some(self.active_slave_index)
                } else {
                    None
                }
            }
            BondMode::RoundRobin => {
                // Scan up to slaves.len() times to find a link-up slave
                let count = self.slaves.len();
                for _ in 0..count {
                    let idx = self.rr_counter % count;
                    self.rr_counter = self.rr_counter.wrapping_add(1);
                    if self.slaves[idx].link_up {
                        return Some(idx);
                    }
                }
                None
            }
        }
    }

    /// Handle a link state change on a slave interface
    pub fn handle_link_change(&mut self, slave_name: &str, link_up: bool) {
        let Some(idx) = self.slaves.iter().position(|s| s.name == slave_name) else {
            return;
        };

        self.slaves[idx].link_up = link_up;

        match self.mode {
            BondMode::ActiveBackup => {
                if !link_up && self.slaves[idx].is_active {
                    // Active slave went down -- failover
                    self.slaves[idx].is_active = false;
                    self.promote_next_slave();
                } else if link_up && !self.has_active_slave() {
                    // No active slave, promote this one
                    self.slaves[idx].is_active = true;
                    self.active_slave_index = idx;
                }
            }
            BondMode::RoundRobin => {
                // RoundRobin just skips downed slaves during selection
                self.slaves[idx].is_active = link_up;
            }
        }
    }

    /// Returns `true` if any slave is currently marked active
    fn has_active_slave(&self) -> bool {
        self.slaves.iter().any(|s| s.is_active)
    }

    /// Promote the next link-up slave to active (ActiveBackup mode)
    fn promote_next_slave(&mut self) {
        for (i, slave) in self.slaves.iter_mut().enumerate() {
            if slave.link_up {
                slave.is_active = true;
                self.active_slave_index = i;
                return;
            }
        }
        // No link-up slave found -- bond is fully down
    }

    /// Return the number of slaves with link up
    pub fn link_up_count(&self) -> usize {
        self.slaves.iter().filter(|s| s.link_up).count()
    }
}

// ============================================================================
// Bond Manager (global state)
// ============================================================================

/// Manages all bond interfaces on the system
#[derive(Default)]
pub struct BondManager {
    /// Map of bond name -> BondInterface
    pub bonds: BTreeMap<String, BondInterface>,
}

impl BondManager {
    /// Create a new empty bond manager
    pub fn new() -> Self {
        Self::default()
    }
}

/// Global bond manager state
static BOND_MANAGER: GlobalState<RwLock<BondManager>> = GlobalState::new();

/// Initialize the bond manager
pub fn init() -> Result<(), KernelError> {
    BOND_MANAGER
        .init(RwLock::new(BondManager::new()))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "bond_manager",
            id: 0,
        })?;
    Ok(())
}

/// Create a new bond interface
pub fn create_bond(name: &str, mode: BondMode) -> Result<(), BondError> {
    BOND_MANAGER
        .with(|lock| {
            let mut mgr = lock.write();
            if mgr.bonds.contains_key(name) {
                return Err(BondError::BondAlreadyExists);
            }
            mgr.bonds
                .insert(String::from(name), BondInterface::new(name, mode));
            Ok(())
        })
        .unwrap_or(Err(BondError::NotInitialized))
}

/// Add a slave interface to an existing bond
pub fn add_slave(bond_name: &str, slave_name: &str, mac: [u8; 6]) -> Result<(), BondError> {
    BOND_MANAGER
        .with(|lock| {
            let mut mgr = lock.write();
            let bond = mgr
                .bonds
                .get_mut(bond_name)
                .ok_or(BondError::BondNotFound)?;
            bond.add_slave(slave_name, mac)
        })
        .unwrap_or(Err(BondError::NotInitialized))
}

/// Remove a slave interface from a bond
pub fn remove_slave(bond_name: &str, slave_name: &str) -> Result<(), BondError> {
    BOND_MANAGER
        .with(|lock| {
            let mut mgr = lock.write();
            let bond = mgr
                .bonds
                .get_mut(bond_name)
                .ok_or(BondError::BondNotFound)?;
            bond.remove_slave(slave_name)
        })
        .unwrap_or(Err(BondError::NotInitialized))
}

/// Select the next slave for transmission on a bond
pub fn select_tx_slave(bond_name: &str) -> Option<usize> {
    BOND_MANAGER
        .with(|lock| {
            let mut mgr = lock.write();
            let bond = mgr.bonds.get_mut(bond_name)?;
            bond.select_tx_slave()
        })
        .flatten()
}

/// Handle a link state change on a slave interface
pub fn handle_link_change(slave_name: &str, link_up: bool) {
    BOND_MANAGER.with(|lock| {
        let mut mgr = lock.write();
        for bond in mgr.bonds.values_mut() {
            bond.handle_link_change(slave_name, link_up);
        }
    });
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mac(last: u8) -> [u8; 6] {
        [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, last]
    }

    // --- BondInterface unit tests (no global state needed) ---

    #[test]
    fn test_create_bond_interface() {
        let bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        assert_eq!(bond.name, "bond0");
        assert_eq!(bond.mode, BondMode::ActiveBackup);
        assert!(bond.slaves.is_empty());
        assert_eq!(bond.mac_address, [0u8; 6]);
    }

    #[test]
    fn test_add_slave_sets_mac() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        let mac = make_mac(0x01);
        bond.add_slave("eth0", mac).unwrap();

        assert_eq!(bond.slaves.len(), 1);
        assert_eq!(bond.mac_address, mac);
        assert!(bond.slaves[0].is_active); // first slave becomes active
    }

    #[test]
    fn test_add_duplicate_slave_fails() {
        let mut bond = BondInterface::new("bond0", BondMode::RoundRobin);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        let result = bond.add_slave("eth0", make_mac(0x02));
        assert_eq!(result, Err(BondError::SlaveAlreadyExists));
    }

    #[test]
    fn test_remove_slave() {
        let mut bond = BondInterface::new("bond0", BondMode::RoundRobin);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();
        assert_eq!(bond.slaves.len(), 2);

        bond.remove_slave("eth0").unwrap();
        assert_eq!(bond.slaves.len(), 1);
        assert_eq!(bond.slaves[0].name, "eth1");
    }

    #[test]
    fn test_remove_nonexistent_slave_fails() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        let result = bond.remove_slave("eth99");
        assert_eq!(result, Err(BondError::SlaveNotFound));
    }

    #[test]
    fn test_active_backup_failover() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();

        // eth0 should be active
        assert!(bond.slaves[0].is_active);
        assert!(!bond.slaves[1].is_active);
        assert_eq!(bond.active_slave_index, 0);

        // Simulate eth0 link down
        bond.handle_link_change("eth0", false);

        // eth1 should now be active
        assert!(!bond.slaves[0].is_active);
        assert!(bond.slaves[1].is_active);
        assert_eq!(bond.active_slave_index, 1);
    }

    #[test]
    fn test_active_backup_select_tx() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();

        assert_eq!(bond.select_tx_slave(), Some(0));

        // Down the active slave
        bond.handle_link_change("eth0", false);
        assert_eq!(bond.select_tx_slave(), Some(1));

        // Down all slaves
        bond.handle_link_change("eth1", false);
        assert_eq!(bond.select_tx_slave(), None);
    }

    #[test]
    fn test_round_robin_selection() {
        let mut bond = BondInterface::new("bond0", BondMode::RoundRobin);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();
        bond.add_slave("eth2", make_mac(0x03)).unwrap();

        // Should cycle through 0, 1, 2, 0, 1, 2 ...
        assert_eq!(bond.select_tx_slave(), Some(0));
        assert_eq!(bond.select_tx_slave(), Some(1));
        assert_eq!(bond.select_tx_slave(), Some(2));
        assert_eq!(bond.select_tx_slave(), Some(0));
    }

    #[test]
    fn test_round_robin_skips_down_slave() {
        let mut bond = BondInterface::new("bond0", BondMode::RoundRobin);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();
        bond.add_slave("eth2", make_mac(0x03)).unwrap();

        // Down eth1
        bond.handle_link_change("eth1", false);

        // Should skip eth1
        assert_eq!(bond.select_tx_slave(), Some(0));
        assert_eq!(bond.select_tx_slave(), Some(2));
        assert_eq!(bond.select_tx_slave(), Some(0));
    }

    #[test]
    fn test_link_up_count() {
        let mut bond = BondInterface::new("bond0", BondMode::RoundRobin);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();
        assert_eq!(bond.link_up_count(), 2);

        bond.handle_link_change("eth0", false);
        assert_eq!(bond.link_up_count(), 1);
    }

    #[test]
    fn test_arp_monitor_tick() {
        let mut mon = ArpMonitor::new(1000);
        mon.add_target([192, 168, 1, 1]);

        // First tick at time 0 fires immediately (0 - 0 >= 1000 is false,
        // but the very first tick with last_check=0 and now=0 won't fire)
        assert!(!mon.tick(0));
        assert!(!mon.tick(500));
        assert!(mon.tick(1000));
        // After firing, last_check is updated to 1000
        assert!(!mon.tick(1500));
        assert!(mon.tick(2000));
    }

    #[test]
    fn test_arp_monitor_zero_interval() {
        let mut mon = ArpMonitor::new(0);
        // Zero interval means monitoring is disabled
        assert!(!mon.tick(0));
        assert!(!mon.tick(1000));
    }

    #[test]
    fn test_arp_monitor_no_duplicate_targets() {
        let mut mon = ArpMonitor::new(1000);
        mon.add_target([10, 0, 0, 1]);
        mon.add_target([10, 0, 0, 1]);
        assert_eq!(mon.targets.len(), 1);
    }

    #[test]
    fn test_remove_active_slave_promotes_next() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();

        // eth0 is active
        assert!(bond.slaves[0].is_active);

        // Remove active slave
        bond.remove_slave("eth0").unwrap();
        assert_eq!(bond.slaves.len(), 1);
        assert_eq!(bond.slaves[0].name, "eth1");
        assert!(bond.slaves[0].is_active);
    }

    #[test]
    fn test_all_slaves_down_then_recovery() {
        let mut bond = BondInterface::new("bond0", BondMode::ActiveBackup);
        bond.add_slave("eth0", make_mac(0x01)).unwrap();
        bond.add_slave("eth1", make_mac(0x02)).unwrap();

        // Down both
        bond.handle_link_change("eth0", false);
        bond.handle_link_change("eth1", false);
        assert_eq!(bond.select_tx_slave(), None);
        assert!(!bond.has_active_slave());

        // Bring eth1 back up
        bond.handle_link_change("eth1", true);
        assert!(bond.slaves[1].is_active);
        assert_eq!(bond.select_tx_slave(), Some(1));
    }
}
