//! Landlock as Linux 7.3 defines it (`security/landlock/`, ABI 11): rulesets,
//! the stacked domains made from them, and the access checks.
//!
//! The kernel (ADR 0029) and the user-space Linux runtime (Track M, linuxd)
//! share this crate, so a ruleset means the same thing on both sides of the
//! move; v0.29's directory and network capabilities reuse its arithmetic.
//! It is `no_std` (with `alloc`) and has no `unsafe`.
//!
//! Filesystem checks take an [`Ancestry`]: the object keys of the target and
//! of every directory above it up to the real root, as the caller's path walk
//! found them. A rule names an object (an inode), never a path, so a renamed
//! or bind-mounted object keeps its rules, as on Linux.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};

/// What a filesystem rule is anchored on: the identity of an object (its
/// filesystem and inode), as the kernel's `fs::inode_key` gives it.
pub type ObjKey = (u64, u64);

/// `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)`.
pub const ABI_VERSION: u32 = 11;
/// `LANDLOCK_CREATE_RULESET_ERRATA`: the fixes this implementation has from
/// the start (Linux's errata 1 to 4).
pub const ERRATA: u32 = 0b1111;
/// The most layers a domain stacks (LANDLOCK_MAX_NUM_LAYERS).
pub const MAX_LAYERS: usize = 16;

/// Filesystem access rights (LANDLOCK_ACCESS_FS_*).
pub mod fs {
    pub const EXECUTE: u32 = 1 << 0;
    pub const WRITE_FILE: u32 = 1 << 1;
    pub const READ_FILE: u32 = 1 << 2;
    pub const READ_DIR: u32 = 1 << 3;
    pub const REMOVE_DIR: u32 = 1 << 4;
    pub const REMOVE_FILE: u32 = 1 << 5;
    pub const MAKE_CHAR: u32 = 1 << 6;
    pub const MAKE_DIR: u32 = 1 << 7;
    pub const MAKE_REG: u32 = 1 << 8;
    pub const MAKE_SOCK: u32 = 1 << 9;
    pub const MAKE_FIFO: u32 = 1 << 10;
    pub const MAKE_BLOCK: u32 = 1 << 11;
    pub const MAKE_SYM: u32 = 1 << 12;
    pub const REFER: u32 = 1 << 13;
    pub const TRUNCATE: u32 = 1 << 14;
    pub const IOCTL_DEV: u32 = 1 << 15;
    pub const RESOLVE_UNIX: u32 = 1 << 16;
    /// Every right (LANDLOCK_MASK_ACCESS_FS).
    pub const MASK: u32 = (RESOLVE_UNIX << 1) - 1;
    /// The rights that make sense on a file, not only a directory
    /// (ACCESS_FILE).
    pub const FILE: u32 = EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV | RESOLVE_UNIX;
    /// Denied by every layer, handled or not
    /// (_LANDLOCK_ACCESS_FS_INITIALLY_DENIED).
    pub const INITIALLY_DENIED: u32 = REFER;
}

/// Network access rights (LANDLOCK_ACCESS_NET_*).
pub mod net {
    pub const BIND_TCP: u32 = 1 << 0;
    pub const CONNECT_TCP: u32 = 1 << 1;
    pub const BIND_UDP: u32 = 1 << 2;
    pub const CONNECT_SEND_UDP: u32 = 1 << 3;
    pub const MASK: u32 = (CONNECT_SEND_UDP << 1) - 1;
}

/// Scopes (LANDLOCK_SCOPE_*).
pub mod scope {
    pub const ABSTRACT_UNIX_SOCKET: u32 = 1 << 0;
    pub const SIGNAL: u32 = 1 << 1;
    pub const MASK: u32 = (SIGNAL << 1) - 1;
}

/// `landlock_restrict_self` flags (LANDLOCK_RESTRICT_SELF_*).
pub mod restrict {
    pub const LOG_SAME_EXEC_OFF: u32 = 1 << 0;
    pub const LOG_NEW_EXEC_ON: u32 = 1 << 1;
    pub const LOG_SUBDOMAINS_OFF: u32 = 1 << 2;
    pub const TSYNC: u32 = 1 << 3;
    pub const NO_NEW_PRIVS: u32 = 1 << 4;
    pub const MASK: u32 = (NO_NEW_PRIVS << 1) - 1;
}

/// LANDLOCK_ADD_RULE_QUIET.
pub const ADD_RULE_QUIET: u32 = 1 << 0;

/// What a call fails with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// EINVAL.
    Invalid,
    /// ENOMSG: nothing handled, or a rule allowing nothing.
    NoMessage,
    /// E2BIG: a domain would have more than [`MAX_LAYERS`].
    TooBig,
    /// EACCES.
    Denied,
    /// EXDEV: a link or rename would give the object more access.
    CrossDevice,
}

/// Access masks of one layer or ruleset (`struct access_masks`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccessMasks {
    pub fs: u32,
    pub net: u32,
    pub scope: u32,
}

impl AccessMasks {
    fn or(self, o: Self) -> Self {
        Self {
            fs: self.fs | o.fs,
            net: self.net | o.net,
            scope: self.scope | o.scope,
        }
    }
}

/// `struct landlock_ruleset_attr` once checked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RulesetAttr {
    pub handled: AccessMasks,
    pub quiet: AccessMasks,
}

