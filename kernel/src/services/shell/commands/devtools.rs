//! Development tools commands (git, make, gdb, profiler, CI).

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String, vec::Vec};

use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

// ============================================================================
// Git Command
// ============================================================================

pub(in crate::services::shell) struct GitCommand;
impl BuiltinCommand for GitCommand {
    fn name(&self) -> &str {
        "git"
    }
    fn description(&self) -> &str {
        "Version control system"
    }
    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: git init|status|log|add|commit|diff|clone");
            return CommandResult::Success(1);
        }
        let pwd = shell.get_env("PWD").unwrap_or_else(|| String::from("/"));
        match args[0].as_str() {
            "init" => {
                let repo = crate::devtools::git::commands::Repository::init(&pwd);
                let branch = repo.refs.current_branch().unwrap_or("main");
                crate::println!("Initialized empty git repository in {}/.git/", pwd);
                crate::println!("Default branch: {}", branch);
            }
            "status" => {
                let repo = crate::devtools::git::commands::Repository::init(&pwd);
                let lines = repo.status();
                for line in &lines {
                    crate::println!("{}", line);
                }
            }
            "log" => {
                let repo = crate::devtools::git::commands::Repository::init(&pwd);
                if repo.refs.head().is_none() {
                    crate::println!("fatal: your current branch has no commits yet");
                    return CommandResult::Success(1);
                }
                let entries = repo.log(10);
                if entries.is_empty() {
                    crate::println!("(no commits)");
                } else {
                    for entry in &entries {
                        crate::println!("commit {:?}", entry.id);
                        crate::println!("Author: {}", entry.author);
                        crate::println!("");
                        crate::println!("    {}", entry.message);
                        crate::println!("");
                    }
                }
            }
            "add" => {
                if args.len() < 2 {
                    crate::println!("git add: missing file argument");
                    return CommandResult::Success(1);
                }
                let path = &args[1];
                let full_path = if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("{}/{}", pwd, path)
                };
                match crate::fs::read_file(&full_path) {
                    Ok(content) => {
                        let mut repo = crate::devtools::git::commands::Repository::init(&pwd);
                        let id = repo.add(path, &content);
                        crate::println!("Added '{}' to staging area ({:?})", path, id);
                    }
                    Err(e) => {
                        crate::println!("git add: cannot read '{}': {:?}", path, e);
                        return CommandResult::Success(1);
                    }
                }
            }
            "commit" => {
                if args.len() >= 3 && args[1] == "-m" {
                    let msg = args[2..].join(" ");
                    let mut repo = crate::devtools::git::commands::Repository::init(&pwd);
                    match repo.commit(&msg) {
                        Some(id) => {
                            let branch = repo.refs.current_branch().unwrap_or("HEAD");
                            crate::println!("[{} {:?}] {}", branch, id, msg);
                        }
                        None => {
                            crate::println!("nothing to commit (create files and use 'git add')");
                            return CommandResult::Success(1);
                        }
                    }
                } else {
                    crate::println!("git commit: missing -m <message>");
                    return CommandResult::Success(1);
                }
            }
            "diff" => {
                let repo = crate::devtools::git::commands::Repository::init(&pwd);
                if repo.index_count() == 0 {
                    crate::println!("(no staged changes to diff)");
                } else {
                    crate::println!(
                        "{} file(s) staged, use 'git commit -m <msg>' to commit",
                        repo.index_count()
                    );
                }
            }
            "clone" => {
                if args.len() < 2 {
                    crate::println!("git clone: missing <url> argument");
                    return CommandResult::Success(1);
                }
                crate::println!("git: clone not supported (no network route)");
            }
            other => {
                crate::println!("git: '{}' is not a git command", other);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Make Command
// ============================================================================

pub(in crate::services::shell) struct MakeCommand;
impl BuiltinCommand for MakeCommand {
    fn name(&self) -> &str {
        "make"
    }
    fn description(&self) -> &str {
        "Build system"
    }
    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let pwd = shell.get_env("PWD").unwrap_or_else(|| String::from("/"));
        let makefile_path = format!("{}/Makefile", pwd);

        if !crate::fs::file_exists(&makefile_path) {
            crate::println!("make: no Makefile found in '{}'", pwd);
            return CommandResult::Success(2);
        }

        // Read and parse Makefile for target names
        let mut targets: Vec<String> = Vec::new();
        if let Ok(content) = crate::fs::read_file(&makefile_path) {
            if let Ok(text) = core::str::from_utf8(&content) {
                for line in text.lines() {
                    // Targets are lines matching "name:" at the start (not indented)
                    if !line.starts_with('\t')
                        && !line.starts_with(' ')
                        && !line.starts_with('#')
                        && !line.is_empty()
                    {
                        if let Some(colon_pos) = line.find(':') {
                            let target_name = line[..colon_pos].trim();
                            if !target_name.is_empty() && !target_name.starts_with('.') {
                                targets.push(String::from(target_name));
                            }
                        }
                    }
                }
            }
        }

        if args.is_empty() {
            crate::println!("make: entering directory '{}'", pwd);
            if targets.is_empty() {
                crate::println!("make: nothing to be done for 'all'");
            } else {
                crate::println!("make: nothing to be done for '{}'", targets[0]);
                crate::println!("Available targets: {}", targets.join(", "));
            }
        } else {
            let requested = &args[0];
            if targets.iter().any(|t| t == requested.as_str()) {
                crate::println!("make: entering directory '{}'", pwd);
                crate::println!("make: nothing to be done for '{}'", requested);
            } else {
                crate::println!("make: *** No rule to make target '{}'. Stop.", requested);
                if !targets.is_empty() {
                    crate::println!("Available targets: {}", targets.join(", "));
                }
                return CommandResult::Success(2);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// GDB Command
// ============================================================================

pub(in crate::services::shell) struct GdbCommand;
impl BuiltinCommand for GdbCommand {
    fn name(&self) -> &str {
        "gdb"
    }
    fn description(&self) -> &str {
        "GDB debugger stub"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: gdb attach <pid>|info");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "attach" => {
                if args.len() < 2 {
                    crate::println!("gdb attach: missing <pid>");
                    return CommandResult::Success(1);
                }
                let pid_str = &args[1];
                let pid: u64 = match pid_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        crate::println!("gdb attach: invalid pid '{}'", pid_str);
                        return CommandResult::Success(1);
                    }
                };
                match crate::services::process_server::try_get_process_server() {
                    Some(ps) => {
                        let process_id = crate::process::pcb::ProcessId(pid);
                        match ps.get_process_info(process_id) {
                            Some(info) => {
                                crate::println!(
                                    "Attaching to process {} ({})... attached",
                                    pid,
                                    info.name
                                );
                                crate::println!(
                                    "  State: {:?}, Threads: {}, Memory: {} KB",
                                    info.state,
                                    info.thread_count,
                                    info.memory_usage / 1024
                                );
                            }
                            None => {
                                crate::println!("gdb attach: no process with pid {}", pid);
                                return CommandResult::Success(1);
                            }
                        }
                    }
                    None => {
                        crate::println!("gdb attach: process server not initialized");
                        return CommandResult::Success(1);
                    }
                }
            }
            "info" => {
                #[cfg(target_arch = "x86_64")]
                {
                    let active = crate::debug::gdb_stub::is_gdb_active();
                    if active {
                        crate::println!("GDB stub: active, listening on COM2 (:1234)");
                    } else {
                        crate::println!("GDB stub: inactive (no debugger attached)");
                        crate::println!("  Start QEMU with -s -S to enable GDB on :1234");
                    }
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    crate::println!("GDB stub: not available on this architecture");
                }
            }
            other => {
                crate::println!("gdb: unknown subcommand '{}'", other);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Profiler Command
// ============================================================================

pub(in crate::services::shell) struct ProfilerCommand;
impl BuiltinCommand for ProfilerCommand {
    fn name(&self) -> &str {
        "profiler"
    }
    fn description(&self) -> &str {
        "Performance profiler"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: profiler start|stop|report");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "start" => {
                if crate::perf::trace::is_enabled() {
                    crate::println!(
                        "Profiler already running ({} events captured)",
                        crate::perf::trace::total_events()
                    );
                } else {
                    crate::perf::trace::enable();
                    crate::println!("Profiler started (tracing enabled, 10 event types)");
                }
            }
            "stop" => {
                if crate::perf::trace::is_enabled() {
                    let events = crate::perf::trace::total_events();
                    crate::perf::trace::disable();
                    crate::println!("Profiler stopped. Captured {} events", events);
                } else {
                    crate::println!("Profiler is not running");
                }
            }
            "report" => {
                let events = crate::perf::trace::total_events();
                if events == 0 {
                    crate::println!("No profiling data available. Run 'profiler start' first");
                } else {
                    let count = if args.len() >= 2 {
                        args[1].parse::<usize>().unwrap_or(20)
                    } else {
                        20
                    };
                    crate::perf::trace::dump_trace(count);
                }
            }
            other => {
                crate::println!("profiler: unknown subcommand '{}'", other);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// CI Command
// ============================================================================

pub(in crate::services::shell) struct CiCommand;
impl BuiltinCommand for CiCommand {
    fn name(&self) -> &str {
        "ci"
    }
    fn description(&self) -> &str {
        "CI runner"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: ci run|status|list");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "run" => {
                use crate::devtools::ci::runner::{Job, Pipeline};

                let mut pipeline = Pipeline::new("shell-ci");

                let mut build_job = Job::new("build");
                build_job.add_step("compile", "cargo build --release");
                pipeline.add_job(build_job);

                let mut test_job = Job::new("test");
                test_job.add_step("unit-tests", "cargo test");
                test_job.dependencies.push(String::from("build"));
                pipeline.add_job(test_job);

                let mut lint_job = Job::new("lint");
                lint_job.add_step("clippy", "cargo clippy -- -D warnings");
                lint_job.dependencies.push(String::from("build"));
                pipeline.add_job(lint_job);

                crate::println!("Running CI pipeline '{}'...", pipeline.name);
                let success = pipeline.execute();

                for job in &pipeline.jobs {
                    let status_str = match job.status {
                        crate::devtools::ci::runner::JobStatus::Passed => "PASS",
                        crate::devtools::ci::runner::JobStatus::Failed => "FAIL",
                        crate::devtools::ci::runner::JobStatus::Skipped => "SKIP",
                        crate::devtools::ci::runner::JobStatus::Running => "RUN",
                        crate::devtools::ci::runner::JobStatus::Pending => "PEND",
                    };
                    crate::println!("  {}: {}", job.name, status_str);
                }

                let report = pipeline.generate_report(0);
                if success {
                    crate::println!(
                        "Pipeline: SUCCESS ({}/{} passed)",
                        report.passed,
                        report.total_jobs
                    );
                } else {
                    crate::println!(
                        "Pipeline: FAILED ({} passed, {} failed, {} skipped)",
                        report.passed,
                        report.failed,
                        report.skipped
                    );
                    return CommandResult::Success(1);
                }
            }
            "status" => {
                use crate::devtools::ci::runner::Pipeline;

                let pipeline = Pipeline::new("shell-ci");
                crate::println!(
                    "Pipeline '{}': {} jobs configured",
                    pipeline.name,
                    pipeline.job_count()
                );
                crate::println!(
                    "  Passed: {}, Failed: {}",
                    pipeline.passed_count(),
                    pipeline.failed_count()
                );
            }
            "list" => {
                crate::println!("Default pipeline jobs:");
                crate::println!("  build  - Compile the project (cargo build --release)");
                crate::println!("  test   - Run unit tests (cargo test) [depends: build]");
                crate::println!("  lint   - Run clippy lints (cargo clippy) [depends: build]");
            }
            other => {
                crate::println!("ci: unknown subcommand '{}'", other);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}
