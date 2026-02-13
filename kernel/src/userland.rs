//! Userland Module
//!
//! Re-exports userland test programs and infrastructure for kernel integration.

#![allow(clippy::type_complexity)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Re-export userland functionality for kernel integration
pub use crate::userspace::*;

// Import userland test infrastructure from root
pub mod test_programs {
    //! Test programs for user-space validation

    #[cfg(feature = "alloc")]
    extern crate alloc;

    /// Hello world test program
    pub mod hello_world {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Hello, World from VeridianOS!");
            Ok(())
        }
    }

    /// Process management test - verifies process server has processes
    pub mod process_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Process test: checking process server...");
            let ps = crate::services::process_server::get_process_server();
            let processes = ps.list_processes();
            if processes.is_empty() {
                return Err(String::from("Process server has no processes"));
            }
            crate::println!("  Found {} processes", processes.len());
            Ok(())
        }
    }

    /// Thread management test - verifies thread manager responds
    pub mod thread_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Thread test: checking thread manager...");
            let tm = crate::thread_api::get_thread_manager();
            if tm.get_current_thread_id().is_some() {
                crate::println!("  Thread manager responding");
                Ok(())
            } else {
                Err(String::from("Thread manager not responding"))
            }
        }
    }

    /// Filesystem test - exercises VFS write/read/mkdir/readdir
    pub mod filesystem_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Filesystem test: exercising VFS...");

            // Write a file
            crate::fs::write_file("/tmp/fs_test.txt", b"VFS works").map_err(String::from)?;

            // Read it back
            let data = crate::fs::read_file("/tmp/fs_test.txt").map_err(String::from)?;
            if data != b"VFS works" {
                return Err(String::from("Read data mismatch"));
            }
            crate::println!("  Write/read verified");

            // List /tmp
            let vfs = crate::fs::get_vfs().read();
            let node = vfs.resolve_path("/tmp").map_err(String::from)?;
            let entries = node.readdir().map_err(String::from)?;
            let found = entries.iter().any(|e| e.name == "fs_test.txt");
            if !found {
                return Err(String::from("File not found in directory listing"));
            }
            crate::println!("  Directory listing verified");

            Ok(())
        }
    }

    /// Network test - checks if network subsystem is reachable
    pub mod network_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Network test: checking network subsystem...");
            let _initialized = crate::drivers::network::is_network_initialized();
            crate::println!("  Network subsystem checked (non-critical)");
            Ok(())
        }
    }

    /// Driver test - verifies driver framework has registered drivers
    pub mod driver_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Driver test: checking driver framework...");
            let df = crate::services::driver_framework::get_driver_framework();
            let stats = df.get_statistics();
            crate::println!(
                "  Drivers: {}, Buses: {}",
                stats.total_drivers,
                stats.total_buses
            );
            if stats.total_drivers == 0 && stats.total_buses == 0 {
                return Err(String::from("No drivers or buses registered"));
            }
            Ok(())
        }
    }

    /// Shell test - runs built-in commands programmatically
    pub mod shell_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Shell test: executing built-in commands...");
            let shell = crate::services::shell::get_shell();

            // Test pwd
            if !matches!(
                shell.execute_command("pwd"),
                crate::services::shell::CommandResult::Success(_)
            ) {
                return Err(String::from("pwd command failed"));
            }
            crate::println!("  pwd: ok");

            // Test env
            if !matches!(
                shell.execute_command("env"),
                crate::services::shell::CommandResult::Success(_)
            ) {
                return Err(String::from("env command failed"));
            }
            crate::println!("  env: ok");

            // Test nonexistent command returns NotFound
            if !matches!(
                shell.execute_command("nonexistent_cmd_xyz"),
                crate::services::shell::CommandResult::NotFound
            ) {
                return Err(String::from("Expected NotFound for unknown command"));
            }
            crate::println!("  not-found detection: ok");

            Ok(())
        }
    }

    /// Standard library test
    pub mod stdlib_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Standard library test: basic validation...");
            // Verify alloc works (String, Vec)
            let s = String::from("hello");
            let v: alloc::vec::Vec<u32> = alloc::vec![1, 2, 3];
            if s.len() != 5 || v.len() != 3 {
                return Err(String::from("Basic alloc types broken"));
            }
            crate::println!("  alloc types: ok");
            Ok(())
        }
    }
}

pub mod test_runner {
    //! Test runner for user-space programs

    #[cfg(feature = "alloc")]
    extern crate alloc;

    use alloc::{format, string::String, vec::Vec};

