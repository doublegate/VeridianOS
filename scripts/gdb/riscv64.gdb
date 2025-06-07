# RISC-V 64-specific GDB configuration for VeridianOS

# Load common kernel debugging commands
source scripts/gdb/kernel.gdb

# Set architecture
set-arch-riscv64

# RISC-V specific settings
set riscv use-compressed-breakpoints on

# Connect to QEMU gdbserver
target remote localhost:1234

# Load kernel symbols
kernel-symbols riscv64gc-veridian

# RISC-V specific commands
define dump-regs
    echo === RISC-V General Purpose Registers ===\n
    info registers zero ra sp gp tp t0 t1 t2
    info registers s0 s1 a0 a1 a2 a3 a4 a5
    info registers a6 a7 s2 s3 s4 s5 s6 s7
    info registers s8 s9 s10 s11 t3 t4 t5 t6
    echo === Program Counter ===\n
    info registers pc
end

define dump-csr
    echo === RISC-V Control and Status Registers ===\n
    # Machine mode CSRs
    echo mstatus: 
    p/x $mstatus
    echo mtvec: 
    p/x $mtvec
    echo mepc: 
    p/x $mepc
    echo mcause: 
    p/x $mcause
    echo mtval: 
    p/x $mtval
    echo mhartid: 
    p/x $mhartid
end

define dump-uart-8250
    echo === 8250 UART Registers ===\n
    set $uart_base = 0x10000000
    echo RBR/THR (Receive/Transmit): 
    x/1xb $uart_base
    echo IER (Interrupt Enable): 
    x/1xb ($uart_base + 1)
    echo IIR/FCR (Interrupt ID/FIFO Control): 
    x/1xb ($uart_base + 2)
    echo LCR (Line Control): 
    x/1xb ($uart_base + 3)
    echo MCR (Modem Control): 
    x/1xb ($uart_base + 4)
    echo LSR (Line Status): 
    x/1xb ($uart_base + 5)
    echo MSR (Modem Status): 
    x/1xb ($uart_base + 6)
end

define examine-opensbi
    echo === OpenSBI Region ===\n
    # OpenSBI typically loads at 0x80000000
    echo OpenSBI Entry (0x80000000):\n
    x/16i 0x80000000
    echo \nKernel Load Address (0x80200000):\n
    x/16i 0x80200000
end

# Page table walk for RISC-V Sv39
define walk-page-table-sv39
    if $argc != 1
        echo Usage: walk-page-table-sv39 <virtual_address>\n
    else
        set $vaddr = $arg0
        echo Virtual Address: 
        printf "0x%016lx\n", $vaddr
        
        # Sv39: 3-level page table
        set $vpn2 = ($vaddr >> 30) & 0x1ff
        set $vpn1 = ($vaddr >> 21) & 0x1ff
        set $vpn0 = ($vaddr >> 12) & 0x1ff
        set $offset = $vaddr & 0xfff
        
        echo VPN[2]: 
        printf "%d\n", $vpn2
        echo VPN[1]: 
        printf "%d\n", $vpn1
        echo VPN[0]: 
        printf "%d\n", $vpn0
        echo Page Offset: 
        printf "0x%03x\n", $offset
    end
end

# Trap debugging
define analyze-trap
    echo === RISC-V Trap Analysis ===\n
    echo mcause: 
    p/x $mcause
    set $is_interrupt = ($mcause >> 63) & 1
    set $exception_code = $mcause & 0x7fffffffffffffff
    
    if $is_interrupt
        echo Type: Interrupt\n
        if $exception_code == 3
            echo Cause: Machine software interrupt\n
        end
        if $exception_code == 7
            echo Cause: Machine timer interrupt\n
        end
        if $exception_code == 11
            echo Cause: Machine external interrupt\n
        end
    else
        echo Type: Exception\n
        if $exception_code == 0
            echo Cause: Instruction address misaligned\n
        end
        if $exception_code == 2
            echo Cause: Illegal instruction\n
        end
        if $exception_code == 5
            echo Cause: Load access fault\n
        end
        if $exception_code == 7
            echo Cause: Store/AMO access fault\n
        end
    end
    
    echo mtval (trap value): 
    p/x $mtval
    echo mepc (exception PC): 
    p/x $mepc
end

# Aliases for RISC-V specific commands
alias dr = dump-regs
alias dcsr = dump-csr
alias duart = dump-uart-8250
alias esbi = examine-opensbi
alias wpt39 = walk-page-table-sv39
alias trap = analyze-trap

echo [GDB] RISC-V 64-specific configuration loaded\n
echo [GDB] Additional commands: dump-regs, dump-csr, dump-uart-8250, examine-opensbi, analyze-trap\n

# Set initial breakpoints
break-boot riscv64
break-panic

echo [GDB] Ready to debug RISC-V kernel. Use 'continue' to start.\n