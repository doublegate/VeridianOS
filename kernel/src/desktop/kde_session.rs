//! KDE Plasma 6 session manager
//!
//! Orchestrates launching KDE Plasma 6 as the default desktop session.
//! Initializes the desktop subsystem, hands off the framebuffer to KWin,
//! launches the KDE init script as a user process, and restores the text
//! console when the session ends.
//!
//! If the KDE session exits within a few seconds (indicating startup
//! failure), automatically falls back to the built-in desktop environment.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

/// Minimum session lifetime (in approximate loop iterations) before we
/// consider the session to have started successfully. If the KDE init
/// script exits before this threshold, we treat it as a startup failure
/// and fall back to the built-in DE.
///
/// Each iteration of the wait loop is roughly 10ms via a busy-wait yield,
/// so 500 iterations ~ 5 seconds.
const MIN_SESSION_LIFETIME_ITERS: u64 = 500;

/// KDE init script path (used when /bin/sh is available).
const KDE_INIT_SCRIPT: &str = "/usr/share/veridian/veridian-kde-init.sh";

/// KWin Wayland compositor binary path (direct exec, no shell needed).
const KWIN_WAYLAND: &str = "/usr/bin/kwin_wayland";

/// D-Bus daemon binary path.
const DBUS_DAEMON: &str = "/usr/bin/dbus-daemon";

/// Environment variables passed to the KDE init script.
///
/// Key notes for KWin DRM mode (KWin IS the Wayland compositor):
/// - QT_QPA_PLATFORM must match the statically linked QPA plugin name
///   ("wayland-org.kde.kwin.qpa"), NOT "veridian" or "wayland".
/// - WAYLAND_DISPLAY must NOT be set -- KWin creates the Wayland display, it
///   does not connect to one. Setting it causes KWin to try to connect as a
///   client of a nonexistent compositor, which fails.
/// - DISPLAY must NOT be set -- no X11 server running.
/// - QT_QPA_EGLFS_INTEGRATION can hint to use KMS/DRM directly.
/// - KWIN_DRM_USE_EGL=0 tells KWin DRM to skip EGL init (softpipe only).
/// - KWIN_FORCE_OWN_QPA=1 bypasses the
///   applicationFilePath().endsWith("kwin_wayland") check in
///   KWinIntegrationPlugin::create().  Without /proc/self/exe support, Qt's
///   applicationFilePath() returns an empty or unresolvable string and the KWin
///   QPA plugin refuses to load.
const KDE_ENV: &[&str] = &[
    "HOME=/root",
    "USER=root",
    "SHELL=/bin/sh",
    "PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
    "XDG_RUNTIME_DIR=/run/user/0",
    "XDG_SESSION_TYPE=wayland",
    "XDG_CURRENT_DESKTOP=KDE",
    "XDG_SESSION_ID=1",
    "XDG_SEAT=seat0",
    "XDG_VTNR=1",
    "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket",
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/0/bus",
    "QT_QPA_PLATFORM=wayland-org.kde.kwin.qpa",
    "KWIN_FORCE_OWN_QPA=1",
    "KWIN_DRM_USE_EGL=0",
    // Tell KWin's DRM backend to use /dev/dri/card0 directly, bypassing
    // udev device enumeration (which requires a running udev daemon).
    "KWIN_DRM_DEVICES=/dev/dri/card0",
    // Force noop/direct session backend -- avoids logind D-Bus dependency.
    // libseat noop backend opens DRM devices directly without seat management.
    "LIBSEAT_BACKEND=noop",
    // Skip libinput initialization -- avoids udev device enumeration for
    // input devices which would fail without a running udev daemon.
    "KWIN_WAYLAND_NO_LIBINPUT=1",
    "LANG=en_US.UTF-8",
];

