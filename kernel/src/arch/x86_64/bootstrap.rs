// x86_64-specific bootstrap output functions

use crate::early_println;

pub fn stage1_start() {
    early_println!("[BOOTSTRAP] Starting multi-stage kernel initialization...");
    early_println!("[BOOTSTRAP] Stage 1: Hardware initialization");
}

pub fn stage1_complete() {
    early_println!("[BOOTSTRAP] Architecture initialized");
}

pub fn stage2_start() {
    early_println!("[BOOTSTRAP] Stage 2: Memory management");
}

pub fn stage2_complete() {
    early_println!("[BOOTSTRAP] Memory management initialized");
}

pub fn stage3_start() {
    early_println!("[BOOTSTRAP] Stage 3: Process management");
}

pub fn stage3_complete() {
    early_println!("[BOOTSTRAP] Process management initialized");
}

pub fn stage4_start() {
    early_println!("[BOOTSTRAP] Stage 4: Kernel services");
}

pub fn stage4_complete() {
    early_println!("[BOOTSTRAP] Core services initialized");
}

pub fn stage5_start() {
    early_println!("[BOOTSTRAP] Stage 5: Scheduler activation");
}

pub fn stage5_complete() {
    early_println!("[BOOTSTRAP] Scheduler activated - entering main scheduling loop");
}

pub fn stage6_start() {
    early_println!("[BOOTSTRAP] Stage 6: User space transition");
}

pub fn stage6_complete() {
    early_println!("[BOOTSTRAP] User space transition prepared");
    early_println!("[KERNEL] Boot sequence complete!");
    early_println!("BOOTOK");
}