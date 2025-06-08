#!/bin/bash
# readelf helper script for VeridianOS kernel ELF analysis

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
ARCH="x86_64"
MODE="headers"
KERNEL_PATH=""

# Help function
show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -a, --arch ARCH     Target architecture (x86_64, aarch64, riscv64) [default: x86_64]"
    echo "  -m, --mode MODE     Display mode:"
    echo "                        headers  - ELF file header (default)"
    echo "                        sections - Section headers"
    echo "                        segments - Program headers"
    echo "                        symbols  - Symbol table"
    echo "                        dynamic  - Dynamic section"
    echo "                        notes    - Note sections"
    echo "                        all      - All information"
    echo "  -k, --kernel PATH   Path to kernel binary (optional)"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 -a x86_64 -m headers"
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

# Map architecture to target directory and readelf binary
case $ARCH in
    x86_64)
        TARGET_DIR="x86_64-veridian"
        READELF="readelf"
        ;;
    aarch64)
        TARGET_DIR="aarch64-veridian"
        READELF="aarch64-linux-gnu-readelf"
        ;;
    riscv64)
        TARGET_DIR="riscv64gc-veridian"
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

# Check if readelf is available
if ! command -v $READELF &> /dev/null; then
    echo -e "${YELLOW}Warning: $READELF not found, trying generic readelf${NC}"
    READELF="readelf"
fi

echo -e "${BLUE}Analyzing kernel: $KERNEL_PATH${NC}"
echo -e "${BLUE}Architecture: $ARCH${NC}"
echo -e "${BLUE}Mode: $MODE${NC}"
echo ""

# Execute readelf based on mode
case $MODE in
    headers)
        echo -e "${GREEN}=== ELF File Header ===${NC}"
        $READELF -h $KERNEL_PATH
        ;;
    sections)
        echo -e "${GREEN}=== Section Headers ===${NC}"
        $READELF -S $KERNEL_PATH
        echo ""
        echo -e "${GREEN}=== Section to Segment Mapping ===${NC}"
        $READELF -l $KERNEL_PATH | grep -A20 "Section to Segment"
        ;;
    segments)
        echo -e "${GREEN}=== Program Headers ===${NC}"
        $READELF -l $KERNEL_PATH
        ;;
    symbols)
        echo -e "${GREEN}=== Symbol Table ===${NC}"
        $READELF -s $KERNEL_PATH | grep -E "FUNC|OBJECT" | grep -v "UND" | sort -k 2
        ;;
    dynamic)
        echo -e "${GREEN}=== Dynamic Section ===${NC}"
        $READELF -d $KERNEL_PATH 2>/dev/null || echo "No dynamic section (static binary)"
        ;;
    notes)
        echo -e "${GREEN}=== Note Sections ===${NC}"
        $READELF -n $KERNEL_PATH 2>/dev/null || echo "No note sections found"
        ;;
    all)
        echo -e "${GREEN}=== All ELF Information ===${NC}"
        $READELF -a $KERNEL_PATH | less
        ;;
    *)
        echo -e "${RED}Error: Invalid mode '$MODE'${NC}"
        exit 1
        ;;
esac