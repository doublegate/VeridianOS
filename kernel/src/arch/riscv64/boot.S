.section .data
.balign 8
.global veridian_dtb_pa
# Physical address of the device tree blob SBI firmware passes in a1.
# Saved before anything else touches a1; .data so nothing clears it.
veridian_dtb_pa:
    .dword 0

.global veridian_boot_hartid
# Hart id of the boot hart, which SBI passes in a0. It need not be 0
# (OpenSBI picks the boot hart), so it is saved before the registers are
# cleared below.
veridian_boot_hartid:
    .dword 0

# Hart lottery (HR-19): the first hart to reach _start boots; the others
# stop. Firmware without SBI HSM starts every hart here at once.
veridian_boot_lottery:
    .word 0

.balign 8
.global veridian_kernel_satp
# The kernel's satp (Sv48 and the kernel root, no identity map).
veridian_kernel_satp:
    .dword 0
# The trampoline's satp: the kernel root plus the identity map, for the
# few instructions between turning paging on at the physical address and
# running at the kernel's own. Secondary harts use it the same way.
veridian_trampoline_satp:
    .dword 0

# Early page tables (E-RV3), zeroed with the rest of .bss.
.section .bss
.balign 4096
# The kernel root: L4[511] only, so the lower half is free for user
# address spaces (they share that entry). Everything the kernel maps is in
# Sv39's kernel half, 0xFFFF_FFC0_0000_0000 and up (ADR 0018).
.global veridian_kernel_root
veridian_kernel_root:
    .space 4096
# The trampoline root: the same plus L4[0], the identity map.
veridian_early_root:
    .space 4096
# The identity map, for the trampoline only: 512 GiB in 1 GiB pages at
# their physical address (L4[0]).
veridian_early_l3_phys:
    .space 4096
# The kernel half (L4[511]), in 1 GiB pages:
#   L3[256..383]  the physical map, 128 GiB: virtual = physical +
#                 0xFFFF_FFC0_0000_0000;
#   L3[384..399]  the kernel stacks (mm::kstack), filled in later;
#   L3[510..511]  the image at its link address, 0xFFFF_FFFF_8000_0000 +
#                 2 MiB = physical 2..4 GiB.
veridian_early_l3_kernel:
    .space 4096

.section .text.boot
.global _start

_start:
    la t0, veridian_boot_lottery
    li t1, 1
    # The assembler may not see the target's A extension here (release
    # LTO builds this file without it), so name it.
    .option push
    .option arch, +a
    amoadd.w t1, t1, (t0)
    .option pop
    bnez t1, .Llost_lottery

    la t0, veridian_dtb_pa
    sd a1, 0(t0)
    la t0, veridian_boot_hartid
    sd a0, 0(t0)

    # Use SBI console putchar (ecall) for output
    # SBI legacy extension: function ID 0x01 for putchar
    
    li a0, 'R'           # Character to print
    li a7, 0x01          # SBI function ID 0x01 = console_putchar (legacy)
    ecall
    
    li a0, 'V'
    li a7, 0x01
    ecall
    
    li a0, '6'
    li a7, 0x01
    ecall
    
    li a0, '4'
    li a7, 0x01
    ecall
    
    li a0, '\n'
    li a7, 0x01
    ecall
    
    # Clear all registers
    li x1, 0
    li x2, 0
    li x3, 0
    li x4, 0
    li x5, 0
    li x6, 0
    li x7, 0
    li x8, 0
    li x9, 0
    li x10, 0
    li x11, 0
    li x12, 0
    li x13, 0
    li x14, 0
    li x15, 0
    li x16, 0
    li x17, 0
    li x18, 0
    li x19, 0
    li x20, 0
    li x21, 0
    li x22, 0
    li x23, 0
    li x24, 0
    li x25, 0
    li x26, 0
    li x27, 0
    li x28, 0
    li x29, 0
    li x30, 0
    li x31, 0

    # Set up stack - use __stack_top from linker script
    la sp, __stack_top

    # Zero .bss (HR-01): QEMU's ELF loader does, but a flat Image loaded
    # by U-Boot or EFI carries no bss contents.
    la t0, __bss_start
    la t1, __bss_end
3:
    bgeu t0, t1, 4f
    sd zero, 0(t0)
    addi t0, t0, 8
    j 3b
4:

    # FP off (HR-20): the kernel is soft-float (N-178), so an FP
    # instruction in kernel code traps instead of using user registers.
    li t0, 0x6000
    csrc sstatus, t0

    # Paging (E-RV3). This code runs at its physical address (`la` is
    # PC-relative), the kernel is linked in the upper half. Leaves are
    # 1 GiB pages: V R W X G A D = 0xEF; a table pointer is V = 1; the
    # PPN sits at bit 10, so an aligned physical address >> 2.
    la a0, veridian_early_l3_phys
    li t1, 0
    li t2, 512
    li t3, 0x40000000
    li t4, 0xEF
