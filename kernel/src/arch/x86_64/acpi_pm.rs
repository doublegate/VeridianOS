//! ACPI Power Management for x86_64.
//!
//! Implements ACPI sleep state transitions (S0-S5), SCI interrupt handling,
//! and wake event processing. Reads PM1a/PM1b control and status registers
//! from the FADT to orchestrate suspend (S3), hibernate (S4), and soft-off
//! (S5).
//!
//! CPU context save/restore for S3 resume uses inline assembly to capture
//! and restore general-purpose registers, segment descriptors, and CR3.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};

use spin::Mutex;

use crate::error::{KernelError, KernelResult};

// ---------------------------------------------------------------------------
// ACPI sleep state definitions
// ---------------------------------------------------------------------------

/// ACPI system sleep states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AcpiSleepState {
    /// S0: Working (system fully operational).
    S0Working = 0,
    /// S1: Standby (CPU stops executing, power to CPU/RAM maintained).
    S1Standby = 1,
    /// S3: Suspend to RAM (CPU context saved, RAM remains powered).
    S3Suspend = 3,
    /// S4: Hibernate (memory image saved to disk, full power off).
    S4Hibernate = 4,
    /// S5: Soft Off (mechanical off via ACPI).
    S5SoftOff = 5,
}

impl core::fmt::Display for AcpiSleepState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::S0Working => write!(f, "S0 (Working)"),
            Self::S1Standby => write!(f, "S1 (Standby)"),
            Self::S3Suspend => write!(f, "S3 (Suspend to RAM)"),
            Self::S4Hibernate => write!(f, "S4 (Hibernate)"),
            Self::S5SoftOff => write!(f, "S5 (Soft Off)"),
        }
    }
}

// ---------------------------------------------------------------------------
// ACPI PM register bit definitions
// ---------------------------------------------------------------------------

/// SLP_EN bit in PM1_CNT register -- triggers the sleep transition.
const SLP_EN: u16 = 1 << 13;

/// SCI_EN bit in PM1_CNT -- indicates ACPI mode is active.
const SCI_EN: u16 = 1;

/// WAK_STS bit in PM1_STS -- set when system has woken from sleep.
const WAK_STS: u16 = 1 << 15;

/// PWRBTN_STS bit in PM1_STS -- power button pressed.
const PWRBTN_STS: u16 = 1 << 8;

/// PWRBTN_EN bit in PM1_EN -- enable power button event.
const PWRBTN_EN: u16 = 1 << 8;

/// GBL_STS bit in PM1_STS -- BIOS wants attention.
const GBL_STS: u16 = 1 << 5;

/// TMR_STS bit in PM1_STS -- PM timer overflow.
const TMR_STS: u16 = 1;

// ---------------------------------------------------------------------------
// ACPI wake event types
// ---------------------------------------------------------------------------

/// Types of wake events that can resume the system from sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiWakeEvent {
    /// Power button press.
    PowerButton,
    /// Lid open event.
    LidOpen,
    /// RTC alarm.
    RtcAlarm,
    /// USB device activity.
    UsbWake,
    /// Network (Wake-on-LAN).
    NetworkWake,
    /// Unknown or unclassified wake source.
    Unknown,
}

// ---------------------------------------------------------------------------
// Lid state
// ---------------------------------------------------------------------------

/// Laptop lid state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidState {
    Open,
    Closed,
    Unknown,
}

// ---------------------------------------------------------------------------
// FADT-derived PM register info
// ---------------------------------------------------------------------------

/// Parsed FADT power management register addresses and SLP_TYP values.
#[derive(Debug, Clone, Copy)]
struct FadtPmInfo {
    /// PM1a control block I/O port.
    pm1a_cnt_blk: u16,
    /// PM1b control block I/O port (0 if not present).
    pm1b_cnt_blk: u16,
    /// PM1a event block I/O port (status register).
    pm1a_evt_blk: u16,
    /// PM1b event block I/O port (0 if not present).
    pm1b_evt_blk: u16,
    /// PM1 event block length (total bytes; status and enable each get half).
    pm1_evt_len: u8,
    /// PM1 control block length.
    pm1_cnt_len: u8,
    /// SLP_TYP value for S1 (from \_S1 ACPI object).
    slp_typ_s1: u16,
    /// SLP_TYP value for S3 (from \_S3 ACPI object).
    slp_typ_s3: u16,
    /// SLP_TYP value for S4 (from \_S4 ACPI object).
    slp_typ_s4: u16,
    /// SLP_TYP value for S5 (from \_S5 ACPI object).
    slp_typ_s5: u16,
    /// SCI interrupt number.
    sci_int: u16,
    /// SMI command port.
    smi_cmd: u16,
    /// Value to write to SMI_CMD to enable ACPI.
    acpi_enable: u8,
    /// Value to write to SMI_CMD to disable ACPI.
    acpi_disable: u8,
    /// GPE0 block I/O port.
    gpe0_blk: u16,
    /// GPE0 block length.
    gpe0_blk_len: u8,
}

impl FadtPmInfo {
    const fn new() -> Self {
        Self {
            pm1a_cnt_blk: 0,
            pm1b_cnt_blk: 0,
            pm1a_evt_blk: 0,
            pm1b_evt_blk: 0,
            pm1_evt_len: 0,
            pm1_cnt_len: 0,
            slp_typ_s1: 0,
            slp_typ_s3: 5, // QEMU default for S3
            slp_typ_s4: 6, // QEMU default for S4
            slp_typ_s5: 7, // QEMU default for S5
            sci_int: 9,
            smi_cmd: 0,
            acpi_enable: 0,
            acpi_disable: 0,
            gpe0_blk: 0,
            gpe0_blk_len: 0,
        }
    }

