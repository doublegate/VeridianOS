//! Local APIC and I/O APIC support for x86_64.
//!
//! Provides initialization and control of the Local APIC (interrupt delivery to
//! the local CPU) and I/O APIC (external interrupt routing). This module is
//! additive to the existing PIC (8259) setup -- the PIC remains as a fallback
//! while the APIC handles advanced interrupt routing.
//!
//! The Local APIC is memory-mapped at 0xFEE0_0000 (identity-mapped by the
//! bootloader). The I/O APIC is at 0xFEC0_0000 with indirect register access
//! via IOREGSEL/IOWIN.

use core::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// MSR addresses
// ---------------------------------------------------------------------------

/// IA32_APIC_BASE MSR address. Contains the APIC base physical address and
/// enable/BSP flags.
const IA32_APIC_BASE_MSR: u32 = 0x1B;

/// Bit 11 of IA32_APIC_BASE: global APIC enable.
const IA32_APIC_BASE_ENABLE: u64 = 1 << 11;

// ---------------------------------------------------------------------------
// Local APIC register offsets (byte offsets from APIC base)
// ---------------------------------------------------------------------------

/// Local APIC ID register.
const LAPIC_ID: u32 = 0x020;
/// Local APIC Version register.
const LAPIC_VERSION: u32 = 0x030;
/// Task Priority Register -- controls interrupt priority filtering.
const LAPIC_TPR: u32 = 0x080;
/// End-Of-Interrupt register -- write 0 to signal interrupt completion.
const LAPIC_EOI: u32 = 0x0B0;
/// Spurious Interrupt Vector register -- also contains the software enable bit.
const LAPIC_SVR: u32 = 0x0F0;
/// In-Service Register (ISR) base -- 8 consecutive 32-bit registers.
#[allow(dead_code)]
const LAPIC_ISR_BASE: u32 = 0x100;
/// Trigger Mode Register (TMR) base.
#[allow(dead_code)]
const LAPIC_TMR_BASE: u32 = 0x180;
/// Interrupt Request Register (IRR) base.
#[allow(dead_code)]
const LAPIC_IRR_BASE: u32 = 0x200;
/// Error Status Register.
#[allow(dead_code)]
const LAPIC_ESR: u32 = 0x280;
/// Interrupt Command Register (low 32 bits).
const LAPIC_ICR_LOW: u32 = 0x300;
/// Interrupt Command Register (high 32 bits -- destination field).
const LAPIC_ICR_HIGH: u32 = 0x310;
/// LVT Timer register.
const LAPIC_LVT_TIMER: u32 = 0x320;
/// LVT LINT0 register.
const LAPIC_LVT_LINT0: u32 = 0x350;
/// LVT LINT1 register.
const LAPIC_LVT_LINT1: u32 = 0x360;
/// LVT Error register.
const LAPIC_LVT_ERROR: u32 = 0x370;
/// Timer Initial Count register.
const LAPIC_TIMER_INIT_COUNT: u32 = 0x380;
/// Timer Current Count register (read-only).
const LAPIC_TIMER_CUR_COUNT: u32 = 0x390;
/// Timer Divide Configuration register.
const LAPIC_TIMER_DIV: u32 = 0x3E0;

/// LVT mask bit (bit 16) -- when set, the interrupt is masked.
const LVT_MASK: u32 = 1 << 16;

/// Spurious Vector Register software enable bit (bit 8).
const SVR_ENABLE: u32 = 1 << 8;

/// Default spurious interrupt vector number (0xFF by convention).
const SPURIOUS_VECTOR: u8 = 0xFF;

// ---------------------------------------------------------------------------
// LVT Timer mode bits
// ---------------------------------------------------------------------------

/// One-shot timer mode (bits 18:17 = 00).
#[allow(dead_code)]
const TIMER_MODE_ONESHOT: u32 = 0b00 << 17;
/// Periodic timer mode (bits 18:17 = 01).
const TIMER_MODE_PERIODIC: u32 = 0b01 << 17;

// ---------------------------------------------------------------------------
// I/O APIC
// ---------------------------------------------------------------------------

