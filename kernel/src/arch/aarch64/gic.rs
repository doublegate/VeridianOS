//! AArch64 Generic Interrupt Controller (GICv2) driver for QEMU virt machine.
//!
//! This module implements GICv2 support for the QEMU virt platform, providing
//! interrupt distribution and CPU interface configuration. The GICv2 consists
//! of two main components:
//!
//! - **Distributor (GICD)**: Routes interrupts to CPU interfaces, manages
//!   enable/disable, priority, and target CPU for each interrupt line.
//! - **CPU Interface (GICC)**: Per-CPU interface for acknowledging interrupts,
//!   signaling end-of-interrupt, and priority masking.
//!
//! ## Interrupt ID Ranges
//!
//! - SGIs (Software Generated Interrupts): 0-15 -- inter-processor signaling
//! - PPIs (Private Peripheral Interrupts): 16-31 -- per-CPU peripherals (e.g.,
//!   PPI 30 = physical timer on QEMU virt)
//! - SPIs (Shared Peripheral Interrupts): 32-1019 -- shared device interrupts
//!
//! ## QEMU virt Machine Addresses
//!
//! - GICD base: `0x0800_0000`
//! - GICC base: `0x0801_0000`

use core::ptr;

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    sync::once_lock::GlobalState,
};

// ---------------------------------------------------------------------------
// QEMU virt machine GICv2 base addresses
// ---------------------------------------------------------------------------

/// GIC Distributor base address on QEMU virt machine.
const GICD_BASE: usize = 0x0800_0000;

/// GIC CPU Interface base address on QEMU virt machine.
const GICC_BASE: usize = 0x0801_0000;

// ---------------------------------------------------------------------------
// GIC Distributor (GICD) register offsets
// ---------------------------------------------------------------------------

/// Distributor Control Register -- enables/disables the distributor.
const GICD_CTLR: usize = 0x000;
/// Interrupt Controller Type Register -- reports number of interrupt lines.
const GICD_TYPER: usize = 0x004;
// Hardware register definition -- retained for completeness per ARM GICv2 spec
#[allow(dead_code)]
const GICD_IIDR: usize = 0x008;
/// Interrupt Group Registers (one bit per interrupt).
const GICD_IGROUPR: usize = 0x080;
/// Interrupt Set-Enable Registers (one bit per interrupt).
const GICD_ISENABLER: usize = 0x100;
/// Interrupt Clear-Enable Registers (one bit per interrupt).
const GICD_ICENABLER: usize = 0x180;
// Hardware register definitions -- retained for completeness per ARM GICv2 spec
#[allow(dead_code)]
const GICD_ISPENDR: usize = 0x200;
#[allow(dead_code)]
const GICD_ICPENDR: usize = 0x280;
/// Interrupt Priority Registers (one byte per interrupt).
const GICD_IPRIORITYR: usize = 0x400;
/// Interrupt Processor Targets Registers (one byte per interrupt).
const GICD_ITARGETSR: usize = 0x800;
/// Interrupt Configuration Registers (2 bits per interrupt).
const GICD_ICFGR: usize = 0xC00;

// ---------------------------------------------------------------------------
// GIC CPU Interface (GICC) register offsets
// ---------------------------------------------------------------------------

/// CPU Interface Control Register -- enables/disables the CPU interface.
const GICC_CTLR: usize = 0x000;
/// Interrupt Priority Mask Register -- filters interrupts by priority.
const GICC_PMR: usize = 0x004;
/// Binary Point Register -- controls priority grouping for preemption.
const GICC_BPR: usize = 0x008;
/// Interrupt Acknowledge Register -- read to acknowledge an interrupt.
const GICC_IAR: usize = 0x00C;
/// End of Interrupt Register -- write to signal interrupt handling complete.
const GICC_EOIR: usize = 0x010;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of interrupt lines supported by GICv2.
const GIC_MAX_IRQS: u32 = 1020;

/// Spurious interrupt ID returned by IAR when no interrupt is pending.
const GIC_SPURIOUS_IRQ: u32 = 1023;

