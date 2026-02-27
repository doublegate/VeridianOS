//! Wayland Wire Protocol Parser/Serializer
//!
//! Implements the Wayland wire protocol message format for kernel-side
//! protocol handling. Messages use a fixed header followed by typed arguments.
//!
//! Wire format: `[object_id: u32][size_opcode: u32][arguments...]`
//! - size = upper 16 bits of second word (total message size in bytes)
//! - opcode = lower 16 bits of second word

use alloc::{vec, vec::Vec};

use crate::error::KernelError;

// ---------------------------------------------------------------------------
// Wayland protocol constants
// ---------------------------------------------------------------------------

/// Minimum message size: object_id (4) + size_opcode (4)
const HEADER_SIZE: usize = 8;

/// Maximum message size (64 KB per Wayland spec)
#[allow(dead_code)] // Phase 6: used in future message size validation
const MAX_MESSAGE_SIZE: usize = 65536;

// -- Well-known interface identifiers used during dispatch -------------------

/// wl_display interface -- object ID 1 is always the display
pub const WL_DISPLAY_ID: u32 = 1;

// wl_display opcodes (requests)
/// Client requests: sync
pub const WL_DISPLAY_SYNC: u16 = 0;
/// Client requests: get_registry
pub const WL_DISPLAY_GET_REGISTRY: u16 = 1;

// wl_display event opcodes (server -> client)
/// Server events: error
pub const WL_DISPLAY_ERROR: u16 = 0;
/// Server events: delete_id
pub const WL_DISPLAY_DELETE_ID: u16 = 1;

// wl_registry opcodes (events server -> client)
/// Registry announces a global
pub const WL_REGISTRY_GLOBAL: u16 = 0;
/// Registry removes a global
#[allow(dead_code)] // Phase 6: emitted when globals are removed at runtime
pub const WL_REGISTRY_GLOBAL_REMOVE: u16 = 1;

// wl_registry opcodes (requests client -> server)
/// Client binds a global
pub const WL_REGISTRY_BIND: u16 = 0;

// wl_compositor opcodes (requests)
/// create_surface
pub const WL_COMPOSITOR_CREATE_SURFACE: u16 = 0;
/// create_region
#[allow(dead_code)] // Phase 6: region-based input/clipping
pub const WL_COMPOSITOR_CREATE_REGION: u16 = 1;

// wl_shm opcodes (requests)
/// create_pool
pub const WL_SHM_CREATE_POOL: u16 = 0;

// wl_shm event opcodes
/// format announcement
pub const WL_SHM_FORMAT: u16 = 0;

// wl_shm_pool opcodes (requests)
/// create_buffer
pub const WL_SHM_POOL_CREATE_BUFFER: u16 = 0;
/// destroy
#[allow(dead_code)] // Phase 6: pool lifecycle management
pub const WL_SHM_POOL_DESTROY: u16 = 2;

// wl_surface opcodes (requests)
/// destroy
#[allow(dead_code)] // Phase 6: surface destruction
pub const WL_SURFACE_DESTROY: u16 = 0;
/// attach
pub const WL_SURFACE_ATTACH: u16 = 1;
/// damage
pub const WL_SURFACE_DAMAGE: u16 = 2;
/// frame
#[allow(dead_code)] // Phase 6: frame callback for vsync
pub const WL_SURFACE_FRAME: u16 = 3;
/// commit
pub const WL_SURFACE_COMMIT: u16 = 6;

// wl_surface event opcodes
/// enter output
#[allow(dead_code)] // Phase 6: multi-output support
pub const WL_SURFACE_ENTER: u16 = 0;

// Pixel format constants matching Wayland wl_shm.format enum
/// ARGB8888 format code
pub const WL_SHM_FORMAT_ARGB8888: u32 = 0;
/// XRGB8888 format code
pub const WL_SHM_FORMAT_XRGB8888: u32 = 1;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Wayland protocol-specific error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaylandError {
    /// Message is too short to contain a valid header
    MessageTooShort,
    /// Declared size in header does not match available data
    SizeMismatch { declared: usize, available: usize },
    /// String argument is not properly NUL-terminated
    InvalidString,
    /// Unsupported argument encoding encountered
    InvalidArgument,
    /// Object ID not found in client object map
    UnknownObject { id: u32 },
    /// Opcode not recognized for the target interface
    UnknownOpcode { object_id: u32, opcode: u16 },
    /// A required new_id argument was missing
    #[allow(dead_code)] // Phase 6: validated during bind dispatching
    MissingNewId,
}

