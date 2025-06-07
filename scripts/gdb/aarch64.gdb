# AArch64-specific GDB configuration for VeridianOS

# Load common kernel debugging commands
source scripts/gdb/kernel.gdb

# Set architecture
set-arch-aarch64

# AArch64 specific settings
set arm abi AAPCS64

# Connect to QEMU gdbserver
target remote localhost:1234

# Load kernel symbols
kernel-symbols aarch64-veridian

# AArch64 specific commands
define dump-regs
    echo === AArch64 General Purpose Registers ===\n
    info registers x0 x1 x2 x3 x4 x5 x6 x7
    info registers x8 x9 x10 x11 x12 x13 x14 x15
    info registers x16 x17 x18 x19 x20 x21 x22 x23
    info registers x24 x25 x26 x27 x28 x29 x30 sp
    echo === Special Registers ===\n
    info registers pc cpsr
end

define dump-system-regs
    echo === System Registers ===\n
    # Current EL
    p/x $CurrentEL
    # Stack pointer for EL1
    p/x $SP_EL1
    # Exception Link Register
    p/x $ELR_EL1
    # Saved Program Status Register
    p/x $SPSR_EL1
end

define dump-uart-pl011
    echo === PL011 UART Registers ===\n
    set $uart_base = 0x09000000
    echo DR (Data Register): 
    x/1xw $uart_base
    echo FR (Flag Register): 
    x/1xw ($uart_base + 0x18)
    echo IBRD (Integer Baud Rate): 
    x/1xw ($uart_base + 0x24)
    echo FBRD (Fractional Baud Rate): 
    x/1xw ($uart_base + 0x28)
    echo LCR_H (Line Control): 
    x/1xw ($uart_base + 0x2c)
    echo CR (Control Register): 
    x/1xw ($uart_base + 0x30)
end

define examine-boot-area
    echo === Boot Area Memory ===\n
    echo Kernel Load Address (0x40080000):\n
    x/16i 0x40080000
    echo \nStack Area (0x80000):\n
    x/8gx 0x80000
end

# Translation table walk for AArch64
define walk-page-table-aa64
    if $argc != 1
        echo Usage: walk-page-table-aa64 <virtual_address>\n
    else
        set $vaddr = $arg0
        echo Virtual Address: 
        printf "0x%016lx\n", $vaddr
        
        # 4KB pages, 4-level translation
        set $l0_idx = ($vaddr >> 39) & 0x1ff
        set $l1_idx = ($vaddr >> 30) & 0x1ff
        set $l2_idx = ($vaddr >> 21) & 0x1ff
        set $l3_idx = ($vaddr >> 12) & 0x1ff
        set $offset = $vaddr & 0xfff
        
        echo Level 0 Index: 
        printf "%d\n", $l0_idx
        echo Level 1 Index: 
        printf "%d\n", $l1_idx
        echo Level 2 Index: 
        printf "%d\n", $l2_idx
        echo Level 3 Index: 
        printf "%d\n", $l3_idx
        echo Page Offset: 
        printf "0x%03x\n", $offset
    end
end

# Aliases for AArch64 specific commands
alias dr = dump-regs
alias dsr = dump-system-regs
alias duart = dump-uart-pl011
alias eba = examine-boot-area
alias wpt64 = walk-page-table-aa64

echo [GDB] AArch64-specific configuration loaded\n
echo [GDB] Additional commands: dump-regs, dump-system-regs, dump-uart-pl011, examine-boot-area\n

# Set initial breakpoints
break-boot aarch64
break-panic

# Also break on our known working entry points
break *0x40080000
break _start_rust

echo [GDB] Ready to debug AArch64 kernel. Use 'continue' to start.\n