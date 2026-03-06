//! VM Snapshots
//!
//! Complete state capture/restore with memory and device state.

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

use super::{
    lapic::{LvtEntry, VirtualLapic},
    migration::SerializedVmcs,
    GuestRegisters, PAGE_SIZE, SNAPSHOT_MAGIC, SNAPSHOT_VERSION,
};

// ---------------------------------------------------------------------------
// 6. VM Snapshots
// ---------------------------------------------------------------------------

/// Snapshot region type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SnapshotRegionType {
    /// VMCS state
    VmcsState = 1,
    /// General-purpose registers
    GeneralRegisters = 2,
    /// MSR values
    MsrValues = 3,
    /// Guest memory page
    MemoryPage = 4,
    /// Virtual device state
    DeviceState = 5,
    /// LAPIC state
    LapicState = 6,
    /// vCPU state
    VcpuState = 7,
}

/// Snapshot file header
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SnapshotHeader {
    /// Magic number (SNAPSHOT_MAGIC)
    pub magic: u32,
    /// Format version
    pub version: u32,
    /// VM ID
    pub vm_id: u64,
    /// Number of vCPUs
    pub vcpu_count: u32,
    /// Total memory pages
    pub memory_pages: u64,
    /// Number of regions in the snapshot
    pub region_count: u32,
    /// Total snapshot size in bytes
    pub total_size: u64,
    /// Timestamp (kernel ticks at snapshot time)
    pub timestamp: u64,
    /// Checksum (simple XOR-based)
    pub checksum: u64,
}

impl Default for SnapshotHeader {
    fn default() -> Self {
        Self {
            magic: SNAPSHOT_MAGIC,
            version: SNAPSHOT_VERSION,
            vm_id: 0,
            vcpu_count: 0,
            memory_pages: 0,
            region_count: 0,
            total_size: 0,
            timestamp: 0,
            checksum: 0,
        }
    }
}

impl SnapshotHeader {
    pub fn is_valid(&self) -> bool {
        self.magic == SNAPSHOT_MAGIC && self.version == SNAPSHOT_VERSION
    }

    /// Compute simple XOR checksum over header fields (excluding checksum
    /// itself)
    pub fn compute_checksum(&self) -> u64 {
        let mut ck: u64 = 0;
        ck ^= self.magic as u64;
        ck ^= self.version as u64;
        ck ^= self.vm_id;
        ck ^= self.vcpu_count as u64;
        ck ^= self.memory_pages;
        ck ^= self.region_count as u64;
        ck ^= self.total_size;
        ck ^= self.timestamp;
        ck
    }
}

/// Snapshot region descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SnapshotRegionDescriptor {
    /// Region type
    pub region_type: u32,
    /// vCPU index (for per-CPU state)
    pub vcpu_index: u32,
    /// Offset from start of snapshot data
    pub offset: u64,
    /// Size of this region in bytes
    pub size: u64,
}

/// MSR entry for snapshot
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MsrEntry {
    pub index: u32,
    pub _reserved: u32,
    pub value: u64,
}

/// Common MSR indices for snapshot
#[allow(unused)]
pub struct CommonMsrs;

#[allow(unused)]
impl CommonMsrs {
    pub const IA32_EFER: u32 = 0xC000_0080;
    pub const IA32_STAR: u32 = 0xC000_0081;
    pub const IA32_LSTAR: u32 = 0xC000_0082;
    pub const IA32_CSTAR: u32 = 0xC000_0083;
    pub const IA32_FMASK: u32 = 0xC000_0084;
    pub const IA32_FS_BASE: u32 = 0xC000_0100;
    pub const IA32_GS_BASE: u32 = 0xC000_0101;
    pub const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
    pub const IA32_TSC_AUX: u32 = 0xC000_0103;
    pub const IA32_SYSENTER_CS: u32 = 0x174;
    pub const IA32_SYSENTER_ESP: u32 = 0x175;
    pub const IA32_SYSENTER_EIP: u32 = 0x176;
    pub const IA32_PAT: u32 = 0x277;
    pub const IA32_DEBUGCTL: u32 = 0x1D9;
    pub const IA32_APIC_BASE: u32 = 0x1B;
}

