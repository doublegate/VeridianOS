//! Formal Verification Module
//!
//! Provides model-checking infrastructure and proof harnesses for verifying
//! critical kernel invariants: boot chain integrity, IPC correctness,
//! memory allocator safety, and capability system soundness.

pub mod alloc_proofs;
pub mod boot_chain;
pub mod cap_proofs;
pub mod ipc_proofs;
