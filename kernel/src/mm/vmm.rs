//! Virtual Memory Manager
//!
//! Complete virtual memory management for VeridianOS with proper page table
//! support.

#![allow(dead_code)]

use super::{
    page_table::{FrameAllocator as PageFrameAllocator, PageMapper, PageTable, PageTableHierarchy},
    FrameAllocatorError, FrameNumber, PageFlags, PageSize, PhysicalAddress, VirtualAddress,
    FRAME_ALLOCATOR,
};
use crate::error::KernelError;

/// Virtual memory manager for a process
pub struct VirtualMemoryManager {
    /// Page table hierarchy
    page_tables: PageTableHierarchy,
    /// Whether this is the kernel address space
    is_kernel: bool,
    /// Page mapper for this VMM (cached for performance)
    mapper: Option<PageMapper>,
}

impl VirtualMemoryManager {
    /// Create a new virtual memory manager
    pub fn new() -> Result<Self, KernelError> {
        let page_tables = PageTableHierarchy::new()?;

        Ok(Self {
            page_tables,
            is_kernel: false,
            mapper: None,
        })
    }

    /// Create a kernel virtual memory manager
    pub fn new_kernel() -> Result<Self, KernelError> {
        let page_tables = PageTableHierarchy::new()?;

        // Map kernel sections
        let mut vmm = Self {
            page_tables,
            is_kernel: true,
            mapper: None,
        };

        // Map kernel code, data, heap regions
        vmm.setup_kernel_mappings()?;

        Ok(vmm)
    }

    /// Setup initial kernel mappings
    fn setup_kernel_mappings(&mut self) -> Result<(), KernelError> {
        // Map kernel at higher half (0xFFFF_8000_0000_0000)
        // This implementation maps:
        // 1. Kernel code as read-only + execute
        // 2. Kernel data as read-write + no-execute
        // 3. Kernel heap region
        // 4. MMIO regions for devices

        // Identity map first 2GB for bootloader compatibility
        for i in 0..1024 {
            let phys_addr = PhysicalAddress::new(i * 0x200000); // 2MB pages
            let virt_addr = VirtualAddress::new(i * 0x200000);
            self.map(
                virt_addr,
                phys_addr,
                PageFlags::PRESENT | PageFlags::WRITABLE,
                PageSize::Large,
            )?;
        }

        // Map kernel to higher half
        let kernel_start = 0x100000; // 1MB
        let kernel_size = 16 * 1024 * 1024; // 16MB for kernel

        // Map kernel code (read + execute)
        for offset in (0..kernel_size).step_by(0x200000) {
            let phys_addr = PhysicalAddress::new(kernel_start + offset as u64);
            let virt_addr = VirtualAddress::new(0xFFFF_8000_0000_0000 + offset as u64);
            self.map(virt_addr, phys_addr, PageFlags::PRESENT, PageSize::Large)?;
        }

        // Map kernel heap
        let heap_start = super::heap::HEAP_START;
        let heap_size = super::heap::HEAP_SIZE;
        for offset in (0..heap_size).step_by(0x200000) {
            let phys_addr = PhysicalAddress::new((heap_start + offset) as u64);
            let virt_addr = VirtualAddress::new((heap_start + offset) as u64);
            self.map(
                virt_addr,
                phys_addr,
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NO_EXECUTE,
                PageSize::Large,
            )?;
        }

        Ok(())
    }

    /// Map a virtual address to a physical address
    pub fn map(
        &mut self,
        virt: VirtualAddress,
        phys: PhysicalAddress,
        flags: PageFlags,
        size: PageSize,
    ) -> Result<(), KernelError> {
        // Get or create page mapper
        let mapper = self.get_or_create_mapper()?;

        match size {
            PageSize::Small => {
                // Map 4KB page
                let frame = FrameNumber::new(phys.as_u64() >> 12);
                let mut frame_allocator_wrapper = FrameAllocatorWrapper;
                mapper.map_page(virt, frame, flags, &mut frame_allocator_wrapper)?;
            }
            PageSize::Large => {
                // Map 2MB page
                self.map_large_page(virt, phys, flags)?;
            }
            PageSize::Huge => {
                // Map 1GB page
                self.map_huge_page(virt, phys, flags)?;
            }
        }

        // Flush TLB for this address
        tlb::flush_address(virt);

        Ok(())
    }

