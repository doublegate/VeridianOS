//! Standard Library Foundation
//!
//! Core library functions and utilities for user-space applications.

use alloc::string::String;
use alloc::boxed::Box;
use alloc::format;
use core::ptr;
use core::slice;

/// Memory allocation functions
pub mod memory {
    use super::*;
    
    /// Allocate memory
    pub unsafe fn malloc(size: usize) -> *mut u8 {
        if size == 0 {
            return ptr::null_mut();
        }
        
        // For now, use kernel allocator (in real implementation, use user-space allocator)
        let layout = core::alloc::Layout::from_size_align(size, 8)
            .unwrap_or_else(|_| core::alloc::Layout::from_size_align(1, 1).unwrap());
        
        alloc::alloc::alloc(layout)
    }
    
    /// Allocate zeroed memory
    pub unsafe fn calloc(count: usize, size: usize) -> *mut u8 {
        let total_size = count.saturating_mul(size);
        if total_size == 0 {
            return ptr::null_mut();
        }
        
        let layout = core::alloc::Layout::from_size_align(total_size, 8)
            .unwrap_or_else(|_| core::alloc::Layout::from_size_align(1, 1).unwrap());
        
        alloc::alloc::alloc_zeroed(layout)
    }
    
    /// Reallocate memory
    pub unsafe fn realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            return malloc(new_size);
        }
        
        if new_size == 0 {
            free(ptr);
            return ptr::null_mut();
        }
        
        // For now, allocate new and copy (in real implementation, try to expand in place)
        let new_ptr = malloc(new_size);
        if !new_ptr.is_null() {
            // Copy old data (we don't know the old size, so this is a limitation)
            // In a real implementation, we'd track allocation sizes
            ptr::copy_nonoverlapping(ptr, new_ptr, new_size.min(1024)); // Assume max 1KB copy
            free(ptr);
        }
        
        new_ptr
    }
    
    /// Free memory
    pub unsafe fn free(ptr: *mut u8) {
        if !ptr.is_null() {
            // In real implementation, we'd track allocation size
            let layout = core::alloc::Layout::from_size_align(1, 1).unwrap();
            alloc::alloc::dealloc(ptr, layout);
        }
    }
    
    /// Copy memory
    pub unsafe fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        ptr::copy_nonoverlapping(src, dest, n);
        dest
    }
    
    /// Move memory (handles overlapping regions)
    pub unsafe fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        ptr::copy(src, dest, n);
        dest
    }
    
    /// Set memory to value
    pub unsafe fn memset(dest: *mut u8, value: i32, n: usize) -> *mut u8 {
        ptr::write_bytes(dest, value as u8, n);
        dest
    }
    
    /// Compare memory
    pub unsafe fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
        let slice1 = slice::from_raw_parts(s1, n);
        let slice2 = slice::from_raw_parts(s2, n);
        
        for i in 0..n {
            match slice1[i].cmp(&slice2[i]) {
                core::cmp::Ordering::Less => return -1,
                core::cmp::Ordering::Greater => return 1,
                core::cmp::Ordering::Equal => continue,
            }
        }
        
        0
    }
}

/// String manipulation functions
pub mod string {
    use super::*;
    
    /// Calculate string length
    pub unsafe fn strlen(s: *const u8) -> usize {
        let mut len = 0;
        while *s.add(len) != 0 {
            len += 1;
        }
        len
    }
    
    /// Copy string
    pub unsafe fn strcpy(dest: *mut u8, src: *const u8) -> *mut u8 {
        let mut i = 0;
        loop {
            let c = *src.add(i);
            *dest.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
        dest
    }
    
    /// Copy string with maximum length
    pub unsafe fn strncpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        let mut i = 0;
        while i < n {
            let c = *src.add(i);
            *dest.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
        
        // Pad with zeros if necessary
        while i < n {
            *dest.add(i) = 0;
            i += 1;
        }
        
        dest
    }
    
    /// Concatenate strings
    pub unsafe fn strcat(dest: *mut u8, src: *const u8) -> *mut u8 {
        let dest_len = strlen(dest);
        strcpy(dest.add(dest_len), src);
        dest
    }
    
    /// Compare strings
    pub unsafe fn strcmp(s1: *const u8, s2: *const u8) -> i32 {
        let mut i = 0;
        loop {
            let c1 = *s1.add(i);
            let c2 = *s2.add(i);
            
            if c1 != c2 {
                return if c1 < c2 { -1 } else { 1 };
            }
            
            if c1 == 0 {
                return 0;
            }
            
            i += 1;
        }
    }
    
    /// Compare strings with maximum length
    pub unsafe fn strncmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
        for i in 0..n {
            let c1 = *s1.add(i);
            let c2 = *s2.add(i);
            
            if c1 != c2 {
                return if c1 < c2 { -1 } else { 1 };
            }
            
            if c1 == 0 {
                return 0;
            }
        }
        
        0
    }
    