/// Complete VM snapshot (in-memory representation)
#[cfg(feature = "alloc")]
pub struct VmSnapshot {
    /// Snapshot header
    pub header: SnapshotHeader,
    /// Region descriptors
    pub regions: Vec<SnapshotRegionDescriptor>,
    /// Serialized VMCS per vCPU
    pub vmcs_states: Vec<SerializedVmcs>,
    /// General register state per vCPU
    pub register_states: Vec<GuestRegisters>,
    /// MSR values per vCPU
    pub msr_states: Vec<Vec<MsrEntry>>,
    /// LAPIC state per vCPU (serialized as raw register values)
    pub lapic_states: Vec<LapicSnapshot>,
    /// Dirty page indices (pages included in snapshot)
    pub memory_page_indices: Vec<u64>,
    /// Device state blobs
    pub device_states: Vec<DeviceStateBlob>,
}

/// Serialized LAPIC state for snapshot
#[derive(Debug, Clone)]
pub struct LapicSnapshot {
    pub id: u32,
    pub tpr: u32,
    pub svr: u32,
    pub isr: [u32; 8],
    pub irr: [u32; 8],
    pub tmr: [u32; 8],
    pub lvt_timer_raw: u32,
    pub lvt_thermal_raw: u32,
    pub lvt_perfmon_raw: u32,
    pub lvt_lint0_raw: u32,
    pub lvt_lint1_raw: u32,
    pub lvt_error_raw: u32,
    pub timer_initial_count: u32,
    pub timer_current_count: u32,
    pub timer_divide_config: u32,
    pub tsc_deadline: u64,
    pub icr_low: u32,
    pub icr_high: u32,
    pub ldr: u32,
    pub dfr: u32,
    pub enabled: bool,
}

impl LapicSnapshot {
    pub fn from_lapic(lapic: &VirtualLapic) -> Self {
        Self {
            id: lapic.id,
            tpr: lapic.tpr,
            svr: lapic.svr,
            isr: lapic.isr,
            irr: lapic.irr,
            tmr: lapic.tmr,
            lvt_timer_raw: lapic.lvt_timer.raw,
            lvt_thermal_raw: lapic.lvt_thermal.raw,
            lvt_perfmon_raw: lapic.lvt_perfmon.raw,
            lvt_lint0_raw: lapic.lvt_lint0.raw,
            lvt_lint1_raw: lapic.lvt_lint1.raw,
            lvt_error_raw: lapic.lvt_error.raw,
            timer_initial_count: lapic.timer_initial_count,
            timer_current_count: lapic.timer_current_count,
            timer_divide_config: lapic.timer_divide_config,
            tsc_deadline: lapic.tsc_deadline,
            icr_low: lapic.icr_low,
            icr_high: lapic.icr_high,
            ldr: lapic.ldr,
            dfr: lapic.dfr,
            enabled: lapic.enabled,
        }
    }

    /// Restore LAPIC from snapshot
    pub fn restore_to_lapic(&self, lapic: &mut VirtualLapic) {
        lapic.id = self.id;
        lapic.tpr = self.tpr;
        lapic.svr = self.svr;
        lapic.isr = self.isr;
        lapic.irr = self.irr;
        lapic.tmr = self.tmr;
        lapic.lvt_timer = LvtEntry {
            raw: self.lvt_timer_raw,
        };
        lapic.lvt_thermal = LvtEntry {
            raw: self.lvt_thermal_raw,
        };
        lapic.lvt_perfmon = LvtEntry {
            raw: self.lvt_perfmon_raw,
        };
        lapic.lvt_lint0 = LvtEntry {
            raw: self.lvt_lint0_raw,
        };
        lapic.lvt_lint1 = LvtEntry {
            raw: self.lvt_lint1_raw,
        };
        lapic.lvt_error = LvtEntry {
            raw: self.lvt_error_raw,
        };
        lapic.timer_initial_count = self.timer_initial_count;
        lapic.timer_current_count = self.timer_current_count;
        lapic.timer_divide_config = self.timer_divide_config;
        lapic.tsc_deadline = self.tsc_deadline;
        lapic.icr_low = self.icr_low;
        lapic.icr_high = self.icr_high;
        lapic.ldr = self.ldr;
        lapic.dfr = self.dfr;
        lapic.enabled = self.enabled;
    }
}

