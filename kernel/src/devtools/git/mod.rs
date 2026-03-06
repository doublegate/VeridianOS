//! Native Git Client
//!
//! Implements Git object model, porcelain commands, and network transport
//! for a fully native Git client on VeridianOS.

pub mod commands;
pub mod deflate;
pub mod objects;
pub mod refs;
pub mod transport;
