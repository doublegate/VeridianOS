.section ".text.boot"
.global _start

_start:
    // Check current exception level
    mrs x0, CurrentEL
    and x0, x0, #0xC
    cmp x0, #0x8
    b.eq el2_entry
    cmp x0, #0x4
    b.eq el1_entry
    b halt  // Unsupported EL3

el2_entry:
    // Configure EL1 execution state
    mov x0, #0x3c5  // EL1h, D/A/I/F masked
    msr spsr_el2, x0
    
    // Set EL1 entry point
    adr x0, el1_entry
    msr elr_el2, x0
    
    // Configure HCR_EL2 (RW=1 for AArch64)
    mov x0, #(1 << 31)
    msr hcr_el2, x0

    // Generic timer: let EL1 use the physical counter/timer (CNTHCTL_EL2
    // EL1PCTEN|EL1PCEN), and zero CNTVOFF_EL2 -- it has no defined reset
    // value, and the virtual counter is CNTPCT - CNTVOFF.
    mov x0, #3
    msr cnthctl_el2, x0
    msr cntvoff_el2, xzr
    
    // Configure SCTLR_EL1 (safe defaults)
    mov x0, #0x0
    msr sctlr_el1, x0
    
    // Return to EL1
    eret

el1_entry:
    // Enable FP/NEON - LLVM may emit SIMD instructions (e.g. movi/str q0
    // for buffer zeroing). Without this, those instructions trap silently.
    mov x0, #(3 << 20)
    msr cpacr_el1, x0
    isb

    // Write early boot indicator to UART
    // Load UART base address 0x09000000 using ldr
    ldr x0, =0x09000000
    
    // Write "AA64\n" to UART
    mov w1, #'A'
    strb w1, [x0]
    strb w1, [x0]  // Second 'A'
    mov w1, #'6'
    strb w1, [x0]
    mov w1, #'4'
    strb w1, [x0]
    mov w1, #'\n'
    strb w1, [x0]
    
    // Set up stack using linker-defined symbol
    adrp x1, __stack_top
    add x1, x1, :lo12:__stack_top
    // Ensure 16-byte alignment
    and sp, x1, #~15
    
    // Initialize frame pointer for ABI compliance
    mov x29, #0
    mov x30, #0
    
    // Write stack canary value at bottom of stack
    adrp x2, __stack_bottom
    add x2, x2, :lo12:__stack_bottom
    movz x3, #0xDEAD
    movk x3, #0xBEEF, lsl #16
    movk x3, #0xDEAD, lsl #32
    movk x3, #0xBEEF, lsl #48
    str x3, [x2]
    
    // Add memory barrier to ensure stack writes are visible
    dsb sy
    isb
    
    // Clear BSS
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:
    cmp x0, x1
    b.eq 2f
    str xzr, [x0], #8
    b 1b
2:
    
    // Call Rust main
    bl _start_rust
    
    // Halt if we return (should never happen)
halt:
1:
    wfe
    b 1b

// Secondary CPU entry (PSCI CPU_ON): MMU and caches off, DAIF masked,
// x0 = context_id = &ApBootArgs (arch/smp_boot.rs). Entered at EL2 when the
// CPU has it, else EL1; drops to EL1 like the boot CPU, then takes the
// stack and the per-CPU block from the boot args and enters Rust.
.section ".text.boot"
.global veridian_secondary_entry
veridian_secondary_entry:
    mov x19, x0
    mrs x1, CurrentEL
    and x1, x1, #0xC
    cmp x1, #0x8
    b.ne 3f
    mov x1, #0x3c5
    msr spsr_el2, x1
    adr x1, 3f
    msr elr_el2, x1
    mov x1, #(1 << 31)
    msr hcr_el2, x1
    mov x1, #3
    msr cnthctl_el2, x1
    msr cntvoff_el2, xzr
    msr sctlr_el1, xzr
    eret
3:
    mov x1, #(3 << 20)
    msr cpacr_el1, x1
    isb
    ldr x1, [x19, #0x08]
    and sp, x1, #~15
    ldr x1, [x19, #0x10]
    msr tpidr_el1, x1
    mov x29, #0
    mov x30, #0
    mov x0, x19
    bl veridian_secondary_rust
4:
    wfe
    b 4b

.section .bss
.align 16
_stack_bottom:
    .space 0x4000
_stack_top: