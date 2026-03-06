//! Container Network Interface (CNI) implementation
//!
//! Provides plugin-based container networking with bridge/VXLAN overlays
//! and IPAM address management.

#![allow(dead_code)]

pub mod ipam;
pub mod overlay;
pub mod plugin;
