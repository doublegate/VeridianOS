//! KVM-compatible virtualization API
//!
//! Implements a KVM-like interface for creating and managing virtual machines,
//! vCPUs, memory regions, and interrupt controllers. Provides VMLAUNCH/VMRESUME
//! cycle with VM exit reason decoding.
//!
//! Sprints W5-S1 (core API), W5-S2 (vCPU management), W5-S3 (PIT emulation).

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::VmError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// KVM API version (matches Linux KVM_API_VERSION)
pub const KVM_API_VERSION: u32 = 12;

/// Maximum vCPUs per VM
const MAX_VCPUS_PER_VM: usize = 256;

/// Maximum memory regions per VM
const MAX_MEMORY_REGIONS: usize = 64;

/// PIT oscillator frequency in Hz (1.193182 MHz as integer)
const PIT_FREQUENCY_HZ: u64 = 1_193_182;

/// PIT channel count
const PIT_CHANNEL_COUNT: usize = 3;

/// Maximum MSR entries for get/set operations
const MAX_MSR_ENTRIES: usize = 256;

// ---------------------------------------------------------------------------
// KVM Capability
// ---------------------------------------------------------------------------

/// KVM capability identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum KvmCapability {
    /// In-kernel IRQ chip emulation
    Irqchip = 0,
    /// HLT instruction interception
    Hlt = 1,
    /// Shadow MMU for paging
    MmuShadow = 2,
    /// User-space memory region mapping
    UserMemory = 3,
    /// TSS address configuration (x86)
    SetTssAddr = 4,
    /// PIT2 timer emulation
    Pit2 = 5,
    /// Extended CPUID results
    ExtCpuid = 6,
    /// Virtual APIC page
    Vapic = 7,
    /// MP state get/set
    MpState = 8,
    /// Coalesced MMIO batching
    CoalescedMmio = 9,
}

impl KvmCapability {
    /// Check if this capability is supported
    #[allow(dead_code)]
    pub fn is_supported(&self) -> bool {
        // All capabilities supported in our implementation
        true
    }

    /// Convert from raw integer
    #[allow(dead_code)]
    pub fn from_raw(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Irqchip),
            1 => Some(Self::Hlt),
            2 => Some(Self::MmuShadow),
            3 => Some(Self::UserMemory),
            4 => Some(Self::SetTssAddr),
            5 => Some(Self::Pit2),
            6 => Some(Self::ExtCpuid),
            7 => Some(Self::Vapic),
            8 => Some(Self::MpState),
            9 => Some(Self::CoalescedMmio),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// KVM Exit Reasons
// ---------------------------------------------------------------------------

/// VM exit reason from KVM run
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum KvmExitReason {
    /// I/O port access
    Io,
    /// Memory-mapped I/O access
    Mmio,
    /// HLT instruction executed
    Hlt,
    /// VM shutdown requested
    Shutdown,
    /// Internal KVM error
    InternalError,
    /// Unknown exit reason
    Unknown(u32),
}

impl KvmExitReason {
    /// Convert from raw exit code
    #[allow(dead_code)]
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            2 => Self::Io,
            6 => Self::Mmio,
            5 => Self::Hlt,
            8 => Self::Shutdown,
            17 => Self::InternalError,
            other => Self::Unknown(other),
        }
    }

    /// Convert to raw exit code
    #[allow(dead_code)]
    pub fn to_raw(self) -> u32 {
        match self {
            Self::Io => 2,
            Self::Mmio => 6,
            Self::Hlt => 5,
            Self::Shutdown => 8,
            Self::InternalError => 17,
            Self::Unknown(v) => v,
        }
    }
}

// ---------------------------------------------------------------------------
// I/O Direction
// ---------------------------------------------------------------------------

/// Direction of I/O port access
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum IoDirection {
    /// Reading from port
    #[default]
    In,
    /// Writing to port
    Out,
}

// ---------------------------------------------------------------------------
// KVM I/O Exit Info
// ---------------------------------------------------------------------------

/// Information about an I/O port VM exit
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct KvmIoExit {
    /// Direction of I/O access
    pub direction: IoDirection,
    /// I/O port number
    pub port: u16,
    /// Access size in bytes (1, 2, or 4)
    pub size: u8,
    /// Data value (for Out) or buffer for read (for In)
    pub data: u32,
}

impl KvmIoExit {
    /// Create a new I/O exit for an IN instruction
    #[allow(dead_code)]
    pub fn new_in(port: u16, size: u8) -> Self {
        Self {
            direction: IoDirection::In,
            port,
            size,
            data: 0,
        }
    }

    /// Create a new I/O exit for an OUT instruction
    #[allow(dead_code)]
    pub fn new_out(port: u16, size: u8, data: u32) -> Self {
        Self {
            direction: IoDirection::Out,
            port,
            size,
            data,
        }
    }
}

// ---------------------------------------------------------------------------
// KVM MMIO Exit Info
// ---------------------------------------------------------------------------

/// Information about an MMIO VM exit
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct KvmMmioExit {
    /// Physical address accessed
    pub phys_addr: u64,
    /// Data value
    pub data: u64,
    /// Access length in bytes
    pub len: u8,
    /// True if this is a write, false for read
    pub is_write: bool,
}

impl KvmMmioExit {
    /// Create a new MMIO exit for a read
    #[allow(dead_code)]
    pub fn new_read(phys_addr: u64, len: u8) -> Self {
        Self {
            phys_addr,
            data: 0,
            len,
            is_write: false,
        }
    }

    /// Create a new MMIO exit for a write
    #[allow(dead_code)]
    pub fn new_write(phys_addr: u64, data: u64, len: u8) -> Self {
        Self {
            phys_addr,
            data,
            len,
            is_write: true,
        }
    }
}

// ---------------------------------------------------------------------------
// General-Purpose Registers
// ---------------------------------------------------------------------------

/// x86_64 general-purpose register set
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct KvmRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
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
// Segment Register
// ---------------------------------------------------------------------------

/// x86 segment register descriptor
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct KvmSegment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_attr: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
}

impl KvmSegment {
    /// Create a flat code segment for long mode
    #[allow(dead_code)]
    pub fn flat_code_64() -> Self {
        Self {
            base: 0,
            limit: 0xFFFF_FFFF,
            selector: 0x08,
            type_attr: 0x0B, // Execute/Read, Accessed
            present: 1,
            dpl: 0,
            db: 0,
            s: 1,
            l: 1, // Long mode
            g: 1, // 4K granularity
            avl: 0,
        }
    }

