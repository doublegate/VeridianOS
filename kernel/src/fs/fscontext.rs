//! The filesystem context a descriptor from fsopen or fspick names, as
//! Linux 7.3's `fs/fsopen.c` and `fs/fs_context.c`: fsconfig sets its
//! parameters and creates or reconfigures the filesystem, fsmount mounts
//! what it created, and read(2) returns the messages it logged, one per
//! call.
//!
//! The parameters are checked one at a time, as Linux's
//! `vfs_parse_fs_param`: the common superblock flags first (`ro`, `rw`,
//! `sync` and the rest), then the filesystem's own options
//! ([`mount::check_option`]), then `source`. The filesystem itself is made
//! by [`mount::new_filesystem`] from the options collected, as mount(2)
//! makes it from its option string.

use alloc::{collections::VecDeque, string::String, sync::Arc, vec::Vec};

use spin::Mutex;

use super::{
    mount::{self, Mounts},
    DirEntry, Filesystem, Metadata, NodeType, Permissions, VfsNode,
};
use crate::error::{FsError, KernelError};

/// How many messages a context keeps: the oldest goes when a ninth comes
/// (Linux's fc_log ring of 8).
pub const LOG_SIZE: usize = 8;

/// The most bytes of options a context collects: one page, the most data
/// mount(2) takes (copy_mount_options), so a context holder cannot grow the
/// heap without bound by repeating fsconfig (PR #47 review).
pub const MAX_OPTIONS: usize = 4096;

/// Where a context is in its life (Linux's fs_context_phase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// fsopen: taking parameters for a new filesystem.
    CreateParams,
    /// FSCONFIG_CMD_CREATE succeeded; fsmount may mount it.
    AwaitingMount,
    /// A create or reconfigure failed; only close is left.
    Failed,
    /// fspick, or a context after fsmount: taking parameters for a
    /// reconfiguration.
    ReconfParams,
}

/// What the context is for.
pub enum Purpose {
    /// A new filesystem of this (Linux) type.
    New { ty: String },
    /// Reconfiguring the filesystem of the mount `target` names (fspick,
    /// or a context after fsmount), of (Linux) type `ty`.
    Reconfigure { ty: String, target: Target },
}

/// The mount a reconfiguring context is for.
pub enum Target {
    /// `mount`, in `tree`, where it is now: picked at `point`, which it
    /// may have left since (PR #47 review).
    At {
        tree: Arc<Mounts>,
        point: String,
        mount: Arc<super::mount::Mount>,
    },
    /// `mount`, the root of the detached tree with this id, wherever that
    /// tree is now (`mount::detached_place`).
    Detached {
        id: u64,
        mount: Arc<super::mount::Mount>,
    },
}

/// A parameter's value, as fsconfig passes it.
pub enum Value<'a> {
    /// FSCONFIG_SET_FLAG.
    Flag,
    /// FSCONFIG_SET_STRING.
    String(&'a str),
    /// FSCONFIG_SET_BINARY, FSCONFIG_SET_PATH(_EMPTY) and FSCONFIG_SET_FD:
    /// none of the filesystems here takes one.
    Other,
}

/// A context's state, behind its lock (Linux's uapi_mutex).
pub struct FsContext {
    pub phase: Phase,
    pub purpose: Purpose,
    /// The filesystem's own options, comma-separated, in the order set.
    pub options: String,
    /// `source`, if set.
    pub source: Option<String>,
    /// `ro` (Some(true)) or `rw` (Some(false)), the last one set.
    pub sb_readonly: Option<bool>,
    /// The filesystem FSCONFIG_CMD_CREATE made.
    pub fs: Option<Arc<dyn Filesystem>>,
    /// Who opened the context: a new filesystem's root is theirs.
    pub owner: (u32, u32),
    /// The mount namespace the context was made in (fsopen, fspick). Its
    /// changes need MODIFY over that namespace, whichever namespace the
    /// caller is in when it makes them (PR #47 review).
    pub ns: Option<Arc<crate::process::ns::MntNs>>,
    log: VecDeque<String>,
}

impl FsContext {
    /// A context for `purpose`, in its first phase.
    pub fn new(purpose: Purpose, owner: (u32, u32)) -> Self {
        let phase = match purpose {
            Purpose::New { .. } => Phase::CreateParams,
            Purpose::Reconfigure { .. } => Phase::ReconfParams,
        };
        Self {
            phase,
            purpose,
            options: String::new(),
            source: None,
            sb_readonly: None,
            fs: None,
            owner,
            ns: None,
            log: VecDeque::new(),
        }
    }

