//! Git Porcelain Commands
//!
//! Implements user-facing git commands: init, add, commit, log, diff,
//! branch, checkout, status.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use super::{
    objects::{Blob, Commit, GitObject, ObjectId, ObjectType, Person, Tree, TreeEntry},
    refs::{RefStore, RefValue},
};

/// Index entry (staging area)
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub path: String,
    pub id: ObjectId,
    pub mode: u32,
    pub size: u64,
}

/// Git repository (in-memory)
pub struct Repository {
    /// Object store (id -> serialized data)
    objects: BTreeMap<ObjectId, Vec<u8>>,
    /// Reference store
    pub refs: RefStore,
    /// Index (staging area)
    index: Vec<IndexEntry>,
    /// Working directory path
    pub workdir: String,
    /// User configuration
    pub config: GitConfig,
}

/// Git configuration
#[derive(Debug, Clone)]
pub struct GitConfig {
    pub user_name: String,
    pub user_email: String,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            user_name: "VeridianOS User".to_string(),
            user_email: "user@veridian.local".to_string(),
        }
    }
}

/// Diff hunk
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

/// Diff line
#[derive(Debug, Clone)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

/// Log entry for display
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub id: ObjectId,
    pub author: String,
    pub timestamp: u64,
    pub message: String,
}

impl Repository {
    /// Initialize a new repository
    pub fn init(workdir: &str) -> Self {
        Self {
            objects: BTreeMap::new(),
            refs: RefStore::new(),
            index: Vec::new(),
            workdir: workdir.to_string(),
            config: GitConfig::default(),
        }
    }

    /// Store a git object and return its ID
    pub fn store_object(&mut self, obj: &GitObject) -> ObjectId {
        let id = obj.compute_id();
        self.objects.entry(id).or_insert_with(|| obj.serialize());
        id
    }

    /// Retrieve a git object by ID
    pub fn get_object(&self, id: &ObjectId) -> Option<GitObject> {
        let data = self.objects.get(id)?;
        GitObject::deserialize(data)
    }

    /// Check if an object exists
    pub fn has_object(&self, id: &ObjectId) -> bool {
        self.objects.contains_key(id)
    }

    /// Object count
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    // -----------------------------------------------------------------------
    // Porcelain commands
    // -----------------------------------------------------------------------

    /// `git add` -- stage a file
    pub fn add(&mut self, path: &str, content: &[u8]) -> ObjectId {
        let blob = Blob::new(content.to_vec());
        let obj = blob.to_object();
        let id = self.store_object(&obj);

        // Update index
        if let Some(entry) = self.index.iter_mut().find(|e| e.path == path) {
            entry.id = id;
            entry.size = content.len() as u64;
        } else {
            self.index.push(IndexEntry {
                path: path.to_string(),
                id,
                mode: 0o100644,
                size: content.len() as u64,
            });
        }

        id
    }

    /// `git commit` -- create a commit from the index
    pub fn commit(&mut self, message: &str) -> Option<ObjectId> {
        if self.index.is_empty() {
            return None;
        }

        // Build tree from index
        let tree_id = self.build_tree_from_index();

        // Create commit
        let timestamp = 0; // Would use RTC in real implementation
        let author = Person::new(&self.config.user_name, &self.config.user_email, timestamp);
        let mut commit = Commit::new(tree_id, author, message);

        // Set parent to current HEAD
        if let Some(head_id) = self.refs.head() {
            commit.parents.push(*head_id);
        }

        let obj = commit.to_object();
        let commit_id = self.store_object(&obj);

        // Update current branch
        match self.refs.get("HEAD") {
            Some(RefValue::Symbolic(target)) => {
                let target = target.clone();
                self.refs.set_direct(&target, commit_id);
            }
            _ => {
                self.refs.set_direct("HEAD", commit_id);
            }
        }

        Some(commit_id)
    }

    /// Build a tree object from the index
    fn build_tree_from_index(&mut self) -> ObjectId {
        let mut tree = Tree::new();

        // Sort index entries
        let mut sorted_index = self.index.clone();
        sorted_index.sort_by(|a, b| a.path.cmp(&b.path));

        for entry in &sorted_index {
            // Simple: flat tree (no subdirectories)
            tree.add_entry(TreeEntry::new(entry.mode, &entry.path, entry.id));
        }

        let obj = tree.to_object();
        self.store_object(&obj)
    }

