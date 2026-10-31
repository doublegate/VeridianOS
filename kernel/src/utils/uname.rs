//! What the system reports about itself (ADR 0021): uname(2), the kernel
//! shell's `uname`, `/proc/version` and `/proc/sys/kernel/{ostype,osrelease,
//! version}` all take their strings from here.

/// The system name VeridianOS programs see.
pub const SYSNAME: &str = "VeridianOS";
/// The system name a process in Linux uname mode sees: build scripts and
/// runtimes that test for Linux (autoconf's `uname -s`, Python's
/// `platform.system()`) need it.
pub const LINUX_SYSNAME: &str = "Linux";

/// The system name for a process in Linux uname mode (`linux`) or not.
pub const fn sysname(linux: bool) -> &'static str {
    if linux {
        LINUX_SYSNAME
    } else {
        SYSNAME
    }
}

/// A process's Linux uname mode (ADR 0021). A mode the process chose, with
/// prctl(PR_VERIDIAN_UNAME) or the `linux-uname` launcher, holds across fork
/// and exec, so everything it runs sees Linux. One that the program image
/// asked for (a GNU ABI tag naming Linux, as glibc programs carry, or the
/// VeridianOS feature note) holds for that image and its forks only: each
/// exec decides again from the program it loads, so a glibc shell does not
/// carry the mode into everything started from it.
pub struct UnameMode(core::sync::atomic::AtomicU8);

impl UnameMode {
    /// uname reports Linux.
    const LINUX: u8 = 1;
    /// The process chose the mode, so exec keeps it.
    const CHOSEN: u8 = 2;

    /// Native: uname reports VeridianOS.
    pub const fn new() -> Self {
        Self(core::sync::atomic::AtomicU8::new(0))
    }

    /// Whether uname reports Linux.
    pub fn is_linux(&self) -> bool {
        self.0.load(core::sync::atomic::Ordering::Acquire) & Self::LINUX != 0
    }

    /// The process's own choice (prctl): Linux and kept across exec, or
    /// native.
    pub fn set(&self, linux: bool) {
        let v = if linux { Self::LINUX | Self::CHOSEN } else { 0 };
        self.0.store(v, core::sync::atomic::Ordering::Release);
    }

    /// Exec of an image that does (`image_wants`) or does not ask for Linux
    /// mode.
    pub fn exec(&self, image_wants: bool) {
        let chosen = self.0.load(core::sync::atomic::Ordering::Acquire) & Self::CHOSEN;
        let v = if chosen != 0 {
            Self::LINUX | Self::CHOSEN
        } else if image_wants {
            Self::LINUX
        } else {
            0
        };
        self.0.store(v, core::sync::atomic::Ordering::Release);
    }

    /// Fork: the child runs the same image, so it has the parent's mode and
    /// whether it was chosen.
    pub fn inherit(&self, parent: &Self) {
        self.0.store(
            parent.0.load(core::sync::atomic::Ordering::Acquire),
            core::sync::atomic::Ordering::Release,
        );
    }
}

impl Default for UnameMode {
    fn default() -> Self {
        Self::new()
    }
}

/// The system name the calling process sees; [`SYSNAME`] outside any
/// process.
pub fn current_sysname() -> &'static str {
    sysname(crate::process::current_process().is_some_and(|p| p.uname_linux.is_linux()))
}
/// The host name (until sethostname and UTS namespaces exist).
pub const NODENAME: &str = "veridian";
/// The release: the Linux version whose sources are the kernel's
/// specification, so programs that compare kernel versions (LTP among them)
/// see a Linux they recognise.
pub const RELEASE: &str = "7.3.0-veridian";
/// The version field, which carries VeridianOS's own version.
pub const VERSION: &str = concat!("#1 SMP VeridianOS ", env!("CARGO_PKG_VERSION"));

/// The machine (hardware) name.
pub const fn machine() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "riscv64"
    }
}

/// `/proc/version`, in Linux's layout (`<sysname> version <release>
/// (<builder>) <version>`).
#[cfg(feature = "alloc")]
pub fn proc_version(sysname: &str) -> alloc::string::String {
    alloc::format!(
        "{} version {} (veridian@veridian) {}\n",
        sysname,
        RELEASE,
        VERSION
    )
}

