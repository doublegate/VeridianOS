//! Hot-plug support for CPUs, memory, and PCI devices
//!
//! Implements ACPI Generic Event Device (GED), CPU hot-plug with lifecycle
//! management, memory DIMM hot-add/remove, PCI SHPC and PCIe native hot-plug.
//!
//! Sprints W5-S9 (CPU + memory hot-plug), W5-S10 (PCI hot-plug).

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{collections::VecDeque, vec, vec::Vec};

use super::VmError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum CPUs for hot-plug
const MAX_HOTPLUG_CPUS: usize = 256;

/// Maximum memory DIMMs
const MAX_DIMMS: usize = 64;

/// Maximum PCI hot-plug slots
const MAX_PCI_SLOTS: usize = 32;

/// Maximum ACPI GED events in queue
const MAX_GED_EVENTS: usize = 64;

/// Bits per u64 in CPU bitmap
const BITS_PER_WORD: usize = 64;

// ---------------------------------------------------------------------------
// Hot-plug Type
// ---------------------------------------------------------------------------

/// Type of hot-plug event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugType {
    /// CPU hot-plug (add or remove)
    Cpu,
    /// Memory hot-plug (add or remove DIMM)
    Memory,
    /// PCI device hot-plug
    PciDevice,
}

// ---------------------------------------------------------------------------
// Hot-plug Event
// ---------------------------------------------------------------------------

/// A hot-plug event notification
#[derive(Debug, Clone)]
pub struct HotplugEvent {
    /// Type of device being hot-plugged
    pub event_type: HotplugType,
    /// Device-specific info (slot/cpu_id/dimm_id encoded as u64)
    pub device_info: u64,
    /// Timestamp (monotonic counter)
    pub timestamp: u64,
    /// Whether this is an add (true) or remove (false) event
    pub is_add: bool,
}

impl HotplugEvent {
    /// Create a new hot-plug event
    pub fn new(event_type: HotplugType, device_info: u64, timestamp: u64, is_add: bool) -> Self {
        Self {
            event_type,
            device_info,
            timestamp,
            is_add,
        }
    }

    /// Create a CPU add event
    pub fn cpu_add(cpu_id: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::Cpu, cpu_id as u64, timestamp, true)
    }

    /// Create a CPU remove event
    pub fn cpu_remove(cpu_id: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::Cpu, cpu_id as u64, timestamp, false)
    }

    /// Create a memory add event
    pub fn memory_add(dimm_slot: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::Memory, dimm_slot as u64, timestamp, true)
    }

    /// Create a memory remove event
    pub fn memory_remove(dimm_slot: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::Memory, dimm_slot as u64, timestamp, false)
    }

    /// Create a PCI device add event
    pub fn pci_add(slot_id: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::PciDevice, slot_id as u64, timestamp, true)
    }

    /// Create a PCI device remove event
    pub fn pci_remove(slot_id: u32, timestamp: u64) -> Self {
        Self::new(HotplugType::PciDevice, slot_id as u64, timestamp, false)
    }
}

// ---------------------------------------------------------------------------
// ACPI Generic Event Device (GED)
// ---------------------------------------------------------------------------

/// ACPI Generic Event Device for hot-plug notifications
#[cfg(feature = "alloc")]
pub struct AcpiGed {
    /// Pending events queue
    events: VecDeque<HotplugEvent>,
    /// Event counter for timestamps
    event_counter: u64,
    /// Whether the GED is enabled
    enabled: bool,
}

