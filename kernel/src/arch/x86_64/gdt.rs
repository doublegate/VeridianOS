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

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();

        // Set up the kernel stack for privilege level 0
        // This is used when transitioning from user mode to kernel mode.
        // Must be 16-byte aligned for the x86_64 ABI (movaps et al.).
        tss.privilege_stack_table[0] = {
            const STACK_SIZE: usize = 4096 * 5;
            #[repr(align(16))]
            #[allow(dead_code)] // Alignment wrapper -- field accessed via raw pointer
            struct AlignedStack([u8; STACK_SIZE]);
            static mut KERNEL_STACK: AlignedStack = AlignedStack([0; STACK_SIZE]);

            let stack_ptr = &raw const KERNEL_STACK;
            let stack_start = VirtAddr::from_ptr(stack_ptr);
            stack_start + STACK_SIZE as u64
        };

        // Set up the double fault stack (16-byte aligned)
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            #[repr(align(16))]
            #[allow(dead_code)] // Alignment wrapper -- field accessed via raw pointer
            struct AlignedStack([u8; STACK_SIZE]);
            static mut STACK: AlignedStack = AlignedStack([0; STACK_SIZE]);

            let stack_ptr = &raw const STACK;
            let stack_start = VirtAddr::from_ptr(stack_ptr);
            stack_start + STACK_SIZE as u64
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

/// Returns a reference to the GDT selectors (kernel and user mode).
///
/// Must only be called after `init()` has been called. The lazy_static
/// ensures the GDT is initialized on first access.
pub fn selectors() -> &'static Selectors {
    &GDT.1
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
    unsafe {
        let tss_ptr = &*TSS as *const TaskStateSegment as *mut TaskStateSegment;
        (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(stack_top);
    }
}

/// Read the current kernel stack pointer from the TSS (RSP0).
pub fn get_kernel_stack() -> u64 {
    TSS.privilege_stack_table[0].as_u64()
}
