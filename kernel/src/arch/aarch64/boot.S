.section .text.boot
.global _start
.extern _start_rust

_start:
    // Set up stack pointer
    adr x0, _stack_top
    mov sp, x0

    // Clear BSS
    adr x0, __bss_start
    adr x1, __bss_end
    sub x2, x1, x0
    cbz x2, 2f
1:
    str xzr, [x0], #8
    subs x2, x2, #8
    b.ne 1b
2:

    // Write 'A' to UART for debug
    mov x0, #0x09000000
    mov w1, #65
    str w1, [x0]
    
    // Write 'B' before branch
    mov w1, #66
    str w1, [x0]

    // Branch to test function first
    bl test_func
    
    // Write 'C' 
    mov w1, #67
    str w1, [x0]
    
    // Branch to Rust code using BL
    bl _start_rust

test_func:
    // Write 'T' to show we can call functions
    mov x0, #0x09000000
    mov w1, #84  // 'T'
    str w1, [x0]
    ret

.section .bss
.align 16
_stack:
    .space 0x10000
_stack_top: