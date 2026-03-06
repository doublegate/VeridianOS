//! Load Balancer implementation
//!
//! Provides L4 (TCP/UDP) and L7 (HTTP) load balancing with multiple
//! algorithms, health checks, rate limiting, and sticky sessions.

#![allow(dead_code)]

pub mod config;
pub mod l4;
pub mod l7;
