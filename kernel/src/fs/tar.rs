//! TAR archive parser and VFS loader
//!
//! Parses ustar-format TAR archives from an in-memory byte buffer and
//! creates corresponding files and directories in the VFS (RamFS).
//! Supports regular files (typeflag '0' or '\0'), directories
//! (typeflag '5'), and symlinks (typeflag '2', resolved as file copies).

use alloc::{string::String, vec::Vec};

use crate::{
    error::KernelError,
    fs::{get_vfs, Permissions},
};

/// TAR block size (every header and data region is a multiple of this).
const BLOCK_SIZE: usize = 512;

/// TAR header field offsets and sizes (ustar format).
mod field {
    /// File name (100 bytes, null-terminated ASCII).
    pub const NAME_OFF: usize = 0;
    pub const NAME_LEN: usize = 100;

    /// File mode in octal ASCII (8 bytes).
    pub const MODE_OFF: usize = 100;
    pub const MODE_LEN: usize = 8;

    /// File size in octal ASCII (12 bytes).
    pub const SIZE_OFF: usize = 124;
    pub const SIZE_LEN: usize = 12;

    /// Type flag (1 byte): '0' or '\0' = regular file, '2' = symlink, '5' =
    /// directory.
    pub const TYPE_OFF: usize = 156;

    /// Link name for symlinks/hard links (100 bytes, null-terminated ASCII).
    pub const LINK_OFF: usize = 157;
    pub const LINK_LEN: usize = 100;

    /// Name prefix for paths > 100 chars (155 bytes, null-terminated).
    pub const PREFIX_OFF: usize = 345;
    pub const PREFIX_LEN: usize = 155;

    /// Magic field ("ustar\0" for POSIX TAR).
    pub const MAGIC_OFF: usize = 257;
    pub const MAGIC_LEN: usize = 6;
}

/// Parse a null-terminated ASCII string from a fixed-size TAR field.
fn parse_str(buf: &[u8]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..end]).unwrap_or("")
}

/// Parse an octal ASCII number from a TAR field.
///
/// Handles both null/space-terminated octal strings and the GNU
/// base-256 extension (high bit set in the first byte).
fn parse_octal(buf: &[u8]) -> usize {
    // GNU base-256 extension: if the high bit of the first byte is set
    // the remaining bytes are a big-endian binary value.
    if !buf.is_empty() && (buf[0] & 0x80) != 0 {
        let mut val: usize = 0;
        for &b in &buf[1..] {
            val = val.wrapping_shl(8) | (b as usize);
        }
        return val;
    }

    let s = parse_str(buf).trim();
    if s.is_empty() {
        return 0;
    }
    usize::from_str_radix(s, 8).unwrap_or(0)
}

/// Check whether a 512-byte block is all zeros (end-of-archive marker).
fn is_zero_block(block: &[u8]) -> bool {
    block.iter().all(|&b| b == 0)
}

/// Ensure that every component of `path` exists as a directory in the VFS.
///
/// For example, given `/usr/local/bin`, this will create `/usr`, `/usr/local`,
/// and `/usr/local/bin` as directories if they don't already exist.
fn ensure_parent_dirs(path: &str) -> Result<(), KernelError> {
    let vfs = get_vfs().read();

    let mut accumulated = String::new();
    for component in path.split('/').filter(|c| !c.is_empty()) {
        accumulated.push('/');
        accumulated.push_str(component);

        // If this path component already exists we can skip it.
        if vfs.resolve_path(&accumulated).is_ok() {
            continue;
        }

        // Split into parent + name and create the directory.
        let (parent_path, dir_name) = if let Some(pos) = accumulated.rfind('/') {
            if pos == 0 {
                ("/", &accumulated[1..])
            } else {
                (&accumulated[..pos], &accumulated[pos + 1..])
            }
        } else {
            continue;
        };

        let parent_node = vfs.resolve_path(parent_path)?;
        // Ignore AlreadyExists -- another entry may have created it.
        let _ = parent_node.mkdir(dir_name, Permissions::default());
    }

    Ok(())
}

