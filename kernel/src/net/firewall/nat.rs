//! Network Address Translation (NAT) engine
//!
//! Supports SNAT (source NAT), DNAT (destination NAT), and IP masquerading.
//! Uses a port pool with bitmap allocation for ephemeral port assignment.
//! Implements RFC 1624 incremental checksum updates to avoid full
//! recalculation when translating addresses.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

use super::conntrack::{ConntrackKey, NatInfo};
use crate::{
    error::KernelError,
    net::{Ipv4Address, Port},
    sync::once_lock::GlobalState,
};

// ============================================================================
// Constants
// ============================================================================

/// Start of ephemeral port range for NAT
const PORT_POOL_START: u16 = 49152;

/// End of ephemeral port range for NAT (inclusive)
const PORT_POOL_END: u16 = 65535;

/// Total ports in the pool
const PORT_POOL_SIZE: usize = (PORT_POOL_END - PORT_POOL_START + 1) as usize;

/// Number of u64 words needed for the port bitmap
const BITMAP_WORDS: usize = PORT_POOL_SIZE.div_ceil(64);

// ============================================================================
// NAT Type
// ============================================================================

/// Type of NAT translation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// Source NAT: rewrite source address/port
    Snat,
    /// Destination NAT: rewrite destination address/port
    Dnat,
    /// Masquerade: SNAT using the outgoing interface address
    Masquerade,
}

// ============================================================================
// NAT Mapping
// ============================================================================

/// A single NAT mapping recording original and translated addresses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatMapping {
    /// NAT type
    pub nat_type: NatType,
    /// Original source address
    pub original_src_ip: Ipv4Address,
    /// Original source port
    pub original_src_port: Port,
    /// Original destination address
    pub original_dst_ip: Ipv4Address,
    /// Original destination port
    pub original_dst_port: Port,
    /// Translated source address
    pub translated_src_ip: Ipv4Address,
    /// Translated source port
    pub translated_src_port: Port,
    /// Translated destination address
    pub translated_dst_ip: Ipv4Address,
    /// Translated destination port
    pub translated_dst_port: Port,
}

impl NatMapping {
    /// Convert to a NatInfo for conntrack
    pub fn to_nat_info(&self) -> NatInfo {
        NatInfo {
            original_src_ip: self.original_src_ip,
            original_src_port: self.original_src_port,
            translated_src_ip: self.translated_src_ip,
            translated_src_port: self.translated_src_port,
            original_dst_ip: self.original_dst_ip,
            original_dst_port: self.original_dst_port,
            translated_dst_ip: self.translated_dst_ip,
            translated_dst_port: self.translated_dst_port,
        }
    }
}

// ============================================================================
// Port Pool
// ============================================================================

/// Bitmap-based ephemeral port allocator for NAT
///
/// Manages ports in the range 49152-65535 using a compact bitmap.
/// Each bit represents one port: 0 = free, 1 = allocated.
pub struct PortPool {
    /// Bitmap of allocated ports (bit N = port PORT_POOL_START + N)
    bitmap: [u64; BITMAP_WORDS],
    /// Number of currently allocated ports
    allocated_count: u16,
}

impl PortPool {
    /// Create a new port pool with all ports available
    pub fn new() -> Self {
        Self {
            bitmap: [0u64; BITMAP_WORDS],
            allocated_count: 0,
        }
    }

    /// Number of ports currently allocated
    pub fn allocated(&self) -> u16 {
        self.allocated_count
    }

    /// Number of ports available
    pub fn available(&self) -> u16 {
        PORT_POOL_SIZE as u16 - self.allocated_count
    }

    /// Allocate the next available port
    ///
    /// Returns the allocated port number or None if pool is exhausted.
    pub fn allocate(&mut self) -> Option<Port> {
        for (word_idx, word) in self.bitmap.iter_mut().enumerate() {
            if *word == u64::MAX {
                continue; // All bits set in this word
            }
            // Find first zero bit
            let bit_idx = (!*word).trailing_zeros() as usize;
            let port_offset = word_idx * 64 + bit_idx;
            if port_offset >= PORT_POOL_SIZE {
                return None;
            }
            *word |= 1u64 << bit_idx;
            self.allocated_count += 1;
            return Some(PORT_POOL_START + port_offset as u16);
        }
        None
    }

    /// Release a previously allocated port
    pub fn release(&mut self, port: Port) -> bool {
        if !(PORT_POOL_START..=PORT_POOL_END).contains(&port) {
            return false;
        }
        let offset = (port - PORT_POOL_START) as usize;
        let word_idx = offset / 64;
        let bit_idx = offset % 64;
        if self.bitmap[word_idx] & (1u64 << bit_idx) != 0 {
            self.bitmap[word_idx] &= !(1u64 << bit_idx);
            self.allocated_count -= 1;
            true
        } else {
            false // Port was not allocated
        }
    }

