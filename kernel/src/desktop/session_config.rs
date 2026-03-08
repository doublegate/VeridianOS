//! Session configuration reader
//!
//! Reads `/etc/veridian/session.conf` to determine the preferred desktop
//! session type (KDE Plasma 6 or built-in DE). Falls back to Plasma if
//! the config file is missing or unreadable.

#[cfg(feature = "alloc")]
extern crate alloc;

/// Preferred session type read from config or CLI argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPreference {
    /// KDE Plasma 6 desktop session.
    Plasma,
    /// Built-in kernel-space desktop environment.
    Builtin,
}

/// Default session config path.
const SESSION_CONF_PATH: &str = "/etc/veridian/session.conf";

/// Path to KDE init script (existence check for KDE availability).
const KDE_INIT_SCRIPT: &str = "/usr/share/veridian/veridian-kde-init.sh";

/// Read the session preference from `/etc/veridian/session.conf`.
///
/// The config file format is simple key=value lines. We look for
/// `session_type=plasma` or `session_type=builtin`. Missing file or
/// unrecognized value defaults to `Plasma`.
#[cfg(feature = "alloc")]
pub fn read_session_preference() -> SessionPreference {
    match crate::fs::read_file(SESSION_CONF_PATH) {
        Ok(data) => parse_session_config(&data),
        Err(_) => {
            // No config file -- default to Plasma
            SessionPreference::Plasma
        }
    }
}

/// Parse session config bytes into a preference.
#[cfg(feature = "alloc")]
fn parse_session_config(data: &[u8]) -> SessionPreference {
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return SessionPreference::Plasma,
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("session_type=") {
            return match value.trim() {
                "plasma" | "kde" => SessionPreference::Plasma,
                "builtin" | "default" => SessionPreference::Builtin,
                _ => SessionPreference::Plasma,
            };
        }
    }

    // No session_type line found -- default to Plasma
    SessionPreference::Plasma
}

/// Check whether KDE binaries are available on the filesystem.
///
/// Returns `true` if the KDE init script exists at the expected path.
#[cfg(feature = "alloc")]
pub fn kde_binaries_available() -> bool {
    crate::fs::read_file(KDE_INIT_SCRIPT).is_ok()
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plasma() {
        let data = b"session_type=plasma\n";
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }

    #[test]
    fn test_parse_kde_alias() {
        let data = b"session_type=kde\n";
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }

    #[test]
    fn test_parse_builtin() {
        let data = b"session_type=builtin\n";
        assert_eq!(parse_session_config(data), SessionPreference::Builtin);
    }

    #[test]
    fn test_parse_default_alias() {
        let data = b"session_type=default\n";
        assert_eq!(parse_session_config(data), SessionPreference::Builtin);
    }

    #[test]
    fn test_parse_empty_file() {
        let data = b"";
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }

    #[test]
    fn test_parse_comments_only() {
        let data = b"# This is a comment\n# Another comment\n";
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }

    #[test]
    fn test_parse_unknown_value() {
        let data = b"session_type=gnome\n";
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }

    #[test]
    fn test_parse_with_comments_and_whitespace() {
        let data = b"# Session configuration\n\nsession_type=builtin\n";
        assert_eq!(parse_session_config(data), SessionPreference::Builtin);
    }

    #[test]
    fn test_parse_invalid_utf8() {
        let data = &[0xFF, 0xFE, 0x00];
        assert_eq!(parse_session_config(data), SessionPreference::Plasma);
    }
}
