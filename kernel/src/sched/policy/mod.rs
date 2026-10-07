//! Scheduling policy (ADR 0007): deadline, real-time and EEVDF fair classes,
//! per-CPU run queues, and SMP placement/balancing decisions.
//!
//! Pure code: no locks, no architecture dependency, no knowledge of how a
//! task is switched to (that is the dispatcher, ADR 0006). Tasks are named by
//! an opaque [`TaskKey`] chosen by the dispatcher. All arithmetic is integer.

pub mod balance;
pub mod dl;
pub mod fair;
pub mod rq;
pub mod rt;

pub use rq::{Class, Entity, Policy, RunQueue};

/// Opaque task identifier (the dispatcher uses the task's address).
pub type TaskKey = u64;
