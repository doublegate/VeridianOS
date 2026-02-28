//! Path and OS string types for VeridianOS.
//!
//! Provides `OsStr` / `OsString` (byte-string wrappers that do not require
//! valid UTF-8) and `Path` / `PathBuf` with `/`-separated path manipulation.
//!
//! These mirror the API shape of `std::ffi::OsStr` and `std::path::Path`
//! but are implemented without depending on the host `std`.

extern crate alloc;
use alloc::{string::String, vec::Vec};

// ============================================================================
// OsStr / OsString
// ============================================================================

/// Borrowed OS string -- a byte sequence that may or may not be valid UTF-8.
///
/// On VeridianOS (and all Unix-like systems) file names and environment
/// variables are arbitrary byte strings with no embedded NUL.
#[repr(transparent)]
pub struct OsStr {
    inner: [u8],
}

impl OsStr {
    /// Create an `&OsStr` from a byte slice.
    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        // SAFETY: OsStr is `repr(transparent)` over `[u8]`.
        unsafe { &*(bytes as *const [u8] as *const OsStr) }
    }

    /// Create an `&OsStr` from a `&str`.
    #[inline]
    pub fn new(s: &str) -> &Self {
        Self::from_bytes(s.as_bytes())
    }

    /// View the underlying bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }

    /// Attempt to convert to a `&str`.
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.inner).ok()
    }

    /// Convert to an owned `OsString`.
    pub fn to_os_string(&self) -> OsString {
        OsString {
            inner: self.inner.into(),
        }
    }

    /// Returns `true` if the string is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the byte length.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Convert to a `&Path`.
    #[inline]
    pub fn as_path(&self) -> &Path {
        Path::from_os_str(self)
    }
}

impl core::fmt::Debug for OsStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.to_str() {
            Some(s) => write!(f, "\"{}\"", s),
            None => write!(f, "OsStr({:?})", &self.inner),
        }
    }
}

impl core::fmt::Display for OsStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.to_str() {
            Some(s) => f.write_str(s),
            None => write!(f, "<non-utf8>"),
        }
    }
}

impl PartialEq for OsStr {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for OsStr {}

impl PartialEq<str> for OsStr {
    fn eq(&self, other: &str) -> bool {
        &self.inner == other.as_bytes()
    }
}

/// Owned OS string.
#[derive(Clone, PartialEq, Eq)]
pub struct OsString {
    inner: Vec<u8>,
}

impl OsString {
    /// Create an empty `OsString`.
    pub fn new() -> Self {
        OsString { inner: Vec::new() }
    }

    /// Create from a byte vector.
    pub fn from_vec(v: Vec<u8>) -> Self {
        OsString { inner: v }
    }

    /// Create from a `&str`.
    pub fn from_str(s: &str) -> Self {
        OsString {
            inner: s.as_bytes().into(),
        }
    }

    /// Borrow as `&OsStr`.
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        OsStr::from_bytes(&self.inner)
    }

    /// Push a `&OsStr` onto the end.
    pub fn push(&mut self, s: &OsStr) {
        self.inner.extend_from_slice(&s.inner);
    }

    /// Consume and return the underlying byte vector.
    pub fn into_vec(self) -> Vec<u8> {
        self.inner
    }

    /// Consume and attempt conversion to `String`.
    pub fn into_string(self) -> Result<String, OsString> {
        match String::from_utf8(self.inner) {
            Ok(s) => Ok(s),
            Err(e) => Err(OsString {
                inner: e.into_bytes(),
            }),
        }
    }

    /// Returns `true` if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the byte length.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Clear the contents.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// View the underlying bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }
}

impl Default for OsString {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for OsString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_os_str().fmt(f)
    }
}

impl core::fmt::Display for OsString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_os_str().fmt(f)
    }
}

impl core::ops::Deref for OsString {
    type Target = OsStr;
    #[inline]
    fn deref(&self) -> &OsStr {
        self.as_os_str()
    }
}

impl From<&str> for OsString {
    fn from(s: &str) -> Self {
        OsString::from_str(s)
    }
}

