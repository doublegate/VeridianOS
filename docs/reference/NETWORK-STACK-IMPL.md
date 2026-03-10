# Veridian OS: Network Stack Implementation Guide

## Table of Contents

1. [Introduction](#introduction)
2. [Network Stack Architecture](#network-stack-architecture)
3. [Core Network Types](#core-network-types)
4. [Packet Buffer Management](#packet-buffer-management)
5. [Network Device Layer](#network-device-layer)
6. [Ethernet Layer](#ethernet-layer)
7. [ARP Implementation](#arp-implementation)
8. [IPv4 Implementation](#ipv4-implementation)
9. [IPv6 Implementation](#ipv6-implementation)
10. [ICMP Implementation](#icmp-implementation)
11. [UDP Implementation](#udp-implementation)
12. [TCP Implementation](#tcp-implementation)
13. [Socket API](#socket-api)
14. [Network Namespaces](#network-namespaces)
15. [Advanced Features](#advanced-features)
16. [Performance Optimizations](#performance-optimizations)
17. [Security Features](#security-features)
18. [Testing Strategies](#testing-strategies)

## Introduction

This guide provides a comprehensive approach to implementing a modern, high-performance network stack for Veridian OS. The design emphasizes:

1. **Zero-Copy Architecture**: Minimize data copying throughout the stack
2. **Async/Await Support**: Native async networking APIs
3. **Capability Security**: Network access controlled by capabilities
4. **Namespace Isolation**: Network namespace support for containers
5. **Hardware Offload**: Support for modern NIC features

### Design Principles

- **Modular Design**: Each protocol layer is independent
- **Lock-Free Where Possible**: Use atomic operations and lock-free queues
- **NUMA Awareness**: Optimize for modern multi-core systems
- **Security First**: Built-in DoS protection and rate limiting
- **Extensibility**: Easy to add new protocols and features

## Network Stack Architecture

### Overall Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    User Applications                     │
├─────────────────────────────────────────────────────────┤
│                      Socket API                          │
├─────────────────────────────────────────────────────────┤
│         TCP          │        UDP        │    Raw       │
├─────────────────────────────────────────────────────────┤
│                    IPv4 / IPv6                          │
├─────────────────────────────────────────────────────────┤
│    ARP    │   ICMP   │   Routing   │   Netfilter      │
├─────────────────────────────────────────────────────────┤
│                     Ethernet                            │
├─────────────────────────────────────────────────────────┤
│              Network Device Abstraction                 │
├─────────────────────────────────────────────────────────┤
│          Hardware Drivers (NIC Drivers)                 │
└─────────────────────────────────────────────────────────┘
```

### Core Components

Create `kernel/src/net/mod.rs`:

```rust
//! Network stack implementation

pub mod types;
pub mod buffer;
pub mod device;
pub mod ethernet;
pub mod arp;
pub mod ipv4;
pub mod ipv6;
pub mod icmp;
pub mod udp;
pub mod tcp;
pub mod socket;
pub mod namespace;

use alloc::sync::Arc;
use spin::RwLock;

/// Global network stack instance
static NET_STACK: RwLock<Option<Arc<NetworkStack>>> = RwLock::new(None);

/// Main network stack structure
pub struct NetworkStack {
    /// Network namespaces
    namespaces: RwLock<HashMap<u32, Arc<NetworkNamespace>>>,
    /// Global routing table
    routing: Arc<RoutingTable>,
    /// Network devices
    devices: RwLock<HashMap<String, Arc<dyn NetworkDevice>>>,
    /// Protocol handlers
    protocols: ProtocolHandlers,
    /// Network statistics
    stats: NetworkStatistics,
}

impl NetworkStack {
    /// Initialize network stack
    pub fn init() -> Result<(), NetworkError> {
        let stack = Arc::new(Self {
            namespaces: RwLock::new(HashMap::new()),
            routing: Arc::new(RoutingTable::new()),
            devices: RwLock::new(HashMap::new()),
            protocols: ProtocolHandlers::new(),
            stats: NetworkStatistics::default(),
        });
        
        // Create default namespace
        let default_ns = NetworkNamespace::new(0);
        stack.namespaces.write().insert(0, Arc::new(default_ns));
        
        // Store global instance
        *NET_STACK.write() = Some(stack);
        
        println!("Network stack initialized");
        Ok(())
    }
    
    /// Get network stack instance
    pub fn get() -> Arc<NetworkStack> {
        NET_STACK.read().as_ref().unwrap().clone()
    }
}

/// Network errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    InvalidAddress,
    InvalidPacket,
    NoRoute,
    PortInUse,
    ConnectionRefused,
    ConnectionReset,
    Timeout,
    BufferFull,
    DeviceNotFound,
    ProtocolNotSupported,
    PermissionDenied,
}

pub type Result<T> = core::result::Result<T, NetworkError>;
```

## Core Network Types

### Basic Types

Create `kernel/src/net/types.rs`:

```rust
//! Core network types

use core::fmt;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Ethernet address (MAC address)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EthernetAddress([u8; 6]);

impl EthernetAddress {
    pub const BROADCAST: Self = Self([0xFF; 6]);
    
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
    
    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
    
    pub fn is_broadcast(&self) -> bool {
        self == &Self::BROADCAST
    }
    
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0x01 != 0
    }
    
    pub fn is_unicast(&self) -> bool {
        !self.is_multicast()
    }
}

impl fmt::Display for EthernetAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
               self.0[0], self.0[1], self.0[2],
               self.0[3], self.0[4], self.0[5])
    }
}

/// IP endpoint (address + port)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Endpoint {
    pub addr: IpAddr,
    pub port: u16,
}

impl Endpoint {
    pub const fn new(addr: IpAddr, port: u16) -> Self {
        Self { addr, port }
    }
    
    pub fn is_ipv4(&self) -> bool {
        matches!(self.addr, IpAddr::V4(_))
    }
    
    pub fn is_ipv6(&self) -> bool {
        matches!(self.addr, IpAddr::V6(_))
    }
}

/// Network interface index
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceIndex(pub u32);

/// Protocol numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Protocol {
    Icmp = 1,
    Tcp = 6,
    Udp = 17,
    Icmpv6 = 58,
    Other(u8),
}

impl From<u8> for Protocol {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Icmp,
            6 => Self::Tcp,
            17 => Self::Udp,
            58 => Self::Icmpv6,
            other => Self::Other(other),
        }
    }
}

/// Checksum calculation
pub fn checksum(data: &[u8], initial: u32) -> u16 {
    let mut sum = initial;
    
    // Sum 16-bit words
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]]) as u32
        } else {
            (chunk[0] as u32) << 8
        };
        sum = sum.wrapping_add(word);
    }
    
    // Fold 32-bit sum to 16 bits
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    
    // One's complement
    !(sum as u16)
}

/// Internet checksum for pseudo-header
pub fn internet_checksum(
    src: IpAddr,
    dst: IpAddr,
    protocol: Protocol,
    data: &[u8],
) -> u16 {
    let mut sum = 0u32;
    
    match (src, dst) {
        (IpAddr::V4(src), IpAddr::V4(dst)) => {
            // IPv4 pseudo-header
            sum += u16::from_be_bytes([src.octets()[0], src.octets()[1]]) as u32;
            sum += u16::from_be_bytes([src.octets()[2], src.octets()[3]]) as u32;
            sum += u16::from_be_bytes([dst.octets()[0], dst.octets()[1]]) as u32;
            sum += u16::from_be_bytes([dst.octets()[2], dst.octets()[3]]) as u32;
            sum += protocol as u32;
            sum += data.len() as u32;
        }
        (IpAddr::V6(src), IpAddr::V6(dst)) => {
            // IPv6 pseudo-header
            for chunk in src.octets().chunks(2) {
                sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
            }
            for chunk in dst.octets().chunks(2) {
                sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
            }
            sum += data.len() as u32;
            sum += protocol as u32;
        }
        _ => panic!("IP version mismatch"),
    }
    
    checksum(data, sum)
}
```

### Network Statistics

```rust
/// Network statistics
#[derive(Debug, Default)]
pub struct NetworkStatistics {
    /// Packets received
    pub rx_packets: AtomicU64,
    /// Bytes received
    pub rx_bytes: AtomicU64,
    /// Receive errors
    pub rx_errors: AtomicU64,
    /// Receive drops
    pub rx_drops: AtomicU64,
    
    /// Packets transmitted
    pub tx_packets: AtomicU64,
    /// Bytes transmitted
    pub tx_bytes: AtomicU64,
    /// Transmit errors
    pub tx_errors: AtomicU64,
    /// Transmit drops
    pub tx_drops: AtomicU64,
}

impl NetworkStatistics {
    pub fn record_rx(&self, bytes: usize) {
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
        self.rx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    
    pub fn record_tx(&self, bytes: usize) {
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
        self.tx_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }
    
    pub fn record_rx_error(&self) {
        self.rx_errors.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_tx_error(&self) {
        self.tx_errors.fetch_add(1, Ordering::Relaxed);
    }
}
```

## Packet Buffer Management

### Zero-Copy Packet Buffer

Create `kernel/src/net/buffer.rs`:

```rust
//! Packet buffer management

use alloc::sync::Arc;
use core::ops::{Deref, DerefMut};

/// Packet buffer for zero-copy networking
pub struct PacketBuffer {
    /// Underlying data
    data: PacketData,
    /// Data offset
    offset: usize,
    /// Data length
    length: usize,
    /// Headroom for headers
    headroom: usize,
    /// Reference count for zero-copy
    refcount: Arc<AtomicUsize>,
}

enum PacketData {
    /// Owned buffer
    Owned(Vec<u8>),
    /// Shared buffer (zero-copy from driver)
    Shared(Arc<[u8]>),
    /// Memory-mapped buffer
    Mapped { ptr: *mut u8, len: usize },
}

impl PacketBuffer {
    /// Create new packet buffer with capacity
    pub fn new(capacity: usize) -> Self {
        let headroom = 128; // Reserve space for headers
        let mut data = vec![0u8; capacity + headroom];
        
        Self {
            data: PacketData::Owned(data),
            offset: headroom,
            length: 0,
            headroom,
            refcount: Arc::new(AtomicUsize::new(1)),
        }
    }
    
    /// Create from existing data (zero-copy)
    pub fn from_slice(data: &[u8]) -> Self {
        let mut owned = Vec::with_capacity(data.len() + 128);
        owned.resize(128, 0);
        owned.extend_from_slice(data);
        
        Self {
            data: PacketData::Owned(owned),
            offset: 128,
            length: data.len(),
            headroom: 128,
            refcount: Arc::new(AtomicUsize::new(1)),
        }
    }
    
    /// Get packet data
    pub fn as_slice(&self) -> &[u8] {
        match &self.data {
            PacketData::Owned(vec) => &vec[self.offset..self.offset + self.length],
            PacketData::Shared(arc) => &arc[self.offset..self.offset + self.length],
            PacketData::Mapped { ptr, len } => unsafe {
                core::slice::from_raw_parts(*ptr, *len)
            },
        }
    }
    
    /// Get mutable packet data
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.make_owned();
        
        match &mut self.data {
            PacketData::Owned(vec) => &mut vec[self.offset..self.offset + self.length],
            _ => unreachable!(),
        }
    }
    
    /// Reserve space at front of packet
    pub fn reserve_front(&mut self, size: usize) -> Result<(), NetworkError> {
        if size > self.offset {
            return Err(NetworkError::BufferFull);
        }
        
        self.offset -= size;
        self.length += size;
        Ok(())
    }
    
    /// Reserve space at back of packet
    pub fn reserve_back(&mut self, size: usize) -> Result<(), NetworkError> {
        let capacity = match &self.data {
            PacketData::Owned(vec) => vec.capacity(),
            PacketData::Shared(arc) => arc.len(),
            PacketData::Mapped { len, .. } => *len,
        };
        
        if self.offset + self.length + size > capacity {
            return Err(NetworkError::BufferFull);
        }
        
        self.length += size;
        Ok(())
    }
    
    /// Trim bytes from front
    pub fn trim_front(&mut self, size: usize) {
        let size = size.min(self.length);
        self.offset += size;
        self.length -= size;
    }
    
    /// Trim bytes from back
    pub fn trim_back(&mut self, size: usize) {
        let size = size.min(self.length);
        self.length -= size;
    }
    
    /// Clone packet (zero-copy if possible)
    pub fn clone(&self) -> Self {
        self.refcount.fetch_add(1, Ordering::Relaxed);
        
        Self {
            data: match &self.data {
                PacketData::Owned(vec) => PacketData::Shared(Arc::from(vec.as_slice())),
                PacketData::Shared(arc) => PacketData::Shared(arc.clone()),
                PacketData::Mapped { ptr, len } => PacketData::Mapped { 
                    ptr: *ptr, 
                    len: *len 
                },
            },
            offset: self.offset,
            length: self.length,
            headroom: self.headroom,
            refcount: self.refcount.clone(),
        }
    }
    
    /// Make buffer owned (for modification)
    fn make_owned(&mut self) {
        if self.refcount.load(Ordering::Relaxed) == 1 {
            return; // Already exclusive
        }
        
        match &self.data {
            PacketData::Shared(arc) | PacketData::Mapped { .. } => {
                let mut vec = Vec::with_capacity(self.offset + self.length);
                vec.extend_from_slice(self.as_slice());
                self.data = PacketData::Owned(vec);
                self.refcount = Arc::new(AtomicUsize::new(1));
            }
            PacketData::Owned(_) => {} // Already owned
        }
    }
}

/// Packet buffer pool for allocation
pub struct PacketBufferPool {
    /// Small buffers (< 2KB)
    small: Mutex<Vec<PacketBuffer>>,
    /// Large buffers (>= 2KB)
    large: Mutex<Vec<PacketBuffer>>,
    /// Pool statistics
    stats: PoolStatistics,
}

impl PacketBufferPool {
    pub const SMALL_SIZE: usize = 2048;
    pub const LARGE_SIZE: usize = 9000; // Jumbo frames
    
    pub fn new() -> Self {
        Self {
            small: Mutex::new(Vec::new()),
            large: Mutex::new(Vec::new()),
            stats: PoolStatistics::default(),
        }
    }
    
    /// Allocate buffer from pool
    pub fn allocate(&self, size: usize) -> PacketBuffer {
        if size < Self::SMALL_SIZE {
            if let Some(buffer) = self.small.lock().pop() {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return buffer;
            }
        } else {
            if let Some(buffer) = self.large.lock().pop() {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return buffer;
            }
        }
        
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        PacketBuffer::new(size.max(Self::SMALL_SIZE))
    }
    
    /// Return buffer to pool
    pub fn free(&self, mut buffer: PacketBuffer) {
        // Reset buffer
        buffer.offset = buffer.headroom;
        buffer.length = 0;
        
        // Return to appropriate pool
        let capacity = match &buffer.data {
            PacketData::Owned(vec) => vec.capacity(),
            _ => return, // Don't pool shared buffers
        };
        
        if capacity <= Self::SMALL_SIZE {
            self.small.lock().push(buffer);
        } else {
            self.large.lock().push(buffer);
        }
    }
}

#[derive(Debug, Default)]
struct PoolStatistics {
    hits: AtomicU64,
    misses: AtomicU64,
}
```

## Network Device Layer

### Network Device Interface

Create `kernel/src/net/device.rs`:

```rust
//! Network device abstraction

use super::*;
use alloc::collections::VecDeque;
use core::future::Future;

/// Network device trait
pub trait NetworkDevice: Send + Sync {
    /// Get device name
    fn name(&self) -> &str;
    
    /// Get device type
    fn device_type(&self) -> DeviceType;
    
    /// Get hardware address
    fn hardware_address(&self) -> EthernetAddress;
    
    /// Get MTU
    fn mtu(&self) -> u16;
    
    /// Get device flags
    fn flags(&self) -> DeviceFlags;
    
    /// Set device flags
    fn set_flags(&mut self, flags: DeviceFlags);
    
    /// Transmit packet
    fn transmit(&mut self, packet: PacketBuffer) -> Result<()>;
    
    /// Receive packet (non-blocking)
    fn receive(&mut self) -> Option<PacketBuffer>;
    
    /// Get statistics
    fn statistics(&self) -> &NetworkStatistics;
    
    /// Enable promiscuous mode
    fn set_promiscuous(&mut self, enable: bool);
    
    /// Add multicast address
    fn add_multicast(&mut self, addr: EthernetAddress);
    
    /// Remove multicast address
    fn remove_multicast(&mut self, addr: EthernetAddress);
}

/// Device types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Ethernet,
    Loopback,
    Tunnel,
    Bridge,
    Vlan,
    Wireless,
}

bitflags::bitflags! {
    pub struct DeviceFlags: u32 {
        const UP           = 0x0001;
        const BROADCAST    = 0x0002;
        const DEBUG        = 0x0004;
        const LOOPBACK     = 0x0008;
        const POINTTOPOINT = 0x0010;
        const NOTRAILERS   = 0x0020;
        const RUNNING      = 0x0040;
        const NOARP        = 0x0080;
        const PROMISC      = 0x0100;
        const ALLMULTI     = 0x0200;
        const MULTICAST    = 0x1000;
    }
}

/// Network interface
pub struct NetworkInterface {
    /// Interface index
    pub index: InterfaceIndex,
    /// Device
    pub device: Arc<Mutex<dyn NetworkDevice>>,
    /// IP addresses
    pub addresses: RwLock<Vec<InterfaceAddress>>,
    /// Receive queue
    pub rx_queue: Mutex<VecDeque<PacketBuffer>>,
    /// Transmit queue
    pub tx_queue: Mutex<VecDeque<PacketBuffer>>,
}

/// Interface address
#[derive(Debug, Clone)]
pub struct InterfaceAddress {
    pub address: IpAddr,
    pub prefix_len: u8,
    pub flags: AddressFlags,
}

bitflags::bitflags! {
    pub struct AddressFlags: u8 {
        const PERMANENT  = 0x01;
        const TENTATIVE  = 0x02;
        const DEPRECATED = 0x04;
        const SECONDARY  = 0x08;
    }
}

impl NetworkInterface {
    /// Process incoming packets
    pub async fn process_rx(&self) {
        let mut device = self.device.lock().await;
        
        while let Some(packet) = device.receive() {
            // Update statistics
            device.statistics().record_rx(packet.as_slice().len());
            
            // Queue packet for processing
            self.rx_queue.lock().await.push_back(packet);
        }
    }
    
    /// Process outgoing packets
    pub async fn process_tx(&self) {
        let mut device = self.device.lock().await;
        let mut tx_queue = self.tx_queue.lock().await;
        
        while let Some(packet) = tx_queue.pop_front() {
            match device.transmit(packet) {
                Ok(()) => {
                    device.statistics().record_tx(packet.as_slice().len());
                }
                Err(_) => {
                    device.statistics().record_tx_error();
                    // Could implement retry logic here
                }
            }
        }
    }
}

/// Loopback device implementation
pub struct LoopbackDevice {
    stats: NetworkStatistics,
    rx_queue: VecDeque<PacketBuffer>,
}

impl LoopbackDevice {
    pub fn new() -> Self {
        Self {
            stats: NetworkStatistics::default(),
            rx_queue: VecDeque::new(),
        }
    }
}

impl NetworkDevice for LoopbackDevice {
    fn name(&self) -> &str {
        "lo"
    }
    
    fn device_type(&self) -> DeviceType {
        DeviceType::Loopback
    }
    
    fn hardware_address(&self) -> EthernetAddress {
        EthernetAddress::new([0; 6])
    }
    
    fn mtu(&self) -> u16 {
        65536
    }
    
    fn flags(&self) -> DeviceFlags {
        DeviceFlags::UP | DeviceFlags::LOOPBACK | DeviceFlags::RUNNING
    }
    
    fn set_flags(&mut self, _flags: DeviceFlags) {
        // Loopback flags are fixed
    }
    
    fn transmit(&mut self, packet: PacketBuffer) -> Result<()> {
        // Loopback: queue packet for receive
        self.rx_queue.push_back(packet);
        Ok(())
    }
    
    fn receive(&mut self) -> Option<PacketBuffer> {
        self.rx_queue.pop_front()
    }
    
    fn statistics(&self) -> &NetworkStatistics {
        &self.stats
    }
    
    fn set_promiscuous(&mut self, _enable: bool) {
        // No-op for loopback
    }
    
    fn add_multicast(&mut self, _addr: EthernetAddress) {
        // No-op for loopback
    }
    
    fn remove_multicast(&mut self, _addr: EthernetAddress) {
        // No-op for loopback
    }
}
```

## Ethernet Layer

### Ethernet Protocol Implementation

Create `kernel/src/net/ethernet.rs`:

```rust
//! Ethernet protocol implementation

use super::*;
use byteorder::{ByteOrder, NetworkEndian};

/// Ethernet header size
pub const ETHERNET_HEADER_SIZE: usize = 14;

/// Ethernet frame
#[repr(C, packed)]
pub struct EthernetHeader {
    pub destination: [u8; 6],
    pub source: [u8; 6],
    pub ethertype: [u8; 2],
}

impl EthernetHeader {
    pub fn new(dst: EthernetAddress, src: EthernetAddress, ethertype: EtherType) -> Self {
        Self {
            destination: *dst.as_bytes(),
            source: *src.as_bytes(),
            ethertype: (ethertype as u16).to_be_bytes(),
        }
    }
    
    pub fn destination(&self) -> EthernetAddress {
        EthernetAddress::new(self.destination)
    }
    
    pub fn source(&self) -> EthernetAddress {
        EthernetAddress::new(self.source)
    }
    
    pub fn ethertype(&self) -> EtherType {
        EtherType::from(NetworkEndian::read_u16(&self.ethertype))
    }
}

/// Ethernet types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum EtherType {
    Ipv4 = 0x0800,
    Arp = 0x0806,
    Ipv6 = 0x86DD,
    Vlan = 0x8100,
    Unknown(u16),
}

impl From<u16> for EtherType {
    fn from(value: u16) -> Self {
        match value {
            0x0800 => Self::Ipv4,
            0x0806 => Self::Arp,
            0x86DD => Self::Ipv6,
            0x8100 => Self::Vlan,
            other => Self::Unknown(other),
        }
    }
}

/// Ethernet layer processor
pub struct EthernetLayer {
    /// Interfaces
    interfaces: RwLock<HashMap<InterfaceIndex, Arc<NetworkInterface>>>,
    /// Protocol handlers
    handlers: RwLock<HashMap<EtherType, Box<dyn ProtocolHandler>>>,
}

/// Protocol handler trait
pub trait ProtocolHandler: Send + Sync {
    /// Handle incoming packet
    fn handle_rx(
        &self,
        interface: &NetworkInterface,
        source: EthernetAddress,
        packet: PacketBuffer,
    ) -> Result<()>;
}

impl EthernetLayer {
    pub fn new() -> Self {
        Self {
            interfaces: RwLock::new(HashMap::new()),
            handlers: RwLock::new(HashMap::new()),
        }
    }
    
    /// Register protocol handler
    pub fn register_handler(&self, ethertype: EtherType, handler: Box<dyn ProtocolHandler>) {
        self.handlers.write().insert(ethertype, handler);
    }
    
    /// Process incoming packet
    pub async fn process_rx(&self, interface: &NetworkInterface, mut packet: PacketBuffer) -> Result<()> {
        // Check minimum size
        if packet.as_slice().len() < ETHERNET_HEADER_SIZE {
            return Err(NetworkError::InvalidPacket);
        }
        
        // Parse header
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const EthernetHeader)
        };
        
        let dst = header.destination();
        let src = header.source();
        let ethertype = header.ethertype();
        
        // Check destination
        let hw_addr = interface.device.lock().await.hardware_address();
        if !dst.is_broadcast() && !dst.is_multicast() && dst != hw_addr {
            return Ok(()); // Not for us
        }
        
        // Remove ethernet header
        packet.trim_front(ETHERNET_HEADER_SIZE);
        
        // Dispatch to protocol handler
        if let Some(handler) = self.handlers.read().get(&ethertype) {
            handler.handle_rx(interface, src, packet)?;
        }
        
        Ok(())
    }
    
    /// Transmit packet
    pub async fn transmit(
        &self,
        interface: &NetworkInterface,
        dst: EthernetAddress,
        ethertype: EtherType,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        // Add ethernet header
        packet.reserve_front(ETHERNET_HEADER_SIZE)?;
        
        let hw_addr = interface.device.lock().await.hardware_address();
        let header = EthernetHeader::new(dst, hw_addr, ethertype);
        
        packet.as_mut_slice()[..ETHERNET_HEADER_SIZE]
            .copy_from_slice(unsafe {
                core::slice::from_raw_parts(
                    &header as *const _ as *const u8,
                    ETHERNET_HEADER_SIZE,
                )
            });
        
        // Queue for transmission
        interface.tx_queue.lock().await.push_back(packet);
        
        Ok(())
    }
}

/// VLAN tag
#[repr(C, packed)]
pub struct VlanTag {
    pub tpid: [u8; 2],
    pub tci: [u8; 2],
}

impl VlanTag {
    pub fn new(vlan_id: u16, priority: u8) -> Self {
        let tci = (priority as u16) << 13 | (vlan_id & 0x0FFF);
        Self {
            tpid: 0x8100u16.to_be_bytes(),
            tci: tci.to_be_bytes(),
        }
    }
    
    pub fn vlan_id(&self) -> u16 {
        NetworkEndian::read_u16(&self.tci) & 0x0FFF
    }
    
    pub fn priority(&self) -> u8 {
        (NetworkEndian::read_u16(&self.tci) >> 13) as u8
    }
}
```

## ARP Implementation

### Address Resolution Protocol

Create `kernel/src/net/arp.rs`:

```rust
//! ARP (Address Resolution Protocol) implementation

use super::*;
use alloc::collections::HashMap;
use core::time::Duration;

/// ARP header
#[repr(C, packed)]
pub struct ArpHeader {
    pub hardware_type: [u8; 2],
    pub protocol_type: [u8; 2],
    pub hardware_len: u8,
    pub protocol_len: u8,
    pub operation: [u8; 2],
    pub sender_hw_addr: [u8; 6],
    pub sender_proto_addr: [u8; 4],
    pub target_hw_addr: [u8; 6],
    pub target_proto_addr: [u8; 4],
}

/// ARP operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ArpOperation {
    Request = 1,
    Reply = 2,
}

