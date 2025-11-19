//! Memory Protection Features
//!
//! Implements ASLR, stack canaries, and other memory protection mechanisms.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::RwLock;

use crate::{crypto::random::get_random, error::KernelError};

/// ASLR (Address Space Layout Randomization) manager
pub struct Aslr {
    /// Base entropy for randomization
    entropy_pool: RwLock<[u64; 16]>,
    /// Counter for mixing
    counter: AtomicU64,
}

impl Aslr {
    /// Create new ASLR instance
    pub fn new() -> Result<Self, KernelError> {
        let mut entropy_pool = [0u64; 16];

        // Fill with secure random data
        let rng = get_random();
        for entry in entropy_pool.iter_mut() {
            *entry = rng.next_u64();
        }

        Ok(Self {
            entropy_pool: RwLock::new(entropy_pool),
            counter: AtomicU64::new(0),
        })
    }

    /// Randomize address for given address space region
    pub fn randomize_address(&self, base: usize, region_type: RegionType) -> usize {
        let entropy = {
            let pool = self.entropy_pool.read();
            let index = (self.counter.fetch_add(1, Ordering::Relaxed) % 16) as usize;
            pool[index]
        };

        let randomization_bits = match region_type {
            RegionType::Stack => 28,      // 28 bits = 256MB range
            RegionType::Heap => 28,       // 28 bits = 256MB range
            RegionType::Executable => 24, // 24 bits = 16MB range
            RegionType::Library => 28,    // 28 bits = 256MB range
            RegionType::Mmap => 28,       // 28 bits = 256MB range
        };

        // Create mask for randomization
        let mask = (1u64 << randomization_bits) - 1;
        let random_offset = (entropy & mask) as usize;

        // Page-align the offset (4KB alignment)
        let aligned_offset = random_offset & !0xFFF;

        base.wrapping_add(aligned_offset)
    }

    /// Get random stack offset for stack canary
    pub fn get_stack_canary(&self) -> u64 {
        let rng = get_random();
        rng.next_u64()
    }

    /// Refresh entropy pool
    pub fn refresh_entropy(&self) {
        let mut pool = self.entropy_pool.write();
        let rng = get_random();

        for entry in pool.iter_mut() {
            *entry = rng.next_u64();
        }
    }
}

impl Default for Aslr {
    fn default() -> Self {
        Self::new().expect("Failed to create ASLR")
    }
}

/// Address space region types for ASLR
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionType {
    Stack,
    Heap,
    Executable,
    Library,
    Mmap,
}

/// Stack canary for detecting buffer overflows
pub struct StackCanary {
    /// Canary value
    value: u64,
}

impl StackCanary {
    /// Create new stack canary with random value
    pub fn new() -> Self {
        let rng = get_random();
        Self {
            value: rng.next_u64(),
        }
    }

    /// Get canary value
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Verify canary hasn't been modified
    pub fn verify(&self, observed_value: u64) -> bool {
        self.value == observed_value
    }

    /// Place canary on stack
    pub fn place(&self, stack_ptr: *mut u64) {
        unsafe {
            *stack_ptr = self.value;
        }
    }

    /// Check canary on stack
    pub fn check(&self, stack_ptr: *const u64) -> bool {
        unsafe { *stack_ptr == self.value }
    }
}

impl Default for StackCanary {
    fn default() -> Self {
        Self::new()
    }
}

/// Guard page for detecting stack overflow
pub struct GuardPage {
    /// Address of guard page
    address: usize,
    /// Size of guard page
    size: usize,
}

impl GuardPage {
    /// Create new guard page
    pub fn new(address: usize, size: usize) -> Self {
        Self { address, size }
    }

    /// Get guard page address
    pub fn address(&self) -> usize {
        self.address
    }

    /// Get guard page size
    pub fn size(&self) -> usize {
        self.size
    }

    /// Check if address is within guard page
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.address && addr < self.address + self.size
    }
}

/// Memory protection manager
pub struct MemoryProtection {
    aslr: Aslr,
    stack_canaries_enabled: bool,
    guard_pages_enabled: bool,
    dep_enabled: bool, // Data Execution Prevention
}

impl MemoryProtection {
    /// Create new memory protection manager
    pub fn new() -> Result<Self, KernelError> {
        Ok(Self {
            aslr: Aslr::new()?,
            stack_canaries_enabled: true,
            guard_pages_enabled: true,
            dep_enabled: true,
        })
    }

    /// Get ASLR instance
    pub fn aslr(&self) -> &Aslr {
        &self.aslr
    }

    /// Enable/disable stack canaries
    pub fn set_stack_canaries(&mut self, enabled: bool) {
        self.stack_canaries_enabled = enabled;
    }

    /// Check if stack canaries are enabled
    pub fn stack_canaries_enabled(&self) -> bool {
        self.stack_canaries_enabled
    }

    /// Enable/disable guard pages
    pub fn set_guard_pages(&mut self, enabled: bool) {
        self.guard_pages_enabled = enabled;
    }

    /// Check if guard pages are enabled
    pub fn guard_pages_enabled(&self) -> bool {
        self.guard_pages_enabled
    }

    /// Enable/disable DEP
    pub fn set_dep(&mut self, enabled: bool) {
        self.dep_enabled = enabled;
    }

    /// Check if DEP is enabled
    pub fn dep_enabled(&self) -> bool {
        self.dep_enabled
    }

    /// Create stack canary if enabled
    pub fn create_canary(&self) -> Option<StackCanary> {
        if self.stack_canaries_enabled {
            Some(StackCanary::new())
        } else {
            None
        }
    }

    /// Create guard page if enabled
    pub fn create_guard_page(&self, address: usize, size: usize) -> Option<GuardPage> {
        if self.guard_pages_enabled {
            Some(GuardPage::new(address, size))
        } else {
            None
        }
    }
}

impl Default for MemoryProtection {
    fn default() -> Self {
        Self::new().expect("Failed to create MemoryProtection")
    }
}

/// Global memory protection instance
static mut MEMORY_PROTECTION: Option<MemoryProtection> = None;

/// Initialize memory protection
pub fn init() -> Result<(), KernelError> {
    unsafe {
        MEMORY_PROTECTION = Some(MemoryProtection::new()?);
    }

    crate::println!("[MEMORY-PROTECTION] ASLR, stack canaries, and guard pages enabled");
    Ok(())
}

/// Get global memory protection instance
pub fn get_memory_protection() -> &'static MemoryProtection {
    unsafe {
        MEMORY_PROTECTION
            .as_ref()
            .expect("Memory protection not initialized")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_aslr_randomization() {
        let aslr = Aslr::new().unwrap();

        let base = 0x400000;
        let addr1 = aslr.randomize_address(base, RegionType::Stack);
        let addr2 = aslr.randomize_address(base, RegionType::Stack);

        // Addresses should be different
        assert_ne!(addr1, addr2);

        // Addresses should be page-aligned
        assert_eq!(addr1 & 0xFFF, 0);
        assert_eq!(addr2 & 0xFFF, 0);
    }

    #[test_case]
    fn test_stack_canary() {
        let canary = StackCanary::new();
        let value = canary.value();

        assert!(canary.verify(value));
        assert!(!canary.verify(value ^ 1));
    }
}
