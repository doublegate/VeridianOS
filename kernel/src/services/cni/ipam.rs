//! IPAM (IP Address Management)
//!
//! Provides bitmap-based IP address allocation for container networking
//! with per-subnet management and overflow detection.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// IPAM Config
// ---------------------------------------------------------------------------

/// IPAM configuration for a subnet.
#[derive(Debug, Clone)]
pub struct IpamConfig {
    /// Subnet base address (host byte order).
    pub subnet: u32,
    /// Prefix length (e.g., 24 for /24).
    pub prefix_len: u8,
    /// Gateway address.
    pub gateway: u32,
    /// Start of allocatable range (host part only).
    pub range_start: u32,
    /// End of allocatable range (host part only, inclusive).
    pub range_end: u32,
}

impl IpamConfig {
    /// Create a new IPAM config.
    ///
    /// `subnet` is the network address (e.g., 10.244.0.0 = 0x0AF40000).
    /// `prefix_len` is the CIDR prefix (e.g., 24).
    /// `gateway` is the gateway host part (e.g., 1 for .1).
    pub fn new(subnet: u32, prefix_len: u8, gateway_host: u32) -> Self {
        let mask = if prefix_len >= 32 {
            0xFFFF_FFFF
        } else {
            !((1u32 << (32 - prefix_len)) - 1)
        };
        let subnet_base = subnet & mask;
        let host_bits = 32 - prefix_len as u32;
        let max_host = if host_bits >= 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << host_bits) - 1
        };

        IpamConfig {
            subnet: subnet_base,
            prefix_len,
            gateway: subnet_base | gateway_host,
            range_start: 2, // skip .0 (network) and .1 (gateway)
            range_end: max_host.saturating_sub(1), // skip broadcast
        }
    }

    /// Get the subnet mask.
    pub fn mask(&self) -> u32 {
        if self.prefix_len >= 32 {
            0xFFFF_FFFF
        } else {
            !((1u32 << (32 - self.prefix_len)) - 1)
        }
    }

    /// Get the total number of allocatable addresses.
    pub fn total_addresses(&self) -> u32 {
        if self.range_end >= self.range_start {
            self.range_end - self.range_start + 1
        } else {
            0
        }
    }

    /// Format an IP address as a string.
    pub fn format_ip(ip: u32) -> String {
        alloc::format!(
            "{}.{}.{}.{}",
            (ip >> 24) & 0xFF,
            (ip >> 16) & 0xFF,
            (ip >> 8) & 0xFF,
            ip & 0xFF
        )
    }
}

// ---------------------------------------------------------------------------
// IPAM Error
// ---------------------------------------------------------------------------

/// IPAM error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpamError {
    /// No addresses available.
    Exhausted,
    /// Address not allocated.
    NotAllocated(u32),
    /// Address already allocated.
    AlreadyAllocated(u32),
    /// Address out of range.
    OutOfRange(u32),
}

// ---------------------------------------------------------------------------
// IPAM Allocator
// ---------------------------------------------------------------------------

/// Bitmap-based IP address allocator for a single subnet.
#[derive(Debug)]
pub struct IpamAllocator {
    /// Bitmap of allocated addresses (1 bit per host address).
    bitmap: Vec<u64>,
    /// Subnet base address.
    subnet_base: u32,
    /// Prefix length.
    prefix_len: u8,
    /// Range start (host part).
    range_start: u32,
    /// Range end (host part, inclusive).
    range_end: u32,
    /// Number of currently allocated addresses.
    allocated: u32,
    /// Gateway address.
    gateway: u32,
}

impl IpamAllocator {
    /// Create a new IPAM allocator from config.
    pub fn new(config: &IpamConfig) -> Self {
        // Calculate bitmap size: enough u64s to cover the range
        let total = config.total_addresses();
        let bitmap_len = (total as usize).div_ceil(64);

        IpamAllocator {
            bitmap: alloc::vec![0u64; bitmap_len],
            subnet_base: config.subnet,
            prefix_len: config.prefix_len,
            range_start: config.range_start,
            range_end: config.range_end,
            allocated: 0,
            gateway: config.gateway,
        }
    }

    /// Allocate the next available IP address.
    ///
    /// Returns the full IP address.
    pub fn allocate(&mut self) -> Result<u32, IpamError> {
        let total = self.total_addresses();
        if self.allocated >= total {
            return Err(IpamError::Exhausted);
        }

        // Find first free bit
        for (word_idx, word) in self.bitmap.iter_mut().enumerate() {
            if *word == u64::MAX {
                continue;
            }
            // Find first zero bit
            let bit = (!*word).trailing_zeros();
            if bit >= 64 {
                continue;
            }
            let offset = (word_idx as u32) * 64 + bit;
            let host_part = self.range_start + offset;
            if host_part > self.range_end {
                break;
            }

            *word |= 1u64 << bit;
            self.allocated += 1;
            return Ok(self.subnet_base | host_part);
        }

        Err(IpamError::Exhausted)
    }

