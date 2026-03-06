//! Git Reference Management
//!
//! Manages HEAD, branches, tags, and symbolic references.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use super::objects::ObjectId;

/// Reference types
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefValue {
    /// Direct reference to an object ID
    Direct(ObjectId),
    /// Symbolic reference (e.g., HEAD -> refs/heads/main)
    Symbolic(String),
}

/// Reference entry
#[derive(Debug, Clone)]
pub(crate) struct Ref {
    pub(crate) name: String,
    pub(crate) value: RefValue,
}

impl Ref {
    pub(crate) fn direct(name: &str, id: ObjectId) -> Self {
        Self {
            name: name.to_string(),
            value: RefValue::Direct(id),
        }
    }

    pub(crate) fn symbolic(name: &str, target: &str) -> Self {
        Self {
            name: name.to_string(),
            value: RefValue::Symbolic(target.to_string()),
        }
    }

    /// Resolve a reference to a direct object ID
    pub(crate) fn resolve<'a>(&'a self, store: &'a RefStore) -> Option<&'a ObjectId> {
        match &self.value {
            RefValue::Direct(id) => Some(id),
            RefValue::Symbolic(target) => store.resolve(target),
        }
    }
}

/// Reference store (in-memory)
#[derive(Debug, Clone)]
pub(crate) struct RefStore {
    refs: BTreeMap<String, RefValue>,
}

impl Default for RefStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RefStore {
    pub(crate) fn new() -> Self {
        let mut refs = BTreeMap::new();
        // Default HEAD -> refs/heads/main
        refs.insert(
            "HEAD".to_string(),
            RefValue::Symbolic("refs/heads/main".to_string()),
        );
        Self { refs }
    }

    /// Get a reference value
    pub(crate) fn get(&self, name: &str) -> Option<&RefValue> {
        self.refs.get(name)
    }

    /// Set a direct reference
    pub(crate) fn set_direct(&mut self, name: &str, id: ObjectId) {
        self.refs.insert(name.to_string(), RefValue::Direct(id));
    }

    /// Set a symbolic reference
    pub(crate) fn set_symbolic(&mut self, name: &str, target: &str) {
        self.refs
            .insert(name.to_string(), RefValue::Symbolic(target.to_string()));
    }

    /// Resolve a reference name to an object ID, following symbolic refs
    pub(crate) fn resolve(&self, name: &str) -> Option<&ObjectId> {
        let mut current = name;
        let mut depth = 0;
        loop {
            if depth > 10 {
                return None; // Prevent infinite loops
            }
            match self.refs.get(current)? {
                RefValue::Direct(id) => return Some(id),
                RefValue::Symbolic(target) => {
                    current = target;
                    depth += 1;
                }
            }
        }
    }

    /// Get HEAD object ID
    pub(crate) fn head(&self) -> Option<&ObjectId> {
        self.resolve("HEAD")
    }

    /// Get current branch name (if HEAD is symbolic)
    pub(crate) fn current_branch(&self) -> Option<&str> {
        match self.refs.get("HEAD")? {
            RefValue::Symbolic(target) => target.strip_prefix("refs/heads/"),
            RefValue::Direct(_) => None, // Detached HEAD
        }
    }

    /// List all branches
    pub(crate) fn branches(&self) -> Vec<String> {
        self.refs
            .keys()
            .filter(|k| k.starts_with("refs/heads/"))
            .map(|k| k.strip_prefix("refs/heads/").unwrap_or(k).to_string())
            .collect()
    }

    /// List all tags
    pub(crate) fn tags(&self) -> Vec<String> {
        self.refs
            .keys()
            .filter(|k| k.starts_with("refs/tags/"))
            .map(|k| k.strip_prefix("refs/tags/").unwrap_or(k).to_string())
            .collect()
    }

    /// Delete a reference
    pub(crate) fn delete(&mut self, name: &str) -> bool {
        self.refs.remove(name).is_some()
    }

    /// Create a branch pointing to a commit
    pub(crate) fn create_branch(&mut self, name: &str, id: ObjectId) {
        let ref_name = alloc::format!("refs/heads/{}", name);
        self.set_direct(&ref_name, id);
    }

    /// Create a lightweight tag
    pub(crate) fn create_tag(&mut self, name: &str, id: ObjectId) {
        let ref_name = alloc::format!("refs/tags/{}", name);
        self.set_direct(&ref_name, id);
    }