    /// Test suite summary
    #[derive(Debug, Clone)]
    pub struct TestSuiteSummary {
        pub total_tests: usize,
        pub passed: usize,
        pub failed: usize,
        pub errors: Vec<String>,
    }

    impl Default for TestSuiteSummary {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TestSuiteSummary {
        pub fn new() -> Self {
            Self {
                total_tests: 0,
                passed: 0,
                failed: 0,
                errors: Vec::new(),
            }
        }

        pub fn success_rate(&self) -> f32 {
            if self.total_tests == 0 {
                100.0
            } else {
                (self.passed as f32 / self.total_tests as f32) * 100.0
            }
        }
    }

    /// Run Phase 2 validation
    pub fn run_phase2_validation() -> TestSuiteSummary {
        let mut summary = TestSuiteSummary::new();

        crate::println!("🚀 Running Phase 2 Validation Tests...");

        // Run all test programs
        let tests: [(&str, fn() -> Result<(), String>); 8] = [
            ("Hello World", super::test_programs::hello_world::run),
            ("Process Test", super::test_programs::process_test::run),
            ("Thread Test", super::test_programs::thread_test::run),
            (
                "Filesystem Test",
                super::test_programs::filesystem_test::run,
            ),
            ("Network Test", super::test_programs::network_test::run),
            ("Driver Test", super::test_programs::driver_test::run),
            ("Shell Test", super::test_programs::shell_test::run),
            ("Stdlib Test", super::test_programs::stdlib_test::run),
        ];

        for (name, test_fn) in &tests {
            summary.total_tests += 1;
            match test_fn() {
                Ok(()) => {
                    crate::println!("✅ {} - PASSED", name);
                    summary.passed += 1;
                }
                Err(e) => {
                    crate::println!("❌ {} - FAILED: {}", name, e);
                    summary.failed += 1;
                    summary.errors.push(format!("{}: {}", name, e));
                }
            }
        }

        crate::println!("");
        crate::println!(
            "📊 Test Results: {}/{} passed ({:.1}%)",
            summary.passed,
            summary.total_tests,
            summary.success_rate()
        );

        summary
    }

    /// Run critical tests
    pub fn run_critical_tests() -> TestSuiteSummary {
        let mut summary = TestSuiteSummary::new();

        crate::println!("🔥 Running Critical Tests...");

        // Critical tests
        let tests: [(&str, fn() -> Result<(), String>); 3] = [
            ("Process Test", super::test_programs::process_test::run),
            (
                "Filesystem Test",
                super::test_programs::filesystem_test::run,
            ),
            ("Driver Test", super::test_programs::driver_test::run),
        ];

        for (name, test_fn) in &tests {
            summary.total_tests += 1;
            match test_fn() {
                Ok(()) => {
                    crate::println!("✅ {} - PASSED", name);
                    summary.passed += 1;
                }
                Err(e) => {
                    crate::println!("❌ {} - FAILED: {}", name, e);
                    summary.failed += 1;
                    summary.errors.push(format!("{}: {}", name, e));
                }
            }
        }

        summary
    }

    /// Run specific tests
    pub fn run_specific_tests(test_names: &[&str]) -> TestSuiteSummary {
        let mut summary = TestSuiteSummary::new();

        crate::println!("🎯 Running Specific Tests...");

        for name in test_names {
            summary.total_tests += 1;
            let result = match *name {
                "hello_world" => super::test_programs::hello_world::run(),
                "process" => super::test_programs::process_test::run(),
                "thread" => super::test_programs::thread_test::run(),
                "filesystem" => super::test_programs::filesystem_test::run(),
                "network" => super::test_programs::network_test::run(),
                "driver" => super::test_programs::driver_test::run(),
                "shell" => super::test_programs::shell_test::run(),
                "stdlib" => super::test_programs::stdlib_test::run(),
                _ => Err(format!("Unknown test: {}", name)),
            };

            match result {
                Ok(()) => {
                    crate::println!("✅ {} - PASSED", name);
                    summary.passed += 1;
                }
                Err(e) => {
                    crate::println!("❌ {} - FAILED: {}", name, e);
                    summary.failed += 1;
                    summary.errors.push(format!("{}: {}", name, e));
                }
            }
        }

        summary
    }

    /// Interactive test menu (placeholder for kernel context)
    pub fn interactive_test_menu() -> TestSuiteSummary {
        crate::println!("📋 Interactive test menu not available in kernel context");
        crate::println!("Running full Phase 2 validation instead...");
        run_phase2_validation()
    }
}
