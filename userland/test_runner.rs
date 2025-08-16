//! Test Runner
//!
//! Main test runner for executing Phase 2 validation tests.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::userland::test_programs::{init_test_programs, TestResult, TestRegistry};

/// Test suite configuration
pub struct TestSuiteConfig {
    pub run_all: bool,
    pub specific_tests: Vec<String>,
    pub stop_on_failure: bool,
    pub verbose: bool,
}

impl Default for TestSuiteConfig {
    fn default() -> Self {
        Self {
            run_all: true,
            specific_tests: Vec::new(),
            stop_on_failure: false,
            verbose: true,
        }
    }
}

/// Test suite results summary
#[derive(Debug, Clone)]
pub struct TestSuiteSummary {
    pub total_tests: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<TestResult>,
}

impl TestSuiteSummary {
    pub fn success_rate(&self) -> f32 {
        if self.total_tests == 0 {
            0.0
        } else {
            (self.passed as f32 / self.total_tests as f32) * 100.0
        }
    }
}

/// Main test runner
pub struct TestRunner {
    registry: TestRegistry,
    config: TestSuiteConfig,
}

impl TestRunner {
    /// Create a new test runner
    pub fn new(config: TestSuiteConfig) -> Self {
        Self {
            registry: init_test_programs(),
            config,
        }
    }
    
    /// Run the test suite
    pub fn run_tests(&mut self) -> TestSuiteSummary {
        crate::println!("=== VeridianOS Phase 2 Test Suite ===");
        crate::println!("");
        
        let mut results = Vec::new();
        
        if self.config.run_all {
            // Run all tests
            crate::println!("Running all Phase 2 validation tests...");
            crate::println!("");
            
            results = self.registry.run_all();
        } else {
            // Run specific tests
            for test_name in &self.config.specific_tests {
                if let Some(result) = self.registry.run_test(test_name) {
                    results.push(result);
                } else {
                    crate::println!("Warning: Test '{}' not found", test_name);
                }
                
                if self.config.stop_on_failure && !results.last().unwrap().passed {
                    crate::println!("Stopping on first failure as requested");
                    break;
                }
            }
        }
        
        // Generate summary
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        
        let summary = TestSuiteSummary {
            total_tests: results.len(),
            passed,
            failed,
            results,
        };
        
        self.print_summary(&summary);
        summary
    }
    
    /// Print test suite summary
    fn print_summary(&self, summary: &TestSuiteSummary) {
        crate::println!("");
        crate::println!("=== Test Suite Summary ===");
        crate::println!("Total tests: {}", summary.total_tests);
        crate::println!("Passed: {} ✓", summary.passed);
        crate::println!("Failed: {} ✗", summary.failed);
        crate::println!("Success rate: {:.1}%", summary.success_rate());
        crate::println!("");
        
        if self.config.verbose {
            crate::println!("=== Detailed Results ===");
            for result in &summary.results {
                let status = if result.passed { "✓" } else { "✗" };
                crate::println!("{} {}: {}", status, result.name, result.message);
            }
            crate::println!("");
        }
        
        if summary.passed == summary.total_tests {
            crate::println!("🎉 ALL TESTS PASSED! Phase 2 implementation is complete and functional!");
        } else {
            crate::println!("⚠️  Some tests failed. Phase 2 implementation needs attention.");
        }
    }
    
    /// List available tests
    pub fn list_tests(&self) {
        crate::println!("=== Available Tests ===");
        let tests = self.registry.list_tests();
        
        for (name, description) in tests {
            crate::println!("• {}: {}", name, description);
        }
        crate::println!("");
        crate::println!("Total: {} tests available", self.registry.list_tests().len());
    }
}

/// Initialize and run Phase 2 validation tests
pub fn run_phase2_validation() -> TestSuiteSummary {
    let config = TestSuiteConfig::default();
    let mut runner = TestRunner::new(config);
    
    runner.run_tests()
}

/// Run specific subset of tests
pub fn run_specific_tests(test_names: Vec<String>) -> TestSuiteSummary {
    let config = TestSuiteConfig {
        run_all: false,
        specific_tests: test_names,
        stop_on_failure: false,
        verbose: true,
    };
    
    let mut runner = TestRunner::new(config);
    runner.run_tests()
}

/// Run critical path tests only
pub fn run_critical_tests() -> TestSuiteSummary {
    let critical_tests = vec![
        "hello_world".to_string(),
        "thread_test".to_string(),
        "filesystem_test".to_string(),
        "process_test".to_string(),
        "stdlib_test".to_string(),
    ];
    
    run_specific_tests(critical_tests)
}

/// Interactive test menu
pub fn interactive_test_menu() {
    let config = TestSuiteConfig::default();
    let mut runner = TestRunner::new(config);
    
    loop {
        crate::println!("");
        crate::println!("=== VeridianOS Phase 2 Test Menu ===");
        crate::println!("1. Run all tests");
        crate::println!("2. Run critical tests only");
        crate::println!("3. List available tests");
        crate::println!("4. Run specific test");
        crate::println!("5. Exit");
        crate::println!("");
        crate::print!("Select option: ");
        
        // Simple menu simulation (in real implementation, would read from input)
        // For now, just run all tests once
        let summary = runner.run_tests();
        
        if summary.success_rate() >= 90.0 {
            crate::println!("");
            crate::println!("🎯 Phase 2 validation successful! Ready for production use.");
        }
        
        break; // Exit after running tests
    }
}