/// Default priority for SPIs (lower numerical value = higher priority).
const DEFAULT_SPI_PRIORITY: u8 = 0xA0;

/// Physical timer PPI on QEMU virt machine (INTID 30).
#[allow(dead_code)] // Hardware constant -- needed when timer interrupts are enabled
pub const TIMER_PPI: u32 = 30;

// ---------------------------------------------------------------------------
// Global GIC instance
// ---------------------------------------------------------------------------

/// Global GIC instance, initialized once during boot.
///
/// Uses `GlobalState` (backed by `spin::Mutex<Option<T>>`) instead of
/// `OnceLock` because GIC init runs in Stage 1 before the heap allocator
/// is available. `OnceLock::set()` requires `Box::new()` which would
/// trigger an allocation failure panic on AArch64 at this early stage.
static GIC: GlobalState<Mutex<Gic>> = GlobalState::new();

// ---------------------------------------------------------------------------
// GIC state structure
// ---------------------------------------------------------------------------

/// GICv2 controller state.
///
/// Holds the base addresses for the distributor and CPU interface MMIO
/// regions and the total number of supported interrupt lines.
struct Gic {
    /// Base address of the GIC Distributor registers.
    gicd_base: usize,
    /// Base address of the GIC CPU Interface registers.
    gicc_base: usize,
    /// Total number of interrupt lines supported (read from GICD_TYPER).
    num_irqs: u32,
}

impl Gic {
    /// Create a new GIC instance with the given base addresses.
    const fn new(gicd_base: usize, gicc_base: usize) -> Self {
        Self {
            gicd_base,
            gicc_base,
            num_irqs: 0,
        }
    }

    // -----------------------------------------------------------------------
    // MMIO helpers
    // -----------------------------------------------------------------------

    /// Read a 32-bit value from a distributor register.
    fn gicd_read(&self, offset: usize) -> u32 {
        let addr = self.gicd_base + offset;
        // SAFETY: The distributor base address (0x0800_0000) is the MMIO region
        // for the GICv2 distributor on the QEMU virt machine. The offset is
        // validated by the caller to be a valid GICD register offset. Volatile
        // read is required for MMIO to prevent compiler reordering/elision.
        unsafe { ptr::read_volatile(addr as *const u32) }
    }

    /// Write a 32-bit value to a distributor register.
    fn gicd_write(&self, offset: usize, value: u32) {
        let addr = self.gicd_base + offset;
        // SAFETY: The distributor base address (0x0800_0000) is the MMIO region
        // for the GICv2 distributor on the QEMU virt machine. The offset is
        // validated by the caller to be a valid GICD register offset. Volatile
        // write is required for MMIO to ensure the write reaches the device.
        unsafe { ptr::write_volatile(addr as *mut u32, value) }
    }

    /// Read a 32-bit value from a CPU interface register.
    fn gicc_read(&self, offset: usize) -> u32 {
        let addr = self.gicc_base + offset;
        // SAFETY: The CPU interface base address (0x0801_0000) is the MMIO region
        // for the GICv2 CPU interface on the QEMU virt machine. The offset is
        // validated by the caller to be a valid GICC register offset. Volatile
        // read is required for MMIO to prevent compiler reordering/elision.
        unsafe { ptr::read_volatile(addr as *const u32) }
    }

    /// Write a 32-bit value to a CPU interface register.
    fn gicc_write(&self, offset: usize, value: u32) {
        let addr = self.gicc_base + offset;
        // SAFETY: The CPU interface base address (0x0801_0000) is the MMIO region
        // for the GICv2 CPU interface on the QEMU virt machine. The offset is
        // validated by the caller to be a valid GICC register offset. Volatile
        // write is required for MMIO to ensure the write reaches the device.
        unsafe { ptr::write_volatile(addr as *mut u32, value) }
    }