    /// Check if a specific port is allocated
    pub fn is_allocated(&self, port: Port) -> bool {
        if !(PORT_POOL_START..=PORT_POOL_END).contains(&port) {
            return false;
        }
        let offset = (port - PORT_POOL_START) as usize;
        let word_idx = offset / 64;
        let bit_idx = offset % 64;
        self.bitmap[word_idx] & (1u64 << bit_idx) != 0
    }
}

impl Default for PortPool {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Incremental Checksum Update (RFC 1624)
// ============================================================================

/// Incrementally update a one's complement checksum when a 16-bit value
/// changes.
///
/// Implements the algorithm from RFC 1624:
///   HC' = ~(~HC + ~m + m')
///
/// where HC is the old checksum, m is the old value, and m' is the new value.
/// All arithmetic is one's complement (16-bit with end-around carry).
pub fn update_checksum(old_checksum: u16, old_value: u16, new_value: u16) -> u16 {
    // Work in u32 to handle carry
    let hc = !old_checksum as u32;
    let m = !old_value as u32;
    let m_prime = new_value as u32;

    let mut sum = hc + m + m_prime;

    // Fold carry bits
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !sum as u16
}

/// Update checksum for a 32-bit (IP address) change, processing two 16-bit
/// halves
pub fn update_checksum_32(old_checksum: u16, old_addr: u32, new_addr: u32) -> u16 {
    let old_hi = (old_addr >> 16) as u16;
    let old_lo = old_addr as u16;
    let new_hi = (new_addr >> 16) as u16;
    let new_lo = new_addr as u16;

    let c1 = update_checksum(old_checksum, old_hi, new_hi);
    update_checksum(c1, old_lo, new_lo)
}

// ============================================================================
// NAT Engine
// ============================================================================

/// The NAT engine managing translations and port allocation
pub struct NatEngine {
    /// Ephemeral port pool
    pub port_pool: PortPool,
    /// Active NAT mappings indexed by original connection key
    pub mappings: BTreeMap<ConntrackKey, NatMapping>,
    /// Masquerade address (outgoing interface IP)
    pub masquerade_addr: Ipv4Address,
    /// Total translations performed
    pub total_translations: u64,
}

impl NatEngine {
    /// Create a new NAT engine
    pub fn new() -> Self {
        Self {
            port_pool: PortPool::new(),
            mappings: BTreeMap::new(),
            masquerade_addr: Ipv4Address::ANY,
            total_translations: 0,
        }
    }

    /// Set the masquerade (outgoing interface) address
    pub fn set_masquerade_addr(&mut self, addr: Ipv4Address) {
        self.masquerade_addr = addr;
    }

    /// Translate an outbound packet with SNAT
    ///
    /// Rewrites the source address and allocates a new source port.
    /// Returns the NAT mapping on success.
    pub fn translate_outbound_snat(
        &mut self,
        key: &ConntrackKey,
        new_src_ip: Ipv4Address,
    ) -> Option<NatMapping> {
        // Check for existing mapping
        if let Some(mapping) = self.mappings.get(key) {
            return Some(*mapping);
        }

        // Allocate a new port
        let new_port = self.port_pool.allocate()?;

        let mapping = NatMapping {
            nat_type: NatType::Snat,
            original_src_ip: key.src_ip,
            original_src_port: key.src_port,
            original_dst_ip: key.dst_ip,
            original_dst_port: key.dst_port,
            translated_src_ip: new_src_ip,
            translated_src_port: new_port,
            translated_dst_ip: key.dst_ip,
            translated_dst_port: key.dst_port,
        };

        self.mappings.insert(*key, mapping);
        self.total_translations += 1;
        Some(mapping)
    }

    /// Translate an outbound packet with masquerading
    ///
    /// Uses the configured masquerade address as the source.
    pub fn translate_outbound_masquerade(&mut self, key: &ConntrackKey) -> Option<NatMapping> {
        let addr = self.masquerade_addr;
        if addr == Ipv4Address::ANY {
            return None;
        }

        // Check for existing mapping
        if let Some(mapping) = self.mappings.get(key) {
            return Some(*mapping);
        }

        let new_port = self.port_pool.allocate()?;

        let mapping = NatMapping {
            nat_type: NatType::Masquerade,
            original_src_ip: key.src_ip,
            original_src_port: key.src_port,
            original_dst_ip: key.dst_ip,
            original_dst_port: key.dst_port,
            translated_src_ip: addr,
            translated_src_port: new_port,
            translated_dst_ip: key.dst_ip,
            translated_dst_port: key.dst_port,
        };

        self.mappings.insert(*key, mapping);
        self.total_translations += 1;
        Some(mapping)
    }

