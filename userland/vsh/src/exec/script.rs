//! Script file execution.
//!
//! Reads an entire file and evaluates it as shell commands.

extern crate alloc;

use alloc::{string::String, vec::Vec};

use crate::{eprintln, error::Result, syscall, Shell};

/// Read and execute a script file.
pub fn run_script_file(shell: &mut Shell, path: &str) -> Result<i32> {
    // Open the file
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path.as_bytes());
    path_buf.push(0);

    let fd = syscall::sys_open(path_buf.as_ptr(), syscall::O_RDONLY, 0);
    if fd < 0 {
        eprintln!("vsh: {}: No such file or directory", path);
        return Ok(127);
    }

    // Read the file content
    let mut content = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let n = syscall::sys_read(fd as i32, &mut buf);
        if n <= 0 {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }

    syscall::sys_close(fd as i32);

    // Convert to string
    let script = match core::str::from_utf8(&content) {
        Ok(s) => String::from(s),
        Err(_) => {
            eprintln!("vsh: {}: invalid UTF-8", path);
            return Ok(2);
        }
    };

    // Execute
    super::eval::eval_string(shell, &script)
}