    /// Release an allocated IP address.
    pub fn release(&mut self, ip: u32) -> Result<(), IpamError> {
        let host_part = ip & !self.mask();

        if host_part < self.range_start || host_part > self.range_end {
            return Err(IpamError::OutOfRange(ip));
        }

        let offset = host_part - self.range_start;
        let word_idx = (offset / 64) as usize;
        let bit = offset % 64;

        if word_idx >= self.bitmap.len() {
            return Err(IpamError::OutOfRange(ip));
        }

        if self.bitmap[word_idx] & (1u64 << bit) == 0 {
            return Err(IpamError::NotAllocated(ip));
        }

        self.bitmap[word_idx] &= !(1u64 << bit);
        self.allocated = self.allocated.saturating_sub(1);
        Ok(())
    }

    /// Check if an IP address is currently allocated.
    pub fn is_allocated(&self, ip: u32) -> bool {
        let host_part = ip & !self.mask();
        if host_part < self.range_start || host_part > self.range_end {
            return false;
        }
        let offset = host_part - self.range_start;
        let word_idx = (offset / 64) as usize;
        let bit = offset % 64;
        if word_idx >= self.bitmap.len() {
            return false;
        }
        self.bitmap[word_idx] & (1u64 << bit) != 0
    }

    /// Get the number of available addresses.
    pub fn available_count(&self) -> u32 {
        self.total_addresses().saturating_sub(self.allocated)
    }

    /// Get the number of allocated addresses.
    pub fn allocated_count(&self) -> u32 {
        self.allocated
    }

    /// Get the total number of allocatable addresses.
    pub fn total_addresses(&self) -> u32 {
        if self.range_end >= self.range_start {
            self.range_end - self.range_start + 1
        } else {
            0
        }
    }

    /// Get the subnet mask.
    fn mask(&self) -> u32 {
        if self.prefix_len >= 32 {
            0xFFFF_FFFF
        } else {
            !((1u32 << (32 - self.prefix_len)) - 1)
        }
    }

    /// Get the gateway address.
    pub fn gateway(&self) -> u32 {
        self.gateway
    }

    /// Format the allocator status as a string.
    pub fn status_string(&self) -> String {
        alloc::format!(
            "subnet={}/{} allocated={}/{} available={}",
            IpamConfig::format_ip(self.subnet_base),
            self.prefix_len,
            self.allocated,
            self.total_addresses(),
            self.available_count()
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> u32 {
        ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | (d as u32)
    }

    fn make_allocator() -> IpamAllocator {
        let config = IpamConfig::new(ip(10, 244, 0, 0), 24, 1);
        IpamAllocator::new(&config)
    }

    #[test]
    fn test_ipam_config_mask() {
        let config = IpamConfig::new(ip(10, 244, 0, 0), 24, 1);
        assert_eq!(config.mask(), 0xFFFFFF00);
        assert_eq!(config.gateway, ip(10, 244, 0, 1));
    }

    #[test]
    fn test_ipam_config_total_addresses() {
        let config = IpamConfig::new(ip(10, 244, 0, 0), 24, 1);
        // /24: .2 through .254 = 253
        assert_eq!(config.total_addresses(), 253);
    }

    #[test]
    fn test_allocate_first() {
        let mut alloc = make_allocator();
        let addr = alloc.allocate().unwrap();
        assert_eq!(addr, ip(10, 244, 0, 2));
    }

    #[test]
    fn test_allocate_sequential() {
        let mut alloc = make_allocator();
        let a1 = alloc.allocate().unwrap();
        let a2 = alloc.allocate().unwrap();
        let a3 = alloc.allocate().unwrap();
        assert_eq!(a1, ip(10, 244, 0, 2));
        assert_eq!(a2, ip(10, 244, 0, 3));
        assert_eq!(a3, ip(10, 244, 0, 4));
        assert_eq!(alloc.allocated_count(), 3);
    }

    #[test]
    fn test_release_and_realloc() {
        let mut alloc = make_allocator();
        let a1 = alloc.allocate().unwrap();
        let _a2 = alloc.allocate().unwrap();
        alloc.release(a1).unwrap();
        assert_eq!(alloc.allocated_count(), 1);

        // Should reuse the released address
        let a3 = alloc.allocate().unwrap();
        assert_eq!(a3, a1);
    }

    #[test]
    fn test_is_allocated() {
        let mut alloc = make_allocator();
        let addr = alloc.allocate().unwrap();
        assert!(alloc.is_allocated(addr));
        alloc.release(addr).unwrap();
        assert!(!alloc.is_allocated(addr));
    }

    #[test]
    fn test_release_not_allocated() {
        let mut alloc = make_allocator();
        assert_eq!(
            alloc.release(ip(10, 244, 0, 50)),
            Err(IpamError::NotAllocated(ip(10, 244, 0, 50)))
        );
    }

    #[test]
    fn test_release_out_of_range() {
        let mut alloc = make_allocator();
        assert_eq!(
            alloc.release(ip(192, 168, 0, 1)),
            Err(IpamError::OutOfRange(ip(192, 168, 0, 1)))
        );
    }

    #[test]
    fn test_format_ip() {
        assert_eq!(IpamConfig::format_ip(ip(10, 244, 0, 1)), "10.244.0.1");
        assert_eq!(IpamConfig::format_ip(ip(192, 168, 1, 100)), "192.168.1.100");
    }
}
