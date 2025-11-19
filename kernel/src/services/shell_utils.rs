//! Additional Shell Utilities
//!
//! Implements find, grep, wc, head, tail, and other common Unix utilities.

use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

use super::shell::{BuiltinCommand, CommandResult, Shell};

/// Find command - search for files
pub struct FindCommand;

impl BuiltinCommand for FindCommand {
    fn name(&self) -> &str {
        "find"
    }
    fn description(&self) -> &str {
        "Search for files in directory hierarchy"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let start_path = if args.is_empty() {
            shell.get_cwd()
        } else {
            args[0].clone()
        };

        let pattern = if args.len() > 1 { Some(&args[1]) } else { None };

        match find_files(&start_path, pattern) {
            Ok(files) => {
                for file in files {
                    crate::println!("{}", file);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("find: {}", e)),
        }
    }
}

/// Grep command - search file contents
pub struct GrepCommand;

impl BuiltinCommand for GrepCommand {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search for patterns in files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("grep: missing pattern or file"));
        }

        let pattern = &args[0];
        let file_path = &args[1];

        match grep_file(pattern, file_path) {
            Ok(matches) => {
                for line in matches {
                    crate::println!("{}", line);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("grep: {}", e)),
        }
    }
}

/// Word count command
pub struct WcCommand;

impl BuiltinCommand for WcCommand {
    fn name(&self) -> &str {
        "wc"
    }
    fn description(&self) -> &str {
        "Count lines, words, and characters"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("wc: missing file operand"));
        }

        for file_path in args {
            match count_file(file_path) {
                Ok((lines, words, chars)) => {
                    crate::println!("{:6} {:6} {:6} {}", lines, words, chars, file_path);
                }
                Err(e) => {
                    return CommandResult::Error(format!("wc: {}: {}", file_path, e));
                }
            }
        }

        CommandResult::Success(0)
    }
}

/// Head command - show first lines of file
pub struct HeadCommand;

impl BuiltinCommand for HeadCommand {
    fn name(&self) -> &str {
        "head"
    }
    fn description(&self) -> &str {
        "Output the first part of files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("head: missing file operand"));
        }

        let num_lines = 10; // Default to 10 lines
        let file_path = &args[0];

        match head_file(file_path, num_lines) {
            Ok(lines) => {
                for line in lines {
                    crate::println!("{}", line);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("head: {}: {}", file_path, e)),
        }
    }
}

/// Tail command - show last lines of file
pub struct TailCommand;

impl BuiltinCommand for TailCommand {
    fn name(&self) -> &str {
        "tail"
    }
    fn description(&self) -> &str {
        "Output the last part of files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("tail: missing file operand"));
        }

        let num_lines = 10; // Default to 10 lines
        let file_path = &args[0];

        match tail_file(file_path, num_lines) {
            Ok(lines) => {
                for line in lines {
                    crate::println!("{}", line);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("tail: {}: {}", file_path, e)),
        }
    }
}

/// Diff command - compare files
pub struct DiffCommand;

impl BuiltinCommand for DiffCommand {
    fn name(&self) -> &str {
        "diff"
    }
    fn description(&self) -> &str {
        "Compare files line by line"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("diff: missing file operands"));
        }

        let file1 = &args[0];
        let file2 = &args[1];

        match diff_files(file1, file2) {
            Ok(differences) => {
                if differences.is_empty() {
                    crate::println!("Files are identical");
                } else {
                    for diff in differences {
                        crate::println!("{}", diff);
                    }
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("diff: {}", e)),
        }
    }
}

/// Sort command - sort lines of text
pub struct SortCommand;

impl BuiltinCommand for SortCommand {
    fn name(&self) -> &str {
        "sort"
    }
    fn description(&self) -> &str {
        "Sort lines of text files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("sort: missing file operand"));
        }

        match sort_file(&args[0]) {
            Ok(lines) => {
                for line in lines {
                    crate::println!("{}", line);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("sort: {}", e)),
        }
    }
}

/// Uniq command - report or omit repeated lines
pub struct UniqCommand;

impl BuiltinCommand for UniqCommand {
    fn name(&self) -> &str {
        "uniq"
    }
    fn description(&self) -> &str {
        "Report or omit repeated lines"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("uniq: missing file operand"));
        }

        match uniq_file(&args[0]) {
            Ok(lines) => {
                for line in lines {
                    crate::println!("{}", line);
                }
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("uniq: {}", e)),
        }
    }
}

// Helper functions

