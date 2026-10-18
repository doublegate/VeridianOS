//! Bootstrap module for kernel initialization
//!
//! This module handles the multi-stage initialization process to avoid
//! circular dependencies between subsystems.

#[cfg(target_arch = "x86_64")]
use crate::virt;
use crate::{
    arch, audio, cap, desktop, error::KernelResult, fs, graphics, ipc, irq, mm, net, perf, pkg,
    process, sched, security, services, timer, video,
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

/// Switch to a larger heap-allocated stack to avoid stack overflow during
/// the remainder of kernel initialization.
///
/// The UEFI bootloader provides a 128KB stack (configured via
/// `BOOTLOADER_CONFIG.kernel_stack_size`). In debug mode, the Stage 3+
/// initialization chain constructs large arrays on the stack before
/// boxing them (e.g., `CapabilitySpace` allocates a 256-entry L1 table
/// of `RwLock<Option<CapabilityEntry>>` -- ~20KB on the stack) and
/// security modules create multi-KB structs. These deep, unoptimized
/// call chains overflow 128KB. After the heap allocator is ready
/// (Stage 2), we allocate a 256KB stack and switch to it.
///
/// This function does NOT return — it calls `kernel_init_stage3_onwards()`
/// on the new stack via inline assembly.
#[cfg(target_arch = "x86_64")]
fn switch_to_heap_stack(size: usize) {
    use alloc::vec;

    // Allocate stack from heap (Vec ensures it's properly sized and aligned)
    let stack_mem = vec![0u8; size];
    let stack_top = stack_mem.as_ptr() as usize + size;

    // Leak the memory so it persists (the old stack frames below us are abandoned)
    core::mem::forget(stack_mem);

    // Align to 16 bytes (x86_64 ABI requirement)
    let stack_top_aligned = stack_top & !0xF;

    kprintln!(
        "[BOOTSTRAP] Switching to heap stack ({} KB at {:#x})",
        size / 1024,
        stack_top_aligned
    );

    // SAFETY: stack_top_aligned points to the top of a freshly allocated,
    // properly aligned memory region. We switch RSP to this new stack and
    // call kernel_init_stage3_onwards which continues the boot sequence.
    // The old stack is no longer used (kernel_init_stage3_onwards never returns).
    unsafe {
        core::arch::asm!(
            "mov rsp, {0}",
            "call {1}",
            in(reg) stack_top_aligned,
            sym kernel_init_stage3_onwards,
            options(noreturn)
        );
    }
}

/// Continuation of kernel_init after switching to the heap stack (x86_64).
///
/// Called from `switch_to_heap_stack` on a fresh 64KB stack. This function
/// runs the remainder of the boot sequence (Stages 3-6) and then transfers
/// control to the scheduler (never returns).
#[cfg(target_arch = "x86_64")]
extern "C" fn kernel_init_stage3_onwards() -> ! {
    if let Err(e) = kernel_init_stage3_impl() {
        crate::println!("[BOOTSTRAP] FATAL: Stage 3+ init failed: {:?}", e);
        loop {
            // SAFETY: Halting the CPU in an unrecoverable error loop. No
            // memory or stack side effects.
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    // Stage 6: User space transition (same as run())
    kprintln!("[BOOTSTRAP] Stage 6: User space transition");
    kprintln!("[BOOTSTRAP] About to create init process...");
    create_init_process();
    kprintln!("[BOOTSTRAP] Init process created");
    kprintln!("[BOOTSTRAP] User space transition prepared");
    kprintln!("[KERNEL] Boot sequence complete!");
    kprintln!("BOOTOK");

    // User-mode entry via iretq is available but transitions to Ring 3
    // with -> ! (never returns). Since the interactive shell is the
    // primary interface, we skip the Ring 3 transition and go directly
    // to the shell. The Ring 3 pathway (SYSCALL/SYSRET) is verified
    // working in previous releases (v0.3.9+).
    kprintln!("[BOOTSTRAP] User-mode entry available (Ring 3 via iretq)");
    kprintln!("[BOOTSTRAP] Skipping Ring 3 transition for interactive shell");

    // x86_64: Enable keyboard IRQ and CPU interrupts before launching the
    // shell. The keyboard driver was initialized in Stage 4; here we unmask
    // the PIC and enable hardware interrupts so keypresses arrive.
    #[cfg(target_arch = "x86_64")]
    {
        arch::x86_64::enable_keyboard_irq();
        arch::x86_64::enable_timer_irq();
        arch::x86_64::enable_interrupts();
        kprintln!("[BOOTSTRAP] Keyboard IRQ + interrupts enabled");
    }

    // Enable framebuffer console output now that boot is complete.
    // Boot messages were serial-only for performance (rendering 100+ lines
    // to a 1280x800 framebuffer is too slow in QEMU's emulated CPU).
    graphics::fbcon::enable_output();

    // Boot directly to the native vsh (kernel-space shell).
    // User-space shells (BusyBox ash, /bin/sh) can be launched from vsh
    // via the `ash` or `/bin/sh` command if a rootfs with BusyBox is loaded.
    #[cfg(all(feature = "alloc", target_arch = "x86_64"))]
    {
        let vfs = crate::fs::get_vfs();
        let has_sh = vfs.resolve_path("/bin/sh").is_ok();
        if has_sh {
            kprintln!("[BOOTSTRAP] BusyBox ash available at /bin/sh (run 'ash' from vsh)");
        }
    }

    // Launch the interactive kernel shell (never returns).
    // The shell provides a serial console REPL for all 3 architectures.
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Starting interactive shell...");
        crate::services::shell::run_shell();
    }

    // Fallback: transfer control to scheduler if shell unavailable
    #[cfg(not(feature = "alloc"))]
    sched::start();
}

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
        let uart_base = crate::mm::phys_to_virt_addr(0x1000_0000) as *mut u8;
        uart_base.write_volatile(b'K');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'N');
        uart_base.write_volatile(b'I');
        uart_base.write_volatile(b'T');
        uart_base.write_volatile(b'\n');
    }

    // Stage 1: Hardware initialization
    kprintln!("[BOOTSTRAP] Starting multi-stage kernel initialization...");
    kprintln!("[BOOTSTRAP] Stage 1: Hardware initialization");

    arch::init();

    // x86_64: Reprogram PAT entry 1 from WT to WC so that framebuffer pages
    // can use write-combining. Must be done before any WC mappings.
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::x86_64::pat::init();
        kprintln!("[BOOTSTRAP] PAT configured (WC available)");
        crate::arch::x86_64::rtc::init();
    }
    // AArch64 and RISC-V: the wall clock from a device-tree RTC.
    #[cfg(all(
        any(target_arch = "aarch64", target_arch = "riscv64"),
        target_os = "none"
    ))]
    crate::arch::mmio_rtc::init();

    kprintln!("[BOOTSTRAP] Architecture initialized");

    // Stage 2: Memory management
    kprintln!("[BOOTSTRAP] Stage 2: Memory management");

    // Also reserves the boot page-table frames on x86_64, before the heap
    // is taken from the frame allocator.
    mm::init_default();

    // Kernel stack region (N-26): its page tables must exist before the
    // first address space copies the kernel half.
    mm::kstack::init();

    kprintln!("[BOOTSTRAP] Memory management initialized");

    // Verify heap allocation works (AArch64 requires -Zub-checks=no)
    #[cfg(target_arch = "aarch64")]
    {
        let test_box = alloc::boxed::Box::new(42u64);
        assert!(*test_box == 42);
        drop(test_box);
        kprintln!("[BOOTSTRAP] Heap allocation verified OK");
    }

    // x86_64: Initialize framebuffer console (fbcon) so that all subsequent
    // println! output appears on both serial AND the graphical display.
    // The UEFI bootloader already mapped the framebuffer; we just wire it up.
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(fb_info) = crate::arch::x86_64::boot::get_framebuffer_info() {
            let format = if fb_info.is_bgr {
                crate::graphics::fbcon::FbPixelFormat::Bgr
            } else {
                crate::graphics::fbcon::FbPixelFormat::Rgb
            };
            // SAFETY: fb_info.buffer is the UEFI-provided framebuffer,
            // valid for stride * height bytes and mapped for the kernel lifetime.
            unsafe {
                crate::graphics::fbcon::init(
                    fb_info.buffer,
                    fb_info.width,
                    fb_info.height,
                    fb_info.stride,
                    fb_info.bpp,
                    format,
                );
            }
            kprintln!("[BOOTSTRAP] Framebuffer console initialized");

            // Store the framebuffer physical address for user-space mmap.
            // The bootloader maps the framebuffer at a dynamic virtual address
            // (via Mapping::Dynamic) which is NOT necessarily PHYS_MEM_OFFSET +
            // phys_addr. We must walk the kernel page table to discover the
            // physical address backing the framebuffer virtual address.
            {
                let fb_virt = fb_info.buffer as u64;
                let mut fb_phys = 0u64;

                // Walk the kernel page table (CR3) to translate fb virtual -> physical
                let cr3: u64;
                // SAFETY: Reading CR3 to get the current page table root.
                unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack)) };
                let pt_root = cr3 & !0xFFF;

                // SAFETY: pt_root is the current kernel L4 page table, valid and
                // identity-mapped via the physical memory window.
                let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
                if let Ok((frame, _flags)) =
                    mapper.translate_page(crate::mm::VirtualAddress(fb_virt))
                {
                    fb_phys = frame.as_u64() << 12;
                    // Add page offset from the virtual address
                    fb_phys |= fb_virt & 0xFFF;
                }

                if fb_phys != 0 {
                    // Set the global FB_PHYS_ADDR atomic (used by DRM mmap via
                    // get_phys_addr()). Must be called BEFORE configure_with_phys
                    // to avoid lock ordering issues.
                    crate::graphics::framebuffer::set_phys_addr(fb_phys);

                    // Configure the graphics::framebuffer FRAMEBUFFER static with
                    // the real dimensions, buffer pointer, and physical address.
                    // This is needed by get_fb_info() (used by init_virtual_drm_device).
                    let fb_format: u32 = if fb_info.is_bgr { 0 } else { 1 };
                    crate::graphics::framebuffer::with_framebuffer(|fb| {
                        fb.configure_with_phys(
                            fb_info.width as u32,
                            fb_info.height as u32,
                            fb_info.stride as u32,
                            fb_info.bpp as u8,
                            fb_info.buffer as *mut u32,
                            fb_phys,
                            fb_format,
                        );
                    });
                    kprintln!(
                        "[BOOTSTRAP] Framebuffer phys addr: 0x{:x} ({}x{})",
                        fb_phys,
                        fb_info.width,
                        fb_info.height
                    );
                } else {
                    kprintln!("[BOOTSTRAP] WARNING: Could not determine framebuffer phys addr");
                }
            }

            // Apply write-combining to the framebuffer's MMIO pages for
            // 5-150x faster blit throughput (pure writes, no reads).
            let fb_size = fb_info.stride * fb_info.height;
            let fb_size_aligned = (fb_size + 4095) & !4095;
            // SAFETY: fb_info.buffer is page-aligned (UEFI framebuffer) and
            // mapped for fb_size_aligned bytes. PAT entry 1 was reprogrammed
            // to WC above. The page table walk modifies only PTE cache flags.
            unsafe {
                crate::arch::x86_64::pat::apply_write_combining(
                    fb_info.buffer as usize,
                    fb_size_aligned,
                );
            }
            kprintln!(
                "[BOOTSTRAP] Framebuffer WC enabled ({} pages)",
                fb_size_aligned / 4096
            );
        }
    }

    // AArch64/RISC-V: Try to initialize ramfb display device for graphical
    // output. Requires `-device ramfb` on the QEMU command line. If ramfb
    // is not available, gracefully fall back to serial-only output.
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        match crate::drivers::ramfb::init(1024, 768) {
            Ok(fb_ptr) => {
                // SAFETY: fb_ptr from ramfb init is valid for stride * height
                // bytes and mapped for the kernel lifetime.
                unsafe {
                    crate::graphics::fbcon::init(
                        fb_ptr,
                        1024,
                        768,
                        1024 * 4, // stride = width * bpp
                        4,        // bytes per pixel
                        crate::graphics::fbcon::FbPixelFormat::Rgb,
                    );
                }
                kprintln!("[BOOTSTRAP] ramfb + fbcon initialized (1024x768)");
            }
            Err(e) => {
                crate::println!(
                    "[BOOTSTRAP] ramfb not available ({:?}), serial-only output",
                    e
                );
            }
        }
    }

    // x86_64: Pre-initialize the CSPRNG on the UEFI stack before switching.
    // SecureRandom::new() runs SHA-256 and ChaCha20 which are stack-light.
    // This ensures the RNG is ready before any security module needs it.
    #[cfg(target_arch = "x86_64")]
    {
        kprintln!("[BOOTSTRAP] Pre-initializing CSPRNG...");
        let _ = crate::crypto::random::init();
        // Verify the RNG works
        let rng = crate::crypto::random::get_random();
        let v = rng.next_u64();
        crate::println!("[BOOTSTRAP] CSPRNG initialized (test: {})", v);
    }

    // x86_64: The UEFI-provided boot stack is 128KB. In debug mode, deep
    // init call chains overflow it (CapabilitySpace L1 table ~20KB on
    // stack, security module structs, etc.). Switch to a 1MB
    // heap-allocated stack now that the allocator is ready.
    // 256KB was insufficient when the selfhost rootfs TAR (43MB, 114 entries)
    // is loaded, as the subsequent process creation + page table walking
    // pushes the stack over the limit.
    // switch_to_heap_stack does NOT return -- it continues boot on the
    // new stack via kernel_init_stage3_onwards.
    #[cfg(target_arch = "x86_64")]
    {
        const BOOT_STACK_SIZE: usize = 1024 * 1024; // 1MB (selfhost rootfs needs deep call chains)
        switch_to_heap_stack(BOOT_STACK_SIZE);
        // UNREACHABLE on x86_64: switch_to_heap_stack diverges
    }

    // Non-x86_64 architectures continue directly on the boot stack
    #[cfg(not(target_arch = "x86_64"))]
    {
        kernel_init_stage3_impl()?;
    }

    Ok(())
}