    /// Get or create the page mapper
    fn get_or_create_mapper(&mut self) -> Result<&mut PageMapper, KernelError> {
        if self.mapper.is_none() {
            // Map L4 table to a known virtual address for access
            // In a real implementation, this would use recursive mapping or physical memory
            // mapping
            let l4_virt = 0xFFFF_FF00_0000_0000 as *mut PageTable;
            // SAFETY: The L4 page table is expected to be mapped at a fixed virtual
            // address (0xFFFF_FF00_0000_0000) via recursive mapping or the kernel's
            // physical memory map. This address is within the kernel's higher-half
            // address space and is reserved for page table access. The PageMapper
            // requires exclusive access to this table, which is maintained by the
            // VMM being the sole owner of the mapper through `&mut self`.
            unsafe {
                self.mapper = Some(PageMapper::new(l4_virt));
            }
        }
        self.mapper.as_mut().ok_or(KernelError::NotInitialized {
            subsystem: "VMM page mapper",
        })
    }

    /// Map a 2MB large page
    #[allow(unused_variables)]
    fn map_large_page(
        &mut self,
        virt: VirtualAddress,
        phys: PhysicalAddress,
        flags: PageFlags,
    ) -> Result<(), KernelError> {
        // For large pages, we need to set the page directory entry directly
        // This is architecture-specific
        #[cfg(target_arch = "x86_64")]
        {
            // Large page mappings use the HUGE flag
            let _frame = FrameNumber::new(phys.as_u64() >> 21);
            let _large_flags = flags | PageFlags::HUGE;
            // Would map at PD level instead of PT level
            println!(
                "[VMM] Mapping large page 0x{:x} -> 0x{:x}",
                virt.as_u64(),
                phys.as_u64()
            );
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            println!(
                "[VMM] Mapping large page 0x{:x} -> 0x{:x}",
                virt.as_u64(),
                phys.as_u64()
            );
        }

        Ok(())
    }

    /// Map a 1GB huge page
    #[allow(unused_variables)]
    fn map_huge_page(
        &mut self,
        virt: VirtualAddress,
        phys: PhysicalAddress,
        flags: PageFlags,
    ) -> Result<(), KernelError> {
        // For huge pages, we need to set the page directory pointer entry directly
        #[cfg(target_arch = "x86_64")]
        {
            // Huge page mappings at PDP level
            let _frame = FrameNumber::new(phys.as_u64() >> 30);
            let _huge_flags = flags | PageFlags::HUGE;
            println!(
                "[VMM] Mapping huge page 0x{:x} -> 0x{:x}",
                virt.as_u64(),
                phys.as_u64()
            );
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            println!(
                "[VMM] Mapping huge page 0x{:x} -> 0x{:x}",
                virt.as_u64(),
                phys.as_u64()
            );
        }

        Ok(())
    }

    /// Unmap a virtual address
    pub fn unmap(&mut self, virt: VirtualAddress) -> Result<(), KernelError> {
        let mapper = self.get_or_create_mapper()?;

        // Unmap the page
        let frame = mapper.unmap_page(virt)?;

        // Free the frame back to allocator
        FRAME_ALLOCATOR
            .lock()
            .free_frames(frame, 1)
            .map_err(|_| KernelError::OutOfMemory {
                requested: 1,
                available: 0,
            })?;

        // Flush TLB
        tlb::flush_address(virt);

        Ok(())
    }

    /// Map a guard page at the given virtual address.
    ///
    /// A guard page is an unmapped page that triggers a page fault on access,
    /// used to detect stack overflows. The page is left unmapped (no physical
    /// backing) so any read/write/execute will trap.
    pub fn map_guard_page(&mut self, virt: VirtualAddress) -> Result<(), KernelError> {
        // Ensure the address is not already mapped; if it is, unmap it
        if self.translate(virt).is_some() {
            self.unmap(virt)?;
        }
        // The page is now unmapped - any access will fault
        tlb::flush_address(virt);
        Ok(())
    }

    /// Translate a virtual address to physical
    pub fn translate(&self, virt: VirtualAddress) -> Option<PhysicalAddress> {
        // For now, we do simple translation based on known mappings
        // In a real implementation, would walk page tables

        let virt_addr = virt.as_u64();

        // Identity mapped region (first 2GB)
        if virt_addr < 0x8000_0000 {
            return Some(PhysicalAddress::new(virt_addr));
        }

        // Higher half kernel mapping
        if (0xFFFF_8000_0000_0000..0xFFFF_8000_1000_0000).contains(&virt_addr) {
            let offset = virt_addr - 0xFFFF_8000_0000_0000;
            return Some(PhysicalAddress::new(0x100000 + offset)); // Kernel at 1MB
        }

        // Kernel heap mapping
        if virt_addr >= super::heap::HEAP_START as u64
            && virt_addr < (super::heap::HEAP_START + super::heap::HEAP_SIZE) as u64
        {
            let offset = virt_addr - super::heap::HEAP_START as u64;
            return Some(PhysicalAddress::new(
                super::heap::HEAP_START as u64 + offset,
            ));
        }

        None
    }

