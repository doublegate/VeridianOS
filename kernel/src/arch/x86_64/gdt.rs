// Global Descriptor Table

use lazy_static::lazy_static;
use x86_64::{
    structures::{
        gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
        tss::TaskStateSegment,
    },
    VirtAddr,
};

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const PAGE_FAULT_IST_INDEX: u16 = 1;
pub const GENERAL_IST_INDEX: u16 = 2;
pub const HARDWARE_IRQ_IST_INDEX: u16 = 3;

/// Size of each TSS stack (RSP0 and the IST entries).
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

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();

        // Set up the kernel stack for privilege level 0
        // This is used when transitioning from user mode to kernel mode.
        // Must be 16-byte aligned for the x86_64 ABI (movaps et al.).
        tss.privilege_stack_table[0] = {
            static KERNEL_STACK: IstStack = IstStack::new();
            KERNEL_STACK.top()
        };

        // Set up the double fault stack (16-byte aligned)
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            static DF_STACK: IstStack = IstStack::new();
            DF_STACK.top()
        };

        // Set up the page fault stack (16-byte aligned).
        // The page fault handler needs its own IST stack so that Ring 3
        // faults can be handled even when TSS.RSP0 is stale or unmapped
        // in the current process's page tables. Without this, a user-mode
        // page fault escalates directly to a Double Fault because the CPU
        // cannot push the exception frame onto the RSP0 stack.
        tss.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] = {
            static PF_STACK: IstStack = IstStack::new();
            PF_STACK.top()
        };

        // Set up the general exception stack (16-byte aligned).
        // Used by GPF and other non-PF/DF exceptions from Ring 3.
        // Without IST, these exceptions use TSS.RSP0 for the privilege
        // switch. If RSP0 is stale or unmapped in the user process's
        // page tables, the exception delivery fails and escalates to DF.
        tss.interrupt_stack_table[GENERAL_IST_INDEX as usize] = {
            static GP_STACK: IstStack = IstStack::new();
            GP_STACK.top()
        };

        // Set up the hardware IRQ stack (16-byte aligned).
        // Hardware interrupts (timer, keyboard, APIC timer, IPIs) that fire
        // from Ring 3 normally switch to TSS.RSP0. If RSP0 is stale or
        // unmapped in the user process's page tables, the interrupt delivery
        // fails and escalates to a Double Fault. Using a dedicated IST stack
        // bypasses RSP0 entirely for these vectors.
        tss.interrupt_stack_table[HARDWARE_IRQ_IST_INDEX as usize] = {
            static IRQ_STACK: IstStack = IstStack::new();
            IRQ_STACK.top()
        };
        tss
    };
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.append(Descriptor::kernel_code_segment());     // 0x08
        let data_selector = gdt.append(Descriptor::kernel_data_segment());     // 0x10
        let tss_selector = gdt.append(Descriptor::tss_segment(&TSS));          // 0x18 (2 entries)
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

    let tss: &'static mut TaskStateSegment = Box::leak(Box::new(TaskStateSegment::new()));
    tss.privilege_stack_table[0] = stack();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack();
    tss.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] = stack();
    tss.interrupt_stack_table[GENERAL_IST_INDEX as usize] = stack();
    tss.interrupt_stack_table[HARDWARE_IRQ_IST_INDEX as usize] = stack();
    let tss: &'static TaskStateSegment = tss;

    let gdt: &'static mut GlobalDescriptorTable = Box::leak(Box::new(GlobalDescriptorTable::new()));
    let code = gdt.append(Descriptor::kernel_code_segment());
    let data = gdt.append(Descriptor::kernel_data_segment());
    let tss_sel = gdt.append(Descriptor::tss_segment(tss));
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

/// Print TSS stack addresses for debugging.
/// Call after serial output is available.
pub fn debug_print_tss_stacks() {
    // SAFETY: Port I/O writes to COM1 for serial diagnostic output.
    unsafe {
        super::idt::raw_serial_str(b"[TSS] RSP0=0x");
        super::idt::raw_serial_hex(TSS.privilege_stack_table[0].as_u64());
        super::idt::raw_serial_str(b" IST0(DF)=0x");
        super::idt::raw_serial_hex(
            TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize].as_u64(),
        );
        super::idt::raw_serial_str(b" IST1(PF)=0x");
        super::idt::raw_serial_hex(
            TSS.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize].as_u64(),
        );
        super::idt::raw_serial_str(b" IST2(GP)=0x");
        super::idt::raw_serial_hex(TSS.interrupt_stack_table[GENERAL_IST_INDEX as usize].as_u64());
        super::idt::raw_serial_str(b" IST3(IRQ)=0x");
        super::idt::raw_serial_hex(
            TSS.interrupt_stack_table[HARDWARE_IRQ_IST_INDEX as usize].as_u64(),
        );
        super::idt::raw_serial_str(b"\n");
    }
}

