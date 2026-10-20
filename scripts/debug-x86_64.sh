#!/bin/bash
# Debug script for the x86_64 VeridianOS kernel: builds the dev kernel, starts
# QEMU paused with a GDB server on port 1234, and attaches GDB with
# scripts/gdb/x86_64.gdb (symbols from target/x86_64-veridian/debug/veridian-kernel).
# GDB reads its commands from stdin, so a command file can be piped in.

set -e
cd "$(dirname "$0")/.."

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${GREEN}VeridianOS x86_64 Debug Script${NC}"

echo -e "${YELLOW}Building the dev kernel...${NC}"
./build-kernel.sh x86_64 dev

echo -e "${YELLOW}Starting QEMU (paused) with a GDB server on port 1234...${NC}"
qemu-system-x86_64 -enable-kvm -m 256M \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -serial file:target/debug-x86_64-serial.log -display none -monitor none \
    -s -S &
QEMU_PID=$!
trap 'kill $QEMU_PID 2>/dev/null || true; wait $QEMU_PID 2>/dev/null || true' EXIT

sleep 2
if ! kill -0 $QEMU_PID 2>/dev/null; then
    echo -e "${RED}QEMU failed to start${NC}"
    exit 1
fi

echo -e "${GREEN}QEMU started (PID $QEMU_PID); serial output: target/debug-x86_64-serial.log${NC}"
echo ""
echo "=== GDB Quick Reference ==="
echo "continue (c)     - Start/continue execution"
echo "break <symbol>   - Set breakpoint"
echo "next (n)         - Step over"
echo "step (s)         - Step into"
echo "info registers   - Show registers"
echo "x/<n><f> <addr>  - Examine memory"
echo ""
echo "Under KVM, use hbreak (hardware breakpoints) for code the bootloader has"
echo "not loaded yet: a software breakpoint set before then is overwritten."
echo "==========================="
echo ""

GDB_CMD=gdb
$GDB_CMD -q -x scripts/gdb/x86_64.gdb

echo -e "${YELLOW}Debug session ended.${NC}"
