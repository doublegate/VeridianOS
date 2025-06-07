# x86_64-specific GDB configuration for VeridianOS

# Load common kernel debugging commands
source scripts/gdb/kernel.gdb

# Set architecture
set-arch-x86_64

# x86_64 specific settings
set disassembly-flavor intel

# Connect to QEMU gdbserver
target remote localhost:1234

# Load kernel symbols
kernel-symbols x86_64-veridian

# x86_64 specific commands
define dump-gdt
    echo === Global Descriptor Table ===\n
    # Assuming GDT location from kernel
    x/8gx 0x0
end

define dump-idt
    echo === Interrupt Descriptor Table ===\n
    # IDT entries
    x/32gx 0x0
end

define dump-cr
    echo === Control Registers ===\n
    info registers cr0 cr2 cr3 cr4
end

define dump-vga
    echo === VGA Text Buffer (First Line) ===\n
    x/80hx 0xb8000
end

# Page table walking for x86_64
define walk-page-table
    if $argc != 1
        echo Usage: walk-page-table <virtual_address>\n
    else
        set $vaddr = $arg0
        set $cr3 = $cr3
        
        # Extract indices
        set $pml4_idx = ($vaddr >> 39) & 0x1ff
        set $pdp_idx = ($vaddr >> 30) & 0x1ff
        set $pd_idx = ($vaddr >> 21) & 0x1ff
        set $pt_idx = ($vaddr >> 12) & 0x1ff
        set $offset = $vaddr & 0xfff
        
        echo Virtual Address: 
        printf "0x%016lx\n", $vaddr
        echo PML4 Index: 
        printf "%d\n", $pml4_idx
        echo PDP Index: 
        printf "%d\n", $pdp_idx
        echo PD Index: 
        printf "%d\n", $pd_idx
        echo PT Index: 
        printf "%d\n", $pt_idx
        echo Offset: 
        printf "0x%03x\n", $offset
    end
end

# Aliases for x86_64 specific commands
alias dgdt = dump-gdt
alias didt = dump-idt
alias dcr = dump-cr
alias dvga = dump-vga
alias wpt = walk-page-table

echo [GDB] x86_64-specific configuration loaded\n
echo [GDB] Additional commands: dump-gdt, dump-idt, dump-cr, dump-vga, walk-page-table\n

# Set initial breakpoints
break-boot x86_64
break-panic

echo [GDB] Ready to debug x86_64 kernel. Use 'continue' to start.\n