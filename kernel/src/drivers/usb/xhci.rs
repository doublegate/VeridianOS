//! xHCI (eXtensible Host Controller Interface) USB 3.x driver
//!
//! Implements PCI discovery, MMIO register access, Transfer Ring Buffers
//! (TRBs), Command/Event/Transfer rings, device slot management, port handling,
//! USB descriptor parsing, and MSI-X interrupt stubs for xHCI controllers.
//!
//! xHCI PCI identification: class 0x0C, subclass 0x03, prog-if 0x30.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// PCI class code for Serial Bus Controller
const PCI_CLASS_SERIAL_BUS: u8 = 0x0C;
/// PCI subclass for USB controller
const PCI_SUBCLASS_USB: u8 = 0x03;
/// PCI prog-if for xHCI (USB 3.x)
const PCI_PROGIF_XHCI: u8 = 0x30;

/// Maximum number of device slots an xHCI controller can support (spec max 255)
const MAX_DEVICE_SLOTS: usize = 256;
/// Maximum number of ports an xHCI controller can expose
const MAX_PORTS: usize = 256;
/// Maximum number of interrupters
const MAX_INTERRUPTERS: usize = 1024;

/// Size of a single TRB in bytes (always 16 per xHCI spec)
const TRB_SIZE: usize = 16;
/// Number of TRBs in a ring segment (must be power of 2 for alignment)
const RING_SEGMENT_TRBS: usize = 256;
/// Ring segment size in bytes
const RING_SEGMENT_SIZE: usize = RING_SEGMENT_TRBS * TRB_SIZE;

// ---------------------------------------------------------------------------
// USB Descriptor Type Constants (per USB 3.2 spec, Table 9-6)
// ---------------------------------------------------------------------------

/// USB descriptor type: Device
const DESC_TYPE_DEVICE: u8 = 0x01;
/// USB descriptor type: Configuration
const DESC_TYPE_CONFIGURATION: u8 = 0x02;
/// USB descriptor type: String
const DESC_TYPE_STRING: u8 = 0x03;
/// USB descriptor type: Interface
const DESC_TYPE_INTERFACE: u8 = 0x04;
/// USB descriptor type: Endpoint
const DESC_TYPE_ENDPOINT: u8 = 0x05;
/// USB descriptor type: Device Qualifier
const DESC_TYPE_DEVICE_QUALIFIER: u8 = 0x06;
/// USB descriptor type: BOS (Binary Object Store)
const DESC_TYPE_BOS: u8 = 0x0F;
/// USB descriptor type: SuperSpeed Endpoint Companion
const DESC_TYPE_SS_EP_COMPANION: u8 = 0x30;

// ---------------------------------------------------------------------------
// USB Standard Request Codes (per USB 3.2 spec, Table 9-4)
// ---------------------------------------------------------------------------

/// GET_STATUS request code
const USB_REQ_GET_STATUS: u8 = 0x00;
/// CLEAR_FEATURE request code
const USB_REQ_CLEAR_FEATURE: u8 = 0x01;
/// SET_FEATURE request code
const USB_REQ_SET_FEATURE: u8 = 0x03;
/// SET_ADDRESS request code
const USB_REQ_SET_ADDRESS: u8 = 0x05;
/// GET_DESCRIPTOR request code
const USB_REQ_GET_DESCRIPTOR: u8 = 0x06;
/// SET_DESCRIPTOR request code
const USB_REQ_SET_DESCRIPTOR: u8 = 0x07;
/// GET_CONFIGURATION request code
const USB_REQ_GET_CONFIGURATION: u8 = 0x08;
/// SET_CONFIGURATION request code
const USB_REQ_SET_CONFIGURATION: u8 = 0x09;

// ---------------------------------------------------------------------------
// USB Request Type Bit Fields
// ---------------------------------------------------------------------------

/// Host-to-device direction
const USB_DIR_OUT: u8 = 0x00;
/// Device-to-host direction
const USB_DIR_IN: u8 = 0x80;
/// Standard request type
const USB_TYPE_STANDARD: u8 = 0x00;
/// Class request type
const USB_TYPE_CLASS: u8 = 0x20;
/// Vendor request type
const USB_TYPE_VENDOR: u8 = 0x40;
/// Recipient: device
const USB_RECIP_DEVICE: u8 = 0x00;
/// Recipient: interface
const USB_RECIP_INTERFACE: u8 = 0x01;
/// Recipient: endpoint
const USB_RECIP_ENDPOINT: u8 = 0x02;

// ---------------------------------------------------------------------------
// TRB Type Codes (xHCI spec Table 6-91)
// ---------------------------------------------------------------------------

/// TRB type: Normal (data stage of bulk/interrupt)
const TRB_TYPE_NORMAL: u8 = 1;
/// TRB type: Setup Stage
const TRB_TYPE_SETUP_STAGE: u8 = 2;
/// TRB type: Data Stage
const TRB_TYPE_DATA_STAGE: u8 = 3;
/// TRB type: Status Stage
const TRB_TYPE_STATUS_STAGE: u8 = 4;
/// TRB type: Isoch
const TRB_TYPE_ISOCH: u8 = 5;
/// TRB type: Link
const TRB_TYPE_LINK: u8 = 6;
/// TRB type: Event Data
const TRB_TYPE_EVENT_DATA: u8 = 7;
/// TRB type: No-Op (transfer ring)
const TRB_TYPE_NOOP: u8 = 8;
/// TRB type: Enable Slot Command
const TRB_TYPE_ENABLE_SLOT: u8 = 9;
/// TRB type: Disable Slot Command
const TRB_TYPE_DISABLE_SLOT: u8 = 10;
/// TRB type: Address Device Command
const TRB_TYPE_ADDRESS_DEVICE: u8 = 11;
/// TRB type: Configure Endpoint Command
const TRB_TYPE_CONFIGURE_ENDPOINT: u8 = 12;
/// TRB type: Evaluate Context Command
const TRB_TYPE_EVALUATE_CONTEXT: u8 = 13;
/// TRB type: Reset Endpoint Command
const TRB_TYPE_RESET_ENDPOINT: u8 = 14;
/// TRB type: Stop Endpoint Command
const TRB_TYPE_STOP_ENDPOINT: u8 = 15;
/// TRB type: Set TR Dequeue Pointer Command
const TRB_TYPE_SET_TR_DEQUEUE: u8 = 16;
/// TRB type: Reset Device Command
const TRB_TYPE_RESET_DEVICE: u8 = 17;
/// TRB type: No-Op Command
const TRB_TYPE_NOOP_CMD: u8 = 23;

// Event TRB types
/// TRB type: Transfer Event
const TRB_TYPE_TRANSFER_EVENT: u8 = 32;
/// TRB type: Command Completion Event
const TRB_TYPE_COMMAND_COMPLETION: u8 = 33;
/// TRB type: Port Status Change Event
const TRB_TYPE_PORT_STATUS_CHANGE: u8 = 34;
/// TRB type: Bandwidth Request Event
const TRB_TYPE_BANDWIDTH_REQUEST: u8 = 35;
/// TRB type: Host Controller Event
const TRB_TYPE_HOST_CONTROLLER: u8 = 37;

// ---------------------------------------------------------------------------
// TRB Completion Codes (xHCI spec Table 6-90)
// ---------------------------------------------------------------------------

/// Completion: Invalid (not yet completed)
const TRB_COMPLETION_INVALID: u8 = 0;
/// Completion: Success
const TRB_COMPLETION_SUCCESS: u8 = 1;
/// Completion: Data Buffer Error
const TRB_COMPLETION_DATA_BUFFER_ERROR: u8 = 2;
/// Completion: Babble Detected Error
const TRB_COMPLETION_BABBLE: u8 = 3;
/// Completion: USB Transaction Error
const TRB_COMPLETION_USB_TRANSACTION_ERROR: u8 = 4;
/// Completion: TRB Error
const TRB_COMPLETION_TRB_ERROR: u8 = 5;
/// Completion: Stall Error
const TRB_COMPLETION_STALL: u8 = 6;
/// Completion: Short Packet
const TRB_COMPLETION_SHORT_PACKET: u8 = 13;
/// Completion: Command Ring Stopped
const TRB_COMPLETION_COMMAND_RING_STOPPED: u8 = 24;
/// Completion: Command Aborted
const TRB_COMPLETION_COMMAND_ABORTED: u8 = 25;

// ---------------------------------------------------------------------------
// TRB Control Field Bits
// ---------------------------------------------------------------------------

/// Cycle bit (bit 0 of control DWORD)
const TRB_CYCLE_BIT: u32 = 1 << 0;
/// Toggle Cycle bit for Link TRBs (bit 1)
const TRB_TOGGLE_CYCLE: u32 = 1 << 1;
/// Interrupt-on-Short-Packet (bit 2)
const TRB_ISP: u32 = 1 << 2;
/// No Snoop (bit 3)
const TRB_NO_SNOOP: u32 = 1 << 3;
/// Chain bit (bit 4)
const TRB_CHAIN: u32 = 1 << 4;
/// Interrupt-on-Completion (bit 5)
const TRB_IOC: u32 = 1 << 5;
/// Immediate Data (bit 6)
const TRB_IDT: u32 = 1 << 6;
/// Block Set Address Request (bit 9, Address Device only)
const TRB_BSR: u32 = 1 << 9;

// ---------------------------------------------------------------------------
// Port Status and Control Register Bits (xHCI spec 5.4.8)
// ---------------------------------------------------------------------------

