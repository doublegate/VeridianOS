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

    #[test]
    fn test_l1_and_l2_tables() {
        let cap_space = space::CapabilitySpace::new();

        // Test L1 table (ID < 256)
        let cap_l1 = token::CapabilityToken::new(100, 1, 0, 0);
        let obj = object::ObjectRef::Process { pid: ProcessId(1) };
        let rights = token::Rights::READ;

        assert!(cap_space.insert(cap_l1, obj.clone(), rights).is_ok());
        assert!(cap_space.lookup(cap_l1).is_some());

        // Test L2 table (ID >= 256)
        #[cfg(feature = "alloc")]
        {
            let cap_l2 = token::CapabilityToken::new(1000, 1, 0, 0);
            assert!(cap_space.insert(cap_l2, obj, rights).is_ok());
            assert!(cap_space.lookup(cap_l2).is_some());
        }
    }
}

mod manager_tests {
    use super::*;

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