    /// Find character in string
    pub unsafe fn strchr(s: *const u8, c: i32) -> *const u8 {
        let target = c as u8;
        let mut i = 0;
        
        loop {
            let ch = *s.add(i);
            if ch == target {
                return s.add(i);
            }
            if ch == 0 {
                break;
            }
            i += 1;
        }
        
        ptr::null()
    }
    
    /// Find substring in string
    pub unsafe fn strstr(haystack: *const u8, needle: *const u8) -> *const u8 {
        let needle_len = strlen(needle);
        if needle_len == 0 {
            return haystack;
        }
        
        let haystack_len = strlen(haystack);
        if needle_len > haystack_len {
            return ptr::null();
        }
        
        for i in 0..=(haystack_len - needle_len) {
            if strncmp(haystack.add(i), needle, needle_len) == 0 {
                return haystack.add(i);
            }
        }
        
        ptr::null()
    }
}

/// File I/O functions
pub mod io {
    use super::*;
    use crate::fs::{get_vfs, OpenFlags};
    use alloc::sync::Arc;
    
    /// File handle
    pub struct File {
        node: Arc<dyn crate::fs::VfsNode>,
        position: usize,
        flags: OpenFlags,
    }
    
    /// Standard file descriptors
    pub const STDIN: i32 = 0;
    pub const STDOUT: i32 = 1;
    pub const STDERR: i32 = 2;
    
    /// File open modes
    pub const O_RDONLY: i32 = 0;
    pub const O_WRONLY: i32 = 1;
    pub const O_RDWR: i32 = 2;
    pub const O_CREAT: i32 = 0x40;
    pub const O_TRUNC: i32 = 0x200;
    pub const O_APPEND: i32 = 0x400;
    
    /// Seek modes
    pub const SEEK_SET: i32 = 0;
    pub const SEEK_CUR: i32 = 1;
    pub const SEEK_END: i32 = 2;
    
    /// Open a file
    pub fn open(path: &str, flags: i32) -> Result<*mut File, &'static str> {
        let open_flags = if flags & O_RDWR != 0 {
            OpenFlags::read_write()
        } else if flags & O_WRONLY != 0 {
            OpenFlags::write_only()
        } else {
            OpenFlags::read_only()
        };
        
        match get_vfs().read().open(path, open_flags) {
            Ok(node) => {
                let file = Box::new(File {
                    node,
                    position: 0,
                    flags: open_flags,
                });
                Ok(Box::into_raw(file))
            }
            Err(e) => Err(e),
        }
    }
    
    /// Close a file
    pub unsafe fn close(file: *mut File) -> i32 {
        if file.is_null() {
            return -1;
        }
        
        drop(Box::from_raw(file));
        0
    }
    
    /// Read from file
    pub unsafe fn read(file: *mut File, buf: *mut u8, count: usize) -> isize {
        if file.is_null() || buf.is_null() {
            return -1;
        }
        
        let file_ref = &mut *file;
        let buffer = slice::from_raw_parts_mut(buf, count);
        
        match file_ref.node.read(file_ref.position, buffer) {
            Ok(bytes_read) => {
                file_ref.position += bytes_read;
                bytes_read as isize
            }
            Err(_) => -1,
        }
    }
    
    /// Write to file
    pub unsafe fn write(file: *mut File, buf: *const u8, count: usize) -> isize {
        if file.is_null() || buf.is_null() {
            return -1;
        }
        
        let file_ref = &mut *file;
        let buffer = slice::from_raw_parts(buf, count);
        
        match file_ref.node.write(file_ref.position, buffer) {
            Ok(bytes_written) => {
                file_ref.position += bytes_written;
                bytes_written as isize
            }
            Err(_) => -1,
        }
    }
    
    /// Seek in file
    pub unsafe fn seek(file: *mut File, offset: isize, whence: i32) -> isize {
        if file.is_null() {
            return -1;
        }
        
        let file_ref = &mut *file;
        
        let new_position = match whence {
            SEEK_SET => {
                if offset < 0 {
                    return -1;
                }
                offset as usize
            }
            SEEK_CUR => {
                if offset < 0 && (-offset) as usize > file_ref.position {
                    return -1;
                }
                if offset >= 0 {
                    file_ref.position + offset as usize
                } else {
                    file_ref.position - (-offset) as usize
                }
            }
            SEEK_END => {
                // Get file size
                match file_ref.node.metadata() {
                    Ok(meta) => {
                        if offset < 0 && (-offset) as usize > meta.size {
                            return -1;
                        }
                        if offset >= 0 {
                            meta.size + offset as usize
                        } else {
                            meta.size - (-offset) as usize
                        }
                    }
                    Err(_) => return -1,
                }
            }
            _ => return -1,
        };
        
        file_ref.position = new_position;
        new_position as isize
    }
    
    /// Print to stdout
    pub fn printf(fmt: &str, args: &[&dyn core::fmt::Display]) -> i32 {
        // Simple printf implementation
        let mut output = String::new();
        let mut arg_index = 0;
        let mut chars = fmt.chars();
        
        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(next_ch) = chars.next() {
                    match next_ch {
                        's' | 'd' | 'x' | 'c' => {
                            if arg_index < args.len() {
                                output.push_str(&format!("{}", args[arg_index]));
                                arg_index += 1;
                            }
                        }
                        '%' => output.push('%'),
                        _ => {
                            output.push('%');
                            output.push(next_ch);
                        }
                    }
                } else {
                    output.push('%');
                }
            } else {
                output.push(ch);
            }
        }
        
        crate::print!("{}", output);
        output.len() as i32
    }
    
    /// Print to stdout with newline
    pub fn puts(s: &str) -> i32 {
        crate::println!("{}", s);
        (s.len() + 1) as i32
    }
}