/// Start a KDE Plasma 6 session.
///
/// This function blocks until the KDE session ends (user logout) or
/// KDE fails to start. On failure, it falls back to the built-in DE.
///
/// # Flow
/// 1. Initialize desktop subsystem (Wayland, fonts, etc.)
/// 2. Disable fbcon output (KWin will drive the framebuffer)
/// 3. Launch KDE init script via `load_user_program`
/// 4. Wait for the process to complete (blocks kernel thread)
/// 5. Restore fbcon on return
///
/// If the process exits too quickly, falls back to built-in DE.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub fn start_kde_session() {
    println!("[KDE] Starting KDE Plasma 6 session...");

    // Step 1: Initialize desktop subsystem if not already done.
    // If already initialized (e.g. by the boot sequence), that's fine --
    // the built-in DE was showing and we're replacing it with KDE.
    match crate::desktop::init() {
        Ok(()) => println!("[KDE] Desktop subsystem initialized"),
        Err(crate::error::KernelError::InvalidState { .. }) => {
            println!("[KDE] Desktop already initialized, switching to KDE session");
        }
        Err(e) => {
            println!("[KDE] Desktop subsystem init failed: {:?}", e);
            println!("[KDE] Falling back to built-in desktop...");
            crate::desktop::renderer::start_desktop();
            return;
        }
    }

    // Step 2: Disable fbcon -- KWin will take over DRM/framebuffer
    crate::graphics::fbcon::disable_output();
    println!("[KDE] Framebuffer console disabled (KWin will drive display)");

    // Step 2b: Pre-create a D-Bus system bus socket so that kwin's
    // connect() call succeeds (returns 0) instead of returning -ENOENT.
    // KWin uses D-Bus to query logind for session/seat info. Without a
    // listening socket, connect() fails and KWin may leave its session
    // object partially initialized, leading to a NULL dereference later.
    // The socket accepts connections but no D-Bus daemon responds -- KWin
    // handles D-Bus communication failures more gracefully than connect
    // failures.
    setup_dbus_stub_socket();

    // Step 3: Launch KDE session (tries init script, then direct exec)
    let pid = match launch_kde_init() {
        Ok(pid) => {
            println!("[KDE] KDE process launched (PID {})", pid.0);
            pid
        }
        Err(e) => {
            println!("[KDE] Failed to launch KDE session: {:?}", e);
            println!("[KDE] Falling back to built-in desktop...");
            crate::graphics::fbcon::enable_output();
            crate::graphics::fbcon::mark_all_dirty_and_flush();
            crate::desktop::renderer::start_desktop();
            return;
        }
    };

    // Step 4: Record start time and run the user process (blocks)
    let start_iter = get_iteration_counter();
    run_kde_process(pid);
    let end_iter = get_iteration_counter();

    // Step 5: Restore fbcon
    crate::graphics::fbcon::enable_output();
    crate::graphics::fbcon::mark_all_dirty_and_flush();
    println!("[KDE] Session ended, text console restored");

    // Step 6: Check if session exited too quickly (startup failure)
    let elapsed = end_iter.saturating_sub(start_iter);
    if elapsed < MIN_SESSION_LIFETIME_ITERS {
        println!(
            "[KDE] Session exited unexpectedly (within ~{}s of launch)",
            elapsed / 100
        );
        println!("[KDE] Falling back to built-in desktop...");
        crate::desktop::renderer::start_desktop();
    }
}