#[cfg(feature = "alloc")]
impl Default for AcpiGed {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl AcpiGed {
    /// Create a new ACPI GED
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
            event_counter: 0,
            enabled: true,
        }
    }

    /// Inject a hot-plug event
    pub(crate) fn inject_event(&mut self, mut event: HotplugEvent) -> Result<(), VmError> {
        if !self.enabled {
            return Err(VmError::DeviceError);
        }
        if self.events.len() >= MAX_GED_EVENTS {
            return Err(VmError::DeviceError);
        }
        self.event_counter = self.event_counter.saturating_add(1);
        event.timestamp = self.event_counter;
        self.events.push_back(event);
        Ok(())
    }

    /// Poll the next pending event
    pub(crate) fn poll_event(&mut self) -> Option<HotplugEvent> {
        self.events.pop_front()
    }

    /// Check if there are pending events
    pub(crate) fn has_pending_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// Get number of pending events
    pub(crate) fn pending_count(&self) -> usize {
        self.events.len()
    }

    /// Clear all pending events
    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }

    /// Enable or disable the GED
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if enabled
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }
}

// ---------------------------------------------------------------------------
// CPU Lifecycle State
// ---------------------------------------------------------------------------

/// CPU hot-plug lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuLifecycleState {
    /// CPU slot is empty (not allocated)
    #[default]
    Empty,
    /// CPU has been allocated (resources reserved)
    Allocated,
    /// CPU is being initialized (firmware/microcode)
    Initializing,
    /// CPU is online and running
    Online,
    /// CPU is being taken offline
    Removing,
    /// CPU has been removed
    Removed,
}

// ---------------------------------------------------------------------------
// CPU Hot-plug
// ---------------------------------------------------------------------------

/// CPU hot-plug manager
#[cfg(feature = "alloc")]
pub struct CpuHotplug {
    /// Maximum CPUs supported
    pub max_cpus: u32,
    /// Online CPU bitmap
    online_cpus: Vec<u64>,
    /// Per-CPU lifecycle state
    cpu_states: Vec<CpuLifecycleState>,
    /// Number of currently online CPUs
    online_count: u32,
    /// Boot CPU (cannot be removed)
    boot_cpu: u32,
}

#[cfg(feature = "alloc")]
impl CpuHotplug {
    /// Create a new CPU hot-plug manager
    pub fn new(max_cpus: u32, boot_cpu: u32) -> Self {
        let max = max_cpus.min(MAX_HOTPLUG_CPUS as u32);
        let bitmap_words = (max as usize).div_ceil(BITS_PER_WORD);
        let mut cpu_states = vec![CpuLifecycleState::Empty; max as usize];

        // Boot CPU is always online
        let mut online_cpus = vec![0u64; bitmap_words];
        if (boot_cpu as usize) < cpu_states.len() {
            cpu_states[boot_cpu as usize] = CpuLifecycleState::Online;
            let word = boot_cpu as usize / BITS_PER_WORD;
            let bit = boot_cpu as usize % BITS_PER_WORD;
            if word < online_cpus.len() {
                online_cpus[word] |= 1u64 << bit;
            }
        }

        Self {
            max_cpus: max,
            online_cpus,
            cpu_states,
            online_count: 1,
            boot_cpu,
        }
    }

    /// Add (hot-plug) a CPU
    pub(crate) fn add_cpu(&mut self, cpu_id: u32) -> Result<(), VmError> {
        if cpu_id >= self.max_cpus {
            return Err(VmError::InvalidVmState);
        }
        let idx = cpu_id as usize;
        if self.cpu_states[idx] != CpuLifecycleState::Empty
            && self.cpu_states[idx] != CpuLifecycleState::Removed
        {
            return Err(VmError::InvalidVmState);
        }

        // Transition: Empty/Removed -> Allocated -> Initializing -> Online
        self.cpu_states[idx] = CpuLifecycleState::Allocated;
        self.cpu_states[idx] = CpuLifecycleState::Initializing;
        // In a real implementation, firmware init happens here
        self.cpu_states[idx] = CpuLifecycleState::Online;

        let word = idx / BITS_PER_WORD;
        let bit = idx % BITS_PER_WORD;
        if word < self.online_cpus.len() {
            self.online_cpus[word] |= 1u64 << bit;
        }
        self.online_count = self.online_count.saturating_add(1);
        Ok(())
    }

