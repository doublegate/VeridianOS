#!/bin/bash
# Create GRUB-bootable ISO image for x86_64 VeridianOS kernel

set -e

KERNEL_PATH="target/x86_64-unknown-none/debug/veridian-kernel"
ISO_DIR="iso_build"
GRUB_DIR="$ISO_DIR/boot/grub"

echo "Creating GRUB bootable ISO for VeridianOS x86_64..."

# Clean and create directories
rm -rf "$ISO_DIR"
mkdir -p "$GRUB_DIR"

# Check if kernel exists
if [ ! -f "$KERNEL_PATH" ]; then
    echo "Error: Kernel not found at $KERNEL_PATH"
    echo "Please build the kernel first with: cargo build --target x86_64-unknown-none -p veridian-kernel"
    exit 1
fi

# Copy kernel
cp "$KERNEL_PATH" "$ISO_DIR/boot/veridian-kernel"

# Create GRUB configuration
cat > "$GRUB_DIR/grub.cfg" << 'EOF'
set timeout=3
set default=0

menuentry "VeridianOS x86_64" {
    multiboot2 /boot/veridian-kernel
    boot
}

menuentry "VeridianOS x86_64 (Serial Debug)" {
    multiboot2 /boot/veridian-kernel
    boot
}
EOF

# Create ISO with GRUB
echo "Creating ISO image..."
grub2-mkrescue -o veridian-os.iso "$ISO_DIR" 2>/dev/null || {
    echo "Error: grub2-mkrescue failed. Make sure GRUB tools are installed:"
    echo "  sudo dnf install grub2-tools-minimal xorriso"
    exit 1
}

echo "ISO created: veridian-os.iso"
echo ""
echo "To test with QEMU:"
echo "  qemu-system-x86_64 -cdrom veridian-os.iso -serial stdio -display none"
echo ""
echo "To test with hardware serial debugging:"
echo "  qemu-system-x86_64 -cdrom veridian-os.iso -serial stdio"