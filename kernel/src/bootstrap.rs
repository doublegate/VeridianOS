//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

#[cfg(not(target_arch = "aarch64"))]
use crate::security;
use crate::{
    arch, cap, error::KernelResult, fs, graphics, ipc, mm, net, perf, pkg, process, sched, services,
};

#[cfg(feature = "alloc")]
extern crate alloc;

/// Macro to generate the 12 bootstrap stage tracking functions.
///
/// Each architecture provides its own `$print_fn` macro that accepts a single
/// string literal and outputs it (with a trailing newline) to the
/// architecture's early console.  This eliminates the otherwise-identical
/// stage function bodies duplicated across x86_64, AArch64, and RISC-V.
///
/// # Usage
///
/// ```ignore
/// // In arch/<arch>/bootstrap.rs:
/// macro_rules! arch_boot_print {
///     ($s:expr) => { /* arch-specific print */ };
/// }
/// crate::bootstrap::define_bootstrap_stages!(arch_boot_print);
/// ```
#[macro_export]
macro_rules! define_bootstrap_stages {
    ($print_fn:ident) => {
        pub fn stage1_start() {
            $print_fn!("[BOOTSTRAP] Starting multi-stage kernel initialization...");
            $print_fn!("[BOOTSTRAP] Stage 1: Hardware initialization");
        }

        pub fn stage1_complete() {
            $print_fn!("[BOOTSTRAP] Architecture initialized");
        }

        pub fn stage2_start() {
            $print_fn!("[BOOTSTRAP] Stage 2: Memory management");
        }

        pub fn stage2_complete() {
            $print_fn!("[BOOTSTRAP] Memory management initialized");
        }

        pub fn stage3_start() {
            $print_fn!("[BOOTSTRAP] Stage 3: Process management");
        }

        pub fn stage3_complete() {
            $print_fn!("[BOOTSTRAP] Process management initialized");
        }

        pub fn stage4_start() {
            $print_fn!("[BOOTSTRAP] Stage 4: Kernel services");
        }

        pub fn stage4_complete() {
            $print_fn!("[BOOTSTRAP] Core services initialized");
        }

        pub fn stage5_start() {
            $print_fn!("[BOOTSTRAP] Stage 5: Scheduler activation");
        }

        pub fn stage5_complete() {
            $print_fn!("[BOOTSTRAP] Scheduler activated - entering main scheduling loop");
        }

        pub fn stage6_start() {
            $print_fn!("[BOOTSTRAP] Stage 6: User space transition");
        }

        pub fn stage6_complete() {
            $print_fn!("[BOOTSTRAP] User space transition prepared");
            $print_fn!("[KERNEL] Boot sequence complete!");
            $print_fn!("BOOTOK");
        }
    };
}

/// Bootstrap task ID (runs before scheduler is fully initialized)
pub const BOOTSTRAP_PID: u64 = 0;
pub const BOOTSTRAP_TID: u64 = 0;

