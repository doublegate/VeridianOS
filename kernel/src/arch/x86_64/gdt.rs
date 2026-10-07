// Global Descriptor Table

use lazy_static::lazy_static;
use x86_64::{
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

/// Interrupt stack table slots. Only the three exceptions that can arrive
/// with an unusable stack use one (N-169): a double fault (the kernel stack
/// may have overflowed into its guard page), an NMI and a machine check
/// (either can interrupt any instruction, including the few between an
/// entry and its stack switch). Every other vector runs on the current
/// kernel stack, or on TSS.RSP0 when it comes from ring 3, so that it can
/// nest and the code it calls can block.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const NMI_IST_INDEX: u16 = 1;
pub const MACHINE_CHECK_IST_INDEX: u16 = 2;

/// Size of each TSS stack (the boot RSP0 and the IST entries).
const TSS_STACK_SIZE: usize = 4096 * 5;

/// A stack the CPU switches to through the TSS. Only the CPU writes to it,
/// and never through a Rust reference, so it is an `UnsafeCell` behind a
/// plain `static` rather than a `static mut` (project rule: no new
/// `static mut`; review of the v0.26.0 stack, PR #14).
#[repr(C, align(16))]
struct IstStack(core::cell::UnsafeCell<[u8; TSS_STACK_SIZE]>);

// SAFETY: Rust code never reads or writes the stack memory; only its
// address is taken, once, while the TSS is built. The CPU uses it as a
// stack for exactly one CPU (the TSS is per CPU).
unsafe impl Sync for IstStack {}

impl IstStack {
    const fn new() -> Self {
        Self(core::cell::UnsafeCell::new([0; TSS_STACK_SIZE]))
    }

    /// The initial stack pointer: one past the highest byte (stacks grow
    /// down), 16-byte aligned for the x86_64 ABI.
    fn top(&'static self) -> VirtAddr {
        VirtAddr::from_ptr(self.0.get()) + TSS_STACK_SIZE as u64
    }
}

/// The boot CPU's TSS. RSP0 changes at run time (`percpu::set_entry_stack`)
/// and the CPU reads it, so it lives in an `UnsafeCell` and Rust never holds
/// a reference to it after `init`.
#[repr(transparent)]
struct TssCell(core::cell::UnsafeCell<TaskStateSegment>);

// SAFETY: written only by the boot CPU: once in `init`, then the RSP0 field
// through `percpu::set_entry_stack` on that CPU.
unsafe impl Sync for TssCell {}

static TSS: TssCell = TssCell(core::cell::UnsafeCell::new(TaskStateSegment::new()));

/// Fill in a TSS: the initial RSP0 and the three IST stacks.
fn fill_tss(tss: &mut TaskStateSegment, mut stack: impl FnMut() -> VirtAddr) {
    tss.privilege_stack_table[0] = stack();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack();
    tss.interrupt_stack_table[NMI_IST_INDEX as usize] = stack();
    tss.interrupt_stack_table[MACHINE_CHECK_IST_INDEX as usize] = stack();
}

/// Address of a TSS's RSP0 field.
fn rsp0_slot(tss: *mut TaskStateSegment) -> u64 {
    // SAFETY: only the field address is computed; nothing is read.
    unsafe { core::ptr::addr_of_mut!((*tss).privilege_stack_table[0]) as u64 }
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.append(Descriptor::kernel_code_segment());     // 0x08
        let data_selector = gdt.append(Descriptor::kernel_data_segment());     // 0x10
        // SAFETY: the TSS is a static that lives for the kernel's lifetime;
        // `init` fills it in before the selector is loaded.
        let tss_selector = gdt.append(unsafe { Descriptor::tss_segment_unchecked(TSS.0.get()) }); // 0x18 (2 entries)
        let user_data_selector = gdt.append(Descriptor::user_data_segment());  // 0x28 (+ RPL 3 = 0x2B)
        let user_code_selector = gdt.append(Descriptor::user_code_segment());  // 0x30 (+ RPL 3 = 0x33)
        (
            gdt,
            Selectors {
                code_selector,
                data_selector,
                tss_selector,
                user_data_selector,
                user_code_selector,
            },
        )
    };
}

/// GDT segment selectors for kernel and user mode.
///
/// Layout:
/// - 0x00: Null descriptor
/// - 0x08: Kernel code segment (Ring 0)
/// - 0x10: Kernel data segment (Ring 0)
/// - 0x18: TSS (occupies 2 entries, 0x18-0x20)
/// - 0x28: User data segment (Ring 3, selector 0x2B with RPL)
/// - 0x30: User code segment (Ring 3, selector 0x33 with RPL)
///
/// The user data/code order matches SYSRET expectations:
/// SYSRET computes SS = STAR[63:48]+8, CS = STAR[63:48]+16.
pub struct Selectors {
    pub code_selector: SegmentSelector,
    pub data_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
    pub user_data_selector: SegmentSelector,
    pub user_code_selector: SegmentSelector,
}

