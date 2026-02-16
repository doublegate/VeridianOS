//! RISC-V Platform-Level Interrupt Controller (PLIC) driver
//!
//! Implements the SiFive PLIC specification for the QEMU virt machine.
//! The PLIC aggregates external interrupt sources and delivers them to
//! hart contexts based on priority and enable configuration.
//!
//! # QEMU virt machine PLIC memory map (base: 0x0C00_0000)
//!
//! | Region            | Offset      | Size per entry | Description                      |
//! |--------------------|-------------|----------------|----------------------------------|
//! | Priority           | 0x00_0000   | 4 bytes        | Per-source priority (up to 1024) |
//! | Pending            | 0x00_1000   | 1 bit/source   | Packed into 32-bit words         |
//! | Enable             | 0x00_2000   | 0x80/context   | 1 bit per source per context     |
//! | Priority threshold | 0x20_0000   | 0x1000/context | Minimum priority to deliver      |
//! | Claim/complete     | 0x20_0004   | 0x1000/context | Read to claim, write to complete |
//!
//! # Context mapping
//!
//! Each hart has two contexts: M-mode (even) and S-mode (odd).
//! For hart N: M-mode context = N*2, S-mode context = N*2 + 1.
//! On hart 0: S-mode context = 1.

use core::sync::atomic::{fence, Ordering};

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    sync::once_lock::GlobalState,
};

// ---------------------------------------------------------------------------
// PLIC base address and register offsets (SiFive PLIC specification)
// ---------------------------------------------------------------------------

/// PLIC base address on the QEMU virt machine.
const PLIC_BASE: usize = 0x0C00_0000;

/// Offset of the priority register array from PLIC base.
/// Each source has a 4-byte priority register. Source 0 is reserved.
const PLIC_PRIORITY_OFFSET: usize = 0x00_0000;

/// Offset of the pending bit array from PLIC base.
/// One bit per source, packed into 32-bit words.
const PLIC_PENDING_OFFSET: usize = 0x00_1000;

/// Offset of the enable bit arrays from PLIC base.
/// Each context has 0x80 bytes (1024 bits) of enable bits.
const PLIC_ENABLE_OFFSET: usize = 0x00_2000;

/// Stride between enable arrays for consecutive contexts.
const PLIC_ENABLE_STRIDE: usize = 0x80;

/// Offset of the threshold/claim region from PLIC base.
/// Each context has a 4-byte threshold at offset 0 and a 4-byte
/// claim/complete register at offset 4, with 0x1000 stride.
const PLIC_THRESHOLD_OFFSET: usize = 0x20_0000;

/// Offset of the claim/complete register relative to the threshold region.
const PLIC_CLAIM_OFFSET: usize = 0x20_0004;

/// Stride between threshold/claim regions for consecutive contexts.
const PLIC_CONTEXT_STRIDE: usize = 0x1000;

// ---------------------------------------------------------------------------
// QEMU virt machine IRQ assignments
// ---------------------------------------------------------------------------

/// UART0 interrupt source (QEMU virt machine specific).
pub const IRQ_UART0: u32 = 10;

/// VirtIO device interrupt sources (QEMU virt machine specific).
/// VirtIO devices use IRQs 1 through 8 on the QEMU virt platform.
pub const IRQ_VIRTIO_START: u32 = 1;

/// Last VirtIO interrupt source (inclusive).
pub const IRQ_VIRTIO_END: u32 = 8;

// ---------------------------------------------------------------------------
// PLIC configuration limits
// ---------------------------------------------------------------------------

/// Maximum number of interrupt sources supported.
/// The QEMU virt machine typically uses sources 1-127.
/// Source 0 is reserved (no interrupt) per the PLIC specification.
const MAX_SOURCES: u32 = 128;

/// Maximum valid priority value. The SiFive PLIC supports 7 priority
/// levels (1-7), with 0 meaning "never interrupt" (disabled).
const MAX_PRIORITY: u32 = 7;

// ---------------------------------------------------------------------------
// Global PLIC instance
// ---------------------------------------------------------------------------

/// Global PLIC state, initialized once during `init()`.
static PLIC: GlobalState<Mutex<Plic>> = GlobalState::new();

// ---------------------------------------------------------------------------
// PLIC driver
// ---------------------------------------------------------------------------

