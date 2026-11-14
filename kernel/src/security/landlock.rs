//! Landlock (ADR 0028): the kernel's side of `veridian-landlock`.
//!
//! The crate holds the rulesets, the stacked domains and the access
//! arithmetic. Here are what only the kernel has: the domain each thread is
//! restricted by, the ruleset descriptor, and what the authority entry
//! (`cap::authority::holds_path` and `holds_net`), which the VFS, the
//! socket layer and the process calls ask at Linux's hook points, needs.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;
pub use veridian_landlock as ll;

use crate::{
    error::{FsError, KernelError},
    fs::{DirEntry, Metadata, NodeType, Permissions, VfsNode},
};

/// A domain and the objects its rules are anchored on. Holding the nodes
/// keeps the identity a rule names from going to another object while the
/// domain lives, as Linux's `landlock_object` holds its inode: a key made
/// from a node's address stays unique while the node is held.
pub struct KDomain {
    pub dom: ll::Domain,
    _anchors: Vec<Arc<dyn VfsNode>>,
}

impl core::fmt::Debug for KDomain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "KDomain({} layers)", self.dom.depth())
    }
}

impl KDomain {
    /// Whether some layer handles a filesystem right (`any_fs`): such a
    /// domain may not change mounts.
    pub fn handles_fs(&self) -> bool {
        self.dom.handled().fs != 0
    }

    /// Whether some layer handles a network right.
    pub fn handles_net(&self) -> bool {
        self.dom.handled().net != 0
    }

    /// Whether some layer sets `scope`.
    pub fn handles_scope(&self, scope: u32) -> bool {
        self.dom.handled().scope & scope != 0
    }
}

/// A thread's domain (Linux's `landlock_cred_security`): shared with the
/// threads and processes it creates and kept across exec. Restricting
/// replaces it with a deeper one; nothing removes one.
pub struct ThreadLandlock(Mutex<Option<Arc<KDomain>>>);

impl ThreadLandlock {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub fn get(&self) -> Option<Arc<KDomain>> {
        self.0.lock().clone()
    }

    pub fn set(&self, d: Arc<KDomain>) {
        *self.0.lock() = Some(d);
    }

    /// A new thread or process takes its creator's.
    pub fn copy_from(&self, other: &ThreadLandlock) {
        let d = other.get();
        *self.0.lock() = d;
    }
}

impl Default for ThreadLandlock {
    fn default() -> Self {
        Self::new()
    }
}

/// The calling thread's domain, `None` when it is not restricted (also in
/// the kernel's own context).
pub fn current() -> Option<Arc<KDomain>> {
    crate::process::current_thread().and_then(|t| t.landlock.get())
}

/// Layer identities, unique for the machine's life (`landlock_hierarchy`).
static NEXT_LAYER: AtomicU64 = AtomicU64::new(1);

/// `parent` restricted further by `ruleset` (landlock_merge_ruleset).
pub fn restrict(parent: Option<&KDomain>, ruleset: &RulesetNode) -> Result<KDomain, ll::Error> {
    let r = ruleset.inner.lock();
    let id = NEXT_LAYER.fetch_add(1, Ordering::Relaxed);
    let dom = ll::Domain::restrict(parent.map(|p| &p.dom), &r.ruleset, id)?;
    let mut anchors = parent.map_or_else(Vec::new, |p| p._anchors.clone());
    anchors.extend(r.anchors.iter().cloned());
    Ok(KDomain {
        dom,
        _anchors: anchors,
    })
}

/// The rule anchor of `node`: its identity as `fs::inode_key` gives it,
/// through an idmapped view to the real node.
pub fn key_of(node: &Arc<dyn VfsNode>) -> ll::ObjKey {
    crate::fs::inode_key(node)
}

/// What a Landlock refusal is as a kernel error: EACCES, or EXDEV for a
/// link or rename that would give an object more access.
pub fn to_kernel(e: ll::Error) -> KernelError {
    match e {
        ll::Error::CrossDevice => KernelError::FsError(FsError::CrossDevice),
        _ => KernelError::FsError(FsError::PermissionDenied),
    }
}

/// A mount change by a thread whose domain handles a filesystem right
/// (hook_sb_mount and the others): EPERM.
pub fn may_change_mounts() -> Result<(), KernelError> {
    match current() {
        Some(d) if d.handles_fs() => Err(KernelError::FsError(FsError::OperationNotPermitted)),
        _ => Ok(()),
    }
}

