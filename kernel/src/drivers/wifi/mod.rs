//! WiFi 802.11 Driver Stack
//!
//! Implements IEEE 802.11 MAC layer and WPA2/WPA3 authentication
//! for wireless network connectivity.

#[cfg(feature = "alloc")]
pub mod mac80211;
#[cfg(feature = "alloc")]
pub mod wpa;
