#!/bin/bash
# Debug script for AArch64 VeridianOS kernel

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}VeridianOS AArch64 Debug Script${NC}"
echo "==================================="

# Build the kernel with debug info
echo -e "${YELLOW}Building kernel with debug symbols...${NC}"
cargo build --target targets/aarch64-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# Check if build succeeded
if [ $? -ne 0 ]; then
    echo -e "${RED}Build failed!${NC}"
    exit 1
fi

echo -e "${GREEN}Build successful!${NC}"

# Start QEMU in the background with GDB server
echo -e "${YELLOW}Starting QEMU with GDB server on port 1234...${NC}"
qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a53 \
    -nographic \
    -kernel target/aarch64-veridian/debug/veridian-kernel \
    -serial mon:stdio \
    -s -S &

QEMU_PID=$!

# Give QEMU time to start
sleep 2

# Check if QEMU is running
if ! ps -p $QEMU_PID > /dev/null; then
    echo -e "${RED}QEMU failed to start!${NC}"
    exit 1
fi

echo -e "${GREEN}QEMU started (PID: $QEMU_PID)${NC}"
echo -e "${YELLOW}Starting GDB...${NC}"
echo ""
echo "=== GDB Quick Reference ==="
echo "continue (c)     - Start/continue execution"
echo "break <symbol>   - Set breakpoint"
echo "next (n)         - Step over"
echo "step (s)         - Step into"
echo "info registers   - Show registers"
echo "x/<n><f> <addr> - Examine memory"
echo ""
echo "=== AArch64 Specific ==="
echo "dr               - Dump all registers"
echo "duart            - Dump UART registers"
echo "eba              - Examine boot area"
echo "==========================="
echo ""

# Use gdb-multiarch if available, otherwise regular gdb
if command -v gdb-multiarch &> /dev/null; then
    GDB_CMD="gdb-multiarch"
else
    GDB_CMD="gdb"
fi

# Start GDB with our AArch64 configuration
$GDB_CMD -x scripts/gdb/aarch64.gdb

# When GDB exits, kill QEMU
echo -e "${YELLOW}Cleaning up...${NC}"
kill $QEMU_PID 2>/dev/null || true
wait $QEMU_PID 2>/dev/null || true

echo -e "${GREEN}Debug session ended.${NC}"