//! Advanced Hypervisor Enhancements
//!
//! Implements 6 hypervisor features for Phase 7.5 Wave 7:
//! 1. Nested Virtualization -- L2 VMCS shadowing with field forwarding
//! 2. VirtIO Device Passthrough -- device assignment, MMIO mapping, interrupt
//!    forwarding
//! 3. Live Migration -- VMCS serialization, dirty page pre-copy, stop-and-copy
//! 4. Guest SMP -- multi-vCPU VMs with per-vCPU VMCS, IPI, SIPI emulation
//! 5. Virtual LAPIC -- full LAPIC register emulation with timer modes
//! 6. VM Snapshots -- complete state capture/restore with memory and device
//!    state

#[cfg(feature = "alloc")]
extern crate alloc;

use core::sync::atomic::{AtomicU64, Ordering};

pub mod lapic;
pub mod migration;
pub mod nested;
pub mod passthrough;
pub mod smp;
pub mod snapshot;

// Re-export everything from submodules
pub use self::{lapic::*, migration::*, nested::*, passthrough::*, smp::*, snapshot::*};

// ---------------------------------------------------------------------------
// Constants (shared across submodules)
// ---------------------------------------------------------------------------

/// Maximum vCPUs per VM
pub(crate) const MAX_VCPUS: usize = 16;

/// Maximum VMs tracked by the hypervisor
pub(crate) const _MAX_VMS: usize = 64;

/// LAPIC base MMIO address (standard x86 location)
pub(crate) const LAPIC_BASE_ADDR: u64 = 0xFEE0_0000;

/// LAPIC register space size (4 KiB)
pub(crate) const LAPIC_REGION_SIZE: u64 = 0x1000;

/// Page size constant
pub(crate) const PAGE_SIZE: u64 = 4096;

/// Number of VMCS field groups for serialization
pub(crate) const _VMCS_FIELD_GROUP_COUNT: usize = 7;

/// Maximum pages per pre-copy iteration
pub(crate) const PRECOPY_BATCH_SIZE: u64 = 256;

/// Dirty page bitmap granularity: bits per u64
pub(crate) const BITS_PER_U64: u64 = 64;

/// Snapshot magic number
pub(crate) const SNAPSHOT_MAGIC: u32 = 0x564D_534E; // "VMSN"

/// Snapshot format version
pub(crate) const SNAPSHOT_VERSION: u32 = 1;

/// Maximum passthrough devices per VM
pub(crate) const _MAX_PASSTHROUGH_DEVICES: usize = 32;

