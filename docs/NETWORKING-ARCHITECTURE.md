# VeridianOS Networking Architecture

**Version:** v0.8.0 (Phase 7 Wave 4)
**Status:** Production networking stack with IPv4/IPv6 dual-stack, zero-copy I/O, and hardware NIC drivers

---

## Overview

VeridianOS implements a full networking stack within the microkernel, designed around
three core principles:

1. **Capability-Based Security** -- Every socket, device handle, and DMA buffer
   access is gated by the capability system. No process can transmit or receive
   packets without holding the appropriate capability token.

2. **Zero-Copy Data Path** -- Scatter-gather DMA, page pinning, and sendfile
   minimize memory copies between user space, kernel, and hardware. The DMA
   buffer pool provides pre-allocated, physically contiguous buffers below 4GB
   for direct hardware access.

3. **Microkernel Isolation** -- Hardware drivers (E1000, VirtIO-Net) register
   through the device framework and are accessed via trait objects behind
   `spin::Mutex`. The network stack itself lives in `kernel/src/net/`, while
   the driver framework layer lives in `kernel/src/drivers/`.

### Stack Initialization Order

The network stack initializes in `net::init()` with strict ordering:

```
DMA Pool -> Device Layer -> IP -> TCP -> UDP -> IPv6 -> ICMPv6 -> Socket -> Epoll -> Driver Registration
```

Each stage depends on the previous one. Driver registration (PCI scan for E1000,
VirtIO-Net) is non-fatal -- the stack operates with only the loopback device if
no hardware is detected.

---

## Architecture Diagram

```
+------------------------------------------------------------------+
|                       User Space                                  |
|  +----------+  +----------+  +----------+  +------------------+  |
|  | ping/    |  | curl/    |  | netcat/  |  | ping6 / ndp /    |  |
|  | ping6    |  | wget     |  | nc       |  | ifconfig/netstat |  |
|  +----+-----+  +----+-----+  +----+-----+  +--------+---------+  |
|       |              |              |                 |            |
+-------+--------------+--------------+-----------------+-----------+
        |              |              |                 |
  ======|==============|==============|=================|=======  syscall boundary
        |              |              |                 |
+-------+--------------+--------------+-----------------+-----------+
|                   Socket Layer (AF_INET, AF_INET6, AF_UNIX)       |
|              net/socket.rs -- BSD socket API                      |
+-------------------+-------------------+---------------------------+
                    |                   |
          +---------+---------+  +------+--------+
          |   TCP (stream)    |  |  UDP (dgram)  |
          |   net/tcp.rs      |  |  net/udp.rs   |
          +---------+---------+  +------+--------+
                    |                   |
          +---------+-------------------+--------+
          |              IP Layer                 |
          |  +-------------+  +--------------+   |
          |  | IPv4        |  | IPv6         |   |
          |  | net/ip.rs   |  | net/ipv6.rs  |   |
          |  +------+------+  +------+-------+   |
          |         |                |            |
          |  +------+------+  +-----+--------+   |
          |  | ARP         |  | ICMPv6 / NDP |   |
          |  | net/arp.rs  |  | net/icmpv6.rs|   |
          |  +-------------+  +--------------+   |
          +------------------+-------------------+
                             |
          +------------------+-------------------+
          |         Device Layer                  |
          |  net/device.rs -- NetworkDevice trait  |
          |  +----------+  +----------+  +-----+ |
          |  | Loopback |  | E1000    |  |VirtIO| |
          |  | (lo0)    |  | (eth0)   |  |-Net  | |
          |  +----------+  +----+-----+  +--+--+ |
          +---------------------+------------+----+
                                |            |
          +---------------------+------------+----+
          |         DMA Buffer Pool               |
          |  net/dma_pool.rs                      |
          |  Pre-allocated frames < 4GB           |
          +---------------------+-----------------+
                                |
          ======================|===================  MMIO / physical memory
                                |
          +---------------------+-----------------+
          |          Hardware NIC                  |
          |   TX/RX descriptor rings              |
          |   MMIO registers                      |
          +---------------------------------------+
```

---

## DMA Buffer Pool

**Source:** `kernel/src/net/dma_pool.rs`

The DMA buffer pool provides pre-allocated, physically contiguous buffers for
network packet I/O. Buffers are allocated at initialization time to avoid
allocation latency on the hot path.

### Design Constraints

