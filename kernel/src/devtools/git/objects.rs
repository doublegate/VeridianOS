//! Git Object Model
//!
//! Implements the four Git object types (blob, tree, commit, tag),
//! SHA-1 object IDs, and object storage/retrieval.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// SHA-1 object identifier (20 bytes / 40 hex chars)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId([u8; 20]);

impl ObjectId {
    pub const ZERO: Self = Self([0u8; 20]);

    pub fn from_bytes(bytes: &[u8; 20]) -> Self {
        Self(*bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// Parse from 40-character hex string
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 40 {
            return None;
        }
        let mut bytes = [0u8; 20];
        let hex_bytes = hex.as_bytes();
        for (i, byte) in bytes.iter_mut().enumerate() {
            let hi = hex_digit(hex_bytes[i * 2])?;
            let lo = hex_digit(hex_bytes[i * 2 + 1])?;
            *byte = (hi << 4) | lo;
        }
        Some(Self(bytes))
    }

    /// Convert to 40-character hex string
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(40);
        for &b in &self.0 {
            s.push(hex_char((b >> 4) & 0x0F));
            s.push(hex_char(b & 0x0F));
        }
        s
    }

    /// Return first 7 chars of hex (short form)
    pub fn short(&self) -> String {
        self.to_hex()[..7].to_string()
    }
}

impl core::fmt::Display for ObjectId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_char(n: u8) -> char {
    if n < 10 {
        (b'0' + n) as char
    } else {
        (b'a' + n - 10) as char
    }
}

/// Git object types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl ObjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
            Self::Tag => "tag",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "blob" => Some(Self::Blob),
            "tree" => Some(Self::Tree),
            "commit" => Some(Self::Commit),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

impl core::fmt::Display for ObjectType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A raw Git object (type + data)
#[derive(Debug, Clone)]
pub struct GitObject {
    pub obj_type: ObjectType,
    pub data: Vec<u8>,
}

impl GitObject {
    pub fn new(obj_type: ObjectType, data: Vec<u8>) -> Self {
        Self { obj_type, data }
    }

    /// Serialize to Git's loose object format: "type size\0data"
    pub fn serialize(&self) -> Vec<u8> {
        let header = alloc::format!("{} {}\0", self.obj_type.as_str(), self.data.len());
        let mut buf = Vec::with_capacity(header.len() + self.data.len());
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Compute the SHA-1 object ID
    pub fn compute_id(&self) -> ObjectId {
        let serialized = self.serialize();
        let hash = sha1_hash(&serialized);
        ObjectId(hash)
    }

    /// Parse from serialized format
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        // Find null byte separating header from content
        let null_pos = data.iter().position(|&b| b == 0)?;
        let header = core::str::from_utf8(&data[..null_pos]).ok()?;

        let space_pos = header.find(' ')?;
        let type_str = &header[..space_pos];
        let size_str = &header[space_pos + 1..];

        let obj_type = ObjectType::parse(type_str)?;
        let size: usize = size_str.parse().ok()?;

        let content = &data[null_pos + 1..];
        if content.len() != size {
            return None;
        }

        Some(Self {
            obj_type,
            data: content.to_vec(),
        })
    }
}

/// Blob object (file content)
#[derive(Debug, Clone)]
pub struct Blob {
    pub data: Vec<u8>,
}

impl Blob {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn to_object(&self) -> GitObject {
        GitObject::new(ObjectType::Blob, self.data.clone())
    }
}

/// Tree entry (file mode + name + object ID)
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: u32,
    pub name: String,
    pub id: ObjectId,
}

impl TreeEntry {
    pub fn new(mode: u32, name: &str, id: ObjectId) -> Self {
        Self {
            mode,
            name: name.to_string(),
            id,
        }
    }

    /// Check if this is a directory entry
    pub fn is_tree(&self) -> bool {
        self.mode == 0o040000
    }

    /// Check if this is a regular file
    pub fn is_blob(&self) -> bool {
        self.mode == 0o100644 || self.mode == 0o100755
    }
}

