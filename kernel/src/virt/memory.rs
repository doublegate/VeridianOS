//! Extended Page Tables (EPT) for guest physical-to-host physical address
//! translation

#[cfg(feature = "alloc")]
extern crate alloc;

use super::VmError;

const EPT_ENTRIES_PER_TABLE: usize = 512;
const PAGE_SIZE: u64 = 4096;
#[cfg(target_arch = "x86_64")]
const INDEX_MASK: u64 = 0x1FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EptPermissions {
    bits: u8,
}

impl EptPermissions {
    pub const READ: Self = Self { bits: 1 };
    pub const WRITE: Self = Self { bits: 2 };
    pub const EXECUTE: Self = Self { bits: 4 };
    pub const READ_WRITE: Self = Self { bits: 3 };
    pub const ALL: Self = Self { bits: 7 };
    pub const NONE: Self = Self { bits: 0 };

    pub fn read(self) -> bool {
        self.bits & 1 != 0
    }
    pub fn write(self) -> bool {
        self.bits & 2 != 0
    }
    pub fn execute(self) -> bool {
        self.bits & 4 != 0
    }
    pub fn as_bits(self) -> u64 {
        self.bits as u64
    }
    pub fn from_bits(bits: u8) -> Self {
        Self { bits: bits & 0x7 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EptMemoryType {
    Uncacheable = 0,
    WriteCombining = 1,
    WriteThrough = 4,
    WriteProtected = 5,
    WriteBack = 6,
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct EptEntry(u64);

impl EptEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub fn new_table(table_phys: u64, perms: EptPermissions) -> Self {
        Self((table_phys & 0x000F_FFFF_FFFF_F000) | perms.as_bits())
    }

    pub fn new_page(host_phys: u64, perms: EptPermissions, mem_type: EptMemoryType) -> Self {
        Self(
            (host_phys & 0x000F_FFFF_FFFF_F000)
                | ((mem_type as u64) << 3)
                | (1 << 6)
                | perms.as_bits(),
        )
    }

    pub fn is_present(self) -> bool {
        self.0 & 0x7 != 0
    }
    pub fn address(self) -> u64 {
        self.0 & 0x000F_FFFF_FFFF_F000
    }
    pub fn permissions(self) -> EptPermissions {
        EptPermissions::from_bits((self.0 & 0x7) as u8)
    }
    pub fn raw(self) -> u64 {
        self.0
    }
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl core::fmt::Debug for EptEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "EptEntry(0x{:016x} addr=0x{:x} R={} W={} X={})",
            self.0,
            self.address(),
            u8::from(self.permissions().read()),
            u8::from(self.permissions().write()),
            u8::from(self.permissions().execute())
        )
    }
}

#[repr(C, align(4096))]
pub struct EptTable {
    entries: [EptEntry; EPT_ENTRIES_PER_TABLE],
}

impl EptTable {
    pub const fn new() -> Self {
        Self {
            entries: [EptEntry::empty(); EPT_ENTRIES_PER_TABLE],
        }
    }
    pub fn entry(&self, index: usize) -> &EptEntry {
        &self.entries[index]
    }
    pub fn entry_mut(&mut self, index: usize) -> &mut EptEntry {
        &mut self.entries[index]
    }
}

impl Default for EptTable {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EptManager {
    pml4_frame: crate::mm::FrameNumber,
    mapped_pages: u64,
}

impl EptManager {
    #[cfg(target_arch = "x86_64")]
    pub fn new() -> Result<Self, VmError> {
        use crate::mm::frame_allocator::FRAME_ALLOCATOR;
        let frame = {
            let allocator = FRAME_ALLOCATOR.lock();
            allocator
                .allocate_frames(1, None)
                .map_err(|_| VmError::EptMappingFailed)?
        };
        let phys = frame.as_u64() * crate::mm::FRAME_SIZE as u64;
        let virt = crate::mm::phys_to_virt_addr(phys);
        // SAFETY: Exclusively owned frame.
        unsafe {
            core::ptr::write_bytes(virt as *mut u8, 0, crate::mm::FRAME_SIZE);
        }
        Ok(Self {
            pml4_frame: frame,
            mapped_pages: 0,
        })
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn new() -> Result<Self, VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn map_page(
        &mut self,
        guest_phys: u64,
        host_phys: u64,
        perms: EptPermissions,
    ) -> Result<(), VmError> {
        self.map_page_with_type(guest_phys, host_phys, perms, EptMemoryType::WriteBack)
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn map_page(&mut self, _gp: u64, _hp: u64, _p: EptPermissions) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn map_page_with_type(
        &mut self,
        guest_phys: u64,
        host_phys: u64,
        perms: EptPermissions,
        mem_type: EptMemoryType,
    ) -> Result<(), VmError> {
        use crate::mm::frame_allocator::FRAME_ALLOCATOR;
        let pml4_idx = ((guest_phys >> 39) & INDEX_MASK) as usize;
        let pdpt_idx = ((guest_phys >> 30) & INDEX_MASK) as usize;
        let pd_idx = ((guest_phys >> 21) & INDEX_MASK) as usize;
        let pt_idx = ((guest_phys >> 12) & INDEX_MASK) as usize;

        let pml4_phys = self.pml4_frame.as_u64() * crate::mm::FRAME_SIZE as u64;
        let pml4_virt = crate::mm::phys_to_virt_addr(pml4_phys);
        // SAFETY: We own the PML4 frame.
        let pml4_table = unsafe { &mut *(pml4_virt as *mut EptTable) };

        if !pml4_table.entry(pml4_idx).is_present() {
            let f = {
                let a = FRAME_ALLOCATOR.lock();
                a.allocate_frames(1, None)
                    .map_err(|_| VmError::EptMappingFailed)?
            };
            let p = f.as_u64() * crate::mm::FRAME_SIZE as u64;
            let v = crate::mm::phys_to_virt_addr(p);
            unsafe {
                core::ptr::write_bytes(v as *mut u8, 0, crate::mm::FRAME_SIZE);
            }
            *pml4_table.entry_mut(pml4_idx) = EptEntry::new_table(p, EptPermissions::ALL);
        }

        let pdpt_phys = pml4_table.entry(pml4_idx).address();
        let pdpt_table =
            unsafe { &mut *(crate::mm::phys_to_virt_addr(pdpt_phys) as *mut EptTable) };

        if !pdpt_table.entry(pdpt_idx).is_present() {
            let f = {
                let a = FRAME_ALLOCATOR.lock();
                a.allocate_frames(1, None)
                    .map_err(|_| VmError::EptMappingFailed)?
            };
            let p = f.as_u64() * crate::mm::FRAME_SIZE as u64;
            let v = crate::mm::phys_to_virt_addr(p);
            unsafe {
                core::ptr::write_bytes(v as *mut u8, 0, crate::mm::FRAME_SIZE);
            }
            *pdpt_table.entry_mut(pdpt_idx) = EptEntry::new_table(p, EptPermissions::ALL);
        }

        let pd_phys = pdpt_table.entry(pdpt_idx).address();
        let pd_table = unsafe { &mut *(crate::mm::phys_to_virt_addr(pd_phys) as *mut EptTable) };

        if !pd_table.entry(pd_idx).is_present() {
            let f = {
                let a = FRAME_ALLOCATOR.lock();
                a.allocate_frames(1, None)
                    .map_err(|_| VmError::EptMappingFailed)?
            };
            let p = f.as_u64() * crate::mm::FRAME_SIZE as u64;
            let v = crate::mm::phys_to_virt_addr(p);
            unsafe {
                core::ptr::write_bytes(v as *mut u8, 0, crate::mm::FRAME_SIZE);
            }
            *pd_table.entry_mut(pd_idx) = EptEntry::new_table(p, EptPermissions::ALL);
        }

        let pt_phys = pd_table.entry(pd_idx).address();
        let pt_table = unsafe { &mut *(crate::mm::phys_to_virt_addr(pt_phys) as *mut EptTable) };
        *pt_table.entry_mut(pt_idx) = EptEntry::new_page(host_phys, perms, mem_type);
        self.mapped_pages += 1;
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn map_page_with_type(
        &mut self,
        _gp: u64,
        _hp: u64,
        _p: EptPermissions,
        _mt: EptMemoryType,
    ) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    #[cfg(target_arch = "x86_64")]
    pub fn unmap_page(&mut self, guest_phys: u64) -> Result<(), VmError> {
        let pml4_idx = ((guest_phys >> 39) & INDEX_MASK) as usize;
        let pdpt_idx = ((guest_phys >> 30) & INDEX_MASK) as usize;
        let pd_idx = ((guest_phys >> 21) & INDEX_MASK) as usize;
        let pt_idx = ((guest_phys >> 12) & INDEX_MASK) as usize;

        let pml4_phys = self.pml4_frame.as_u64() * crate::mm::FRAME_SIZE as u64;
        let pml4_table = unsafe { &*(crate::mm::phys_to_virt_addr(pml4_phys) as *const EptTable) };
        if !pml4_table.entry(pml4_idx).is_present() {
            return Ok(());
        }

        let pdpt_table = unsafe {
            &*(crate::mm::phys_to_virt_addr(pml4_table.entry(pml4_idx).address())
                as *const EptTable)
        };
        if !pdpt_table.entry(pdpt_idx).is_present() {
            return Ok(());
        }

        let pd_table = unsafe {
            &*(crate::mm::phys_to_virt_addr(pdpt_table.entry(pdpt_idx).address())
                as *const EptTable)
        };
        if !pd_table.entry(pd_idx).is_present() {
            return Ok(());
        }

        let pt_table = unsafe {
            &mut *(crate::mm::phys_to_virt_addr(pd_table.entry(pd_idx).address()) as *mut EptTable)
        };
        pt_table.entry_mut(pt_idx).clear();
        if self.mapped_pages > 0 {
            self.mapped_pages -= 1;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub fn unmap_page(&mut self, _gp: u64) -> Result<(), VmError> {
        Err(VmError::VmxNotSupported)
    }

    pub fn identity_map_range(
        &mut self,
        start: u64,
        end: u64,
        perms: EptPermissions,
    ) -> Result<(), VmError> {
        let mut addr = start & !0xFFF;
        while addr < end {
            self.map_page(addr, addr, perms)?;
            addr += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn eptp(&self) -> u64 {
        let pml4_phys = self.pml4_frame.as_u64() * crate::mm::FRAME_SIZE as u64;
        (pml4_phys & 0x000F_FFFF_FFFF_F000) | (3 << 3) | 6
    }

    pub fn mapped_page_count(&self) -> u64 {
        self.mapped_pages
    }
    pub fn pml4_physical_address(&self) -> u64 {
        self.pml4_frame.as_u64() * crate::mm::FRAME_SIZE as u64
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EptViolationInfo {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub ept_readable: bool,
    pub ept_writable: bool,
    pub ept_executable: bool,
    pub guest_physical_addr: u64,
    pub guest_linear_addr: Option<u64>,
}

impl EptViolationInfo {
    pub fn from_exit_qualification(qualification: u64, guest_phys: u64, guest_lin: u64) -> Self {
        Self {
            read: qualification & 1 != 0,
            write: (qualification >> 1) & 1 != 0,
            execute: (qualification >> 2) & 1 != 0,
            ept_readable: (qualification >> 3) & 1 != 0,
            ept_writable: (qualification >> 4) & 1 != 0,
            ept_executable: (qualification >> 5) & 1 != 0,
            guest_physical_addr: guest_phys,
            guest_linear_addr: if (qualification >> 7) & 1 != 0 {
                Some(guest_lin)
            } else {
                None
            },
        }
    }
}

pub fn handle_ept_violation(info: &EptViolationInfo) -> Result<(), VmError> {
    crate::println!(
        "  [ept] EPT violation at guest_phys=0x{:x} (R={} W={} X={})",
        info.guest_physical_addr,
        info.read as u8,
        info.write as u8,
        info.execute as u8
    );
    Err(VmError::EptMappingFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ept_permissions() {
        assert!(EptPermissions::READ.read());
        assert!(!EptPermissions::READ.write());
        assert!(EptPermissions::ALL.read());
        assert!(EptPermissions::ALL.write());
        assert!(EptPermissions::ALL.execute());
        assert_eq!(EptPermissions::ALL.as_bits(), 7);
        assert_eq!(EptPermissions::NONE.as_bits(), 0);
    }

    #[test]
    fn test_ept_entry_empty() {
        let e = EptEntry::empty();
        assert!(!e.is_present());
        assert_eq!(e.address(), 0);
    }

    #[test]
    fn test_ept_entry_table() {
        let e = EptEntry::new_table(0x1000_0000, EptPermissions::ALL);
        assert!(e.is_present());
        assert_eq!(e.address(), 0x1000_0000);
    }

    #[test]
    fn test_ept_entry_page() {
        let e = EptEntry::new_page(0x2000_0000, EptPermissions::READ, EptMemoryType::WriteBack);
        assert!(e.is_present());
        assert_eq!(e.address(), 0x2000_0000);
        assert!(e.permissions().read());
        assert!(!e.permissions().write());
    }

    #[test]
    fn test_ept_entry_clear() {
        let mut e = EptEntry::new_table(0x3000_0000, EptPermissions::ALL);
        e.clear();
        assert!(!e.is_present());
    }

    #[test]
    fn test_ept_violation_info() {
        let info = EptViolationInfo::from_exit_qualification(0x81, 0x1000, 0x7FFF_0000);
        assert!(info.read);
        assert!(!info.write);
        assert!(info.guest_linear_addr.is_some());
    }
}
