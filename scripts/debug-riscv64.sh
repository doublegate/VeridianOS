#!/bin/bash
# Debug script for the RISC-V 64 VeridianOS kernel: builds the dev kernel, starts
# QEMU paused with a GDB server on port 1234, and attaches GDB with
# scripts/gdb/riscv64.gdb (symbols from target/riscv64imac-unknown-none-elf/debug/veridian-kernel).
# GDB reads its commands from stdin, so a command file can be piped in.

set -e
cd "$(dirname "$0")/.."

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}VeridianOS RISC-V 64 Debug Script${NC}"

echo -e "${YELLOW}Building the dev kernel...${NC}"
./build-kernel.sh riscv64 dev

echo -e "${YELLOW}Starting QEMU (paused) with a GDB server on port 1234...${NC}"
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64imac-unknown-none-elf/debug/veridian-kernel \
    -serial file:target/debug-riscv64-serial.log -display none -monitor none \
    -s -S &
QEMU_PID=$!
trap 'kill $QEMU_PID 2>/dev/null || true; wait $QEMU_PID 2>/dev/null || true' EXIT

sleep 2
if ! kill -0 $QEMU_PID 2>/dev/null; then
    echo -e "${RED}QEMU failed to start${NC}"
    exit 1
fi

echo -e "${GREEN}QEMU started (PID $QEMU_PID); serial output: target/debug-riscv64-serial.log${NC}"
echo ""
echo "=== GDB Quick Reference ==="
echo "continue (c)     - Start/continue execution"
echo "break <symbol>   - Set breakpoint"
echo "next (n)         - Step over"
echo "step (s)         - Step into"
echo "info registers   - Show registers"
echo "x/<n><f> <addr>  - Examine memory"
echo ""
echo "=== RISC-V Specific ==="
echo "dcsr             - Dump CSRs"
echo "trap             - Analyze trap"
echo "==========================="
echo ""

if command -v gdb-multiarch >/dev/null 2>&1; then
    GDB_CMD=gdb-multiarch
else
    GDB_CMD=gdb
fi
$GDB_CMD -q -x scripts/gdb/riscv64.gdb

echo -e "${YELLOW}Debug session ended.${NC}"
