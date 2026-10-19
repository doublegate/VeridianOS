/* The kernel is linked in the upper half (TTBR1) and loaded 2 MiB into RAM
 * (ADR 0015): 0x40200000 on QEMU virt, like the Linux arm64 text offset.
 * For a bare-metal ELF QEMU puts the device tree at the start of RAM only
 * if it fits below the image, and the virt DTB is 1 MiB, so the old base of
 * 0x40080000 (512 KiB) left the kernel without one. boot.S runs at the load
 * address until it turns the MMU on, and measures the real load address,
 * so an arm64 Image also runs from any other 2 MiB-aligned base. */
KERNEL_VMA = 0xFFFFFFFF80200000;
KERNEL_LMA = 0x40200000;

/* QEMU enters a bare ELF at e_entry with the MMU off: the physical address
 * of _start. */
veridian_phys_start = KERNEL_LMA;
ENTRY(veridian_phys_start)

SECTIONS
{
    . = KERNEL_VMA;

    .text : AT(KERNEL_LMA) ALIGN(4K) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    /* Section bounds for the image's page permissions (W^X): code is
     * [_start, __text_end), read-only data up to __rodata_end, the rest
     * (data, BSS, boot stack) writable and not executable. */
    . = ALIGN(4K);
    __text_end = .;
    .rodata : AT(ADDR(.rodata) - KERNEL_VMA + KERNEL_LMA) {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4K);
    __rodata_end = .;
    .data : AT(ADDR(.data) - KERNEL_VMA + KERNEL_LMA) {
        *(.data .data.*)
    }

    . = ALIGN(4K);
    .bss (NOLOAD) : AT(ADDR(.bss) - KERNEL_VMA + KERNEL_LMA) {
        __bss_start = .;
        /* The boot translation tables (boot.S), page aligned, first so
         * that their alignment costs nothing. */
        . = ALIGN(4K);
        *(.bss.pgtables)
        *(.bss.stack)
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }

    /* Stack grows downward, place it after BSS */
    . = ALIGN(16K);
    __stack_bottom = .;
    . = . + 0x100000; /* 1MB stack */
    __stack_top = .;

    /* Kernel end marker */
    . = ALIGN(4K);
    __kernel_end = .;
    /* The Image header's image_size: what a loader must leave free from
     * the load address, BSS and the boot stack included (HA-03). */
    __veridian_image_size = __kernel_end - _start;

    /* Discard unwanted sections */
    /DISCARD/ : {
        *(.comment)
        *(.gnu*)
        *(.note*)
        *(.eh_frame*)
        *(.ARM.attributes)
        *(.ARM.exidx*)
    }
}