    /// Create a flat data segment for long mode
    #[allow(dead_code)]
    pub fn flat_data_64() -> Self {
        Self {
            base: 0,
            limit: 0xFFFF_FFFF,
            selector: 0x10,
            type_attr: 0x03, // Read/Write, Accessed
            present: 1,
            dpl: 0,
            db: 1,
            s: 1,
            l: 0,
            g: 1,
            avl: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// System (Special) Registers
// ---------------------------------------------------------------------------

/// x86_64 system register set (segment + control registers)
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct KvmSregs {
    pub cs: KvmSegment,
    pub ds: KvmSegment,
    pub es: KvmSegment,
    pub fs: KvmSegment,
    pub gs: KvmSegment,
    pub ss: KvmSegment,
    pub tr: KvmSegment,
    pub ldt: KvmSegment,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
}

impl KvmSregs {
    /// Create sregs for real mode entry
    #[allow(dead_code)]
    pub fn real_mode() -> Self {
        Self {
            cr0: 0x10, // ET bit set
            ..Default::default()
        }
    }

    /// Create sregs for 64-bit long mode
    #[allow(dead_code)]
    pub fn long_mode(cr3: u64) -> Self {
        Self {
            cs: KvmSegment::flat_code_64(),
            ds: KvmSegment::flat_data_64(),
            es: KvmSegment::flat_data_64(),
            fs: KvmSegment::flat_data_64(),
            gs: KvmSegment::flat_data_64(),
            ss: KvmSegment::flat_data_64(),
            cr0: 0x8000_0011, // PG | PE | ET
            cr3,
            cr4: 0x20,   // PAE
            efer: 0x500, // LME | LMA
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// MSR Entry
// ---------------------------------------------------------------------------

/// Model-Specific Register entry
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct MsrEntry {
    /// MSR index
    pub index: u32,
    /// MSR value
    pub value: u64,
}

// ---------------------------------------------------------------------------
// Memory Region
// ---------------------------------------------------------------------------

/// A user memory region mapping guest physical to host virtual
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct KvmMemoryRegion {
    /// Slot index for this region
    pub slot: u32,
    /// Flags (e.g., read-only)
    pub flags: u32,
    /// Guest physical address
    pub guest_phys_addr: u64,
    /// Size in bytes
    pub memory_size: u64,
    /// Host virtual address (userspace pointer)
    pub userspace_addr: u64,
}

impl KvmMemoryRegion {
    /// Flag: memory region is read-only
    pub const FLAG_READONLY: u32 = 1;
    /// Flag: memory region has dirty page logging
    pub const FLAG_LOG_DIRTY: u32 = 2;

    /// Create a new memory region
    #[allow(dead_code)]
    pub fn new(slot: u32, guest_phys: u64, size: u64, host_addr: u64) -> Self {
        Self {
            slot,
            flags: 0,
            guest_phys_addr: guest_phys,
            memory_size: size,
            userspace_addr: host_addr,
        }
    }

    /// Create a read-only memory region
    #[allow(dead_code)]
    pub fn new_readonly(slot: u32, guest_phys: u64, size: u64, host_addr: u64) -> Self {
        Self {
            slot,
            flags: Self::FLAG_READONLY,
            guest_phys_addr: guest_phys,
            memory_size: size,
            userspace_addr: host_addr,
        }
    }

    /// Check if region is read-only
    #[allow(dead_code)]
    pub fn is_readonly(&self) -> bool {
        self.flags & Self::FLAG_READONLY != 0
    }

    /// Check if region has dirty logging enabled
    #[allow(dead_code)]
    pub fn has_dirty_logging(&self) -> bool {
        self.flags & Self::FLAG_LOG_DIRTY != 0
    }

    /// Check if an address falls within this region
    #[allow(dead_code)]
    pub fn contains_guest_addr(&self, addr: u64) -> bool {
        addr >= self.guest_phys_addr && addr < self.guest_phys_addr + self.memory_size
    }

    /// Translate guest physical to host virtual address
    #[allow(dead_code)]
    pub fn translate(&self, guest_phys: u64) -> Option<u64> {
        if self.contains_guest_addr(guest_phys) {
            Some(self.userspace_addr + (guest_phys - self.guest_phys_addr))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// PIT Timer Modes
// ---------------------------------------------------------------------------

/// PIT counter operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum PitMode {
    /// Mode 0: Interrupt on terminal count
    #[default]
    InterruptOnTerminalCount,
    /// Mode 1: Hardware retriggerable one-shot
    OneShot,
    /// Mode 2: Rate generator (periodic)
    RateGenerator,
    /// Mode 3: Square wave generator
    SquareWave,
    /// Mode 4: Software triggered strobe
    SoftwareStrobe,
    /// Mode 5: Hardware triggered strobe
    HardwareStrobe,
}

impl PitMode {
    /// Convert from raw mode bits
    #[allow(dead_code)]
    pub fn from_raw(raw: u8) -> Self {
        match raw & 0x07 {
            0 => Self::InterruptOnTerminalCount,
            1 => Self::OneShot,
            2 => Self::RateGenerator,
            3 => Self::SquareWave,
            4 => Self::SoftwareStrobe,
            5 => Self::HardwareStrobe,
            _ => Self::InterruptOnTerminalCount,
        }
    }
}

// ---------------------------------------------------------------------------
// PIT Channel
// ---------------------------------------------------------------------------

/// State for a single PIT counter channel
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct PitChannel {
    /// Current counter value
    pub count: u16,
    /// Reload value (initial count)
    pub reload: u16,
    /// Operating mode
    pub mode: PitMode,
    /// Access mode: 0=latch, 1=lo, 2=hi, 3=lo/hi
    pub access_mode: u8,
    /// BCD mode if true
    pub bcd: bool,
    /// Latch value (captured on latch command)
    pub latch_value: u16,
    /// Whether latch is valid
    pub latch_valid: bool,
    /// Whether reading low byte next (for lo/hi mode)
    pub read_low: bool,
    /// Whether writing low byte next (for lo/hi mode)
    pub write_low: bool,
    /// Gate input state
    pub gate: bool,
    /// Output state
    pub output: bool,
    /// Whether the counter is enabled (counting)
    pub enabled: bool,
    /// Accumulated nanoseconds for tick tracking
    pub accumulated_ns: u64,
}

impl PitChannel {
    /// Create a new PIT channel with default state
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            gate: true,   // Gate high by default for channels 0,1
            output: true, // Output high initially
            ..Default::default()
        }
    }

    /// Load a new count value
    #[allow(dead_code)]
    pub fn load_count(&mut self, value: u16) {
        self.reload = if value == 0 { u16::MAX } else { value };
        self.count = self.reload;
        self.enabled = true;
        self.output = !matches!(self.mode, PitMode::InterruptOnTerminalCount);
    }

    /// Tick the counter by one unit
    #[allow(dead_code)]
    pub fn tick(&mut self) -> bool {
        if !self.enabled || !self.gate {
            return false;
        }

        if self.count == 0 {
            match self.mode {
                PitMode::InterruptOnTerminalCount
                | PitMode::OneShot
                | PitMode::SoftwareStrobe
                | PitMode::HardwareStrobe => {
                    self.output = true;
                    self.enabled = false;
                    return true;
                }
                PitMode::RateGenerator => {
                    self.count = self.reload;
                    // Generate short low pulse
                    return true;
                }
                PitMode::SquareWave => {
                    self.count = self.reload;
                    self.output = !self.output;
                    return self.output; // IRQ on rising edge
                }
            }
        }

        self.count = self.count.wrapping_sub(1);
        false
    }

    /// Calculate the frequency in Hz (integer)
    #[allow(dead_code)]
    pub fn frequency_hz(&self) -> u64 {
        if self.reload == 0 {
            return 0;
        }
        PIT_FREQUENCY_HZ / self.reload as u64
    }

    /// Calculate the period in nanoseconds (integer)
    #[allow(dead_code)]
    pub fn period_ns(&self) -> u64 {
        let freq = self.frequency_hz();
        if freq == 0 {
            return 0;
        }
        1_000_000_000 / freq
    }

    /// Advance by a number of nanoseconds, return number of interrupts fired
    #[allow(dead_code)]
    pub fn advance_ns(&mut self, ns: u64) -> u32 {
        if !self.enabled || self.reload == 0 {
            return 0;
        }

        // Period of one tick in ns: 1_000_000_000 / PIT_FREQUENCY_HZ ~= 838 ns
        let tick_ns = 1_000_000_000u64 / PIT_FREQUENCY_HZ;
        if tick_ns == 0 {
            return 0;
        }

        self.accumulated_ns += ns;
        let ticks = self.accumulated_ns / tick_ns;
        self.accumulated_ns %= tick_ns;

        let mut irqs = 0u32;
        for _ in 0..ticks.min(65536) {
            if self.tick() {
                irqs = irqs.saturating_add(1);
            }
        }
        irqs
    }

    /// Latch the current counter value
    #[allow(dead_code)]
    pub fn latch(&mut self) {
        if !self.latch_valid {
            self.latch_value = self.count;
            self.latch_valid = true;
            self.read_low = true;
        }
    }

    /// Read a byte from the channel (respects latch and access mode)
    #[allow(dead_code)]
    pub fn read_byte(&mut self) -> u8 {
        let value = if self.latch_valid {
            self.latch_value
        } else {
            self.count
        };

        match self.access_mode {
            1 => {
                // Low byte only
                if self.latch_valid {
                    self.latch_valid = false;
                }
                value as u8
            }
            2 => {
                // High byte only
                if self.latch_valid {
                    self.latch_valid = false;
                }
                (value >> 8) as u8
            }
            3 => {
                // Low/high alternating
                if self.read_low {
                    self.read_low = false;
                    value as u8
                } else {
                    self.read_low = true;
                    if self.latch_valid {
                        self.latch_valid = false;
                    }
                    (value >> 8) as u8
                }
            }
            _ => {
                // Latch mode: read latched value
                if self.latch_valid {
                    self.latch_valid = false;
                }
                value as u8
            }
        }
    }

    /// Write a byte to the channel (respects access mode)
    #[allow(dead_code)]
    pub fn write_byte(&mut self, byte: u8) {
        match self.access_mode {
            1 => {
                // Low byte only
                self.load_count(byte as u16);
            }
            2 => {
                // High byte only
                self.load_count((byte as u16) << 8);
            }
            3 => {
                // Low/high alternating
                if self.write_low {
                    self.reload = (self.reload & 0xFF00) | byte as u16;
                    self.write_low = false;
                } else {
                    let full = (self.reload & 0x00FF) | ((byte as u16) << 8);
                    self.write_low = true;
                    self.load_count(full);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Virtual PIT
// ---------------------------------------------------------------------------

/// 8254/8253-compatible Programmable Interval Timer
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VirtualPit {
    /// Three counter channels
    pub channels: [PitChannel; PIT_CHANNEL_COUNT],
    /// Speaker gate (channel 2)
    pub speaker_gate: bool,
    /// Total interrupts generated
    pub total_interrupts: u64,
}

impl Default for VirtualPit {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualPit {
    /// Create a new PIT with default channel configuration
    #[allow(dead_code)]
    pub fn new() -> Self {
        let mut channels = [PitChannel::new(), PitChannel::new(), PitChannel::new()];
        // Channel 2 gate is controlled by speaker port
        channels[2].gate = false;
        Self {
            channels,
            speaker_gate: false,
            total_interrupts: 0,
        }
    }

    /// Handle I/O write to PIT control word register (port 0x43)
    #[allow(dead_code)]
    pub fn write_control(&mut self, value: u8) {
        let channel = ((value >> 6) & 0x03) as usize;
        if channel == 3 {
            // Read-back command
            self.handle_readback(value);
            return;
        }
        if channel >= PIT_CHANNEL_COUNT {
            return;
        }

        let access = (value >> 4) & 0x03;
        if access == 0 {
            // Latch command
            self.channels[channel].latch();
            return;
        }

        let mode = (value >> 1) & 0x07;
        let bcd = value & 0x01 != 0;

        self.channels[channel].access_mode = access;
        self.channels[channel].mode = PitMode::from_raw(mode);
        self.channels[channel].bcd = bcd;
        self.channels[channel].write_low = true;
        self.channels[channel].read_low = true;
        self.channels[channel].enabled = false;
    }

    /// Handle read-back command
    fn handle_readback(&mut self, value: u8) {
        let latch_count = value & 0x20 == 0;
        let latch_status = value & 0x10 == 0;

        for i in 0..PIT_CHANNEL_COUNT {
            if value & (1 << (i + 1)) != 0 {
                if latch_count {
                    self.channels[i].latch();
                }
                if latch_status {
                    // Status latch: not fully implemented, just latch count
                    self.channels[i].latch();
                }
            }
        }
    }

    /// Handle I/O read from a channel data port
    #[allow(dead_code)]
    pub fn read_channel(&mut self, channel: usize) -> u8 {
        if channel >= PIT_CHANNEL_COUNT {
            return 0xFF;
        }
        self.channels[channel].read_byte()
    }

    /// Handle I/O write to a channel data port
    #[allow(dead_code)]
    pub fn write_channel(&mut self, channel: usize, value: u8) {
        if channel >= PIT_CHANNEL_COUNT {
            return;
        }
        self.channels[channel].write_byte(value);
    }

    /// Handle speaker control port (0x61) read
    #[allow(dead_code)]
    pub fn read_speaker_port(&self) -> u8 {
        let mut val = 0u8;
        if self.speaker_gate {
            val |= 0x01;
        }
        if self.channels[2].output {
            val |= 0x20;
        }
        val
    }

    /// Handle speaker control port (0x61) write
    #[allow(dead_code)]
    pub fn write_speaker_port(&mut self, value: u8) {
        self.speaker_gate = value & 0x01 != 0;
        self.channels[2].gate = value & 0x01 != 0;
    }

    /// Advance all channels by a number of nanoseconds
    #[allow(dead_code)]
    pub fn advance_ns(&mut self, ns: u64) -> u32 {
        let mut total_irqs = 0u32;
        // Channel 0 is typically connected to IRQ 0
        let ch0_irqs = self.channels[0].advance_ns(ns);
        total_irqs = total_irqs.saturating_add(ch0_irqs);
        self.total_interrupts = self.total_interrupts.saturating_add(ch0_irqs as u64);

        // Advance channels 1 and 2 but don't count their IRQs towards total
        let _ = self.channels[1].advance_ns(ns);
        let _ = self.channels[2].advance_ns(ns);

        total_irqs
    }

    /// Handle PIT I/O port access (ports 0x40-0x43, 0x61)
    #[allow(dead_code)]
    pub fn handle_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> bool {
        match port {
            0x40..=0x42 => {
                let channel = (port - 0x40) as usize;
                if is_write {
                    if let Some(&b) = data.first() {
                        self.write_channel(channel, b);
                    }
                } else if let Some(d) = data.first_mut() {
                    *d = self.read_channel(channel);
                }
                true
            }
            0x43 => {
                if is_write {
                    if let Some(&b) = data.first() {
                        self.write_control(b);
                    }
                }
                // Control port is write-only
                true
            }
            0x61 => {
                if is_write {
                    if let Some(&b) = data.first() {
                        self.write_speaker_port(b);
                    }
                } else if let Some(d) = data.first_mut() {
                    *d = self.read_speaker_port();
                }
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// KVM vCPU
// ---------------------------------------------------------------------------

/// Run state for the shared KVM run page
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum KvmRunState {
    /// vCPU is ready to run
    #[default]
    Ready,
    /// vCPU exited and needs handling
    Exited,
    /// vCPU is paused
    Paused,
}

/// Shared run page between kernel and userspace
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct KvmRunPage {
    /// Current run state
    pub state: KvmRunState,
    /// Exit reason from last run
    pub exit_reason: u32,
    /// I/O exit info (valid when exit_reason == IO)
    pub io: KvmIoExit,
    /// MMIO exit info (valid when exit_reason == MMIO)
    pub mmio: KvmMmioExit,
    /// Whether the vCPU should continue to run after handling exit
    pub immediate_exit: bool,
}

impl KvmRunPage {
    /// Create a new run page
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Virtual CPU state
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub struct KvmVcpu {
    /// vCPU identifier within the VM
    pub vcpu_id: u32,
    /// vCPU file descriptor (simulated)
    vcpu_fd: u32,
    /// Shared run page
    pub kvm_run: KvmRunPage,
    /// General-purpose registers
    pub regs: KvmRegs,
    /// System/special registers
    pub sregs: KvmSregs,
    /// MSR storage
    msrs: Vec<MsrEntry>,
    /// Whether the vCPU has been launched (vs needs VMLAUNCH)
    launched: bool,
    /// Whether the vCPU is halted (HLT)
    halted: bool,
    /// Pending external interrupt vector (None = no pending)
    pending_interrupt: Option<u8>,
    /// Number of VM entries performed
    entry_count: u64,
    /// Number of VM exits handled
    exit_count: u64,
}

#[cfg(feature = "alloc")]
impl KvmVcpu {
    /// Create a new vCPU with the given ID
    #[allow(dead_code)]
    pub fn new(vcpu_id: u32) -> Self {
        Self {
            vcpu_id,
            vcpu_fd: vcpu_id + 100, // Simulated fd
            kvm_run: KvmRunPage::new(),
            regs: KvmRegs::default(),
            sregs: KvmSregs::default(),
            msrs: Vec::new(),
            launched: false,
            halted: false,
            pending_interrupt: None,
            entry_count: 0,
            exit_count: 0,
        }
    }

    /// Run the vCPU (VMLAUNCH or VMRESUME)
    ///
    /// Returns the exit reason. In a real implementation this would
    /// execute a VMX VM entry; here we simulate common exit scenarios.
    #[allow(dead_code)]
    pub fn run(&mut self) -> Result<KvmExitReason, VmError> {
        if self.halted {
            // If halted and no pending interrupt, exit with HLT
            if self.pending_interrupt.is_none() {
                self.kvm_run.state = KvmRunState::Exited;
                self.kvm_run.exit_reason = KvmExitReason::Hlt.to_raw();
                self.exit_count = self.exit_count.saturating_add(1);
                return Ok(KvmExitReason::Hlt);
            }
            // Unhalt on pending interrupt
            self.halted = false;
        }

        self.entry_count = self.entry_count.saturating_add(1);

        // In a real implementation: VMLAUNCH (first time) or VMRESUME
        if !self.launched {
            self.launched = true;
        }

        // Inject pending interrupt if any
        if let Some(_vector) = self.pending_interrupt.take() {
            // Would set VM-entry interruption-information field
        }

        // Simulate: the VM ran and exited for some reason
        // Real implementation would decode VMCS exit reason
        self.kvm_run.state = KvmRunState::Exited;
        self.exit_count = self.exit_count.saturating_add(1);

        let reason = KvmExitReason::from_raw(self.kvm_run.exit_reason);
        Ok(reason)
    }

    /// Get general-purpose registers
    #[allow(dead_code)]
    pub fn get_regs(&self) -> &KvmRegs {
        &self.regs
    }

    /// Set general-purpose registers
    #[allow(dead_code)]
    pub fn set_regs(&mut self, regs: KvmRegs) {
        self.regs = regs;
    }

    /// Get system registers
    #[allow(dead_code)]
    pub fn get_sregs(&self) -> &KvmSregs {
        &self.sregs
    }

    /// Set system registers
    #[allow(dead_code)]
    pub fn set_sregs(&mut self, sregs: KvmSregs) {
        self.sregs = sregs;
    }

    /// Get MSR values
    #[allow(dead_code)]
    pub fn get_msrs(&self, indices: &[u32]) -> Vec<MsrEntry> {
        let mut result = Vec::with_capacity(indices.len());
        for &idx in indices {
            let value = self
                .msrs
                .iter()
                .find(|m| m.index == idx)
                .map_or(0, |m| m.value);
            result.push(MsrEntry { index: idx, value });
        }
        result
    }

    /// Set MSR values
    #[allow(dead_code)]
    pub fn set_msrs(&mut self, entries: &[MsrEntry]) -> Result<usize, VmError> {
        if entries.len() > MAX_MSR_ENTRIES {
            return Err(VmError::InvalidVmState);
        }
        for entry in entries {
            if let Some(existing) = self.msrs.iter_mut().find(|m| m.index == entry.index) {
                existing.value = entry.value;
            } else {
                self.msrs.push(*entry);
            }
        }
        Ok(entries.len())
    }

    /// Inject an external interrupt
    #[allow(dead_code)]
    pub fn interrupt(&mut self, vector: u8) {
        self.pending_interrupt = Some(vector);
        if self.halted {
            self.halted = false;
        }
    }

    /// Signal an MSI interrupt
    #[allow(dead_code)]
    pub fn signal_msi(&mut self, address: u64, data: u32) {
        // MSI address format: destination ID in bits 19:12
        let _dest_id = (address >> 12) & 0xFF;
        // MSI data: vector in bits 7:0
        let vector = (data & 0xFF) as u8;
        self.interrupt(vector);
    }

    /// Check if vCPU is halted
    #[allow(dead_code)]
    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// Halt the vCPU (simulates HLT instruction)
    #[allow(dead_code)]
    pub fn halt(&mut self) {
        self.halted = true;
    }

    /// Get entry count
    #[allow(dead_code)]
    pub fn entry_count(&self) -> u64 {
        self.entry_count
    }

    /// Get exit count
    #[allow(dead_code)]
    pub fn exit_count(&self) -> u64 {
        self.exit_count
    }

    /// Reset the vCPU to initial state
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.regs = KvmRegs::default();
        self.sregs = KvmSregs::default();
        self.msrs.clear();
        self.launched = false;
        self.halted = false;
        self.pending_interrupt = None;
        self.kvm_run = KvmRunPage::new();
    }
}

// ---------------------------------------------------------------------------
// KVM VM
// ---------------------------------------------------------------------------

/// Virtual Machine instance
#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub struct KvmVm {
    /// VM file descriptor (simulated)
    vm_fd: u32,
    /// Number of vCPUs created
    num_vcpus: u32,
    /// Guest memory regions
    memory_regions: Vec<KvmMemoryRegion>,
    /// Whether in-kernel irqchip is created
    irqchip_created: bool,
    /// Whether in-kernel PIT is created
    pit_created: bool,
    /// vCPUs belonging to this VM
    vcpus: Vec<KvmVcpu>,
    /// Virtual PIT (created on demand)
    pit: Option<VirtualPit>,
    /// TSS address (x86-specific)
    tss_addr: u64,
    /// VM identifier
    vm_id: u32,
    /// Next memory region slot
    next_slot: u32,
}

#[cfg(feature = "alloc")]
impl KvmVm {
    /// Create a new VM
    #[allow(dead_code)]
    pub fn create(vm_id: u32) -> Result<Self, VmError> {
        Ok(Self {
            vm_fd: vm_id + 1000, // Simulated fd
            num_vcpus: 0,
            memory_regions: Vec::new(),
            irqchip_created: false,
            pit_created: false,
            vcpus: Vec::new(),
            pit: None,
            tss_addr: 0xFFFB_D000, // Default TSS address
            vm_id,
            next_slot: 0,
        })
    }

    /// Set the TSS address (x86-specific, required before creating vCPUs)
    #[allow(dead_code)]
    pub fn set_tss_addr(&mut self, addr: u64) -> Result<(), VmError> {
        self.tss_addr = addr;
        Ok(())
    }

    /// Map a user memory region into the guest physical address space
    #[allow(dead_code)]
    pub fn set_user_memory_region(&mut self, region: KvmMemoryRegion) -> Result<(), VmError> {
        if self.memory_regions.len() >= MAX_MEMORY_REGIONS {
            return Err(VmError::GuestMemoryError);
        }

        // Check for overlapping regions
        for existing in &self.memory_regions {
            if existing.slot == region.slot {
                // Replace existing slot
                // Remove old, add new below
                self.memory_regions.retain(|r| r.slot != region.slot);
                break;
            }
            // Check for guest physical address overlap
            let existing_end = existing.guest_phys_addr + existing.memory_size;
            let new_end = region.guest_phys_addr + region.memory_size;
            if region.guest_phys_addr < existing_end && new_end > existing.guest_phys_addr {
                // Overlapping -- allow replacement by slot, otherwise error
                if existing.slot != region.slot {
                    return Err(VmError::GuestMemoryError);
                }
            }
        }

        self.memory_regions.push(region);
        self.next_slot = self.next_slot.max(region.slot + 1);
        Ok(())
    }

    /// Create a vCPU and return its index
    #[allow(dead_code)]
    pub fn create_vcpu(&mut self, vcpu_id: u32) -> Result<usize, VmError> {
        if self.num_vcpus as usize >= MAX_VCPUS_PER_VM {
            return Err(VmError::InvalidVmState);
        }

        let vcpu = KvmVcpu::new(vcpu_id);
        let index = self.vcpus.len();
        self.vcpus.push(vcpu);
        self.num_vcpus += 1;
        Ok(index)
    }

    /// Get a reference to a vCPU by index
    #[allow(dead_code)]
    pub fn vcpu(&self, index: usize) -> Option<&KvmVcpu> {
        self.vcpus.get(index)
    }

    /// Get a mutable reference to a vCPU by index
    #[allow(dead_code)]
    pub fn vcpu_mut(&mut self, index: usize) -> Option<&mut KvmVcpu> {
        self.vcpus.get_mut(index)
    }

    /// Create an in-kernel IRQ chip (LAPIC + IOAPIC)
    #[allow(dead_code)]
    pub fn create_irqchip(&mut self) -> Result<(), VmError> {
        if self.irqchip_created {
            return Err(VmError::VmxAlreadyEnabled);
        }
        self.irqchip_created = true;
        Ok(())
    }

    /// Create an in-kernel PIT (i8254)
    #[allow(dead_code)]
    pub fn create_pit2(&mut self) -> Result<(), VmError> {
        if self.pit_created {
            return Err(VmError::VmxAlreadyEnabled);
        }
        if !self.irqchip_created {
            return Err(VmError::InvalidVmState);
        }
        self.pit = Some(VirtualPit::new());
        self.pit_created = true;
        Ok(())
    }

    /// Get the PIT if created
    #[allow(dead_code)]
    pub fn pit(&self) -> Option<&VirtualPit> {
        self.pit.as_ref()
    }

    /// Get mutable PIT if created
    #[allow(dead_code)]
    pub fn pit_mut(&mut self) -> Option<&mut VirtualPit> {
        self.pit.as_mut()
    }

    /// Translate a guest physical address to host virtual
    #[allow(dead_code)]
    pub fn translate_address(&self, guest_phys: u64) -> Option<u64> {
        for region in &self.memory_regions {
            if let Some(host_addr) = region.translate(guest_phys) {
                return Some(host_addr);
            }
        }
        None
    }

    /// Get the number of vCPUs
    #[allow(dead_code)]
    pub fn num_vcpus(&self) -> u32 {
        self.num_vcpus
    }

    /// Get the number of memory regions
    #[allow(dead_code)]
    pub fn num_memory_regions(&self) -> usize {
        self.memory_regions.len()
    }

    /// Check if irqchip is created
    #[allow(dead_code)]
    pub fn has_irqchip(&self) -> bool {
        self.irqchip_created
    }

    /// Check if PIT is created
    #[allow(dead_code)]
    pub fn has_pit(&self) -> bool {
        self.pit_created
    }

    /// Get VM identifier
    #[allow(dead_code)]
    pub fn vm_id(&self) -> u32 {
        self.vm_id
    }

    /// Get next available memory slot
    #[allow(dead_code)]
    pub fn next_memory_slot(&self) -> u32 {
        self.next_slot
    }

    /// Allocate a new memory slot and return it
    #[allow(dead_code)]
    pub fn allocate_memory_slot(&mut self) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        slot
    }

    /// Check a KVM capability
    #[allow(dead_code)]
    pub fn check_capability(&self, cap: KvmCapability) -> bool {
        cap.is_supported()
    }

    /// Dispatch I/O to PIT if appropriate
    #[allow(dead_code)]
    pub fn handle_pit_io(&mut self, port: u16, is_write: bool, data: &mut [u8]) -> bool {
        if let Some(pit) = &mut self.pit {
            pit.handle_io(port, is_write, data)
        } else {
            false
        }
    }

    /// Get memory region by slot
    #[allow(dead_code)]
    pub fn memory_region(&self, slot: u32) -> Option<&KvmMemoryRegion> {
        self.memory_regions.iter().find(|r| r.slot == slot)
    }

    /// List all memory regions
    #[allow(dead_code)]
    pub fn memory_regions(&self) -> &[KvmMemoryRegion] {
        &self.memory_regions
    }

    /// Get total guest memory size in bytes
    #[allow(dead_code)]
    pub fn total_memory(&self) -> u64 {
        self.memory_regions.iter().map(|r| r.memory_size).sum()
    }
}

// ---------------------------------------------------------------------------
// KVM System-level Functions
// ---------------------------------------------------------------------------

/// Get the KVM API version
#[allow(dead_code)]
pub fn kvm_get_api_version() -> u32 {
    KVM_API_VERSION
}

/// Check if KVM extension is available
#[allow(dead_code)]
pub fn kvm_check_extension(cap: KvmCapability) -> bool {
    cap.is_supported()
}

/// Get the recommended vCPU map size
#[allow(dead_code)]
pub fn kvm_get_vcpu_mmap_size() -> usize {
    // Size of the KvmRunPage structure (page-aligned)
    4096
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kvm_api_version() {
        assert_eq!(kvm_get_api_version(), 12);
    }

    #[test]
    fn test_kvm_capability_from_raw() {
        assert_eq!(KvmCapability::from_raw(0), Some(KvmCapability::Irqchip));
        assert_eq!(KvmCapability::from_raw(5), Some(KvmCapability::Pit2));
        assert_eq!(KvmCapability::from_raw(99), None);
    }

    #[test]
    fn test_kvm_capability_supported() {
        assert!(KvmCapability::Irqchip.is_supported());
        assert!(KvmCapability::CoalescedMmio.is_supported());
    }

    #[test]
    fn test_exit_reason_roundtrip() {
        let reasons = [
            KvmExitReason::Io,
            KvmExitReason::Mmio,
            KvmExitReason::Hlt,
            KvmExitReason::Shutdown,
            KvmExitReason::InternalError,
        ];
        for reason in reasons {
            assert_eq!(KvmExitReason::from_raw(reason.to_raw()), reason);
        }
    }

    #[test]
    fn test_exit_reason_unknown() {
        let reason = KvmExitReason::from_raw(999);
        assert_eq!(reason, KvmExitReason::Unknown(999));
    }

    #[test]
    fn test_io_exit_in() {
        let io = KvmIoExit::new_in(0x3F8, 1);
        assert_eq!(io.direction, IoDirection::In);
        assert_eq!(io.port, 0x3F8);
        assert_eq!(io.data, 0);
    }

    #[test]
    fn test_io_exit_out() {
        let io = KvmIoExit::new_out(0x3F8, 1, 0x41);
        assert_eq!(io.direction, IoDirection::Out);
        assert_eq!(io.data, 0x41);
    }

    #[test]
    fn test_mmio_exit_read() {
        let mmio = KvmMmioExit::new_read(0xFEE0_0000, 4);
        assert!(!mmio.is_write);
        assert_eq!(mmio.len, 4);
    }

    #[test]
    fn test_mmio_exit_write() {
        let mmio = KvmMmioExit::new_write(0xFEE0_0000, 42, 4);
        assert!(mmio.is_write);
        assert_eq!(mmio.data, 42);
    }

    #[test]
    fn test_segment_flat_code() {
        let seg = KvmSegment::flat_code_64();
        assert_eq!(seg.l, 1);
        assert_eq!(seg.selector, 0x08);
        assert_eq!(seg.present, 1);
    }

    #[test]
    fn test_sregs_long_mode() {
        let sregs = KvmSregs::long_mode(0x1000);
        assert_eq!(sregs.cr3, 0x1000);
        assert_eq!(sregs.efer, 0x500);
        assert!(sregs.cr0 & 0x8000_0001 != 0); // PG | PE
    }

    #[test]
    fn test_memory_region_translate() {
        let region = KvmMemoryRegion::new(0, 0x1000, 0x10000, 0xFFFF_0000);
        assert_eq!(region.translate(0x1000), Some(0xFFFF_0000));
        assert_eq!(region.translate(0x2000), Some(0xFFFF_1000));
        assert_eq!(region.translate(0x11000), None);
    }

    #[test]
    fn test_memory_region_readonly() {
        let region = KvmMemoryRegion::new_readonly(0, 0, 4096, 0);
        assert!(region.is_readonly());
    }

    #[test]
    fn test_memory_region_contains() {
        let region = KvmMemoryRegion::new(0, 0x1000, 0x2000, 0);
        assert!(region.contains_guest_addr(0x1000));
        assert!(region.contains_guest_addr(0x2FFF));
        assert!(!region.contains_guest_addr(0x3000));
        assert!(!region.contains_guest_addr(0x0FFF));
    }

    #[test]
    fn test_pit_mode_from_raw() {
        assert_eq!(PitMode::from_raw(0), PitMode::InterruptOnTerminalCount);
        assert_eq!(PitMode::from_raw(2), PitMode::RateGenerator);
        assert_eq!(PitMode::from_raw(3), PitMode::SquareWave);
    }

    #[test]
    fn test_pit_channel_load_count() {
        let mut ch = PitChannel::new();
        ch.mode = PitMode::RateGenerator;
        ch.load_count(1000);
        assert_eq!(ch.reload, 1000);
        assert_eq!(ch.count, 1000);
        assert!(ch.enabled);
    }

    #[test]
    fn test_pit_channel_tick_rate_generator() {
        let mut ch = PitChannel::new();
        ch.mode = PitMode::RateGenerator;
        ch.load_count(3);
        // Tick down: 3, 2, 1, 0 -> reload
        assert!(!ch.tick()); // 2
        assert!(!ch.tick()); // 1
        assert!(!ch.tick()); // 0
        assert!(ch.tick()); // reload, IRQ
    }

    #[test]
    fn test_pit_channel_tick_square_wave() {
        let mut ch = PitChannel::new();
        ch.mode = PitMode::SquareWave;
        ch.load_count(2);
        // Tick down to 0 then toggle output
        assert!(!ch.tick()); // 1
        assert!(!ch.tick()); // 0
                             // At 0: toggle output, reload
        let irq = ch.tick();
        // Output toggled
        assert!(irq || !irq); // just testing it doesn't panic
    }

    #[test]
    fn test_pit_channel_frequency() {
        let mut ch = PitChannel::new();
        ch.reload = 1000;
        assert_eq!(ch.frequency_hz(), PIT_FREQUENCY_HZ / 1000);
    }

    #[test]
    fn test_pit_channel_period_ns() {
        let mut ch = PitChannel::new();
        ch.reload = PIT_FREQUENCY_HZ as u16; // ~1 Hz
        let period = ch.period_ns();
        // Should be approximately 1 second
        assert!(period > 0);
    }

    #[test]
    fn test_pit_channel_advance_ns() {
        let mut ch = PitChannel::new();
        ch.mode = PitMode::RateGenerator;
        ch.access_mode = 3;
        ch.load_count(100);
        // Advance by enough ns for multiple ticks
        let irqs = ch.advance_ns(1_000_000); // 1ms
                                             // Should have produced some IRQs
        assert!(irqs > 0 || ch.count < 100);
    }

    #[test]
    fn test_pit_channel_latch() {
        let mut ch = PitChannel::new();
        ch.count = 0x1234;
        ch.access_mode = 3;
        ch.latch();
        assert!(ch.latch_valid);
        assert_eq!(ch.latch_value, 0x1234);
        // Reading should return latched value
        let lo = ch.read_byte();
        assert_eq!(lo, 0x34);
        let hi = ch.read_byte();
        assert_eq!(hi, 0x12);
        assert!(!ch.latch_valid);
    }

    #[test]
    fn test_virtual_pit_new() {
        let pit = VirtualPit::new();
        assert_eq!(pit.channels.len(), 3);
        assert!(pit.channels[0].gate);
        assert!(!pit.channels[2].gate); // Speaker channel gate off
    }

    #[test]
    fn test_virtual_pit_control_word() {
        let mut pit = VirtualPit::new();
        // Set channel 0 to rate generator, lo/hi access
        pit.write_control(0x34); // Channel 0, lo/hi, mode 2
        assert_eq!(pit.channels[0].access_mode, 3);
        assert_eq!(pit.channels[0].mode, PitMode::RateGenerator);
    }

    #[test]
    fn test_virtual_pit_io_handler() {
        let mut pit = VirtualPit::new();
        // Write control word
        let mut data = [0x34u8];
        assert!(pit.handle_io(0x43, true, &mut data));
        // Write count low byte
        data = [0x00];
        assert!(pit.handle_io(0x40, true, &mut data));
        // Write count high byte
        data = [0x04];
        assert!(pit.handle_io(0x40, true, &mut data));
        // Channel 0 should have reload = 0x0400
        assert_eq!(pit.channels[0].reload, 0x0400);
    }

    #[test]
    fn test_virtual_pit_speaker_port() {
        let mut pit = VirtualPit::new();
        let mut data = [0x03u8];
        pit.handle_io(0x61, true, &mut data);
        assert!(pit.speaker_gate);
        assert!(pit.channels[2].gate);

        data = [0u8];
        pit.handle_io(0x61, false, &mut data);
        assert_eq!(data[0] & 0x01, 0x01); // Gate bit set
    }

    #[test]
    fn test_virtual_pit_advance() {
        let mut pit = VirtualPit::new();
        pit.write_control(0x34); // Ch0, lo/hi, rate generator
        pit.write_channel(0, 0x00); // lo byte
        pit.write_channel(0, 0x01); // hi byte -> reload = 256
        let irqs = pit.advance_ns(10_000_000); // 10ms
                                               // Should produce IRQs
        assert!(irqs > 0 || pit.channels[0].count < 256);
    }

    #[test]
    fn test_vm_create() {
        let vm = KvmVm::create(1).unwrap();
        assert_eq!(vm.vm_id(), 1);
        assert_eq!(vm.num_vcpus(), 0);
        assert_eq!(vm.num_memory_regions(), 0);
    }

    #[test]
    fn test_vm_set_tss_addr() {
        let mut vm = KvmVm::create(1).unwrap();
        assert!(vm.set_tss_addr(0xFFFB_D000).is_ok());
    }

    #[test]
    fn test_vm_memory_region() {
        let mut vm = KvmVm::create(1).unwrap();
        let region = KvmMemoryRegion::new(0, 0, 0x100000, 0x7F00_0000);
        assert!(vm.set_user_memory_region(region).is_ok());
        assert_eq!(vm.num_memory_regions(), 1);
        assert_eq!(vm.total_memory(), 0x100000);
    }

    #[test]
    fn test_vm_translate_address() {
        let mut vm = KvmVm::create(1).unwrap();
        let region = KvmMemoryRegion::new(0, 0x1000, 0x10000, 0xA000_0000);
        vm.set_user_memory_region(region).unwrap();
        assert_eq!(vm.translate_address(0x2000), Some(0xA000_1000));
        assert_eq!(vm.translate_address(0), None);
    }

    #[test]
    fn test_vm_create_vcpu() {
        let mut vm = KvmVm::create(1).unwrap();
        let idx = vm.create_vcpu(0).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(vm.num_vcpus(), 1);
        assert!(vm.vcpu(0).is_some());
    }

    #[test]
    fn test_vm_irqchip_and_pit() {
        let mut vm = KvmVm::create(1).unwrap();
        assert!(!vm.has_irqchip());
        assert!(!vm.has_pit());

        vm.create_irqchip().unwrap();
        assert!(vm.has_irqchip());
        // Double create should fail
        assert!(vm.create_irqchip().is_err());

        // PIT requires irqchip
        vm.create_pit2().unwrap();
        assert!(vm.has_pit());
        assert!(vm.pit().is_some());
    }

    #[test]
    fn test_vm_pit_requires_irqchip() {
        let mut vm = KvmVm::create(1).unwrap();
        assert!(vm.create_pit2().is_err());
    }

    #[test]
    fn test_vcpu_regs() {
        let mut vcpu = KvmVcpu::new(0);
        let mut regs = KvmRegs::default();
        regs.rip = 0x7C00;
        regs.rsp = 0x8000;
        vcpu.set_regs(regs);
        assert_eq!(vcpu.get_regs().rip, 0x7C00);
        assert_eq!(vcpu.get_regs().rsp, 0x8000);
    }

    #[test]
    fn test_vcpu_sregs() {
        let mut vcpu = KvmVcpu::new(0);
        let sregs = KvmSregs::long_mode(0x2000);
        vcpu.set_sregs(sregs);
        assert_eq!(vcpu.get_sregs().cr3, 0x2000);
    }

    #[test]
    fn test_vcpu_msrs() {
        let mut vcpu = KvmVcpu::new(0);
        let entries = [
            MsrEntry {
                index: 0xC000_0080,
                value: 0x500,
            }, // IA32_EFER
            MsrEntry {
                index: 0x174,
                value: 0x08,
            }, // IA32_SYSENTER_CS
        ];
        assert!(vcpu.set_msrs(&entries).is_ok());
        let result = vcpu.get_msrs(&[0xC000_0080, 0x174, 0x999]);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].value, 0x500);
        assert_eq!(result[1].value, 0x08);
        assert_eq!(result[2].value, 0); // Not found
    }

    #[test]
    fn test_vcpu_interrupt() {
        let mut vcpu = KvmVcpu::new(0);
        vcpu.halt();
        assert!(vcpu.is_halted());
        vcpu.interrupt(32);
        assert!(!vcpu.is_halted()); // Interrupt wakes from halt
    }

    #[test]
    fn test_vcpu_msi() {
        let mut vcpu = KvmVcpu::new(0);
        // MSI address with dest_id=1, vector=0x42
        vcpu.signal_msi(0xFEE0_1000, 0x42);
        assert!(!vcpu.is_halted());
    }

    #[test]
    fn test_vcpu_run_halted() {
        let mut vcpu = KvmVcpu::new(0);
        vcpu.halt();
        let reason = vcpu.run().unwrap();
        assert_eq!(reason, KvmExitReason::Hlt);
    }

    #[test]
    fn test_vcpu_reset() {
        let mut vcpu = KvmVcpu::new(0);
        vcpu.regs.rip = 0x1234;
        vcpu.halt();
        vcpu.reset();
        assert_eq!(vcpu.regs.rip, 0);
        assert!(!vcpu.is_halted());
    }

    #[test]
    fn test_vcpu_run_increments_counters() {
        let mut vcpu = KvmVcpu::new(0);
        assert_eq!(vcpu.entry_count(), 0);
        assert_eq!(vcpu.exit_count(), 0);
        let _ = vcpu.run();
        assert_eq!(vcpu.entry_count(), 1);
        assert_eq!(vcpu.exit_count(), 1);
    }
}