| Constant          | Value   | Rationale                                    |
|-------------------|---------|----------------------------------------------|
| `DMA_BUFFER_SIZE` | 2048    | 1500 MTU + Ethernet/IP/TCP headers + padding |
| `MAX_BUFFERS`     | 512     | Upper bound per pool (1MB total)             |
| `DMA_PHYS_LIMIT`  | 4 GB    | 32-bit DMA engine compatibility              |

### DmaBuffer

Each buffer wraps a single 4KB physical frame:

```rust
pub struct DmaBuffer {
    virt_addr: usize,           // Kernel virtual address (via phys_to_virt_addr)
    phys_addr: PhysicalAddress, // Physical address for DMA descriptors
    size: usize,                // Usable size (DMA_BUFFER_SIZE = 2048)
    refcount: AtomicU64,        // Reference count for shared ownership
    index: u16,                 // Position in the pool's buffer array
    frame: FrameNumber,         // Backing frame (for future reclamation)
}
```

Key operations:
- `acquire()` / `release()` -- Atomic reference counting
- `as_slice()` / `as_mut_slice()` -- Zero-copy access to buffer memory
- `from_frame()` -- Construct from a freshly allocated physical frame

### DmaBufferPool

The pool manages a free list of buffer indices:

```rust
pub struct DmaBufferPool {
    buffers: Vec<DmaBuffer>,      // All allocated buffers
    free_list: Vec<u16>,          // Indices of available buffers
    total_buffers: usize,
    allocations: AtomicU64,       // Statistics
    deallocations: AtomicU64,
    allocation_failures: AtomicU64,
}
```

Allocation strategy:
1. Allocate frames from the Normal zone (16MB -- max)
2. Filter: reject any frame with physical address >= 4GB
3. Zero-initialize each frame before use (prevents information leaks)
4. If a frame is above 4GB, free it and continue (graceful degradation)

### Global Pool

```rust
static NETWORK_DMA_POOL: Mutex<Option<DmaBufferPool>> = Mutex::new(None);

pub fn init_network_pool(num_buffers: usize) -> Result<(), KernelError>;
pub fn with_network_pool<R, F: FnOnce(&mut DmaBufferPool) -> R>(f: F) -> Result<R, KernelError>;
```

The pool is initialized with 256 buffers during `net::init()`. Access is
serialized through `with_network_pool()`, which takes `&mut DmaBufferPool`
to allow allocation and deallocation.

---

## Zero-Copy Networking

**Source:** `kernel/src/net/zero_copy.rs`

The zero-copy subsystem eliminates memory copies along the data path through
several complementary techniques.

### Scatter-Gather I/O

`ScatterGatherList` collects physically discontiguous buffer segments into a
single logical packet:

```rust
pub struct ScatterGatherList {
    segments: Vec<ScatterGatherSegment>,
}

pub struct ScatterGatherSegment {
    pub physical_addr: u64,
    pub length: usize,
}
```

Operations:
- `add_segment()` -- Append a physical address + length pair
- `total_length()` -- Sum of all segment lengths
- `copy_to_buffer()` -- Flatten SG list into contiguous kernel buffer
- `assemble()` -- Allocate + flatten (fallback when hardware SG unavailable)

### ZeroCopySend

Transmits data from user or kernel buffers via scatter-gather DMA:

```rust
pub struct ZeroCopySend {
    sg_list: ScatterGatherList,
    completion: Option<fn()>,
}
```

The critical method is `add_user_buffer()`, which translates user virtual
addresses to physical addresses by walking the current process's page tables
(VAS page table walk):

1. Validate the user address range via `is_user_addr_valid()`
2. Walk page-by-page (4KB), translating each virtual page to its physical frame
3. Add each (phys_addr + page_offset, bytes_in_page) as an SG segment

This avoids copying user data into kernel buffers entirely. The `execute()`
method assembles the SG list and transmits through `eth0` or `lo0`.

### SendFile

Kernel-to-kernel transfer bypassing user space, equivalent to Linux `sendfile()`:

```rust
pub struct SendFile {
    source_fd: u32,
    dest_socket: u32,
    offset: u64,
    count: usize,
}
```

Two code paths:
- **Scatter-gather path** (transfers >= 64KB): Reads file data into DMA buffers,
  builds an SG list, assembles once, and writes to the destination
