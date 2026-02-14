//! Standard Library Foundation
//!
//! Core library functions and utilities for user-space applications.

use alloc::{boxed::Box, format, string::String};
use core::{ptr, slice};

/// Memory allocation functions
pub mod memory {
    use super::*;

    /// Allocate memory
    ///
    /// # Safety
    /// Caller must eventually call `free` on the returned pointer (if non-null)
    /// with the same allocator.  The returned memory is uninitialized.
    pub unsafe fn malloc(size: usize) -> *mut u8 {
        if size == 0 {
            return ptr::null_mut();
        }

        // For now, use kernel allocator (in real implementation, use user-space
        // allocator)
        // Layout::from_size_align(1, 1) is infallible (size=1, align=1 is always valid)
        let layout = core::alloc::Layout::from_size_align(size, 8)
            .unwrap_or_else(|_| core::alloc::Layout::from_size_align(1, 1).expect("1-byte layout"));

        alloc::alloc::alloc(layout)
    }

    /// Allocate zeroed memory
    ///
    /// # Safety
    /// Caller must eventually call `free` on the returned pointer (if
    /// non-null). `count * size` must not overflow (saturating_mul is used
    /// as a safeguard).
    pub unsafe fn calloc(count: usize, size: usize) -> *mut u8 {
        let total_size = count.saturating_mul(size);
        if total_size == 0 {
            return ptr::null_mut();
        }

        // Layout::from_size_align(1, 1) is infallible (size=1, align=1 is always valid)
        let layout = core::alloc::Layout::from_size_align(total_size, 8)
            .unwrap_or_else(|_| core::alloc::Layout::from_size_align(1, 1).expect("1-byte layout"));

        alloc::alloc::alloc_zeroed(layout)
    }

    /// Reallocate memory
    ///
    /// # Safety
    /// `ptr` must be null or a pointer previously returned by
    /// `malloc`/`calloc`. The old allocation is freed; do not use `ptr`
    /// after this call. WARNING: current implementation assumes max 1KB old
    /// allocation size.
    pub unsafe fn realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            return malloc(new_size);
        }

        if new_size == 0 {
            free(ptr);
            return ptr::null_mut();
        }

        // For now, allocate new and copy (in real implementation, try to expand in
        // place)
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
    ///
    /// # Safety
    /// `ptr` must be null or a pointer previously returned by `malloc`/`calloc`
    /// that has not already been freed.  Double-free is undefined behavior.
    pub unsafe fn free(ptr: *mut u8) {
        if !ptr.is_null() {
            // In real implementation, we'd track allocation size
            // Layout(1, 1) is always valid: size > 0, align is power of 2
            let layout = core::alloc::Layout::from_size_align(1, 1).expect("1-byte layout");
            alloc::alloc::dealloc(ptr, layout);
        }
    }

    /// Copy memory
    ///
    /// # Safety
    /// `dest` and `src` must be valid for `n` bytes.  The regions must not
    /// overlap; use `memmove` for overlapping copies.
    pub unsafe fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        ptr::copy_nonoverlapping(src, dest, n);
        dest
    }

    /// Move memory (handles overlapping regions)
    ///
    /// # Safety
    /// `dest` and `src` must be valid for `n` bytes.  Regions may overlap.
    pub unsafe fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
        ptr::copy(src, dest, n);
        dest
    }

    /// Set memory to value
    ///
    /// # Safety
    /// `dest` must be valid for writing `n` bytes.
    pub unsafe fn memset(dest: *mut u8, value: i32, n: usize) -> *mut u8 {
        ptr::write_bytes(dest, value as u8, n);
        dest
    }

    /// Compare memory
    ///
    /// # Safety
    /// Both `s1` and `s2` must be valid for reading `n` bytes.
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
    ///
    /// # Safety
    /// `s` must point to a valid, NUL-terminated C string.
    pub unsafe fn strlen(s: *const u8) -> usize {
        let mut len = 0;
        while *s.add(len) != 0 {
            len += 1;
        }
        len
    }

    /// Copy string
    ///
    /// # Safety
    /// `src` must be NUL-terminated.  `dest` must have room for the
    /// entire source string including its NUL terminator.
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
    ///
    /// # Safety
    /// `dest` must be valid for writing `n` bytes.  `src` must be valid
    /// for reading up to `n` bytes or until a NUL terminator is found.
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
    ///
    /// # Safety
    /// Both `dest` and `src` must be NUL-terminated.  `dest` must have
    /// room for its current content plus the entirety of `src` plus NUL.
    pub unsafe fn strcat(dest: *mut u8, src: *const u8) -> *mut u8 {
        let dest_len = strlen(dest);
        strcpy(dest.add(dest_len), src);
        dest
    }

    /// Compare strings
    ///
    /// # Safety
    /// Both `s1` and `s2` must point to valid, NUL-terminated C strings.
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
    ///
    /// # Safety
    /// Both `s1` and `s2` must be valid for reading up to `n` bytes
    /// or until a NUL terminator is found.
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
    ///
    /// # Safety
    /// `s` must point to a valid, NUL-terminated C string.
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
    ///
    /// # Safety
    /// Both `haystack` and `needle` must point to valid, NUL-terminated C
    /// strings.
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
    use alloc::sync::Arc;

    use super::*;
    use crate::fs::{get_vfs, OpenFlags};

    /// File handle
    ///
    /// Fields are accessed through raw pointer dereference in the io module's
    /// read/write/seek/close functions, so the compiler cannot see direct
    /// usage.
    #[allow(dead_code)]
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
    pub fn open(path: &str, flags: i32) -> Result<*mut File, crate::error::KernelError> {
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
    ///
    /// # Safety
    /// `file` must be null or a pointer previously returned by `open`
    /// that has not already been closed.  After this call, `file` is
    /// dangling and must not be dereferenced.
    pub unsafe fn close(file: *mut File) -> i32 {
        if file.is_null() {
            return -1;
        }

        drop(Box::from_raw(file));
        0
    }

    /// Read from file
    ///
    /// # Safety
    /// `file` must be a valid, open `File` pointer from `open`.
    /// `buf` must be valid for writing `count` bytes.
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
    ///
    /// # Safety
    /// `file` must be a valid, open `File` pointer from `open`.
    /// `buf` must be valid for reading `count` bytes.
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
    ///
    /// # Safety
    /// `file` must be a valid, open `File` pointer from `open`.
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
        // Process should never return after exit.
        // Use the arch-specific idle function to avoid duplicating
        // inline asm that already exists in each arch module.
        loop {
            crate::arch::idle();
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
        crate::process::get_current_process()
            .and_then(|p| p.parent)
            .map(|ppid| ppid.0 as u32)
            .unwrap_or(1) // Return init (PID 1) if no parent or no current
                          // process
    }

    /// Fork process
    ///
    /// Creates a copy of the current process. Returns:
    /// - 0 to the child process
    /// - Child's PID to the parent process
    /// - -1 on error
    pub fn fork() -> i32 {
        match crate::process::fork_process() {
            Ok(child_pid) => {
                // fork_process() returns the child PID to the parent
                // The child process will have return value 0 set in its context
                child_pid.0 as i32
            }
            Err(_) => -1,
        }
    }

    /// Execute program
    pub fn exec(path: &str, args: &[&str]) -> i32 {
        match crate::userspace::load_user_program(path, args, &[]) {
            Ok(pid) => pid.0 as i32,
            Err(_) => -1,
        }
    }

    /// Wait for child process
    ///
    /// Waits for any child process to terminate and returns its PID.
    /// If `status` is non-null, the exit status is stored there.
    /// Returns -1 if there are no child processes or on error.
    ///
    /// # Safety
    /// `status` must be either null or point to valid, writable, aligned
    /// memory for a single `i32`.  If the pointer is invalid, writing the
    /// exit status will corrupt memory or fault.
    pub unsafe fn wait(status: *mut i32) -> i32 {
        match crate::process::wait_for_child(None) {
            Ok((pid, exit_status)) => {
                // SAFETY: Caller guarantees `status` is null or a valid
                // writable i32 pointer.  We check for null before writing.
                if !status.is_null() {
                    *status = exit_status;
                }
                pid.0 as i32
            }
            Err(_) => -1,
        }
    }
}