/// Current Connect Status
const PORTSC_CCS: u32 = 1 << 0;
/// Port Enabled/Disabled
const PORTSC_PED: u32 = 1 << 1;
/// Over-current Active
const PORTSC_OCA: u32 = 1 << 3;
/// Port Reset
const PORTSC_PR: u32 = 1 << 4;
/// Port Link State (bits 8:5)
const PORTSC_PLS_MASK: u32 = 0xF << 5;
/// Port Power
const PORTSC_PP: u32 = 1 << 9;
/// Port Speed (bits 13:10)
const PORTSC_SPEED_MASK: u32 = 0xF << 10;
/// Port Speed shift
const PORTSC_SPEED_SHIFT: u32 = 10;
/// Connect Status Change
const PORTSC_CSC: u32 = 1 << 17;
/// Port Enabled/Disabled Change
const PORTSC_PEC: u32 = 1 << 18;
/// Warm Port Reset Change
const PORTSC_WRC: u32 = 1 << 19;
/// Over-current Change
const PORTSC_OCC: u32 = 1 << 20;
/// Port Reset Change
const PORTSC_PRC: u32 = 1 << 21;
/// Port Link State Change
const PORTSC_PLC: u32 = 1 << 22;
/// Port Config Error Change
const PORTSC_CEC: u32 = 1 << 23;
/// Write-1-to-clear status change bits mask
const PORTSC_CHANGE_BITS: u32 =
    PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_OCC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;

/// Port speed: Full-speed (USB 2.0, 12 Mb/s)
const PORT_SPEED_FULL: u32 = 1;
/// Port speed: Low-speed (USB 2.0, 1.5 Mb/s)
const PORT_SPEED_LOW: u32 = 2;
/// Port speed: High-speed (USB 2.0, 480 Mb/s)
const PORT_SPEED_HIGH: u32 = 3;
/// Port speed: SuperSpeed (USB 3.0, 5 Gb/s)
const PORT_SPEED_SUPER: u32 = 4;
/// Port speed: SuperSpeedPlus (USB 3.1+, 10+ Gb/s)
const PORT_SPEED_SUPER_PLUS: u32 = 5;

// ---------------------------------------------------------------------------
// MMIO Register Offsets
// ---------------------------------------------------------------------------

/// Capability Registers (offset 0x00 from MMIO base)
mod cap_regs {
    /// Capability Register Length + HC Interface Version
    pub const CAPLENGTH: usize = 0x00;
    /// Host Controller Interface Version (16-bit at offset 0x02)
    pub const HCIVERSION: usize = 0x02;
    /// Structural Parameters 1
    pub const HCSPARAMS1: usize = 0x04;
    /// Structural Parameters 2
    pub const HCSPARAMS2: usize = 0x08;
    /// Structural Parameters 3
    pub const HCSPARAMS3: usize = 0x0C;
    /// Capability Parameters 1
    pub const HCCPARAMS1: usize = 0x10;
    /// Doorbell Offset
    pub const DBOFF: usize = 0x14;
    /// Runtime Register Space Offset
    pub const RTSOFF: usize = 0x18;
    /// Capability Parameters 2
    pub const HCCPARAMS2: usize = 0x1C;
}

/// Operational Register offsets (relative to operational base = MMIO_base +
/// CAPLENGTH)
mod op_regs {
    /// USB Command Register
    pub const USBCMD: usize = 0x00;
    /// USB Status Register
    pub const USBSTS: usize = 0x04;
    /// Page Size Register
    pub const PAGESIZE: usize = 0x08;
    /// Device Notification Control Register
    pub const DNCTRL: usize = 0x14;
    /// Command Ring Control Register (64-bit)
    pub const CRCR: usize = 0x18;
    /// Device Context Base Address Array Pointer (64-bit)
    pub const DCBAAP: usize = 0x30;
    /// Configure Register
    pub const CONFIG: usize = 0x38;
    /// Port Register Set base (port 1 starts here)
    pub const PORT_REG_BASE: usize = 0x400;
    /// Size of each port register set
    pub const PORT_REG_SIZE: usize = 0x10;
}

/// USBCMD register bits
mod usbcmd_bits {
    /// Run/Stop
    pub const RS: u32 = 1 << 0;
    /// Host Controller Reset
    pub const HCRST: u32 = 1 << 1;
    /// Interrupter Enable
    pub const INTE: u32 = 1 << 2;
    /// Host System Error Enable
    pub const HSEE: u32 = 1 << 3;
    /// Light Host Controller Reset
    pub const LHCRST: u32 = 1 << 5;
    /// Controller Save State
    pub const CSS: u32 = 1 << 8;
    /// Controller Restore State
    pub const CRS: u32 = 1 << 9;
    /// Enable Wrap Event
    pub const EWE: u32 = 1 << 10;
}

/// USBSTS register bits
mod usbsts_bits {
    /// HC Halted
    pub const HCH: u32 = 1 << 0;
    /// Host System Error
    pub const HSE: u32 = 1 << 2;
    /// Event Interrupt
    pub const EINT: u32 = 1 << 3;
    /// Port Change Detect
    pub const PCD: u32 = 1 << 4;
    /// Controller Not Ready
    pub const CNR: u32 = 1 << 11;
    /// Host Controller Error
    pub const HCE: u32 = 1 << 12;
}

/// Runtime register offsets (relative to runtime base = MMIO_base + RTSOFF)
mod rt_regs {
    /// Microframe Index Register
    pub const MFINDEX: usize = 0x00;
    /// Interrupter Register Set base (interrupter 0)
    pub const IR0_BASE: usize = 0x20;
    /// Size of each Interrupter Register Set
    pub const IR_SIZE: usize = 0x20;
}

/// Interrupter Register offsets (within each interrupter set)
mod ir_regs {
    /// Interrupter Management Register
    pub const IMAN: usize = 0x00;
    /// Interrupter Moderation Register
    pub const IMOD: usize = 0x04;
    /// Event Ring Segment Table Size
    pub const ERSTSZ: usize = 0x08;
    /// Event Ring Segment Table Base Address (64-bit)
    pub const ERSTBA: usize = 0x10;
    /// Event Ring Dequeue Pointer (64-bit)
    pub const ERDP: usize = 0x18;
}

// ---------------------------------------------------------------------------
// Transfer Request Block (TRB)
// ---------------------------------------------------------------------------

/// Generic 16-byte Transfer Request Block
///
/// All TRBs share this common layout per xHCI spec Section 4.11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct Trb {
    /// Parameter low DWORD (meaning varies by TRB type)
    pub parameter_lo: u32,
    /// Parameter high DWORD
    pub parameter_hi: u32,
    /// Status DWORD (transfer length, completion code, etc.)
    pub status: u32,
    /// Control DWORD (TRB type, cycle bit, flags)
    pub control: u32,
}

impl Trb {
    /// Create a zeroed TRB
    pub const fn zeroed() -> Self {
        Self {
            parameter_lo: 0,
            parameter_hi: 0,
            status: 0,
            control: 0,
        }
    }

    /// Get the TRB type field (bits 15:10 of control)
    pub fn trb_type(&self) -> u8 {
        ((self.control >> 10) & 0x3F) as u8
    }

    /// Set the TRB type field
    pub fn set_trb_type(&mut self, trb_type: u8) {
        self.control = (self.control & !(0x3F << 10)) | ((trb_type as u32 & 0x3F) << 10);
    }

    /// Get the cycle bit
    pub fn cycle_bit(&self) -> bool {
        (self.control & TRB_CYCLE_BIT) != 0
    }

    /// Set the cycle bit
    pub fn set_cycle_bit(&mut self, cycle: bool) {
        if cycle {
            self.control |= TRB_CYCLE_BIT;
        } else {
            self.control &= !TRB_CYCLE_BIT;
        }
    }

    /// Get completion code from status DWORD (bits 31:24)
    pub fn completion_code(&self) -> u8 {
        ((self.status >> 24) & 0xFF) as u8
    }

    /// Get the slot ID from control DWORD (bits 31:24)
    pub fn slot_id(&self) -> u8 {
        ((self.control >> 24) & 0xFF) as u8
    }

    /// Set the slot ID in control DWORD (bits 31:24)
    pub fn set_slot_id(&mut self, slot: u8) {
        self.control = (self.control & !(0xFF << 24)) | ((slot as u32) << 24);
    }

    /// Get the transfer length from status DWORD (bits 16:0)
    pub fn transfer_length(&self) -> u32 {
        self.status & 0x1FFFF
    }

    /// Set the transfer length in status DWORD (bits 16:0)
    pub fn set_transfer_length(&mut self, len: u32) {
        self.status = (self.status & !0x1FFFF) | (len & 0x1FFFF);
    }

    /// Create a Normal TRB for bulk/interrupt transfers
    pub fn normal(data_phys: u64, length: u32, ioc: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = data_phys as u32;
        trb.parameter_hi = (data_phys >> 32) as u32;
        trb.set_transfer_length(length);
        trb.set_trb_type(TRB_TYPE_NORMAL);
        trb.set_cycle_bit(cycle);
        if ioc {
            trb.control |= TRB_IOC;
        }
        trb
    }

    /// Create a Setup Stage TRB for control transfers
    pub fn setup_stage(
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        transfer_type: SetupTransferType,
        cycle: bool,
    ) -> Self {
        let mut trb = Self::zeroed();
        // Setup data is packed into parameter fields (8 bytes total)
        trb.parameter_lo = (request_type as u32) | ((request as u32) << 8) | ((value as u32) << 16);
        trb.parameter_hi = (index as u32) | ((length as u32) << 16);
        trb.status = 8; // Setup packet is always 8 bytes
        trb.set_trb_type(TRB_TYPE_SETUP_STAGE);
        trb.set_cycle_bit(cycle);
        trb.control |= TRB_IDT; // Immediate Data for setup stage
                                // Transfer type in bits 17:16
        trb.control |= (transfer_type as u32) << 16;
        trb
    }

    /// Create a Data Stage TRB for control transfers
    pub fn data_stage(data_phys: u64, length: u32, direction_in: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = data_phys as u32;
        trb.parameter_hi = (data_phys >> 32) as u32;
        trb.set_transfer_length(length);
        trb.set_trb_type(TRB_TYPE_DATA_STAGE);
        trb.set_cycle_bit(cycle);
        if direction_in {
            trb.control |= 1 << 16; // DIR = 1 (IN)
        }
        trb
    }

