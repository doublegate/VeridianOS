//! Capability system tests
//!
//! Comprehensive test suite for capability-based security.

#![cfg(test)]

use super::*;
use crate::process::ProcessId;

mod token_tests {
    use super::*;

    #[test]
    fn test_capability_token_creation() {
        let cap = token::CapabilityToken::new(0x123456789ABC, 0x42, 0x7, 0xF);

        assert_eq!(cap.id(), 0x123456789ABC);
        assert_eq!(cap.generation(), 0x42);
        assert_eq!(cap.cap_type(), 0x7);
        assert_eq!(cap.flags(), 0xF);
    }

    #[test]
    fn test_capability_token_packing() {
        let cap = token::CapabilityToken::new(0xFFFFFFFFFFFF, 0xFF, 0xF, 0xF);
        let packed = cap.to_u64();
        let unpacked = token::CapabilityToken::from_u64(packed);

        assert_eq!(cap, unpacked);
    }

    #[test]
    fn test_null_capability() {
        let cap = token::CapabilityToken::null();
        assert!(cap.is_null());
        assert_eq!(cap.to_u64(), 0);
    }

    #[test]
    fn test_rights_operations() {
        let r1 = token::Rights::READ | token::Rights::WRITE;
        let r2 = token::Rights::WRITE | token::Rights::EXECUTE;

        assert!(r1.contains(token::Rights::READ));
        assert!(r1.contains(token::Rights::WRITE));
        assert!(!r1.contains(token::Rights::EXECUTE));

        let intersection = r1.intersection(r2);
        assert_eq!(intersection, token::Rights::WRITE);

        let union = r1.union(r2);
        assert!(union.contains(token::Rights::READ));
        assert!(union.contains(token::Rights::WRITE));
        assert!(union.contains(token::Rights::EXECUTE));

        let removed = r1.remove(token::Rights::WRITE);
        assert!(removed.contains(token::Rights::READ));
        assert!(!removed.contains(token::Rights::WRITE));
    }
}

mod space_tests {
    use super::*;