    /// Remove (hot-unplug) a CPU
    pub(crate) fn remove_cpu(&mut self, cpu_id: u32) -> Result<(), VmError> {
        if cpu_id >= self.max_cpus {
            return Err(VmError::InvalidVmState);
        }
        if cpu_id == self.boot_cpu {
            return Err(VmError::InvalidVmState); // Cannot remove boot CPU
        }
        let idx = cpu_id as usize;
        if self.cpu_states[idx] != CpuLifecycleState::Online {
            return Err(VmError::InvalidVmState);
        }

        // Transition: Online -> Removing -> Removed
        self.cpu_states[idx] = CpuLifecycleState::Removing;
        // In a real implementation, drain tasks, send IPI, etc.
        self.cpu_states[idx] = CpuLifecycleState::Removed;

        let word = idx / BITS_PER_WORD;
        let bit = idx % BITS_PER_WORD;
        if word < self.online_cpus.len() {
            self.online_cpus[word] &= !(1u64 << bit);
        }
        self.online_count = self.online_count.saturating_sub(1);
        Ok(())
    }

    /// Check if a CPU is online
    pub(crate) fn is_online(&self, cpu_id: u32) -> bool {
        if cpu_id >= self.max_cpus {
            return false;
        }
        let word = cpu_id as usize / BITS_PER_WORD;
        let bit = cpu_id as usize % BITS_PER_WORD;
        if word < self.online_cpus.len() {
            self.online_cpus[word] & (1u64 << bit) != 0
        } else {
            false
        }
    }

    /// Get CPU lifecycle state
    pub(crate) fn cpu_state(&self, cpu_id: u32) -> CpuLifecycleState {
        if (cpu_id as usize) < self.cpu_states.len() {
            self.cpu_states[cpu_id as usize]
        } else {
            CpuLifecycleState::Empty
        }
    }

    /// Get number of online CPUs
    pub(crate) fn online_count(&self) -> u32 {
        self.online_count
    }

    /// Get maximum CPUs
    pub(crate) fn max_cpus(&self) -> u32 {
        self.max_cpus
    }

    /// Get list of online CPU IDs
    pub(crate) fn online_cpu_ids(&self) -> Vec<u32> {
        let mut ids = Vec::new();
        for (word_idx, &word) in self.online_cpus.iter().enumerate() {
            if word == 0 {
                continue;
            }
            for bit in 0..BITS_PER_WORD {
                if word & (1u64 << bit) != 0 {
                    let cpu_id = (word_idx * BITS_PER_WORD + bit) as u32;
                    if cpu_id < self.max_cpus {
                        ids.push(cpu_id);
                    }
                }
            }
        }
        ids
    }
}

// ---------------------------------------------------------------------------
// Memory DIMM
// ---------------------------------------------------------------------------

/// A memory DIMM slot for hot-plug
#[derive(Debug, Clone, Copy)]
pub struct MemoryDimm {
    /// Slot number
    pub slot: u32,
    /// DIMM size in MB
    pub size_mb: u32,
    /// Base physical address when online
    pub base_addr: u64,
    /// Whether this DIMM is online
    pub online: bool,
}

impl MemoryDimm {
    /// Create a new DIMM
    pub fn new(slot: u32, size_mb: u32, base_addr: u64) -> Self {
        Self {
            slot,
            size_mb,
            base_addr,
            online: false,
        }
    }

    /// Get DIMM size in bytes
    pub(crate) fn size_bytes(&self) -> u64 {
        self.size_mb as u64 * 1024 * 1024
    }

    /// Get end address (exclusive)
    pub(crate) fn end_addr(&self) -> u64 {
        self.base_addr + self.size_bytes()
    }
}

// ---------------------------------------------------------------------------
// Memory Hot-plug
// ---------------------------------------------------------------------------

/// Memory hot-plug manager
#[cfg(feature = "alloc")]
pub struct MemoryHotplug {
    /// Maximum DIMM slots
    pub max_dimms: u32,
    /// Installed DIMMs
    pub dimms: Vec<MemoryDimm>,
    /// Next available base address for hot-added memory
    next_base_addr: u64,
    /// Total online memory in MB
    total_online_mb: u64,
}

