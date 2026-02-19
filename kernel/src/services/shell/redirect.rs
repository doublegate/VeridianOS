//! I/O redirection support for the shell.
//!
//! Parses `>`, `>>`, `<`, `<<<`, `2>`, and `2>&1` from the command token
//! stream and applies them before command execution.

#![allow(dead_code)]

use alloc::{string::String, vec::Vec};

/// A single I/O redirection.
#[derive(Debug, Clone)]
pub enum Redirection {
    /// `> path` — redirect stdout to file (truncate).
    StdoutTo(String),
    /// `>> path` — redirect stdout to file (append).
    StdoutAppend(String),
    /// `< path` — redirect stdin from file.
    StdinFrom(String),
    /// `<<< word` — here-string: feed `word` as stdin.
    HereString(String),
    /// `2> path` — redirect stderr to file (truncate).
    StderrTo(String),
    /// `2>> path` — redirect stderr to file (append).
    StderrAppend(String),
    /// `2>&1` — redirect stderr to stdout.
    StderrToStdout,
}

/// Parse redirections from a list of tokens.
///
/// Returns the remaining command tokens (without redirection operators and
/// their arguments) and the list of parsed redirections.
pub fn parse_redirections(tokens: &[String]) -> (Vec<String>, Vec<Redirection>) {
    let mut command_tokens = Vec::new();
    let mut redirections = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let token = &tokens[i];

        match token.as_str() {
            ">" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::StdoutTo(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    // Missing filename — pass through as-is (will error later)
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            ">>" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::StdoutAppend(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            "<<<" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::HereString(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            "<" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::StdinFrom(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            "2>" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::StderrTo(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            "2>>" => {
                if i + 1 < tokens.len() {
                    redirections.push(Redirection::StderrAppend(tokens[i + 1].clone()));
                    i += 2;
                } else {
                    command_tokens.push(token.clone());
                    i += 1;
                }
            }
            "2>&1" => {
                redirections.push(Redirection::StderrToStdout);
                i += 1;
            }
            _ => {
                // Check for combined tokens like ">file" or "<<<word" (no space)
                if token.starts_with(">>") && token.len() > 2 {
                    redirections.push(Redirection::StdoutAppend(String::from(&token[2..])));
                } else if token.starts_with('>') && token.len() > 1 {
                    redirections.push(Redirection::StdoutTo(String::from(&token[1..])));
                } else if token.starts_with("<<<") && token.len() > 3 {
                    redirections.push(Redirection::HereString(String::from(&token[3..])));
                } else if token.starts_with('<') && token.len() > 1 {
                    redirections.push(Redirection::StdinFrom(String::from(&token[1..])));
                } else {
                    command_tokens.push(token.clone());
                }
                i += 1;
            }
        }
    }

    (command_tokens, redirections)
}

/// Apply output redirections: capture command output to the specified file.
///
/// For kernel-space commands (builtins), we capture the output by running the
/// command and writing results to VFS files. This is called after command
/// execution.
pub fn apply_stdout_redirect(
    output: &str,
    redirection: &Redirection,
) -> Result<(), crate::error::KernelError> {
    match redirection {
        Redirection::StdoutTo(path) => {
            crate::fs::write_file(path, output.as_bytes())?;
            Ok(())
        }
        Redirection::StdoutAppend(path) => {
            crate::fs::append_file(path, output.as_bytes())?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Read input from a file or here-string for stdin redirection.
pub fn read_stdin_redirect(
    redirection: &Redirection,
) -> Result<alloc::vec::Vec<u8>, crate::error::KernelError> {
    match redirection {
        Redirection::StdinFrom(path) => crate::fs::read_file(path),
        Redirection::HereString(text) => {
            // Here-strings provide the text as stdin with a trailing newline
            let mut data = text.as_bytes().to_vec();
            data.push(b'\n');
            Ok(data)
        }
        _ => Err(crate::error::KernelError::InvalidArgument {
            name: "redirection",
            value: "not a stdin redirect",
        }),
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;

    use super::*;

    fn s(val: &str) -> String {
        val.to_string()
    }

    #[test]
    fn test_no_redirections() {
        let tokens = [s("ls"), s("-la"), s("/tmp")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("ls"), s("-la"), s("/tmp")]);
        assert!(redir.is_empty());
    }

    #[test]
    fn test_stdout_redirect() {
        let tokens = [s("echo"), s("hello"), s(">"), s("/tmp/out.txt")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("echo"), s("hello")]);
        assert_eq!(redir.len(), 1);
        assert!(matches!(&redir[0], Redirection::StdoutTo(p) if p == "/tmp/out.txt"));
    }

    #[test]
    fn test_stdout_append() {
        let tokens = [s("echo"), s("hello"), s(">>"), s("/tmp/out.txt")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("echo"), s("hello")]);
        assert!(matches!(&redir[0], Redirection::StdoutAppend(p) if p == "/tmp/out.txt"));
    }

    #[test]
    fn test_stdin_redirect() {
        let tokens = [s("cat"), s("<"), s("/tmp/in.txt")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("cat")]);
        assert!(matches!(&redir[0], Redirection::StdinFrom(p) if p == "/tmp/in.txt"));
    }

    #[test]
    fn test_stderr_to_stdout() {
        let tokens = [s("cmd"), s("2>&1")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("cmd")]);
        assert!(matches!(&redir[0], Redirection::StderrToStdout));
    }

    #[test]
    fn test_combined_redirect_no_space() {
        let tokens = [s("echo"), s("hi"), s(">file.txt")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("echo"), s("hi")]);
        assert!(matches!(&redir[0], Redirection::StdoutTo(p) if p == "file.txt"));
    }

    #[test]
    fn test_multiple_redirections() {
        let tokens = [s("cmd"), s("<"), s("in"), s(">"), s("out"), s("2>&1")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("cmd")]);
        assert_eq!(redir.len(), 3);
    }

    #[test]
    fn test_here_string() {
        let tokens = [s("cat"), s("<<<"), s("hello world")];
        let (cmd, redir) = parse_redirections(&tokens);
        assert_eq!(cmd, vec![s("cat")]);
        assert_eq!(redir.len(), 1);
        assert!(matches!(&redir[0], Redirection::HereString(s) if s == "hello world"));
    }

    #[test]
    fn test_here_string_missing_word() {
        let tokens = [s("cat"), s("<<<")];
        let (cmd, _redir) = parse_redirections(&tokens);
        // Missing word — token passed through as-is
        assert_eq!(cmd, vec![s("cat"), s("<<<")]);
    }
}
