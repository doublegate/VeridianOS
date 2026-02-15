//! x86_64 context switching implementation

use core::arch::asm;

use crate::sched::task::TaskContext;

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
        // Adjust stack pointer to leave room for a fake return address
        // This prevents issues if the called function tries to access stack arguments
        let adjusted_sp = (stack_pointer - 8) as u64;

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

            // Set stack pointer with adjustment
            rsp: adjusted_sp,

            // Set instruction pointer to entry point
            rip: entry_point as u64,

            // Default RFLAGS (interrupts disabled for now)
            rflags: 0x002,

            // Kernel segments
            cs: 0x08, // Kernel code segment
            ss: 0x10, // Kernel data segment
            ds: 0x10,
            es: 0x10,
            fs: 0x00,
            gs: 0x00,

            // Initialize with current CR3
            // SAFETY: Reading CR3 is always valid in kernel mode. It returns
            // the current page table base address. No side effects.
            cr3: unsafe {
                let mut cr3: u64;
                asm!("mov {}, cr3", out(reg) cr3);
                cr3
            },

            // Will be allocated if FPU is used
            fpu_state: core::ptr::null_mut(),
        }
    }

    /// Create a user-mode context for a process.
    ///
    /// Uses Ring 3 segment selectors (CS=0x33, SS/DS/ES=0x2B) and enables
    /// interrupts (RFLAGS IF bit). The CR3 is set to the current page table
    /// so the process initially shares the kernel's address space (with
    /// user-accessible mappings added separately).
    #[allow(dead_code)]
    pub fn new_user(entry_point: usize, stack_pointer: usize) -> Self {
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

            // Set stack pointer
            rsp: stack_pointer as u64,

            // Set instruction pointer to user entry point
            rip: entry_point as u64,

            // RFLAGS with IF set (interrupts enabled in user mode)
            rflags: 0x202,

            // Ring 3 segment selectors
            cs: 0x33, // User code segment (GDT offset 0x30 + RPL 3)
            ss: 0x2B, // User data segment (GDT offset 0x28 + RPL 3)
            ds: 0x2B,
            es: 0x2B,
            fs: 0x00,
            gs: 0x00,

            // Use current page table (caller must ensure user mappings exist)
            // SAFETY: Reading CR3 is always valid in kernel mode. It returns
            // the current page table base address. No side effects.
            cr3: unsafe {
                let mut cr3: u64;
                asm!("mov {}, cr3", out(reg) cr3);
                cr3
            },

            // No FPU state initially
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
        // TODO(future): Set up kernel stack in TSS for ring transitions
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
        // TODO(future): Return kernel stack pointer from TSS
        0
    }

    fn set_kernel_stack(&mut self, _sp: usize) {
        // TODO(future): Set kernel stack in TSS for ring transitions
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

/// Switch context using the ThreadContext interface.
/// Called from `crate::arch::context::switch_context()`.
pub fn switch_context(from: &mut X86_64Context, to: &X86_64Context) {
    // SAFETY: Both `from` and `to` are valid references to X86_64Context
    // structs. context_switch is an assembly routine that saves the current
    // CPU state into `from` and restores state from `to`. Both contexts
    // must have valid register values for safe execution.
    unsafe {
        context_switch(from as *mut _, to as *const _);
    }
}

/// Save FPU state. Called from `crate::arch::context::save_fpu_state()`.
pub fn save_fpu_state(state: &mut FpuState) {
    // SAFETY: `state` is a valid mutable reference to a FpuState struct.
    // The FXSAVE instruction stores the FPU/SSE state into the provided
    // 512-byte aligned memory region. FpuState is repr(C, align(16)).
    unsafe {
        asm!("fxsave [{}]", in(reg) state as *mut FpuState);
    }
}

/// Restore FPU state. Called from `crate::arch::context::restore_fpu_state()`.
pub fn restore_fpu_state(state: &FpuState) {
    // SAFETY: `state` is a valid reference to a FpuState struct containing
    // previously saved FPU/SSE state. The FXRSTOR instruction restores the
    // FPU/SSE state from the provided memory region.
    unsafe {
        asm!("fxrstor [{}]", in(reg) state as *const FpuState);
    }
}

/// Initialize FPU for current CPU. Called from
/// `crate::arch::context::init_fpu()`.
pub fn init_fpu() {
    // SAFETY: FPU initialization modifies CR0 and CR4 control registers to
    // enable floating point and SSE support. This must only be called once
    // during early kernel initialization. The register modifications are
    // standard x86_64 FPU setup: clear EM, set MP and NE in CR0, set
    // OSFXSR and OSXMMEXCPT in CR4.
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
    // SAFETY: CPUID with leaf 1 is always valid on x86_64. We check
    // bit 26 of ECX (XSAVE feature flag). The push/pop of RBX is
    // required because CPUID clobbers RBX and Rust may use it.
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
        // SAFETY: XSAVE support was verified by has_xsave() above.
        // Setting the OSXSAVE bit (bit 18) in CR4 enables the OS to
        // use XSAVE/XRSTOR instructions for extended state management.
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
            rsp: 0,
            rip: 0,
            rflags: 0x202,
            cs: 0x08,
            ss: 0x10,
            ds: 0x10,
            es: 0x10,
            fs: 0x00,
            gs: 0x00,
            cr3: 0,
            fpu_state: core::ptr::null_mut(),
        }
    }
}

// SAFETY: X86_64Context can be safely sent between threads
// The FPU state pointer is either null or points to thread-local data
unsafe impl Send for X86_64Context {}
unsafe impl Sync for X86_64Context {}

/// Load context for first time (no previous context to save)
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[no_mangle]
pub unsafe extern "C" fn load_context(context: *const X86_64Context) {
    // Load context directly using inline assembly
    // For kernel-to-kernel context switch, we can use a simpler approach
    asm!(
        // rdi = context pointer

        // Load CR3 (page table) first if not zero
        "mov rax, [rdi + 160]", // cr3
        "test rax, rax",
        "jz 2f",
        "mov cr3, rax",
        "2:",

        // Load segment registers
        "mov ax, [rdi + 148]", // ds
        "mov ds, ax",
        "mov ax, [rdi + 150]", // es
        "mov es, ax",

        // Load stack pointer and push return address
        "mov rsp, [rdi + 120]",
        "push qword ptr [rdi + 128]", // Push RIP as return address

        // Load RFLAGS
        "push qword ptr [rdi + 136]",
        "popfq",

        // Load general purpose registers
        "mov r15, [rdi]",
        "mov r14, [rdi + 8]",
        "mov r13, [rdi + 16]",
        "mov r12, [rdi + 24]",
        "mov r11, [rdi + 32]",
        "mov r10, [rdi + 40]",
        "mov r9,  [rdi + 48]",
        "mov r8,  [rdi + 56]",
        "mov rsi, [rdi + 72]",
        "mov rbp, [rdi + 80]",
        "mov rbx, [rdi + 88]",
        "mov rdx, [rdi + 96]",
        "mov rcx, [rdi + 104]",
        "mov rax, [rdi + 112]",

        // Load final register
        "mov rdi, [rdi + 64]",

        // Return to loaded context (RIP was pushed earlier)
        "ret",
        in("rdi") context,
        options(noreturn)
    );
}
