//! Object-capability authority behind Linux privilege checks (ADR 0024).
//!
//! A POSIX capability, uid or mode-bit check says whether a caller is
//! privileged enough; the object capability checked here says whether it
//! may act on the object at all, and both must hold. A program the kernel
//! starts holds every such capability for its namespaces, the clocks and
//! the system ([`grant_initial`]), keeps them across fork and exec, and
//! can drop them ([`sys_capability_drop`]) to confine what it runs next.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{
    inheritance::InheritanceFlags,
    manager::cap_manager,
    object::{ClockObject, ObjectRef, TYPE_CLOCK, TYPE_NAMESPACE, TYPE_SYSTEM},
    space::CapabilitySpace,
    token::{CapabilityToken, Rights},
};

/// The live capabilities in `space` that `select` accepts, with their
/// rights. Collected under the space's lock and checked for revocation
/// after it, so the manager's registry is never locked inside the space's.
#[cfg(feature = "alloc")]
fn matching(
    space: &CapabilitySpace,
    mut select: impl FnMut(&ObjectRef) -> bool,
) -> Vec<(CapabilityToken, Rights)> {
    let mut found = Vec::new();
    let _ = space.iter_capabilities(|entry| {
        if select(&entry.object) {
            found.push((entry.capability, entry.rights));
        }
        true
    });
    found
}

/// Whether `space` holds a live (not revoked) capability naming exactly
/// `object` with at least `rights`. Unlike `manager::check_capability`, the
/// object's kind and identity are part of the check.
#[cfg(feature = "alloc")]
pub fn holds(space: &CapabilitySpace, object: &ObjectRef, rights: Rights) -> bool {
    matching(space, |o| o == object)
        .into_iter()
        .any(|(cap, held)| held.contains(rights) && cap_manager().is_valid(cap))
}

/// [`holds`] for the calling process. Kernel context (no current process)
/// has every authority: the kernel acts for itself there, not for a program.
#[cfg(feature = "alloc")]
pub fn current_holds(object: &ObjectRef, rights: Rights) -> bool {
    match crate::process::current_process() {
        Some(process) => holds(&process.capability_space.lock(), object, rights),
        None => true,
    }
}

/// The objects a program the kernel starts has authority over: the
/// namespaces it is in (`user` and `pid` are its process's, the others its
/// main thread's), both clocks, and the system.
#[cfg(feature = "alloc")]
pub fn initial_objects(
    ns: &crate::process::ns::Namespaces,
    user: crate::process::ns::NsIdent,
    pid: crate::process::ns::NsIdent,
) -> Vec<ObjectRef> {
    use crate::process::ns::NsKind;
    let mut idents: Vec<crate::process::ns::NsIdent> = NsKind::ALL
        .into_iter()
        .filter(|k| !matches!(k, NsKind::User | NsKind::Pid))
        .map(|k| ns.get(k).ident())
        .collect();
    // The current time namespace too, when children get another one.
    idents.push(ns.time.ident);
    idents.push(user);
    idents.push(pid);
    let mut objects: Vec<ObjectRef> = Vec::new();
    for ident in idents {
        let object = ObjectRef::Namespace {
            kind: ident.kind,
            id: ident.id,
        };
        if !objects.contains(&object) {
            objects.push(object);
        }
    }
    objects.push(ObjectRef::Clock {
        clock: ClockObject::System,
    });
    objects.push(ObjectRef::Clock {
        clock: ClockObject::Alarm,
    });
    objects.push(ObjectRef::System);
    objects
}

/// Give `space` a capability with every right over each of `objects`,
/// inherited at fork and kept across exec. An object `space` already holds
/// with every right is skipped.
#[cfg(feature = "alloc")]
pub fn grant(space: &CapabilitySpace, objects: &[ObjectRef]) -> Result<(), super::CapError> {
    for object in objects {
        if holds(space, object, Rights::ALL) {
            continue;
        }
        cap_manager().create_capability_with_flags(
            object.clone(),
            Rights::ALL,
            space,
            InheritanceFlags::INHERITABLE | InheritanceFlags::PRESERVE_EXEC,
        )?;
    }
    Ok(())
}

