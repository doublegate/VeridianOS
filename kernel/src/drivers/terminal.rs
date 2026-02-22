//! Terminal state tracking for /dev/console and /dev/tty*
//!
//! Provides POSIX-compatible terminal attributes (struct termios equivalent)
//! and window size tracking. The terminal state is queried and modified via
//! SYS_IOCTL (syscall 112) with TCGETS/TCSETS/TIOCGWINSZ requests.
//!
//! The state is used by the console read path to implement canonical vs raw
//! mode, echo control, and control character handling.

use spin::Mutex;

use crate::sync::once_lock::OnceLock;

// =========================================================================
// Terminal ioctl request codes (matching Linux values for ABI compatibility)
// =========================================================================

/// Get terminal attributes (struct termios).
pub const TCGETS: usize = 0x5401;
/// Set terminal attributes immediately.
pub const TCSETS: usize = 0x5402;
/// Set terminal attributes after draining output.
pub const TCSETSW: usize = 0x5403;
/// Set terminal attributes after draining output and flushing input.
pub const TCSETSF: usize = 0x5404;
/// Get terminal window size (struct winsize).
pub const TIOCGWINSZ: usize = 0x5413;
/// Set terminal window size.
pub const TIOCSWINSZ: usize = 0x5414;
/// Get foreground process group ID.
pub const TIOCGPGRP: usize = 0x540F;
/// Set foreground process group ID.
pub const TIOCSPGRP: usize = 0x5410;

// =========================================================================
// termios flag constants (matching POSIX / Linux values)
// =========================================================================

// c_iflag bits
pub const ICRNL: u32 = 0o0000400;
pub const IXON: u32 = 0o0002000;

// c_oflag bits
pub const OPOST: u32 = 0o0000001;
pub const ONLCR: u32 = 0o0000004;

// c_cflag bits
pub const CS8: u32 = 0o0000060;
pub const CREAD: u32 = 0o0000200;
pub const HUPCL: u32 = 0o0002000;

// c_lflag bits
pub const ISIG: u32 = 0o0000001;
pub const ICANON: u32 = 0o0000002;
pub const ECHO: u32 = 0o0000010;
pub const ECHOE: u32 = 0o0000020;
pub const ECHOK: u32 = 0o0000040;
pub const IEXTEN: u32 = 0o0100000;

// c_cc indices
pub const VINTR: usize = 0;
pub const VQUIT: usize = 1;
pub const VERASE: usize = 2;
pub const VKILL: usize = 3;
pub const VEOF: usize = 4;
pub const VTIME: usize = 5;
pub const VMIN: usize = 6;
pub const VSTART: usize = 8;
pub const VSTOP: usize = 9;
pub const VSUSP: usize = 10;

/// Number of control characters.
pub const NCCS: usize = 32;

// =========================================================================
// Terminal state structures (repr(C) to match user-space struct termios)
// =========================================================================

/// Terminal attributes, matching the C `struct termios` layout exactly.
///
/// Fields are in the same order as the userland header `termios.h`:
///   c_iflag, c_oflag, c_cflag, c_lflag, c_cc[32], c_ispeed, c_ospeed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KernelTermios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_cc: [u8; NCCS],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

impl KernelTermios {
    /// Create default terminal attributes (cooked mode, echo on).
    ///
    /// These match the standard POSIX defaults for a serial console:
    /// canonical mode, echo enabled, signal processing on.
    pub const fn default_console() -> Self {
        let mut cc = [0u8; NCCS];
        cc[VINTR] = 3; // Ctrl-C
        cc[VQUIT] = 28; // Ctrl-backslash
        cc[VERASE] = 127; // DEL
        cc[VKILL] = 21; // Ctrl-U
        cc[VEOF] = 4; // Ctrl-D
        cc[VTIME] = 0;
        cc[VMIN] = 1;
        cc[VSTART] = 17; // Ctrl-Q
        cc[VSTOP] = 19; // Ctrl-S
        cc[VSUSP] = 26; // Ctrl-Z

        Self {
            c_iflag: ICRNL | IXON,
            c_oflag: OPOST | ONLCR,
            c_cflag: CS8 | CREAD | HUPCL,
            c_lflag: ECHO | ECHOE | ECHOK | ICANON | ISIG | IEXTEN,
            c_cc: cc,
            c_ispeed: 38400,
            c_ospeed: 38400,
        }
    }

    /// Check if canonical (line-buffered) mode is enabled.
    #[inline]
    pub fn is_canonical(&self) -> bool {
        self.c_lflag & ICANON != 0
    }

    /// Check if echo is enabled.
    #[inline]
    pub fn is_echo(&self) -> bool {
        self.c_lflag & ECHO != 0
    }

    /// Get the VMIN value (minimum characters for non-canonical read).
    #[inline]
    pub fn vmin(&self) -> u8 {
        self.c_cc[VMIN]
    }

