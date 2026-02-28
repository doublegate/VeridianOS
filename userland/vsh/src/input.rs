//! Raw terminal input handling.
//!
//! Provides byte-by-byte reading from fd 0 (stdin) using the read syscall.
//! Handles line editing basics: backspace, arrow keys, ctrl-C, ctrl-D.

use alloc::{string::String, vec::Vec};

use crate::{output::Writer, syscall};

/// Maximum length of a single input line.
const MAX_LINE_LEN: usize = 4096;

/// Read a single byte from stdin. Returns `Some(byte)` on success, `None`
/// on EOF or error.
pub fn read_byte() -> Option<u8> {
    let mut buf = [0u8; 1];
    let n = syscall::sys_read(0, &mut buf);
    if n > 0 {
        Some(buf[0])
    } else {
        None
    }
}

/// Read a complete line from stdin with basic line editing support.
///
/// Supports:
/// - Backspace / Delete (0x7F, 0x08): erase last character
/// - Ctrl-C (0x03): cancel current line, return empty
/// - Ctrl-D (0x04): EOF if line is empty
/// - Enter (0x0A, 0x0D): submit line
///
/// The returned string does NOT include the trailing newline.
/// Returns `None` on EOF (Ctrl-D on empty line).
pub fn read_line(prompt: &str) -> Option<String> {
    let out = Writer::stdout();
    out.write_str(prompt);

    let mut line = Vec::with_capacity(128);
    let mut cursor = 0usize;

    loop {
        let byte = match read_byte() {
            Some(b) => b,
            None => {
                // EOF
                if line.is_empty() {
                    return None;
                }
                break;
            }
        };

        match byte {
            // Ctrl-D: EOF on empty line, ignore otherwise
            0x04 => {
                if line.is_empty() {
                    out.write_str("\n");
                    return None;
                }
            }
            // Ctrl-C: cancel current line
            0x03 => {
                out.write_str("^C\n");
                return Some(String::new());
            }
            // Enter (LF or CR)
            0x0A | 0x0D => {
                out.write_str("\n");
                break;
            }
            // Backspace or DEL
            0x08 | 0x7F => {
                if cursor > 0 {
                    cursor -= 1;
                    line.remove(cursor);
                    // Move cursor back, write space, move back again
                    out.write_bytes(b"\x08 \x08");
                }
            }
            // Escape sequence (arrow keys, etc.)
            0x1B => {
                // Read the next two bytes for CSI sequences
                if let Some(b'[') = read_byte() {
                    match read_byte() {
                        Some(b'A') => {} // Up arrow -- TODO: history
                        Some(b'B') => {} // Down arrow -- TODO: history
                        Some(b'C') => {
                            // Right arrow
                            if cursor < line.len() {
                                cursor += 1;
                                out.write_bytes(b"\x1B[C");
                            }
                        }
                        Some(b'D') => {
                            // Left arrow
                            if cursor > 0 {
                                cursor -= 1;
                                out.write_bytes(b"\x1B[D");
                            }
                        }
                        _ => {} // Unknown escape sequence -- ignore
                    }
                }
            }
            // Ctrl-A: move to beginning of line
            0x01 => {
                while cursor > 0 {
                    out.write_bytes(b"\x1B[D");
                    cursor -= 1;
                }
            }
            // Ctrl-E: move to end of line
            0x05 => {
                while cursor < line.len() {
                    out.write_bytes(b"\x1B[C");
                    cursor += 1;
                }
            }
            // Ctrl-U: kill to beginning of line
            0x15 => {
                while cursor > 0 {
                    cursor -= 1;
                    line.remove(cursor);
                    out.write_bytes(b"\x08 \x08");
                }
            }
            // Ctrl-K: kill to end of line
            0x0B => {
                let remaining = line.len() - cursor;
                line.truncate(cursor);
                // Clear the rest of the line on screen
                for _ in 0..remaining {
                    out.write_bytes(b" ");
                }
                for _ in 0..remaining {
                    out.write_bytes(b"\x08");
                }
            }
            // Ctrl-W: kill previous word
            0x17 => {
                // Skip trailing whitespace
                while cursor > 0 && line[cursor - 1] == b' ' {
                    cursor -= 1;
                    line.remove(cursor);
                    out.write_bytes(b"\x08 \x08");
                }
                // Delete word
                while cursor > 0 && line[cursor - 1] != b' ' {
                    cursor -= 1;
                    line.remove(cursor);
                    out.write_bytes(b"\x08 \x08");
                }
            }
            // Ctrl-L: clear screen
            0x0C => {
                out.write_bytes(b"\x1B[2J\x1B[H");
                out.write_str(prompt);
                out.write_bytes(&line);
            }
            // Tab: auto-complete placeholder
            0x09 => {
                // TODO: tab completion
            }
            // Regular printable character
            b if (0x20..0x7F).contains(&b) => {
                if line.len() < MAX_LINE_LEN {
                    line.insert(cursor, b);
                    cursor += 1;
                    out.write_bytes(&[b]);
                }
            }
            // Other control characters: ignore
            _ => {}
        }
    }

    // Convert bytes to string (we only insert ASCII, so this is safe)
    Some(String::from_utf8(line).unwrap_or_default())
}

/// Read raw bytes from a file descriptor into a buffer.
/// Returns the number of bytes read, or 0 on EOF/error.
#[allow(dead_code)] // Public API for script/heredoc input
pub fn read_fd(fd: i32, buf: &mut [u8]) -> usize {
    let n = syscall::sys_read(fd, buf);
    if n > 0 {
        n as usize
    } else {
        0
    }
}