/// Tree object (directory listing)
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add_entry(&mut self, entry: TreeEntry) {
        self.entries.push(entry);
    }

    /// Serialize to Git tree format: "mode name\0<20-byte-sha1>"
    pub fn to_object(&self) -> GitObject {
        let mut data = Vec::new();
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| {
            // Git sorts trees with trailing '/' for comparison
            let a_name = if a.is_tree() {
                alloc::format!("{}/", a.name)
            } else {
                a.name.clone()
            };
            let b_name = if b.is_tree() {
                alloc::format!("{}/", b.name)
            } else {
                b.name.clone()
            };
            a_name.cmp(&b_name)
        });

        for entry in &sorted {
            let mode_str = alloc::format!("{:o}", entry.mode);
            data.extend_from_slice(mode_str.as_bytes());
            data.push(b' ');
            data.extend_from_slice(entry.name.as_bytes());
            data.push(0);
            data.extend_from_slice(entry.id.as_bytes());
        }

        GitObject::new(ObjectType::Tree, data)
    }

    /// Parse from tree object data
    pub fn from_data(data: &[u8]) -> Option<Self> {
        let mut tree = Self::new();
        let mut pos = 0;

        while pos < data.len() {
            // Find space after mode
            let space = data[pos..].iter().position(|&b| b == b' ')?;
            let mode_str = core::str::from_utf8(&data[pos..pos + space]).ok()?;
            let mode = u32::from_str_radix(mode_str, 8).ok()?;
            pos += space + 1;

            // Find null after name
            let null = data[pos..].iter().position(|&b| b == 0)?;
            let name = core::str::from_utf8(&data[pos..pos + null]).ok()?;
            pos += null + 1;

            // Read 20-byte SHA-1
            if pos + 20 > data.len() {
                return None;
            }
            let mut id_bytes = [0u8; 20];
            id_bytes.copy_from_slice(&data[pos..pos + 20]);
            pos += 20;

            tree.add_entry(TreeEntry::new(mode, name, ObjectId::from_bytes(&id_bytes)));
        }

        Some(tree)
    }
}

/// Person info (author/committer)
#[derive(Debug, Clone)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub timestamp: u64,
    pub tz_offset: i16,
}

impl Person {
    pub fn new(name: &str, email: &str, timestamp: u64) -> Self {
        Self {
            name: name.to_string(),
            email: email.to_string(),
            timestamp,
            tz_offset: 0,
        }
    }

    pub fn format(&self) -> String {
        let sign = if self.tz_offset >= 0 { '+' } else { '-' };
        let offset_abs = self.tz_offset.unsigned_abs();
        let hours = offset_abs / 60;
        let mins = offset_abs % 60;
        alloc::format!(
            "{} <{}> {} {}{:02}{:02}",
            self.name,
            self.email,
            self.timestamp,
            sign,
            hours,
            mins
        )
    }
}

/// Commit object
#[derive(Debug, Clone)]
pub struct Commit {
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Person,
    pub committer: Person,
    pub message: String,
}

impl Commit {
    pub fn new(tree: ObjectId, author: Person, message: &str) -> Self {
        Self {
            tree,
            parents: Vec::new(),
            committer: author.clone(),
            author,
            message: message.to_string(),
        }
    }

    pub fn to_object(&self) -> GitObject {
        let mut data = String::new();
        data.push_str(&alloc::format!("tree {}\n", self.tree));
        for parent in &self.parents {
            data.push_str(&alloc::format!("parent {}\n", parent));
        }
        data.push_str(&alloc::format!("author {}\n", self.author.format()));
        data.push_str(&alloc::format!("committer {}\n", self.committer.format()));
        data.push('\n');
        data.push_str(&self.message);
        if !self.message.ends_with('\n') {
            data.push('\n');
        }

        GitObject::new(ObjectType::Commit, data.into_bytes())
    }