impl RulesetAttr {
    /// The checks of `landlock_create_ruleset`, in its order.
    pub fn check(self) -> Result<Self, Error> {
        let (h, q) = (self.handled, self.quiet);
        if h.fs & !fs::MASK != 0 || h.net & !net::MASK != 0 || h.scope & !scope::MASK != 0 {
            return Err(Error::Invalid);
        }
        if q.fs & !h.fs != 0 || q.net & !h.net != 0 || q.scope & !h.scope != 0 {
            return Err(Error::Invalid);
        }
        if h == AccessMasks::default() {
            return Err(Error::NoMessage);
        }
        Ok(self)
    }
}

/// One rule's grant in one layer (`struct landlock_layer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grant {
    /// 1-based layer.
    level: u8,
    access: u32,
    quiet: bool,
}

/// A ruleset being built (`struct landlock_ruleset`): one layer to be.
#[derive(Debug, Clone, Default)]
pub struct Ruleset {
    pub attr: RulesetAttr,
    fs_rules: BTreeMap<ObjKey, (u32, bool)>,
    net_rules: BTreeMap<u64, (u32, bool)>,
}

impl Ruleset {
    pub fn new(attr: RulesetAttr) -> Result<Self, Error> {
        Ok(Self {
            attr: attr.check()?,
            ..Self::default()
        })
    }

    /// LANDLOCK_RULE_PATH_BENEATH on object `key` (a directory if `is_dir`)
    /// with `allowed` (add_rule_path_beneath, landlock_append_fs_rule). The
    /// caller has resolved the descriptor.
    pub fn add_fs_rule(
        &mut self,
        key: ObjKey,
        is_dir: bool,
        allowed: u32,
        flags: u32,
    ) -> Result<(), Error> {
        let quiet = self.check_rule(allowed, flags, self.attr.handled.fs, self.attr.quiet.fs)?;
        if !is_dir && allowed & !fs::FILE != 0 {
            return Err(Error::Invalid);
        }
        // Rights the ruleset does not handle are allowed (absolute form).
        let absolute = allowed | (fs::MASK & !(self.attr.handled.fs | fs::INITIALLY_DENIED));
        let e = self.fs_rules.entry(key).or_insert((0, false));
        e.0 |= absolute;
        e.1 |= quiet;
        Ok(())
    }

    /// LANDLOCK_RULE_NET_PORT for `port` with `allowed` (add_rule_net_port).
    pub fn add_net_rule(&mut self, port: u64, allowed: u32, flags: u32) -> Result<(), Error> {
        let quiet = self.check_rule(allowed, flags, self.attr.handled.net, self.attr.quiet.net)?;
        if port > u64::from(u16::MAX) {
            return Err(Error::Invalid);
        }
        let absolute = allowed | (net::MASK & !self.attr.handled.net);
        let e = self.net_rules.entry(port).or_insert((0, false));
        e.0 |= absolute;
        e.1 |= quiet;
        Ok(())
    }

    /// The checks of a path rule before its descriptor is looked at
    /// (add_rule_path_beneath): ENOMSG, then EINVAL.
    pub fn check_fs_rule(&self, allowed: u32, flags: u32) -> Result<(), Error> {
        self.check_rule(allowed, flags, self.attr.handled.fs, self.attr.quiet.fs)
            .map(|_| ())
    }

    /// The checks of a port rule (add_rule_net_port), before the port's.
    pub fn check_net_rule(&self, allowed: u32, flags: u32) -> Result<(), Error> {
        self.check_rule(allowed, flags, self.attr.handled.net, self.attr.quiet.net)
            .map(|_| ())
    }

    fn check_rule(
        &self,
        allowed: u32,
        flags: u32,
        handled: u32,
        quiet: u32,
    ) -> Result<bool, Error> {
        if flags & !ADD_RULE_QUIET != 0 {
            return Err(Error::Invalid);
        }
        if flags == 0 && allowed == 0 {
            return Err(Error::NoMessage);
        }
        if allowed & !handled != 0 {
            return Err(Error::Invalid);
        }
        if flags & ADD_RULE_QUIET != 0 && quiet == 0 {
            return Err(Error::Invalid);
        }
        Ok(flags & ADD_RULE_QUIET != 0)
    }
}

/// A process's Landlock domain (`struct landlock_domain`): the layers it is
/// restricted by, oldest first, and their rules merged per object. Shared
/// and immutable; restricting again makes a new one.
#[derive(Debug, Clone)]
pub struct Domain {
    layers: Vec<AccessMasks>,
    /// The identity of each layer (`landlock_hierarchy`), oldest first: two
    /// domains share a layer if they have the same id at the same depth.
    hierarchy: Vec<u64>,
    fs_rules: BTreeMap<ObjKey, Vec<Grant>>,
    net_rules: BTreeMap<u64, Vec<Grant>>,
}