fn find_files(start_path: &str, pattern: Option<&String>) -> Result<Vec<String>, &'static str> {
    let mut results = Vec::new();

    // Recursively search directory tree
    fn search_dir(
        path: &str,
        pattern: Option<&String>,
        results: &mut Vec<String>,
    ) -> Result<(), &'static str> {
        let vfs = crate::fs::get_vfs().read();
        let node = vfs.resolve_path(path)?;

        match node.readdir() {
            Ok(entries) => {
                for entry in entries {
                    let full_path = if path.ends_with('/') {
                        format!("{}{}", path, entry.name)
                    } else {
                        format!("{}/{}", path, entry.name)
                    };

                    // Check if matches pattern
                    if let Some(pat) = pattern {
                        if entry.name.contains(pat.as_str()) {
                            results.push(full_path.clone());
                        }
                    } else {
                        results.push(full_path.clone());
                    }

                    // Recurse into subdirectories
                    if entry.node_type == crate::fs::NodeType::Directory {
                        let _ = search_dir(&full_path, pattern, results);
                    }
                }
                Ok(())
            }
            Err(_) => Ok(()), // Skip directories we can't read
        }
    }

    search_dir(start_path, pattern, &mut results)?;
    Ok(results)
}

fn grep_file(pattern: &str, file_path: &str) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let mut matches = Vec::new();
    for line in content.lines() {
        if line.contains(pattern) {
            matches.push(String::from(line));
        }
    }

    Ok(matches)
}

fn count_file(file_path: &str) -> Result<(usize, usize, usize), &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let lines = content.lines().count();
    let words = content.split_whitespace().count();
    let chars = content.chars().count();

    Ok((lines, words, chars))
}

fn head_file(file_path: &str, num_lines: usize) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let lines: Vec<String> = content.lines().take(num_lines).map(String::from).collect();

    Ok(lines)
}

fn tail_file(file_path: &str, num_lines: usize) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let all_lines: Vec<String> = content.lines().map(String::from).collect();
    let start = if all_lines.len() > num_lines {
        all_lines.len() - num_lines
    } else {
        0
    };

    Ok(all_lines[start..].to_vec())
}

fn diff_files(file1: &str, file2: &str) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();

    let node1 = vfs.resolve_path(file1)?;
    let node2 = vfs.resolve_path(file2)?;

    let mut buffer1 = [0u8; 4096];
    let mut buffer2 = [0u8; 4096];

    let bytes1 = node1.read(0, &mut buffer1)?;
    let bytes2 = node2.read(0, &mut buffer2)?;

    let content1 =
        core::str::from_utf8(&buffer1[..bytes1]).map_err(|_| "Invalid UTF-8 in file1")?;
    let content2 =
        core::str::from_utf8(&buffer2[..bytes2]).map_err(|_| "Invalid UTF-8 in file2")?;

    let lines1: Vec<&str> = content1.lines().collect();
    let lines2: Vec<&str> = content2.lines().collect();

    let mut differences = Vec::new();

    let max_len = lines1.len().max(lines2.len());
    for i in 0..max_len {
        let line1 = lines1.get(i).copied();
        let line2 = lines2.get(i).copied();

        if line1 != line2 {
            if let Some(l1) = line1 {
                differences.push(format!("< {}", l1));
            }
            if let Some(l2) = line2 {
                differences.push(format!("> {}", l2));
            }
        }
    }

    Ok(differences)
}

fn sort_file(file_path: &str) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    lines.sort();

    Ok(lines)
}

fn uniq_file(file_path: &str) -> Result<Vec<String>, &'static str> {
    let vfs = crate::fs::get_vfs().read();
    let node = vfs.resolve_path(file_path)?;

    let mut buffer = [0u8; 4096];
    let bytes_read = node.read(0, &mut buffer)?;

    let content = core::str::from_utf8(&buffer[..bytes_read]).map_err(|_| "Invalid UTF-8")?;

    let mut unique_lines = Vec::new();
    let mut last_line: Option<String> = None;

    for line in content.lines() {
        if Some(line) != last_line.as_deref() {
            unique_lines.push(String::from(line));
            last_line = Some(String::from(line));
        }
    }

    Ok(unique_lines)
}

/// Register all utility commands with the shell
pub fn register_utils(shell: &Shell) {
    // Use the public API to register commands
    let commands: Vec<Box<dyn BuiltinCommand>> = vec![
        Box::new(FindCommand),
        Box::new(GrepCommand),
        Box::new(WcCommand),
        Box::new(HeadCommand),
        Box::new(TailCommand),
        Box::new(DiffCommand),
        Box::new(SortCommand),
        Box::new(UniqCommand),
    ];

    shell.register_builtins_batch(commands);
}