impl From<String> for OsString {
    fn from(s: String) -> Self {
        OsString {
            inner: s.into_bytes(),
        }
    }
}

// ============================================================================
// Path / PathBuf
// ============================================================================

/// The primary separator for VeridianOS paths.
pub const SEPARATOR: u8 = b'/';

/// Borrowed path reference.
///
/// Like `std::path::Path`, this is an unsized type that is always used
/// behind a reference.  Internally it is a byte string.
#[repr(transparent)]
pub struct Path {
    inner: OsStr,
}

impl Path {
    /// Create a `&Path` from an `&OsStr`.
    #[inline]
    pub fn from_os_str(s: &OsStr) -> &Self {
        // SAFETY: Path is repr(transparent) over OsStr.
        unsafe { &*(s as *const OsStr as *const Path) }
    }

    /// Create a `&Path` from a string slice.
    #[inline]
    pub fn new(s: &str) -> &Self {
        Self::from_os_str(OsStr::new(s))
    }

    /// Create a `&Path` from a byte slice.
    #[inline]
    pub fn from_bytes(b: &[u8]) -> &Self {
        Self::from_os_str(OsStr::from_bytes(b))
    }

    /// View the path as an `&OsStr`.
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        &self.inner
    }

    /// View the path as bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Attempt to view the path as a `&str`.
    #[inline]
    pub fn to_str(&self) -> Option<&str> {
        self.inner.to_str()
    }

    /// Convert to an owned `PathBuf`.
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf {
            inner: self.inner.to_os_string(),
        }
    }

    /// Is this an absolute path (starts with `/`)?
    #[inline]
    pub fn is_absolute(&self) -> bool {
        self.as_bytes().first() == Some(&SEPARATOR)
    }

    /// Is this a relative path?
    #[inline]
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Get the parent directory, or `None` if this is a root or prefix.
    pub fn parent(&self) -> Option<&Path> {
        let bytes = self.as_bytes();
        if bytes.is_empty() {
            return None;
        }
        // Strip trailing slashes.
        let trimmed = strip_trailing_sep(bytes);
        if trimmed.is_empty() {
            // Path was all slashes -- root has no parent.
            return None;
        }
        // Find the last separator.
        match memrchr(SEPARATOR, trimmed) {
            Some(pos) => {
                if pos == 0 {
                    // Parent is root "/".
                    Some(Path::from_bytes(&bytes[..1]))
                } else {
                    Some(Path::from_bytes(&trimmed[..pos]))
                }
            }
            None => {
                // No separator -- parent is empty (current directory).
                Some(Path::from_bytes(b""))
            }
        }
    }

    /// Get the final component of the path (file or directory name).
    pub fn file_name(&self) -> Option<&OsStr> {
        let bytes = self.as_bytes();
        let trimmed = strip_trailing_sep(bytes);
        if trimmed.is_empty() {
            return None;
        }
        match memrchr(SEPARATOR, trimmed) {
            Some(pos) => Some(OsStr::from_bytes(&trimmed[pos + 1..])),
            None => Some(OsStr::from_bytes(trimmed)),
        }
    }

    /// Get the file stem (file name without the final extension).
    pub fn file_stem(&self) -> Option<&OsStr> {
        let name = self.file_name()?;
        let name_bytes = name.as_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            return Some(name);
        }
        match memrchr(b'.', name_bytes) {
            Some(0) | None => Some(name),
            Some(pos) => Some(OsStr::from_bytes(&name_bytes[..pos])),
        }
    }

    /// Get the file extension (without the leading dot).
    pub fn extension(&self) -> Option<&OsStr> {
        let name = self.file_name()?;
        let name_bytes = name.as_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            return None;
        }
        match memrchr(b'.', name_bytes) {
            Some(0) | None => None,
            Some(pos) => Some(OsStr::from_bytes(&name_bytes[pos + 1..])),
        }
    }

    /// Produce an owned `PathBuf` by joining `self` with `path`.
    ///
    /// If `path` is absolute the result is just `path`.
    pub fn join(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        let mut buf = self.to_path_buf();
        buf.push(path);
        buf
    }

    /// Convenience: join with a string slice.
    pub fn join_str(&self, s: &str) -> PathBuf {
        self.join(Path::new(s))
    }

    /// Iterate over the components of the path.
    pub fn components(&self) -> Components<'_> {
        Components {
            bytes: self.as_bytes(),
            pos: 0,
            has_root: self.is_absolute(),
            emitted_root: false,
        }
    }

    /// Returns `true` if the path starts with the given prefix.
    pub fn starts_with(&self, prefix: &Path) -> bool {
        let mut self_comp = self.components();
        let mut prefix_comp = prefix.components();
        loop {
            match (prefix_comp.next(), self_comp.next()) {
                (None, _) => return true,
                (Some(_), None) => return false,
                (Some(a), Some(b)) => {
                    if a.as_bytes() != b.as_bytes() {
                        return false;
                    }
                }
            }
        }
    }

    /// Returns `true` if the path ends with the given suffix.
    pub fn ends_with(&self, suffix: &Path) -> bool {
        let self_comps: Vec<&OsStr> = self.components().collect();
        let suffix_comps: Vec<&OsStr> = suffix.components().collect();
        if suffix_comps.len() > self_comps.len() {
            return false;
        }
        let offset = self_comps.len() - suffix_comps.len();
        for (i, sc) in suffix_comps.iter().enumerate() {
            if self_comps[offset + i].as_bytes() != sc.as_bytes() {
                return false;
            }
        }
        true
    }

    /// Create a null-terminated copy of this path suitable for passing
    /// to syscalls.  Returns a `Vec<u8>` with a trailing NUL.
    pub fn to_cstring(&self) -> Vec<u8> {
        let bytes = self.as_bytes();
        let mut v = Vec::with_capacity(bytes.len() + 1);
        v.extend_from_slice(bytes);
        v.push(0);
        v
    }
}