impl Domain {
    /// `parent` restricted further by `ruleset` as a new layer with id
    /// `layer_id` (unique for the machine's life): landlock_merge_ruleset.
    /// E2BIG past [`MAX_LAYERS`].
    pub fn restrict(
        parent: Option<&Domain>,
        ruleset: &Ruleset,
        layer_id: u64,
    ) -> Result<Self, Error> {
        let mut d = parent.cloned().unwrap_or(Domain {
            layers: Vec::new(),
            hierarchy: Vec::new(),
            fs_rules: BTreeMap::new(),
            net_rules: BTreeMap::new(),
        });
        if d.layers.len() >= MAX_LAYERS {
            return Err(Error::TooBig);
        }
        d.layers.push(ruleset.attr.handled);
        d.hierarchy.push(layer_id);
        let level = d.layers.len() as u8;
        for (key, &(access, quiet)) in &ruleset.fs_rules {
            d.fs_rules.entry(*key).or_default().push(Grant {
                level,
                access,
                quiet,
            });
        }
        for (port, &(access, quiet)) in &ruleset.net_rules {
            d.net_rules.entry(*port).or_default().push(Grant {
                level,
                access,
                quiet,
            });
        }
        Ok(d)
    }

    /// How many layers.
    pub fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Every right some layer handles (landlock_union_access_masks), the
    /// initially denied ones included when any filesystem right is.
    pub fn handled(&self) -> AccessMasks {
        let mut u = self
            .layers
            .iter()
            .fold(AccessMasks::default(), |a, &l| a.or(l));
        if u.fs != 0 {
            u.fs |= fs::INITIALLY_DENIED;
        }
        u
    }

    fn fs_handled(&self, layer: usize) -> u32 {
        self.layers[layer].fs | fs::INITIALLY_DENIED
    }

    /// Masks for `request` (landlock_init_layer_masks): per layer, the
    /// requested rights it handles; and their union.
    fn init_masks(&self, request: u32, kind: Kind) -> (u32, LayerMasks) {
        let mut m = LayerMasks::default();
        let mut handled = 0;
        if request == 0 {
            return (0, m);
        }
        for i in 0..self.layers.len() {
            let h = match kind {
                Kind::Fs => self.fs_handled(i),
                Kind::Net => self.layers[i].net,
            };
            m.0[i] = request & h;
            handled |= m.0[i];
        }
        (handled, m)
    }

    /// Clear in `masks` the rights `key`'s rule grants, within `request`
    /// (landlock_unmask_layers, as unmask_layers_fs passes it): whether every
    /// layer is now satisfied.
    fn unmask<K: Ord>(rules: &BTreeMap<K, Vec<Grant>>, key: &K, masks: &mut LayerMasks) -> bool {
        let Some(rule) = rules.get(key) else {
            return false;
        };
        for g in rule {
            masks.0[g.level as usize - 1] &= !g.access;
        }
        masks.allowed()
    }

    /// Whether `client` may reach `server` across `scope`
    /// (domain_is_scoped): it may unless a layer of the client's that sets
    /// the scope is not also one of the server's.
    pub fn is_scoped(client: &Domain, server: Option<&Domain>, scope: u32) -> bool {
        let server_layers = server.map_or(0, |s| s.layers.len());
        for l in (0..client.layers.len()).rev() {
            if client.layers[l].scope & scope == 0 {
                continue;
            }
            if l >= server_layers {
                return true;
            }
            return server.is_none_or(|s| s.hierarchy[l] != client.hierarchy[l]);
        }
        false
    }

    /// Whether `child` is at least as restricted as `parent`: `parent`'s
    /// newest layer is one of `child`'s (domain_scope_le). Governs ptrace and
    /// the other inspections of another process.
    pub fn scope_le(parent: Option<&Domain>, child: Option<&Domain>) -> bool {
        let Some(parent) = parent else {
            return true;
        };
        let Some(child) = child else {
            return false;
        };
        let n = parent.hierarchy.len();
        n <= child.hierarchy.len() && child.hierarchy[n - 1] == parent.hierarchy[n - 1]
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Fs,
    Net,
}

/// Per layer, the requested rights no rule has granted yet
/// (`struct layer_masks`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerMasks(pub [u32; MAX_LAYERS]);

impl LayerMasks {
    fn allowed(&self) -> bool {
        self.0.iter().all(|&a| a == 0)
    }

    /// Keep only `request` (scope_to_request): whether that is now all
    /// granted.
    fn scope_to(&mut self, request: u32) -> bool {
        let mut ok = true;
        for a in &mut self.0 {
            *a &= request;
            ok &= *a == 0;
        }
        ok
    }

    /// Whether a right other than REFER is still denied (is_eacces): REFER
    /// alone is EXDEV.
    fn is_eacces(&self, request: u32) -> bool {
        self.0.iter().any(|&a| a & request & !fs::REFER != 0)
    }
}

/// The objects a path goes through, as the walk found them: `keys[0]` is the
/// target, then each directory above it, ending at the real root. A mount's
/// root and the directory it covers both appear, as Linux's walk visits both
/// (`follow_up`). `None` is an object with no rule anchor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ancestry {
    pub keys: Vec<Option<ObjKey>>,
    /// A pseudo filesystem no one can mount (pipes, sockets) or an internal
    /// one reached through `/proc` (nsfs): always allowed, as
    /// is_nouser_or_private and the MNT_INTERNAL case.
    pub internal: bool,
}

