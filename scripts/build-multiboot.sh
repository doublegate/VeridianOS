#!/bin/bash
# Build x86_64 kernel with multiboot2 support for GRUB

set -e

echo "Building VeridianOS x86_64 kernel with multiboot2 support..."

# Build the kernel with multiboot feature
cargo build --target x86_64-unknown-none -p veridian-kernel --features multiboot

echo "Kernel built successfully!"
echo ""
echo "To create GRUB ISO:"
echo "  ./scripts/create-grub-image.sh"
echo ""
echo "To test:"
echo "  qemu-system-x86_64 -cdrom veridian-os.iso -serial stdio -display none"