// Global Descriptor Table

use x86_64::{
    structures::{
        gdt::{Descriptor, SegmentSelector},
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

/// GDT entries: Linux's x86_64 layout (asm/segment.h), so the user
/// selectors and the 32-bit code segment are where Linux programs (and
/// modify_ldt users) expect them.
///
/// | Selector | Entry |
/// |---|---|
/// | 0x08 | kernel code (64-bit) |
/// | 0x10 | kernel data |
/// | 0x18 | unused |
/// | 0x23 | user code, 32-bit (`__USER32_CS`, compatibility mode) |
/// | 0x2b | user data (`__USER_DS`) |
/// | 0x33 | user code, 64-bit (`__USER_CS`) |
/// | 0x38 | unused |
/// | 0x40 | TSS (two entries) |
/// | 0x50 | LDT (two entries; written at each switch, `ldt::install`) |
///
/// SYSRET's base selector is 0x23: SYSRETQ returns to 0x33 with stack
/// 0x2b, SYSRETL to 0x23 with 0x2b (STAR, `syscall::init_syscall_msrs`).
pub const GDT_ENTRIES: usize = 12;
pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER32_CS: u16 = 0x23;
pub const USER_DS: u16 = 0x2b;
pub const USER_CS: u16 = 0x33;
pub const TSS_SELECTOR: u16 = 0x40;
pub const LDT_SELECTOR: u16 = 0x50;

/// Linux's descriptors (GDT_ENTRY_INIT in cpu/common.c), accessed bit set.
const KERNEL_CS_DESC: u64 = 0x00af_9b00_0000_ffff;
const KERNEL_DS_DESC: u64 = 0x00cf_9300_0000_ffff;
const USER32_CS_DESC: u64 = 0x00cf_fb00_0000_ffff;
const USER_DS_DESC: u64 = 0x00cf_f300_0000_ffff;
const USER_CS_DESC: u64 = 0x00af_fb00_0000_ffff;

/// One CPU's GDT. Atomic entries, because the LDT slot is rewritten while
/// the table is loaded (and the CPU itself sets the TSS busy bit).
#[repr(C, align(16))]
pub struct Gdt([core::sync::atomic::AtomicU64; GDT_ENTRIES]);

impl Gdt {
    const fn new() -> Self {
        Self([const { core::sync::atomic::AtomicU64::new(0) }; GDT_ENTRIES])
    }

    /// Fill in every entry, with the TSS at `tss`.
    fn fill(&self, tss: *const TaskStateSegment) {
        use core::sync::atomic::Ordering::Relaxed;
        let e = &self.0;
        e[1].store(KERNEL_CS_DESC, Relaxed);
        e[2].store(KERNEL_DS_DESC, Relaxed);
        e[4].store(USER32_CS_DESC, Relaxed);
        e[5].store(USER_DS_DESC, Relaxed);
        e[6].store(USER_CS_DESC, Relaxed);
        // SAFETY: `tss` lives for the kernel's lifetime (a static or a
        // leaked box); only its address is used.
        let tss_desc = unsafe { Descriptor::tss_segment_unchecked(tss) };
        if let Descriptor::SystemSegment(lo, hi) = tss_desc {
            e[8].store(lo, Relaxed);
            e[9].store(hi, Relaxed);
        }
    }

    /// Load this table and its segments on the calling CPU.
    ///
    /// # Safety
    /// `self` must live for the kernel's lifetime and be filled in; its
    /// TSS must not be loaded by another CPU.
    unsafe fn load(&'static self) {
        use x86_64::{
            instructions::{
                segmentation::{Segment, CS, DS, ES, SS},
                tables::{lgdt, load_tss},
            },
            structures::DescriptorTablePointer,
            PrivilegeLevel,
        };
        let ptr = DescriptorTablePointer {
            limit: (GDT_ENTRIES * 8 - 1) as u16,
            base: VirtAddr::from_ptr(self.0.as_ptr()),
        };
        let kcs = SegmentSelector::new(KERNEL_CS >> 3, PrivilegeLevel::Ring0);
        let kds = SegmentSelector::new(KERNEL_DS >> 3, PrivilegeLevel::Ring0);
        // SAFETY: the table is valid and permanent (the contract); the
        // selectors name its kernel code and data entries and its TSS.
        unsafe {
            lgdt(&ptr);
            CS::set_reg(kcs);
            DS::set_reg(kds);
            ES::set_reg(kds);
            SS::set_reg(kds);
            load_tss(SegmentSelector::new(
                TSS_SELECTOR >> 3,
                PrivilegeLevel::Ring0,
            ));
            core::arch::asm!("lldt {0:x}", in(reg) 0u16, options(nostack, preserves_flags));
            // fs and gs null too: firmware leaves its own selectors there,
            // which this layout makes null or system descriptors, and a
            // task switched out records them (PR #50 review). gs between two
            // `swapgs`, so the per-CPU base already in GS_BASE stays.
            core::arch::asm!(
                "mov fs, {0:x}", "swapgs", "mov gs, {0:x}", "swapgs",
                in(reg) 0u16, options(nostack, preserves_flags)
            );
        }
    }
}

/// The calling CPU's GDT (its GDTR base).
pub fn this_cpu_gdt() -> u64 {
    x86_64::instructions::tables::sgdt().base.as_u64()
}

/// Write the LDT descriptor (two entries at [`LDT_SELECTOR`]) of the GDT at
/// `gdt` (a `Gdt`'s address, [`this_cpu_gdt`]).
///
/// # Safety
/// `gdt` must be the calling CPU's table, with interrupts off: `lldt`
/// reads the slot right after.
pub unsafe fn set_ldt_descriptor(gdt: u64, lo: u64, hi: u64) {
    let e = gdt as *const core::sync::atomic::AtomicU64;
    let i = (LDT_SELECTOR >> 3) as usize;
    // SAFETY: `gdt` is a live `Gdt` of GDT_ENTRIES entries (the contract)
    // and `i + 1` is within it.
    unsafe {
        (*e.add(i)).store(lo, core::sync::atomic::Ordering::Relaxed);
        (*e.add(i + 1)).store(hi, core::sync::atomic::Ordering::Relaxed);
    }
}

/// The boot CPU's table.
static BOOT_GDT: Gdt = Gdt::new();

/// GDT segment selectors for kernel and user mode (see [`GDT_ENTRIES`]).
pub struct Selectors {
    pub code_selector: SegmentSelector,
    pub data_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
    pub user_data_selector: SegmentSelector,
    pub user_code_selector: SegmentSelector,
}

static SELECTORS: Selectors = Selectors {
    code_selector: SegmentSelector(KERNEL_CS),
    data_selector: SegmentSelector(KERNEL_DS),
    tss_selector: SegmentSelector(TSS_SELECTOR),
    user_data_selector: SegmentSelector(USER_DS),
    user_code_selector: SegmentSelector(USER_CS),
};

pub fn init() {
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

    BOOT_GDT.fill(TSS.0.get());
    // SAFETY: a static table, just filled in, with the boot CPU's TSS.
    unsafe { BOOT_GDT.load() };
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

    // Guarded (an unmapped page below each, N-180) and never freed: they
    // are this CPU's for the kernel's lifetime.
    fn stack() -> VirtAddr {
        let pages = TSS_STACK_SIZE / 4096;
        let s = crate::mm::kstack::allocate(pages).expect("AP TSS stack");
        VirtAddr::new((s.base + pages * 4096) as u64 & !0xF)
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

    let gdt: &'static Gdt = Box::leak(Box::new(Gdt::new()));
    gdt.fill(tss);
    // SAFETY: leaked, so it lives for the kernel's lifetime, as does the
    // TSS it describes, which no other CPU loads.
    unsafe { gdt.load() };
}

/// Returns the GDT selectors (kernel and user mode).
pub fn selectors() -> &'static Selectors {
    &SELECTORS
}

/// Set the stack this CPU enters on from ring 3 (syscall and TSS.RSP0).
pub fn set_kernel_stack(stack_top: u64) {
    crate::arch::percpu::set_entry_stack(stack_top);
}

/// The stack this CPU enters on from ring 3.
pub fn get_kernel_stack() -> u64 {
    crate::arch::percpu::entry_stack()
}
