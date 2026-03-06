//! Software RAID Implementation
//!
//! Provides software RAID levels 0, 1, and 5 with stripe mapping,
//! parity computation, health monitoring, and hot-spare replacement.

#![allow(dead_code)]

pub mod manager;
