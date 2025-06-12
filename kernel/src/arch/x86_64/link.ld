/* Linker script for VeridianOS x86_64 kernel */

ENTRY(_start)

SECTIONS {
    /* Higher half kernel at -2GB */
    . = 0xFFFF800000100000;

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
    }
}