/// Default I/O APIC MMIO base address (QEMU virt machine).
const IOAPIC_BASE: usize = 0xFEC0_0000;

/// I/O APIC Register Select (write the register index here).
const IOREGSEL: u32 = 0x00;
/// I/O APIC Window (read/write the selected register through here).
const IOWIN: u32 = 0x10;

/// I/O APIC ID register.
#[allow(dead_code)]
const IOAPIC_REG_ID: u32 = 0x00;
/// I/O APIC Version register.
const IOAPIC_REG_VER: u32 = 0x01;

/// I/O APIC redirection table entry base (each entry uses two 32-bit
/// registers).
const IOAPIC_REDTBL_BASE: u32 = 0x10;

// ---------------------------------------------------------------------------
// Redirection table entry bitfields
// ---------------------------------------------------------------------------

/// Redirection table entry -- represents a 64-bit I/O APIC routing entry.
///
/// Layout:
/// - Bits  7:0  -- Interrupt vector
/// - Bits 10:8  -- Delivery mode (000=Fixed, 001=LowestPri, 010=SMI, 100=NMI,
///   101=INIT, 111=ExtINT)
/// - Bit  11    -- Destination mode (0=Physical, 1=Logical)
/// - Bit  12    -- Delivery status (read-only: 0=idle, 1=pending)
/// - Bit  13    -- Pin polarity (0=active high, 1=active low)
/// - Bit  14    -- Remote IRR (read-only, level-triggered)
/// - Bit  15    -- Trigger mode (0=edge, 1=level)
/// - Bit  16    -- Mask (1=masked)
/// - Bits 63:56 -- Destination APIC ID (physical mode)
#[derive(Debug, Clone, Copy)]
pub struct RedirectionEntry {
    raw: u64,
}

impl RedirectionEntry {
    /// Create a new masked redirection entry with the given vector.
    pub const fn new(vector: u8) -> Self {
        Self {
            raw: (vector as u64) | ((1u64) << 16), // masked by default
        }
    }

    /// Set the interrupt vector (bits 7:0).
    pub fn set_vector(&mut self, vector: u8) {
        self.raw = (self.raw & !0xFF) | (vector as u64);
    }

    /// Get the interrupt vector.
    #[allow(dead_code)]
    pub fn vector(&self) -> u8 {
        (self.raw & 0xFF) as u8
    }

    /// Set delivery mode (bits 10:8).
    /// 0=Fixed, 1=LowestPriority, 2=SMI, 4=NMI, 5=INIT, 7=ExtINT.
    #[allow(dead_code)]
    pub fn set_delivery_mode(&mut self, mode: u8) {
        self.raw = (self.raw & !(0b111 << 8)) | (((mode & 0b111) as u64) << 8);
    }

    /// Set destination mode (bit 11). 0=Physical, 1=Logical.
    #[allow(dead_code)]
    pub fn set_dest_mode_logical(&mut self, logical: bool) {
        if logical {
            self.raw |= 1 << 11;
        } else {
            self.raw &= !(1 << 11);
        }
    }

    /// Set pin polarity (bit 13). false=active high, true=active low.
    #[allow(dead_code)]
    pub fn set_active_low(&mut self, active_low: bool) {
        if active_low {
            self.raw |= 1 << 13;
        } else {
            self.raw &= !(1 << 13);
        }
    }

    /// Set trigger mode (bit 15). false=edge, true=level.
    #[allow(dead_code)]
    pub fn set_level_triggered(&mut self, level: bool) {
        if level {
            self.raw |= 1 << 15;
        } else {
            self.raw &= !(1 << 15);
        }
    }

    /// Set mask bit (bit 16). true=masked.
    pub fn set_masked(&mut self, masked: bool) {
        if masked {
            self.raw |= 1 << 16;
        } else {
            self.raw &= !(1 << 16);
        }
    }

    /// Check if the entry is masked.
    pub fn is_masked(&self) -> bool {
        self.raw & (1 << 16) != 0
    }

    /// Set destination APIC ID (bits 63:56).
    pub fn set_destination(&mut self, dest: u8) {
        self.raw = (self.raw & !(0xFFu64 << 56)) | ((dest as u64) << 56);
    }

