//! Architecture-independent IRQ abstraction layer
//!
//! Provides a generic interface for interrupt management that delegates to
//! per-architecture interrupt controllers:
//! - x86_64: Local APIC + I/O APIC
//! - AArch64: GICv2 (Generic Interrupt Controller)
//! - RISC-V: PLIC (Platform-Level Interrupt Controller)
//!
//! This module implements the IRQ object abstraction (H-003) from the
//! remediation backlog, providing a unified API for registering handlers,
//! enabling/disabling IRQ lines, and dispatching interrupts.

// IRQ management

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

use spin::Mutex;

use crate::{
    error::{KernelError, KernelResult},
    sync::once_lock::GlobalState,
};

// ---------------------------------------------------------------------------
// IRQ number newtype
// ---------------------------------------------------------------------------

/// Architecture-independent IRQ number.
///
/// Wraps a `u32` to provide type safety and prevent accidental misuse of
/// raw integer values as IRQ numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrqNumber(pub u32);

impl IrqNumber {
    /// Create a new IRQ number.
    pub const fn new(irq: u32) -> Self {
        Self(irq)
    }

    /// Get the raw IRQ number as a `u32`.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

impl From<u32> for IrqNumber {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<IrqNumber> for u32 {
    fn from(irq: IrqNumber) -> u32 {
        irq.0
    }
}

impl core::fmt::Display for IrqNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "IRQ#{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// IRQ handler type
// ---------------------------------------------------------------------------

/// Type alias for IRQ handler functions.
///
/// An IRQ handler is a function that takes the IRQ number that triggered the
/// interrupt. Handlers are registered via [`register_handler`] and invoked
/// by [`dispatch`] when the corresponding interrupt fires.
pub type IrqHandler = fn(IrqNumber);

// ---------------------------------------------------------------------------
// IRQ controller trait
// ---------------------------------------------------------------------------

/// Architecture-independent interrupt controller interface.
///
/// Each architecture implements this trait for its hardware interrupt
/// controller (APIC, GIC, PLIC). The [`IrqManager`] delegates hardware
/// operations through this trait.
pub trait IrqController {
    /// Enable an interrupt line so it can be delivered to the CPU.
    fn enable(&self, irq: IrqNumber) -> KernelResult<()>;

    /// Disable an interrupt line to prevent delivery.
    fn disable(&self, irq: IrqNumber) -> KernelResult<()>;

    /// Acknowledge receipt of an interrupt.
    ///
    /// On some architectures this is a separate step from EOI.
    fn acknowledge(&self, irq: IrqNumber) -> KernelResult<()>;

    /// Signal end-of-interrupt to the controller.
    ///
    /// Must be called after the interrupt handler has finished processing.
    fn eoi(&self, irq: IrqNumber) -> KernelResult<()>;

    /// Set the priority of an interrupt line.
    ///
    /// The meaning of `priority` is architecture-dependent:
    /// - x86_64 APIC: Not directly supported per-IRQ (no-op)
    /// - AArch64 GIC: 0x00 = highest, 0xFF = lowest
    /// - RISC-V PLIC: 0 = disabled, 1-7 = priority levels
    fn set_priority(&self, irq: IrqNumber, priority: u8) -> KernelResult<()>;

    /// Check whether an interrupt is pending.
    fn is_pending(&self, irq: IrqNumber) -> KernelResult<bool>;
}

// ---------------------------------------------------------------------------
// IRQ manager
// ---------------------------------------------------------------------------

/// Maximum number of IRQ lines supported.
///
/// This covers the common range across all supported architectures:
/// - x86_64 I/O APIC: typically 24 lines
/// - AArch64 GIC: up to 1020 interrupt IDs
/// - RISC-V PLIC: up to 127 sources
const MAX_IRQ: u32 = 256;

/// Central IRQ manager that maintains registered handlers and delegates
/// hardware operations to the architecture-specific controller.
///
/// The manager stores a mapping from IRQ numbers to handler functions.
/// When an interrupt fires, the architecture-specific entry point calls
/// [`dispatch`] which looks up and invokes the registered handler.
pub struct IrqManager {
    /// Registered IRQ handlers, keyed by raw IRQ number.
    #[cfg(feature = "alloc")]
    handlers: BTreeMap<u32, IrqHandler>,

    /// Number of interrupts dispatched (for statistics).
    dispatch_count: u64,
}

impl IrqManager {
    /// Create a new IRQ manager with no registered handlers.
    fn new() -> Self {
        Self {
            #[cfg(feature = "alloc")]
            handlers: BTreeMap::new(),
            dispatch_count: 0,
        }
    }

