//! Guest SMP Support
//!
//! Multi-vCPU VMs with per-vCPU VMCS, IPI, and SIPI emulation.

#[cfg(feature = "alloc")]
use alloc::{collections::BTreeMap, vec::Vec};

use super::{GuestRegisters, MAX_VCPUS};
use crate::virt::VmError;

// ---------------------------------------------------------------------------
// 4. Guest SMP Support
// ---------------------------------------------------------------------------

/// vCPU execution state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VcpuState {
    /// Not yet started
    #[default]
    Created,
    /// Running guest code
    Running,
    /// Halted (HLT instruction)
    Halted,
    /// Waiting for SIPI
    WaitingForSipi,
    /// Paused by hypervisor
    Paused,
    /// Stopped / destroyed
    Stopped,
}

/// Inter-Processor Interrupt delivery mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpiDeliveryMode {
    /// Fixed: deliver to specific vCPU
    Fixed,
    /// Lowest priority: deliver to lowest-priority vCPU
    LowestPriority,
    /// NMI: deliver NMI
    Nmi,
    /// INIT: send INIT signal
    Init,
    /// SIPI: Startup IPI (with vector for real-mode entry point)
    Sipi,
    /// ExtINT: external interrupt
    ExtInt,
}

/// IPI message between vCPUs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpiMessage {
    /// Source vCPU ID
    pub source: u8,
    /// Destination vCPU ID (0xFF = broadcast)
    pub destination: u8,
    /// Delivery mode
    pub delivery_mode: IpiDeliveryMode,
    /// Vector number (for Fixed/SIPI)
    pub vector: u8,
    /// Level (0 = deassert, 1 = assert)
    pub level: bool,
    /// Trigger mode (true = level, false = edge)
    pub trigger_level: bool,
}

/// Virtual CPU
#[cfg(feature = "alloc")]
pub struct VirtualCpu {
    /// vCPU ID (0 = BSP, 1+ = APs)
    pub id: u8,
    /// Current execution state
    pub state: VcpuState,
    /// General-purpose registers
    pub registers: GuestRegisters,
    /// LAPIC ID for this vCPU
    pub apic_id: u8,
    /// Pending IPIs (queue)
    pub pending_ipis: Vec<IpiMessage>,
    /// Whether this is the bootstrap processor
    pub is_bsp: bool,
    /// Host thread affinity (which host CPU to schedule on)
    pub host_affinity: Option<u32>,
    /// SIPI vector (real-mode entry = vector * 0x1000)
    pub sipi_vector: u8,
    /// VMCS field values for this vCPU
    pub vmcs_fields: BTreeMap<u32, u64>,
}

#[cfg(feature = "alloc")]
impl VirtualCpu {
    pub fn new(id: u8, is_bsp: bool) -> Self {
        let initial_state = if is_bsp {
            VcpuState::Created
        } else {
            VcpuState::WaitingForSipi
        };

        Self {
            id,
            state: initial_state,
            registers: GuestRegisters::default(),
            apic_id: id,
            pending_ipis: Vec::new(),
            is_bsp,
            host_affinity: None,
            sipi_vector: 0,
            vmcs_fields: BTreeMap::new(),
        }
    }

    /// Deliver an IPI to this vCPU
    pub fn deliver_ipi(&mut self, ipi: IpiMessage) {
        match ipi.delivery_mode {
            IpiDeliveryMode::Init => {
                // INIT resets vCPU to wait-for-SIPI state
                self.state = VcpuState::WaitingForSipi;
                self.registers = GuestRegisters::default();
            }
            IpiDeliveryMode::Sipi => {
                if self.state == VcpuState::WaitingForSipi {
                    // SIPI: entry point = vector * 0x1000 in real mode
                    self.sipi_vector = ipi.vector;
                    self.registers.rip = (ipi.vector as u64) << 12;
                    self.state = VcpuState::Running;
                }
                // Ignore SIPI if not in wait-for-SIPI state
            }
            IpiDeliveryMode::Nmi => {
                // Wake from HLT for NMI
                if self.state == VcpuState::Halted {
                    self.state = VcpuState::Running;
                }
                self.pending_ipis.push(ipi);
            }
            _ => {
                if self.state == VcpuState::Halted {
                    self.state = VcpuState::Running;
                }
                self.pending_ipis.push(ipi);
            }
        }
    }