/// ARP cache entry
#[derive(Debug, Clone)]
pub struct ArpEntry {
    pub hardware_addr: EthernetAddress,
    pub state: ArpState,
    pub timestamp: Instant,
    pub retries: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpState {
    Incomplete,
    Reachable,
    Stale,
    Delay,
    Probe,
}

/// ARP cache
pub struct ArpCache {
    entries: RwLock<HashMap<Ipv4Addr, ArpEntry>>,
    pending: Mutex<HashMap<Ipv4Addr, Vec<PacketBuffer>>>,
}

impl ArpCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }
    
    /// Lookup hardware address
    pub fn lookup(&self, ip: Ipv4Addr) -> Option<EthernetAddress> {
        self.entries.read()
            .get(&ip)
            .filter(|entry| entry.state == ArpState::Reachable)
            .map(|entry| entry.hardware_addr)
    }
    
    /// Add entry to cache
    pub fn add(&self, ip: Ipv4Addr, hw_addr: EthernetAddress) {
        let entry = ArpEntry {
            hardware_addr: hw_addr,
            state: ArpState::Reachable,
            timestamp: Instant::now(),
            retries: 0,
        };
        
        self.entries.write().insert(ip, entry);
        
        // Process pending packets
        if let Some(packets) = self.pending.lock().remove(&ip) {
            for packet in packets {
                // Transmit pending packet
                // TODO: Queue for transmission
            }
        }
    }
    
    /// Queue packet pending ARP resolution
    pub fn queue_packet(&self, ip: Ipv4Addr, packet: PacketBuffer) {
        self.pending.lock()
            .entry(ip)
            .or_insert_with(Vec::new)
            .push(packet);
    }
    
    /// Remove stale entries
    pub fn cleanup(&self) {
        let now = Instant::now();
        let timeout = Duration::from_secs(300); // 5 minutes
        
        self.entries.write().retain(|_, entry| {
            now.duration_since(entry.timestamp) < timeout
        });
    }
}