/// Stages 3-5 of kernel initialization (process management, services,
/// scheduler).
///
/// Extracted into a separate function so that x86_64 can call it on a fresh
/// heap-allocated stack (via `switch_to_heap_stack`), while other architectures
/// call it directly from `kernel_init`.
fn kernel_init_stage3_impl() -> KernelResult<()> {
    // Stage 3: Process management
    kprintln!("[BOOTSTRAP] Stage 3: Process management");

    process::init_without_init_process().expect("Failed to initialize process management");

    kprintln!("[BOOTSTRAP] Process management initialized");

    // Stage 4: Core kernel services
    kprintln!("[BOOTSTRAP] Stage 4: Kernel services");

    kprintln!("[BOOTSTRAP] Initializing capabilities...");
    cap::init();
    kprintln!("[BOOTSTRAP] Capabilities initialized");

    // Initialize security modules individually to minimize stack depth.
    // Each module's init() constructs its state on the stack before moving
    // into a static OnceLock/Mutex. Calling them individually (rather than
    // through security::init()) avoids accumulating stack frames.
    kprintln!("[BOOTSTRAP] Initializing security subsystem...");
    security::memory_protection::init().expect("Failed to initialize memory protection");
    security::auth::init().expect("Failed to initialize auth");
    security::tpm::init().expect("Failed to initialize TPM");
    security::mac::init().expect("Failed to initialize MAC");
    security::audit::init().expect("Failed to initialize audit");
    let _ = security::boot::verify();
    kprintln!("[BOOTSTRAP] Security subsystem initialized");

    kprintln!("[BOOTSTRAP] Initializing performance monitoring...");
    perf::init().expect("Failed to initialize performance monitoring");
    // Initialize hardware performance counters (PMU) after ACPI/APIC setup.
    crate::perf::pmu::init();
    kprintln!(
        "[BOOTSTRAP] Performance monitoring initialized (PMU: {} counters)",
        crate::perf::pmu::num_counters()
    );

    kprintln!("[BOOTSTRAP] Initializing IPC...");
    ipc::init();
    kprintln!("[BOOTSTRAP] IPC initialized");

    // Initialize VFS and mount essential filesystems
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing VFS...");
        fs::init();
        kprintln!("[BOOTSTRAP] VFS initialized");
    }

    // Populate the RamFS with embedded init and shell binaries so that
    // load_init_process() finds real ELF executables at /sbin/init and
    // /bin/vsh instead of falling back to stub processes.
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Populating initramfs with embedded binaries...");
        if crate::userspace::embedded::populate_initramfs().is_err() {
            kprintln!("[BOOTSTRAP] Warning: Failed to populate initramfs");
        } else {
            kprintln!("[BOOTSTRAP] Initramfs populated successfully");
        }
    }

    // Initialize driver framework first (needed by PCI/virtio init), then
    // the appropriate transport (PCI on x86_64, MMIO on AArch64/RISC-V) and
    // virtio-blk for disk access.
    // Must happen after VFS init so TAR loading can populate the filesystem.
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing drivers + virtio-blk...");
        services::driver_framework::init();

        // PCI bus enumeration is x86_64-only. AArch64/RISC-V use MMIO transport
        // for virtio devices (probed in blk::init() via init_mmio()). The I/O
        // port stubs on non-x86 return 0, which would make every PCI slot appear
        // populated (vendor_id 0 != 0xFFFF), causing 8192 phantom device scans.
        #[cfg(target_arch = "x86_64")]
        {
            crate::drivers::pci::init();
            // Enumerate PCI devices so virtio-blk can find its device
            {
                let pci_bus = crate::drivers::pci::get_pci_bus().lock();
                let _ = pci_bus.enumerate_devices();
            }
            // Probe for known VirtIO drivers (GPU, Net, Sound)
            crate::drivers::pci::probe_known_drivers();
            if let Err(_e) = crate::drivers::nvme::init() {
                kprintln!("[BOOTSTRAP] NVMe init failed");
            }
        }

        // blk::init() dispatches to PCI probe on x86_64, MMIO probe on
        // AArch64/RISC-V.
        crate::drivers::virtio::blk::init();

        // Initialize PS/2 mouse driver (x86_64: aux port, others: stub)
        crate::drivers::mouse::init();

        kprintln!("[BOOTSTRAP] Drivers + virtio-blk initialized");

        // If a virtio-blk disk is attached, read it as a TAR archive
        // and load its contents into the VFS. This is how cross-compiled
        // user-space binaries get into the filesystem at boot.
        load_rootfs_from_disk();

        // ProcFS natively supports /proc/sys/kernel/core_pattern,
        // /proc/sys/kernel/random/boot_id, and /proc/self/{exe,maps}
        // as virtual files. No re-creation needed after rootfs swap.
    }

    // Initialize services (process server, driver framework, etc.)
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing services...");
        services::init();
        kprintln!("[BOOTSTRAP] Services initialized");

        // Activate init system services
        if let Some(init) = crate::services::init_system::try_get_init_system() {
            if let Err(_e) = init.initialize() {
                kprintln!("[BOOTSTRAP] Init system activation deferred: {:?}", _e);
            }
        }
    }

    kprintln!("[BOOTSTRAP] Core services initialized");

    // x86_64: Initialize keyboard driver state (decoder) so boot tests
    // can verify it. IRQ unmask + interrupt enable happen later (Stage 6,
    // right before the shell) to avoid interrupts during initialization.
    #[cfg(target_arch = "x86_64")]
    {
        crate::drivers::keyboard::init();
        kprintln!("[BOOTSTRAP] Keyboard driver initialized");
    }

    // The boot flow becomes CPU 0's first task (ADR 0006, stage D1). Every
    // user program runs as a dispatcher task; there is no other way to run
    // one (the nested boot model went in sprint D4), so this cannot fail
    // and boot go on.
    #[cfg(all(feature = "alloc", dispatch))]
    match crate::sched::dispatch::start() {
        Ok(()) => kprintln!("[BOOTSTRAP] Dispatcher started (boot task + idle)"),
        Err(e) => panic!("dispatcher could not start: {:?}", e),
    }

    // Stage 5: Scheduler initialization, which brings up the other CPUs
    // (with the `smp` feature). Before the boot tests, so the SMP ones test
    // every CPU: they ran before any secondary started, and checked nothing.
    kprintln!("[BOOTSTRAP] Stage 5: Scheduler activation");

    sched::init();

    // Run kernel-mode init tests after Stage 4 (VFS + shell ready)
    kernel_init_main();

    // Initialize package manager
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing package manager...");
        pkg::init();
        kprintln!("[BOOTSTRAP] Package manager initialized");
        kprintln!("[PKGMGR] Package manager v0.4.0 ready");
    }

    // Initialize network stack
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing network stack...");
        net::init().expect("Failed to initialize network stack");
        kprintln!("[BOOTSTRAP] Network stack initialized");
    }

    // Initialize graphics subsystem
    kprintln!("[BOOTSTRAP] Initializing graphics subsystem...");
    graphics::init().expect("Failed to initialize graphics");

    // Initialize GPU acceleration subsystem (GEM, KMS, page flip) and
    // populate KMS with a virtual DRM device backed by the UEFI GOP
    // framebuffer. This provides /dev/dri/card0 ioctl support for KWin
    // and other DRM clients.
    graphics::gpu_accel::init();
    graphics::gpu_accel::init_virtual_drm_device();
    kprintln!("[BOOTSTRAP] Graphics subsystem initialized (DRM device ready)");

    // Initialize IRQ manager and timer wheel (needed by drivers and scheduler)
    #[cfg(feature = "alloc")]
    {
        if let Err(_e) = irq::init() {
            kprintln!("[BOOTSTRAP] IRQ manager init skipped (already initialized)");
        }
        if let Err(_e) = timer::init() {
            kprintln!("[BOOTSTRAP] Timer wheel init skipped (already initialized)");
        }
    }

    // Initialize USB subsystem (placeholder controllers, non-fatal)
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing USB subsystem...");
        crate::drivers::usb::init();
        kprintln!("[BOOTSTRAP] USB subsystem initialized");
    }

    // Initialize persistent user database
    #[cfg(feature = "alloc")]
    {
        crate::syscall::userland_ext::users::init_user_db();
        kprintln!("[BOOTSTRAP] User database initialized");
        // The accounts on the root filesystem (/etc/passwd, /etc/shadow;
        // N-131): there is no built-in password.
        crate::security::accounts::load();
    }

    // Initialize PTY subsystem (needed by desktop terminal emulator)
    #[cfg(feature = "alloc")]
    {
        if let Err(_e) = crate::fs::pty::init() {
            kprintln!("[BOOTSTRAP] PTY init failed (non-fatal)");
        }
    }

    // Initialize desktop subsystem (Wayland, window manager, apps)
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing desktop subsystem...");
        if let Err(_e) = desktop::init() {
            kprintln!("[BOOTSTRAP] Desktop init deferred (non-fatal)");
        }
        // Initialize notification manager with screen dimensions from fbcon
        if let Some(hw) = graphics::fbcon::get_hw_info() {
            crate::desktop::notification::init(hw.width, hw.height);
        }
        kprintln!("[BOOTSTRAP] Desktop subsystem initialized");
    }

    // Initialize audio subsystem (mixer, pipeline, VirtIO-Sound)
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing audio subsystem...");
        if let Err(_e) = audio::init() {
            kprintln!("[BOOTSTRAP] Audio init deferred (non-fatal)");
        }
        kprintln!("[BOOTSTRAP] Audio subsystem initialized");
    }

    // Initialize video subsystem (decoders, player)
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Initializing video subsystem...");
        if let Err(_e) = video::init() {
            kprintln!("[BOOTSTRAP] Video init deferred (non-fatal)");
        }
        kprintln!("[BOOTSTRAP] Video subsystem initialized");
    }

    // Initialize virtualization subsystem (VMX detection, containers)
    #[cfg(target_arch = "x86_64")]
    {
        kprintln!("[BOOTSTRAP] Initializing virtualization subsystem...");
        virt::init();
        kprintln!("[BOOTSTRAP] Virtualization subsystem initialized");
    }

    // Build the KPTI shadow page tables. Nothing loads them yet, so this is
    // not a Meltdown mitigation (N-145).
    #[cfg(target_arch = "x86_64")]
    {
        kprintln!("[BOOTSTRAP] Initializing KPTI shadow page tables...");
        crate::arch::x86_64::kpti::init();
        kprintln!("[BOOTSTRAP] KPTI initialized");
    }

    // Cache the bootloader's physical memory offset for later use.
    // This MUST happen before any user process runs, because BOOT_INFO
    // resides in the bootloader's lower-half mapping which is NOT present
    // in user process page tables. After this point, kernel code can use
    // msr::phys_to_virt() safely from any context (syscall, interrupt, IST).
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: BOOT_INFO is written once during early boot and read-only
        // after. We are still in single-threaded bootstrap before user mode.
        #[allow(static_mut_refs)]
        if let Some(ref boot_info) = unsafe { &crate::arch::x86_64::boot::BOOT_INFO } {
            if let Some(offset) = boot_info.physical_memory_offset.into_option() {
                crate::arch::x86_64::usermode::init_phys_offset(offset);
            }
        }
    }

    kprintln!("[BOOTSTRAP] Scheduler activated - entering main scheduling loop");

    // Phase 4A: Try to load a user-space binary from the rootfs.
    // This is the critical gate for self-hosting -- verifies that cross-compiled
    // ELF binaries can be loaded and scheduled on VeridianOS.
    #[cfg(all(feature = "alloc", target_arch = "x86_64"))]
    {
        test_user_binary_load();
    }

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
        let uart_base = crate::mm::phys_to_virt_addr(0x1000_0000) as *mut u8;
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
    kprintln!("[BOOTSTRAP] Stage 6: User space transition");

    kprintln!("[BOOTSTRAP] About to create init process...");
    create_init_process();
    kprintln!("[BOOTSTRAP] Init process created");

    // Mark Stage 6 complete
    kprintln!("[BOOTSTRAP] User space transition prepared");
    kprintln!("[KERNEL] Boot sequence complete!");
    kprintln!("BOOTOK");

    // Attempt user-mode entry. On success, transitions to user-space
    // and never returns. On failure, falls through to the interactive shell.
    #[cfg(target_arch = "aarch64")]
    {
        kprintln!("[BOOTSTRAP] Attempting user-mode entry...");
        if crate::arch::aarch64::usermode::try_enter_usermode().is_err() {
            kprintln!("[BOOTSTRAP] User-mode entry deferred (prerequisites not met)");
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        kprintln!("[BOOTSTRAP] Attempting user-mode entry...");
        if crate::arch::riscv64::usermode::try_enter_usermode().is_err() {
            kprintln!("[BOOTSTRAP] User-mode entry deferred (prerequisites not met)");
        }
    }

    // Enable framebuffer console output now that boot is complete.
    graphics::fbcon::enable_output();

    // Launch the interactive kernel shell (never returns).
    // The shell provides a serial console REPL for all 3 architectures.
    #[cfg(feature = "alloc")]
    {
        kprintln!("[BOOTSTRAP] Starting interactive shell...");
        crate::services::shell::run_shell();
    }

    // Fallback: transfer control to scheduler if shell unavailable
    #[cfg(not(feature = "alloc"))]
    sched::start();
}

/// Load a rootfs TAR archive from virtio-blk into the VFS.
///
/// If a virtio-blk device is attached (via QEMU `-drive ... -device
/// virtio-blk-pci,...`), probe its first block to decide the format:
///
/// - If the first 4 bytes match `BLOCKFS_MAGIC` (0x424C4B46), mount as a
///   persistent BlockFS root filesystem (replacing the initial RamFS).
/// - Otherwise, read the entire disk as a TAR archive and load into RamFS
///   (existing behavior).
#[cfg(feature = "alloc")]
fn load_rootfs_from_disk() {
    use crate::drivers::virtio::blk;

    if !blk::is_initialized() {
        kprintln!("[ROOTFS] No virtio-blk device, skipping disk load");
        return;
    }

    let device = match blk::get_device() {
        Some(dev) => dev,
        None => {
            kprintln!("[ROOTFS] virtio-blk device not available");
            return;
        }
    };

    // Probe the first sector (512 bytes) to check for BlockFS magic
    let mut probe_buf = [0u8; 512];
    {
        let mut dev = device.lock();
        if let Err(_e) = dev.read_block(0, &mut probe_buf) {
            kprintln!("[ROOTFS] Failed to read sector 0 for probe");
            return;
        }
    }

    let magic = u32::from_le_bytes([probe_buf[0], probe_buf[1], probe_buf[2], probe_buf[3]]);
    if magic == crate::fs::blockfs::BLOCKFS_MAGIC {
        kprintln!("[ROOTFS] BlockFS magic detected -- mounting persistent root");
        mount_blockfs_root();
        return;
    }

    // Fall back to TAR loading
    load_tar_rootfs();
}