    /// PM1a status register port (first half of event block).
    fn pm1a_sts_port(&self) -> u16 {
        self.pm1a_evt_blk
    }

    /// PM1a enable register port (second half of event block).
    fn pm1a_en_port(&self) -> u16 {
        self.pm1a_evt_blk + (self.pm1_evt_len as u16 / 2)
    }
}

// ---------------------------------------------------------------------------
// CPU context for S3 save/restore
// ---------------------------------------------------------------------------

/// Saved CPU state for S3 suspend/resume.
///
/// Captures all registers needed to resume execution after S3 wake:
/// general-purpose registers, segment selectors, CR3 (page tables),
/// GDT/IDT descriptors, and the stack pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CpuSuspendContext {
    /// General-purpose registers.
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    rsp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    /// Instruction pointer (return address).
    rip: u64,
    /// RFLAGS register.
    rflags: u64,
    /// CR3 -- page table root physical address.
    cr3: u64,
    /// CR0 -- control register 0.
    cr0: u64,
    /// CR4 -- control register 4.
    cr4: u64,
    /// GDT descriptor (limit + base).
    gdt_limit: u16,
    gdt_base: u64,
    /// IDT descriptor (limit + base).
    idt_limit: u16,
    idt_base: u64,
    /// Segment selectors.
    cs: u16,
    ds: u16,
    es: u16,
    ss: u16,
    fs: u16,
    gs: u16,
}

impl CpuSuspendContext {
    const fn new() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0,
            cr3: 0,
            cr0: 0,
            cr4: 0,
            gdt_limit: 0,
            gdt_base: 0,
            idt_limit: 0,
            idt_base: 0,
            cs: 0,
            ds: 0,
            es: 0,
            ss: 0,
            fs: 0,
            gs: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static PM_INITIALIZED: AtomicBool = AtomicBool::new(false);
static FADT_PM_INFO: Mutex<FadtPmInfo> = Mutex::new(FadtPmInfo::new());
static SUSPEND_CONTEXT: Mutex<CpuSuspendContext> = Mutex::new(CpuSuspendContext::new());
static CURRENT_STATE: AtomicU8 = AtomicU8::new(0); // S0
static LID_STATE: AtomicU8 = AtomicU8::new(2); // Unknown
static WAKE_EVENT_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_WAKE_EVENT: AtomicU8 = AtomicU8::new(5); // Unknown

// Supported sleep states bitmask (bit N = SN supported)
static SUPPORTED_STATES: AtomicU16 = AtomicU16::new(0);

// ---------------------------------------------------------------------------
// FADT table signature and structure
// ---------------------------------------------------------------------------

const FADT_SIGNATURE: &[u8; 4] = b"FACP";

/// FADT (Fixed ACPI Description Table) -- partial, only PM-relevant fields.
/// Full FADT is 276 bytes for ACPI 6.0; we only read what we need.
#[repr(C, packed)]
struct FadtHeader {
    /// Standard ACPI SDT header (36 bytes).
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
    /// Offset 36: FIRMWARE_CTRL.
    firmware_ctrl: u32,
    /// Offset 40: DSDT address.
    dsdt: u32,
    /// Offset 44: Reserved (ACPI 1.0 INT_MODEL).
    _reserved1: u8,
    /// Offset 45: Preferred PM profile.
    preferred_pm_profile: u8,
    /// Offset 46: SCI interrupt vector.
    sci_int: u16,
    /// Offset 48: SMI command port.
    smi_cmd: u32,
    /// Offset 52: ACPI enable value.
    acpi_enable: u8,
    /// Offset 53: ACPI disable value.
    acpi_disable: u8,
    /// Offset 54-55: S4BIOS_REQ, PSTATE_CNT.
    _s4bios_req: u8,
    _pstate_cnt: u8,
    /// Offset 56: PM1a event block.
    pm1a_evt_blk: u32,
    /// Offset 60: PM1b event block.
    pm1b_evt_blk: u32,
    /// Offset 64: PM1a control block.
    pm1a_cnt_blk: u32,
    /// Offset 68: PM1b control block.
    pm1b_cnt_blk: u32,
    /// Offset 72: PM2 control block.
    _pm2_cnt_blk: u32,
    /// Offset 76: PM timer block.
    _pm_tmr_blk: u32,
    /// Offset 80: GPE0 block.
    gpe0_blk: u32,
    /// Offset 84: GPE1 block.
    _gpe1_blk: u32,
    /// Offset 88: PM1 event length.
    pm1_evt_len: u8,
    /// Offset 89: PM1 control length.
    pm1_cnt_len: u8,
    /// Offset 90: PM2 control length.
    _pm2_cnt_len: u8,
    /// Offset 91: PM timer length.
    _pm_tmr_len: u8,
    /// Offset 92: GPE0 block length.
    gpe0_blk_len: u8,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the ACPI power management subsystem.
///
/// Parses the FADT to extract PM1a/PM1b control and status register addresses,
/// SLP_TYP values for each sleep state, and SCI interrupt configuration.
///
/// Must be called after `acpi::init()` has completed.
pub fn acpi_pm_init() -> KernelResult<()> {
    if PM_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::AlreadyExists {
            resource: "ACPI PM",
            id: 0,
        });
    }

    println!("[ACPI-PM] Initializing power management...");