    /// Issue a data synchronization barrier followed by an instruction
    /// synchronization barrier. Required after GIC configuration writes to
    /// ensure the MMIO writes complete and are visible before proceeding.
    fn barrier() {
        // SAFETY: DSB SY ensures all preceding data accesses (including MMIO
        // writes to GIC registers) complete before any subsequent instruction
        // executes. ISB flushes the pipeline so subsequent instructions see
        // the effect of the barrier. Both are non-destructive architectural
        // barrier instructions available at all exception levels.
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
            core::arch::asm!("isb", options(nostack, preserves_flags));
        }
    }

    // -----------------------------------------------------------------------
    // Distributor initialization
    // -----------------------------------------------------------------------

    /// Initialize the GIC Distributor.
    ///
    /// This disables the distributor, configures all SPIs (group 0, target
    /// CPU 0, default priority, level-triggered), then re-enables it.
    fn init_distributor(&mut self) {
        // Disable distributor during configuration
        self.gicd_write(GICD_CTLR, 0);
        Self::barrier();

        // Read the number of supported interrupt lines from GICD_TYPER.
        // ITLinesNumber field is bits [4:0], giving N where the total
        // number of interrupts is 32 * (N + 1), capped at GIC_MAX_IRQS.
        let typer = self.gicd_read(GICD_TYPER);
        let it_lines_number = typer & 0x1F;
        self.num_irqs = ((it_lines_number + 1) * 32).min(GIC_MAX_IRQS);

        // Configure all SPIs (interrupts 32 and above).
        // SGIs (0-15) and PPIs (16-31) are banked per-CPU and configured
        // separately via the CPU interface or left at reset defaults.
        let num_regs = (self.num_irqs / 32) as usize;

        // Set all SPIs to Group 0 (secure/FIQ on GICv2; the distinction
        // does not matter for our non-secure EL1 kernel on QEMU).
        // Register index 0 covers interrupts 0-31 (SGIs/PPIs) -- skip it.
        for i in 1..num_regs {
            self.gicd_write(GICD_IGROUPR + i * 4, 0x0000_0000);
        }

        // Disable all SPIs initially. The kernel will enable specific
        // interrupts as drivers register for them.
        for i in 1..num_regs {
            self.gicd_write(GICD_ICENABLER + i * 4, 0xFFFF_FFFF);
        }

        // Set default priority for all SPIs.
        // IPRIORITYRn packs 4 interrupt priorities per register (1 byte each).
        // Interrupts 0-31 are banked per-CPU; start from interrupt 32.
        let priority_word = u32::from_be_bytes([
            DEFAULT_SPI_PRIORITY,
            DEFAULT_SPI_PRIORITY,
            DEFAULT_SPI_PRIORITY,
            DEFAULT_SPI_PRIORITY,
        ]);
        for i in 8..(self.num_irqs as usize / 4) {
            self.gicd_write(GICD_IPRIORITYR + i * 4, priority_word);
        }

        // Target all SPIs to CPU 0.
        // ITARGETSRn packs 4 interrupt targets per register (1 byte each).
        // Bit 0 of each byte = CPU 0.
        let target_word: u32 = 0x0101_0101;
        for i in 8..(self.num_irqs as usize / 4) {
            self.gicd_write(GICD_ITARGETSR + i * 4, target_word);
        }

        // Configure all SPIs as level-triggered.
        // ICFGRn uses 2 bits per interrupt: bit[1] = 0 for level, 1 for edge.
        // Registers 0-1 cover SGIs/PPIs (banked); start from register 2 (int 32).
        for i in 2..(self.num_irqs as usize / 16) {
            self.gicd_write(GICD_ICFGR + i * 4, 0x0000_0000);
        }

        Self::barrier();

        // Enable the distributor
        self.gicd_write(GICD_CTLR, 1);
        Self::barrier();
    }

    // -----------------------------------------------------------------------
    // CPU Interface initialization
    // -----------------------------------------------------------------------

    /// Initialize the GIC CPU Interface for the current CPU.
    ///
    /// Sets the priority mask to accept all priorities, configures the binary
    /// point register for full preemption granularity, and enables the
    /// interface.
    fn init_cpu_interface(&self) {
        // Set Priority Mask Register to 0xFF: accept all interrupt priorities.
        // Only interrupts with priority numerically lower (= higher urgency)
        // than this value will be signaled to the CPU.
        self.gicc_write(GICC_PMR, 0xFF);

        // Set Binary Point Register to 0: all 8 priority bits are used for
        // the group priority (preemption), no bits reserved for subpriority.
        self.gicc_write(GICC_BPR, 0);

        // Enable the CPU interface. Bit 0 = Enable Group 0 interrupts.
        self.gicc_write(GICC_CTLR, 1);

        Self::barrier();
    }

    // -----------------------------------------------------------------------
    // Interrupt management
    // -----------------------------------------------------------------------

    /// Enable a specific interrupt by ID.
    ///
    /// Writes to the GICD_ISENABLER register corresponding to the given
    /// interrupt. SGIs (0-15) are always enabled; this is typically used
    /// for PPIs and SPIs.
    fn enable_interrupt(&self, id: u32) {
        if id >= self.num_irqs {
            return;
        }
        let reg_index = (id / 32) as usize;
        let bit = 1u32 << (id % 32);
        self.gicd_write(GICD_ISENABLER + reg_index * 4, bit);
        Self::barrier();
    }

    /// Disable a specific interrupt by ID.
    ///
    /// Writes to the GICD_ICENABLER register. Note that SGIs (0-15) cannot
    /// be disabled.
    fn disable_interrupt(&self, id: u32) {
        if id >= self.num_irqs {
            return;
        }
        let reg_index = (id / 32) as usize;
        let bit = 1u32 << (id % 32);
        self.gicd_write(GICD_ICENABLER + reg_index * 4, bit);
        Self::barrier();
    }

    /// Set the priority of a specific interrupt.
    ///
    /// Lower numerical values indicate higher priority. The priority byte
    /// is written to the appropriate position within the GICD_IPRIORITYR
    /// register.
    fn set_priority(&self, id: u32, priority: u8) {
        if id >= self.num_irqs {
            return;
        }
        let reg_index = (id / 4) as usize;
        let byte_offset = (id % 4) as usize;
        let shift = byte_offset * 8;

        let mut val = self.gicd_read(GICD_IPRIORITYR + reg_index * 4);
        val &= !(0xFF << shift);
        val |= (priority as u32) << shift;
        self.gicd_write(GICD_IPRIORITYR + reg_index * 4, val);
        Self::barrier();
    }

    /// Set the target CPU mask for a specific interrupt.
    ///
    /// Each bit in `cpu_mask` corresponds to a CPU (bit 0 = CPU 0, etc.).
    /// Only meaningful for SPIs (32+); SGI/PPI targets are banked per-CPU.
    fn set_target(&self, id: u32, cpu_mask: u8) {
        if id >= self.num_irqs {
            return;
        }
        let reg_index = (id / 4) as usize;
        let byte_offset = (id % 4) as usize;
        let shift = byte_offset * 8;

        let mut val = self.gicd_read(GICD_ITARGETSR + reg_index * 4);
        val &= !(0xFF << shift);
        val |= (cpu_mask as u32) << shift;
        self.gicd_write(GICD_ITARGETSR + reg_index * 4, val);
        Self::barrier();
    }

    /// Acknowledge a pending interrupt.
    ///
    /// Reads the GICC_IAR register, which returns the interrupt ID of the
    /// highest-priority pending interrupt and marks it as active. Returns
    /// `None` if the read yields a spurious interrupt (ID 1023).
    fn acknowledge(&self) -> Option<u32> {
        let iar = self.gicc_read(GICC_IAR);
        let irq_id = iar & 0x3FF; // Bits [9:0] = interrupt ID

        if irq_id == GIC_SPURIOUS_IRQ {
            None
        } else {
            Some(irq_id)
        }
    }

    /// Signal end of interrupt processing.
    ///
    /// Writes the interrupt ID to GICC_EOIR, transitioning the interrupt
    /// from active to inactive state. Must be called after the interrupt
    /// handler has completed processing.
    fn end_of_interrupt(&self, id: u32) {
        self.gicc_write(GICC_EOIR, id);
        Self::barrier();
    }
}

