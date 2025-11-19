// AArch64-specific bootstrap output functions

use crate::arch::aarch64::direct_uart::uart_write_str;

pub fn stage1_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Starting multi-stage kernel initialization...\n");
        uart_write_str("[BOOTSTRAP] Stage 1: Hardware initialization\n");
    }
}

pub fn stage1_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Architecture initialized\n");
    }
}

pub fn stage2_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Stage 2: Memory management\n");
    }
}

pub fn stage2_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Memory management initialized\n");
    }
}

pub fn stage3_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Stage 3: Process management\n");
    }
}

pub fn stage3_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Process management initialized\n");
    }
}

pub fn stage4_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Stage 4: Kernel services\n");
    }
}

pub fn stage4_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Core services initialized\n");
    }
}

pub fn stage5_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Stage 5: Scheduler activation\n");
    }
}

pub fn stage5_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Scheduler activated - entering main scheduling loop\n");
    }
}

pub fn stage6_start() {
    unsafe {
        uart_write_str("[BOOTSTRAP] Stage 6: User space transition\n");
    }
}

pub fn stage6_complete() {
    unsafe {
        uart_write_str("[BOOTSTRAP] User space transition prepared\n");
        uart_write_str("[KERNEL] Boot sequence complete!\n");
        uart_write_str("BOOTOK\n");
    }
}
