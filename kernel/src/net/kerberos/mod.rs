//! Kerberos v5 Protocol Implementation (RFC 4120)
//!
//! Provides Kerberos authentication with AS-REQ/AS-REP, TGS-REQ/TGS-REP
//! message construction and parsing, ticket caching, and key derivation
//! stubs for AES-CTS-HMAC-SHA1.

#![allow(dead_code)]

pub mod ccache;
pub mod protocol;