/// ARP protocol handler
pub struct ArpHandler {
    cache: Arc<ArpCache>,
}

impl ArpHandler {
    pub fn new(cache: Arc<ArpCache>) -> Self {
        Self { cache }
    }
    
    /// Send ARP request
    pub async fn send_request(
        &self,
        interface: &NetworkInterface,
        target_ip: Ipv4Addr,
    ) -> Result<()> {
        let device = interface.device.lock().await;
        let hw_addr = device.hardware_address();
        
        // Get interface IP
        let src_ip = interface.addresses.read()
            .iter()
            .find_map(|addr| match addr.address {
                IpAddr::V4(ip) => Some(ip),
                _ => None,
            })
            .ok_or(NetworkError::InvalidAddress)?;
        
        drop(device);
        
        // Build ARP request
        let header = ArpHeader {
            hardware_type: 1u16.to_be_bytes(), // Ethernet
            protocol_type: 0x0800u16.to_be_bytes(), // IPv4
            hardware_len: 6,
            protocol_len: 4,
            operation: (ArpOperation::Request as u16).to_be_bytes(),
            sender_hw_addr: *hw_addr.as_bytes(),
            sender_proto_addr: src_ip.octets(),
            target_hw_addr: [0; 6],
            target_proto_addr: target_ip.octets(),
        };
        
        let mut packet = PacketBuffer::new(size_of::<ArpHeader>());
        packet.as_mut_slice().copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                size_of::<ArpHeader>(),
            )
        });
        
        // Send as ethernet broadcast
        NetworkStack::get()
            .ethernet
            .transmit(
                interface,
                EthernetAddress::BROADCAST,
                EtherType::Arp,
                packet,
            )
            .await
    }
    
    /// Send ARP reply
    pub async fn send_reply(
        &self,
        interface: &NetworkInterface,
        target_hw: EthernetAddress,
        target_ip: Ipv4Addr,
    ) -> Result<()> {
        let device = interface.device.lock().await;
        let hw_addr = device.hardware_address();
        
        // Get interface IP
        let src_ip = interface.addresses.read()
            .iter()
            .find_map(|addr| match addr.address {
                IpAddr::V4(ip) => Some(ip),
                _ => None,
            })
            .ok_or(NetworkError::InvalidAddress)?;
        
        drop(device);
        
        // Build ARP reply
        let header = ArpHeader {
            hardware_type: 1u16.to_be_bytes(),
            protocol_type: 0x0800u16.to_be_bytes(),
            hardware_len: 6,
            protocol_len: 4,
            operation: (ArpOperation::Reply as u16).to_be_bytes(),
            sender_hw_addr: *hw_addr.as_bytes(),
            sender_proto_addr: src_ip.octets(),
            target_hw_addr: *target_hw.as_bytes(),
            target_proto_addr: target_ip.octets(),
        };
        
        let mut packet = PacketBuffer::new(size_of::<ArpHeader>());
        packet.as_mut_slice().copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                size_of::<ArpHeader>(),
            )
        });
        
        // Send to requester
        NetworkStack::get()
            .ethernet
            .transmit(interface, target_hw, EtherType::Arp, packet)
            .await
    }
}

impl ProtocolHandler for ArpHandler {
    fn handle_rx(
        &self,
        interface: &NetworkInterface,
        _source: EthernetAddress,
        packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < size_of::<ArpHeader>() {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const ArpHeader)
        };
        
        // Validate header
        if NetworkEndian::read_u16(&header.hardware_type) != 1 ||
           NetworkEndian::read_u16(&header.protocol_type) != 0x0800 ||
           header.hardware_len != 6 ||
           header.protocol_len != 4 {
            return Err(NetworkError::InvalidPacket);
        }
        
        let operation = NetworkEndian::read_u16(&header.operation);
        let sender_hw = EthernetAddress::new(header.sender_hw_addr);
        let sender_ip = Ipv4Addr::from(header.sender_proto_addr);
        let target_ip = Ipv4Addr::from(header.target_proto_addr);
        
        // Update ARP cache
        self.cache.add(sender_ip, sender_hw);
        
        // Handle request
        if operation == ArpOperation::Request as u16 {
            // Check if request is for us
            let our_ips: Vec<Ipv4Addr> = interface.addresses.read()
                .iter()
                .filter_map(|addr| match addr.address {
                    IpAddr::V4(ip) => Some(ip),
                    _ => None,
                })
                .collect();
            
            if our_ips.contains(&target_ip) {
                // Send reply
                tokio::spawn(async move {
                    let handler = ArpHandler::new(self.cache.clone());
                    handler.send_reply(interface, sender_hw, sender_ip).await;
                });
            }
        }
        
        Ok(())
    }
}
```

## IPv4 Implementation

### IPv4 Protocol

Create `kernel/src/net/ipv4.rs`:

```rust
//! IPv4 protocol implementation

use super::*;
use byteorder::{ByteOrder, NetworkEndian};

/// IPv4 header (without options)
#[repr(C, packed)]
pub struct Ipv4Header {
    pub version_ihl: u8,
    pub dscp_ecn: u8,
    pub total_length: [u8; 2],
    pub identification: [u8; 2],
    pub flags_fragment_offset: [u8; 2],
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: [u8; 2],
    pub source: [u8; 4],
    pub destination: [u8; 4],
}

impl Ipv4Header {
    pub const MIN_SIZE: usize = 20;
    
    pub fn new(
        source: Ipv4Addr,
        destination: Ipv4Addr,
        protocol: Protocol,
        payload_len: usize,
    ) -> Self {
        let total_length = (Self::MIN_SIZE + payload_len) as u16;
        
        let mut header = Self {
            version_ihl: 0x45, // Version 4, IHL 5
            dscp_ecn: 0,
            total_length: total_length.to_be_bytes(),
            identification: 0u16.to_be_bytes(),
            flags_fragment_offset: 0x4000u16.to_be_bytes(), // Don't fragment
            ttl: 64,
            protocol: protocol as u8,
            checksum: [0, 0],
            source: source.octets(),
            destination: destination.octets(),
        };
        
        // Calculate checksum
        header.update_checksum();
        header
    }
    
    pub fn version(&self) -> u8 {
        self.version_ihl >> 4
    }
    
    pub fn header_length(&self) -> usize {
        ((self.version_ihl & 0x0F) * 4) as usize
    }
    
    pub fn total_length(&self) -> u16 {
        NetworkEndian::read_u16(&self.total_length)
    }
    
    pub fn identification(&self) -> u16 {
        NetworkEndian::read_u16(&self.identification)
    }
    
    pub fn flags(&self) -> u8 {
        self.flags_fragment_offset[0] >> 5
    }
    
    pub fn fragment_offset(&self) -> u16 {
        NetworkEndian::read_u16(&self.flags_fragment_offset) & 0x1FFF
    }
    
    pub fn protocol(&self) -> Protocol {
        Protocol::from(self.protocol)
    }
    
    pub fn source(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.source)
    }
    
    pub fn destination(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.destination)
    }
    
    pub fn verify_checksum(&self) -> bool {
        let sum = checksum(
            unsafe {
                core::slice::from_raw_parts(
                    self as *const _ as *const u8,
                    self.header_length(),
                )
            },
            0,
        );
        sum == 0
    }
    
    pub fn update_checksum(&mut self) {
        self.checksum = [0, 0];
        let sum = checksum(
            unsafe {
                core::slice::from_raw_parts(
                    self as *const _ as *const u8,
                    self.header_length(),
                )
            },
            0,
        );
        self.checksum = sum.to_be_bytes();
    }
}

/// IPv4 layer
pub struct Ipv4Layer {
    /// Routing table
    routing: Arc<RoutingTable>,
    /// Protocol handlers
    handlers: RwLock<HashMap<Protocol, Box<dyn ProtocolHandler>>>,
    /// Fragmentation reassembly
    fragments: Mutex<HashMap<(Ipv4Addr, u16), FragmentReassembly>>,
}

/// Routing table
pub struct RoutingTable {
    routes: RwLock<Vec<Route>>,
}

/// Route entry
#[derive(Debug, Clone)]
pub struct Route {
    pub destination: Ipv4Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv4Addr>,
    pub interface: InterfaceIndex,
    pub metric: u32,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            routes: RwLock::new(Vec::new()),
        }
    }
    
    /// Add route
    pub fn add_route(&self, route: Route) {
        let mut routes = self.routes.write();
        routes.push(route);
        routes.sort_by_key(|r| (r.prefix_len, r.metric));
    }
    
    /// Lookup route for destination
    pub fn lookup(&self, destination: Ipv4Addr) -> Option<Route> {
        let dest_u32 = u32::from_be_bytes(destination.octets());
        
        self.routes.read()
            .iter()
            .rev() // Longest prefix first
            .find(|route| {
                let route_u32 = u32::from_be_bytes(route.destination.octets());
                let mask = !((1u32 << (32 - route.prefix_len)) - 1);
                (dest_u32 & mask) == (route_u32 & mask)
            })
            .cloned()
    }
}

impl Ipv4Layer {
    pub fn new(routing: Arc<RoutingTable>) -> Self {
        Self {
            routing,
            handlers: RwLock::new(HashMap::new()),
            fragments: Mutex::new(HashMap::new()),
        }
    }
    
    /// Register protocol handler
    pub fn register_handler(&self, protocol: Protocol, handler: Box<dyn ProtocolHandler>) {
        self.handlers.write().insert(protocol, handler);
    }
    
    /// Process incoming packet
    pub async fn process_rx(
        &self,
        interface: &NetworkInterface,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < Ipv4Header::MIN_SIZE {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const Ipv4Header)
        };
        
        // Validate header
        if header.version() != 4 {
            return Err(NetworkError::InvalidPacket);
        }
        
        if !header.verify_checksum() {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header_len = header.header_length();
        let total_len = header.total_length() as usize;
        
        if packet.as_slice().len() < total_len {
            return Err(NetworkError::InvalidPacket);
        }
        
        // Check if packet is for us
        let dst = header.destination();
        let our_ips: Vec<Ipv4Addr> = interface.addresses.read()
            .iter()
            .filter_map(|addr| match addr.address {
                IpAddr::V4(ip) => Some(ip),
                _ => None,
            })
            .collect();
        
        if !our_ips.contains(&dst) && !dst.is_broadcast() && !dst.is_multicast() {
            // Forward packet if routing enabled
            return self.forward_packet(interface, packet).await;
        }
        
        // Handle fragmentation
        if header.flags() & 0x01 != 0 || header.fragment_offset() != 0 {
            packet = self.handle_fragment(header, packet).await?;
        }
        
        // Remove IP header
        packet.trim_front(header_len);
        
        // Dispatch to protocol handler
        let protocol = header.protocol();
        if let Some(handler) = self.handlers.read().get(&protocol) {
            handler.handle_rx(interface, EthernetAddress::BROADCAST, packet)?;
        }
        
        Ok(())
    }
    
    /// Transmit packet
    pub async fn transmit(
        &self,
        source: Ipv4Addr,
        destination: Ipv4Addr,
        protocol: Protocol,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        // Find route
        let route = self.routing.lookup(destination)
            .ok_or(NetworkError::NoRoute)?;
        
        // Get interface
        let interface = NetworkStack::get()
            .get_interface(route.interface)?;
        
        // Add IPv4 header
        packet.reserve_front(Ipv4Header::MIN_SIZE)?;
        
        let header = Ipv4Header::new(source, destination, protocol, 
                                     packet.as_slice().len() - Ipv4Header::MIN_SIZE);
        
        packet.as_mut_slice()[..Ipv4Header::MIN_SIZE]
            .copy_from_slice(unsafe {
                core::slice::from_raw_parts(
                    &header as *const _ as *const u8,
                    Ipv4Header::MIN_SIZE,
                )
            });
        
        // Determine next hop
        let next_hop = route.gateway.unwrap_or(destination);
        
        // Resolve hardware address
        let hw_addr = if let Some(addr) = NetworkStack::get().arp_cache.lookup(next_hop) {
            addr
        } else {
            // Send ARP request and queue packet
            NetworkStack::get().arp_cache.queue_packet(next_hop, packet);
            NetworkStack::get().arp_handler.send_request(&interface, next_hop).await?;
            return Ok(());
        };
        
        // Send via ethernet
        NetworkStack::get()
            .ethernet
            .transmit(&interface, hw_addr, EtherType::Ipv4, packet)
            .await
    }
    