    /// Get the low 32 bits of the entry.
    pub fn low(&self) -> u32 {
        self.raw as u32
    }

    /// Get the high 32 bits of the entry.
    pub fn high(&self) -> u32 {
        (self.raw >> 32) as u32
    }

    /// Construct from low and high 32-bit halves.
    pub fn from_parts(low: u32, high: u32) -> Self {
        Self {
            raw: (low as u64) | ((high as u64) << 32),
        }
    }
}

// ---------------------------------------------------------------------------
// Local APIC
// ---------------------------------------------------------------------------

/// Local APIC controller.
///
/// Wraps the memory-mapped register file for the per-CPU Local APIC. All
/// register accesses use volatile reads/writes to prevent compiler reordering.
pub struct LocalApic {
    /// Virtual address of the APIC MMIO base (identity-mapped at 0xFEE0_0000).
    base: usize,
}

impl LocalApic {
    /// Create a new `LocalApic` handle with the given MMIO base address.
    fn new(base: usize) -> Self {
        Self { base }
    }

    /// Read a 32-bit Local APIC register at the given byte offset.
    fn read(&self, offset: u32) -> u32 {
        let addr = self.base + offset as usize;
        // SAFETY: The address `self.base + offset` points to a well-known Local
        // APIC MMIO register. The APIC region at 0xFEE0_0000 is identity-mapped
        // by the bootloader and reserved in the frame allocator. Volatile read
        // ensures the compiler does not elide or reorder the access.
        unsafe { ptr::read_volatile(addr as *const u32) }
    }

    /// Write a 32-bit value to a Local APIC register at the given byte offset.
    fn write(&self, offset: u32, value: u32) {
        let addr = self.base + offset as usize;
        // SAFETY: Same as `read` -- the address is a valid APIC MMIO register.
        // Volatile write ensures the hardware sees the store in program order.
        unsafe { ptr::write_volatile(addr as *mut u32, value) }
    }

    /// Read the Local APIC ID (bits 31:24 of the ID register).
    pub fn read_id(&self) -> u8 {
        ((self.read(LAPIC_ID) >> 24) & 0xFF) as u8
    }

    /// Read the Local APIC version register.
    #[allow(dead_code)]
    pub fn read_version(&self) -> u32 {
        self.read(LAPIC_VERSION)
    }

    /// Enable the Local APIC by setting the software-enable bit in the
    /// Spurious Interrupt Vector register and configuring the spurious vector.
    fn enable(&self) {
        // Set spurious vector to 0xFF and set the software-enable bit (bit 8).
        self.write(LAPIC_SVR, SVR_ENABLE | SPURIOUS_VECTOR as u32);
    }

    /// Mask all Local Vector Table entries (Timer, LINT0, LINT1, Error) to
    /// prevent unexpected interrupts before they are explicitly configured.
    fn mask_all_lvt(&self) {
        self.write(LAPIC_LVT_TIMER, LVT_MASK);
        self.write(LAPIC_LVT_LINT0, LVT_MASK);
        self.write(LAPIC_LVT_LINT1, LVT_MASK);
        self.write(LAPIC_LVT_ERROR, LVT_MASK);
    }

    /// Send an End-Of-Interrupt signal. Must be called at the end of every
    /// Local APIC interrupt handler.
    pub fn send_eoi(&self) {
        self.write(LAPIC_EOI, 0);
    }

    /// Set the Task Priority Register to allow all interrupts (priority 0).
    fn set_task_priority(&self, priority: u8) {
        self.write(LAPIC_TPR, priority as u32);
    }

    /// Configure the APIC timer for periodic interrupts.
    ///
    /// - `vector`: IDT vector number for the timer interrupt.
    /// - `divide`: Timer divisor encoded as the Divide Configuration Register
    ///   value (e.g., 0x03 = divide by 16, 0x0B = divide by 1).
    /// - `initial_count`: Initial countdown value. The timer fires when it
    ///   reaches zero and reloads automatically in periodic mode.
    pub fn setup_timer(&self, vector: u8, divide: u8, initial_count: u32) {
        // Stop the timer first.
        self.write(LAPIC_TIMER_INIT_COUNT, 0);

        // Set the divide configuration.
        self.write(LAPIC_TIMER_DIV, divide as u32);

        // Configure LVT Timer: periodic mode, unmasked, with the given vector.
        self.write(LAPIC_LVT_TIMER, TIMER_MODE_PERIODIC | vector as u32);

        // Setting the initial count starts the timer.
        self.write(LAPIC_TIMER_INIT_COUNT, initial_count);
    }