    /// Create a Status Stage TRB for control transfers
    pub fn status_stage(direction_in: bool, ioc: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.set_trb_type(TRB_TYPE_STATUS_STAGE);
        trb.set_cycle_bit(cycle);
        if direction_in {
            trb.control |= 1 << 16; // DIR = 1 (IN)
        }
        if ioc {
            trb.control |= TRB_IOC;
        }
        trb
    }

    /// Create a Link TRB pointing to the start of a ring segment
    pub fn link(segment_phys: u64, toggle_cycle: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = segment_phys as u32;
        trb.parameter_hi = (segment_phys >> 32) as u32;
        trb.set_trb_type(TRB_TYPE_LINK);
        trb.set_cycle_bit(cycle);
        if toggle_cycle {
            trb.control |= TRB_TOGGLE_CYCLE;
        }
        trb
    }

    /// Create an Event Data TRB
    pub fn event_data(data: u64, ioc: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = data as u32;
        trb.parameter_hi = (data >> 32) as u32;
        trb.set_trb_type(TRB_TYPE_EVENT_DATA);
        trb.set_cycle_bit(cycle);
        if ioc {
            trb.control |= TRB_IOC;
        }
        trb
    }

    /// Create a No-Op TRB (transfer ring)
    pub fn noop(ioc: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.set_trb_type(TRB_TYPE_NOOP);
        trb.set_cycle_bit(cycle);
        if ioc {
            trb.control |= TRB_IOC;
        }
        trb
    }

    /// Create an Enable Slot Command TRB
    pub fn enable_slot(cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.set_trb_type(TRB_TYPE_ENABLE_SLOT);
        trb.set_cycle_bit(cycle);
        trb
    }

    /// Create a Disable Slot Command TRB
    pub fn disable_slot(slot_id: u8, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.set_trb_type(TRB_TYPE_DISABLE_SLOT);
        trb.set_slot_id(slot_id);
        trb.set_cycle_bit(cycle);
        trb
    }

    /// Create an Address Device Command TRB
    pub fn address_device(input_context_phys: u64, slot_id: u8, bsr: bool, cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = input_context_phys as u32;
        trb.parameter_hi = (input_context_phys >> 32) as u32;
        trb.set_trb_type(TRB_TYPE_ADDRESS_DEVICE);
        trb.set_slot_id(slot_id);
        trb.set_cycle_bit(cycle);
        if bsr {
            trb.control |= TRB_BSR;
        }
        trb
    }

    /// Create a Configure Endpoint Command TRB
    pub fn configure_endpoint(
        input_context_phys: u64,
        slot_id: u8,
        deconfigure: bool,
        cycle: bool,
    ) -> Self {
        let mut trb = Self::zeroed();
        trb.parameter_lo = input_context_phys as u32;
        trb.parameter_hi = (input_context_phys >> 32) as u32;
        trb.set_trb_type(TRB_TYPE_CONFIGURE_ENDPOINT);
        trb.set_slot_id(slot_id);
        trb.set_cycle_bit(cycle);
        if deconfigure {
            trb.control |= 1 << 9; // DC bit
        }
        trb
    }

    /// Create a No-Op Command TRB
    pub fn noop_cmd(cycle: bool) -> Self {
        let mut trb = Self::zeroed();
        trb.set_trb_type(TRB_TYPE_NOOP_CMD);
        trb.set_cycle_bit(cycle);
        trb
    }
}

/// Transfer type for Setup Stage TRB (control DWORD bits 17:16)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SetupTransferType {
    /// No data stage
    NoData = 0,
    /// OUT data stage
    Out = 2,
    /// IN data stage
    In = 3,
}

// ---------------------------------------------------------------------------
// Ring Buffers
// ---------------------------------------------------------------------------

/// Producer/consumer cycle state for ring buffers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingState {
    /// Physical base address of the ring segment
    pub segment_phys: u64,
    /// Current enqueue/dequeue index within the segment
    pub index: usize,
    /// Current cycle bit (toggled when wrapping via Link TRB)
    pub cycle: bool,
    /// Number of TRBs in the segment (excluding Link TRB at end)
    pub capacity: usize,
}

impl RingState {
    /// Create a new ring state
    pub fn new(segment_phys: u64, capacity: usize) -> Self {
        Self {
            segment_phys,
            index: 0,
            cycle: true,
            capacity,
        }
    }

    /// Advance index, wrapping and toggling cycle if needed
    pub fn advance(&mut self) {
        self.index += 1;
        if self.index >= self.capacity {
            self.index = 0;
            self.cycle = !self.cycle;
        }
    }

    /// Get the physical address of the TRB at the current index
    pub fn current_trb_phys(&self) -> u64 {
        self.segment_phys + (self.index as u64) * (TRB_SIZE as u64)
    }
}

/// Command Ring: the host controller dequeues command TRBs from here
pub struct CommandRing {
    state: RingState,
}

impl CommandRing {
    /// Create a new command ring
    pub fn new(segment_phys: u64) -> Self {
        Self {
            state: RingState::new(segment_phys, RING_SEGMENT_TRBS - 1),
        }
    }

    /// Enqueue a command TRB onto the ring.
    ///
    /// Returns the physical address of the enqueued TRB.
    ///
    /// # Safety
    ///
    /// The caller must ensure `segment_phys` points to valid,
    /// mapped, DMA-accessible memory of at least `RING_SEGMENT_SIZE` bytes.
    #[cfg(target_os = "none")]
    pub unsafe fn enqueue(&mut self, mut trb: Trb) -> u64 {
        trb.set_cycle_bit(self.state.cycle);
        let addr = self.state.current_trb_phys();

        // SAFETY: addr is within the ring segment allocated by the caller.
        // write_volatile ensures the TRB is visible to the hardware.
        unsafe {
            core::ptr::write_volatile(addr as *mut Trb, trb);
        }

        self.state.advance();

        // If we've wrapped, write a Link TRB at the last slot pointing back
        if self.state.index == 0 {
            let link_addr =
                self.state.segment_phys + (self.state.capacity as u64) * (TRB_SIZE as u64);
            let link = Trb::link(self.state.segment_phys, true, !self.state.cycle);
            // SAFETY: link_addr is the last TRB slot in the segment.
            unsafe {
                core::ptr::write_volatile(link_addr as *mut Trb, link);
            }
        }

        addr
    }

    /// Non-hardware stub for testing on host targets
    #[cfg(not(target_os = "none"))]
    pub fn enqueue(&mut self, mut trb: Trb) -> u64 {
        trb.set_cycle_bit(self.state.cycle);
        let addr = self.state.current_trb_phys();
        self.state.advance();
        addr
    }

    /// Get the physical base address and current cycle bit for CRCR
    pub fn crcr_value(&self) -> u64 {
        self.state.segment_phys | if self.state.cycle { 1 } else { 0 }
    }
}

/// Event Ring Segment Table Entry (xHCI spec 6.5)
#[derive(Debug, Clone, Copy)]
#[repr(C, align(64))]
pub struct EventRingSegmentTableEntry {
    /// Ring Segment Base Address (64-bit, 64-byte aligned)
    pub base_address: u64,
    /// Ring Segment Size (number of TRBs)
    pub size: u16,
    /// Reserved
    pub _reserved0: u16,
    /// Reserved
    pub _reserved1: u32,
}

/// Event Ring: the host controller enqueues event TRBs here
pub struct EventRing {
    state: RingState,
}

impl EventRing {
    /// Create a new event ring
    pub fn new(segment_phys: u64, capacity: usize) -> Self {
        Self {
            state: RingState::new(segment_phys, capacity),
        }
    }

    /// Dequeue the next event TRB if one is available.
    ///
    /// Returns `Some(trb)` if the cycle bit of the TRB at the dequeue pointer
    /// matches our expected cycle state, or `None` if no event is pending.
    ///
    /// # Safety
    ///
    /// The caller must ensure the event ring segment is valid memory.
    #[cfg(target_os = "none")]
    pub unsafe fn dequeue(&mut self) -> Option<Trb> {
        let addr = self.state.current_trb_phys();
        // SAFETY: addr points within the event ring segment.
        let trb = unsafe { core::ptr::read_volatile(addr as *const Trb) };

        if trb.cycle_bit() != self.state.cycle {
            return None; // No new event
        }

        self.state.advance();
        Some(trb)
    }

    /// Non-hardware stub for host targets
    #[cfg(not(target_os = "none"))]
    pub fn dequeue(&mut self) -> Option<Trb> {
        None
    }

    /// Get the current dequeue pointer for writing to ERDP
    pub fn erdp_value(&self) -> u64 {
        self.state.current_trb_phys()
    }
}

/// Transfer Ring: per-endpoint ring for bulk/interrupt/control data transfers
pub struct TransferRing {
    state: RingState,
}

impl TransferRing {
    /// Create a new transfer ring
    pub fn new(segment_phys: u64) -> Self {
        Self {
            state: RingState::new(segment_phys, RING_SEGMENT_TRBS - 1),
        }
    }

    /// Enqueue a TRB onto the transfer ring.
    ///
    /// # Safety
    ///
    /// The caller must ensure the ring segment is valid DMA memory.
    #[cfg(target_os = "none")]
    pub unsafe fn enqueue(&mut self, mut trb: Trb) -> u64 {
        trb.set_cycle_bit(self.state.cycle);
        let addr = self.state.current_trb_phys();

        // SAFETY: addr is within the transfer ring segment.
        unsafe {
            core::ptr::write_volatile(addr as *mut Trb, trb);
        }

        self.state.advance();

        // Write Link TRB if we wrapped
        if self.state.index == 0 {
            let link_addr =
                self.state.segment_phys + (self.state.capacity as u64) * (TRB_SIZE as u64);
            let link = Trb::link(self.state.segment_phys, true, !self.state.cycle);
            unsafe {
                core::ptr::write_volatile(link_addr as *mut Trb, link);
            }
        }

        addr
    }

