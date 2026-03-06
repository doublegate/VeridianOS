//! Container Runtime Interface (CRI) implementation
//!
//! Provides gRPC-based container runtime services including pod sandbox
//! management, container lifecycle, image management, and exec/attach
//! streaming.

#![allow(dead_code)]

pub mod grpc;
pub mod image;
pub mod runtime;
pub mod streaming;