    /// The (Linux) filesystem type.
    pub fn fs_type(&self) -> &str {
        match &self.purpose {
            Purpose::New { ty } | Purpose::Reconfigure { ty, .. } => ty,
        }
    }

    /// Log a message at `level` ('e', 'w' or 'i'), as Linux's logfc:
    /// `"<level> <message>\n"`, the oldest dropped from a full log.
    pub fn log(&mut self, level: char, message: &str) {
        if self.log.len() == LOG_SIZE {
            self.log.pop_front();
        }
        self.log.push_back(alloc::format!("{level} {message}\n"));
    }

    /// Log an error and return EINVAL (Linux's invalf).
    fn invalid(&mut self, message: &str) -> Result<(), CtxError> {
        self.log('e', message);
        Err(CtxError::Invalid)
    }

    /// The oldest message, taken off the log, if it fits in `len` bytes:
    /// ENODATA when there is none, EMSGSIZE (the message kept) when it is
    /// longer (Linux's fetch_message_locked).
    pub fn take_message(&mut self, len: usize) -> Result<String, KernelError> {
        let first = self
            .log
            .front()
            .ok_or(KernelError::FsError(FsError::NoData))?;
        if first.len() > len {
            return Err(KernelError::FsError(FsError::MessageTooLong));
        }
        Ok(self.log.pop_front().unwrap_or_default())
    }

    /// Set parameter `key` to `value` (Linux's vfs_parse_fs_param), in
    /// either parameter phase (EBUSY otherwise).
    pub fn set(&mut self, key: &str, value: Value<'_>) -> Result<(), CtxError> {
        if !matches!(self.phase, Phase::CreateParams | Phase::ReconfParams) {
            return Err(CtxError::Busy);
        }
        // The common superblock flags (vfs_parse_sb_flag), whatever the
        // value's type. Only SB_RDONLY means anything here.
        match key {
            "ro" => {
                self.sb_readonly = Some(true);
                return Ok(());
            }
            "rw" => {
                self.sb_readonly = Some(false);
                return Ok(());
            }
            "dirsync" | "lazytime" | "mand" | "sync" | "async" | "nolazytime" | "nomand" => {
                return Ok(())
            }
            _ => {}
        }
        // The filesystem's own options.
        let option = match value {
            Value::Flag => Some(String::from(key)),
            Value::String(v) => Some(alloc::format!("{key}={v}")),
            Value::Other => None,
        };
        if key != "source" {
            if let Some(option) = option {
                if !option.contains(',')
                    && mount::check_option(self.fs_type(), &option, self.owner).is_ok()
                {
                    let comma = usize::from(!self.options.is_empty());
                    if self.options.len() + comma + option.len() > MAX_OPTIONS {
                        let message = alloc::format!("{}: options too large", self.fs_type());
                        return self.invalid(&message);
                    }
                    if comma == 1 {
                        self.options.push(',');
                    }
                    self.options.push_str(&option);
                    return Ok(());
                }
            }
            let message = alloc::format!("{}: Unknown parameter '{}'", self.fs_type(), key);
            return self.invalid(&message);
        }
        // `source` (vfs_parse_fs_param_source).
        let Value::String(source) = value else {
            return self.invalid("Non-string source");
        };
        if self.source.is_some() {
            return self.invalid("Multiple sources");
        }
        self.source = Some(String::from(source));
        Ok(())
    }
}

/// Why a context refused a parameter or command, as Linux's errno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxError {
    /// EINVAL (with a message logged).
    Invalid,
    /// EBUSY: the wrong phase.
    Busy,
}

/// An open filesystem context (Linux's "[fscontext]" anonymous inode).
pub struct FsContextNode {
    pub ctx: Mutex<FsContext>,
}

impl FsContextNode {
    pub fn new(ctx: FsContext) -> Self {
        Self {
            ctx: Mutex::new(ctx),
        }
    }
}

impl VfsNode for FsContextNode {
    /// An anonymous inode, as on Linux.
    fn statfs(&self) -> Result<crate::fs::statfs::FsStats, KernelError> {
        Ok(crate::fs::statfs::FsStats::simple(
            crate::fs::statfs::ANON_INODE_FS_MAGIC,
        ))
    }