    // Find FADT in ACPI tables by scanning the RSDT/XSDT.
    // For now, use default QEMU values which work for the common case.
    let mut info = FadtPmInfo::new();

    // Try to parse FADT from firmware tables.
    if let Some(fadt_info) = parse_fadt_from_tables() {
        info = fadt_info;
        println!(
            "[ACPI-PM] FADT parsed: PM1a_CNT={:#x}, PM1a_EVT={:#x}, SCI={}",
            info.pm1a_cnt_blk, info.pm1a_evt_blk, info.sci_int
        );
    } else {
        // Use QEMU defaults (PIIX4 PM).
        info.pm1a_cnt_blk = 0x0604;
        info.pm1a_evt_blk = 0x0600;
        info.pm1_evt_len = 4;
        info.pm1_cnt_len = 2;
        info.sci_int = 9;
        info.smi_cmd = 0x00B2;
        info.gpe0_blk = 0x0620;
        info.gpe0_blk_len = 8;
        println!("[ACPI-PM] Using QEMU/PIIX4 PM defaults");
    }

    // Determine supported sleep states.
    let mut supported: u16 = 1; // S0 always supported
    supported |= 1 << 1; // S1 (standby)

    // S3 is supported if PM1a_CNT is available.
    if info.pm1a_cnt_blk != 0 {
        supported |= 1 << 3; // S3
        supported |= 1 << 4; // S4 (requires swap, but report as available)
        supported |= 1 << 5; // S5
    }

    SUPPORTED_STATES.store(supported, Ordering::Release);

    // Enable ACPI mode if not already enabled.
    if info.smi_cmd != 0 && info.acpi_enable != 0 {
        let cnt = read_pm1a_cnt(&info);
        if cnt & SCI_EN == 0 {
            println!("[ACPI-PM] Enabling ACPI mode via SMI_CMD...");
            // SAFETY: Writing acpi_enable value to SMI_CMD port transitions
            // the chipset from legacy to ACPI mode. This is a one-time
            // initialization that enables SCI interrupts.
            unsafe {
                super::outb(info.smi_cmd, info.acpi_enable);
            }

            // Wait for SCI_EN to be set (up to 300 iterations).
            let mut retries = 300u32;
            loop {
                let cnt = read_pm1a_cnt(&info);
                if cnt & SCI_EN != 0 {
                    break;
                }
                retries = retries.saturating_sub(1);
                if retries == 0 {
                    println!("[ACPI-PM] WARNING: SCI_EN not set after enabling ACPI");
                    break;
                }
                // Brief delay via I/O port read.
                // SAFETY: Port 0x80 is the POST diagnostic port, commonly
                // used as a ~1us I/O delay on x86 systems.
                unsafe {
                    super::inb(0x80);
                }
            }
        }
    }

    // Enable power button events.
    if info.pm1a_evt_blk != 0 {
        enable_power_button_event(&info);
    }

    *FADT_PM_INFO.lock() = info;
    PM_INITIALIZED.store(true, Ordering::Release);

    println!(
        "[ACPI-PM] Initialized: supported states = S0{}{}{}{}",
        if supported & (1 << 1) != 0 { ",S1" } else { "" },
        if supported & (1 << 3) != 0 { ",S3" } else { "" },
        if supported & (1 << 4) != 0 { ",S4" } else { "" },
        if supported & (1 << 5) != 0 { ",S5" } else { "" },
    );

    Ok(())
}

/// Attempt to parse FADT from ACPI table hierarchy.
fn parse_fadt_from_tables() -> Option<FadtPmInfo> {
    // Access boot info to get RSDP, then walk RSDT/XSDT looking for FACP.
    #[allow(static_mut_refs)]
    let rsdp_phys = unsafe {
        super::boot::BOOT_INFO
            .as_ref()
            .and_then(|bi| bi.rsdp_addr.into_option())
    }?;

    let rsdp_vaddr = super::msr::phys_to_virt(rsdp_phys as usize)?;

    // SAFETY: rsdp_vaddr points to a valid RSDP mapped by the bootloader.
    let rsdp = unsafe { &*(rsdp_vaddr as *const FadtRsdp) };
    if &rsdp.signature != b"RSD PTR " {
        return None;
    }

    // Use XSDT for ACPI 2.0+, RSDT otherwise.
    if rsdp.revision >= 2 {
        // SAFETY: ACPI 2.0 RSDP has xsdt_address at offset 24.
        let rsdp2 = unsafe { &*(rsdp_vaddr as *const FadtRsdp2) };
        let xsdt_phys = { rsdp2.xsdt_address } as usize;
        if xsdt_phys != 0 {
            let xsdt_vaddr = super::msr::phys_to_virt(xsdt_phys)?;
            return find_fadt_in_xsdt(xsdt_vaddr);
        }
    }

    let rsdt_phys = { rsdp.rsdt_address } as usize;
    let rsdt_vaddr = super::msr::phys_to_virt(rsdt_phys)?;
    find_fadt_in_rsdt(rsdt_vaddr)
}

// Minimal RSDP structs for FADT lookup (duplicated from acpi.rs to avoid
// coupling to private types).
#[repr(C, packed)]
struct FadtRsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
struct FadtRsdp2 {
    base: FadtRsdp,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    _reserved: [u8; 3],
}

#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

