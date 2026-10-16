.section ".data"
.balign 8
.global veridian_dtb_pa
// Physical address of the device tree from x0 (HA-01): the Linux arm64
// boot protocol passes it there; QEMU does so for an Image, not for a
// bare ELF (x0 = 0, the blob then sits at the start of RAM).
veridian_dtb_pa:
    .dword 0

// SCTLR_EL1 with the MMU and caches off (HA-14/HA-15, N-177): only the
// bits that are RES1 in ARMv8.0 (LSMAOE, nTLSMD, SPAN, EIS, TSCXT, EOS),
// little-endian, no alignment checks. Writing 0 cleared those RES1 bits.
.equ VERIDIAN_SCTLR_EL1_MMU_OFF, 0x30D00800

.section ".text.boot"
.global _start

_start:
    // arm64 Image header (HA-03; Linux Documentation/arch/arm64/booting.rst),
    // so that the `objcopy -O binary` image (veridian-kernel.img) boots
    // through QEMU's and U-Boot's `booti` path, which pass the device tree
    // in x0. The ELF enters here too and takes the branch.
    b veridian_aarch64_entry         // code0
    .word 0                          // code1
    .quad 0x200000                   // text_offset: the link base is RAM + 2 MiB
    .quad __veridian_image_size      // image_size, BSS and boot stack included
    .quad 0x2                        // flags: little-endian, 4 KiB pages,
                                     // 2 MiB-aligned base near the start of RAM
    .quad 0, 0, 0                    // res2-res4
    .byte 0x41, 0x52, 0x4d, 0x64     // magic "ARM\x64" at 0x38
    .word 0                          // res5 (no PE/COFF header)

veridian_aarch64_entry:
    // x20 keeps x0 (the device tree) through the EL2 -> EL1 drop.
    mov x20, x0
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

    // EL2 state the EL1 kernel relies on (HA-14, as Linux's init_el2):
    // FP/SIMD not trapped (CPTR_EL2 RES1 bits only, TFP = 0), no
    // coprocessor traps, and EL1 reads the real MIDR/MPIDR.
    mov x0, #0x33ff
    msr cptr_el2, x0
    msr hstr_el2, xzr
    mrs x0, midr_el1
    msr vpidr_el2, x0
    mrs x0, mpidr_el1
    msr vmpidr_el2, x0
    
    // SCTLR_EL1: RES1 bits, MMU and caches off
    ldr x0, =VERIDIAN_SCTLR_EL1_MMU_OFF
    msr sctlr_el1, x0
    
    // Return to EL1
    eret

el1_entry:
    // Entered at EL1 directly, SCTLR_EL1 holds whatever firmware left.
    ldr x0, =VERIDIAN_SCTLR_EL1_MMU_OFF
    msr sctlr_el1, x0
    isb
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

    // Exception vectors as soon as there is a stack for them (HARDWARE-AARCH64.md bring-up step 1), so
    // that a fault anywhere after this is reported (exceptions.rs) rather
    // than vectoring wherever firmware left VBAR_EL1.
    adrp x1, veridian_aarch64_vectors
    add x1, x1, :lo12:veridian_aarch64_vectors
    msr vbar_el1, x1
    isb
    
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

    // The device tree pointer, now that nothing will clear it.
    adrp x0, veridian_dtb_pa
    str x20, [x0, :lo12:veridian_dtb_pa]
    
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
    mov x1, #0x33ff
    msr cptr_el2, x1
    msr hstr_el2, xzr
    mrs x1, midr_el1
    msr vpidr_el2, x1
    mrs x1, mpidr_el1
    msr vmpidr_el2, x1
    ldr x1, =VERIDIAN_SCTLR_EL1_MMU_OFF
    msr sctlr_el1, x1
    eret
3:
    ldr x1, =VERIDIAN_SCTLR_EL1_MMU_OFF
    msr sctlr_el1, x1
    isb
    mov x1, #(3 << 20)
    msr cpacr_el1, x1
    isb
    ldr x1, [x19, #0x08]
    and sp, x1, #~15
    // Exception vectors before any Rust code, as on the boot CPU.
    adrp x1, veridian_aarch64_vectors
    add x1, x1, :lo12:veridian_aarch64_vectors
    msr vbar_el1, x1
    isb
    ldr x1, [x19, #0x10]
    msr tpidr_el1, x1
    mov x29, #0
    mov x30, #0
    mov x0, x19
    bl veridian_secondary_rust
4:
    wfe
    b 4b