impl From<WaylandError> for KernelError {
    fn from(e: WaylandError) -> Self {
        match e {
            WaylandError::MessageTooShort => KernelError::InvalidArgument {
                name: "wayland_message",
                value: "too short",
            },
            WaylandError::SizeMismatch { .. } => KernelError::InvalidArgument {
                name: "wayland_message_size",
                value: "mismatch",
            },
            WaylandError::InvalidString => KernelError::InvalidArgument {
                name: "wayland_string",
                value: "invalid encoding",
            },
            WaylandError::InvalidArgument => KernelError::InvalidArgument {
                name: "wayland_argument",
                value: "invalid",
            },
            WaylandError::UnknownObject { id } => KernelError::NotFound {
                resource: "wayland_object",
                id: id as u64,
            },
            WaylandError::UnknownOpcode { .. } => KernelError::OperationNotSupported {
                operation: "wayland opcode",
            },
            WaylandError::MissingNewId => KernelError::InvalidArgument {
                name: "wayland_new_id",
                value: "missing",
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Message header
// ---------------------------------------------------------------------------

/// Wayland message header (8 bytes on the wire)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MessageHeader {
    /// Target object ID
    pub object_id: u32,
    /// Combined size (upper 16) and opcode (lower 16)
    pub size_opcode: u32,
}

impl MessageHeader {
    /// Extract the opcode from the combined word.
    pub fn opcode(&self) -> u16 {
        (self.size_opcode & 0xFFFF) as u16
    }

    /// Extract the total message size from the combined word.
    pub fn size(&self) -> u16 {
        ((self.size_opcode >> 16) & 0xFFFF) as u16
    }

    /// Build the combined size_opcode word.
    pub fn encode(opcode: u16, size: u16) -> u32 {
        ((size as u32) << 16) | (opcode as u32)
    }
}

// ---------------------------------------------------------------------------
// Argument types
// ---------------------------------------------------------------------------

/// Typed argument in a Wayland message.
#[derive(Debug, Clone)]
pub enum Argument {
    /// Signed 32-bit integer
    Int(i32),
    /// Unsigned 32-bit integer
    Uint(u32),
    /// Fixed-point number (24.8 format, stored as i32)
    Fixed(i32),
    /// Length-prefixed, NUL-terminated UTF-8 string
    String(Vec<u8>),
    /// Reference to an existing object (0 = null)
    Object(u32),
    /// Newly allocated object ID
    NewId(u32),
    /// Length-prefixed byte array
    Array(Vec<u8>),
    /// File descriptor (out-of-band, stored as i32 index)
    Fd(i32),
}

// ---------------------------------------------------------------------------
// Parsed message
// ---------------------------------------------------------------------------

/// A fully parsed Wayland protocol message.
#[derive(Debug, Clone)]
pub struct WaylandMessage {
    /// Target object ID
    pub object_id: u32,
    /// Opcode (method on the target interface)
    pub opcode: u16,
    /// Parsed argument list
    pub args: Vec<Argument>,
}

impl WaylandMessage {
    /// Convenience constructor.
    pub fn new(object_id: u32, opcode: u16, args: Vec<Argument>) -> Self {
        Self {
            object_id,
            opcode,
            args,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser helpers
// ---------------------------------------------------------------------------

/// Read a little-endian u32 from a byte slice at the given offset.
/// Returns `None` if out of bounds.
fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }
    Some(u32::from_ne_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Read a little-endian i32 from a byte slice at the given offset.
fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    read_u32(data, offset).map(|v| v as i32)
}

/// Round up to the next multiple of 4 (Wayland wire alignment).
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

// ---------------------------------------------------------------------------
// Public API: parse
// ---------------------------------------------------------------------------

/// Parse a single Wayland wire-protocol message from the front of `data`.
///
/// Returns the parsed message and the number of bytes consumed so that the
/// caller can advance through a buffer containing multiple messages.
pub fn parse_message(data: &[u8]) -> Result<(WaylandMessage, usize), WaylandError> {
    if data.len() < HEADER_SIZE {
        return Err(WaylandError::MessageTooShort);
    }

    let object_id = read_u32(data, 0).ok_or(WaylandError::MessageTooShort)?;
    let size_opcode = read_u32(data, 4).ok_or(WaylandError::MessageTooShort)?;

    let header = MessageHeader {
        object_id,
        size_opcode,
    };
    let total_size = header.size() as usize;
    let opcode = header.opcode();

    if total_size < HEADER_SIZE || total_size > data.len() {
        return Err(WaylandError::SizeMismatch {
            declared: total_size,
            available: data.len(),
        });
    }

    // The argument payload starts right after the 8-byte header.
    let payload = &data[HEADER_SIZE..total_size];

    let msg = WaylandMessage {
        object_id,
        opcode,
        args: Vec::new(),
    };

    // We parse raw arguments; the caller (interface dispatch) interprets the
    // meaning of each positional argument. Here we just return the header +
    // the raw payload bytes as a flat Uint sequence (each 4-byte word).
    let mut args = Vec::new();
    let mut off = 0;
    while off + 4 <= payload.len() {
        let word = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)?;
        args.push(Argument::Uint(word));
        off += 4;
    }

    Ok((
        WaylandMessage {
            object_id: msg.object_id,
            opcode: msg.opcode,
            args,
        },
        total_size,
    ))
}

/// Parse a typed argument list from raw payload bytes according to a format
/// string where each character describes one argument:
///   i = Int, u = Uint, f = Fixed, s = String, o = Object, n = NewId,
///   a = Array, h = Fd
///
/// This is used by interface dispatchers that know the expected signature.
pub fn parse_args(payload: &[u8], signature: &[u8]) -> Result<Vec<Argument>, WaylandError> {
    let mut args = Vec::new();
    let mut off: usize = 0;

    for &ch in signature {
        match ch {
            b'i' => {
                let v = read_i32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::Int(v));
                off += 4;
            }
            b'u' => {
                let v = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::Uint(v));
                off += 4;
            }
            b'f' => {
                let v = read_i32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::Fixed(v));
                off += 4;
            }
            b's' => {
                let len = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)? as usize;
                off += 4;
                if off + len > payload.len() {
                    return Err(WaylandError::InvalidArgument);
                }
                // String includes trailing NUL in the length.
                let bytes = if len > 0 && payload[off + len - 1] == 0 {
                    payload[off..off + len - 1].to_vec()
                } else if len == 0 {
                    Vec::new()
                } else {
                    return Err(WaylandError::InvalidString);
                };
                args.push(Argument::String(bytes));
                off += align4(len);
            }
            b'o' => {
                let v = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::Object(v));
                off += 4;
            }
            b'n' => {
                let v = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::NewId(v));
                off += 4;
            }
            b'a' => {
                let len = read_u32(payload, off).ok_or(WaylandError::InvalidArgument)? as usize;
                off += 4;
                if off + len > payload.len() {
                    return Err(WaylandError::InvalidArgument);
                }
                let bytes = payload[off..off + len].to_vec();
                args.push(Argument::Array(bytes));
                off += align4(len);
            }
            b'h' => {
                let v = read_i32(payload, off).ok_or(WaylandError::InvalidArgument)?;
                args.push(Argument::Fd(v));
                off += 4;
            }
            _ => return Err(WaylandError::InvalidArgument),
        }
    }
    Ok(args)
}

