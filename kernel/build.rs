use std::{env, path::PathBuf, process::Command};

fn main() {
    let target = env::var("TARGET").expect("TARGET not set");
    let _out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // Get git hash
    let git_hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "0000000000000000000000000000000000000000".to_string());

    println!("cargo:rustc-env=GIT_HASH={}", git_hash.trim());

    // Get build timestamp
    let build_timestamp = Command::new("date")
        .args(["+%s"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "0".to_string());

    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_timestamp.trim());

    // Get the manifest directory (where Cargo.toml is)
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let kernel_dir = PathBuf::from(manifest_dir);

    // Set the linker script based on target architecture
    // Note: For x86_64 with bootloader 0.11+, we don't use a custom linker script
    // The bootloader handles loading the PIE kernel and setting up virtual mappings
    if target.contains("x86_64") {
        // Check for custom x86_64-veridian target (uses custom linker script)
        // Standard x86_64-unknown-none works with bootloader 0.11 without custom linker
        if target == "x86_64-veridian" {
            let linker_script = kernel_dir.join("src/arch/x86_64/link.ld");
            println!("cargo:rustc-link-arg=-T{}", linker_script.display());
        }
        // For x86_64-unknown-none, let bootloader handle the linking
    } else if target.contains("aarch64") {
        let linker_script = kernel_dir.join("src/arch/aarch64/link.ld");
        println!("cargo:rustc-link-arg=-T{}", linker_script.display());
    } else if target.contains("riscv") {
        let linker_script = kernel_dir.join("src/arch/riscv64/link.ld");
        println!("cargo:rustc-link-arg=-T{}", linker_script.display());
    }

    // Rebuild if linker script changes
    println!("cargo:rerun-if-changed=src/arch/x86_64/link.ld");
    println!("cargo:rerun-if-changed=src/arch/aarch64/link.ld");
    println!("cargo:rerun-if-changed=src/arch/riscv64/link.ld");
}