/// Whether a thread with domain `client` may reach one with domain `server`
/// across `scope` (domain_is_scoped): `false` is a refusal.
pub fn may_reach(client: Option<&KDomain>, server: Option<&KDomain>, scope: u32) -> bool {
    match client {
        None => true,
        Some(c) => !ll::Domain::is_scoped(&c.dom, server.map(|s| &s.dom), scope),
    }
}

/// The domain of `process` as another process's check sees it: its
/// leader's (Linux checks the named task; a process is named by its
/// leader), else any thread's.
pub fn of_process(process: &crate::process::Process) -> Option<Arc<KDomain>> {
    let threads = process.threads.lock();
    threads
        .get(&crate::process::ThreadId(process.pid.0))
        .or_else(|| threads.values().next())
        .and_then(|t| t.landlock.get())
}

/// Whether the calling thread may inspect `target` (ptrace, process_vm,
/// pidfd_getfd, kcmp, `/proc/<pid>` files that need ptrace access):
/// hook_ptrace_access_check.
pub fn current_may_inspect(target: Option<&KDomain>) -> bool {
    may_inspect(current().as_deref(), target)
}

/// Whether a thread with domain `tracer` may inspect one with domain
/// `tracee` (hook_ptrace_access_check, domain_scope_le): the tracee must be
/// in the tracer's domain or below it.
pub fn may_inspect(tracer: Option<&KDomain>, tracee: Option<&KDomain>) -> bool {
    ll::Domain::scope_le(tracer.map(|d| &d.dom), tracee.map(|d| &d.dom))
}

/// The device ioctls a file opened without IOCTL_DEV may still use
/// (is_masked_device_ioctl): the generic ones `fcntl` also reaches and those
/// about the filesystem, not the device.
pub fn is_masked_device_ioctl(cmd: u32) -> bool {
    const FIOCLEX: u32 = 0x5451;
    const FIONCLEX: u32 = 0x5450;
    const FIONBIO: u32 = 0x5421;
    const FIOASYNC: u32 = 0x5452;
    const FIOQSIZE: u32 = 0x5460;
    const FIFREEZE: u32 = 0xc004_5877;
    const FITHAW: u32 = 0xc004_5878;
    const FS_IOC_FIEMAP: u32 = 0xc020_660b;
    const FIGETBSZ: u32 = 0x2;
    const FICLONE: u32 = 0x4004_9409;
    const FICLONERANGE: u32 = 0x4020_940d;
    const FIDEDUPERANGE: u32 = 0xc018_9436;
    const FS_IOC_GETFSUUID: u32 = 0x8011_1500;
    const FS_IOC_GETFSSYSFSPATH: u32 = 0x8081_1501;
    matches!(
        cmd,
        FIOCLEX
            | FIONCLEX
            | FIONBIO
            | FIOASYNC
            | FIOQSIZE
            | FIFREEZE
            | FITHAW
            | FS_IOC_FIEMAP
            | FIGETBSZ
            | FICLONE
            | FICLONERANGE
            | FIDEDUPERANGE
            | FS_IOC_GETFSUUID
            | FS_IOC_GETFSSYSFSPATH
    )
}

/// A ruleset being built, behind a ruleset descriptor
/// (`anon_inode:[landlock-ruleset]`): the rules and the nodes they name.
pub struct RulesetNode {
    inner: Mutex<RulesetState>,
}

struct RulesetState {
    ruleset: ll::Ruleset,
    anchors: Vec<Arc<dyn VfsNode>>,
}

impl RulesetNode {
    pub fn new(ruleset: ll::Ruleset) -> Self {
        Self {
            inner: Mutex::new(RulesetState {
                ruleset,
                anchors: Vec::new(),
            }),
        }
    }

    /// LANDLOCK_RULE_PATH_BENEATH on `node`.
    pub fn add_path(
        &self,
        node: &Arc<dyn VfsNode>,
        allowed: u32,
        flags: u32,
    ) -> Result<(), ll::Error> {
        let is_dir = node.node_type() == NodeType::Directory;
        let base = crate::fs::base_node(node);
        let mut s = self.inner.lock();
        s.ruleset
            .add_fs_rule(key_of(&base), is_dir, allowed, flags)?;
        if !s.anchors.iter().any(|a| Arc::ptr_eq(a, &base)) {
            s.anchors.push(base);
        }
        Ok(())
    }

