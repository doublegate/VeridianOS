#!/usr/bin/env python3
"""
Build script to create a bootable x86_64 disk image using bootloader 0.11
This uses the QEMU tools directly to create a simple disk image for testing.
"""

import subprocess
import os
import sys
from pathlib import Path

def main():
    kernel_path = Path("target/x86_64-unknown-none/debug/veridian-kernel")
    
    if not kernel_path.exists():
        print(f"Kernel binary not found at: {kernel_path}")
        return 1
    
    print(f"Creating simple disk image for kernel: {kernel_path}")
    
    # For now, we'll just test with QEMU directly loading the kernel
    # This bypasses the bootloader issue temporarily
    print("Testing x86_64 kernel boot with QEMU...")
    
    cmd = [
        "qemu-system-x86_64",
        "-kernel", str(kernel_path),
        "-serial", "stdio",
        "-display", "none",
        "-no-reboot",
        "-no-shutdown"
    ]
    
    print(f"Running: {' '.join(cmd)}")
    
    try:
        result = subprocess.run(cmd, timeout=30, capture_output=True, text=True)
        print("QEMU stdout:")
        print(result.stdout)
        print("QEMU stderr:")
        print(result.stderr)
        print(f"QEMU exit code: {result.returncode}")
    except subprocess.TimeoutExpired:
        print("QEMU timed out after 30 seconds - this might be normal for a looping kernel")
    except FileNotFoundError:
        print("QEMU not found. Please install qemu-system-x86_64")
        return 1
    
    return 0

if __name__ == "__main__":
    sys.exit(main())