    /// Register a handler for the given IRQ number.
    ///
    /// Returns an error if a handler is already registered for this IRQ.
    #[cfg(feature = "alloc")]
    fn register(&mut self, irq: IrqNumber, handler: IrqHandler) -> KernelResult<()> {
        if irq.0 >= MAX_IRQ {
            return Err(KernelError::InvalidArgument {
                name: "irq",
                value: "IRQ number exceeds maximum",
            });
        }

        if self.handlers.contains_key(&irq.0) {
            return Err(KernelError::AlreadyExists {
                resource: "IRQ handler",
                id: irq.0 as u64,
            });
        }

        self.handlers.insert(irq.0, handler);
        Ok(())
    }

    /// Unregister the handler for the given IRQ number.
    ///
    /// Returns an error if no handler is registered for this IRQ.
    #[cfg(feature = "alloc")]
    fn unregister(&mut self, irq: IrqNumber) -> KernelResult<()> {
        if self.handlers.remove(&irq.0).is_none() {
            return Err(KernelError::NotFound {
                resource: "IRQ handler",
                id: irq.0 as u64,
            });
        }
        Ok(())
    }

    /// Dispatch an interrupt to the registered handler.
    ///
    /// If no handler is registered for the given IRQ, this is a no-op
    /// (spurious interrupts are silently ignored).
    #[cfg(feature = "alloc")]
    fn dispatch(&mut self, irq: IrqNumber) {
        self.dispatch_count += 1;
        if let Some(&handler) = self.handlers.get(&irq.0) {
            handler(irq);
        }
    }

    /// Get the number of dispatched interrupts.
    fn dispatch_count(&self) -> u64 {
        self.dispatch_count
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Global IRQ manager instance, protected by a spin::Mutex.
///
/// Initialized once by [`init`]. Uses the GlobalState pattern for safe
/// global access without `static mut`.
static IRQ_MANAGER: GlobalState<Mutex<IrqManager>> = GlobalState::new();

// ---------------------------------------------------------------------------
// Architecture-specific delegation
// ---------------------------------------------------------------------------

/// Enable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "x86_64")]
fn arch_enable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::x86_64::apic::unmask_irq(irq as u8)
}

/// Enable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "aarch64")]
fn arch_enable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::aarch64::gic::enable_irq(irq)
}

/// Enable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "riscv64")]
fn arch_enable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::riscv::plic::enable(irq)
}

/// Disable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "x86_64")]
fn arch_disable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::x86_64::apic::mask_irq(irq as u8)
}

/// Disable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "aarch64")]
fn arch_disable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::aarch64::gic::disable_irq(irq)
}

/// Disable an IRQ line on the architecture-specific interrupt controller.
#[cfg(target_arch = "riscv64")]
fn arch_disable_irq(irq: u32) -> KernelResult<()> {
    crate::arch::riscv::plic::disable(irq)
}

/// Send end-of-interrupt to the architecture-specific controller.
#[cfg(target_arch = "x86_64")]
fn arch_eoi(_irq: u32) -> KernelResult<()> {
    crate::arch::x86_64::apic::send_eoi();
    Ok(())
}

/// Send end-of-interrupt to the architecture-specific controller.
#[cfg(target_arch = "aarch64")]
fn arch_eoi(irq: u32) -> KernelResult<()> {
    crate::arch::aarch64::gic::eoi(irq);
    Ok(())
}

/// Send end-of-interrupt to the architecture-specific controller.
#[cfg(target_arch = "riscv64")]
fn arch_eoi(irq: u32) -> KernelResult<()> {
    crate::arch::riscv::plic::complete(irq)
}

/// Set IRQ priority on the architecture-specific controller.
#[cfg(target_arch = "x86_64")]
fn arch_set_priority(_irq: u32, _priority: u8) -> KernelResult<()> {
    // x86_64 I/O APIC does not support per-IRQ priority in the same way;
    // priority is managed via the Task Priority Register. This is a no-op.
    Ok(())
}

/// Set IRQ priority on the architecture-specific controller.
#[cfg(target_arch = "aarch64")]
fn arch_set_priority(irq: u32, priority: u8) -> KernelResult<()> {
    crate::arch::aarch64::gic::set_irq_priority(irq, priority)
}

/// Set IRQ priority on the architecture-specific controller.
#[cfg(target_arch = "riscv64")]
fn arch_set_priority(irq: u32, priority: u8) -> KernelResult<()> {
    crate::arch::riscv::plic::set_priority(irq, priority as u32)
}

/// Check if an IRQ is pending on the architecture-specific controller.
#[cfg(target_arch = "x86_64")]
fn arch_is_pending(_irq: u32) -> KernelResult<bool> {
    // x86_64: Checking individual IRQ pending status requires reading the
    // IRR (Interrupt Request Register) from the Local APIC, which is not
    // yet exposed. Return false as a safe default.
    Ok(false)
}

/// Check if an IRQ is pending on the architecture-specific controller.
#[cfg(target_arch = "aarch64")]
fn arch_is_pending(_irq: u32) -> KernelResult<bool> {
    // AArch64 GIC: Checking individual pending bits via GICD_ISPENDR is
    // not yet exposed in the public GIC API. Return false as a safe default.
    Ok(false)
}