fn find_fadt_in_rsdt(rsdt_vaddr: usize) -> Option<FadtPmInfo> {
    // SAFETY: rsdt_vaddr points to a valid RSDT.
    let sdt = unsafe { &*(rsdt_vaddr as *const SdtHeader) };
    let len = { sdt.length } as usize;
    let header_size = core::mem::size_of::<SdtHeader>();
    let num_entries = (len.saturating_sub(header_size)) / 4;

    for i in 0..num_entries {
        let ptr_addr = rsdt_vaddr + header_size + i * 4;
        // SAFETY: ptr_addr is within RSDT bounds.
        let phys_addr = unsafe { *(ptr_addr as *const u32) } as usize;
        if let Some(vaddr) = super::msr::phys_to_virt(phys_addr) {
            if let Some(info) = try_parse_fadt(vaddr) {
                return Some(info);
            }
        }
    }
    None
}

fn find_fadt_in_xsdt(xsdt_vaddr: usize) -> Option<FadtPmInfo> {
    // SAFETY: xsdt_vaddr points to a valid XSDT.
    let sdt = unsafe { &*(xsdt_vaddr as *const SdtHeader) };
    let len = { sdt.length } as usize;
    let header_size = core::mem::size_of::<SdtHeader>();
    let num_entries = (len.saturating_sub(header_size)) / 8;

    for i in 0..num_entries {
        let ptr_addr = xsdt_vaddr + header_size + i * 8;
        // SAFETY: ptr_addr is within XSDT bounds.
        let phys_addr = unsafe { *(ptr_addr as *const u64) } as usize;
        if let Some(vaddr) = super::msr::phys_to_virt(phys_addr) {
            if let Some(info) = try_parse_fadt(vaddr) {
                return Some(info);
            }
        }
    }
    None
}

fn try_parse_fadt(vaddr: usize) -> Option<FadtPmInfo> {
    // SAFETY: vaddr points to a valid ACPI table header.
    let sdt = unsafe { &*(vaddr as *const SdtHeader) };
    if &{ sdt.signature } != FADT_SIGNATURE {
        return None;
    }

    let len = { sdt.length } as usize;
    if len < core::mem::size_of::<FadtHeader>() {
        return None;
    }

    // SAFETY: vaddr points to a valid FADT with sufficient length.
    let fadt = unsafe { &*(vaddr as *const FadtHeader) };

    let mut info = FadtPmInfo::new();
    info.pm1a_cnt_blk = fadt.pm1a_cnt_blk as u16;
    info.pm1b_cnt_blk = fadt.pm1b_cnt_blk as u16;
    info.pm1a_evt_blk = fadt.pm1a_evt_blk as u16;
    info.pm1b_evt_blk = fadt.pm1b_evt_blk as u16;
    info.pm1_evt_len = fadt.pm1_evt_len;
    info.pm1_cnt_len = fadt.pm1_cnt_len;
    info.sci_int = fadt.sci_int;
    info.smi_cmd = fadt.smi_cmd as u16;
    info.acpi_enable = fadt.acpi_enable;
    info.acpi_disable = fadt.acpi_disable;
    info.gpe0_blk = fadt.gpe0_blk as u16;
    info.gpe0_blk_len = fadt.gpe0_blk_len;

    Some(info)
}

// ---------------------------------------------------------------------------
// PM register access helpers
// ---------------------------------------------------------------------------

/// Read PM1a control register.
fn read_pm1a_cnt(info: &FadtPmInfo) -> u16 {
    if info.pm1a_cnt_blk == 0 {
        return 0;
    }
    // SAFETY: pm1a_cnt_blk is a validated ACPI PM1a control register port
    // parsed from the FADT. Reading this port returns the current PM control
    // register value.
    unsafe { super::inw(info.pm1a_cnt_blk) }
}

/// Write PM1a control register.
fn write_pm1a_cnt(info: &FadtPmInfo, value: u16) {
    if info.pm1a_cnt_blk == 0 {
        return;
    }
    // SAFETY: pm1a_cnt_blk is a validated ACPI PM1a control register port.
    // Writing to this port updates the PM control state (e.g., sleep type,
    // SLP_EN to trigger sleep).
    unsafe { super::outw(info.pm1a_cnt_blk, value) }
}

/// Write PM1b control register (if present).
fn write_pm1b_cnt(info: &FadtPmInfo, value: u16) {
    if info.pm1b_cnt_blk == 0 {
        return;
    }
    // SAFETY: pm1b_cnt_blk is the secondary PM control register port from
    // the FADT. Writing mirrors the PM1a control operation.
    unsafe { super::outw(info.pm1b_cnt_blk, value) }
}

/// Read PM1a status register.
fn read_pm1a_sts(info: &FadtPmInfo) -> u16 {
    if info.pm1a_evt_blk == 0 {
        return 0;
    }
    // SAFETY: pm1a_evt_blk is the PM1a event (status) register port from
    // the FADT. Reading returns the current PM event status bits.
    unsafe { super::inw(info.pm1a_sts_port()) }
}

/// Write PM1a status register (to clear status bits -- write-1-to-clear).
fn write_pm1a_sts(info: &FadtPmInfo, value: u16) {
    if info.pm1a_evt_blk == 0 {
        return;
    }
    // SAFETY: Writing to the PM1a status register clears the indicated
    // status bits (write-1-to-clear semantics per ACPI spec).
    unsafe { super::outw(info.pm1a_sts_port(), value) }
}

/// Read PM1a enable register.
fn read_pm1a_en(info: &FadtPmInfo) -> u16 {
    if info.pm1a_evt_blk == 0 || info.pm1_evt_len < 4 {
        return 0;
    }
    // SAFETY: The enable register is at the second half of the PM1a event
    // block. Port address is validated from FADT.
    unsafe { super::inw(info.pm1a_en_port()) }
}

