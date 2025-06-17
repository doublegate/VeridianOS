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
        // This is used when transitioning from user mode to kernel mode
        tss.privilege_stack_table[0] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut KERNEL_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            
            let stack_ptr = &raw const KERNEL_STACK;
            let stack_start = VirtAddr::from_ptr(stack_ptr);
            stack_start + STACK_SIZE as u64
        };
        
        // Set up the double fault stack
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

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
        let code_selector = gdt.append(Descriptor::kernel_code_segment());
        let data_selector = gdt.append(Descriptor::kernel_data_segment());
        let tss_selector = gdt.append(Descriptor::tss_segment(&TSS));
        (
            gdt,
            Selectors {
                code_selector,
                data_selector,
                tss_selector,
            },
        )
    };
}

#[allow(dead_code)]
struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

#[allow(dead_code)]
pub fn init() {
    use x86_64::instructions::{
        segmentation::{Segment, CS, DS},
        tables::load_tss,
    };

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        DS::set_reg(GDT.1.data_selector);
        load_tss(GDT.1.tss_selector);
    }
}