/// Load a TAR archive from a byte buffer into the VFS.
///
/// Iterates over the 512-byte headers in `data`, creating directories and
/// files in the RamFS-backed VFS. Returns the number of entries (files +
/// directories) successfully loaded.
///
/// # Arguments
/// * `data` - The raw bytes of a TAR archive (ustar format).
///
/// # Returns
/// * `Ok(count)` - Number of files/directories loaded.
/// * `Err(KernelError)` - On parse or VFS errors.
pub fn load_tar_to_vfs(data: &[u8]) -> Result<usize, KernelError> {
    #[allow(unused_imports)]
    use crate::println;

    if data.len() < BLOCK_SIZE {
        return Ok(0);
    }

    let mut offset: usize = 0;
    let mut count: usize = 0;
    // Deferred symlinks: (symlink_path, target_path, mode) for second-pass
    // resolution.
    let mut deferred_symlinks: Vec<(String, String, u32)> = Vec::new();

    while offset + BLOCK_SIZE <= data.len() {
        let header = &data[offset..offset + BLOCK_SIZE];

        // Two consecutive zero blocks mark end of archive.
        if is_zero_block(header) {
            if offset + 2 * BLOCK_SIZE <= data.len()
                && is_zero_block(&data[offset + BLOCK_SIZE..offset + 2 * BLOCK_SIZE])
            {
                break;
            }
            // Single zero block -- skip it.
            offset += BLOCK_SIZE;
            continue;
        }

        // Validate magic (optional -- some archives lack it).
        let magic = parse_str(&header[field::MAGIC_OFF..field::MAGIC_OFF + field::MAGIC_LEN]);
        if !magic.is_empty() && !magic.starts_with("ustar") {
            // Not a ustar header; skip this block.
            offset += BLOCK_SIZE;
            continue;
        }

        // Parse header fields.
        let prefix = parse_str(&header[field::PREFIX_OFF..field::PREFIX_OFF + field::PREFIX_LEN]);
        let name_raw = parse_str(&header[field::NAME_OFF..field::NAME_OFF + field::NAME_LEN]);
        let mode = parse_octal(&header[field::MODE_OFF..field::MODE_OFF + field::MODE_LEN]);
        let size = parse_octal(&header[field::SIZE_OFF..field::SIZE_OFF + field::SIZE_LEN]);
        let typeflag = header[field::TYPE_OFF];

        // Assemble full path (prefix + name).
        let full_name = if prefix.is_empty() {
            String::from(name_raw)
        } else {
            let mut s = String::from(prefix);
            s.push('/');
            s.push_str(name_raw);
            s
        };

        // Normalise: ensure the path starts with '/'.
        let path = if full_name.starts_with('/') {
            full_name.clone()
        } else {
            let mut s = String::from("/");
            s.push_str(&full_name);
            s
        };

        // Strip trailing '/' for directory paths.
        let path = if path.len() > 1 && path.ends_with('/') {
            String::from(&path[..path.len() - 1])
        } else {
            path
        };

        // Advance past the header block.
        offset += BLOCK_SIZE;

        match typeflag {
            b'5' => {
                // Directory entry.
                ensure_parent_dirs(&path)?;
                // The directory itself may already exist from ensure_parent_dirs.
                let vfs = get_vfs().read();
                if vfs.resolve_path(&path).is_err() {
                    let (parent_path, dir_name) = split_path(&path)?;
                    let parent = vfs.resolve_path(parent_path)?;
                    let _ = parent.mkdir(dir_name, Permissions::from_mode(mode as u32));
                }
                count += 1;
            }
            b'0' | b'\0' => {
                // Regular file entry.
                // Ensure parent directories exist.
                if let Some(pos) = path.rfind('/') {
                    if pos > 0 {
                        ensure_parent_dirs(&path[..pos])?;
                    }
                }

                // Extract file data.
                let file_data = if size > 0 && offset + size <= data.len() {
                    &data[offset..offset + size]
                } else {
                    &[] as &[u8]
                };

                // Create the file in the VFS.
                let vfs = get_vfs().read();
                let (parent_path, file_name) = split_path(&path)?;
                let parent = vfs.resolve_path(parent_path)?;

                // Remove existing file if present (overwrite semantics).
                let _ = parent.unlink(file_name);

                let node = parent.create(file_name, Permissions::from_mode(mode as u32))?;
                if !file_data.is_empty() {
                    node.write(0, file_data)?;
                }

                count += 1;

                // Advance past data blocks (rounded up to BLOCK_SIZE).
                let data_blocks = size.div_ceil(BLOCK_SIZE);
                offset += data_blocks * BLOCK_SIZE;
            }
            b'2' => {
                // Symbolic link entry -- resolve as a file copy of the target.
                // BusyBox uses symlinks (e.g. /bin/ash -> busybox) for its
                // multi-call binary. Since VeridianOS VFS has no native symlink
                // nodes, we copy the target file's contents to the new path.
                let link_target_raw =
                    parse_str(&header[field::LINK_OFF..field::LINK_OFF + field::LINK_LEN]);

                // Resolve the link target to an absolute path.
                let link_target = if link_target_raw.starts_with('/') {
                    String::from(link_target_raw)
                } else {
                    // Relative symlink: resolve relative to the symlink's parent dir.
                    if let Some(pos) = path.rfind('/') {
                        let parent_dir = if pos == 0 { "/" } else { &path[..pos] };
                        let mut abs = String::from(parent_dir);
                        abs.push('/');
                        abs.push_str(link_target_raw);
                        abs
                    } else {
                        let mut abs = String::from("/");
                        abs.push_str(link_target_raw);
                        abs
                    }
                };

                // Ensure parent directories for the symlink path exist.
                if let Some(pos) = path.rfind('/') {
                    if pos > 0 {
                        ensure_parent_dirs(&path[..pos])?;
                    }
                }

                // Read the target file's contents and copy them.
                let vfs = get_vfs().read();
                match vfs.resolve_path(&link_target) {
                    Ok(target_node) => {
                        // Read target file data (up to 4MB limit for safety).
                        let target_size = target_node.metadata().map(|m| m.size).unwrap_or(0);
                        if target_size > 0 && target_size <= 4 * 1024 * 1024 {
                            let mut buf = alloc::vec![0u8; target_size];
                            if let Ok(bytes_read) = target_node.read(0, &mut buf) {
                                let (parent_path, file_name) = split_path(&path)?;
                                let parent = vfs.resolve_path(parent_path)?;
                                let _ = parent.unlink(file_name);
                                let node = parent
                                    .create(file_name, Permissions::from_mode(mode as u32))?;
                                node.write(0, &buf[..bytes_read])?;
                                count += 1;
                            }
                        } else if target_size == 0 {
                            // Empty target -- create empty file.
                            let (parent_path, file_name) = split_path(&path)?;
                            let parent = vfs.resolve_path(parent_path)?;
                            let _ = parent.unlink(file_name);
                            let _ =
                                parent.create(file_name, Permissions::from_mode(mode as u32))?;
                            count += 1;
                        }
                    }
                    Err(_) => {
                        // Target doesn't exist yet -- defer to a second pass.
                        deferred_symlinks.push((path.clone(), link_target, mode as u32));
                    }
                }

                // Symlinks have no data blocks in the archive.
                let data_blocks = size.div_ceil(BLOCK_SIZE);
                offset += data_blocks * BLOCK_SIZE;
            }
            _ => {
                // Unsupported type (hard link, etc.) -- skip data.
                let data_blocks = size.div_ceil(BLOCK_SIZE);
                offset += data_blocks * BLOCK_SIZE;
            }
        }
    }

    // Second pass: resolve deferred symlinks (targets that appeared after the
    // link).
    for (sym_path, target_path, sym_mode) in &deferred_symlinks {
        if let Some(pos) = sym_path.rfind('/') {
            if pos > 0 {
                let _ = ensure_parent_dirs(&sym_path[..pos]);
            }
        }

        let vfs = get_vfs().read();
        if let Ok(target_node) = vfs.resolve_path(target_path) {
            let target_size = target_node.metadata().map(|m| m.size).unwrap_or(0);
            if target_size > 0 && target_size <= 4 * 1024 * 1024 {
                let mut buf = alloc::vec![0u8; target_size];
                if let Ok(bytes_read) = target_node.read(0, &mut buf) {
                    if let Ok((parent_path, file_name)) = split_path(sym_path) {
                        if let Ok(parent) = vfs.resolve_path(parent_path) {
                            let _ = parent.unlink(file_name);
                            if let Ok(node) =
                                parent.create(file_name, Permissions::from_mode(*sym_mode))
                            {
                                let _ = node.write(0, &buf[..bytes_read]);
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if !deferred_symlinks.is_empty() {
        let resolved = deferred_symlinks.len();
        println!("[TAR] Resolved {} deferred symlinks", resolved);
    }

    println!("[TAR] Loaded {} entries into VFS", count);
    Ok(count)
}

/// Split a path into (parent, name).
///
/// Returns `("/", "foo")` for `/foo`, or `("/a/b", "c")` for `/a/b/c`.
fn split_path(path: &str) -> Result<(&str, &str), KernelError> {
    if let Some(pos) = path.rfind('/') {
        let parent = if pos == 0 { "/" } else { &path[..pos] };
        let name = &path[pos + 1..];
        if name.is_empty() {
            return Err(KernelError::FsError(crate::error::FsError::InvalidPath));
        }
        Ok((parent, name))
    } else {
        Err(KernelError::FsError(crate::error::FsError::InvalidPath))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Helper: build a minimal ustar TAR header ---

    fn make_tar_header(name: &str, size: usize, typeflag: u8, mode: u32) -> [u8; 512] {
        let mut header = [0u8; 512];

        // Name field (offset 0, 100 bytes)
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(100);
        header[..len].copy_from_slice(&name_bytes[..len]);

        // Mode field (offset 100, 8 bytes) -- octal ASCII
        let mode_str = alloc::format!("{:07o}\0", mode);
        header[100..108].copy_from_slice(mode_str.as_bytes());

        // Size field (offset 124, 12 bytes) -- octal ASCII
        let size_str = alloc::format!("{:011o}\0", size);
        header[124..136].copy_from_slice(size_str.as_bytes());

        // Typeflag (offset 156, 1 byte)
        header[156] = typeflag;

        // Magic (offset 257, 6 bytes)
        header[257..263].copy_from_slice(b"ustar\0");

        // Version (offset 263, 2 bytes)
        header[263..265].copy_from_slice(b"00");

        // Compute checksum (offset 148, 8 bytes).
        // Per spec: treat checksum field as spaces during calculation.
        header[148..156].copy_from_slice(b"        ");
        let cksum: u32 = header.iter().map(|&b| b as u32).sum();
        let cksum_str = alloc::format!("{:06o}\0 ", cksum);
        header[148..156].copy_from_slice(&cksum_str.as_bytes()[..8]);

        header
    }

    /// Build a complete TAR archive in memory from a list of entries.
    fn build_tar(entries: &[(&str, usize, u8, u32, &[u8])]) -> alloc::vec::Vec<u8> {
        let mut archive = alloc::vec::Vec::new();
        for &(name, size, typeflag, mode, data) in entries {
            let header = make_tar_header(name, size, typeflag, mode);
            archive.extend_from_slice(&header);
            if !data.is_empty() {
                archive.extend_from_slice(data);
                // Pad to block boundary
                let remainder = data.len() % 512;
                if remainder != 0 {
                    let padding = 512 - remainder;
                    archive.extend(core::iter::repeat(0u8).take(padding));
                }
            }
        }
        // Two zero blocks to terminate
        archive.extend(core::iter::repeat(0u8).take(1024));
        archive
    }

    #[test]
    fn test_parse_octal_basic() {
        assert_eq!(parse_octal(b"0000755\0"), 0o755);
        assert_eq!(parse_octal(b"0000644\0"), 0o644);
        assert_eq!(parse_octal(b"00000000013\0"), 11); // 13 octal = 11 decimal
    }

    #[test]
    fn test_parse_octal_empty() {
        assert_eq!(parse_octal(b"\0\0\0\0"), 0);
        assert_eq!(parse_octal(b""), 0);
    }

    #[test]
    fn test_parse_str() {
        assert_eq!(parse_str(b"hello\0world"), "hello");
        assert_eq!(parse_str(b"hello"), "hello");
        assert_eq!(parse_str(b"\0"), "");
    }

    #[test]
    fn test_is_zero_block() {
        let zero = [0u8; 512];
        assert!(is_zero_block(&zero));

        let mut nonzero = [0u8; 512];
        nonzero[100] = 1;
        assert!(!is_zero_block(&nonzero));
    }

    #[test]
    fn test_split_path() {
        let (parent, name) = split_path("/bin/ls").unwrap();
        assert_eq!(parent, "/bin");
        assert_eq!(name, "ls");

        let (parent, name) = split_path("/hello").unwrap();
        assert_eq!(parent, "/");
        assert_eq!(name, "hello");
    }

    #[test]
    fn test_split_path_trailing_slash_fails() {
        assert!(split_path("/foo/").is_err());
    }

    #[test]
    fn test_make_tar_header_magic() {
        let header = make_tar_header("test.txt", 5, b'0', 0o644);
        let magic = parse_str(&header[257..263]);
        assert!(magic.starts_with("ustar"));
    }

    #[test]
    fn test_build_tar_not_empty() {
        let tar = build_tar(&[("file.txt", 5, b'0', 0o644, b"hello")]);
        // At least header (512) + data (512 padded) + terminator (1024)
        assert!(tar.len() >= 512 + 512 + 1024);
    }
}