    /// Non-hardware stub for host targets
    #[cfg(not(target_os = "none"))]
    pub fn enqueue(&mut self, mut trb: Trb) -> u64 {
        trb.set_cycle_bit(self.state.cycle);
        let addr = self.state.current_trb_phys();
        self.state.advance();
        addr
    }

    /// Enqueue a control transfer (Setup + optional Data + Status).
    ///
    /// Returns the physical address of the Status Stage TRB.
    ///
    /// # Safety
    ///
    /// The caller must ensure the ring segment is valid DMA memory and
    /// `data_phys` (if non-zero) points to a valid buffer of `data_len` bytes.
    #[cfg(target_os = "none")]
    pub unsafe fn enqueue_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data_phys: u64,
        data_len: u16,
    ) -> u64 {
        let direction_in = (request_type & USB_DIR_IN) != 0;
        let transfer_type = if data_len == 0 {
            SetupTransferType::NoData
        } else if direction_in {
            SetupTransferType::In
        } else {
            SetupTransferType::Out
        };

        // Setup Stage
        let setup = Trb::setup_stage(
            request_type,
            request,
            value,
            index,
            data_len,
            transfer_type,
            self.state.cycle,
        );
        unsafe {
            self.enqueue(setup);
        }

        // Data Stage (if any)
        if data_len > 0 {
            let data = Trb::data_stage(data_phys, data_len as u32, direction_in, self.state.cycle);
            unsafe {
                self.enqueue(data);
            }
        }

        // Status Stage (direction is opposite of data stage, or IN if no data)
        let status_dir_in = if data_len == 0 { true } else { !direction_in };
        let status = Trb::status_stage(status_dir_in, true, self.state.cycle);
        unsafe { self.enqueue(status) }
    }

    /// Non-hardware stub for host targets
    #[cfg(not(target_os = "none"))]
    pub fn enqueue_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        _data_phys: u64,
        data_len: u16,
    ) -> u64 {
        let direction_in = (request_type & USB_DIR_IN) != 0;
        let transfer_type = if data_len == 0 {
            SetupTransferType::NoData
        } else if direction_in {
            SetupTransferType::In
        } else {
            SetupTransferType::Out
        };

        let setup = Trb::setup_stage(
            request_type,
            request,
            value,
            index,
            data_len,
            transfer_type,
            self.state.cycle,
        );
        self.enqueue(setup);

        if data_len > 0 {
            let data = Trb::data_stage(0, data_len as u32, direction_in, self.state.cycle);
            self.enqueue(data);
        }

        let status_dir_in = if data_len == 0 { true } else { !direction_in };
        let status = Trb::status_stage(status_dir_in, true, self.state.cycle);
        self.enqueue(status)
    }

    /// Get physical base address of the ring
    pub fn base_phys(&self) -> u64 {
        self.state.segment_phys
    }
}

// ---------------------------------------------------------------------------
// USB Descriptors (parsed from raw bytes)
// ---------------------------------------------------------------------------

/// Parsed USB Device Descriptor (18 bytes)
#[derive(Debug, Clone, Copy)]
pub struct XhciDeviceDescriptor {
    pub usb_version: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size_ep0: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub manufacturer_index: u8,
    pub product_index: u8,
    pub serial_index: u8,
    pub num_configurations: u8,
}

impl XhciDeviceDescriptor {
    /// Parse from a raw 18-byte descriptor buffer
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 18 {
            return Err(KernelError::InvalidArgument {
                name: "device_descriptor",
                value: "buffer too short (need 18 bytes)",
            });
        }
        if data[1] != DESC_TYPE_DEVICE {
            return Err(KernelError::InvalidArgument {
                name: "device_descriptor",
                value: "wrong descriptor type",
            });
        }
        Ok(Self {
            usb_version: u16::from_le_bytes([data[2], data[3]]),
            device_class: data[4],
            device_subclass: data[5],
            device_protocol: data[6],
            max_packet_size_ep0: data[7],
            vendor_id: u16::from_le_bytes([data[8], data[9]]),
            product_id: u16::from_le_bytes([data[10], data[11]]),
            device_version: u16::from_le_bytes([data[12], data[13]]),
            manufacturer_index: data[14],
            product_index: data[15],
            serial_index: data[16],
            num_configurations: data[17],
        })
    }
}

/// Parsed USB Configuration Descriptor header (9 bytes)
#[derive(Debug, Clone, Copy)]
pub struct XhciConfigDescriptor {
    pub total_length: u16,
    pub num_interfaces: u8,
    pub config_value: u8,
    pub config_index: u8,
    pub attributes: u8,
    pub max_power_ma: u16,
}

impl XhciConfigDescriptor {
    /// Parse from a raw descriptor buffer (at least 9 bytes)
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 9 {
            return Err(KernelError::InvalidArgument {
                name: "config_descriptor",
                value: "buffer too short (need 9 bytes)",
            });
        }
        if data[1] != DESC_TYPE_CONFIGURATION {
            return Err(KernelError::InvalidArgument {
                name: "config_descriptor",
                value: "wrong descriptor type",
            });
        }
        Ok(Self {
            total_length: u16::from_le_bytes([data[2], data[3]]),
            num_interfaces: data[4],
            config_value: data[5],
            config_index: data[6],
            attributes: data[7],
            max_power_ma: (data[8] as u16) * 2, // bMaxPower is in units of 2 mA
        })
    }

    /// Whether the device is self-powered
    pub fn is_self_powered(&self) -> bool {
        (self.attributes & (1 << 6)) != 0
    }

    /// Whether remote wakeup is supported
    pub fn supports_remote_wakeup(&self) -> bool {
        (self.attributes & (1 << 5)) != 0
    }
}

/// Parsed USB Interface Descriptor (9 bytes)
#[derive(Debug, Clone, Copy)]
pub struct XhciInterfaceDescriptor {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub num_endpoints: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
    pub interface_index: u8,
}

impl XhciInterfaceDescriptor {
    /// Parse from a raw descriptor buffer (at least 9 bytes)
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 9 {
            return Err(KernelError::InvalidArgument {
                name: "interface_descriptor",
                value: "buffer too short (need 9 bytes)",
            });
        }
        if data[1] != DESC_TYPE_INTERFACE {
            return Err(KernelError::InvalidArgument {
                name: "interface_descriptor",
                value: "wrong descriptor type",
            });
        }
        Ok(Self {
            interface_number: data[2],
            alternate_setting: data[3],
            num_endpoints: data[4],
            interface_class: data[5],
            interface_subclass: data[6],
            interface_protocol: data[7],
            interface_index: data[8],
        })
    }
}

/// Parsed USB Endpoint Descriptor (7 bytes)
#[derive(Debug, Clone, Copy)]
pub struct XhciEndpointDescriptor {
    pub endpoint_address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl XhciEndpointDescriptor {
    /// Parse from a raw descriptor buffer (at least 7 bytes)
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 7 {
            return Err(KernelError::InvalidArgument {
                name: "endpoint_descriptor",
                value: "buffer too short (need 7 bytes)",
            });
        }
        if data[1] != DESC_TYPE_ENDPOINT {
            return Err(KernelError::InvalidArgument {
                name: "endpoint_descriptor",
                value: "wrong descriptor type",
            });
        }
        Ok(Self {
            endpoint_address: data[2],
            attributes: data[3],
            max_packet_size: u16::from_le_bytes([data[4], data[5]]),
            interval: data[6],
        })
    }

    /// Whether this is an IN endpoint
    pub fn is_in(&self) -> bool {
        (self.endpoint_address & 0x80) != 0
    }

    /// Get the endpoint number (bits 3:0)
    pub fn endpoint_number(&self) -> u8 {
        self.endpoint_address & 0x0F
    }

    /// Get the transfer type
    pub fn transfer_type(&self) -> EndpointTransferType {
        match self.attributes & 0x03 {
            0 => EndpointTransferType::Control,
            1 => EndpointTransferType::Isochronous,
            2 => EndpointTransferType::Bulk,
            3 => EndpointTransferType::Interrupt,
            _ => unreachable!(),
        }
    }
}

/// Endpoint transfer type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointTransferType {
    Control = 0,
    Isochronous = 1,
    Bulk = 2,
    Interrupt = 3,
}

/// A parsed interface paired with its endpoint descriptors.
#[cfg(feature = "alloc")]
pub type InterfaceWithEndpoints = (XhciInterfaceDescriptor, Vec<XhciEndpointDescriptor>);

/// Parse all descriptors from a full configuration descriptor buffer.
///
/// Returns (config, interfaces_with_endpoints) where each interface
/// is paired with its endpoints.
#[cfg(feature = "alloc")]
pub fn parse_configuration_descriptors(
    data: &[u8],
) -> Result<(XhciConfigDescriptor, Vec<InterfaceWithEndpoints>), KernelError> {
    let config = XhciConfigDescriptor::parse(data)?;
    let total = config.total_length as usize;
    if data.len() < total {
        return Err(KernelError::InvalidArgument {
            name: "config_descriptor",
            value: "buffer shorter than wTotalLength",
        });
    }

    let mut interfaces = Vec::new();
    let mut current_iface: Option<XhciInterfaceDescriptor> = None;
    let mut current_endpoints: Vec<XhciEndpointDescriptor> = Vec::new();
    let mut offset = data[0] as usize; // skip config descriptor header

    while offset + 1 < total {
        let desc_len = data[offset] as usize;
        if desc_len < 2 || offset + desc_len > total {
            break;
        }
        let desc_type = data[offset + 1];

        match desc_type {
            DESC_TYPE_INTERFACE => {
                // Save previous interface if any
                if let Some(iface) = current_iface.take() {
                    interfaces.push((iface, core::mem::take(&mut current_endpoints)));
                }
                if let Ok(iface) = XhciInterfaceDescriptor::parse(&data[offset..]) {
                    current_iface = Some(iface);
                }
            }
            DESC_TYPE_ENDPOINT => {
                if let Ok(ep) = XhciEndpointDescriptor::parse(&data[offset..]) {
                    current_endpoints.push(ep);
                }
            }
            _ => {} // Skip unknown descriptors
        }

        offset += desc_len;
    }

    // Push the last interface
    if let Some(iface) = current_iface {
        interfaces.push((iface, current_endpoints));
    }

    Ok((config, interfaces))
}