/// Process management functions
pub mod process {
    
    
    
    /// Exit current process
    pub fn exit(status: i32) -> ! {
        crate::process::lifecycle::exit_process(status);
        // Process should never return after exit
        loop {
            #[cfg(target_arch = "x86_64")]
            unsafe { core::arch::asm!("hlt") }
            
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("wfi") }
            
            #[cfg(target_arch = "riscv64")]
            unsafe { core::arch::asm!("wfi") }
            
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64")))]
            core::hint::spin_loop();
        }
    }
    
    /// Get current process ID
    pub fn getpid() -> u32 {
        crate::process::get_current_process()
            .map(|p| p.pid.0 as u32)
            .unwrap_or(0) // Return 0 if no current process
    }
    
    /// Get parent process ID
    pub fn getppid() -> u32 {
        // TODO: Get actual parent PID
        1 // Return init for now
    }
    
    /// Fork process
    pub fn fork() -> i32 {
        // TODO: Implement actual fork
        -1 // Not implemented
    }
    
    /// Execute program
    pub fn exec(path: &str, args: &[&str]) -> i32 {
        match crate::userspace::load_user_program(path, args, &[]) {
            Ok(pid) => pid.0 as i32,
            Err(_) => -1,
        }
    }
    
    /// Wait for child process
    pub fn wait(status: *mut i32) -> i32 {
        // TODO: Implement actual wait
        -1 // Not implemented
    }
}

/// Math functions
pub mod math {
    /// Absolute value
    pub fn abs(x: i32) -> i32 {
        if x < 0 { -x } else { x }
    }
    
    /// Floating point absolute value
    pub fn fabs(x: f64) -> f64 {
        if x < 0.0 { -x } else { x }
    }
    
    /// Power function (integer)
    pub fn pow_int(base: i32, exp: u32) -> i32 {
        let mut result = 1;
        let mut b = base;
        let mut e = exp;
        
        while e > 0 {
            if e & 1 != 0 {
                result *= b;
            }
            b *= b;
            e >>= 1;
        }
        
        result
    }
    
    /// Square root (integer approximation)
    pub fn sqrt_int(x: u32) -> u32 {
        if x == 0 {
            return 0;
        }
        
        let mut guess = x / 2;
        let mut last_guess = x;
        
        while guess != last_guess {
            last_guess = guess;
            guess = (guess + x / guess) / 2;
        }
        
        guess
    }
    
    /// Minimum
    pub fn min(a: i32, b: i32) -> i32 {
        if a < b { a } else { b }
    }
    
    /// Maximum
    pub fn max(a: i32, b: i32) -> i32 {
        if a > b { a } else { b }
    }
}

/// Time functions
pub mod time {
    /// Get current time in seconds since epoch
    pub fn time() -> u64 {
        // TODO: Get actual system time
        0
    }
    
    /// Sleep for seconds
    pub fn sleep(seconds: u32) {
        crate::thread_api::sleep_ms(seconds as u64 * 1000);
    }
    
    /// Sleep for microseconds
    pub fn usleep(microseconds: u32) {
        crate::thread_api::sleep_ms(microseconds as u64 / 1000);
    }
}

