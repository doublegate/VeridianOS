//! Formal Verification Module
//!
//! Provides model-checking infrastructure and proof harnesses for verifying
//! critical kernel invariants: boot chain integrity, IPC correctness,
//! memory allocator safety, and capability system soundness.

#[allow(dead_code)]
pub mod alloc_proofs;
#[allow(dead_code)]
pub mod boot_chain;
#[allow(dead_code)]
pub mod cap_proofs;
#[allow(dead_code)]
pub mod ipc_proofs;
