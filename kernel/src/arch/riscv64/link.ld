ENTRY(_start)

SECTIONS
{
    /* OpenSBI loads kernel at 0x80200000 */
    . = 0x80200000;
    
    .text : ALIGN(4K) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }
    
    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    
    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    
    . = ALIGN(4K);
    .bss : {
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
    . = . + 0x20000; /* 128KB stack */
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
        *(.riscv.attributes)
    }
}