/// The right to make an object of type `mode` (`S_IFMT` bits), as
/// get_mode_access: weird files count as regular ones.
pub fn make_access(mode: u32) -> u32 {
    match mode & 0o170000 {
        0o120000 => fs::MAKE_SYM,
        0o040000 => fs::MAKE_DIR,
        0o020000 => fs::MAKE_CHAR,
        0o060000 => fs::MAKE_BLOCK,
        0o010000 => fs::MAKE_FIFO,
        0o140000 => fs::MAKE_SOCK,
        _ => fs::MAKE_REG,
    }
}

/// The right to remove an object (maybe_remove).
pub fn remove_access(is_dir: bool) -> u32 {
    if is_dir {
        fs::REMOVE_DIR
    } else {
        fs::REMOVE_FILE
    }
}

impl Domain {
    /// Whether `request` on the path whose objects are `path` is allowed
    /// (current_check_access_path): EACCES if not.
    pub fn check_path(&self, path: &Ancestry, request: u32) -> Result<(), Error> {
        let (request, mut masks) = self.init_masks(request, Kind::Fs);
        if request == 0 || path.internal || masks.allowed() {
            return Ok(());
        }
        for key in path.keys.iter().flatten() {
            if Self::unmask(&self.fs_rules, key, &mut masks) {
                return Ok(());
            }
        }
        Err(Error::Denied)
    }

    /// [`Self::check_path`] for a connection to a UNIX socket created by a
    /// process with domain `server` (hook_unix_find): the layers this
    /// domain shares with the server's grant `request` without a rule
    /// (unmask_scoped_access).
    pub fn check_path_scoped(
        &self,
        path: &Ancestry,
        request: u32,
        server: Option<&Domain>,
    ) -> Result<(), Error> {
        let (request, mut masks) = self.init_masks(request, Kind::Fs);
        if request == 0 || path.internal {
            return Ok(());
        }
        if let Some(server) = server {
            let shared = self.hierarchy.len().min(server.hierarchy.len());
            for (l, m) in masks.0.iter_mut().enumerate().take(shared) {
                if self.hierarchy[l] == server.hierarchy[l] {
                    *m &= !request;
                }
            }
        }
        if masks.allowed() {
            return Ok(());
        }
        for key in path.keys.iter().flatten() {
            if Self::unmask(&self.fs_rules, key, &mut masks) {
                return Ok(());
            }
        }
        Err(Error::Denied)
    }

    /// The rights still denied on `path` for each layer, of all the
    /// filesystem rights it handles, walking only `keys[..upto]`
    /// (collect_domain_accesses): (masks, whether all were granted).
    fn collect(&self, path: &Ancestry, upto: usize) -> (LayerMasks, bool) {
        let (handled, mut masks) = self.init_masks(fs::MASK, Kind::Fs);
        if handled == 0 || path.internal {
            return (masks, true);
        }
        for key in path.keys.iter().take(upto).flatten() {
            if Self::unmask(&self.fs_rules, key, &mut masks) {
                return (masks, true);
            }
        }
        (masks, false)
    }

    /// Masks of a child object for a refer check: the handled rights its own
    /// rule does not grant.
    fn child_masks(&self, key: Option<ObjKey>) -> LayerMasks {
        let (handled, mut masks) = self.init_masks(fs::MASK, Kind::Fs);
        if handled != 0 {
            if let Some(k) = key {
                Self::unmask(&self.fs_rules, &k, &mut masks);
            }
        }
        masks
    }

    /// A link or rename (current_check_refer_path). `old` is the object
    /// moved or linked, `new` the object at the destination for an exchange
    /// (or one replaced, for `maybe_remove`). `old_dir` and `new_dir` are the
    /// two parent directories' ancestries, on one mount; `common` is how
    /// many entries at the end of each are above that mount's root, so
    /// `keys[..keys.len() - common]` runs from the directory up to the mount
    /// root, which it includes (collect_domain_accesses), and the walk past
    /// it starts at the mount root.
    #[allow(clippy::too_many_arguments)]
    pub fn check_refer(
        &self,
        old: Entry,
        new: Option<Entry>,
        old_dir: &Ancestry,
        new_dir: &Ancestry,
        common: usize,
        same_dir: bool,
        removable: bool,
        exchange: bool,
    ) -> Result<(), Error> {
        // A domain no layer of which handles a filesystem right does not
        // apply (landlock_get_applicable_subject with every filesystem
        // right): REFER, initially denied in each layer's own mask, is
        // asked only once some layer handles the filesystem (PR #49
        // review).
        if self.layers.iter().all(|l| l.fs == 0) {
            return Ok(());
        }
        let mut request1 = if exchange {
            new.map_or(0, |n| make_access(n.mode))
        } else {
            0
        };
        let mut request2 = make_access(old.mode);
        if removable {
            request1 |= remove_access(old.is_dir());
            if let Some(n) = new {
                request2 |= remove_access(n.is_dir());
            }
        }
        if same_dir {
            // No reparenting: no REFER needed.
            return self.check_path(new_dir, request1 | request2);
        }
        request1 |= fs::REFER;
        request2 |= fs::REFER;
        let below = |a: &Ancestry| a.keys.len().saturating_sub(common);
        let (mut parent1, allow1) = self.collect(old_dir, below(old_dir));
        let (mut parent2, allow2) = self.collect(new_dir, below(new_dir));
        if allow1 && allow2 {
            return Ok(());
        }
        let child1 = self.child_masks(old.key);
        let child2 = exchange.then(|| self.child_masks(new.and_then(|n| n.key)));
        let mount_root = Ancestry {
            keys: new_dir.keys[below(new_dir).saturating_sub(1)..].to_vec(),
            internal: new_dir.internal,
        };
        if self.paths_allowed_refer(
            &mount_root,
            request1,
            &mut parent1,
            &child1,
            old.is_dir(),
            request2,
            &mut parent2,
            child2.as_ref(),
            new.is_some_and(|n| n.is_dir()),
        ) {
            return Ok(());
        }
        if parent1.is_eacces(request1) || parent2.is_eacces(request2) {
            return Err(Error::Denied);
        }
        Err(Error::CrossDevice)
    }

