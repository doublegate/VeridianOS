//! Platform dispatch module.
//!
//! In a full Rust std port, this module would conditionally compile the
//! correct platform backend based on `cfg(target_os)`. For VeridianOS,
//! we always use the `veridian` backend.

pub mod veridian;