    /// `git log` -- walk commit history
    pub fn log(&self, max_entries: usize) -> Vec<LogEntry> {
        let mut entries = Vec::new();
        let mut current = match self.refs.head() {
            Some(id) => *id,
            None => return entries,
        };

        for _ in 0..max_entries {
            let obj = match self.get_object(&current) {
                Some(o) if o.obj_type == ObjectType::Commit => o,
                _ => break,
            };

            let commit = match Commit::from_data(&obj.data) {
                Some(c) => c,
                None => break,
            };

            entries.push(LogEntry {
                id: current,
                author: alloc::format!("{} <{}>", commit.author.name, commit.author.email),
                timestamp: commit.author.timestamp,
                message: commit.message.clone(),
            });

            // Follow first parent
            current = match commit.parents.first() {
                Some(p) => *p,
                None => break,
            };
        }

        entries
    }

    /// `git diff` -- compare two blobs (simple line diff)
    pub fn diff_blobs(&self, old: &[u8], new: &[u8]) -> Vec<DiffHunk> {
        let old_lines: Vec<&str> = core::str::from_utf8(old).unwrap_or("").lines().collect();
        let new_lines: Vec<&str> = core::str::from_utf8(new).unwrap_or("").lines().collect();

        // Simple LCS-based diff
        let mut hunks = Vec::new();
        let mut lines = Vec::new();
        let mut old_idx = 0;
        let mut new_idx = 0;

        while old_idx < old_lines.len() || new_idx < new_lines.len() {
            if old_idx < old_lines.len()
                && new_idx < new_lines.len()
                && old_lines[old_idx] == new_lines[new_idx]
            {
                lines.push(DiffLine::Context(old_lines[old_idx].to_string()));
                old_idx += 1;
                new_idx += 1;
            } else if old_idx < old_lines.len()
                && (new_idx >= new_lines.len()
                    || !new_lines[new_idx..].contains(&old_lines[old_idx]))
            {
                lines.push(DiffLine::Removed(old_lines[old_idx].to_string()));
                old_idx += 1;
            } else if new_idx < new_lines.len() {
                lines.push(DiffLine::Added(new_lines[new_idx].to_string()));
                new_idx += 1;
            }
        }

        if !lines.is_empty() {
            hunks.push(DiffHunk {
                old_start: 1,
                old_count: old_lines.len(),
                new_start: 1,
                new_count: new_lines.len(),
                lines,
            });
        }

        hunks
    }

    /// `git branch` -- list or create branches
    pub fn branch_list(&self) -> Vec<String> {
        self.refs.branches()
    }

    pub fn branch_create(&mut self, name: &str) -> bool {
        if let Some(head_id) = self.refs.head() {
            let id = *head_id;
            self.refs.create_branch(name, id);
            true
        } else {
            false
        }
    }

    pub fn branch_delete(&mut self, name: &str) -> bool {
        let ref_name = alloc::format!("refs/heads/{}", name);
        self.refs.delete(&ref_name)
    }

    /// `git checkout` -- switch branches
    pub fn checkout(&mut self, branch: &str) -> bool {
        self.refs.checkout_branch(branch)
    }

    /// `git status` -- show index and working tree status
    pub fn status(&self) -> Vec<String> {
        let mut lines = Vec::new();

        if let Some(branch) = self.refs.current_branch() {
            lines.push(alloc::format!("On branch {}", branch));
        } else {
            lines.push("HEAD detached".to_string());
        }

        if self.index.is_empty() {
            lines.push("nothing to commit".to_string());
        } else {
            lines.push(alloc::format!(
                "Changes to be committed ({} files):",
                self.index.len()
            ));
            for entry in &self.index {
                lines.push(alloc::format!("  new file: {}", entry.path));
            }
        }

        lines
    }

    /// `git tag` -- create a lightweight tag
    pub fn tag_create(&mut self, name: &str) -> bool {
        if let Some(head_id) = self.refs.head() {
            let id = *head_id;
            self.refs.create_tag(name, id);
            true
        } else {
            false
        }
    }