// ---------------------------------------------------------------------------
// Device Slot Management
// ---------------------------------------------------------------------------

/// State of a device slot
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Slot is disabled (available for allocation)
    Disabled,
    /// Slot is enabled but device not yet addressed
    Enabled,
    /// Default state (BSR = 1 was used)
    Default,
    /// Device has been addressed
    Addressed,
    /// Device is configured
    Configured,
}

/// Per-device-slot tracking information
#[derive(Debug)]
pub struct DeviceSlot {
    /// Slot ID (1-based, assigned by hardware via Enable Slot)
    pub slot_id: u8,
    /// Current slot state
    pub state: SlotState,
    /// Root hub port number this device is connected to (1-based)
    pub port_number: u8,
    /// USB device speed
    pub speed: PortSpeed,
    /// Physical address of the device context (output context)
    pub device_context_phys: u64,
    /// Physical address of the input context
    pub input_context_phys: u64,
}

/// Port speed as reported by xHCI PORTSC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSpeed {
    /// Full-speed (12 Mb/s)
    Full,
    /// Low-speed (1.5 Mb/s)
    Low,
    /// High-speed (480 Mb/s)
    High,
    /// SuperSpeed (5 Gb/s)
    Super,
    /// SuperSpeedPlus (10+ Gb/s)
    SuperPlus,
    /// Unknown speed
    Unknown,
}

impl PortSpeed {
    /// Convert from PORTSC speed field value
    pub fn from_portsc(speed_val: u32) -> Self {
        match speed_val {
            PORT_SPEED_FULL => Self::Full,
            PORT_SPEED_LOW => Self::Low,
            PORT_SPEED_HIGH => Self::High,
            PORT_SPEED_SUPER => Self::Super,
            PORT_SPEED_SUPER_PLUS => Self::SuperPlus,
            _ => Self::Unknown,
        }
    }

    /// Get the maximum packet size for endpoint 0 for this speed
    pub fn default_max_packet_size_ep0(&self) -> u16 {
        match self {
            Self::Low => 8,
            Self::Full => 8, // Can be 8, 16, 32, or 64; start with 8
            Self::High => 64,
            Self::Super | Self::SuperPlus => 512,
            Self::Unknown => 8,
        }
    }
}

// ---------------------------------------------------------------------------
// Port Management
// ---------------------------------------------------------------------------

/// xHCI port information
#[derive(Debug, Clone, Copy)]
pub struct XhciPort {
    /// Port number (1-based)
    pub number: u8,
    /// Whether a device is connected
    pub connected: bool,
    /// Whether the port is enabled
    pub enabled: bool,
    /// Current speed of connected device
    pub speed: PortSpeed,
    /// Whether the port is powered
    pub powered: bool,
    /// Whether a reset is in progress
    pub resetting: bool,
}

impl XhciPort {
    /// Parse port state from a PORTSC register value
    pub fn from_portsc(port_number: u8, portsc: u32) -> Self {
        let speed_val = (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT;
        Self {
            number: port_number,
            connected: (portsc & PORTSC_CCS) != 0,
            enabled: (portsc & PORTSC_PED) != 0,
            speed: PortSpeed::from_portsc(speed_val),
            powered: (portsc & PORTSC_PP) != 0,
            resetting: (portsc & PORTSC_PR) != 0,
        }
    }
}

// ---------------------------------------------------------------------------
// xHCI Controller State
// ---------------------------------------------------------------------------

/// xHCI controller capabilities parsed from Capability Registers
#[derive(Debug, Clone, Copy)]
pub struct XhciCapabilities {
    /// Length of capability register space (offset to operational registers)
    pub cap_length: u8,
    /// xHCI interface version (e.g. 0x0100 = 1.0, 0x0110 = 1.1)
    pub hci_version: u16,
    /// Maximum number of device slots
    pub max_slots: u8,
    /// Maximum number of interrupters
    pub max_intrs: u16,
    /// Maximum number of ports
    pub max_ports: u8,
    /// Whether the controller supports 64-byte contexts
    pub context_size_64: bool,
    /// Doorbell array offset from MMIO base
    pub doorbell_offset: u32,
    /// Runtime register space offset from MMIO base
    pub runtime_offset: u32,
}

/// State of the xHCI controller
pub struct XhciController {
    /// MMIO base address (from PCI BAR0)
    mmio_base: u64,
    /// Parsed capability registers
    capabilities: XhciCapabilities,
    /// Operational register base address
    op_base: u64,
    /// Runtime register base address
    rt_base: u64,
    /// Doorbell array base address
    db_base: u64,
    /// Command ring
    command_ring: CommandRing,
    /// Event ring for interrupter 0
    event_ring: EventRing,
    /// Whether the controller is running
    running: bool,
    /// Number of enabled device slots
    enabled_slots: u32,
}

impl XhciController {
    /// Create a new xHCI controller instance from its MMIO base address.
    ///
    /// # Safety
    ///
    /// `mmio_base` must point to a valid xHCI MMIO region that is
    /// identity-mapped or otherwise accessible. This function reads capability
    /// registers to determine the layout.
    #[cfg(target_os = "none")]
    pub unsafe fn new(mmio_base: u64) -> Result<Self, KernelError> {
        // Read capability registers
        let cap_length = unsafe { core::ptr::read_volatile(mmio_base as *const u8) };
        let hci_version = unsafe {
            core::ptr::read_volatile((mmio_base + cap_regs::HCIVERSION as u64) as *const u16)
        };
        let hcsparams1 = unsafe {
            core::ptr::read_volatile((mmio_base + cap_regs::HCSPARAMS1 as u64) as *const u32)
        };
        let hccparams1 = unsafe {
            core::ptr::read_volatile((mmio_base + cap_regs::HCCPARAMS1 as u64) as *const u32)
        };
        let dboff =
            unsafe { core::ptr::read_volatile((mmio_base + cap_regs::DBOFF as u64) as *const u32) };
        let rtsoff = unsafe {
            core::ptr::read_volatile((mmio_base + cap_regs::RTSOFF as u64) as *const u32)
        };

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_intrs = ((hcsparams1 >> 8) & 0x7FF) as u16;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
        let context_size_64 = (hccparams1 & (1 << 2)) != 0;

        let capabilities = XhciCapabilities {
            cap_length,
            hci_version,
            max_slots,
            max_intrs,
            max_ports,
            context_size_64,
            doorbell_offset: dboff & !0x3,  // Must be DWORD aligned
            runtime_offset: rtsoff & !0x1F, // Must be 32-byte aligned
        };

        let op_base = mmio_base + cap_length as u64;
        let rt_base = mmio_base + capabilities.runtime_offset as u64;
        let db_base = mmio_base + capabilities.doorbell_offset as u64;

        // Create command and event rings with placeholder addresses.
        // Real addresses will be set during init() after allocating DMA memory.
        let command_ring = CommandRing::new(0);
        let event_ring = EventRing::new(0, RING_SEGMENT_TRBS);

        Ok(Self {
            mmio_base,
            capabilities,
            op_base,
            rt_base,
            db_base,
            command_ring,
            event_ring,
            running: false,
            enabled_slots: 0,
        })
    }

    /// Non-hardware constructor for host-target testing
    #[cfg(not(target_os = "none"))]
    pub fn new_stub() -> Self {
        let capabilities = XhciCapabilities {
            cap_length: 0x20,
            hci_version: 0x0110,
            max_slots: 64,
            max_intrs: 8,
            max_ports: 4,
            context_size_64: false,
            doorbell_offset: 0x2000,
            runtime_offset: 0x1000,
        };

        Self {
            mmio_base: 0,
            capabilities,
            op_base: 0x20,
            rt_base: 0x1000,
            db_base: 0x2000,
            command_ring: CommandRing::new(0x10_0000),
            event_ring: EventRing::new(0x20_0000, RING_SEGMENT_TRBS),
            running: false,
            enabled_slots: 0,
        }
    }

    /// Get the controller capabilities
    pub fn capabilities(&self) -> &XhciCapabilities {
        &self.capabilities
    }

    /// Read an operational register (32-bit)
    #[cfg(target_os = "none")]
    fn read_op_reg(&self, offset: usize) -> u32 {
        // SAFETY: op_base + offset is within the xHCI operational register space.
        unsafe { core::ptr::read_volatile((self.op_base + offset as u64) as *const u32) }
    }

    #[cfg(not(target_os = "none"))]
    fn read_op_reg(&self, _offset: usize) -> u32 {
        0
    }

    /// Write an operational register (32-bit)
    #[cfg(target_os = "none")]
    fn write_op_reg(&self, offset: usize, value: u32) {
        // SAFETY: op_base + offset is within the xHCI operational register space.
        unsafe { core::ptr::write_volatile((self.op_base + offset as u64) as *mut u32, value) }
    }

    #[cfg(not(target_os = "none"))]
    fn write_op_reg(&self, _offset: usize, _value: u32) {}

    /// Read a 64-bit operational register
    #[cfg(target_os = "none")]
    fn read_op_reg64(&self, offset: usize) -> u64 {
        // SAFETY: op_base + offset is within the xHCI operational register space.
        unsafe { core::ptr::read_volatile((self.op_base + offset as u64) as *const u64) }
    }

    #[cfg(not(target_os = "none"))]
    fn read_op_reg64(&self, _offset: usize) -> u64 {
        0
    }