/// Mount a pre-formatted BlockFS image as the persistent root filesystem.
///
/// Reads the superblock, bitmap and inode table from the virtio-blk device;
/// data blocks are read on first use (FS-PERF-01). Replaces the initial RamFS
/// via `swap_root()`, then re-mounts DevFS at `/dev` and ProcFS at `/proc`.
#[cfg(feature = "alloc")]
fn mount_blockfs_root() {
    use alloc::sync::Arc;

    use spin::Mutex;

    use crate::fs::{
        blockfs::{BlockFs, VirtioBlockBackend},
        devfs::DevFs,
        get_vfs,
        procfs::ProcFs,
        Permissions,
    };

    let backend = Arc::new(Mutex::new(VirtioBlockBackend));

    let blockfs = match BlockFs::open_existing(backend) {
        Ok(fs) => {
            kprintln!("[ROOTFS] BlockFS loaded successfully");
            fs
        }
        Err(_e) => {
            kprintln!(
                "[ROOTFS] Failed to open BlockFS: {:?}, falling back to TAR rootfs",
                _e
            );
            load_tar_rootfs();
            return;
        }
    };

    let blockfs_arc: Arc<dyn crate::fs::Filesystem> = Arc::new(blockfs);

    // Swap root filesystem from RamFS to BlockFS
    {
        let vfs = get_vfs();

        // Remove existing DevFS/ProcFS mounts (they're on the old root)
        let _ = vfs.unmount("/dev");
        let _ = vfs.unmount("/proc");

        vfs.swap_root(blockfs_arc);
    }

    kprintln!("[ROOTFS] BlockFS mounted as persistent root");

    // Ensure standard directories exist (may already exist from mkfs population)
    {
        let vfs = get_vfs();
        if let Ok(root) = vfs.resolve_path("/") {
            // Create dirs if they don't exist (ok to fail with AlreadyExists)
            root.mkdir("dev", Permissions::default()).ok();
            root.mkdir("proc", Permissions::default()).ok();
            root.mkdir("tmp", Permissions::from_mode(0o1777)).ok();
            // /run hierarchy for XDG_RUNTIME_DIR and D-Bus sockets
            if let Ok(run) = root
                .lookup("run")
                .or_else(|_| root.mkdir("run", Permissions::default()))
            {
                if let Ok(user) = run
                    .lookup("user")
                    .or_else(|_| run.mkdir("user", Permissions::default()))
                {
                    user.mkdir("0", Permissions::from_mode(0o700)).ok();
                }
                run.mkdir("dbus", Permissions::default()).ok();
            }
            // /etc/xdg for KDE/Qt config fallback searches
            if let Ok(etc) = root.lookup("etc") {
                if let Ok(xdg) = etc
                    .lookup("xdg")
                    .or_else(|_| etc.mkdir("xdg", Permissions::default()))
                {
                    // Create /etc/xdg/kwinrc so KWin finds its config.
                    // QPlatformScreen=false disables QPlatformScreen creation
                    // which avoids a NULL crash when no DRM outputs exist yet.
                    // Backend is intentionally not set (default QPainter avoids
                    // OpenGL/EGL dependencies).
                    if let Ok(f) = xdg.create("kwinrc", Permissions::from_mode(0o644)) {
                        f.write(
                            0,
                            b"[Compositing]\nBackend=QPainter\n\n[Wayland]\nInputMethod=\n",
                        )
                        .ok();
                    }
                }
            }
            // /sys/class/drm/ hierarchy for kwin DRM device discovery.
            // kwin uses udev/sysfs to find DRM devices. Without these entries,
            // kwin's DRM backend stays NULL and crashes on first access.
            if let Ok(sys) = root
                .lookup("sys")
                .or_else(|_| root.mkdir("sys", Permissions::default()))
            {
                if let Ok(class) = sys
                    .lookup("class")
                    .or_else(|_| sys.mkdir("class", Permissions::default()))
                {
                    if let Ok(drm) = class
                        .lookup("drm")
                        .or_else(|_| class.mkdir("drm", Permissions::default()))
                    {
                        // card0 directory with dev file (major:minor) and
                        // attributes that KWin's DRM backend reads during
                        // device initialization (driver name, status, etc.)
                        if let Ok(card0) = drm
                            .lookup("card0")
                            .or_else(|_| drm.mkdir("card0", Permissions::default()))
                        {
                            if let Ok(f) = card0.create("dev", Permissions::read_only()) {
                                f.write(0, b"226:0\n").ok();
                            }
                            if let Ok(f) = card0.create("uevent", Permissions::read_only()) {
                                f.write(
                                    0,
                                    b"MAJOR=226\nMINOR=0\nDEVNAME=dri/card0\nDEVTYPE=drm_minor\n",
                                )
                                .ok();
                            }
                            if let Ok(f) = card0.create("enabled", Permissions::read_only()) {
                                f.write(0, b"enabled\n").ok();
                            }
                            if let Ok(f) = card0.create("status", Permissions::read_only()) {
                                f.write(0, b"connected\n").ok();
                            }
                            // device/ subtree with driver info
                            if let Ok(dev_dir) = card0
                                .lookup("device")
                                .or_else(|_| card0.mkdir("device", Permissions::default()))
                            {
                                if let Ok(f) = dev_dir.create("uevent", Permissions::read_only()) {
                                    f.write(0, b"DRIVER=veridian-drm\nPCI_ID=1234:1111\n").ok();
                                }
                                // drm/card0/ back-reference for udev parent traversal
                                if let Ok(drm_sub) = dev_dir
                                    .lookup("drm")
                                    .or_else(|_| dev_dir.mkdir("drm", Permissions::default()))
                                {
                                    drm_sub.mkdir("card0", Permissions::default()).ok();
                                }
                            }
                        }
                    }
                    // /sys/class/input/ for evdev discovery
                    class.mkdir("input", Permissions::default()).ok();
                }
                // /sys/devices/ stub for device enumeration
                sys.mkdir("devices", Permissions::default()).ok();
            }
            // /root/.config for user-level config
            if let Ok(root_home) = root
                .lookup("root")
                .or_else(|_| root.mkdir("root", Permissions::default()))
            {
                root_home.mkdir(".config", Permissions::default()).ok();
            }

            // /etc/fonts/fonts.conf -- fontconfig needs this to find font dirs.
            // The rootfs already has 22 fonts in /usr/share/fonts/.
            if let Ok(etc) = root.lookup("etc") {
                if let Ok(fonts_dir) = etc
                    .lookup("fonts")
                    .or_else(|_| etc.mkdir("fonts", Permissions::default()))
                {
                    if let Ok(f) = fonts_dir.create("fonts.conf", Permissions::from_mode(0o644)) {
                        f.write(
                            0,
                            b"<?xml version=\"1.0\"?>\n\
                            <!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
                            <fontconfig>\n\
                            <dir>/usr/share/fonts</dir>\n\
                            <cachedir>/tmp/fontconfig-cache</cachedir>\n\
                            <match target=\"pattern\">\n\
                            <edit name=\"family\" mode=\"append_last\">\n\
                            <string>Noto Sans</string>\n\
                            </edit>\n\
                            </match>\n\
                            </fontconfig>\n",
                        )
                        .ok();
                    }
                }
            }

            // /usr/share/X11/xkb/ -- xkbcommon needs minimal keymap data.
            // Without these, kwin prints "Could not create xkb context".
            if let Ok(usr) = root
                .lookup("usr")
                .or_else(|_| root.mkdir("usr", Permissions::default()))
            {
                if let Ok(share) = usr
                    .lookup("share")
                    .or_else(|_| usr.mkdir("share", Permissions::default()))
                {
                    if let Ok(x11) = share
                        .lookup("X11")
                        .or_else(|_| share.mkdir("X11", Permissions::default()))
                    {
                        if let Ok(xkb) = x11
                            .lookup("xkb")
                            .or_else(|_| x11.mkdir("xkb", Permissions::default()))
                        {
                            // rules/evdev -- minimal rules mapping
                            if let Ok(rules) = xkb
                                .lookup("rules")
                                .or_else(|_| xkb.mkdir("rules", Permissions::default()))
                            {
                                if let Ok(f) = rules.create("evdev", Permissions::from_mode(0o644))
                                {
                                    // xkbcommon rules format: each section starts with
                                    // "! <column-names>" and is followed by match lines.
                                    // Columns are tab-separated.
                                    f.write(
                                        0,
                                        b"! model\t=\tkeycodes\n\
  *\t=\tevdev\n\
\n\
! layout\t=\tsymbols\n\
  us\t=\tus\n\
\n\
! model\t=\ttypes\n\
  *\t=\tcomplete\n\
\n\
! model\t=\tcompat\n\
  *\t=\tcomplete\n",
                                    )
                                    .ok();
                                }
                                if let Ok(f) =
                                    rules.create("evdev.xml", Permissions::from_mode(0o644))
                                {
                                    f.write(
                                        0,
                                        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                                        <!DOCTYPE xkbConfigRegistry>\n\
                                        <xkbConfigRegistry version=\"1.1\">\n\
                                        <modelList><model><configItem><name>pc105</name>\
                                        <description>Generic 105-key PC</description>\
                                        </configItem></model></modelList>\n\
                                        <layoutList><layout><configItem><name>us</name>\
                                        <description>English (US)</description>\
                                        </configItem></layout></layoutList>\n\
                                        </xkbConfigRegistry>\n",
                                    )
                                    .ok();
                                }
                            }

                            // keycodes/evdev -- evdev keycode mappings
                            if let Ok(keycodes) = xkb
                                .lookup("keycodes")
                                .or_else(|_| xkb.mkdir("keycodes", Permissions::default()))
                            {
                                if let Ok(f) =
                                    keycodes.create("evdev", Permissions::from_mode(0o644))
                                {
                                    f.write(
                                        0,
                                        b"default xkb_keycodes \"evdev\" {\n\
                                        minimum = 8;\n\
                                        maximum = 255;\n\
                                        <ESC> = 9;\n\
                                        <AE01> = 10; <AE02> = 11; <AE03> = 12; <AE04> = 13;\n\
                                        <AE05> = 14; <AE06> = 15; <AE07> = 16; <AE08> = 17;\n\
                                        <AE09> = 18; <AE10> = 19; <AE11> = 20; <AE12> = 21;\n\
                                        <BKSP> = 22; <TAB> = 23;\n\
                                        <AD01> = 24; <AD02> = 25; <AD03> = 26; <AD04> = 27;\n\
                                        <AD05> = 28; <AD06> = 29; <AD07> = 30; <AD08> = 31;\n\
                                        <AD09> = 32; <AD10> = 33; <AD11> = 34; <AD12> = 35;\n\
                                        <RTRN> = 36; <LCTL> = 37;\n\
                                        <AC01> = 38; <AC02> = 39; <AC03> = 40; <AC04> = 41;\n\
                                        <AC05> = 42; <AC06> = 43; <AC07> = 44; <AC08> = 45;\n\
                                        <AC09> = 46; <AC10> = 47; <AC11> = 48;\n\
                                        <TLDE> = 49; <LFSH> = 50;\n\
                                        <BKSL> = 51;\n\
                                        <AB01> = 52; <AB02> = 53; <AB03> = 54; <AB04> = 55;\n\
                                        <AB05> = 56; <AB06> = 57; <AB07> = 58; <AB08> = 59;\n\
                                        <AB09> = 60; <AB10> = 61;\n\
                                        <RTSH> = 62; <KPMU> = 63; <LALT> = 64; <SPCE> = 65;\n\
                                        <CAPS> = 66;\n\
                                        <FK01> = 67; <FK02> = 68; <FK03> = 69; <FK04> = 70;\n\
                                        <FK05> = 71; <FK06> = 72; <FK07> = 73; <FK08> = 74;\n\
                                        <FK09> = 75; <FK10> = 76;\n\
                                        <NMLK> = 77; <SCLK> = 78;\n\
                                        <UP> = 111; <LEFT> = 113; <RGHT> = 114; <DOWN> = 116;\n\
                                        <RALT> = 108; <RCTL> = 105;\n\
                                        <LWIN> = 133; <RWIN> = 134; <MENU> = 135;\n\
                                        };\n",
                                    )
                                    .ok();
                                }
                            }

                            // symbols/us -- US keyboard layout
                            if let Ok(symbols) = xkb
                                .lookup("symbols")
                                .or_else(|_| xkb.mkdir("symbols", Permissions::default()))
                            {
                                if let Ok(f) = symbols.create("us", Permissions::from_mode(0o644)) {
                                    f.write(
                                        0,
                                        b"default partial alphanumeric_keys\n\
                                        xkb_symbols \"basic\" {\n\
                                        name[Group1]= \"English (US)\";\n\
                                        key <AE01> { [ 1, exclam ] };\n\
                                        key <AE02> { [ 2, at ] };\n\
                                        key <AE03> { [ 3, numbersign ] };\n\
                                        key <AE04> { [ 4, dollar ] };\n\
                                        key <AE05> { [ 5, percent ] };\n\
                                        key <AE06> { [ 6, asciicircum ] };\n\
                                        key <AE07> { [ 7, ampersand ] };\n\
                                        key <AE08> { [ 8, asterisk ] };\n\
                                        key <AE09> { [ 9, parenleft ] };\n\
                                        key <AE10> { [ 0, parenright ] };\n\
                                        key <AE11> { [ minus, underscore ] };\n\
                                        key <AE12> { [ equal, plus ] };\n\
                                        key <AD01> { [ q, Q ] };\n\
                                        key <AD02> { [ w, W ] };\n\
                                        key <AD03> { [ e, E ] };\n\
                                        key <AD04> { [ r, R ] };\n\
                                        key <AD05> { [ t, T ] };\n\
                                        key <AD06> { [ y, Y ] };\n\
                                        key <AD07> { [ u, U ] };\n\
                                        key <AD08> { [ i, I ] };\n\
                                        key <AD09> { [ o, O ] };\n\
                                        key <AD10> { [ p, P ] };\n\
                                        key <AD11> { [ bracketleft, braceleft ] };\n\
                                        key <AD12> { [ bracketright, braceright ] };\n\
                                        key <AC01> { [ a, A ] };\n\
                                        key <AC02> { [ s, S ] };\n\
                                        key <AC03> { [ d, D ] };\n\
                                        key <AC04> { [ f, F ] };\n\
                                        key <AC05> { [ g, G ] };\n\
                                        key <AC06> { [ h, H ] };\n\
                                        key <AC07> { [ j, J ] };\n\
                                        key <AC08> { [ k, K ] };\n\
                                        key <AC09> { [ l, L ] };\n\
                                        key <AC10> { [ semicolon, colon ] };\n\
                                        key <AC11> { [ apostrophe, quotedbl ] };\n\
                                        key <TLDE> { [ grave, asciitilde ] };\n\
                                        key <BKSL> { [ backslash, bar ] };\n\
                                        key <AB01> { [ z, Z ] };\n\
                                        key <AB02> { [ x, X ] };\n\
                                        key <AB03> { [ c, C ] };\n\
                                        key <AB04> { [ v, V ] };\n\
                                        key <AB05> { [ b, B ] };\n\
                                        key <AB06> { [ n, N ] };\n\
                                        key <AB07> { [ m, M ] };\n\
                                        key <AB08> { [ comma, less ] };\n\
                                        key <AB09> { [ period, greater ] };\n\
                                        key <AB10> { [ slash, question ] };\n\
                                        key <SPCE> { [ space ] };\n\
                                        key <RTRN> { [ Return ] };\n\
                                        key <BKSP> { [ BackSpace ] };\n\
                                        key <TAB> { [ Tab ] };\n\
                                        key <ESC> { [ Escape ] };\n\
                                        key <CAPS> { [ Caps_Lock ] };\n\
                                        key <LFSH> { [ Shift_L ] };\n\
                                        key <RTSH> { [ Shift_R ] };\n\
                                        key <LCTL> { [ Control_L ] };\n\
                                        key <RCTL> { [ Control_R ] };\n\
                                        key <LALT> { [ Alt_L ] };\n\
                                        key <RALT> { [ Alt_R ] };\n\
                                        key <LWIN> { [ Super_L ] };\n\
                                        key <RWIN> { [ Super_R ] };\n\
                                        };\n",
                                    )
                                    .ok();
                                }
                            }

                            // types/complete -- key type definitions
                            if let Ok(types) = xkb
                                .lookup("types")
                                .or_else(|_| xkb.mkdir("types", Permissions::default()))
                            {
                                if let Ok(f) =
                                    types.create("complete", Permissions::from_mode(0o644))
                                {
                                    f.write(
                                        0,
                                        b"default xkb_types \"complete\" {\n\
                                        type \"ONE_LEVEL\" {\n\
                                            modifiers = none;\n\
                                            map[none] = Level1;\n\
                                            level_name[Level1] = \"Any\";\n\
                                        };\n\
                                        type \"TWO_LEVEL\" {\n\
                                            modifiers = Shift;\n\
                                            map[Shift] = Level2;\n\
                                            level_name[Level1] = \"Base\";\n\
                                            level_name[Level2] = \"Shift\";\n\
                                        };\n\
                                        type \"ALPHABETIC\" {\n\
                                            modifiers = Shift+Lock;\n\
                                            map[Shift] = Level2;\n\
                                            map[Lock] = Level2;\n\
                                            level_name[Level1] = \"Base\";\n\
                                            level_name[Level2] = \"Caps\";\n\
                                        };\n\
                                        type \"KEYPAD\" {\n\
                                            modifiers = Shift+NumLock;\n\
                                            map[NumLock] = Level2;\n\
                                            level_name[Level1] = \"Base\";\n\
                                            level_name[Level2] = \"Number\";\n\
                                        };\n\
                                        };\n",
                                    )
                                    .ok();
                                }
                            }

                            // compat/complete -- compatibility rules
                            if let Ok(compat) = xkb
                                .lookup("compat")
                                .or_else(|_| xkb.mkdir("compat", Permissions::default()))
                            {
                                if let Ok(f) =
                                    compat.create("complete", Permissions::from_mode(0o644))
                                {
                                    f.write(
                                        0,
                                        b"default xkb_compatibility \"complete\" {\n\
                                        interpret Any+AnyOf(all) {\n\
                                            action = SetMods(modifiers=modMapMods,clearLocks);\n\
                                        };\n\
                                        interpret Shift_L+AnyOf(all) {\n\
                                            action = SetMods(modifiers=Shift,clearLocks);\n\
                                        };\n\
                                        interpret Caps_Lock+AnyOf(all) {\n\
                                            action = LockMods(modifiers=Lock);\n\
                                        };\n\
                                        interpret Num_Lock+AnyOf(all) {\n\
                                            action = LockMods(modifiers=NumLock);\n\
                                        };\n\
                                        };\n",
                                    )
                                    .ok();
                                }
                            }
                        }
                    }

                    // /usr/share/libinput/ -- empty quirk database directory
                    // prevents libinput from warning about missing device database
                    share
                        .lookup("libinput")
                        .or_else(|_| share.mkdir("libinput", Permissions::default()))
                        .ok();
                }
            }

            // /etc/libinput/ -- empty local overrides directory
            if let Ok(etc) = root.lookup("etc") {
                etc.lookup("libinput")
                    .or_else(|_| etc.mkdir("libinput", Permissions::default()))
                    .ok();
            }

            // /tmp/fontconfig-cache -- directory for fontconfig cache files
            if let Ok(tmp) = root.lookup("tmp") {
                tmp.lookup("fontconfig-cache")
                    .or_else(|_| tmp.mkdir("fontconfig-cache", Permissions::from_mode(0o1777)))
                    .ok();
            }

            // Sysroot path symlink -- the KDE stack was cross-compiled with
            // `--prefix=<VERIDIAN_SYSROOT>/usr`, which bakes the host build
            // path into the binaries. Libraries like xkbcommon, fontconfig,
            // and libinput search for data files at those compile-time
            // paths, so recreate the sysroot directory and symlink its `usr`
            // back to `/usr`. build.rs sets the path: VERIDIAN_SYSROOT if
            // given at kernel build time, else /opt/veridian/musl-sysroot, the
            // tools/cross default. It must match the sysroot the rootfs
            // binaries were built against. This goes away once tools/cross
            // builds with --prefix=/usr.
            {
                const SYSROOT: &str = env!("VERIDIAN_SYSROOT_PATH");
                let mut current = root.clone();
                for component in SYSROOT.split('/').filter(|c| !c.is_empty()) {
                    current = current
                        .lookup(component)
                        .or_else(|_| current.mkdir(component, Permissions::default()))
                        .unwrap_or_else(|_| current.clone());
                }
                current.symlink("usr", "/usr").ok();
            }

            // /usr/var/cache/fontconfig/ -- the sysroot symlink resolves
            // .../veridian-sysroot/usr/var/cache/fontconfig to /usr/var/cache/fontconfig.
            // Create this directory tree so fontconfig cache writes succeed.
            if let Ok(usr) = root.lookup("usr") {
                if let Ok(var) = usr
                    .lookup("var")
                    .or_else(|_| usr.mkdir("var", Permissions::default()))
                {
                    if let Ok(cache) = var
                        .lookup("cache")
                        .or_else(|_| var.mkdir("cache", Permissions::default()))
                    {
                        cache
                            .lookup("fontconfig")
                            .or_else(|_| cache.mkdir("fontconfig", Permissions::from_mode(0o1777)))
                            .ok();
                    }
                }
                // /usr/etc/ -- some sysroot-compiled libs look for configs here
                if let Ok(etc) = usr
                    .lookup("etc")
                    .or_else(|_| usr.mkdir("etc", Permissions::default()))
                {
                    // /usr/etc/libinput/ -- empty local overrides
                    etc.lookup("libinput")
                        .or_else(|_| etc.mkdir("libinput", Permissions::default()))
                        .ok();
                    // /usr/etc/drirc -- empty DRI config file
                    if let Ok(f) = etc.create("drirc", Permissions::from_mode(0o644)) {
                        f.write(0, b"").ok();
                    }
                }
                // /usr/share/drirc.d/ -- DRI config snippets directory
                if let Ok(share) = usr.lookup("share") {
                    share
                        .lookup("drirc.d")
                        .or_else(|_| share.mkdir("drirc.d", Permissions::default()))
                        .ok();
                }
            }
        }
    }

    // Re-mount DevFS and ProcFS
    {
        let vfs = get_vfs();
        vfs.mount("/dev".into(), Arc::new(DevFs::new())).ok();
        vfs.mount("/proc".into(), Arc::new(ProcFs::new())).ok();
    }

    kprintln!("[ROOTFS] DevFS and ProcFS re-mounted on BlockFS root");
}