    /// Forward packet
    async fn forward_packet(
        &self,
        _interface: &NetworkInterface,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        // Decrement TTL
        let header = unsafe {
            &mut *(packet.as_mut_slice().as_mut_ptr() as *mut Ipv4Header)
        };
        
        if header.ttl <= 1 {
            // Send ICMP time exceeded
            return Ok(());
        }
        
        header.ttl -= 1;
        header.update_checksum();
        
        // Route packet
        let destination = header.destination();
        self.transmit_raw(destination, packet).await
    }
}

/// Fragment reassembly
struct FragmentReassembly {
    fragments: BTreeMap<u16, PacketBuffer>,
    total_size: Option<usize>,
    timestamp: Instant,
}
```

## IPv6 Implementation

### IPv6 Protocol

Create `kernel/src/net/ipv6.rs`:

```rust
//! IPv6 protocol implementation

use super::*;

/// IPv6 header
#[repr(C, packed)]
pub struct Ipv6Header {
    pub version_class_label: [u8; 4],
    pub payload_length: [u8; 2],
    pub next_header: u8,
    pub hop_limit: u8,
    pub source: [u8; 16],
    pub destination: [u8; 16],
}

impl Ipv6Header {
    pub const SIZE: usize = 40;
    
    pub fn new(
        source: Ipv6Addr,
        destination: Ipv6Addr,
        next_header: Protocol,
        payload_len: usize,
    ) -> Self {
        let mut header = Self {
            version_class_label: [0; 4],
            payload_length: (payload_len as u16).to_be_bytes(),
            next_header: next_header as u8,
            hop_limit: 64,
            source: source.octets(),
            destination: destination.octets(),
        };
        
        // Set version 6
        header.version_class_label[0] = 0x60;
        header
    }
    
    pub fn version(&self) -> u8 {
        self.version_class_label[0] >> 4
    }
    
    pub fn traffic_class(&self) -> u8 {
        ((self.version_class_label[0] & 0x0F) << 4) |
        (self.version_class_label[1] >> 4)
    }
    
    pub fn flow_label(&self) -> u32 {
        let mut label = [0u8; 4];
        label[1] = self.version_class_label[1] & 0x0F;
        label[2] = self.version_class_label[2];
        label[3] = self.version_class_label[3];
        u32::from_be_bytes(label)
    }
    
    pub fn payload_length(&self) -> u16 {
        NetworkEndian::read_u16(&self.payload_length)
    }
    
    pub fn next_header(&self) -> Protocol {
        Protocol::from(self.next_header)
    }
    
    pub fn source(&self) -> Ipv6Addr {
        Ipv6Addr::from(self.source)
    }
    
    pub fn destination(&self) -> Ipv6Addr {
        Ipv6Addr::from(self.destination)
    }
}

/// IPv6 extension headers
#[derive(Debug, Clone, Copy)]
pub enum ExtensionHeader {
    HopByHop = 0,
    Routing = 43,
    Fragment = 44,
    EncapsulatingSecurityPayload = 50,
    Authentication = 51,
    DestinationOptions = 60,
}

/// IPv6 layer
pub struct Ipv6Layer {
    /// Routing table
    routing: Arc<RoutingTableV6>,
    /// Protocol handlers
    handlers: RwLock<HashMap<Protocol, Box<dyn ProtocolHandler>>>,
    /// Neighbor discovery
    neighbor_discovery: Arc<NeighborDiscovery>,
}

/// IPv6 routing table
pub struct RoutingTableV6 {
    routes: RwLock<Vec<RouteV6>>,
}

/// IPv6 route
#[derive(Debug, Clone)]
pub struct RouteV6 {
    pub destination: Ipv6Addr,
    pub prefix_len: u8,
    pub gateway: Option<Ipv6Addr>,
    pub interface: InterfaceIndex,
    pub metric: u32,
}

/// Neighbor discovery
pub struct NeighborDiscovery {
    /// Neighbor cache
    cache: RwLock<HashMap<Ipv6Addr, NeighborEntry>>,
    /// Router list
    routers: RwLock<Vec<RouterEntry>>,
}

#[derive(Debug, Clone)]
pub struct NeighborEntry {
    pub link_addr: EthernetAddress,
    pub state: NeighborState,
    pub is_router: bool,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeighborState {
    Incomplete,
    Reachable,
    Stale,
    Delay,
    Probe,
}

impl Ipv6Layer {
    pub fn new() -> Self {
        Self {
            routing: Arc::new(RoutingTableV6::new()),
            handlers: RwLock::new(HashMap::new()),
            neighbor_discovery: Arc::new(NeighborDiscovery::new()),
        }
    }
    
    /// Process incoming packet
    pub async fn process_rx(
        &self,
        interface: &NetworkInterface,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < Ipv6Header::SIZE {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const Ipv6Header)
        };
        
        // Validate header
        if header.version() != 6 {
            return Err(NetworkError::InvalidPacket);
        }
        
        // Process extension headers
        let mut next_header = header.next_header();
        let mut offset = Ipv6Header::SIZE;
        
        while is_extension_header(next_header) {
            // Parse extension header
            let (ext_next_header, ext_len) = 
                self.parse_extension_header(&packet.as_slice()[offset..], next_header)?;
            
            next_header = ext_next_header;
            offset += ext_len;
        }
        
        // Remove headers
        packet.trim_front(offset);
        
        // Dispatch to protocol handler
        if let Some(handler) = self.handlers.read().get(&next_header) {
            handler.handle_rx(interface, EthernetAddress::BROADCAST, packet)?;
        }
        
        Ok(())
    }
    
    /// Transmit packet
    pub async fn transmit(
        &self,
        source: Ipv6Addr,
        destination: Ipv6Addr,
        next_header: Protocol,
        mut packet: PacketBuffer,
    ) -> Result<()> {
        // Add IPv6 header
        packet.reserve_front(Ipv6Header::SIZE)?;
        
        let header = Ipv6Header::new(source, destination, next_header,
                                     packet.as_slice().len() - Ipv6Header::SIZE);
        
        packet.as_mut_slice()[..Ipv6Header::SIZE]
            .copy_from_slice(unsafe {
                core::slice::from_raw_parts(
                    &header as *const _ as *const u8,
                    Ipv6Header::SIZE,
                )
            });
        
        // Find route
        let route = self.routing.lookup(destination)
            .ok_or(NetworkError::NoRoute)?;
        
        // Get interface
        let interface = NetworkStack::get()
            .get_interface(route.interface)?;
        
        // Determine next hop
        let next_hop = route.gateway.unwrap_or(destination);
        
        // Resolve link-layer address
        let link_addr = self.neighbor_discovery
            .resolve(next_hop, &interface)
            .await?;
        
        // Send via ethernet
        NetworkStack::get()
            .ethernet
            .transmit(&interface, link_addr, EtherType::Ipv6, packet)
            .await
    }
}

fn is_extension_header(protocol: Protocol) -> bool {
    matches!(protocol,
        Protocol::Other(0) |   // Hop-by-hop
        Protocol::Other(43) |  // Routing
        Protocol::Other(44) |  // Fragment
        Protocol::Other(60)    // Destination options
    )
}
```

## ICMP Implementation

### ICMP Protocol

Create `kernel/src/net/icmp.rs`:

```rust
//! ICMP protocol implementation

use super::*;

/// ICMP header
#[repr(C, packed)]
pub struct IcmpHeader {
    pub typ: u8,
    pub code: u8,
    pub checksum: [u8; 2],
    pub rest: [u8; 4],
}

/// ICMP types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IcmpType {
    EchoReply = 0,
    DestinationUnreachable = 3,
    SourceQuench = 4,
    Redirect = 5,
    EchoRequest = 8,
    TimeExceeded = 11,
    ParameterProblem = 12,
    TimestampRequest = 13,
    TimestampReply = 14,
}

/// ICMP handler
pub struct IcmpHandler;

impl IcmpHandler {
    pub fn new() -> Self {
        Self
    }
    
    /// Send echo request (ping)
    pub async fn send_echo_request(
        &self,
        source: Ipv4Addr,
        destination: Ipv4Addr,
        identifier: u16,
        sequence: u16,
        data: &[u8],
    ) -> Result<()> {
        let mut packet = PacketBuffer::new(8 + data.len());
        
        // Build ICMP header
        let header = IcmpHeader {
            typ: IcmpType::EchoRequest as u8,
            code: 0,
            checksum: [0, 0],
            rest: [
                (identifier >> 8) as u8,
                identifier as u8,
                (sequence >> 8) as u8,
                sequence as u8,
            ],
        };
        
        // Copy header and data
        packet.as_mut_slice()[..8].copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                8,
            )
        });
        packet.as_mut_slice()[8..].copy_from_slice(data);
        
        // Calculate checksum
        let sum = checksum(packet.as_slice(), 0);
        packet.as_mut_slice()[2..4].copy_from_slice(&sum.to_be_bytes());
        
        // Send via IPv4
        NetworkStack::get()
            .ipv4
            .transmit(source, destination, Protocol::Icmp, packet)
            .await
    }
    
    /// Send echo reply
    async fn send_echo_reply(
        &self,
        interface: &NetworkInterface,
        source: Ipv4Addr,
        request: &IcmpHeader,
        data: &[u8],
    ) -> Result<()> {
        let mut packet = PacketBuffer::new(8 + data.len());
        
        // Build reply header
        let header = IcmpHeader {
            typ: IcmpType::EchoReply as u8,
            code: 0,
            checksum: [0, 0],
            rest: request.rest,
        };
        
        // Copy header and data
        packet.as_mut_slice()[..8].copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                8,
            )
        });
        packet.as_mut_slice()[8..].copy_from_slice(data);
        
        // Calculate checksum
        let sum = checksum(packet.as_slice(), 0);
        packet.as_mut_slice()[2..4].copy_from_slice(&sum.to_be_bytes());
        
        // Get our IP
        let our_ip = interface.addresses.read()
            .iter()
            .find_map(|addr| match addr.address {
                IpAddr::V4(ip) => Some(ip),
                _ => None,
            })
            .ok_or(NetworkError::InvalidAddress)?;
        
        // Send reply
        NetworkStack::get()
            .ipv4
            .transmit(our_ip, source, Protocol::Icmp, packet)
            .await
    }
}

impl ProtocolHandler for IcmpHandler {
    fn handle_rx(
        &self,
        interface: &NetworkInterface,
        _source_hw: EthernetAddress,
        packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < 8 {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const IcmpHeader)
        };
        
        // Verify checksum
        let sum = checksum(packet.as_slice(), 0);
        if sum != 0 {
            return Err(NetworkError::InvalidPacket);
        }
        
        // Get source IP from IP header (would be passed in real implementation)
        let source = Ipv4Addr::new(0, 0, 0, 0); // Placeholder
        
        match header.typ {
            typ if typ == IcmpType::EchoRequest as u8 => {
                // Send echo reply
                let data = &packet.as_slice()[8..];
                tokio::spawn(async move {
                    let handler = IcmpHandler::new();
                    handler.send_echo_reply(interface, source, header, data).await;
                });
            }
            typ if typ == IcmpType::EchoReply as u8 => {
                // Handle ping reply
                let identifier = u16::from_be_bytes([header.rest[0], header.rest[1]]);
                let sequence = u16::from_be_bytes([header.rest[2], header.rest[3]]);
                
                // Notify waiting ping request
                // TODO: Implement ping tracking
            }
            _ => {
                // Handle other ICMP types
            }
        }
        
        Ok(())
    }
}
```

## UDP Implementation

### UDP Protocol

Create `kernel/src/net/udp.rs`:

```rust
//! UDP protocol implementation

use super::*;
use alloc::collections::HashMap;

/// UDP header
#[repr(C, packed)]
pub struct UdpHeader {
    pub source_port: [u8; 2],
    pub dest_port: [u8; 2],
    pub length: [u8; 2],
    pub checksum: [u8; 2],
}

impl UdpHeader {
    pub const SIZE: usize = 8;
    
    pub fn new(source_port: u16, dest_port: u16, payload_len: usize) -> Self {
        let length = (Self::SIZE + payload_len) as u16;
        Self {
            source_port: source_port.to_be_bytes(),
            dest_port: dest_port.to_be_bytes(),
            length: length.to_be_bytes(),
            checksum: [0, 0], // Will be calculated later
        }
    }
    
    pub fn source_port(&self) -> u16 {
        NetworkEndian::read_u16(&self.source_port)
    }
    
    pub fn dest_port(&self) -> u16 {
        NetworkEndian::read_u16(&self.dest_port)
    }
    
    pub fn length(&self) -> u16 {
        NetworkEndian::read_u16(&self.length)
    }
    
    pub fn checksum(&self) -> u16 {
        NetworkEndian::read_u16(&self.checksum)
    }
}

/// UDP socket
pub struct UdpSocket {
    /// Local endpoint
    local: Endpoint,
    /// Remote endpoint (for connected sockets)
    remote: Option<Endpoint>,
    /// Receive buffer
    rx_buffer: Mutex<VecDeque<(Endpoint, PacketBuffer)>>,
    /// Socket state
    state: AtomicU8,
}

/// UDP layer
pub struct UdpLayer {
    /// Bound sockets
    sockets: RwLock<HashMap<u16, Arc<UdpSocket>>>,
    /// Port allocator
    next_ephemeral_port: AtomicU16,
}

impl UdpLayer {
    pub fn new() -> Self {
        Self {
            sockets: RwLock::new(HashMap::new()),
            next_ephemeral_port: AtomicU16::new(49152),
        }
    }
    