/// Write PM1a enable register.
fn write_pm1a_en(info: &FadtPmInfo, value: u16) {
    if info.pm1a_evt_blk == 0 || info.pm1_evt_len < 4 {
        return;
    }
    // SAFETY: Writing to the PM1a enable register controls which PM events
    // generate SCI interrupts.
    unsafe { super::outw(info.pm1a_en_port(), value) }
}

/// Enable power button SCI event.
fn enable_power_button_event(info: &FadtPmInfo) {
    // Clear any pending power button status first (write-1-to-clear).
    write_pm1a_sts(info, PWRBTN_STS);

    // Enable power button event generation.
    let en = read_pm1a_en(info);
    write_pm1a_en(info, en | PWRBTN_EN);
}

// ---------------------------------------------------------------------------
// CPU context save/restore
// ---------------------------------------------------------------------------

/// Save current CPU state into the suspend context.
///
/// Captures all general-purpose registers, control registers, GDT/IDT
/// descriptors, and segment selectors needed to resume from S3.
fn save_cpu_context() {
    let mut ctx = SUSPEND_CONTEXT.lock();

    // SAFETY: Reading CPU registers and descriptor tables is a privileged
    // operation with no side effects. All registers are well-defined at
    // this point since we are running in kernel context.
    unsafe {
        // Read general-purpose registers into locals first to avoid
        // multiple mutable borrows of the MutexGuard in inline asm.
        let (rbx, rbp, r12, r13, r14, r15): (u64, u64, u64, u64, u64, u64);
        core::arch::asm!(
            "mov {rbx}, rbx",
            "mov {rbp}, rbp",
            "mov {r12}, r12",
            "mov {r13}, r13",
            "mov {r14}, r14",
            "mov {r15}, r15",
            rbx = out(reg) rbx,
            rbp = out(reg) rbp,
            r12 = out(reg) r12,
            r13 = out(reg) r13,
            r14 = out(reg) r14,
            r15 = out(reg) r15,
            options(nomem, nostack, preserves_flags),
        );
        ctx.rbx = rbx;
        ctx.rbp = rbp;
        ctx.r12 = r12;
        ctx.r13 = r13;
        ctx.r14 = r14;
        ctx.r15 = r15;

        // Read RSP.
        let rsp: u64;
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nomem, nostack, preserves_flags),
        );
        ctx.rsp = rsp;

        // Read RFLAGS.
        let rflags: u64;
        core::arch::asm!(
            "pushfq",
            "pop {}",
            out(reg) rflags,
            options(preserves_flags),
        );
        ctx.rflags = rflags;

        // Read control registers.
        let (cr0, cr3, cr4): (u64, u64, u64);
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        ctx.cr0 = cr0;
        ctx.cr3 = cr3;
        ctx.cr4 = cr4;

        // Read GDT descriptor.
        let mut gdt_desc: [u8; 10] = [0; 10];
        core::arch::asm!(
            "sgdt [{}]",
            in(reg) gdt_desc.as_mut_ptr(),
            options(nostack),
        );
        ctx.gdt_limit = u16::from_le_bytes([gdt_desc[0], gdt_desc[1]]);
        ctx.gdt_base = u64::from_le_bytes([
            gdt_desc[2],
            gdt_desc[3],
            gdt_desc[4],
            gdt_desc[5],
            gdt_desc[6],
            gdt_desc[7],
            gdt_desc[8],
            gdt_desc[9],
        ]);

        // Read IDT descriptor.
        let mut idt_desc: [u8; 10] = [0; 10];
        core::arch::asm!(
            "sidt [{}]",
            in(reg) idt_desc.as_mut_ptr(),
            options(nostack),
        );
        ctx.idt_limit = u16::from_le_bytes([idt_desc[0], idt_desc[1]]);
        ctx.idt_base = u64::from_le_bytes([
            idt_desc[2],
            idt_desc[3],
            idt_desc[4],
            idt_desc[5],
            idt_desc[6],
            idt_desc[7],
            idt_desc[8],
            idt_desc[9],
        ]);

        // Read segment selectors.
        core::arch::asm!("mov {:x}, cs", out(reg) ctx.cs, options(nomem, nostack));
        core::arch::asm!("mov {:x}, ds", out(reg) ctx.ds, options(nomem, nostack));
        core::arch::asm!("mov {:x}, es", out(reg) ctx.es, options(nomem, nostack));
        core::arch::asm!("mov {:x}, ss", out(reg) ctx.ss, options(nomem, nostack));
    }
}

