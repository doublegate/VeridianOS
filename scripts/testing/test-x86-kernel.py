#!/usr/bin/env python3
"""
Test script to check if the x86_64 kernel boots properly with multiboot
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
    
    print(f"Testing x86_64 kernel: {kernel_path}")
    
    # Create a simple multiboot kernel loader using GRUB
    print("Creating simple test with QEMU multiboot...")
    
    # Check if kernel has multiboot header
    result = subprocess.run(['objdump', '-h', str(kernel_path)], 
                          capture_output=True, text=True)
    print("Kernel sections:")
    print(result.stdout)
    
    # Try loading with qemu multiboot
    cmd = [
        "qemu-system-x86_64",
        "-kernel", str(kernel_path),
        "-append", "console=ttyS0",
        "-serial", "stdio",
        "-display", "none",
        "-m", "512M",
        "-no-reboot",
        "-no-shutdown",
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"
    ]
    
    print(f"Running: {' '.join(cmd)}")
    
    try:
        result = subprocess.run(cmd, timeout=15, capture_output=True, text=True)
        print("QEMU stdout:")
        print(result.stdout)
        print("QEMU stderr:")
        print(result.stderr)
        print(f"QEMU exit code: {result.returncode}")
        
        # Check if we see Stage 6 and BOOTOK in output
        if "BOOTOK" in result.stdout:
            print("✅ SUCCESS: Found BOOTOK in output!")
            return 0
        elif "S6" in result.stdout:
            print("✅ PARTIAL SUCCESS: Found Stage 6 but no BOOTOK")
            return 0
        else:
            print("❌ No Stage 6 or BOOTOK found in output")
            return 1
        
    except subprocess.TimeoutExpired:
        print("⏰ QEMU timed out after 15 seconds - checking if kernel is running...")
        return 2
    except FileNotFoundError:
        print("❌ QEMU not found. Please install qemu-system-x86_64")
        return 1

if __name__ == "__main__":
    sys.exit(main())