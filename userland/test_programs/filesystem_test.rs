//! Filesystem Test Program
//!
//! Tests VFS operations including file creation, reading, writing, and directory operations.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use super::{TestProgram, TestResult};
use crate::services::vfs::{get_vfs, OpenFlags, FileType};

pub struct FilesystemTest;

impl FilesystemTest {
    pub fn new() -> Self {
        Self
    }
    
    fn test_file_operations(&mut self) -> bool {
        let vfs = get_vfs();
        let test_file = "/tmp/test_file.txt";
        let test_data = b"Hello, VeridianOS filesystem!";
        
        // Test file creation and writing
        match vfs.create(test_file, FileType::Regular) {
            Ok(_) => {
                crate::println!("[VFS] Created file: {}", test_file);
                
                // Open file for writing
                match vfs.open(test_file, OpenFlags::WRITE) {
                    Ok(fd) => {
                        // Write test data
                        match vfs.write(fd, test_data) {
                            Ok(bytes_written) => {
                                crate::println!("[VFS] Wrote {} bytes", bytes_written);
                                vfs.close(fd);
                                
                                // Test reading the file back
                                match vfs.open(test_file, OpenFlags::READ) {
                                    Ok(read_fd) => {
                                        let mut buffer = vec![0u8; test_data.len()];
                                        match vfs.read(read_fd, &mut buffer) {
                                            Ok(bytes_read) => {
                                                crate::println!("[VFS] Read {} bytes", bytes_read);
                                                vfs.close(read_fd);
                                                
                                                // Verify data integrity
                                                if buffer == test_data {
                                                    crate::println!("[VFS] Data integrity verified");
                                                    true
                                                } else {
                                                    crate::println!("[VFS] Data integrity check failed");
                                                    false
                                                }
                                            }
                                            Err(e) => {
                                                crate::println!("[VFS] Read failed: {}", e);
                                                vfs.close(read_fd);
                                                false
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        crate::println!("[VFS] Open for read failed: {}", e);
                                        false
                                    }
                                }
                            }
                            Err(e) => {
                                crate::println!("[VFS] Write failed: {}", e);
                                vfs.close(fd);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[VFS] Open for write failed: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[VFS] File creation failed: {}", e);
                false
            }
        }
    }
    
    fn test_directory_operations(&mut self) -> bool {
        let vfs = get_vfs();
        let test_dir = "/tmp/test_directory";
        
        // Test directory creation
        match vfs.create(test_dir, FileType::Directory) {
            Ok(_) => {
                crate::println!("[VFS] Created directory: {}", test_dir);
                
                // Test directory listing
                match vfs.read_dir(test_dir) {
                    Ok(entries) => {
                        crate::println!("[VFS] Directory has {} entries", entries.len());
                        
                        // Create a file in the directory
                        let nested_file = "/tmp/test_directory/nested_file.txt";
                        match vfs.create(nested_file, FileType::Regular) {
                            Ok(_) => {
                                crate::println!("[VFS] Created nested file: {}", nested_file);
                                
                                // List directory again
                                match vfs.read_dir(test_dir) {
                                    Ok(new_entries) => {
                                        crate::println!("[VFS] Directory now has {} entries", new_entries.len());
                                        new_entries.len() > entries.len()
                                    }
                                    Err(e) => {
                                        crate::println!("[VFS] Second directory read failed: {}", e);
                                        false
                                    }
                                }
                            }
                            Err(e) => {
                                crate::println!("[VFS] Nested file creation failed: {}", e);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[VFS] Directory read failed: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[VFS] Directory creation failed: {}", e);
                false
            }
        }
    }
    
    fn test_file_metadata(&mut self) -> bool {
        let vfs = get_vfs();
        let test_file = "/tmp/metadata_test.txt";
        
        // Create a file with some content
        match vfs.create(test_file, FileType::Regular) {
            Ok(_) => {
                match vfs.open(test_file, OpenFlags::WRITE) {
                    Ok(fd) => {
                        let data = b"Test data for metadata";
                        vfs.write(fd, data).unwrap();
                        vfs.close(fd);
                        
                        // Test stat operation
                        match vfs.stat(test_file) {
                            Ok(metadata) => {
                                crate::println!("[VFS] File size: {} bytes", metadata.size);
                                crate::println!("[VFS] File type: {:?}", metadata.file_type);
                                crate::println!("[VFS] Modified time: {}", metadata.modified_time);
                                
                                metadata.size == data.len() as u64 && metadata.file_type == FileType::Regular
                            }
                            Err(e) => {
                                crate::println!("[VFS] Stat failed: {}", e);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        crate::println!("[VFS] Failed to open for metadata test: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                crate::println!("[VFS] Failed to create file for metadata test: {}", e);
                false
            }
        }
    }
}

impl TestProgram for FilesystemTest {
    fn name(&self) -> &str {
        "filesystem_test"
    }
    
    fn description(&self) -> &str {
        "VFS file and directory operations test"
    }
    
    fn run(&mut self) -> TestResult {
        let mut passed = true;
        let mut messages = Vec::new();
        
        // Test file operations
        if self.test_file_operations() {
            messages.push("✓ File operations");
        } else {
            messages.push("✗ File operations");
            passed = false;
        }
        
        // Test directory operations
        if self.test_directory_operations() {
            messages.push("✓ Directory operations");
        } else {
            messages.push("✗ Directory operations");
            passed = false;
        }
        
        // Test file metadata
        if self.test_file_metadata() {
            messages.push("✓ File metadata");
        } else {
            messages.push("✗ File metadata");
            passed = false;
        }
        
        TestResult {
            name: self.name().to_string(),
            passed,
            message: messages.join(", "),
        }
    }
}