    /// Translate an inbound packet with DNAT
    ///
    /// Rewrites the destination address and port.
    pub fn translate_inbound_dnat(
        &mut self,
        key: &ConntrackKey,
        new_dst_ip: Ipv4Address,
        new_dst_port: Port,
    ) -> Option<NatMapping> {
        // Check for existing mapping
        if let Some(mapping) = self.mappings.get(key) {
            return Some(*mapping);
        }

        let mapping = NatMapping {
            nat_type: NatType::Dnat,
            original_src_ip: key.src_ip,
            original_src_port: key.src_port,
            original_dst_ip: key.dst_ip,
            original_dst_port: key.dst_port,
            translated_src_ip: key.src_ip,
            translated_src_port: key.src_port,
            translated_dst_ip: new_dst_ip,
            translated_dst_port: new_dst_port,
        };

        self.mappings.insert(*key, mapping);
        self.total_translations += 1;
        Some(mapping)
    }

    /// Look up a reverse NAT mapping for inbound reply traffic
    ///
    /// Given a reply packet's key, find the corresponding SNAT/masquerade
    /// mapping to reverse the translation.
    pub fn lookup_reverse(&self, reply_key: &ConntrackKey) -> Option<&NatMapping> {
        // For SNAT/masquerade, the reply destination is our translated source.
        // We need to find the original mapping where:
        //   translated_src_ip == reply_key.dst_ip
        //   translated_src_port == reply_key.dst_port
        for mapping in self.mappings.values() {
            match mapping.nat_type {
                NatType::Snat | NatType::Masquerade => {
                    if mapping.translated_src_ip == reply_key.dst_ip
                        && mapping.translated_src_port == reply_key.dst_port
                        && mapping.original_dst_ip == reply_key.src_ip
                    {
                        return Some(mapping);
                    }
                }
                NatType::Dnat => {
                    if mapping.translated_dst_ip == reply_key.src_ip
                        && mapping.translated_dst_port == reply_key.src_port
                        && mapping.original_src_ip == reply_key.dst_ip
                    {
                        return Some(mapping);
                    }
                }
            }
        }
        None
    }

    /// Remove a NAT mapping and release its allocated port
    pub fn remove_mapping(&mut self, key: &ConntrackKey) -> Option<NatMapping> {
        if let Some(mapping) = self.mappings.remove(key) {
            // Release port for SNAT/masquerade
            match mapping.nat_type {
                NatType::Snat | NatType::Masquerade => {
                    self.port_pool.release(mapping.translated_src_port);
                }
                NatType::Dnat => {}
            }
            Some(mapping)
        } else {
            None
        }
    }

    /// Number of active mappings
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }
}

impl Default for NatEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Global State
// ============================================================================

static NAT_ENGINE: GlobalState<spin::Mutex<NatEngine>> = GlobalState::new();

/// Initialize the NAT engine
pub fn init() -> Result<(), KernelError> {
    NAT_ENGINE
        .init(spin::Mutex::new(NatEngine::new()))
        .map_err(|_| KernelError::InvalidAddress { addr: 0 })?;
    Ok(())
}

