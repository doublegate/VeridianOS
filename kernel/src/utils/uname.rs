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

/// The system name the calling process sees; [`SYSNAME`] outside any
/// process.
pub fn current_sysname() -> &'static str {
    sysname(
        crate::process::current_process()
            .is_some_and(|p| p.uname_linux.load(core::sync::atomic::Ordering::Acquire)),
    )
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
/// name alone without options. The shell is not a Linux program, so it
/// always names VeridianOS.
#[cfg(feature = "alloc")]
pub fn command_output(args: &[&str]) -> alloc::string::String {
    let all = args.contains(&"-a");
    let want = |flag: &str| all || args.contains(&flag);
    let fields = [
        (args.is_empty() || want("-s"), SYSNAME),
        (want("-n"), NODENAME),
        (want("-r"), RELEASE),
        (want("-v"), VERSION),
        (want("-m"), machine()),
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
