# Network Programming Guide

VeridianOS provides a high-performance network stack with both POSIX compatibility and native APIs. This guide covers network programming from basic sockets to advanced zero-copy techniques.

## Architecture Overview

### Network Stack Layers

```
┌─────────────────────────────────────────┐
│         Application Layer               │
├─────────────────────────────────────────┤
│    Socket API (POSIX Compatible)        │
├─────────────────────────────────────────┤
│        VeridianOS Native API            │
├─────────────────────────────────────────┤
│    Protocol Layer (TCP/UDP/QUIC)        │
├─────────────────────────────────────────┤
│      Network Layer (IPv4/IPv6)          │
├─────────────────────────────────────────┤
│      Link Layer (Ethernet)              │
├─────────────────────────────────────────┤
│    Device Driver (User Space)           │
└─────────────────────────────────────────┘
```

### Design Principles

1. **Zero-Copy Architecture**: Minimize data copying
2. **Async/Await Native**: Built for async programming
3. **Capability Security**: Network access requires capabilities
4. **NUMA Aware**: Optimize for multi-socket systems
5. **Hardware Offload**: Utilize NIC features

## POSIX Socket API

### Basic TCP Server

```rust
use veridian::net::posix::*;

fn tcp_server() -> Result<(), Error> {
    // Create socket
    let sock = socket(AF_INET, SOCK_STREAM, 0)?;
    
    // Bind to address
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    bind(sock, &addr)?;
    
    // Listen for connections
    listen(sock, 128)?;
    
    loop {
        // Accept connection
        let (client, client_addr) = accept(sock)?;
        
        // Handle client in new thread
        thread::spawn(move || {
            handle_client(client);
        });
    }
}

fn handle_client(sock: Socket) -> Result<(), Error> {
    let mut buf = [0u8; 1024];
    
    loop {
        // Receive data
        let n = recv(sock, &mut buf, 0)?;
        if n == 0 {
            break; // Connection closed
        }
        
        // Echo back
        send(sock, &buf[..n], 0)?;
    }
    
    close(sock)
}
```

### UDP Socket

```rust
fn udp_server() -> Result<(), Error> {
    // Create UDP socket
    let sock = socket(AF_INET, SOCK_DGRAM, 0)?;
    
    // Bind to port
    let addr = SocketAddr::from(([0, 0, 0, 0], 9999));
    bind(sock, &addr)?;
    
    let mut buf = [0u8; 65535];
    loop {
        // Receive datagram
        let (n, from) = recvfrom(sock, &mut buf, 0)?;
        
        // Process and reply
        let response = process_request(&buf[..n]);
        sendto(sock, response, 0, &from)?;
    }
}
```

## Native Network API

### High-Performance TCP

```rust
use veridian::net::native::*;

async fn native_tcp_server() -> Result<(), Error> {
    // Create endpoint with capability
    let endpoint = TcpEndpoint::create(
        "0.0.0.0:8080",
        NetworkCapability::new()?,
    ).await?;
    
    // Configure for performance
    endpoint.set_option(TcpOption::NoDelay(true))?;
    endpoint.set_option(TcpOption::ReusePort(true))?;
    
    loop {
        // Accept with zero-copy
        let (stream, peer) = endpoint.accept().await?;
        
        // Handle concurrently
        tokio::spawn(handle_stream(stream));
    }
}

async fn handle_stream(mut stream: TcpStream) -> Result<(), Error> {
    // Zero-copy receive
    let mut buffer = stream.alloc_buffer(64 * 1024)?;
    
    loop {
        // Receive into pre-allocated buffer
        let n = stream.recv_zero_copy(&mut buffer).await?;
        if n == 0 {
            break;
        }
        
        // Process in-place
        process_in_place(&mut buffer[..n]);
        
        // Zero-copy send
        stream.send_zero_copy(&buffer[..n]).await?;
    }
    
    Ok(())
}
```

### io_uring Integration

```rust
use veridian::net::io_uring::*;

async fn io_uring_server() -> Result<(), Error> {
    // Create io_uring instance
    let mut ring = IoUring::new(256)?;
    
    // Register fixed buffers
    let buffers = ring.register_buffers(1024, 4096)?;
    
    // Setup multishot accept
    let listener = TcpListener::bind("0.0.0.0:8080")?;
    ring.submit_multishot_accept(listener.as_fd())?;
    
    // Event loop
    loop {
        // Wait for completions
        let cqes = ring.wait_cqes(1).await?;
        
        for cqe in cqes {
            match cqe.user_data() {
                ACCEPT_ID => {
                    let fd = cqe.result()?;
                    // Submit read
                    ring.submit_read(fd, buffers.get_mut(fd)?, 0)?;
                }
                _ => {
                    // Handle read/write completion
                    handle_completion(cqe)?;
                }
            }
        }
    }
}
```

## Zero-Copy Techniques

### Shared Memory Buffers

```rust
pub struct ZeroCopyBuffer {
    // Shared memory region
    memory: SharedMemory,
    
    // Producer/consumer indices
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
}

impl ZeroCopyBuffer {
    pub fn write_packet(&self, packet: &[u8]) -> Result<(), Error> {
        let idx = self.write_idx.load(Ordering::Acquire);
        let slot = self.get_slot(idx);
        
        // Copy header only
        slot.header.len = packet.len() as u32;
        
        // Zero-copy payload
        if packet.len() <= INLINE_SIZE {
            slot.inline_data[..packet.len()].copy_from_slice(packet);
        } else {
            // Reference to external buffer
            slot.external_ref = packet.as_ptr() as u64;
        }
        
        self.write_idx.store(idx + 1, Ordering::Release);
        Ok(())
    }
}
```

