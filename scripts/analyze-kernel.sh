#!/bin/bash
# Combined kernel analysis script for VeridianOS

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Default values
ARCH="x86_64"
KERNEL_PATH=""

# Help function
show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Comprehensive kernel binary analysis tool for VeridianOS"
    echo ""
    echo "Options:"
    echo "  -a, --arch ARCH     Target architecture (x86_64, aarch64, riscv64) [default: x86_64]"
    echo "  -k, --kernel PATH   Path to kernel binary (optional)"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "This script provides a comprehensive analysis including:"
    echo "  - Binary size and section breakdown"
    echo "  - Entry point and key symbols"
    echo "  - Memory layout"
    echo "  - Boot sequence analysis"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            ARCH="$2"
            shift 2
            ;;
        -k|--kernel)
            KERNEL_PATH="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo -e "${RED}Error: Unknown option $1${NC}"
            show_help
            exit 1
            ;;
    esac
done

# Validate architecture
case $ARCH in
    x86_64|aarch64|riscv64)
        ;;
    *)
        echo -e "${RED}Error: Invalid architecture '$ARCH'${NC}"
        echo "Valid architectures: x86_64, aarch64, riscv64"
        exit 1
        ;;
esac

# Map architecture to target directory
case $ARCH in
    x86_64)
        TARGET_DIR="x86_64-veridian"
        OBJDUMP="objdump"
        READELF="readelf"
        ;;
    aarch64)
        TARGET_DIR="aarch64-veridian"
        OBJDUMP="aarch64-linux-gnu-objdump"
        READELF="aarch64-linux-gnu-readelf"
        ;;
    riscv64)
        TARGET_DIR="riscv64gc-veridian"
        OBJDUMP="riscv64-linux-gnu-objdump"
        READELF="riscv64-linux-gnu-readelf"
        ;;
esac

# Determine kernel path if not provided
if [ -z "$KERNEL_PATH" ]; then
    KERNEL_PATH="target/$TARGET_DIR/debug/veridian-kernel"
fi

# Check if kernel binary exists
if [ ! -f "$KERNEL_PATH" ]; then
    echo -e "${RED}Error: Kernel binary not found at $KERNEL_PATH${NC}"
    echo "Please build the kernel first with: cargo build --target targets/$TARGET_DIR.json"
    exit 1
fi

# Fallback to generic tools if architecture-specific ones aren't available
if ! command -v $OBJDUMP &> /dev/null; then
    OBJDUMP="objdump"
fi
if ! command -v $READELF &> /dev/null; then
    READELF="readelf"
fi

echo -e "${CYAN}=== VeridianOS Kernel Analysis ===${NC}"
echo -e "${BLUE}Architecture: $ARCH${NC}"
echo -e "${BLUE}Kernel Path: $KERNEL_PATH${NC}"
echo ""

# File information
echo -e "${GREEN}=== File Information ===${NC}"
file $KERNEL_PATH
echo ""
ls -lh $KERNEL_PATH
echo ""

# Size breakdown
echo -e "${GREEN}=== Size Breakdown ===${NC}"
size $KERNEL_PATH 2>/dev/null || $OBJDUMP -h $KERNEL_PATH | grep -E "\.text|\.data|\.bss|\.rodata" | awk '{printf "%-15s %8s bytes\n", $2, $3}'
echo ""

# Entry point
echo -e "${GREEN}=== Entry Point ===${NC}"
$READELF -h $KERNEL_PATH | grep "Entry point"
echo ""

# Key symbols
echo -e "${GREEN}=== Key Kernel Symbols ===${NC}"
$OBJDUMP -t $KERNEL_PATH | grep -E "kernel_main|_start|boot|panic_handler|rust_begin_unwind" | sort -k 1 | head -20
echo ""

# Section layout
echo -e "${GREEN}=== Memory Layout ===${NC}"
$READELF -S $KERNEL_PATH | grep -E "\.text|\.data|\.bss|\.rodata|\.got|\.boot" | awk '{printf "%-20s %s - %s (%s bytes)\n", $2, $4, $4+$6, $6}'
echo ""

# Program headers
echo -e "${GREEN}=== Program Headers (Load Segments) ===${NC}"
$READELF -l $KERNEL_PATH | grep -E "LOAD|Entry"
echo ""

# Architecture-specific analysis
case $ARCH in
    x86_64)
        echo -e "${GREEN}=== x86_64 Specific Analysis ===${NC}"
        echo "GDT/IDT handlers:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "gdt|idt|tss" | head -10
        echo ""
        echo "Interrupt handlers:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "interrupt|exception" | head -10
        ;;
    aarch64)
        echo -e "${GREEN}=== AArch64 Specific Analysis ===${NC}"
        echo "Boot sequence symbols:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "_start|boot|el[0-3]" | head -10
        echo ""
        echo "Exception vectors:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "vector|exception" | head -10
        ;;
    riscv64)
        echo -e "${GREEN}=== RISC-V Specific Analysis ===${NC}"
        echo "Boot and trap symbols:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "_start|boot|trap" | head -10
        echo ""
        echo "M-mode/S-mode symbols:"
        $OBJDUMP -t $KERNEL_PATH | grep -E "mret|sret|[ms]status" | head -10
        ;;
esac

echo ""
echo -e "${CYAN}=== Analysis Complete ===${NC}"
echo ""
echo "For more detailed analysis, use:"
echo "  ./scripts/objdump-kernel.sh -a $ARCH -m <mode>"
echo "  ./scripts/readelf-kernel.sh -a $ARCH -m <mode>"