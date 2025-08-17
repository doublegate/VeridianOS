#!/bin/bash

echo "Testing AArch64 boot with detailed output..."
timeout 5 qemu-system-aarch64 -M virt -cpu cortex-a57 \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none 2>&1 | tail -50