/// Multi-stage kernel initialization
///
/// This function implements the recommended boot sequence from
/// DEEP-RECOMMENDATIONS.md to avoid circular dependencies between process
/// management and scheduler.
pub fn kernel_init() -> KernelResult<()> {
    // Direct UART output for RISC-V debugging
    #[cfg(target_arch = "riscv64")]
    // SAFETY: 0x1000_0000 is the UART data register on the QEMU virt
    // machine.  This address is always mapped and writable during early
    // boot on this platform.  write_volatile ensures the compiler does
    // not elide or reorder the MMIO stores.
    unsafe {
        let uart_base = 0x1000_0000 as *mut u8;
        // Write "KINIT" to show kernel_init reached
        uart_base.write_volatile(b'K');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'N');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'T');
        uart_base.write_volatile(b'\n');
    }

    // Stage 1: Hardware initialization
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage1_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage1_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage1_start();

    arch::init();

    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage1_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage1_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage1_complete();

    // Stage 2: Memory management
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage2_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage2_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage2_start();

    mm::init_default();

    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage2_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage2_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage2_complete();

    // Verify heap allocation works (AArch64 requires -Zub-checks=no)
    #[cfg(target_arch = "aarch64")]
    {
        let test_box = alloc::boxed::Box::new(42u64);
        assert!(*test_box == 42);
        drop(test_box);
        // SAFETY: uart_write_str performs MMIO writes to the QEMU virt
        // machine UART at 0x0900_0000.  The address is mapped and valid
        // during early boot.  Only writes bytes to the data register.
        unsafe {
            crate::arch::aarch64::direct_uart::uart_write_str(
                "[BOOTSTRAP] Heap allocation verified OK\n",
            );
        }
    }

    // Stage 3: Process management
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage3_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage3_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage3_start();

    process::init_without_init_process().expect("Failed to initialize process management");

    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage3_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage3_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage3_complete();

    // Stage 4: Core kernel services
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage4_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage4_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage4_start();

    println!("[BOOTSTRAP] Initializing capabilities...");
    cap::init();
    println!("[BOOTSTRAP] Capabilities initialized");

    // Security subsystem uses spin::Mutex which hangs on AArch64 bare metal
    // (CAS-based spinlocks interact badly with AArch64 exclusive monitor).
    // Skip security init on AArch64 - not needed for Phase 2 VFS/shell tests.
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("[BOOTSTRAP] Initializing security subsystem...");
        security::init().expect("Failed to initialize security");
        println!("[BOOTSTRAP] Security subsystem initialized");
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000, which is mapped
    // and valid during boot.  Only writes bytes to the UART data register.
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[BOOTSTRAP] Security subsystem skipped (AArch64 spinlock issue)\n");
    }

    #[cfg(target_arch = "aarch64")]
    // SAFETY: MMIO write to QEMU virt UART (see above).
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[BOOTSTRAP] Initializing perf/IPC...\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Initializing performance monitoring...");
    perf::init().expect("Failed to initialize performance monitoring");
    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Performance monitoring initialized");

    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] Initializing IPC...");
    ipc::init();
    #[cfg(target_arch = "aarch64")]
    // SAFETY: MMIO write to QEMU virt UART (see safety note at top of Stage 4).
    unsafe {
        use crate::arch::aarch64::direct_uart::uart_write_str;
        uart_write_str("[BOOTSTRAP] Perf/IPC initialized\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("[BOOTSTRAP] IPC initialized");

    // Initialize VFS and mount essential filesystems
    #[cfg(feature = "alloc")]
    {
        // Add early debug output for AArch64
        #[cfg(target_arch = "aarch64")]
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] About to initialize VFS (AArch64 direct UART)...\n");
        }

        println!("[BOOTSTRAP] Initializing VFS...");
        fs::init();

        #[cfg(target_arch = "aarch64")]
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] VFS initialized (AArch64 direct UART)\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] VFS initialized");
    }

    // Initialize services (process server, driver framework, etc.)
    #[cfg(feature = "alloc")]
    {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Initializing services (AArch64)...\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] Initializing services...");

        services::init();

        #[cfg(target_arch = "aarch64")]
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Services initialized (AArch64)\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        println!("[BOOTSTRAP] Services initialized");
    }

    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage4_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage4_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage4_complete();

    // Run kernel-mode init tests after Stage 4 (VFS + shell ready)
    // Must run BEFORE Stage 5 scheduler init on RISC-V where the allocator
    // gets corrupted during scheduler's 72KB ready queue allocation
    kernel_init_main();

    // Stage 5: Scheduler initialization
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage5_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage5_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage5_start();

    sched::init();

    // Initialize package manager
    #[cfg(feature = "alloc")]
    {
        println!("[BOOTSTRAP] Initializing package manager...");
        pkg::init();
        println!("[BOOTSTRAP] Package manager initialized");
    }

    // Initialize network stack
    #[cfg(feature = "alloc")]
    {
        println!("[BOOTSTRAP] Initializing network stack...");
        net::init().expect("Failed to initialize network stack");
        println!("[BOOTSTRAP] Network stack initialized");
    }

    // Initialize graphics subsystem
    println!("[BOOTSTRAP] Initializing graphics subsystem...");
    graphics::init().expect("Failed to initialize graphics");
    println!("[BOOTSTRAP] Graphics subsystem initialized");

    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage5_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage5_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage5_complete();

    Ok(())
}