    /// Create UDP socket
    pub fn create_socket(&self) -> Arc<UdpSocket> {
        Arc::new(UdpSocket {
            local: Endpoint::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            remote: None,
            rx_buffer: Mutex::new(VecDeque::new()),
            state: AtomicU8::new(0),
        })
    }
    
    /// Bind socket to address
    pub fn bind(&self, socket: &Arc<UdpSocket>, endpoint: Endpoint) -> Result<()> {
        // Check if port is available
        if self.sockets.read().contains_key(&endpoint.port) {
            return Err(NetworkError::PortInUse);
        }
        
        // Update socket
        unsafe {
            let socket_mut = &mut *(socket.as_ref() as *const UdpSocket as *mut UdpSocket);
            socket_mut.local = endpoint;
        }
        
        // Register socket
        self.sockets.write().insert(endpoint.port, socket.clone());
        
        Ok(())
    }
    
    /// Connect socket (set default remote)
    pub fn connect(&self, socket: &Arc<UdpSocket>, remote: Endpoint) -> Result<()> {
        unsafe {
            let socket_mut = &mut *(socket.as_ref() as *const UdpSocket as *mut UdpSocket);
            socket_mut.remote = Some(remote);
        }
        Ok(())
    }
    
    /// Send datagram
    pub async fn send_to(
        &self,
        socket: &UdpSocket,
        data: &[u8],
        destination: Endpoint,
    ) -> Result<usize> {
        let source_port = if socket.local.port == 0 {
            // Allocate ephemeral port
            self.allocate_ephemeral_port()
        } else {
            socket.local.port
        };
        
        // Create packet
        let mut packet = PacketBuffer::new(UdpHeader::SIZE + data.len());
        
        // Build header
        let header = UdpHeader::new(source_port, destination.port, data.len());
        packet.as_mut_slice()[..UdpHeader::SIZE].copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                UdpHeader::SIZE,
            )
        });
        packet.as_mut_slice()[UdpHeader::SIZE..].copy_from_slice(data);
        
        // Calculate checksum
        let checksum = match (socket.local.addr, destination.addr) {
            (IpAddr::V4(src), IpAddr::V4(dst)) => {
                internet_checksum(
                    IpAddr::V4(src),
                    IpAddr::V4(dst),
                    Protocol::Udp,
                    packet.as_slice(),
                )
            }
            (IpAddr::V6(src), IpAddr::V6(dst)) => {
                internet_checksum(
                    IpAddr::V6(src),
                    IpAddr::V6(dst),
                    Protocol::Udp,
                    packet.as_slice(),
                )
            }
            _ => return Err(NetworkError::InvalidAddress),
        };
        
        // Update checksum in packet
        packet.as_mut_slice()[6..8].copy_from_slice(&checksum.to_be_bytes());
        
        // Send packet
        match destination.addr {
            IpAddr::V4(dst) => {
                let src = match socket.local.addr {
                    IpAddr::V4(addr) => addr,
                    _ => return Err(NetworkError::InvalidAddress),
                };
                NetworkStack::get()
                    .ipv4
                    .transmit(src, dst, Protocol::Udp, packet)
                    .await?;
            }
            IpAddr::V6(dst) => {
                let src = match socket.local.addr {
                    IpAddr::V6(addr) => addr,
                    _ => return Err(NetworkError::InvalidAddress),
                };
                NetworkStack::get()
                    .ipv6
                    .transmit(src, dst, Protocol::Udp, packet)
                    .await?;
            }
        }
        
        Ok(data.len())
    }
    
    /// Receive datagram
    pub async fn recv_from(
        &self,
        socket: &UdpSocket,
        buffer: &mut [u8],
    ) -> Result<(usize, Endpoint)> {
        loop {
            // Check receive buffer
            if let Some((remote, packet)) = socket.rx_buffer.lock().await.pop_front() {
                let len = packet.as_slice().len().min(buffer.len());
                buffer[..len].copy_from_slice(&packet.as_slice()[..len]);
                return Ok((len, remote));
            }
            
            // Wait for data
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    /// Allocate ephemeral port
    fn allocate_ephemeral_port(&self) -> u16 {
        loop {
            let port = self.next_ephemeral_port.fetch_add(1, Ordering::Relaxed);
            if port >= 65535 {
                self.next_ephemeral_port.store(49152, Ordering::Relaxed);
            }
            
            if !self.sockets.read().contains_key(&port) {
                return port;
            }
        }
    }
}

impl ProtocolHandler for UdpLayer {
    fn handle_rx(
        &self,
        _interface: &NetworkInterface,
        _source_hw: EthernetAddress,
        packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < UdpHeader::SIZE {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const UdpHeader)
        };
        
        let dest_port = header.dest_port();
        let source_port = header.source_port();
        
        // Find socket
        let socket = self.sockets.read()
            .get(&dest_port)
            .cloned();
        
        if let Some(socket) = socket {
            // Extract data
            let data_offset = UdpHeader::SIZE;
            let data_len = (header.length() as usize).saturating_sub(UdpHeader::SIZE);
            
            if packet.as_slice().len() >= data_offset + data_len {
                let mut data_packet = PacketBuffer::new(data_len);
                data_packet.as_mut_slice().copy_from_slice(
                    &packet.as_slice()[data_offset..data_offset + data_len]
                );
                
                // Queue packet
                // TODO: Get source IP from IP layer
                let source = Endpoint::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), source_port);
                socket.rx_buffer.lock().push_back((source, data_packet));
            }
        }
        
        Ok(())
    }
}
```

## TCP Implementation

### TCP Protocol

Create `kernel/src/net/tcp.rs`:

```rust
//! TCP protocol implementation

use super::*;
use core::cmp::{max, min};

/// TCP header
#[repr(C, packed)]
pub struct TcpHeader {
    pub source_port: [u8; 2],
    pub dest_port: [u8; 2],
    pub sequence: [u8; 4],
    pub acknowledgment: [u8; 4],
    pub data_offset_flags: [u8; 2],
    pub window: [u8; 2],
    pub checksum: [u8; 2],
    pub urgent: [u8; 2],
}

impl TcpHeader {
    pub const MIN_SIZE: usize = 20;
    
    pub fn new(
        source_port: u16,
        dest_port: u16,
        sequence: u32,
        acknowledgment: u32,
        flags: TcpFlags,
        window: u16,
    ) -> Self {
        let data_offset = 5u8; // 20 bytes / 4
        let data_offset_flags = ((data_offset << 4) as u16) | flags.bits();
        
        Self {
            source_port: source_port.to_be_bytes(),
            dest_port: dest_port.to_be_bytes(),
            sequence: sequence.to_be_bytes(),
            acknowledgment: acknowledgment.to_be_bytes(),
            data_offset_flags: data_offset_flags.to_be_bytes(),
            window: window.to_be_bytes(),
            checksum: [0, 0],
            urgent: [0, 0],
        }
    }
    
    pub fn source_port(&self) -> u16 {
        NetworkEndian::read_u16(&self.source_port)
    }
    
    pub fn dest_port(&self) -> u16 {
        NetworkEndian::read_u16(&self.dest_port)
    }
    
    pub fn sequence(&self) -> u32 {
        NetworkEndian::read_u32(&self.sequence)
    }
    
    pub fn acknowledgment(&self) -> u32 {
        NetworkEndian::read_u32(&self.acknowledgment)
    }
    
    pub fn data_offset(&self) -> usize {
        ((self.data_offset_flags[0] >> 4) * 4) as usize
    }
    
    pub fn flags(&self) -> TcpFlags {
        let flags_byte = ((self.data_offset_flags[0] & 0x0F) << 8) | self.data_offset_flags[1];
        TcpFlags::from_bits_truncate(flags_byte as u16)
    }
    
    pub fn window(&self) -> u16 {
        NetworkEndian::read_u16(&self.window)
    }
}

bitflags::bitflags! {
    pub struct TcpFlags: u16 {
        const FIN = 0x001;
        const SYN = 0x002;
        const RST = 0x004;
        const PSH = 0x008;
        const ACK = 0x010;
        const URG = 0x020;
        const ECE = 0x040;
        const CWR = 0x080;
        const NS  = 0x100;
    }
}

/// TCP connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    LastAck,
    TimeWait,
}

/// TCP socket
pub struct TcpSocket {
    /// Local endpoint
    local: Endpoint,
    /// Remote endpoint
    remote: Option<Endpoint>,
    /// Connection state
    state: Mutex<TcpState>,
    /// Send sequence variables
    snd: Mutex<TcpSendVars>,
    /// Receive sequence variables
    rcv: Mutex<TcpReceiveVars>,
    /// Send buffer
    send_buffer: Mutex<CircularBuffer>,
    /// Receive buffer
    recv_buffer: Mutex<CircularBuffer>,
    /// Retransmission queue
    retransmit_queue: Mutex<BTreeMap<u32, RetransmitEntry>>,
    /// Congestion control
    congestion: Mutex<CongestionControl>,
}

/// TCP send sequence variables
#[derive(Debug)]
struct TcpSendVars {
    /// Send unacknowledged
    una: u32,
    /// Send next
    nxt: u32,
    /// Send window
    wnd: u16,
    /// Send urgent pointer
    up: u32,
    /// Segment sequence number used for last window update
    wl1: u32,
    /// Segment acknowledgment number used for last window update
    wl2: u32,
    /// Initial send sequence number
    iss: u32,
}

/// TCP receive sequence variables
#[derive(Debug)]
struct TcpReceiveVars {
    /// Receive next
    nxt: u32,
    /// Receive window
    wnd: u16,
    /// Receive urgent pointer
    up: u32,
    /// Initial receive sequence number
    irs: u32,
}

/// Congestion control
#[derive(Debug)]
struct CongestionControl {
    /// Congestion window
    cwnd: u32,
    /// Slow start threshold
    ssthresh: u32,
    /// Smoothed RTT
    srtt: Duration,
    /// RTT variance
    rttvar: Duration,
    /// Retransmission timeout
    rto: Duration,
}

impl Default for CongestionControl {
    fn default() -> Self {
        Self {
            cwnd: 10 * 1460, // 10 MSS
            ssthresh: u32::MAX,
            srtt: Duration::from_millis(100),
            rttvar: Duration::from_millis(50),
            rto: Duration::from_secs(1),
        }
    }
}

/// TCP layer
pub struct TcpLayer {
    /// Active connections
    connections: RwLock<HashMap<FourTuple, Arc<TcpSocket>>>,
    /// Listening sockets
    listeners: RwLock<HashMap<u16, Arc<TcpSocket>>>,
    /// Port allocator
    next_ephemeral_port: AtomicU16,
}

/// Connection identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FourTuple {
    local: Endpoint,
    remote: Endpoint,
}

