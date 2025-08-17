.section .text.boot
.global _start

_start:
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

    # Jump to Rust code
    jal _start_rust

    # Hang if we return
1:
    wfi
    j 1b