/// Run the bootstrap sequence
pub fn run() -> ! {
    // Direct UART output for RISC-V debugging
    #[cfg(target_arch = "riscv64")]
    // SAFETY: 0x1000_0000 is the UART data register on the QEMU virt
    // machine.  This address is always mapped and writable during early
    // boot.  write_volatile ensures the compiler does not elide the
    // MMIO stores.
    unsafe {
        let uart_base = 0x1000_0000 as *mut u8;
        // Write "RUN" to show run() reached
        uart_base.write_volatile(b'R');
        uart_base.write_volatile(b'U');
        uart_base.write_volatile(b'N');
        uart_base.write_volatile(b'\n');
    }

    if let Err(e) = kernel_init() {
        // Panic is intentional: kernel_init failure during boot is unrecoverable.
        // No subsystems are available for graceful error handling at this point.
        panic!("Bootstrap failed: {:?}", e);
    }

    // Stage 6: User space transition
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage6_start();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage6_start();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage6_start();

    // Create init process
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] About to create init process...\n");
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        println!("[BOOTSTRAP] About to create init process...");
    }

    create_init_process();

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: MMIO write to QEMU virt UART at 0x0900_0000.
        unsafe {
            use crate::arch::aarch64::direct_uart::uart_write_str;
            uart_write_str("[BOOTSTRAP] Init process created\n");
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        println!("[BOOTSTRAP] Init process created");
    }

    // Mark Stage 6 complete
    #[cfg(target_arch = "x86_64")]
    arch::x86_64::bootstrap::stage6_complete();
    #[cfg(target_arch = "aarch64")]
    arch::aarch64::bootstrap::stage6_complete();
    #[cfg(target_arch = "riscv64")]
    arch::riscv64::bootstrap::stage6_complete();

    // Transfer control to scheduler (kernel_init_main runs inside start())
    sched::start();
}