/// Launch the KDE session as a user process.
///
/// Tries three strategies in order:
/// 1. Shell-based: `/bin/sh` running the init script (full orchestration)
/// 2. Direct KWin: load `kwin_wayland` directly (no shell needed)
/// 3. Direct D-Bus: load `dbus-daemon` as a smoke test
///
/// Strategy 1 requires a real `/bin/sh` in the rootfs. Strategies 2-3
/// load cross-compiled static ELF binaries directly from BlockFS.
#[cfg(feature = "alloc")]
fn launch_kde_init() -> Result<crate::process::ProcessId, crate::error::KernelError> {
    // Strategy 1: Try shell-based init script (full KDE startup sequence)
    let shell_argv: &[&str] = &["sh", KDE_INIT_SCRIPT, "--from-kernel"];
    if let Ok(pid) = crate::userspace::load_user_program("/bin/sh", shell_argv, KDE_ENV) {
        println!("[KDE] Launched via init script (/bin/sh)");
        return Ok(pid);
    }

    // Strategy 2: Direct-exec kwin_wayland.
    // KWin auto-selects the DRM/KMS backend when run as a standalone compositor.
    // The KWIN_DRM_DEVICES env var tells it which device to open, bypassing udev.
    // NOTE: We intentionally omit --drm because KWin 6.3.5's DRM platform plugin
    // may not be registered in the static binary. KWin auto-detection is more
    // robust for statically linked builds.
    let kwin_argv: &[&str] = &["kwin_wayland"];
    if let Ok(pid) = crate::userspace::load_user_program(KWIN_WAYLAND, kwin_argv, KDE_ENV) {
        println!("[KDE] Launched kwin_wayland directly");
        return Ok(pid);
    }
    println!("[KDE] kwin_wayland not loadable, trying dbus-daemon...");

    // Strategy 3: Direct-exec dbus-daemon as smoke test
    let dbus_argv: &[&str] = &["dbus-daemon", "--session", "--nofork"];
    if let Ok(pid) = crate::userspace::load_user_program(DBUS_DAEMON, dbus_argv, KDE_ENV) {
        println!("[KDE] Launched dbus-daemon (smoke test)");
        return Ok(pid);
    }

    Err(crate::error::KernelError::NotFound {
        resource: "KDE binaries (kwin_wayland, dbus-daemon)",
        id: 0,
    })
}

/// Run a KDE process using the same pattern as
/// `bootstrap::run_user_process_scheduled`.
///
/// This blocks the current kernel thread until the process exits, then
/// cleans up page tables and reaps the zombie process entry.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn run_kde_process(pid: crate::process::ProcessId) {
    use crate::process::get_process;

    // Save page table root before running (needed for cleanup after exit)
    let saved_pt_root = if let Some(proc) = get_process(pid) {
        proc.memory_space.lock().get_page_table()
    } else {
        0
    };

    // Look up first thread ID for boot-current tracking
    let tid = if let Some(proc) = get_process(pid) {
        let threads = proc.threads.lock();
        threads.values().next().map(|t| t.tid)
    } else {
        None
    };

    // Run the process (blocks until it exits)
    if let Some(tid) = tid {
        crate::process::set_boot_current(pid, tid);
        crate::bootstrap::run_user_process(pid);
        crate::process::clear_boot_current();
    } else {
        crate::bootstrap::run_user_process(pid);
    }

    // Boot CR3 is now restored. Free page table hierarchy frames.
    // If the process called exec(), the page table was replaced -- free both.
    let current_pt_root = if let Some(proc) = get_process(pid) {
        proc.memory_space.lock().get_page_table()
    } else {
        0
    };

    if current_pt_root != 0 && current_pt_root != saved_pt_root {
        crate::mm::vas::free_user_page_table_frames(current_pt_root);
    }
    if saved_pt_root != 0 {
        crate::mm::vas::free_user_page_table_frames(saved_pt_root);
    }

    // Clear page_table_root to prevent double-free
    if let Some(proc) = get_process(pid) {
        proc.memory_space.lock().set_page_table(0);
    }

    // Reap zombie process
    if let Some(proc) = get_process(pid) {
        let state = proc.get_state();
        if state == crate::process::ProcessState::Zombie
            || state == crate::process::ProcessState::Dead
        {
            crate::process::table::remove_process(pid);
        }
    }
}

/// Simple monotonic counter for measuring elapsed time.
///
/// Uses a static atomic counter incremented by a periodic timer or
/// approximated via TSC reads.
fn get_iteration_counter() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // Use TSC as a rough monotonic counter
        // SAFETY: RDTSC is always available on x86_64
        unsafe { core::arch::x86_64::_rdtsc() / 1_000_000 }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// Create a stub D-Bus system bus socket at `/run/dbus/system_bus_socket`.
