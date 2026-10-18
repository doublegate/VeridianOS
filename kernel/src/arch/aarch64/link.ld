ENTRY(_start)

SECTIONS
{
    /* 2 MiB into RAM (0x40000000 on QEMU virt), like the Linux arm64 text
     * offset. For a bare-metal ELF QEMU puts the device tree at the start of
     * RAM only if it fits below the image, and the virt DTB is 1 MiB, so the
     * old base of 0x40080000 (512 KiB) left the kernel without one. */
    . = 0x40200000;
    
    .text : ALIGN(4K) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }
    
    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
    }
    
    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
    }
    
    . = ALIGN(4K);
    .bss : {
        __bss_start = .;
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