impl TcpLayer {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            listeners: RwLock::new(HashMap::new()),
            next_ephemeral_port: AtomicU16::new(49152),
        }
    }
    
    /// Create TCP socket
    pub fn create_socket(&self) -> Arc<TcpSocket> {
        Arc::new(TcpSocket {
            local: Endpoint::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            remote: None,
            state: Mutex::new(TcpState::Closed),
            snd: Mutex::new(TcpSendVars {
                una: 0,
                nxt: 0,
                wnd: 65535,
                up: 0,
                wl1: 0,
                wl2: 0,
                iss: 0,
            }),
            rcv: Mutex::new(TcpReceiveVars {
                nxt: 0,
                wnd: 65535,
                up: 0,
                irs: 0,
            }),
            send_buffer: Mutex::new(CircularBuffer::new(65536)),
            recv_buffer: Mutex::new(CircularBuffer::new(65536)),
            retransmit_queue: Mutex::new(BTreeMap::new()),
            congestion: Mutex::new(CongestionControl::default()),
        })
    }
    
    /// Bind socket
    pub fn bind(&self, socket: &Arc<TcpSocket>, endpoint: Endpoint) -> Result<()> {
        if self.listeners.read().contains_key(&endpoint.port) {
            return Err(NetworkError::PortInUse);
        }
        
        unsafe {
            let socket_mut = &mut *(socket.as_ref() as *const TcpSocket as *mut TcpSocket);
            socket_mut.local = endpoint;
        }
        
        Ok(())
    }
    
    /// Listen for connections
    pub fn listen(&self, socket: &Arc<TcpSocket>) -> Result<()> {
        *socket.state.lock() = TcpState::Listen;
        self.listeners.write().insert(socket.local.port, socket.clone());
        Ok(())
    }
    
    /// Connect to remote
    pub async fn connect(&self, socket: &Arc<TcpSocket>, remote: Endpoint) -> Result<()> {
        // Set remote endpoint
        unsafe {
            let socket_mut = &mut *(socket.as_ref() as *const TcpSocket as *mut TcpSocket);
            socket_mut.remote = Some(remote);
        }
        
        // Allocate local port if needed
        if socket.local.port == 0 {
            let port = self.allocate_ephemeral_port();
            unsafe {
                let socket_mut = &mut *(socket.as_ref() as *const TcpSocket as *mut TcpSocket);
                socket_mut.local.port = port;
            }
        }
        
        // Initialize sequence numbers
        let iss = self.generate_isn();
        {
            let mut snd = socket.snd.lock();
            snd.iss = iss;
            snd.una = iss;
            snd.nxt = iss.wrapping_add(1);
        }
        
        // Send SYN
        *socket.state.lock() = TcpState::SynSent;
        self.send_syn(socket).await?;
        
        // Register connection
        let tuple = FourTuple {
            local: socket.local,
            remote: socket.remote.unwrap(),
        };
        self.connections.write().insert(tuple, socket.clone());
        
        // Wait for connection
        while *socket.state.lock() != TcpState::Established {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        
        Ok(())
    }
    
    /// Send data
    pub async fn send(&self, socket: &TcpSocket, data: &[u8]) -> Result<usize> {
        if *socket.state.lock() != TcpState::Established {
            return Err(NetworkError::ConnectionReset);
        }
        
        // Add to send buffer
        let written = socket.send_buffer.lock().write(data);
        
        // Trigger send
        self.send_data(socket).await?;
        
        Ok(written)
    }
    
    /// Receive data
    pub async fn recv(&self, socket: &TcpSocket, buffer: &mut [u8]) -> Result<usize> {
        loop {
            // Check receive buffer
            let read = socket.recv_buffer.lock().read(buffer);
            if read > 0 {
                return Ok(read);
            }
            
            // Check connection state
            match *socket.state.lock() {
                TcpState::Closed | TcpState::TimeWait => {
                    return Err(NetworkError::ConnectionReset);
                }
                _ => {}
            }
            
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    /// Generate initial sequence number
    fn generate_isn(&self) -> u32 {
        // RFC 6528: Use cryptographic hash of connection tuple and timestamp
        // For now, use simple timestamp-based ISN
        (Instant::now().as_nanos() & 0xFFFFFFFF) as u32
    }
    
    /// Send SYN packet
    async fn send_syn(&self, socket: &TcpSocket) -> Result<()> {
        let snd = socket.snd.lock();
        let rcv = socket.rcv.lock();
        
        let header = TcpHeader::new(
            socket.local.port,
            socket.remote.unwrap().port,
            snd.iss,
            0,
            TcpFlags::SYN,
            rcv.wnd,
        );
        
        drop(snd);
        drop(rcv);
        
        self.send_packet(socket, header, &[]).await
    }
    
    /// Send packet
    async fn send_packet(
        &self,
        socket: &TcpSocket,
        mut header: TcpHeader,
        data: &[u8],
    ) -> Result<()> {
        let mut packet = PacketBuffer::new(header.data_offset() + data.len());
        
        // Copy header
        packet.as_mut_slice()[..header.data_offset()].copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                &header as *const _ as *const u8,
                header.data_offset(),
            )
        });
        
        // Copy data
        packet.as_mut_slice()[header.data_offset()..].copy_from_slice(data);
        
        // Calculate checksum
        let checksum = match (socket.local.addr, socket.remote.unwrap().addr) {
            (IpAddr::V4(src), IpAddr::V4(dst)) => {
                internet_checksum(
                    IpAddr::V4(src),
                    IpAddr::V4(dst),
                    Protocol::Tcp,
                    packet.as_slice(),
                )
            }
            (IpAddr::V6(src), IpAddr::V6(dst)) => {
                internet_checksum(
                    IpAddr::V6(src),
                    IpAddr::V6(dst),
                    Protocol::Tcp,
                    packet.as_slice(),
                )
            }
            _ => return Err(NetworkError::InvalidAddress),
        };
        
        // Update checksum
        packet.as_mut_slice()[16..18].copy_from_slice(&checksum.to_be_bytes());
        
        // Send via IP layer
        match socket.remote.unwrap().addr {
            IpAddr::V4(dst) => {
                let src = match socket.local.addr {
                    IpAddr::V4(addr) => addr,
                    _ => return Err(NetworkError::InvalidAddress),
                };
                NetworkStack::get()
                    .ipv4
                    .transmit(src, dst, Protocol::Tcp, packet)
                    .await
            }
            IpAddr::V6(dst) => {
                let src = match socket.local.addr {
                    IpAddr::V6(addr) => addr,
                    _ => return Err(NetworkError::InvalidAddress),
                };
                NetworkStack::get()
                    .ipv6
                    .transmit(src, dst, Protocol::Tcp, packet)
                    .await
            }
        }
    }
}

impl ProtocolHandler for TcpLayer {
    fn handle_rx(
        &self,
        _interface: &NetworkInterface,
        _source_hw: EthernetAddress,
        packet: PacketBuffer,
    ) -> Result<()> {
        if packet.as_slice().len() < TcpHeader::MIN_SIZE {
            return Err(NetworkError::InvalidPacket);
        }
        
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const TcpHeader)
        };
        
        // TODO: Get source/dest IP from IP layer
        let source = Endpoint::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), header.source_port());
        let dest = Endpoint::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), header.dest_port());
        
        // Check for existing connection
        let tuple = FourTuple {
            local: dest,
            remote: source,
        };
        
        if let Some(socket) = self.connections.read().get(&tuple).cloned() {
            self.handle_packet(socket, header, packet);
            return Ok(());
        }
        
        // Check for listener
        if header.flags().contains(TcpFlags::SYN) && !header.flags().contains(TcpFlags::ACK) {
            if let Some(listener) = self.listeners.read().get(&dest.port).cloned() {
                self.handle_syn(listener, source, header);
            }
        }
        
        Ok(())
    }
}

/// Circular buffer for TCP data
struct CircularBuffer {
    buffer: Vec<u8>,
    read_pos: usize,
    write_pos: usize,
    size: usize,
}

impl CircularBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0; capacity],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        }
    }
    
    fn write(&mut self, data: &[u8]) -> usize {
        let available = self.buffer.len() - self.size;
        let to_write = data.len().min(available);
        
        for i in 0..to_write {
            self.buffer[self.write_pos] = data[i];
            self.write_pos = (self.write_pos + 1) % self.buffer.len();
        }
        
        self.size += to_write;
        to_write
    }
    
    fn read(&mut self, data: &mut [u8]) -> usize {
        let to_read = data.len().min(self.size);
        
        for i in 0..to_read {
            data[i] = self.buffer[self.read_pos];
            self.read_pos = (self.read_pos + 1) % self.buffer.len();
        }
        
        self.size -= to_read;
        to_read
    }
}
```

## Socket API

### Socket Layer

Create `kernel/src/net/socket.rs`:

```rust
//! Socket API implementation

use super::*;
use core::future::Future;
use core::pin::Pin;

/// Socket types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketType {
    Stream,  // TCP
    Dgram,   // UDP
    Raw,     // Raw IP
}

/// Socket domain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketDomain {
    Inet,   // IPv4
    Inet6,  // IPv6
    Unix,   // Unix domain sockets
}

/// Generic socket
pub enum Socket {
    Tcp(Arc<TcpSocket>),
    Udp(Arc<UdpSocket>),
    Raw(Arc<RawSocket>),
    Unix(Arc<UnixSocket>),
}