#[cfg(feature = "alloc")]
impl MemoryHotplug {
    /// Create a new memory hot-plug manager
    ///
    /// `initial_base` is the address above which hot-added DIMMs are placed.
    pub fn new(max_dimms: u32, initial_base: u64) -> Self {
        Self {
            max_dimms: max_dimms.min(MAX_DIMMS as u32),
            dimms: Vec::new(),
            next_base_addr: initial_base,
            total_online_mb: 0,
        }
    }

    /// Add (hot-plug) a memory DIMM
    pub(crate) fn add_dimm(&mut self, size_mb: u32) -> Result<u32, VmError> {
        if self.dimms.len() >= self.max_dimms as usize {
            return Err(VmError::GuestMemoryError);
        }
        if size_mb == 0 {
            return Err(VmError::GuestMemoryError);
        }

        let slot = self.dimms.len() as u32;
        let base_addr = self.next_base_addr;
        let size_bytes = size_mb as u64 * 1024 * 1024;

        let mut dimm = MemoryDimm::new(slot, size_mb, base_addr);
        dimm.online = true;
        self.dimms.push(dimm);

        self.next_base_addr = self.next_base_addr.saturating_add(size_bytes);
        self.total_online_mb = self.total_online_mb.saturating_add(size_mb as u64);

        // In a real implementation, this would:
        // 1. Map the new memory region in EPT
        // 2. Notify the guest via ACPI GED
        // 3. Guest OS then onlines the memory

        Ok(slot)
    }

    /// Remove (hot-unplug) a memory DIMM
    pub(crate) fn remove_dimm(&mut self, slot: u32) -> Result<(), VmError> {
        let dimm = self
            .dimms
            .iter_mut()
            .find(|d| d.slot == slot)
            .ok_or(VmError::GuestMemoryError)?;

        if !dimm.online {
            return Err(VmError::GuestMemoryError);
        }

        // In a real implementation:
        // 1. Notify guest to offline the memory
        // 2. Wait for guest acknowledgment
        // 3. Unmap from EPT

        self.total_online_mb = self.total_online_mb.saturating_sub(dimm.size_mb as u64);
        dimm.online = false;
        Ok(())
    }

    /// Get DIMM by slot
    pub(crate) fn dimm(&self, slot: u32) -> Option<&MemoryDimm> {
        self.dimms.iter().find(|d| d.slot == slot)
    }

    /// Get total online memory in MB
    pub(crate) fn total_online_mb(&self) -> u64 {
        self.total_online_mb
    }

    /// Get number of installed DIMMs
    pub(crate) fn dimm_count(&self) -> usize {
        self.dimms.len()
    }

    /// Get number of online DIMMs
    pub(crate) fn online_dimm_count(&self) -> usize {
        self.dimms.iter().filter(|d| d.online).count()
    }
}

// ---------------------------------------------------------------------------
// PCI Hot-plug Indicators
// ---------------------------------------------------------------------------

/// Power indicator state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerIndicator {
    /// Indicator off
    #[default]
    Off,
    /// Indicator on (steady)
    On,
    /// Indicator blinking
    Blink,
}

/// Attention indicator state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttentionIndicator {
    /// Indicator off
    #[default]
    Off,
    /// Indicator on (steady)
    On,
    /// Indicator blinking
    Blink,
}

// ---------------------------------------------------------------------------
// PCI Hot-plug Slot
// ---------------------------------------------------------------------------

/// A PCI hot-plug slot
#[derive(Debug, Clone, Copy, Default)]
pub struct PciHotplugSlot {
    /// Slot identifier
    pub slot_id: u32,
    /// Whether a device is present in the slot
    pub occupied: bool,
    /// PCI address of the device (if present)
    pub device_addr: u32, // BDF encoded
    /// Power indicator state
    pub power_indicator: PowerIndicator,
    /// Attention indicator state
    pub attention_indicator: AttentionIndicator,
    /// Whether the slot has power
    pub powered: bool,
    /// Whether surprise removal is supported
    pub surprise_removal_supported: bool,
}