    /// is_access_to_paths_allowed for a double (refer) request, walking
    /// from the common mount root up.
    #[allow(clippy::too_many_arguments)]
    fn paths_allowed_refer(
        &self,
        path: &Ancestry,
        request1: u32,
        parent1: &mut LayerMasks,
        child1: &LayerMasks,
        child1_dir: bool,
        request2: u32,
        parent2: &mut LayerMasks,
        child2: Option<&LayerMasks>,
        child2_dir: bool,
    ) -> bool {
        if path.internal {
            return true;
        }
        let mut allowed1 = parent1.allowed();
        let mut allowed2 = parent2.allowed();
        let mut dom_check = true;
        let mut keys = path.keys.iter();
        loop {
            if dom_check && no_more_access(parent1, child1, child1_dir, parent2, child2, child2_dir)
            {
                // From here on only the requested rights matter.
                dom_check = false;
                allowed1 = allowed1 || parent1.scope_to(request1);
                allowed2 = allowed2 || parent2.scope_to(request2);
                if allowed1 && allowed2 {
                    break;
                }
            }
            let Some(key) = keys.next() else {
                break;
            };
            if let Some(k) = key {
                allowed1 = allowed1 || Self::unmask(&self.fs_rules, k, parent1);
                allowed2 = allowed2 || Self::unmask(&self.fs_rules, k, parent2);
            }
            if allowed1 && allowed2 {
                break;
            }
        }
        allowed1 && allowed2
    }

    /// Whether `access` to `port` is allowed (hook_socket_bind, connect,
    /// sendmsg): EACCES if not.
    pub fn check_net(&self, port: u16, access: u32) -> Result<(), Error> {
        let (request, mut masks) = self.init_masks(access, Kind::Net);
        if request == 0 {
            return Ok(());
        }
        if Self::unmask(&self.net_rules, &u64::from(port), &mut masks) {
            return Ok(());
        }
        Err(Error::Denied)
    }

    /// The part of `request` an open of `path` is allowed, in one walk
    /// (hook_file_open): the open's own rights and the optional ones
    /// (TRUNCATE, IOCTL_DEV) the file keeps for its life. A right no layer
    /// handles is allowed.
    pub fn open_rights(&self, path: &Ancestry, request: u32) -> u32 {
        let (handled, mut masks) = self.init_masks(request, Kind::Fs);
        if handled == 0 || path.internal || masks.allowed() {
            return request;
        }
        for key in path.keys.iter().flatten() {
            if Self::unmask(&self.fs_rules, key, &mut masks) {
                return request;
            }
        }
        masks.0.iter().fold(request, |a, &m| a & !m)
    }
}

/// One object of a link or rename: its key and `S_IFMT` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub key: Option<ObjKey>,
    pub mode: u32,
}

impl Entry {
    fn is_dir(self) -> bool {
        self.mode & 0o170000 == 0o040000
    }
}

/// may_refer: the child would have no more rights under `new_parent`.
fn may_refer(
    src_parent: &LayerMasks,
    src_child: &LayerMasks,
    new_parent: &LayerMasks,
    child_dir: bool,
) -> bool {
    for i in 0..MAX_LAYERS {
        let mut child = src_parent.0[i] & src_child.0[i];
        let mut parent = new_parent.0[i];
        if !child_dir {
            child &= fs::FILE;
            parent &= fs::FILE;
        }
        // Every right still denied for the child must be denied at the new
        // parent too (access_mask_subset).
        if child & !parent != 0 {
            return false;
        }
    }
    true
}

/// no_more_access: both directions of an exchange.
fn no_more_access(
    parent1: &LayerMasks,
    child1: &LayerMasks,
    child1_dir: bool,
    parent2: &LayerMasks,
    child2: Option<&LayerMasks>,
    child2_dir: bool,
) -> bool {
    if !may_refer(parent1, child1, parent2, child1_dir) {
        return false;
    }
    match child2 {
        None => true,
        Some(c2) => may_refer(parent2, c2, parent1, child2_dir),
    }
}

/// A shared domain: what a thread holds.
pub type SharedDomain = Arc<Domain>;

#[cfg(test)]
mod tests {
    use super::*;

    fn masks(layers: &[(usize, u32)]) -> LayerMasks {
        let mut m = LayerMasks::default();
        for &(i, a) in layers {
            m.0[i] = a;
        }
        m
    }