    /// Parse from commit object data
    pub fn from_data(data: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(data).ok()?;
        let mut tree = ObjectId::ZERO;
        let mut parents = Vec::new();
        let mut author = Person::new("", "", 0);
        let mut committer = Person::new("", "", 0);

        let mut lines = text.lines();
        let mut message_start = false;
        let mut message = String::new();

        for line in &mut lines {
            if message_start {
                if !message.is_empty() {
                    message.push('\n');
                }
                message.push_str(line);
                continue;
            }

            if line.is_empty() {
                message_start = true;
                continue;
            }

            if let Some(rest) = line.strip_prefix("tree ") {
                tree = ObjectId::from_hex(rest)?;
            } else if let Some(rest) = line.strip_prefix("parent ") {
                parents.push(ObjectId::from_hex(rest)?);
            } else if let Some(rest) = line.strip_prefix("author ") {
                author = parse_person(rest)?;
            } else if let Some(rest) = line.strip_prefix("committer ") {
                committer = parse_person(rest)?;
            }
        }

        // Collect remaining lines for message
        for line in lines {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str(line);
        }

        Some(Self {
            tree,
            parents,
            author,
            committer,
            message,
        })
    }
}

/// Tag object
#[derive(Debug, Clone)]
pub struct Tag {
    pub object: ObjectId,
    pub obj_type: ObjectType,
    pub tag_name: String,
    pub tagger: Person,
    pub message: String,
}

impl Tag {
    pub fn new(object: ObjectId, tag_name: &str, tagger: Person, message: &str) -> Self {
        Self {
            object,
            obj_type: ObjectType::Commit,
            tag_name: tag_name.to_string(),
            tagger,
            message: message.to_string(),
        }
    }

    pub fn to_object(&self) -> GitObject {
        let mut data = String::new();
        data.push_str(&alloc::format!("object {}\n", self.object));
        data.push_str(&alloc::format!("type {}\n", self.obj_type));
        data.push_str(&alloc::format!("tag {}\n", self.tag_name));
        data.push_str(&alloc::format!("tagger {}\n", self.tagger.format()));
        data.push('\n');
        data.push_str(&self.message);
        if !self.message.ends_with('\n') {
            data.push('\n');
        }

        GitObject::new(ObjectType::Tag, data.into_bytes())
    }
}

fn parse_person(s: &str) -> Option<Person> {
    // Format: "Name <email> timestamp +0000"
    let lt = s.find('<')?;
    let gt = s.find('>')?;
    let name = s[..lt].trim();
    let email = &s[lt + 1..gt];
    let rest = s[gt + 1..].trim();

    let parts: Vec<&str> = rest.split_whitespace().collect();
    let timestamp: u64 = parts.first()?.parse().ok()?;
    let tz_str = parts.get(1).unwrap_or(&"+0000");

    let tz_sign = if tz_str.starts_with('-') { -1i16 } else { 1 };
    let tz_val = &tz_str[1..];
    let tz_hours: i16 = tz_val.get(..2).unwrap_or("0").parse().unwrap_or(0);
    let tz_mins: i16 = tz_val.get(2..4).unwrap_or("0").parse().unwrap_or(0);
    let tz_offset = tz_sign * (tz_hours * 60 + tz_mins);

    Some(Person {
        name: name.to_string(),
        email: email.to_string(),
        timestamp,
        tz_offset,
    })
}

// ---------------------------------------------------------------------------
// SHA-1 Implementation
// ---------------------------------------------------------------------------