impl core::fmt::Debug for Path {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.inner.fmt(f)
    }
}

impl core::fmt::Display for Path {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.inner.fmt(f)
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Path {}

// ============================================================================
// PathBuf
// ============================================================================

/// Owned, mutable path.
#[derive(Clone, PartialEq, Eq)]
pub struct PathBuf {
    inner: OsString,
}

impl PathBuf {
    /// Create an empty `PathBuf`.
    pub fn new() -> Self {
        PathBuf {
            inner: OsString::new(),
        }
    }

    /// Create from a string.
    pub fn from_str(s: &str) -> Self {
        PathBuf {
            inner: OsString::from_str(s),
        }
    }

    /// Create from a byte vector.
    pub fn from_vec(v: Vec<u8>) -> Self {
        PathBuf {
            inner: OsString::from_vec(v),
        }
    }

    /// Borrow as `&Path`.
    #[inline]
    pub fn as_path(&self) -> &Path {
        Path::from_os_str(self.inner.as_os_str())
    }

    /// Append a path component, inserting a separator if needed.
    pub fn push(&mut self, path: &Path) {
        if path.is_absolute() {
            self.inner = path.as_os_str().to_os_string();
            return;
        }
        let bytes = self.inner.as_bytes();
        if !bytes.is_empty() && bytes[bytes.len() - 1] != SEPARATOR {
            self.inner.push(OsStr::from_bytes(&[SEPARATOR]));
        }
        self.inner.push(path.as_os_str());
    }

    /// Append a string component.
    pub fn push_str(&mut self, s: &str) {
        self.push(Path::new(s));
    }

    /// Remove the last component.  Returns `false` if the path is empty.
    pub fn pop(&mut self) -> bool {
        match self.as_path().parent() {
            Some(p) => {
                self.inner = p.as_os_str().to_os_string();
                true
            }
            None => false,
        }
    }

    /// Set the file name (replace the last component).
    pub fn set_file_name(&mut self, name: &OsStr) {
        if self.as_path().file_name().is_some() {
            self.pop();
        }
        self.inner.push(OsStr::from_bytes(&[SEPARATOR]));
        self.inner.push(name);
    }