    /// LANDLOCK_RULE_NET_PORT.
    pub fn add_port(&self, port: u64, allowed: u32, flags: u32) -> Result<(), ll::Error> {
        self.inner.lock().ruleset.add_net_rule(port, allowed, flags)
    }

    /// The checks of a path rule before its descriptor is looked at.
    pub fn check_fs_rule(&self, allowed: u32, flags: u32) -> Result<(), ll::Error> {
        self.inner.lock().ruleset.check_fs_rule(allowed, flags)
    }

    /// The checks of a port rule before the port's.
    pub fn check_net_rule(&self, allowed: u32, flags: u32) -> Result<(), ll::Error> {
        self.inner.lock().ruleset.check_net_rule(allowed, flags)
    }

    /// The access masks the ruleset handles, for the rule checks.
    pub fn handled(&self) -> ll::AccessMasks {
        self.inner.lock().ruleset.attr.handled
    }
}

impl VfsNode for RulesetNode {
    /// An anonymous inode, as on Linux.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::ANON_INODE_FS_MAGIC,
        ))
    }

    fn node_type(&self) -> NodeType {
        NodeType::File
    }

    fn is_stream(&self) -> bool {
        true
    }

    /// fop_dummy_read: EINVAL (it exists so the file can be opened for
    /// reading, which restrict_self asks).
    fn read(&self, _offset: usize, _buffer: &mut [u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a Landlock ruleset cannot be read",
        })
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a Landlock ruleset cannot be written",
        })
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::File,
            permissions: Permissions::from_mode(0o600),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: Permissions,
    ) -> Result<Arc<dyn VfsNode>, KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::FsError(FsError::NotADirectory))
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a Landlock ruleset cannot be truncated",
        })
    }
}

/// Address families the network rules look at.
pub const AF_UNSPEC: u16 = 0;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;

/// What a refused socket operation is (current_check_access_socket).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetRefusal {
    /// EINVAL: an address too short, or of another family than the socket.
    Invalid,
    /// EAFNOSUPPORT: an AF_UNSPEC bind that is not to the any address.
    AfNotSupported,
    /// EACCES.
    Denied,
}

/// Landlock's check of a TCP or UDP socket operation with address `addr`
/// (the raw `struct sockaddr`) on a socket of family `family`, for the
/// network right `access` (current_check_access_socket). `connecting` is a
/// connect's, where AF_UNSPEC disconnects. Rights `domain` does not handle
/// pass, and so do families other than AF_INET and AF_INET6.
pub fn check_sockaddr(
    domain: &KDomain,
    family: u16,
    addr: &[u8],
    access: u32,
    connecting: bool,
) -> Result<(), NetRefusal> {
    use ll::net;
    if domain.dom.handled().net & access == 0 {
        return Ok(());
    }
    if addr.len() < 2 {
        return Err(NetRefusal::Invalid);
    }
    let sa_family = u16::from_ne_bytes([addr[0], addr[1]]);
    let bind = access == net::BIND_TCP || access == net::BIND_UDP;
    if sa_family == AF_UNSPEC {
        if access == net::CONNECT_TCP || (access == net::CONNECT_SEND_UDP && connecting) {
            return Ok(());
        }
        if access == net::CONNECT_SEND_UDP && family == AF_INET6 {
            return Err(NetRefusal::Denied);
        }
        if bind {
            if family == AF_INET {
                if addr.len() < 16 {
                    return Err(NetRefusal::Invalid);
                }
                if addr[4..8] != [0, 0, 0, 0] {
                    return Err(NetRefusal::AfNotSupported);
                }
            } else {
                return Err(if addr.len() < 24 {
                    NetRefusal::Invalid
                } else {
                    NetRefusal::AfNotSupported
                });
            }
        }
    }
    // AF_UNSPEC goes on as AF_INET.
    let min = match sa_family {
        AF_UNSPEC | AF_INET => 16,
        AF_INET6 => 24,
        _ => return Ok(()),
    };
    if addr.len() < min {
        return Err(NetRefusal::Invalid);
    }
    if sa_family != family && sa_family != AF_UNSPEC {
        return Err(NetRefusal::Invalid);
    }
    let port = u16::from_be_bytes([addr[2], addr[3]]);
    domain
        .dom
        .check_net(port, access)
        .map_err(|_| NetRefusal::Denied)
}

