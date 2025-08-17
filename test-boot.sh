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
timeout 5 qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-unknown-none/debug/bootimage-veridian-kernel.bin \
    -serial stdio -display none 2>&1 | grep -E "Stage|BOOTOK" | head -10