    /// Pop next pending IPI
    pub fn pop_ipi(&mut self) -> Option<IpiMessage> {
        if self.pending_ipis.is_empty() {
            None
        } else {
            Some(self.pending_ipis.remove(0))
        }
    }

    /// Set host CPU affinity for scheduling
    pub fn set_affinity(&mut self, host_cpu: u32) {
        self.host_affinity = Some(host_cpu);
    }

    pub fn pending_ipi_count(&self) -> usize {
        self.pending_ipis.len()
    }

    /// Halt the vCPU (from HLT instruction)
    pub fn halt(&mut self) {
        self.state = VcpuState::Halted;
    }

    /// Pause the vCPU (hypervisor request)
    pub fn pause(&mut self) {
        if self.state == VcpuState::Running {
            self.state = VcpuState::Paused;
        }
    }

    /// Resume the vCPU
    pub fn resume(&mut self) {
        if self.state == VcpuState::Paused {
            self.state = VcpuState::Running;
        }
    }

    /// Stop the vCPU permanently
    pub fn stop(&mut self) {
        self.state = VcpuState::Stopped;
    }
}

/// Multi-vCPU VM
#[cfg(feature = "alloc")]
pub struct SmpVm {
    /// VM identifier
    pub vm_id: u64,
    /// Virtual CPUs
    pub vcpus: Vec<VirtualCpu>,
    /// Maximum vCPUs allowed
    pub max_vcpus: usize,
}

#[cfg(feature = "alloc")]
impl SmpVm {
    pub fn new(vm_id: u64, vcpu_count: usize) -> Result<Self, VmError> {
        if vcpu_count == 0 || vcpu_count > MAX_VCPUS {
            return Err(VmError::InvalidVmState);
        }

        let mut vcpus = Vec::with_capacity(vcpu_count);
        for i in 0..vcpu_count {
            vcpus.push(VirtualCpu::new(i as u8, i == 0));
        }

        Ok(Self {
            vm_id,
            vcpus,
            max_vcpus: vcpu_count,
        })
    }

    /// Send IPI from one vCPU to another
    pub fn send_ipi(
        &mut self,
        source: u8,
        dest: u8,
        mode: IpiDeliveryMode,
        vector: u8,
    ) -> Result<(), VmError> {
        if source as usize >= self.vcpus.len() {
            return Err(VmError::InvalidVmState);
        }

        let ipi = IpiMessage {
            source,
            destination: dest,
            delivery_mode: mode,
            vector,
            level: true,
            trigger_level: false,
        };

        if dest == 0xFF {
            // Broadcast (excluding self)
            for vcpu in &mut self.vcpus {
                if vcpu.id != source {
                    vcpu.deliver_ipi(ipi);
                }
            }
        } else {
            let target = self.vcpus.iter_mut().find(|v| v.apic_id == dest);
            if let Some(vcpu) = target {
                vcpu.deliver_ipi(ipi);
            } else {
                return Err(VmError::InvalidVmState);
            }
        }

        Ok(())
    }

    /// Emulate the AP startup sequence: BSP sends INIT then SIPI
    pub fn startup_ap(&mut self, ap_id: u8, sipi_vector: u8) -> Result<(), VmError> {
        // Send INIT
        self.send_ipi(0, ap_id, IpiDeliveryMode::Init, 0)?;
        // Send SIPI
        self.send_ipi(0, ap_id, IpiDeliveryMode::Sipi, sipi_vector)?;
        Ok(())
    }

    pub fn vcpu_count(&self) -> usize {
        self.vcpus.len()
    }

    pub fn running_vcpu_count(&self) -> usize {
        self.vcpus
            .iter()
            .filter(|v| v.state == VcpuState::Running)
            .count()
    }

    /// Get a vCPU by ID
    pub fn vcpu(&self, id: u8) -> Option<&VirtualCpu> {
        self.vcpus.iter().find(|v| v.id == id)
    }

    /// Get a mutable reference to a vCPU by ID
    pub fn vcpu_mut(&mut self, id: u8) -> Option<&mut VirtualCpu> {
        self.vcpus.iter_mut().find(|v| v.id == id)
    }

    /// Pause all vCPUs
    pub fn pause_all(&mut self) {
        for vcpu in &mut self.vcpus {
            vcpu.pause();
        }
    }

    /// Resume all vCPUs
    pub fn resume_all(&mut self) {
        for vcpu in &mut self.vcpus {
            vcpu.resume();
        }
    }
}
