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

/// The objects whose capabilities stand for namespace `ns`: `ns` itself,
/// then the user namespace that owns it and each of that one's ancestors.
#[cfg(feature = "alloc")]
fn standing_for(ns: &crate::process::ns::NsRef) -> Vec<ObjectRef> {
    use crate::process::ns::NsKind;
    let ident = ns.ident();
    let mut wanted = Vec::new();
    wanted.push(ObjectRef::Namespace {
        kind: ident.kind,
        id: ident.id,
    });
    let mut owner = ns.owner();
    while let Some(user) = owner {
        wanted.push(ObjectRef::Namespace {
            kind: NsKind::User,
            id: user.ident.id,
        });
        owner = user.parent.clone();
    }
    wanted
}

/// Whether `space` has `rights` over namespace `ns` (ADR 0024, decision 7):
/// through a capability on `ns` itself, or on the user namespace that owns
/// it or one of that one's ancestors, as Linux's `ns_capable(ns->owner, ..)`
/// reaches every namespace a user namespace owns. For a user namespace the
/// owner is its parent.
#[cfg(feature = "alloc")]
pub fn holds_namespace(
    space: &CapabilitySpace,
    ns: &crate::process::ns::NsRef,
    rights: Rights,
) -> bool {
    let wanted = standing_for(ns);
    matching(space, |o| wanted.contains(o))
        .into_iter()
        .any(|(cap, held)| held.contains(rights) && cap_manager().is_valid(cap))
}

/// The rights over mount namespace `mnt` that file access needs, read
/// from `space` in one pass (ADR 0024, step 6): each of READ, WRITE and
/// CREATE is held if some live capability on `mnt`, or on a user namespace
/// above it, carries it. Checking each right on its own is all a file
/// check asks, so the union over the capabilities is the same answer as
/// one [`holds_namespace`] per right.
#[cfg(feature = "alloc")]
pub fn fs_authority(
    space: &CapabilitySpace,
    mnt: &crate::process::ns::NsRef,
) -> crate::process::creds::FsAuthority {
    let wanted = standing_for(mnt);
    let mut held = Rights::new(0);
    for (cap, rights) in matching(space, |o| wanted.contains(o)) {
        if cap_manager().is_valid(cap) {
            held |= rights;
        }
    }
    crate::process::creds::FsAuthority {
        read: held.contains(Rights::READ),
        write: held.contains(Rights::WRITE),
        create: held.contains(Rights::CREATE),
    }
}

/// [`holds_namespace`] for the calling process; kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_holds_namespace(ns: &crate::process::ns::NsRef, rights: Rights) -> bool {
    match crate::process::current_process() {
        Some(process) => holds_namespace(&process.capability_space.lock(), ns, rights),
        None => true,
    }
}

/// Whether the caller may change the mounts of the mount namespace it is
/// in: a capability with MODIFY on it, or on a user namespace above it
/// (ADR 0024, decision 8). mount(2), umount2, pivot_root and the private
/// mount and umount calls check this after CAP_SYS_ADMIN; EPERM when it
/// fails. Kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_may_change_mounts() -> bool {
    let Some(thread) = crate::process::current_thread() else {
        return true;
    };
    let mnt = crate::process::ns::NsRef::Mnt(thread.namespaces().mnt.clone());
    current_holds_namespace(&mnt, Rights::MODIFY)
}

/// Whether `space` may create the namespaces `flags` asks for (ADR 0024,
/// step 3): CREATE on each namespace a new one is made from -- the one of
/// that kind in `current`, for PID and time the one for children, below
/// which a new one is made -- and, for CLONE_NEWUSER, on `user`, the
/// caller's user namespace. A capability on an owning user namespace
/// stands for the namespaces it owns ([`holds_namespace`]). unshare and
/// clone check this after their POSIX checks; EPERM when it fails.
#[cfg(feature = "alloc")]
pub fn may_create_namespaces(
    space: &CapabilitySpace,
    current: &crate::process::ns::Namespaces,
    user: &alloc::sync::Arc<crate::process::userns::UserNs>,
    flags: u64,
) -> bool {
    use crate::process::ns::{NsKind, NsRef};
    NsKind::ALL
        .into_iter()
        .filter(|kind| flags & kind.clone_flag() != 0)
        .all(|kind| {
            let parent = if kind == NsKind::User {
                NsRef::User(user.clone())
            } else {
                current.get(kind)
            };
            holds_namespace(space, &parent, Rights::CREATE)
        })
}

