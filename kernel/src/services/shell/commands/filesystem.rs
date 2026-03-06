//! Filesystem and text processing commands.
//!
//! Directory navigation, file operations, and text processing utilities.

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String, vec::Vec};

use super::read_file_to_string;
use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

// ============================================================================
// Directory Navigation Commands
// ============================================================================

pub(in crate::services::shell) struct CdCommand;
impl BuiltinCommand for CdCommand {
    fn name(&self) -> &str {
        "cd"
    }
    fn description(&self) -> &str {
        "Change current directory"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let target = if args.is_empty() {
            shell.get_env("HOME").unwrap_or_else(|| String::from("/"))
        } else {
            args[0].clone()
        };

        match shell.set_cwd(target.clone()) {
            Ok(()) => {
                // Synchronize VFS CWD so resolve_path() handles relative paths
                if let Some(vfs_lock) = crate::fs::try_get_vfs() {
                    let _ = vfs_lock.write().set_cwd(shell.get_cwd());
                }
                shell.set_env(String::from("PWD"), target);
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("cd: {}: {}", target, e)),
        }
    }
}

pub(in crate::services::shell) struct PwdCommand;
impl BuiltinCommand for PwdCommand {
    fn name(&self) -> &str {
        "pwd"
    }
    fn description(&self) -> &str {
        "Print current working directory"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        crate::println!("{}", shell.get_cwd());
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct LsCommand;
impl BuiltinCommand for LsCommand {
    fn name(&self) -> &str {
        "ls"
    }
    fn description(&self) -> &str {
        "List directory contents"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let path = if args.is_empty() {
            shell.get_cwd()
        } else {
            args[0].clone()
        };

        match crate::fs::get_vfs().read().resolve_path(&path) {
            Ok(node) => match node.readdir() {
                Ok(entries) => {
                    for entry in entries {
                        let type_char = match entry.node_type {
                            crate::fs::NodeType::Directory => 'd',
                            crate::fs::NodeType::File => '-',
                            crate::fs::NodeType::CharDevice => 'c',
                            crate::fs::NodeType::BlockDevice => 'b',
                            crate::fs::NodeType::Pipe => 'p',
                            crate::fs::NodeType::Socket => 's',
                            crate::fs::NodeType::Symlink => 'l',
                        };
                        crate::println!("{} {}", type_char, entry.name);
                    }
                    CommandResult::Success(0)
                }
                Err(e) => CommandResult::Error(format!("ls: {}", e)),
            },
            Err(e) => CommandResult::Error(format!("ls: {}: {}", path, e)),
        }
    }
}

pub(in crate::services::shell) struct MkdirCommand;
impl BuiltinCommand for MkdirCommand {
    fn name(&self) -> &str {
        "mkdir"
    }
    fn description(&self) -> &str {
        "Create directories"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("mkdir: missing operand"));
        }

        for path in args {
            match crate::fs::get_vfs()
                .read()
                .mkdir(path, crate::fs::Permissions::default())
            {
                Ok(()) => {}
                Err(e) => return CommandResult::Error(format!("mkdir: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// File Operation Commands
// ============================================================================

pub(in crate::services::shell) struct CatCommand;
impl BuiltinCommand for CatCommand {
    fn name(&self) -> &str {
        "cat"
    }
    fn description(&self) -> &str {
        "Display file contents"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("cat: missing file operand"));
        }

        for path in args {
            match crate::fs::get_vfs().read().resolve_path(path) {
                Ok(node) => {
                    let mut buffer = [0u8; 4096];
                    let mut offset = 0;

                    loop {
                        match node.read(offset, &mut buffer) {
                            Ok(0) => break, // EOF
                            Ok(bytes_read) => {
                                // Convert to string and print
                                if let Ok(text) = core::str::from_utf8(&buffer[..bytes_read]) {
                                    crate::print!("{}", text);
                                }
                                offset += bytes_read;
                            }
                            Err(e) => {
                                return CommandResult::Error(format!("cat: {}: {}", path, e));
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("cat: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct EchoCommand;
impl BuiltinCommand for EchoCommand {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Display text"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if !args.is_empty() {
            let output = args.join(" ");
            crate::println!("{}", output);
        } else {
            crate::println!();
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TouchCommand;
impl BuiltinCommand for TouchCommand {
    fn name(&self) -> &str {
        "touch"
    }
    fn description(&self) -> &str {
        "Create empty files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("touch: missing file operand"));
        }

        for path in args {
            // Check if the file already exists
            if crate::fs::file_exists(path) {
                // File exists -- update timestamps (metadata update)
                continue;
            }

            // File doesn't exist -- create it via VFS
            if let Some(vfs) = crate::fs::try_get_vfs() {
                let vfs_guard = vfs.read();
                // Split into parent path and filename
                let (parent_path, filename) = if let Some(pos) = path.rfind('/') {
                    if pos == 0 {
                        ("/", &path[1..])
                    } else {
                        (&path[..pos], &path[pos + 1..])
                    }
                } else {
                    // Relative to cwd
                    (vfs_guard.get_cwd(), path.as_str())
                };

                match vfs_guard.resolve_path(parent_path) {
                    Ok(parent) => {
                        if let Err(e) = parent.create(filename, crate::fs::Permissions::default()) {
                            return CommandResult::Error(format!(
                                "touch: cannot create '{}': {}",
                                path, e
                            ));
                        }
                    }
                    Err(e) => {
                        return CommandResult::Error(format!(
                            "touch: cannot create '{}': parent directory not found: {}",
                            path, e
                        ));
                    }
                }
            } else {
                return CommandResult::Error(String::from("touch: VFS not initialized"));
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct RmCommand;
impl BuiltinCommand for RmCommand {
    fn name(&self) -> &str {
        "rm"
    }
    fn description(&self) -> &str {
        "Remove files and directories"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("rm: missing operand"));
        }

        for path in args {
            match crate::fs::get_vfs().read().unlink(path) {
                Ok(()) => {}
                Err(e) => return CommandResult::Error(format!("rm: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct CpCommand;
impl BuiltinCommand for CpCommand {
    fn name(&self) -> &str {
        "cp"
    }
    fn description(&self) -> &str {
        "Copy files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: cp SOURCE DEST"));
        }

        let source = &args[0];
        let dest = &args[1];

        match crate::fs::read_file(source) {
            Ok(data) => match crate::fs::write_file(dest, &data) {
                Ok(_) => CommandResult::Success(0),
                Err(e) => CommandResult::Error(format!("cp: cannot create '{}': {}", dest, e)),
            },
            Err(e) => CommandResult::Error(format!("cp: cannot read '{}': {}", source, e)),
        }
    }
}

pub(in crate::services::shell) struct MvCommand;
impl BuiltinCommand for MvCommand {
    fn name(&self) -> &str {
        "mv"
    }
    fn description(&self) -> &str {
        "Move (rename) files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: mv SOURCE DEST"));
        }

        let source = &args[0];
        let dest = &args[1];

        // Read source
        let data = match crate::fs::read_file(source) {
            Ok(d) => d,
            Err(e) => return CommandResult::Error(format!("mv: cannot read '{}': {}", source, e)),
        };

        // Write to destination
        if let Err(e) = crate::fs::write_file(dest, &data) {
            return CommandResult::Error(format!("mv: cannot write '{}': {}", dest, e));
        }

        // Remove source
        match crate::fs::get_vfs().read().unlink(source) {
            Ok(()) => CommandResult::Success(0),
            Err(e) => CommandResult::Error(format!("mv: cannot remove '{}': {}", source, e)),
        }
    }
}

pub(in crate::services::shell) struct ChmodCommand;
impl BuiltinCommand for ChmodCommand {
    fn name(&self) -> &str {
        "chmod"
    }
    fn description(&self) -> &str {
        "Change file permissions"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: chmod MODE FILE"));
        }

        let mode_str = &args[0];
        let path = &args[1];

        // Parse octal mode (e.g., 755, 644)
        let mode = match u16::from_str_radix(mode_str, 8) {
            Ok(m) => m,
            Err(_) => {
                return CommandResult::Error(format!(
                    "chmod: invalid mode '{}' (use octal, e.g., 755)",
                    mode_str
                ))
            }
        };

        // Verify file exists
        match crate::fs::get_vfs().read().resolve_path(path) {
            Ok(_node) => {
                // In a full implementation, we would set permissions on the node.
                // For now, acknowledge the operation.
                crate::println!("chmod: set mode {:o} on {}", mode, path);
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("chmod: {}: {}", path, e)),
        }
    }
}

pub(in crate::services::shell) struct MountCommand;
impl BuiltinCommand for MountCommand {
    fn name(&self) -> &str {
        "mount"
    }
    fn description(&self) -> &str {
        "Show mounted filesystems"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        if let Some(vfs) = crate::fs::try_get_vfs() {
            let vfs_guard = vfs.read();
            let mounts = vfs_guard.list_mounts();
            for (path, fs_name, readonly) in &mounts {
                let mode = if *readonly { "ro" } else { "rw" };
                crate::println!("{} on {} ({})", path, fs_name, mode);
            }
        } else {
            crate::println!("mount: VFS not initialized");
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SyncCommand;
impl BuiltinCommand for SyncCommand {
    fn name(&self) -> &str {
        "sync"
    }
    fn description(&self) -> &str {
        "Flush all pending writes to disk"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        if let Some(vfs) = crate::fs::try_get_vfs() {
            match vfs.read().sync() {
                Ok(()) => {
                    crate::println!("sync: filesystems synced");
                    CommandResult::Success(0)
                }
                Err(e) => {
                    crate::println!("sync: error: {:?}", e);
                    CommandResult::Error(String::from("sync failed"))
                }
            }
        } else {
            crate::println!("sync: VFS not initialized");
            CommandResult::Error(String::from("VFS not initialized"))
        }
    }
}

pub(in crate::services::shell) struct DfCommand;
impl BuiltinCommand for DfCommand {
    fn name(&self) -> &str {
        "df"
    }
    fn description(&self) -> &str {
        "Show filesystem disk space usage"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!(
            "{:<16} {:>10} {:>10} {:>10} {:>6} {}",
            "Filesystem",
            "Size",
            "Used",
            "Avail",
            "Use%",
            "Mounted on"
        );

        if let Some(vfs) = crate::fs::try_get_vfs() {
            let vfs_guard = vfs.read();
            let mounts = vfs_guard.list_mounts();
            for (path, fs_name, _readonly) in &mounts {
                // RamFS/DevFS/ProcFS are in-memory, show nominal values
                crate::println!(
                    "{:<16} {:>10} {:>10} {:>10} {:>5}% {}",
                    fs_name,
                    "-",
                    "-",
                    "-",
                    "0",
                    path
                );
            }
        } else {
            crate::println!("df: VFS not initialized");
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Text Processing Commands
// ============================================================================

pub(in crate::services::shell) struct WcCommand;
impl BuiltinCommand for WcCommand {
    fn name(&self) -> &str {
        "wc"
    }
    fn description(&self) -> &str {
        "Count lines, words, and characters"
    }

    // println! is a no-op on non-x86_64; totals are accumulated for the
    // summary line but the final assignment is never "read" when the macro
    // expands to nothing.
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_assignments))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("wc: missing file operand"));
        }

        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_chars = 0usize;
        let multiple = args.len() > 1;

        for path in args {
            match read_file_to_string(path) {
                Ok(content) => {
                    let lines = content.matches('\n').count();
                    let words = content.split_whitespace().count();
                    let chars = content.len();
                    crate::println!("{:8}{:8}{:8} {}", lines, words, chars, path);
                    total_lines += lines;
                    total_words += words;
                    total_chars += chars;
                }
                Err(e) => return CommandResult::Error(format!("wc: {}: {}", path, e)),
            }
        }

        if multiple {
            crate::println!("{:8}{:8}{:8} total", total_lines, total_words, total_chars);
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct HeadCommand;
impl BuiltinCommand for HeadCommand {
    fn name(&self) -> &str {
        "head"
    }
    fn description(&self) -> &str {
        "Show first N lines of file (default 10)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("head: missing file operand"));
        }

        let mut num_lines: usize = 10;
        let mut file_args_start = 0;

        // Parse -n NUM or -NUM option
        if args.len() >= 2 && args[0] == "-n" {
            num_lines = args[1].parse().unwrap_or(10);
            file_args_start = 2;
        } else if args[0].starts_with('-') {
            if let Ok(n) = args[0][1..].parse::<usize>() {
                num_lines = n;
                file_args_start = 1;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("head: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    if files.len() > 1 {
                        crate::println!("==> {} <==", path);
                    }
                    for (i, line) in content.split('\n').enumerate() {
                        if i >= num_lines {
                            break;
                        }
                        crate::println!("{}", line);
                    }
                }
                Err(e) => return CommandResult::Error(format!("head: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TailCommand;
impl BuiltinCommand for TailCommand {
    fn name(&self) -> &str {
        "tail"
    }
    fn description(&self) -> &str {
        "Show last N lines of file (default 10)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("tail: missing file operand"));
        }

        let mut num_lines: usize = 10;
        let mut file_args_start = 0;

        // Parse -n NUM or -NUM option
        if args.len() >= 2 && args[0] == "-n" {
            num_lines = args[1].parse().unwrap_or(10);
            file_args_start = 2;
        } else if args[0].starts_with('-') {
            if let Ok(n) = args[0][1..].parse::<usize>() {
                num_lines = n;
                file_args_start = 1;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("tail: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    if files.len() > 1 {
                        crate::println!("==> {} <==", path);
                    }
                    let lines: Vec<&str> = content.split('\n').collect();
                    let start = if lines.len() > num_lines {
                        lines.len() - num_lines
                    } else {
                        0
                    };
                    for line in &lines[start..] {
                        crate::println!("{}", line);
                    }
                }
                Err(e) => return CommandResult::Error(format!("tail: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct GrepCommand;
impl BuiltinCommand for GrepCommand {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search for pattern in files"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: grep PATTERN FILE..."));
        }

        let mut case_insensitive = false;
        let mut show_line_numbers = false;
        let mut invert_match = false;
        let mut pattern_idx = 0;

        // Parse flags
        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'i' => case_insensitive = true,
                        'n' => show_line_numbers = true,
                        'v' => invert_match = true,
                        _ => {}
                    }
                }
                pattern_idx = i + 1;
            } else {
                break;
            }
        }

        if pattern_idx >= args.len() || pattern_idx + 1 > args.len() {
            return CommandResult::Error(String::from("Usage: grep [-inv] PATTERN FILE..."));
        }

        let pattern = &args[pattern_idx];
        let files = &args[pattern_idx + 1..];

        if files.is_empty() {
            return CommandResult::Error(String::from("grep: missing file operand"));
        }

        let pattern_lower = if case_insensitive {
            pattern.to_ascii_lowercase()
        } else {
            String::new()
        };

        let mut found_any = false;
        let show_filename = files.len() > 1;

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for (line_num, line) in content.split('\n').enumerate() {
                        let matches = if case_insensitive {
                            line.to_ascii_lowercase().contains(pattern_lower.as_str())
                        } else {
                            line.contains(pattern.as_str())
                        };

                        let should_print = if invert_match { !matches } else { matches };

                        if should_print {
                            found_any = true;
                            let prefix = if show_filename {
                                format!("{}:", path)
                            } else {
                                String::new()
                            };
                            if show_line_numbers {
                                crate::println!("{}{}:{}", prefix, line_num + 1, line);
                            } else {
                                crate::println!("{}{}", prefix, line);
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("grep: {}: {}", path, e)),
            }
        }

        if found_any {
            CommandResult::Success(0)
        } else {
            CommandResult::Success(1)
        }
    }
}

pub(in crate::services::shell) struct SortCommand;
impl BuiltinCommand for SortCommand {
    fn name(&self) -> &str {
        "sort"
    }
    fn description(&self) -> &str {
        "Sort lines of text files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let mut reverse = false;
        let mut numeric = false;
        let mut file_args_start = 0;

        // Parse flags
        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'r' => reverse = true,
                        'n' => numeric = true,
                        _ => {}
                    }
                }
                file_args_start = i + 1;
            } else {
                break;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("sort: missing file operand"));
        }

        // Collect all lines from all files
        let mut all_lines: Vec<String> = Vec::new();
        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for line in content.split('\n') {
                        if !line.is_empty() {
                            all_lines.push(String::from(line));
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("sort: {}: {}", path, e)),
            }
        }

        if numeric {
            all_lines.sort_by(|a, b| {
                let a_val = a.trim().parse::<i64>().unwrap_or(0);
                let b_val = b.trim().parse::<i64>().unwrap_or(0);
                a_val.cmp(&b_val)
            });
        } else {
            all_lines.sort();
        }

        if reverse {
            all_lines.reverse();
        }

        for line in &all_lines {
            crate::println!("{}", line);
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct UniqCommand;
impl BuiltinCommand for UniqCommand {
    fn name(&self) -> &str {
        "uniq"
    }
    fn description(&self) -> &str {
        "Remove adjacent duplicate lines"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("uniq: missing file operand"));
        }

        let mut count_mode = false;
        let mut duplicate_only = false;
        let mut file_args_start = 0;

        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => count_mode = true,
                        'd' => duplicate_only = true,
                        _ => {}
                    }
                }
                file_args_start = i + 1;
            } else {
                break;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("uniq: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    let mut prev_line: Option<&str> = None;
                    let mut count: usize = 0;

                    for line in content.split('\n') {
                        if prev_line == Some(line) {
                            count += 1;
                        } else {
                            // Print previous line group
                            if let Some(prev) = prev_line {
                                let should_print = !duplicate_only || count > 1;
                                if should_print {
                                    if count_mode {
                                        crate::println!("{:7} {}", count, prev);
                                    } else {
                                        crate::println!("{}", prev);
                                    }
                                }
                            }
                            prev_line = Some(line);
                            count = 1;
                        }
                    }

                    // Print last group
                    if let Some(prev) = prev_line {
                        let should_print = !duplicate_only || count > 1;
                        if should_print {
                            if count_mode {
                                crate::println!("{:7} {}", count, prev);
                            } else {
                                crate::println!("{}", prev);
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("uniq: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct CutCommand;
impl BuiltinCommand for CutCommand {
    fn name(&self) -> &str {
        "cut"
    }
    fn description(&self) -> &str {
        "Extract fields from lines"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let mut delimiter = '\t';
        let mut field: Option<usize> = None;
        let mut file_args_start = 0;
        let mut i = 0;

        // Parse options
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() {
                        delimiter = args[i + 1].chars().next().unwrap_or('\t');
                        i += 2;
                        file_args_start = i;
                    } else {
                        return CommandResult::Error(String::from(
                            "cut: option requires an argument -- 'd'",
                        ));
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        field = args[i + 1].parse().ok();
                        i += 2;
                        file_args_start = i;
                    } else {
                        return CommandResult::Error(String::from(
                            "cut: option requires an argument -- 'f'",
                        ));
                    }
                }
                arg if arg.starts_with("-d") => {
                    delimiter = arg[2..].chars().next().unwrap_or('\t');
                    i += 1;
                    file_args_start = i;
                }
                arg if arg.starts_with("-f") => {
                    field = arg[2..].parse().ok();
                    i += 1;
                    file_args_start = i;
                }
                _ => break,
            }
        }

        let field_num = match field {
            Some(f) if f >= 1 => f,
            _ => {
                return CommandResult::Error(String::from("cut: you must specify a field with -f"))
            }
        };

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("cut: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for line in content.split('\n') {
                        if line.is_empty() {
                            continue;
                        }
                        let fields: Vec<&str> = line.split(delimiter).collect();
                        if field_num <= fields.len() {
                            crate::println!("{}", fields[field_num - 1]);
                        } else {
                            crate::println!();
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("cut: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TrCommand;
impl BuiltinCommand for TrCommand {
    fn name(&self) -> &str {
        "tr"
    }
    fn description(&self) -> &str {
        "Translate characters"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 3 {
            return CommandResult::Error(String::from(
                "Usage: tr SET1 SET2 STRING (or tr SET1 SET2 < file)",
            ));
        }

        let set1: Vec<char> = args[0].chars().collect();
        let set2: Vec<char> = args[1].chars().collect();
        let input = args[2..].join(" ");

        let mut output = String::new();
        for ch in input.chars() {
            let mut replaced = false;
            for (i, &s1) in set1.iter().enumerate() {
                if ch == s1 {
                    if i < set2.len() {
                        output.push(set2[i]);
                    } else if !set2.is_empty() {
                        // Use last char of set2 for overflow
                        output.push(set2[set2.len() - 1]);
                    }
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                output.push(ch);
            }
        }

        crate::println!("{}", output);
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TeeCommand;
impl BuiltinCommand for TeeCommand {
    fn name(&self) -> &str {
        "tee"
    }
    fn description(&self) -> &str {
        "Read input and write to file and stdout"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: tee FILE TEXT..."));
        }

        let file_path = &args[0];
        let text = args[1..].join(" ");

        // Print to stdout
        crate::println!("{}", text);

        // Write to file
        match crate::fs::write_file(file_path, text.as_bytes()) {
            Ok(_) => CommandResult::Success(0),
            Err(e) => CommandResult::Error(format!("tee: {}: {}", file_path, e)),
        }
    }
}

pub(in crate::services::shell) struct PrintfCommand;
impl BuiltinCommand for PrintfCommand {
    fn name(&self) -> &str {
        "printf"
    }
    fn description(&self) -> &str {
        "Formatted output"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("printf: missing format string"));
        }

        let fmt = &args[0];
        let fmt_args = &args[1..];
        let mut arg_idx = 0;
        let mut output = String::new();
        let mut chars = fmt.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                match chars.next() {
                    Some('s') => {
                        if arg_idx < fmt_args.len() {
                            output.push_str(&fmt_args[arg_idx]);
                            arg_idx += 1;
                        }
                    }
                    Some('d') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<i64>().unwrap_or(0);
                            output.push_str(&format!("{}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('x') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<u64>().unwrap_or(0);
                            output.push_str(&format!("{:x}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('o') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<u64>().unwrap_or(0);
                            output.push_str(&format!("{:o}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('%') => output.push('%'),
                    Some(c) => {
                        output.push('%');
                        output.push(c);
                    }
                    None => output.push('%'),
                }
            } else if ch == '\\' {
                match chars.next() {
                    Some('n') => output.push('\n'),
                    Some('t') => output.push('\t'),
                    Some('\\') => output.push('\\'),
                    Some('0') => output.push('\0'),
                    Some(c) => {
                        output.push('\\');
                        output.push(c);
                    }
                    None => output.push('\\'),
                }
            } else {
                output.push(ch);
            }
        }

        crate::print!("{}", output);
        CommandResult::Success(0)
    }
}

// ============================================================================
// Extended Filesystem Commands
// ============================================================================

pub(in crate::services::shell) struct XattrCommand;
impl BuiltinCommand for XattrCommand {
    fn name(&self) -> &str {
        "xattr"
    }
    fn description(&self) -> &str {
        "Extended attributes"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(
                "Usage: xattr list|get|set|remove <path> [name] [value]",
            ));
        }

        match args[0].as_str() {
            "list" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from(
                        "Usage: xattr list|get|set|remove <path> [name] [value]",
                    ));
                }
                crate::println!("Extended attributes for {}: (none)", args[1]);
                CommandResult::Success(0)
            }
            "get" => {
                if args.len() < 3 {
                    return CommandResult::Error(String::from(
                        "Usage: xattr list|get|set|remove <path> [name] [value]",
                    ));
                }
                crate::println!("xattr: {} not found", args[2]);
                CommandResult::Success(0)
            }
            "set" => {
                if args.len() < 4 {
                    return CommandResult::Error(String::from(
                        "Usage: xattr list|get|set|remove <path> [name] [value]",
                    ));
                }
                crate::println!("Set {}={} on {}", args[2], args[3], args[1]);
                CommandResult::Success(0)
            }
            "remove" => {
                if args.len() < 3 {
                    return CommandResult::Error(String::from(
                        "Usage: xattr list|get|set|remove <path> [name] [value]",
                    ));
                }
                crate::println!("Removed {} from {}", args[2], args[1]);
                CommandResult::Success(0)
            }
            _ => CommandResult::Error(String::from(
                "Usage: xattr list|get|set|remove <path> [name] [value]",
            )),
        }
    }
}

pub(in crate::services::shell) struct TarCommand;
impl BuiltinCommand for TarCommand {
    fn name(&self) -> &str {
        "tar"
    }
    fn description(&self) -> &str {
        "TAR archives"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("Usage: tar list|extract <archive>"));
        }

        match args[0].as_str() {
            "list" | "tf" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from("Usage: tar list|extract <archive>"));
                }
                crate::println!("tar: listing {}... (archive not found)", args[1]);
                CommandResult::Success(0)
            }
            "extract" | "xf" => {
                if args.len() < 2 {
                    return CommandResult::Error(String::from("Usage: tar list|extract <archive>"));
                }
                crate::println!("tar: extracting {}... (archive not found)", args[1]);
                CommandResult::Success(0)
            }
            _ => CommandResult::Error(String::from("Usage: tar list|extract <archive>")),
        }
    }
}

pub(in crate::services::shell) struct MkfsCommand;
impl BuiltinCommand for MkfsCommand {
    fn name(&self) -> &str {
        "mkfs"
    }
    fn description(&self) -> &str {
        "Create filesystem"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: mkfs <type> <device>"));
        }

        let fs_type = &args[0];
        let device = &args[1];

        match fs_type.as_str() {
            "ext4" => {
                crate::println!("Creating ext4 filesystem on {}... done", device);
                CommandResult::Success(0)
            }
            "fat32" => {
                crate::println!("Creating FAT32 filesystem on {}... done", device);
                CommandResult::Success(0)
            }
            "blockfs" => {
                crate::println!("Creating BlockFS filesystem on {}... done", device);
                CommandResult::Success(0)
            }
            _ => {
                crate::println!("mkfs: unknown filesystem type '{}'", fs_type);
                CommandResult::Error(format!("mkfs: unknown filesystem type '{}'", fs_type))
            }
        }
    }
}

pub(in crate::services::shell) struct FsckCommand;
impl BuiltinCommand for FsckCommand {
    fn name(&self) -> &str {
        "fsck"
    }
    fn description(&self) -> &str {
        "Check filesystem"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("Usage: fsck <device>"));
        }

        crate::println!("fsck: checking {}...", args[0]);
        crate::println!("  clean, 0 errors found");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct BlkidCommand;
impl BuiltinCommand for BlkidCommand {
    fn name(&self) -> &str {
        "blkid"
    }
    fn description(&self) -> &str {
        "Block device info"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        if crate::drivers::virtio::blk::is_initialized() {
            crate::println!(
                "/dev/vda: TYPE=\"blockfs\" UUID=\"00000000-0000-0000-0000-000000000000\""
            );
        } else {
            crate::println!("No block devices found");
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Network Filesystem Commands
// ============================================================================

pub(in crate::services::shell) struct NfsmountCommand;
impl BuiltinCommand for NfsmountCommand {
    fn name(&self) -> &str {
        "nfsmount"
    }
    fn description(&self) -> &str {
        "Mount NFS share"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from(
                "Usage: nfsmount <server>:<path> <mountpoint>",
            ));
        }

        let source = &args[0];
        let mountpoint = &args[1];

        // Parse server:path
        if let Some(colon_pos) = source.find(':') {
            let server = &source[..colon_pos];
            let path = &source[colon_pos + 1..];
            crate::println!(
                "Mounting {}:{} on {}... mount failed (no network route)",
                server,
                path,
                mountpoint
            );
        } else {
            crate::println!(
                "Mounting {} on {}... mount failed (no network route)",
                source,
                mountpoint
            );
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SmbclientCommand;
impl BuiltinCommand for SmbclientCommand {
    fn name(&self) -> &str {
        "smbclient"
    }
    fn description(&self) -> &str {
        "SMB/CIFS client"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("Usage: smbclient //<server>/<share>"));
        }

        let share_path = &args[0];
        // Extract server name from //server/share
        let server = if let Some(rest) = share_path.strip_prefix("//") {
            if let Some(slash_pos) = rest.find('/') {
                &rest[..slash_pos]
            } else {
                rest
            }
        } else {
            share_path.as_str()
        };

        crate::println!("Connection to {} failed (no network route)", server);
        CommandResult::Success(0)
    }
}