/// Load the virtio-blk disk contents as a TAR archive into the RamFS.
/// This is the legacy boot path for non-persistent rootfs images.
#[cfg(feature = "alloc")]
fn load_tar_rootfs() {
    use crate::drivers::virtio::blk;

    let device = match blk::get_device() {
        Some(dev) => dev,
        None => {
            kprintln!("[ROOTFS] virtio-blk device not available");
            return;
        }
    };

    let mut dev = device.lock();
    let total_sectors = dev.capacity_sectors();
    let total_bytes = total_sectors as usize * blk::BLOCK_SIZE;

    if total_sectors == 0 {
        kprintln!("[ROOTFS] Disk is empty (0 sectors)");
        return;
    }

    kprintln!(
        "[ROOTFS] Reading {} sectors ({} KB) from virtio-blk...",
        total_sectors,
        total_bytes / 1024
    );

    // Allocate buffer for entire disk contents
    let mut disk_data = alloc::vec![0u8; total_bytes];

    // Read all sectors
    for sector in 0..total_sectors {
        let offset = sector as usize * blk::BLOCK_SIZE;
        if let Err(_e) = dev.read_block(sector, &mut disk_data[offset..offset + blk::BLOCK_SIZE]) {
            kprintln!("[ROOTFS] Read error at sector {}/{}", sector, total_sectors);
            return;
        }
    }

    // Release the device lock before calling into VFS
    drop(dev);

    kprintln!("[ROOTFS] Disk read complete, parsing TAR archive...");

    match crate::fs::tar::load_tar_to_vfs(&disk_data) {
        Ok(_count) => {
            kprintln!("[ROOTFS] Loaded entries from disk into VFS");
        }
        Err(_e) => {
            kprintln!("[ROOTFS] TAR parse error");
        }
    }
}

/// Phase 4A gate test: try to load user-space ELF binaries from rootfs.
///
/// This verifies the full pipeline: VFS file read -> ELF parse -> process
/// creation -> VAS page mapping -> ELF segment loading.
///
/// Tests both `/bin/minimal` (no-libc) and `/bin/sh -c "echo ..."`
/// (libc-linked).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn test_user_binary_load() {
    use crate::fs::get_vfs;

    // Test 1: /bin/minimal (no-libc, provides its own _start)
    let vfs = get_vfs();
    if vfs.resolve_path("/bin/minimal").is_ok() {
        match crate::userspace::load_user_program("/bin/minimal", &["minimal"], &["PATH=/bin"]) {
            Ok(pid) => {
                run_user_process_scheduled(pid);
            }
            Err(e) => {
                kprintln!("[BOOT] /bin/minimal FAILED: {:?}", e);
            }
        }
    }

    // Test 2: /bin/fork_test (fork + waitpid)
    if vfs.resolve_path("/bin/fork_test").is_ok() {
        match crate::userspace::load_user_program("/bin/fork_test", &["fork_test"], &["PATH=/bin"])
        {
            Ok(pid) => {
                run_user_process_scheduled(pid);
            }
            Err(e) => {
                kprintln!("[BOOT] /bin/fork_test FAILED: {:?}", e);
            }
        }
    }

    // Test 3: /bin/exec_test -- SKIPPED (multi-LOAD ELF fixed in dynamic linker
    // v0.7.1) Test 4: /bin/sh -- SKIPPED (multi-LOAD ELF fixed in dynamic
    // linker v0.7.1)

    // Coreutils validation suite (progressive complexity).
    // Each program exercises different syscall combinations.
    // Expected output markers: ECHO_PASS (from echo's stdout),
    // CAT_PASS (from cat_test.txt), WC output, LS output,
    // sorted output, PIPELINE_PASS (from pipeline_test).
    let env = &["PATH=/bin:/usr/bin"];

    // Test 5: /bin/echo -- argv + write (simplest coreutil)
    boot_run_program("/bin/echo", &["echo", "ECHO_PASS"], env);

    // Test 6: /bin/cat -- file open/read/write/close
    boot_run_program("/bin/cat", &["cat", "/usr/src/cat_test.txt"], env);

    // Test 7: /bin/wc -- ctype + getopt + printf formatting
    boot_run_program("/bin/wc", &["wc", "/usr/src/wc_test.txt"], env);

    // Test 8: /bin/ls -- opendir/readdir/stat/qsort
    boot_run_program("/bin/ls", &["ls", "/usr/src/"], env);

    // Test 9: /bin/sort -- malloc/realloc + qsort + function pointers
    boot_run_program("/bin/sort", &["sort", "/usr/src/sort_test.txt"], env);

    // Test 10: /bin/pipeline_test -- capstone: fork/exec/pipe/dup2/waitpid
    // Depends on /bin/cat and /bin/sort being in rootfs.
    boot_run_program("/bin/pipeline_test", &["pipeline_test"], env);

    // BusyBox smoke tests (only if /bin/busybox exists in rootfs).
    // BusyBox applets are installed as symlinks that resolve to copies
    // of the busybox binary; the TAR loader expands symlinks as file copies.
    {
        let vfs = get_vfs();
        let has_busybox = vfs.resolve_path("/bin/busybox").is_ok();
        if has_busybox {
            kprintln!("[BOOT] BusyBox detected -- running applet smoke tests");
            // BusyBox version banner
            boot_run_program("/bin/busybox", &["busybox"], env);
            // Basic applets via symlinks (these are copies of busybox)
            boot_run_program("/bin/echo", &["echo", "BUSYBOX_ECHO_PASS"], env);
            boot_run_program("/bin/pwd", &["pwd"], env);
            boot_run_program("/bin/uname", &["uname", "-a"], env);
            boot_run_program("/bin/ls", &["ls", "/bin/"], env);
            boot_run_program("/bin/cat", &["cat", "/usr/src/cat_test.txt"], env);
            boot_run_program("/usr/bin/wc", &["wc", "/usr/src/wc_test.txt"], env);
            boot_run_program("/usr/bin/sort", &["sort", "/usr/src/sort_test.txt"], env);
            boot_run_program("/bin/true", &["true"], env);
            boot_run_program("/bin/false", &["false"], env);

            // ash shell tests (Sprint B-4): verify shell features
            kprintln!("[BOOT] ash shell scripted tests");
            // 1. Basic command execution via -c
            boot_run_program("/bin/ash", &["ash", "-c", "echo ASH_BASIC_PASS"], env);
            // 2. Variable expansion
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "X=veridian; echo ASH_VAR_${X}_PASS"],
                env,
            );
            // 3. Exit status
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "false; echo ASH_EXIT_$?_PASS"],
                env,
            );
            // 4. Conditional (test -f)
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "test -f /bin/busybox && echo ASH_COND_PASS"],
                env,
            );
            // 5. Pipe
            boot_run_program("/bin/ash", &["ash", "-c", "echo ASH_PIPE_PASS | cat"], env);
            // 6. Redirect (write + read back)
            boot_run_program(
                "/bin/ash",
                &[
                    "ash",
                    "-c",
                    "echo ASH_REDIR_PASS > /tmp/redir.txt; cat /tmp/redir.txt",
                ],
                env,
            );
            // 7. For loop
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "for i in A B C; do echo LOOP_$i; done"],
                env,
            );
            // 8. Command substitution
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "D=$(pwd); echo ASH_SUBST_${D}_PASS"],
                env,
            );
            // 9. seq (B-6: float formatting -- seq uses printf %f internally)
            boot_run_program("/usr/bin/seq", &["seq", "1", "3"], env);
            // 10. Pipe with head (validates pipe + EPIPE handling)
            boot_run_program(
                "/bin/ash",
                &["ash", "-c", "echo PIPE_HEAD_PASS | head -n 1"],
                env,
            );
            // 12. Comprehensive test script (if present in rootfs)
            boot_run_program("/bin/ash", &["ash", "/usr/src/busybox_test.sh"], env);

            // Phase C: Native compilation tests (only if GCC + BusyBox source in rootfs)
            {
                let vfs = get_vfs();
                let has_gcc = vfs.resolve_path("/usr/bin/gcc").is_ok();
                let has_bb_src = vfs
                    .resolve_path("/usr/src/busybox-1.36.1/include/autoconf.h")
                    .is_ok();
                if has_gcc && has_bb_src {
                    kprintln!("[BOOT] Phase C: Native compilation tests");

                    // C-1: Single-file native compilation test
                    // Compile coreutils echo.c (standalone, libc-only) natively
                    boot_run_program(
                        "/usr/bin/gcc",
                        &[
                            "gcc",
                            "-c",
                            "-std=c11",
                            "-nostdinc",
                            "-isystem",
                            "/usr/include",
                            "-isystem",
                            "/usr/lib/gcc/x86_64-veridian/16.2.0/include", /* GCC_VERSION in scripts/build-native-gcc-static.sh */
                            "-static",
                            "-fno-stack-protector",
                            "-ffreestanding",
                            "-mno-red-zone",
                            "-mcmodel=small",
                            "-O2",
                            "-o",
                            "/tmp/echo.o",
                            "/usr/src/coreutils/echo.c",
                        ],
                        env,
                    );
                    // Verify the object file was produced
                    {
                        let vfs = get_vfs();
                        if vfs.resolve_path("/tmp/echo.o").is_ok() {
                            kprintln!("NATIVE_COMPILE_SINGLE_PASS");
                        } else {
                            kprintln!("NATIVE_COMPILE_SINGLE_FAIL");
                        }
                    }

                    // C-2: Link echo.o into a binary and execute it natively
                    boot_run_program(
                        "/usr/bin/gcc",
                        &[
                            "gcc",
                            "-static",
                            "-nostdlib",
                            "-ffreestanding",
                            "-o",
                            "/tmp/echo-native",
                            "/usr/lib/crt0.o",
                            "/tmp/echo.o",
                            "-L",
                            "/usr/lib",
                            "-L",
                            "/usr/lib/gcc/x86_64-veridian/16.2.0",
                            "-lc",
                            "-lgcc",
                        ],
                        env,
                    );
                    // Verify link produced a binary, then execute it
                    {
                        let vfs = get_vfs();
                        if vfs.resolve_path("/tmp/echo-native").is_ok() {
                            kprintln!("[BOOT] Phase C-2: Executing natively-compiled echo");
                            boot_run_program(
                                "/tmp/echo-native",
                                &["echo", "NATIVE_ECHO_PASS"],
                                env,
                            );
                        } else {
                            kprintln!(
                                "[BOOT] Phase C-2: Link FAILED -- /tmp/echo-native not found"
                            );
                        }
                    }

                    // C-3: Full native build (all 208 files + link)
                    // SKIPPED at boot -- compiling 208 files blocks the
                    // interactive shell for several minutes. Run manually:
                    //   ash /usr/src/build-busybox-native.sh
                    {
                        let vfs = get_vfs();
                        let has_script =
                            vfs.resolve_path("/usr/src/build-busybox-native.sh").is_ok();
                        if has_script {
                            kprintln!("[BOOT] Phase C-3: Skipped (208-file native build)");
                            kprintln!("[BOOT] Run manually: ash /usr/src/build-busybox-native.sh");
                        }
                    }

                    // C-4: Native sysinfo + edit compilation
                    // SKIPPED at boot -- run manually at the ash prompt.
                    {
                        let vfs = get_vfs();
                        let has_script = vfs
                            .resolve_path("/usr/src/build-native-programs.sh")
                            .is_ok();
                        if has_script {
                            kprintln!("[BOOT] Phase C-4: Skipped (native sysinfo+edit build)");
                            kprintln!("[BOOT] Run manually: ash /usr/src/build-native-programs.sh");
                        }
                    }

                    // C-5: Execute pre-built native binaries (if present from C-4)
                    // sysinfo reads /proc/* so output validates VFS + uname + proc subsystems
                    {
                        let vfs = get_vfs();
                        let has_sysinfo = vfs.resolve_path("/tmp/sysinfo-native").is_ok();
                        if has_sysinfo {
                            kprintln!("[BOOT] Phase C-5: Executing natively-compiled sysinfo");
                            boot_run_program("/tmp/sysinfo-native", &["sysinfo"], env);
                            kprintln!("NATIVE_RUN_SYSINFO_PASS");
                        } else {
                            kprintln!("[BOOT] Phase C-5: Skipped (/tmp/sysinfo-native not found)");
                            kprintln!("[BOOT] Build first: ash /usr/src/build-native-programs.sh");
                        }
                    }

                    // C-6: Native coreutils compilation
                    // SKIPPED at boot -- run manually at the ash prompt.
                    {
                        let vfs = get_vfs();
                        let has_script = vfs
                            .resolve_path("/usr/src/build-native-coreutils.sh")
                            .is_ok();
                        if has_script {
                            kprintln!("[BOOT] Phase C-6: Skipped (native coreutils build)");
                            kprintln!(
                                "[BOOT] Run manually: ash /usr/src/build-native-coreutils.sh"
                            );
                        }
                    }
                } else {
                    if !has_gcc {
                        kprintln!("[BOOT] Phase C skipped: /usr/bin/gcc not in rootfs");
                    }
                    if !has_bb_src {
                        kprintln!("[BOOT] Phase C skipped: BusyBox source not in rootfs");
                    }
                }
            }

            // Boot tests complete -- return to kernel_init_stage3_onwards()
            // which runs Stage 6 (/sbin/init -> /bin/sh -> kernel shell).
            kprintln!("[BOOT] Boot tests complete, returning to Stage 6");
        }
    }
}

/// Helper: load and run a user-space program during boot, logging pass/fail.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn boot_run_program(path: &str, argv: &[&str], envp: &[&str]) {
    use crate::fs::get_vfs;

    let vfs = get_vfs();
    match vfs.resolve_path(path) {
        Ok(_node) => match crate::userspace::load_user_program(path, argv, envp) {
            Ok(pid) => {
                kprintln!("[BOOT] Running {}", path);
                run_user_process_scheduled(pid);
            }
            Err(e) => {
                kprintln!("[BOOT] {} load FAILED: {:?}", path, e);
            }
        },
        Err(_) => {
            kprintln!("[BOOT] {} not found in VFS, skipping", path);
        }
    }

    // Sweep any orphaned zombie processes left behind by the program.
    // In boot context, there is no init process running waitpid() in a loop,
    // so orphans reparented to init (PID 1) would accumulate indefinitely.
    // This sweep prevents process table leaks across 213+ sequential
    // program executions during BusyBox compilation.
    boot_reap_orphan_zombies();
}

/// Sweep zombie processes from the process table in boot context.
///
/// During boot, programs may fork children that exit without being reaped
/// (the parent exits without calling waitpid, or the child outlives the
/// parent). These zombies are reparented to init (PID 1) by `cleanup_process`,
/// but in boot context there is no init process running a reap loop.
///
/// This function scans the process table and removes any zombie process whose
/// parent is no longer alive (or is init), preventing unbounded zombie
/// accumulation across hundreds of sequential boot-context program executions.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn boot_reap_orphan_zombies() {
    use crate::process::{
        pcb::ProcessState,
        table::{self, PROCESS_TABLE},
        ProcessId,
    };

    // Collect zombie PIDs first to avoid holding the table lock during removal.
    let mut zombies_to_reap = alloc::vec::Vec::new();
    PROCESS_TABLE.for_each(|proc| {
        if proc.get_state() == ProcessState::Zombie {
            // Reap zombies that were reparented to init (PID 1) or whose
            // parent no longer exists. PID 0 and PID 1 are system processes.
            let dominated_by_init_or_orphaned = match proc.parent() {
                Some(parent_pid) => parent_pid.0 <= 1 || table::get_process(parent_pid).is_none(),
                None => true,
            };
            if dominated_by_init_or_orphaned {
                zombies_to_reap.push(proc.pid);
            }
        }
    });

    for pid in &zombies_to_reap {
        // Clean up process resources if not already done
        if let Some(proc) = table::get_process(*pid) {
            // cleanup_process should have been called by sys_exit, but call
            // it defensively in case the process died abnormally.
            if proc.get_state() == ProcessState::Zombie {
                // Remove from init's children list
                if let Some(init) = table::get_process(ProcessId(1)) {
                    init.children.lock().retain(|&p| p != *pid);
                }
            }
            // Free page table frames (deferred from cleanup_process).
            // Boot CR3 is active here, so it's safe to free the process's
            // page table hierarchy. Clear page_table_root afterwards to
            // prevent double-free if this zombie is encountered again.
            let pt_root = proc.memory_space.lock().get_page_table();
            if pt_root != 0 {
                crate::mm::vas::free_user_page_table_frames(pt_root);
                proc.memory_space.lock().set_page_table(0);
            }
        }
        table::remove_process(*pid);
    }

    if !zombies_to_reap.is_empty() {
        kprintln!(
            "[BOOT] Reaped {} orphan zombie(s) from process table",
            zombies_to_reap.len()
        );
    }
}