/// Serialized device state blob
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct DeviceStateBlob {
    /// Device name
    pub name: String,
    /// Serialized state bytes
    pub data: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl VmSnapshot {
    /// Create a new empty snapshot
    pub fn new(vm_id: u64, vcpu_count: u32, memory_pages: u64, timestamp: u64) -> Self {
        let header = SnapshotHeader {
            vm_id,
            vcpu_count,
            memory_pages,
            timestamp,
            ..Default::default()
        };

        Self {
            header,
            regions: Vec::new(),
            vmcs_states: Vec::new(),
            register_states: Vec::new(),
            msr_states: Vec::new(),
            lapic_states: Vec::new(),
            memory_page_indices: Vec::new(),
            device_states: Vec::new(),
        }
    }

    /// Add register state for a vCPU
    pub fn add_register_state(&mut self, vcpu_idx: u32, regs: GuestRegisters) {
        self.register_states.push(regs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::GeneralRegisters as u32,
            vcpu_index: vcpu_idx,
            offset: 0, // Computed during serialization
            size: core::mem::size_of::<GuestRegisters>() as u64,
        });
        self.header.region_count += 1;
    }

    /// Add VMCS state for a vCPU
    pub fn add_vmcs_state(&mut self, vcpu_idx: u32, vmcs: SerializedVmcs) {
        let field_size = vmcs.field_count() as u64 * 12; // 4 bytes encoding + 8 bytes value
        self.vmcs_states.push(vmcs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::VmcsState as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size: field_size,
        });
        self.header.region_count += 1;
    }

    /// Add MSR state for a vCPU
    pub fn add_msr_state(&mut self, vcpu_idx: u32, msrs: Vec<MsrEntry>) {
        let size = msrs.len() as u64 * 16; // 16 bytes per MsrEntry
        self.msr_states.push(msrs);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::MsrValues as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size,
        });
        self.header.region_count += 1;
    }

    /// Add LAPIC state for a vCPU
    pub fn add_lapic_state(&mut self, vcpu_idx: u32, lapic: &VirtualLapic) {
        self.lapic_states.push(LapicSnapshot::from_lapic(lapic));
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::LapicState as u32,
            vcpu_index: vcpu_idx,
            offset: 0,
            size: core::mem::size_of::<LapicSnapshot>() as u64,
        });
        self.header.region_count += 1;
    }

    /// Add a memory page to the snapshot
    pub fn add_memory_page(&mut self, page_index: u64) {
        self.memory_page_indices.push(page_index);
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::MemoryPage as u32,
            vcpu_index: 0,
            offset: 0,
            size: PAGE_SIZE,
        });
        self.header.region_count += 1;
    }

    /// Add device state blob
    pub fn add_device_state(&mut self, name: String, data: Vec<u8>) {
        let size = data.len() as u64;
        self.device_states.push(DeviceStateBlob { name, data });
        self.regions.push(SnapshotRegionDescriptor {
            region_type: SnapshotRegionType::DeviceState as u32,
            vcpu_index: 0,
            offset: 0,
            size,
        });
        self.header.region_count += 1;
    }

    /// Finalize snapshot: compute total size and checksum
    pub fn finalize(&mut self) {
        let mut total: u64 = core::mem::size_of::<SnapshotHeader>() as u64;

        // Region descriptors
        total = total.saturating_add(
            (self.regions.len() as u64)
                .checked_mul(core::mem::size_of::<SnapshotRegionDescriptor>() as u64)
                .unwrap_or(0),
        );

        // Region data
        for region in &self.regions {
            total = total.saturating_add(region.size);
        }

        self.header.total_size = total;
        self.header.checksum = self.header.compute_checksum();
    }

    /// Validate snapshot header
    pub fn validate(&self) -> bool {
        self.header.is_valid() && self.header.checksum == self.header.compute_checksum()
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn memory_page_count(&self) -> usize {
        self.memory_page_indices.len()
    }

    pub fn vcpu_state_count(&self) -> usize {
        self.register_states.len()
    }

    pub fn device_state_count(&self) -> usize {
        self.device_states.len()
    }
}

/// Restore a VirtualLapic from a snapshot
#[cfg(feature = "alloc")]
pub fn restore_lapic_from_snapshot(snapshot: &VmSnapshot, vcpu_idx: usize) -> Option<VirtualLapic> {
    let lapic_snap = snapshot.lapic_states.get(vcpu_idx)?;
    let mut lapic = VirtualLapic::new(lapic_snap.id);
    lapic_snap.restore_to_lapic(&mut lapic);
    Some(lapic)
}