/// Platform-Level Interrupt Controller driver.
///
/// Manages interrupt source priorities, per-context enable masks,
/// priority thresholds, and the claim/complete handshake.
struct Plic {
    /// MMIO base address of the PLIC.
    base: usize,
    /// Number of usable interrupt sources (1..=max_irq).
    max_irq: u32,
    /// S-mode context ID for the boot hart (hart 0).
    s_context: u32,
}

impl Plic {
    /// Create a new PLIC instance.
    ///
    /// `base` is the MMIO base address. `max_irq` is the highest valid
    /// source number (inclusive). `hart_id` is the boot hart ID.
    fn new(base: usize, max_irq: u32, hart_id: u32) -> Self {
        Self {
            base,
            max_irq,
            // S-mode context for a given hart: hart_id * 2 + 1
            s_context: hart_id * 2 + 1,
        }
    }

    // -- Register address helpers ------------------------------------------

    /// Address of the priority register for interrupt source `irq`.
    #[inline]
    fn priority_addr(&self, irq: u32) -> *mut u32 {
        (self.base + PLIC_PRIORITY_OFFSET + (irq as usize) * 4) as *mut u32
    }

    /// Address of the pending word that contains bit for source `irq`.
    #[inline]
    fn pending_addr(&self, irq: u32) -> *const u32 {
        (self.base + PLIC_PENDING_OFFSET + ((irq as usize) / 32) * 4) as *const u32
    }

    /// Address of the enable word that contains bit for source `irq`
    /// in the given context.
    #[inline]
    fn enable_addr(&self, irq: u32, context: u32) -> *mut u32 {
        (self.base
            + PLIC_ENABLE_OFFSET
            + (context as usize) * PLIC_ENABLE_STRIDE
            + ((irq as usize) / 32) * 4) as *mut u32
    }

    /// Address of the priority threshold register for the given context.
    #[inline]
    fn threshold_addr(&self, context: u32) -> *mut u32 {
        (self.base + PLIC_THRESHOLD_OFFSET + (context as usize) * PLIC_CONTEXT_STRIDE) as *mut u32
    }

    /// Address of the claim/complete register for the given context.
    #[inline]
    fn claim_complete_addr(&self, context: u32) -> *mut u32 {
        (self.base + PLIC_CLAIM_OFFSET + (context as usize) * PLIC_CONTEXT_STRIDE) as *mut u32
    }

    // -- Validation --------------------------------------------------------

    /// Validate that an IRQ number is within the supported range (1..=max_irq).
    /// Source 0 is reserved by the PLIC specification.
    fn validate_irq(&self, irq: u32) -> KernelResult<()> {
        if irq == 0 || irq > self.max_irq {
            return Err(KernelError::InvalidArgument {
                name: "irq",
                value: "out of range",
            });
        }
        Ok(())
    }

    // -- Core operations ---------------------------------------------------