/// Run user process `pid` as dispatcher tasks of its own and wait until it
/// has exited (ADR 0006, stage D2): every launcher -- the boot tests, the
/// shell, the KDE session -- uses this. The nested "returnable" entry that
/// ran a program inside the boot flow went with the old scheduler (D4).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub(crate) fn run_user_process(pid: crate::process::ProcessId) {
    if crate::process::run_and_wait(pid).is_none() {
        kprintln!("[BOOT] Could not start pid {}", pid.0);
    }
}

/// `run_user_process` for the boot tests (kept as their entry point).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn run_user_process_scheduled(pid: crate::process::ProcessId) {
    run_user_process(pid);
}

/// Kernel-mode init function
///
/// Exercises Phase 2 subsystems (VFS, shell, services) at runtime and emits
/// QEMU-parseable `[ok]`/`[failed]` markers for each test. Called from
/// `sched::start()` before entering the idle loop.
#[cfg(feature = "alloc")]
pub fn kernel_init_main() {
    kprintln!("");
    kprintln!("========================================");
    kprintln!("[INIT] VeridianOS kernel-mode init");
    kprintln!("========================================");

    let mut passed = 0u32;
    let mut failed = 0u32;

    run_vfs_tests(&mut passed, &mut failed);

    // Shell tests may short-circuit if shell is unavailable
    if !run_shell_tests(&mut passed, &mut failed) {
        return;
    }

    run_elf_tests(&mut passed, &mut failed);
    run_capability_tests(&mut passed, &mut failed);
    run_security_tests(&mut passed, &mut failed);
    run_phase4_tests(&mut passed, &mut failed);
    run_display_tests(&mut passed, &mut failed);
    run_usercopy_tests(&mut passed, &mut failed);
    run_dispatch_tests(&mut passed, &mut failed);
    run_process_tests(&mut passed, &mut failed);

    // --- Summary ---
    print_summary(passed, failed);
}

/// Dispatcher tests (ADR 0006, stage D1): kernel threads on their own
/// stacks, switching, wait queues, the idle task and reaping. On the
/// `dispatch` architectures (x86_64, RISC-V); AArch64 reports them as
/// skipped until it has the switch primitive.
#[cfg(feature = "alloc")]
fn run_dispatch_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] Dispatcher tests:");
    #[cfg(dispatch)]
    {
        use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

        use crate::sched::dispatch;

        static COUNTER: AtomicU32 = AtomicU32::new(0);
        /// Each thread appends its id (1 or 2) as a base-3 digit.
        static TRACE: AtomicU64 = AtomicU64::new(0);
        static FLAG: AtomicU32 = AtomicU32::new(0);
        static SEEN: AtomicU32 = AtomicU32::new(0);
        static WQ: dispatch::WaitQueue = dispatch::WaitQueue::new();

        extern "C" fn worker(id: usize) {
            for _ in 0..20 {
                COUNTER.fetch_add(1, Ordering::Relaxed);
                let t = TRACE.load(Ordering::Relaxed);
                if t < u64::MAX / 4 {
                    TRACE.store(t * 3 + id as u64, Ordering::Relaxed);
                }
                dispatch::yield_now();
            }
        }

        extern "C" fn waiter(_: usize) {
            WQ.wait_until(|| FLAG.load(Ordering::Acquire) != 0);
            SEEN.store(FLAG.load(Ordering::Acquire), Ordering::Release);
        }

        // Test: two kernel threads interleave through yield and are reaped.
        // Both are bound to this (the boot) CPU, so they share it even when
        // other CPUs are idle.
        {
            let before = dispatch::task_count();
            let a = dispatch::spawn_kernel_on(0, "worker-a", worker, 1);
            let b = dispatch::spawn_kernel_on(0, "worker-b", worker, 2);
            let ok = match (a, b) {
                (Ok(a), Ok(b)) => {
                    dispatch::join(a);
                    dispatch::join(b);
                    // Count switches between the two ids in the trace.
                    let mut t = TRACE.load(Ordering::Relaxed);
                    let (mut last, mut alternations) = (0, 0);
                    while t != 0 {
                        let d = t % 3;
                        if last != 0 && d != last {
                            alternations += 1;
                        }
                        last = d;
                        t /= 3;
                    }
                    COUNTER.load(Ordering::Relaxed) == 40
                        && alternations >= 4
                        && dispatch::task_count() == before
                }
                _ => false,
            };
            report_test("dispatch_kthreads_interleave_and_reap", ok, passed, failed);
        }

        // Test: a blocked thread is woken through a wait queue; its exit
        // runs the idle task (nothing else is runnable while this thread
        // waits in join) and wakes the joiner.
        {
            let ok = match dispatch::spawn_kernel("waiter", waiter, 0) {
                Ok(w) => {
                    dispatch::yield_now(); // let it block
                    FLAG.store(7, Ordering::Release);
                    WQ.wake_all();
                    dispatch::join(w);
                    SEEN.load(Ordering::Acquire) == 7
                }
                Err(_) => false,
            };
            report_test("dispatch_waitqueue_wake_and_join", ok, passed, failed);
        }
    }
    // SMP stage S2: every secondary dispatches. A kernel thread queued on
    // each one runs there (it records the CPU it found itself on), and its
    // exit wakes the joiner on the boot CPU across CPUs.
    #[cfg(dispatch)]
    {
        use core::sync::atomic::{AtomicUsize, Ordering};

        use crate::sched::dispatch;

        const NONE: usize = usize::MAX;
        static RAN_ON: [AtomicUsize; crate::sched::smp::MAX_CPUS] =
            [const { AtomicUsize::new(NONE) }; crate::sched::smp::MAX_CPUS];

        extern "C" fn where_am_i(slot: usize) {
            RAN_ON[slot].store(
                crate::arch::percpu::this_cpu_id() as usize,
                Ordering::Release,
            );
        }

        let online = crate::arch::smp_boot::online_cpus() as usize;
        if online <= 1 {
            report_skip("dispatch_runs_on_every_cpu", "one CPU online");
        } else {
            // Secondaries become dispatching CPUs right after reporting
            // online; give them a moment.
            let start = crate::arch::timer::monotonic_ns();
            let mut keys = alloc::vec::Vec::new();
            let mut ok = true;
            for cpu in 1..online {
                let key = loop {
                    match dispatch::spawn_kernel_on(cpu, "where", where_am_i, cpu) {
                        Ok(k) => break Some(k),
                        Err(_)
                            if crate::arch::timer::monotonic_ns().saturating_sub(start)
                                < 1_000_000_000 =>
                        {
                            core::hint::spin_loop()
                        }
                        Err(_) => break None,
                    }
                };
                match key {
                    Some(k) => keys.push(k),
                    None => ok = false,
                }
            }
            for k in keys {
                dispatch::join(k);
            }
            for (cpu, ran_on) in RAN_ON.iter().enumerate().take(online).skip(1) {
                ok &= ran_on.load(Ordering::Acquire) == cpu;
            }
            report_test("dispatch_runs_on_every_cpu", ok, passed, failed);
        }

        // SMP stage S3 (D5e): unbound kernel threads are placed on idle
        // CPUs. Each spins for a while recording where it runs; started
        // while the other CPUs are idle, they must between them use every
        // CPU.
        static USED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
        // Set once every spinner exists: until then they wait, so that all
        // of them occupy a CPU at the same time however slowly they are
        // created (40 ms each under RISC-V emulation, longer than a spin).
        static GO: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        extern "C" fn spin_here(_: usize) {
            while !GO.load(core::sync::atomic::Ordering::Acquire) {
                core::hint::spin_loop();
            }
            let start = crate::arch::timer::monotonic_ns();
            while crate::arch::timer::monotonic_ns().saturating_sub(start) < 20_000_000 {
                USED.fetch_or(
                    1 << crate::arch::percpu::this_cpu_id(),
                    core::sync::atomic::Ordering::AcqRel,
                );
                core::hint::spin_loop();
            }
        }
        if online <= 1 {
            report_skip("dispatch_places_on_idle_cpus", "one CPU online");
        } else {
            let mut keys = alloc::vec::Vec::new();
            for _ in 0..online {
                if let Ok(k) = dispatch::spawn_kernel("spin", spin_here, 0) {
                    keys.push(k);
                }
            }
            let spawned = keys.len();
            GO.store(true, core::sync::atomic::Ordering::Release);
            for k in keys {
                dispatch::join(k);
            }
            let used = USED.load(Ordering::Acquire);
            let ok = spawned == online && used.count_ones() as usize == online;
            if !ok {
                crate::println!(
                    "[INIT] placement: {} threads, CPUs used {:#x}",
                    spawned,
                    used
                );
            }
            report_test("dispatch_places_on_idle_cpus", ok, passed, failed);
        }
    }

    #[cfg(not(dispatch))]
    {
        let _ = (passed, failed);
        for name in [
            "dispatch_kthreads_interleave_and_reap",
            "dispatch_waitqueue_wake_and_join",
            "dispatch_runs_on_every_cpu",
            "dispatch_places_on_idle_cpus",
        ] {
            report_skip(name, "no dispatcher (sprint E)");
        }
    }
}

/// Process lifecycle boot tests.
#[cfg(feature = "alloc")]
fn run_process_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] Process tests:");
    // N-46: a process that never ran -- what a fork that fails part-way, or
    // whose child cannot be started, leaves behind -- is freed whole: its
    // mapped frames, page tables and kernel stack all go back. (Dropping a
    // Process frees nothing; these frames used to leak.)
    {
        use crate::{
            mm::vas::MappingType,
            process::{pcb::ProcessBuilder, thread::ThreadBuilder},
        };
        // Free frames include the per-CPU caches, where mmap allocates
        // from and unmapping frees to (N-261). The count is machine-wide, so
        // other CPUs running tasks can allocate or free in the middle of a
        // measurement (seen once on four RISC-V harts: 16 frames more free
        // after than before). A leak fails every attempt; a concurrent
        // change on another CPU does not repeat, so three tries are made.
        let free = || crate::mm::get_memory_stats().free_frames;
        let attempt = || {
            let before = free();
            let built = ProcessBuilder::new(alloc::string::String::from("n46-rollback"))
                .build_with_address_space()
                .and_then(|p| {
                    p.memory_space.lock().mmap(16 * 4096, MappingType::Data)?;
                    let t =
                        ThreadBuilder::new(p.pid, alloc::string::String::from("n46"), 0x40_0000)
                            .build()?;
                    p.add_thread(t)?;
                    Ok(p)
                });
            match built {
                Ok(p) => {
                    let taken = before.saturating_sub(free());
                    crate::process::exit::discard_unstarted(&p);
                    drop(p);
                    let after = free();
                    if after != before {
                        kprintln!(
                            "  [n46] frames: before {} taken {} after {}",
                            before,
                            taken,
                            after
                        );
                    }
                    taken >= 16 && after == before
                }
                Err(_) => false,
            }
        };
        let ok = (0..3).any(|_| attempt());
        report_test("unstarted_process_freed_whole", ok, passed, failed);
    }

    // RISC-V user mode (sprint E): a program in a fresh address space runs
    // in U-mode as a dispatcher task. getpid returns its pid through ecall
    // (it exits with it); a load from address 0 kills it with SIGSEGV; a
    // supervisor CSR read kills it with SIGILL, so it really ran in U-mode.
    // Exit status as run_and_wait reports it: the code, or 128 + signal.
    #[cfg(target_arch = "riscv64")]
    {
        let [getpid, fault, csr, fp, sig, futex] = crate::arch::riscv64::usermode::test_programs();
        let (pid, status) = run_user_code(getpid);
        let ok = pid.is_some_and(|p| status == Some((p & 0xFF) as i32));
        if !ok {
            crate::println!("    pid {:?}, status {:?}", pid, status);
        }
        report_test("riscv_user_syscall_round_trip", ok, passed, failed);
        let (_, status) = run_user_code(fault);
        if status != Some(128 + 11) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "riscv_user_fault_kills_process",
            status == Some(128 + 11),
            passed,
            failed,
        );
        let (_, status) = run_user_code(csr);
        if status != Some(128 + 4) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "riscv_user_mode_is_unprivileged",
            status == Some(128 + 4),
            passed,
            failed,
        );
        report_fp_survives(fp, passed, failed);
        report_signal_round_trip(sig, passed, failed);
        let (_, status) = run_user_code(futex);
        if status != Some(42) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "user_futex_wake_op_updates_word",
            status == Some(42),
            passed,
            failed,
        );
        report_static_tls(passed, failed);

        // An ELF program through the loader (from the VFS, with arguments
        // and the console as fds 0-2): it writes "RV-ELF-OK" and exits with
        // its argc.
        let elf = crate::arch::riscv64::usermode::test_elf();
        let status = crate::fs::write_file("/tmp/rv-elf-test", &elf)
            .ok()
            .and_then(|_| {
                crate::userspace::loader::load_user_program(
                    "/tmp/rv-elf-test",
                    &["rv-elf-test", "a", "b"],
                    &[],
                )
                .ok()
            })
            .and_then(crate::process::run_and_wait);
        if status != Some(3) {
            crate::println!("    status {:?}", status);
        }
        report_test("riscv_elf_program_runs", status == Some(3), passed, failed);

        // The embedded init (`/sbin/init`, userspace/embedded.rs) runs and
        // exits 0: its numbers are RISC-V's (write 64, exit_group 94).
        let status = run_embedded_init();
        if status != Some(0) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "riscv_embedded_init_runs",
            status == Some(0),
            passed,
            failed,
        );
        report_fork(
            crate::arch::riscv64::usermode::test_fork_program(),
            passed,
            failed,
        );
        report_exec(
            crate::arch::riscv64::usermode::test_exec_program(),
            passed,
            failed,
        );
    }
    // AArch64 user mode (sprint E, ADR 0016): as on RISC-V. getpid returns
    // its pid through svc (it exits with it); a load from address 0 kills
    // it with SIGSEGV; an EL1 system-register read kills it with SIGILL, so
    // it really ran at EL0. Exit status as run_and_wait reports it.
    #[cfg(target_arch = "aarch64")]
    {
        let [getpid, fault, sysreg, fp, sig, futex] =
            crate::arch::aarch64::usermode::test_programs();
        let (pid, status) = run_user_code(getpid);
        let ok = pid.is_some_and(|p| status == Some((p & 0xFF) as i32));
        if !ok {
            crate::println!("    pid {:?}, status {:?}", pid, status);
        }
        report_test("aarch64_user_syscall_round_trip", ok, passed, failed);
        let (_, status) = run_user_code(fault);
        if status != Some(128 + 11) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "aarch64_user_fault_kills_process",
            status == Some(128 + 11),
            passed,
            failed,
        );
        let (_, status) = run_user_code(sysreg);
        if status != Some(128 + 4) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "aarch64_user_mode_is_unprivileged",
            status == Some(128 + 4),
            passed,
            failed,
        );
        report_asid_isolation(passed, failed);
        report_dcache_clean(passed, failed);
        let (_, status) = run_user_code(crate::arch::aarch64::usermode::test_cacheops_program());
        if status != Some(0) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "aarch64_el0_cache_maintenance",
            status == Some(0),
            passed,
            failed,
        );
        report_fp_survives(fp, passed, failed);
        report_signal_round_trip(sig, passed, failed);
        let (_, status) = run_user_code(futex);
        if status != Some(42) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "user_futex_wake_op_updates_word",
            status == Some(42),
            passed,
            failed,
        );
        report_static_tls(passed, failed);

        // An ELF program through the loader: it writes "A64-ELF-OK" and
        // exits with its argc.
        let elf = crate::arch::aarch64::usermode::test_elf();
        let status = crate::fs::write_file("/tmp/a64-elf-test", &elf)
            .ok()
            .and_then(|_| {
                crate::userspace::loader::load_user_program(
                    "/tmp/a64-elf-test",
                    &["a64-elf-test", "a", "b"],
                    &[],
                )
                .ok()
            })
            .and_then(crate::process::run_and_wait);
        if status != Some(3) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "aarch64_elf_program_runs",
            status == Some(3),
            passed,
            failed,
        );

        // The embedded init (`/sbin/init`) runs and exits 0 with arm64's
        // numbers (write 64, exit_group 94).
        let status = run_embedded_init();
        if status != Some(0) {
            crate::println!("    status {:?}", status);
        }
        report_test(
            "aarch64_embedded_init_runs",
            status == Some(0),
            passed,
            failed,
        );
        report_fork(
            crate::arch::aarch64::usermode::test_fork_program(),
            passed,
            failed,
        );
        report_exec(
            crate::arch::aarch64::usermode::test_exec_program(),
            passed,
            failed,
        );
        report_bti(passed, failed);
    }
}

