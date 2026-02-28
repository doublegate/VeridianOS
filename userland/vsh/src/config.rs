//! Shell configuration and option management.
//!
//! Manages shell options (set -e, set -x, etc.) and the `shopt` builtin
//! options that control extended shell behavior.

/// Shell options controlled by the `set` builtin.
#[derive(Debug, Clone)]
pub struct SetOptions {
    /// -e: Exit immediately if a command exits with non-zero status.
    pub errexit: bool,
    /// -u: Treat unset variables as an error during expansion.
    pub nounset: bool,
    /// -x: Print commands and their arguments as they are executed.
    pub xtrace: bool,
    /// -v: Print shell input lines as they are read.
    pub verbose: bool,
    /// -f: Disable pathname expansion (globbing).
    pub noglob: bool,
    /// -n: Read commands but do not execute them (syntax check only).
    pub noexec: bool,
    /// -C: Prevent output redirection from overwriting existing files.
    pub noclobber: bool,
    /// -a: Mark variables for export when they are set or modified.
    pub allexport: bool,
    /// -b: Report terminated background jobs immediately.
    pub notify: bool,
    /// -h: Remember the location of commands as they are looked up (hash).
    pub hashall: bool,
    /// -m: Enable job control.
    pub monitor: bool,
    /// -H: Enable ! style history substitution.
    pub histexpand: bool,
    /// -B: Enable brace expansion.
    pub braceexpand: bool,
    /// -p: Pipefail: return value of a pipeline is the status of the last
    ///     command to exit with non-zero status, or zero if all succeeded.
    pub pipefail: bool,
}

impl Default for SetOptions {
    fn default() -> Self {
        Self {
            errexit: false,
            nounset: false,
            xtrace: false,
            verbose: false,
            noglob: false,
            noexec: false,
            noclobber: false,
            allexport: false,
            notify: false,
            hashall: true,    // On by default in bash
            monitor: false,   // Will be set true for interactive shells
            histexpand: true, // On by default for interactive shells
            braceexpand: true,
            pipefail: false,
        }
    }
}

impl SetOptions {
    /// Apply a single-letter option flag. `enable` = true means set (turn on).
    /// Returns true if the option was recognized.
    pub fn apply(&mut self, flag: char, enable: bool) -> bool {
        match flag {
            'e' => {
                self.errexit = enable;
                true
            }
            'u' => {
                self.nounset = enable;
                true
            }
            'x' => {
                self.xtrace = enable;
                true
            }
            'v' => {
                self.verbose = enable;
                true
            }
            'f' => {
                self.noglob = enable;
                true
            }
            'n' => {
                self.noexec = enable;
                true
            }
            'C' => {
                self.noclobber = enable;
                true
            }
            'a' => {
                self.allexport = enable;
                true
            }
            'b' => {
                self.notify = enable;
                true
            }
            'h' => {
                self.hashall = enable;
                true
            }
            'm' => {
                self.monitor = enable;
                true
            }
            'H' => {
                self.histexpand = enable;
                true
            }
            'B' => {
                self.braceexpand = enable;
                true
            }
            _ => false,
        }
    }

    /// Get the current flags as a string (for `$-`).
    pub fn flags_string(&self) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        if self.errexit {
            s.push('e');
        }
        if self.nounset {
            s.push('u');
        }
        if self.xtrace {
            s.push('x');
        }
        if self.verbose {
            s.push('v');
        }
        if self.noglob {
            s.push('f');
        }
        if self.noexec {
            s.push('n');
        }
        if self.noclobber {
            s.push('C');
        }
        if self.allexport {
            s.push('a');
        }
        if self.notify {
            s.push('b');
        }
        if self.hashall {
            s.push('h');
        }
        if self.monitor {
            s.push('m');
        }
        if self.histexpand {
            s.push('H');
        }
        if self.braceexpand {
            s.push('B');
        }
        s
    }
}

/// Extended shell options controlled by the `shopt` builtin.
#[derive(Debug, Clone)]
pub struct ShoptOptions {
    /// extglob: Enable extended pattern matching operators.
    pub extglob: bool,
    /// globstar: `**` matches zero or more directories.
    pub globstar: bool,
    /// dotglob: Include filenames beginning with `.` in glob results.
    pub dotglob: bool,
    /// nullglob: Allow patterns that match no files to expand to empty.
    pub nullglob: bool,
    /// failglob: Patterns that fail to match produce an error.
    pub failglob: bool,
    /// nocaseglob: Case-insensitive globbing.
    pub nocaseglob: bool,
    /// nocasematch: Case-insensitive pattern matching in `case` and `[[`.
    pub nocasematch: bool,
    /// expand_aliases: Expand aliases.
    pub expand_aliases: bool,
    /// sourcepath: Search PATH for the argument to `source`.
    pub sourcepath: bool,
    /// lastpipe: Run the last command of a pipeline in the current shell.
    pub lastpipe: bool,
    /// checkwinsize: Check window size after each command.
    pub checkwinsize: bool,
    /// cmdhist: Save multi-line commands as single history entries.
    pub cmdhist: bool,
    /// lithist: Save multi-line commands with embedded newlines.
    pub lithist: bool,
    /// histappend: Append to history file on exit.
    pub histappend: bool,
    /// autocd: Treat a directory name as a cd command.
    pub autocd: bool,
    /// cdspell: Correct minor spelling errors in cd arguments.
    pub cdspell: bool,
}

impl Default for ShoptOptions {
    fn default() -> Self {
        Self {
            extglob: false,
            globstar: false,
            dotglob: false,
            nullglob: false,
            failglob: false,
            nocaseglob: false,
            nocasematch: false,
            expand_aliases: true,
            sourcepath: true,
            lastpipe: false,
            checkwinsize: true,
            cmdhist: true,
            lithist: false,
            histappend: false,
            autocd: false,
            cdspell: false,
        }
    }
}

/// Top-level shell configuration combining set options, shopt options,
/// and interactive state.
#[derive(Debug, Clone, Default)]
pub struct ShellConfig {
    pub set_opts: SetOptions,
    pub shopt_opts: ShoptOptions,
    /// Whether this shell is interactive (has a controlling terminal).
    pub interactive: bool,
    /// Whether this shell is a login shell.
    pub login: bool,
    /// Whether this shell is running a script (not interactive).
    pub script_mode: bool,
}

impl ShellConfig {
    /// Create a configuration for an interactive shell.
    pub fn interactive() -> Self {
        Self {
            interactive: true,
            set_opts: SetOptions {
                monitor: true,
                histexpand: true,
                ..SetOptions::default()
            },
            shopt_opts: ShoptOptions {
                expand_aliases: true,
                checkwinsize: true,
                ..ShoptOptions::default()
            },
            ..Self::default()
        }
    }

    /// Create a configuration for a non-interactive (script) shell.
    pub fn script() -> Self {
        Self {
            script_mode: true,
            ..Self::default()
        }
    }
}