6:
    srli t5, t1, 2
    or t5, t5, t4
    sd t5, 0(a0)
    addi a0, a0, 8
    add t1, t1, t3
    addi t2, t2, -1
    bnez t2, 6b

    # The physical map: L3[256 + i] = physical i GiB, for 128 GiB.
    la a0, veridian_early_l3_kernel
    li t6, 256 * 8
    add a0, a0, t6
    li t1, 0
    li t2, 128
9:
    srli t5, t1, 2
    or t5, t5, t4
    sd t5, 0(a0)
    addi a0, a0, 8
    add t1, t1, t3
    addi t2, t2, -1
    bnez t2, 9b

    la a0, veridian_early_l3_kernel
    li t6, 510 * 8
    add t6, a0, t6
    li t1, 0x80000000
    srli t5, t1, 2
    or t5, t5, t4
    sd t5, 0(t6)
    li t1, 0xC0000000
    srli t5, t1, 2
    or t5, t5, t4
    sd t5, 8(t6)

    # Both roots: L4[511] (the kernel half); the trampoline root also L4[0]
    # (identity).
    la t1, veridian_early_l3_phys
    srli t2, t1, 2
    ori t2, t2, 1
    la t1, veridian_early_l3_kernel
    srli t3, t1, 2
    ori t3, t3, 1
    li t1, 511 * 8
    la a0, veridian_kernel_root
    add t6, a0, t1
    sd t3, 0(t6)
    la a0, veridian_early_root
    sd t2, 0(a0)
    add t6, a0, t1
    sd t3, 0(t6)

    # satp = Sv48 (mode 9) | root PPN, for each root. satp is WARL: a hart
    # without Sv48 keeps it 0, and then the jump below would fault, so stop
    # instead.
    li t6, 9
    slli t6, t6, 60
    la a0, veridian_kernel_root
    srli t5, a0, 12
    or t5, t5, t6
    la a0, veridian_kernel_satp
    sd t5, 0(a0)
    la a0, veridian_early_root
    srli t5, a0, 12
    or t5, t5, t6
    la a0, veridian_trampoline_satp
    sd t5, 0(a0)
    sfence.vma
    csrw satp, t5
    sfence.vma
    csrr t6, satp
    bne t6, t5, .Lno_sv48

    # Continue at the link (upper-half) address of .Lhigh.
    ld t0, .Lhigh_addr
    jr t0
.balign 8
.Lhigh_addr:
    .dword .Lhigh
.Lhigh:
    # Now at the kernel's own address: drop the identity map.
    ld t0, veridian_kernel_satp
    csrw satp, t0
    sfence.vma
    la sp, __stack_top

    # Jump to Rust code (use `call` for 32-bit range; `jal` only reaches +/-1MB)
    call _start_rust

    # Hang if we return
1:
    wfi
    j 1b

# No Sv48: print "no Sv48" through SBI and stop (Sv39 is HR-09).
.Lno_sv48:
    la t0, .Lno_sv48_msg
7:
    lbu a0, 0(t0)
    beqz a0, 8f
    li a7, 0x01
    ecall
    addi t0, t0, 1
    j 7b
8:
    wfi
    j 8b
.Lno_sv48_msg:
    .asciz "no Sv48: cannot enable paging\n"
.balign 4

# A hart that lost the lottery asks SBI HSM to stop it (EID "HSM", FID 1
# hart_stop), so the boot hart can start it later as a secondary. It
# returns only if HSM is missing; then the hart waits here for good.
.Llost_lottery:
    li a7, 0x48534D
    li a6, 1
    ecall
5:
    wfi
    j 5b

# Secondary hart entry (SBI HSM hart_start): S-mode, satp = 0, sstatus.SIE =
# 0, a0 = hartid, a1 = &ApBootArgs (arch/smp_boot.rs). Takes the stack and
# the per-CPU block from the boot args, installs the trap vector, then
# enters Rust.
.global veridian_hart_entry
veridian_hart_entry:
    csrw sie, zero
    csrw sip, zero
    li t0, 0x6000
    csrc sstatus, t0
    # Entered at the physical address with the MMU off: the kernel's
    # tables first, then the upper-half code; a1 (the boot block) is a
    # kernel virtual address, used only after this.
    ld t0, veridian_trampoline_satp
    sfence.vma
    csrw satp, t0
    sfence.vma
    ld t0, .Lhart_high_addr
    jr t0
.balign 8
.Lhart_high_addr:
    .dword .Lhart_high
.Lhart_high:
    ld t0, veridian_kernel_satp
    csrw satp, t0
    sfence.vma
    ld sp, 8(a1)
    ld tp, 16(a1)
    csrw sscratch, zero
    la t0, veridian_riscv_trap
    csrw stvec, t0
    call veridian_secondary_rust
2:
    wfi
    j 2b