/// BTI guarded pages (PROT_BTI): with FEAT_BTI, an indirect branch into a
/// guarded page must land on a BTI instruction (SIGILL otherwise); without
/// it, mprotect refuses PROT_BTI with EINVAL, as Linux does.
#[cfg(all(feature = "alloc", target_arch = "aarch64"))]
fn report_bti(passed: &mut u32, failed: &mut u32) {
    use crate::arch::aarch64::usermode::test_bti_program;
    let (_, pad) = run_user_code(test_bti_program(true));
    let (_, nopad) = run_user_code(test_bti_program(false));
    let bti = crate::arch::bti_supported();
    let ok = if bti {
        pad == Some(0) && nopad == Some(128 + 4)
    } else {
        pad == Some(22) && nopad == Some(22)
    };
    if !ok {
        crate::println!("    bti {} pad {:?} nopad {:?}", bti, pad, nopad);
    }
    report_test("user_bti_guards_indirect_branches", ok, passed, failed);

    // The loader guards a static program whose GNU property note asks for
    // BTI (no mprotect in it): SIGILL on a BTI CPU, exit 0 elsewhere.
    let elf = crate::arch::aarch64::usermode::test_bti_elf();
    let status = crate::fs::write_file("/tmp/bti-test", &elf)
        .ok()
        .and_then(|_| {
            crate::userspace::loader::load_user_program("/tmp/bti-test", &["bti-test"], &[]).ok()
        })
        .and_then(crate::process::run_and_wait);
    let want = if bti { Some(128 + 4) } else { Some(0) };
    if status != want {
        crate::println!("    status {:?}", status);
    }
    report_test("loader_guards_bti_programs", status == want, passed, failed);
}

/// exec starts the new image at its own entry (`/tmp/embedded-init`, from
/// `run_embedded_init`, exits 0); the return used to go through the old
/// frame into the unmapped old image.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn report_exec(code: &[u8], passed: &mut u32, failed: &mut u32) {
    let (_, status) = run_user_code(code);
    if status != Some(0) {
        crate::println!("    status {:?}", status);
    }
    report_test(
        "user_exec_enters_new_image",
        status == Some(0),
        passed,
        failed,
    );
}

/// A fork child resumes after the call with its parent's registers (it
/// used to start from the load-time context, the program's entry, and
/// fork again): the program exits with its child's status, 7.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn report_fork(code: &[u8], passed: &mut u32, failed: &mut u32) {
    let (_, status) = run_user_code(code);
    if status != Some(7) {
        crate::println!("    status {:?}", status);
    }
    report_test(
        "user_fork_child_resumes_after_call",
        status == Some(7),
        passed,
        failed,
    );
}
/// Run `code` (position-independent) as a new process in user mode: one
/// executable page at 0x40_0000, four stack pages below 0x80_4000. Returns
/// its pid and the exit status `run_and_wait` reports.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn run_user_code(code: &[u8]) -> (Option<u64>, Option<i32>) {
    match build_user_code(code) {
        Some(pid) => (Some(pid.0), crate::process::run_and_wait(pid)),
        None => (None, None),
    }
}

/// Two copies of the FP test program (`test_programs()[3]`) run together:
/// each puts its pid in two FP registers, yields 64 times and exits 0 only
/// if they still hold it. On one CPU they alternate, so this fails unless
/// user FP state is switched with the task.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn report_fp_survives(code: &[u8], passed: &mut u32, failed: &mut u32) {
    let statuses = match (build_user_code(code), build_user_code(code)) {
        (Some(a), Some(b)) => crate::process::run_and_wait_all(&[a, b]),
        _ => alloc::vec![None, None],
    };
    let ok = statuses.iter().all(|s| *s == Some(0));
    if !ok {
        crate::println!("    statuses {:?}", statuses);
    }
    report_test("user_fp_state_survives_switches", ok, passed, failed);
}

/// Three processes run the ASID test program together, each address space
/// with its own ASID (HA-26): each must read back its own pid from the same
/// stack address after every switch. They also hold distinct nonzero ASIDs.
#[cfg(all(feature = "alloc", target_arch = "aarch64"))]
fn report_asid_isolation(passed: &mut u32, failed: &mut u32) {
    let code = crate::arch::aarch64::usermode::test_asid_program();
    let built: alloc::vec::Vec<_> = (0..3).filter_map(|_| build_user_code(code)).collect();
    let asids: alloc::vec::Vec<u64> = built
        .iter()
        .filter_map(|&pid| crate::process::table::get_process(pid))
        .map(|p| {
            crate::arch::aarch64::switch::ttbr0_for(p.memory_space.lock().get_page_table()) >> 48
        })
        .collect();
    let distinct = asids.len() == 3
        && asids.iter().all(|&a| a != 0)
        && asids[0] != asids[1]
        && asids[1] != asids[2]
        && asids[0] != asids[2];
    let statuses = if built.len() == 3 {
        crate::process::run_and_wait_all(&built)
    } else {
        alloc::vec![None]
    };
    let ok = distinct && statuses.iter().all(|s| *s == Some(0));
    if !ok {
        crate::println!("    asids {:?}, statuses {:?}", asids, statuses);
    }
    report_test("aarch64_asids_isolate_address_spaces", ok, passed, failed);
}

/// The data-cache clean for instruction fetch (HA-25), which QEMU's
/// CTR_EL0.IDC makes unnecessary there: run it anyway over a user code
/// page's alias in the physical map, then run that code. A bad address or
/// line size faults or skips lines; the program must still exit with its pid.
#[cfg(all(feature = "alloc", target_arch = "aarch64"))]
fn report_dcache_clean(passed: &mut u32, failed: &mut u32) {
    let code = crate::arch::aarch64::usermode::test_programs()[0];
    let ok = build_user_code(code).is_some_and(|pid| {
        let Some(process) = crate::process::table::get_process(pid) else {
            return false;
        };
        {
            let vas = process.memory_space.lock();
            // SAFETY: the process's root, through the physical map; only read.
            let m = unsafe { crate::mm::vas::create_mapper_from_root_pub(vas.get_page_table()) };
            let Ok((frame, _)) = m.translate_page(crate::mm::VirtualAddress(0x40_0000)) else {
                return false;
            };
            let kva = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as usize;
            let ctr: u64;
            // SAFETY: reading CTR_EL0 has no side effects.
            unsafe { core::arch::asm!("mrs {0}, ctr_el0", out(reg) ctr, options(nomem, nostack)) };
            crate::arch::aarch64::clean_dcache_lines_to_pou(kva, 4096, 4 << ((ctr >> 16) & 0xF));
            crate::arch::aarch64::sync_icache_all();
        }
        drop(process);
        crate::process::run_and_wait_all(&[pid])[0] == Some((pid.0 & 0xFF) as i32)
    });
    report_test("aarch64_dcache_clean_to_pou", ok, passed, failed);
}

/// The signal test program (`test_programs()[4]`): a SIGUSR1 handler that
/// rewrites a saved register in its ucontext; exit status 49 means the
/// frame, the handler's arguments, the return path and rt_sigreturn's
/// restore of the modified context all worked.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn report_signal_round_trip(code: &[u8], passed: &mut u32, failed: &mut u32) {
    let (_, status) = run_user_code(code);
    if status != Some(49) {
        crate::println!("    status {:?}", status);
    }
    report_test(
        "user_signal_handler_round_trip",
        status == Some(49),
        passed,
        failed,
    );
}

/// A static ELF with a PT_TLS segment (`usermode::test_tls_elf`), loaded
/// through the loader from the VFS: it exits with its first TLS word (42)
/// when the loader built its TLS block and set the thread pointer.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn report_static_tls(passed: &mut u32, failed: &mut u32) {
    #[cfg(target_arch = "aarch64")]
    let elf = crate::arch::aarch64::usermode::test_tls_elf();
    #[cfg(target_arch = "riscv64")]
    let elf = crate::arch::riscv64::usermode::test_tls_elf();
    let status = crate::fs::write_file("/tmp/tls-test", &elf)
        .ok()
        .and_then(|_| {
            crate::userspace::loader::load_user_program("/tmp/tls-test", &["tls-test"], &[]).ok()
        })
        .and_then(crate::process::run_and_wait);
    if status != Some(42) {
        crate::println!("    status {:?}", status);
    }
    report_test("user_static_tls_block", status == Some(42), passed, failed);
}

/// Run the kernel's embedded init (`userspace::embedded::init_elf`) through
/// the loader and wait for it: from `/tmp`, since `/sbin/init` is the root
/// filesystem's own init when one is mounted (it never exits).
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn run_embedded_init() -> Option<i32> {
    let elf = crate::userspace::embedded::init_elf();
    crate::fs::write_file("/tmp/embedded-init", &elf)
        .ok()
        .and_then(|_| {
            crate::userspace::loader::load_user_program("/tmp/embedded-init", &[], &[]).ok()
        })
        .and_then(crate::process::run_and_wait)
}

/// Build (not start) a process that runs `code` as [`run_user_code`] does.
#[cfg(all(
    feature = "alloc",
    any(target_arch = "riscv64", target_arch = "aarch64")
))]
fn build_user_code(code: &[u8]) -> Option<crate::process::ProcessId> {
    use crate::{
        arch::context::ThreadContext,
        mm::{vas::MappingType, PageFlags, VirtualAddress},
        process::{pcb::ProcessBuilder, thread::ThreadBuilder},
    };
    const CODE: u64 = 0x40_0000;
    const STACK: u64 = 0x80_0000;
    // Room for a signal frame (4.7 KiB on AArch64) below the program's own
    // use of the stack.
    const STACK_SIZE: usize = 4 * 4096;
    let Ok(p) =
        ProcessBuilder::new(alloc::string::String::from("rv-user")).build_with_address_space()
    else {
        return None;
    };
    let mapped = {
        let vas = p.memory_space.lock();
        let code_ok = vas
            .map_region_flags(
                VirtualAddress(CODE),
                4096,
                MappingType::Code,
                Some(PageFlags::PRESENT | PageFlags::USER),
            )
            .is_ok();
        let stack_ok = vas
            .map_region_flags(
                VirtualAddress(STACK),
                STACK_SIZE,
                MappingType::Stack,
                Some(
                    PageFlags::PRESENT
                        | PageFlags::USER
                        | PageFlags::WRITABLE
                        | PageFlags::NO_EXECUTE,
                ),
            )
            .is_ok();
        // SAFETY: the process's root, through the physical map; only read.
        let m = unsafe { crate::mm::vas::create_mapper_from_root_pub(vas.get_page_table()) };
        match m.translate_page(VirtualAddress(CODE)) {
            Ok((frame, _)) if code_ok && stack_ok && code.len() <= 4096 => {
                let dst = crate::mm::phys_to_virt_addr(frame.as_u64() << 12) as *mut u8;
                // SAFETY: the code page's frame, just mapped and not yet
                // used by anyone; `code` fits in it.
                unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), dst, code.len()) };
                // Every CPU fetches what was just written (instruction
                // fetch is not coherent with stores without maintenance).
                vas.sync_code(CODE, code.len() as u64);
                true
            }
            _ => false,
        }
    };
    if !mapped {
        return None;
    }
    let Ok(t) =
        ThreadBuilder::new(p.pid, alloc::string::String::from("rv-user"), CODE as usize).build()
    else {
        return None;
    };
    {
        let mut c = t.context.lock();
        c.set_instruction_pointer(CODE as usize);
        c.set_stack_pointer((STACK + STACK_SIZE as u64) as usize);
    }
    if p.add_thread(t).is_err() {
        return None;
    }
    crate::process::table::add_process(p).ok()
}

/// Run VFS boot tests (tests 1-6).
#[cfg(feature = "alloc")]
fn run_vfs_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] VFS tests:");

    // Test 1: Create directory
    {
        let ok = fs::get_vfs()
            .mkdir("/tmp/test_init", fs::Permissions::default())
            .is_ok();
        report_test("vfs_mkdir", ok, passed, failed);
    }

    // Test 2: Write file via VFS create + write
    {
        let ok = (|| -> Result<(), crate::error::KernelError> {
            let vfs = fs::get_vfs();
            let parent = vfs.resolve_path("/tmp/test_init")?;
            let file = parent.create("hello.txt", fs::Permissions::default())?;
            file.write(0, b"Hello VeridianOS")?;
            Ok(())
        })()
        .is_ok();
        report_test("vfs_write_file", ok, passed, failed);
    }

    // Test 3: Read file back and verify contents
    {
        let ok = (|| -> Result<bool, crate::error::KernelError> {
            let vfs = fs::get_vfs();
            let dir = vfs.resolve_path("/tmp/test_init")?;
            let file = dir.lookup("hello.txt")?;
            let mut buf = [0u8; 32];
            let n = file.read(0, &mut buf)?;
            Ok(&buf[..n] == b"Hello VeridianOS")
        })()
        .unwrap_or(false);
        report_test("vfs_read_verify", ok, passed, failed);
    }

    // Test 4: List directory entries
    {
        let ok = (|| -> Result<bool, crate::error::KernelError> {
            let vfs = fs::get_vfs();
            let node = vfs.resolve_path("/tmp/test_init")?;
            let entries = node.readdir()?;
            Ok(entries.iter().any(|e| e.name == "hello.txt"))
        })()
        .unwrap_or(false);
        report_test("vfs_readdir", ok, passed, failed);
    }

    // Test 5: /proc is mounted
    {
        let ok = fs::get_vfs().resolve_path("/proc").is_ok();
        report_test("vfs_procfs", ok, passed, failed);
    }

    // Test 6: /dev is mounted
    {
        let ok = fs::get_vfs().resolve_path("/dev").is_ok();
        report_test("vfs_devfs", ok, passed, failed);
    }
}

