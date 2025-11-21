// RISC-V-specific bootstrap output functions

#[allow(unused_imports)]
use crate::println;

pub fn stage1_start() {
    println!("[BOOTSTRAP] Starting multi-stage kernel initialization...");
    println!("[BOOTSTRAP] Stage 1: Hardware initialization");
}

pub fn stage1_complete() {
    println!("[BOOTSTRAP] Architecture initialized");
}

pub fn stage2_start() {
    println!("[BOOTSTRAP] Stage 2: Memory management");
}

pub fn stage2_complete() {
    println!("[BOOTSTRAP] Memory management initialized");
}

pub fn stage3_start() {
    println!("[BOOTSTRAP] Stage 3: Process management");
}

pub fn stage3_complete() {
    println!("[BOOTSTRAP] Process management initialized");
}

pub fn stage4_start() {
    println!("[BOOTSTRAP] Stage 4: Kernel services");
}

pub fn stage4_complete() {
    println!("[BOOTSTRAP] Core services initialized");
}

pub fn stage5_start() {
    println!("[BOOTSTRAP] Stage 5: Scheduler activation");
}

pub fn stage5_complete() {
    println!("[BOOTSTRAP] Scheduler activated - entering main scheduling loop");
}

pub fn stage6_start() {
    println!("[BOOTSTRAP] Stage 6: User space transition");
}

pub fn stage6_complete() {
    println!("[BOOTSTRAP] User space transition prepared");
    println!("[KERNEL] Boot sequence complete!");
    println!("BOOTOK");
}