/// Check if an IRQ is pending on the architecture-specific controller.
#[cfg(target_arch = "riscv64")]
fn arch_is_pending(irq: u32) -> KernelResult<bool> {
    crate::arch::riscv::plic::is_pending(irq)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the IRQ manager subsystem.
///
/// Creates the global IRQ manager instance. Must be called after the
/// architecture-specific interrupt controller has been initialized
/// (APIC, GIC, or PLIC).
///
/// # Errors
///
/// Returns `KernelError::AlreadyExists` if the IRQ manager has already
/// been initialized.
pub fn init() -> KernelResult<()> {
    IRQ_MANAGER
        .init(Mutex::new(IrqManager::new()))
        .map_err(|_| KernelError::AlreadyExists {
            resource: "IRQ manager",
            id: 0,
        })?;

    kprintln!("[IRQ] IRQ manager initialized");
    Ok(())
}

/// Register an IRQ handler for the given interrupt number.
///
/// The handler will be invoked when the interrupt fires and [`dispatch`]
/// is called. Only one handler may be registered per IRQ number.
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the IRQ manager has not been initialized.
/// - `KernelError::AlreadyExists` if a handler is already registered for this
///   IRQ.
/// - `KernelError::InvalidArgument` if the IRQ number exceeds the maximum.
#[cfg(feature = "alloc")]
pub fn register_handler(irq: IrqNumber, handler: IrqHandler) -> KernelResult<()> {
    IRQ_MANAGER
        .with_mut(|mtx| {
            let mut mgr = mtx.lock();
            mgr.register(irq, handler)
        })
        .unwrap_or(Err(KernelError::NotInitialized {
            subsystem: "IRQ manager",
        }))
}

/// Unregister the IRQ handler for the given interrupt number.
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the IRQ manager has not been initialized.
/// - `KernelError::NotFound` if no handler is registered for this IRQ.
#[cfg(feature = "alloc")]
pub fn unregister_handler(irq: IrqNumber) -> KernelResult<()> {
    IRQ_MANAGER
        .with_mut(|mtx| {
            let mut mgr = mtx.lock();
            mgr.unregister(irq)
        })
        .unwrap_or(Err(KernelError::NotInitialized {
            subsystem: "IRQ manager",
        }))
}

/// Dispatch an interrupt to the registered handler.
///
/// Called by the architecture-specific interrupt entry point when an
/// external interrupt is received. Looks up the handler for the given
/// IRQ number and invokes it. If no handler is registered, the interrupt
/// is silently ignored (spurious).
#[cfg(feature = "alloc")]
pub fn dispatch(irq: IrqNumber) {
    IRQ_MANAGER.with_mut(|mtx| {
        let mut mgr = mtx.lock();
        mgr.dispatch(irq);
    });
}

/// Enable an IRQ line on the hardware interrupt controller.
///
/// Delegates to the architecture-specific controller:
/// - x86_64: unmasks the IRQ in the I/O APIC
/// - AArch64: enables the interrupt in the GIC distributor
/// - RISC-V: enables the interrupt source in the PLIC
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the interrupt controller has not been
///   initialized.
pub fn enable_irq(irq: IrqNumber) -> KernelResult<()> {
    arch_enable_irq(irq.0)
}

/// Disable an IRQ line on the hardware interrupt controller.
///
/// Delegates to the architecture-specific controller:
/// - x86_64: masks the IRQ in the I/O APIC
/// - AArch64: disables the interrupt in the GIC distributor
/// - RISC-V: disables the interrupt source in the PLIC
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the interrupt controller has not been
///   initialized.
pub fn disable_irq(irq: IrqNumber) -> KernelResult<()> {
    arch_disable_irq(irq.0)
}

/// Send end-of-interrupt to the hardware controller.
///
/// Must be called after the interrupt handler has finished processing.
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the interrupt controller has not been
///   initialized.
pub fn eoi(irq: IrqNumber) -> KernelResult<()> {
    arch_eoi(irq.0)
}

/// Set the priority of an IRQ line.
///
/// The interpretation of `priority` is architecture-dependent. See
/// [`IrqController::set_priority`] for details.
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the interrupt controller has not been
///   initialized.
/// - `KernelError::InvalidArgument` if the priority value is out of range.
pub fn set_priority(irq: IrqNumber, priority: u8) -> KernelResult<()> {
    arch_set_priority(irq.0, priority)
}

/// Check whether an IRQ is pending.
///
/// # Errors
///
/// - `KernelError::NotInitialized` if the interrupt controller has not been
///   initialized.
pub fn is_pending(irq: IrqNumber) -> KernelResult<bool> {
    arch_is_pending(irq.0)
}

/// Get the number of interrupts dispatched since initialization.
pub fn dispatch_count() -> u64 {
    IRQ_MANAGER
        .with(|mtx| {
            let mgr = mtx.lock();
            mgr.dispatch_count()
        })
        .unwrap_or(0)
}