/// Read a raw IDT entry for diagnostic purposes.
/// Returns the handler address from the IDT entry at the given vector.
pub fn debug_idt_handler_addr(vector: u8) -> u64 {
    // Read IDT base and limit from IDTR
    let mut idtr: [u8; 10] = [0; 10];
    // SAFETY: sidt only stores the 10-byte IDTR image into `idtr`, a local
    // buffer of exactly that size; it changes no processor state.
    unsafe {
        core::arch::asm!("sidt [{}]", in(reg) &mut idtr, options(nostack));
    }
    let idt_limit = u16::from_le_bytes([idtr[0], idtr[1]]);
    let idt_base = u64::from_le_bytes([
        idtr[2], idtr[3], idtr[4], idtr[5], idtr[6], idtr[7], idtr[8], idtr[9],
    ]);

    let entry_size = 16u64; // Each IDT entry is 16 bytes on x86_64
    let entry_offset = vector as u64 * entry_size;
    if entry_offset + entry_size > idt_limit as u64 + 1 {
        return 0;
    }

    let entry_ptr = (idt_base + entry_offset) as *const u8;
    // SAFETY: `idt_base` comes from IDTR, so it is the live, mapped IDT the
    // CPU itself uses, and the bounds check above keeps the 16-byte entry at
    // `entry_offset` inside its `limit`. The bytes are only read.
    unsafe {
        let offset_low = u16::from_le_bytes([*entry_ptr, *entry_ptr.add(1)]);
        let offset_mid = u16::from_le_bytes([*entry_ptr.add(6), *entry_ptr.add(7)]);
        let offset_high = u32::from_le_bytes([
            *entry_ptr.add(8),
            *entry_ptr.add(9),
            *entry_ptr.add(10),
            *entry_ptr.add(11),
        ]);
        let addr = (offset_low as u64) | ((offset_mid as u64) << 16) | ((offset_high as u64) << 32);
        // Also print IST index from byte 4 (bits 0-2)
        let ist_index = *entry_ptr.add(4) & 0x7;
        super::idt::raw_serial_str(b"(IST=");
        super::idt::raw_serial_hex(ist_index as u64);
        super::idt::raw_serial_str(b")");
        addr
    }
}

/// Verify that critical kernel addresses (IST stacks, handler code) are
/// mapped in a given L4 page table. Walks the 4-level hierarchy for each
/// address and reports whether it's present.
///
/// This catches the case where `map_kernel_space()` failed to copy the L4
/// entry containing the kernel BSS (IST stacks, IDT, TSS).
pub fn debug_verify_ist_in_cr3(process_cr3: u64) {
    let phys_offset = crate::mm::PHYS_MEM_OFFSET.load(core::sync::atomic::Ordering::Acquire);
    if phys_offset == 0 {
        return;
    }

    // Also check the current boot stack (which will become TSS.RSP0 and
    // per-CPU kernel_rsp after enter_usermode_returnable). This is the
    // critical stack used for hardware interrupts WITHOUT IST (timer, etc.).
    let current_rsp: u64;
    // SAFETY: copying RSP into a register has no side effects.
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) current_rsp, options(nomem, nostack));
    }

    let addrs: [u64; 6] = [
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize].as_u64(),
        TSS.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize].as_u64(),
        TSS.interrupt_stack_table[GENERAL_IST_INDEX as usize].as_u64(),
        TSS.privilege_stack_table[0].as_u64(),
        // Also check the TSS itself -- the CPU reads IST values from here
        &*TSS as *const _ as u64,
        // Check the actual boot stack (RSP) that will be used as RSP0
        current_rsp,
    ];
    let labels: [&[u8]; 6] = [
        b"IST_DF", b"IST_PF", b"IST_GP", b"RSP0  ", b"TSS   ", b"BtStk ",
    ];

    // SAFETY: Walking page table structures via physical memory window.
    // All pointers are derived from valid page table entries + phys_offset.
    unsafe {
        let l4_virt = (process_cr3 + phys_offset) as *const u64;
        for (addr, label) in addrs.iter().zip(labels.iter()) {
            let l4i = ((*addr >> 39) & 0x1FF) as usize;
            let l3i = ((*addr >> 30) & 0x1FF) as usize;
            let l2i = ((*addr >> 21) & 0x1FF) as usize;
            let l1i = ((*addr >> 12) & 0x1FF) as usize;

            super::idt::raw_serial_str(b"[PT_CHK] ");
            super::idt::raw_serial_str(label);
            super::idt::raw_serial_str(b"=0x");
            super::idt::raw_serial_hex(*addr);

            let l4e = core::ptr::read_volatile(l4_virt.add(l4i));
            if l4e & 1 == 0 {
                super::idt::raw_serial_str(b" L4[");
                super::idt::raw_serial_hex(l4i as u64);
                super::idt::raw_serial_str(b"]=ABSENT!\n");
                continue;
            }

            let l3_phys = l4e & 0x000F_FFFF_FFFF_F000;
            let l3_virt = (l3_phys + phys_offset) as *const u64;
            let l3e = core::ptr::read_volatile(l3_virt.add(l3i));
            if l3e & 1 == 0 {
                super::idt::raw_serial_str(b" L3=ABSENT!\n");
                continue;
            }
            // Check for 1GB huge page
            if l3e & (1 << 7) != 0 {
                super::idt::raw_serial_str(b" 1GB_HUGE(");
                if l3e & 2 != 0 {
                    super::idt::raw_serial_str(b"W");
                } else {
                    super::idt::raw_serial_str(b"-");
                }
                if l3e & 4 != 0 {
                    super::idt::raw_serial_str(b"U");
                } else {
                    super::idt::raw_serial_str(b"-");
                }
                if l3e & (1u64 << 63) != 0 {
                    super::idt::raw_serial_str(b"NX");
                } else {
                    super::idt::raw_serial_str(b"X");
                }
                super::idt::raw_serial_str(b")\n");
                continue;
            }

            let l2_phys = l3e & 0x000F_FFFF_FFFF_F000;
            let l2_virt = (l2_phys + phys_offset) as *const u64;
            let l2e = core::ptr::read_volatile(l2_virt.add(l2i));
            if l2e & 1 == 0 {
                super::idt::raw_serial_str(b" L2=ABSENT!\n");
                continue;
            }
            // Check for 2MB huge page
            if l2e & (1 << 7) != 0 {
                super::idt::raw_serial_str(b" 2MB_HUGE(");
                if l2e & 2 != 0 {
                    super::idt::raw_serial_str(b"W");
                } else {
                    super::idt::raw_serial_str(b"-");
                }
                if l2e & 4 != 0 {
                    super::idt::raw_serial_str(b"U");
                } else {
                    super::idt::raw_serial_str(b"-");
                }
                if l2e & (1u64 << 63) != 0 {
                    super::idt::raw_serial_str(b"NX");
                } else {
                    super::idt::raw_serial_str(b"X");
                }
                super::idt::raw_serial_str(b")\n");
                continue;
            }

            let l1_phys = l2e & 0x000F_FFFF_FFFF_F000;
            let l1_virt = (l1_phys + phys_offset) as *const u64;
            let l1e = core::ptr::read_volatile(l1_virt.add(l1i));
            if l1e & 1 == 0 {
                super::idt::raw_serial_str(b" L1=ABSENT!\n");
                continue;
            }

            // Print L1 flags: W=writable, U=user, NX=no-execute
            super::idt::raw_serial_str(b" OK(");
            if l1e & 2 != 0 {
                super::idt::raw_serial_str(b"W");
            } else {
                super::idt::raw_serial_str(b"-");
            }
            if l1e & 4 != 0 {
                super::idt::raw_serial_str(b"U");
            } else {
                super::idt::raw_serial_str(b"-");
            }
            if l1e & (1u64 << 63) != 0 {
                super::idt::raw_serial_str(b"NX");
            } else {
                super::idt::raw_serial_str(b"X");
            }
            super::idt::raw_serial_str(b")\n");
        }
    }
}

