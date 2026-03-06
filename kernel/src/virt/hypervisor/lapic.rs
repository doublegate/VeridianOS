//! Virtual LAPIC Emulation
//!
//! Full LAPIC register emulation with timer modes for guest VMs.

use super::{
    smp::{IpiDeliveryMode, IpiMessage},
    LAPIC_BASE_ADDR, LAPIC_REGION_SIZE,
};

// ---------------------------------------------------------------------------
// 5. Virtual LAPIC Emulation
// ---------------------------------------------------------------------------

/// LAPIC timer mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LapicTimerMode {
    /// One-shot: fires once, then stops
    #[default]
    OneShot,
    /// Periodic: fires repeatedly at interval
    Periodic,
    /// TSC-deadline: fires when TSC >= deadline
    TscDeadline,
}

/// Local Vector Table (LVT) entry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LvtEntry {
    /// Raw 32-bit register value
    pub raw: u32,
}

impl Default for LvtEntry {
    fn default() -> Self {
        Self { raw: 0x0001_0000 } // Masked by default
    }
}

impl LvtEntry {
    pub fn vector(&self) -> u8 {
        (self.raw & 0xFF) as u8
    }

    pub fn delivery_mode(&self) -> u8 {
        ((self.raw >> 8) & 0x7) as u8
    }

    pub fn is_masked(&self) -> bool {
        self.raw & (1 << 16) != 0
    }

    pub fn trigger_mode(&self) -> bool {
        self.raw & (1 << 15) != 0
    }

    pub fn timer_mode(&self) -> LapicTimerMode {
        match (self.raw >> 17) & 0x3 {
            0 => LapicTimerMode::OneShot,
            1 => LapicTimerMode::Periodic,
            2 => LapicTimerMode::TscDeadline,
            _ => LapicTimerMode::OneShot,
        }
    }
}

/// Virtual LAPIC register offsets
#[allow(unused)]
pub struct LapicRegs;

#[allow(unused)]
impl LapicRegs {
    pub const ID: u32 = 0x020;
    pub const VERSION: u32 = 0x030;
    pub const TPR: u32 = 0x080;
    pub const APR: u32 = 0x090;
    pub const PPR: u32 = 0x0A0;
    pub const EOI: u32 = 0x0B0;
    pub const RRD: u32 = 0x0C0;
    pub const LDR: u32 = 0x0D0;
    pub const DFR: u32 = 0x0E0;
    pub const SVR: u32 = 0x0F0;
    pub const ISR_BASE: u32 = 0x100;
    pub const TMR_BASE: u32 = 0x180;
    pub const IRR_BASE: u32 = 0x200;
    pub const ESR: u32 = 0x280;
    pub const ICR_LOW: u32 = 0x300;
    pub const ICR_HIGH: u32 = 0x310;
    pub const LVT_TIMER: u32 = 0x320;
    pub const LVT_THERMAL: u32 = 0x330;
    pub const LVT_PERFMON: u32 = 0x340;
    pub const LVT_LINT0: u32 = 0x350;
    pub const LVT_LINT1: u32 = 0x360;
    pub const LVT_ERROR: u32 = 0x370;
    pub const TIMER_INITIAL_COUNT: u32 = 0x380;
    pub const TIMER_CURRENT_COUNT: u32 = 0x390;
    pub const TIMER_DIVIDE_CONFIG: u32 = 0x3E0;
}

/// Virtual LAPIC state
pub struct VirtualLapic {
    /// LAPIC ID
    pub id: u32,
    /// Task Priority Register
    pub tpr: u32,
    /// Spurious Interrupt Vector Register
    pub svr: u32,
    /// In-Service Register (256 bits = 8 x u32)
    pub isr: [u32; 8],
    /// Interrupt Request Register (256 bits = 8 x u32)
    pub irr: [u32; 8],
    /// Trigger Mode Register (256 bits = 8 x u32)
    pub tmr: [u32; 8],
    /// LVT Timer entry
    pub lvt_timer: LvtEntry,
    /// LVT Thermal entry
    pub lvt_thermal: LvtEntry,
    /// LVT Performance Monitor entry
    pub lvt_perfmon: LvtEntry,
    /// LVT LINT0 entry
    pub lvt_lint0: LvtEntry,
    /// LVT LINT1 entry
    pub lvt_lint1: LvtEntry,
    /// LVT Error entry
    pub lvt_error: LvtEntry,
    /// Timer initial count
    pub timer_initial_count: u32,
    /// Timer current count (decrements)
    pub timer_current_count: u32,
    /// Timer divide configuration
    pub timer_divide_config: u32,
    /// TSC deadline value
    pub tsc_deadline: u64,
    /// Error status register
    pub esr: u32,
    /// Interrupt Command Register (low 32 bits)
    pub icr_low: u32,
    /// Interrupt Command Register (high 32 bits)
    pub icr_high: u32,
    /// Logical Destination Register
    pub ldr: u32,
    /// Destination Format Register
    pub dfr: u32,
    /// Whether the LAPIC is enabled (via SVR bit 8)
    pub enabled: bool,
}

