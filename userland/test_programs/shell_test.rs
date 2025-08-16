//! Shell Test Program
//!
//! Tests shell command parsing and built-in command execution.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::{TestProgram, TestResult};
use crate::services::shell::{get_shell, CommandResult};

pub struct ShellTest;

impl ShellTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_command_parsing(&mut self) -> bool {
        let shell = get_shell();
        let mut shell_instance = shell.lock();
        
        // Test simple command parsing
        let simple_cmd = "echo hello world";
        match shell_instance.parse_command(simple_cmd) {
            Ok(parsed) => {
                crate::println!("[SHELL] Parsed command: {} with {} args", 
                    parsed.command, parsed.args.len());
                parsed.command == "echo" && parsed.args.len() == 2
            }
            Err(e) => {
                crate::println!("[SHELL] Command parsing failed: {}", e);
                false
            }
        }
    }
    
    fn test_builtin_commands(&mut self) -> bool {
        let shell = get_shell();
        let mut shell_instance = shell.lock();
        let mut all_passed = true;
        
        // Test echo command
        match shell_instance.execute_command("echo testing shell builtins") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Echo output: {}", output);
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] Echo command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] Echo command failed: {}", e);
                all_passed = false;
            }
        }
        
        // Test pwd command
        match shell_instance.execute_command("pwd") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Current directory: {}", output);
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] PWD command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] PWD command failed: {}", e);
                all_passed = false;
            }
        }
        
        // Test help command
        match shell_instance.execute_command("help") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Help command successful (length: {})", output.len());
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] Help command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] Help command failed: {}", e);
                all_passed = false;
            }
        }
        
        all_passed
    }
    
    fn test_environment_variables(&mut self) -> bool {
        let shell = get_shell();
        let mut shell_instance = shell.lock();
        
        // Test setting environment variable
        match shell_instance.execute_command("export TEST_VAR=hello_world") {
            Ok(CommandResult::Success(_)) => {
                crate::println!("[SHELL] Environment variable set");
                
                // Test getting environment variable
                match shell_instance.execute_command("echo $TEST_VAR") {
                    Ok(CommandResult::Success(output)) => {
                        crate::println!("[SHELL] Environment variable value: {}", output);
                        output.contains("hello_world")
                    }
                    Ok(CommandResult::Error(e)) => {
                        crate::println!("[SHELL] Environment variable read error: {}", e);
                        false
                    }
                    Err(e) => {
                        crate::println!("[SHELL] Environment variable read failed: {}", e);
                        false
                    }
                }
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] Environment variable set error: {}", e);
                false
            }
            Err(e) => {
                crate::println!("[SHELL] Environment variable set failed: {}", e);
                false
            }
        }
    }
    
    fn test_history_functionality(&mut self) -> bool {
        let shell = get_shell();
        let mut shell_instance = shell.lock();
        
        // Execute a few commands to build history
        let commands = vec![
            "echo command1",
            "pwd", 
            "echo command2",
        ];
        
        for cmd in &commands {
            shell_instance.execute_command(cmd).ok();
        }
        
        // Test history command
        match shell_instance.execute_command("history") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Command history (length: {})", output.len());
                // History should contain our test commands
                commands.iter().all(|cmd| output.contains(cmd))
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] History command error: {}", e);
                false
            }
            Err(e) => {
                crate::println!("[SHELL] History command failed: {}", e);
                false
            }
        }
    }
    
    fn test_system_info_commands(&mut self) -> bool {
        let shell = get_shell();
        let mut shell_instance = shell.lock();
        let mut all_passed = true;
        
        // Test ps command (process list)
        match shell_instance.execute_command("ps") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Process list: {}", output);
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] PS command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] PS command failed: {}", e);
                all_passed = false;
            }
        }
        
        // Test mem command (memory info)
        match shell_instance.execute_command("mem") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Memory info: {}", output);
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] MEM command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] MEM command failed: {}", e);
                all_passed = false;
            }
        }
        
        // Test uptime command
        match shell_instance.execute_command("uptime") {
            Ok(CommandResult::Success(output)) => {
                crate::println!("[SHELL] Uptime: {}", output);
            }
            Ok(CommandResult::Error(e)) => {
                crate::println!("[SHELL] Uptime command error: {}", e);
                all_passed = false;
            }
            Err(e) => {
                crate::println!("[SHELL] Uptime command failed: {}", e);
                all_passed = false;
            }
        }
        
        all_passed
    }
}

impl TestProgram for ShellTest {
    fn name(&self) -> &str {
        "shell_test"
    }
    
    fn description(&self) -> &str {
        "Shell command parsing and execution test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test command parsing
        if self.test_command_parsing() {
            messages.push("✓ Command parsing");
        } else {
            messages.push("✗ Command parsing");
            passed = false;
        }
        
        // Test builtin commands
        if self.test_builtin_commands() {
            messages.push("✓ Builtin commands");
        } else {
            messages.push("✗ Builtin commands");
            passed = false;
        }
        
        // Test environment variables
        if self.test_environment_variables() {
            messages.push("✓ Environment variables");
        } else {
            messages.push("✗ Environment variables");
            passed = false;
        }
        
        // Test history functionality
        if self.test_history_functionality() {
            messages.push("✓ Command history");
        } else {
            messages.push("✗ Command history");
            passed = false;
        }
        
        // Test system info commands
        if self.test_system_info_commands() {
            messages.push("✓ System info commands");
        } else {
            messages.push("✗ System info commands");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}