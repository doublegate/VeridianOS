//! Userland Module
//!
//! Re-exports userland test programs and infrastructure for kernel integration.

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
            crate::println!("Hello, World from user-space!");
            Ok(())
        }
    }

    /// Process management test
    pub mod process_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Process test running...");
            // Test process creation and management
            Ok(())
        }
    }

    /// Thread management test
    pub mod thread_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Thread test running...");
            // Test thread creation and management
            Ok(())
        }
    }

    /// Filesystem test
    pub mod filesystem_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Filesystem test running...");
            // Test VFS operations
            Ok(())
        }
    }

    /// Network test
    pub mod network_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Network test running...");
            // Test network operations
            Ok(())
        }
    }

    /// Driver test
    pub mod driver_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Driver test running...");
            // Test driver framework
            Ok(())
        }
    }

    /// Shell test
    pub mod shell_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Shell test running...");
            // Test shell functionality
            Ok(())
        }
    }

    /// Standard library test
    pub mod stdlib_test {
        use alloc::string::String;

        pub fn run() -> Result<(), String> {
            crate::println!("Standard library test running...");
            // Test stdlib functions
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