/// Kernel-mode init function
///
/// Exercises Phase 2 subsystems (VFS, shell, services) at runtime and emits
/// QEMU-parseable `[ok]`/`[failed]` markers for each test. Called from
/// `sched::start()` before entering the idle loop.
#[cfg(feature = "alloc")]
pub fn kernel_init_main() {
    // AArch64 println! is a no-op (LLVM bug), so use direct UART everywhere
    macro_rules! kprintln {
        ($s:literal) => {{
            #[cfg(target_arch = "aarch64")]
            {
                crate::arch::aarch64::direct_uart::direct_print_str($s);
                crate::arch::aarch64::direct_uart::direct_print_str("\n");
            }
            #[cfg(not(target_arch = "aarch64"))]
            println!($s);
        }};
    }

    kprintln!("");
    kprintln!("========================================");
    kprintln!("[INIT] VeridianOS kernel-mode init");
    kprintln!("========================================");

    let mut passed = 0u32;
    let mut failed = 0u32;

    // --- VFS Tests ---
    kprintln!("[INIT] VFS tests:");

    // Test 1: Create directory
    {
        let ok = fs::get_vfs()
            .read()
            .mkdir("/tmp/test_init", fs::Permissions::default())
            .is_ok();
        report_test("vfs_mkdir", ok, &mut passed, &mut failed);
    }

    // Test 2: Write file via VFS create + write
    {
        let ok = (|| -> Result<(), &'static str> {
            let vfs = fs::get_vfs().read();
            let parent = vfs.resolve_path("/tmp/test_init")?;
            let file = parent.create("hello.txt", fs::Permissions::default())?;
            file.write(0, b"Hello VeridianOS")?;
            Ok(())
        })()
        .is_ok();
        report_test("vfs_write_file", ok, &mut passed, &mut failed);
    }

    // Test 3: Read file back and verify contents
    {
        let ok = (|| -> Result<bool, &'static str> {
            let vfs = fs::get_vfs().read();
            let dir = vfs.resolve_path("/tmp/test_init")?;
            let file = dir.lookup("hello.txt")?;
            let mut buf = [0u8; 32];
            let n = file.read(0, &mut buf)?;
            Ok(&buf[..n] == b"Hello VeridianOS")
        })()
        .unwrap_or(false);
        report_test("vfs_read_verify", ok, &mut passed, &mut failed);
    }

    // Test 4: List directory entries
    {
        let ok = (|| -> Result<bool, &'static str> {
            let vfs = fs::get_vfs().read();
            let node = vfs.resolve_path("/tmp/test_init")?;
            let entries = node.readdir()?;
            Ok(entries.iter().any(|e| e.name == "hello.txt"))
        })()
        .unwrap_or(false);
        report_test("vfs_readdir", ok, &mut passed, &mut failed);
    }

    // Test 5: /proc is mounted
    {
        let ok = fs::get_vfs().read().resolve_path("/proc").is_ok();
        report_test("vfs_procfs", ok, &mut passed, &mut failed);
    }

    // Test 6: /dev is mounted
    {
        let ok = fs::get_vfs().read().resolve_path("/dev").is_ok();
        report_test("vfs_devfs", ok, &mut passed, &mut failed);
    }

    // --- Shell Tests ---
    kprintln!("[INIT] Shell tests:");

    let shell = match services::shell::try_get_shell() {
        Some(s) => s,
        None => {
            kprintln!("  shell unavailable [failed]");
            failed += 6;
            print_summary(passed, failed);
            return;
        }
    };

    // Test 7: help command
    {
        let ok = matches!(
            shell.execute_command("help"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_help", ok, &mut passed, &mut failed);
    }

    // Test 8: pwd command
    {
        let ok = matches!(
            shell.execute_command("pwd"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_pwd", ok, &mut passed, &mut failed);
    }

    // Test 9: ls / command
    {
        let ok = matches!(
            shell.execute_command("ls /"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_ls", ok, &mut passed, &mut failed);
    }

    // Test 10: env command
    {
        let ok = matches!(
            shell.execute_command("env"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_env", ok, &mut passed, &mut failed);
    }

    // Test 11: echo command
    {
        let ok = matches!(
            shell.execute_command("echo hello"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_echo", ok, &mut passed, &mut failed);
    }

    // Test 12: mkdir + verification via VFS
    {
        let ok = matches!(
            shell.execute_command("mkdir /tmp/shell_test"),
            services::shell::CommandResult::Success(_)
        ) && fs::file_exists("/tmp/shell_test");
        report_test("shell_mkdir_verify", ok, &mut passed, &mut failed);
    }

    // --- Summary ---
    print_summary(passed, failed);
}

#[cfg(not(feature = "alloc"))]
pub fn kernel_init_main() {
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::aarch64::direct_uart::direct_print_str("BOOTOK\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    println!("BOOTOK");
}

/// Print test summary and BOOTOK/BOOTFAIL
fn print_summary(passed: u32, failed: u32) {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::arch::aarch64::direct_uart::{direct_print_num, direct_print_str};
        direct_print_str("========================================\n");
        direct_print_str("[INIT] Results: ");
        direct_print_num(passed as u64);
        direct_print_str("/");
        direct_print_num((passed + failed) as u64);
        direct_print_str(" passed\n");
        if failed == 0 {
            direct_print_str("BOOTOK\n");
        } else {
            direct_print_str("BOOTFAIL\n");
        }
        direct_print_str("========================================\n");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        println!("========================================");
        println!("[INIT] Results: {}/{} passed", passed, passed + failed);
        if failed == 0 {
            println!("BOOTOK");
        } else {
            println!("BOOTFAIL");
        }
        println!("========================================");
    }
}

/// Report a single test result with QEMU-parseable markers
fn report_test(name: &str, ok: bool, passed: &mut u32, failed: &mut u32) {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::arch::aarch64::direct_uart::direct_print_str;
        direct_print_str("  ");
        direct_print_str(name);
        if ok {
            direct_print_str("...[ok]\n");
        } else {
            direct_print_str("...[failed]\n");
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    if ok {
        println!("  {}...[ok]", name);
    } else {
        println!("  {}...[failed]", name);
    }

    if ok {
        *passed += 1;
    } else {
        *failed += 1;
    }
}

/// Create the init process
fn create_init_process() {
    #[cfg(feature = "alloc")]
    {
        // Try to load init from the filesystem
        match crate::userspace::load_init_process() {
            Ok(_init_pid) => {
                println!("[BOOTSTRAP] Init process created with PID {}", _init_pid.0);

                // Try to load a shell as well.
                // Skip on RISC-V: the bump allocator (4 MB, no dealloc) is
                // nearly exhausted by this point and the additional process
                // creation triggers heap allocations whose zero-fill
                // overwrites VFS_PTR in adjacent BSS, causing a panic.
                // User-space execution is not functional yet on any
                // architecture, so the shell PCB is not needed.
                #[cfg(not(target_arch = "riscv64"))]
                if let Ok(_shell_pid) = crate::userspace::loader::load_shell() {
                    println!(
                        "[BOOTSTRAP] Shell process created with PID {}",
                        _shell_pid.0
                    );
                }
            }
            Err(_e) => {
                println!("[BOOTSTRAP] Failed to create init process: {}", _e);
                // Fall back to creating a minimal test process
                use alloc::string::String;
                if let Ok(_pid) = process::lifecycle::create_process(String::from("init"), 0) {
                    println!(
                        "[BOOTSTRAP] Created fallback init process with PID {}",
                        _pid.0
                    );
                }
            }
        }
    }
}