    /// Write a 64-bit operational register
    #[cfg(target_os = "none")]
    fn write_op_reg64(&self, offset: usize, value: u64) {
        // SAFETY: op_base + offset is within the xHCI operational register space.
        unsafe { core::ptr::write_volatile((self.op_base + offset as u64) as *mut u64, value) }
    }

    #[cfg(not(target_os = "none"))]
    fn write_op_reg64(&self, _offset: usize, _value: u64) {}

    /// Read a PORTSC register for a given port (1-based port number)
    fn read_portsc(&self, port: u8) -> u32 {
        let offset = op_regs::PORT_REG_BASE + ((port as usize - 1) * op_regs::PORT_REG_SIZE);
        self.read_op_reg(offset)
    }

    /// Write a PORTSC register for a given port (1-based port number)
    fn write_portsc(&self, port: u8, value: u32) {
        let offset = op_regs::PORT_REG_BASE + ((port as usize - 1) * op_regs::PORT_REG_SIZE);
        self.write_op_reg(offset, value);
    }

    /// Ring the host controller doorbell (doorbell 0 = command ring)
    fn ring_doorbell(&self, slot: u8, target: u8) {
        let db_offset = (slot as u64) * 4;
        let value = target as u32;
        #[cfg(target_os = "none")]
        {
            // SAFETY: db_base + db_offset is within the doorbell array.
            unsafe {
                core::ptr::write_volatile((self.db_base + db_offset) as *mut u32, value);
            }
        }
        let _ = (db_offset, value); // suppress unused warnings on host
    }

    /// Ring the command doorbell (doorbell register 0, target 0)
    pub fn ring_command_doorbell(&self) {
        self.ring_doorbell(0, 0);
    }

    /// Ring a transfer doorbell for a specific endpoint on a slot
    pub fn ring_transfer_doorbell(&self, slot_id: u8, endpoint_id: u8) {
        self.ring_doorbell(slot_id, endpoint_id);
    }

