#!/bin/bash
# objdump helper script for VeridianOS kernel analysis

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
ARCH="x86_64"
MODE="disasm"
KERNEL_PATH=""

# Help function
show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -a, --arch ARCH     Target architecture (x86_64, aarch64, riscv64) [default: x86_64]"
    echo "  -m, --mode MODE     Display mode:"
    echo "                        disasm   - Disassembly (default)"
    echo "                        headers  - File headers"
    echo "                        sections - Section headers"
    echo "                        symbols  - Symbol table"
    echo "                        relocs   - Relocations"
    echo "                        all      - All information"
    echo "  -k, --kernel PATH   Path to kernel binary (optional)"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 -a x86_64 -m disasm"
    echo "  $0 -a aarch64 -m symbols"
    echo "  $0 -a riscv64 -m all -k target/riscv64gc-veridian/debug/veridian-kernel"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--arch)
            ARCH="$2"
            shift 2
            ;;
        -m|--mode)
            MODE="$2"
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

# Map architecture to target directory and objdump binary
case $ARCH in
    x86_64)
        TARGET_DIR="x86_64-veridian"
        OBJDUMP="objdump"
        ;;
    aarch64)
        TARGET_DIR="aarch64-veridian"
        OBJDUMP="aarch64-linux-gnu-objdump"
        ;;
    riscv64)
        TARGET_DIR="riscv64gc-veridian"
        OBJDUMP="riscv64-linux-gnu-objdump"
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

# Check if objdump is available
if ! command -v $OBJDUMP &> /dev/null; then
    echo -e "${YELLOW}Warning: $OBJDUMP not found, trying generic objdump${NC}"
    OBJDUMP="objdump"
fi

echo -e "${BLUE}Analyzing kernel: $KERNEL_PATH${NC}"
echo -e "${BLUE}Architecture: $ARCH${NC}"
echo -e "${BLUE}Mode: $MODE${NC}"
echo ""

# Execute objdump based on mode
case $MODE in
    disasm)
        echo -e "${GREEN}=== Disassembly ===${NC}"
        $OBJDUMP -d -M intel $KERNEL_PATH | less
        ;;
    headers)
        echo -e "${GREEN}=== File Headers ===${NC}"
        $OBJDUMP -f $KERNEL_PATH
        ;;
    sections)
        echo -e "${GREEN}=== Section Headers ===${NC}"
        $OBJDUMP -h $KERNEL_PATH
        ;;
    symbols)
        echo -e "${GREEN}=== Symbol Table ===${NC}"
        $OBJDUMP -t $KERNEL_PATH | grep -E "\.text|kernel_main|boot|panic" | sort
        ;;
    relocs)
        echo -e "${GREEN}=== Relocations ===${NC}"
        $OBJDUMP -r $KERNEL_PATH
        ;;
    all)
        echo -e "${GREEN}=== All Information ===${NC}"
        $OBJDUMP -x $KERNEL_PATH | less
        ;;
    *)
        echo -e "${RED}Error: Invalid mode '$MODE'${NC}"
        exit 1
        ;;
esac