// ---------------------------------------------------------------------------
// Top-level public API
// ---------------------------------------------------------------------------

/// Initialize the GICv2 controller.
///
/// Configures both the distributor and the CPU interface for the QEMU virt
/// machine. This must be called once during early kernel initialization
/// (from `arch::aarch64::init()`).
///
/// Returns an error if the GIC has already been initialized.
pub fn init() -> KernelResult<()> {
    let mut gic = Gic::new(GICD_BASE, GICC_BASE);
    gic.init_distributor();
    gic.init_cpu_interface();

    // Print initialization info via direct UART (println! is a no-op on AArch64)
    // SAFETY: uart_write_str performs raw MMIO writes to the PL011 UART at
    // 0x0900_0000. The UART is memory-mapped by QEMU's virt machine and the
    // write is non-destructive. Called during single-threaded kernel init.
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[GIC] GICv2 initialized: ");
        crate::arch::aarch64::direct_uart::direct_print_num(gic.num_irqs as u64);
        uart_write_str(" interrupt lines\n");
    }

    GIC.init(Mutex::new(gic))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "GIC",
            id: 0,
        })
}

/// Enable a specific IRQ line.
///
/// Enables the interrupt with the given ID in the GIC distributor.
/// Valid for SGIs (0-15), PPIs (16-31), and SPIs (32+).
pub fn enable_irq(irq: u32) -> KernelResult<()> {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.enable_interrupt(irq);
    })
    .ok_or(KernelError::NotInitialized { subsystem: "GIC" })
}

