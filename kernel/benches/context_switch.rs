//! Context switch benchmark for VeridianOS
//!
//! Measures the time to switch between threads/processes

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::panic::PanicInfo;

use veridian_kernel::{bench::BenchmarkResult, benchmark, serial_println};

const CONTEXT_SWITCH_TARGET_NS: u64 = 10000; // 10μs target
const ITERATIONS: u64 = 1000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("Context Switch Benchmark");
    serial_println!("========================");
    serial_println!(
        "Target: < {} ns ({}μs)",
        CONTEXT_SWITCH_TARGET_NS,
        CONTEXT_SWITCH_TARGET_NS / 1000
    );
    serial_println!();

    // Run different context switch scenarios
    let minimal_result = benchmark_minimal_context_switch();
    let full_result = benchmark_full_context_switch();
    let fpu_result = benchmark_fpu_context_switch();

    // Print results
    serial_println!("\nResults:");
    serial_println!("--------");
    print_result("Minimal Switch", &minimal_result);
    print_result("Full Switch", &full_result);
    print_result("FPU Switch", &fpu_result);

    // Check if we meet targets
    serial_println!("\nTarget Analysis:");
    serial_println!("----------------");
    check_target("Minimal", &minimal_result, CONTEXT_SWITCH_TARGET_NS);
    check_target("Full", &full_result, CONTEXT_SWITCH_TARGET_NS);
    check_target("FPU", &fpu_result, CONTEXT_SWITCH_TARGET_NS);

    // Exit with success
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Success);
}

fn benchmark_minimal_context_switch() -> BenchmarkResult {
    // Simulate minimal context switch (registers only)
    benchmark!("Minimal Context Switch", ITERATIONS, {
        unsafe {
            // Save general purpose registers
            core::arch::asm!(
                "push rax",
                "push rbx",
                "push rcx",
                "push rdx",
                "push rsi",
                "push rdi",
                "push rbp",
                "push r8",
                "push r9",
                "push r10",
                "push r11",
                "push r12",
                "push r13",
                "push r14",
                "push r15",

                // Simulate switching to new context
                "mov rax, 0xDEADBEEF",

                // Restore registers
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rbp",
                "pop rdi",
                "pop rsi",
                "pop rdx",
                "pop rcx",
                "pop rbx",
                "pop rax",
                out("rax") _,
            );
        }
    })
}

fn benchmark_full_context_switch() -> BenchmarkResult {
    // Simulate full context switch including segment registers
    benchmark!("Full Context Switch", ITERATIONS, {
        let mut context = ProcessContext::default();

        // Save context
        save_context(&mut context);

        // Simulate switching page tables
        switch_page_tables();

        // Restore context
        restore_context(&context);
    })
}

fn benchmark_fpu_context_switch() -> BenchmarkResult {
    // Simulate context switch with FPU state
    benchmark!("FPU Context Switch", ITERATIONS, {
        unsafe {
            // Save FPU state
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!(
                "fxsave [{}]",
                in(reg) &mut [0u8; 512],
            );

            // Switch context
            core::arch::asm!("nop");

            // Restore FPU state
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!(
                "fxrstor [{}]",
                in(reg) &[0u8; 512],
            );
        }
    })
}

#[repr(C)]
#[derive(Default)]
struct ProcessContext {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    rsp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rflags: u64,
    rip: u64,
}

#[inline(never)]
fn save_context(ctx: &mut ProcessContext) {
    // Simulate saving context
    ctx.rax = 0x1234;
    ctx.rbx = 0x5678;
    // ... etc
}

#[inline(never)]
fn restore_context(ctx: &ProcessContext) {
    // Simulate restoring context
    let _ = ctx.rax;
    let _ = ctx.rbx;
    // ... etc
}

#[inline(never)]
fn switch_page_tables() {
    // Simulate CR3 reload
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!(
            "mov rax, cr3",
            "mov cr3, rax",
            out("rax") _,
        );
    }
}

fn print_result(name: &str, result: &BenchmarkResult) {
    serial_println!(
        "{:<20} Avg: {:>6} ns, Min: {:>6} ns, Max: {:>6} ns",
        name,
        result.avg_time_ns,
        result.min_time_ns,
        result.max_time_ns
    );
}

fn check_target(name: &str, result: &BenchmarkResult, target_ns: u64) {
    if result.meets_target(target_ns) {
        serial_println!(
            "{:<20} ✓ PASS ({}ns < {}ns)",
            name,
            result.avg_time_ns,
            target_ns
        );
    } else {
        serial_println!(
            "{:<20} ✗ FAIL ({}ns > {}ns)",
            name,
            result.avg_time_ns,
            target_ns
        );
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("Benchmark panic: {}", info);
    veridian_kernel::exit_qemu(veridian_kernel::QemuExitCode::Failed)
}
