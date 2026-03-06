//! NFS v4 Client (RFC 7530)
//!
//! Implements NFS v4 compound operations with XDR encoding/decoding,
//! file handle management, and VFS mount point integration.

#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

// ---------------------------------------------------------------------------
// NFS File Handle
// ---------------------------------------------------------------------------

/// Maximum NFS file handle length (RFC 7530 Section 4).
const NFS4_FHSIZE: usize = 128;

/// Opaque NFS file handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfsFileHandle {
    /// Raw handle bytes (up to 128).
    data: [u8; NFS4_FHSIZE],
    /// Actual length of the handle.
    len: usize,
}

impl Default for NfsFileHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl NfsFileHandle {
    /// Create an empty file handle.
    pub fn new() -> Self {
        Self {
            data: [0u8; NFS4_FHSIZE],
            len: 0,
        }
    }

    /// Create a file handle from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > NFS4_FHSIZE {
            return None;
        }
        let mut fh = Self::new();
        fh.data[..bytes.len()].copy_from_slice(bytes);
        fh.len = bytes.len();
        Some(fh)
    }

    /// Get the handle bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Returns true if the handle is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

// ---------------------------------------------------------------------------
// NFS Types
// ---------------------------------------------------------------------------

/// NFS file type (nfs_ftype4 per RFC 7530 Section 5.8.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NfsFtype {
    Regular = 1,
    Directory = 2,
    BlockDevice = 3,
    CharDevice = 4,
    Symlink = 5,
    Socket = 6,
    Fifo = 7,
}

impl NfsFtype {
    /// Convert from wire value.
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::Regular),
            2 => Some(Self::Directory),
            3 => Some(Self::BlockDevice),
            4 => Some(Self::CharDevice),
            5 => Some(Self::Symlink),
            6 => Some(Self::Socket),
            7 => Some(Self::Fifo),
            _ => None,
        }
    }
}

/// NFS file attributes (fattr4 subset).
#[derive(Debug, Clone)]
pub struct NfsAttr {
    pub file_type: NfsFtype,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub used: u64,
    pub rdev: u64,
    pub fsid: u64,
    pub fileid: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

impl Default for NfsAttr {
    fn default() -> Self {
        Self {
            file_type: NfsFtype::Regular,
            mode: 0,
            nlink: 0,
            uid: 0,
            gid: 0,
            size: 0,
            used: 0,
            rdev: 0,
            fsid: 0,
            fileid: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// NFS Operations
// ---------------------------------------------------------------------------

/// NFS v4 operations (opcode values per RFC 7530 Section 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NfsOpcode {
    Access = 3,
    Close = 4,
    Commit = 5,
    Create = 6,
    GetAttr = 9,
    GetFH = 10,
    Lookup = 15,
    Open = 18,
    PutFH = 22,
    PutRootFH = 24,
    Read = 25,
    ReadDir = 26,
    Remove = 28,
    Rename = 29,
    SetAttr = 34,
    Write = 38,
}

/// NFS v4 operation with associated data.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub enum NfsOperation {
    Access {
        access_mask: u32,
    },
    Close {
        state_id: [u8; 16],
    },
    Commit {
        offset: u64,
        count: u32,
    },
    Create {
        name: String,
        file_type: NfsFtype,
    },
    GetAttr {
        attr_request: u64,
    },
    GetFH,
    Lookup {
        name: String,
    },
    Open {
        name: String,
        access: u32,
        deny: u32,
    },
    PutFH {
        handle: NfsFileHandle,
    },
    PutRootFH,
    Read {
        state_id: [u8; 16],
        offset: u64,
        count: u32,
    },
    ReadDir {
        cookie: u64,
        count: u32,
    },
    Remove {
        name: String,
    },
    Rename {
        old_name: String,
        new_name: String,
    },
    SetAttr {
        state_id: [u8; 16],
        attrs: NfsAttr,
    },
    Write {
        state_id: [u8; 16],
        offset: u64,
        data: Vec<u8>,
        stable: bool,
    },
}

/// NFS v4 status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NfsStatus {
    Ok = 0,
    Perm = 1,
    NoEnt = 2,
    IO = 5,
    Access = 13,
    Exist = 17,
    NotDir = 20,
    IsDir = 21,
    Inval = 22,
    FBig = 27,
    NoSpc = 28,
    RoFs = 30,
    Stale = 70,
    BadHandle = 10001,
    NotSupp = 10004,
    ServerFault = 10006,
}

impl NfsStatus {
    /// Convert from wire value.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Ok,
            1 => Self::Perm,
            2 => Self::NoEnt,
            5 => Self::IO,
            13 => Self::Access,
            17 => Self::Exist,
            20 => Self::NotDir,
            21 => Self::IsDir,
            22 => Self::Inval,
            27 => Self::FBig,
            28 => Self::NoSpc,
            30 => Self::RoFs,
            70 => Self::Stale,
            10001 => Self::BadHandle,
            10004 => Self::NotSupp,
            10006 => Self::ServerFault,
            _ => Self::ServerFault,
        }
    }
}