    /// Get index entry count
    pub fn index_count(&self) -> usize {
        self.index.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repository_init() {
        let repo = Repository::init("/tmp/test");
        assert_eq!(repo.workdir, "/tmp/test");
        assert_eq!(repo.object_count(), 0);
        assert_eq!(repo.index_count(), 0);
    }

    #[test]
    fn test_add_file() {
        let mut repo = Repository::init("/tmp/test");
        let id = repo.add("hello.txt", b"Hello, World!\n");
        assert_ne!(id, ObjectId::ZERO);
        assert_eq!(repo.index_count(), 1);
        assert!(repo.has_object(&id));
    }

    #[test]
    fn test_add_overwrites_same_path() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"v1");
        repo.add("file.txt", b"v2");
        assert_eq!(repo.index_count(), 1);
    }

    #[test]
    fn test_commit() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        let id = repo.commit("Initial commit");
        assert!(id.is_some());

        let obj = repo.get_object(&id.unwrap()).unwrap();
        assert_eq!(obj.obj_type, ObjectType::Commit);
    }

    #[test]
    fn test_commit_empty_index() {
        let mut repo = Repository::init("/tmp/test");
        assert!(repo.commit("Empty").is_none());
    }

    #[test]
    fn test_commit_chain() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("a.txt", b"a");
        let first = repo.commit("First").unwrap();

        repo.add("b.txt", b"b");
        let second = repo.commit("Second").unwrap();

        // Second commit should have first as parent
        let obj = repo.get_object(&second).unwrap();
        let commit = Commit::from_data(&obj.data).unwrap();
        assert_eq!(commit.parents.len(), 1);
        assert_eq!(commit.parents[0], first);
    }

    #[test]
    fn test_log() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("a.txt", b"a");
        repo.commit("First commit");
        repo.add("b.txt", b"b");
        repo.commit("Second commit");

        let log = repo.log(10);
        assert_eq!(log.len(), 2);
        assert!(log[0].message.contains("Second commit"));
        assert!(log[1].message.contains("First commit"));
    }

    #[test]
    fn test_log_empty() {
        let repo = Repository::init("/tmp/test");
        let log = repo.log(10);
        assert!(log.is_empty());
    }

    #[test]
    fn test_diff_blobs() {
        let repo = Repository::init("/tmp/test");
        let old = b"line1\nline2\nline3\n";
        let new = b"line1\nmodified\nline3\n";
        let hunks = repo.diff_blobs(old, new);
        assert!(!hunks.is_empty());
    }

    #[test]
    fn test_diff_identical() {
        let repo = Repository::init("/tmp/test");
        let content = b"same\n";
        let hunks = repo.diff_blobs(content, content);
        // All context lines, still produces a hunk
        assert!(!hunks.is_empty());
    }

    #[test]
    fn test_branch_operations() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        repo.commit("Initial");

        assert!(repo.branch_create("dev"));
        let branches = repo.branch_list();
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"dev".to_string()));
    }

    #[test]
    fn test_checkout() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        repo.commit("Initial");
        repo.branch_create("dev");

        assert!(repo.checkout("dev"));
        assert_eq!(repo.refs.current_branch(), Some("dev"));
    }

    #[test]
    fn test_checkout_nonexistent() {
        let mut repo = Repository::init("/tmp/test");
        assert!(!repo.checkout("nonexistent"));
    }

    #[test]
    fn test_branch_delete() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        repo.commit("Initial");
        repo.branch_create("temp");
        assert!(repo.branch_delete("temp"));
        assert!(!repo.branch_delete("temp"));
    }

    #[test]
    fn test_status_empty() {
        let repo = Repository::init("/tmp/test");
        let status = repo.status();
        assert!(status.iter().any(|s| s.contains("nothing to commit")));
    }

    #[test]
    fn test_status_with_staged() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        let status = repo.status();
        assert!(status.iter().any(|s| s.contains("1 files")));
    }

    #[test]
    fn test_tag_create() {
        let mut repo = Repository::init("/tmp/test");
        repo.add("file.txt", b"content");
        repo.commit("Release");
        assert!(repo.tag_create("v1.0"));
        assert!(repo.refs.tags().contains(&"v1.0".to_string()));
    }

    #[test]
    fn test_config_default() {
        let config = GitConfig::default();
        assert!(!config.user_name.is_empty());
        assert!(!config.user_email.is_empty());
    }

    #[test]
    fn test_store_and_retrieve_object() {
        let mut repo = Repository::init("/tmp/test");
        let obj = GitObject::new(ObjectType::Blob, b"hello".to_vec());
        let id = repo.store_object(&obj);
        let retrieved = repo.get_object(&id).unwrap();
        assert_eq!(retrieved.obj_type, ObjectType::Blob);
        assert_eq!(&retrieved.data, b"hello");
    }
}
