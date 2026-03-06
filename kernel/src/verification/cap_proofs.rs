#![allow(unexpected_cfgs)]
//! Capability Formal Model
//!
//! Formal verification of the capability system: non-forgery, rights
//! monotonicity (derivation produces subsets), cascading revocation,
//! generation-based invalidation, and cross-address-space isolation.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Rights bitmask constants
pub const CAP_RIGHT_READ: u32 = 1 << 0;
pub const CAP_RIGHT_WRITE: u32 = 1 << 1;
pub const CAP_RIGHT_EXECUTE: u32 = 1 << 2;
pub const CAP_RIGHT_GRANT: u32 = 1 << 3;
pub const CAP_RIGHT_REVOKE: u32 = 1 << 4;
pub const CAP_RIGHT_MAP: u32 = 1 << 5;
pub const CAP_RIGHT_DERIVE: u32 = 1 << 6;
pub const CAP_ALL_RIGHTS: u32 = 0x7F;

/// Model of a capability token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityModel {
    /// Unique token value
    pub token: u64,
    /// Rights bitmask
    pub rights: u32,
    /// Owner process ID
    pub owner: u64,
    /// Generation counter (incremented on revocation)
    pub generation: u32,
    /// Parent token (0 = root capability)
    pub parent_token: u64,
    /// Address space this capability belongs to
    pub address_space: u64,
}

impl CapabilityModel {
    /// Encode capability fields into a single u64 token
    ///
    /// Layout: [generation:16][rights:16][index:32]
    pub fn encode(index: u32, rights: u32, generation: u32) -> u64 {
        let gen = (generation as u64 & 0xFFFF) << 48;
        let rts = ((rights as u64) & 0xFFFF) << 32;
        let idx = index as u64;
        gen | rts | idx
    }

    /// Decode a token into (index, rights, generation)
    pub fn decode(token: u64) -> (u32, u32, u32) {
        let generation = ((token >> 48) & 0xFFFF) as u32;
        let rights = ((token >> 32) & 0xFFFF) as u32;
        let index = (token & 0xFFFF_FFFF) as u32;
        (index, rights, generation)
    }

    /// Derive a child capability with a subset of rights
    pub fn derive(
        &self,
        child_token: u64,
        child_rights: u32,
        child_owner: u64,
    ) -> Result<CapabilityModel, CapModelError> {
        // Rights can only be reduced, never expanded
        if child_rights & !self.rights != 0 {
            return Err(CapModelError::RightsEscalation);
        }

        // Must have derive right
        if self.rights & CAP_RIGHT_DERIVE == 0 {
            return Err(CapModelError::NoDeriveRight);
        }

        Ok(CapabilityModel {
            token: child_token,
            rights: child_rights,
            owner: child_owner,
            generation: self.generation,
            parent_token: self.token,
            address_space: child_owner, // Each process has its own address space
        })
    }

    /// Check if this capability has specific rights
    pub fn has_rights(&self, required: u32) -> bool {
        self.rights & required == required
    }
}

/// Capability space model (per-process capability table)
#[derive(Debug, Clone, Default)]
pub struct CapSpaceModel {
    /// All capabilities indexed by token
    #[cfg(feature = "alloc")]
    capabilities: BTreeMap<u64, CapabilityModel>,
    /// Parent-child relationships for revocation
    #[cfg(feature = "alloc")]
    children: BTreeMap<u64, Vec<u64>>,
    /// Revocation log
    #[cfg(feature = "alloc")]
    revocation_log: Vec<u64>,
    /// Current generation for new capabilities
    current_generation: u32,
    /// Next token counter (kernel-controlled)
    next_token: u64,
}

#[cfg(feature = "alloc")]
impl CapSpaceModel {
    /// Create a new capability space
    pub fn new() -> Self {
        Self {
            capabilities: BTreeMap::new(),
            children: BTreeMap::new(),
            revocation_log: Vec::new(),
            current_generation: 0,
            next_token: 1, // Token 0 is reserved
        }
    }

    /// Create a root capability (only the kernel can do this)
    pub fn create_root(&mut self, rights: u32, owner: u64) -> CapabilityModel {
        let token = self.next_token;
        self.next_token += 1;

        let cap = CapabilityModel {
            token,
            rights,
            owner,
            generation: self.current_generation,
            parent_token: 0,
            address_space: owner,
        };

        self.capabilities.insert(token, cap);
        cap
    }

