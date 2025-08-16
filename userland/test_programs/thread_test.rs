//! Thread Management Test Program
//!
//! Tests thread creation, synchronization, and thread-local storage.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::sync::Arc;
use spin::Mutex;
use super::{TestProgram, TestResult};
use crate::thread_api::{get_thread_manager, ThreadConfig, ThreadPriority};

pub struct ThreadTest {
    shared_counter: Arc<Mutex<u32>>,
}

impl ThreadTest {
    pub fn new() -> Self {
        Self {
            shared_counter: Arc::new(Mutex::new(0)),
        }
    }
    
    fn test_thread_creation(&mut self) -> bool {
        let thread_manager = get_thread_manager();
        
        // Test basic thread creation
        let config = ThreadConfig {
            name: Some("test_thread".to_string()),
            stack_size: 8192,
            priority: ThreadPriority::Normal,
            cpu_affinity: None,
        };
        
        match thread_manager.create_thread(config, || {
            crate::println!("[THREAD] Test thread running");
            42
        }) {
            Ok(handle) => {
                crate::println!("[THREAD] Created thread: {:?}", handle.get_id());
                
                // Test thread joining
                match handle.join() {
                    Ok(result) => {
                        crate::println!("[THREAD] Thread returned: {}", result);
                        result == 42
                    }
                    Err(e) => {
                        crate::println!("[THREAD] Join failed: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[THREAD] Thread creation failed: {}", e);
                false
            }
        }
    }
    
    fn test_thread_synchronization(&mut self) -> bool {
        let thread_manager = get_thread_manager();
        let counter = self.shared_counter.clone();
        let mut handles = Vec::new();
        
        // Create multiple threads that increment shared counter
        for i in 0..3 {
            let counter_clone = counter.clone();
            let config = ThreadConfig {
                name: Some(format!("sync_thread_{}", i)),
                stack_size: 8192,
                priority: ThreadPriority::Normal,
                cpu_affinity: None,
            };
            
            match thread_manager.create_thread(config, move || {
                for _ in 0..10 {
                    let mut count = counter_clone.lock();
                    *count += 1;
                    crate::println!("[THREAD] Thread {} incremented counter to {}", i, *count);
                }
                i
            }) {
                Ok(handle) => handles.push(handle),
                Err(e) => {
                    crate::println!("[THREAD] Failed to create sync thread {}: {}", i, e);
                    return false;
                }
            }
        }
        
        // Wait for all threads to complete
        for handle in handles {
            if let Err(e) = handle.join() {
                crate::println!("[THREAD] Sync thread join failed: {}", e);
                return false;
            }
        }
        
        let final_count = *counter.lock();
        crate::println!("[THREAD] Final counter value: {}", final_count);
        final_count == 30 // 3 threads * 10 increments each
    }
    
    fn test_thread_local_storage(&mut self) -> bool {
        let thread_manager = get_thread_manager();
        
        // Allocate TLS key
        let destructor = |_data: *mut u8| {
            crate::println!("[THREAD] TLS destructor called");
        };
        
        match thread_manager.tls_alloc(Some(destructor)) {
            Ok(key) => {
                crate::println!("[THREAD] Allocated TLS key: {:?}", key);
                
                // Test setting and getting TLS value
                let test_value = 0x12345678u64;
                if thread_manager.tls_set(key, &test_value as *const u64 as *mut u8).is_ok() {
                    match thread_manager.tls_get(key) {
                        Some(ptr) => {
                            let retrieved_value = unsafe { *(ptr as *const u64) };
                            crate::println!("[THREAD] TLS value: 0x{:x}", retrieved_value);
                            
                            // Free TLS key
                            thread_manager.tls_free(key);
                            retrieved_value == test_value
                        }
                        None => {
                            crate::println!("[THREAD] TLS get failed");
                            false
                        }
                    }
                } else {
                    crate::println!("[THREAD] TLS set failed");
                    false
                }
            }
            Err(e) => {
                crate::println!("[THREAD] TLS allocation failed: {}", e);
                false
            }
        }
    }
}

impl TestProgram for ThreadTest {
    fn name(&self) -> &str {
        "thread_test"
    }
    
    fn description(&self) -> &str {
        "Thread creation, synchronization, and TLS test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test thread creation
        if self.test_thread_creation() {
            messages.push("✓ Thread creation");
        } else {
            messages.push("✗ Thread creation");
            passed = false;
        }
        
        // Test thread synchronization
        if self.test_thread_synchronization() {
            messages.push("✓ Thread synchronization");
        } else {
            messages.push("✗ Thread synchronization");
            passed = false;
        }
        
        // Test thread-local storage
        if self.test_thread_local_storage() {
            messages.push("✓ Thread-local storage");
        } else {
            messages.push("✗ Thread-local storage");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}