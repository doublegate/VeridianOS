//! Source/dot command implementation.
//!
//! `source filename` / `. filename` reads and executes commands from the
//! given file in the current shell environment (not a subshell).

extern crate alloc;

use alloc::string::String;

use crate::{error::Result, Shell};

/// Source a file: read and execute it in the current shell.
///
/// Unlike `run_script_file`, this does not modify the shell's interactive
/// state or positional parameters.
pub fn source_file(shell: &mut Shell, path: &str) -> Result<i32> {
    // If sourcepath is enabled and path doesn't contain '/', search PATH
    let resolved = if !path.contains('/') && shell.config.shopt_opts.sourcepath {
        match crate::builtin::find_in_path(path, shell.env.get_str("PATH")) {
            Some(p) => p,
            None => String::from(path),
        }
    } else {
        String::from(path)
    };

    super::script::run_script_file(shell, &resolved)
}
