ENTRY(_start)

SECTIONS
{
    /* QEMU loads the kernel at 0x40080000 for -machine virt */
    . = 0x40080000;
    
    .text : ALIGN(4K) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }
    
    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }
    
    .data : ALIGN(4K) {
        *(.data .data.*)
    }
    
    .bss : ALIGN(4K) {
        __bss_start = .;
        *(.bss.stack)
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }
    
    /* Discard unwanted sections */
    /DISCARD/ : {
        *(.comment)
        *(.gnu*)
        *(.note*)
        *(.eh_frame*)
    }
}