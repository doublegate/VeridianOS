//! I/O redirection setup and teardown.
//!
//! Handles `<`, `>`, `>>`, `<<`, `<<<`, `<>`, `>&`, `<&`, `>|`, `&>`, `&>>`.
//! Uses dup2 to remap file descriptors and saves the original fds for
//! restoration after the command completes.

extern crate alloc;

use alloc::{string::String, vec::Vec};

use crate::{
    error::{Result, VshError},
    parser::ast::{Redirect, RedirectOp, RedirectTarget},
    syscall, Shell,
};

/// A saved file descriptor, to be restored after the command.
#[derive(Debug)]
pub struct SavedFd {
    /// The original fd number.
    pub original_fd: i32,
    /// The dup'd copy of the original fd (or -1 if it was not open).
    pub saved_copy: i32,
}

/// Set up redirections. Returns saved fds for later restoration.
pub fn setup_redirects(shell: &mut Shell, redirects: &[Redirect]) -> Result<Vec<SavedFd>> {
    let mut saved = Vec::new();

    for redir in redirects {
        apply_redirect(shell, redir, &mut saved)?;
    }

    Ok(saved)
}

/// Restore saved file descriptors.
pub fn restore_redirects(saved: &[SavedFd]) {
    for s in saved.iter().rev() {
        if s.saved_copy >= 0 {
            syscall::sys_dup2(s.saved_copy, s.original_fd);
            syscall::sys_close(s.saved_copy);
        } else {
            syscall::sys_close(s.original_fd);
        }
    }
}

fn apply_redirect(shell: &mut Shell, redir: &Redirect, saved: &mut Vec<SavedFd>) -> Result<()> {
    match redir.op {
        RedirectOp::Input => {
            // < file
            let fd = redir.fd.unwrap_or(0);
            let filename = expand_target(shell, &redir.target)?;
            let file_fd = open_file(&filename, syscall::O_RDONLY, 0)?;
            save_and_dup(fd, file_fd, saved);
            Ok(())
        }
        RedirectOp::Output => {
            // > file
            let fd = redir.fd.unwrap_or(1);
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_TRUNC;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(fd, file_fd, saved);
            Ok(())
        }
        RedirectOp::Append => {
            // >> file
            let fd = redir.fd.unwrap_or(1);
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_APPEND;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(fd, file_fd, saved);
            Ok(())
        }
        RedirectOp::Clobber => {
            // >| file (ignore noclobber)
            let fd = redir.fd.unwrap_or(1);
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_TRUNC;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(fd, file_fd, saved);
            Ok(())
        }
        RedirectOp::ReadWrite => {
            // <> file
            let fd = redir.fd.unwrap_or(0);
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_RDWR | syscall::O_CREAT;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(fd, file_fd, saved);
            Ok(())
        }
        RedirectOp::DupOutput => {
            // >&N or >&-
            let fd = redir.fd.unwrap_or(1);
            match &redir.target {
                RedirectTarget::Fd(target_fd) => {
                    save_and_dup(fd, *target_fd, saved);
                    Ok(())
                }
                RedirectTarget::Close => {
                    save_fd(fd, saved);
                    syscall::sys_close(fd);
                    Ok(())
                }
                _ => {
                    // >&file means redirect both stdout and stderr to file
                    let filename = expand_target(shell, &redir.target)?;
                    let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_TRUNC;
                    let file_fd = open_file(&filename, flags, 0o644)?;
                    save_and_dup(1, file_fd, saved);
                    // Also redirect stderr
                    save_and_dup(2, file_fd, saved);
                    Ok(())
                }
            }
        }
        RedirectOp::DupInput => {
            // <&N or <&-
            let fd = redir.fd.unwrap_or(0);
            match &redir.target {
                RedirectTarget::Fd(target_fd) => {
                    save_and_dup(fd, *target_fd, saved);
                    Ok(())
                }
                RedirectTarget::Close => {
                    save_fd(fd, saved);
                    syscall::sys_close(fd);
                    Ok(())
                }
                _ => Err(VshError::Redirection(String::from(
                    "invalid duplicate input target",
                ))),
            }
        }
        RedirectOp::AndOutput => {
            // &> file (redirect both stdout and stderr)
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_TRUNC;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(1, file_fd, saved);
            // dup the same fd for stderr
            let ret = syscall::sys_dup2(file_fd, 2);
            if ret >= 0 {
                save_fd(2, saved);
            }
            Ok(())
        }
        RedirectOp::AndAppend => {
            // &>> file (append both stdout and stderr)
            let filename = expand_target(shell, &redir.target)?;
            let flags = syscall::O_WRONLY | syscall::O_CREAT | syscall::O_APPEND;
            let file_fd = open_file(&filename, flags, 0o644)?;
            save_and_dup(1, file_fd, saved);
            let ret = syscall::sys_dup2(file_fd, 2);
            if ret >= 0 {
                save_fd(2, saved);
            }
            Ok(())
        }
        RedirectOp::HereDoc | RedirectOp::HereDocStrip => {
            // << DELIM or <<- DELIM
            let fd = redir.fd.unwrap_or(0);
            let body = match &redir.target {
                RedirectTarget::HereDocBody(s) => s.clone(),
                _ => String::new(),
            };

            // Create a pipe and write the heredoc body to it
            let mut pipefd = [0i32; 2];
            let ret = syscall::sys_pipe(&mut pipefd);
            if ret < 0 {
                return Err(VshError::PipeFailed);
            }

            // Write body to write end
            syscall::sys_write(pipefd[1], body.as_bytes());
            syscall::sys_close(pipefd[1]);

            // Redirect read end to stdin (or specified fd)
            save_and_dup(fd, pipefd[0], saved);
            syscall::sys_close(pipefd[0]);

            Ok(())
        }
        RedirectOp::HereString => {
            // <<< word
            let fd = redir.fd.unwrap_or(0);
            let content = match &redir.target {
                RedirectTarget::HereString(w) => {
                    let expanded = super::expand_word(shell, w);
                    let mut s = expanded.join(" ");
                    s.push('\n');
                    s
                }
                _ => String::from("\n"),
            };

            let mut pipefd = [0i32; 2];
            let ret = syscall::sys_pipe(&mut pipefd);
            if ret < 0 {
                return Err(VshError::PipeFailed);
            }

            syscall::sys_write(pipefd[1], content.as_bytes());
            syscall::sys_close(pipefd[1]);

            save_and_dup(fd, pipefd[0], saved);
            syscall::sys_close(pipefd[0]);

            Ok(())
        }
    }
}

