//! Hello World Test Program
//!
//! Basic test to verify console output and basic kernel functionality.

use alloc::string::{String, ToString};
use super::{TestProgram, TestResult};

pub struct HelloWorldTest;

impl HelloWorldTest {
    pub fn new() -> Self {
        Self
    }
}

impl TestProgram for HelloWorldTest {
    fn name(&self) -> &str {
        "hello_world"
    }
    
    fn description(&self) -> &str {
        "Basic console output test"
    }
    
    fn run(&mut self) -> TestResult {
        // Test console output
        crate::println!("Hello, VeridianOS!");
        crate::println!("Testing console output...");
        
        // Test string formatting
        let version = "Phase 2";
        crate::println!("Running VeridianOS {}", version);
        
        // Test different output types
        let number = 42;
        let hex_value = 0xDEADBEEF;
        crate::println!("Number: {}, Hex: 0x{:X}", number, hex_value);
        
        TestResult {
            name: self.name().to_string(),
            passed: true,
            message: "Console output working correctly".to_string(),
        }
    }
}