    /// Linux's test_no_more_access (security/landlock/fs.c), case by case.
    #[test]
    fn no_more_access_matches_linux() {
        let rx0 = masks(&[(0, fs::EXECUTE | fs::READ_FILE)]);
        let mx0 = masks(&[(0, fs::EXECUTE | fs::MAKE_REG)]);
        let x0 = masks(&[(0, fs::EXECUTE)]);
        let x1 = masks(&[(1, fs::EXECUTE)]);
        let x01 = masks(&[(0, fs::EXECUTE), (1, fs::EXECUTE)]);
        let all = LayerMasks::default();
        let t =
            |p1, c1, d1, p2, c2: Option<&LayerMasks>, d2| no_more_access(p1, c1, d1, p2, c2, d2);
        assert!(t(&x0, &all, false, &all, None, false));
        assert!(t(&all, &x0, false, &all, None, false));
        assert!(!t(&x0, &x0, false, &all, None, false));
        assert!(t(&x0, &x0, false, &rx0, None, false));
        assert!(t(&rx0, &rx0, false, &rx0, None, false));
        assert!(!t(&rx0, &rx0, false, &x0, None, false));
        assert!(!t(&rx0, &rx0, false, &x1, None, false));
        assert!(t(&x0, &x1, false, &x0, None, false));
        assert!(t(&x1, &x0, false, &x0, None, false));
        assert!(t(&x0, &x01, false, &x0, None, false));
        assert!(t(&x0, &x01, false, &rx0, None, false));
        assert!(t(&x01, &x0, false, &x0, None, false));
        assert!(t(&x01, &x0, false, &rx0, None, false));
        assert!(!t(&x01, &x01, false, &x0, None, false));
        assert!(!t(&rx0, &rx0, true, &x0, None, false));
        assert!(t(&mx0, &mx0, false, &x0, None, false));
        assert!(!t(&mx0, &mx0, true, &x0, None, false));
        assert!(t(&mx0, &mx0, true, &mx0, Some(&mx0), true));
        assert!(t(&mx0, &mx0, true, &mx0, Some(&x0), true));
        assert!(!t(&mx0, &mx0, true, &x0, Some(&mx0), true));
        assert!(!t(&mx0, &mx0, true, &x0, Some(&x0), true));
        assert!(!t(&mx0, &mx0, true, &x1, Some(&x1), true));
        assert!(t(&mx0, &mx0, false, &mx0, Some(&mx0), false));
        assert!(t(&mx0, &mx0, false, &mx0, Some(&x0), false));
        assert!(t(&mx0, &mx0, false, &x0, Some(&mx0), false));
        assert!(t(&mx0, &mx0, false, &x0, Some(&x0), false));
        assert!(t(&rx0, &rx0, false, &rx0, Some(&rx0), false));
        assert!(t(&rx0, &rx0, false, &rx0, Some(&x0), false));
        assert!(!t(&rx0, &rx0, false, &x0, Some(&rx0), false));
        assert!(!t(&rx0, &rx0, false, &x0, Some(&x0), false));
        assert!(!t(&rx0, &rx0, false, &x1, Some(&x1), false));
        assert!(!t(&x1, &x1, false, &x0, None, false));
        assert!(!t(&x1, &x1, false, &rx0, None, false));
        assert!(!t(&x1, &x1, true, &x0, None, false));
        assert!(!t(&x1, &x1, true, &rx0, None, false));
        assert!(t(&x1, &x1, false, &x01, None, false));
        assert!(!t(&x1, &x1, false, &x01, Some(&x0), false));
        assert!(!t(&x1, &x1, false, &x01, Some(&x01), false));
        assert!(!t(&x1, &x1, false, &x0, Some(&x0), false));
        assert!(!t(&x1, &x1, false, &x0, Some(&x0), true));
        assert!(!t(&x1, &x1, true, &x0, Some(&x0), false));
        assert!(!t(&x1, &x1, true, &x0, Some(&x0), true));
    }

    /// Linux's scope_to_request and is_eacces tests.
    #[test]
    fn scope_to_request_and_is_eacces_match_linux() {
        let mut m = LayerMasks::default();
        assert!(m.scope_to(fs::EXECUTE));
        let mut m = masks(&[(0, fs::EXECUTE), (1, fs::WRITE_FILE)]);
        assert!(!m.scope_to(fs::EXECUTE));
        assert_eq!((m.0[0], m.0[1]), (fs::EXECUTE, 0));
        let mut m = masks(&[(0, fs::EXECUTE), (1, fs::WRITE_FILE)]);
        assert!(m.scope_to(0));
        let refer = masks(&[(0, fs::REFER)]);
        assert!(!refer.is_eacces(fs::REFER) && !refer.is_eacces(fs::WRITE_FILE));
        let write = masks(&[(0, fs::WRITE_FILE)]);
        assert!(write.is_eacces(fs::WRITE_FILE) && !write.is_eacces(fs::EXECUTE));
    }

    fn ruleset(fs_h: u32, net_h: u32) -> Ruleset {
        Ruleset::new(RulesetAttr {
            handled: AccessMasks {
                fs: fs_h,
                net: net_h,
                scope: 0,
            },
            quiet: AccessMasks::default(),
        })
        .unwrap()
    }

    fn path(keys: &[u64]) -> Ancestry {
        Ancestry {
            keys: keys.iter().map(|&k| Some((0, k))).collect(),
            internal: false,
        }
    }