### Kernel Bypass

```rust
use veridian::net::dpdk::*;

fn dpdk_packet_processing() -> Result<(), Error> {
    // Initialize DPDK
    let mut dpdk = DpdkContext::init(&[
        "-n", "4",
        "-l", "0-3",
        "--", 
    ])?;
    
    // Configure port
    let port = dpdk.configure_port(0, PortConfig {
        rx_queues: 4,
        tx_queues: 4,
        rx_desc: 1024,
        tx_desc: 1024,
    })?;
    
    // Setup memory pool
    let mempool = dpdk.create_mempool("packets", 8192, 2048)?;
    
    // Packet processing loop
    loop {
        let mut packets = [MBuf::null(); 32];
        
        // Receive burst
        let n = port.rx_burst(0, &mut packets)?;
        
        // Process packets
        for i in 0..n {
            process_packet(&mut packets[i]);
        }
        
        // Transmit burst
        port.tx_burst(0, &packets[..n])?;
    }
}
```

## Protocol Implementation

### Custom Protocol

```rust
pub struct CustomProtocol {
    // Protocol state
    state: ProtocolState,
    
    // Registered handlers
    handlers: HashMap<MessageType, Handler>,
}

impl Protocol for CustomProtocol {
    fn process_packet(&mut self, packet: &mut Packet) -> Result<Action, Error> {
        // Parse header
        let header = CustomHeader::parse(&packet.data)?;
        
        // Validate
        if !self.validate_sequence(header.seq) {
            return Ok(Action::Drop);
        }
        
        // Dispatch to handler
        if let Some(handler) = self.handlers.get(&header.msg_type) {
            handler(self, packet)
        } else {
            Ok(Action::Drop)
        }
    }
}
```

### Protocol Stack Integration

```rust
pub fn register_custom_protocol() -> Result<(), Error> {
    let mut stack = NetworkStack::get()?;
    
    // Register at transport layer
    stack.register_transport_protocol(
        IPPROTO_CUSTOM,
        Box::new(CustomProtocol::new()),
    )?;
    
    // Setup flow director
    stack.configure_flow_director(|packet| {
        if packet.protocol() == IPPROTO_CUSTOM {
            // Direct to specific CPU
            FlowAction::SteerToCpu(2)
        } else {
            FlowAction::Default
        }
    })?;
    
    Ok(())
}
```

## Performance Optimization

### CPU Affinity

```rust
pub struct AffinityConfig {
    // Interrupt affinity
    irq_cpu: CpuSet,
    
    // Worker thread affinity
    workers: Vec<CpuSet>,
    
    // NUMA node binding
    numa_node: Option<NumaNode>,
}

pub fn optimize_network_performance(config: AffinityConfig) -> Result<(), Error> {
    // Bind interrupts
    for irq in network_irqs()? {
        irq.set_affinity(&config.irq_cpu)?;
    }
    
    // Create workers with affinity
    for (i, cpuset) in config.workers.iter().enumerate() {
        thread::Builder::new()
            .name(format!("net-worker-{}", i))
            .spawn_on(cpuset.clone(), || {
                network_worker_loop()
            })?;
    }
    
    Ok(())
}
```

### Batching

```rust
pub struct BatchedSender {
    socket: UdpSocket,
    batch: Vec<(SocketAddr, Vec<u8>)>,
    max_batch: usize,
}

impl BatchedSender {
    pub async fn send(&mut self, addr: SocketAddr, data: Vec<u8>) -> Result<(), Error> {
        self.batch.push((addr, data));
        
        if self.batch.len() >= self.max_batch {
            self.flush().await?;
        }
        
        Ok(())
    }
    
    pub async fn flush(&mut self) -> Result<(), Error> {
        if self.batch.is_empty() {
            return Ok(());
        }
        
        // Vectored I/O
        let iovecs: Vec<IoVec> = self.batch.iter()
            .map(|(_, data)| IoVec::from_slice(data))
            .collect();
        
        // Single syscall
        self.socket.sendmmsg(&self.batch, &iovecs).await?;
        
        self.batch.clear();
        Ok(())
    }
}
```

## Security Considerations

### Network Capabilities

```rust
pub fn create_restricted_socket() -> Result<Socket, Error> {
    // Request network capability
    let net_cap = request_capability(CapabilityType::Network)?;
    
    // Restrict to specific operations
    let restricted = net_cap.derive(NetworkRights::TCP | NetworkRights::CONNECT)?;
    
    // Create socket with capability
    Socket::create_with_cap(restricted)
}
```

### Rate Limiting

```rust
pub struct RateLimiter {
    buckets: HashMap<IpAddr, TokenBucket>,
    max_rate: u64,
}

impl RateLimiter {
    pub fn check_and_update(&mut self, addr: IpAddr) -> bool {
        let bucket = self.buckets.entry(addr)
            .or_insert_with(|| TokenBucket::new(self.max_rate));
        
        bucket.try_consume(1)
    }
}
```

## Best Practices

1. **Use async/await**: Better scalability than threads
2. **Batch operations**: Reduce syscall overhead
3. **Zero-copy when possible**: Especially for large transfers
4. **Set appropriate buffers**: Tune for workload
5. **Monitor performance**: Use built-in metrics
6. **Handle errors gracefully**: Network is unreliable
7. **Implement timeouts**: Prevent resource exhaustion
8. **Use hardware offload**: Checksums, segmentation