    /// Derive a child capability from a parent
    pub fn derive(
        &mut self,
        parent_token: u64,
        child_rights: u32,
        child_owner: u64,
    ) -> Result<CapabilityModel, CapModelError> {
        let parent = self
            .capabilities
            .get(&parent_token)
            .ok_or(CapModelError::InvalidToken)?;

        let child_token = self.next_token;
        self.next_token += 1;

        let child = parent.derive(child_token, child_rights, child_owner)?;

        self.capabilities.insert(child_token, child);
        self.children
            .entry(parent_token)
            .or_default()
            .push(child_token);

        Ok(child)
    }

    /// Revoke a capability and all its descendants
    pub fn revoke(&mut self, token: u64) -> Result<usize, CapModelError> {
        if !self.capabilities.contains_key(&token) {
            return Err(CapModelError::InvalidToken);
        }

        let mut revoked = Vec::new();
        self.collect_descendants(token, &mut revoked);
        revoked.push(token);

        let count = revoked.len();
        for t in &revoked {
            self.capabilities.remove(t);
            self.children.remove(t);
            self.revocation_log.push(*t);
        }

        // Bump generation to invalidate any stale references
        self.current_generation = self.current_generation.saturating_add(1);

        Ok(count)
    }

    /// Collect all descendant tokens recursively
    fn collect_descendants(&self, token: u64, result: &mut Vec<u64>) {
        if let Some(children) = self.children.get(&token) {
            for &child in children {
                result.push(child);
                self.collect_descendants(child, result);
            }
        }
    }

    /// Look up a capability by token
    pub fn lookup(&self, token: u64) -> Option<&CapabilityModel> {
        self.capabilities.get(&token)
    }

    /// Validate that a token is still valid (correct generation)
    pub fn validate(
        &self,
        token: u64,
        expected_gen: u32,
    ) -> Result<&CapabilityModel, CapModelError> {
        let cap = self
            .capabilities
            .get(&token)
            .ok_or(CapModelError::InvalidToken)?;
        if cap.generation != expected_gen {
            return Err(CapModelError::StaleGeneration);
        }
        Ok(cap)
    }

    /// Get the number of capabilities
    pub fn count(&self) -> usize {
        self.capabilities.len()
    }

    /// Get the revocation log
    pub fn revocation_log(&self) -> &[u64] {
        &self.revocation_log
    }
}

/// Capability invariant checker
pub struct CapInvariantChecker;

#[cfg(feature = "alloc")]
impl CapInvariantChecker {
    /// Verify non-forgery: capabilities can only be created through the kernel
    /// API
    ///
    /// A random u64 should not match any valid capability in the space.
    pub fn verify_non_forgery(
        space: &CapSpaceModel,
        random_token: u64,
    ) -> Result<(), CapModelError> {
        // If the random token happens to be in the space, that's only valid if
        // it was created through create_root() or derive() (tracked by next_token)
        if random_token >= space.next_token && space.capabilities.contains_key(&random_token) {
            return Err(CapModelError::ForgeryDetected);
        }
        Ok(())
    }

    /// Verify rights monotonicity: derived capabilities have subset of parent
    /// rights
    pub fn verify_rights_monotonicity(space: &CapSpaceModel) -> Result<(), CapModelError> {
        for (_, cap) in space.capabilities.iter() {
            if cap.parent_token != 0 {
                if let Some(parent) = space.capabilities.get(&cap.parent_token) {
                    if cap.rights & !parent.rights != 0 {
                        return Err(CapModelError::RightsEscalation);
                    }
                }
            }
        }
        Ok(())
    }

    /// Verify revocation completeness: revoking a parent removes all children
    pub fn verify_revocation_completeness(
        space: &CapSpaceModel,
        revoked_token: u64,
    ) -> Result<(), CapModelError> {
        // After revocation, neither the token nor any descendants should exist
        if space.capabilities.contains_key(&revoked_token) {
            return Err(CapModelError::RevocationIncomplete);
        }

        // Check no children reference the revoked token as parent
        for (_, cap) in space.capabilities.iter() {
            if cap.parent_token == revoked_token {
                return Err(CapModelError::OrphanCapability);
            }
        }

        Ok(())
    }

    /// Verify generation integrity: bumping generation invalidates old tokens
    pub fn verify_generation_integrity(
        space: &CapSpaceModel,
        old_gen: u32,
    ) -> Result<(), CapModelError> {
        if space.current_generation <= old_gen {
            return Err(CapModelError::StaleGeneration);
        }
        Ok(())
    }
}

/// Errors from capability verification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapModelError {
    /// Token is not valid
    InvalidToken,
    /// Attempted rights escalation
    RightsEscalation,
    /// No derive right on parent
    NoDeriveRight,
    /// Token generation is stale
    StaleGeneration,
    /// Forged capability detected
    ForgeryDetected,
    /// Revocation did not complete
    RevocationIncomplete,
    /// Orphan capability found after revocation
    OrphanCapability,
    /// Capability crossed address space boundary
    IsolationBreach,
}