// ---------------------------------------------------------------------------
// Public API: serialize
// ---------------------------------------------------------------------------

/// Serialize a `WaylandMessage` into its wire-protocol byte representation.
pub fn serialize_message(msg: &WaylandMessage) -> Vec<u8> {
    // First, serialize arguments to compute total size.
    let mut arg_bytes = Vec::new();
    for arg in &msg.args {
        serialize_arg(&mut arg_bytes, arg);
    }

    let total_size = (HEADER_SIZE + arg_bytes.len()) as u16;

    let mut out = Vec::with_capacity(total_size as usize);

    // Object ID
    out.extend_from_slice(&msg.object_id.to_ne_bytes());
    // size_opcode
    let size_opcode = MessageHeader::encode(msg.opcode, total_size);
    out.extend_from_slice(&size_opcode.to_ne_bytes());
    // Arguments
    out.extend_from_slice(&arg_bytes);

    out
}

/// Serialize a single argument, appending bytes to `buf`.
fn serialize_arg(buf: &mut Vec<u8>, arg: &Argument) {
    match arg {
        Argument::Int(v) => buf.extend_from_slice(&v.to_ne_bytes()),
        Argument::Uint(v) => buf.extend_from_slice(&v.to_ne_bytes()),
        Argument::Fixed(v) => buf.extend_from_slice(&v.to_ne_bytes()),
        Argument::String(bytes) => {
            // Wire format: u32 length (including NUL), data, NUL, padding to 4
            let len_with_nul = bytes.len() + 1;
            buf.extend_from_slice(&(len_with_nul as u32).to_ne_bytes());
            buf.extend_from_slice(bytes);
            buf.push(0); // NUL terminator
                         // Pad to 4-byte alignment
            let padded = align4(len_with_nul);
            for _ in len_with_nul..padded {
                buf.push(0);
            }
        }
        Argument::Object(v) => buf.extend_from_slice(&v.to_ne_bytes()),
        Argument::NewId(v) => buf.extend_from_slice(&v.to_ne_bytes()),
        Argument::Array(bytes) => {
            buf.extend_from_slice(&(bytes.len() as u32).to_ne_bytes());
            buf.extend_from_slice(bytes);
            let padded = align4(bytes.len());
            for _ in bytes.len()..padded {
                buf.push(0);
            }
        }
        Argument::Fd(v) => buf.extend_from_slice(&v.to_ne_bytes()),
    }
}