    /// Stop the APIC timer by zeroing the initial count and masking the LVT
    /// Timer entry.
    pub fn stop_timer(&self) {
        self.write(LAPIC_TIMER_INIT_COUNT, 0);
        self.write(LAPIC_LVT_TIMER, LVT_MASK);
    }

    /// Read the current timer count (counts down from the initial value).
    #[allow(dead_code)]
    pub fn read_timer_count(&self) -> u32 {
        self.read(LAPIC_TIMER_CUR_COUNT)
    }

    /// Write the Interrupt Command Register to send an IPI.
    ///
    /// - `dest`: Destination APIC ID.
    /// - `vector`: Interrupt vector.
    #[allow(dead_code)]
    pub fn send_ipi(&self, dest: u8, vector: u8) {
        // Write high dword first (destination in bits 31:24).
        self.write(LAPIC_ICR_HIGH, (dest as u32) << 24);
        // Write low dword (vector + delivery mode Fixed). Writing ICR low
        // triggers the IPI.
        self.write(LAPIC_ICR_LOW, vector as u32);
    }
}

// ---------------------------------------------------------------------------
// I/O APIC
// ---------------------------------------------------------------------------

/// I/O APIC controller.
///
/// The I/O APIC uses indirect register access: write the register index to
/// IOREGSEL, then read/write the value through IOWIN.
pub struct IoApic {
    /// Virtual address of the I/O APIC MMIO base (identity-mapped at
    /// 0xFEC0_0000).
    base: usize,
}

impl IoApic {
    /// Create a new `IoApic` handle with the given MMIO base address.
    fn new(base: usize) -> Self {
        Self { base }
    }

    /// Read a 32-bit I/O APIC register.
    pub fn read_register(&self, reg: u32) -> u32 {
        // SAFETY: IOREGSEL at base+0x00 and IOWIN at base+0x10 are the I/O
        // APIC's indirect register access ports. The base address 0xFEC0_0000
        // is identity-mapped by the bootloader. Volatile writes ensure the
        // register select is visible to hardware before the window read.
        unsafe {
            ptr::write_volatile((self.base + IOREGSEL as usize) as *mut u32, reg);
            ptr::read_volatile((self.base + IOWIN as usize) as *const u32)
        }
    }

    /// Write a 32-bit value to an I/O APIC register.
    pub fn write_register(&self, reg: u32, value: u32) {
        // SAFETY: Same as `read_register` -- indirect MMIO access through
        // IOREGSEL/IOWIN. The volatile operations guarantee ordering.
        unsafe {
            ptr::write_volatile((self.base + IOREGSEL as usize) as *mut u32, reg);
            ptr::write_volatile((self.base + IOWIN as usize) as *mut u32, value);
        }
    }

    /// Read the maximum number of redirection entries supported by this I/O
    /// APIC (from bits 23:16 of the version register, plus one).
    pub fn max_redirection_entries(&self) -> u8 {
        let ver = self.read_register(IOAPIC_REG_VER);
        (((ver >> 16) & 0xFF) + 1) as u8
    }

    /// Read a full 64-bit redirection table entry for the given IRQ.
    fn read_redirection(&self, irq: u8) -> RedirectionEntry {
        let reg_base = IOAPIC_REDTBL_BASE + (irq as u32) * 2;
        let low = self.read_register(reg_base);
        let high = self.read_register(reg_base + 1);
        RedirectionEntry::from_parts(low, high)
    }

    /// Write a full 64-bit redirection table entry for the given IRQ.
    fn write_redirection(&self, irq: u8, entry: RedirectionEntry) {
        let reg_base = IOAPIC_REDTBL_BASE + (irq as u32) * 2;
        // Write high dword first to avoid a transient unmasked state if the
        // low dword unmasks the entry.
        self.write_register(reg_base + 1, entry.high());
        self.write_register(reg_base, entry.low());
    }

