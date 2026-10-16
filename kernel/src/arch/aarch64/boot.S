.section ".data"
.balign 8
.global veridian_dtb_pa
// Physical address of the device tree from x0 (HA-01): the Linux arm64
// boot protocol passes it there; QEMU does so for an Image, not for a
// bare ELF (x0 = 0, the blob then sits at the start of RAM).
veridian_dtb_pa:
    .dword 0

// The kernel's virtual minus physical address (ADR 0015), measured by
// boot.S before the MMU is on (`aarch64::boot::kernel_phys`).
.global veridian_kernel_offset
veridian_kernel_offset:
    .dword 0

// The boot CPU's MMU registers, for secondary CPUs: MAIR, TCR, TTBR0 (the
// identity root, for the switch), TTBR1, and the physical address of the
// empty root TTBR0 holds afterwards. Written before the MMU is on (so they
// are in memory, not only in a cache) and read by each secondary before it
// turns its own MMU on.
.global veridian_mmu_regs
veridian_mmu_regs:
    .dword 0, 0, 0, 0, 0

// SCTLR_EL1 with the MMU and caches off (HA-14/HA-15, N-177): only the
// bits that are RES1 in ARMv8.0 (LSMAOE, nTLSMD, SPAN, EIS, TSCXT, EOS),
// little-endian, no alignment checks. Writing 0 cleared those RES1 bits.
.equ VERIDIAN_SCTLR_EL1_MMU_OFF, 0x30D00800
// The same with the MMU (M), data cache (C) and instruction cache (I) on.
.equ VERIDIAN_SCTLR_EL1_MMU_ON, 0x30D01805

// MAIR_EL1: attribute 0 Normal write-back non-transient (0xFF), attribute 1
// Device-nGnRE (0x04); `mm::pte::arm` uses the same indices.
.equ VERIDIAN_MAIR_EL1, 0x04FF
// TCR_EL1 without IPS: T0SZ = T1SZ = 16 (48-bit halves), 4 KiB granules
// (TG0 = 0, TG1 = 2), inner-shareable write-back walks in both halves.
// IPS (bits 32-34) comes from ID_AA64MMFR0_EL1.PARange at boot.
.equ VERIDIAN_TCR_EL1, 0xB5103510

// Block descriptors (level 1, 1 GiB; level 2, 2 MiB) of the boot tables:
// valid, AF, and the memory type. Device: attribute 1, never executable.
// Normal: attribute 0, inner shareable. The physical map is never
// executable; the identity map is executable at EL1 only (the code between
// enabling the MMU and the jump to the upper half runs there), and so is
// the kernel image.
.equ VERIDIAN_BLOCK_DEVICE, 0x0060000000000405
.equ VERIDIAN_BLOCK_NORMAL_NX, 0x0060000000000701
.equ VERIDIAN_BLOCK_NORMAL_X, 0x0040000000000701

// The boot translation tables (ADR 0015), zeroed with the rest of .bss:
// the TTBR1 root, the physical map's level-1 table (root entry 256), the
// kernel image's level-1 and level-2 tables (root entry 511, level-1 entry
// 510), for TTBR0 an identity root with its own level-1 table (used only
// across the switch), and the empty root TTBR0 holds after it until a
// process address space is loaded.
.section ".bss.pgtables", "aw", %nobits
.balign 4096
veridian_pt_start:
veridian_pt_root:       .skip 4096
veridian_pt_physmap:    .skip 4096
veridian_pt_kernel_l1:  .skip 4096
veridian_pt_kernel_l2:  .skip 4096
veridian_pt_idroot:     .skip 4096
veridian_pt_ident:      .skip 4096
veridian_pt_empty:      .skip 4096
veridian_pt_end:

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
    // Until the MMU is on, this code runs at its physical address: it uses
    // only PC-relative addresses (adr, adrp) and constants.
    ldr x0, =VERIDIAN_SCTLR_EL1_MMU_OFF
    msr sctlr_el1, x0
    isb
    // CPACR_EL1.FPEN = 01: FP/SIMD traps at EL0 and not at EL1. The kernel
    // is soft-float (N-178) and uses none; user FP state is not switched
    // with the task yet, so an EL0 FP instruction is SIGILL (ADR 0016).
    mov x0, #(1 << 20)
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

    // Exception vectors at their physical address while the MMU is off
    // (HARDWARE-AARCH64.md bring-up step 1); reloaded high below.
    adrp x1, veridian_aarch64_vectors
    add x1, x1, :lo12:veridian_aarch64_vectors
    msr vbar_el1, x1
    isb

    // Clear BSS (the boot tables are in it).
    adrp x0, __bss_start
    add x0, x0, :lo12:__bss_start
    adrp x1, __bss_end
    add x1, x1, :lo12:__bss_end