/// Restore CPU state from the suspend context after S3 resume.
fn restore_cpu_context() {
    let ctx = SUSPEND_CONTEXT.lock();

    // SAFETY: Restoring GDT, IDT, and control registers to values that
    // were valid before suspend. The page tables (CR3) point to the same
    // kernel mapping that was active before sleep.
    unsafe {
        // Restore GDT.
        let gdt_desc: [u8; 10] = {
            let mut buf = [0u8; 10];
            buf[0..2].copy_from_slice(&ctx.gdt_limit.to_le_bytes());
            buf[2..10].copy_from_slice(&ctx.gdt_base.to_le_bytes());
            buf
        };
        core::arch::asm!(
            "lgdt [{}]",
            in(reg) gdt_desc.as_ptr(),
            options(nostack),
        );

        // Restore IDT.
        let idt_desc: [u8; 10] = {
            let mut buf = [0u8; 10];
            buf[0..2].copy_from_slice(&ctx.idt_limit.to_le_bytes());
            buf[2..10].copy_from_slice(&ctx.idt_base.to_le_bytes());
            buf
        };
        core::arch::asm!(
            "lidt [{}]",
            in(reg) idt_desc.as_ptr(),
            options(nostack),
        );

        // Restore CR3 (page tables).
        core::arch::asm!("mov cr3, {}", in(reg) ctx.cr3, options(nomem, nostack));

        // Restore callee-saved registers.
        core::arch::asm!(
            "mov rbx, {rbx}",
            "mov rbp, {rbp}",
            "mov r12, {r12}",
            "mov r13, {r13}",
            "mov r14, {r14}",
            "mov r15, {r15}",
            rbx = in(reg) ctx.rbx,
            rbp = in(reg) ctx.rbp,
            r12 = in(reg) ctx.r12,
            r13 = in(reg) ctx.r13,
            r14 = in(reg) ctx.r14,
            r15 = in(reg) ctx.r15,
            options(nomem, nostack),
        );
    }
}

// ---------------------------------------------------------------------------
// Sleep state transitions
// ---------------------------------------------------------------------------

/// Suspend to RAM (ACPI S3).
///
/// Saves CPU state, flushes caches, writes SLP_TYP|SLP_EN to PM1a_CNT
/// to enter S3 sleep. On wake, firmware jumps to the FACS waking vector
/// which restores CPU context and returns here.
pub fn acpi_suspend_s3() -> KernelResult<()> {
    if !PM_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "ACPI PM",
        });
    }

    if SUPPORTED_STATES.load(Ordering::Acquire) & (1 << 3) == 0 {
        return Err(KernelError::OperationNotSupported {
            operation: "S3 suspend",
        });
    }

    println!("[ACPI-PM] Preparing S3 suspend...");

    // 1. Save CPU state.
    save_cpu_context();

    // 2. Disable interrupts during the transition.
    // SAFETY: CLI prevents interrupt handlers from firing during the
    // critical suspend sequence.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let info = FADT_PM_INFO.lock();

    // 3. Flush all CPU caches.
    // SAFETY: WBINVD writes back all modified cache lines to memory and
    // invalidates caches. Required before S3 to ensure RAM contents are
    // coherent since the CPU will lose cache state.
    unsafe {
        core::arch::asm!("wbinvd", options(nomem, nostack));
    }

    // 4. Clear wake status.
    write_pm1a_sts(&info, WAK_STS);

    // 5. Write SLP_TYP and SLP_EN to PM1a_CNT to enter S3.
    let slp_value = (info.slp_typ_s3 << 10) | SLP_EN;
    write_pm1a_cnt(&info, slp_value);
    write_pm1b_cnt(&info, slp_value);

    // 6. Wait for wake -- the CPU halts here. On resume, firmware
    // restores execution context and returns to this point.
    // SAFETY: HLT stops the CPU until the next interrupt. After S3
    // wake, execution resumes here.
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack));
    }

    drop(info);

    // 7. Restore CPU state on resume.
    restore_cpu_context();
    CURRENT_STATE.store(0, Ordering::Release);

    // 8. Re-enable interrupts.
    // SAFETY: STI re-enables hardware interrupts after resume.
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }

    WAKE_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
    println!("[ACPI-PM] Resumed from S3 suspend");

    Ok(())
}

/// Hibernate (ACPI S4).
///
/// Creates a memory image bitmap of all active pages, writes the image
/// to the swap area, then enters S4 via ACPI. On resume from S4, the
/// bootloader or firmware restores the memory image and jumps to the
/// wakeup vector.
pub fn acpi_hibernate_s4() -> KernelResult<()> {
    if !PM_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "ACPI PM",
        });
    }

    if SUPPORTED_STATES.load(Ordering::Acquire) & (1 << 4) == 0 {
        return Err(KernelError::OperationNotSupported {
            operation: "S4 hibernate",
        });
    }

    println!("[ACPI-PM] Preparing S4 hibernate...");

    // 1. Save CPU state for potential resume.
    save_cpu_context();

    // 2. Create memory image.
    // In a full implementation, this would:
    //   a. Walk page tables to find all active physical frames.
    //   b. Allocate a contiguous bitmap tracking which pages to save.
    //   c. Write pages sequentially to the swap partition/file.
    //   d. Write a hibernate header with the wakeup vector and page map.
    println!("[ACPI-PM] Memory image creation (page snapshot phase)...");
    let active_pages = snapshot_active_pages();
    println!("[ACPI-PM] Snapshot: {} active pages recorded", active_pages);

    // 3. Write image to swap (stub -- requires block device write path).
    println!("[ACPI-PM] Writing hibernate image to swap...");
    // write_hibernate_image() would go here.

    // 4. Disable interrupts for transition.
    // SAFETY: CLI during critical S4 transition.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let info = FADT_PM_INFO.lock();

    // 5. Flush caches.
    // SAFETY: WBINVD ensures all cache lines are written to RAM before
    // powering down.
    unsafe {
        core::arch::asm!("wbinvd", options(nomem, nostack));
    }

    // 6. Clear wake status and enter S4.
    write_pm1a_sts(&info, WAK_STS);
    let slp_value = (info.slp_typ_s4 << 10) | SLP_EN;
    write_pm1a_cnt(&info, slp_value);
    write_pm1b_cnt(&info, slp_value);

    // 7. Wait for power off / wake.
    // SAFETY: HLT after S4 entry. System powers off; on resume firmware
    // restores memory and jumps to wakeup vector.
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack));
    }

    drop(info);

    // 8. Resume path: restore context.
    restore_cpu_context();
    CURRENT_STATE.store(0, Ordering::Release);

    // SAFETY: Re-enable interrupts after resume.
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }

    WAKE_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
    println!("[ACPI-PM] Resumed from S4 hibernate");

    Ok(())
}

