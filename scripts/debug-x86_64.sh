#!/bin/bash
# Debug script for x86_64 VeridianOS kernel

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}VeridianOS x86_64 Debug Script${NC}"
echo "================================"

# Build the kernel with debug info
echo -e "${YELLOW}Building kernel with debug symbols...${NC}"
cargo build --target targets/x86_64-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# Check if build succeeded
if [ $? -ne 0 ]; then
    echo -e "${RED}Build failed!${NC}"
    exit 1
fi

echo -e "${GREEN}Build successful!${NC}"

# Build bootimage for x86_64
echo -e "${YELLOW}Creating bootimage...${NC}"
cargo bootimage --target targets/x86_64-veridian.json

if [ $? -ne 0 ]; then
    echo -e "${RED}Bootimage creation failed!${NC}"
    exit 1
fi

# Start QEMU in the background with GDB server
echo -e "${YELLOW}Starting QEMU with GDB server on port 1234...${NC}"
qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-veridian/debug/bootimage-veridian-kernel.bin \
    -serial stdio \
    -display none \
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
echo "==========================="
echo ""

# Start GDB with our x86_64 configuration
gdb -x scripts/gdb/x86_64.gdb

# When GDB exits, kill QEMU
echo -e "${YELLOW}Cleaning up...${NC}"
kill $QEMU_PID 2>/dev/null || true
wait $QEMU_PID 2>/dev/null || true

echo -e "${GREEN}Debug session ended.${NC}"