impl PciHotplugSlot {
    /// Create a new empty slot
    pub fn new(slot_id: u32) -> Self {
        Self {
            slot_id,
            surprise_removal_supported: false,
            ..Default::default()
        }
    }

    /// Create a slot with surprise removal support
    pub fn with_surprise_removal(slot_id: u32) -> Self {
        Self {
            slot_id,
            surprise_removal_supported: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// SHPC (Standard Hot-Plug Controller)
// ---------------------------------------------------------------------------

/// Standard Hot-Plug Controller for conventional PCI hot-plug
#[cfg(feature = "alloc")]
pub struct ShpcController {
    /// Hot-plug slots
    pub slots: Vec<PciHotplugSlot>,
    /// Controller base address (MMIO)
    pub base_addr: u64,
    /// Whether the controller is enabled
    pub enabled: bool,
}

#[cfg(feature = "alloc")]
impl ShpcController {
    /// Create a new SHPC controller
    pub fn new(num_slots: u32, base_addr: u64) -> Self {
        let count = num_slots.min(MAX_PCI_SLOTS as u32);
        let mut slots = Vec::with_capacity(count as usize);
        for i in 0..count {
            slots.push(PciHotplugSlot::new(i));
        }
        Self {
            slots,
            base_addr,
            enabled: true,
        }
    }

    /// Power on a slot (insert device)
    pub(crate) fn slot_power_on(&mut self, slot_id: u32, device_bdf: u32) -> Result<(), VmError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.slot_id == slot_id)
            .ok_or(VmError::DeviceError)?;

        if slot.occupied {
            return Err(VmError::DeviceError);
        }

        slot.occupied = true;
        slot.device_addr = device_bdf;
        slot.powered = true;
        slot.power_indicator = PowerIndicator::On;
        Ok(())
    }

    /// Power off a slot (eject device)
    pub(crate) fn slot_power_off(&mut self, slot_id: u32) -> Result<(), VmError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.slot_id == slot_id)
            .ok_or(VmError::DeviceError)?;

        if !slot.occupied {
            return Err(VmError::DeviceError);
        }

        slot.powered = false;
        slot.power_indicator = PowerIndicator::Off;
        slot.occupied = false;
        slot.device_addr = 0;
        Ok(())
    }

    /// Handle surprise removal of a device
    pub(crate) fn surprise_removal(&mut self, slot_id: u32) -> Result<HotplugEvent, VmError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.slot_id == slot_id)
            .ok_or(VmError::DeviceError)?;

        if !slot.occupied {
            return Err(VmError::DeviceError);
        }
        if !slot.surprise_removal_supported {
            return Err(VmError::DeviceError);
        }

        let event = HotplugEvent::pci_remove(slot_id, 0);
        slot.occupied = false;
        slot.powered = false;
        slot.power_indicator = PowerIndicator::Off;
        slot.attention_indicator = AttentionIndicator::Blink;
        slot.device_addr = 0;

        Ok(event)
    }

    /// Get slot state
    pub(crate) fn slot(&self, slot_id: u32) -> Option<&PciHotplugSlot> {
        self.slots.iter().find(|s| s.slot_id == slot_id)
    }

    /// Get number of occupied slots
    pub(crate) fn occupied_count(&self) -> usize {
        self.slots.iter().filter(|s| s.occupied).count()
    }

    /// Get number of total slots
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }
}

// ---------------------------------------------------------------------------
// PCIe Native Hot-plug
// ---------------------------------------------------------------------------

/// Slot event type for PCIe native hot-plug
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotEvent {
    /// Attention button pressed
    AttentionButton,
    /// Power fault detected
    PowerFault,
    /// Presence detect changed
    PresenceChanged,
    /// Command completed
    CommandCompleted,
    /// Data link layer state changed
    DllStateChanged,
}