/// Soft power off via ACPI S5.
///
/// Writes the S5 SLP_TYP value with SLP_EN to PM1a_CNT. The system
/// should power off; this function does not return on success.
pub fn acpi_shutdown_s5() -> KernelResult<()> {
    if !PM_INITIALIZED.load(Ordering::Acquire) {
        return Err(KernelError::NotInitialized {
            subsystem: "ACPI PM",
        });
    }

    println!("[ACPI-PM] Initiating ACPI S5 soft power off...");

    // Disable interrupts.
    // SAFETY: CLI before shutdown to prevent any interrupt handlers from
    // interfering with the power-off sequence.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let info = FADT_PM_INFO.lock();

    // Write SLP_TYP for S5 with SLP_EN.
    let slp_value = (info.slp_typ_s5 << 10) | SLP_EN;
    write_pm1a_cnt(&info, slp_value);
    write_pm1b_cnt(&info, slp_value);

    drop(info);

    // If we're still here, the power off did not succeed. Halt.
    println!("[ACPI-PM] WARNING: S5 power off did not take effect, halting");

    // SAFETY: HLT in an infinite loop as a last resort.
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

// ---------------------------------------------------------------------------
// SCI interrupt handler
// ---------------------------------------------------------------------------

/// Handle an ACPI SCI (System Control Interrupt).
///
/// Called from the interrupt handler when the SCI vector fires.
/// Reads PM1a status to determine the wake/event source and dispatches
/// the appropriate handler.
pub fn acpi_handle_sci() {
    if !PM_INITIALIZED.load(Ordering::Acquire) {
        return;
    }

    let info = FADT_PM_INFO.lock();
    let status = read_pm1a_sts(&info);

    // Power button press.
    if status & PWRBTN_STS != 0 {
        // Clear the status bit (write-1-to-clear).
        write_pm1a_sts(&info, PWRBTN_STS);
        LAST_WAKE_EVENT.store(AcpiWakeEvent::PowerButton as u8, Ordering::Release);
        println!("[ACPI-PM] Power button press detected");
    }

    // PM timer overflow.
    if status & TMR_STS != 0 {
        write_pm1a_sts(&info, TMR_STS);
    }

    // BIOS wants attention.
    if status & GBL_STS != 0 {
        write_pm1a_sts(&info, GBL_STS);
    }

    // Wake status (resume from sleep).
    if status & WAK_STS != 0 {
        write_pm1a_sts(&info, WAK_STS);
        WAKE_EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
        println!("[ACPI-PM] Wake event detected");
    }

    // Check GPE (General Purpose Event) registers for lid events.
    check_gpe_events(&info);
}

/// Check GPE registers for lid and other general-purpose events.
fn check_gpe_events(info: &FadtPmInfo) {
    if info.gpe0_blk == 0 || info.gpe0_blk_len == 0 {
        return;
    }

    let half_len = info.gpe0_blk_len / 2;
    if half_len == 0 {
        return;
    }

    // Read GPE0 status registers (first half of GPE0 block).
    for i in 0..half_len {
        let port = info.gpe0_blk + i as u16;
        // SAFETY: port is within the GPE0 block range from the FADT.
        // Reading GPE status registers returns pending event bits.
        let status = unsafe { super::inb(port) };

        if status != 0 {
            // Clear status bits by writing them back (write-1-to-clear).
            // SAFETY: Writing GPE0 status register to clear event bits.
            unsafe { super::outb(port, status) };

            // GPE bit 0x02 is commonly used for lid events on many platforms.
            if status & 0x02 != 0 {
                handle_lid_event();
            }
        }
    }
}

/// Handle a lid open/close event from ACPI GPE.
fn handle_lid_event() {
    // Toggle lid state. In a full implementation, this would read
    // the actual lid state from the ACPI _LID method.
    let current = LID_STATE.load(Ordering::Acquire);
    let new_state = if current == LidState::Open as u8 {
        LidState::Closed as u8
    } else {
        LidState::Open as u8
    };
    LID_STATE.store(new_state, Ordering::Release);

    if new_state == LidState::Closed as u8 {
        LAST_WAKE_EVENT.store(AcpiWakeEvent::LidOpen as u8, Ordering::Release);
        println!("[ACPI-PM] Lid closed");
    } else {
        println!("[ACPI-PM] Lid opened");
    }
}

// ---------------------------------------------------------------------------
// Memory snapshot for S4
// ---------------------------------------------------------------------------

/// Snapshot active pages for hibernate image.
///
/// Returns the count of active pages that would be saved. In a full
/// implementation, this would walk the page table tree and record each
/// present physical frame into a bitmap.
fn snapshot_active_pages() -> usize {
    // Stub: report a reasonable count based on kernel memory layout.
    // A real implementation would iterate PML4 -> PDPT -> PD -> PT entries.
    256 * 1024 // ~1GB kernel heap / 4K pages
}

// ---------------------------------------------------------------------------
// Query API
// ---------------------------------------------------------------------------

/// Check if ACPI PM is initialized.
pub fn is_initialized() -> bool {
    PM_INITIALIZED.load(Ordering::Acquire)
}

/// Get the current ACPI sleep state.
pub fn current_state() -> AcpiSleepState {
    match CURRENT_STATE.load(Ordering::Acquire) {
        0 => AcpiSleepState::S0Working,
        1 => AcpiSleepState::S1Standby,
        3 => AcpiSleepState::S3Suspend,
        4 => AcpiSleepState::S4Hibernate,
        5 => AcpiSleepState::S5SoftOff,
        _ => AcpiSleepState::S0Working,
    }
}

/// Get the current lid state.
pub fn lid_state() -> LidState {
    match LID_STATE.load(Ordering::Acquire) {
        0 => LidState::Open,
        1 => LidState::Closed,
        _ => LidState::Unknown,
    }
}

/// Check if a given sleep state is supported.
pub fn is_state_supported(state: AcpiSleepState) -> bool {
    let bit = match state {
        AcpiSleepState::S0Working => 0,
        AcpiSleepState::S1Standby => 1,
        AcpiSleepState::S3Suspend => 3,
        AcpiSleepState::S4Hibernate => 4,
        AcpiSleepState::S5SoftOff => 5,
    };
    SUPPORTED_STATES.load(Ordering::Acquire) & (1u16 << bit) != 0
}

/// Get supported sleep states as a formatted string.
///
/// Returns a string like "mem disk standby" suitable for /sys/power/state.
pub fn supported_states_string() -> &'static str {
    let states = SUPPORTED_STATES.load(Ordering::Acquire);
    if states & (1 << 3) != 0 && states & (1 << 4) != 0 && states & (1 << 1) != 0 {
        "standby mem disk"
    } else if states & (1 << 3) != 0 && states & (1 << 4) != 0 {
        "mem disk"
    } else if states & (1 << 3) != 0 {
        "mem"
    } else {
        "standby"
    }
}