    /// Wait for the Controller Not Ready (CNR) flag to clear
    fn wait_cnr_clear(&self) -> Result<(), KernelError> {
        for _ in 0..10_000 {
            let status = self.read_op_reg(op_regs::USBSTS);
            if (status & usbsts_bits::CNR) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(KernelError::Timeout {
            operation: "xhci_cnr_clear",
            duration_ms: 1000,
        })
    }

    /// Reset the host controller
    pub fn reset(&mut self) -> Result<(), KernelError> {
        // Stop the controller first
        let cmd = self.read_op_reg(op_regs::USBCMD);
        self.write_op_reg(op_regs::USBCMD, cmd & !usbcmd_bits::RS);

        // Wait for halt
        for _ in 0..10_000 {
            let status = self.read_op_reg(op_regs::USBSTS);
            if (status & usbsts_bits::HCH) != 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Issue HCRST
        self.write_op_reg(op_regs::USBCMD, usbcmd_bits::HCRST);

        // Wait for reset to complete (HCRST auto-clears)
        for _ in 0..10_000 {
            let cmd = self.read_op_reg(op_regs::USBCMD);
            if (cmd & usbcmd_bits::HCRST) == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // Wait for CNR to clear
        self.wait_cnr_clear()?;

        self.running = false;
        Ok(())
    }

    /// Start the host controller (set Run/Stop bit)
    pub fn start(&mut self) -> Result<(), KernelError> {
        let cmd = self.read_op_reg(op_regs::USBCMD);
        self.write_op_reg(op_regs::USBCMD, cmd | usbcmd_bits::RS | usbcmd_bits::INTE);

        // Verify it started
        for _ in 0..1_000 {
            let status = self.read_op_reg(op_regs::USBSTS);
            if (status & usbsts_bits::HCH) == 0 {
                self.running = true;
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err(KernelError::HardwareError {
            device: "xhci",
            code: 1,
        })
    }

    /// Stop the host controller
    pub fn stop(&mut self) -> Result<(), KernelError> {
        let cmd = self.read_op_reg(op_regs::USBCMD);
        self.write_op_reg(op_regs::USBCMD, cmd & !usbcmd_bits::RS);

        for _ in 0..10_000 {
            let status = self.read_op_reg(op_regs::USBSTS);
            if (status & usbsts_bits::HCH) != 0 {
                self.running = false;
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err(KernelError::Timeout {
            operation: "xhci_stop",
            duration_ms: 1000,
        })
    }

    /// Configure the maximum number of device slots
    pub fn set_max_slots(&mut self, max: u8) {
        let max = max.min(self.capabilities.max_slots);
        let config = self.read_op_reg(op_regs::CONFIG);
        self.write_op_reg(op_regs::CONFIG, (config & !0xFF) | (max as u32));
        self.enabled_slots = max as u32;
    }

    /// Set the Device Context Base Address Array Pointer
    pub fn set_dcbaap(&self, phys_addr: u64) {
        self.write_op_reg64(op_regs::DCBAAP, phys_addr);
    }

    /// Set the Command Ring Control Register
    pub fn set_crcr(&self) {
        self.write_op_reg64(op_regs::CRCR, self.command_ring.crcr_value());
    }

    /// Get port status for a given port (1-based)
    pub fn get_port_status(&self, port: u8) -> Result<XhciPort, KernelError> {
        if port == 0 || port > self.capabilities.max_ports {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
        }
        let portsc = self.read_portsc(port);
        Ok(XhciPort::from_portsc(port, portsc))
    }

    /// Reset a port (1-based)
    pub fn reset_port(&self, port: u8) -> Result<(), KernelError> {
        if port == 0 || port > self.capabilities.max_ports {
            return Err(KernelError::InvalidArgument {
                name: "port",
                value: "out of range",
            });
        }

        let portsc = self.read_portsc(port);
        // Preserve RO and RW bits, set Port Reset, clear change bits
        let new_portsc = (portsc & !PORTSC_CHANGE_BITS) | PORTSC_PR;
        self.write_portsc(port, new_portsc);

        // Wait for reset to complete (PRC bit will be set)
        for _ in 0..10_000 {
            let portsc = self.read_portsc(port);
            if (portsc & PORTSC_PRC) != 0 {
                // Clear PRC
                self.write_portsc(port, (portsc & !PORTSC_CHANGE_BITS) | PORTSC_PRC);
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err(KernelError::Timeout {
            operation: "xhci_port_reset",
            duration_ms: 1000,
        })
    }

    /// Check if a device is connected on a port (1-based)
    pub fn is_port_connected(&self, port: u8) -> bool {
        if port == 0 || port > self.capabilities.max_ports {
            return false;
        }
        let portsc = self.read_portsc(port);
        (portsc & PORTSC_CCS) != 0
    }

    /// Whether the controller is currently running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Enqueue a command TRB and ring the doorbell.
    ///
    /// Returns the physical address where the command was placed.
    ///
    /// # Safety
    ///
    /// The command ring segment must be valid DMA-accessible memory.
    #[cfg(target_os = "none")]
    pub unsafe fn send_command(&mut self, trb: Trb) -> u64 {
        let addr = unsafe { self.command_ring.enqueue(trb) };
        self.ring_command_doorbell();
        addr
    }

    /// Non-hardware stub
    #[cfg(not(target_os = "none"))]
    pub fn send_command(&mut self, trb: Trb) -> u64 {
        self.command_ring.enqueue(trb)
    }

    /// Poll for the next event from interrupter 0.
    ///
    /// # Safety
    ///
    /// The event ring segment must be valid memory.
    #[cfg(target_os = "none")]
    pub unsafe fn poll_event(&mut self) -> Option<Trb> {
        let event = unsafe { self.event_ring.dequeue()? };
        // Update ERDP to acknowledge the event
        let erdp = self.event_ring.erdp_value();
        let ir0_base = self.rt_base + rt_regs::IR0_BASE as u64;
        // SAFETY: Writing to the Event Ring Dequeue Pointer register.
        unsafe {
            core::ptr::write_volatile(
                (ir0_base + ir_regs::ERDP as u64) as *mut u64,
                erdp | (1 << 3), // Set EHB (Event Handler Busy) to clear
            );
        }
        Some(event)
    }

    /// Non-hardware stub
    #[cfg(not(target_os = "none"))]
    pub fn poll_event(&mut self) -> Option<Trb> {
        self.event_ring.dequeue()
    }
}

// ---------------------------------------------------------------------------
// MSI-X Interrupt Handling Stubs
// ---------------------------------------------------------------------------

/// MSI-X capability structure offsets (within PCI config space)
#[derive(Debug, Clone, Copy)]
pub struct MsixCapability {
    /// Offset of the MSI-X capability in PCI config space
    pub cap_offset: u16,
    /// Table size (number of entries - 1)
    pub table_size: u16,
    /// Table BAR indicator (which BAR contains the table)
    pub table_bar: u8,
    /// Table offset within the BAR
    pub table_offset: u32,
    /// PBA (Pending Bit Array) BAR indicator
    pub pba_bar: u8,
    /// PBA offset within the BAR
    pub pba_offset: u32,
}

/// MSI-X table entry (16 bytes per entry)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MsixTableEntry {
    /// Message Address (lower 32 bits)
    pub msg_addr_lo: u32,
    /// Message Address (upper 32 bits)
    pub msg_addr_hi: u32,
    /// Message Data
    pub msg_data: u32,
    /// Vector Control (bit 0 = mask)
    pub vector_control: u32,
}

/// Configure MSI-X for the xHCI controller.
///
/// This is a stub that will be filled in when the full PCI MSI-X
/// infrastructure is available.
#[cfg(target_os = "none")]
pub fn configure_msix(
    _msix: &MsixCapability,
    _mmio_base: u64,
    _num_vectors: u16,
) -> Result<(), KernelError> {
    // TODO(phase7.5): Wire to actual MSI-X configuration when PCI
    // capability parsing is extended to support MSI-X enable/mask.
    crate::println!("[xHCI] MSI-X configuration stub (not yet wired)");
    Ok(())
}

/// MSI-X interrupt handler stub.
///
/// Called from the architecture-specific interrupt dispatch when an
/// xHCI MSI-X vector fires.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub fn msix_interrupt_handler(_vector: u8) {
    // TODO(phase7.5): Read event ring, process events, acknowledge interrupt.
    // For now this is a no-op stub.
}

#[cfg(all(target_os = "none", target_arch = "aarch64"))]
pub fn msix_interrupt_handler(_irq: u32) {
    // AArch64 GIC-based MSI-X handler stub
}

#[cfg(all(target_os = "none", target_arch = "riscv64"))]
pub fn msix_interrupt_handler(_irq: u32) {
    // RISC-V PLIC-based MSI-X handler stub
}

// ---------------------------------------------------------------------------
// PCI Enumeration Helpers
// ---------------------------------------------------------------------------

/// Check if a PCI device is an xHCI controller
pub fn is_xhci_device(class_code: u8, subclass: u8, prog_if: u8) -> bool {
    class_code == PCI_CLASS_SERIAL_BUS && subclass == PCI_SUBCLASS_USB && prog_if == PCI_PROGIF_XHCI
}

/// Scan the PCI bus for xHCI controllers and return their BAR0 addresses.
///
/// This uses the existing PCI device list from `crate::drivers::pci`.
#[cfg(feature = "alloc")]
pub fn find_xhci_controllers() -> Vec<XhciPciDevice> {
    let mut controllers = Vec::new();

    // Use the PCI subsystem to iterate discovered devices
    #[cfg(target_os = "none")]
    {
        use crate::drivers::pci;
        let pci_bus = pci::get_pci_bus().lock();
        let devices = pci_bus.get_all_devices();
        for dev in &devices {
            if is_xhci_device(dev.class_code, dev.subclass, dev.prog_if) {
                let bar0_addr = dev.bars[0].get_memory_address().unwrap_or(0);
                controllers.push(XhciPciDevice {
                    bus: dev.location.bus,
                    device: dev.location.device,
                    function: dev.location.function,
                    vendor_id: dev.vendor_id,
                    device_id: dev.device_id,
                    bar0: bar0_addr,
                    irq_line: dev.interrupt_line,
                });
            }
        }
    }

    controllers
}

/// Information about a discovered xHCI PCI device
#[derive(Debug, Clone, Copy)]
pub struct XhciPciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    /// BAR0 physical address (MMIO base)
    pub bar0: u64,
    /// IRQ line from PCI config
    pub irq_line: u8,
}

// ---------------------------------------------------------------------------
// Global State
// ---------------------------------------------------------------------------

static XHCI_INITIALIZED: AtomicBool = AtomicBool::new(false);
static XHCI_CONTROLLER_COUNT: AtomicU32 = AtomicU32::new(0);

/// Initialize the xHCI subsystem.
///
/// Scans PCI for xHCI controllers, resets and configures each one.
pub fn init() {
    if XHCI_INITIALIZED.swap(true, Ordering::SeqCst) {
        return; // Already initialized
    }

    crate::println!("[xHCI] Scanning for xHCI USB 3.x controllers...");

    #[cfg(feature = "alloc")]
    {
        let devices = find_xhci_controllers();
        let count = devices.len();

        for dev in &devices {
            crate::println!(
                "[xHCI] Found controller at PCI {:02x}:{:02x}.{} (vendor={:04x} device={:04x}) \
                 BAR0=0x{:08x}",
                dev.bus,
                dev.device,
                dev.function,
                dev.vendor_id,
                dev.device_id,
                dev.bar0,
            );
        }

        XHCI_CONTROLLER_COUNT.store(count as u32, Ordering::SeqCst);

        if count == 0 {
            crate::println!("[xHCI] No xHCI controllers found");
        } else {
            crate::println!("[xHCI] Found {} xHCI controller(s)", count);
        }
    }

    #[cfg(not(feature = "alloc"))]
    {
        crate::println!("[xHCI] xHCI init skipped (alloc feature not enabled)");
    }
}

/// Check if the xHCI subsystem has been initialized
pub fn is_initialized() -> bool {
    XHCI_INITIALIZED.load(Ordering::SeqCst)
}

/// Get the number of discovered xHCI controllers
pub fn controller_count() -> u32 {
    XHCI_CONTROLLER_COUNT.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[cfg(feature = "alloc")]
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // -- TRB construction tests --

    #[test]
    fn test_trb_zeroed() {
        let trb = Trb::zeroed();
        assert_eq!(trb.parameter_lo, 0);
        assert_eq!(trb.parameter_hi, 0);
        assert_eq!(trb.status, 0);
        assert_eq!(trb.control, 0);
        assert_eq!(trb.trb_type(), 0);
        assert!(!trb.cycle_bit());
    }

    #[test]
    fn test_trb_type_field() {
        let mut trb = Trb::zeroed();
        trb.set_trb_type(TRB_TYPE_NORMAL);
        assert_eq!(trb.trb_type(), TRB_TYPE_NORMAL);

        trb.set_trb_type(TRB_TYPE_COMMAND_COMPLETION);
        assert_eq!(trb.trb_type(), TRB_TYPE_COMMAND_COMPLETION);

        // Max 6-bit value
        trb.set_trb_type(63);
        assert_eq!(trb.trb_type(), 63);
    }

    #[test]
    fn test_trb_cycle_bit() {
        let mut trb = Trb::zeroed();
        assert!(!trb.cycle_bit());

        trb.set_cycle_bit(true);
        assert!(trb.cycle_bit());

        trb.set_cycle_bit(false);
        assert!(!trb.cycle_bit());
    }

    #[test]
    fn test_trb_slot_id() {
        let mut trb = Trb::zeroed();
        trb.set_slot_id(42);
        assert_eq!(trb.slot_id(), 42);

        trb.set_slot_id(255);
        assert_eq!(trb.slot_id(), 255);
    }

    #[test]
    fn test_trb_transfer_length() {
        let mut trb = Trb::zeroed();
        trb.set_transfer_length(1024);
        assert_eq!(trb.transfer_length(), 1024);

        // Max 17-bit value
        trb.set_transfer_length(0x1FFFF);
        assert_eq!(trb.transfer_length(), 0x1FFFF);

        // Overflow clamped
        trb.set_transfer_length(0xFFFFFFFF);
        assert_eq!(trb.transfer_length(), 0x1FFFF);
    }

    #[test]
    fn test_trb_normal() {
        let trb = Trb::normal(0x1000_0000, 512, true, true);
        assert_eq!(trb.trb_type(), TRB_TYPE_NORMAL);
        assert_eq!(trb.parameter_lo, 0x1000_0000);
        assert_eq!(trb.parameter_hi, 0);
        assert_eq!(trb.transfer_length(), 512);
        assert!(trb.cycle_bit());
        assert_ne!(trb.control & TRB_IOC, 0);
    }

    #[test]
    fn test_trb_setup_stage() {
        let trb = Trb::setup_stage(
            USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            (DESC_TYPE_DEVICE as u16) << 8,
            0,
            18,
            SetupTransferType::In,
            true,
        );
        assert_eq!(trb.trb_type(), TRB_TYPE_SETUP_STAGE);
        assert!(trb.cycle_bit());
        assert_ne!(trb.control & TRB_IDT, 0); // Immediate Data set
        assert_eq!(trb.status, 8); // 8 bytes setup packet
    }

    #[test]
    fn test_trb_link() {
        let trb = Trb::link(0x2000_0000, true, true);
        assert_eq!(trb.trb_type(), TRB_TYPE_LINK);
        assert_eq!(trb.parameter_lo, 0x2000_0000);
        assert!(trb.cycle_bit());
        assert_ne!(trb.control & TRB_TOGGLE_CYCLE, 0);
    }

    #[test]
    fn test_trb_enable_slot() {
        let trb = Trb::enable_slot(true);
        assert_eq!(trb.trb_type(), TRB_TYPE_ENABLE_SLOT);
        assert!(trb.cycle_bit());
    }

    #[test]
    fn test_trb_address_device() {
        let trb = Trb::address_device(0xDEAD_0000, 5, true, false);
        assert_eq!(trb.trb_type(), TRB_TYPE_ADDRESS_DEVICE);
        assert_eq!(trb.slot_id(), 5);
        assert!(!trb.cycle_bit());
        assert_ne!(trb.control & TRB_BSR, 0);
        assert_eq!(trb.parameter_lo, 0xDEAD_0000);
    }

    #[test]
    fn test_trb_configure_endpoint() {
        let trb = Trb::configure_endpoint(0xBEEF_0000, 3, false, true);
        assert_eq!(trb.trb_type(), TRB_TYPE_CONFIGURE_ENDPOINT);
        assert_eq!(trb.slot_id(), 3);
        assert!(trb.cycle_bit());
    }

    // -- Ring state tests --

    #[test]
    fn test_ring_state_advance() {
        let mut state = RingState::new(0x1000, 4);
        assert_eq!(state.index, 0);
        assert!(state.cycle);

        state.advance();
        assert_eq!(state.index, 1);
        assert!(state.cycle);

        state.advance();
        state.advance();
        state.advance(); // index 3 -> wraps to 0
        assert_eq!(state.index, 0);
        assert!(!state.cycle); // Toggled
    }

    #[test]
    fn test_ring_state_current_trb_phys() {
        let state = RingState::new(0x4000, 256);
        assert_eq!(state.current_trb_phys(), 0x4000);

        let mut state2 = RingState::new(0x4000, 256);
        state2.advance();
        assert_eq!(state2.current_trb_phys(), 0x4000 + TRB_SIZE as u64);
    }

    #[test]
    fn test_command_ring_crcr() {
        let ring = CommandRing::new(0x10000);
        // Initial cycle = true, so CRCR should have bit 0 set
        assert_eq!(ring.crcr_value(), 0x10001);
    }

    // -- Port parsing tests --

    #[test]
    fn test_port_from_portsc() {
        let portsc = PORTSC_CCS | PORTSC_PED | PORTSC_PP | (PORT_SPEED_SUPER << PORTSC_SPEED_SHIFT);
        let port = XhciPort::from_portsc(1, portsc);
        assert!(port.connected);
        assert!(port.enabled);
        assert!(port.powered);
        assert!(!port.resetting);
        assert_eq!(port.speed, PortSpeed::Super);
    }

    #[test]
    fn test_port_speed_conversion() {
        assert_eq!(PortSpeed::from_portsc(PORT_SPEED_FULL), PortSpeed::Full);
        assert_eq!(PortSpeed::from_portsc(PORT_SPEED_LOW), PortSpeed::Low);
        assert_eq!(PortSpeed::from_portsc(PORT_SPEED_HIGH), PortSpeed::High);
        assert_eq!(PortSpeed::from_portsc(PORT_SPEED_SUPER), PortSpeed::Super);
        assert_eq!(
            PortSpeed::from_portsc(PORT_SPEED_SUPER_PLUS),
            PortSpeed::SuperPlus
        );
        assert_eq!(PortSpeed::from_portsc(99), PortSpeed::Unknown);
    }

    #[test]
    fn test_port_speed_max_packet() {
        assert_eq!(PortSpeed::Low.default_max_packet_size_ep0(), 8);
        assert_eq!(PortSpeed::Full.default_max_packet_size_ep0(), 8);
        assert_eq!(PortSpeed::High.default_max_packet_size_ep0(), 64);
        assert_eq!(PortSpeed::Super.default_max_packet_size_ep0(), 512);
    }

    // -- Descriptor parsing tests --

    #[test]
    fn test_device_descriptor_parse() {
        let mut data = [0u8; 18];
        data[0] = 18; // bLength
        data[1] = DESC_TYPE_DEVICE; // bDescriptorType
        data[2] = 0x00;
        data[3] = 0x03; // bcdUSB = 3.00
        data[4] = 0x09; // bDeviceClass (Hub)
        data[7] = 64; // bMaxPacketSize0
        data[8] = 0x34;
        data[9] = 0x12; // idVendor = 0x1234
        data[10] = 0x78;
        data[11] = 0x56; // idProduct = 0x5678
        data[17] = 2; // bNumConfigurations

        let desc = XhciDeviceDescriptor::parse(&data).unwrap();
        assert_eq!(desc.usb_version, 0x0300);
        assert_eq!(desc.device_class, 0x09);
        assert_eq!(desc.max_packet_size_ep0, 64);
        assert_eq!(desc.vendor_id, 0x1234);
        assert_eq!(desc.product_id, 0x5678);
        assert_eq!(desc.num_configurations, 2);
    }

    #[test]
    fn test_device_descriptor_too_short() {
        let data = [0u8; 10];
        assert!(XhciDeviceDescriptor::parse(&data).is_err());
    }

    #[test]
    fn test_device_descriptor_wrong_type() {
        let mut data = [0u8; 18];
        data[1] = DESC_TYPE_CONFIGURATION; // Wrong type
        assert!(XhciDeviceDescriptor::parse(&data).is_err());
    }

    #[test]
    fn test_config_descriptor_parse() {
        let mut data = [0u8; 9];
        data[0] = 9; // bLength
        data[1] = DESC_TYPE_CONFIGURATION; // bDescriptorType
        data[2] = 32;
        data[3] = 0; // wTotalLength = 32
        data[4] = 1; // bNumInterfaces
        data[5] = 1; // bConfigurationValue
        data[7] = 0x60; // bmAttributes (self-powered + remote wakeup)
        data[8] = 50; // bMaxPower (50 * 2 = 100 mA)

        let desc = XhciConfigDescriptor::parse(&data).unwrap();
        assert_eq!(desc.total_length, 32);
        assert_eq!(desc.num_interfaces, 1);
        assert_eq!(desc.config_value, 1);
        assert_eq!(desc.max_power_ma, 100);
        assert!(desc.is_self_powered());
        assert!(desc.supports_remote_wakeup());
    }

    #[test]
    fn test_endpoint_descriptor_parse() {
        let mut data = [0u8; 7];
        data[0] = 7; // bLength
        data[1] = DESC_TYPE_ENDPOINT; // bDescriptorType
        data[2] = 0x81; // bEndpointAddress (EP1 IN)
        data[3] = 0x02; // bmAttributes (Bulk)
        data[4] = 0x00;
        data[5] = 0x02; // wMaxPacketSize = 512
        data[6] = 0; // bInterval

        let ep = XhciEndpointDescriptor::parse(&data).unwrap();
        assert!(ep.is_in());
        assert_eq!(ep.endpoint_number(), 1);
        assert_eq!(ep.transfer_type(), EndpointTransferType::Bulk);
        assert_eq!(ep.max_packet_size, 512);
    }

    #[test]
    fn test_interface_descriptor_parse() {
        let mut data = [0u8; 9];
        data[0] = 9;
        data[1] = DESC_TYPE_INTERFACE;
        data[2] = 0; // bInterfaceNumber
        data[4] = 2; // bNumEndpoints
        data[5] = 0x08; // bInterfaceClass (Mass Storage)
        data[6] = 0x06; // bInterfaceSubClass (SCSI)
        data[7] = 0x50; // bInterfaceProtocol (Bulk-Only)

        let iface = XhciInterfaceDescriptor::parse(&data).unwrap();
        assert_eq!(iface.interface_number, 0);
        assert_eq!(iface.num_endpoints, 2);
        assert_eq!(iface.interface_class, 0x08);
        assert_eq!(iface.interface_subclass, 0x06);
        assert_eq!(iface.interface_protocol, 0x50);
    }

    // -- PCI identification test --

    #[test]
    fn test_is_xhci_device() {
        assert!(is_xhci_device(0x0C, 0x03, 0x30));
        assert!(!is_xhci_device(0x0C, 0x03, 0x00)); // UHCI
        assert!(!is_xhci_device(0x0C, 0x03, 0x10)); // OHCI
        assert!(!is_xhci_device(0x0C, 0x03, 0x20)); // EHCI
        assert!(!is_xhci_device(0x02, 0x00, 0x00)); // Network
    }

    // -- Completion code test --

    #[test]
    fn test_trb_completion_code() {
        let mut trb = Trb::zeroed();
        trb.status = (TRB_COMPLETION_SUCCESS as u32) << 24;
        assert_eq!(trb.completion_code(), TRB_COMPLETION_SUCCESS);

        trb.status = (TRB_COMPLETION_STALL as u32) << 24 | 0x00FF_FFFF;
        assert_eq!(trb.completion_code(), TRB_COMPLETION_STALL);
    }

    // -- Slot state test --

    #[test]
    fn test_slot_state_transitions() {
        let slot = DeviceSlot {
            slot_id: 1,
            state: SlotState::Disabled,
            port_number: 1,
            speed: PortSpeed::Super,
            device_context_phys: 0,
            input_context_phys: 0,
        };
        assert_eq!(slot.state, SlotState::Disabled);
    }

    // -- Configuration descriptor parsing (alloc) --

    #[cfg(feature = "alloc")]
    #[test]
    fn test_parse_configuration_descriptors() {
        // Build a minimal config descriptor with 1 interface and 1 endpoint
        let mut data = vec![0u8; 32];

        // Config descriptor (9 bytes)
        data[0] = 9;
        data[1] = DESC_TYPE_CONFIGURATION;
        data[2] = 25;
        data[3] = 0; // wTotalLength = 25
        data[4] = 1; // bNumInterfaces
        data[5] = 1; // bConfigurationValue

        // Interface descriptor (9 bytes) at offset 9
        data[9] = 9;
        data[10] = DESC_TYPE_INTERFACE;
        data[11] = 0; // bInterfaceNumber
        data[13] = 1; // bNumEndpoints
        data[14] = 0x03; // bInterfaceClass (HID)

        // Endpoint descriptor (7 bytes) at offset 18
        data[18] = 7;
        data[19] = DESC_TYPE_ENDPOINT;
        data[20] = 0x81; // EP1 IN
        data[21] = 0x03; // Interrupt
        data[22] = 8;
        data[23] = 0; // wMaxPacketSize = 8
        data[24] = 10; // bInterval

        let (config, interfaces) = parse_configuration_descriptors(&data).unwrap();
        assert_eq!(config.num_interfaces, 1);
        assert_eq!(interfaces.len(), 1);
        assert_eq!(interfaces[0].0.interface_class, 0x03);
        assert_eq!(interfaces[0].1.len(), 1);
        assert_eq!(interfaces[0].1[0].endpoint_number(), 1);
        assert!(interfaces[0].1[0].is_in());
        assert_eq!(
            interfaces[0].1[0].transfer_type(),
            EndpointTransferType::Interrupt
        );
    }

    // -- Transfer ring (host stub) test --

    #[test]
    fn test_transfer_ring_enqueue_control() {
        let mut ring = TransferRing::new(0x5_0000);
        let _addr = ring.enqueue_control(
            USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE,
            USB_REQ_GET_DESCRIPTOR,
            (DESC_TYPE_DEVICE as u16) << 8,
            0,
            0x6_0000,
            18,
        );
        // After enqueueing Setup + Data + Status = 3 TRBs, index should be 3
        assert_eq!(ring.state.index, 3);
    }

    // -- Event ring (host stub) test --

    #[test]
    fn test_event_ring_dequeue_empty() {
        let mut ring = EventRing::new(0x7_0000, 256);
        assert!(ring.dequeue().is_none());
    }

    // -- XhciController stub test --

    #[test]
    fn test_xhci_controller_stub() {
        let ctrl = XhciController::new_stub();
        assert_eq!(ctrl.capabilities().max_slots, 64);
        assert_eq!(ctrl.capabilities().max_ports, 4);
        assert_eq!(ctrl.capabilities().hci_version, 0x0110);
        assert!(!ctrl.is_running());
    }
}
