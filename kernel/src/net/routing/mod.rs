//! Routing daemon implementations for VeridianOS
//!
//! Provides dynamic routing protocol support:
//! - RIP v2 (RFC 2453) - Distance-vector routing
//! - OSPF (RFC 2328) - Link-state routing (single area)

pub mod ospf;
pub mod rip;
