//! Wayland Protocol Message Format
//!
//! Wire protocol for Wayland communication.

/// Wayland message header
#[repr(C)]
pub struct MessageHeader {
    /// Object ID
    pub object_id: u32,
    /// Opcode (method/event)
    pub opcode: u16,
    /// Message size in bytes
    pub size: u16,
}

/// Parse message from bytes
pub fn parse_message(_data: &[u8]) -> Result<Message, &'static str> {
    // TODO(phase6): Implement Wayland wire protocol message parsing
    Err("Not implemented")
}

/// Wayland message
pub struct Message {
    pub object_id: u32,
    pub opcode: u16,
    pub arguments: alloc::vec::Vec<Argument>,
}

/// Message argument
pub enum Argument {
    Int(i32),
    Uint(u32),
    Fixed(i32), // Fixed-point (1/256 precision)
    String(alloc::string::String),
    Object(u32),
    NewId(u32),
    Array(alloc::vec::Vec<u8>),
    Fd(i32),
}