    /// Get the VTIME value (timeout for non-canonical read, in tenths of a
    /// second).
    #[inline]
    pub fn vtime(&self) -> u8 {
        self.c_cc[VTIME]
    }

    /// Get the erase character (typically DEL or backspace).
    #[inline]
    pub fn verase(&self) -> u8 {
        self.c_cc[VERASE]
    }

    /// Get the kill character (typically Ctrl-U).
    #[inline]
    pub fn vkill(&self) -> u8 {
        self.c_cc[VKILL]
    }

    /// Get the EOF character (typically Ctrl-D).
    #[inline]
    pub fn veof(&self) -> u8 {
        self.c_cc[VEOF]
    }
}

/// Terminal window size, matching the C `struct winsize` layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KernelWinsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl KernelWinsize {
    /// Default 80x24 terminal (standard VT100 size).
    pub const fn default_console() -> Self {
        Self {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

/// Combined terminal state for a single console device.
pub struct TerminalState {
    /// Terminal attributes (termios).
    pub termios: KernelTermios,
    /// Window size.
    pub winsize: KernelWinsize,
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalState {
    /// Create a new terminal state with POSIX defaults.
    pub const fn new() -> Self {
        Self {
            termios: KernelTermios::default_console(),
            winsize: KernelWinsize::default_console(),
        }
    }
}

// =========================================================================
// Global terminal state (single console for now)
// =========================================================================

/// Global terminal state for /dev/console.
///
/// Protected by a spin::Mutex since it is accessed from syscall context.
/// Future: per-device terminal state when multiple TTYs are supported.
static CONSOLE_TERMINAL: OnceLock<Mutex<TerminalState>> = OnceLock::new();

/// Initialize the global console terminal state.
///
/// Called during boot (before any user-space programs run).
pub fn init() {
    let _ = CONSOLE_TERMINAL.set(Mutex::new(TerminalState::new()));
}

/// Get the global console terminal state.
///
/// Returns `None` if the terminal subsystem has not been initialized.
pub fn get_console_terminal() -> Option<&'static Mutex<TerminalState>> {
    CONSOLE_TERMINAL.get()
}

// =========================================================================
// Query helpers for the console read path
// =========================================================================

/// Check if the console is in canonical (line-buffered) mode.
///
/// Returns `true` if ICANON is set (the default). Returns `true` if the
/// terminal subsystem has not been initialized (conservative default).
pub fn is_canonical_mode() -> bool {
    match get_console_terminal() {
        Some(term) => term.lock().termios.is_canonical(),
        None => true, // Default to canonical before init
    }
}

/// Check if echo is enabled on the console.
///
/// Returns `true` if ECHO is set (the default). Returns `true` if the
/// terminal subsystem has not been initialized.
pub fn is_echo_enabled() -> bool {
    match get_console_terminal() {
        Some(term) => term.lock().termios.is_echo(),
        None => true,
    }
}

/// Get the VMIN value from the console terminal state.
///
/// VMIN specifies the minimum number of characters for a non-canonical
/// read to return. Default is 1.
pub fn get_vmin() -> u8 {
    match get_console_terminal() {
        Some(term) => term.lock().termios.vmin(),
        None => 1,
    }
}

/// Get the VTIME value from the console terminal state.
///
/// VTIME specifies the timeout in tenths of a second for non-canonical
/// read. Default is 0 (no timeout, block until VMIN characters).
pub fn get_vtime() -> u8 {
    match get_console_terminal() {
        Some(term) => term.lock().termios.vtime(),
        None => 0,
    }
}

/// Get the erase character from the console terminal state.
pub fn get_verase() -> u8 {
    match get_console_terminal() {
        Some(term) => term.lock().termios.verase(),
        None => 127, // DEL
    }
}

/// Get a snapshot of the current termios (for the ioctl handler).
pub fn get_termios_snapshot() -> KernelTermios {
    match get_console_terminal() {
        Some(term) => term.lock().termios,
        None => KernelTermios::default_console(),
    }
}

/// Get a snapshot of the current winsize (for the ioctl handler).
pub fn get_winsize_snapshot() -> KernelWinsize {
    match get_console_terminal() {
        Some(term) => term.lock().winsize,
        None => KernelWinsize::default_console(),
    }
}

/// Set the terminal attributes (from TCSETS/TCSETSW/TCSETSF ioctl).
pub fn set_termios(new_termios: &KernelTermios) {
    if let Some(term) = get_console_terminal() {
        term.lock().termios = *new_termios;
    }
}

/// Set the window size (from TIOCSWINSZ ioctl).
pub fn set_winsize(new_ws: &KernelWinsize) {
    if let Some(term) = get_console_terminal() {
        term.lock().winsize = *new_ws;
    }
}