/// PCIe native hot-plug controller (per-slot)
#[derive(Debug, Clone, Copy, Default)]
pub struct PcieNativeHotplug {
    /// Slot ID
    pub slot_id: u32,
    /// Presence detect state
    pub presence_detect: bool,
    /// Power fault detected
    pub power_fault: bool,
    /// Attention button pressed
    pub attention_button: bool,
    /// Power controller enabled
    pub power_enabled: bool,
    /// Power indicator
    pub power_indicator: PowerIndicator,
    /// Attention indicator
    pub attention_indicator: AttentionIndicator,
    /// Slot capabilities
    pub surprise_supported: bool,
    /// Hot-plug capable
    pub hotplug_capable: bool,
    /// Data link layer active
    pub dll_active: bool,
}

impl PcieNativeHotplug {
    /// Create a new PCIe hot-plug controller for a slot
    pub fn new(slot_id: u32) -> Self {
        Self {
            slot_id,
            hotplug_capable: true,
            surprise_supported: true,
            ..Default::default()
        }
    }

    /// Handle a slot event
    pub(crate) fn handle_slot_event(&mut self, event: SlotEvent) -> Option<HotplugEvent> {
        match event {
            SlotEvent::AttentionButton => {
                self.attention_button = true;
                self.attention_indicator = AttentionIndicator::Blink;
                // Start 5-second attention button timer (simplified)
                if self.presence_detect {
                    // Button pressed on occupied slot -> request eject
                    Some(HotplugEvent::pci_remove(self.slot_id, 0))
                } else {
                    None
                }
            }
            SlotEvent::PowerFault => {
                self.power_fault = true;
                self.power_enabled = false;
                self.power_indicator = PowerIndicator::Blink;
                self.attention_indicator = AttentionIndicator::On;
                None
            }
            SlotEvent::PresenceChanged => {
                self.presence_detect = !self.presence_detect;
                if self.presence_detect {
                    // Device inserted
                    self.power_indicator = PowerIndicator::Blink;
                    Some(HotplugEvent::pci_add(self.slot_id, 0))
                } else {
                    // Device removed
                    self.power_indicator = PowerIndicator::Off;
                    self.power_enabled = false;
                    Some(HotplugEvent::pci_remove(self.slot_id, 0))
                }
            }
            SlotEvent::CommandCompleted => {
                // Command completed, no further action needed
                None
            }
            SlotEvent::DllStateChanged => {
                self.dll_active = !self.dll_active;
                None
            }
        }
    }

    /// Enable power to the slot
    pub(crate) fn power_on(&mut self) {
        self.power_enabled = true;
        self.power_indicator = PowerIndicator::On;
        self.power_fault = false;
    }

    /// Disable power to the slot
    pub(crate) fn power_off(&mut self) {
        self.power_enabled = false;
        self.power_indicator = PowerIndicator::Off;
    }

    /// Check if a device is present
    pub(crate) fn is_present(&self) -> bool {
        self.presence_detect
    }