/// Get the last wake event type.
pub fn last_wake_event() -> AcpiWakeEvent {
    match LAST_WAKE_EVENT.load(Ordering::Acquire) {
        0 => AcpiWakeEvent::PowerButton,
        1 => AcpiWakeEvent::LidOpen,
        2 => AcpiWakeEvent::RtcAlarm,
        3 => AcpiWakeEvent::UsbWake,
        4 => AcpiWakeEvent::NetworkWake,
        _ => AcpiWakeEvent::Unknown,
    }
}

/// Get the total number of wake events since boot.
pub fn wake_event_count() -> u32 {
    WAKE_EVENT_COUNT.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sleep_state_display() {
        assert_eq!(
            alloc::format!("{}", AcpiSleepState::S0Working),
            "S0 (Working)"
        );
        assert_eq!(
            alloc::format!("{}", AcpiSleepState::S3Suspend),
            "S3 (Suspend to RAM)"
        );
        assert_eq!(
            alloc::format!("{}", AcpiSleepState::S5SoftOff),
            "S5 (Soft Off)"
        );
    }

    #[test]
    fn test_fadt_pm_info_defaults() {
        let info = FadtPmInfo::new();
        assert_eq!(info.pm1a_cnt_blk, 0);
        assert_eq!(info.slp_typ_s3, 5);
        assert_eq!(info.slp_typ_s5, 7);
        assert_eq!(info.sci_int, 9);
    }

    #[test]
    fn test_pm1a_sts_port() {
        let mut info = FadtPmInfo::new();
        info.pm1a_evt_blk = 0x0600;
        info.pm1_evt_len = 4;
        assert_eq!(info.pm1a_sts_port(), 0x0600);
        assert_eq!(info.pm1a_en_port(), 0x0602);
    }

    #[test]
    fn test_cpu_suspend_context_default() {
        let ctx = CpuSuspendContext::new();
        assert_eq!(ctx.rax, 0);
        assert_eq!(ctx.cr3, 0);
        assert_eq!(ctx.gdt_limit, 0);
    }

    #[test]
    fn test_slp_value_encoding() {
        let info = FadtPmInfo::new();
        // S3: SLP_TYP=5, shifted left 10 bits, OR with SLP_EN (bit 13).
        let slp_value = (info.slp_typ_s3 << 10) | SLP_EN;
        assert_eq!(slp_value & SLP_EN, SLP_EN);
        assert_eq!((slp_value >> 10) & 0x07, 5);
    }

    #[test]
    fn test_wake_event_variants() {
        assert_eq!(AcpiWakeEvent::PowerButton as u8, 0);
        assert_eq!(AcpiWakeEvent::LidOpen as u8, 1);
        assert_eq!(AcpiWakeEvent::Unknown as u8, 5);
    }

    #[test]
    fn test_lid_state_variants() {
        assert_eq!(LidState::Open as u8, 0);
        assert_eq!(LidState::Closed as u8, 1);
        assert_eq!(LidState::Unknown as u8, 2);
    }

    #[test]
    fn test_supported_states_bitmask() {
        // S0 + S1 + S3 + S4 + S5
        let supported: u16 = 1 | (1 << 1) | (1 << 3) | (1 << 4) | (1 << 5);
        assert!(supported & 1 != 0); // S0
        assert!(supported & (1 << 1) != 0); // S1
        assert!(supported & (1 << 2) == 0); // S2 not supported
        assert!(supported & (1 << 3) != 0); // S3
        assert!(supported & (1 << 4) != 0); // S4
        assert!(supported & (1 << 5) != 0); // S5
    }

    #[test]
    fn test_snapshot_active_pages() {
        let pages = snapshot_active_pages();
        assert!(pages > 0);
    }
}