/// [`may_create_namespaces`] for the calling process; kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_may_create_namespaces(
    current: &crate::process::ns::Namespaces,
    user: &alloc::sync::Arc<crate::process::userns::UserNs>,
    flags: u64,
) -> bool {
    match crate::process::current_process() {
        Some(process) => {
            may_create_namespaces(&process.capability_space.lock(), current, user, flags)
        }
        None => true,
    }
}

/// Whether `space` may use `clock` as a privileged caller would (ADR 0024,
/// step 4): MODIFY on the system clock to set or steer it (clock_settime,
/// settimeofday, adjtimex and clock_adjtime past reading), WRITE on the
/// alarm clocks to arm a timer on one. Checked after CAP_SYS_TIME and
/// CAP_WAKE_ALARM; EPERM when it fails.
#[cfg(feature = "alloc")]
pub fn may_use_clock(space: &CapabilitySpace, clock: ClockObject) -> bool {
    let rights = match clock {
        ClockObject::System => Rights::MODIFY,
        ClockObject::Alarm => Rights::WRITE,
    };
    holds(space, &ObjectRef::Clock { clock }, rights)
}

/// [`may_use_clock`] for the calling process; kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_may_use_clock(clock: ClockObject) -> bool {
    match crate::process::current_process() {
        Some(process) => may_use_clock(&process.capability_space.lock(), clock),
        None => true,
    }
}

/// Whether `space` has `rights` over the process `pid` in PID namespace
/// `pid_ns` (ADR 0024, step 5): a capability on `Process { pid }` with
/// them, or on the PID namespace, itself or through a user namespace above
/// it ([`holds_namespace`]). WRITE to signal it; MODIFY to trace it, read
/// its `/proc` files as ptrace allows, change its scheduling or its
/// limits. Checked after the Linux rule (uid match, CAP_KILL,
/// CAP_SYS_PTRACE, CAP_SYS_NICE, CAP_SYS_RESOURCE); EPERM when it fails.
#[cfg(feature = "alloc")]
pub fn may_act_on_process(
    space: &CapabilitySpace,
    pid: crate::process::ProcessId,
    pid_ns: &alloc::sync::Arc<crate::process::pidns::PidNs>,
    rights: Rights,
) -> bool {
    holds(space, &ObjectRef::Process { pid }, rights)
        || holds_namespace(
            space,
            &crate::process::ns::NsRef::Pid(pid_ns.clone()),
            rights,
        )
}

/// [`may_act_on_process`] for the calling process acting on `target`. A
/// process acting on itself needs neither, as Linux never asks for
/// privilege there; kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_may_act_on_process(target: &crate::process::Process, rights: Rights) -> bool {
    current_may_act_on(target.pid, &target.pid_ns(), rights)
}

/// [`current_may_act_on_process`] for the process `pid` in `pid_ns`, for a
/// caller that found it without keeping it (under the process table's
/// lock, which the capability check must not be made under).
#[cfg(feature = "alloc")]
pub fn current_may_act_on(
    pid: crate::process::ProcessId,
    pid_ns: &alloc::sync::Arc<crate::process::pidns::PidNs>,
    rights: Rights,
) -> bool {
    let Some(me) = crate::process::current_process() else {
        return true;
    };
    me.pid == pid || may_act_on_process(&me.capability_space.lock(), pid, pid_ns, rights)
}

/// Whether `space` may choose the child's numbers in the `levels` PID
/// namespaces from `ns` upward (clone3's set_tid, ADR 0024 step 5): MODIFY
/// on each, itself or through a user namespace above it. Checked after
/// CAP_CHECKPOINT_RESTORE or CAP_SYS_ADMIN; EPERM when it fails.
#[cfg(feature = "alloc")]
pub fn may_choose_pids(
    space: &CapabilitySpace,
    ns: &alloc::sync::Arc<crate::process::pidns::PidNs>,
    levels: usize,
) -> bool {
    let mut level = Some(ns.clone());
    for _ in 0..levels {
        let Some(pid_ns) = level else {
            return true;
        };
        if !holds_namespace(
            space,
            &crate::process::ns::NsRef::Pid(pid_ns.clone()),
            Rights::MODIFY,
        ) {
            return false;
        }
        level = pid_ns.parent.clone();
    }
    true
}

/// [`may_choose_pids`] for the calling process; kernel context passes.
#[cfg(feature = "alloc")]
pub fn current_may_choose_pids(
    ns: &alloc::sync::Arc<crate::process::pidns::PidNs>,
    levels: usize,
) -> bool {
    match crate::process::current_process() {
        Some(process) => may_choose_pids(&process.capability_space.lock(), ns, levels),
        None => true,
    }
}

