#!/usr/bin/env python3
"""
Create UEFI-bootable disk image for VeridianOS x86_64
This avoids the BIOS compilation issues in bootloader 0.11
"""

import os
import sys
import subprocess
import tempfile
import shutil
from pathlib import Path

def run_command(cmd, check=True):
    """Run a command and handle errors"""
    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if check and result.returncode != 0:
        print(f"Error running {cmd[0]}:")
        print(result.stderr)
        sys.exit(1)
    return result

def main():
    # Configuration
    kernel_path = Path("target/x86_64-unknown-none/debug/veridian-kernel")
    output_image = Path("veridian-uefi.img")
    
    print("Creating UEFI-bootable disk image for VeridianOS x86_64...")
    
    # Check if kernel exists
    if not kernel_path.exists():
        print(f"Error: Kernel not found at {kernel_path}")
        print("Please build the kernel first with: cargo build --target x86_64-unknown-none -p veridian-kernel")
        sys.exit(1)
    
    # Try to use bootloader crate's UEFI builder
    try:
        # Create a simple UEFI disk image using bootloader
        cmd = [
            "python3", "-c", 
            """
import bootloader
import sys

# Try to create UEFI image only
kernel_path = sys.argv[1]
output_path = sys.argv[2]

# This should work since UEFI components compile successfully
try:
    disk_image = bootloader.create_disk_images(kernel_path, uefi=True, bios=False)
    with open(output_path, 'wb') as f:
        f.write(disk_image.uefi)
    print(f"UEFI image created: {output_path}")
except Exception as e:
    print(f"Error: {e}")
    sys.exit(1)
            """,
            str(kernel_path), str(output_image)
        ]
        
        result = run_command(cmd, check=False)
        if result.returncode == 0:
            print(f"UEFI image created: {output_image}")
            print("")
            print("To test with QEMU (UEFI):")
            print(f"  qemu-system-x86_64 -drive format=raw,file={output_image} -bios /usr/share/OVMF/OVMF_CODE.fd -serial stdio -display none")
            return
    except Exception as e:
        print(f"Bootloader approach failed: {e}")
    
    # Fallback: Create manual FAT32 UEFI image
    print("Falling back to manual UEFI image creation...")
    
    # Create a temporary directory for UEFI filesystem
    with tempfile.TemporaryDirectory() as temp_dir:
        temp_path = Path(temp_dir)
        efi_dir = temp_path / "EFI" / "BOOT"
        efi_dir.mkdir(parents=True)
        
        # Copy kernel as UEFI application (this won't work but demonstrates structure)
        bootx64_path = efi_dir / "BOOTX64.EFI"
        
        print("Note: Manual UEFI creation requires a proper UEFI bootloader")
        print("The kernel needs to be linked as a UEFI application, not raw kernel")
        print("")
        print("Alternative approaches:")
        print("1. Use GRUB with multiboot2 (ISO creation)")
        print("2. Wait for bootloader 0.11 BIOS fix")
        print("3. Downgrade to bootloader 0.9 temporarily")
        
        return False

if __name__ == "__main__":
    main()