/// Result of an individual NFS operation.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub enum NfsResult {
    Access {
        status: NfsStatus,
        supported: u32,
        access: u32,
    },
    Close {
        status: NfsStatus,
    },
    Commit {
        status: NfsStatus,
    },
    Create {
        status: NfsStatus,
    },
    GetAttr {
        status: NfsStatus,
        attrs: Option<NfsAttr>,
    },
    GetFH {
        status: NfsStatus,
        handle: Option<NfsFileHandle>,
    },
    Lookup {
        status: NfsStatus,
    },
    Open {
        status: NfsStatus,
        state_id: [u8; 16],
    },
    PutFH {
        status: NfsStatus,
    },
    PutRootFH {
        status: NfsStatus,
    },
    Read {
        status: NfsStatus,
        eof: bool,
        data: Vec<u8>,
    },
    ReadDir {
        status: NfsStatus,
        entries: Vec<NfsDirEntry>,
    },
    Remove {
        status: NfsStatus,
    },
    Rename {
        status: NfsStatus,
    },
    SetAttr {
        status: NfsStatus,
    },
    Write {
        status: NfsStatus,
        count: u32,
        committed: bool,
    },
}

/// NFS directory entry.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct NfsDirEntry {
    pub cookie: u64,
    pub name: String,
    pub fileid: u64,
}

// ---------------------------------------------------------------------------
// Compound Request / Response
// ---------------------------------------------------------------------------

/// NFS v4 COMPOUND request (RFC 7530 Section 16.2).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CompoundRequest {
    pub tag: String,
    pub minor_version: u32,
    pub operations: Vec<NfsOperation>,
}

/// NFS v4 COMPOUND response.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct CompoundResponse {
    pub status: NfsStatus,
    pub tag: String,
    pub results: Vec<NfsResult>,
}

// ---------------------------------------------------------------------------
// XDR Encoder / Decoder
// ---------------------------------------------------------------------------

/// XDR (RFC 4506) encoder for NFS wire format.
#[cfg(feature = "alloc")]
pub struct XdrEncoder {
    buf: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl Default for XdrEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "alloc")]
impl XdrEncoder {
    /// Create a new encoder.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Create encoder with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Encode a u32.
    pub fn encode_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    /// Encode a u64.
    pub fn encode_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    /// Encode a bool.
    pub fn encode_bool(&mut self, v: bool) {
        self.encode_u32(if v { 1 } else { 0 });
    }

    /// Encode an opaque byte array (length-prefixed, padded to 4-byte
    /// boundary).
    pub fn encode_opaque(&mut self, data: &[u8]) {
        self.encode_u32(data.len() as u32);
        self.buf.extend_from_slice(data);
        // Pad to 4-byte boundary
        let pad = (4 - (data.len() % 4)) % 4;
        for _ in 0..pad {
            self.buf.push(0);
        }
    }

    /// Encode a string (same as opaque).
    pub fn encode_string(&mut self, s: &str) {
        self.encode_opaque(s.as_bytes());
    }

    /// Encode a fixed-size opaque array (no length prefix, padded).
    pub fn encode_opaque_fixed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        let pad = (4 - (data.len() % 4)) % 4;
        for _ in 0..pad {
            self.buf.push(0);
        }
    }

    /// Consume the encoder and return the buffer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Current encoded length.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// XDR decoder for NFS wire format.
#[cfg(feature = "alloc")]
pub struct XdrDecoder<'a> {
    data: &'a [u8],
    pos: usize,
}