impl VirtualLapic {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            tpr: 0,
            svr: 0xFF, // Disabled by default (bit 8 = 0)
            isr: [0; 8],
            irr: [0; 8],
            tmr: [0; 8],
            lvt_timer: LvtEntry::default(),
            lvt_thermal: LvtEntry::default(),
            lvt_perfmon: LvtEntry::default(),
            lvt_lint0: LvtEntry::default(),
            lvt_lint1: LvtEntry::default(),
            lvt_error: LvtEntry::default(),
            timer_initial_count: 0,
            timer_current_count: 0,
            timer_divide_config: 0,
            tsc_deadline: 0,
            esr: 0,
            icr_low: 0,
            icr_high: 0,
            ldr: 0,
            dfr: 0xFFFF_FFFF,
            enabled: false,
        }
    }

    /// Handle MMIO read from LAPIC register space
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset {
            LapicRegs::ID => self.id << 24,
            LapicRegs::VERSION => 0x0005_0014, // version 0x14, max LVT 5
            LapicRegs::TPR => self.tpr,
            LapicRegs::PPR => self.compute_ppr(),
            LapicRegs::LDR => self.ldr,
            LapicRegs::DFR => self.dfr,
            LapicRegs::SVR => self.svr,
            LapicRegs::ESR => self.esr,
            LapicRegs::ICR_LOW => self.icr_low,
            LapicRegs::ICR_HIGH => self.icr_high,
            LapicRegs::LVT_TIMER => self.lvt_timer.raw,
            LapicRegs::LVT_THERMAL => self.lvt_thermal.raw,
            LapicRegs::LVT_PERFMON => self.lvt_perfmon.raw,
            LapicRegs::LVT_LINT0 => self.lvt_lint0.raw,
            LapicRegs::LVT_LINT1 => self.lvt_lint1.raw,
            LapicRegs::LVT_ERROR => self.lvt_error.raw,
            LapicRegs::TIMER_INITIAL_COUNT => self.timer_initial_count,
            LapicRegs::TIMER_CURRENT_COUNT => self.timer_current_count,
            LapicRegs::TIMER_DIVIDE_CONFIG => self.timer_divide_config,
            // ISR/IRR/TMR: 8 registers each at 0x10 intervals
            off if (LapicRegs::ISR_BASE..LapicRegs::ISR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::ISR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.isr[idx]
                } else {
                    0
                }
            }
            off if (LapicRegs::TMR_BASE..LapicRegs::TMR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::TMR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.tmr[idx]
                } else {
                    0
                }
            }
            off if (LapicRegs::IRR_BASE..LapicRegs::IRR_BASE + 0x80).contains(&off) => {
                let idx = ((off - LapicRegs::IRR_BASE) / 0x10) as usize;
                if idx < 8 {
                    self.irr[idx]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// Handle MMIO write to LAPIC register space
    pub fn write_register(&mut self, offset: u32, value: u32) {
        match offset {
            LapicRegs::ID => self.id = value >> 24,
            LapicRegs::TPR => self.tpr = value & 0xFF,
            LapicRegs::LDR => self.ldr = value & 0xFF00_0000,
            LapicRegs::DFR => self.dfr = value | 0x0FFF_FFFF,
            LapicRegs::SVR => {
                self.svr = value;
                self.enabled = value & (1 << 8) != 0;
            }
            LapicRegs::EOI => {
                // End of interrupt: clear highest-priority bit in ISR
                self.handle_eoi();
            }
            LapicRegs::ESR => {
                // Write clears ESR
                self.esr = 0;
            }
            LapicRegs::ICR_LOW => {
                self.icr_low = value;
                // Writing ICR low triggers IPI send
            }
            LapicRegs::ICR_HIGH => {
                self.icr_high = value;
            }
            LapicRegs::LVT_TIMER => {
                self.lvt_timer = LvtEntry { raw: value };
            }
            LapicRegs::LVT_THERMAL => {
                self.lvt_thermal = LvtEntry { raw: value };
            }
            LapicRegs::LVT_PERFMON => {
                self.lvt_perfmon = LvtEntry { raw: value };
            }
            LapicRegs::LVT_LINT0 => {
                self.lvt_lint0 = LvtEntry { raw: value };
            }
            LapicRegs::LVT_LINT1 => {
                self.lvt_lint1 = LvtEntry { raw: value };
            }
            LapicRegs::LVT_ERROR => {
                self.lvt_error = LvtEntry { raw: value };
            }
            LapicRegs::TIMER_INITIAL_COUNT => {
                self.timer_initial_count = value;
                self.timer_current_count = value;
            }
            LapicRegs::TIMER_DIVIDE_CONFIG => {
                self.timer_divide_config = value & 0xB;
            }
            _ => {}
        }
    }

    /// Compute the processor priority register
    fn compute_ppr(&self) -> u32 {
        let isrv = self.highest_isr_priority();
        let tpr_class = self.tpr >> 4;
        let isr_class = isrv >> 4;
        if tpr_class >= isr_class {
            self.tpr
        } else {
            isrv
        }
    }

    /// Find highest-priority bit set in ISR
    fn highest_isr_priority(&self) -> u32 {
        for i in (0..8).rev() {
            if self.isr[i] != 0 {
                let bit = 31 - self.isr[i].leading_zeros();
                return (i as u32) * 32 + bit;
            }
        }
        0
    }

    /// Find highest-priority bit set in IRR
    fn highest_irr_priority(&self) -> Option<u32> {
        for i in (0..8).rev() {
            if self.irr[i] != 0 {
                let bit = 31 - self.irr[i].leading_zeros();
                return Some((i as u32) * 32 + bit);
            }
        }
        None
    }

    /// Handle End-Of-Interrupt: clear highest ISR bit
    fn handle_eoi(&mut self) {
        for i in (0..8).rev() {
            if self.isr[i] != 0 {
                let bit = 31 - self.isr[i].leading_zeros();
                self.isr[i] &= !(1 << bit);
                return;
            }
        }
    }

    /// Accept an interrupt: set IRR bit
    pub fn accept_interrupt(&mut self, vector: u8) {
        let idx = (vector / 32) as usize;
        let bit = (vector % 32) as u32;
        if idx < 8 {
            self.irr[idx] |= 1 << bit;
        }
    }

    /// Try to deliver next pending interrupt (IRR -> ISR)
    pub fn deliver_pending_interrupt(&mut self) -> Option<u8> {
        if !self.enabled {
            return None;
        }

        let ppr = self.compute_ppr();
        let ppr_class = ppr >> 4;

        if let Some(vector) = self.highest_irr_priority() {
            let vector_class = vector >> 4;
            if vector_class > ppr_class {
                // Move from IRR to ISR
                let idx = (vector / 32) as usize;
                let bit = vector % 32;
                self.irr[idx] &= !(1 << bit);
                self.isr[idx] |= 1 << bit;
                return Some(vector as u8);
            }
        }
        None
    }

    /// Tick the LAPIC timer (called periodically by hypervisor)
    /// Returns true if timer interrupt should fire
    pub fn tick_timer(&mut self, ticks: u32) -> bool {
        if self.timer_initial_count == 0 || self.lvt_timer.is_masked() {
            return false;
        }

        let mode = self.lvt_timer.timer_mode();
        match mode {
            LapicTimerMode::OneShot => {
                if self.timer_current_count > 0 {
                    if self.timer_current_count <= ticks {
                        self.timer_current_count = 0;
                        return true;
                    }
                    self.timer_current_count -= ticks;
                }
                false
            }
            LapicTimerMode::Periodic => {
                if self.timer_current_count <= ticks {
                    self.timer_current_count = self.timer_initial_count;
                    true
                } else {
                    self.timer_current_count -= ticks;
                    false
                }
            }
            LapicTimerMode::TscDeadline => {
                // TSC deadline mode handled separately
                false
            }
        }
    }

    /// Get the timer divide value from config register
    pub fn timer_divide_value(&self) -> u32 {
        let bits = ((self.timer_divide_config & 0x8) >> 1) | (self.timer_divide_config & 0x3);
        match bits {
            0b000 => 2,
            0b001 => 4,
            0b010 => 8,
            0b011 => 16,
            0b100 => 32,
            0b101 => 64,
            0b110 => 128,
            0b111 => 1,
            _ => 1,
        }
    }

    /// Extract IPI delivery info from ICR
    pub fn extract_ipi(&self) -> IpiMessage {
        let vector = (self.icr_low & 0xFF) as u8;
        let delivery = match (self.icr_low >> 8) & 0x7 {
            0 => IpiDeliveryMode::Fixed,
            1 => IpiDeliveryMode::LowestPriority,
            4 => IpiDeliveryMode::Nmi,
            5 => IpiDeliveryMode::Init,
            6 => IpiDeliveryMode::Sipi,
            7 => IpiDeliveryMode::ExtInt,
            _ => IpiDeliveryMode::Fixed,
        };
        let level = self.icr_low & (1 << 14) != 0;
        let trigger = self.icr_low & (1 << 15) != 0;
        let dest = (self.icr_high >> 24) as u8;

        IpiMessage {
            source: self.id as u8,
            destination: dest,
            delivery_mode: delivery,
            vector,
            level,
            trigger_level: trigger,
        }
    }

    /// Check if the LAPIC is software-enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the LAPIC base address
    pub fn base_address() -> u64 {
        LAPIC_BASE_ADDR
    }

    /// Get the LAPIC region size
    pub fn region_size() -> u64 {
        LAPIC_REGION_SIZE
    }
}