/// Give the process whose capability space is `space` [`authority_rights`]
/// over user namespace `ns`, which it is creating (ADR 0024, decision 7): the
/// namespaces made inside it then belong to a user namespace it has
/// authority over, as the owner has on Linux.
///
/// Its capabilities on user namespaces that no longer exist go first
/// ([`drop_dead_user_ns`]): a launcher making one per child it then reaps
/// would else fill its space's quota after a few hundred (PR #39 review).
#[cfg(feature = "alloc")]
pub fn grant_user_ns(
    space: &CapabilitySpace,
    ns: &crate::process::userns::UserNs,
) -> Result<(), super::CapError> {
    drop_dead_user_ns(space);
    grant(
        space,
        &[ObjectRef::Namespace {
            kind: crate::process::ns::NsKind::User,
            id: ns.ident.id,
        }],
    )
}

/// Remove from `space` its capabilities on user namespaces that are gone.
/// Namespace ids are never reused, so such a capability names nothing and
/// can name nothing later. Returns how many were removed.
#[cfg(feature = "alloc")]
pub fn drop_dead_user_ns(space: &CapabilitySpace) -> usize {
    use crate::process::ns::NsKind;
    matching(space, |o| {
        matches!(o, ObjectRef::Namespace { kind: NsKind::User, id }
            if !crate::process::userns::is_live(*id))
    })
    .into_iter()
    .filter(|(cap, _)| space.remove(*cap).is_some())
    .count()
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

/// The rights these authority capabilities carry: every right a check asks
/// for (READ, WRITE, EXECUTE, MODIFY, CREATE), but not GRANT, REVOKE or
/// DELETE. Fork copies the token itself into the child, so a holder that
/// could revoke it (`CapabilityRevoke` needs only REVOKE on its own copy)
/// would revoke its parent's and siblings' copies too; a process gives up
/// its own authority with `CapabilityDrop` instead (ADR 0025).
pub fn authority_rights() -> Rights {
    Rights::ALL
        .remove(Rights::GRANT)
        .remove(Rights::REVOKE)
        .remove(Rights::DELETE)
}

/// Give `space` a capability with [`authority_rights`] over each of
/// `objects`, inherited at fork and kept across exec. An object `space`
/// already holds with those rights is skipped.
#[cfg(feature = "alloc")]
pub fn grant(space: &CapabilitySpace, objects: &[ObjectRef]) -> Result<(), super::CapError> {
    for object in objects {
        if holds(space, object, authority_rights()) {
            continue;
        }
        // Only spaces hold these, so the registry forgets one once no space
        // does: every launched program gets its own (PR #39 review).
        cap_manager().create_space_only_capability(
            object.clone(),
            authority_rights(),
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
/// kind of object that can be dropped. [`drop_rights`] with no mask.
#[cfg(feature = "alloc")]
pub fn drop_object(space: &CapabilitySpace, kind: usize, id: usize) -> Option<usize> {
    drop_rights(space, kind, id, 0)
}

/// Take the rights `mask` (the token's rights encoding) off every
/// capability in `space` naming the object `kind` and `id` select (ADR
/// 0026): mask 0 removes each whole, as does a mask covering every right it
/// has; otherwise each keeps its rights minus the mask, and one left with
/// none is removed. Never adds a right. Returns how many were narrowed or
/// removed (one already without those rights is not counted), or None for
/// a selector naming nothing that can be dropped or a mask with bits above
/// CREATE.
#[cfg(feature = "alloc")]
pub fn drop_rights(space: &CapabilitySpace, kind: usize, id: usize, mask: usize) -> Option<usize> {
    if mask > Rights::ALL.bits() as usize {
        return None;
    }
    let select = drop_selector(kind, id)?;
    let mask = if mask == 0 {
        Rights::ALL
    } else {
        Rights::new(mask as u32)
    };
    let tokens = matching(space, select);
    Some(
        tokens
            .into_iter()
            .filter(|(cap, held)| {
                if held.intersection(mask).is_empty() {
                    return false;
                }
                match space.narrow(*cap, mask) {
                    Some(left) if left.is_empty() => space.remove(*cap).is_some(),
                    Some(_) => true,
                    None => false,
                }
            })
            .count(),
    )
}

/// `CapabilityDrop(kind, id, rights)`: take `rights` off, or with 0 delete,
/// every capability in the caller's own space naming that object (ADR 0024,
/// ADR 0026). Unlike `CapabilityRevoke`, which revokes the token in every
/// space holding a copy, this affects only the caller (and what it starts
/// afterwards). Returns the number narrowed or removed; EINVAL for a kind
/// and id naming nothing that can be dropped, or rights bits above CREATE.
#[cfg(feature = "alloc")]
pub fn sys_capability_drop(kind: usize, id: usize, rights: usize) -> crate::syscall::SyscallResult {
    let process =
        crate::process::current_process().ok_or(crate::syscall::SyscallError::InvalidState)?;
    let space = process.capability_space.lock();
    drop_rights(&space, kind, id, rights).ok_or(crate::syscall::SyscallError::InvalidArgument)
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
                holds(
                    &space,
                    &ObjectRef::Namespace { kind, id },
                    authority_rights()
                ),
                "{kind:?}"
            );
            assert!(!holds(
                &space,
                &ObjectRef::Namespace { kind, id },
                Rights::REVOKE
            ));
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
    fn setting_the_clock_needs_modify_and_an_alarm_timer_write() {
        let space = initial_space();
        assert!(may_use_clock(&space, ClockObject::System));
        assert!(may_use_clock(&space, ClockObject::Alarm));
        // Dropping the system clock leaves the alarm clocks, and the reverse.
        let child = CapabilitySpace::new();
        child.clone_from(&space).unwrap();
        assert_eq!(drop_object(&child, TYPE_CLOCK as usize, 0), Some(1));
        assert!(!may_use_clock(&child, ClockObject::System));
        assert!(may_use_clock(&child, ClockObject::Alarm));
        assert_eq!(drop_object(&child, TYPE_CLOCK as usize, 1), Some(1));
        assert!(!may_use_clock(&child, ClockObject::Alarm));
        // WRITE arms an alarm timer, but setting the clock needs MODIFY.
        let write_only = CapabilitySpace::new();
        for clock in [ClockObject::System, ClockObject::Alarm] {
            cap_manager()
                .create_capability(ObjectRef::Clock { clock }, Rights::WRITE, &write_only)
                .unwrap();
        }
        assert!(!may_use_clock(&write_only, ClockObject::System));
        assert!(may_use_clock(&write_only, ClockObject::Alarm));
        // The system's authority does not stand for a clock.
        let system_only = CapabilitySpace::new();
        cap_manager()
            .create_capability(ObjectRef::System, Rights::ALL, &system_only)
            .unwrap();
        assert!(!may_use_clock(&system_only, ClockObject::System));
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

    /// A launcher making a user namespace per child and reaping each one
    /// keeps a bounded space: the capability on a namespace that is gone
    /// goes before the next grant (PR #39 review: 245 such clones used to
    /// fill the 256-entry quota, and the next clone failed).
    #[test]
    fn grants_on_user_namespaces_that_are_gone_are_reclaimed() {
        use crate::process::userns;
        let init = userns::initial();
        let space = CapabilitySpace::new();
        let user_caps = |space: &CapabilitySpace| {
            matching(space, |o| {
                matches!(
                    o,
                    ObjectRef::Namespace {
                        kind: NsKind::User,
                        ..
                    }
                )
            })
            .len()
        };
        for _ in 0..300 {
            let ns = userns::UserNs::create(&init, 1000, 1000, false, false).unwrap();
            grant_user_ns(&space, &ns).unwrap();
            assert!(holds(
                &space,
                &ObjectRef::Namespace {
                    kind: NsKind::User,
                    id: ns.ident.id,
                },
                Rights::CREATE
            ));
            assert!(userns::is_live(ns.ident.id));
            let id = ns.ident.id;
            drop(ns);
            assert!(!userns::is_live(id));
            assert!(user_caps(&space) <= 1, "only the last one is left");
        }
        // One that is still there is kept.
        let live = userns::UserNs::create(&init, 1000, 1000, false, false).unwrap();
        grant_user_ns(&space, &live).unwrap();
        let other = userns::UserNs::create(&init, 1000, 1000, false, false).unwrap();
        grant_user_ns(&space, &other).unwrap();
        assert_eq!(user_caps(&space), 2);
        assert!(userns::is_live(init.ident.id), "the initial one always is");
    }

    /// The registry forgets an authority capability once no space holds
    /// it, so programs started one after another do not grow it (PR #39
    /// review); while a forked copy holds one, it stays.
    #[test]
    fn an_authority_capability_no_space_holds_leaves_the_registry() {
        let parent = initial_space();
        let mut ids = Vec::new();
        let _ = parent.iter_capabilities(|entry| {
            ids.push(entry.capability.id());
            true
        });
        assert_eq!(ids.len(), 11);
        let child = CapabilitySpace::new();
        child.clone_from(&parent).unwrap();
        drop(parent);
        assert!(ids.iter().all(|&id| cap_manager().is_registered(id)));
        assert!(holds(&child, &ObjectRef::System, Rights::MODIFY));
        drop(child);
        assert!(ids.iter().all(|&id| !cap_manager().is_registered(id)));
    }

    #[test]
    fn a_forked_copy_cannot_revoke_the_parents_authority() {
        // Fork copies the token; a revocable copy would let a confined child
        // revoke the token in its parent's space too.
        let parent = initial_space();
        let child = CapabilitySpace::new();
        child.clone_from(&parent).unwrap();
        let mut token = None;
        let _ = child.iter_capabilities(|entry| {
            if entry.object == ObjectRef::System {
                token = Some(entry.capability);
            }
            true
        });
        assert!(crate::cap::revocation::revoke_held(&child, token.unwrap()).is_err());
        assert!(holds(&parent, &ObjectRef::System, Rights::MODIFY));
        assert!(holds(&child, &ObjectRef::System, Rights::MODIFY));
    }

    #[test]
    fn authority_reaches_what_a_user_namespace_owns_and_below() {
        use crate::process::{ns::NsRef, userns};
        let init = userns::initial();
        let child = userns::UserNs::create(&init, 1000, 1000, false, false).unwrap();
        let set = ns::initial()
            .copy_for(
                ns::CLONE_NEWUTS,
                &child,
                &crate::process::pidns::initial(),
                1000,
            )
            .unwrap();
        let uts = set.get(NsKind::Uts);
        let space = CapabilitySpace::new();
        assert!(!holds_namespace(&space, &uts, Rights::MODIFY));

        // The creator's grant covers the namespace itself and what it owns.
        grant_user_ns(&space, &child).unwrap();
        assert!(holds_namespace(&space, &uts, Rights::MODIFY));
        assert!(holds_namespace(
            &space,
            &NsRef::User(child.clone()),
            Rights::MODIFY
        ));
        // Not upward: the parent and what it owns directly.
        assert!(!holds_namespace(
            &space,
            &NsRef::User(init.clone()),
            Rights::READ
        ));
        assert!(!holds_namespace(
            &space,
            &ns::initial().get(NsKind::Uts),
            Rights::READ
        ));

        // An ancestor of the owner reaches it too, and rights still count.
        let root = CapabilitySpace::new();
        cap_manager()
            .create_capability(
                ObjectRef::Namespace {
                    kind: NsKind::User,
                    id: init.ident.id,
                },
                Rights::READ,
                &root,
            )
            .unwrap();
        assert!(holds_namespace(&root, &uts, Rights::READ));
        assert!(!holds_namespace(&root, &uts, Rights::MODIFY));
    }

    #[test]
    fn acting_on_a_process_needs_its_pid_namespace_or_the_process() {
        use crate::process::{pidns, ProcessId};
        let space = initial_space();
        let set = ns::initial();
        let pid_ns = pidns::initial();
        let target = ProcessId(4242);
        assert!(may_act_on_process(&space, target, &pid_ns, Rights::MODIFY));
        // Dropping the PID namespace leaves it standing through the user
        // namespace that owns it; both go to deny it.
        drop_object(&space, TYPE_NAMESPACE as usize, pid_ns.ident.id as usize).unwrap();
        assert!(may_act_on_process(&space, target, &pid_ns, Rights::WRITE));
        let user = set.get(NsKind::User).ident().id as usize;
        drop_object(&space, TYPE_NAMESPACE as usize, user).unwrap();
        assert!(!may_act_on_process(&space, target, &pid_ns, Rights::WRITE));

        // A capability on the process itself, for what its rights cover.
        cap_manager()
            .create_capability(ObjectRef::Process { pid: target }, Rights::WRITE, &space)
            .unwrap();
        assert!(may_act_on_process(&space, target, &pid_ns, Rights::WRITE));
        assert!(!may_act_on_process(&space, target, &pid_ns, Rights::MODIFY));
        assert!(!may_act_on_process(
            &space,
            ProcessId(4243),
            &pid_ns,
            Rights::WRITE
        ));
    }

    #[test]
    fn choosing_pids_needs_modify_on_each_level_chosen() {
        use crate::process::{pidns, userns};
        let init = userns::initial();
        let child_user = userns::UserNs::create(&init, 1000, 1000, false, false).unwrap();
        let outer = pidns::initial();
        let inner = pidns::PidNs::create(&outer, &child_user, 1000).unwrap();
        // Authority over the child user namespace reaches the inner PID
        // namespace only.
        let space = CapabilitySpace::new();
        grant_user_ns(&space, &child_user).unwrap();
        assert!(may_choose_pids(&space, &inner, 0));
        assert!(may_choose_pids(&space, &inner, 1));
        assert!(!may_choose_pids(&space, &inner, 2));
        // A started program holds both levels.
        assert!(may_choose_pids(&initial_space(), &inner, 2));
        // Rights below MODIFY do not do.
        let reader = CapabilitySpace::new();
        cap_manager()
            .create_capability(
                ObjectRef::Namespace {
                    kind: NsKind::Pid,
                    id: outer.ident.id,
                },
                Rights::READ | Rights::WRITE,
                &reader,
            )
            .unwrap();
        assert!(!may_choose_pids(&reader, &outer, 1));
    }

    #[test]
    fn a_dropped_mount_namespace_still_stands_through_its_owner() {
        // What a launcher must drop to deny mounting: the mount namespace and
        // the user namespace above it.
        let space = initial_space();
        let set = ns::initial();
        let mnt = set.get(NsKind::Mnt);
        let user = set.get(NsKind::User).ident().id as usize;
        drop_object(&space, TYPE_NAMESPACE as usize, mnt.ident().id as usize).unwrap();
        assert!(holds_namespace(&space, &mnt, Rights::MODIFY));
        drop_object(&space, TYPE_NAMESPACE as usize, user).unwrap();
        assert!(!holds_namespace(&space, &mnt, Rights::MODIFY));
    }

    #[test]
    fn creating_namespaces_needs_create_on_what_they_are_made_from() {
        use crate::process::{ns::CLONE_NS_ALL, userns};
        let set = ns::initial();
        let init = userns::initial();
        let space = initial_space();
        assert!(may_create_namespaces(&space, &set, &init, CLONE_NS_ALL));
        assert!(may_create_namespaces(
            &CapabilitySpace::new(),
            &set,
            &init,
            0
        ));
        assert!(!may_create_namespaces(
            &CapabilitySpace::new(),
            &set,
            &init,
            ns::CLONE_NEWUTS
        ));

        // A UTS namespace stands through the user namespace that owns it:
        // without both, no new UTS namespace, but the others still come.
        let user = set.get(NsKind::User).ident().id as usize;
        drop_object(&space, TYPE_NAMESPACE as usize, set.uts.ident.id as usize).unwrap();
        assert!(may_create_namespaces(&space, &set, &init, ns::CLONE_NEWUTS));
        drop_object(&space, TYPE_NAMESPACE as usize, user).unwrap();
        assert!(!may_create_namespaces(
            &space,
            &set,
            &init,
            ns::CLONE_NEWUTS
        ));
        assert!(!may_create_namespaces(
            &space,
            &set,
            &init,
            ns::CLONE_NEWUSER
        ));
        assert!(may_create_namespaces(&space, &set, &init, ns::CLONE_NEWNS));
        assert!(!may_create_namespaces(
            &space,
            &set,
            &init,
            ns::CLONE_NEWNS | ns::CLONE_NEWUTS
        ));
    }

    #[test]
    fn a_new_pid_namespace_is_made_from_the_one_for_children() {
        use crate::process::userns;
        let init = userns::initial();
        let set = ns::initial();
        // Only a capability on the PID namespace for children, READ and CREATE.
        let space = CapabilitySpace::new();
        cap_manager()
            .create_capability(
                ObjectRef::Namespace {
                    kind: NsKind::Pid,
                    id: set.pid_for_children.ident.id,
                },
                Rights::CREATE,
                &space,
            )
            .unwrap();
        assert!(may_create_namespaces(&space, &set, &init, ns::CLONE_NEWPID));
        assert!(!may_create_namespaces(
            &space,
            &set,
            &init,
            ns::CLONE_NEWNET
        ));
        let read_only = CapabilitySpace::new();
        cap_manager()
            .create_capability(
                ObjectRef::Namespace {
                    kind: NsKind::Pid,
                    id: set.pid_for_children.ident.id,
                },
                Rights::READ,
                &read_only,
            )
            .unwrap();
        assert!(!may_create_namespaces(
            &read_only,
            &set,
            &init,
            ns::CLONE_NEWPID
        ));
    }

    #[test]
    fn drop_rejects_what_cannot_be_named() {
        let space = initial_space();
        assert_eq!(drop_object(&space, 0, 0), None, "Memory");
        assert_eq!(drop_object(&space, TYPE_NAMESPACE as usize, 0), None);
        assert_eq!(drop_object(&space, TYPE_CLOCK as usize, 2), None);
        assert_eq!(drop_object(&space, TYPE_SYSTEM as usize, 1), None);
        assert_eq!(drop_object(&space, 0x100 + TYPE_SYSTEM as usize, 0), None);
        // Rights above CREATE.
        assert_eq!(drop_rights(&space, TYPE_SYSTEM as usize, 0, 0x100), None);
        assert_eq!(space.used(), 11);
    }

    /// A rights mask narrows a capability instead of removing it (ADR
    /// 0026); the rights taken off are gone, the rest stay, and nothing
    /// is added back.
    #[test]
    fn a_rights_mask_narrows_and_never_widens() {
        let space = initial_space();
        let system = ObjectRef::System;
        let modify = Rights::MODIFY.bits() as usize;
        assert_eq!(
            drop_rights(&space, TYPE_SYSTEM as usize, 0, modify),
            Some(1)
        );
        assert!(!holds(&space, &system, Rights::MODIFY));
        assert!(holds(&space, &system, Rights::READ.union(Rights::WRITE)));
        assert_eq!(space.used(), 11, "narrowed, not removed");
        // Again: nothing left to take, so nothing counted.
        assert_eq!(
            drop_rights(&space, TYPE_SYSTEM as usize, 0, modify),
            Some(0)
        );
        // A mask naming rights it never had (GRANT) changes nothing.
        let grant = Rights::GRANT.bits() as usize;
        assert_eq!(drop_rights(&space, TYPE_SYSTEM as usize, 0, grant), Some(0));
        assert!(!holds(&space, &system, Rights::MODIFY), "not widened");
        // Taking the rest removes it.
        let rest = authority_rights().remove(Rights::MODIFY).bits() as usize;
        assert_eq!(drop_rights(&space, TYPE_SYSTEM as usize, 0, rest), Some(1));
        assert!(!holds(&space, &system, Rights::READ));
        assert_eq!(space.used(), 10);
    }

    /// Mask 0 is the whole drop, and a mask covering every right held
    /// removes too.
    #[test]
    fn mask_zero_or_every_right_removes_the_capability() {
        let space = initial_space();
        assert_eq!(drop_rights(&space, TYPE_CLOCK as usize, 0, 0), Some(1));
        assert_eq!(space.used(), 10);
        let all = Rights::ALL.bits() as usize;
        assert_eq!(drop_rights(&space, TYPE_CLOCK as usize, 1, all), Some(1));
        assert_eq!(space.used(), 9);
    }

    /// MODIFY off the mount namespace and the user namespace above it leaves
    /// READ and WRITE through either (ADR 0026, the case step 6 needs); the
    /// next test shows what else denying mounts takes.
    #[test]
    fn modify_dropped_along_the_owner_chain_keeps_read_and_write() {
        let space = initial_space();
        let set = ns::initial();
        let mnt = set.get(NsKind::Mnt);
        let user = set.get(NsKind::User).ident().id as usize;
        let modify = Rights::MODIFY.bits() as usize;
        let mnt_id = mnt.ident().id as usize;
        assert_eq!(
            drop_rights(&space, TYPE_NAMESPACE as usize, mnt_id, modify),
            Some(1)
        );
        assert!(
            holds_namespace(&space, &mnt, Rights::MODIFY),
            "the owner stands"
        );
        assert_eq!(
            drop_rights(&space, TYPE_NAMESPACE as usize, user, modify),
            Some(1)
        );
        assert!(!holds_namespace(&space, &mnt, Rights::MODIFY));
        assert!(holds_namespace(
            &space,
            &mnt,
            Rights::READ.union(Rights::WRITE)
        ));
        assert!(holds_namespace(&space, &mnt, Rights::CREATE));
    }

    /// MODIFY off the mount namespace and the user namespaces above it still
    /// leaves CREATE on the user namespace, so a new user namespace can be
    /// made, its creator is granted it, and mounting inside a mount
    /// namespace it owns passes. Taking CREATE off the caller's user
    /// namespace too closes that: no new user namespace, while namespaces
    /// of other kinds still pass on their own capabilities (side review of
    /// ADR 0026's recipe).
    #[test]
    fn denying_mounts_also_takes_create_off_the_user_namespace() {
        use crate::process::{
            ns::{CLONE_NEWNS, CLONE_NEWUSER, CLONE_NEWUTS},
            userns,
        };
        let space = initial_space();
        let set = ns::initial();
        let init = userns::initial();
        let modify = Rights::MODIFY.bits() as usize;
        let create = Rights::CREATE.bits() as usize;
        let mnt_id = set.get(NsKind::Mnt).ident().id as usize;
        let user_id = init.ident.id as usize;
        drop_rights(&space, TYPE_NAMESPACE as usize, mnt_id, modify).unwrap();
        drop_rights(&space, TYPE_NAMESPACE as usize, user_id, modify).unwrap();
        // MODIFY alone: a new user namespace is still allowed.
        assert!(may_create_namespaces(&space, &set, &init, CLONE_NEWUSER));
        // The way round it: a new one, granted to its creator, owns a new
        // mount namespace's MODIFY.
        let private = userns::UserNs::create(&init, 0, 0, false, true).unwrap();
        let copy = set
            .copy_for(CLONE_NEWNS, &private, &crate::process::pidns::initial(), 0)
            .unwrap();
        grant_user_ns(&space, &private).unwrap();
        assert!(holds_namespace(
            &space,
            &copy.get(NsKind::Mnt),
            Rights::MODIFY
        ));
        // CREATE off the caller's user namespace closes it.
        let fresh = initial_space();
        for (id, mask) in [(mnt_id, modify), (user_id, modify | create)] {
            drop_rights(&fresh, TYPE_NAMESPACE as usize, id, mask).unwrap();
        }
        assert!(!may_create_namespaces(&fresh, &set, &init, CLONE_NEWUSER));
        assert!(may_create_namespaces(&fresh, &set, &init, CLONE_NEWUTS));
        assert!(may_create_namespaces(&fresh, &set, &init, CLONE_NEWNS));
        assert!(!holds_namespace(
            &fresh,
            &set.get(NsKind::Mnt),
            Rights::MODIFY
        ));
    }

    /// File access reads the mount namespace's rights (ADR 0024, step 6):
    /// a launched program has them all; a MODIFY-only drop, the recipe for
    /// "cannot mount", keeps them; taking READ, WRITE or CREATE off the
    /// mount namespace and every user namespace above it takes that right
    /// away, and a capability left on any one of them keeps it.
    #[test]
    fn fs_authority_follows_the_mount_namespace_and_its_owners() {
        use crate::process::creds::FsAuthority;
        let set = ns::initial();
        let mnt = set.get(NsKind::Mnt);
        let mnt_id = mnt.ident().id as usize;
        let user_id = crate::process::userns::initial().ident.id as usize;
        let bits = |r: Rights| r.bits() as usize;
        let space = initial_space();
        assert_eq!(fs_authority(&space, &mnt), FsAuthority::ALL);
        for id in [mnt_id, user_id] {
            drop_rights(&space, TYPE_NAMESPACE as usize, id, bits(Rights::MODIFY)).unwrap();
        }
        assert_eq!(fs_authority(&space, &mnt), FsAuthority::ALL);
        // WRITE off the mount namespace alone: the user namespace above
        // it still stands for it.
        let partial = initial_space();
        drop_rights(
            &partial,
            TYPE_NAMESPACE as usize,
            mnt_id,
            bits(Rights::WRITE),
        )
        .unwrap();
        assert_eq!(fs_authority(&partial, &mnt), FsAuthority::ALL);
        let read_only = initial_space();
        for id in [mnt_id, user_id] {
            let mask = bits(Rights::WRITE) | bits(Rights::CREATE);
            drop_rights(&read_only, TYPE_NAMESPACE as usize, id, mask).unwrap();
        }
        assert_eq!(
            fs_authority(&read_only, &mnt),
            FsAuthority {
                read: true,
                write: false,
                create: false
            }
        );
        let none = initial_space();
        for id in [mnt_id, user_id] {
            drop_rights(&none, TYPE_NAMESPACE as usize, id, 0).unwrap();
        }
        assert_eq!(fs_authority(&none, &mnt), FsAuthority::NONE);
        assert_eq!(
            fs_authority(&CapabilitySpace::new(), &mnt),
            FsAuthority::NONE
        );
    }

    /// Narrowing is the caller's own: a forked copy's narrowing leaves the
    /// parent's rights alone, and a child forked after it inherits it.
    #[test]
    fn narrowing_touches_only_this_space_and_is_inherited() {
        let parent = initial_space();
        let child = CapabilitySpace::new();
        child.clone_from(&parent).unwrap();
        let modify = Rights::MODIFY.bits() as usize;
        assert_eq!(
            drop_rights(&child, TYPE_SYSTEM as usize, 0, modify),
            Some(1)
        );
        assert!(holds(&parent, &ObjectRef::System, Rights::MODIFY));
        let grandchild = CapabilitySpace::new();
        grandchild.clone_from(&child).unwrap();
        assert!(!holds(&grandchild, &ObjectRef::System, Rights::MODIFY));
        assert!(holds(&grandchild, &ObjectRef::System, Rights::READ));
    }
}
