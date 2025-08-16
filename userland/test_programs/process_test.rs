//! Process Management Test Program
//!
//! Tests process server operations including process creation, management, and cleanup.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::{TestProgram, TestResult};
use crate::services::process_server::{get_process_server, ProcessPriority, ProcessGroup};

pub struct ProcessTest;

impl ProcessTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_process_creation(&mut self) -> bool {
        let process_server = get_process_server();
        let mut server = process_server.lock();
        
        // Test basic process creation
        match server.create_process("/bin/test_program", &[], &[]) {
            Ok(pid) => {
                crate::println!("[PROC] Created process with PID: {}", pid);
                
                // Check if process exists
                match server.get_process_info(pid) {
                    Ok(info) => {
                        crate::println!("[PROC] Process info: {} (state: {:?})", 
                            info.name, info.state);
                        
                        // Test process termination
                        match server.terminate_process(pid, 0) {
                            Ok(_) => {
                                crate::println!("[PROC] Process terminated successfully");
                                true
                            }
                            Err(e) => {
                                crate::println!("[PROC] Process termination failed: {}", e);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[PROC] Failed to get process info: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[PROC] Process creation failed: {}", e);
                false
            }
        }
    }
    
    fn test_process_listing(&mut self) -> bool {
        let process_server = get_process_server();
        let server = process_server.lock();
        
        // List all processes
        let processes = server.list_processes();
        crate::println!("[PROC] Found {} running processes", processes.len());
        
        for process in &processes {
            crate::println!("[PROC] PID {}: {} (state: {:?}, priority: {:?})", 
                process.pid, process.name, process.state, process.priority);
        }
        
        // Should have at least kernel processes
        !processes.is_empty()
    }
    
    fn test_process_priority(&mut self) -> bool {
        let process_server = get_process_server();
        let mut server = process_server.lock();
        
        // Create a process and test priority changes
        match server.create_process("/bin/priority_test", &[], &[]) {
            Ok(pid) => {
                crate::println!("[PROC] Created process for priority test: {}", pid);
                
                // Test setting priority
                match server.set_priority(pid, ProcessPriority::High) {
                    Ok(_) => {
                        crate::println!("[PROC] Set process priority to High");
                        
                        // Verify priority change
                        match server.get_process_info(pid) {
                            Ok(info) => {
                                let priority_correct = matches!(info.priority, ProcessPriority::High);
                                if priority_correct {
                                    crate::println!("[PROC] Priority change verified");
                                } else {
                                    crate::println!("[PROC] Priority change verification failed");
                                }
                                
                                // Clean up
                                server.terminate_process(pid, 0).ok();
                                priority_correct
                            }
                            Err(e) => {
                                crate::println!("[PROC] Failed to verify priority: {}", e);
                                server.terminate_process(pid, 0).ok();
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[PROC] Failed to set priority: {}", e);
                        server.terminate_process(pid, 0).ok();
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[PROC] Failed to create process for priority test: {}", e);
                false
            }
        }
    }
    
    fn test_process_groups(&mut self) -> bool {
        let process_server = get_process_server();
        let mut server = process_server.lock();
        
        // Create a process group
        match server.create_process_group(String::from("test_group")) {
            Ok(group_id) => {
                crate::println!("[PROC] Created process group: {}", group_id);
                
                // Create processes in the group
                let mut group_pids = Vec::new();
                for i in 0..3 {
                    match server.create_process(&format!("/bin/group_test_{}", i), &[], &[]) {
                        Ok(pid) => {
                            if let Err(e) = server.set_process_group(pid, group_id) {
                                crate::println!("[PROC] Failed to add process {} to group: {}", pid, e);
                                return false;
                            }
                            group_pids.push(pid);
                        }
                        Err(e) => {
                            crate::println!("[PROC] Failed to create group process {}: {}", i, e);
                            return false;
                        }
                    }
                }
                
                crate::println!("[PROC] Created {} processes in group", group_pids.len());
                
                // Test group operations
                match server.get_group_processes(group_id) {
                    Ok(group_processes) => {
                        crate::println!("[PROC] Group has {} processes", group_processes.len());
                        
                        // Clean up group
                        for pid in &group_pids {
                            server.terminate_process(*pid, 0).ok();
                        }
                        
                        group_processes.len() == group_pids.len()
                    }
                    Err(e) => {
                        crate::println!("[PROC] Failed to get group processes: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[PROC] Failed to create process group: {}", e);
                false
            }
        }
    }
    
    fn test_resource_limits(&mut self) -> bool {
        let process_server = get_process_server();
        let mut server = process_server.lock();
        
        // Create a process and test resource limits
        match server.create_process("/bin/resource_test", &[], &[]) {
            Ok(pid) => {
                crate::println!("[PROC] Created process for resource test: {}", pid);
                
                // Test setting memory limit
                match server.set_memory_limit(pid, 1024 * 1024) { // 1 MB limit
                    Ok(_) => {
                        crate::println!("[PROC] Set memory limit to 1 MB");
                        
                        // Test setting CPU limit
                        match server.set_cpu_limit(pid, 50) { // 50% CPU limit
                            Ok(_) => {
                                crate::println!("[PROC] Set CPU limit to 50%");
                                
                                // Get resource usage
                                match server.get_resource_usage(pid) {
                                    Ok(usage) => {
                                        crate::println!("[PROC] Memory usage: {} bytes", usage.memory_bytes);
                                        crate::println!("[PROC] CPU time: {} ms", usage.cpu_time_ms);
                                        
                                        // Clean up
                                        server.terminate_process(pid, 0).ok();
                                        true
                                    }
                                    Err(e) => {
                                        crate::println!("[PROC] Failed to get resource usage: {}", e);
                                        server.terminate_process(pid, 0).ok();
                                        false
                                    }
                                }
                            }
                            Err(e) => {
                                crate::println!("[PROC] Failed to set CPU limit: {}", e);
                                server.terminate_process(pid, 0).ok();
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[PROC] Failed to set memory limit: {}", e);
                        server.terminate_process(pid, 0).ok();
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[PROC] Failed to create process for resource test: {}", e);
                false
            }
        }
    }
}

impl TestProgram for ProcessTest {
    fn name(&self) -> &str {
        "process_test"
    }
    
    fn description(&self) -> &str {
        "Process server management and operations test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test process creation
        if self.test_process_creation() {
            messages.push("✓ Process creation");
        } else {
            messages.push("✗ Process creation");
            passed = false;
        }
        
        // Test process listing
        if self.test_process_listing() {
            messages.push("✓ Process listing");
        } else {
            messages.push("✗ Process listing");
            passed = false;
        }
        
        // Test process priority
        if self.test_process_priority() {
            messages.push("✓ Process priority");
        } else {
            messages.push("✗ Process priority");
            passed = false;
        }
        
        // Test process groups
        if self.test_process_groups() {
            messages.push("✓ Process groups");
        } else {
            messages.push("✗ Process groups");
            passed = false;
        }
        
        // Test resource limits
        if self.test_resource_limits() {
            messages.push("✓ Resource limits");
        } else {
            messages.push("✗ Resource limits");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}