/// Disable a specific IRQ line.
///
/// Disables the interrupt with the given ID in the GIC distributor.
/// SGIs (0-15) cannot be disabled per the GICv2 specification.
pub fn disable_irq(irq: u32) -> KernelResult<()> {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.disable_interrupt(irq);
    })
    .ok_or(KernelError::NotInitialized { subsystem: "GIC" })
}

/// Set the priority of a specific IRQ.
///
/// Lower numerical values indicate higher priority. Typical values:
/// - `0x00`: Highest priority
/// - `0xA0`: Default SPI priority
/// - `0xFF`: Lowest priority (masked by default PMR)
pub fn set_irq_priority(irq: u32, priority: u8) -> KernelResult<()> {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.set_priority(irq, priority);
    })
    .ok_or(KernelError::NotInitialized { subsystem: "GIC" })
}

/// Set the target CPU mask for a specific IRQ.
///
/// Each bit corresponds to a CPU target (bit 0 = CPU 0, bit 1 = CPU 1, etc.).
/// Only meaningful for SPIs (32+); PPI/SGI targets are per-CPU banked.
pub fn set_irq_target(irq: u32, cpu_mask: u8) -> KernelResult<()> {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.set_target(irq, cpu_mask);
    })
    .ok_or(KernelError::NotInitialized { subsystem: "GIC" })
}

/// Acknowledge and return the highest-priority pending interrupt.
///
/// Reads the GICC_IAR to acknowledge the interrupt and transition it to
/// the active state. Returns `Some(irq_id)` if a real interrupt is pending,
/// or `None` if the interrupt is spurious (ID 1023).
///
/// The caller **must** call [`eoi`] with the returned IRQ ID after the
/// interrupt has been handled.
pub fn handle_irq() -> Option<u32> {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.acknowledge()
    })?
}

/// Signal end of interrupt processing for the given IRQ.
///
/// Writes to GICC_EOIR to transition the interrupt from active to inactive.
/// Must be called after the interrupt handler has finished processing.
pub fn eoi(irq: u32) {
    GIC.with(|mtx| {
        let gic = mtx.lock();
        gic.end_of_interrupt(irq);
    });
}

/// Check whether the GIC has been initialized.
pub fn is_initialized() -> bool {
    GIC.with(|_| ()).is_some()
}
