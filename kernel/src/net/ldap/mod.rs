//! LDAP Client Implementation (RFC 4511)
//!
//! Provides an LDAPv3 client for directory service operations including
//! bind, search, compare, modify, add, and delete. Includes Active Directory
//! integration helpers and a TTL-based credential cache.

#![allow(dead_code)]

pub mod client;