/// Error handling
pub mod error {
    /// Error numbers (errno values)
    pub const EPERM: i32 = 1;      // Operation not permitted
    pub const ENOENT: i32 = 2;     // No such file or directory
    pub const ESRCH: i32 = 3;      // No such process
    pub const EINTR: i32 = 4;      // Interrupted system call
    pub const EIO: i32 = 5;        // I/O error
    pub const ENXIO: i32 = 6;      // No such device or address
    pub const E2BIG: i32 = 7;      // Argument list too long
    pub const ENOEXEC: i32 = 8;    // Exec format error
    pub const EBADF: i32 = 9;      // Bad file number
    pub const ECHILD: i32 = 10;    // No child processes
    pub const EAGAIN: i32 = 11;    // Try again
    pub const ENOMEM: i32 = 12;    // Out of memory
    pub const EACCES: i32 = 13;    // Permission denied
    pub const EFAULT: i32 = 14;    // Bad address
    pub const EBUSY: i32 = 16;     // Device or resource busy
    pub const EEXIST: i32 = 17;    // File exists
    pub const ENODEV: i32 = 19;    // No such device
    pub const ENOTDIR: i32 = 20;   // Not a directory
    pub const EISDIR: i32 = 21;    // Is a directory
    pub const EINVAL: i32 = 22;    // Invalid argument
    pub const EMFILE: i32 = 24;    // Too many open files
    pub const ENOSPC: i32 = 28;    // No space left on device
    pub const EROFS: i32 = 30;     // Read-only file system
    
    static mut ERRNO: i32 = 0;
    
    /// Get error number
    pub fn get_errno() -> i32 {
        unsafe { ERRNO }
    }
    
    /// Set error number
    pub fn set_errno(errno: i32) {
        unsafe { ERRNO = errno; }
    }
    
    /// Get error string
    pub fn strerror(errno: i32) -> &'static str {
        match errno {
            EPERM => "Operation not permitted",
            ENOENT => "No such file or directory",
            ESRCH => "No such process",
            EINTR => "Interrupted system call",
            EIO => "I/O error",
            ENXIO => "No such device or address",
            E2BIG => "Argument list too long",
            ENOEXEC => "Exec format error",
            EBADF => "Bad file number",
            ECHILD => "No child processes",
            EAGAIN => "Try again",
            ENOMEM => "Out of memory",
            EACCES => "Permission denied",
            EFAULT => "Bad address",
            EBUSY => "Device or resource busy",
            EEXIST => "File exists",
            ENODEV => "No such device",
            ENOTDIR => "Not a directory",
            EISDIR => "Is a directory",
            EINVAL => "Invalid argument",
            EMFILE => "Too many open files",
            ENOSPC => "No space left on device",
            EROFS => "Read-only file system",
            _ => "Unknown error",
        }
    }
}

/// Random number generation
pub mod random {
    static mut SEED: u32 = 1;
    
    /// Set random seed
    pub fn srand(seed: u32) {
        unsafe { SEED = seed; }
    }
    
    /// Generate random number
    pub fn rand() -> i32 {
        unsafe {
            SEED = SEED.wrapping_mul(1103515245).wrapping_add(12345);
            (SEED / 65536) as i32 % 32768
        }
    }
    
    /// Generate random number in range
    pub fn rand_range(min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        min + (rand() % (max - min))
    }
}

/// System information
pub mod system {
    
    use alloc::string::String;
    
    /// System information structure
    #[derive(Debug)]
    pub struct SystemInfo {
        pub os_name: &'static str,
        pub version: &'static str,
        pub architecture: &'static str,
        pub total_memory: u64,
        pub available_memory: u64,
        pub cpu_count: u32,
        pub uptime: u64,
    }
    
    /// Get system information
    pub fn get_system_info() -> SystemInfo {
        SystemInfo {
            os_name: "VeridianOS",
            version: "0.2.1",
            architecture: if cfg!(target_arch = "x86_64") {
                "x86_64"
            } else if cfg!(target_arch = "aarch64") {
                "aarch64"
            } else if cfg!(target_arch = "riscv64") {
                "riscv64"
            } else {
                "unknown"
            },
            total_memory: 1024 * 1024 * 1024, // 1 GB
            available_memory: 512 * 1024 * 1024, // 512 MB
            cpu_count: 1,
            uptime: 0,
        }
    }
    
    /// Get environment variable
    pub fn getenv(name: &str) -> Option<String> {
        // TODO: Access actual environment
        match name {
            "PATH" => Some(String::from("/bin:/usr/bin")),
            "HOME" => Some(String::from("/")),
            "USER" => Some(String::from("root")),
            _ => None,
        }
    }
    
    /// Set environment variable
    pub fn setenv(name: &str, value: &str) -> i32 {
        // TODO: Set actual environment variable
        crate::println!("setenv: {}={}", name, value);
        0
    }
}

/// Initialize standard library
pub fn init() {
    crate::println!("[STDLIB] Standard library foundation initialized");
}