// ============================================================================
// Kani Proof Harnesses
// ============================================================================

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof: Token encoding and decoding is a roundtrip
    #[kani::proof]
    fn proof_token_encoding_roundtrip() {
        let index: u32 = kani::any();
        let rights: u32 = kani::any();
        let generation: u32 = kani::any();

        // Constrain to valid bit widths
        kani::assume(rights <= 0xFFFF);
        kani::assume(generation <= 0xFFFF);

        let encoded = CapabilityModel::encode(index, rights, generation);
        let (dec_index, dec_rights, dec_gen) = CapabilityModel::decode(encoded);

        assert_eq!(dec_index, index);
        assert_eq!(dec_rights, rights);
        assert_eq!(dec_gen, generation);
    }

    /// Proof: A random u64 that was not created via the API is not a valid
    /// capability
    #[kani::proof]
    fn proof_no_forgery() {
        let mut space = CapSpaceModel::new();
        space.create_root(CAP_ALL_RIGHTS, 1);

        let random: u64 = kani::any();
        kani::assume(random >= space.next_token); // Not a valid token

        assert!(space.lookup(random).is_none());
    }

    /// Proof: Derived capability rights are always a subset of parent rights
    #[kani::proof]
    fn proof_derivation_subset() {
        let parent_rights: u32 = kani::any();
        kani::assume(parent_rights & CAP_RIGHT_DERIVE != 0); // Must have derive right
        kani::assume(parent_rights <= CAP_ALL_RIGHTS);

        let child_rights: u32 = kani::any();
        kani::assume(child_rights <= CAP_ALL_RIGHTS);

        let parent = CapabilityModel {
            token: 1,
            rights: parent_rights,
            owner: 1,
            generation: 0,
            parent_token: 0,
            address_space: 1,
        };

        let result = parent.derive(2, child_rights, 2);

        match result {
            Ok(child) => {
                // If derivation succeeded, child rights must be subset
                assert_eq!(child.rights & !parent.rights, 0);
            }
            Err(CapModelError::RightsEscalation) => {
                // If it failed, child had rights not in parent
                assert!(child_rights & !parent_rights != 0);
            }
            _ => panic!("Unexpected error"),
        }
    }

    /// Proof: Revoking a parent revokes all descendants
    #[kani::proof]
    fn proof_cascading_revocation() {
        let mut space = CapSpaceModel::new();

        let root = space.create_root(CAP_ALL_RIGHTS, 1);
        let child = space.derive(root.token, CAP_RIGHT_READ, 2).unwrap();
        let grandchild = space.derive(child.token, CAP_RIGHT_READ, 3).unwrap();

        let root_token = root.token;
        let child_token = child.token;
        let grandchild_token = grandchild.token;

        space.revoke(root_token).unwrap();

        assert!(space.lookup(root_token).is_none());
        assert!(space.lookup(child_token).is_none());
        assert!(space.lookup(grandchild_token).is_none());
    }

    /// Proof: Bumping generation invalidates old tokens
    #[kani::proof]
    fn proof_generation_invalidation() {
        let mut space = CapSpaceModel::new();

        let cap = space.create_root(CAP_ALL_RIGHTS, 1);
        let old_gen = cap.generation;

        space.revoke(cap.token).unwrap();

        // Generation must have been bumped
        assert!(space.current_generation > old_gen);
    }

    /// Proof: AND/OR/NOT on rights bitmask produces correct results
    #[kani::proof]
    fn proof_rights_mask_operations() {
        let r1: u32 = kani::any();
        let r2: u32 = kani::any();
        kani::assume(r1 <= CAP_ALL_RIGHTS);
        kani::assume(r2 <= CAP_ALL_RIGHTS);

        // AND: intersection
        let intersection = r1 & r2;
        assert_eq!(intersection & !r1, 0);
        assert_eq!(intersection & !r2, 0);

        // OR: union
        let union = r1 | r2;
        assert_eq!(union & r1, r1);
        assert_eq!(union & r2, r2);

        // NOT + mask: complement within valid range
        let complement = !r1 & CAP_ALL_RIGHTS;
        assert_eq!(complement & r1, 0);
    }

    /// Proof: Capabilities don't cross address spaces
    #[kani::proof]
    fn proof_capability_isolation() {
        let mut space = CapSpaceModel::new();

        let cap1 = space.create_root(CAP_ALL_RIGHTS, 1);
        let cap2 = space.create_root(CAP_ALL_RIGHTS, 2);

        assert_eq!(cap1.address_space, 1);
        assert_eq!(cap2.address_space, 2);
        assert_ne!(cap1.address_space, cap2.address_space);
    }

    /// Proof: No orphan capabilities remain after revocation
    #[kani::proof]
    fn proof_revocation_completeness() {
        let mut space = CapSpaceModel::new();

        let root = space.create_root(CAP_ALL_RIGHTS, 1);
        let _child = space
            .derive(root.token, CAP_RIGHT_READ | CAP_RIGHT_DERIVE, 2)
            .unwrap();

        let root_token = root.token;
        space.revoke(root_token).unwrap();

        // Verify completeness
        assert!(CapInvariantChecker::verify_revocation_completeness(&space, root_token).is_ok());
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_encode_decode() {
        let encoded = CapabilityModel::encode(42, 0x1F, 3);
        let (idx, rights, gen) = CapabilityModel::decode(encoded);
        assert_eq!(idx, 42);
        assert_eq!(rights, 0x1F);
        assert_eq!(gen, 3);
    }

    #[test]
    fn test_has_rights() {
        let cap = CapabilityModel {
            token: 1,
            rights: CAP_RIGHT_READ | CAP_RIGHT_WRITE,
            owner: 1,
            generation: 0,
            parent_token: 0,
            address_space: 1,
        };

        assert!(cap.has_rights(CAP_RIGHT_READ));
        assert!(cap.has_rights(CAP_RIGHT_WRITE));
        assert!(cap.has_rights(CAP_RIGHT_READ | CAP_RIGHT_WRITE));
        assert!(!cap.has_rights(CAP_RIGHT_EXECUTE));
    }

    #[test]
    fn test_derive_subset() {
        let parent = CapabilityModel {
            token: 1,
            rights: CAP_RIGHT_READ | CAP_RIGHT_WRITE | CAP_RIGHT_DERIVE,
            owner: 1,
            generation: 0,
            parent_token: 0,
            address_space: 1,
        };

        let child = parent.derive(2, CAP_RIGHT_READ, 2).unwrap();
        assert_eq!(child.rights, CAP_RIGHT_READ);
        assert_eq!(child.parent_token, 1);
    }

    #[test]
    fn test_derive_escalation_fails() {
        let parent = CapabilityModel {
            token: 1,
            rights: CAP_RIGHT_READ | CAP_RIGHT_DERIVE,
            owner: 1,
            generation: 0,
            parent_token: 0,
            address_space: 1,
        };

        let result = parent.derive(2, CAP_RIGHT_WRITE, 2);
        assert_eq!(result, Err(CapModelError::RightsEscalation));
    }

    #[test]
    fn test_derive_without_derive_right() {
        let parent = CapabilityModel {
            token: 1,
            rights: CAP_RIGHT_READ,
            owner: 1,
            generation: 0,
            parent_token: 0,
            address_space: 1,
        };

        let result = parent.derive(2, CAP_RIGHT_READ, 2);
        assert_eq!(result, Err(CapModelError::NoDeriveRight));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_capspace_create_and_lookup() {
        let mut space = CapSpaceModel::new();
        let cap = space.create_root(CAP_ALL_RIGHTS, 1);
        assert!(space.lookup(cap.token).is_some());
        assert_eq!(space.count(), 1);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_capspace_derive() {
        let mut space = CapSpaceModel::new();
        let root = space.create_root(CAP_ALL_RIGHTS, 1);
        let child = space.derive(root.token, CAP_RIGHT_READ, 2).unwrap();
        assert_eq!(child.rights, CAP_RIGHT_READ);
        assert_eq!(space.count(), 2);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_cascading_revocation() {
        let mut space = CapSpaceModel::new();
        let root = space.create_root(CAP_ALL_RIGHTS, 1);
        let child = space
            .derive(root.token, CAP_RIGHT_READ | CAP_RIGHT_DERIVE, 2)
            .unwrap();
        let _grandchild = space.derive(child.token, CAP_RIGHT_READ, 3).unwrap();

        assert_eq!(space.count(), 3);

        let revoked = space.revoke(root.token).unwrap();
        assert_eq!(revoked, 3); // root + child + grandchild
        assert_eq!(space.count(), 0);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_generation_bump() {
        let mut space = CapSpaceModel::new();
        let gen_before = space.current_generation;

        let cap = space.create_root(CAP_ALL_RIGHTS, 1);
        space.revoke(cap.token).unwrap();

        assert!(space.current_generation > gen_before);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn test_rights_monotonicity() {
        let mut space = CapSpaceModel::new();
        let root = space.create_root(CAP_ALL_RIGHTS, 1);
        let _child = space.derive(root.token, CAP_RIGHT_READ, 2).unwrap();

        assert!(CapInvariantChecker::verify_rights_monotonicity(&space).is_ok());
    }
}