pub fn init() {
    use x86_64::instructions::{
        segmentation::{Segment, CS, DS},
        tables::load_tss,
    };

    /// The boot CPU's RSP0 (until the first `set_entry_stack`) and IST stacks.
    static BOOT_STACKS: [IstStack; 4] = [const { IstStack::new() }; 4];
    let mut stacks = BOOT_STACKS.iter();
    // SAFETY: `init` runs once, on the boot CPU, before the TSS is loaded,
    // so nothing else accesses it.
    fill_tss(unsafe { &mut *TSS.0.get() }, || {
        stacks.next().expect("four boot TSS stacks").top()
    });
    // SAFETY: the RSP0 field of the TSS this CPU loads below; a static.
    unsafe { crate::arch::percpu::set_rsp0_slot(0, rsp0_slot(TSS.0.get())) };

    GDT.0.load();
    // SAFETY: After loading the GDT, segment registers must be updated to reference
    // the new descriptors. CS must be reloaded via a far return/jump. DS and TSS
    // are loaded directly. The selectors come from GDT.1 which was computed
    // from the same GDT we just loaded, so they reference valid descriptors.
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        DS::set_reg(GDT.1.data_selector);
        load_tss(GDT.1.tss_selector);
    }
}

/// Give the calling secondary CPU its own GDT and TSS (with its own RSP0
/// and IST stacks) and load them. A TSS cannot be shared: `ltr` marks its
/// descriptor busy, and a second CPU's `ltr` of a busy TSS raises #GP. The
/// selector layout is the boot CPU's, so the STAR MSR values and the IDT
/// selectors are the same on every CPU. The tables and stacks come from
/// the kernel heap, which every process page table maps.
#[cfg(feature = "alloc")]
pub fn init_ap() {
    use alloc::boxed::Box;

    use x86_64::instructions::{
        segmentation::{Segment, CS, DS, SS},
        tables::load_tss,
    };

    fn stack() -> VirtAddr {
        let mem: &'static mut [u8] = alloc::vec![0u8; TSS_STACK_SIZE].leak();
        VirtAddr::new((mem.as_ptr() as u64 + TSS_STACK_SIZE as u64) & !0xF)
    }

    // Leaked as a raw pointer: RSP0 is rewritten at run time, so no Rust
    // reference to the TSS is kept.
    let tss: *mut TaskStateSegment = Box::into_raw(Box::new(TaskStateSegment::new()));
    // SAFETY: just allocated; nothing else knows of it yet.
    fill_tss(unsafe { &mut *tss }, stack);
    // SAFETY: the RSP0 field of the TSS this CPU loads below, leaked for the
    // kernel's lifetime. `install` has already run on this CPU.
    unsafe {
        let cpu = (*crate::arch::percpu::this_arch_cpu()).cpu_id as usize;
        crate::arch::percpu::set_rsp0_slot(cpu, rsp0_slot(tss));
    }

    let gdt: &'static mut GlobalDescriptorTable = Box::leak(Box::new(GlobalDescriptorTable::new()));
    let code = gdt.append(Descriptor::kernel_code_segment());
    let data = gdt.append(Descriptor::kernel_data_segment());
    // SAFETY: the TSS is leaked, so it lives for the kernel's lifetime.
    let tss_sel = gdt.append(unsafe { Descriptor::tss_segment_unchecked(tss) });
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::user_code_segment());
    let gdt: &'static GlobalDescriptorTable = gdt;
    gdt.load();
    // SAFETY: the selectors index the GDT just loaded, which lives for the
    // kernel's lifetime (leaked), as does the TSS it describes.
    unsafe {
        CS::set_reg(code);
        DS::set_reg(data);
        SS::set_reg(data);
        load_tss(tss_sel);
    }
}

/// Returns a reference to the GDT selectors (kernel and user mode).
///
/// Must only be called after `init()` has been called. The lazy_static
/// ensures the GDT is initialized on first access.
pub fn selectors() -> &'static Selectors {
    &GDT.1
}

/// Set the stack this CPU enters on from ring 3 (syscall and TSS.RSP0).
pub fn set_kernel_stack(stack_top: u64) {
    crate::arch::percpu::set_entry_stack(stack_top);
}

/// The stack this CPU enters on from ring 3.
pub fn get_kernel_stack() -> u64 {
    crate::arch::percpu::entry_stack()
}