/// SHA-1 hash (for Git object IDs)
pub fn sha1_hash(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    // Pre-processing: add padding
    let msg_len = data.len();
    let bit_len = (msg_len as u64) * 8;

    let mut padded = Vec::with_capacity(msg_len + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);

    while (padded.len() % 64) != 56 {
        padded.push(0);
    }

    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) chunk
    for chunk in padded.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut result = [0u8; 20];
    result[0..4].copy_from_slice(&h0.to_be_bytes());
    result[4..8].copy_from_slice(&h1.to_be_bytes());
    result[8..12].copy_from_slice(&h2.to_be_bytes());
    result[12..16].copy_from_slice(&h3.to_be_bytes());
    result[16..20].copy_from_slice(&h4.to_be_bytes());
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha1_empty() {
        let hash = sha1_hash(b"");
        let expected = ObjectId::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
        assert_eq!(&hash, expected.as_bytes());
    }

    #[test]
    fn test_sha1_hello() {
        let hash = sha1_hash(b"hello");
        let oid = ObjectId(hash);
        assert_eq!(oid.to_hex(), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }

    #[test]
    fn test_object_id_from_hex() {
        let hex = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let oid = ObjectId::from_hex(hex).unwrap();
        assert_eq!(oid.to_hex(), hex);
    }

    #[test]
    fn test_object_id_short() {
        let hex = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        let oid = ObjectId::from_hex(hex).unwrap();
        assert_eq!(oid.short(), "da39a3e");
    }

    #[test]
    fn test_object_id_from_hex_invalid() {
        assert!(ObjectId::from_hex("short").is_none());
        assert!(ObjectId::from_hex("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_none());
    }

    #[test]
    fn test_object_type_from_str() {
        assert_eq!(ObjectType::parse("blob"), Some(ObjectType::Blob));
        assert_eq!(ObjectType::parse("tree"), Some(ObjectType::Tree));
        assert_eq!(ObjectType::parse("commit"), Some(ObjectType::Commit));
        assert_eq!(ObjectType::parse("tag"), Some(ObjectType::Tag));
        assert_eq!(ObjectType::parse("unknown"), None);
    }

    #[test]
    fn test_blob_object() {
        let blob = Blob::new(b"Hello, World!\n".to_vec());
        let obj = blob.to_object();
        assert_eq!(obj.obj_type, ObjectType::Blob);
        assert_eq!(&obj.data, b"Hello, World!\n");
    }

    #[test]
    fn test_git_object_serialize_deserialize() {
        let obj = GitObject::new(ObjectType::Blob, b"test content".to_vec());
        let serialized = obj.serialize();
        let deserialized = GitObject::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.obj_type, ObjectType::Blob);
        assert_eq!(&deserialized.data, b"test content");
    }

    #[test]
    fn test_tree_entry() {
        let entry = TreeEntry::new(0o100644, "file.txt", ObjectId::ZERO);
        assert!(entry.is_blob());
        assert!(!entry.is_tree());

        let dir = TreeEntry::new(0o040000, "subdir", ObjectId::ZERO);
        assert!(dir.is_tree());
        assert!(!dir.is_blob());
    }

    #[test]
    fn test_tree_serialize_deserialize() {
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(0o100644, "hello.txt", ObjectId::ZERO));
        tree.add_entry(TreeEntry::new(0o040000, "src", ObjectId::ZERO));

        let obj = tree.to_object();
        let parsed = Tree::from_data(&obj.data).unwrap();
        assert_eq!(parsed.entries.len(), 2);
    }

    #[test]
    fn test_person_format() {
        let p = Person::new("Alice", "alice@example.com", 1234567890);
        let formatted = p.format();
        assert!(formatted.contains("Alice"));
        assert!(formatted.contains("<alice@example.com>"));
        assert!(formatted.contains("1234567890"));
    }

    #[test]
    fn test_commit_object() {
        let author = Person::new("Test", "test@test.com", 1000000);
        let commit = Commit::new(ObjectId::ZERO, author, "Initial commit");
        let obj = commit.to_object();
        assert_eq!(obj.obj_type, ObjectType::Commit);

        let parsed = Commit::from_data(&obj.data).unwrap();
        assert_eq!(parsed.tree, ObjectId::ZERO);
        assert!(parsed.parents.is_empty());
        assert_eq!(parsed.author.name, "Test");
        assert!(parsed.message.contains("Initial commit"));
    }

    #[test]
    fn test_commit_with_parent() {
        let author = Person::new("Dev", "dev@os.com", 2000000);
        let mut commit = Commit::new(ObjectId::ZERO, author, "Second commit");
        let parent_hex = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        commit.parents.push(ObjectId::from_hex(parent_hex).unwrap());

        let obj = commit.to_object();
        let parsed = Commit::from_data(&obj.data).unwrap();
        assert_eq!(parsed.parents.len(), 1);
        assert_eq!(parsed.parents[0].to_hex(), parent_hex);
    }

    #[test]
    fn test_tag_object() {
        let tagger = Person::new("Release", "rel@os.com", 3000000);
        let tag = Tag::new(ObjectId::ZERO, "v1.0", tagger, "Release 1.0");
        let obj = tag.to_object();
        assert_eq!(obj.obj_type, ObjectType::Tag);
    }

    #[test]
    fn test_object_id_display() {
        let oid = ObjectId::ZERO;
        let s = alloc::format!("{}", oid);
        assert_eq!(s, "0000000000000000000000000000000000000000");
    }
}
