//! Special shell variables.
//!
//! Provides access to `BASH_VERSION`, `BASH_VERSINFO`, `RANDOM`, `LINENO`,
//! `SECONDS`, `FUNCNAME`, `BASH_SOURCE`, `BASH_LINENO`, `PIPESTATUS`,
//! `COMP_WORDS`, and other built-in variables.

use alloc::string::String;
use alloc::format;

/// Get the value of a special/built-in variable by name.
///
/// Returns `None` if the name is not a recognized special variable.
pub fn get_special_var(name: &str) -> Option<String> {
    match name {
        "BASH" => Some(String::from("/bin/vsh")),
        "BASH_VERSION" | "VSH_VERSION" => Some(String::from("0.1.0")),
        "BASH_VERSINFO" => Some(String::from("0")),
        "HOSTNAME" => Some(String::from("veridian")),
        "HOSTTYPE" => Some(String::from("x86_64")),
        "MACHTYPE" => Some(String::from("x86_64-unknown-veridian")),
        "OSTYPE" => Some(String::from("veridian")),
        "SHELL" => Some(String::from("/bin/vsh")),
        "TERM" => Some(String::from("vt100")),
        "UID" => {
            let uid = crate::syscall::sys_getpid(); // placeholder
            Some(format!("{}", uid))
        }
        "EUID" => Some(String::from("0")),
        _ => None,
    }
}
