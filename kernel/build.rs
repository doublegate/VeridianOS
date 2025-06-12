use std::{env, path::PathBuf};

fn main() {
    let target = env::var("TARGET").expect("TARGET not set");
    let _out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // Get the manifest directory (where Cargo.toml is)
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let kernel_dir = PathBuf::from(manifest_dir);

    // Set the linker script based on target architecture
    if target.contains("x86_64") {
        let linker_script = kernel_dir.join("src/arch/x86_64/link.ld");
        println!("cargo:rustc-link-arg=-T{}", linker_script.display());
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