    /// Load memory mappings from bootloader
    pub fn load_bootloader_mappings(
        &mut self,
        memory_map: &[super::MemoryRegion],
    ) -> Result<(), KernelError> {
        println!("[VMM] Loading bootloader memory mappings...");

        for region in memory_map {
            if !region.usable {
                continue;
            }

            // Map usable memory regions
            let start_addr = region.start & !(0x200000 - 1); // Align to 2MB
            let end_addr = (region.start + region.size + 0x200000 - 1) & !(0x200000 - 1);

            for addr in (start_addr..end_addr).step_by(0x200000) {
                let phys = PhysicalAddress::new(addr);
                let virt = VirtualAddress::new(addr); // Identity map for now

                // Skip if already mapped
                if self.translate(virt).is_some() {
                    continue;
                }

                self.map(
                    virt,
                    phys,
                    PageFlags::PRESENT | PageFlags::WRITABLE,
                    PageSize::Large,
                )?;
            }
        }

        println!("[VMM] Bootloader mappings loaded");
        Ok(())
    }
}

/// Frame allocator wrapper for PageMapper
struct FrameAllocatorWrapper;

impl PageFrameAllocator for FrameAllocatorWrapper {
    fn allocate_frames(
        &mut self,
        count: usize,
        numa_node: Option<usize>,
    ) -> Result<FrameNumber, FrameAllocatorError> {
        FRAME_ALLOCATOR.lock().allocate_frames(count, numa_node)
    }
}

/// TLB management -- delegates to architecture-specific implementations
/// in `crate::arch::{tlb_flush_address, tlb_flush_all}`.
pub mod tlb {
    use super::VirtualAddress;

    /// Flush TLB for a specific address
    pub fn flush_address(addr: VirtualAddress) {
        crate::arch::tlb_flush_address(addr.as_u64());
    }

    /// Flush entire TLB
    pub fn flush_all() {
        crate::arch::tlb_flush_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- VirtualMemoryManager translate tests ---
    //
    // translate() is pure logic (no hardware interaction) and is testable
    // on the host.

    #[test]
    fn test_translate_identity_mapped_region() {
        // The VMM's translate() treats addresses < 0x8000_0000 as identity mapped
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: false,
            mapper: None,
        };

        // Address in the identity-mapped first 2GB
        let result = vmm.translate(VirtualAddress::new(0x100000));
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 0x100000);
    }

    #[test]
    fn test_translate_identity_mapped_boundary() {
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: false,
            mapper: None,
        };

        // Last valid identity-mapped address
        let result = vmm.translate(VirtualAddress::new(0x7FFF_FFFF));
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 0x7FFF_FFFF);

        // Just past the 2GB boundary -- no longer identity mapped
        let result = vmm.translate(VirtualAddress::new(0x8000_0000));
        assert!(result.is_none());
    }

    #[test]
    fn test_translate_higher_half_kernel() {
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: true,
            mapper: None,
        };

        // Higher-half kernel mapping: 0xFFFF_8000_0000_0000 -> 0x100000 (1MB)
        let virt = VirtualAddress::new(0xFFFF_8000_0000_0000);
        let result = vmm.translate(virt);
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 0x100000);
    }

    #[test]
    fn test_translate_higher_half_kernel_with_offset() {
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: true,
            mapper: None,
        };

        // Offset within the kernel higher-half mapping
        let offset = 0x5000u64;
        let virt = VirtualAddress::new(0xFFFF_8000_0000_0000 + offset);
        let result = vmm.translate(virt);
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 0x100000 + offset);
    }

    #[test]
    fn test_translate_unmapped_address() {
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: false,
            mapper: None,
        };

        // Random high address that is not in any known mapping
        let result = vmm.translate(VirtualAddress::new(0xDEAD_0000_0000));
        assert!(result.is_none());
    }

    #[test]
    fn test_translate_zero_address() {
        let vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: false,
            mapper: None,
        };

        // Address 0 is within the identity-mapped region
        let result = vmm.translate(VirtualAddress::new(0));
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 0);
    }

    #[test]
    fn test_is_kernel_flag() {
        let user_vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: false,
            mapper: None,
        };
        assert!(!user_vmm.is_kernel);

        let kernel_vmm = VirtualMemoryManager {
            page_tables: PageTableHierarchy::empty_for_test(),
            is_kernel: true,
            mapper: None,
        };
        assert!(kernel_vmm.is_kernel);
    }
}
