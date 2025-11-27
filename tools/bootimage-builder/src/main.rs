//! Bootimage Builder for VeridianOS
//!
//! Creates bootable BIOS and UEFI disk images from the compiled kernel.
//! Uses the bootloader 0.11+ crate for image creation.

use anyhow::{Context, Result};
use bootloader::{BiosBoot, UefiBoot, DiskImageBuilder};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bootimage-builder")]
#[command(about = "Creates bootable disk images for VeridianOS")]
struct Args {
    /// Path to the kernel ELF file
    #[arg(short, long)]
    kernel: PathBuf,

    /// Output directory for the disk images
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Boot mode to create image for
    #[arg(short, long, value_enum, default_value = "bios")]
    mode: BootMode,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum BootMode {
    /// Create BIOS-bootable disk image
    Bios,
    /// Create UEFI-bootable disk image
    Uefi,
    /// Create both BIOS and UEFI images
    Both,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("VeridianOS Bootimage Builder");
    println!("============================");
    println!("Kernel: {}", args.kernel.display());
    println!("Output: {}", args.output.display());
    println!("Mode: {:?}", args.mode);
    println!();

    // Verify kernel exists
    if !args.kernel.exists() {
        anyhow::bail!("Kernel file not found: {}", args.kernel.display());
    }

    // Create output directory if needed
    std::fs::create_dir_all(&args.output)
        .context("Failed to create output directory")?;

    match args.mode {
        BootMode::Bios => create_bios_image(&args.kernel, &args.output)?,
        BootMode::Uefi => create_uefi_image(&args.kernel, &args.output)?,
        BootMode::Both => {
            create_bios_image(&args.kernel, &args.output)?;
            create_uefi_image(&args.kernel, &args.output)?;
        }
    }

    println!("\nDisk image creation complete!");
    Ok(())
}

fn create_bios_image(kernel_path: &PathBuf, output_dir: &PathBuf) -> Result<()> {
    println!("Creating BIOS disk image...");

    let bios_image_path = output_dir.join("veridian-bios.img");

    // Create BIOS bootable disk image
    let bios_boot = BiosBoot::new(kernel_path);
    bios_boot
        .create_disk_image(&bios_image_path)
        .context("Failed to create BIOS disk image")?;

    println!("  Created: {}", bios_image_path.display());
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