///
/// KWin connects to D-Bus to query logind for session/seat information.
/// If `connect()` returns `-ENOENT` (no socket file), KWin's session
/// initialization may leave internal pointers NULL, causing a crash later
/// when the DRM backend tries to access session properties.
///
/// By pre-creating a bound+listening Unix domain socket at the expected
/// path, `connect()` succeeds. KWin's D-Bus client will then fail at
/// the protocol level (no daemon responds), but this failure is handled
/// gracefully -- KWin falls back to a noop session.
#[cfg(feature = "alloc")]
fn setup_dbus_stub_socket() {
    use crate::net::unix_socket;

    // Create stub sockets for both the system bus and session bus.
    // KWin uses the system bus for logind/session management, and KConfig
    // uses the session bus for KConfigWatcher (config change notifications).
    // Without listening sockets, connect() returns -ENOENT and Qt's D-Bus
    // client may leave internal state inconsistent, causing NULL pointer
    // crashes in KConfigWatcher's constructor.
    let bus_paths = ["/run/dbus/system_bus_socket", "/run/user/0/bus"];

    for path in &bus_paths {
        let stub_id = match unix_socket::socket_create(unix_socket::UnixSocketType::Stream, 0) {
            Ok(id) => id,
            Err(_) => {
                println!("[KDE] D-Bus stub ({}): create failed", path);
                continue;
            }
        };

        if unix_socket::socket_bind(stub_id, path).is_err() {
            println!("[KDE] D-Bus stub ({}): bind failed", path);
            continue;
        }

        match unix_socket::socket_listen(stub_id, 8) {
            Ok(()) => {
                println!("[KDE] D-Bus stub listening at {}", path);
            }
            Err(_) => {
                println!("[KDE] D-Bus stub ({}): listen failed", path);
            }
        }
    }
}

/// Stub for non-x86_64 architectures.
#[cfg(not(all(feature = "alloc", target_arch = "x86_64")))]
pub fn start_kde_session() {
    crate::println!("[KDE] KDE session not supported on this architecture");
    crate::println!("[KDE] Falling back to built-in desktop...");
    crate::desktop::renderer::start_desktop();
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kde_env_has_required_vars() {
        let has_desktop = KDE_ENV
            .iter()
            .any(|e| e.starts_with("XDG_CURRENT_DESKTOP="));
        let has_qpa = KDE_ENV
            .iter()
            .any(|e| e == &"QT_QPA_PLATFORM=wayland-org.kde.kwin.qpa");
        let has_runtime_dir = KDE_ENV.iter().any(|e| e.starts_with("XDG_RUNTIME_DIR="));
        // WAYLAND_DISPLAY must NOT be set -- KWin IS the compositor, it creates the
        // display
        let has_wayland_display = KDE_ENV.iter().any(|e| e.starts_with("WAYLAND_DISPLAY="));
        // DISPLAY must NOT be set -- no X11 server
        let has_display = KDE_ENV.iter().any(|e| e.starts_with("DISPLAY="));
        let has_force_qpa = KDE_ENV.iter().any(|e| e == &"KWIN_FORCE_OWN_QPA=1");
        assert!(has_desktop, "XDG_CURRENT_DESKTOP must be set");
        assert!(has_qpa, "QT_QPA_PLATFORM must be wayland-org.kde.kwin.qpa");
        assert!(
            has_force_qpa,
            "KWIN_FORCE_OWN_QPA must be set (no /proc/self/exe)"
        );
        assert!(has_runtime_dir, "XDG_RUNTIME_DIR must be set");
        assert!(
            !has_wayland_display,
            "WAYLAND_DISPLAY must NOT be set (KWin creates it)"
        );
        assert!(!has_display, "DISPLAY must NOT be set (no X11)");
    }

    #[test]
    fn test_min_session_lifetime() {
        // 500 iterations * ~10ms = ~5 seconds
        assert!(MIN_SESSION_LIFETIME_ITERS >= 100);
    }

    #[test]
    fn test_kde_init_script_path() {
        assert!(KDE_INIT_SCRIPT.starts_with("/usr/share/"));
        assert!(KDE_INIT_SCRIPT.ends_with(".sh"));
    }

    #[test]
    fn test_kde_binary_paths() {
        assert!(KWIN_WAYLAND.starts_with("/usr/bin/"));
        assert!(DBUS_DAEMON.starts_with("/usr/bin/"));
    }
}