- **Chunked copy path** (< 64KB): 4KB loop through a stack buffer

### TcpCork

Nagle-style write coalescing for TCP:

```rust
pub struct TcpCork {
    pending: Vec<u8>,
    max_pending: usize,
    socket_id: Option<usize>,
    remote: Option<SocketAddr>,
}
```

Small writes accumulate in `pending`. When the buffer exceeds `max_pending`
bytes, or when `flush()` is called explicitly, the coalesced data is sent
through `tcp::transmit_data()` as a single TCP segment.

### TcpZeroCopySend

Combines scatter-gather collection with TCP segmentation:

```rust
pub struct TcpZeroCopySend {
    sg_list: ScatterGatherList,
    socket_id: usize,
    remote: SocketAddr,
    mss: usize,  // Default: 1460 (1500 MTU - 20 IP - 20 TCP)
}
```

Supports both kernel buffers (`add_buffer()`) and user buffers
(`add_user_buffer()`, which reuses `ZeroCopySend`'s page-pinning logic).
The `execute()` method assembles the SG list and passes the data to
`tcp::transmit_data()`, which handles MSS segmentation, sequence numbers,
and retransmission.

### Statistics

Global zero-copy statistics track efficiency:

```rust
pub static ZERO_COPY_STATS: ZeroCopyStats = ZeroCopyStats::new();
```

The `get_efficiency()` method reports the percentage of bytes transferred
without intermediate copies.

---

## Hardware NIC Drivers

### E1000 Driver

**Source:** `kernel/src/drivers/e1000.rs`

Full Intel 82540EM driver with DMA descriptor rings and MMIO register access.
This is the primary NIC driver for QEMU (`-device e1000`).

#### Register Map (MMIO offsets)

| Register | Offset | Purpose                          |
|----------|--------|----------------------------------|
| CTRL     | 0x0000 | Device Control                   |
| STATUS   | 0x0008 | Device Status                    |
| EEPROM   | 0x0014 | EEPROM Read                      |
| ICR      | 0x00C0 | Interrupt Cause Read             |
| IMS      | 0x00D0 | Interrupt Mask Set               |
| RCTL     | 0x0100 | Receive Control                  |
| TCTL     | 0x0400 | Transmit Control                 |
| RDBAL    | 0x2800 | RX Descriptor Base Address Low   |
| RDBAH    | 0x2804 | RX Descriptor Base Address High  |
| RDLEN    | 0x2808 | RX Descriptor Ring Length        |
| RDH      | 0x2810 | RX Descriptor Head               |
| RDT      | 0x2818 | RX Descriptor Tail               |
| TDBAL    | 0x3800 | TX Descriptor Base Address Low   |
| TDBAH    | 0x3804 | TX Descriptor Base Address High  |
| TDLEN    | 0x3808 | TX Descriptor Ring Length         |
| TDH      | 0x3810 | TX Descriptor Head               |
| TDT      | 0x3818 | TX Descriptor Tail               |

#### Descriptor Rings

TX and RX use `#[repr(C)]` descriptor structures for DMA:

```rust
#[repr(C)]
struct RxDescriptor {
    addr: u64,       // Physical address of receive buffer
    length: u16,     // Received packet length
    checksum: u16,   // Hardware checksum
    status: u8,      // Descriptor Done (DD) flag
    errors: u8,      // Error flags
    special: u16,    // VLAN tag
}

#[repr(C)]
struct TxDescriptor {
    addr: u64,       // Physical address of transmit buffer
    length: u16,     // Packet length
    cso: u8,         // Checksum offset
    cmd: u8,         // Command bits (EOP, IFCS, RS)
    status: u8,      // Descriptor Done (DD) flag
    css: u8,         // Checksum start
    special: u16,    // VLAN tag
}
```

Ring sizes:
- RX: 32 descriptors, each backed by a 2048-byte buffer
- TX: 8 descriptors, each backed by a 2048-byte buffer

#### MMIO Access

All register reads and writes use `read_volatile` / `write_volatile` to
prevent compiler reordering:

```rust
fn read_reg(&self, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((self.mmio_base + offset) as *const u32) }
}

fn write_reg(&self, offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((self.mmio_base + offset) as *mut u32, value); }
}
```

#### Transmit Flow

1. Copy packet data into `tx_buffers[tx_current]`
2. Set descriptor: `addr` = buffer physical address, `length` = packet length
3. Set `cmd` = EOP | IFCS | RS (end of packet, insert FCS, report status)
4. Write `tx_current` to TDT register (doorbell)
5. Advance `tx_current = (tx_current + 1) % NUM_TX_DESC`

#### Receive Flow

1. Check `rx_descriptors[rx_current].status & DD` (descriptor done)
2. Read `length` bytes from `rx_buffers[rx_current]`
3. Clear the status field and write `rx_current` to RDT register
4. Advance `rx_current = (rx_current + 1) % NUM_RX_DESC`

### VirtIO-Net Driver

**Source:** `kernel/src/drivers/virtio_net.rs`

VirtIO network driver for paravirtualized environments. Uses split virtqueues
for TX/RX. Registered via PCI scan (vendor 0x1AF4, device 0x1000 legacy /
0x1041 modern) on x86_64, or via MMIO at platform-specific addresses on
AArch64 and RISC-V.

### NetworkDevice Trait

All drivers implement the common `NetworkDevice` trait:

```rust
pub trait NetworkDevice: Send {
    fn name(&self) -> &str;
    fn mac_address(&self) -> MacAddress;
    fn capabilities(&self) -> DeviceCapabilities;
    fn state(&self) -> DeviceState;
    fn set_state(&mut self, state: DeviceState) -> Result<(), KernelError>;
    fn statistics(&self) -> DeviceStatistics;
    fn transmit(&mut self, packet: &Packet) -> Result<(), KernelError>;
    fn receive(&mut self) -> Result<Option<Packet>, KernelError>;
}
```

Capabilities include MTU, VLAN support, checksum offload, TSO, and LRO flags.
The loopback device (`lo0`) is always registered; hardware devices are added
during PCI/MMIO enumeration.

---

## IPv6 Dual-Stack

### IPv6 Protocol

**Source:** `kernel/src/net/ipv6.rs`

Full IPv6 implementation with 40-byte fixed header parsing and construction,
address classification, and NDP integration.

#### IPv6 Header

```rust
#[repr(C)]
pub struct Ipv6Header {
    pub version_tc_flow: u32,    // Version(4) + Traffic Class(8) + Flow Label(20)
    pub payload_length: u16,     // Payload length (big-endian)
    pub next_header: u8,         // TCP=6, UDP=17, ICMPv6=58
    pub hop_limit: u8,           // TTL equivalent
    pub source: [u8; 16],        // Source address
    pub destination: [u8; 16],   // Destination address
}
```

Key constants:
- `IPV6_HEADER_SIZE` = 40 bytes (fixed, unlike IPv4)
- `IPV6_MIN_MTU` = 1280 bytes (minimum link MTU for IPv6)
- `DEFAULT_HOP_LIMIT` = 64

#### Address Utilities

Classification functions for IPv6 address types:

| Function              | Prefix      | Description                  |
|-----------------------|-------------|------------------------------|
| `is_link_local()`     | `fe80::/10` | Link-local scope             |
| `is_multicast()`      | `ff00::/8`  | Multicast addresses          |
| `is_global_unicast()` | `2000::/3`  | Global unicast               |
| `is_unique_local()`   | `fc00::/7`  | Unique local (ULA)           |
| `is_ipv4_mapped()`    | `::ffff/96` | IPv4-mapped IPv6             |
| `is_loopback()`       | `::1`       | Loopback                     |

#### EUI-64 Interface ID

`link_local_from_mac()` generates a link-local address from a MAC address:

1. Insert `FF:FE` in the middle of the 48-bit MAC (creating 64-bit EUI-64)
2. Flip the Universal/Local (U/L) bit (bit 6 of byte 0)
3. Prepend `fe80::/10` prefix

Example: MAC `52:54:00:12:34:56` becomes `fe80::5054:ff:fe12:3456`

### NDP (Neighbor Discovery Protocol)

The NDP cache (`NdpCache`) maps IPv6 addresses to MAC addresses using a
`BTreeMap<Ipv6Address, NdpEntry>`:

```rust
pub struct NdpEntry {
    pub mac: MacAddress,
    pub state: NdpState,    // Incomplete | Reachable | Stale | Delay | Probe
    pub timestamp: u64,
    pub probe_count: u8,
}
```

Cache parameters:
- Maximum entries: 128 (LRU eviction when full)
- Reachable timeout: 30 ticks
- Stale timeout: 600 ticks (transitions to Incomplete)

NDP message types handled by `handle_ndp()`:

| Type | Code | Handler                         | Response            |
|------|------|---------------------------------|---------------------|
| 133  | RS   | Ignored (not a router)          | None                |
| 134  | RA   | `handle_router_advertisement()` | SLAAC configuration |
| 135  | NS   | `handle_neighbor_solicitation()` | NA (type 136)      |
| 136  | NA   | `handle_neighbor_advertisement()`| Cache update        |

#### SLAAC (Stateless Address Autoconfiguration)

Router Advertisement processing extracts prefix options (type 3) and generates
global addresses:

1. Parse RA: hop limit, router lifetime, reachable time
2. For each prefix option with the Autonomous flag set:
   - Extract prefix and prefix length
   - Generate interface ID via EUI-64 from MAC
   - Combine prefix + interface ID to form a global address
3. Add the address to `DualStackConfig.ipv6_addresses` with `Ipv6Scope::Global`

### ICMPv6

**Source:** `kernel/src/net/icmpv6.rs`

ICMPv6 message handling with checksum verification via the IPv6 pseudo-header.

Message types:

| Type | Name                  | Handler                     |
|------|-----------------------|-----------------------------|
| 1    | Destination Unreachable| Log with reason code        |
| 2    | Packet Too Big        | Log MTU value               |
| 3    | Time Exceeded         | Log with reason code        |
| 128  | Echo Request          | Build + send Echo Reply     |
| 129  | Echo Reply            | Update reply tracker        |
| 133-136 | NDP messages       | Delegate to `ipv6::handle_ndp` |

Message construction functions:
- `build_echo_request()` / `build_echo_reply()` -- Ping/pong
- `build_dest_unreachable()` -- With invoking packet (up to min MTU)
- `build_packet_too_big()` -- With path MTU value
- `build_time_exceeded()` -- Hop limit or reassembly timeout

All construction functions compute and fill the ICMPv6 checksum using the
IPv6 pseudo-header (source + destination + length + next header = 58).

### Dual-Stack Configuration

```rust
pub struct DualStackConfig {
    pub ipv4_enabled: bool,
    pub ipv6_enabled: bool,
    pub prefer_ipv6: bool,
    pub ipv6_addresses: Vec<Ipv6InterfaceAddr>,
}

pub struct Ipv6InterfaceAddr {
    pub address: Ipv6Address,
    pub prefix_len: u8,
    pub scope: Ipv6Scope,  // LinkLocal | Global | SiteLocal
}
```

The socket layer (`net/socket.rs`) supports `AF_INET6` alongside `AF_INET`
and `AF_UNIX`:

```rust
pub enum SocketDomain {
    Inet,   // AF_INET  (IPv4)
    Inet6,  // AF_INET6 (IPv6)
    Unix,   // AF_UNIX
}
```

---

## Driver Layer Integration

**Source:** `kernel/src/net/integration.rs`

The integration module bridges PCI hardware discovery with the network device
registry.

### Device Discovery

On x86_64, PCI bus enumeration searches for known network controllers:

| Vendor | Device | Driver     |
|--------|--------|------------|
| 0x8086 | 0x100E | E1000      |
| 0x8086 | 0x10D3 | E1000E     |
| 0x1AF4 | 0x1000 | VirtIO-Net (legacy) |
| 0x1AF4 | 0x1041 | VirtIO-Net (modern) |

On AArch64 and RISC-V, VirtIO MMIO devices are probed at platform-specific
base addresses.

### Registration Flow

```
PCI scan -> find_devices_by_id() -> get BAR0 address
         -> phys_to_virt() mapping
         -> E1000Driver::new(virt_addr) or VirtioNetDriver::new(base_addr)
         -> device::register_device(Box::new(driver))
```

The driver framework layer (`kernel/src/drivers/network.rs`) provides a higher-level
`NetworkManager` with interface registration, default route management, and
aggregate statistics. Hardware TX is routed through `net::device::with_device_mut()`,
and RX packets are polled in the interrupt handler via `dev.receive()`.

### PCI Validation

Network devices are validated by PCI class code (`DeviceClass::Network`,
class code `0x02`) during the driver probe phase. Only devices matching
the expected class are attached.

---

## Shell Commands

The following network commands are available in the VeridianOS shell:

| Command    | Description                                       |
|------------|---------------------------------------------------|
| `ifconfig` | Display interface configuration, addresses, stats |
| `netstat`  | Show active connections and socket statistics     |
| `dhcp`     | Trigger DHCP address acquisition                  |
| `arp`      | Display and manage the ARP cache                  |
| `ping6`    | Send ICMPv6 Echo Requests to an IPv6 address      |
| `ndp`      | Display and manage the NDP neighbor cache          |

### ping6

```
Usage: ping6 <ipv6-address> [count]
```

Sends ICMPv6 Echo Request messages and tracks replies via the global
`LAST_ECHO_REPLY` atomic counter. Displays per-packet round-trip information
and summary statistics.

### ndp

Displays the NDP neighbor cache entries with IPv6 address, MAC address,
and state (Incomplete / Reachable / Stale / Delay / Probe).

---

## Key Source Files

| Path                              | Purpose                                     |
|-----------------------------------|---------------------------------------------|
| `kernel/src/net/mod.rs`           | Network stack entry point, init(), stats     |
| `kernel/src/net/dma_pool.rs`      | DMA buffer pool (pre-allocated, < 4GB)       |
| `kernel/src/net/zero_copy.rs`     | Scatter-gather, sendfile, TCP cork           |
| `kernel/src/net/device.rs`        | NetworkDevice trait, device registry          |
| `kernel/src/net/ip.rs`            | IPv4 header parsing, routing                 |
| `kernel/src/net/ipv6.rs`          | IPv6 header, NDP cache, SLAAC, dual-stack    |
| `kernel/src/net/icmpv6.rs`        | ICMPv6 messages, checksum, echo              |
| `kernel/src/net/tcp.rs`           | TCP state machine, 3-way handshake           |
| `kernel/src/net/udp.rs`           | UDP datagram handling                        |
| `kernel/src/net/socket.rs`        | BSD socket API (AF_INET, AF_INET6, AF_UNIX)  |
| `kernel/src/net/arp.rs`           | ARP cache and resolution                     |
| `kernel/src/net/dhcp.rs`          | DHCP client                                  |
| `kernel/src/net/ethernet.rs`      | Ethernet frame handling                      |
| `kernel/src/net/epoll.rs`         | epoll I/O multiplexing                       |
| `kernel/src/net/integration.rs`   | PCI scan, driver registration                |
| `kernel/src/net/unix_socket.rs`   | Unix domain sockets                          |
| `kernel/src/drivers/e1000.rs`     | Intel E1000 NIC driver (MMIO + DMA rings)    |
| `kernel/src/drivers/virtio_net.rs`| VirtIO-Net paravirtual driver                |
| `kernel/src/drivers/network.rs`   | NetworkManager, EthernetDriver framework     |

---

## Performance Characteristics

| Metric                  | Target     | Notes                                    |
|-------------------------|------------|------------------------------------------|
| DMA buffer allocation   | O(1)       | Free list pop                            |
| NDP cache lookup        | O(log n)   | BTreeMap, max 128 entries                |
| Device registry lookup  | O(n)       | Linear scan by name (small n)            |
| Zero-copy user TX       | 0 copies   | Page pinning + SG list                   |
| SendFile (>= 64KB)      | 1 copy     | SG assembly into contiguous buffer       |
| SendFile (< 64KB)       | 2 copies   | Read + write through 4KB stack buffer    |
| TCP cork flush          | 1 copy     | Pending buffer to tcp::transmit_data()   |
| E1000 TX                | 1 copy     | Packet data into TX ring buffer          |

The zero-copy statistics (`ZERO_COPY_STATS`) track the ratio of zero-copy
bytes to copied bytes, allowing runtime efficiency monitoring.

---

## Future Work (Phase 7 Waves 4-6)

- **Wave 4 (Multimedia):** Audio/video streaming over network sockets,
  RTP/RTSP protocol support
- **Wave 5 (Virtualization):** Network namespaces, virtual bridge devices,
  veth pairs for container networking
- **Wave 6 (Cloud-Native):** OCI container networking, overlay networks,
  service mesh integration
- **Hardware offload:** TSO/LRO support in E1000 and VirtIO-Net drivers
- **Path MTU Discovery:** Dynamic MSS adjustment in TcpZeroCopySend
- **RSS (Receive Side Scaling):** Multi-queue support for SMP packet
  distribution