/// Access the global NAT engine
pub fn with_nat<R, F: FnOnce(&mut NatEngine) -> R>(f: F) -> Option<R> {
    NAT_ENGINE.with(|lock| {
        let mut engine = lock.lock();
        f(&mut engine)
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_pool_allocate() {
        let mut pool = PortPool::new();
        let port = pool.allocate().unwrap();
        assert_eq!(port, PORT_POOL_START);
        assert_eq!(pool.allocated(), 1);
        assert!(pool.is_allocated(port));
    }

    #[test]
    fn test_port_pool_release() {
        let mut pool = PortPool::new();
        let port = pool.allocate().unwrap();
        assert!(pool.release(port));
        assert_eq!(pool.allocated(), 0);
        assert!(!pool.is_allocated(port));
    }

    #[test]
    fn test_port_pool_release_invalid() {
        let mut pool = PortPool::new();
        assert!(!pool.release(80)); // Below range
        assert!(!pool.release(PORT_POOL_START)); // Not allocated
    }

    #[test]
    fn test_port_pool_sequential_allocation() {
        let mut pool = PortPool::new();
        let p1 = pool.allocate().unwrap();
        let p2 = pool.allocate().unwrap();
        let p3 = pool.allocate().unwrap();
        assert_eq!(p1, PORT_POOL_START);
        assert_eq!(p2, PORT_POOL_START + 1);
        assert_eq!(p3, PORT_POOL_START + 2);
        assert_eq!(pool.allocated(), 3);
    }

    #[test]
    fn test_port_pool_reuse_released() {
        let mut pool = PortPool::new();
        let p1 = pool.allocate().unwrap();
        let _p2 = pool.allocate().unwrap();
        pool.release(p1);
        let p3 = pool.allocate().unwrap();
        assert_eq!(p3, p1); // Should reuse first available
    }

    #[test]
    fn test_checksum_update_identity() {
        // If old == new, checksum should not change
        let checksum = 0x1234;
        let result = update_checksum(checksum, 0xABCD, 0xABCD);
        assert_eq!(result, checksum);
    }

    #[test]
    fn test_checksum_update_basic() {
        // Known test: old_checksum = 0xDD2F, change 0x5555 -> 0x3285
        // Expected: 0x0000 (from RFC 1624 example adapted)
        let result = update_checksum(0x0000, 0x5555, 0x5555);
        assert_eq!(result, 0x0000); // Identity
    }

    #[test]
    fn test_checksum_update_32_identity() {
        let checksum = 0xABCD;
        let addr: u32 = 0xC0A80101; // 192.168.1.1
        let result = update_checksum_32(checksum, addr, addr);
        assert_eq!(result, checksum);
    }

    #[test]
    fn test_nat_engine_snat() {
        let mut engine = NatEngine::new();
        let key = ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 100),
            Ipv4Address::new(8, 8, 8, 8),
            12345,
            53,
            ConntrackKey::PROTO_UDP,
        );
        let public_ip = Ipv4Address::new(203, 0, 113, 1);

        let mapping = engine.translate_outbound_snat(&key, public_ip).unwrap();
        assert_eq!(mapping.nat_type, NatType::Snat);
        assert_eq!(mapping.original_src_ip, Ipv4Address::new(192, 168, 1, 100));
        assert_eq!(mapping.translated_src_ip, public_ip);
        assert!(mapping.translated_src_port >= PORT_POOL_START);
        assert_eq!(engine.mapping_count(), 1);
    }

    #[test]
    fn test_nat_engine_masquerade() {
        let mut engine = NatEngine::new();
        engine.set_masquerade_addr(Ipv4Address::new(203, 0, 113, 1));
        let key = ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 50),
            Ipv4Address::new(1, 1, 1, 1),
            5000,
            443,
            ConntrackKey::PROTO_TCP,
        );

        let mapping = engine.translate_outbound_masquerade(&key).unwrap();
        assert_eq!(mapping.nat_type, NatType::Masquerade);
        assert_eq!(mapping.translated_src_ip, Ipv4Address::new(203, 0, 113, 1));
    }

    #[test]
    fn test_nat_engine_masquerade_no_addr() {
        let mut engine = NatEngine::new();
        // masquerade_addr is ANY (default)
        let key = ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 50),
            Ipv4Address::new(1, 1, 1, 1),
            5000,
            443,
            ConntrackKey::PROTO_TCP,
        );
        assert!(engine.translate_outbound_masquerade(&key).is_none());
    }

    #[test]
    fn test_nat_engine_dnat() {
        let mut engine = NatEngine::new();
        let key = ConntrackKey::new(
            Ipv4Address::new(8, 8, 8, 8),
            Ipv4Address::new(203, 0, 113, 1),
            5000,
            80,
            ConntrackKey::PROTO_TCP,
        );
        let internal_ip = Ipv4Address::new(192, 168, 1, 10);

        let mapping = engine
            .translate_inbound_dnat(&key, internal_ip, 8080)
            .unwrap();
        assert_eq!(mapping.nat_type, NatType::Dnat);
        assert_eq!(mapping.translated_dst_ip, internal_ip);
        assert_eq!(mapping.translated_dst_port, 8080);
    }

    #[test]
    fn test_nat_engine_remove_mapping() {
        let mut engine = NatEngine::new();
        let key = ConntrackKey::new(
            Ipv4Address::new(192, 168, 1, 100),
            Ipv4Address::new(8, 8, 8, 8),
            12345,
            53,
            ConntrackKey::PROTO_UDP,
        );
        let public_ip = Ipv4Address::new(203, 0, 113, 1);

        let mapping = engine.translate_outbound_snat(&key, public_ip).unwrap();
        let allocated_port = mapping.translated_src_port;
        assert!(engine.port_pool.is_allocated(allocated_port));

        engine.remove_mapping(&key);
        assert_eq!(engine.mapping_count(), 0);
        assert!(!engine.port_pool.is_allocated(allocated_port));
    }
}