/// Math functions
pub mod math {
    /// Absolute value
    pub fn abs(x: i32) -> i32 {
        if x < 0 {
            -x
        } else {
            x
        }
    }

    /// Floating point absolute value
    pub fn fabs(x: f64) -> f64 {
        if x < 0.0 {
            -x
        } else {
            x
        }
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
        if a < b {
            a
        } else {
            b
        }
    }

    /// Maximum
    pub fn max(a: i32, b: i32) -> i32 {
        if a > b {
            a
        } else {
            b
        }
    }
}

/// Time functions
pub mod time {
    use core::sync::atomic::{AtomicU64, Ordering};

    /// Boot timestamp base (could be set from RTC on boot)
    static BOOT_TIME_SECONDS: AtomicU64 = AtomicU64::new(0);

    /// Set boot time from RTC (call during system initialization)
    pub fn set_boot_time(seconds_since_epoch: u64) {
        BOOT_TIME_SECONDS.store(seconds_since_epoch, Ordering::Release);
    }

    /// Get current time in seconds since epoch
    ///
    /// Returns the current system time based on:
    /// - Boot time (set from RTC during initialization)
    /// - Timer ticks elapsed since boot
    pub fn time() -> u64 {
        let ticks = crate::arch::timer::get_ticks();
        // Assuming ~1000 ticks per second (typical timer frequency)
        // Convert ticks to seconds since boot
        let seconds_since_boot = ticks / 1000;

        BOOT_TIME_SECONDS.load(Ordering::Acquire) + seconds_since_boot
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
    pub const EPERM: i32 = 1; // Operation not permitted
    pub const ENOENT: i32 = 2; // No such file or directory
    pub const ESRCH: i32 = 3; // No such process
    pub const EINTR: i32 = 4; // Interrupted system call
    pub const EIO: i32 = 5; // I/O error
    pub const ENXIO: i32 = 6; // No such device or address
    pub const E2BIG: i32 = 7; // Argument list too long
    pub const ENOEXEC: i32 = 8; // Exec format error
    pub const EBADF: i32 = 9; // Bad file number
    pub const ECHILD: i32 = 10; // No child processes
    pub const EAGAIN: i32 = 11; // Try again
    pub const ENOMEM: i32 = 12; // Out of memory
    pub const EACCES: i32 = 13; // Permission denied
    pub const EFAULT: i32 = 14; // Bad address
    pub const EBUSY: i32 = 16; // Device or resource busy
    pub const EEXIST: i32 = 17; // File exists
    pub const ENODEV: i32 = 19; // No such device
    pub const ENOTDIR: i32 = 20; // Not a directory
    pub const EISDIR: i32 = 21; // Is a directory
    pub const EINVAL: i32 = 22; // Invalid argument
    pub const EMFILE: i32 = 24; // Too many open files
    pub const ENOSPC: i32 = 28; // No space left on device
    pub const EROFS: i32 = 30; // Read-only file system

    use core::sync::atomic::{AtomicI32, Ordering};

    static ERRNO: AtomicI32 = AtomicI32::new(0);

    /// Get error number
    pub fn get_errno() -> i32 {
        ERRNO.load(Ordering::Relaxed)
    }

    /// Set error number
    pub fn set_errno(errno: i32) {
        ERRNO.store(errno, Ordering::Relaxed);
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
    use core::sync::atomic::{AtomicU32, Ordering};

    static SEED: AtomicU32 = AtomicU32::new(1);

    /// Set random seed
    pub fn srand(seed: u32) {
        SEED.store(seed, Ordering::Relaxed);
    }

    /// Generate random number
    pub fn rand() -> i32 {
        let new_seed = SEED
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
                Some(s.wrapping_mul(1103515245).wrapping_add(12345))
            })
            .unwrap_or(1);
        let result = new_seed.wrapping_mul(1103515245).wrapping_add(12345);
        (result / 65536) as i32 % 32768
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
    use alloc::{collections::BTreeMap, string::String};

    use spin::RwLock;

    /// Global environment variable storage
    static ENVIRONMENT: RwLock<Option<BTreeMap<String, String>>> = RwLock::new(None);

    /// Initialize environment with default values
    pub fn init_environment() {
        let mut env = BTreeMap::new();
        env.insert(
            String::from("PATH"),
            String::from("/bin:/usr/bin:/sbin:/usr/sbin"),
        );
        env.insert(String::from("HOME"), String::from("/root"));
        env.insert(String::from("USER"), String::from("root"));
        env.insert(String::from("SHELL"), String::from("/bin/vsh"));
        env.insert(String::from("TERM"), String::from("vt100"));
        env.insert(String::from("PWD"), String::from("/"));
        env.insert(String::from("LANG"), String::from("C"));
        *ENVIRONMENT.write() = Some(env);
    }

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
            total_memory: 1024 * 1024 * 1024,    // 1 GB
            available_memory: 512 * 1024 * 1024, // 512 MB
            cpu_count: 1,
            uptime: 0,
        }
    }

    /// Get environment variable
    ///
    /// Retrieves the value of the specified environment variable.
    /// Returns None if the variable is not set.
    pub fn getenv(name: &str) -> Option<String> {
        let env_guard = ENVIRONMENT.read();
        if let Some(ref env) = *env_guard {
            env.get(name).cloned()
        } else {
            // Fallback defaults if environment not initialized
            match name {
                "PATH" => Some(String::from("/bin:/usr/bin")),
                "HOME" => Some(String::from("/root")),
                "USER" => Some(String::from("root")),
                _ => None,
            }
        }
    }

    /// Set environment variable
    ///
    /// Sets the environment variable `name` to `value`.
    /// Returns 0 on success, -1 on error.
    pub fn setenv(name: &str, value: &str) -> i32 {
        let mut env_guard = ENVIRONMENT.write();
        if env_guard.is_none() {
            // Initialize environment if not already done
            *env_guard = Some(BTreeMap::new());
        }

        if let Some(ref mut env) = *env_guard {
            env.insert(String::from(name), String::from(value));
            0
        } else {
            -1
        }
    }

    /// Unset environment variable
    ///
    /// Removes the environment variable `name`.
    /// Returns 0 on success, -1 if variable doesn't exist.
    pub fn unsetenv(name: &str) -> i32 {
        let mut env_guard = ENVIRONMENT.write();
        if let Some(ref mut env) = *env_guard {
            if env.remove(name).is_some() {
                0
            } else {
                -1
            }
        } else {
            -1
        }
    }

    /// Get all environment variables
    ///
    /// Returns a copy of all environment variables as a BTreeMap.
    pub fn get_all_env() -> BTreeMap<String, String> {
        let env_guard = ENVIRONMENT.read();
        if let Some(ref env) = *env_guard {
            env.clone()
        } else {
            BTreeMap::new()
        }
    }
}

/// Initialize standard library
pub fn init() {
    // Initialize environment variables with defaults
    system::init_environment();
    crate::println!("[STDLIB] Standard library foundation initialized");
}