    #[test]
    fn create_and_add_check_as_linux() {
        assert_eq!(
            Ruleset::new(RulesetAttr::default()).unwrap_err(),
            Error::NoMessage
        );
        let bad = RulesetAttr {
            handled: AccessMasks {
                fs: fs::MASK + 1,
                ..AccessMasks::default()
            },
            ..RulesetAttr::default()
        };
        assert_eq!(bad.check().unwrap_err(), Error::Invalid);
        let mut r = ruleset(fs::READ_FILE | fs::READ_DIR, net::BIND_TCP);
        assert_eq!(r.add_fs_rule((0, 1), true, 0, 0), Err(Error::NoMessage));
        assert_eq!(
            r.add_fs_rule((0, 1), true, fs::WRITE_FILE, 0),
            Err(Error::Invalid)
        );
        // READ_DIR on a file is EINVAL.
        assert_eq!(
            r.add_fs_rule((0, 1), false, fs::READ_DIR, 0),
            Err(Error::Invalid)
        );
        assert_eq!(
            r.add_fs_rule((0, 1), true, fs::READ_FILE, ADD_RULE_QUIET),
            Err(Error::Invalid)
        );
        assert_eq!(r.add_net_rule(70000, net::BIND_TCP, 0), Err(Error::Invalid));
        r.add_net_rule(8080, net::BIND_TCP, 0).unwrap();
    }

    /// Each layer must be satisfied, by any ancestor's rule; rights a layer
    /// does not handle are free.
    #[test]
    fn every_layer_must_grant() {
        let mut a = ruleset(fs::READ_FILE | fs::WRITE_FILE, 0);
        a.add_fs_rule((0, 10), true, fs::READ_FILE | fs::WRITE_FILE, 0)
            .unwrap();
        let d1 = Domain::restrict(None, &a, 1).unwrap();
        // /10/11/12: allowed below 10, denied elsewhere.
        assert!(d1
            .check_path(&path(&[12, 11, 10, 1]), fs::WRITE_FILE)
            .is_ok());
        assert_eq!(
            d1.check_path(&path(&[22, 20, 1]), fs::READ_FILE),
            Err(Error::Denied)
        );
        // Not handled: allowed.
        assert!(d1.check_path(&path(&[22, 20, 1]), fs::EXECUTE).is_ok());
        let mut b = ruleset(fs::WRITE_FILE, 0);
        b.add_fs_rule((0, 11), true, fs::WRITE_FILE, 0).unwrap();
        let d2 = Domain::restrict(Some(&d1), &b, 2).unwrap();
        assert!(d2
            .check_path(&path(&[12, 11, 10, 1]), fs::WRITE_FILE)
            .is_ok());
        // Under 10 but not 11: layer 2 denies.
        assert_eq!(
            d2.check_path(&path(&[13, 10, 1]), fs::WRITE_FILE),
            Err(Error::Denied)
        );
        assert!(d2.check_path(&path(&[13, 10, 1]), fs::READ_FILE).is_ok());
    }

    /// An open's rights are worked out together: what is not granted is
    /// dropped, what no layer handles kept.
    #[test]
    fn open_rights_drop_what_no_rule_grants() {
        let mut r = ruleset(fs::READ_FILE | fs::TRUNCATE, 0);
        r.add_fs_rule((0, 10), true, fs::READ_FILE, 0).unwrap();
        let d = Domain::restrict(None, &r, 1).unwrap();
        let want = fs::READ_FILE | fs::TRUNCATE | fs::IOCTL_DEV;
        assert_eq!(
            d.open_rights(&path(&[11, 10, 1]), want),
            fs::READ_FILE | fs::IOCTL_DEV
        );
        assert_eq!(d.open_rights(&path(&[21, 20, 1]), want), fs::IOCTL_DEV);
        let internal = Ancestry {
            keys: Vec::new(),
            internal: true,
        };
        assert_eq!(d.open_rights(&internal, want), want);
    }

    #[test]
    fn at_most_sixteen_layers() {
        let r = ruleset(fs::READ_FILE, 0);
        let mut d = Domain::restrict(None, &r, 1).unwrap();
        for i in 2..=16 {
            d = Domain::restrict(Some(&d), &r, i).unwrap();
        }
        assert_eq!(
            Domain::restrict(Some(&d), &r, 17).unwrap_err(),
            Error::TooBig
        );
    }

    #[test]
    fn net_rules_by_port() {
        let mut r = ruleset(0, net::BIND_TCP | net::CONNECT_TCP);
        r.add_net_rule(8080, net::BIND_TCP, 0).unwrap();
        let d = Domain::restrict(None, &r, 1).unwrap();
        assert!(d.check_net(8080, net::BIND_TCP).is_ok());
        assert_eq!(d.check_net(8080, net::CONNECT_TCP), Err(Error::Denied));
        assert_eq!(d.check_net(80, net::BIND_TCP), Err(Error::Denied));
        assert!(d.check_net(80, net::BIND_UDP).is_ok());
    }