    /// Switch HEAD to a branch
    pub(crate) fn checkout_branch(&mut self, name: &str) -> bool {
        let ref_name = alloc::format!("refs/heads/{}", name);
        if self.refs.contains_key(&ref_name) {
            self.set_symbolic("HEAD", &ref_name);
            true
        } else {
            false
        }
    }

    /// Ref count
    pub(crate) fn count(&self) -> usize {
        self.refs.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_oid() -> ObjectId {
        ObjectId::from_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap()
    }

    #[test]
    fn test_ref_store_new() {
        let store = RefStore::new();
        assert_eq!(store.current_branch(), Some("main"));
    }

    #[test]
    fn test_ref_store_set_get() {
        let mut store = RefStore::new();
        store.set_direct("refs/heads/main", test_oid());
        assert_eq!(store.resolve("refs/heads/main"), Some(&test_oid()));
    }

    #[test]
    fn test_ref_store_head() {
        let mut store = RefStore::new();
        store.set_direct("refs/heads/main", test_oid());
        assert_eq!(store.head(), Some(&test_oid()));
    }

    #[test]
    fn test_ref_store_branches() {
        let mut store = RefStore::new();
        store.create_branch("main", test_oid());
        store.create_branch("dev", test_oid());
        let branches = store.branches();
        assert_eq!(branches.len(), 2);
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"dev".to_string()));
    }

    #[test]
    fn test_ref_store_tags() {
        let mut store = RefStore::new();
        store.create_tag("v1.0", test_oid());
        let tags = store.tags();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], "v1.0");
    }

    #[test]
    fn test_ref_store_delete() {
        let mut store = RefStore::new();
        store.create_branch("temp", test_oid());
        assert!(store.delete("refs/heads/temp"));
        assert!(!store.delete("refs/heads/temp"));
    }

    #[test]
    fn test_ref_store_checkout() {
        let mut store = RefStore::new();
        store.create_branch("main", test_oid());
        store.create_branch("dev", test_oid());

        assert!(store.checkout_branch("dev"));
        assert_eq!(store.current_branch(), Some("dev"));

        assert!(!store.checkout_branch("nonexistent"));
    }

    #[test]
    fn test_symbolic_ref_chain() {
        let mut store = RefStore::new();
        store.set_direct("refs/heads/main", test_oid());
        // HEAD -> refs/heads/main -> ObjectId
        assert_eq!(store.resolve("HEAD"), Some(&test_oid()));
    }

    #[test]
    fn test_detached_head() {
        let mut store = RefStore::new();
        store.set_direct("HEAD", test_oid());
        assert_eq!(store.current_branch(), None);
        assert_eq!(store.head(), Some(&test_oid()));
    }

    #[test]
    fn test_ref_value_eq() {
        let a = RefValue::Direct(test_oid());
        let b = RefValue::Direct(test_oid());
        assert_eq!(a, b);
    }

    #[test]
    fn test_ref_direct() {
        let r = Ref::direct("refs/heads/main", test_oid());
        assert_eq!(r.name, "refs/heads/main");
        match &r.value {
            RefValue::Direct(id) => assert_eq!(*id, test_oid()),
            _ => panic!("Expected direct ref"),
        }
    }

    #[test]
    fn test_ref_symbolic() {
        let r = Ref::symbolic("HEAD", "refs/heads/main");
        match &r.value {
            RefValue::Symbolic(target) => assert_eq!(target, "refs/heads/main"),
            _ => panic!("Expected symbolic ref"),
        }
    }

    #[test]
    fn test_ref_resolve() {
        let mut store = RefStore::new();
        store.set_direct("refs/heads/main", test_oid());

        let r = Ref::symbolic("HEAD", "refs/heads/main");
        assert_eq!(r.resolve(&store), Some(&test_oid()));
    }

    #[test]
    fn test_ref_store_count() {
        let mut store = RefStore::new();
        let initial = store.count(); // HEAD
        store.create_branch("dev", test_oid());
        assert_eq!(store.count(), initial + 1);
    }

    #[test]
    fn test_infinite_symbolic_loop() {
        let mut store = RefStore::new();
        store.set_symbolic("A", "B");
        store.set_symbolic("B", "A");
        assert!(store.resolve("A").is_none());
    }
}