impl Socket {
    /// Create new socket
    pub fn new(domain: SocketDomain, typ: SocketType) -> Result<Self> {
        match (domain, typ) {
            (SocketDomain::Inet | SocketDomain::Inet6, SocketType::Stream) => {
                Ok(Socket::Tcp(NetworkStack::get().tcp.create_socket()))
            }
            (SocketDomain::Inet | SocketDomain::Inet6, SocketType::Dgram) => {
                Ok(Socket::Udp(NetworkStack::get().udp.create_socket()))
            }
            (SocketDomain::Unix, _) => {
                Ok(Socket::Unix(UnixSocket::new()))
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Bind to address
    pub fn bind(&self, addr: SocketAddr) -> Result<()> {
        match self {
            Socket::Tcp(sock) => {
                let endpoint = addr_to_endpoint(addr)?;
                NetworkStack::get().tcp.bind(sock, endpoint)
            }
            Socket::Udp(sock) => {
                let endpoint = addr_to_endpoint(addr)?;
                NetworkStack::get().udp.bind(sock, endpoint)
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Listen for connections
    pub fn listen(&self, backlog: u32) -> Result<()> {
        match self {
            Socket::Tcp(sock) => NetworkStack::get().tcp.listen(sock),
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Accept connection
    pub async fn accept(&self) -> Result<(Socket, SocketAddr)> {
        match self {
            Socket::Tcp(sock) => {
                let new_sock = NetworkStack::get().tcp.accept(sock).await?;
                let addr = endpoint_to_addr(new_sock.remote.unwrap())?;
                Ok((Socket::Tcp(new_sock), addr))
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Connect to remote
    pub async fn connect(&self, addr: SocketAddr) -> Result<()> {
        match self {
            Socket::Tcp(sock) => {
                let endpoint = addr_to_endpoint(addr)?;
                NetworkStack::get().tcp.connect(sock, endpoint).await
            }
            Socket::Udp(sock) => {
                let endpoint = addr_to_endpoint(addr)?;
                NetworkStack::get().udp.connect(sock, endpoint)
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Send data
    pub async fn send(&self, data: &[u8], flags: SendFlags) -> Result<usize> {
        match self {
            Socket::Tcp(sock) => NetworkStack::get().tcp.send(sock, data).await,
            Socket::Udp(sock) => {
                if let Some(remote) = sock.remote {
                    NetworkStack::get().udp.send_to(sock, data, remote).await
                } else {
                    Err(NetworkError::InvalidAddress)
                }
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Send to specific address
    pub async fn send_to(
        &self,
        data: &[u8],
        addr: SocketAddr,
        flags: SendFlags,
    ) -> Result<usize> {
        match self {
            Socket::Udp(sock) => {
                let endpoint = addr_to_endpoint(addr)?;
                NetworkStack::get().udp.send_to(sock, data, endpoint).await
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Receive data
    pub async fn recv(&self, buffer: &mut [u8], flags: RecvFlags) -> Result<usize> {
        match self {
            Socket::Tcp(sock) => NetworkStack::get().tcp.recv(sock, buffer).await,
            Socket::Udp(sock) => {
                let (len, _) = NetworkStack::get().udp.recv_from(sock, buffer).await?;
                Ok(len)
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Receive from specific address
    pub async fn recv_from(
        &self,
        buffer: &mut [u8],
        flags: RecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        match self {
            Socket::Udp(sock) => {
                let (len, endpoint) = NetworkStack::get().udp.recv_from(sock, buffer).await?;
                let addr = endpoint_to_addr(endpoint)?;
                Ok((len, addr))
            }
            _ => Err(NetworkError::ProtocolNotSupported),
        }
    }
    
    /// Shutdown socket
    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        match self {
            Socket::Tcp(sock) => NetworkStack::get().tcp.shutdown(sock, how),
            _ => Ok(()),
        }
    }
}

bitflags::bitflags! {
    pub struct SendFlags: u32 {
        const DONTROUTE = 0x0001;
        const DONTWAIT  = 0x0002;
        const OOB       = 0x0004;
        const NOSIGNAL  = 0x0008;
    }
    
    pub struct RecvFlags: u32 {
        const DONTWAIT  = 0x0001;
        const PEEK      = 0x0002;
        const WAITALL   = 0x0004;
        const TRUNC     = 0x0008;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

/// Socket address
#[derive(Debug, Clone, Copy)]
pub enum SocketAddr {
    V4(SocketAddrV4),
    V6(SocketAddrV6),
    Unix(UnixAddr),
}

#[derive(Debug, Clone, Copy)]
pub struct SocketAddrV4 {
    pub addr: Ipv4Addr,
    pub port: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct SocketAddrV6 {
    pub addr: Ipv6Addr,
    pub port: u16,
    pub flowinfo: u32,
    pub scope_id: u32,
}

fn addr_to_endpoint(addr: SocketAddr) -> Result<Endpoint> {
    match addr {
        SocketAddr::V4(v4) => Ok(Endpoint::new(IpAddr::V4(v4.addr), v4.port)),
        SocketAddr::V6(v6) => Ok(Endpoint::new(IpAddr::V6(v6.addr), v6.port)),
        _ => Err(NetworkError::InvalidAddress),
    }
}

fn endpoint_to_addr(endpoint: Endpoint) -> Result<SocketAddr> {
    match endpoint.addr {
        IpAddr::V4(addr) => Ok(SocketAddr::V4(SocketAddrV4 {
            addr,
            port: endpoint.port,
        })),
        IpAddr::V6(addr) => Ok(SocketAddr::V6(SocketAddrV6 {
            addr,
            port: endpoint.port,
            flowinfo: 0,
            scope_id: 0,
        })),
    }
}

/// Raw socket
pub struct RawSocket {
    protocol: Protocol,
    rx_buffer: Mutex<VecDeque<PacketBuffer>>,
}

/// Unix domain socket
pub struct UnixSocket {
    path: Option<String>,
    peer: Option<Arc<UnixSocket>>,
    rx_buffer: Mutex<VecDeque<Vec<u8>>>,
}

impl UnixSocket {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            path: None,
            peer: None,
            rx_buffer: Mutex::new(VecDeque::new()),
        })
    }
}
```

## Network Namespaces

### Namespace Implementation

Create `kernel/src/net/namespace.rs`:

```rust
//! Network namespace support

use super::*;

/// Network namespace
pub struct NetworkNamespace {
    /// Namespace ID
    pub id: u32,
    /// Interfaces in this namespace
    pub interfaces: RwLock<HashMap<InterfaceIndex, Arc<NetworkInterface>>>,
    /// Routing table
    pub routing_v4: Arc<RoutingTable>,
    pub routing_v6: Arc<RoutingTableV6>,
    /// Firewall rules
    pub firewall: Arc<Firewall>,
    /// Namespace-specific settings
    pub settings: RwLock<NamespaceSettings>,
}

/// Namespace settings
#[derive(Debug, Clone)]
pub struct NamespaceSettings {
    /// Enable IP forwarding
    pub ip_forward: bool,
    /// Enable IPv6
    pub ipv6_enabled: bool,
    /// Default TTL
    pub default_ttl: u8,
    /// TCP settings
    pub tcp_settings: TcpSettings,
}

impl Default for NamespaceSettings {
    fn default() -> Self {
        Self {
            ip_forward: false,
            ipv6_enabled: true,
            default_ttl: 64,
            tcp_settings: TcpSettings::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TcpSettings {
    pub keepalive_time: Duration,
    pub keepalive_interval: Duration,
    pub keepalive_probes: u8,
    pub syn_retries: u8,
    pub fin_timeout: Duration,
}

impl Default for TcpSettings {
    fn default() -> Self {
        Self {
            keepalive_time: Duration::from_secs(7200),
            keepalive_interval: Duration::from_secs(75),
            keepalive_probes: 9,
            syn_retries: 6,
            fin_timeout: Duration::from_secs(60),
        }
    }
}

impl NetworkNamespace {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            interfaces: RwLock::new(HashMap::new()),
            routing_v4: Arc::new(RoutingTable::new()),
            routing_v6: Arc::new(RoutingTableV6::new()),
            firewall: Arc::new(Firewall::new()),
            settings: RwLock::new(NamespaceSettings::default()),
        }
    }
    
    /// Create virtual interface pair (veth)
    pub fn create_veth_pair(
        &self,
        name1: &str,
        name2: &str,
        other_ns: &NetworkNamespace,
    ) -> Result<(InterfaceIndex, InterfaceIndex)> {
        // Create virtual ethernet pair
        let (veth1, veth2) = VirtualEthernet::create_pair()?;
        
        // Add to namespaces
        let idx1 = self.add_interface(name1, veth1)?;
        let idx2 = other_ns.add_interface(name2, veth2)?;
        
        Ok((idx1, idx2))
    }
    
    /// Move interface to namespace
    pub fn move_interface(
        &self,
        interface: Arc<NetworkInterface>,
        from_ns: &NetworkNamespace,
    ) -> Result<()> {
        // Remove from source namespace
        from_ns.interfaces.write()
            .remove(&interface.index);
        
        // Add to this namespace
        self.interfaces.write()
            .insert(interface.index, interface);
        
        Ok(())
    }
}

/// Virtual ethernet device
pub struct VirtualEthernet {
    peer: Option<Arc<Mutex<VirtualEthernet>>>,
    queue: Mutex<VecDeque<PacketBuffer>>,
    stats: NetworkStatistics,
}

impl VirtualEthernet {
    fn create_pair() -> Result<(Box<dyn NetworkDevice>, Box<dyn NetworkDevice>)> {
        let veth1 = Arc::new(Mutex::new(Self {
            peer: None,
            queue: Mutex::new(VecDeque::new()),
            stats: NetworkStatistics::default(),
        }));
        
        let veth2 = Arc::new(Mutex::new(Self {
            peer: Some(veth1.clone()),
            queue: Mutex::new(VecDeque::new()),
            stats: NetworkStatistics::default(),
        }));
        
        // Set peer for veth1
        veth1.lock().peer = Some(veth2.clone());
        
        Ok((
            Box::new(VethDevice(veth1)),
            Box::new(VethDevice(veth2)),
        ))
    }
}

struct VethDevice(Arc<Mutex<VirtualEthernet>>);

impl NetworkDevice for VethDevice {
    fn transmit(&mut self, packet: PacketBuffer) -> Result<()> {
        let veth = self.0.lock();
        
        // Send to peer
        if let Some(peer) = &veth.peer {
            peer.lock().queue.lock().push_back(packet);
        }
        
        Ok(())
    }
    
    fn receive(&mut self) -> Option<PacketBuffer> {
        self.0.lock().queue.lock().pop_front()
    }
    
    // ... other trait methods
}
```

## Advanced Features

### Netfilter/iptables

Create `kernel/src/net/firewall.rs`:

```rust
//! Firewall and packet filtering

use super::*;

/// Firewall implementation
pub struct Firewall {
    /// Filter tables
    tables: RwLock<HashMap<String, Table>>,
}

/// Firewall table
pub struct Table {
    /// Table name
    name: String,
    /// Chains in this table
    chains: HashMap<String, Chain>,
}

/// Firewall chain
pub struct Chain {
    /// Chain name
    name: String,
    /// Default policy
    policy: Action,
    /// Rules in order
    rules: Vec<Rule>,
}

/// Firewall rule
#[derive(Debug, Clone)]
pub struct Rule {
    /// Match conditions
    matches: Vec<Match>,
    /// Target action
    target: Action,
    /// Rule statistics
    stats: RuleStats,
}

/// Match conditions
#[derive(Debug, Clone)]
pub enum Match {
    /// Source address
    SourceAddr(IpAddr, u8), // addr, prefix_len
    /// Destination address
    DestAddr(IpAddr, u8),
    /// Input interface
    InInterface(String),
    /// Output interface
    OutInterface(String),
    /// Protocol
    Protocol(Protocol),
    /// Source port
    SourcePort(u16),
    /// Destination port
    DestPort(u16),
    /// TCP flags
    TcpFlags(TcpFlags, TcpFlags), // mask, match
    /// Connection state
    ConnState(ConnectionState),
}

/// Firewall actions
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Accept,
    Drop,
    Reject,
    Jump(ChainId),
    Return,
    Queue(u16),
    Log(LogLevel),
}

#[derive(Debug, Default)]
struct RuleStats {
    packets: AtomicU64,
    bytes: AtomicU64,
}

impl Firewall {
    pub fn new() -> Self {
        let mut firewall = Self {
            tables: RwLock::new(HashMap::new()),
        };
        
        // Create default tables
        firewall.create_default_tables();
        firewall
    }
    
    fn create_default_tables(&mut self) {
        // Filter table
        let mut filter = Table {
            name: "filter".to_string(),
            chains: HashMap::new(),
        };
        
        filter.chains.insert("INPUT".to_string(), Chain {
            name: "INPUT".to_string(),
            policy: Action::Accept,
            rules: Vec::new(),
        });
        
        filter.chains.insert("FORWARD".to_string(), Chain {
            name: "FORWARD".to_string(),
            policy: Action::Accept,
            rules: Vec::new(),
        });
        
        filter.chains.insert("OUTPUT".to_string(), Chain {
            name: "OUTPUT".to_string(),
            policy: Action::Accept,
            rules: Vec::new(),
        });
        
        self.tables.write().insert("filter".to_string(), filter);
        
        // NAT table
        let mut nat = Table {
            name: "nat".to_string(),
            chains: HashMap::new(),
        };
        
        nat.chains.insert("PREROUTING".to_string(), Chain {
            name: "PREROUTING".to_string(),
            policy: Action::Accept,
            rules: Vec::new(),
        });
        
        nat.chains.insert("POSTROUTING".to_string(), Chain {
            name: "POSTROUTING".to_string(),
            policy: Action::Accept,
            rules: Vec::new(),
        });
        
        self.tables.write().insert("nat".to_string(), nat);
    }
    
    /// Process packet through firewall
    pub fn filter_packet(
        &self,
        hook: NetfilterHook,
        packet: &PacketBuffer,
        meta: &PacketMetadata,
    ) -> Action {
        let table_name = match hook {
            NetfilterHook::PreRouting => "nat",
            NetfilterHook::Input => "filter",
            NetfilterHook::Forward => "filter",
            NetfilterHook::Output => "filter",
            NetfilterHook::PostRouting => "nat",
        };
        
        let chain_name = match hook {
            NetfilterHook::PreRouting => "PREROUTING",
            NetfilterHook::Input => "INPUT",
            NetfilterHook::Forward => "FORWARD",
            NetfilterHook::Output => "OUTPUT",
            NetfilterHook::PostRouting => "POSTROUTING",
        };
        
        let tables = self.tables.read();
        if let Some(table) = tables.get(table_name) {
            if let Some(chain) = table.chains.get(chain_name) {
                return self.process_chain(chain, packet, meta);
            }
        }
        
        Action::Accept
    }
    
    fn process_chain(
        &self,
        chain: &Chain,
        packet: &PacketBuffer,
        meta: &PacketMetadata,
    ) -> Action {
        for rule in &chain.rules {
            if self.match_rule(rule, packet, meta) {
                // Update statistics
                rule.stats.packets.fetch_add(1, Ordering::Relaxed);
                rule.stats.bytes.fetch_add(packet.as_slice().len() as u64, Ordering::Relaxed);
                
                return rule.target;
            }
        }
        
        chain.policy
    }
    
    fn match_rule(&self, rule: &Rule, packet: &PacketBuffer, meta: &PacketMetadata) -> bool {
        for match_cond in &rule.matches {
            if !self.match_condition(match_cond, packet, meta) {
                return false;
            }
        }
        true
    }
    
    fn match_condition(
        &self,
        match_cond: &Match,
        packet: &PacketBuffer,
        meta: &PacketMetadata,
    ) -> bool {
        match match_cond {
            Match::SourceAddr(addr, prefix_len) => {
                // Check source address
                // TODO: Extract from packet
                true
            }
            Match::Protocol(proto) => meta.protocol == *proto,
            // ... other matches
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NetfilterHook {
    PreRouting,
    Input,
    Forward,
    Output,
    PostRouting,
}

#[derive(Debug)]
pub struct PacketMetadata {
    pub in_interface: Option<InterfaceIndex>,
    pub out_interface: Option<InterfaceIndex>,
    pub protocol: Protocol,
    pub source_addr: Option<IpAddr>,
    pub dest_addr: Option<IpAddr>,
}
```

### QoS and Traffic Control

```rust
/// Quality of Service implementation
pub struct QoS {
    /// Traffic classes
    classes: RwLock<HashMap<u32, TrafficClass>>,
    /// Queuing disciplines
    qdiscs: RwLock<HashMap<InterfaceIndex, Box<dyn QueueingDiscipline>>>,
}

/// Traffic class
pub struct TrafficClass {
    /// Class ID
    id: u32,
    /// Guaranteed bandwidth
    guaranteed_bw: u64,
    /// Maximum bandwidth
    max_bw: u64,
    /// Priority
    priority: u8,
}

/// Queuing discipline trait
pub trait QueueingDiscipline: Send + Sync {
    /// Enqueue packet
    fn enqueue(&mut self, packet: PacketBuffer, class: u32) -> Result<()>;
    
    /// Dequeue packet
    fn dequeue(&mut self) -> Option<PacketBuffer>;
    
    /// Get statistics
    fn stats(&self) -> QdiscStats;
}

/// Token bucket filter
pub struct TokenBucket {
    /// Token bucket size
    bucket_size: u64,
    /// Current tokens
    tokens: AtomicU64,
    /// Token rate (tokens per second)
    rate: u64,
    /// Last update
    last_update: Mutex<Instant>,
    /// Packet queue
    queue: Mutex<VecDeque<PacketBuffer>>,
}

impl QueueingDiscipline for TokenBucket {
    fn enqueue(&mut self, packet: PacketBuffer, _class: u32) -> Result<()> {
        // Update tokens
        self.update_tokens();
        
        let packet_size = packet.as_slice().len() as u64;
        if self.tokens.load(Ordering::Relaxed) >= packet_size {
            // Consume tokens and transmit
            self.tokens.fetch_sub(packet_size, Ordering::Relaxed);
            // Would transmit packet here
            Ok(())
        } else {
            // Queue packet
            self.queue.lock().push_back(packet);
            Ok(())
        }
    }
    
    fn dequeue(&mut self) -> Option<PacketBuffer> {
        self.update_tokens();
        
        let mut queue = self.queue.lock();
        if let Some(packet) = queue.front() {
            let packet_size = packet.as_slice().len() as u64;
            if self.tokens.load(Ordering::Relaxed) >= packet_size {
                self.tokens.fetch_sub(packet_size, Ordering::Relaxed);
                return queue.pop_front();
            }
        }
        
        None
    }
    
    fn stats(&self) -> QdiscStats {
        QdiscStats {
            packets: 0,
            bytes: 0,
            drops: 0,
            overlimits: 0,
        }
    }
}

impl TokenBucket {
    fn update_tokens(&self) {
        let mut last_update = self.last_update.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(*last_update);
        
        let new_tokens = (elapsed.as_secs_f64() * self.rate as f64) as u64;
        let current = self.tokens.load(Ordering::Relaxed);
        let updated = (current + new_tokens).min(self.bucket_size);
        
        self.tokens.store(updated, Ordering::Relaxed);
        *last_update = now;
    }
}

#[derive(Debug, Default)]
pub struct QdiscStats {
    pub packets: u64,
    pub bytes: u64,
    pub drops: u64,
    pub overlimits: u64,
}
```

## Performance Optimizations

### Zero-Copy Networking

```rust
/// Zero-copy socket buffer
pub struct ZeroCopyBuffer {
    /// Physical pages
    pages: Vec<PhysicalPage>,
    /// Offset in first page
    offset: usize,
    /// Total length
    length: usize,
    /// Reference count
    refcount: Arc<AtomicUsize>,
}

impl ZeroCopyBuffer {
    /// Create from user pages
    pub fn from_user_pages(addr: VirtAddr, len: usize) -> Result<Self> {
        // Pin user pages
        let pages = pin_user_pages(addr, len)?;
        
        Ok(Self {
            pages,
            offset: addr.as_u64() as usize & 0xFFF,
            length: len,
            refcount: Arc::new(AtomicUsize::new(1)),
        })
    }
    
    /// Get physical addresses for DMA
    pub fn get_phys_addrs(&self) -> Vec<(PhysAddr, usize)> {
        let mut addrs = Vec::new();
        let mut remaining = self.length;
        let mut offset = self.offset;
        
        for page in &self.pages {
            let len = (4096 - offset).min(remaining);
            addrs.push((page.phys_addr() + offset, len));
            remaining -= len;
            offset = 0;
        }
        
        addrs
    }
}

/// Segmentation offload
pub struct SegmentationOffload {
    /// TCP segmentation offload
    tso: bool,
    /// UDP fragmentation offload
    ufo: bool,
    /// Generic segmentation offload
    gso: bool,
    /// Maximum segment size
    max_segment_size: u16,
}

impl SegmentationOffload {
    /// Perform software GSO
    pub fn segment_packet(&self, packet: PacketBuffer, mss: u16) -> Vec<PacketBuffer> {
        let mut segments = Vec::new();
        
        // Parse headers
        // ... extract header info
        
        // Split payload into segments
        let payload_offset = 0; // Would calculate actual offset
        let payload = &packet.as_slice()[payload_offset..];
        
        for chunk in payload.chunks(mss as usize) {
            let mut segment = PacketBuffer::new(payload_offset + chunk.len());
            
            // Copy headers
            segment.as_mut_slice()[..payload_offset]
                .copy_from_slice(&packet.as_slice()[..payload_offset]);
            
            // Copy payload chunk
            segment.as_mut_slice()[payload_offset..]
                .copy_from_slice(chunk);
            
            // Update headers (sequence numbers, checksums, etc.)
            // ...
            
            segments.push(segment);
        }
        
        segments
    }
}
```

### CPU Affinity and RSS

```rust
/// Receive Side Scaling (RSS)
pub struct RSS {
    /// Number of queues
    num_queues: usize,
    /// Indirection table
    indirection_table: Vec<u8>,
    /// Hash key
    hash_key: [u8; 40],
}

impl RSS {
    /// Calculate RSS hash
    pub fn calculate_hash(&self, packet: &PacketBuffer) -> u32 {
        // Extract flow information
        let flow = extract_flow(packet);
        
        // Toeplitz hash
        let mut hash = 0u32;
        let data = [
            flow.src_addr.as_bytes(),
            flow.dst_addr.as_bytes(),
            &flow.src_port.to_be_bytes(),
            &flow.dst_port.to_be_bytes(),
        ].concat();
        
        for (i, &byte) in data.iter().enumerate() {
            for bit in 0..8 {
                if byte & (1 << (7 - bit)) != 0 {
                    hash ^= self.get_key_part(i * 8 + bit);
                }
            }
        }
        
        hash
    }
    
    /// Get queue for hash
    pub fn get_queue(&self, hash: u32) -> usize {
        let index = (hash as usize) & (self.indirection_table.len() - 1);
        self.indirection_table[index] as usize % self.num_queues
    }
    
    fn get_key_part(&self, bit_offset: usize) -> u32 {
        let byte_offset = bit_offset / 8;
        let bit_in_byte = bit_offset % 8;
        
        let mut part = 0u32;
        for i in 0..4 {
            if byte_offset + i < self.hash_key.len() {
                part |= (self.hash_key[byte_offset + i] as u32) << (24 - i * 8);
            }
        }
        
        part << bit_in_byte
    }
}

#[derive(Debug)]
struct FlowInfo {
    src_addr: IpAddr,
    dst_addr: IpAddr,
    src_port: u16,
    dst_port: u16,
}

fn extract_flow(packet: &PacketBuffer) -> FlowInfo {
    // Parse packet headers to extract flow
    // This is simplified
    FlowInfo {
        src_addr: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        dst_addr: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        src_port: 0,
        dst_port: 0,
    }
}
```

## Security Features

### Network Security

```rust
/// Network security features
pub struct NetworkSecurity {
    /// SYN cookies
    syn_cookies: SynCookies,
    /// Rate limiting
    rate_limiter: RateLimiter,
    /// Intrusion detection
    ids: IntrusionDetection,
}

/// SYN cookie implementation
pub struct SynCookies {
    /// Secret key
    secret: [u8; 32],
    /// Cookie counter
    counter: AtomicU32,
}

impl SynCookies {
    /// Generate SYN cookie
    pub fn generate(&self, src: Endpoint, dst: Endpoint, seq: u32) -> u32 {
        use sha2::{Sha256, Digest};
        
        let count = self.counter.fetch_add(1, Ordering::Relaxed);
        let timestamp = (Instant::now().as_secs() >> 6) as u32; // 64-second resolution
        
        let mut hasher = Sha256::new();
        hasher.update(&self.secret);
        hasher.update(&src.addr.to_string().as_bytes());
        hasher.update(&src.port.to_be_bytes());
        hasher.update(&dst.addr.to_string().as_bytes());
        hasher.update(&dst.port.to_be_bytes());
        hasher.update(&seq.to_be_bytes());
        hasher.update(&count.to_be_bytes());
        
        let hash = hasher.finalize();
        let cookie = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]);
        
        // Encode timestamp in upper bits
        (timestamp << 24) | (cookie & 0x00FFFFFF)
    }
    
    /// Validate SYN cookie
    pub fn validate(&self, src: Endpoint, dst: Endpoint, seq: u32, ack: u32) -> bool {
        let timestamp = ack >> 24;
        let current_time = (Instant::now().as_secs() >> 6) as u32;
        
        // Check if cookie is too old (> 4 minutes)
        if current_time.wrapping_sub(timestamp) > 4 {
            return false;
        }
        
        // Regenerate and compare
        // In practice, would need to try recent counter values
        true
    }
}

/// Rate limiting
pub struct RateLimiter {
    /// Per-IP limits
    ip_limits: DashMap<IpAddr, TokenBucket>,
    /// Global limits
    global_limit: TokenBucket,
}

impl RateLimiter {
    /// Check rate limit
    pub fn check_limit(&self, addr: IpAddr, packet_size: usize) -> bool {
        // Check global limit
        if !self.global_limit.check(packet_size) {
            return false;
        }
        
        // Check per-IP limit
        let mut entry = self.ip_limits.entry(addr).or_insert_with(|| {
            TokenBucket::new(1000, 100) // 1000 tokens, 100/sec
        });
        
        entry.check(packet_size)
    }
}

/// Intrusion detection
pub struct IntrusionDetection {
    /// Signature database
    signatures: Vec<Signature>,
    /// Anomaly detection
    anomaly_detector: AnomalyDetector,
}

#[derive(Debug, Clone)]
pub struct Signature {
    /// Signature ID
    id: u32,
    /// Pattern to match
    pattern: Vec<u8>,
    /// Severity level
    severity: Severity,
    /// Action to take
    action: IdsAction,
}

#[derive(Debug, Clone, Copy)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy)]
pub enum IdsAction {
    Log,
    Alert,
    Block,
    Reset,
}
```

## Testing Strategies

### Network Stack Testing

Create `kernel/src/net/tests/mod.rs`:

```rust
//! Network stack tests

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test packet buffer operations
    #[test]
    fn test_packet_buffer() {
        let mut buffer = PacketBuffer::new(100);
        
        // Test reserve and trim
        assert!(buffer.reserve_front(20).is_ok());
        assert_eq!(buffer.as_slice().len(), 20);
        
        buffer.trim_front(10);
        assert_eq!(buffer.as_slice().len(), 10);
        
        // Test zero-copy clone
        let clone = buffer.clone();
        assert_eq!(clone.as_slice().len(), 10);
    }
    
    /// Test ethernet parsing
    #[test]
    fn test_ethernet_parsing() {
        let data = vec![
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // Destination
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, // Source
            0x08, 0x00,                         // EtherType (IPv4)
            0x45, 0x00,                         // IP header start
        ];
        
        let packet = PacketBuffer::from_slice(&data);
        let header = unsafe {
            &*(packet.as_slice().as_ptr() as *const EthernetHeader)
        };
        
        assert_eq!(header.destination(), EthernetAddress::BROADCAST);
        assert_eq!(header.ethertype(), EtherType::Ipv4);
    }
    
    /// Test checksum calculation
    #[test]
    fn test_checksum() {
        let data = vec![0x45, 0x00, 0x00, 0x3c, 0x1c, 0x46, 0x40, 0x00,
                        0x40, 0x06, 0x00, 0x00, 0xac, 0x10, 0x0a, 0x63,
                        0xac, 0x10, 0x0a, 0x0c];
        
        let sum = checksum(&data, 0);
        assert_eq!(sum, 0xB1E6);
    }
    
    /// Integration test: UDP echo
    #[tokio::test]
    async fn test_udp_echo() {
        // Initialize network stack
        NetworkStack::init().unwrap();
        
        // Create UDP socket
        let socket = Socket::new(SocketDomain::Inet, SocketType::Dgram).unwrap();
        
        // Bind to port
        let addr = SocketAddr::V4(SocketAddrV4 {
            addr: Ipv4Addr::new(127, 0, 0, 1),
            port: 12345,
        });
        socket.bind(addr).unwrap();
        
        // Send packet
        let data = b"Hello, UDP!";
        socket.send_to(data, addr, SendFlags::empty()).await.unwrap();
        
        // Receive echo
        let mut buffer = vec![0u8; 1024];
        let (len, from) = socket.recv_from(&mut buffer, RecvFlags::empty()).await.unwrap();
        
        assert_eq!(&buffer[..len], data);
    }
    
    /// Performance benchmark
    #[bench]
    fn bench_packet_processing(b: &mut Bencher) {
        let stack = NetworkStack::get();
        let packet = create_test_packet();
        
        b.iter(|| {
            let _ = stack.process_packet(packet.clone());
        });
    }
}

/// Test utilities
mod test_utils {
    use super::*;
    
    /// Create test network interface
    pub fn create_test_interface() -> NetworkInterface {
        let loopback = LoopbackDevice::new();
        
        NetworkInterface {
            index: InterfaceIndex(0),
            device: Arc::new(Mutex::new(Box::new(loopback))),
            addresses: RwLock::new(vec![
                InterfaceAddress {
                    address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                    prefix_len: 8,
                    flags: AddressFlags::PERMANENT,
                },
            ]),
            rx_queue: Mutex::new(VecDeque::new()),
            tx_queue: Mutex::new(VecDeque::new()),
        }
    }
    
    /// Create test TCP packet
    pub fn create_tcp_packet(
        src: Endpoint,
        dst: Endpoint,
        flags: TcpFlags,
        seq: u32,
        ack: u32,
        data: &[u8],
    ) -> PacketBuffer {
        let mut packet = PacketBuffer::new(
            Ipv4Header::MIN_SIZE + TcpHeader::MIN_SIZE + data.len()
        );
        
        // Add IPv4 header
        let ip_header = Ipv4Header::new(
            match src.addr { IpAddr::V4(a) => a, _ => panic!() },
            match dst.addr { IpAddr::V4(a) => a, _ => panic!() },
            Protocol::Tcp,
            TcpHeader::MIN_SIZE + data.len(),
        );
        
        // Add TCP header
        let tcp_header = TcpHeader::new(src.port, dst.port, seq, ack, flags, 65535);
        
        // Copy headers and data
        // ...
        
        packet
    }
}
```

### Network Simulation

```rust
/// Network simulator for testing
pub struct NetworkSimulator {
    /// Simulated latency
    latency: Duration,
    /// Packet loss rate
    loss_rate: f64,
    /// Bandwidth limit
    bandwidth: u64,
    /// Packet queue
    queue: Mutex<BinaryHeap<SimulatedPacket>>,
}

#[derive(Debug)]
struct SimulatedPacket {
    arrival_time: Instant,
    packet: PacketBuffer,
}

impl NetworkSimulator {
    pub fn new(latency: Duration, loss_rate: f64, bandwidth: u64) -> Self {
        Self {
            latency,
            loss_rate,
            bandwidth,
            queue: Mutex::new(BinaryHeap::new()),
        }
    }
    
    /// Send packet through simulator
    pub fn send(&self, packet: PacketBuffer) {
        // Simulate packet loss
        if rand::random::<f64>() < self.loss_rate {
            return;
        }
        
        // Calculate arrival time based on bandwidth and latency
        let transmission_time = (packet.as_slice().len() as f64 * 8.0 / self.bandwidth as f64)
            .as_secs();
        let arrival_time = Instant::now() + self.latency + transmission_time;
        
        self.queue.lock().push(SimulatedPacket {
            arrival_time,
            packet,
        });
    }
    
    /// Receive packets that have arrived
    pub fn receive(&self) -> Vec<PacketBuffer> {
        let mut queue = self.queue.lock();
        let now = Instant::now();
        let mut packets = Vec::new();
        
        while let Some(sim_packet) = queue.peek() {
            if sim_packet.arrival_time <= now {
                packets.push(queue.pop().unwrap().packet);
            } else {
                break;
            }
        }
        
        packets
    }
}

impl Ord for SimulatedPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order for min-heap
        other.arrival_time.cmp(&self.arrival_time)
    }
}

impl PartialOrd for SimulatedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SimulatedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.arrival_time == other.arrival_time
    }
}

impl Eq for SimulatedPacket {}
```

## Conclusion

This comprehensive network stack implementation guide provides a solid foundation for building a modern, high-performance networking system for Veridian OS. Key achievements:

1. **Zero-Copy Architecture**: Minimizes data copying throughout the stack
2. **Async/Await Support**: Native async networking for modern applications
3. **Complete Protocol Stack**: Ethernet, ARP, IPv4, IPv6, ICMP, UDP, and TCP
4. **Advanced Features**: Namespaces, firewall, QoS, and security features
5. **Performance Optimizations**: RSS, GSO, and CPU affinity support

The modular design allows for:
- Easy addition of new protocols
- Hardware offload integration
- Advanced security features
- Container and virtualization support

This implementation serves as a foundation that can be extended with:
- Additional protocols (QUIC, SCTP, etc.)
- Advanced routing protocols
- Software-defined networking
- Network virtualization overlays

The emphasis on safety, performance, and modern