    /// Renames: same directory needs only the make and remove rights;
    /// across directories REFER and no gain of rights, else EXDEV, with
    /// EACCES first.
    #[test]
    fn refer_as_linux() {
        let h = fs::MAKE_REG | fs::REMOVE_FILE | fs::READ_FILE | fs::REFER;
        let mut r = ruleset(h, 0);
        // /1 (root) / 10 (src dir) and /1 / 20 (dst dir), one mount.
        r.add_fs_rule((0, 10), true, fs::MAKE_REG | fs::REMOVE_FILE | fs::REFER, 0)
            .unwrap();
        r.add_fs_rule(
            (0, 20),
            true,
            fs::MAKE_REG | fs::REMOVE_FILE | fs::REFER | fs::READ_FILE,
            0,
        )
        .unwrap();
        let d = Domain::restrict(None, &r, 1).unwrap();
        let file = Entry {
            key: Some((0, 11)),
            mode: 0o100644,
        };
        let src = path(&[10, 1]);
        let dst = path(&[20, 1]);
        // Same directory.
        assert!(d
            .check_refer(file, None, &src, &src, 0, true, true, false)
            .is_ok());
        // 10 -> 20 gains READ_FILE: EXDEV.
        assert_eq!(
            d.check_refer(file, None, &src, &dst, 0, false, true, false),
            Err(Error::CrossDevice)
        );
        // 20 -> 10 loses rights: allowed.
        assert!(d
            .check_refer(file, None, &dst, &src, 0, false, true, false)
            .is_ok());
        // Into a directory with no rule: EACCES (no MAKE_REG there).
        let other = path(&[30, 1]);
        assert_eq!(
            d.check_refer(file, None, &dst, &other, 0, false, true, false),
            Err(Error::Denied)
        );
    }

    /// A domain that handles only network rights or scopes does not stop a
    /// rename across directories (PR #49 review); one with a filesystem
    /// layer under the network one does, through that layer.
    #[test]
    fn refer_needs_a_filesystem_layer() {
        let file = Entry {
            key: Some((0, 11)),
            mode: 0o100644,
        };
        let (src, dst) = (path(&[10, 1]), path(&[20, 1]));
        let net_only = Domain::restrict(None, &ruleset(0, net::BIND_TCP), 1).unwrap();
        assert!(net_only
            .check_refer(file, None, &src, &dst, 0, false, true, false)
            .is_ok());
        let fs_layer = Domain::restrict(None, &ruleset(fs::MAKE_REG, 0), 1).unwrap();
        let both = Domain::restrict(Some(&fs_layer), &ruleset(0, net::BIND_TCP), 2).unwrap();
        assert!(both
            .check_refer(file, None, &src, &dst, 0, false, true, false)
            .is_err());
    }

    /// A socket made in the same domain (or a lower one) is reached
    /// without a rule; one made elsewhere needs RESOLVE_UNIX.
    #[test]
    fn unix_sockets_of_the_same_domain_need_no_rule() {
        let r = ruleset(fs::RESOLVE_UNIX, 0);
        let parent = Domain::restrict(None, &r, 1).unwrap();
        let child = Domain::restrict(Some(&parent), &r, 2).unwrap();
        let sock = path(&[11, 10, 1]);
        assert!(parent
            .check_path_scoped(&sock, fs::RESOLVE_UNIX, Some(&parent))
            .is_ok());
        // The child's own layer is not the server's: denied.
        assert_eq!(
            child.check_path_scoped(&sock, fs::RESOLVE_UNIX, Some(&parent)),
            Err(Error::Denied)
        );
        assert!(child
            .check_path_scoped(&sock, fs::RESOLVE_UNIX, Some(&child))
            .is_ok());
        assert_eq!(
            parent.check_path_scoped(&sock, fs::RESOLVE_UNIX, None),
            Err(Error::Denied)
        );
        let mut allow = ruleset(fs::RESOLVE_UNIX, 0);
        allow
            .add_fs_rule((0, 10), true, fs::RESOLVE_UNIX, 0)
            .unwrap();
        let d = Domain::restrict(None, &allow, 3).unwrap();
        assert!(d.check_path_scoped(&sock, fs::RESOLVE_UNIX, None).is_ok());
    }

    /// Scopes and ptrace follow the layer hierarchy.
    #[test]
    fn scopes_follow_the_hierarchy() {
        let mut attr = RulesetAttr::default();
        attr.handled.scope = scope::SIGNAL;
        let r = Ruleset::new(attr).unwrap();
        let parent = Domain::restrict(None, &r, 1).unwrap();
        let child = Domain::restrict(Some(&parent), &r, 2).unwrap();
        let other = Domain::restrict(None, &r, 3).unwrap();
        // A domain reaches itself and the domains below it.
        assert!(!Domain::is_scoped(&parent, Some(&child), scope::SIGNAL));
        assert!(!Domain::is_scoped(&parent, Some(&parent), scope::SIGNAL));
        assert!(Domain::is_scoped(&child, Some(&parent), scope::SIGNAL));
        assert!(Domain::is_scoped(&parent, Some(&other), scope::SIGNAL));
        assert!(Domain::is_scoped(&parent, None, scope::SIGNAL));
        assert!(!Domain::is_scoped(
            &parent,
            None,
            scope::ABSTRACT_UNIX_SOCKET
        ));
        assert!(Domain::scope_le(Some(&parent), Some(&child)));
        assert!(!Domain::scope_le(Some(&child), Some(&parent)));
        assert!(!Domain::scope_le(Some(&parent), Some(&other)));
        assert!(Domain::scope_le(None, Some(&other)) && !Domain::scope_le(Some(&other), None));
    }
}