    #[test]
    fn test_capability_space_creation() {
        let cap_space = space::CapabilitySpace::new();
        let stats = cap_space.stats();

        assert_eq!(
            stats.total_caps.load(core::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// N-04: L2 keys were `(id >> 8) as u16`, so ids 2^24 apart aliased the
    /// same L2 slot and the second insert failed.
    #[test]
    fn test_l2_index_does_not_alias_large_ids() {
        let cap_space = space::CapabilitySpace::new();
        let obj = || object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let low = token::CapabilityToken::new(0x100, 0, 0, 0);
        let high = token::CapabilityToken::new(0x1_0000_0100, 0, 0, 0);
        assert!(cap_space.insert(low, obj(), token::Rights::READ).is_ok());
        assert!(cap_space.insert(high, obj(), token::Rights::WRITE).is_ok());
        assert_eq!(cap_space.lookup(low), Some(token::Rights::READ));
        assert_eq!(cap_space.lookup(high), Some(token::Rights::WRITE));
    }

    /// N-04: remove() took the slot before comparing tokens, so removing
    /// with a stale (wrong-generation) token deleted the live capability.
    #[test]
    fn test_remove_with_mismatched_token_keeps_entry() {
        let cap_space = space::CapabilitySpace::new();
        let obj = || object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        for id in [42u64, 300] {
            let live = token::CapabilityToken::new(id, 2, 0, 0);
            let stale = token::CapabilityToken::new(id, 1, 0, 0);
            cap_space.insert(live, obj(), token::Rights::READ).unwrap();
            assert!(cap_space.remove(stale).is_none(), "id {}", id);
            assert_eq!(
                cap_space.lookup(live),
                Some(token::Rights::READ),
                "id {}",
                id
            );
            assert!(cap_space.remove(live).is_some());
        }
    }

    #[test]
    fn test_capability_insertion_and_lookup() {
        let cap_space = space::CapabilitySpace::new();
        let cap = token::CapabilityToken::new(42, 1, 0, 0xF);
        let obj = object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let rights = token::Rights::READ | token::Rights::WRITE;

        // Insert capability
        assert!(cap_space.insert(cap, obj, rights).is_ok());

        // Lookup capability
        let found_rights = cap_space.lookup(cap);
        assert!(found_rights.is_some());
        assert_eq!(found_rights.unwrap(), rights);

        // Check rights
        assert!(cap_space.check_rights(cap, token::Rights::READ));
        assert!(cap_space.check_rights(cap, token::Rights::WRITE));
        assert!(!cap_space.check_rights(cap, token::Rights::EXECUTE));
    }

    #[test]
    fn test_capability_removal() {
        let cap_space = space::CapabilitySpace::new();
        let cap = token::CapabilityToken::new(42, 1, 0, 0xF);
        let obj = object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let rights = token::Rights::READ;

        // Insert and remove
        cap_space.insert(cap, obj.clone(), rights).unwrap();
        let removed = cap_space.remove(cap);

        assert!(removed.is_some());

        // Should not be found after removal
        assert!(cap_space.lookup(cap).is_none());
    }

    fn obj(pid: u64) -> object::ObjectRef {
        object::ObjectRef::Process {
            pid: ProcessId(pid),
        }
    }

    /// CAP-PERF-01: IDs are global, so a process's capabilities have
    /// arbitrary, widely spread IDs. Memory must follow the number held,
    /// not the ID values (the old L2 arrays cost ~14 KiB per capability).
    #[test]
    fn test_memory_follows_capabilities_held() {
        let cap_space = space::CapabilitySpace::new();
        assert_eq!(
            cap_space.table_capacity(),
            0,
            "empty space allocates nothing"
        );
        let ids = [100u64, 1000, 70_000, 1 << 40, (1 << 48) - 1];
        for &id in &ids {
            let cap = token::CapabilityToken::new(id, 1, 0, 0);
            cap_space.insert(cap, obj(id), token::Rights::READ).unwrap();
        }
        assert_eq!(cap_space.table_capacity(), 8);
        for &id in &ids {
            let cap = token::CapabilityToken::new(id, 1, 0, 0);
            assert_eq!(cap_space.lookup(cap), Some(token::Rights::READ));
            // A token with another generation is a different capability.
            assert!(cap_space
                .lookup(token::CapabilityToken::new(id, 2, 0, 0))
                .is_none());
        }
        for &id in &ids {
            assert!(cap_space
                .remove(token::CapabilityToken::new(id, 1, 0, 0))
                .is_some());
        }
        assert_eq!(cap_space.used(), 0);
        assert_eq!(cap_space.table_capacity(), 0, "memory returned when empty");
    }

    /// The hash table against a BTreeMap model through inserts, removes
    /// (tombstones), stale-generation removes, regrowth and reuse.
    #[test]
    fn test_table_matches_model() {
        use alloc::collections::BTreeMap;

        let cap_space = space::CapabilitySpace::with_quota(10_000);
        let mut model: BTreeMap<u64, u64> = BTreeMap::new();
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        for step in 0..20_000u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let id = (x % 600) * 97 + 1; // collisions and reuse
            let cap = token::CapabilityToken::new(id, 1, 0, 0);
            match x % 3 {
                0 | 1 => {
                    let r = cap_space.insert(cap, obj(step), token::Rights::READ);
                    assert_eq!(r.is_ok(), !model.contains_key(&id));
                    model.entry(id).or_insert(step);
                }
                _ => {
                    // A stale token never removes the live entry (N-04).
                    let stale = token::CapabilityToken::new(id, 2, 0, 0);
                    assert!(cap_space.remove(stale).is_none());
                    let removed = cap_space.remove(cap);
                    assert_eq!(removed.is_some(), model.remove(&id).is_some());
                }
            }
            assert_eq!(cap_space.used(), model.len());
        }
        for id in 0..600 * 97 + 2 {
            let cap = token::CapabilityToken::new(id, 1, 0, 0);
            assert_eq!(cap_space.lookup(cap).is_some(), model.contains_key(&id));
        }
        assert_eq!(cap_space.entries().len(), model.len());
        assert!(cap_space.table_capacity() <= 4 * model.len().max(2));
    }

    #[test]
    fn test_quota_enforced() {
        let cap_space = space::CapabilitySpace::with_quota(3);
        for id in 1..=3 {
            let cap = token::CapabilityToken::new(id * 1000, 1, 0, 0);
            cap_space.insert(cap, obj(id), token::Rights::READ).unwrap();
        }
        let extra = token::CapabilityToken::new(9999, 1, 0, 0);
        assert!(cap_space
            .insert(extra, obj(9), token::Rights::READ)
            .is_err());
        cap_space.remove(token::CapabilityToken::new(1000, 1, 0, 0));
        assert!(cap_space.insert(extra, obj(9), token::Rights::READ).is_ok());
    }
}

mod manager_tests {
    use super::*;

    /// IPC-INC-01: genuine endpoint tokens carry type/generation/flags in
    /// their high bits, so they are >= 2^32. The fast path's old range
    /// check rejected every one of them and accepted any smaller integer.
    /// Send permission is now decided by the sender's capability space.
    #[test]
    fn test_send_permission_uses_capability_space() {
        use alloc::sync::Arc;

        let cap_space = space::CapabilitySpace::new();
        let endpoint = object::ObjectRef::Endpoint {
            endpoint: Arc::new(crate::ipc::Endpoint::new(ProcessId(7))),
        };
        let cap = manager::cap_manager()
            .create_capability(endpoint, ipc_integration::IpcRights::SEND, &cap_space)
            .unwrap();

        assert!(
            cap.to_u64() >= 0x1_0000_0000,
            "genuine token {:#x}",
            cap.to_u64()
        );
        assert!(ipc_integration::check_send_permission(cap, &cap_space).is_ok());

        let forged = token::CapabilityToken::from_u64(0x1337_cafe);
        assert!(ipc_integration::check_send_permission(forged, &cap_space).is_err());

        let other_space = space::CapabilitySpace::new();
        assert!(
            ipc_integration::check_send_permission(cap, &other_space).is_err(),
            "a token is only valid in the space that holds it"
        );

        let read_only = manager::cap_manager()
            .create_capability(
                object::ObjectRef::Endpoint {
                    endpoint: Arc::new(crate::ipc::Endpoint::new(ProcessId(7))),
                },
                token::Rights::READ,
                &cap_space,
            )
            .unwrap();
        assert!(ipc_integration::check_send_permission(read_only, &cap_space).is_err());
    }

    #[test]
    fn test_capability_creation() {
        let cap_space = space::CapabilitySpace::new();
        let obj = object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let rights = token::Rights::READ | token::Rights::WRITE;

        let cap = manager::cap_manager()
            .create_capability(obj, rights, &cap_space)
            .unwrap();

        assert!(!cap.is_null());
        assert!(cap_space.check_rights(cap, token::Rights::READ));
        assert!(cap_space.check_rights(cap, token::Rights::WRITE));
    }

    #[test]
    fn test_capability_delegation() {
        let source_space = space::CapabilitySpace::new();
        let target_space = space::CapabilitySpace::new();

        // Create capability with grant permission
        let obj = object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let rights = token::Rights::READ | token::Rights::WRITE | token::Rights::GRANT;

        let cap = manager::cap_manager()
            .create_capability(obj, rights, &source_space)
            .unwrap();

        // Delegate with reduced rights
        let new_rights = token::Rights::READ;
        let new_cap = manager::cap_manager()
            .delegate(cap, &source_space, &target_space, new_rights)
            .unwrap();

        // Verify delegation
        assert!(target_space.check_rights(new_cap, token::Rights::READ));
        assert!(!target_space.check_rights(new_cap, token::Rights::WRITE));
        assert!(!target_space.check_rights(new_cap, token::Rights::GRANT));
    }

    #[test]
    fn test_capability_check() {
        let cap_space = space::CapabilitySpace::new();
        let obj = object::ObjectRef::Process {
            pid: ProcessId(1234),
        };
        let rights = token::Rights::READ;

        let cap = manager::cap_manager()
            .create_capability(obj, rights, &cap_space)
            .unwrap();

        // Check valid permission
        assert!(manager::check_capability(cap, token::Rights::READ, &cap_space).is_ok());

        // Check invalid permission
        assert!(manager::check_capability(cap, token::Rights::WRITE, &cap_space).is_err());
    }
}

// revocation_tests require process_server::init() which cannot run on host.
#[cfg(target_os = "none")]
mod revocation_tests {
    use super::*;

    #[test]
    fn test_capability_revocation() {
        let cap = token::CapabilityToken::new(123, 1, 0, 0);

        assert!(!revocation::is_revoked(cap));

        revocation::revoke_capability(cap).unwrap();

        assert!(revocation::is_revoked(cap));
    }

    #[test]
    fn test_revocation_cache() {
        let cache = revocation::RevocationCache::new();
        let cap = token::CapabilityToken::new(456, 1, 0, 0);

        // Initially not revoked
        assert!(!cache.is_revoked(cap));

        // Revoke and check cache
        revocation::revoke_capability(cap).unwrap();
        assert!(cache.is_revoked(cap));
    }
}

mod inheritance_tests {
    use super::*;

    #[test]
    fn test_rights_reduction() {
        let original = token::Rights::READ
            | token::Rights::WRITE
            | token::Rights::GRANT
            | token::Rights::REVOKE;

        let reduced = inheritance::reduce_rights_for_inheritance(original);

        assert!(reduced.contains(token::Rights::READ));
        assert!(reduced.contains(token::Rights::WRITE));
        assert!(!reduced.contains(token::Rights::GRANT));
        assert!(!reduced.contains(token::Rights::REVOKE));
    }

    #[test]
    fn test_inheritance_policies() {
        let cap = token::CapabilityToken::new(123, 1, 0, 0);

        // None policy
        assert!(!inheritance::should_inherit(
            cap,
            inheritance::InheritanceFlags::INHERITABLE,
            inheritance::InheritancePolicy::None
        ));

        // All policy
        assert!(inheritance::should_inherit(
            cap,
            0,
            inheritance::InheritancePolicy::All
        ));

        // Inheritable policy
        assert!(inheritance::should_inherit(
            cap,
            inheritance::InheritanceFlags::INHERITABLE,
            inheritance::InheritancePolicy::Inheritable
        ));
        assert!(!inheritance::should_inherit(
            cap,
            0,
            inheritance::InheritancePolicy::Inheritable
        ));
    }

    fn entry(id: u64, flags: u32) -> space::CapabilityEntry {
        space::CapabilityEntry::new(
            token::CapabilityToken::new(id, 1, 0, 0),
            object::ObjectRef::Process { pid: ProcessId(id) },
            token::Rights::READ | token::Rights::GRANT,
        )
        .with_flags(flags)
    }

    /// exec used to scan IDs 0..256 only. IDs are global, so after the
    /// first 256 capabilities system-wide every PRESERVE_EXEC capability
    /// was silently dropped.
    #[test]
    fn test_exec_keeps_preserved_capabilities_of_any_id() {
        use inheritance::InheritanceFlags as F;
        let old = space::CapabilitySpace::new();
        old.insert_entry(entry(7, F::PRESERVE_EXEC)).unwrap();
        old.insert_entry(entry(5000, F::PRESERVE_EXEC)).unwrap();
        old.insert_entry(entry(1 << 40, F::PRESERVE_EXEC | F::REDUCE_RIGHTS))
            .unwrap();
        old.insert_entry(entry(6000, F::INHERITABLE)).unwrap();

        let new = space::CapabilitySpace::new();
        inheritance::exec_inherit_capabilities(&old, &new).unwrap();
        let tok = |id| token::CapabilityToken::new(id, 1, 0, 0);
        assert!(new.lookup(tok(7)).is_some());
        assert!(new.lookup(tok(5000)).is_some());
        assert_eq!(new.lookup(tok(1 << 40)), Some(token::Rights::READ));
        assert!(new.lookup(tok(6000)).is_none());
        assert_eq!(new.used(), 3);
    }

    /// fork copies every capability and keeps its inheritance flags, so a
    /// capability that must not survive exec still won't in the child.
    #[test]
    fn test_fork_copies_all_with_flags() {
        use inheritance::InheritanceFlags as F;
        let parent = space::CapabilitySpace::new();
        parent.insert_entry(entry(3, 0)).unwrap();
        parent
            .insert_entry(entry(90_000, F::PRESERVE_EXEC))
            .unwrap();
        let child = space::CapabilitySpace::new();
        inheritance::fork_inherit_capabilities(&parent, &child).unwrap();
        assert_eq!(child.used(), 2);
        assert_eq!(child.get_entry(3).unwrap().inheritance_flags, 0);
        assert_eq!(
            child.get_entry(90_000).unwrap().inheritance_flags,
            F::PRESERVE_EXEC
        );
        // Spawn (Inheritable policy) takes only the INHERITABLE one.
        let spawned = space::CapabilitySpace::new();
        let n = inheritance::inherit_capabilities(
            &parent,
            &spawned,
            inheritance::InheritancePolicy::Inheritable,
        )
        .unwrap();
        assert_eq!(n, 0);
    }
}

mod integration_tests {
    use super::*;

    #[test]
    fn test_ipc_capability_integration() {
        let cap_space = space::CapabilitySpace::new();

        // Create IPC endpoint capability
        let endpoint_id: crate::ipc::EndpointId = 123;
        let owner = ProcessId(456);
        let rights = ipc_integration::IpcRights::SEND | ipc_integration::IpcRights::RECEIVE;

        let cap =
            ipc_integration::create_endpoint_capability(endpoint_id, owner, rights, &cap_space)
                .unwrap();

        // Check permissions
        assert!(ipc_integration::check_send_permission(cap, &cap_space).is_ok());
        assert!(ipc_integration::check_receive_permission(cap, &cap_space).is_ok());
        assert!(ipc_integration::check_bind_permission(cap, &cap_space).is_err());
    }

    #[test]
    fn test_memory_capability_integration() {
        let cap_space = space::CapabilitySpace::new();

        // Create memory capability
        let phys_addr = 0x1000usize;
        let size = 4096;
        let attrs = object::MemoryAttributes::normal();
        let rights =
            memory_integration::MemoryRights::READ | memory_integration::MemoryRights::WRITE;

        let cap = memory_integration::create_memory_capability(
            phys_addr, size, attrs, rights, &cap_space,
        )
        .unwrap();

        // Check permissions
        assert!(memory_integration::check_read_permission(cap, &cap_space).is_ok());
        assert!(memory_integration::check_write_permission(cap, &cap_space).is_ok());
        assert!(memory_integration::check_execute_permission(cap, &cap_space).is_err());
    }
}

mod security_tests {
    use super::*;

    #[test]
    fn test_capability_forgery_prevention() {
        let cap_space = space::CapabilitySpace::new();

        // Create a forged capability (not through manager)
        let forged_cap = token::CapabilityToken::new(999999, 1, 0, 0xF);

        // Should not be found in capability space
        assert!(cap_space.lookup(forged_cap).is_none());

        // Should fail capability check
        assert!(manager::check_capability(forged_cap, token::Rights::READ, &cap_space).is_err());
    }

    #[test]
    fn test_insufficient_rights() {
        let cap_space = space::CapabilitySpace::new();
        let obj = object::ObjectRef::Process { pid: ProcessId(1) };

        // Create read-only capability
        let cap = manager::cap_manager()
            .create_capability(obj, token::Rights::READ, &cap_space)
            .unwrap();

        // Try to check write permission
        let result = manager::check_capability(cap, token::Rights::WRITE, &cap_space);

        assert!(matches!(result, Err(manager::CapError::InsufficientRights)));
    }

    #[test]
    fn test_delegation_without_grant() {
        let source_space = space::CapabilitySpace::new();
        let target_space = space::CapabilitySpace::new();
        let obj = object::ObjectRef::Process { pid: ProcessId(1) };

        // Create capability without grant permission
        let cap = manager::cap_manager()
            .create_capability(obj, token::Rights::READ, &source_space)
            .unwrap();

        // Try to delegate
        let result =
            manager::cap_manager().delegate(cap, &source_space, &target_space, token::Rights::READ);

        assert!(matches!(result, Err(manager::CapError::PermissionDenied)));
    }
}
