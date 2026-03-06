//! Netfilter-style firewall for VeridianOS
//!
//! Provides packet filtering, connection tracking, and NAT capabilities
//! modeled on the Linux netfilter/iptables architecture with five hook
//! points (PreRouting, Input, Forward, Output, PostRouting).

#![allow(dead_code)]

pub mod chain;
pub mod conntrack;
pub mod nat;
pub mod rules;

use crate::error::KernelError;

/// Initialize the firewall subsystem
pub fn init() -> Result<(), KernelError> {
    chain::init()?;
    conntrack::init()?;
    nat::init()?;
    Ok(())
}
