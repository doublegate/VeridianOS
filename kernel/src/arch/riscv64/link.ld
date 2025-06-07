ENTRY(_start)

SECTIONS
{
    . = 0x80200000;
    
    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }
    
    .rodata : {
        *(.rodata .rodata.*)
    }
    
    .data : {
        *(.data .data.*)
    }
    
    .bss : {
        . = ALIGN(16);
        __bss_start = .;
        *(.bss .bss.*)
        __bss_end = .;
    }
    
    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}