/// A UDP socket with no local port yet is bound to one by a connect or a
/// send: that needs BIND_UDP on port 0 (current_check_autobind_udp_socket).
pub fn check_udp_autobind(domain: &KDomain, bound: bool) -> Result<(), NetRefusal> {
    if bound || domain.dom.handled().net & ll::net::BIND_UDP == 0 {
        return Ok(());
    }
    domain
        .dom
        .check_net(0, ll::net::BIND_UDP)
        .map_err(|_| NetRefusal::Denied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(handled_net: u32, port: u64, allowed: u32) -> KDomain {
        let mut r = ll::Ruleset::new(ll::RulesetAttr {
            handled: ll::AccessMasks {
                fs: 0,
                net: handled_net,
                scope: 0,
            },
            quiet: ll::AccessMasks::default(),
        })
        .unwrap();
        r.add_net_rule(port, allowed, 0).unwrap();
        let node = RulesetNode::new(r);
        restrict(None, &node).unwrap()
    }

    fn sin(family: u16, port: u16, addr: [u8; 4]) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..2].copy_from_slice(&family.to_ne_bytes());
        b[2..4].copy_from_slice(&port.to_be_bytes());
        b[4..8].copy_from_slice(&addr);
        b
    }

    /// The port rules and Linux's address checks, case by case.
    #[test]
    fn sockets_are_checked_as_linux() {
        use ll::net;
        let d = domain(
            net::BIND_TCP | net::CONNECT_TCP | net::CONNECT_SEND_UDP,
            8080,
            net::BIND_TCP,
        );
        let check =
            |a: &[u8], access, connecting| check_sockaddr(&d, AF_INET, a, access, connecting);
        assert_eq!(
            check(&sin(AF_INET, 8080, [0; 4]), net::BIND_TCP, false),
            Ok(())
        );
        assert_eq!(
            check(&sin(AF_INET, 8081, [0; 4]), net::BIND_TCP, false),
            Err(NetRefusal::Denied)
        );
        assert_eq!(
            check(&sin(AF_INET, 8080, [127, 0, 0, 1]), net::CONNECT_TCP, true),
            Err(NetRefusal::Denied)
        );
        // Not handled: allowed.
        assert_eq!(
            check(&sin(AF_INET, 1, [0; 4]), net::BIND_UDP, false),
            Ok(())
        );
        // Short, or of the other family.
        assert_eq!(
            check(&[2, 0], net::BIND_TCP, false),
            Err(NetRefusal::Invalid)
        );
        assert_eq!(check(&[2], net::BIND_TCP, false), Err(NetRefusal::Invalid));
        let mut six = [0u8; 28];
        six[0..2].copy_from_slice(&AF_INET6.to_ne_bytes());
        assert_eq!(check(&six, net::BIND_TCP, false), Err(NetRefusal::Invalid));
        // AF_UNSPEC: a TCP connect disconnects; a bind must be to the any
        // address, then is checked as AF_INET.
        assert_eq!(
            check(&sin(AF_UNSPEC, 80, [0; 4]), net::CONNECT_TCP, true),
            Ok(())
        );
        assert_eq!(
            check(&sin(AF_UNSPEC, 8080, [1, 2, 3, 4]), net::BIND_TCP, false),
            Err(NetRefusal::AfNotSupported)
        );
        assert_eq!(
            check(&sin(AF_UNSPEC, 8080, [0; 4]), net::BIND_TCP, false),
            Ok(())
        );
        // UDP: connecting to AF_UNSPEC disconnects; sending to it from an
        // AF_INET6 socket is refused.
        assert_eq!(
            check(&sin(AF_UNSPEC, 53, [0; 4]), net::CONNECT_SEND_UDP, true),
            Ok(())
        );
        assert_eq!(
            check_sockaddr(
                &d,
                AF_INET6,
                &sin(AF_UNSPEC, 53, [0; 4]),
                net::CONNECT_SEND_UDP,
                false
            ),
            Err(NetRefusal::Denied)
        );
        // Other families are not Landlock's.
        assert_eq!(check(&[1, 0, b'/', 0], net::CONNECT_TCP, true), Ok(()));
    }

    #[test]
    fn udp_autobind_needs_port_zero() {
        use ll::net;
        let d = domain(net::BIND_UDP, 0, net::BIND_UDP);
        assert_eq!(check_udp_autobind(&d, false), Ok(()));
        let d = domain(net::BIND_UDP, 53, net::BIND_UDP);
        assert_eq!(check_udp_autobind(&d, false), Err(NetRefusal::Denied));
        assert_eq!(check_udp_autobind(&d, true), Ok(()));
    }
}
