.section ".text.boot"
.global _start

_start:
    // Write 'S' for start
    mov x0, #0x09000000
    mov w1, #83
    str w1, [x0]
    
    // Set up stack at a fixed location
    mov sp, #0x80000
    
    // Write 'T' for stack
    mov w1, #84
    str w1, [x0]
    
    // Clear BSS
    ldr x0, =__bss_start
    ldr x1, =__bss_end
    mov x2, #0x09000000  // Keep UART address in x2
1:
    cmp x0, x1
    b.eq 2f
    str xzr, [x0], #8
    b 1b
2:
    
    // Write 'B' for BSS cleared
    mov w1, #66
    str w1, [x2]
    
    // Write newline
    mov w1, #10
    str w1, [x2]
    
    // Call Rust main
    bl _start_rust
    
    // If we get here, write 'E' for error
    mov x0, #0x09000000
    mov w1, #69
    str w1, [x0]
    
    // Halt
1:
    wfe
    b 1b

.section .bss
.align 16
_stack_bottom:
    .space 0x4000
_stack_top: