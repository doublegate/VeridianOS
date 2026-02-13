//! Bootimage Builder for VeridianOS
//!
//! Creates a bootable UEFI disk image from the compiled kernel.
//! Uses the bootloader 0.11+ crate for image creation.
//!
//! Note: BIOS mode is not supported because bootloader 0.11's BIOS stage
//! compiles 16-bit real mode code that fails with R_386_16 relocation errors
//! on newer LLVM toolchains. UEFI mode avoids this entirely.

use anyhow::{Context, Result};
use bootloader::UefiBoot;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bootimage-builder")]
#[command(about = "Creates bootable UEFI disk images for VeridianOS")]
struct Args {
    /// Path to the kernel ELF file
    #[arg(short, long)]
    kernel: PathBuf,

    /// Output directory for the disk images
    #[arg(short, long, default_value = ".")]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("VeridianOS Bootimage Builder (UEFI)");
    println!("====================================");
    println!("Kernel: {}", args.kernel.display());
    println!("Output: {}", args.output.display());
    println!();

    // Verify kernel exists
    if !args.kernel.exists() {
        anyhow::bail!("Kernel file not found: {}", args.kernel.display());
    }

    // Create output directory if needed
    std::fs::create_dir_all(&args.output)
        .context("Failed to create output directory")?;

    create_uefi_image(&args.kernel, &args.output)?;

    println!("\nDisk image creation complete!");
    Ok(())
}

fn create_uefi_image(kernel_path: &PathBuf, output_dir: &PathBuf) -> Result<()> {
    println!("Creating UEFI disk image...");

    let uefi_image_path = output_dir.join("veridian-uefi.img");

    // Create UEFI bootable disk image
    let uefi_boot = UefiBoot::new(kernel_path);
    uefi_boot
        .create_disk_image(&uefi_image_path)
        .context("Failed to create UEFI disk image")?;

    println!("  Created: {}", uefi_image_path.display());
    Ok(())
}
