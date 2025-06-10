//! x86_64 context switching implementation

use core::arch::asm;
use crate::sched::task::TaskContext;
use crate::arch::context::ThreadContext;

/// x86_64 CPU context
#[repr(C)]
#[derive(Debug, Clone)]
pub struct X86_64Context {
    /// General purpose registers
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,

    /// Stack pointer
    pub rsp: u64,

    /// Instruction pointer
    pub rip: u64,

    /// CPU flags
    pub rflags: u64,

    /// Segment registers
    pub cs: u16,
    pub ss: u16,
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,

    /// Control registers
    pub cr3: u64, // Page table base

    /// Floating point state pointer
    pub fpu_state: *mut FpuState,
}

/// x86_64 FPU/SSE/AVX state
#[repr(C, align(64))]
pub struct FpuState {
    /// FXSAVE area (512 bytes)
    pub fxsave: [u8; 512],
    /// Extended state (AVX, etc.)
    pub xsave: [u8; 2048],
}

impl X86_64Context {
    /// Create new context for a task
    pub fn new(entry_point: usize, stack_pointer: usize) -> Self {
        Self {
            // Clear all general purpose registers
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rdi: 0,
            rsi: 0,
            rbp: 0,
            rbx: 0,
            rdx: 0,
            rcx: 0,
            rax: 0,

            // Set stack pointer to top of stack
            rsp: stack_pointer as u64,

            // Set instruction pointer to entry point
            rip: entry_point as u64,

            // Default RFLAGS (interrupts enabled)
            rflags: 0x202,

            // Kernel segments
            cs: 0x08, // Kernel code segment
            ss: 0x10, // Kernel data segment
            ds: 0x10,
            es: 0x10,
            fs: 0x00,
            gs: 0x00,

            // Will be set to actual page table
            cr3: 0,

            // Will be allocated if FPU is used
            fpu_state: core::ptr::null_mut(),
        }
    }
}

impl crate::arch::context::ThreadContext for X86_64Context {
    fn new() -> Self {
        Self::default()
    }
    
    fn init(&mut self, entry_point: usize, stack_pointer: usize, _kernel_stack: usize) {
        self.rip = entry_point as u64;
        self.rsp = stack_pointer as u64;
        // TODO: Set up kernel stack in TSS
    }
    
    fn get_instruction_pointer(&self) -> usize {
        self.rip as usize
    }
    
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.rip = ip as u64;
    }
    
    fn get_stack_pointer(&self) -> usize {
        self.rsp as usize
    }
    
    fn set_stack_pointer(&mut self, sp: usize) {
        self.rsp = sp as u64;
    }
    
    fn get_kernel_stack(&self) -> usize {
        // TODO: Return from TSS
        0
    }
    
    fn set_kernel_stack(&mut self, _sp: usize) {
        // TODO: Set in TSS
    }
    
    fn set_return_value(&mut self, value: usize) {
        self.rax = value as u64;
    }
    
    fn clone_from(&mut self, other: &Self) {
        *self = other.clone();
    }
    
    fn to_task_context(&self) -> TaskContext {
        TaskContext::X86_64(self.clone())
    }
}

