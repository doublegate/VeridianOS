//! Service Mesh implementation
//!
//! Provides sidecar proxy, service discovery, and SPIFFE-based identity
//! management for microservice communication.

#![allow(dead_code)]

pub mod discovery;
pub mod identity;
pub mod proxy;