    /// Set the priority of interrupt source `irq`.
    ///
    /// Priority 0 effectively disables the source. Valid range: 0..=7.
    fn set_priority(&self, irq: u32, priority: u32) -> KernelResult<()> {
        self.validate_irq(irq)?;
        if priority > MAX_PRIORITY {
            return Err(KernelError::InvalidArgument {
                name: "priority",
                value: "exceeds maximum (7)",
            });
        }
        // SAFETY: `priority_addr` returns a pointer into the PLIC MMIO region,
        // which is memory-mapped hardware. The address is valid because `irq`
        // has been validated to be within [1, max_irq]. write_volatile is
        // required for MMIO to prevent compiler reordering or elision.
        unsafe {
            core::ptr::write_volatile(self.priority_addr(irq), priority);
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Enable interrupt source `irq` in the boot hart's S-mode context.
    fn enable_irq(&self, irq: u32) -> KernelResult<()> {
        self.validate_irq(irq)?;
        let addr = self.enable_addr(irq, self.s_context);
        let bit = 1u32 << (irq % 32);
        // SAFETY: `enable_addr` returns a pointer into the PLIC MMIO enable
        // region. The address is valid because `irq` and `s_context` are
        // within bounds. We perform a read-modify-write to set only the
        // target bit, preserving other enable bits.
        unsafe {
            let current = core::ptr::read_volatile(addr);
            core::ptr::write_volatile(addr, current | bit);
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Disable interrupt source `irq` in the boot hart's S-mode context.
    fn disable_irq(&self, irq: u32) -> KernelResult<()> {
        self.validate_irq(irq)?;
        let addr = self.enable_addr(irq, self.s_context);
        let bit = 1u32 << (irq % 32);
        // SAFETY: `enable_addr` returns a pointer into the PLIC MMIO enable
        // region. The address is valid because `irq` and `s_context` are
        // within bounds. We perform a read-modify-write to clear only the
        // target bit.
        unsafe {
            let current = core::ptr::read_volatile(addr);
            core::ptr::write_volatile(addr, current & !bit);
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Set the priority threshold for the boot hart's S-mode context.
    ///
    /// The PLIC will only deliver interrupts with priority strictly
    /// greater than the threshold. A threshold of 0 allows all enabled
    /// interrupts (priority >= 1) through.
    fn set_threshold(&self, threshold: u32) -> KernelResult<()> {
        if threshold > MAX_PRIORITY {
            return Err(KernelError::InvalidArgument {
                name: "threshold",
                value: "exceeds maximum (7)",
            });
        }
        // SAFETY: `threshold_addr` returns a pointer into the PLIC MMIO
        // threshold register for the S-mode context. Valid because
        // `s_context` is computed from the boot hart ID.
        unsafe {
            core::ptr::write_volatile(self.threshold_addr(self.s_context), threshold);
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Claim the highest-priority pending interrupt for the boot hart's
    /// S-mode context.
    ///
    /// Returns `Some(irq)` if an interrupt is pending, or `None` if the
    /// claim register reads 0 (no pending interrupt).
    fn claim(&self) -> Option<u32> {
        // SAFETY: `claim_complete_addr` returns a pointer to the PLIC MMIO
        // claim/complete register for the S-mode context. Reading this
        // register atomically claims the highest-priority pending interrupt
        // and clears its pending bit. The address is valid because
        // `s_context` is derived from the boot hart ID during init.
        let irq = unsafe { core::ptr::read_volatile(self.claim_complete_addr(self.s_context)) };
        if irq == 0 {
            None
        } else {
            Some(irq)
        }
    }

    /// Signal end-of-interrupt for source `irq`.
    ///
    /// Must be called after handling an interrupt that was obtained via
    /// `claim()`. Writing the source ID to the claim/complete register
    /// informs the PLIC that the interrupt has been serviced.
    fn complete(&self, irq: u32) -> KernelResult<()> {
        self.validate_irq(irq)?;
        // SAFETY: `claim_complete_addr` returns a pointer to the PLIC MMIO
        // claim/complete register. Writing the IRQ number signals EOI to the
        // PLIC. The address is valid because `s_context` is computed from
        // the boot hart ID. The IRQ has been validated.
        unsafe {
            core::ptr::write_volatile(self.claim_complete_addr(self.s_context), irq);
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// Check whether interrupt source `irq` is pending.
    fn is_pending(&self, irq: u32) -> KernelResult<bool> {
        self.validate_irq(irq)?;
        let bit = 1u32 << (irq % 32);
        // SAFETY: `pending_addr` returns a pointer into the PLIC MMIO
        // pending bit array. The address is valid because `irq` is within
        // [1, max_irq]. read_volatile is required for MMIO.
        let word = unsafe { core::ptr::read_volatile(self.pending_addr(irq)) };
        Ok((word & bit) != 0)
    }

    /// Perform full hardware reset of the PLIC:
    /// - Set all source priorities to 0 (disabled)
    /// - Clear all enable bits for the S-mode context
    /// - Set priority threshold to 0 (accept all priorities > 0)
    /// - Drain any pending claims
    fn reset(&self) {
        // Disable all sources by setting priority to 0
        for irq in 1..=self.max_irq {
            // SAFETY: The priority register for each source in [1, max_irq]
            // is within the PLIC MMIO region. Writing 0 disables the source.
            unsafe {
                core::ptr::write_volatile(self.priority_addr(irq), 0);
            }
        }

        // Clear all enable bits for the S-mode context.
        // Each enable word covers 32 sources; we need ceil(max_irq+1 / 32) words.
        let enable_words = ((self.max_irq as usize) + 32) / 32;
        for word_idx in 0..enable_words {
            let addr = (self.base
                + PLIC_ENABLE_OFFSET
                + (self.s_context as usize) * PLIC_ENABLE_STRIDE
                + word_idx * 4) as *mut u32;
            // SAFETY: Address is within the PLIC enable region for the
            // S-mode context. Writing 0 disables all sources in this word.
            unsafe {
                core::ptr::write_volatile(addr, 0);
            }
        }

        // Set threshold to 0 so that any enabled source with priority >= 1
        // can deliver an interrupt.
        // SAFETY: The threshold register for the S-mode context is at a
        // fixed offset within the PLIC MMIO region. Writing 0 sets the
        // lowest possible threshold.
        unsafe {
            core::ptr::write_volatile(self.threshold_addr(self.s_context), 0);
        }

        // Drain any pending claims. The PLIC specification says reading
        // the claim register returns 0 when nothing is pending.
        loop {
            // SAFETY: Reading the claim/complete register either returns a
            // pending IRQ number or 0. This is a standard PLIC drain sequence
            // to clear stale claims from before our init.
            let claimed =
                unsafe { core::ptr::read_volatile(self.claim_complete_addr(self.s_context)) };
            if claimed == 0 {
                break;
            }
            // Complete the stale claim
            // SAFETY: Writing the claimed IRQ back to the claim/complete
            // register signals EOI, allowing the PLIC to deliver future
            // interrupts for this source.
            unsafe {
                core::ptr::write_volatile(self.claim_complete_addr(self.s_context), claimed);
            }
        }

        fence(Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the PLIC for the boot hart (hart 0) in S-mode.
///
/// This sets all interrupt source priorities to 0 (disabled), clears all
/// enable bits, sets the priority threshold to 0, and drains pending claims.
///
/// # Errors
///
/// Returns `KernelError::AlreadyExists` if the PLIC has already been
/// initialized.
pub fn init() -> KernelResult<()> {
    let hart_id: u32 = 0; // Boot hart

    let plic = Plic::new(PLIC_BASE, MAX_SOURCES - 1, hart_id);
    plic.reset();

    crate::println!(
        "[PLIC] Initialized: base=0x{:08X}, sources=1-{}, S-mode context={}",
        PLIC_BASE,
        plic.max_irq,
        plic.s_context
    );

    PLIC.init(Mutex::new(plic))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "PLIC",
            id: 0,
        })?;

    Ok(())
}

/// Set the priority of an interrupt source.
///
/// Priority 0 disables the source. Valid range: 0..=7.
/// Higher values mean higher priority.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `irq` or `priority` is out of
/// range.
pub fn set_priority(irq: u32, priority: u32) -> KernelResult<()> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.set_priority(irq, priority)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}

/// Enable an interrupt source in the boot hart's S-mode context.
///
/// The source must also have a non-zero priority to actually deliver
/// interrupts.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `irq` is out of range.
pub fn enable(irq: u32) -> KernelResult<()> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.enable_irq(irq)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}

/// Disable an interrupt source in the boot hart's S-mode context.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `irq` is out of range.
pub fn disable(irq: u32) -> KernelResult<()> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.disable_irq(irq)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}

/// Set the priority threshold for the boot hart's S-mode context.
///
/// Only interrupts with priority strictly greater than `threshold` will
/// be delivered. A threshold of 0 accepts all priorities >= 1.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `threshold` exceeds 7.
pub fn set_threshold(threshold: u32) -> KernelResult<()> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.set_threshold(threshold)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}

/// Claim the highest-priority pending interrupt.
///
/// Returns `Some(irq)` if an interrupt is pending, `None` otherwise.
/// After handling the interrupt, the caller must call `complete(irq)`.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
pub fn claim() -> KernelResult<Option<u32>> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.claim()
    })
    .ok_or(KernelError::NotInitialized { subsystem: "PLIC" })
}

/// Signal end-of-interrupt for the given source.
///
/// Must be called after handling an interrupt obtained via `claim()`.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `irq` is out of range.
pub fn complete(irq: u32) -> KernelResult<()> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.complete(irq)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}

/// Check whether an interrupt source is pending.
///
/// # Errors
///
/// Returns `KernelError::NotInitialized` if the PLIC has not been initialized.
/// Returns `KernelError::InvalidArgument` if `irq` is out of range.
pub fn is_pending(irq: u32) -> KernelResult<bool> {
    PLIC.with(|mtx| {
        let plic = mtx.lock();
        plic.is_pending(irq)
    })
    .unwrap_or(Err(KernelError::NotInitialized { subsystem: "PLIC" }))
}
