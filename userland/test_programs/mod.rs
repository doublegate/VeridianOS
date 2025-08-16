//! Test Programs Module
//!
//! Provides test binaries for validating Phase 2 user-space functionality.

pub mod hello_world;
pub mod thread_test;
pub mod filesystem_test;
pub mod network_test;
pub mod driver_test;
pub mod shell_test;
pub mod process_test;
pub mod stdlib_test;

use alloc::string::String;
use alloc::vec::Vec;

/// Test program result
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

/// Test program trait
pub trait TestProgram {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn run(&mut self) -> TestResult;
}

/// Test program registry
pub struct TestRegistry {
    programs: Vec<Box<dyn TestProgram>>,
}

impl TestRegistry {
    pub fn new() -> Self {
        Self {
            programs: Vec::new(),
        }
    }
    
    /// Register a test program
    pub fn register(&mut self, program: Box<dyn TestProgram>) {
        self.programs.push(program);
    }
    
    /// Run all test programs
    pub fn run_all(&mut self) -> Vec<TestResult> {
        let mut results = Vec::new();
        
        for program in &mut self.programs {
            crate::println!("[TEST] Running: {}", program.name());
            let result = program.run();
            
            if result.passed {
                crate::println!("[TEST] ✓ {}: {}", result.name, result.message);
            } else {
                crate::println!("[TEST] ✗ {}: {}", result.name, result.message);
            }
            
            results.push(result);
        }
        
        results
    }
    
    /// Run specific test by name
    pub fn run_test(&mut self, name: &str) -> Option<TestResult> {
        for program in &mut self.programs {
            if program.name() == name {
                return Some(program.run());
            }
        }
        None
    }
    
    /// List all test programs
    pub fn list_tests(&self) -> Vec<(String, String)> {
        self.programs.iter()
            .map(|p| (p.name().to_string(), p.description().to_string()))
            .collect()
    }
}

/// Initialize test programs
pub fn init_test_programs() -> TestRegistry {
    let mut registry = TestRegistry::new();
    
    registry.register(Box::new(hello_world::HelloWorldTest::new()));
    registry.register(Box::new(thread_test::ThreadTest::new()));
    registry.register(Box::new(filesystem_test::FilesystemTest::new()));
    registry.register(Box::new(network_test::NetworkTest::new()));
    registry.register(Box::new(driver_test::DriverTest::new()));
    registry.register(Box::new(shell_test::ShellTest::new()));
    registry.register(Box::new(process_test::ProcessTest::new()));
    registry.register(Box::new(stdlib_test::StdlibTest::new()));
    
    crate::println!("[TEST] Initialized {} test programs", registry.programs.len());
    registry
}