/// [`grant`] of [`initial_objects`] to a process the kernel starts.
#[cfg(feature = "alloc")]
pub fn grant_initial(
    space: &CapabilitySpace,
    ns: &crate::process::ns::Namespaces,
    user: crate::process::ns::NsIdent,
    pid: crate::process::ns::NsIdent,
) -> Result<(), super::CapError> {
    grant(space, &initial_objects(ns, user, pid))
}

/// What a `CapabilityDrop` call names: an object kind (the token's type
/// code) and an id within it.
#[cfg(feature = "alloc")]
fn drop_selector(kind: usize, id: usize) -> Option<impl Fn(&ObjectRef) -> bool> {
    if kind > u8::MAX as usize {
        return None;
    }
    // Namespace ids are unique across kinds, so the id alone names one.
    let wanted = match kind as u8 {
        TYPE_NAMESPACE if id > 0 => DropTarget::Namespace(id as u64),
        TYPE_CLOCK if id == 0 => DropTarget::Clock(ClockObject::System),
        TYPE_CLOCK if id == 1 => DropTarget::Clock(ClockObject::Alarm),
        TYPE_SYSTEM if id == 0 => DropTarget::System,
        _ => return None,
    };
    Some(move |object: &ObjectRef| match (wanted, object) {
        (DropTarget::Namespace(want), ObjectRef::Namespace { id, .. }) => *id == want,
        (DropTarget::Clock(want), ObjectRef::Clock { clock }) => *clock == want,
        (DropTarget::System, ObjectRef::System) => true,
        _ => false,
    })
}

#[derive(Clone, Copy)]
enum DropTarget {
    Namespace(u64),
    Clock(ClockObject),
    System,
}

/// Remove from `space` every capability naming the object that `kind` and
/// `id` select; how many were removed, or None for a selector naming no
/// kind of object that can be dropped.
#[cfg(feature = "alloc")]
pub fn drop_object(space: &CapabilitySpace, kind: usize, id: usize) -> Option<usize> {
    let select = drop_selector(kind, id)?;
    let tokens = matching(space, select);
    Some(
        tokens
            .into_iter()
            .filter(|(cap, _)| space.remove(*cap).is_some())
            .count(),
    )
}

/// `CapabilityDrop(kind, id)`: delete from the caller's own space every
/// capability naming that object (ADR 0024). Unlike `CapabilityRevoke`,
/// which revokes the token in every space holding a copy, this affects
/// only the caller (and what it starts afterwards). Returns the number
/// removed; EINVAL for a kind and id naming nothing that can be dropped.
#[cfg(feature = "alloc")]
pub fn sys_capability_drop(kind: usize, id: usize) -> crate::syscall::SyscallResult {
    let process =
        crate::process::current_process().ok_or(crate::syscall::SyscallError::InvalidState)?;
    let space = process.capability_space.lock();
    drop_object(&space, kind, id).ok_or(crate::syscall::SyscallError::InvalidArgument)
}

#[cfg(all(test, feature = "alloc", not(target_os = "none")))]
mod tests {
    use super::*;
    use crate::process::ns::{self, NsKind};

    fn initial_space() -> CapabilitySpace {
        let space = CapabilitySpace::new();
        let set = ns::initial();
        let user = set.get(NsKind::User).ident();
        let pid = set.pid_for_children.ident;
        grant_initial(&space, &set, user, pid).unwrap();
        space
    }

    #[test]
    fn a_started_program_holds_its_namespaces_the_clocks_and_the_system() {
        let space = initial_space();
        let set = ns::initial();
        for kind in NsKind::ALL {
            let id = if kind == NsKind::Pid {
                set.pid_for_children.ident.id
            } else {
                set.get(kind).ident().id
            };
            assert!(
                holds(&space, &ObjectRef::Namespace { kind, id }, Rights::ALL),
                "{kind:?}"
            );
        }
        for clock in [ClockObject::System, ClockObject::Alarm] {
            assert!(holds(&space, &ObjectRef::Clock { clock }, Rights::MODIFY));
        }
        assert!(holds(&space, &ObjectRef::System, Rights::MODIFY));
        // Eight namespaces, two clocks, the system.
        assert_eq!(space.used(), 11);
    }