    /// Check if power is on
    pub(crate) fn is_powered(&self) -> bool {
        self.power_enabled
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotplug_event_cpu() {
        let event = HotplugEvent::cpu_add(4, 100);
        assert_eq!(event.event_type, HotplugType::Cpu);
        assert_eq!(event.device_info, 4);
        assert!(event.is_add);
    }

    #[test]
    fn test_hotplug_event_memory() {
        let event = HotplugEvent::memory_remove(2, 200);
        assert_eq!(event.event_type, HotplugType::Memory);
        assert!(!event.is_add);
    }

    #[test]
    fn test_hotplug_event_pci() {
        let event = HotplugEvent::pci_add(1, 300);
        assert_eq!(event.event_type, HotplugType::PciDevice);
        assert!(event.is_add);
    }

    #[test]
    fn test_acpi_ged_inject_poll() {
        let mut ged = AcpiGed::new();
        assert!(!ged.has_pending_events());

        ged.inject_event(HotplugEvent::cpu_add(1, 0)).unwrap();
        ged.inject_event(HotplugEvent::memory_add(0, 0)).unwrap();
        assert_eq!(ged.pending_count(), 2);

        let ev1 = ged.poll_event().unwrap();
        assert_eq!(ev1.event_type, HotplugType::Cpu);

        let ev2 = ged.poll_event().unwrap();
        assert_eq!(ev2.event_type, HotplugType::Memory);

        assert!(ged.poll_event().is_none());
    }

    #[test]
    fn test_acpi_ged_disabled() {
        let mut ged = AcpiGed::new();
        ged.set_enabled(false);
        assert!(ged.inject_event(HotplugEvent::cpu_add(0, 0)).is_err());
    }

    #[test]
    fn test_acpi_ged_clear() {
        let mut ged = AcpiGed::new();
        ged.inject_event(HotplugEvent::cpu_add(0, 0)).unwrap();
        ged.inject_event(HotplugEvent::cpu_add(1, 0)).unwrap();
        ged.clear();
        assert_eq!(ged.pending_count(), 0);
    }

    #[test]
    fn test_cpu_hotplug_new() {
        let hp = CpuHotplug::new(8, 0);
        assert_eq!(hp.online_count(), 1); // Boot CPU
        assert!(hp.is_online(0));
    }

    #[test]
    fn test_cpu_hotplug_add() {
        let mut hp = CpuHotplug::new(8, 0);
        hp.add_cpu(1).unwrap();
        hp.add_cpu(2).unwrap();
        assert_eq!(hp.online_count(), 3);
        assert!(hp.is_online(1));
        assert!(hp.is_online(2));
        assert_eq!(hp.cpu_state(1), CpuLifecycleState::Online);
    }

    #[test]
    fn test_cpu_hotplug_remove() {
        let mut hp = CpuHotplug::new(8, 0);
        hp.add_cpu(1).unwrap();
        hp.remove_cpu(1).unwrap();
        assert_eq!(hp.online_count(), 1);
        assert!(!hp.is_online(1));
        assert_eq!(hp.cpu_state(1), CpuLifecycleState::Removed);
    }

    #[test]
    fn test_cpu_hotplug_cannot_remove_boot() {
        let mut hp = CpuHotplug::new(8, 0);
        assert!(hp.remove_cpu(0).is_err());
    }

    #[test]
    fn test_cpu_hotplug_re_add() {
        let mut hp = CpuHotplug::new(8, 0);
        hp.add_cpu(3).unwrap();
        hp.remove_cpu(3).unwrap();
        hp.add_cpu(3).unwrap(); // Re-add after removal
        assert!(hp.is_online(3));
    }

    #[test]
    fn test_cpu_hotplug_online_ids() {
        let mut hp = CpuHotplug::new(8, 0);
        hp.add_cpu(2).unwrap();
        hp.add_cpu(5).unwrap();
        let ids = hp.online_cpu_ids();
        assert!(ids.contains(&0));
        assert!(ids.contains(&2));
        assert!(ids.contains(&5));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_memory_dimm() {
        let dimm = MemoryDimm::new(0, 1024, 0x1_0000_0000);
        assert_eq!(dimm.size_bytes(), 1024 * 1024 * 1024);
        assert_eq!(dimm.end_addr(), 0x1_0000_0000 + 1024 * 1024 * 1024);
    }

    #[test]
    fn test_memory_hotplug_add() {
        let mut hp = MemoryHotplug::new(8, 0x1_0000_0000);
        let slot = hp.add_dimm(512).unwrap();
        assert_eq!(slot, 0);
        assert_eq!(hp.total_online_mb(), 512);
        assert_eq!(hp.dimm_count(), 1);
    }

    #[test]
    fn test_memory_hotplug_remove() {
        let mut hp = MemoryHotplug::new(8, 0x1_0000_0000);
        hp.add_dimm(512).unwrap();
        hp.remove_dimm(0).unwrap();
        assert_eq!(hp.total_online_mb(), 0);
        assert_eq!(hp.online_dimm_count(), 0);
    }

    #[test]
    fn test_memory_hotplug_remove_offline() {
        let mut hp = MemoryHotplug::new(8, 0x1_0000_0000);
        hp.add_dimm(256).unwrap();
        hp.remove_dimm(0).unwrap();
        assert!(hp.remove_dimm(0).is_err()); // Already offline
    }

    #[test]
    fn test_shpc_controller_power_on() {
        let mut shpc = ShpcController::new(4, 0xFE00_0000);
        assert_eq!(shpc.slot_count(), 4);
        shpc.slot_power_on(0, 0x0018).unwrap();
        assert_eq!(shpc.occupied_count(), 1);
        let slot = shpc.slot(0).unwrap();
        assert!(slot.occupied);
        assert!(slot.powered);
    }

    #[test]
    fn test_shpc_controller_power_off() {
        let mut shpc = ShpcController::new(4, 0);
        shpc.slot_power_on(0, 0x0018).unwrap();
        shpc.slot_power_off(0).unwrap();
        assert_eq!(shpc.occupied_count(), 0);
    }

    #[test]
    fn test_shpc_surprise_removal() {
        let mut shpc = ShpcController::new(4, 0);
        shpc.slots[0].surprise_removal_supported = true;
        shpc.slot_power_on(0, 0x0018).unwrap();
        let event = shpc.surprise_removal(0).unwrap();
        assert_eq!(event.event_type, HotplugType::PciDevice);
        assert!(!event.is_add);
        assert_eq!(shpc.occupied_count(), 0);
    }

    #[test]
    fn test_shpc_surprise_not_supported() {
        let mut shpc = ShpcController::new(4, 0);
        shpc.slot_power_on(0, 0x0018).unwrap();
        assert!(shpc.surprise_removal(0).is_err());
    }

    #[test]
    fn test_pcie_native_presence_change_insert() {
        let mut hp = PcieNativeHotplug::new(0);
        assert!(!hp.is_present());
        let event = hp.handle_slot_event(SlotEvent::PresenceChanged).unwrap();
        assert_eq!(event.event_type, HotplugType::PciDevice);
        assert!(event.is_add);
        assert!(hp.is_present());
    }

    #[test]
    fn test_pcie_native_presence_change_remove() {
        let mut hp = PcieNativeHotplug::new(0);
        // Insert first
        hp.handle_slot_event(SlotEvent::PresenceChanged);
        hp.power_on();
        // Then remove
        let event = hp.handle_slot_event(SlotEvent::PresenceChanged).unwrap();
        assert!(!event.is_add);
        assert!(!hp.is_present());
        assert!(!hp.is_powered());
    }

    #[test]
    fn test_pcie_native_power_fault() {
        let mut hp = PcieNativeHotplug::new(0);
        hp.power_on();
        assert!(hp.is_powered());
        let event = hp.handle_slot_event(SlotEvent::PowerFault);
        assert!(event.is_none()); // Power fault doesn't generate hot-plug event
        assert!(!hp.is_powered());
        assert!(hp.power_fault);
    }

    #[test]
    fn test_pcie_native_attention_button_empty() {
        let mut hp = PcieNativeHotplug::new(0);
        let event = hp.handle_slot_event(SlotEvent::AttentionButton);
        assert!(event.is_none()); // No device, no eject
    }

    #[test]
    fn test_pcie_native_attention_button_occupied() {
        let mut hp = PcieNativeHotplug::new(0);
        hp.handle_slot_event(SlotEvent::PresenceChanged); // Insert
        let event = hp.handle_slot_event(SlotEvent::AttentionButton);
        assert!(event.is_some());
        assert!(!event.unwrap().is_add); // Request eject
    }
}
