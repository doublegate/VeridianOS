//! Standard Library Test Program
//!
//! Tests standard library functions including memory, string, I/O, and system functions.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ptr;
use super::{TestProgram, TestResult};
use crate::stdlib::*;

pub struct StdlibTest;

impl StdlibTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_memory_functions(&mut self) -> bool {
        // Test malloc and free
        let size = 1024;
        let ptr = unsafe { malloc(size) };
        
        if ptr.is_null() {
            crate::println!("[STDLIB] malloc failed");
            return false;
        }
        
        crate::println!("[STDLIB] malloc allocated {} bytes at {:p}", size, ptr);
        
        // Test memset
        unsafe { memset(ptr, 0xAB, size) };
        
        // Verify memset worked
        let byte_ptr = ptr as *const u8;
        let first_byte = unsafe { *byte_ptr };
        if first_byte != 0xAB {
            crate::println!("[STDLIB] memset verification failed");
            unsafe { free(ptr) };
            return false;
        }
        
        // Test calloc
        let calloc_ptr = unsafe { calloc(10, 64) };
        if calloc_ptr.is_null() {
            crate::println!("[STDLIB] calloc failed");
            unsafe { free(ptr) };
            return false;
        }
        
        // Verify calloc initialized to zero
        let zero_byte = unsafe { *(calloc_ptr as *const u8) };
        if zero_byte != 0 {
            crate::println!("[STDLIB] calloc zero initialization failed");
            unsafe { 
                free(ptr);
                free(calloc_ptr);
            };
            return false;
        }
        
        // Test realloc
        let new_size = 2048;
        let realloc_ptr = unsafe { realloc(ptr, new_size) };
        if realloc_ptr.is_null() {
            crate::println!("[STDLIB] realloc failed");
            unsafe { free(calloc_ptr) };
            return false;
        }
        
        crate::println!("[STDLIB] realloc resized to {} bytes at {:p}", new_size, realloc_ptr);
        
        // Clean up
        unsafe { 
            free(realloc_ptr);
            free(calloc_ptr);
        };
        
