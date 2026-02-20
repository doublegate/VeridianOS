//! Memory Protection Features
//!
//! Implements ASLR, stack canaries, and other memory protection mechanisms.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spin::RwLock;

use crate::{crypto::random::get_random, error::KernelError, sync::once_lock::OnceLock};

/// ASLR (Address Space Layout Randomization) manager
pub struct Aslr {
    /// Base entropy for randomization
    entropy_pool: RwLock<[u64; 16]>,
    /// Counter for mixing
    counter: AtomicU64,
    /// Whether the entropy pool has been seeded
    seeded: AtomicBool,
}

impl Aslr {
    /// Create new ASLR instance (lightweight — defers CSPRNG to first use)
    ///
    /// Entropy seeding is deferred to `ensure_seeded()` to avoid deep crypto
    /// call chains during early boot, which can overflow the small x86_64
    /// kernel stack in debug mode.
    pub fn new() -> Result<Self, KernelError> {
        Ok(Self {
            entropy_pool: RwLock::new([0u64; 16]),
            counter: AtomicU64::new(0),
            seeded: AtomicBool::new(false),
        })
    }

    /// Seed the entropy pool from the CSPRNG (called lazily on first use)
    fn ensure_seeded(&self) {
        if self.seeded.load(Ordering::Acquire) {
            return;
        }

        let rng = get_random();
        let mut pool = self.entropy_pool.write();
        // Double-check after acquiring write lock
        if !self.seeded.load(Ordering::Relaxed) {
            let mut i = 0;
            while i < 16 {
                pool[i] = rng.next_u64();
                i += 1;
            }
            self.seeded.store(true, Ordering::Release);
        }
    }