    /// Route an external IRQ to a specific interrupt vector and destination
    /// APIC ID with edge-triggered, active-high, fixed delivery mode.
    pub fn set_irq_route(&self, irq: u8, vector: u8, dest: u8) {
        let mut entry = RedirectionEntry::new(vector);
        entry.set_destination(dest);
        entry.set_masked(false);
        self.write_redirection(irq, entry);
    }

    /// Mask an IRQ in the I/O APIC redirection table.
    pub fn mask_irq(&self, irq: u8) {
        let mut entry = self.read_redirection(irq);
        entry.set_masked(true);
        self.write_redirection(irq, entry);
    }

    /// Unmask an IRQ in the I/O APIC redirection table.
    pub fn unmask_irq(&self, irq: u8) {
        let mut entry = self.read_redirection(irq);
        entry.set_masked(false);
        self.write_redirection(irq, entry);
    }

    /// Mask all redirection entries.
    fn mask_all(&self) {
        let max = self.max_redirection_entries();
        for irq in 0..max {
            self.mask_irq(irq);
        }
    }
}

// ---------------------------------------------------------------------------
// Global APIC state (no static mut -- uses spin::Mutex)
// ---------------------------------------------------------------------------

/// Combined Local APIC + I/O APIC state, protected by a spinlock.
struct ApicState {
    local_apic: LocalApic,
    io_apic: IoApic,
}

// SAFETY: ApicState contains only raw pointer-like fields (usize base
// addresses) and is always accessed under a spinlock, so there are no data
// races.
unsafe impl Send for ApicState {}

/// Global APIC state. Initialized once by `init()`.
static APIC_STATE: Mutex<Option<ApicState>> = Mutex::new(None);

/// Flag indicating whether the APIC subsystem has been initialized.
static APIC_INITIALIZED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// MSR helpers (delegated to arch::x86_64::msr module)
// ---------------------------------------------------------------------------

use super::msr::{phys_to_virt, rdmsr, wrmsr};

/// Initialize the Local APIC and I/O APIC.
///
/// This function:
/// 1. Reads the APIC base address from the IA32_APIC_BASE MSR.
/// 2. Ensures the global APIC enable bit is set in the MSR.
/// 3. Translates physical MMIO addresses to virtual using the bootloader's
///    physical memory offset.
/// 4. Initializes the Local APIC (software enable, mask all LVTs, set TPR=0).
/// 5. Initializes the I/O APIC (mask all redirection entries).
///
/// Must be called after GDT/IDT initialization but before interrupts are
/// enabled. Safe to call exactly once; subsequent calls return
/// `AlreadyExists`.
pub fn init() -> KernelResult<()> {
    if APIC_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "APIC",
            id: 0,
        });
    }

    // Read APIC base from MSR.
    let apic_base_msr = rdmsr(IA32_APIC_BASE_MSR);
    let apic_base_phys = (apic_base_msr & 0xFFFF_F000) as usize;

    println!(
        "[APIC] IA32_APIC_BASE MSR = {:#x}, physical base = {:#x}",
        apic_base_msr, apic_base_phys
    );

    // Translate physical APIC addresses to virtual addresses.
    // The bootloader maps all physical memory at a dynamic offset; MMIO
    // regions are NOT identity-mapped in a higher-half kernel.
    let lapic_virt = phys_to_virt(apic_base_phys).ok_or(KernelError::NotInitialized {
        subsystem: "physical memory mapping (APIC)",
    })?;
    let ioapic_virt = phys_to_virt(IOAPIC_BASE).ok_or(KernelError::NotInitialized {
        subsystem: "physical memory mapping (I/O APIC)",
    })?;

    println!(
        "[APIC] Virtual addresses: LAPIC={:#x}, IOAPIC={:#x}",
        lapic_virt, ioapic_virt
    );

    // Ensure the global enable bit is set.
    if apic_base_msr & IA32_APIC_BASE_ENABLE == 0 {
        println!("[APIC] Global APIC enable bit not set, enabling...");
        wrmsr(IA32_APIC_BASE_MSR, apic_base_msr | IA32_APIC_BASE_ENABLE);
    }

    // --- Local APIC initialization ---
    let lapic = LocalApic::new(lapic_virt);

    // Mask all LVT entries before enabling to prevent spurious interrupts.
    lapic.mask_all_lvt();

    // Enable the Local APIC via the Spurious Vector Register.
    lapic.enable();

    // Allow all interrupt priorities.
    lapic.set_task_priority(0);

    let apic_id = lapic.read_id();
    println!(
        "[APIC] Local APIC enabled (ID={}, SVR={:#x})",
        apic_id,
        lapic.read(LAPIC_SVR)
    );

    // --- I/O APIC initialization ---
    let ioapic = IoApic::new(ioapic_virt);

    // Mask all I/O APIC redirection entries by default.
    ioapic.mask_all();

    let max_irqs = ioapic.max_redirection_entries();
    println!(
        "[APIC] I/O APIC initialized at {:#x} ({} IRQ lines)",
        ioapic_virt, max_irqs
    );

    // Store global state.
    let mut state = APIC_STATE.lock();
    *state = Some(ApicState {
        local_apic: lapic,
        io_apic: ioapic,
    });
    APIC_INITIALIZED.store(true, Ordering::Release);

    println!("[APIC] APIC subsystem initialized successfully");
    Ok(())
}