    /// A stream, as eventfd's: reads take no file position.
    fn node_type(&self) -> NodeType {
        NodeType::CharDevice
    }

    /// One message per call, without a NUL (fscontext_read).
    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        let message = self.ctx.lock().take_message(buffer.len())?;
        buffer[..message.len()].copy_from_slice(message.as_bytes());
        Ok(message.len())
    }

    fn write(&self, _offset: usize, _data: &[u8]) -> Result<usize, KernelError> {
        Err(KernelError::InvalidArgument {
            name: "fd",
            value: "a filesystem context cannot be written",
        })
    }

    fn as_any(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }

    fn metadata(&self) -> Result<Metadata, KernelError> {
        Ok(Metadata {
            size: 0,
            node_type: NodeType::CharDevice,
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
            value: "a filesystem context cannot be truncated",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpfs_ctx() -> FsContext {
        FsContext::new(
            Purpose::New {
                ty: String::from("tmpfs"),
            },
            (0, 0),
        )
    }

    /// Parameters are checked one at a time in Linux's order, each refusal
    /// logged as Linux words it, and read back one message at a time.
    #[test]
    fn parameters_are_checked_and_refusals_logged() {
        let mut c = tmpfs_ctx();
        assert_eq!(c.set("ro", Value::Flag), Ok(()));
        assert_eq!(c.sb_readonly, Some(true));
        assert_eq!(c.set("size", Value::String("1m")), Ok(()));
        assert_eq!(c.set("mode", Value::String("700")), Ok(()));
        assert_eq!(c.options, "size=1m,mode=700");
        assert_eq!(c.set("bogus", Value::Flag), Err(CtxError::Invalid));
        assert_eq!(c.set("size", Value::Other), Err(CtxError::Invalid));
        // A value cannot smuggle in a second option.
        assert_eq!(
            c.set("mode", Value::String("700,size=1")),
            Err(CtxError::Invalid)
        );
        assert_eq!(c.set("source", Value::Other), Err(CtxError::Invalid));
        assert_eq!(c.set("source", Value::String("none")), Ok(()));
        assert_eq!(
            c.set("source", Value::String("again")),
            Err(CtxError::Invalid)
        );
        let mut read = |len| c.take_message(len);
        assert_eq!(
            read(100).as_deref(),
            Ok("e tmpfs: Unknown parameter 'bogus'\n")
        );
        // Too small a buffer leaves the message queued.
        assert_eq!(read(4), Err(KernelError::FsError(FsError::MessageTooLong)));
        assert_eq!(
            read(100).as_deref(),
            Ok("e tmpfs: Unknown parameter 'size'\n")
        );
        assert_eq!(
            read(100).as_deref(),
            Ok("e tmpfs: Unknown parameter 'mode'\n")
        );
        assert_eq!(read(100).as_deref(), Ok("e Non-string source\n"));
        assert_eq!(read(100).as_deref(), Ok("e Multiple sources\n"));
        assert_eq!(read(100), Err(KernelError::FsError(FsError::NoData)));
    }

    /// The options a context collects stay within one page, as the data of
    /// mount(2) does: a parameter that would cross it is EINVAL and changes
    /// nothing (PR #47 review).
    #[test]
    fn options_stay_within_a_page() {
        let mut c = tmpfs_ctx();
        let mut ok = 0;
        while c.set("mode", Value::String("700")).is_ok() {
            ok += 1;
            assert!(ok < MAX_OPTIONS, "no bound");
        }
        assert!(c.options.len() <= MAX_OPTIONS);
        let before = c.options.clone();
        assert_eq!(c.set("mode", Value::String("700")), Err(CtxError::Invalid));
        assert_eq!(c.options, before);
        assert_eq!(
            c.take_message(100).as_deref(),
            Ok("e tmpfs: options too large\n")
        );
    }

    /// The log keeps the last eight messages; outside a parameter phase a
    /// parameter is EBUSY.
    #[test]
    fn the_log_keeps_eight_and_phases_gate_parameters() {
        let mut c = tmpfs_ctx();
        for i in 0..10 {
            c.log('i', &alloc::format!("m{i}"));
        }
        assert_eq!(c.take_message(64).as_deref(), Ok("i m2\n"));
        c.phase = Phase::AwaitingMount;
        assert_eq!(c.set("ro", Value::Flag), Err(CtxError::Busy));
        c.phase = Phase::Failed;
        assert_eq!(c.set("size", Value::String("1m")), Err(CtxError::Busy));
    }
}