        crate::println!("[STDLIB] Memory functions test passed");
        true
    }
    
    fn test_string_functions(&mut self) -> bool {
        // Test strlen
        let test_str = b"Hello, VeridianOS!\0";
        let len = unsafe { strlen(test_str.as_ptr() as *const i8) };
        if len != 18 {
            crate::println!("[STDLIB] strlen failed: expected 18, got {}", len);
            return false;
        }
        
        // Test strcpy
        let dest = unsafe { malloc(32) as *mut i8 };
        if dest.is_null() {
            return false;
        }
        
        unsafe { strcpy(dest, test_str.as_ptr() as *const i8) };
        let copied_len = unsafe { strlen(dest) };
        if copied_len != len {
            crate::println!("[STDLIB] strcpy failed");
            unsafe { free(dest as *mut u8) };
            return false;
        }
        
        // Test strcmp
        let cmp_result = unsafe { strcmp(dest, test_str.as_ptr() as *const i8) };
        if cmp_result != 0 {
            crate::println!("[STDLIB] strcmp failed");
            unsafe { free(dest as *mut u8) };
            return false;
        }
        
        // Test strcat
        let append_str = b" Testing\0";
        unsafe { strcat(dest, append_str.as_ptr() as *const i8) };
        let final_len = unsafe { strlen(dest) };
        if final_len != len + 8 {
            crate::println!("[STDLIB] strcat failed");
            unsafe { free(dest as *mut u8) };
            return false;
        }
        
        unsafe { free(dest as *mut u8) };
        crate::println!("[STDLIB] String functions test passed");
        true
    }
    
    fn test_math_functions(&mut self) -> bool {
        // Test abs
        if abs(-42) != 42 {
            crate::println!("[STDLIB] abs failed");
            return false;
        }
        
        // Test min/max
        if min(10, 20) != 10 || max(10, 20) != 20 {
            crate::println!("[STDLIB] min/max failed");
            return false;
        }
        
        // Test sqrt (integer approximation)
        let sqrt_result = sqrt(16.0);
        if (sqrt_result - 4.0).abs() > 0.001 {
            crate::println!("[STDLIB] sqrt failed: expected ~4.0, got {}", sqrt_result);
            return false;
        }
        
        // Test pow
        let pow_result = pow(2.0, 3.0);
        if (pow_result - 8.0).abs() > 0.001 {
            crate::println!("[STDLIB] pow failed: expected ~8.0, got {}", pow_result);
            return false;
        }
        
        crate::println!("[STDLIB] Math functions test passed");
        true
    }
    
    fn test_file_operations(&mut self) -> bool {
        let filename = b"/tmp/stdlib_test.txt\0";
        let test_data = b"Standard library file test data\n";
        
        // Test fopen for writing
        let file = unsafe { fopen(filename.as_ptr() as *const i8, b"w\0".as_ptr() as *const i8) };
        if file.is_null() {
            crate::println!("[STDLIB] fopen for write failed");
            return false;
        }
        
        // Test fwrite
        let bytes_written = unsafe { 
            fwrite(test_data.as_ptr() as *const u8, 1, test_data.len(), file) 
        };
        if bytes_written != test_data.len() {
            crate::println!("[STDLIB] fwrite failed");
            unsafe { fclose(file) };
            return false;
        }
        
        // Close and reopen for reading
        unsafe { fclose(file) };
        
        let read_file = unsafe { fopen(filename.as_ptr() as *const i8, b"r\0".as_ptr() as *const i8) };
        if read_file.is_null() {
            crate::println!("[STDLIB] fopen for read failed");
            return false;
        }
        
        // Test fread
        let mut buffer = vec![0u8; test_data.len()];
        let bytes_read = unsafe { 
            fread(buffer.as_mut_ptr(), 1, buffer.len(), read_file) 
        };
        
        if bytes_read != test_data.len() {
            crate::println!("[STDLIB] fread failed");
            unsafe { fclose(read_file) };
            return false;
        }
        
        // Verify data integrity
        if buffer != test_data {
            crate::println!("[STDLIB] File data integrity check failed");
            unsafe { fclose(read_file) };
            return false;
        }
        
        unsafe { fclose(read_file) };
        crate::println!("[STDLIB] File operations test passed");
        true
    }
    
    fn test_system_functions(&mut self) -> bool {
        // Test getpid
        let pid = getpid();
        if pid == 0 {
            crate::println!("[STDLIB] getpid returned invalid PID");
            return false;
        }
        crate::println!("[STDLIB] Current PID: {}", pid);
        
        // Test time functions
        let current_time = time();
        crate::println!("[STDLIB] Current time: {}", current_time);
        
        // Test sleep (short sleep)
        crate::println!("[STDLIB] Testing sleep...");
        sleep(1); // Sleep for 1 second
        crate::println!("[STDLIB] Sleep completed");
        
        // Test getenv/setenv
        let test_var = b"STDLIB_TEST_VAR\0";
        let test_value = b"test_value\0";
        
        if unsafe { setenv(test_var.as_ptr() as *const i8, test_value.as_ptr() as *const i8) } != 0 {
            crate::println!("[STDLIB] setenv failed");
            return false;
        }
        
        let retrieved = unsafe { getenv(test_var.as_ptr() as *const i8) };
        if retrieved.is_null() {
            crate::println!("[STDLIB] getenv failed");
            return false;
        }
        
        let retrieved_len = unsafe { strlen(retrieved) };
        if retrieved_len != 10 { // length of "test_value"
            crate::println!("[STDLIB] Environment variable value incorrect");
            return false;
        }
        
        crate::println!("[STDLIB] System functions test passed");
        true
    }
}

impl TestProgram for StdlibTest {
    fn name(&self) -> &str {
        "stdlib_test"
    }
    
    fn description(&self) -> &str {
        "Standard library functions test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test memory functions
        if self.test_memory_functions() {
            messages.push("✓ Memory functions");
        } else {
            messages.push("✗ Memory functions");
            passed = false;
        }
        
        // Test string functions
        if self.test_string_functions() {
            messages.push("✓ String functions");
        } else {
            messages.push("✗ String functions");
            passed = false;
        }
        
        // Test math functions
        if self.test_math_functions() {
            messages.push("✓ Math functions");
        } else {
            messages.push("✗ Math functions");
            passed = false;
        }
        
        // Test file operations
        if self.test_file_operations() {
            messages.push("✓ File operations");
        } else {
            messages.push("✗ File operations");
            passed = false;
        }
        
        // Test system functions
        if self.test_system_functions() {
            messages.push("✓ System functions");
        } else {
            messages.push("✗ System functions");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}