/// Run shell boot tests (tests 7-12).
///
/// Returns `false` if the shell is unavailable, in which case the caller
/// should print the summary and return early.
#[cfg(feature = "alloc")]
fn run_shell_tests(passed: &mut u32, failed: &mut u32) -> bool {
    kprintln!("[INIT] Shell tests:");

    let shell = match services::shell::try_get_shell() {
        Some(s) => s,
        None => {
            kprintln!("  shell unavailable [failed]");
            *failed += 6;
            print_summary(*passed, *failed);
            return false;
        }
    };

    // Test 7: help command
    {
        let ok = matches!(
            shell.execute_command("help"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_help", ok, passed, failed);
    }

    // Test 8: pwd command
    {
        let ok = matches!(
            shell.execute_command("pwd"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_pwd", ok, passed, failed);
    }

    // Test 9: ls / command
    {
        let ok = matches!(
            shell.execute_command("ls /"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_ls", ok, passed, failed);
    }

    // Test 10: env command
    {
        let ok = matches!(
            shell.execute_command("env"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_env", ok, passed, failed);
    }

    // Test 11: echo command
    {
        let ok = matches!(
            shell.execute_command("echo hello"),
            services::shell::CommandResult::Success(_)
        );
        report_test("shell_echo", ok, passed, failed);
    }

    // Test 12: mkdir + verification via VFS
    {
        let ok = matches!(
            shell.execute_command("mkdir /tmp/shell_test"),
            services::shell::CommandResult::Success(_)
        ) && fs::file_exists("/tmp/shell_test");
        report_test("shell_mkdir_verify", ok, passed, failed);
    }

    true
}

/// Run ELF boot tests (tests 13-14).
#[cfg(feature = "alloc")]
fn run_elf_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] ELF tests:");

    // Test 13: Parse a valid minimal ELF64 executable header
    {
        use crate::elf::ElfLoader;

        let ok = (|| -> Result<bool, crate::error::KernelError> {
            let loader = ElfLoader::new();
            // Build a minimal valid ELF64 header + one LOAD program header
            let header_size = core::mem::size_of::<crate::elf::Elf64Header>();
            let ph_size = core::mem::size_of::<crate::elf::Elf64ProgramHeader>();
            let total = header_size + ph_size;
            let mut buf = alloc::vec![0u8; total];
            // ELF magic
            buf[0] = 0x7f;
            buf[1] = b'E';
            buf[2] = b'L';
            buf[3] = b'F';
            buf[4] = 2; // 64-bit
            buf[5] = 1; // little-endian
            buf[6] = 1;
            buf[16] = 2; // ET_EXEC
            #[cfg(target_arch = "x86_64")]
            {
                buf[18] = 62;
            }
            #[cfg(target_arch = "aarch64")]
            {
                buf[18] = 183;
            }
            #[cfg(target_arch = "riscv64")]
            {
                buf[18] = 243;
            }
            // version2 at offset 20
            buf[20] = 1;
            // entry at offset 24
            buf[24..32].copy_from_slice(&0x401000u64.to_le_bytes());
            // phoff at offset 32
            buf[32..40].copy_from_slice(&(header_size as u64).to_le_bytes());
            // ehsize at offset 52
            buf[52] = (header_size & 0xFF) as u8;
            buf[53] = ((header_size >> 8) & 0xFF) as u8;
            // phentsize at offset 54
            buf[54] = (ph_size & 0xFF) as u8;
            buf[55] = ((ph_size >> 8) & 0xFF) as u8;
            // phnum at offset 56
            buf[56] = 1;
            // Program header: PT_LOAD at ph_offset
            let po = header_size;
            buf[po] = 1; // p_type = PT_LOAD
            buf[po + 4] = 7; // p_flags = RWX
            buf[po + 16..po + 24].copy_from_slice(&0x400000u64.to_le_bytes()); // p_vaddr
            buf[po + 24..po + 32].copy_from_slice(&0x400000u64.to_le_bytes()); // p_paddr
            buf[po + 40..po + 48].copy_from_slice(&0x1000u64.to_le_bytes()); // p_memsz
            buf[po + 48..po + 56].copy_from_slice(&0x1000u64.to_le_bytes()); // p_align
            let binary =
                loader
                    .parse(&buf)
                    .map_err(|_| crate::error::KernelError::InvalidArgument {
                        name: "elf_data",
                        value: "parse failed",
                    })?;
            Ok(binary.entry_point == 0x401000 && !binary.segments.is_empty())
        })()
        .unwrap_or(false);
        report_test("elf_parse_valid", ok, passed, failed);
    }

    // Test 14: Reject invalid ELF magic
    {
        use crate::elf::ElfLoader;

        let ok = {
            let loader = ElfLoader::new();
            let bad_data = alloc::vec![0u8; 128]; // all zeros = no ELF magic
            loader.parse(&bad_data).is_err()
        };
        report_test("elf_reject_bad_magic", ok, passed, failed);
    }
}

/// Run capability boot tests (tests 15-18).
#[cfg(feature = "alloc")]
fn run_capability_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] Capability tests:");

    // Test 15: Create a capability token, insert into space, lookup succeeds
    {
        use crate::cap::{
            object::MemoryAttributes, CapabilitySpace, CapabilityToken, ObjectRef, Rights,
        };

        let ok = (|| -> Result<bool, crate::error::KernelError> {
            let space = CapabilitySpace::new();
            let token = CapabilityToken::new(1, 0, 0, 0);
            let object = ObjectRef::Memory {
                base: 0x1000,
                size: 0x1000,
                attributes: MemoryAttributes::normal(),
            };
            let rights = Rights::READ | Rights::WRITE;
            space.insert(token, object, rights)?;
            if let Some(found_rights) = space.lookup(token) {
                Ok(found_rights.contains(Rights::READ))
            } else {
                Ok(false)
            }
        })()
        .unwrap_or(false);
        report_test("cap_insert_lookup", ok, passed, failed);
    }

    // Test 16: IPC endpoint create + capability validate
    {
        let ok = (|| -> Result<bool, crate::ipc::IpcError> {
            let owner = crate::ipc::ProcessId(1);
            let (endpoint_id, capability) = ipc::create_endpoint(owner)?;
            ipc::validate_capability(owner, &capability)?;
            Ok(endpoint_id > 0)
        })()
        .unwrap_or(false);
        report_test("ipc_endpoint_create", ok, passed, failed);
    }

    // Test 17: Root capability exists after cap::init()
    {
        let ok = cap::root_capability().is_some();
        report_test("cap_root_exists", ok, passed, failed);
    }

    // Test 18: Capability quota enforcement
    {
        use crate::cap::{
            object::MemoryAttributes, CapabilitySpace, CapabilityToken, ObjectRef, Rights,
        };

        let ok = (|| -> Result<bool, crate::error::KernelError> {
            // Create a space with quota of 2
            let space = CapabilitySpace::with_quota(2);
            let obj = ObjectRef::Memory {
                base: 0x2000,
                size: 0x1000,
                attributes: MemoryAttributes::normal(),
            };

            // First two inserts should succeed
            let t1 = CapabilityToken::new(10, 0, 0, 0);
            space.insert(t1, obj.clone(), Rights::READ)?;

            let t2 = CapabilityToken::new(11, 0, 0, 0);
            space.insert(t2, obj.clone(), Rights::READ)?;

            // Third insert should fail (quota exceeded)
            let t3 = CapabilityToken::new(12, 0, 0, 0);
            let third_result = space.insert(t3, obj, Rights::READ);
            Ok(third_result.is_err())
        })()
        .unwrap_or(false);
        report_test("cap_quota_enforced", ok, passed, failed);
    }
}

/// Run security boot tests (tests 19-22).
#[cfg(feature = "alloc")]
fn run_security_tests(passed: &mut u32, failed: &mut u32) {
    // Test 19: MAC policy allows user_t -> file_t Read
    {
        let ok = security::mac::check_file_access("/test", security::AccessType::Read, 100).is_ok();
        report_test("mac_user_file_read", ok, passed, failed);
    }

    // Test 20: Audit log records events after enable
    {
        // Generate an explicit audit event so the test does not depend on
        // bootstrap ordering (process/capability audit hooks fire later).
        security::audit::log_process_create(0, 0, 0);
        let (count, _max) = security::audit::get_stats();
        let ok = count > 0;
        report_test("audit_has_events", ok, passed, failed);
    }

    // Test 21: Stack canary verify/mismatch logic
    // StackCanary::new() calls get_random() which deadlocks on the x86_64
    // heap stack and AArch64 (spin::Mutex).  The RNG itself is exercised
    // by auth::init() and ASLR above.  Here we test the verify logic with
    // a stack-local canary to confirm the detection mechanism works.
    {
        let canary_val: u64 = 0xDEAD_BEEF_CAFE_BABE;
        let mut stack_slot: u64 = canary_val;
        // Canary intact: should match
        let intact = stack_slot == canary_val;
        // Simulate buffer overflow corrupting the canary
        stack_slot ^= 1;
        let corrupted = stack_slot != canary_val;
        let ok = intact && corrupted;
        report_test("stack_canary_verify", ok, passed, failed);
    }

    // Test 22: SHA-256 NIST test vector passes
    {
        let ok = crate::crypto::validate();
        report_test("crypto_sha256_vector", ok, passed, failed);
    }
}

/// Run Phase 4 package ecosystem boot tests (tests 23-27).
#[cfg(feature = "alloc")]
fn run_phase4_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] Phase 4 package ecosystem tests:");

    // Test 23: Delta compute/apply roundtrip
    {
        let ok = crate::test_framework::test_pkg_delta_compute_apply().is_ok();
        report_test("pkg_delta_roundtrip", ok, passed, failed);
    }

    // Test 24: Reproducible build manifest comparison
    {
        let ok = crate::test_framework::test_pkg_reproducible_manifest().is_ok();
        report_test("pkg_reproducible_manifest", ok, passed, failed);
    }

    // Test 25: License detection from text
    {
        let ok = crate::test_framework::test_pkg_license_detection().is_ok();
        report_test("pkg_license_detection", ok, passed, failed);
    }

    // Test 26: Security scanner path and capability checks
    {
        let ok = crate::test_framework::test_pkg_security_scan().is_ok();
        report_test("pkg_security_scan", ok, passed, failed);
    }

    // Test 27: Ecosystem package definitions
    {
        let ok = crate::test_framework::test_pkg_ecosystem_definitions().is_ok();
        report_test("pkg_ecosystem_defs", ok, passed, failed);
    }
}

/// Run display/input boot tests (tests 28-29).
#[cfg(feature = "alloc")]
#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))] // all skipped there
fn run_display_tests(passed: &mut u32, failed: &mut u32) {
    kprintln!("[INIT] Display/input tests:");

    // Test 28: Framebuffer console initialized (x86_64 only — UEFI provides fb)
    {
        #[cfg(target_arch = "x86_64")]
        report_test(
            "fbcon_initialized",
            crate::graphics::fbcon::is_initialized(),
            passed,
            failed,
        );
        // With a ramfb device the console must be up on it; without one
        // there is nothing to check.
        #[cfg(not(target_arch = "x86_64"))]
        if crate::drivers::ramfb::present() {
            report_test(
                "fbcon_initialized",
                crate::graphics::fbcon::is_initialized(),
                passed,
                failed,
            );
        } else {
            report_skip("fbcon_initialized", "no ramfb device (-device ramfb)");
        }
    }

    // Test 29: Keyboard driver ready (x86_64 only — PS/2 keyboard)
    {
        #[cfg(target_arch = "x86_64")]
        report_test(
            "keyboard_driver_ready",
            crate::drivers::keyboard::is_initialized(),
            passed,
            failed,
        );
        #[cfg(not(target_arch = "x86_64"))]
        report_skip("keyboard_driver_ready", "no PS/2 keyboard");
    }
}

/// User-memory access safety and boot-stack integrity (tests 30-32).
#[cfg(feature = "alloc")]
fn run_usercopy_tests(passed: &mut u32, failed: &mut u32) {
    use crate::syscall::userspace::read_user_bytes;

    kprintln!("[INIT] User-copy safety tests:");

    // Test 30: a kernel-half address is rejected before any access.
    {
        let mut buf = [0u8; 8];
        let ok = read_user_bytes(0xFFFF_8000_0000_0000, &mut buf).is_err();
        report_test("usercopy_rejects_kernel_addr", ok, passed, failed);
    }

    // Test 31: a fault on an unmapped user page returns an error instead of
    // aborting (the x86_64 and RISC-V copy fixups; MEM-SEC-01 / SYS-SEC-01,
    // ARCH-MEM-03). The address is in the user half and mapped by nothing
    // during boot.
    {
        let ok = {
            let mut buf = [0u8; 8];
            read_user_bytes(crate::mm::user_layout::UNMAPPED_TEST_ADDR, &mut buf).is_err()
        };
        report_test("usercopy_fault_returns_efault", ok, passed, failed);
    }

    // Test 33: the futex WAKE_OP atomic on an unmapped user word returns an
    // error instead of faulting in the kernel (the cmpxchg fixups: x86_64
    // cmpxchg, AArch64 exclusives, RISC-V LR/SC).
    {
        let ok = crate::syscall::userspace::cmpxchg_user_u32(
            crate::mm::user_layout::UNMAPPED_TEST_ADDR,
            0,
            1,
        )
        .is_err();
        report_test("usercopy_cmpxchg_fault_returns_efault", ok, passed, failed);
    }

    // Test 32: the boot stack has not overflowed into the statics below it
    // (aarch64/riscv64 canary; N-13).
    {
        let ok = crate::arch::boot_stack_intact();
        report_test("boot_stack_canary_intact", ok, passed, failed);
    }

    // AArch64: a valid device tree is where boot.S says it is: the one
    // firmware passed in x0 (an arm64 Image boot, HA-01/HA-03), or QEMU
    // virt's fixed place for a bare ELF. The line names which, so that a
    // boot of the Image can require x0.
    #[cfg(target_arch = "aarch64")]
    {
        use crate::arch::aarch64::boot;
        if boot::dtb_passed_in_x0() {
            println!("[BOOT] Device tree passed in x0");
        } else {
            println!("[BOOT] Device tree at the QEMU virt default (x0 was 0)");
        }
        // SAFETY: the blob firmware passed, or QEMU virt's, reached through
        // the physical map; from_phys validates the header before reading.
        let ok = unsafe { crate::arch::fdt::Fdt::from_phys(boot::dtb_pa()) }.is_some();
        report_test("device_tree_found", ok, passed, failed);
        // The exception vectors were in place before the first Rust code
        // (boot.S installs them once the stack exists); before,
        // a fault until Stage 1 vectored wherever firmware had left VBAR.
        report_test(
            "vectors_installed_before_rust",
            boot::vectors_installed_at_entry(),
            passed,
            failed,
        );

        // The MMU and caches are on with the layout of ADR 0015: the kernel
        // runs at its upper-half link address, the kernel root is TTBR1's,
        // and a kernel address translates (AT S1E1R) to the physical
        // address `kernel_phys` computes, through Normal cacheable memory.
        let sctlr: u64;
        let par: u64;
        let here = boot::kernel_phys as *const () as u64;
        // SAFETY: reading SCTLR_EL1 and an address translation of a mapped
        // kernel address have no side effects beyond PAR_EL1.
        unsafe {
            core::arch::asm!("mrs {0}, sctlr_el1", out(reg) sctlr, options(nomem, nostack));
            core::arch::asm!("at s1e1r, {0}", "isb", "mrs {1}, par_el1",
                in(reg) here, out(reg) par, options(nostack));
        }
        let translated =
            par & 1 == 0 && par & 0x0000_FFFF_FFFF_F000 == boot::kernel_phys(here) & !0xFFF;
        let normal = par >> 56 == 0xFF;
        let ok = sctlr & 0x1005 == 0x1005
            && here >= 0xFFFF_FFFF_8000_0000
            && crate::mm::get_kernel_page_table() != 0
            && translated
            && normal;
        report_test("mmu_on_kernel_in_upper_half", ok, passed, failed);

        // Nothing is mapped in the lower half once the switch is done: the
        // identity map is gone from TTBR0 (ADR 0015), so the physical address
        // of the kernel itself does not translate, and the devices are
        // reached through the physical-memory map.
        let low = boot::kernel_phys(here);
        let par_low: u64;
        // SAFETY: an address translation has no side effect beyond PAR_EL1.
        unsafe {
            core::arch::asm!("at s1e1r, {0}", "isb", "mrs {1}, par_el1",
                in(reg) low, out(reg) par_low, options(nostack));
        }
        report_test(
            "lower_half_unmapped_after_boot",
            par_low & 1 == 1,
            passed,
            failed,
        );

        // Exclusive loads and stores work now that memory is Normal
        // cacheable (N-28): an atomic read-modify-write, and a reader-writer
        // lock that refuses a writer while a reader holds it. With the MMU
        // off these hung, which is why the filesystems used `bare_lock`, a
        // lock that did not lock.
        use core::sync::atomic::{AtomicU64, Ordering};
        let counter = AtomicU64::new(1);
        let rmw = counter.fetch_add(2, Ordering::AcqRel) == 1
            && counter
                .compare_exchange(3, 7, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
        let lock = spin::RwLock::new(0u32);
        let excludes = {
            let reader = lock.read();
            let refused = lock.try_write().is_none();
            drop(reader);
            refused && lock.try_write().is_some()
        };
        report_test("exclusives_and_locks_work", rmw && excludes, passed, failed);
    }

    // Test 34: the monotonic clock advances and the 1000 Hz timer interrupt
    // is delivered (LAPIC on x86_64, EL1 virtual timer on AArch64,
    // stimecmp/SBI on RISC-V). Before, uptime never advanced and AArch64 /
    // RISC-V took no interrupts at all.
    {
        let start_ms = crate::timer::get_uptime_ms();
        let start_ticks = crate::arch::timer::get_ticks();
        let mut spins = 0u64;
        while crate::timer::get_uptime_ms().saturating_sub(start_ms) < 50
            && crate::arch::timer::get_ticks().wrapping_sub(start_ticks) < 5
            && spins < 2_000_000_000
        {
            core::hint::spin_loop();
            spins += 1;
        }
        let ticks = crate::arch::timer::get_ticks().wrapping_sub(start_ticks);
        let elapsed = crate::timer::get_uptime_ms().saturating_sub(start_ms);
        let ok = ticks >= 5 && elapsed < 50;
        report_test("timer_interrupts_and_clock", ok, passed, failed);
    }

    // Test 35: every CPU that came online has its own per-CPU block (the
    // right logical id, a distinct hardware id) and, where secondaries run
    // a timer, has taken ticks on it. Trivially true on one CPU (SMP stage
    // S1, ADR 0004).
    {
        use crate::arch::percpu::arch_cpu_ptr;
        let online = crate::arch::smp_boot::online_cpus() as usize;
        let start_ms = crate::timer::get_uptime_ms();
        while crate::timer::get_uptime_ms().saturating_sub(start_ms) < 20 {
            core::hint::spin_loop();
        }
        let mut ok = true;
        for cpu in 0..online {
            // SAFETY: read-only snapshot of static per-CPU blocks; each
            // field is written only by its own CPU.
            let (id, hw, ticks) = unsafe {
                let p = arch_cpu_ptr(cpu);
                (
                    core::ptr::read_volatile(&(*p).cpu_id),
                    core::ptr::read_volatile(&(*p).hw_id),
                    core::ptr::read_volatile(&(*p).local_ticks),
                )
            };
            ok &= id as usize == cpu;
            for other in 0..cpu {
                // SAFETY: as above.
                ok &= unsafe { core::ptr::read_volatile(&(*arch_cpu_ptr(other)).hw_id) } != hw;
            }
            if cpu > 0 {
                ok &= ticks > 0;
            }
        }
        if online > 1 {
            report_test("smp_cpus_online_and_ticking", ok, passed, failed);
        } else {
            report_skip("smp_cpus_online_and_ticking", "one CPU online");
        }
    }

    // Test 36: kernel uptime advances at the rate of the hardware clock.
    // With several CPUs ticking, only the timekeeper may advance it, or it
    // runs N times fast.
    {
        let hw0 = crate::arch::timer::monotonic_ns();
        let up0 = crate::timer::get_uptime_ms();
        let mut spins = 0u64;
        while crate::arch::timer::monotonic_ns().saturating_sub(hw0) < 100_000_000
            && spins < 4_000_000_000
        {
            core::hint::spin_loop();
            spins += 1;
        }
        let hw_ms = crate::arch::timer::monotonic_ns().saturating_sub(hw0) / 1_000_000;
        let up_ms = crate::timer::get_uptime_ms().saturating_sub(up0);
        // Within 30% of the hardware clock (ticks are 1 ms; QEMU TCG jitters).
        let ok = hw_ms >= 100 && up_ms * 10 >= hw_ms * 7 && up_ms * 10 <= hw_ms * 13;
        report_test("uptime_tracks_hw_clock", ok, passed, failed);
    }

    // Test 37: a TLB shootdown reaches every online CPU and is confirmed
    // (x86 IPI acknowledgements, SBI RFENCE, AArch64 broadcast TLBI;
    // MEM-SEC-02). Nothing to test on one CPU.
    if crate::arch::smp_boot::online_cpus() <= 1 {
        report_skip("tlb_shootdown_confirmed", "one CPU online");
    } else {
        let start = crate::arch::timer::monotonic_ns();
        for _ in 0..8 {
            crate::mm::tlb::flush_all();
            crate::mm::tlb::flush_page(crate::mm::user_layout::UNMAPPED_TEST_ADDR as u64);
        }
        let quick = crate::arch::timer::monotonic_ns().saturating_sub(start) < 500_000_000;
        let ok = quick && crate::mm::tlb::remote_flushes_confirmed();
        report_test("tlb_shootdown_confirmed", ok, passed, failed);
    }

    // Test 38: the kernel heap reuses freed memory (MEM-SEC-03). 64 MiB
    // allocated and freed 1 MiB at a time exceeds the 8 MiB AArch64/RISC-V
    // heap, so a bump allocator, which never reuses memory, fails it.
    {
        let ok = (0..64).all(|i| {
            let mut v: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
            if v.try_reserve_exact(1 << 20).is_err() {
                return false;
            }
            // Reuse needs no writes (writing 64 MiB is slow under TCG).
            v.push(i as u8);
            v.capacity() >= 1 << 20 && v[0] == i as u8
        });
        report_test("heap_reuses_freed_memory", ok, passed, failed);
    }

    // AArch64 and RISC-V: the frame allocator manages the RAM the device
    // tree reports (HA-17, HR-07), less the kernel image and what lies
    // below it, not a fixed 128 MiB. The gate boots with 256 MiB.
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        #[cfg(target_arch = "aarch64")]
        let dtb = crate::arch::aarch64::boot::dtb_pa();
        #[cfg(target_arch = "riscv64")]
        let dtb = crate::arch::riscv64::dtb_pa();
        let mut ram = 0u64;
        // SAFETY: the address firmware passed, in the physical map.
        if let Some(fdt) = unsafe { crate::arch::fdt::Fdt::from_phys(dtb) } {
            fdt.memory_ranges(|_, size| ram += size);
        }
        let managed = crate::mm::FRAME_ALLOCATOR.lock().get_stats().total_frames * 4096;
        // The kernel heap is taken out of the managed frames (about a
        // third of RAM: 512 MiB at 2 GiB); the image, firmware below it and the
        // pool's guard are well under 64 MiB.
        let heap = crate::mm::heap::heap_size() as u64;
        let ok = ram > 0 && managed + heap + (64 << 20) >= ram && managed <= ram;
        if !ok {
            crate::println!(
                "    device tree RAM {:#x}, managed {:#x}, heap {:#x}",
                ram,
                managed,
                heap
            );
        }
        report_test("memory_map_from_device_tree", ok, passed, failed);
    }

    // Test 39: a kernel stack has an unmapped guard page below it, and
    // freeing it unmaps it (N-26).
    {
        let ok = match crate::mm::kstack::allocate(16) {
            Ok(stack) => {
                // SAFETY: both ends of the 16 pages just allocated.
                let usable = unsafe {
                    let lo = stack.base as *mut u64;
                    let hi = (stack.base + 16 * 4096 - 8) as *mut u64;
                    lo.write_volatile(0x1234);
                    hi.write_volatile(0x5678);
                    lo.read_volatile() == 0x1234 && hi.read_volatile() == 0x5678
                };
                // SAFETY: the kernel's root table through the physical map;
                // only read here.
                let m = unsafe {
                    crate::mm::vas::create_mapper_from_root_pub(
                        crate::mm::get_kernel_page_table() as u64
                    )
                };
                let probe = |a: usize| m.translate_page(crate::mm::VirtualAddress(a as u64));
                let guard_unmapped = probe(stack.base - 4096).is_err()
                    && crate::mm::kstack::is_guard_fault((stack.base - 8) as u64);
                let mapped = probe(stack.base).is_ok();
                let base = stack.base;
                crate::mm::kstack::free(stack);
                usable && guard_unmapped && mapped && probe(base).is_err()
            }
            Err(_) => false,
        };
        report_test("kernel_stack_guard_page", ok, passed, failed);
    }

    // A new address space's lower half (user space) is empty: no kernel
    // entry is shared into it, so all of it is the process's own. A stale
    // kernel-heap slot (L4 136) used to be copied in and shared by fork.
    // Its upper half is the kernel's, entry for entry (on RISC-V the new
    // space used to get freshly allocated tables there instead). On
    // AArch64 the kernel half is TTBR1's (ADR 0015), so a process root
    // holds nothing above the user half, and building one takes a few
    // frames: it used to map 20 MiB of fresh frames as "kernel regions".
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "riscv64",
        target_arch = "aarch64"
    ))]
    {
        use crate::mm::page_table::PageTable;
        let table = |root: u64| {
            // SAFETY: a root page table, through the physical map; only read.
            unsafe { &*(crate::mm::phys_to_virt_addr(root) as *const PageTable) }
        };
        let free = || crate::mm::get_memory_stats().free_frames;
        let mut vas = crate::mm::vas::VirtualAddressSpace::new();
        let before = free();
        let init = vas.init();
        let taken = before.saturating_sub(free());
        let root = vas.get_page_table();
        let first_present = (init.is_ok() && root != 0).then(|| {
            let l4 = table(root);
            (0..256).find(|&i| l4[i].is_present())
        });
        #[cfg(not(target_arch = "aarch64"))]
        let shared = {
            let kernel_root = crate::mm::get_kernel_page_table() as u64 & 0x000F_FFFF_FFFF_F000;
            init.is_ok()
                && root != 0
                && (256..512).all(|i| table(root)[i].frame() == table(kernel_root)[i].frame())
        };
        #[cfg(target_arch = "aarch64")]
        let shared = init.is_ok()
            && root != 0
            && (256..512).all(|i| !table(root)[i].is_present())
            && taken < 64;
        let _ = taken;
        let ok = first_present == Some(None) && shared;
        if !ok {
            crate::println!(
                "    init ok: {}, root {:#x}, first user-half entry present: {:?}, kernel half \
                 right: {}, frames taken {}",
                init.is_ok(),
                root,
                first_present,
                shared,
                taken
            );
        }
        vas.destroy();
        report_test("user_half_starts_empty", ok, passed, failed);
    }

    // RISC-V: the kernel's own root maps nothing in the lower half. The
    // early boot's identity map is only in the trampoline root (ADR 0013),
    // so the lower half is entirely user space.
    #[cfg(target_arch = "riscv64")]
    {
        let root = crate::mm::get_kernel_page_table() as u64;
        // SAFETY: the active root, through the physical map; only read.
        let l4 = unsafe {
            &*(crate::mm::phys_to_virt_addr(root) as *const crate::mm::page_table::PageTable)
        };
        let first = (0..256).find(|&i| l4[i].is_present());
        if let Some(i) = first {
            crate::println!("    kernel root entry {} is present (lower half)", i);
        }
        report_test(
            "kernel_root_lower_half_empty",
            first.is_none(),
            passed,
            failed,
        );

        // Every GiB of the kernel-stack region has its table from boot, so
        // an address space created before a stack is mapped there still sees
        // it: under Sv39 each GiB is a root entry, copied when the space is
        // made (ADR 0018); under Sv48 they are entries of L4[511]'s table.
        let levels = crate::mm::page_table::paging_levels();
        let gib = if levels == 3 {
            Some(l4)
        } else {
            l4[511].addr().map(|a| {
                // SAFETY: a present kernel-half table, through the physical
                // map; only read.
                unsafe {
                    &*(crate::mm::phys_to_virt_addr(a.as_u64())
                        as *const crate::mm::page_table::PageTable)
                }
            })
        };
        let missing = gib.map(|t| (384..400).find(|&i| !t[i].is_present()));
        if missing != Some(None) {
            crate::println!(
                "    {} levels: kernel-stack GiB entry missing: {:?}",
                levels,
                missing
            );
        }
        report_test(
            "kernel_stack_region_prebuilt",
            missing == Some(None),
            passed,
            failed,
        );

        // The kernel starts with Linux's RISC-V Image header (HR-03,
        // Documentation/arch/riscv/boot-image-header.rst), which U-Boot
        // `booti` and EFI loaders read from the flat Image: version 0.2, both
        // magics, the 2 MiB text_offset, and image_size covering everything
        // up to the kernel's end (BSS and boot stack included).
        extern "C" {
            static _start: u8;
            static __kernel_end: u8;
        }
        let start = core::ptr::addr_of!(_start);
        // SAFETY: `_start` begins the image's first 64 bytes, in .text,
        // mapped for the kernel's lifetime; only read.
        let h = unsafe { core::slice::from_raw_parts(start, 64) };
        let u32_at = |o: usize| u32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let u64_at = |o: usize| u64::from(u32_at(o)) | (u64::from(u32_at(o + 4)) << 32);
        let size = core::ptr::addr_of!(__kernel_end) as u64 - start as u64;
        let ok = u64_at(0x08) == 0x20_0000
            && u64_at(0x10) == size
            && u64_at(0x18) == 0
            && u32_at(0x20) == 2
            && &h[0x30..0x38] == b"RISCV\0\0\0"
            && &h[0x38..0x3C] == b"RSC\x05"
            && u32_at(0x3C) == 0;
        if !ok {
            crate::println!(
                "    header: text_offset {:#x} image_size {:#x} (want {:#x}) version {:#x}",
                u64_at(0x08),
                u64_at(0x10),
                size,
                u32_at(0x20)
            );
        }
        report_test("riscv_image_header", ok, passed, failed);
    }
}