    #[test]
    fn granting_twice_adds_nothing() {
        let space = initial_space();
        let set = ns::initial();
        let user = set.get(NsKind::User).ident();
        grant_initial(&space, &set, user, set.pid_for_children.ident).unwrap();
        assert_eq!(space.used(), 11);
    }

    #[test]
    fn the_check_matches_kind_and_identity() {
        let space = initial_space();
        let mnt = ns::initial().mnt.ident;
        // Another namespace of the kind, and the same id as another kind.
        assert!(!holds(
            &space,
            &ObjectRef::Namespace {
                kind: NsKind::Mnt,
                id: mnt.id + 1000,
            },
            Rights::READ
        ));
        assert!(!holds(
            &space,
            &ObjectRef::Namespace {
                kind: NsKind::Uts,
                id: mnt.id,
            },
            Rights::READ
        ));
        assert!(!holds(
            &CapabilitySpace::new(),
            &ObjectRef::System,
            Rights::READ
        ));
    }

    #[test]
    fn a_revoked_capability_gives_no_authority() {
        let space = CapabilitySpace::new();
        let cap = cap_manager()
            .create_capability(ObjectRef::System, Rights::ALL, &space)
            .unwrap();
        assert!(holds(&space, &ObjectRef::System, Rights::MODIFY));
        cap_manager().revoke(cap).unwrap();
        assert!(!holds(&space, &ObjectRef::System, Rights::MODIFY));
    }

    #[test]
    fn rights_must_cover_the_request() {
        let space = CapabilitySpace::new();
        cap_manager()
            .create_capability(ObjectRef::System, Rights::READ, &space)
            .unwrap();
        assert!(holds(&space, &ObjectRef::System, Rights::READ));
        assert!(!holds(&space, &ObjectRef::System, Rights::MODIFY));
    }

    #[test]
    fn the_grants_survive_exec_and_fork() {
        let space = initial_space();
        let after_exec = CapabilitySpace::new();
        crate::cap::inheritance::exec_inherit_capabilities(&space, &after_exec).unwrap();
        assert_eq!(after_exec.used(), 11);
        let child = CapabilitySpace::new();
        child.clone_from(&after_exec).unwrap();
        assert!(holds(&child, &ObjectRef::System, Rights::MODIFY));
    }

    #[test]
    fn drop_removes_only_the_named_object_from_this_space() {
        let space = initial_space();
        let child = CapabilitySpace::new();
        child.clone_from(&space).unwrap();
        let clock = ObjectRef::Clock {
            clock: ClockObject::System,
        };
        assert_eq!(drop_object(&child, TYPE_CLOCK as usize, 0), Some(1));
        assert!(!holds(&child, &clock, Rights::READ));
        assert!(holds(&child, &ObjectRef::System, Rights::MODIFY));
        // The parent's copy of the same capability is untouched.
        assert!(holds(&space, &clock, Rights::MODIFY));
        // Dropping again removes nothing.
        assert_eq!(drop_object(&child, TYPE_CLOCK as usize, 0), Some(0));

        let mnt = ns::initial().mnt.ident.id as usize;
        assert_eq!(drop_object(&child, TYPE_NAMESPACE as usize, mnt), Some(1));
        assert_eq!(drop_object(&child, TYPE_SYSTEM as usize, 0), Some(1));
        assert_eq!(child.used(), 8);
    }

    #[test]
    fn drop_rejects_what_cannot_be_named() {
        let space = initial_space();
        assert_eq!(drop_object(&space, 0, 0), None, "Memory");
        assert_eq!(drop_object(&space, TYPE_NAMESPACE as usize, 0), None);
        assert_eq!(drop_object(&space, TYPE_CLOCK as usize, 2), None);
        assert_eq!(drop_object(&space, TYPE_SYSTEM as usize, 1), None);
        assert_eq!(drop_object(&space, 0x100 + TYPE_SYSTEM as usize, 0), None);
        assert_eq!(space.used(), 11);
    }
}