/// Switch from current context to new context
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[no_mangle]
pub unsafe extern "C" fn context_switch(current: *mut X86_64Context, next: *const X86_64Context) {
    // Save current context
    asm!(
        // Save general purpose registers
        "mov [rdi + 0x00], r15",
        "mov [rdi + 0x08], r14",
        "mov [rdi + 0x10], r13",
        "mov [rdi + 0x18], r12",
        "mov [rdi + 0x20], r11",
        "mov [rdi + 0x28], r10",
        "mov [rdi + 0x30], r9",
        "mov [rdi + 0x38], r8",
        "mov [rdi + 0x40], rdi",
        "mov [rdi + 0x48], rsi",
        "mov [rdi + 0x50], rbp",
        "mov [rdi + 0x58], rbx",
        "mov [rdi + 0x60], rdx",
        "mov [rdi + 0x68], rcx",
        "mov [rdi + 0x70], rax",

        // Save stack pointer
        "mov [rdi + 0x78], rsp",

        // Save return address as RIP
        "mov rax, [rsp]",
        "mov [rdi + 0x80], rax",

        // Save RFLAGS
        "pushfq",
        "pop rax",
        "mov [rdi + 0x88], rax",

        in("rdi") current,
        in("rsi") next,
        lateout("rax") _,
        lateout("rcx") _,
        lateout("rdx") _,
    );

    // Load new context
    asm!(
        // Load new CR3 if different
        "mov rax, [rsi + 0xB0]",
        "mov rcx, cr3",
        "cmp rax, rcx",
        "je 2f",
        "mov cr3, rax",
        "2:",

        // Load general purpose registers
        "mov r15, [rsi + 0x00]",
        "mov r14, [rsi + 0x08]",
        "mov r13, [rsi + 0x10]",
        "mov r12, [rsi + 0x18]",
        "mov r11, [rsi + 0x20]",
        "mov r10, [rsi + 0x28]",
        "mov r9,  [rsi + 0x30]",
        "mov r8,  [rsi + 0x38]",
        "mov rdi, [rsi + 0x40]",
        // Skip rsi for now
        "mov rbp, [rsi + 0x50]",
        "mov rbx, [rsi + 0x58]",
        "mov rdx, [rsi + 0x60]",
        "mov rcx, [rsi + 0x68]",
        "mov rax, [rsi + 0x70]",

        // Load RFLAGS
        "push qword ptr [rsi + 0x88]",
        "popfq",

        // Load stack pointer
        "mov rsp, [rsi + 0x78]",

        // Push return address
        "push qword ptr [rsi + 0x80]",

        // Finally load rsi
        "mov rsi, [rsi + 0x48]",

        // Return to new context
        "ret",

        in("rsi") next,
        lateout("rax") _,
        lateout("rcx") _,
        lateout("rdx") _,
        lateout("r8") _,
        lateout("r9") _,
        lateout("r10") _,
        lateout("r11") _,
        lateout("r12") _,
        lateout("r13") _,
        lateout("r14") _,
        lateout("r15") _,
    );
}

/// Switch context using the ThreadContext interface
#[allow(dead_code)]
pub fn switch_context(from: &mut X86_64Context, to: &X86_64Context) {
    unsafe {
        context_switch(from as *mut _, to as *const _);
    }
}

/// Save FPU state
#[allow(dead_code)]
pub fn save_fpu_state(state: &mut FpuState) {
    unsafe {
        asm!("fxsave [{}]", in(reg) state as *mut FpuState);
    }
}

/// Restore FPU state
#[allow(dead_code)]
pub fn restore_fpu_state(state: &FpuState) {
    unsafe {
        asm!("fxrstor [{}]", in(reg) state as *const FpuState);
    }
}

/// Initialize FPU for current CPU
#[allow(dead_code)]
pub fn init_fpu() {
    unsafe {
        // Enable FPU
        asm!(
            "mov rax, cr0",
            "and ax, 0xFFFB",  // Clear EM bit
            "or ax, 0x2",      // Set MP bit
            "mov cr0, rax",

            // Enable SSE
            "mov rax, cr4",
            "or ax, 0x600",    // Set OSFXSR and OSXMMEXCPT
            "mov cr4, rax",

            // Initialize FPU state
            "fninit",
            out("rax") _,
        );
    }
}

/// Check if CPU supports XSAVE
#[allow(dead_code)]
pub fn has_xsave() -> bool {
    unsafe {
        let result: u32;
        asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "mov {0:r}, rcx",
            "pop rbx",
            out(reg) result,
            out("eax") _,
            out("edx") _,
            lateout("ecx") _,
        );
        (result & (1 << 26)) != 0
    }
}

/// Enable XSAVE if supported
#[allow(dead_code)]
pub fn enable_xsave() {
    if has_xsave() {
        unsafe {
            asm!(
                "mov rax, cr4",
                "or rax, 0x40000",  // Set OSXSAVE bit
                "mov cr4, rax",
                out("rax") _,
            );
        }
    }
}

impl Default for X86_64Context {
    fn default() -> Self {
        Self {
            r15: 0, r14: 0, r13: 0, r12: 0, r11: 0, r10: 0, r9: 0, r8: 0,
            rdi: 0, rsi: 0, rbp: 0, rbx: 0, rdx: 0, rcx: 0, rax: 0,
            rsp: 0, rip: 0, rflags: 0x202,
            cs: 0x08, ss: 0x10, ds: 0x10, es: 0x10, fs: 0x00, gs: 0x00,
            cr3: 0, fpu_state: core::ptr::null_mut(),
        }
    }
}

// SAFETY: X86_64Context can be safely sent between threads
// The FPU state pointer is either null or points to thread-local data
unsafe impl Send for X86_64Context {}
unsafe impl Sync for X86_64Context {}