/// Check whether the APIC subsystem has been initialized.
pub fn is_initialized() -> bool {
    APIC_INITIALIZED.load(Ordering::Acquire)
}

/// Send an End-Of-Interrupt to the Local APIC.
///
/// Must be called at the end of every APIC-sourced interrupt handler.
pub fn send_eoi() {
    let state = APIC_STATE.lock();
    if let Some(ref s) = *state {
        s.local_apic.send_eoi();
    }
}

/// Read the Local APIC ID of the current CPU.
pub fn read_id() -> Option<u8> {
    let state = APIC_STATE.lock();
    state.as_ref().map(|s| s.local_apic.read_id())
}

/// Configure the Local APIC timer for periodic interrupts.
///
/// - `vector`: IDT vector number (e.g., 32 for the timer).
/// - `divide`: Divide configuration register value:
///   - `0x00` = divide by 2
///   - `0x01` = divide by 4
///   - `0x02` = divide by 8
///   - `0x03` = divide by 16
///   - `0x08` = divide by 32
///   - `0x09` = divide by 64
///   - `0x0A` = divide by 128
///   - `0x0B` = divide by 1
/// - `initial_count`: Initial countdown value.
pub fn setup_timer(vector: u8, divide: u8, initial_count: u32) -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.local_apic.setup_timer(vector, divide, initial_count);
            println!(
                "[APIC] Timer configured: vector={}, divide={:#x}, count={}",
                vector, divide, initial_count
            );
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}

/// Stop the Local APIC timer.
pub fn stop_timer() -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.local_apic.stop_timer();
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}

/// Route an external IRQ through the I/O APIC to a specific interrupt vector
/// and destination CPU.
///
/// - `irq`: I/O APIC input pin (0-23 typically).
/// - `vector`: IDT vector number.
/// - `dest`: Destination Local APIC ID.
pub fn set_irq_route(irq: u8, vector: u8, dest: u8) -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.io_apic.set_irq_route(irq, vector, dest);
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}

/// Mask an IRQ in the I/O APIC.
pub fn mask_irq(irq: u8) -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.io_apic.mask_irq(irq);
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}

/// Unmask an IRQ in the I/O APIC.
pub fn unmask_irq(irq: u8) -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.io_apic.unmask_irq(irq);
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}

/// Send an Inter-Processor Interrupt via the Local APIC.
///
/// - `dest`: Destination APIC ID.
/// - `vector`: Interrupt vector.
#[allow(dead_code)]
pub fn send_ipi(dest: u8, vector: u8) -> KernelResult<()> {
    let state = APIC_STATE.lock();
    match state.as_ref() {
        Some(s) => {
            s.local_apic.send_ipi(dest, vector);
            Ok(())
        }
        None => Err(KernelError::NotInitialized { subsystem: "APIC" }),
    }
}