1:
    cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:

    // x21 = the kernel's virtual minus physical address: _start's link
    // address (a literal) minus where it runs (adr).
    ldr x21, =_start
    adr x22, _start
    sub x21, x21, x22

    // The boot tables (ADR 0015).
    adrp x0, veridian_pt_root
    adrp x1, veridian_pt_physmap
    adrp x2, veridian_pt_ident
    adrp x3, veridian_pt_kernel_l1
    adrp x4, veridian_pt_kernel_l2
    adrp x5, veridian_pt_idroot
    orr x6, x1, #3
    str x6, [x0, #256 * 8]          // root[256]: the physical map
    orr x6, x3, #3
    str x6, [x0, #511 * 8]          // root[511]: the kernel image
    orr x6, x4, #3
    str x6, [x3, #510 * 8]          // kernel l1[510]: its 2 MiB blocks
    orr x6, x2, #3
    str x6, [x5]                    // identity root[0]
    // 512 x 1 GiB in the physical map and the identity map. The first GiB
    // (QEMU virt's MMIO) is Device, the rest Normal (ADR 0015 decision 3).
    ldr x8, =VERIDIAN_BLOCK_DEVICE
    ldr x9, =VERIDIAN_BLOCK_NORMAL_NX
    ldr x10, =VERIDIAN_BLOCK_NORMAL_X
    mov x7, #0
3:
    lsl x11, x7, #30
    orr x12, x11, x8
    orr x13, x11, x8
    cbz x7, 4f
    orr x12, x11, x9
    orr x13, x11, x10
4:
    str x12, [x1, x7, lsl #3]
    str x13, [x2, x7, lsl #3]
    add x7, x7, #1
    cmp x7, #512
    b.lo 3b
    // The kernel image: 2 MiB blocks from the 2 MiB-aligned base it was
    // loaded at (text_offset below _start), enough for image_size.
    adr x11, _start
    sub x11, x11, #0x200000
    // blocks = (text_offset + image_size + 2 MiB - 1) / 2 MiB
    ldr x12, =__veridian_image_size
    ldr x13, =(0x200000 + 0x1FFFFF)
    add x12, x12, x13
    lsr x12, x12, #21
    mov x7, #0
5:
    add x13, x11, x7, lsl #21
    orr x13, x13, x10
    str x13, [x4, x7, lsl #3]
    add x7, x7, #1
    cmp x7, x12
    b.lo 5b

    // The tables were written with the caches off: drop any stale lines
    // for them before the walker may read through the cache (HA-25).
    dsb sy
    mrs x6, ctr_el0
    ubfx x6, x6, #16, #4
    mov x7, #4
    lsl x7, x7, x6                  // data cache line size
    adrp x8, veridian_pt_start
    adrp x9, veridian_pt_end
6:
    dc ivac, x8
    add x8, x8, x7
    cmp x8, x9
    b.lo 6b
    dsb sy

    // MAIR, TCR (with IPS from PARange, at most 48 bits), the two roots;
    // saved for the secondary CPUs first.
    ldr x6, =VERIDIAN_MAIR_EL1
    ldr x7, =VERIDIAN_TCR_EL1
    mrs x8, id_aa64mmfr0_el1
    and x8, x8, #0x7
    cmp x8, #5
    mov x9, #5
    csel x8, x9, x8, hi
    bfi x7, x8, #32, #3
    adrp x9, veridian_mmu_regs
    add x9, x9, :lo12:veridian_mmu_regs
    stp x6, x7, [x9]
    stp x5, x0, [x9, #16]
    adrp x14, veridian_pt_empty
    str x14, [x9, #32]
    dsb sy
    msr mair_el1, x6
    msr tcr_el1, x7
    msr ttbr0_el1, x5
    msr ttbr1_el1, x0
    isb
    tlbi vmalle1
    dsb nsh
    isb
    ldr x6, =VERIDIAN_SCTLR_EL1_MMU_ON
    msr sctlr_el1, x6
    isb
    ic iallu
    dsb nsh
    isb
    // Continue at the link address, in the upper half.
    ldr x6, =veridian_boot_high
    br x6

veridian_boot_high:
    // The identity map was only for the switch: TTBR0 gets the empty root
    // (x14, a physical address), so the lower half belongs to processes.
    msr ttbr0_el1, x14
    isb
    tlbi vmalle1
    dsb nsh
    isb
    // Stack, vectors and everything else at their link addresses now.
    adrp x1, __stack_top
    add x1, x1, :lo12:__stack_top
    and sp, x1, #~15
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

    // The device tree pointer and the kernel offset.
    adrp x0, veridian_dtb_pa
    str x20, [x0, :lo12:veridian_dtb_pa]
    adrp x0, veridian_kernel_offset
    str x21, [x0, :lo12:veridian_kernel_offset]
    dsb sy

    // Call Rust main
    bl _start_rust

    // Halt if we return (should never happen)
halt:
1:
    wfe
    b 1b

// Secondary CPU entry (PSCI CPU_ON): MMU and caches off, DAIF masked,
// entered at its physical address with x0 = context_id = the physical
// address of ApBootArgs (arch/smp_boot.rs). Entered at EL2 when the
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
    mov x1, #(1 << 20)          // CPACR_EL1.FPEN = 01, as on the boot CPU
    msr cpacr_el1, x1
    isb
    // The boot CPU's MMU registers (written to memory before its MMU was
    // on), then the MMU on and the jump to the upper half, as on the boot
    // CPU. Until then this runs at its physical address.
    adrp x1, veridian_mmu_regs
    add x1, x1, :lo12:veridian_mmu_regs
    ldp x2, x3, [x1]
    ldp x4, x5, [x1, #16]
    msr mair_el1, x2
    msr tcr_el1, x3
    msr ttbr0_el1, x4
    msr ttbr1_el1, x5
    isb
    tlbi vmalle1
    dsb nsh
    isb
    ldr x1, =VERIDIAN_SCTLR_EL1_MMU_ON
    msr sctlr_el1, x1
    isb
    ic iallu
    dsb nsh
    isb
    ldr x1, =veridian_secondary_high
    br x1
veridian_secondary_high:
    // TTBR0: the empty root, as on the boot CPU.
    adrp x1, veridian_mmu_regs
    add x1, x1, :lo12:veridian_mmu_regs
    ldr x1, [x1, #32]
    msr ttbr0_el1, x1
    isb
    tlbi vmalle1
    dsb nsh
    isb
    // x19 is the boot block's physical address: make it a kernel address.
    adrp x1, veridian_kernel_offset
    ldr x1, [x1, :lo12:veridian_kernel_offset]
    add x19, x19, x1
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
