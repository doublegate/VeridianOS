#!/bin/bash
# Debug script for the AArch64 VeridianOS kernel: builds the dev kernel, starts
# QEMU paused with a GDB server on port 1234, and attaches GDB with
# scripts/gdb/aarch64.gdb (symbols from target/aarch64-unknown-none-softfloat/debug/veridian-kernel).
# GDB reads its commands from stdin, so a command file can be piped in.

set -e
cd "$(dirname "$0")/.."

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}VeridianOS AArch64 Debug Script${NC}"

echo -e "${YELLOW}Building the dev kernel...${NC}"
# stdin is GDB's (a piped command file), so the build must not read it.
./build-kernel.sh aarch64 dev </dev/null

echo -e "${YELLOW}Starting QEMU (paused) with a GDB server on port 1234...${NC}"
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none-softfloat/debug/veridian-kernel \
    -serial file:target/debug-aarch64-serial.log -display none -monitor none \
    -s -S &
QEMU_PID=$!
trap 'kill $QEMU_PID 2>/dev/null || true; wait $QEMU_PID 2>/dev/null || true' EXIT

sleep 2
if ! kill -0 $QEMU_PID 2>/dev/null; then
    echo -e "${RED}QEMU failed to start${NC}"
    exit 1
fi

echo -e "${GREEN}QEMU started (PID $QEMU_PID); serial output: target/debug-aarch64-serial.log${NC}"
echo ""
echo "=== GDB Quick Reference ==="
echo "continue (c)     - Start/continue execution"
echo "break <symbol>   - Set breakpoint"
echo "next (n)         - Step over"
echo "step (s)         - Step into"
echo "info registers   - Show registers"
echo "x/<n><f> <addr>  - Examine memory"
echo ""
echo "=== AArch64 Specific ==="
echo "dr               - Dump all registers"
echo "duart            - Dump UART registers"
echo "eba              - Examine boot area"
echo "==========================="
echo ""

if command -v gdb-multiarch >/dev/null 2>&1; then
    GDB_CMD=gdb-multiarch
else
    GDB_CMD=gdb
fi
$GDB_CMD -q -x scripts/gdb/aarch64.gdb

echo -e "${YELLOW}Debug session ended.${NC}"
