ENTRY(_start)

/* The kernel runs in the upper half (sprint E, HR-02): linked at
 * KERNEL_VMA, loaded by OpenSBI/QEMU at KERNEL_LMA (they load each segment
 * at its physical address and enter at the lowest one, so `_start` must
 * stay first). boot.S starts at the physical address with the MMU off,
 * maps both, turns paging on and jumps up. */
KERNEL_LMA = 0x80200000;
KERNEL_VMA = 0xFFFFFFFF80200000;
KERNEL_OFFSET = KERNEL_VMA - KERNEL_LMA;

SECTIONS
{
    . = KERNEL_VMA;

    .text : AT(ADDR(.text) - KERNEL_OFFSET) ALIGN(4K) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    . = ALIGN(4K);
    .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    . = ALIGN(4K);
    .data : AT(ADDR(.data) - KERNEL_OFFSET) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    . = ALIGN(4K);
    .bss : AT(ADDR(.bss) - KERNEL_OFFSET) {
        __bss_start = .;
        *(.bss.stack)
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }

    /* Stack grows downward, place it after BSS */
    . = ALIGN(16K);
    __stack_bottom = .;
    . = . + 0x100000; /* 1MB stack (128KB overflowed into .bss: N-13) */
    __stack_top = .;

    /* Kernel end marker (a virtual address: subtract KERNEL_OFFSET for
     * the physical one) */
    . = ALIGN(4K);
    __kernel_end = .;

    /* Discard unwanted sections */
    /DISCARD/ : {
        *(.comment)
        *(.gnu*)
        *(.note*)
        *(.eh_frame*)
        *(.riscv.attributes)
    }
}