/// Update the kernel stack pointer in the TSS (RSP0).
///
/// Called during context switch to set the stack used for Ring 3 -> Ring 0
/// transitions (interrupts, syscalls). Must be called with interrupts disabled.
///
/// # Safety
///
/// The TSS is a static initialized during boot. Modifying
/// `privilege_stack_table[0]` via raw pointer is safe because this is only
/// called from the scheduler with interrupts disabled, ensuring no concurrent
/// access.
pub fn set_kernel_stack(stack_top: u64) {
    // SAFETY: the TSS is a boot-initialized static that lives for the whole
    // kernel lifetime, so the pointer is valid. Callers are the scheduler
    // with interrupts disabled, so no other code reads or writes the field
    // concurrently. NOTE: the pointer is derived from a shared reference to
    // a `lazy_static` value that has no `UnsafeCell`; the write is outside
    // Rust's aliasing rules and relies on the compiler not caching RSP0.
    unsafe {
        let tss_ptr = &*TSS as *const TaskStateSegment as *mut TaskStateSegment;
        (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(stack_top);
    }
}

/// Read the current kernel stack pointer from the TSS (RSP0).
pub fn get_kernel_stack() -> u64 {
    TSS.privilege_stack_table[0].as_u64()
}

/// Global pointer to TSS.privilege_stack_table[0] (RSP0).
/// Used by enter_usermode_returnable to update TSS.RSP0 from naked asm.
/// Initialized by `init_tss_rsp0_ptr()` after the TSS is created.
pub(crate) static TSS_RSP0_PTR: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Initialize the TSS_RSP0_PTR to point to TSS.privilege_stack_table[0].
/// Must be called after GDT/TSS initialization.
pub fn init_tss_rsp0_ptr() {
    let tss_ptr = &*TSS as *const TaskStateSegment as *mut TaskStateSegment;
    // SAFETY: TSS is a static initialized during boot. We compute the
    // address of privilege_stack_table[0] which is a fixed offset within
    // the TSS structure. This address remains valid for the kernel's lifetime.
    let rsp0_addr = unsafe { core::ptr::addr_of_mut!((*tss_ptr).privilege_stack_table[0]) as u64 };
    TSS_RSP0_PTR.store(rsp0_addr, core::sync::atomic::Ordering::Release);
}