#[cfg(feature = "alloc")]
impl<'a> XdrDecoder<'a> {
    /// Create a new decoder over a byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Decode a u32.
    pub fn decode_u32(&mut self) -> Option<u32> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        let v = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Some(v)
    }

    /// Decode a u64.
    pub fn decode_u64(&mut self) -> Option<u64> {
        let hi = self.decode_u32()? as u64;
        let lo = self.decode_u32()? as u64;
        Some((hi << 32) | lo)
    }

    /// Decode a bool.
    pub fn decode_bool(&mut self) -> Option<bool> {
        self.decode_u32().map(|v| v != 0)
    }

    /// Decode a variable-length opaque byte array.
    pub fn decode_opaque(&mut self) -> Option<Vec<u8>> {
        let len = self.decode_u32()? as usize;
        if self.pos + len > self.data.len() {
            return None;
        }
        let v = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        // Skip padding
        let pad = (4 - (len % 4)) % 4;
        self.pos += pad;
        Some(v)
    }

    /// Decode a string.
    pub fn decode_string(&mut self) -> Option<String> {
        let bytes = self.decode_opaque()?;
        String::from_utf8(bytes).ok()
    }

    /// Decode a fixed-size opaque array.
    pub fn decode_opaque_fixed(&mut self, len: usize) -> Option<Vec<u8>> {
        if self.pos + len > self.data.len() {
            return None;
        }
        let v = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        let pad = (4 - (len % 4)) % 4;
        self.pos += pad;
        Some(v)
    }

    /// Remaining bytes.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
}

// ---------------------------------------------------------------------------
// AUTH_SYS credentials
// ---------------------------------------------------------------------------

/// AUTH_SYS authentication credentials (RFC 5531).
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct AuthSys {
    pub stamp: u32,
    pub machine_name: String,
    pub uid: u32,
    pub gid: u32,
    pub gids: Vec<u32>,
}

#[cfg(feature = "alloc")]
impl Default for AuthSys {
    fn default() -> Self {
        Self {
            stamp: 0,
            machine_name: String::from("veridian"),
            uid: 0,
            gid: 0,
            gids: Vec::new(),
        }
    }
}

#[cfg(feature = "alloc")]
impl AuthSys {
    /// Encode AUTH_SYS credentials to XDR.
    pub fn encode(&self, enc: &mut XdrEncoder) {
        enc.encode_u32(self.stamp);
        enc.encode_string(&self.machine_name);
        enc.encode_u32(self.uid);
        enc.encode_u32(self.gid);
        enc.encode_u32(self.gids.len() as u32);
        for &g in &self.gids {
            enc.encode_u32(g);
        }
    }
}

// ---------------------------------------------------------------------------
// NFS Client
// ---------------------------------------------------------------------------

/// NFS error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NfsError {
    /// Server returned an NFS error status.
    Status(NfsStatus),
    /// XDR encoding/decoding error.
    XdrError,
    /// No file handle set (PUTFH/PUTROOTFH not issued).
    NoFileHandle,
    /// Network transport error.
    TransportError,
    /// Authentication failure.
    AuthError,
    /// Not connected to server.
    NotConnected,
    /// Invalid argument.
    InvalidArgument,
}

/// NFS v4 client.
#[cfg(feature = "alloc")]
pub struct NfsClient {
    /// Server address (IP:port string).
    server_addr: String,
    /// Root file handle obtained from PUTROOTFH.
    root_fh: NfsFileHandle,
    /// Current file handle (set by PUTFH/PUTROOTFH/LOOKUP).
    current_fh: NfsFileHandle,
    /// AUTH_SYS credentials.
    auth: AuthSys,
    /// Whether the client is connected.
    connected: bool,
    /// Next transaction ID.
    xid: u32,
}

#[cfg(feature = "alloc")]
impl NfsClient {
    /// Create a new NFS client for the given server.
    pub fn new(server_addr: String) -> Self {
        Self {
            server_addr,
            root_fh: NfsFileHandle::new(),
            current_fh: NfsFileHandle::new(),
            auth: AuthSys::default(),
            connected: false,
            xid: 1,
        }
    }

    /// Set authentication credentials.
    pub fn set_auth(&mut self, uid: u32, gid: u32, machine_name: String) {
        self.auth.uid = uid;
        self.auth.gid = gid;
        self.auth.machine_name = machine_name;
    }

