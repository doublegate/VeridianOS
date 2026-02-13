#!/bin/bash

# Test boot for all architectures

echo "Testing AArch64 boot..."
timeout 5 qemu-system-aarch64 -M virt -cpu cortex-a57 \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none 2>&1 | grep -E "Stage|BOOTOK" | head -10

echo ""
echo "Testing RISC-V boot..."
timeout 5 qemu-system-riscv64 -M virt \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none 2>&1 | grep -E "Stage|BOOTOK" | head -10

echo ""
echo "Testing x86_64 boot..."
# Note: Using veridian-bios.img from bootloader 0.11+ (or legacy bootimage path as fallback)
BIOS_IMAGE="target/x86_64-veridian/debug/veridian-bios.img"
LEGACY_IMAGE="target/x86_64-unknown-none/debug/bootimage-veridian-kernel.bin"

if [ -f "$BIOS_IMAGE" ]; then
    timeout 5 qemu-system-x86_64 \
        -drive format=raw,file="$BIOS_IMAGE" \
        -serial stdio -display none 2>&1 | grep -E "Stage|BOOTOK" | head -10
elif [ -f "$LEGACY_IMAGE" ]; then
    timeout 5 qemu-system-x86_64 \
        -drive format=raw,file="$LEGACY_IMAGE" \
        -serial stdio -display none 2>&1 | grep -E "Stage|BOOTOK" | head -10
else
    echo "No x86_64 bootable image found. Run './build-kernel.sh x86_64' first."
fi