/// Maximum MSI-X vectors
pub(crate) const MAX_MSIX_VECTORS: usize = 64;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// General-purpose register state for a vCPU
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct GuestRegisters {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

// ---------------------------------------------------------------------------
// Hypervisor Manager (ties everything together)
// ---------------------------------------------------------------------------

/// Hypervisor statistics
#[derive(Debug, Default)]
pub struct HypervisorStats {
    pub total_vm_entries: AtomicU64,
    pub total_vm_exits: AtomicU64,
    pub total_ipis_sent: AtomicU64,
    pub total_lapic_timer_fires: AtomicU64,
    pub total_ept_violations: AtomicU64,
    pub total_snapshots_taken: AtomicU64,
    pub total_migrations_started: AtomicU64,
    pub total_migrations_completed: AtomicU64,
}

impl HypervisorStats {
    pub const fn new() -> Self {
        Self {
            total_vm_entries: AtomicU64::new(0),
            total_vm_exits: AtomicU64::new(0),
            total_ipis_sent: AtomicU64::new(0),
            total_lapic_timer_fires: AtomicU64::new(0),
            total_ept_violations: AtomicU64::new(0),
            total_snapshots_taken: AtomicU64::new(0),
            total_migrations_started: AtomicU64::new(0),
            total_migrations_completed: AtomicU64::new(0),
        }
    }

    pub fn record_vm_entry(&self) {
        self.total_vm_entries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_vm_exit(&self) {
        self.total_vm_exits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ipi(&self) {
        self.total_ipis_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_timer_fire(&self) {
        self.total_lapic_timer_fires.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_ept_violation(&self) {
        self.total_ept_violations.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_snapshot(&self) {
        self.total_snapshots_taken.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_migration_start(&self) {
        self.total_migrations_started
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_migration_complete(&self) {
        self.total_migrations_completed
            .fetch_add(1, Ordering::Relaxed);
    }
}

static HYPERVISOR_STATS: HypervisorStats = HypervisorStats::new();

/// Get global hypervisor statistics
pub fn get_stats() -> &'static HypervisorStats {
    &HYPERVISOR_STATS
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // --- Nested Virtualization Tests ---

    #[test]
    fn test_shadow_vmcs_read_write() {
        use crate::virt::vmx::VmcsFields;
        let mut shadow = ShadowVmcs::new();
        shadow.write_field(VmcsFields::GUEST_RIP, 0x1000);
        assert_eq!(shadow.read_field(VmcsFields::GUEST_RIP), Some(0x1000));
        assert_eq!(shadow.read_field(VmcsFields::GUEST_RSP), None);
        assert_eq!(shadow.field_count(), 1);
    }

    #[test]
    fn test_shadow_vmcs_activate_deactivate() {
        let mut shadow = ShadowVmcs::new();
        assert!(!shadow.is_active());
        shadow.activate(0x2000);
        assert!(shadow.is_active());
        assert_eq!(shadow.link_pointer(), 0x2000);
        shadow.deactivate();
        assert!(!shadow.is_active());
        assert_eq!(shadow.link_pointer(), 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn test_shadow_vmcs_clear() {
        let mut shadow = ShadowVmcs::new();
        shadow.write_field(0x100, 42);
        shadow.write_field(0x200, 84);
        shadow.activate(0x3000);
        shadow.clear();
        assert_eq!(shadow.field_count(), 0);
        assert!(!shadow.is_active());
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_passthrough() {
        use crate::virt::vmx::VmcsFields;
        let mut ctrl = NestedVirtController::new();
        assert!(ctrl
            .handle_l1_vmwrite(VmcsFields::GUEST_CR0, 0x80000011)
            .is_ok());
        assert_eq!(ctrl.handle_l1_vmread(VmcsFields::GUEST_CR0), Ok(0x80000011));
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_hidden_field() {
        use crate::virt::vmx::VmcsFields;
        let ctrl = NestedVirtController::new();
        assert_eq!(
            ctrl.handle_l1_vmread(VmcsFields::HOST_RIP),
            Err(crate::virt::VmError::VmcsFieldError)
        );
    }

    #[test]
    fn test_nested_controller_l1_vmwrite_readonly_field() {
        use crate::virt::vmx::VmcsFields;
        let mut ctrl = NestedVirtController::new();
        assert_eq!(
            ctrl.handle_l1_vmwrite(VmcsFields::VM_EXIT_REASON, 42),
            Err(crate::virt::VmError::VmcsFieldError)
        );
    }

    #[test]
    fn test_nested_enter_exit_l2() {
        use crate::virt::vmx::VmcsFields;
        let mut ctrl = NestedVirtController::new();
        ctrl.enable_nested_vmx();
        ctrl.handle_l1_vmwrite(VmcsFields::GUEST_RIP, 0x5000)
            .unwrap();

        let l1_regs = GuestRegisters {
            rax: 1,
            rip: 0x4000,
            ..Default::default()
        };
        assert!(ctrl.enter_l2(&l1_regs).is_ok());
        assert_eq!(ctrl.nesting_level(), NestingLevel::L2);

        let l1_restored = ctrl.exit_l2(NestedExitReason::Vmcall).unwrap();
        assert_eq!(l1_restored.rax, 1);
        assert_eq!(l1_restored.rip, 0x4000);
        assert_eq!(ctrl.nesting_level(), NestingLevel::L1);
    }

    #[test]
    fn test_nested_exit_l2_stores_reason() {
        use crate::virt::vmx::VmcsFields;
        let mut ctrl = NestedVirtController::new();
        ctrl.enable_nested_vmx();
        ctrl.handle_l1_vmwrite(VmcsFields::GUEST_RIP, 0x1000)
            .unwrap();
        ctrl.enter_l2(&GuestRegisters::default()).unwrap();
        ctrl.exit_l2(NestedExitReason::EptViolation).unwrap();
        // Exit reason 48 stored in shadow VMCS
        assert_eq!(
            ctrl.shadow_vmcs.read_field(VmcsFields::VM_EXIT_REASON),
            Some(48)
        );
    }

    #[test]
    fn test_nested_should_forward() {
        let ctrl = NestedVirtController::new();
        assert!(ctrl.should_forward_to_l1(NestedExitReason::VmxInstruction));
        assert!(ctrl.should_forward_to_l1(NestedExitReason::Vmcall));
        assert!(!ctrl.should_forward_to_l1(NestedExitReason::ExternalInterrupt));
    }

    // --- VirtIO Device Passthrough Tests ---

    #[test]
    fn test_passthrough_device_assign_unassign() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioNet,
            0x1AF4,
            0x1041,
            0x0000_0800,
        );
        assert!(!dev.is_assigned());
        assert!(dev.assign_to_vm(1).is_ok());
        assert!(dev.is_assigned());
        assert_eq!(dev.owner_vm_id, 1);
        // Double assign fails
        assert_eq!(dev.assign_to_vm(2), Err(crate::virt::VmError::DeviceError));
        dev.unassign();
        assert!(!dev.is_assigned());
    }

    #[test]
    fn test_passthrough_msix_remap() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioBlk,
            0x1AF4,
            0x1042,
            0x0000_1000,
        );
        dev.add_msix_remap(32, 64, 0);
        dev.add_msix_remap(33, 65, 1);
        assert_eq!(dev.msix_remap_count(), 2);
        assert_eq!(dev.remap_interrupt(32), Some((64, 0)));
        assert_eq!(dev.remap_interrupt(33), Some((65, 1)));
        assert_eq!(dev.remap_interrupt(99), None);
    }

    #[test]
    fn test_passthrough_mmio_region() {
        let mut dev = PassthroughDevice::new(
            PassthroughDeviceType::VirtioGpu,
            0x1AF4,
            0x1050,
            0x0000_1800,
        );
        dev.add_mmio_region(0xFE00_0000, 0xC000_0000, 0x1000);
        assert_eq!(dev.mmio_region_count(), 1);
        assert!(dev.mmio_regions[0].mapped);
    }

    #[test]
    fn test_pci_config_passthrough() {
        let mut pci = PciConfigPassthrough::new(0x1AF4, 0x1041, 0);
        assert_eq!(pci.read_config(0), 0xF4); // vendor low
        assert_eq!(pci.read_config(1), 0x1A); // vendor high
        assert_eq!(pci.read_config(2), 0x41); // device low
        assert_eq!(pci.read_config(3), 0x10); // device high
                                              // Write to command register (writable)
        pci.write_config(4, 0x07);
        assert_eq!(pci.read_config(4), 0x07);
        // Write to read-only area (should be masked)
        pci.write_config(0, 0xFF);
        assert_eq!(pci.read_config(0), 0xF4); // Unchanged
    }

    // --- Live Migration Tests ---

    #[test]
    fn test_dirty_page_bitmap() {
        let mut bm = DirtyPageBitmap::new(256);
        assert_eq!(bm.dirty_count(), 0);
        assert_eq!(bm.total_pages(), 256);

        bm.set_dirty(0);
        bm.set_dirty(63);
        bm.set_dirty(64);
        bm.set_dirty(255);
        assert_eq!(bm.dirty_count(), 4);
        assert!(bm.is_dirty(0));
        assert!(bm.is_dirty(63));
        assert!(bm.is_dirty(64));
        assert!(bm.is_dirty(255));
        assert!(!bm.is_dirty(1));

        bm.clear_dirty(63);
        assert_eq!(bm.dirty_count(), 3);
        assert!(!bm.is_dirty(63));
    }

    #[test]
    fn test_dirty_page_bitmap_idempotent() {
        let mut bm = DirtyPageBitmap::new(128);
        bm.set_dirty(10);
        bm.set_dirty(10); // Double set
        assert_eq!(bm.dirty_count(), 1);
        bm.clear_dirty(10);
        bm.clear_dirty(10); // Double clear
        assert_eq!(bm.dirty_count(), 0);
    }

    #[test]
    fn test_dirty_page_iterator() {
        let mut bm = DirtyPageBitmap::new(200);
        bm.set_dirty(5);
        bm.set_dirty(100);
        bm.set_dirty(199);
        let pages: Vec<u64> = bm.dirty_pages().collect();
        assert_eq!(pages, vec![5, 100, 199]);
    }

    #[test]
    fn test_dirty_page_clear_all() {
        let mut bm = DirtyPageBitmap::new(128);
        bm.set_dirty(0);
        bm.set_dirty(50);
        bm.set_dirty(127);
        let old_count = bm.clear_all();
        assert_eq!(old_count, 3);
        assert_eq!(bm.dirty_count(), 0);
    }

    #[test]
    fn test_migration_progress_bandwidth() {
        let mut progress = MigrationProgress::default();
        progress.total_bytes = 1_000_000;
        progress.update_bandwidth(500_000, 100); // 5000 bytes/ms
        assert_eq!(progress.bandwidth_bytes_per_ms, 5000);
        assert_eq!(progress.transferred_bytes, 500_000);
        progress.estimate_remaining();
        assert_eq!(progress.estimated_remaining_ms, 100); // 500000 / 5000
    }

    #[test]
    fn test_migration_progress_completion() {
        let mut progress = MigrationProgress::default();
        progress.total_bytes = 1000;
        progress.transferred_bytes = 750;
        assert_eq!(progress.completion_percent(), 75);
    }

    #[test]
    fn test_migration_convergence() {
        let mut progress = MigrationProgress::default();
        progress.previous_dirty_pages = 1000;
        progress.current_dirty_pages = 700;
        // 30% reduction, threshold 20% -> converged
        assert!(progress.has_converged(20));
        progress.current_dirty_pages = 900;
        // 10% reduction, threshold 20% -> not converged
        assert!(!progress.has_converged(20));
    }

    #[test]
    fn test_migration_state_machine() {
        let mut ctrl = MigrationController::new(1);
        assert_eq!(ctrl.state(), MigrationState::Idle);

        ctrl.begin_setup(100).unwrap();
        assert_eq!(ctrl.state(), MigrationState::Setup);

        ctrl.begin_precopy().unwrap();
        assert_eq!(ctrl.state(), MigrationState::PreCopy);

        let pages = ctrl.precopy_iteration().unwrap();
        assert!(!pages.is_empty());
    }

    #[test]
    fn test_serialized_vmcs() {
        use crate::virt::vmx::VmcsFields;
        let mut vmcs = SerializedVmcs::new();
        vmcs.add_field(VmcsFields::GUEST_RIP, 0x1000);
        vmcs.add_field(VmcsFields::GUEST_RSP, 0x7FF0);
        assert_eq!(vmcs.field_count(), 2);
        assert_eq!(vmcs.find_field(VmcsFields::GUEST_RIP), Some(0x1000));
        assert_eq!(vmcs.find_field(VmcsFields::GUEST_CR0), None);
    }

    // --- Guest SMP Tests ---

    #[test]
    fn test_smp_vm_creation() {
        let vm = SmpVm::new(1, 4).unwrap();
        assert_eq!(vm.vcpu_count(), 4);
        assert!(vm.vcpu(0).unwrap().is_bsp);
        assert!(!vm.vcpu(1).unwrap().is_bsp);
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::WaitingForSipi);
    }

    #[test]
    fn test_smp_vm_max_vcpu_limit() {
        assert!(SmpVm::new(1, 0).is_err());
        assert!(SmpVm::new(1, MAX_VCPUS + 1).is_err());
        assert!(SmpVm::new(1, MAX_VCPUS).is_ok());
    }

    #[test]
    fn test_vcpu_sipi_startup() {
        let mut vm = SmpVm::new(1, 2).unwrap();
        // AP starts in WaitingForSipi
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::WaitingForSipi);

        // BSP sends INIT + SIPI to AP
        vm.startup_ap(1, 0x10).unwrap(); // Entry at 0x10000

        let ap = vm.vcpu(1).unwrap();
        assert_eq!(ap.state, VcpuState::Running);
        assert_eq!(ap.registers.rip, 0x10000);
        assert_eq!(ap.sipi_vector, 0x10);
    }

    #[test]
    fn test_vcpu_ipi_delivery() {
        let mut vm = SmpVm::new(1, 4).unwrap();
        // Start all APs
        for i in 1..4 {
            vm.startup_ap(i, 0x20).unwrap();
        }

        // Send fixed IPI from vCPU 0 to vCPU 2
        vm.send_ipi(0, 2, IpiDeliveryMode::Fixed, 0x30).unwrap();
        assert_eq!(vm.vcpu(2).unwrap().pending_ipi_count(), 1);
        let ipi = vm.vcpu_mut(2).unwrap().pop_ipi().unwrap();
        assert_eq!(ipi.vector, 0x30);
        assert_eq!(ipi.source, 0);
    }

    #[test]
    fn test_vcpu_broadcast_ipi() {
        let mut vm = SmpVm::new(1, 4).unwrap();
        for i in 1..4 {
            vm.startup_ap(i, 0x20).unwrap();
        }

        // Broadcast from vCPU 0
        vm.send_ipi(0, 0xFF, IpiDeliveryMode::Fixed, 0x40).unwrap();
        // All except sender should receive
        assert_eq!(vm.vcpu(0).unwrap().pending_ipi_count(), 0);
        assert_eq!(vm.vcpu(1).unwrap().pending_ipi_count(), 1);
        assert_eq!(vm.vcpu(2).unwrap().pending_ipi_count(), 1);
        assert_eq!(vm.vcpu(3).unwrap().pending_ipi_count(), 1);
    }

    #[test]
    fn test_vcpu_halt_and_nmi_wake() {
        let mut vcpu = VirtualCpu::new(0, true);
        vcpu.state = VcpuState::Running;
        vcpu.halt();
        assert_eq!(vcpu.state, VcpuState::Halted);

        vcpu.deliver_ipi(IpiMessage {
            source: 1,
            destination: 0,
            delivery_mode: IpiDeliveryMode::Nmi,
            vector: 0,
            level: true,
            trigger_level: false,
        });
        assert_eq!(vcpu.state, VcpuState::Running);
    }

    #[test]
    fn test_vcpu_pause_resume() {
        let mut vm = SmpVm::new(1, 2).unwrap();
        vm.vcpu_mut(0).unwrap().state = VcpuState::Running;
        vm.startup_ap(1, 0x10).unwrap();
        assert_eq!(vm.running_vcpu_count(), 2);

        vm.pause_all();
        assert_eq!(vm.vcpu(0).unwrap().state, VcpuState::Paused);
        // AP was Running, now Paused
        assert_eq!(vm.vcpu(1).unwrap().state, VcpuState::Paused);

        vm.resume_all();
        assert_eq!(vm.running_vcpu_count(), 2);
    }

    // --- Virtual LAPIC Tests ---

    #[test]
    fn test_lapic_register_rw() {
        let mut lapic = VirtualLapic::new(0);
        // Write TPR
        lapic.write_register(LapicRegs::TPR, 0x20);
        assert_eq!(lapic.read_register(LapicRegs::TPR), 0x20);
        // Read version
        assert_eq!(lapic.read_register(LapicRegs::VERSION), 0x0005_0014);
        // Read ID
        assert_eq!(lapic.read_register(LapicRegs::ID), 0);
    }

    #[test]
    fn test_lapic_enable_via_svr() {
        let mut lapic = VirtualLapic::new(0);
        assert!(!lapic.is_enabled());
        lapic.write_register(LapicRegs::SVR, 0x1FF); // bit 8 set
        assert!(lapic.is_enabled());
    }

    #[test]
    fn test_lapic_accept_and_deliver_interrupt() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF); // Enable
        lapic.accept_interrupt(0x30);
        // IRR should have bit 0x30
        assert!(lapic.irr[1] & (1 << 16) != 0); // 0x30 = 48 = word 1, bit 16
        let vec = lapic.deliver_pending_interrupt();
        assert_eq!(vec, Some(0x30));
        // Now in ISR
        assert!(lapic.isr[1] & (1 << 16) != 0);
    }

    #[test]
    fn test_lapic_eoi() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.accept_interrupt(0x30);
        lapic.deliver_pending_interrupt();
        // ISR has 0x30
        lapic.write_register(LapicRegs::EOI, 0);
        // ISR should be cleared
        assert_eq!(lapic.isr[1] & (1 << 16), 0);
    }

    #[test]
    fn test_lapic_timer_oneshot() {
        let mut lapic = VirtualLapic::new(0);
        lapic.lvt_timer = LvtEntry { raw: 0x0000_0020 }; // vector 0x20, one-shot, unmasked
        lapic.write_register(LapicRegs::TIMER_INITIAL_COUNT, 100);
        assert!(!lapic.tick_timer(50));
        assert_eq!(lapic.timer_current_count, 50);
        assert!(lapic.tick_timer(60)); // Fires
        assert_eq!(lapic.timer_current_count, 0);
        assert!(!lapic.tick_timer(10)); // No more fires
    }

    #[test]
    fn test_lapic_timer_periodic() {
        let mut lapic = VirtualLapic::new(0);
        // Periodic mode: bits 17 = 1
        lapic.lvt_timer = LvtEntry { raw: 0x0002_0020 }; // vector 0x20, periodic
        lapic.write_register(LapicRegs::TIMER_INITIAL_COUNT, 100);
        assert!(lapic.tick_timer(110)); // Fires and reloads
        assert_eq!(lapic.timer_current_count, 100); // Reloaded
    }

    #[test]
    fn test_lapic_timer_divide_value() {
        let mut lapic = VirtualLapic::new(0);
        lapic.timer_divide_config = 0b0000; // divide by 2
        assert_eq!(lapic.timer_divide_value(), 2);
        lapic.timer_divide_config = 0b0011; // divide by 16
        assert_eq!(lapic.timer_divide_value(), 16);
        lapic.timer_divide_config = 0b1011; // divide by 1
        assert_eq!(lapic.timer_divide_value(), 1);
    }

    #[test]
    fn test_lapic_extract_ipi() {
        let mut lapic = VirtualLapic::new(0);
        lapic.icr_low = 0x0000_4030; // vector 0x30, INIT mode (5 << 8)
                                     // Wait, INIT = 5 << 8 = 0x500. Let's set that properly.
        lapic.icr_low = 0x0000_0530; // vector 0x30, INIT delivery mode (5 << 8)
        lapic.icr_high = 0x0200_0000; // dest APIC ID 2
        let ipi = lapic.extract_ipi();
        assert_eq!(ipi.vector, 0x30);
        assert_eq!(ipi.destination, 2);
        assert_eq!(ipi.delivery_mode, IpiDeliveryMode::Init);
    }

    #[test]
    fn test_lapic_priority() {
        let mut lapic = VirtualLapic::new(0);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.write_register(LapicRegs::TPR, 0x40); // Priority class 4

        // Interrupt with vector 0x30 (class 3) should NOT be delivered
        // because TPR class (4) > vector class (3)
        lapic.accept_interrupt(0x30);
        assert_eq!(lapic.deliver_pending_interrupt(), None);

        // Interrupt with vector 0x50 (class 5) should be delivered
        lapic.accept_interrupt(0x50);
        assert_eq!(lapic.deliver_pending_interrupt(), Some(0x50));
    }

    // --- Snapshot Tests ---

    #[test]
    fn test_snapshot_header_validation() {
        let mut header = SnapshotHeader::default();
        header.vm_id = 42;
        header.vcpu_count = 4;
        header.memory_pages = 1024;
        header.checksum = header.compute_checksum();
        assert!(header.is_valid());

        // Corrupt magic
        header.magic = 0;
        assert!(!header.is_valid());
    }

    #[test]
    fn test_snapshot_creation_and_finalize() {
        let mut snap = VmSnapshot::new(1, 2, 1024, 123456);
        snap.add_register_state(
            0,
            GuestRegisters {
                rip: 0x1000,
                ..Default::default()
            },
        );
        snap.add_register_state(
            1,
            GuestRegisters {
                rip: 0x2000,
                ..Default::default()
            },
        );
        snap.add_memory_page(0);
        snap.add_memory_page(100);
        snap.finalize();

        assert!(snap.validate());
        assert_eq!(snap.vcpu_state_count(), 2);
        assert_eq!(snap.memory_page_count(), 2);
        assert!(snap.header.total_size > 0);
    }

    #[test]
    fn test_snapshot_lapic_roundtrip() {
        let mut lapic = VirtualLapic::new(3);
        lapic.write_register(LapicRegs::SVR, 0x1FF);
        lapic.write_register(LapicRegs::TPR, 0x50);
        lapic.accept_interrupt(0x80);
        lapic.timer_initial_count = 5000;
        lapic.timer_current_count = 2500;

        let snap = LapicSnapshot::from_lapic(&lapic);
        let mut restored = VirtualLapic::new(0);
        snap.restore_to_lapic(&mut restored);

        assert_eq!(restored.id, 3);
        assert_eq!(restored.tpr, 0x50);
        assert!(restored.is_enabled());
        assert_eq!(restored.timer_initial_count, 5000);
        assert_eq!(restored.timer_current_count, 2500);
        // IRR should be preserved
        assert!(restored.irr[4] & 1 != 0); // vector 0x80 = word 4, bit 0
    }

    #[test]
    fn test_snapshot_device_state() {
        use alloc::string::String;
        let mut snap = VmSnapshot::new(1, 1, 256, 0);
        snap.add_device_state(String::from("uart0"), vec![0x60, 0x00, 0x00, 0x00]);
        assert_eq!(snap.device_state_count(), 1);
        assert_eq!(snap.device_states[0].name, "uart0");
        assert_eq!(snap.device_states[0].data.len(), 4);
    }

    // --- Hypervisor Stats Tests ---

    #[test]
    fn test_hypervisor_stats() {
        let stats = HypervisorStats::new();
        stats.record_vm_entry();
        stats.record_vm_entry();
        stats.record_vm_exit();
        stats.record_ipi();
        assert_eq!(stats.total_vm_entries.load(Ordering::Relaxed), 2);
        assert_eq!(stats.total_vm_exits.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_ipis_sent.load(Ordering::Relaxed), 1);
    }

    // --- LVT Entry Tests ---

    #[test]
    fn test_lvt_entry_fields() {
        let entry = LvtEntry { raw: 0x0002_0030 }; // periodic, vector 0x30
        assert_eq!(entry.vector(), 0x30);
        assert!(!entry.is_masked());
        assert_eq!(entry.timer_mode(), LapicTimerMode::Periodic);

        let masked = LvtEntry { raw: 0x0001_0020 }; // masked
        assert!(masked.is_masked());
    }

    #[test]
    fn test_nesting_level_default() {
        let level = NestingLevel::default();
        assert_eq!(level, NestingLevel::L0);
    }

    #[test]
    fn test_migration_state_default() {
        let state = MigrationState::default();
        assert_eq!(state, MigrationState::Idle);
    }
}
