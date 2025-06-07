# VeridianOS Kernel GDB Configuration
# Common debugging commands and settings for kernel development

# Set architecture based on target
define set-arch-x86_64
    set architecture i386:x86-64
    echo [GDB] Architecture set to x86_64\n
end

define set-arch-aarch64
    set architecture aarch64
    echo [GDB] Architecture set to AArch64\n
end

define set-arch-riscv64
    set architecture riscv:rv64
    echo [GDB] Architecture set to RISC-V 64\n
end

# Pretty printing for kernel structures
set print pretty on
set print array on
set print array-indexes on

# Pagination off for continuous output
set pagination off

# History settings
set history save on
set history size 10000
set history filename ~/.gdb_history

# Useful kernel debugging commands
define kernel-symbols
    # Load kernel symbols from the ELF file
    symbol-file target/$arg0/debug/veridian-kernel
    echo [GDB] Loaded symbols for $arg0\n
end

# Breakpoint helpers
define break-panic
    break panic_handler
    echo [GDB] Breakpoint set on panic handler\n
end

define break-main
    break kernel_main
    echo [GDB] Breakpoint set on kernel_main\n
end

define break-boot
    # Architecture-specific boot breakpoints
    if $argc == 0
        echo Usage: break-boot <arch>\n
    else
        if $arg0 == "x86_64"
            break _start
            echo [GDB] Breakpoint set on x86_64 _start\n
        end
        if $arg0 == "aarch64"
            break _start
            break _start_rust
            echo [GDB] Breakpoints set on AArch64 _start and _start_rust\n
        end
        if $arg0 == "riscv64"
            break _start
            echo [GDB] Breakpoint set on RISC-V _start\n
        end
    end
end

# Memory examination helpers
define examine-stack
    info registers rsp rbp
    x/32xg $rsp
    echo [GDB] Stack contents displayed\n
end

define examine-uart
    if $argc == 0
        echo Usage: examine-uart <arch>\n
    else
        if $arg0 == "x86_64"
            # COM1 port
            x/4xb 0x3f8
        end
        if $arg0 == "aarch64"
            # PL011 UART
            x/16xw 0x09000000
        end
        if $arg0 == "riscv64"
            # RISC-V UART
            x/16xw 0x10000000
        end
    end
end

# Kernel state inspection
define kernel-state
    echo === Kernel State ===\n
    info registers
    echo \n=== Current Stack ===\n
    where
    echo \n=== Memory Maps ===\n
    info mem
end

# Useful aliases
alias ks = kernel-symbols
alias bp = break-panic
alias bm = break-main
alias bb = break-boot
alias es = examine-stack
alias eu = examine-uart
alias kst = kernel-state

echo [GDB] VeridianOS kernel debugging script loaded\n
echo [GDB] Available commands: kernel-symbols, break-panic, break-main, break-boot\n
echo [GDB] Aliases: ks, bp, bm, bb, es, eu, kst\n