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
#
# GDB cannot compare string arguments on a bare-metal target (a string literal
# needs malloc in the inferior), so the architecture-specific helpers are
# separate commands rather than one command taking the architecture name.
#
# Every panic goes through core::panicking::panic_fmt.
define break-panic
    break core::panicking::panic_fmt
    echo [GDB] Breakpoint set on core::panicking::panic_fmt\n
end

# Hardware variant, for x86_64 under UEFI: the bootloader loads the kernel
# after GDB attaches and overwrites software breakpoints set before then.
define hbreak-panic
    hbreak core::panicking::panic_fmt
    echo [GDB] Hardware breakpoint set on core::panicking::panic_fmt\n
end

define break-main
    break veridian_kernel::bootstrap::kernel_init
    echo [GDB] Breakpoint set on veridian_kernel::bootstrap::kernel_init\n
end

define break-boot
    echo Usage: break-boot-x86_64, break-boot-aarch64 or break-boot-riscv64\n
end

define break-boot-x86_64
    hbreak _start
    echo [GDB] Hardware breakpoint set on x86_64 _start\n
end

define break-boot-aarch64
    break _start
    break _start_rust
    echo [GDB] Breakpoints set on AArch64 _start and _start_rust\n
end

define break-boot-riscv64
    break _start
    break _start_rust
    echo [GDB] Breakpoints set on RISC-V _start and _start_rust\n
end

# Memory examination helpers
define examine-stack
    p $sp
    x/32xg $sp
    echo [GDB] Stack contents displayed\n
end

define examine-uart
    echo Usage: examine-uart-aarch64 or examine-uart-riscv64 (x86_64's COM1 is an I/O port)\n
end

define examine-uart-aarch64
    # PL011 UART
    x/16xw 0x09000000
end

define examine-uart-riscv64
    # 16550 UART
    x/16xw 0x10000000
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
echo [GDB] Available commands: kernel-symbols, break-panic, break-main, break-boot-<arch>, examine-stack, examine-uart-<arch>\n
echo [GDB] Aliases: ks, bp, bm, bb, es, eu, kst\n