    /// Set the extension (replace or add).
    pub fn set_extension(&mut self, ext: &OsStr) -> bool {
        let stem = match self.as_path().file_stem() {
            Some(s) => s.to_os_string(),
            None => return false,
        };
        // Pop the file name, then reassemble with stem + ext.
        if self.as_path().file_name().is_some() {
            self.pop();
        }
        let mut name = stem;
        if !ext.is_empty() {
            name.push(OsStr::from_bytes(b"."));
            name.push(ext);
        }
        self.inner.push(OsStr::from_bytes(&[SEPARATOR]));
        self.inner.push(name.as_os_str());
        true
    }

    /// Consume and return the underlying `OsString`.
    pub fn into_os_string(self) -> OsString {
        self.inner
    }

    /// View as `&OsStr`.
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        self.inner.as_os_str()
    }

    /// View as bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Create a null-terminated copy for syscalls.
    pub fn to_cstring(&self) -> Vec<u8> {
        self.as_path().to_cstring()
    }
}

impl Default for PathBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_path().fmt(f)
    }
}

impl core::fmt::Display for PathBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_path().fmt(f)
    }
}

impl core::ops::Deref for PathBuf {
    type Target = Path;
    #[inline]
    fn deref(&self) -> &Path {
        self.as_path()
    }
}

impl From<&str> for PathBuf {
    fn from(s: &str) -> Self {
        PathBuf::from_str(s)
    }
}

impl From<OsString> for PathBuf {
    fn from(s: OsString) -> Self {
        PathBuf { inner: s }
    }
}

impl From<String> for PathBuf {
    fn from(s: String) -> Self {
        PathBuf {
            inner: OsString::from(s),
        }
    }
}

// ============================================================================
// Path component iterator
// ============================================================================

/// Iterator over the components of a path.
///
/// Yields `&OsStr` slices for each component.  The root `/` is yielded as
/// a single-byte slice containing `b"/"`.
pub struct Components<'a> {
    bytes: &'a [u8],
    pos: usize,
    has_root: bool,
    emitted_root: bool,
}

impl<'a> Iterator for Components<'a> {
    type Item = &'a OsStr;

    fn next(&mut self) -> Option<Self::Item> {
        // Emit root component first.
        if self.has_root && !self.emitted_root {
            self.emitted_root = true;
            // Skip leading slashes.
            while self.pos < self.bytes.len() && self.bytes[self.pos] == SEPARATOR {
                self.pos += 1;
            }
            return Some(OsStr::from_bytes(b"/"));
        }

        // Skip separators.
        while self.pos < self.bytes.len() && self.bytes[self.pos] == SEPARATOR {
            self.pos += 1;
        }

        if self.pos >= self.bytes.len() {
            return None;
        }

        // Find end of component.
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos] != SEPARATOR {
            self.pos += 1;
        }

        Some(OsStr::from_bytes(&self.bytes[start..self.pos]))
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Find the last occurrence of `needle` in `haystack`.
fn memrchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    let mut i = haystack.len();
    while i > 0 {
        i -= 1;
        if haystack[i] == needle {
            return Some(i);
        }
    }
    None
}

/// Strip trailing separator bytes from a path.
fn strip_trailing_sep(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 1 && bytes[end - 1] == SEPARATOR {
        end -= 1;
    }
    &bytes[..end]
}

/// Convert a null-terminated C string pointer to `&[u8]` (excluding the NUL).
///
/// # Safety
/// `ptr` must point to a valid null-terminated string.
pub unsafe fn cstr_to_bytes<'a>(ptr: *const u8) -> &'a [u8] {
    if ptr.is_null() {
        return &[];
    }
    let mut len = 0;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

/// Convert a null-terminated C string pointer to `&OsStr`.
///
/// # Safety
/// `ptr` must point to a valid null-terminated string.
pub unsafe fn cstr_to_os_str<'a>(ptr: *const u8) -> &'a OsStr {
    OsStr::from_bytes(unsafe { cstr_to_bytes(ptr) })
}