    /// Randomize address for given address space region
    pub fn randomize_address(&self, base: usize, region_type: RegionType) -> usize {
        self.ensure_seeded();
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

        // Use index-based loop instead of iter_mut() to avoid AArch64 LLVM hang
        let mut i = 0;
        while i < 16 {
            pool[i] = rng.next_u64();
            i += 1;
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
        // SAFETY: The caller must ensure stack_ptr points to a valid, aligned, writable
        // u64 location within the process's stack. This is used during process creation
        // where the stack is freshly allocated and the canary location is computed from
        // the known stack base.
        unsafe {
            *stack_ptr = self.value;
        }
    }

    /// Check canary on stack
    pub fn check(&self, stack_ptr: *const u64) -> bool {
        // SAFETY: The caller must ensure stack_ptr points to a valid, aligned, readable
        // u64 location where a canary was previously placed via place(). If the canary
        // has been overwritten by a buffer overflow, this read is still safe (it
        // returns valid u64 data), but the comparison will fail indicating
        // corruption.
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

/// W^X (Write XOR Execute) policy enforcement.
///
/// Ensures no memory page is both writable and executable simultaneously.
pub struct WxPolicy {
    enabled: bool,
    violations: AtomicU64,
}

impl WxPolicy {
    pub fn new() -> Self {
        Self {
            enabled: true,
            violations: AtomicU64::new(0),
        }
    }

    /// Check whether a page flags combination violates W^X.
    ///
    /// Returns `true` if the flags are safe (not both writable and executable).
    pub fn check_flags(&self, writable: bool, executable: bool) -> bool {
        if !self.enabled {
            return true;
        }
        if writable && executable {
            self.violations.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    /// Get the number of detected W^X violations.
    pub fn violation_count(&self) -> u64 {
        self.violations.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl Default for WxPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// DEP (Data Execution Prevention) / NX enforcement.
///
/// Tracks pages that should have the NX bit set and provides helpers
/// for ensuring data pages are not executable.
pub struct DepEnforcement {
    enabled: bool,
}

impl DepEnforcement {
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Determine whether a page at the given address should have NX set.
    ///
    /// Data, heap, and stack pages should always be non-executable.
    pub fn should_set_nx(&self, region: RegionType) -> bool {
        if !self.enabled {
            return false;
        }
        matches!(
            region,
            RegionType::Stack | RegionType::Heap | RegionType::Mmap
        )
    }

    /// Apply NX bit to page table entry flags.
    ///
    /// Returns the flags with NO_EXECUTE added if the region type warrants it.
    pub fn enforce_flags(&self, flags: u64, region: RegionType) -> u64 {
        if self.should_set_nx(region) {
            // NX bit is bit 63 on x86_64 page table entries
            flags | (1u64 << 63)
        } else {
            flags
        }
    }
}

impl Default for DepEnforcement {
    fn default() -> Self {
        Self::new()
    }
}

/// Spectre v1 mitigation helpers.
pub struct SpectreMitigation;

impl SpectreMitigation {
    /// Insert a speculation barrier after a bounds check.
    ///
    /// On x86_64 this emits LFENCE, on AArch64 CSDB, on RISC-V FENCE.
    #[inline(always)]
    pub fn speculation_barrier() {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: LFENCE serializes instruction execution, preventing speculative
        // reads past this point. No memory or stack effects.
        unsafe {
            core::arch::asm!("lfence", options(nomem, nostack));
        }

        #[cfg(target_arch = "aarch64")]
        // SAFETY: CSDB (Consumption of Speculative Data Barrier) prevents
        // speculative data access. No memory or stack effects.
        unsafe {
            core::arch::asm!("csdb", options(nomem, nostack));
        }

        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        // SAFETY: FENCE R,R orders prior reads before subsequent reads,
        // serving as a speculation barrier. No memory or stack effects.
        unsafe {
            core::arch::asm!("fence r, r", options(nomem, nostack));
        }
    }

    /// Bounds-checked array access with speculation barrier.
    ///
    /// Returns the value at `index` if in bounds, otherwise returns the
    /// default value. Always inserts a speculation barrier after the check.
    pub fn safe_array_access<T: Copy + Default>(arr: &[T], index: usize) -> T {
        if index < arr.len() {
            Self::speculation_barrier();
            arr[index]
        } else {
            Self::speculation_barrier();
            T::default()
        }
    }
}

/// KPTI (Kernel Page Table Isolation) support for Meltdown mitigation.
///
/// On x86_64, this manages separate kernel and user page tables so that
/// kernel memory is not mapped in user-space page tables.
pub struct Kpti {
    /// Whether KPTI is enabled
    enabled: bool,
    /// Address of user page table (CR3 value for user mode)
    user_cr3: AtomicU64,
    /// Address of kernel page table (CR3 value for kernel mode)
    kernel_cr3: AtomicU64,
}

impl Kpti {
    pub fn new() -> Self {
        Self {
            // KPTI is only relevant on x86_64
            enabled: cfg!(target_arch = "x86_64"),
            user_cr3: AtomicU64::new(0),
            kernel_cr3: AtomicU64::new(0),
        }
    }

    /// Set page table addresses for KPTI.
    pub fn set_page_tables(&self, kernel_cr3: u64, user_cr3: u64) {
        self.kernel_cr3.store(kernel_cr3, Ordering::SeqCst);
        self.user_cr3.store(user_cr3, Ordering::SeqCst);
    }

    /// Check if KPTI is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the kernel page table address.
    pub fn kernel_cr3(&self) -> u64 {
        self.kernel_cr3.load(Ordering::SeqCst)
    }

    /// Get the user page table address.
    pub fn user_cr3(&self) -> u64 {
        self.user_cr3.load(Ordering::SeqCst)
    }

    /// Switch to kernel page table (called on syscall entry / interrupt).
    #[cfg(target_arch = "x86_64")]
    pub fn switch_to_kernel(&self) {
        if !self.enabled {
            return;
        }
        let cr3 = self.kernel_cr3.load(Ordering::SeqCst);
        if cr3 != 0 {
            // SAFETY: Writing CR3 switches the page table root. cr3 was
            // previously set via set_kernel_cr3 and points to a valid PML4.
            unsafe {
                core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
            }
        }
    }

    /// Switch to user page table (called on syscall exit / iret).
    #[cfg(target_arch = "x86_64")]
    pub fn switch_to_user(&self) {
        if !self.enabled {
            return;
        }
        let cr3 = self.user_cr3.load(Ordering::SeqCst);
        if cr3 != 0 {
            // SAFETY: Writing CR3 switches the page table root. cr3 was
            // previously set via set_user_cr3 and points to a valid PML4.
            unsafe {
                core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
            }
        }
    }
}

impl Default for Kpti {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory protection manager
pub struct MemoryProtection {
    aslr: Aslr,
    stack_canaries_enabled: bool,
    guard_pages_enabled: bool,
    dep_enabled: bool, // Data Execution Prevention
    wx_policy: WxPolicy,
    dep_enforcement: DepEnforcement,
    kpti: Kpti,
}

impl MemoryProtection {
    /// Create new memory protection manager
    pub fn new() -> Result<Self, KernelError> {
        Ok(Self {
            aslr: Aslr::new()?,
            stack_canaries_enabled: true,
            guard_pages_enabled: true,
            dep_enabled: true,
            wx_policy: WxPolicy::new(),
            dep_enforcement: DepEnforcement::new(),
            kpti: Kpti::new(),
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

    /// Get W^X policy reference
    pub fn wx_policy(&self) -> &WxPolicy {
        &self.wx_policy
    }

    /// Get DEP enforcement reference
    pub fn dep_enforcement(&self) -> &DepEnforcement {
        &self.dep_enforcement
    }

    /// Get KPTI reference
    pub fn kpti(&self) -> &Kpti {
        &self.kpti
    }
}

impl Default for MemoryProtection {
    fn default() -> Self {
        Self::new().expect("Failed to create MemoryProtection")
    }
}

/// Global memory protection instance
static MEMORY_PROTECTION: OnceLock<MemoryProtection> = OnceLock::new();

/// Initialize memory protection
pub fn init() -> Result<(), KernelError> {
    MEMORY_PROTECTION
        .set(MemoryProtection::new()?)
        .map_err(|_| KernelError::AlreadyExists {
            resource: "memory_protection",
            id: 0,
        })?;

    crate::println!("[MEMORY-PROTECTION] ASLR, stack canaries, and guard pages enabled");
    Ok(())
}

/// Get global memory protection instance
pub fn get_memory_protection() -> &'static MemoryProtection {
    MEMORY_PROTECTION
        .get()
        .expect("Memory protection not initialized")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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

    #[test]
    fn test_stack_canary() {
        let canary = StackCanary::new();
        let value = canary.value();

        assert!(canary.verify(value));
        assert!(!canary.verify(value ^ 1));
    }
}