/// What the kernel shell's `uname [-asnrvm]` prints: the selected fields in
/// uname(1)'s order (system, node, release, version, machine), the system
/// name alone without options. Options combine (`-sr`) and `--all` is
/// `-a`; anything else is uname(1)'s error. The shell is not a Linux
/// program, so it always names VeridianOS.
#[cfg(feature = "alloc")]
pub fn command_output(args: &[&str]) -> alloc::string::String {
    // Fields in output order: s, n, r, v, m.
    let mut on = [false; 5];
    for arg in args {
        let letters = match *arg {
            "--all" => "a",
            a if a.starts_with("--") => {
                return alloc::format!("uname: unrecognized option '{}'", a)
            }
            a if a.starts_with('-') && a.len() > 1 => &a[1..],
            a => return alloc::format!("uname: extra operand '{}'", a),
        };
        for c in letters.chars() {
            match c {
                'a' => on = [true; 5],
                's' => on[0] = true,
                'n' => on[1] = true,
                'r' => on[2] = true,
                'v' => on[3] = true,
                'm' => on[4] = true,
                c => return alloc::format!("uname: invalid option -- '{}'", c),
            }
        }
    }
    if on == [false; 5] {
        on[0] = true;
    }
    let fields = [
        (on[0], SYSNAME),
        (on[1], NODENAME),
        (on[2], RELEASE),
        (on[3], VERSION),
        (on[4], machine()),
    ];
    let words: alloc::vec::Vec<&str> = fields
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, v)| *v)
        .collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0021: a mode the process chose (prctl, the launcher) holds across
    /// fork and every exec; one an image asked for (a GNU ABI tag naming
    /// Linux, the feature note) holds only for that image, so the next exec
    /// decides again from the program it loads.
    #[test]
    fn only_a_chosen_mode_outlives_exec() {
        let m = UnameMode::new();
        assert!(!m.is_linux());
        m.exec(true);
        assert!(m.is_linux());
        let child = UnameMode::new();
        child.inherit(&m);
        assert!(child.is_linux(), "fork keeps the image's mode");
        child.exec(false);
        assert!(!child.is_linux(), "a plain exec after an image's mode");
        m.set(true);
        m.exec(false);
        assert!(m.is_linux(), "a chosen mode survives a plain exec");
        child.inherit(&m);
        child.exec(false);
        child.exec(false);
        assert!(child.is_linux(), "and is inherited as chosen");
        m.set(false);
        assert!(!m.is_linux());
        m.exec(true);
        assert!(m.is_linux(), "turning it off leaves images free to ask");
        m.exec(false);
        assert!(!m.is_linux());
    }

    #[test]
    fn the_release_is_a_linux_version_newer_than_ltp_requires() {
        // LTP's tst_kvercmp parses the leading numbers; it needs at least 4.4.
        let mut parts = RELEASE.split(['.', '-']);
        let major: u32 = parts.next().unwrap().parse().unwrap();
        let minor: u32 = parts.next().unwrap().parse().unwrap();
        assert!((major, minor) >= (4, 4));
        assert!(RELEASE.ends_with("-veridian"));
    }

    #[test]
    fn the_version_field_carries_the_veridian_version() {
        assert!(VERSION.starts_with("#1 SMP VeridianOS "));
        assert!(VERSION.ends_with(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn every_field_fits_utsname() {
        // utsname fields are 65 bytes including the NUL.
        for s in [
            SYSNAME,
            LINUX_SYSNAME,
            NODENAME,
            RELEASE,
            VERSION,
            machine(),
        ] {
            assert!(s.len() < 65, "{s}");
        }
    }

    #[test]
    fn shell_uname_prints_fields_in_uname_order() {
        assert_eq!(command_output(&[]), "VeridianOS");
        assert_eq!(command_output(&["-r"]), "7.3.0-veridian");
        assert_eq!(
            command_output(&["-m", "-s"]),
            alloc::format!("VeridianOS {}", machine())
        );
        let all = command_output(&["-a"]);
        assert!(all.starts_with("VeridianOS veridian 7.3.0-veridian #1 SMP VeridianOS "));
        assert!(all.ends_with(machine()));
        assert_eq!(command_output(&["-sr"]), "VeridianOS 7.3.0-veridian");
        assert_eq!(command_output(&["--all"]), all);
        assert_eq!(command_output(&["-x"]), "uname: invalid option -- 'x'");
        assert_eq!(command_output(&["-sx"]), "uname: invalid option -- 'x'");
        assert_eq!(command_output(&["foo"]), "uname: extra operand 'foo'");
        assert_eq!(command_output(&["--x"]), "uname: unrecognized option '--x'");
    }

    #[test]
    fn linux_mode_changes_only_the_system_name() {
        assert_eq!(sysname(false), "VeridianOS");
        assert_eq!(sysname(true), "Linux");
        // Outside a process (the kernel, host tests): VeridianOS.
        assert_eq!(current_sysname(), "VeridianOS");
        assert!(proc_version(sysname(true)).starts_with("Linux version 7.3.0-veridian ("));
    }

    #[test]
    fn proc_version_follows_linux_layout() {
        let v = proc_version(SYSNAME);
        assert!(v.starts_with("VeridianOS version 7.3.0-veridian ("));
        assert!(v.ends_with(concat!("VeridianOS ", env!("CARGO_PKG_VERSION"), "\n")));
    }
}