    /// Mount the NFS export (PUTROOTFH + GETFH).
    pub fn mount(&mut self) -> Result<NfsFileHandle, NfsError> {
        let request = CompoundRequest {
            tag: String::from("mount"),
            minor_version: 0,
            operations: vec![NfsOperation::PutRootFH, NfsOperation::GetFH],
        };

        let response = self.send_compound(&request)?;

        // Extract root file handle from GETFH result
        for result in &response.results {
            if let NfsResult::GetFH {
                status,
                handle: Some(fh),
            } = result
            {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                self.root_fh = fh.clone();
                self.current_fh = fh.clone();
                return Ok(fh.clone());
            }
        }

        Err(NfsError::NoFileHandle)
    }

    /// Lookup a name relative to a directory file handle.
    pub fn lookup(
        &mut self,
        dir_fh: &NfsFileHandle,
        name: &str,
    ) -> Result<NfsFileHandle, NfsError> {
        let request = CompoundRequest {
            tag: String::from("lookup"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH {
                    handle: dir_fh.clone(),
                },
                NfsOperation::Lookup {
                    name: String::from(name),
                },
                NfsOperation::GetFH,
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::GetFH {
                status,
                handle: Some(fh),
            } = result
            {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                self.current_fh = fh.clone();
                return Ok(fh.clone());
            }
        }

        Err(NfsError::NoFileHandle)
    }

    /// Read data from a file.
    pub fn read(
        &mut self,
        fh: &NfsFileHandle,
        offset: u64,
        count: u32,
    ) -> Result<(Vec<u8>, bool), NfsError> {
        let request = CompoundRequest {
            tag: String::from("read"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH { handle: fh.clone() },
                NfsOperation::Read {
                    state_id: [0u8; 16],
                    offset,
                    count,
                },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::Read { status, eof, data } = result {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok((data.clone(), *eof));
            }
        }

        Err(NfsError::XdrError)
    }

    /// Write data to a file.
    pub fn write(
        &mut self,
        fh: &NfsFileHandle,
        offset: u64,
        data: &[u8],
        stable: bool,
    ) -> Result<(u32, bool), NfsError> {
        let request = CompoundRequest {
            tag: String::from("write"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH { handle: fh.clone() },
                NfsOperation::Write {
                    state_id: [0u8; 16],
                    offset,
                    data: data.to_vec(),
                    stable,
                },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::Write {
                status,
                count,
                committed,
            } = result
            {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok((*count, *committed));
            }
        }

        Err(NfsError::XdrError)
    }

    /// Read directory entries.
    pub fn readdir(
        &mut self,
        dir_fh: &NfsFileHandle,
        cookie: u64,
        count: u32,
    ) -> Result<Vec<NfsDirEntry>, NfsError> {
        let request = CompoundRequest {
            tag: String::from("readdir"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH {
                    handle: dir_fh.clone(),
                },
                NfsOperation::ReadDir { cookie, count },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::ReadDir { status, entries } = result {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok(entries.clone());
            }
        }

        Err(NfsError::XdrError)
    }

    /// Create a file or directory.
    pub fn create(
        &mut self,
        dir_fh: &NfsFileHandle,
        name: &str,
        file_type: NfsFtype,
    ) -> Result<NfsFileHandle, NfsError> {
        let request = CompoundRequest {
            tag: String::from("create"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH {
                    handle: dir_fh.clone(),
                },
                NfsOperation::Create {
                    name: String::from(name),
                    file_type,
                },
                NfsOperation::GetFH,
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::GetFH {
                status,
                handle: Some(fh),
            } = result
            {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok(fh.clone());
            }
        }

        Err(NfsError::NoFileHandle)
    }

    /// Remove a file or directory.
    pub fn remove(&mut self, dir_fh: &NfsFileHandle, name: &str) -> Result<(), NfsError> {
        let request = CompoundRequest {
            tag: String::from("remove"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH {
                    handle: dir_fh.clone(),
                },
                NfsOperation::Remove {
                    name: String::from(name),
                },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::Remove { status } = result {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok(());
            }
        }

        Err(NfsError::XdrError)
    }

    /// Get file attributes.
    pub fn getattr(&mut self, fh: &NfsFileHandle) -> Result<NfsAttr, NfsError> {
        // Request all standard attributes
        let attr_request = 0xFFFF_FFFF_FFFF_FFFF;

        let request = CompoundRequest {
            tag: String::from("getattr"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH { handle: fh.clone() },
                NfsOperation::GetAttr { attr_request },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::GetAttr {
                status,
                attrs: Some(a),
            } = result
            {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok(a.clone());
            }
        }

        Err(NfsError::XdrError)
    }

    /// Set file attributes.
    pub fn setattr(&mut self, fh: &NfsFileHandle, attrs: NfsAttr) -> Result<(), NfsError> {
        let request = CompoundRequest {
            tag: String::from("setattr"),
            minor_version: 0,
            operations: vec![
                NfsOperation::PutFH { handle: fh.clone() },
                NfsOperation::SetAttr {
                    state_id: [0u8; 16],
                    attrs,
                },
            ],
        };

        let response = self.send_compound(&request)?;

        for result in &response.results {
            if let NfsResult::SetAttr { status } = result {
                if *status != NfsStatus::Ok {
                    return Err(NfsError::Status(*status));
                }
                return Ok(());
            }
        }

        Err(NfsError::XdrError)
    }

    /// Build an XDR-encoded compound request.
    pub fn build_compound(&self, req: &CompoundRequest) -> Vec<u8> {
        let mut enc = XdrEncoder::with_capacity(256);

        // RPC header (simplified)
        enc.encode_u32(self.xid);
        enc.encode_u32(0); // call
        enc.encode_u32(2); // RPC version 2
        enc.encode_u32(100003); // NFS program
        enc.encode_u32(4); // NFS v4
        enc.encode_u32(1); // COMPOUND procedure

        // AUTH_SYS
        enc.encode_u32(1); // AUTH_SYS flavor
        self.auth.encode(&mut enc);

        // COMPOUND body
        enc.encode_string(&req.tag);
        enc.encode_u32(req.minor_version);
        enc.encode_u32(req.operations.len() as u32);

        for op in &req.operations {
            self.encode_operation(&mut enc, op);
        }

        enc.into_bytes()
    }

    /// Parse an XDR-encoded compound response.
    pub fn parse_compound(&self, data: &[u8]) -> Result<CompoundResponse, NfsError> {
        let mut dec = XdrDecoder::new(data);

        // Skip RPC reply header
        let _xid = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let _msg_type = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let _reply_stat = dec.decode_u32().ok_or(NfsError::XdrError)?;

        // Skip verifier
        let _verf_flavor = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let _verf_body = dec.decode_opaque().ok_or(NfsError::XdrError)?;

        // Accept stat
        let _accept_stat = dec.decode_u32().ok_or(NfsError::XdrError)?;

        // COMPOUND response
        let status_val = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let status = NfsStatus::from_u32(status_val);
        let tag = dec.decode_string().ok_or(NfsError::XdrError)?;
        let num_results = dec.decode_u32().ok_or(NfsError::XdrError)? as usize;

        let mut results = Vec::with_capacity(num_results);
        for _ in 0..num_results {
            let result = self.decode_result(&mut dec)?;
            results.push(result);
        }

        Ok(CompoundResponse {
            status,
            tag,
            results,
        })
    }

    /// Encode a single NFS operation to XDR.
    fn encode_operation(&self, enc: &mut XdrEncoder, op: &NfsOperation) {
        match op {
            NfsOperation::Access { access_mask } => {
                enc.encode_u32(NfsOpcode::Access as u32);
                enc.encode_u32(*access_mask);
            }
            NfsOperation::Close { state_id } => {
                enc.encode_u32(NfsOpcode::Close as u32);
                enc.encode_u32(0); // seqid
                enc.encode_opaque_fixed(state_id);
            }
            NfsOperation::Commit { offset, count } => {
                enc.encode_u32(NfsOpcode::Commit as u32);
                enc.encode_u64(*offset);
                enc.encode_u32(*count);
            }
            NfsOperation::Create { name, file_type } => {
                enc.encode_u32(NfsOpcode::Create as u32);
                enc.encode_u32(*file_type as u32);
                enc.encode_string(name);
            }
            NfsOperation::GetAttr { attr_request } => {
                enc.encode_u32(NfsOpcode::GetAttr as u32);
                enc.encode_u64(*attr_request);
            }
            NfsOperation::GetFH => {
                enc.encode_u32(NfsOpcode::GetFH as u32);
            }
            NfsOperation::Lookup { name } => {
                enc.encode_u32(NfsOpcode::Lookup as u32);
                enc.encode_string(name);
            }
            NfsOperation::Open { name, access, deny } => {
                enc.encode_u32(NfsOpcode::Open as u32);
                enc.encode_u32(0); // seqid
                enc.encode_u32(*access);
                enc.encode_u32(*deny);
                enc.encode_string(name);
            }
            NfsOperation::PutFH { handle } => {
                enc.encode_u32(NfsOpcode::PutFH as u32);
                enc.encode_opaque(handle.as_bytes());
            }
            NfsOperation::PutRootFH => {
                enc.encode_u32(NfsOpcode::PutRootFH as u32);
            }
            NfsOperation::Read {
                state_id,
                offset,
                count,
            } => {
                enc.encode_u32(NfsOpcode::Read as u32);
                enc.encode_opaque_fixed(state_id);
                enc.encode_u64(*offset);
                enc.encode_u32(*count);
            }
            NfsOperation::ReadDir { cookie, count } => {
                enc.encode_u32(NfsOpcode::ReadDir as u32);
                enc.encode_u64(*cookie);
                enc.encode_u32(*count);
            }
            NfsOperation::Remove { name } => {
                enc.encode_u32(NfsOpcode::Remove as u32);
                enc.encode_string(name);
            }
            NfsOperation::Rename { old_name, new_name } => {
                enc.encode_u32(NfsOpcode::Rename as u32);
                enc.encode_string(old_name);
                enc.encode_string(new_name);
            }
            NfsOperation::SetAttr { state_id, attrs } => {
                enc.encode_u32(NfsOpcode::SetAttr as u32);
                enc.encode_opaque_fixed(state_id);
                enc.encode_u32(attrs.mode);
                enc.encode_u64(attrs.size);
            }
            NfsOperation::Write {
                state_id,
                offset,
                data,
                stable,
            } => {
                enc.encode_u32(NfsOpcode::Write as u32);
                enc.encode_opaque_fixed(state_id);
                enc.encode_u64(*offset);
                enc.encode_u32(if *stable { 2 } else { 0 }); // FILE_SYNC / UNSTABLE4
                enc.encode_opaque(data);
            }
        }
    }

    /// Decode a single NFS result from XDR.
    fn decode_result(&self, dec: &mut XdrDecoder) -> Result<NfsResult, NfsError> {
        let opcode = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let status_val = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let status = NfsStatus::from_u32(status_val);

        match opcode {
            3 => {
                // ACCESS
                let supported = if status == NfsStatus::Ok {
                    dec.decode_u32().ok_or(NfsError::XdrError)?
                } else {
                    0
                };
                let access = if status == NfsStatus::Ok {
                    dec.decode_u32().ok_or(NfsError::XdrError)?
                } else {
                    0
                };
                Ok(NfsResult::Access {
                    status,
                    supported,
                    access,
                })
            }
            4 => Ok(NfsResult::Close { status }),
            5 => Ok(NfsResult::Commit { status }),
            6 => Ok(NfsResult::Create { status }),
            9 => {
                // GETATTR
                let attrs = if status == NfsStatus::Ok {
                    Some(self.decode_fattr(dec)?)
                } else {
                    None
                };
                Ok(NfsResult::GetAttr { status, attrs })
            }
            10 => {
                // GETFH
                let handle = if status == NfsStatus::Ok {
                    let fh_bytes = dec.decode_opaque().ok_or(NfsError::XdrError)?;
                    NfsFileHandle::from_bytes(&fh_bytes)
                } else {
                    None
                };
                Ok(NfsResult::GetFH { status, handle })
            }
            15 => Ok(NfsResult::Lookup { status }),
            18 => {
                // OPEN
                let mut state_id = [0u8; 16];
                if status == NfsStatus::Ok {
                    let sid = dec.decode_opaque_fixed(16).ok_or(NfsError::XdrError)?;
                    state_id.copy_from_slice(&sid);
                }
                Ok(NfsResult::Open { status, state_id })
            }
            22 => Ok(NfsResult::PutFH { status }),
            24 => Ok(NfsResult::PutRootFH { status }),
            25 => {
                // READ
                let (eof, data) = if status == NfsStatus::Ok {
                    let eof = dec.decode_bool().ok_or(NfsError::XdrError)?;
                    let d = dec.decode_opaque().ok_or(NfsError::XdrError)?;
                    (eof, d)
                } else {
                    (false, Vec::new())
                };
                Ok(NfsResult::Read { status, eof, data })
            }
            26 => {
                // READDIR
                let entries = if status == NfsStatus::Ok {
                    self.decode_dir_entries(dec)?
                } else {
                    Vec::new()
                };
                Ok(NfsResult::ReadDir { status, entries })
            }
            28 => Ok(NfsResult::Remove { status }),
            29 => Ok(NfsResult::Rename { status }),
            34 => Ok(NfsResult::SetAttr { status }),
            38 => {
                // WRITE
                let (count, committed) = if status == NfsStatus::Ok {
                    let c = dec.decode_u32().ok_or(NfsError::XdrError)?;
                    let com = dec.decode_u32().ok_or(NfsError::XdrError)?;
                    (c, com == 2)
                } else {
                    (0, false)
                };
                Ok(NfsResult::Write {
                    status,
                    count,
                    committed,
                })
            }
            _ => Err(NfsError::XdrError),
        }
    }

    /// Decode file attributes from XDR.
    fn decode_fattr(&self, dec: &mut XdrDecoder) -> Result<NfsAttr, NfsError> {
        let file_type_val = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let file_type = NfsFtype::from_u32(file_type_val).ok_or(NfsError::XdrError)?;
        let mode = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let nlink = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let uid = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let gid = dec.decode_u32().ok_or(NfsError::XdrError)?;
        let size = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let used = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let rdev = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let fsid = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let fileid = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let atime = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let mtime = dec.decode_u64().ok_or(NfsError::XdrError)?;
        let ctime = dec.decode_u64().ok_or(NfsError::XdrError)?;

        Ok(NfsAttr {
            file_type,
            mode,
            nlink,
            uid,
            gid,
            size,
            used,
            rdev,
            fsid,
            fileid,
            atime,
            mtime,
            ctime,
        })
    }

    /// Decode directory entries from XDR.
    fn decode_dir_entries(&self, dec: &mut XdrDecoder) -> Result<Vec<NfsDirEntry>, NfsError> {
        let count = dec.decode_u32().ok_or(NfsError::XdrError)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let cookie = dec.decode_u64().ok_or(NfsError::XdrError)?;
            let name = dec.decode_string().ok_or(NfsError::XdrError)?;
            let fileid = dec.decode_u64().ok_or(NfsError::XdrError)?;
            entries.push(NfsDirEntry {
                cookie,
                name,
                fileid,
            });
        }
        Ok(entries)
    }

    /// Send a compound request and receive response.
    ///
    /// In a real implementation this would use the network stack.
    /// Currently returns a stub error for transport.
    fn send_compound(&mut self, req: &CompoundRequest) -> Result<CompoundResponse, NfsError> {
        let _encoded = self.build_compound(req);
        self.xid += 1;

        // Transport layer placeholder -- would send via TCP to self.server_addr
        // and receive the response bytes, then call parse_compound().
        Err(NfsError::TransportError)
    }

    /// Get the root file handle.
    pub fn root_fh(&self) -> &NfsFileHandle {
        &self.root_fh
    }

    /// Get the current file handle.
    pub fn current_fh(&self) -> &NfsFileHandle {
        &self.current_fh
    }

    /// Get the server address.
    pub fn server_addr(&self) -> &str {
        &self.server_addr
    }
}

// ---------------------------------------------------------------------------
// VFS Mount Integration
// ---------------------------------------------------------------------------

/// NFS mount point for VFS integration.
#[cfg(feature = "alloc")]
pub struct NfsMountPoint {
    /// Path where NFS share is mounted.
    pub mount_path: String,
    /// NFS export path on the server.
    pub export_path: String,
    /// NFS client for this mount.
    pub client: NfsClient,
}

#[cfg(feature = "alloc")]
impl NfsMountPoint {
    /// Create a new NFS mount point.
    pub fn new(server: &str, export: &str, mount_path: &str) -> Self {
        Self {
            mount_path: String::from(mount_path),
            export_path: String::from(export),
            client: NfsClient::new(String::from(server)),
        }
    }

    /// Mount the NFS export.
    pub fn mount(&mut self) -> Result<(), NfsError> {
        self.client.mount()?;
        Ok(())
    }

    /// Get the mount path.
    pub fn path(&self) -> &str {
        &self.mount_path
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_handle_new() {
        let fh = NfsFileHandle::new();
        assert!(fh.is_empty());
        assert_eq!(fh.as_bytes().len(), 0);
    }

    #[test]
    fn test_file_handle_from_bytes() {
        let data = [1, 2, 3, 4, 5];
        let fh = NfsFileHandle::from_bytes(&data).unwrap();
        assert_eq!(fh.as_bytes(), &data);
        assert!(!fh.is_empty());
    }

    #[test]
    fn test_file_handle_too_large() {
        let data = [0u8; NFS4_FHSIZE + 1];
        assert!(NfsFileHandle::from_bytes(&data).is_none());
    }

    #[test]
    fn test_file_handle_max_size() {
        let data = [0xAB; NFS4_FHSIZE];
        let fh = NfsFileHandle::from_bytes(&data).unwrap();
        assert_eq!(fh.as_bytes().len(), NFS4_FHSIZE);
    }

    #[test]
    fn test_nfs_ftype_roundtrip() {
        assert_eq!(NfsFtype::from_u32(1), Some(NfsFtype::Regular));
        assert_eq!(NfsFtype::from_u32(2), Some(NfsFtype::Directory));
        assert_eq!(NfsFtype::from_u32(7), Some(NfsFtype::Fifo));
        assert_eq!(NfsFtype::from_u32(0), None);
        assert_eq!(NfsFtype::from_u32(8), None);
    }

    #[test]
    fn test_nfs_status_roundtrip() {
        assert_eq!(NfsStatus::from_u32(0), NfsStatus::Ok);
        assert_eq!(NfsStatus::from_u32(2), NfsStatus::NoEnt);
        assert_eq!(NfsStatus::from_u32(70), NfsStatus::Stale);
        assert_eq!(NfsStatus::from_u32(10001), NfsStatus::BadHandle);
    }

    #[test]
    fn test_xdr_encode_u32() {
        let mut enc = XdrEncoder::new();
        enc.encode_u32(0x12345678);
        let bytes = enc.into_bytes();
        assert_eq!(bytes, &[0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_xdr_encode_u64() {
        let mut enc = XdrEncoder::new();
        enc.encode_u64(0x0102030405060708);
        let bytes = enc.into_bytes();
        assert_eq!(bytes, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    }

    #[test]
    fn test_xdr_encode_decode_string() {
        let mut enc = XdrEncoder::new();
        enc.encode_string("hello");
        let bytes = enc.into_bytes();

        let mut dec = XdrDecoder::new(&bytes);
        let s = dec.decode_string().unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_xdr_opaque_padding() {
        let mut enc = XdrEncoder::new();
        enc.encode_opaque(&[1, 2, 3]); // 3 bytes + 1 byte pad
        let bytes = enc.into_bytes();
        // Length(4) + data(3) + pad(1) = 8
        assert_eq!(bytes.len(), 8);

        let mut dec = XdrDecoder::new(&bytes);
        let data = dec.decode_opaque().unwrap();
        assert_eq!(data, &[1, 2, 3]);
    }

    #[test]
    fn test_xdr_bool() {
        let mut enc = XdrEncoder::new();
        enc.encode_bool(true);
        enc.encode_bool(false);
        let bytes = enc.into_bytes();

        let mut dec = XdrDecoder::new(&bytes);
        assert!(dec.decode_bool().unwrap());
        assert!(!dec.decode_bool().unwrap());
    }

    #[test]
    fn test_xdr_decode_empty() {
        let dec_data: &[u8] = &[];
        let mut dec = XdrDecoder::new(dec_data);
        assert!(dec.decode_u32().is_none());
        assert_eq!(dec.remaining(), 0);
    }

    #[test]
    fn test_nfs_client_new() {
        let client = NfsClient::new(String::from("192.168.1.100:2049"));
        assert_eq!(client.server_addr(), "192.168.1.100:2049");
        assert!(client.root_fh().is_empty());
        assert!(client.current_fh().is_empty());
    }

    #[test]
    fn test_nfs_client_build_compound() {
        let client = NfsClient::new(String::from("10.0.0.1:2049"));
        let req = CompoundRequest {
            tag: String::from("test"),
            minor_version: 0,
            operations: vec![NfsOperation::PutRootFH, NfsOperation::GetFH],
        };
        let encoded = client.build_compound(&req);
        assert!(!encoded.is_empty());
        // Should contain at least RPC header + compound header + 2 ops
        assert!(encoded.len() > 40);
    }
}