/// Expand the target of a redirection to a filename string.
fn expand_target(shell: &mut Shell, target: &RedirectTarget) -> Result<String> {
    match target {
        RedirectTarget::File(word) => {
            let expanded = super::expand_word(shell, word);
            if expanded.is_empty() || expanded[0].is_empty() {
                return Err(VshError::Redirection(String::from("ambiguous redirect")));
            }
            if expanded.len() > 1 {
                return Err(VshError::Redirection(String::from("ambiguous redirect")));
            }
            Ok(expanded[0].clone())
        }
        _ => Err(VshError::Redirection(String::from(
            "unexpected target type",
        ))),
    }
}

/// Open a file and return its fd, or an error.
fn open_file(path: &str, flags: usize, mode: usize) -> Result<i32> {
    let mut buf = Vec::with_capacity(path.len() + 1);
    buf.extend_from_slice(path.as_bytes());
    buf.push(0);

    let fd = syscall::sys_open(buf.as_ptr(), flags, mode);
    if fd < 0 {
        return Err(VshError::Redirection(alloc::format!(
            "{}: cannot open ({})",
            path,
            fd
        )));
    }
    Ok(fd as i32)
}

/// Save the current fd and dup the new fd onto it.
fn save_and_dup(target_fd: i32, source_fd: i32, saved: &mut Vec<SavedFd>) {
    save_fd(target_fd, saved);
    if source_fd != target_fd {
        syscall::sys_dup2(source_fd, target_fd);
        // Don't close source_fd here -- caller manages it
    }
}

/// Save a copy of the given fd for later restoration.
fn save_fd(fd: i32, saved: &mut Vec<SavedFd>) {
    // Check if we already saved this fd
    for s in saved.iter() {
        if s.original_fd == fd {
            return;
        }
    }

    // Dup to a high fd number to save it
    // We use a simple approach: dup to fd+100 as a placeholder.
    // A real implementation would use fcntl(F_DUPFD_CLOEXEC).
    let saved_copy = syscall::sys_dup2(fd, fd + 100);
    saved.push(SavedFd {
        original_fd: fd,
        saved_copy: if saved_copy >= 0 { fd + 100 } else { -1 },
    });
}