// ---------------------------------------------------------------------------
// Event builder helpers (server -> client)
// ---------------------------------------------------------------------------

/// Build a wl_display.error event.
#[allow(dead_code)] // Phase 6: sent to clients on protocol errors
pub fn build_display_error(object_id: u32, code: u32, message: &[u8]) -> Vec<u8> {
    let msg = WaylandMessage::new(
        WL_DISPLAY_ID,
        WL_DISPLAY_ERROR,
        vec![
            Argument::Object(object_id),
            Argument::Uint(code),
            Argument::String(message.to_vec()),
        ],
    );
    serialize_message(&msg)
}

/// Build a wl_display.delete_id event.
pub fn build_display_delete_id(id: u32) -> Vec<u8> {
    let msg = WaylandMessage::new(
        WL_DISPLAY_ID,
        WL_DISPLAY_DELETE_ID,
        vec![Argument::Uint(id)],
    );
    serialize_message(&msg)
}

/// Build a wl_registry.global event.
pub fn build_registry_global(
    registry_id: u32,
    name: u32,
    interface: &[u8],
    version: u32,
) -> Vec<u8> {
    let msg = WaylandMessage::new(
        registry_id,
        WL_REGISTRY_GLOBAL,
        vec![
            Argument::Uint(name),
            Argument::String(interface.to_vec()),
            Argument::Uint(version),
        ],
    );
    serialize_message(&msg)
}

/// Build a wl_shm.format event.
pub fn build_shm_format(shm_id: u32, format: u32) -> Vec<u8> {
    let msg = WaylandMessage::new(shm_id, WL_SHM_FORMAT, vec![Argument::Uint(format)]);
    serialize_message(&msg)
}

/// Build a wl_callback.done event (for sync and frame callbacks).
pub fn build_callback_done(callback_id: u32, serial: u32) -> Vec<u8> {
    let msg = WaylandMessage::new(callback_id, 0, vec![Argument::Uint(serial)]);
    serialize_message(&msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_encode_decode() {
        let opcode: u16 = 3;
        let size: u16 = 24;
        let combined = MessageHeader::encode(opcode, size);
        let header = MessageHeader {
            object_id: 7,
            size_opcode: combined,
        };
        assert_eq!(header.opcode(), opcode);
        assert_eq!(header.size(), size);
    }

    #[test]
    fn test_roundtrip_simple_message() {
        let msg = WaylandMessage::new(5, 2, vec![Argument::Uint(42), Argument::Int(-1)]);
        let bytes = serialize_message(&msg);
        let (parsed, consumed) = parse_message(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(parsed.object_id, 5);
        assert_eq!(parsed.opcode, 2);
        // Raw parse returns Uint words; first should be 42
        assert_eq!(parsed.args.len(), 2);
    }

    #[test]
    fn test_parse_too_short() {
        let data = [0u8; 4];
        assert_eq!(
            parse_message(&data).unwrap_err(),
            WaylandError::MessageTooShort
        );
    }

    #[test]
    fn test_serialize_string_alignment() {
        let msg = WaylandMessage::new(1, 0, vec![Argument::String(b"hi".to_vec())]);
        let bytes = serialize_message(&msg);
        // Header(8) + len(4) + "hi\0"(3) + pad(1) = 16
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn test_parse_args_string() {
        // Build a payload with a string "abc"
        let mut payload = Vec::new();
        // length including NUL = 4
        payload.extend_from_slice(&4u32.to_ne_bytes());
        payload.extend_from_slice(b"abc\0");
        let args = parse_args(&payload, b"s").unwrap();
        assert_eq!(args.len(), 1);
        if let Argument::String(s) = &args[0] {
            assert_eq!(s, b"abc");
        } else {
            panic!("expected String argument");
        }
    }
}