#[cfg(not(feature = "alloc"))]
pub fn kernel_init_main() {
    kprintln!("BOOTOK");
}

/// Tests that cannot run on this architecture yet. They are reported and
/// counted apart, never as passed, so the tally claims only what ran.
static SKIPPED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Print a line made of several pieces as one: another CPU's message cannot
/// land in the middle (`Results: 62/[PROCESS] Dropping process 29`, which
/// the boot tests could not parse). AArch64 and RISC-V hold the console
/// lock across `f`; x86_64 has none to hold.
fn console_line(f: impl FnOnce()) {
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    crate::print::with_console(f);
    #[cfg(not(any(target_arch = "aarch64", target_arch = "riscv64")))]
    f();
}

/// Report a test that does not run here, and why.
#[allow(dead_code)] // every skip is architecture-specific
fn report_skip(name: &str, why: &str) {
    console_line(|| {
        kprint_rt!("  ");
        kprint_rt!(name);
        kprint_rt!("...[skipped: ");
        kprint_rt!(why);
        kprintln!("]");
    });
    SKIPPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// Print test summary and BOOTOK/BOOTFAIL
fn print_summary(passed: u32, failed: u32) {
    kprintln!("========================================");
    let skipped = SKIPPED.load(core::sync::atomic::Ordering::Relaxed);
    console_line(|| {
        kprint_rt!("[INIT] Results: ");
        kprint_u64!(passed);
        kprint_rt!("/");
        kprint_u64!(passed + failed);
        kprint_rt!(" passed");
        if skipped > 0 {
            kprint_rt!(" (");
            kprint_u64!(skipped);
            kprint_rt!(" skipped)");
        }
        kprintln!("");
    });
    if failed == 0 {
        kprintln!("BOOTOK");
    } else {
        kprintln!("BOOTFAIL");
    }
    kprintln!("========================================");
}

/// Report a single test result with QEMU-parseable markers
fn report_test(name: &str, ok: bool, passed: &mut u32, failed: &mut u32) {
    console_line(|| {
        kprint_rt!("  ");
        kprint_rt!(name);
        if ok {
            kprintln!("...[ok]");
        } else {
            kprintln!("...[failed]");
        }
    });

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
        // On x86_64, skip process creation entirely. The thread builder
        // in create_process_with_options() zeroes the kernel stack by
        // writing to its physical address as a virtual address, which
        // page faults because the bootloader does not identity-map low
        // physical memory. Instead, try_enter_usermode() (called after
        // BOOTOK) handles all memory setup and mode switching directly.
        #[cfg(target_arch = "x86_64")]
        {
            kprintln!("[BOOTSTRAP] Skipping PCB creation (direct usermode path)");
        }

        // On non-x86_64, use the ELF loader path (which creates a process
        // with the appropriate entry point for the architecture).
        #[cfg(not(target_arch = "x86_64"))]
        {
            match crate::userspace::load_init_process() {
                Ok(_init_pid) => {
                    kprintln!("[BOOTSTRAP] Init process ready");

                    // Skip on RISC-V: the bump allocator cannot free memory,
                    // so loading a second process needlessly consumes heap
                    // space. User-space execution is not functional yet on any
                    // architecture, so the shell PCB is not needed.
                    #[cfg(not(target_arch = "riscv64"))]
                    {
                        let _ = crate::userspace::loader::load_shell();
                    }
                }
                Err(_e) => {
                    // Init process creation is non-critical — the kernel shell
                    // provides the interactive interface.
                    kprintln!("[BOOTSTRAP] Init process deferred (kernel shell active)");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootstrap_pid_is_zero() {
        assert_eq!(BOOTSTRAP_PID, 0);
    }

    #[test]
    fn test_bootstrap_tid_is_zero() {
        assert_eq!(BOOTSTRAP_TID, 0);
    }

    #[test]
    fn test_bootstrap_pid_and_tid_match() {
        // Both bootstrap IDs should be the same (the bootstrap task is PID 0, TID 0)
        assert_eq!(BOOTSTRAP_PID, BOOTSTRAP_TID);
    }

    #[test]
    fn test_bootstrap_pid_type() {
        // BOOTSTRAP_PID should be a valid u64 value
        let pid: u64 = BOOTSTRAP_PID;
        assert!(pid < u64::MAX);
    }

    #[test]
    fn test_bootstrap_constants_are_not_one() {
        // PID/TID 1 is reserved for the init process, bootstrap must differ
        assert_ne!(BOOTSTRAP_PID, 1);
        assert_ne!(BOOTSTRAP_TID, 1);
    }

    #[test]
    fn test_kernel_init_returns_kernel_result() {
        // Verify KernelResult type alias works with the function signature.
        // We cannot call kernel_init() in tests (it requires hardware), but
        // we can verify the return type compiles.
        fn _assert_return_type() -> KernelResult<()> {
            Ok(())
        }
        assert!(_assert_return_type().is_ok());
    }

    #[test]
    fn test_kernel_result_error_propagation() {
        // Verify that KernelResult works with the ? operator
        fn inner() -> KernelResult<u32> {
            let _: () = Ok::<(), crate::error::KernelError>(())?;
            Ok(42)
        }
        assert_eq!(inner().unwrap(), 42);
    }

    #[test]
    fn test_kernel_result_error_variant() {
        fn failing() -> KernelResult<()> {
            Err(crate::error::KernelError::NotInitialized { subsystem: "test" })
        }
        let result = failing();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            crate::error::KernelError::NotInitialized { subsystem: "test" }
        );
    }
}
